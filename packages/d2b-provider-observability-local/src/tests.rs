use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    BoundedProjection, ExportPortOutcome, Factory, LocalObservabilityProvider,
    LocalObservabilityStatus, LocalObservationRecord, MetricLabel, MetricLabelKey,
    OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES, ObservabilityCall, ObservabilityExportIntent,
    ObservabilityExportPort, ObservabilityLimits, ObservabilityPortError, ObservabilityQueryIntent,
    ObservabilityQueryPort, OperationLabel, OutcomeLabel, ProjectionKind, ProjectionPage,
    factory_key, implementation_id, live_observability_capabilities,
};
use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::{ProviderType, RealmId, WorkloadId},
    v2_provider::{
        ImplementationId, MutationState, ObservabilityCursor, ObservabilityExportFormat,
        ObservabilityProvider, ObservabilityView, ProviderFailureKind, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderOperationInput, ProviderPlacement,
        ProviderRemediation, ProviderTarget,
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
    page: Mutex<ProjectionPage>,
    export_outcome: Mutex<ExportPortOutcome>,
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
            page: Mutex::new(page),
            export_outcome: Mutex::new(
                ExportPortOutcome::new(MutationState::NotApplicable, 0, 0, false)
                    .expect("default export outcome"),
            ),
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
    ) -> Result<ExportPortOutcome, ObservabilityPortError> {
        self.export_calls.fetch_add(1, Ordering::Relaxed);
        *self.export_bounds.lock().expect("export bounds") =
            Some((intent.bounds.max_records, intent.bounds.max_bytes));
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
        ProviderHealthState::Healthy,
        metric,
        OperationLabel::Query,
        OutcomeLabel::Success,
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

fn factory() -> Factory {
    let ports = Arc::new(FakePorts::new(page(vec![])));
    Factory::with_clock(
        default_limits(),
        ports.clone(),
        ports,
        Arc::new(DeterministicClock::new(NOW)),
    )
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
        ExportPortOutcome::new(MutationState::Applied, 2, two_records, true)
            .expect("export outcome");
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
    assert_eq!(export.encoded_bytes(), two_records);
    assert!(export.truncated());
    assert_eq!(export.binding().operation_id, request.context.operation_id);
    assert_eq!(
        *ports.export_bounds.lock().expect("export bounds"),
        Some((2, two_records))
    );
    assert_eq!(ports.export_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn export_rejects_port_results_beyond_authorized_bounds() {
    let fixture = fixture();
    let two_records = 2 * OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES;
    let ports = Arc::new(FakePorts::new(page(vec![])));
    *ports.export_outcome.lock().expect("export outcome") =
        ExportPortOutcome::new(MutationState::Applied, 3, two_records, true)
            .expect("globally bounded outcome");
    let provider = provider(&fixture, limits(2, two_records, 60_000), ports.clone());
    let request = fixture
        .request(ProviderMethod::ObservabilityExport)
        .expect("export request");

    let failure = provider
        .bounded_export(&fixture.call_context(&request.context), &request)
        .await
        .expect_err("over-reported export");

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

    let projection = provider
        .bounded_query(&fixture.call_context(&request.context), &request)
        .await
        .expect("query");

    assert!(!format!("{projection:?}").contains(canary));
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
        projection.next_cursor().map(ObservabilityCursor::as_str),
        Some(canary)
    );
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
            ProviderHealthState::Degraded,
            MetricLabel::OperationTotal,
            OperationLabel::Export,
            OutcomeLabel::Unavailable,
            1,
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
