//! End-to-end harness tests over a fake-port Provider.
//!
//! Every fence here is the provider's own: the declaration fence, the
//! child-creation fence, the operation envelope, and the audit ring. Only
//! the effect ports are scripted.

use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use d2b_session::OwnedTransport;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_contracts_resource::v3::resource_schema::CanonicalJsonObject;
use d2b_provider_toolkit::{
    AllocatorEnrollment, AttachError, Cardinality, ChildCreation, ChildCreationFailure,
    ChildCustody, CreationRefusal, DeterministicClock, DrainDeadline, DrainError, DriverDescriptor,
    FaultPlan, GuestAgent, GuestError, GuestFrame, GuestLink, GuestLinkFuture, GuestPlacement,
    HarnessDeclarations, IsolationPosture,
    OperationCtx, OperationDef, OperationFailure, OperationHandler, OperationResult, PlaneCall,
    ProviderAgentAuditOutcome, ProviderBase, ProviderDeclaration, ReconcileCause, ReconcileCtx,
    ReconcileOutcome, ReconcileTarget, RowPhase, StartupStep, StartupStepError, StartupStepExecutor,
    TestHarness, ValidatedPayload, WellKnownType, ZonePlaneHandle, run_guest,
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

/// A foreign-typed creation no driver declared.
static UNDECLARED_FOREIGN_CREATION: ChildCreation = ChildCreation {
    child: WellKnownType::PROCESS,
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
        // The one type the fake provider serves. The child its declaration
        // licenses is a foreign type the provider does not own, so it commits
        // through the declaration rather than through this list.
        owned_types: &[WellKnownType::VOLUME],
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

/// A declared child of a foreign type commits: the declaration handle is the
/// license, not the set of types the provider itself serves.
#[test]
fn a_declared_foreign_child_commits_through_its_declaration() {
    let harness = harness();
    let child = harness
        .create_child(
            WellKnownType::VOLUME,
            &DECLARED_CREATIONS[0],
            "worker-foreign",
            CanonicalJsonObject::parse(CHILD_SPEC).expect("canonical child spec"),
        )
        .expect("the declaring driver's own declaration licenses the foreign child");
    assert_eq!(child.resource_type(), WellKnownType::PROCESS);
    assert_eq!(child.name(), "worker-foreign");
    assert_eq!(harness.created_children().len(), 1);
}

/// A child no declaration licenses stays refused, however foreign its type:
/// the declaring driver's own rows are the only license, and the owned-type
/// fence still refuses a foreign row no declaration covers.
#[test]
fn an_undeclared_foreign_child_is_refused() {
    let harness = harness();
    let spec = CanonicalJsonObject::parse(CHILD_SPEC).expect("canonical child spec");

    let refusal = harness
        .create_child(
            WellKnownType::VOLUME,
            &UNDECLARED_FOREIGN_CREATION,
            "worker-spare",
            spec.clone(),
        )
        .expect_err("no driver declared this pair");
    assert_eq!(refusal.code(), "undeclared-creation");

    let refusal = harness
        .create_child(
            WellKnownType::GUEST,
            &DECLARED_CREATIONS[0],
            "worker-foreign",
            spec.clone(),
        )
        .expect_err("another driver declared this creation");
    assert_eq!(refusal.code(), "foreign-creation");

    assert!(harness.created_children().is_empty());
    harness
        .commit(WellKnownType::PROCESS, "worker-spare", spec.clone())
        .expect("a refused creation commits no row");
    assert_eq!(
        harness
            .admit(WellKnownType::PROCESS, "worker-foreign", spec)
            .expect_err("the provider owns no Process rows")
            .code(),
        "undeclared-type"
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
///
/// It serves one frame by answering it and raises one event of its own, then
/// ends its event stream so the session closes cleanly.
struct FakeGuestAgent {
    served: Arc<Mutex<Vec<Vec<u8>>>>,
    events: Mutex<std::collections::VecDeque<Vec<u8>>>,
}

#[async_trait]
impl GuestAgent for FakeGuestAgent {
    fn declaration(&self) -> &ProviderDeclaration {
        &DECLARATION
    }

    fn drivers(&self) -> &'static [DriverDescriptor] {
        &[]
    }

    async fn serve(&self, frame: GuestFrame) -> Result<Vec<GuestFrame>, GuestError> {
        self.served
            .lock()
            .expect("served frames")
            .push(frame.as_bytes().to_vec());
        Ok(vec![
            GuestFrame::new(b"served".to_vec()).expect("bounded frame"),
        ])
    }

    async fn next_event(&self) -> Option<GuestFrame> {
        let next = self.events.lock().expect("events").pop_front();
        match next {
            Some(payload) => Some(GuestFrame::new(payload).expect("bounded frame")),
            None => std::future::pending().await,
        }
    }

    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError> {
        assert!(!deadline.expired(), "drain runs under a live deadline");
        Ok(())
    }
}

/// A Guest link over an in-memory duplex wearing the real vsock framing.
///
/// The base never learns which transport it is on: this link hands out one
/// duplex-backed `FramedVsockTransport`, and the allocator side of the test
/// holds the other end.
struct FakeVsockLink {
    client: Mutex<Option<tokio::io::DuplexStream>>,
}

impl FakeVsockLink {
    fn new(client: tokio::io::DuplexStream) -> Self {
        Self {
            client: Mutex::new(Some(client)),
        }
    }
}

impl GuestLink for FakeVsockLink {
    fn connect(&self) -> GuestLinkFuture {
        let client = self.client.lock().expect("link").take();
        Box::pin(async move {
            let stream = client.ok_or(GuestError::LinkUnavailable)?;
            let transport: Box<dyn d2b_session::OwnedTransport> =
                Box::new(d2b_session_unix::FramedVsockTransport::new(stream));
            Ok(transport)
        })
    }
}

fn test_placement() -> GuestPlacement {
    GuestPlacement::new(
        d2b_contracts_zone_session::v3::zone_session::ZoneEnrollmentIdentity {
            zone_link_uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "11111111-1111-4111-8111-111111111111",
            )
            .expect("a valid UID"),
            edge: d2b_contracts_zone_session::v3::zone_routing::ZoneTreeEdge::new(
                d2b_contracts_zone_session::v3::zone_routing::ZonePath::new(vec![
                    d2b_contracts_zone_session::v3::zone_routing::ZoneLabelId::parse("k0")
                        .expect("a valid label"),
                ])
                .expect("a valid zone"),
                d2b_contracts_zone_session::v3::zone_routing::ZonePath::new(vec![
                    d2b_contracts_zone_session::v3::zone_routing::ZoneLabelId::parse("k1")
                        .expect("a valid label"),
                    d2b_contracts_zone_session::v3::zone_routing::ZoneLabelId::parse("k0")
                        .expect("a valid label"),
                ])
                .expect("a valid zone"),
            )
            .expect("a direct child edge"),
            controller_generation:
                d2b_contracts_zone_session::v3::zone_routing::ZoneLinkControllerGeneration::parse(
                    "controller-1",
                )
                .expect("a valid generation"),
            reconnect_generation: d2b_contracts_resource::v3::identity::ReconnectGeneration::new(7)
                .expect("a valid generation"),
            schema_fingerprint: [0x11; 32],
        },
        1,
        300_000,
        1_700_000_000_000,
        [0x33; 32],
    )
    .expect("a valid placement")
}

/// Answer the two enrollment calls, then serve one frame and one event.
async fn scripted_allocator(stream: tokio::io::DuplexStream) {
    use d2b_contracts_zone_session::v3::zone_session::{
        ZoneBootstrapCall, ZoneBootstrapReply, ZoneEnrollCall, ZoneEnrollReply,
    };

    let mut transport = d2b_session_unix::FramedVsockTransport::new(stream);
    let bootstrap = transport
        .receive(64 * 1024)
        .await
        .expect("a bootstrap call");
    let call: ZoneBootstrapCall =
        ZoneBootstrapCall::decode(bootstrap.as_bytes()).expect("a decodable bootstrap call");
    assert_eq!(call.issuance, 1);
    transport
        .send(d2b_session::TransportPacket::new(
            ZoneBootstrapReply::Admitted {
                expires_at_unix_ms: 1_700_000_300_000,
            }
            .encode()
            .expect("an encodable reply"),
        ))
        .await
        .expect("the bootstrap reply is sent");

    let enroll = transport.receive(64 * 1024).await.expect("an enroll call");
    let call: ZoneEnrollCall =
        ZoneEnrollCall::decode(enroll.as_bytes()).expect("a decodable enroll call");
    assert_eq!(call.observed_peer_fingerprint, [0x33; 32]);
    transport
        .send(d2b_session::TransportPacket::new(
            ZoneEnrollReply::Enrolled {
                zone: ZoneId::parse("zone-k1").expect("a valid zone"),
                generation: 1,
            }
            .encode()
            .expect("an encodable reply"),
        ))
        .await
        .expect("the enrollment reply is sent");

    // One served frame and the agent's own event, in the order the base
    // writes them.
    transport
        .send(d2b_session::TransportPacket::new(b"relay".to_vec()))
        .await
        .expect("the served frame is sent");
    let reply = transport.receive(64 * 1024).await.expect("a served reply");
    assert_eq!(reply.as_bytes(), b"served");
    let event = transport.receive(64 * 1024).await.expect("an agent event");
    assert_eq!(event.as_bytes(), b"uhid-report");
}

#[test]
fn a_guest_agent_enrolls_serves_and_drains_over_a_faked_vsock_transport() {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let allocator = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the allocator runtime builds");
        runtime.block_on(scripted_allocator(server));
    });

    let served = Arc::new(Mutex::new(Vec::new()));
    let agent = FakeGuestAgent {
        served: Arc::clone(&served),
        events: Mutex::new(std::collections::VecDeque::from(vec![
            b"uhid-report".to_vec(),
        ])),
    };
    let code = run_guest(
        agent,
        Box::new(FakeVsockLink::new(client)),
        Arc::new(AllocatorEnrollment::new(test_placement())),
    );
    assert_eq!(code, 0, "the guest lifecycle completes");
    allocator.join().expect("the allocator task completes");
    assert_eq!(
        served.lock().expect("served frames").as_slice(),
        &[b"relay".to_vec()],
        "the agent served exactly the frame the enrolled session carried"
    );
}

#[test]
fn a_refused_enrollment_ends_the_guest_lifecycle_without_serving() {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let allocator = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the allocator runtime builds");
        runtime.block_on(async move {
            use d2b_contracts_zone_session::v3::zone_session::{
                ZoneBootstrapReply, ZoneEnrollmentRefusal,
            };
            let mut transport = d2b_session_unix::FramedVsockTransport::new(server);
            let _ = transport.receive(64 * 1024).await.expect("a bootstrap call");
            transport
                .send(d2b_session::TransportPacket::new(
                    ZoneBootstrapReply::Refused {
                        reason: ZoneEnrollmentRefusal::BootstrapPskConsumed,
                    }
                    .encode()
                    .expect("an encodable reply"),
                ))
                .await
                .expect("the refusal is sent");
        });
    });

    let agent = FakeGuestAgent {
        served: Arc::new(Mutex::new(Vec::new())),
        events: Mutex::new(std::collections::VecDeque::new()),
    };
    let code = run_guest(
        agent,
        Box::new(FakeVsockLink::new(client)),
        Arc::new(AllocatorEnrollment::new(test_placement())),
    );
    assert_eq!(code, 1, "a refused enrollment is terminal");
    allocator.join().expect("the allocator task completes");
}

#[test]
fn the_envelope_refusal_codes_must_stay_closed_and_grammar_conformant() {
    d2b_provider_toolkit::testing::conformance::check_closed_code_set(
        &d2b_provider_toolkit::operations::ENVELOPE_REFUSALS,
    )
    .expect("the envelope refusal set is closed and grammar-conformant");
}
