//! Bounded, read-only local observability provider.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, Fingerprint, ImplementationId,
        MAX_OBSERVABILITY_EXPORT_RANGE_MS, MAX_OBSERVABILITY_QUERY_BYTES,
        MAX_OBSERVABILITY_QUERY_LIMIT, MAX_PROVIDER_REGISTRY_ENTRIES, MAX_SAFE_JSON_INTEGER,
        MutationReceipt, MutationState, OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES,
        ObservabilityCursor, ObservabilityExportFormat, ObservabilityLabels, ObservabilityProvider,
        ObservabilityQueryResult, ObservabilityRecord, ObservabilityView, ObservationReason,
        ObservedLifecycleState, OperationBinding, Provider, ProviderCallContext,
        ProviderCapability, ProviderCapabilitySet, ProviderContractError, ProviderDescriptor,
        ProviderFactoryKey, ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle,
        ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
        ProviderObservation, ProviderOperationContext, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlacement, ProviderRemediation, ProviderTarget,
        RetryClass,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;

pub use d2b_contracts::v2_provider::{
    ObservabilityMetricLabel as MetricLabel, ObservabilityOperationLabel as OperationLabel,
    ObservabilityOutcomeLabel as OutcomeLabel, ObservabilityProjectionKind as ProjectionKind,
};

pub const MAX_LOCAL_OBSERVABILITY_BYTES: u32 = MAX_OBSERVABILITY_QUERY_BYTES;
pub const OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES: u32 =
    OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES;
pub const IMPLEMENTATION_ID: &str = "local";

pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("static implementation id is valid"))
}

pub fn factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Observability,
        implementation_id: implementation_id(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricLabelKey {
    Locality,
    ProviderType,
    HealthState,
    Metric,
    Operation,
    Outcome,
}

impl MetricLabelKey {
    pub const ALL: [Self; 6] = [
        Self::Locality,
        Self::ProviderType,
        Self::HealthState,
        Self::Metric,
        Self::Operation,
        Self::Outcome,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LocalityLabel {
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClosedMetricLabels {
    locality: LocalityLabel,
    provider_type: ProviderType,
    health_state: ProviderHealthState,
    metric: MetricLabel,
    operation: OperationLabel,
    outcome: OutcomeLabel,
}

impl ClosedMetricLabels {
    pub const fn new(
        provider_type: ProviderType,
        health_state: ProviderHealthState,
        metric: MetricLabel,
        operation: OperationLabel,
        outcome: OutcomeLabel,
    ) -> Self {
        Self {
            locality: LocalityLabel::Local,
            provider_type,
            health_state,
            metric,
            operation,
            outcome,
        }
    }

    pub const fn locality(self) -> LocalityLabel {
        self.locality
    }

    pub const fn provider_type(self) -> ProviderType {
        self.provider_type
    }

    pub const fn health_state(self) -> ProviderHealthState {
        self.health_state
    }

    pub const fn metric(self) -> MetricLabel {
        self.metric
    }

    pub const fn operation(self) -> OperationLabel {
        self.operation
    }

    pub const fn outcome(self) -> OutcomeLabel {
        self.outcome
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalObservationRecord {
    observed_at_unix_ms: u64,
    projection: ProjectionKind,
    labels: ClosedMetricLabels,
    value: u64,
}

impl LocalObservationRecord {
    pub fn new(
        observed_at_unix_ms: u64,
        projection: ProjectionKind,
        labels: ClosedMetricLabels,
        value: u64,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        if observed_at_unix_ms > MAX_SAFE_JSON_INTEGER || value > MAX_SAFE_JSON_INTEGER {
            return Err(ObservabilityProviderBuildError::InvalidProjection);
        }
        Ok(Self {
            observed_at_unix_ms,
            projection,
            labels,
            value,
        })
    }

    pub const fn observed_at_unix_ms(self) -> u64 {
        self.observed_at_unix_ms
    }

    pub const fn projection(self) -> ProjectionKind {
        self.projection
    }

    pub const fn labels(self) -> ClosedMetricLabels {
        self.labels
    }

    pub const fn value(self) -> u64 {
        self.value
    }

    fn allowed_for(self, view: ObservabilityView) -> bool {
        match view {
            ObservabilityView::Health => matches!(
                self.labels.metric(),
                MetricLabel::ProviderHealth | MetricLabel::QueueDepth
            ),
            ObservabilityView::Lifecycle => {
                self.labels.metric() == MetricLabel::LifecycleTransition
            }
            ObservabilityView::Operations => matches!(
                self.labels.metric(),
                MetricLabel::OperationTotal
                    | MetricLabel::OperationDuration
                    | MetricLabel::ExportTruncated
            ),
        }
    }

    fn into_contract(self) -> ObservabilityRecord {
        ObservabilityRecord {
            observed_at_unix_ms: self.observed_at_unix_ms,
            projection: self.projection,
            labels: ObservabilityLabels {
                provider_type: self.labels.provider_type(),
                health_state: self.labels.health_state(),
                metric: self.labels.metric(),
                operation: self.labels.operation(),
                outcome: self.labels.outcome(),
            },
            value: self.value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservabilityLimits {
    max_records: u16,
    max_bytes: u32,
    max_time_window_ms: u64,
}

impl ObservabilityLimits {
    pub fn new(
        max_records: u16,
        max_bytes: u32,
        max_time_window_ms: u64,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        if max_records == 0
            || max_records > MAX_OBSERVABILITY_QUERY_LIMIT
            || !(OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES..=MAX_LOCAL_OBSERVABILITY_BYTES)
                .contains(&max_bytes)
            || max_time_window_ms == 0
            || max_time_window_ms > MAX_OBSERVABILITY_EXPORT_RANGE_MS
        {
            return Err(ObservabilityProviderBuildError::InvalidLimits);
        }
        Ok(Self {
            max_records,
            max_bytes,
            max_time_window_ms,
        })
    }

    pub const fn max_records(self) -> u16 {
        self.max_records
    }

    pub const fn max_bytes(self) -> u32 {
        self.max_bytes
    }

    pub const fn max_time_window_ms(self) -> u64 {
        self.max_time_window_ms
    }

    fn query_bounds(self, requested_records: u16) -> ProjectionBounds {
        ProjectionBounds {
            max_records: requested_records.min(self.max_records),
            max_bytes: self.max_bytes,
        }
    }

    fn export_bounds(self) -> ProjectionBounds {
        ProjectionBounds {
            max_records: self.max_records,
            max_bytes: self.max_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionBounds {
    pub max_records: u16,
    pub max_bytes: u32,
}

impl ProjectionBounds {
    fn record_capacity(self) -> usize {
        usize::from(self.max_records).min(
            usize::try_from(self.max_bytes / OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES)
                .unwrap_or(usize::MAX),
        )
    }
}

#[derive(Clone)]
pub struct ObservabilityCall {
    operation: ProviderOperationContext,
    peer_role: EndpointRole,
    monotonic_deadline_remaining_ms: u32,
}

impl fmt::Debug for ObservabilityCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservabilityCall")
            .field("method", &self.operation.method)
            .field("provider_generation", &self.operation.provider_generation)
            .field("peer_role", &self.peer_role)
            .field(
                "monotonic_deadline_remaining_ms",
                &self.monotonic_deadline_remaining_ms,
            )
            .finish_non_exhaustive()
    }
}

impl ObservabilityCall {
    pub fn operation(&self) -> &ProviderOperationContext {
        &self.operation
    }

    pub fn binding(&self) -> OperationBinding {
        self.operation.binding()
    }

    pub fn scope(&self) -> &AuthorizedProviderScope {
        &self.operation.scope
    }

    pub const fn monotonic_deadline_remaining_ms(&self) -> u32 {
        self.monotonic_deadline_remaining_ms
    }
}

#[derive(Clone)]
pub struct ObservabilityQueryIntent {
    pub view: ObservabilityView,
    cursor: Option<ObservabilityCursor>,
    pub bounds: ProjectionBounds,
}

impl fmt::Debug for ObservabilityQueryIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservabilityQueryIntent")
            .field("view", &self.view)
            .field("cursor", &self.cursor.as_ref().map(|_| "<redacted>"))
            .field("bounds", &self.bounds)
            .finish()
    }
}

impl ObservabilityQueryIntent {
    pub fn cursor(&self) -> Option<&ObservabilityCursor> {
        self.cursor.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservabilityExportIntent {
    pub format: ObservabilityExportFormat,
    pub start_at_unix_ms: u64,
    pub end_at_unix_ms: u64,
    pub bounds: ProjectionBounds,
}

#[derive(Clone)]
pub struct ProjectionPage {
    records: Vec<LocalObservationRecord>,
    next_cursor: Option<ObservabilityCursor>,
    source_truncated: bool,
}

impl fmt::Debug for ProjectionPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionPage")
            .field("record_count", &self.records.len())
            .field("has_next_cursor", &self.next_cursor.is_some())
            .field("source_truncated", &self.source_truncated)
            .finish()
    }
}

impl ProjectionPage {
    pub fn new(
        records: Vec<LocalObservationRecord>,
        next_cursor: Option<ObservabilityCursor>,
        source_truncated: bool,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        if records.len() > usize::from(MAX_OBSERVABILITY_QUERY_LIMIT)
            || records.iter().any(|record| {
                record.observed_at_unix_ms() > MAX_SAFE_JSON_INTEGER
                    || record.value() > MAX_SAFE_JSON_INTEGER
            })
            || records.windows(2).any(|pair| pair[0] >= pair[1])
            || source_truncated != next_cursor.is_some()
        {
            return Err(ObservabilityProviderBuildError::InvalidProjection);
        }
        Ok(Self {
            records,
            next_cursor,
            source_truncated,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BoundedProjection {
    binding: OperationBinding,
    records: Vec<LocalObservationRecord>,
    next_cursor: Option<ObservabilityCursor>,
    encoded_bytes_upper_bound: u32,
    truncated: bool,
}

impl fmt::Debug for BoundedProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedProjection")
            .field("binding", &self.binding)
            .field("record_count", &self.records.len())
            .field("has_next_cursor", &self.next_cursor.is_some())
            .field("encoded_bytes_upper_bound", &self.encoded_bytes_upper_bound)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl BoundedProjection {
    pub fn binding(&self) -> &OperationBinding {
        &self.binding
    }

    pub fn records(&self) -> &[LocalObservationRecord] {
        &self.records
    }

    pub fn next_cursor(&self) -> Option<&ObservabilityCursor> {
        self.next_cursor.as_ref()
    }

    pub const fn encoded_bytes_upper_bound(&self) -> u32 {
        self.encoded_bytes_upper_bound
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSinkStatus {
    Emitted,
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSinkError {
    Closed,
    OutsideWindow,
    Encoding,
    Poisoned,
}

fn projection_label(projection: ProjectionKind) -> &'static str {
    match projection {
        ProjectionKind::Metrics => "metrics",
        ProjectionKind::TraceSummary => "trace-summary",
        ProjectionKind::AuditSummary => "audit-summary",
    }
}

fn health_label(health: ProviderHealthState) -> &'static str {
    match health {
        ProviderHealthState::Healthy => "healthy",
        ProviderHealthState::Degraded => "degraded",
        ProviderHealthState::Unavailable => "unavailable",
        ProviderHealthState::Failed => "failed",
    }
}

fn metric_label(metric: MetricLabel) -> &'static str {
    match metric {
        MetricLabel::ProviderHealth => "provider-health",
        MetricLabel::LifecycleTransition => "lifecycle-transition",
        MetricLabel::OperationTotal => "operation-total",
        MetricLabel::OperationDuration => "operation-duration",
        MetricLabel::QueueDepth => "queue-depth",
        MetricLabel::ExportTruncated => "export-truncated",
    }
}

fn operation_label(operation: OperationLabel) -> &'static str {
    match operation {
        OperationLabel::Health => "health",
        OperationLabel::Plan => "plan",
        OperationLabel::Ensure => "ensure",
        OperationLabel::Start => "start",
        OperationLabel::Stop => "stop",
        OperationLabel::Attach => "attach",
        OperationLabel::Detach => "detach",
        OperationLabel::Adopt => "adopt",
        OperationLabel::Inspect => "inspect",
        OperationLabel::SetState => "set-state",
        OperationLabel::Query => "query",
        OperationLabel::Export => "export",
    }
}

fn outcome_label(outcome: OutcomeLabel) -> &'static str {
    match outcome {
        OutcomeLabel::Success => "success",
        OutcomeLabel::AlreadyApplied => "already-applied",
        OutcomeLabel::Denied => "denied",
        OutcomeLabel::Cancelled => "cancelled",
        OutcomeLabel::DeadlineExpired => "deadline-expired",
        OutcomeLabel::Unavailable => "unavailable",
        OutcomeLabel::Truncated => "truncated",
    }
}

fn otlp_metric_name(metric: MetricLabel) -> &'static str {
    match metric {
        MetricLabel::ProviderHealth => "d2b.provider.health",
        MetricLabel::LifecycleTransition => "d2b.lifecycle.transition",
        MetricLabel::OperationTotal => "d2b.operation.total",
        MetricLabel::OperationDuration => "d2b.operation.duration",
        MetricLabel::QueueDepth => "d2b.queue.depth",
        MetricLabel::ExportTruncated => "d2b.export.truncated",
    }
}

fn protobuf_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn protobuf_key(field: u32, wire_type: u8, output: &mut Vec<u8>) {
    protobuf_varint((u64::from(field) << 3) | u64::from(wire_type), output);
}

fn protobuf_message(field: u32, value: &[u8], output: &mut Vec<u8>) {
    protobuf_key(field, 2, output);
    protobuf_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}

fn protobuf_string(field: u32, value: &str, output: &mut Vec<u8>) {
    protobuf_message(field, value.as_bytes(), output);
}

fn protobuf_fixed64(field: u32, value: u64, output: &mut Vec<u8>) {
    protobuf_key(field, 1, output);
    output.extend_from_slice(&value.to_le_bytes());
}

fn otlp_string_value(value: &str) -> Vec<u8> {
    let mut encoded = Vec::new();
    protobuf_string(1, value, &mut encoded);
    encoded
}

fn otlp_attribute(key: &str, value: &str) -> Vec<u8> {
    let mut encoded = Vec::new();
    protobuf_string(1, key, &mut encoded);
    protobuf_message(2, &otlp_string_value(value), &mut encoded);
    encoded
}

fn encode_otlp_metric(record: LocalObservationRecord) -> Result<Vec<u8>, ExportSinkError> {
    let labels = record.labels();
    let observed_at_unix_nano = record
        .observed_at_unix_ms()
        .checked_mul(1_000_000)
        .ok_or(ExportSinkError::Encoding)?;
    let value = i64::try_from(record.value()).map_err(|_| ExportSinkError::Encoding)?;

    let mut point = Vec::new();
    for (key, value) in [
        ("d2b.locality", "local"),
        ("d2b.projection", projection_label(record.projection())),
        ("d2b.provider.type", labels.provider_type().as_str()),
        ("d2b.health.state", health_label(labels.health_state())),
        ("d2b.metric", metric_label(labels.metric())),
        ("d2b.operation", operation_label(labels.operation())),
        ("d2b.outcome", outcome_label(labels.outcome())),
    ] {
        protobuf_message(7, &otlp_attribute(key, value), &mut point);
    }
    protobuf_fixed64(3, observed_at_unix_nano, &mut point);
    protobuf_fixed64(6, value as u64, &mut point);

    let mut gauge = Vec::new();
    protobuf_message(1, &point, &mut gauge);

    let mut metric = Vec::new();
    protobuf_string(1, otlp_metric_name(labels.metric()), &mut metric);
    protobuf_message(5, &gauge, &mut metric);
    Ok(metric)
}

fn encode_otlp_export(records: &[LocalObservationRecord]) -> Result<Vec<u8>, ExportSinkError> {
    if records.is_empty() {
        return Ok(Vec::new());
    }

    let mut scope = Vec::new();
    protobuf_string(1, "d2b.provider.observability.local", &mut scope);

    let mut scope_metrics = Vec::new();
    protobuf_message(1, &scope, &mut scope_metrics);
    for record in records {
        protobuf_message(2, &encode_otlp_metric(*record)?, &mut scope_metrics);
    }

    let mut resource_metrics = Vec::new();
    protobuf_message(2, &scope_metrics, &mut resource_metrics);

    let mut request = Vec::new();
    protobuf_message(1, &resource_metrics, &mut request);
    Ok(request)
}

fn encode_json_lines(records: &[LocalObservationRecord]) -> Result<Vec<u8>, ExportSinkError> {
    let mut output = Vec::new();
    for record in records {
        output.extend_from_slice(
            &serde_json::to_vec(&record.into_contract()).map_err(|_| ExportSinkError::Encoding)?,
        );
        output.push(b'\n');
    }
    Ok(output)
}

fn encode_export_payload(
    format: ObservabilityExportFormat,
    records: &[LocalObservationRecord],
) -> Result<Vec<u8>, ExportSinkError> {
    match format {
        ObservabilityExportFormat::JsonLines => encode_json_lines(records),
        ObservabilityExportFormat::OtlpProtobuf => encode_otlp_export(records),
    }
}

#[derive(Default)]
struct ExportSinkState {
    records: Vec<LocalObservationRecord>,
    payload: Vec<u8>,
    encoded_bytes: u32,
    truncated: bool,
    closed: bool,
}

#[derive(Clone)]
pub struct BoundedExportSink {
    format: ObservabilityExportFormat,
    bounds: ProjectionBounds,
    start_at_unix_ms: u64,
    end_at_unix_ms: u64,
    state: Arc<Mutex<ExportSinkState>>,
}

impl fmt::Debug for BoundedExportSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.state.lock().ok();
        formatter
            .debug_struct("BoundedExportSink")
            .field("format", &self.format)
            .field("bounds", &self.bounds)
            .field(
                "record_count",
                &snapshot.as_ref().map(|state| state.records.len()),
            )
            .field(
                "encoded_bytes",
                &snapshot.as_ref().map(|state| state.encoded_bytes),
            )
            .field("closed", &snapshot.as_ref().map(|state| state.closed))
            .finish_non_exhaustive()
    }
}

impl BoundedExportSink {
    fn new(
        format: ObservabilityExportFormat,
        bounds: ProjectionBounds,
        start_at_unix_ms: u64,
        end_at_unix_ms: u64,
    ) -> Self {
        Self {
            format,
            bounds,
            start_at_unix_ms,
            end_at_unix_ms,
            state: Arc::new(Mutex::new(ExportSinkState::default())),
        }
    }

    pub fn emit(
        &self,
        record: LocalObservationRecord,
    ) -> Result<ExportSinkStatus, ExportSinkError> {
        let mut state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        if record.observed_at_unix_ms() < self.start_at_unix_ms
            || record.observed_at_unix_ms() > self.end_at_unix_ms
        {
            return Err(ExportSinkError::OutsideWindow);
        }
        if state.records.len() >= usize::from(self.bounds.max_records) {
            state.truncated = true;
            return Ok(ExportSinkStatus::Truncated);
        }
        let mut candidate = state.records.clone();
        candidate.push(record);
        let payload = encode_export_payload(self.format, &candidate)?;
        let encoded_bytes = u32::try_from(payload.len()).map_err(|_| ExportSinkError::Encoding)?;
        if encoded_bytes > self.bounds.max_bytes {
            state.truncated = true;
            return Ok(ExportSinkStatus::Truncated);
        }
        state.records = candidate;
        state.payload = payload;
        state.encoded_bytes = encoded_bytes;
        Ok(ExportSinkStatus::Emitted)
    }

    pub fn mark_source_truncated(&self) -> Result<(), ExportSinkError> {
        let mut state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        state.truncated = true;
        Ok(())
    }

    pub fn encoded_payload(&self) -> Result<Vec<u8>, ExportSinkError> {
        let state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        Ok(state.payload.clone())
    }

    pub fn record_count(&self) -> Result<u16, ExportSinkError> {
        let state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        u16::try_from(state.records.len()).map_err(|_| ExportSinkError::Encoding)
    }

    pub fn encoded_bytes(&self) -> Result<u32, ExportSinkError> {
        let state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        Ok(state.encoded_bytes)
    }

    pub fn truncated(&self) -> Result<bool, ExportSinkError> {
        let state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        Ok(state.truncated)
    }

    fn finish(&self) -> Result<ExportSinkSnapshot, ExportSinkError> {
        let mut state = self.state.lock().map_err(|_| ExportSinkError::Poisoned)?;
        if state.closed {
            return Err(ExportSinkError::Closed);
        }
        state.closed = true;
        Ok(ExportSinkSnapshot {
            records: std::mem::take(&mut state.records),
            encoded_bytes: state.encoded_bytes,
            truncated: state.truncated,
        })
    }
}

struct ExportSinkSnapshot {
    records: Vec<LocalObservationRecord>,
    encoded_bytes: u32,
    truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportPortOutcome {
    state: MutationState,
}

impl ExportPortOutcome {
    pub const fn new(state: MutationState) -> Self {
        Self { state }
    }

    pub const fn state(self) -> MutationState {
        self.state
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BoundedExport {
    binding: OperationBinding,
    state: MutationState,
    records: Vec<LocalObservationRecord>,
    encoded_bytes: u32,
    truncated: bool,
}

impl fmt::Debug for BoundedExport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedExport")
            .field("binding", &self.binding)
            .field("state", &self.state)
            .field("record_count", &self.records.len())
            .field("encoded_bytes", &self.encoded_bytes)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl BoundedExport {
    pub fn binding(&self) -> &OperationBinding {
        &self.binding
    }

    pub const fn state(&self) -> MutationState {
        self.state
    }

    pub fn records(&self) -> &[LocalObservationRecord] {
        &self.records
    }

    pub fn record_count(&self) -> u16 {
        u16::try_from(self.records.len()).unwrap_or(u16::MAX)
    }

    pub const fn encoded_bytes(&self) -> u32 {
        self.encoded_bytes
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalObservabilityStatus {
    pub health_state: ProviderHealthState,
    pub health_reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl LocalObservabilityStatus {
    pub const fn healthy() -> Self {
        Self {
            health_state: ProviderHealthState::Healthy,
            health_reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservabilityPortError {
    Denied,
    Unavailable,
    AmbiguousMutation,
    InvalidProjection,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservabilityDispatch {
    Read,
    Mutation,
}

#[async_trait]
pub trait ObservabilityQueryPort: Send + Sync {
    async fn health(
        &self,
        context: ObservabilityCall,
    ) -> Result<LocalObservabilityStatus, ObservabilityPortError>;

    async fn status(
        &self,
        context: ObservabilityCall,
    ) -> Result<LocalObservabilityStatus, ObservabilityPortError>;

    async fn query(
        &self,
        context: ObservabilityCall,
        intent: ObservabilityQueryIntent,
    ) -> Result<ProjectionPage, ObservabilityPortError>;
}

#[async_trait]
pub trait ObservabilityExportPort: Send + Sync {
    async fn export(
        &self,
        context: ObservabilityCall,
        intent: ObservabilityExportIntent,
        sink: BoundedExportSink,
    ) -> Result<ExportPortOutcome, ObservabilityPortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservabilityProviderBuildError {
    Contract(ProviderContractError),
    WrongProviderType,
    WrongImplementation,
    WrongPlacement,
    CapabilityMismatch,
    InvalidLimits,
    InvalidProjection,
    EmptyFactory,
    TooManyFactoryEntries,
    DuplicateProvider,
}

impl From<ProviderContractError> for ObservabilityProviderBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

fn validate_observability_descriptor(
    descriptor: &ProviderDescriptor,
) -> Result<(), ObservabilityProviderBuildError> {
    descriptor.validate()?;
    if descriptor.provider_type() != ProviderType::Observability {
        return Err(ObservabilityProviderBuildError::WrongProviderType);
    }
    if descriptor.implementation_id != implementation_id() {
        return Err(ObservabilityProviderBuildError::WrongImplementation);
    }
    if !matches!(
        descriptor.placement,
        ProviderPlacement::TrustedFirstPartyInProcess { .. }
    ) {
        return Err(ObservabilityProviderBuildError::WrongPlacement);
    }
    if descriptor.capabilities != live_observability_capabilities()? {
        return Err(ObservabilityProviderBuildError::CapabilityMismatch);
    }
    Ok(())
}

#[derive(Clone)]
pub struct LocalObservabilityFactoryEntry {
    provider_id: ProviderId,
    configuration_schema_fingerprint: Fingerprint,
    configured_scope_digest: Fingerprint,
    placement: ProviderPlacement,
    limits: ObservabilityLimits,
    queries: Arc<dyn ObservabilityQueryPort>,
    exports: Arc<dyn ObservabilityExportPort>,
}

pub type FactoryEntry = LocalObservabilityFactoryEntry;

impl fmt::Debug for LocalObservabilityFactoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalObservabilityFactoryEntry")
            .field("provider_id", &"<redacted>")
            .field("placement", &self.placement)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl LocalObservabilityFactoryEntry {
    pub fn new(
        descriptor: &ProviderDescriptor,
        limits: ObservabilityLimits,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        validate_observability_descriptor(descriptor)?;
        Ok(Self {
            provider_id: descriptor.provider_id.clone(),
            configuration_schema_fingerprint: descriptor.configuration_schema_fingerprint.clone(),
            configured_scope_digest: descriptor.configured_scope_digest.clone(),
            placement: descriptor.placement.clone(),
            limits,
            queries,
            exports,
        })
    }

    fn matches(&self, descriptor: &ProviderDescriptor) -> bool {
        self.provider_id == descriptor.provider_id
            && self.configuration_schema_fingerprint == descriptor.configuration_schema_fingerprint
            && self.configured_scope_digest == descriptor.configured_scope_digest
            && self.placement == descriptor.placement
    }
}

#[derive(Clone)]
pub struct LocalObservabilityFactory {
    entries: Arc<BTreeMap<ProviderId, LocalObservabilityFactoryEntry>>,
    clock: Arc<dyn ProviderClock>,
}

pub type Factory = LocalObservabilityFactory;

impl fmt::Debug for LocalObservabilityFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalObservabilityFactory")
            .field("key", &factory_key())
            .field("entry_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl LocalObservabilityFactory {
    pub fn new(
        entries: Vec<LocalObservabilityFactoryEntry>,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        Self::with_clock(entries, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        entries: Vec<LocalObservabilityFactoryEntry>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        if entries.is_empty() {
            return Err(ObservabilityProviderBuildError::EmptyFactory);
        }
        if entries.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(ObservabilityProviderBuildError::TooManyFactoryEntries);
        }
        let mut indexed = BTreeMap::new();
        for entry in entries {
            if indexed.insert(entry.provider_id.clone(), entry).is_some() {
                return Err(ObservabilityProviderBuildError::DuplicateProvider);
            }
        }
        Ok(Self {
            entries: Arc::new(indexed),
            clock,
        })
    }
}

impl ProviderFactory for LocalObservabilityFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Observability
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let entry = self
            .entries
            .get(&descriptor.provider_id)
            .filter(|entry| entry.matches(descriptor))
            .ok_or(FactoryError::Rejected)?;
        LocalObservabilityProvider::with_clock(
            descriptor.clone(),
            entry.limits,
            entry.queries.clone(),
            entry.exports.clone(),
            self.clock.clone(),
        )
        .map(|provider| ProviderInstance::Observability(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}

#[derive(Clone)]
pub struct LocalObservabilityProvider {
    descriptor: ProviderDescriptor,
    limits: ObservabilityLimits,
    queries: Arc<dyn ObservabilityQueryPort>,
    exports: Arc<dyn ObservabilityExportPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for LocalObservabilityProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalObservabilityProvider")
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl LocalObservabilityProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        limits: ObservabilityLimits,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        Self::with_clock(
            descriptor,
            limits,
            queries,
            exports,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        limits: ObservabilityLimits,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        validate_observability_descriptor(&descriptor)?;
        Ok(Self {
            descriptor,
            limits,
            queries,
            exports,
            clock,
        })
    }

    fn expected_peer_role(&self) -> EndpointRole {
        match &self.descriptor.placement {
            ProviderPlacement::TrustedFirstPartyInProcess {
                controller_role, ..
            } => *controller_role,
            ProviderPlacement::ProviderAgent { endpoint_role, .. }
            | ProviderPlacement::UserAgent { endpoint_role, .. } => *endpoint_role,
        }
    }

    fn now_unix_ms(&self) -> u64 {
        self.clock.now_unix_ms().min(MAX_SAFE_JSON_INTEGER)
    }

    fn failure(
        &self,
        operation: &ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        let mut binding = operation.binding();
        binding.provider_id = self.descriptor.provider_id.clone();
        binding.provider_generation = self.descriptor.registry_generation;
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Observability,
            binding,
            correlation_id: operation.correlation_id.clone(),
            occurred_at_unix_ms: self.now_unix_ms(),
            reason,
            remediation,
        }
    }

    fn invalid_request(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn deadline_failure(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::DeadlineExpired,
            RetryClass::SameOperation,
            ProviderHealthReason::HealthTimeout,
            ProviderRemediation::RetryBounded,
        )
    }

    fn ambiguous_mutation(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::AmbiguousMutation,
            RetryClass::AfterObservation,
            ProviderHealthReason::AdoptionAmbiguous,
            ProviderRemediation::InspectProvider,
        )
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        operation: &ProviderOperationContext,
        expected: ProviderMethod,
    ) -> Result<ObservabilityCall, ProviderFailure> {
        if context.cancelled {
            return Err(self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(operation));
        }
        if context.operation != operation
            || context.peer_role != self.expected_peer_role()
            || context.validate().is_err()
            || operation
                .validate(&self.descriptor, self.now_unix_ms())
                .is_err()
            || operation.method != expected
        {
            return Err(self.invalid_request(operation));
        }
        Ok(ObservabilityCall {
            operation: operation.clone(),
            peer_role: context.peer_role,
            monotonic_deadline_remaining_ms: context.monotonic_deadline_remaining_ms,
        })
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> Result<ObservabilityCall, ProviderFailure> {
        let call = self.validate_call(context, &request.context, expected)?;
        if request
            .validate_method(&self.descriptor, self.now_unix_ms(), expected)
            .is_err()
            || matches!(request.target, ProviderTarget::Handle { .. })
        {
            return Err(self.invalid_request(&request.context));
        }
        Ok(call)
    }

    fn validate_unsupported_subscribe(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> Result<(), ProviderFailure> {
        if context.cancelled {
            return Err(self.failure(
                &request.context,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(&request.context));
        }
        let mut validation_descriptor = self.descriptor.clone();
        validation_descriptor.capabilities = ProviderCapabilitySet::new(vec![
            ProviderCapability(ProviderMethod::ObservabilityStatus),
            ProviderCapability(ProviderMethod::ObservabilityQuery),
            ProviderCapability(ProviderMethod::ObservabilitySubscribe),
            ProviderCapability(ProviderMethod::ObservabilityExport),
        ])
        .map_err(|_| self.invalid_request(&request.context))?;
        if context.operation != &request.context
            || context.peer_role != self.expected_peer_role()
            || context.validate().is_err()
            || request
                .validate_method(
                    &validation_descriptor,
                    self.now_unix_ms(),
                    ProviderMethod::ObservabilitySubscribe,
                )
                .is_err()
            || matches!(request.target, ProviderTarget::Handle { .. })
        {
            return Err(self.invalid_request(&request.context));
        }
        Ok(())
    }

    async fn invoke<T, F>(
        &self,
        operation: &ProviderOperationContext,
        deadline_ms: u32,
        dispatch: ObservabilityDispatch,
        future: F,
    ) -> Result<T, ProviderFailure>
    where
        F: Future<Output = Result<T, ObservabilityPortError>> + Send,
    {
        match tokio::time::timeout(Duration::from_millis(u64::from(deadline_ms)), future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.port_failure(operation, error)),
            Err(_) => Err(match dispatch {
                ObservabilityDispatch::Read => self.deadline_failure(operation),
                ObservabilityDispatch::Mutation => self.ambiguous_mutation(operation),
            }),
        }
    }

    fn port_failure(
        &self,
        operation: &ProviderOperationContext,
        error: ObservabilityPortError,
    ) -> ProviderFailure {
        match error {
            ObservabilityPortError::Denied => self.failure(
                operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            ObservabilityPortError::Unavailable => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            ObservabilityPortError::AmbiguousMutation => self.ambiguous_mutation(operation),
            ObservabilityPortError::InvalidProjection => self.failure(
                operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            ObservabilityPortError::Cancelled => self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
        }
    }

    fn values(
        &self,
        operation: &ProviderOperationContext,
    ) -> Result<ProviderValues, ProviderFailure> {
        ProviderValues::new(&self.descriptor, self.now_unix_ms())
            .map_err(|_| self.invalid_request(operation))
    }

    fn bind_query_result(
        &self,
        request: &ProviderOperationRequest,
        projection: BoundedProjection,
    ) -> Result<ObservabilityQueryResult, ProviderFailure> {
        let observation = self
            .values(&request.context)?
            .observation(
                &request.context,
                None,
                ObservedLifecycleState::Ready,
                AdoptionState::NotAttempted,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            )
            .map_err(|_| self.invalid_request(&request.context))?;
        let records = BoundedVec::new(
            projection
                .records
                .into_iter()
                .map(LocalObservationRecord::into_contract)
                .collect(),
        )
        .map_err(|_| {
            self.port_failure(&request.context, ObservabilityPortError::InvalidProjection)
        })?;
        let result = ObservabilityQueryResult {
            observation,
            records,
            next_cursor: projection.next_cursor,
            encoded_bytes_upper_bound: projection.encoded_bytes_upper_bound,
            truncated: projection.truncated,
        };
        result.validate(request).map_err(|_| {
            self.port_failure(&request.context, ObservabilityPortError::InvalidProjection)
        })?;
        Ok(result)
    }

    fn bound_page(
        &self,
        operation: &ProviderOperationContext,
        mut page: ProjectionPage,
        bounds: ProjectionBounds,
        view: Option<ObservabilityView>,
    ) -> Result<BoundedProjection, ProviderFailure> {
        if page
            .records
            .iter()
            .any(|record| view.is_some_and(|view| !record.allowed_for(view)))
        {
            return Err(self.port_failure(operation, ObservabilityPortError::InvalidProjection));
        }
        let capacity = bounds.record_capacity();
        let source_len = page.records.len();
        page.records.truncate(capacity);
        let truncated = page.source_truncated || source_len > page.records.len();
        let encoded_bytes_upper_bound = u32::try_from(page.records.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES);
        if encoded_bytes_upper_bound > bounds.max_bytes {
            return Err(self.port_failure(operation, ObservabilityPortError::InvalidProjection));
        }
        Ok(BoundedProjection {
            binding: {
                let mut binding = operation.binding();
                binding.provider_id = self.descriptor.provider_id.clone();
                binding.provider_generation = self.descriptor.registry_generation;
                binding
            },
            records: page.records,
            next_cursor: if truncated { page.next_cursor } else { None },
            encoded_bytes_upper_bound,
            truncated,
        })
    }

    pub async fn bounded_query(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> Result<BoundedProjection, ProviderFailure> {
        let call = self.validate_request(context, request, ProviderMethod::ObservabilityQuery)?;
        let ProviderOperationInput::ObservabilityQuery {
            view,
            cursor,
            limit,
        } = &request.input
        else {
            return Err(self.invalid_request(&request.context));
        };
        if *limit > self.limits.max_records {
            return Err(self.failure(
                &request.context,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        let bounds = self.limits.query_bounds(*limit);
        let intent = ObservabilityQueryIntent {
            view: *view,
            cursor: cursor.clone(),
            bounds,
        };
        let deadline = call.monotonic_deadline_remaining_ms();
        let page = self
            .invoke(
                &request.context,
                deadline,
                ObservabilityDispatch::Read,
                self.queries.query(call, intent),
            )
            .await?;
        self.bound_page(&request.context, page, bounds, Some(*view))
    }

    pub async fn bounded_export(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> Result<BoundedExport, ProviderFailure> {
        let call = self.validate_request(context, request, ProviderMethod::ObservabilityExport)?;
        let ProviderOperationInput::ObservabilityExport {
            format,
            start_at_unix_ms,
            end_at_unix_ms,
        } = request.input
        else {
            return Err(self.invalid_request(&request.context));
        };
        if end_at_unix_ms.saturating_sub(start_at_unix_ms) > self.limits.max_time_window_ms {
            return Err(self.failure(
                &request.context,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        let bounds = self.limits.export_bounds();
        let intent = ObservabilityExportIntent {
            format,
            start_at_unix_ms,
            end_at_unix_ms,
            bounds,
        };
        let deadline = call.monotonic_deadline_remaining_ms();
        let sink = BoundedExportSink::new(format, bounds, start_at_unix_ms, end_at_unix_ms);
        let outcome = self
            .invoke(
                &request.context,
                deadline,
                ObservabilityDispatch::Mutation,
                self.exports.export(call, intent, sink.clone()),
            )
            .await;
        let snapshot = sink.finish();
        let outcome = outcome?;
        let snapshot = snapshot.map_err(|_| {
            self.port_failure(&request.context, ObservabilityPortError::InvalidProjection)
        })?;
        let mut binding = request.context.binding();
        binding.provider_id = self.descriptor.provider_id.clone();
        binding.provider_generation = self.descriptor.registry_generation;
        Ok(BoundedExport {
            binding,
            state: outcome.state(),
            records: snapshot.records,
            encoded_bytes: snapshot.encoded_bytes,
            truncated: snapshot.truncated,
        })
    }
}

pub fn live_observability_capabilities() -> Result<ProviderCapabilitySet, ProviderContractError> {
    ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::ObservabilityStatus),
        ProviderCapability(ProviderMethod::ObservabilityQuery),
        ProviderCapability(ProviderMethod::ObservabilityExport),
    ])
}

impl Provider for LocalObservabilityProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            let call = self.validate_call(context, context.operation, context.operation.method)?;
            let deadline = call.monotonic_deadline_remaining_ms();
            let status = self
                .invoke(
                    context.operation,
                    deadline,
                    ObservabilityDispatch::Read,
                    self.queries.health(call),
                )
                .await?;
            self.values(context.operation)?
                .health(
                    status.health_state,
                    status.health_reason,
                    status.remediation,
                )
                .map_err(|_| self.invalid_request(context.operation))
        })
    }
}

impl ObservabilityProvider for LocalObservabilityProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn status<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            let call =
                self.validate_request(context, request, ProviderMethod::ObservabilityStatus)?;
            let deadline = call.monotonic_deadline_remaining_ms();
            let status = self
                .invoke(
                    &request.context,
                    deadline,
                    ObservabilityDispatch::Read,
                    self.queries.status(call),
                )
                .await?;
            self.values(&request.context)?
                .observation(
                    &request.context,
                    None,
                    ObservedLifecycleState::Ready,
                    AdoptionState::NotAttempted,
                    ObservationReason::None,
                    status.health_state,
                    status.health_reason,
                    status.remediation,
                )
                .map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn query<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ObservabilityQueryResult> {
        Box::pin(async move {
            let projection = self.bounded_query(context, request).await?;
            self.bind_query_result(request, projection)
        })
    }

    fn subscribe<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move {
            self.validate_unsupported_subscribe(context, request)?;
            Err(self.failure(
                &request.context,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        })
    }

    fn export<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move {
            let export = self.bounded_export(context, request).await?;
            self.values(&request.context)?
                .receipt(&request.context, export.state())
                .map_err(|_| self.invalid_request(&request.context))
        })
    }
}

#[cfg(test)]
mod tests;
