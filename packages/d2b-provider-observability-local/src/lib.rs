//! Bounded, read-only local observability provider.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{fmt, future::Future, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::ProviderType,
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, ImplementationId,
        MAX_OBSERVABILITY_EXPORT_RANGE_MS, MAX_OBSERVABILITY_QUERY_LIMIT, MAX_SAFE_JSON_INTEGER,
        MutationReceipt, MutationState, ObservabilityCursor, ObservabilityExportFormat,
        ObservabilityProvider, ObservabilityView, ObservationReason, ObservedLifecycleState,
        OperationBinding, Provider, ProviderCallContext, ProviderCapability, ProviderCapabilitySet,
        ProviderContractError, ProviderDescriptor, ProviderFactoryKey, ProviderFailure,
        ProviderFailureKind, ProviderFuture, ProviderHandle, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationContext,
        ProviderOperationInput, ProviderOperationRequest, ProviderPlacement, ProviderRemediation,
        ProviderTarget, RetryClass,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;

pub const MAX_LOCAL_OBSERVABILITY_BYTES: u32 = 1024 * 1024;
pub const OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES: u32 = 512;
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
pub enum MetricLabel {
    ProviderHealth,
    LifecycleTransition,
    OperationTotal,
    OperationDuration,
    QueueDepth,
    ExportTruncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationLabel {
    Health,
    Plan,
    Ensure,
    Start,
    Stop,
    Attach,
    Detach,
    Adopt,
    Inspect,
    SetState,
    Query,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutcomeLabel {
    Success,
    AlreadyApplied,
    Denied,
    Cancelled,
    DeadlineExpired,
    Unavailable,
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionKind {
    Metrics,
    TraceSummary,
    AuditSummary,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalityLabel {
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosedMetricLabels {
    locality: LocalityLabel,
    provider_type: ProviderType,
    health_state: ProviderHealthState,
    metric: MetricLabel,
    operation: OperationLabel,
    outcome: OutcomeLabel,
}

impl ClosedMetricLabels {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        health_state: ProviderHealthState,
        metric: MetricLabel,
        operation: OperationLabel,
        outcome: OutcomeLabel,
        value: u64,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        if observed_at_unix_ms > MAX_SAFE_JSON_INTEGER || value > MAX_SAFE_JSON_INTEGER {
            return Err(ObservabilityProviderBuildError::InvalidProjection);
        }
        Ok(Self {
            observed_at_unix_ms,
            projection,
            labels: ClosedMetricLabels {
                locality: LocalityLabel::Local,
                provider_type: ProviderType::Observability,
                health_state,
                metric,
                operation,
                outcome,
            },
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
                    || record.labels().provider_type() != ProviderType::Observability
            })
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
pub struct ExportPortOutcome {
    state: MutationState,
    record_count: u16,
    encoded_bytes: u32,
    truncated: bool,
}

impl ExportPortOutcome {
    pub fn new(
        state: MutationState,
        record_count: u16,
        encoded_bytes: u32,
        truncated: bool,
    ) -> Result<Self, ObservabilityProviderBuildError> {
        if record_count > MAX_OBSERVABILITY_QUERY_LIMIT
            || encoded_bytes > MAX_LOCAL_OBSERVABILITY_BYTES
        {
            return Err(ObservabilityProviderBuildError::InvalidProjection);
        }
        Ok(Self {
            state,
            record_count,
            encoded_bytes,
            truncated,
        })
    }

    pub const fn state(self) -> MutationState {
        self.state
    }

    pub const fn record_count(self) -> u16 {
        self.record_count
    }

    pub const fn encoded_bytes(self) -> u32 {
        self.encoded_bytes
    }

    pub const fn truncated(self) -> bool {
        self.truncated
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BoundedExport {
    binding: OperationBinding,
    outcome: ExportPortOutcome,
}

impl fmt::Debug for BoundedExport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedExport")
            .field("binding", &self.binding)
            .field("state", &self.outcome.state)
            .field("record_count", &self.outcome.record_count)
            .field("encoded_bytes", &self.outcome.encoded_bytes)
            .field("truncated", &self.outcome.truncated)
            .finish()
    }
}

impl BoundedExport {
    pub fn binding(&self) -> &OperationBinding {
        &self.binding
    }

    pub const fn state(&self) -> MutationState {
        self.outcome.state()
    }

    pub const fn record_count(&self) -> u16 {
        self.outcome.record_count()
    }

    pub const fn encoded_bytes(&self) -> u32 {
        self.outcome.encoded_bytes()
    }

    pub const fn truncated(&self) -> bool {
        self.outcome.truncated()
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
    InvalidProjection,
    Cancelled,
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
}

impl From<ProviderContractError> for ObservabilityProviderBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

#[derive(Clone)]
pub struct LocalObservabilityFactory {
    limits: ObservabilityLimits,
    queries: Arc<dyn ObservabilityQueryPort>,
    exports: Arc<dyn ObservabilityExportPort>,
    clock: Arc<dyn ProviderClock>,
}

pub type Factory = LocalObservabilityFactory;

impl fmt::Debug for LocalObservabilityFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalObservabilityFactory")
            .field("key", &factory_key())
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl LocalObservabilityFactory {
    pub fn new(
        limits: ObservabilityLimits,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
    ) -> Self {
        Self::with_clock(limits, queries, exports, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        limits: ObservabilityLimits,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Self {
        Self {
            limits,
            queries,
            exports,
            clock,
        }
    }
}

impl ProviderFactory for LocalObservabilityFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Observability
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        LocalObservabilityProvider::with_clock(
            descriptor.clone(),
            self.limits,
            self.queries.clone(),
            self.exports.clone(),
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
            ProviderPlacement::ProviderAgent { endpoint_role, .. } => *endpoint_role,
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
        future: F,
    ) -> Result<T, ProviderFailure>
    where
        F: Future<Output = Result<T, ObservabilityPortError>> + Send,
    {
        match tokio::time::timeout(Duration::from_millis(u64::from(deadline_ms)), future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.port_failure(operation, error)),
            Err(_) => Err(self.deadline_failure(operation)),
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
            .invoke(&request.context, deadline, self.queries.query(call, intent))
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
        let outcome = self
            .invoke(
                &request.context,
                deadline,
                self.exports.export(call, intent),
            )
            .await?;
        if outcome.record_count() > bounds.max_records || outcome.encoded_bytes() > bounds.max_bytes
        {
            return Err(
                self.port_failure(&request.context, ObservabilityPortError::InvalidProjection)
            );
        }
        let mut binding = request.context.binding();
        binding.provider_id = self.descriptor.provider_id.clone();
        binding.provider_generation = self.descriptor.registry_generation;
        Ok(BoundedExport { binding, outcome })
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
                .invoke(context.operation, deadline, self.queries.health(call))
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
                .invoke(&request.context, deadline, self.queries.status(call))
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
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            self.bounded_query(context, request).await?;
            self.values(&request.context)?
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
                .map_err(|_| self.invalid_request(&request.context))
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
