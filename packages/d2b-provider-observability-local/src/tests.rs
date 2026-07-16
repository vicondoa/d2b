use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    BoundedExportSink, BoundedProjection, ClosedMetricLabels, ExportPortOutcome, ExportSinkError,
    ExportSinkStatus, Factory, LocalObservabilityFactoryEntry, LocalObservabilityProvider,
    LocalObservabilityStatus, LocalObservationRecord, MetricLabel, MetricLabelKey,
    OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES, ObservabilityCall, ObservabilityExportIntent,
    ObservabilityExportPort, ObservabilityLimits, ObservabilityPortError, ObservabilityQueryIntent,
    ObservabilityQueryPort, OperationLabel, OutcomeLabel, ProjectionBounds, ProjectionKind,
    ProjectionPage, encode_export_payload, factory_key, implementation_id,
    live_observability_capabilities,
};
use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::{ProviderType, RealmId, WorkloadId},
    v2_provider::{
        ImplementationId, MutationState, ObservabilityCursor, ObservabilityExportFormat,
        ObservabilityProvider, ObservabilityView, ProviderFailureKind, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderOperationInput, ProviderPlacement,
        ProviderRemediation, ProviderTarget, RetryClass,
    },
};
use d2b_provider::{FactoryError, ProviderFactory, ProviderInstance, ProviderRegistryBuilder};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

const NOW: u64 = 1_700_000_000_000;

struct FakePorts {
    health_calls: AtomicUsize,
    status_calls: AtomicUsize,
    query_calls: AtomicUsize,
    export_calls: AtomicUsize,
    slow_query: AtomicBool,
    slow_export: AtomicBool,
    page: Mutex<ProjectionPage>,
    export_outcome: Mutex<ExportPortOutcome>,
    export_records: Mutex<Vec<LocalObservationRecord>>,
    export_statuses: Mutex<Vec<ExportSinkStatus>>,
    export_source_truncated: AtomicBool,
    retained_sink: Mutex<Option<BoundedExportSink>>,
    query_debug: Mutex<Option<String>>,
    query_bounds: Mutex<Option<(u16, u32)>>,
    export_bounds: Mutex<Option<(u16, u32)>>,
}

impl FakePorts {
    fn new(page: ProjectionPage) -> Self {
        Self {
            health_calls: AtomicUsize::new(0),
            status_calls: AtomicUsize::new(0),
            query_calls: AtomicUsize::new(0),
            export_calls: AtomicUsize::new(0),
            slow_query: AtomicBool::new(false),
            slow_export: AtomicBool::new(false),
            page: Mutex::new(page),
            export_outcome: Mutex::new(ExportPortOutcome::new(MutationState::NotApplicable)),
            export_records: Mutex::new(Vec::new()),
            export_statuses: Mutex::new(Vec::new()),
            export_source_truncated: AtomicBool::new(false),
            retained_sink: Mutex::new(None),
            query_debug: Mutex::new(None),
            query_bounds: Mutex::new(None),
            export_bounds: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ObservabilityQueryPort for FakePorts {
    async fn health(
        &self,
        _context: ObservabilityCall,
    ) -> Result<LocalObservabilityStatus, ObservabilityPortError> {
        self.health_calls.fetch_add(1, Ordering::Relaxed);
        Ok(LocalObservabilityStatus::healthy())
    }

    async fn status(
        &self,
        _context: ObservabilityCall,
    ) -> Result<LocalObservabilityStatus, ObservabilityPortError> {
        self.status_calls.fetch_add(1, Ordering::Relaxed);
        Ok(LocalObservabilityStatus::healthy())
    }

    async fn query(
        &self,
        _context: ObservabilityCall,
        intent: ObservabilityQueryIntent,
    ) -> Result<ProjectionPage, ObservabilityPortError> {
        self.query_calls.fetch_add(1, Ordering::Relaxed);
        *self.query_debug.lock().expect("query debug") = Some(format!("{intent:?}"));
        *self.query_bounds.lock().expect("query bounds") =
            Some((intent.bounds.max_records, intent.bounds.max_bytes));
        if self.slow_query.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(self.page.lock().expect("page").clone())
    }
}

#[async_trait]
impl ObservabilityExportPort for FakePorts {
    async fn export(
        &self,
        _context: ObservabilityCall,
        intent: ObservabilityExportIntent,
        sink: BoundedExportSink,
    ) -> Result<ExportPortOutcome, ObservabilityPortError> {
        self.export_calls.fetch_add(1, Ordering::Relaxed);
        *self.export_bounds.lock().expect("export bounds") =
            Some((intent.bounds.max_records, intent.bounds.max_bytes));
        *self.retained_sink.lock().expect("retained sink") = Some(sink.clone());
        for record in self
            .export_records
            .lock()
            .expect("export records")
            .iter()
            .copied()
        {
            let status = sink
                .emit(record)
                .map_err(|_| ObservabilityPortError::InvalidProjection)?;
            self.export_statuses
                .lock()
                .expect("export statuses")
                .push(status);
        }
        if self.export_source_truncated.load(Ordering::Relaxed) {
            sink.mark_source_truncated()
                .map_err(|_| ObservabilityPortError::InvalidProjection)?;
        }
        if self.slow_export.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(*self.export_outcome.lock().expect("export outcome"))
    }
}

fn fixture() -> Fixture {
    let realm_id = RealmId::parse("aaaaaaaaaaaaaaaaaaaa").expect("realm id");
    let workload_id = WorkloadId::parse("ccccccccccccccccccca").expect("workload id");
    let mut descriptor = Fixture::new(ProviderType::Observability, 10)
        .expect("base fixture")
        .descriptor;
    descriptor.implementation_id = implementation_id();
    descriptor.capabilities = live_observability_capabilities().expect("capabilities");
    descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: realm_id.clone(),
        controller_role: EndpointRole::RealmController,
    };
    Fixture::from_descriptor(
        descriptor,
        ProviderTarget::Workload {
            realm_id,
            workload_id,
        },
        NOW,
    )
    .expect("fixture")
}

fn page(records: Vec<LocalObservationRecord>) -> ProjectionPage {
    ProjectionPage::new(records, None, false).expect("projection page")
}

fn record(view: ObservabilityView, value: u64) -> LocalObservationRecord {
    let metric = match view {
        ObservabilityView::Health => MetricLabel::ProviderHealth,
        ObservabilityView::Lifecycle => MetricLabel::LifecycleTransition,
        ObservabilityView::Operations => MetricLabel::OperationTotal,
    };
    LocalObservationRecord::new(
        NOW,
        ProjectionKind::Metrics,
        ClosedMetricLabels::new(
            ProviderType::Device,
            ProviderHealthState::Healthy,
            metric,
            OperationLabel::Query,
            OutcomeLabel::Success,
        ),
        value,
    )
    .expect("record")
}

fn limits(max_records: u16, max_bytes: u32, max_time_window_ms: u64) -> ObservabilityLimits {
    ObservabilityLimits::new(max_records, max_bytes, max_time_window_ms).expect("limits")
}

fn provider(
    fixture: &Fixture,
    limits: ObservabilityLimits,
    ports: Arc<FakePorts>,
) -> LocalObservabilityProvider {
    LocalObservabilityProvider::with_clock(
        fixture.descriptor.clone(),
        limits,
        ports.clone(),
        ports,
        Arc::new(DeterministicClock::new(NOW)),
    )
    .expect("provider")
}

fn protobuf_varint_at(input: &[u8], offset: &mut usize) -> u64 {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    loop {
        let byte = input[*offset];
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
        assert!(shift < 64, "bounded protobuf varint");
    }
}

fn protobuf_fields(input: &[u8]) -> Vec<(u32, u8, &[u8])> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < input.len() {
        let key = protobuf_varint_at(input, &mut offset);
        let field = u32::try_from(key >> 3).expect("field number");
        let wire_type = u8::try_from(key & 0x07).expect("wire type");
        let start;
        let end;
        match wire_type {
            0 => {
                start = offset;
                let _ = protobuf_varint_at(input, &mut offset);
                end = offset;
            }
            1 => {
                start = offset;
                offset += 8;
                end = offset;
            }
            2 => {
                let len =
                    usize::try_from(protobuf_varint_at(input, &mut offset)).expect("field length");
                start = offset;
                offset += len;
                end = offset;
            }
            5 => {
                start = offset;
                offset += 4;
                end = offset;
            }
            other => panic!("unsupported protobuf wire type {other}"),
        }
        assert!(end <= input.len(), "protobuf field remains bounded");
        fields.push((field, wire_type, &input[start..end]));
    }
    fields
}

fn protobuf_message_field(input: &[u8], field: u32) -> &[u8] {
    protobuf_fields(input)
        .into_iter()
        .find_map(|(candidate, wire_type, value)| {
            (candidate == field && wire_type == 2).then_some(value)
        })
        .expect("protobuf message field")
}

#[test]
fn provider_owned_export_sink_formats_exact_json_lines_and_otlp_protobuf() {
    let observation = record(ObservabilityView::Operations, 7);
    let json = encode_export_payload(ObservabilityExportFormat::JsonLines, &[observation])
        .expect("JSON Lines");
    assert_eq!(json.last(), Some(&b'\n'));
    let value: serde_json::Value =
        serde_json::from_slice(&json[..json.len() - 1]).expect("JSON record");
    assert_eq!(value["observedAtUnixMs"], NOW);
    assert_eq!(value["projection"], "metrics");
    assert_eq!(value["labels"]["providerType"], "device");
    assert_eq!(value["labels"]["metric"], "operation-total");
    assert_eq!(value["value"], 7);

    let otlp = encode_export_payload(ObservabilityExportFormat::OtlpProtobuf, &[observation])
        .expect("OTLP protobuf");
    let resource_metrics = protobuf_message_field(&otlp, 1);
    let scope_metrics = protobuf_message_field(resource_metrics, 2);
    let scope = protobuf_message_field(scope_metrics, 1);
    assert_eq!(
        protobuf_message_field(scope, 1),
        b"d2b.provider.observability.local"
    );
    let metric = protobuf_message_field(scope_metrics, 2);
    assert_eq!(protobuf_message_field(metric, 1), b"d2b.operation.total");
    let gauge = protobuf_message_field(metric, 5);
    let point = protobuf_message_field(gauge, 1);
    let point_fields = protobuf_fields(point);
    assert!(
        point_fields
            .iter()
            .any(|(field, wire_type, _)| *field == 3 && *wire_type == 1)
    );
    assert!(
        point_fields
            .iter()
            .any(|(field, wire_type, _)| *field == 6 && *wire_type == 1)
    );
    assert_eq!(
        point_fields
            .iter()
            .filter(|(field, wire_type, _)| *field == 7 && *wire_type == 2)
            .count(),
        7
    );
}

#[test]
fn provider_owned_export_sink_enforces_exact_streaming_byte_bound() {
    let first = record(ObservabilityView::Operations, 1);
    let exact_one = encode_export_payload(ObservabilityExportFormat::JsonLines, &[first])
        .expect("one JSON record");
    let sink = BoundedExportSink::new(
        ObservabilityExportFormat::JsonLines,
        ProjectionBounds {
            max_records: 2,
            max_bytes: u32::try_from(exact_one.len()).expect("bounded payload"),
        },
        NOW,
        NOW,
    );
    assert_eq!(sink.emit(first), Ok(ExportSinkStatus::Emitted));
    assert_eq!(
        sink.emit(record(ObservabilityView::Operations, 2)),
        Ok(ExportSinkStatus::Truncated)
    );
    assert_eq!(sink.record_count(), Ok(1));
    assert_eq!(sink.encoded_payload(), Ok(exact_one));
    assert_eq!(sink.truncated(), Ok(true));
}

fn factory() -> Factory {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    let entry = LocalObservabilityFactoryEntry::new(
        &fixture.descriptor,
        default_limits(),
        ports.clone(),
        ports,
    )
    .expect("factory entry");
    Factory::with_clock(vec![entry], Arc::new(DeterministicClock::new(NOW))).expect("factory")
}

#[test]
fn factory_key_constructs_only_the_exact_observability_implementation() {
    let fixture = fixture();
    assert_eq!(factory_key().provider_type, ProviderType::Observability);
    assert_eq!(factory_key().implementation_id, implementation_id());
    let instance = factory()
        .construct(&fixture.descriptor)
        .expect("observability instance");
    assert!(matches!(instance, ProviderInstance::Observability(_)));
    assert_eq!(instance.descriptor(), fixture.descriptor);
    let mut builder = ProviderRegistryBuilder::new(
        fixture.descriptor.registry_generation,
        fixture.descriptor.configured_scope_digest.clone(),
        NOW,
    );
    builder
        .register_factory(factory_key(), Arc::new(factory()))
        .expect("register factory")
        .register_instance(fixture.descriptor.clone())
        .expect("register instance");
    assert_eq!(
        builder
            .finish()
            .expect("registry")
            .snapshot()
            .providers
            .as_slice(),
        std::slice::from_ref(&fixture.descriptor)
    );

    let wrong_type = Fixture::new(ProviderType::Audio, 13).expect("wrong-type fixture");
    assert!(matches!(
        factory().construct(&wrong_type.descriptor),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_implementation = fixture.descriptor.clone();
    wrong_implementation.implementation_id =
        ImplementationId::parse("other-observability").expect("implementation");
    assert!(matches!(
        factory().construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));

    let alternate = Fixture::new(ProviderType::Observability, 46)
        .expect("alternate fixture")
        .descriptor;
    let mut wrong_schema = fixture.descriptor.clone();
    wrong_schema.configuration_schema_fingerprint =
        alternate.configuration_schema_fingerprint.clone();
    assert!(matches!(
        factory().construct(&wrong_schema),
        Err(FactoryError::Rejected)
    ));
    let mut wrong_scope = fixture.descriptor.clone();
    wrong_scope.configured_scope_digest = alternate.configured_scope_digest.clone();
    assert!(matches!(
        factory().construct(&wrong_scope),
        Err(FactoryError::Rejected)
    ));
    let mut wrong_placement = fixture.descriptor.clone();
    wrong_placement.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: RealmId::parse("bbbbbbbbbbbbbbbbbbba").expect("alternate realm"),
        controller_role: EndpointRole::RealmController,
    };
    assert!(matches!(
        factory().construct(&wrong_placement),
        Err(FactoryError::Rejected)
    ));
    let mut wrong_provider = fixture.descriptor.clone();
    wrong_provider.provider_id = alternate.provider_id;
    assert!(matches!(
        factory().construct(&wrong_provider),
        Err(FactoryError::Rejected)
    ));

    let ports = Arc::new(FakePorts::new(page(vec![])));
    let entry = LocalObservabilityFactoryEntry::new(
        &fixture.descriptor,
        default_limits(),
        ports.clone(),
        ports,
    )
    .expect("entry");
    assert!(Factory::new(vec![entry.clone(), entry]).is_err());
}

fn default_limits() -> ObservabilityLimits {
    limits(
        32,
        32 * OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES,
        60_000,
    )
}

fn query_request(
    fixture: &Fixture,
    view: ObservabilityView,
    cursor: Option<ObservabilityCursor>,
    limit: u16,
) -> d2b_contracts::v2_provider::ProviderOperationRequest {
    fixture
        .request_with_input(
            ProviderMethod::ObservabilityQuery,
            ProviderOperationInput::ObservabilityQuery {
                view,
                cursor,
                limit,
            },
        )
        .expect("query request")
}

fn assert_bounded(projection: &BoundedProjection, records: usize, bytes: u32) {
    assert_eq!(projection.records().len(), records);
    assert_eq!(projection.encoded_bytes_upper_bound(), bytes);
    assert!(projection.truncated());
}

#[tokio::test]
async fn query_forwards_hard_bounds_and_truncates_by_byte_budget() {
    let fixture = fixture();
    let two_records = 2 * OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES;
    let records = (0..4)
        .map(|value| record(ObservabilityView::Lifecycle, value))
        .collect();
    let ports = Arc::new(FakePorts::new(page(records)));
    let provider = provider(&fixture, limits(4, two_records, 60_000), ports.clone());
    let request = query_request(&fixture, ObservabilityView::Lifecycle, None, 4);

    let projection = provider
        .bounded_query(&fixture.call_context(&request.context), &request)
        .await
        .expect("bounded query");

    assert_bounded(&projection, 2, two_records);
    assert!(
        projection
            .records()
            .iter()
            .all(|record| record.labels().provider_type() == ProviderType::Device)
    );
    assert_eq!(
        projection.binding().operation_id,
        request.context.operation_id
    );
    assert_eq!(
        projection.binding().idempotency_key,
        request.context.idempotency_key
    );
    assert_eq!(
        *ports.query_bounds.lock().expect("query bounds"),
        Some((4, two_records))
    );
    assert_eq!(ports.query_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn export_is_single_call_bounded_and_reports_truncation() {
    let fixture = fixture();
    let two_records = 2 * OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES;
    let ports = Arc::new(FakePorts::new(page(vec![])));
    *ports.export_outcome.lock().expect("export outcome") =
        ExportPortOutcome::new(MutationState::Applied);
    *ports.export_records.lock().expect("export records") = vec![
        record(ObservabilityView::Lifecycle, 1),
        record(ObservabilityView::Lifecycle, 2),
    ];
    ports.export_source_truncated.store(true, Ordering::Relaxed);
    let provider = provider(&fixture, limits(2, two_records, 60_000), ports.clone());
    let request = fixture
        .request_with_input(
            ProviderMethod::ObservabilityExport,
            ProviderOperationInput::ObservabilityExport {
                format: ObservabilityExportFormat::JsonLines,
                start_at_unix_ms: NOW - 60_000,
                end_at_unix_ms: NOW,
            },
        )
        .expect("export request");

    let export = provider
        .bounded_export(&fixture.call_context(&request.context), &request)
        .await
        .expect("bounded export");

    assert_eq!(export.state(), MutationState::Applied);
    assert_eq!(export.record_count(), 2);
    assert!(export.encoded_bytes() > 0);
    assert!(export.encoded_bytes() <= two_records);
    assert!(export.truncated());
    assert_eq!(
        *ports.export_statuses.lock().expect("export statuses"),
        [ExportSinkStatus::Emitted, ExportSinkStatus::Emitted,]
    );
    assert!(
        export
            .records()
            .iter()
            .all(|record| record.labels().provider_type() == ProviderType::Device)
    );
    assert_eq!(export.binding().operation_id, request.context.operation_id);
    assert_eq!(
        *ports.export_bounds.lock().expect("export bounds"),
        Some((2, two_records))
    );
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        ports
            .retained_sink
            .lock()
            .expect("retained sink")
            .as_ref()
            .expect("sink")
            .emit(record(ObservabilityView::Lifecycle, 3)),
        Err(ExportSinkError::Closed)
    );
}

#[tokio::test]
async fn export_sink_truncates_during_untrusted_port_emission() {
    let fixture = fixture();
    let two_records = 2 * OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES;
    let ports = Arc::new(FakePorts::new(page(vec![])));
    *ports.export_outcome.lock().expect("export outcome") =
        ExportPortOutcome::new(MutationState::Applied);
    *ports.export_records.lock().expect("export records") = vec![
        record(ObservabilityView::Lifecycle, 1),
        record(ObservabilityView::Lifecycle, 2),
        record(ObservabilityView::Lifecycle, 3),
    ];
    let provider = provider(&fixture, limits(2, two_records, 60_000), ports.clone());
    let request = fixture
        .request(ProviderMethod::ObservabilityExport)
        .expect("export request");

    let export = provider
        .bounded_export(&fixture.call_context(&request.context), &request)
        .await
        .expect("bounded export");

    assert_eq!(export.record_count(), 2);
    assert!(export.encoded_bytes() > 0);
    assert!(export.encoded_bytes() <= two_records);
    assert!(export.truncated());
    assert_eq!(
        *ports.export_statuses.lock().expect("export statuses"),
        [
            ExportSinkStatus::Emitted,
            ExportSinkStatus::Emitted,
            ExportSinkStatus::Truncated,
        ]
    );
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn export_sink_rejects_records_outside_the_authorized_window() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    *ports.export_records.lock().expect("export records") = vec![
        LocalObservationRecord::new(
            NOW - 60_001,
            ProjectionKind::Metrics,
            ClosedMetricLabels::new(
                ProviderType::Device,
                ProviderHealthState::Healthy,
                MetricLabel::OperationTotal,
                OperationLabel::Export,
                OutcomeLabel::Success,
            ),
            1,
        )
        .expect("record"),
    ];
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = fixture
        .request(ProviderMethod::ObservabilityExport)
        .expect("export request");

    let failure = provider
        .bounded_export(&fixture.call_context(&request.context), &request)
        .await
        .expect_err("outside window");

    assert_eq!(failure.kind, ProviderFailureKind::InvariantViolation);
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn oversized_limits_wrong_inputs_and_windows_make_no_port_calls() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    let provider = provider(&fixture, default_limits(), ports.clone());

    let oversized = query_request(&fixture, ObservabilityView::Lifecycle, None, 33);
    let failure = provider
        .bounded_query(&fixture.call_context(&oversized.context), &oversized)
        .await
        .expect_err("oversized query");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);

    let wrong = fixture
        .request_with_input(
            ProviderMethod::ObservabilityQuery,
            ProviderOperationInput::NoInput,
        )
        .expect("wrong request");
    let failure = provider
        .bounded_query(&fixture.call_context(&wrong.context), &wrong)
        .await
        .expect_err("wrong input");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);

    let window = fixture
        .request_with_input(
            ProviderMethod::ObservabilityExport,
            ProviderOperationInput::ObservabilityExport {
                format: ObservabilityExportFormat::JsonLines,
                start_at_unix_ms: NOW - 60_001,
                end_at_unix_ms: NOW,
            },
        )
        .expect("window request");
    let failure = provider
        .bounded_export(&fixture.call_context(&window.context), &window)
        .await
        .expect_err("oversized window");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);

    assert_eq!(ports.query_calls.load(Ordering::Relaxed), 0);
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn projection_rejects_wrong_view_and_has_only_closed_cardinality_keys() {
    assert_eq!(
        MetricLabelKey::ALL,
        [
            MetricLabelKey::Locality,
            MetricLabelKey::ProviderType,
            MetricLabelKey::HealthState,
            MetricLabelKey::Metric,
            MetricLabelKey::Operation,
            MetricLabelKey::Outcome,
        ]
    );
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![record(
        ObservabilityView::Health,
        1,
    )])));
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = query_request(&fixture, ObservabilityView::Lifecycle, None, 1);

    let failure = provider
        .bounded_query(&fixture.call_context(&request.context), &request)
        .await
        .expect_err("wrong projection");

    assert_eq!(failure.kind, ProviderFailureKind::InvariantViolation);
    assert_eq!(ports.query_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn query_result_preserves_every_closed_provider_type() {
    let fixture = fixture();
    let records = ProviderType::ALL
        .into_iter()
        .enumerate()
        .map(|(index, provider_type)| {
            LocalObservationRecord::new(
                NOW - 100 + u64::try_from(index).expect("index"),
                ProjectionKind::Metrics,
                ClosedMetricLabels::new(
                    provider_type,
                    ProviderHealthState::Healthy,
                    MetricLabel::ProviderHealth,
                    OperationLabel::Health,
                    OutcomeLabel::Success,
                ),
                1,
            )
            .expect("record")
        })
        .collect();
    let ports = Arc::new(FakePorts::new(page(records)));
    let provider = provider(&fixture, default_limits(), ports);
    let request = query_request(
        &fixture,
        ObservabilityView::Health,
        None,
        u16::try_from(ProviderType::ALL.len()).expect("provider type count"),
    );

    let result = provider
        .query(&fixture.call_context(&request.context), &request)
        .await
        .expect("query result");

    assert_eq!(
        result
            .records
            .iter()
            .map(|record| record.labels.provider_type)
            .collect::<Vec<_>>(),
        ProviderType::ALL
    );
    assert_eq!(
        result.observation.provider_id,
        fixture.descriptor.provider_id
    );
    assert!(!result.truncated);
    assert!(result.next_cursor.is_none());
}

#[tokio::test]
async fn opaque_cursor_and_projection_debug_are_canary_redacted() {
    let fixture = fixture();
    let canary = "supersecretcanary";
    let cursor = ObservabilityCursor::parse(canary).expect("cursor");
    let response_cursor = ObservabilityCursor::parse(canary).expect("response cursor");
    let response = ProjectionPage::new(
        vec![record(ObservabilityView::Lifecycle, 1)],
        Some(response_cursor),
        true,
    )
    .expect("page");
    assert!(!format!("{response:?}").contains(canary));
    let ports = Arc::new(FakePorts::new(response));
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = query_request(&fixture, ObservabilityView::Lifecycle, Some(cursor), 1);
    assert!(!format!("{:?}", request.input).contains(canary));

    let result = provider
        .query(&fixture.call_context(&request.context), &request)
        .await
        .expect("query");

    assert!(!format!("{result:?}").contains(canary));
    assert!(
        !ports
            .query_debug
            .lock()
            .expect("query debug")
            .as_ref()
            .expect("debug value")
            .contains(canary)
    );
    assert_eq!(
        result.next_cursor.as_ref().map(ObservabilityCursor::as_str),
        Some(canary)
    );
    assert!(result.truncated);
    assert_eq!(result.records.len(), 1);
}

#[tokio::test]
async fn cancellation_zero_deadline_and_unsupported_subscribe_call_no_ports() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = query_request(&fixture, ObservabilityView::Lifecycle, None, 1);

    let mut cancelled = fixture.call_context(&request.context);
    cancelled.cancelled = true;
    let failure = provider
        .bounded_query(&cancelled, &request)
        .await
        .expect_err("cancelled");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);

    let mut expired = fixture.call_context(&request.context);
    expired.monotonic_deadline_remaining_ms = 0;
    let failure = provider
        .bounded_query(&expired, &request)
        .await
        .expect_err("expired");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);

    let subscribe = fixture
        .request(ProviderMethod::ObservabilitySubscribe)
        .expect("subscribe request");
    let failure = provider
        .subscribe(&fixture.call_context(&subscribe.context), &subscribe)
        .await
        .expect_err("unsupported subscribe");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);
    assert_eq!(ports.query_calls.load(Ordering::Relaxed), 0);
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn query_deadline_cancels_the_injected_port_future() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    ports.slow_query.store(true, Ordering::Relaxed);
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = query_request(&fixture, ObservabilityView::Lifecycle, None, 1);
    let mut context = fixture.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 1;

    let failure = provider
        .bounded_query(&context, &request)
        .await
        .expect_err("deadline");

    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(ports.query_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn post_dispatch_export_timeout_is_ambiguous_and_closes_sink() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    ports.slow_export.store(true, Ordering::Relaxed);
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = fixture
        .request(ProviderMethod::ObservabilityExport)
        .expect("export request");
    let mut context = fixture.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 1;

    let failure = provider
        .bounded_export(&context, &request)
        .await
        .expect_err("ambiguous export");

    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        ports
            .retained_sink
            .lock()
            .expect("retained sink")
            .as_ref()
            .expect("sink")
            .emit(record(ObservabilityView::Operations, 1)),
        Err(ExportSinkError::Closed)
    );
}

#[test]
fn limits_and_records_fail_closed() {
    assert!(ObservabilityLimits::new(0, OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES, 1).is_err());
    assert!(
        ObservabilityLimits::new(1, OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES - 1, 1).is_err()
    );
    assert!(ObservabilityLimits::new(1, OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES, 0).is_err());
    assert!(
        LocalObservationRecord::new(
            d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER + 1,
            ProjectionKind::AuditSummary,
            ClosedMetricLabels::new(
                ProviderType::Observability,
                ProviderHealthState::Degraded,
                MetricLabel::OperationTotal,
                OperationLabel::Export,
                OutcomeLabel::Unavailable,
            ),
            1,
        )
        .is_err()
    );
    assert!(
        ProjectionPage::new(
            vec![
                record(ObservabilityView::Lifecycle, 2),
                record(ObservabilityView::Lifecycle, 1),
            ],
            None,
            false,
        )
        .is_err()
    );
    assert!(
        ProjectionPage::new(
            vec![],
            Some(ObservabilityCursor::parse("cursor").expect("cursor")),
            false,
        )
        .is_err()
    );
}

#[tokio::test]
async fn status_preserves_closed_health_and_canonical_binding() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    let provider = provider(&fixture, default_limits(), ports.clone());
    let request = fixture
        .request(ProviderMethod::ObservabilityStatus)
        .expect("status request");

    let observation = provider
        .status(&fixture.call_context(&request.context), &request)
        .await
        .expect("status");

    assert_eq!(observation.health.state, ProviderHealthState::Healthy);
    assert_eq!(observation.health.reason, ProviderHealthReason::None);
    assert_eq!(observation.health.remediation, ProviderRemediation::None);
    assert_eq!(observation.provider_id, fixture.descriptor.provider_id);
    assert_eq!(ports.status_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn canonical_observability_conformance_uses_exact_live_capabilities() {
    let fixture = fixture();
    let ports = Arc::new(FakePorts::new(page(vec![])));
    let provider = Arc::new(provider(&fixture, default_limits(), ports));

    check_provider_conformance(&ProviderInstance::Observability(provider), &fixture)
        .await
        .expect("conformance");
}
