//! End-to-end harness tests over a fake-port Provider.
//!
//! Every fence here is the provider's own: the declaration fence, the
//! child-creation fence, the operation envelope, and the audit ring. Only
//! the effect ports are scripted.

use std::sync::{LazyLock, Mutex};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_contracts_resource::v3::resource_schema::CanonicalJsonObject;
use d2b_provider_toolkit::{
    AttachError, Cardinality, ChildCreation, ChildCreationFailure, ChildCustody, CreationRefusal,
    DeterministicClock, DrainDeadline, DrainError, DriverDescriptor, EnrolledRoute,
    EnrollmentRequest, FaultPlan, GuestAgent, GuestEnrollment, GuestError, HarnessDeclarations,
    IsolationPosture, OperationCtx, OperationDef, OperationFailure, OperationHandler,
    OperationResult, PlaneCall, ProviderAgentAuditOutcome, ProviderBase, ProviderDeclaration,
    ReconcileCause, ReconcileCtx, ReconcileOutcome, ReconcileTarget, RowPhase, StartupStep,
    StartupStepError, StartupStepExecutor, TestHarness, ValidatedPayload, WellKnownType,
    ZonePlaneHandle, run_guest, run_guest_with,
};

const PROVIDER_REF: &str = "harness";
const CHILD_PROVIDER: &str = "system-minijail";
const CHILD_SPEC: &[u8] = br#"{"providerRef":"Provider/system-minijail"}"#;

static DECLARATION: ProviderDeclaration = ProviderDeclaration {
    provider_ref: PROVIDER_REF,
    self_bindings: &[],
    required: false,
    cardinality: Cardinality::AtMostOne,
    isolation_posture: IsolationPosture::Standard,
    plane_adapters: &[],
    principals: &[],
    storage_roots: &[],
};

/// The creation a reconcile body realizes.
static DECLARED_CREATIONS: &[ChildCreation] = &[ChildCreation {
    child: WellKnownType::PROCESS,
    provider_ref: CHILD_PROVIDER,
    custody: ChildCustody::DriverOwned,
    order: 1,
}];

/// A creation no driver declared.
static UNDECLARED_CREATION: ChildCreation = ChildCreation {
    child: WellKnownType::VOLUME,
    provider_ref: "volume-local",
    custody: ChildCustody::DriverOwned,
    order: 2,
};

/// A creation declared controller-owned.
static CONTROLLER_OWNED_CREATION: ChildCreation = ChildCreation {
    child: WellKnownType::GUEST,
    provider_ref: "runtime-cloud-hypervisor",
    custody: ChildCustody::ControllerOwned,
    order: 3,
};

static DECLARATION_ROWS: &[(WellKnownType, &[ChildCreation])] = &[
    (WellKnownType::VOLUME, DECLARED_CREATIONS),
    (WellKnownType::GUEST, &[CONTROLLER_OWNED_CREATION]),
];

/// Two declared steps whose derived order is not their declaration order:
/// the listener publishes only after a predecessor commits its input.
static STARTUP_STEPS: &[StartupStep] = &[
    StartupStep {
        id: "publish-listener",
        inputs: &["binding-committed"],
        outputs: &["listener-live"],
    },
    StartupStep {
        id: "commit-binding",
        inputs: &[],
        outputs: &["binding-committed"],
    },
];

static STARTUP_ROWS: &[(WellKnownType, &[StartupStep])] = &[(WellKnownType::VOLUME, STARTUP_STEPS)];

/// The declared operation rows, with their handlers.
static OPERATIONS: LazyLock<[OperationDef; 1]> = LazyLock::new(|| {
    [OperationDef {
        operation_ref: operation_ref(),
        handler: &MintHandler,
    }]
});

/// The invocation identifiers the handler saw, in order.
static SEEN_INVOCATIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct MintHandler;

#[async_trait]
impl OperationHandler for MintHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        if let Ok(mut seen) = SEEN_INVOCATIONS.lock() {
            seen.push(ctx.invocation_id.to_owned());
        }
        if payload.object().get("refuse").is_some() {
            return Err(OperationFailure::new("mint-refused"));
        }
        Ok(OperationResult::new(
            CanonicalJsonObject::parse(br#"{"minted":true}"#).expect("canonical result"),
        ))
    }
}

/// The declared operation reference.
///
/// The committed `Operation` resource type is not in the standard catalog
/// yet, so the reference is provider-qualified - which is exactly what a
/// provider-owned operation type is.
fn operation_ref() -> ResourceRef {
    ResourceRef::parse("harness.d2bus.org.Operation/harness-mint")
        .expect("a declared operation reference")
}

fn declarations() -> HarnessDeclarations {
    HarnessDeclarations {
        owned_types: &[
            WellKnownType::VOLUME,
            WellKnownType::PROCESS,
            WellKnownType::GUEST,
        ],
        creations: DECLARATION_ROWS,
        operations: &OPERATIONS[..],
        startup: STARTUP_ROWS,
    }
}

struct FakePortProvider;

#[async_trait]
impl ProviderBase for FakePortProvider {
    fn declaration(&self) -> &ProviderDeclaration {
        &DECLARATION
    }

    fn drivers(&self) -> &'static [DriverDescriptor] {
        &[]
    }

    async fn attach(&self, _zone: &ZonePlaneHandle) -> Result<(), AttachError> {
        Ok(())
    }

    async fn drain(&self, _deadline: DrainDeadline) -> Result<(), DrainError> {
        Ok(())
    }
}

/// The driver body under test: a scripted reconcile that realizes its
/// declared child through the real creation fence.
struct ReconcileFake;

#[async_trait]
impl ReconcileTarget for ReconcileFake {
    async fn reconcile(&self, ctx: ReconcileCtx<'_>, cause: &ReconcileCause) -> ReconcileOutcome {
        match cause {
            ReconcileCause::OwnedSpecChanged => {
                let child = CanonicalJsonObject::parse(CHILD_SPEC).expect("canonical child spec");
                match ctx
                    .creations
                    .create_child(&DECLARED_CREATIONS[0], "worker-0", child)
                    .await
                {
                    Ok(()) => ReconcileOutcome::Ready,
                    Err(failure) => ReconcileOutcome::Failed {
                        code: failure.code(),
                    },
                }
            }
            ReconcileCause::DependentChanged(_) => ReconcileOutcome::NotYet {
                requeue_after_ms: 250,
            },
            ReconcileCause::TargetChanged(_) => ReconcileOutcome::NotYet {
                requeue_after_ms: u64::MAX,
            },
            ReconcileCause::OwnedStatusChanged
            | ReconcileCause::Requeue
            | ReconcileCause::WatchFired => ReconcileOutcome::Ready,
        }
    }
}

/// A startup-step executor that records the order it ran in.
struct RecordingStartup(Mutex<Vec<&'static str>>);

#[async_trait]
impl StartupStepExecutor for RecordingStartup {
    async fn execute(&self, step: &'static StartupStep) -> Result<(), StartupStepError> {
        if let Ok(mut executed) = self.0.lock() {
            executed.push(step.id);
        }
        Ok(())
    }
}

fn caller() -> ResourceRef {
    ResourceRef::parse("User/alice").expect("a committed caller reference")
}

fn volume_spec() -> CanonicalJsonObject {
    CanonicalJsonObject::parse(br#"{"kind":"local"}"#).expect("canonical volume spec")
}

/// A harness whose store already holds the provider identity its declared
/// child spec references.
fn harness() -> TestHarness<FakePortProvider> {
    let harness = TestHarness::with_declarations(FakePortProvider, declarations());
    harness
        .commit(
            WellKnownType::PROVIDER,
            CHILD_PROVIDER,
            CanonicalJsonObject::empty(),
        )
        .expect("the provider identity commits");
    harness
}

#[tokio::test]
async fn a_reconcile_pass_carries_its_cause_and_realizes_a_declared_child() {
    let harness = harness();
    let row = harness
        .admit(WellKnownType::VOLUME, "media", volume_spec())
        .expect("a declared volume admits");

    let outcome = harness
        .reconcile(&ReconcileFake, &row, ReconcileCause::OwnedSpecChanged)
        .await
        .expect("the row is admitted");
    assert_eq!(outcome, ReconcileOutcome::Ready);
    assert_eq!(row.status().phase(), RowPhase::Ready);
    let children = harness.created_children();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].resource_type(), WellKnownType::PROCESS);
    assert_eq!(children[0].name(), "worker-0");

    let outcome = harness
        .reconcile(
            &ReconcileFake,
            &row,
            ReconcileCause::TargetChanged(row.resource_ref()),
        )
        .await
        .expect("the target resolves");
    assert_eq!(outcome.class(), "not-yet");
    assert_eq!(
        outcome.requeue_after_ms(),
        60_000,
        "an unbounded requeue is clamped to the frozen ceiling"
    );
    assert_eq!(row.status().phase(), RowPhase::NotYet);
    assert_eq!(harness.causes().len(), 2);

    let unadmitted = TestHarness::new(FakePortProvider);
    assert_eq!(
        unadmitted
            .reconcile(&ReconcileFake, &row, ReconcileCause::Requeue)
            .await
            .expect_err("another harness never admitted this row")
            .code(),
        "unadmitted-row"
    );
    assert_eq!(
        harness
            .reconcile(
                &ReconcileFake,
                &row,
                ReconcileCause::TargetChanged(
                    ResourceRef::parse("Volume/absent").expect("a valid reference")
                ),
            )
            .await
            .expect_err("the target is not committed")
            .code(),
        "unresolved-target"
    );
}

#[tokio::test]
async fn a_granted_operation_runs_through_the_envelope_and_is_audited() {
    let harness = harness();
    SEEN_INVOCATIONS.lock().expect("invocation log").clear();
    harness.grant(&caller(), &operation_ref());

    let result = harness
        .call_operation(
            &caller(),
            &operation_ref(),
            CanonicalJsonObject::parse(br#"{"name":"worker"}"#).expect("canonical payload"),
        )
        .await
        .expect("a granted caller reaches the declared handler");
    assert!(result.object().get("minted").is_some());
    let seen = SEEN_INVOCATIONS.lock().expect("invocation log");
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].starts_with("invocation-"),
        "the envelope mints an invocation id"
    );
    drop(seen);

    let events = harness.audit_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome(), ProviderAgentAuditOutcome::Accepted);
    assert_eq!(events[0].method().as_str(), "harness-mint");
}

#[tokio::test]
async fn an_ungranted_and_an_uncommitted_invocation_are_refused_and_audited() {
    let harness = harness();
    let uncommitted = ResourceRef::parse("harness.d2bus.org.Operation/harness-absent")
        .expect("a valid reference");

    assert_eq!(
        harness
            .call_operation(&caller(), &uncommitted, CanonicalJsonObject::empty())
            .await
            .expect_err("no declared handler serves it")
            .code(),
        "uncommitted-operation"
    );
    assert_eq!(
        harness
            .call_operation(&caller(), &operation_ref(), CanonicalJsonObject::empty())
            .await
            .expect_err("the caller holds no grant")
            .code(),
        "ungranted-caller"
    );
    harness.grant(&caller(), &operation_ref());
    harness.envelope().revoke_grant(&caller(), &operation_ref());
    assert_eq!(
        harness
            .call_operation(&caller(), &operation_ref(), CanonicalJsonObject::empty())
            .await
            .expect_err("a revoked grant grants nothing")
            .code(),
        "ungranted-caller"
    );

    let events = harness.audit_events();
    assert_eq!(events.len(), 3);
    assert!(
        events
            .iter()
            .all(|event| event.outcome() == ProviderAgentAuditOutcome::Denied)
    );
    assert_eq!(harness.envelope().grant_count(), 0);
}

#[test]
fn an_undeclared_creation_is_refused_and_a_controller_owned_one_too() {
    let harness = harness();
    let child_spec = CanonicalJsonObject::parse(CHILD_SPEC).expect("canonical child spec");

    let refusal = harness
        .create_child(
            WellKnownType::VOLUME,
            &UNDECLARED_CREATION,
            "spare",
            child_spec.clone(),
        )
        .expect_err("no driver declared this creation");
    assert_eq!(refusal.code(), "undeclared-creation");
    assert_eq!(
        refusal,
        ChildCreationFailure::Declaration(CreationRefusal::Undeclared {
            declaring: WellKnownType::VOLUME,
            child: WellKnownType::VOLUME,
            provider_ref: "volume-local",
        })
    );

    let refusal = harness
        .create_child(
            WellKnownType::GUEST,
            &CONTROLLER_OWNED_CREATION,
            "vm-0",
            child_spec.clone(),
        )
        .expect_err("the controller creates this child");
    assert_eq!(refusal.code(), "controller-owned-creation");

    harness
        .create_child(
            WellKnownType::VOLUME,
            &DECLARED_CREATIONS[0],
            "worker-1",
            child_spec,
        )
        .expect("a declared, driver-owned creation commits");
    assert_eq!(harness.created_children().len(), 1);
    assert_eq!(
        harness
            .expect_created(
                WellKnownType::VOLUME,
                WellKnownType::PROCESS,
                CHILD_PROVIDER
            )
            .expect("the pair is declared")
            .custody,
        ChildCustody::DriverOwned
    );
    assert_eq!(
        harness
            .expect_created(WellKnownType::VOLUME, WellKnownType::PROCESS, "unknown")
            .expect_err("the provider is not declared")
            .code(),
        "undeclared-creation"
    );
}

#[tokio::test]
async fn scripted_faults_are_consumed_in_order_and_bounded_by_the_clock() {
    let harness = harness();
    harness.script_faults(FaultPlan::failing_first(2));
    let faults = harness.faults();
    assert!(faults.check().is_err());
    assert!(faults.check().is_err());
    assert!(
        faults.check().is_ok(),
        "the plan stops injecting once consumed"
    );

    let deadline = harness.drain_deadline(0);
    assert_eq!(
        deadline.budget_ms(),
        1,
        "a zero budget is bounded to one millisecond"
    );
    assert!(!deadline.expired());
    let before = harness.clock().now_unix_ms();
    harness.clock().advance(5);
    assert_eq!(harness.clock().now_unix_ms(), before + 5);
    assert!(deadline.expired());
    harness
        .drain(1_000)
        .await
        .expect("drain under a fresh deadline succeeds");
}

#[tokio::test]
async fn the_derived_startup_order_and_plane_attach_run_through_the_base() {
    let harness = harness();
    assert_eq!(
        harness.startup_order().expect("the declared rows derive"),
        vec!["commit-binding", "publish-listener"],
        "a step runs only after every input a predecessor commits"
    );
    let executor = RecordingStartup(Mutex::new(Vec::new()));
    harness
        .run_startup(Some(&executor))
        .await
        .expect("the declared steps execute");
    assert_eq!(
        executor.0.lock().expect("executed steps").as_slice(),
        ["commit-binding", "publish-listener"]
    );
    assert_eq!(
        harness
            .run_startup(None)
            .await
            .expect_err("a declared step needs an executor")
            .code(),
        "startup-executor-missing"
    );

    harness
        .attach()
        .await
        .expect("attach runs through the base");
    assert!(harness.attached());
    assert_eq!(harness.plane_calls(), Vec::<PlaneCall>::new());
}

#[test]
fn the_harness_uses_a_deterministic_clock() {
    let harness = TestHarness::with_clock(
        FakePortProvider,
        std::sync::Arc::new(DeterministicClock::new(1_700_000_000_000)),
    );
    assert_eq!(harness.clock().now_unix_ms(), 1_700_000_000_000);
}

/// A guest agent: the base's shape minus the host-plane bits.
struct FakeGuestAgent;

#[async_trait]
impl GuestAgent for FakeGuestAgent {
    fn declaration(&self) -> &ProviderDeclaration {
        &DECLARATION
    }

    fn drivers(&self) -> &'static [DriverDescriptor] {
        &[]
    }

    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError> {
        assert!(!deadline.expired(), "drain runs under a live deadline");
        Ok(())
    }
}

/// A guest enrollment that answers, standing in for the allocator handlers.
struct ScriptedEnrollment;

#[async_trait]
impl GuestEnrollment for ScriptedEnrollment {
    async fn enroll(&self, request: &EnrollmentRequest) -> Result<EnrolledRoute, GuestError> {
        assert_eq!(request.provider_ref.name().as_str(), PROVIDER_REF);
        Ok(EnrolledRoute {
            zone: ZoneId::parse("dev").expect("a valid guest zone"),
            generation: 1,
        })
    }
}

#[test]
fn a_guest_agent_runs_on_the_same_base_and_enrollment_refuses_without_handlers() {
    assert_eq!(
        run_guest_with(FakeGuestAgent, ScriptedEnrollment),
        0,
        "a scripted enrollment drives the guest lifecycle"
    );
    assert_eq!(
        run_guest(FakeGuestAgent),
        1,
        "guest enrollment is refused fail-closed until ZoneBootstrap/ZoneEnroll land"
    );
}

#[test]
fn the_envelope_refusal_codes_must_stay_closed_and_grammar_conformant() {
    d2b_provider_toolkit::testing::conformance::check_closed_code_set(
        &d2b_provider_toolkit::operations::ENVELOPE_REFUSALS,
    )
    .expect("the envelope refusal set is closed and grammar-conformant");
}
