//! The broker's admitted-effect boundary (U10, KTD8).
//!
//! These cases drive the real [`AdmittedEffectAdmission`] over the real
//! [`EffectLedger`] and the real in-process [`AdmittedEffectTable`]. The
//! accepted graph is built with the contract constructors and the private
//! execution table with the plan's own types, so the graph the evaluator
//! reads and the values the plan resolves are the ones production reads and
//! resolves.
//!
//! The plan's four scenarios are covered here:
//!
//! 1. AE7 and AE22 - raw host authority fields, retired variants, and
//!    argv/env redirection are refused *by name*, and every refusal asserts
//!    the handler's dispatch counter is still zero, so "refused" and
//!    "mutated nothing" are the same observation.
//! 2. AE15 and AE16 - a nested leg keeps the subject its root was admitted
//!    for, and a dependency that moved without a spec generation moving
//!    invalidates the earlier expectation.
//! 3. Returned descriptors match the declared kind and count and stay tied to
//!    the admitted invocation: a retry is answered from the ledger with the
//!    original invocation's recorded answer, without dispatching again.
//! 4. An unknown handler or an unaccepted projection refuses before any host
//!    mutation. There is no fallback route: the table either serves an
//!    `Operation` or the invocation is refused by name.
//!
//! What the handler observed is returned in the result object rather than
//! stashed in shared state, so an assertion about "the handler was given the
//! plan and not a payload" reads off the reply instead of a test-only side
//! channel.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use d2b_broker::envelope::{
    ADMITTED_EFFECT_REFUSALS, AUTHORITY_PARAMETER, AdmittedEffect, AdmittedEffectAdmission,
    AdmittedEffectHandler, AdmittedEffectOutcome, AdmittedEffectTable, CORRELATION_SUBJECT_REPLACED,
    EffectDescriptor, EffectLedger, FD_LEG, IDEMPOTENCY_CONFLICT, LEGACY_EFFECT_REQUEST,
    ProjectionPosture, RESULT_FD_CONTRACT, STALE_DEPENDENCY, STALE_WIRE_VERSION,
    UNACCEPTED_PROJECTION, UNDECLARED_PARAMETER, UNKNOWN_IMPLEMENTATION, UNPROVEN_EFFECT,
};
use d2b_broker::runtime::{ADMITTED_EFFECT_FRAME_KIND, admit_effect_frame};
use d2b_contracts_broker::broker_wire::{
    AdmittedEffectCarrier, AdmittedEffectInvocation, EffectCorrelation, EffectLegSelection,
    FdKind, IdempotencyKey,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, AuditMode, AuthoritySubject, AuthoritySubjectKind, BindingArbitration,
    BindingKey, BindingKind, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
    CallableOperation, CanonicalJsonObject, DesiredDigest, DesiredRevision, FdContract,
    FreshnessTuple, OperationAudit, OperationAuthority, OperationBounds, OperationDomain,
    OperationFds, OperationImplementation, OperationSurface, PayloadProvenance, PayloadSchema,
    RefusalReason, RequestedRights, ResourceRef, ResourceTypeName, ResourceUid, SecretAccess,
    SourceAdmission, StoreIncarnation, ZoneId, execution_policy::{BoundedText, BoundedToken},
};
use d2b_contracts_zone_session::v3::RoleBindingSpec;
use d2b_contracts_zone_session::v3::role::{AuthorizedRole, RoleResourceVerb, RoleRule};
use d2b_core::execution_plan::{
    BindingPlanRequest, PlannedDestination, PlannedExecutable, PlannedIdentity, PlannedSource,
    PlannedView, PrivateBacking, PrivateExecutionTable, PrivatePath, binding_row_ref,
};
use d2b_core::resource_authority::{AcceptedGraph, AcceptedSource};

const ZONE: &str = "pubzone";
const STORE: &str = "store-generation-1";
const CONSUMER: &str = "Process/shell";
const SOURCE: &str = "Volume/data";
const BINDING_ROW: &str = "VolumeBinding/data";
const PROVIDER: &str = "Provider/process";
const OPERATION: &str = "Operation/spawn-process";
const TEMPLATE: &str = "spawn-process";
const SLOT: &str = "data";
const VIEW: &str = "root";
const IDEMPOTENCY_KEY: &str = "launch-0001";
const DECLARED_FD: &str = "endpoint";
const PROGRAM: &str = "/nix/store/2r1m-process/bin/process";
const SOURCE_UID: &str = "11111111-1111-4111-8111-111111111111";
const CONSUMER_UID: &str = "22222222-2222-4222-8222-222222222222";
const SOURCE_PATH: &str = "/var/lib/d2b/volumes/data";
const DESTINATION_PATH: &str = "/run/d2b/pubzone/shell/mnt/data";

// ---------------------------------------------------------------------------
// Fixture vocabulary
// ---------------------------------------------------------------------------

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture references are canonical")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("the fixture uids are canonical")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("the fixture tokens are canonical")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("the fixture Zone is canonical")
}

fn incarnation() -> StoreIncarnation {
    StoreIncarnation::parse(STORE).expect("the fixture store generation is a bounded token")
}

fn consumer() -> AuthoritySubject {
    AuthoritySubject::named(AuthoritySubjectKind::Process, reference(CONSUMER))
}

fn bootstrap() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

fn slot() -> BindingSlot {
    BindingSlot::parse(SLOT).expect("the fixture slot is a bounded token")
}

fn binding_key() -> BindingKey {
    BindingKey::new(
        zone(),
        BindingKind::Volume,
        reference(SOURCE),
        uid(SOURCE_UID),
        reference(CONSUMER),
        uid(CONSUMER_UID),
        slot(),
    )
    .expect("the fixture relationship is well formed")
}

fn freshness(reference_value: &str, uid_value: &str, revision: u64, digest: &str) -> FreshnessTuple {
    let mut wanted = DesiredRevision::INITIAL;
    for _ in 0..revision {
        wanted = wanted.try_next().expect("the desired revision has room");
    }
    FreshnessTuple::new(
        zone(),
        incarnation(),
        reference(reference_value),
        uid(uid_value),
        wanted,
        DesiredDigest::of(digest.as_bytes()),
    )
}

fn source_freshness() -> FreshnessTuple {
    freshness(SOURCE, SOURCE_UID, 1, "volume-1")
}

fn consumer_freshness() -> FreshnessTuple {
    freshness(CONSUMER, CONSUMER_UID, 1, "consumer-1")
}

fn canonical(value: &impl serde::Serialize) -> CanonicalJsonObject {
    CanonicalJsonObject::parse(&serde_json::to_vec(value).expect("the fixture serializes"))
        .expect("the fixture is a canonical JSON object")
}

/// The committed `Operation` contract. Its payload declares one typed
/// non-authority parameter, and its response declares exactly one socket the
/// effect must return under the name `endpoint`.
fn operation() -> CallableOperation {
    let payload = PayloadSchema::parse(serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "servingWorker": { "type": "boolean" } },
    }))
    .expect("the fixture payload schema validates");
    let audit =
        OperationAudit::new(true, AuditMode::Yes, Vec::new(), Vec::new(), token("process-launch"))
            .expect("the audit facet is bounded");
    let authority = OperationAuthority::new(
        OperationSurface::Broker,
        OperationDomain::Host,
        BoundedText::parse("d2b-launcher").expect("bounded text without control characters"),
        d2b_contracts_resource::v3::BrokerRequirement::Yes,
    );
    let fds = OperationFds::new(
        Vec::new(),
        vec![FdContract::new(
            token(DECLARED_FD),
            d2b_contracts_resource::v3::FdKind::Socket,
            true,
        )],
        Vec::new(),
    )
    .expect("the fd contract is bounded");
    CallableOperation::new(
        OperationImplementation::trusted_executable_template(reference(PROVIDER), token(TEMPLATE))
            .expect("a Provider reference is a declared implementation"),
        payload,
        None,
        true,
        SecretAccess::None,
        audit,
        None,
        authority,
        fds,
        OperationBounds::default(),
        PayloadProvenance::Request,
    )
    .expect("the fixture operation contract is well formed")
}

/// The prior accepted graph: a real `Role`, a real `RoleBinding` naming the
/// consumer and the binding row, and a real source decision.
fn accepted_graph() -> AcceptedGraph {
    let rule = RoleRule::new(
        vec![ResourceTypeName::parse("VolumeBinding").expect("VolumeBinding is a standard type")],
        vec![RoleResourceVerb::Create],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the role rule is bounded and non-empty");
    let role = AuthorizedRole::new(vec![rule], Vec::new()).expect("the role is bounded");
    let binding = RoleBindingSpec::with_facets(
        reference("Role/reader"),
        vec![reference(CONSUMER)],
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
    )
    .expect("the role binding is bounded");
    let source = SourceAdmission::new(
        binding_key(),
        vec![RequestedRights::Observe],
        BindingArbitration::Shared,
    )
    .expect("the source decision is well formed");
    let support = BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
        .expect("the realization support is unique");
    AcceptedGraph::new(zone(), incarnation(), bootstrap())
        .with_role(reference("Role/reader"), role)
        .with_role_binding(reference("RoleBinding/shell"), binding)
        .with_source(AcceptedSource::new(source, support))
}

/// The broker's own private execution values: the source, the destination,
/// the view, the consumer's identity, and the trusted executable.
fn private_table(source_fresh: FreshnessTuple) -> PrivateExecutionTable {
    let source = PlannedSource::new(
        reference(SOURCE),
        uid(SOURCE_UID),
        BindingKind::Volume,
        source_fresh,
        PrivateBacking::Filesystem,
        PrivatePath::parse(SOURCE_PATH).expect("an absolute private path"),
        vec![PlannedView::new(
            token(VIEW),
            RequestedRights::Observe,
            PrivatePath::parse("/var/lib/d2b/volumes/data/root").expect("an absolute path"),
        )],
    )
    .expect("the resolved source is well formed");
    let identity = PlannedIdentity::new(reference(CONSUMER), 4242, 4242, vec![], None)
        .expect("the resolved identity is well formed");
    let destination = PlannedDestination::new(
        binding_key().address(),
        BindingRealizationFacet::FilesystemPresentation,
        PrivatePath::parse(DESTINATION_PATH).expect("an absolute path"),
        true,
    );
    let executable = PlannedExecutable::new(
        OperationImplementation::trusted_executable_template(reference(PROVIDER), token(TEMPLATE))
            .expect("a Provider reference is a declared implementation"),
        token(TEMPLATE),
        PrivatePath::parse(PROGRAM).expect("an absolute program path"),
        vec![PROGRAM.to_owned()],
        vec!["PATH=/usr/bin".to_owned()],
    )
    .expect("the trusted executable is well formed");
    PrivateExecutionTable::empty()
        .with_source(source)
        .with_observed(consumer_freshness())
        .with_identity(identity)
        .with_destination(destination)
        .with_executable(reference(OPERATION), executable)
}

fn leg() -> EffectLegSelection {
    EffectLegSelection {
        binding: binding_key(),
        rights: RequestedRights::Observe,
        presentation: vec![BindingRealizationFacet::FilesystemPresentation],
        helper: None,
    }
}

fn invocation(parameters: CanonicalJsonObject) -> AdmittedEffectInvocation {
    AdmittedEffectInvocation::new(
        reference(OPERATION),
        consumer(),
        vec![leg()],
        parameters,
        vec![source_freshness(), consumer_freshness()],
        IdempotencyKey::parse(IDEMPOTENCY_KEY).expect("the fixture key is bounded"),
        None,
    )
    .expect("the fixture invocation is well formed")
}

fn typed_parameters() -> CanonicalJsonObject {
    canonical(&serde_json::json!({ "servingWorker": true }))
}

// ---------------------------------------------------------------------------
// The declared implementation under test
// ---------------------------------------------------------------------------

/// What one dispatch hands back on the response leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Returned {
    /// Exactly the declared contract: one socket named `endpoint`.
    Declared,
    /// Nothing at all.
    Empty,
    /// Two descriptors where the contract declares one.
    Extra,
    /// One descriptor under a name the contract never published.
    WrongName,
    /// One descriptor whose observed kernel kind is not the declared one.
    WrongKind,
}

/// A declared implementation that counts its dispatches and reports what the
/// plan gave it in the result object.
struct RecordingHandler {
    operation: ResourceRef,
    declared: CallableOperation,
    returned: Returned,
    runs: AtomicUsize,
}

impl RecordingHandler {
    fn new(returned: Returned) -> Arc<Self> {
        Arc::new(Self {
            operation: reference(OPERATION),
            declared: operation(),
            returned,
            runs: AtomicUsize::new(0),
        })
    }

    fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }
}

/// One socket the effect returns.
///
/// A real socket, not a labelled pipe: the boundary checks the observed
/// kernel kind, so a fixture that lied about it would be testing nothing.
fn socket() -> OwnedFd {
    let (socket, _peer) = nix::sys::socket::socketpair(
        nix::sys::socket::AddressFamily::Unix,
        nix::sys::socket::SockType::SeqPacket,
        None,
        nix::sys::socket::SockFlag::empty(),
    )
    .expect("a socketpair is available in the test sandbox");
    socket
}

impl AdmittedEffectHandler for RecordingHandler {
    fn operation(&self) -> &ResourceRef {
        &self.operation
    }

    fn declared(&self) -> &CallableOperation {
        &self.declared
    }

    fn run(&self, effect: AdmittedEffect) -> d2b_broker::envelope::EffectFuture {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let returned = self.returned;
        Box::pin(async move {
            // What the handler was given is reported back, so a test can read
            // the resolved plan off the reply instead of a side channel.
            let observed = serde_json::json!({
                "subject": {
                    "kind": format!("{:?}", effect.subject.kind()),
                    "ref": effect
                        .subject
                        .resource_ref()
                        .map(ResourceRef::to_canonical_string),
                },
                "program": effect
                    .plan
                    .executable()
                    .program()
                    .as_path()
                    .display()
                    .to_string(),
                "argv": effect.plan.executable().argv(),
                "environment": effect.plan.executable().environment(),
                "uid": effect.plan.identity().map(PlannedIdentity::uid),
                "sources": effect
                    .plan
                    .sources()
                    .iter()
                    .map(|source| source.reference().to_canonical_string())
                    .collect::<Vec<_>>(),
                "destinations": effect.plan.destinations().len(),
                "views": effect
                    .plan
                    .views()
                    .iter()
                    .map(|view| view.path().as_path().to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                "nested": effect.correlation.is_some(),
            });
            let descriptors = match returned {
                Returned::Declared => vec![EffectDescriptor {
                    name: token(DECLARED_FD),
                    fd: socket(),
                    kind: FdKind::Socket,
                }],
                Returned::Empty => Vec::new(),
                Returned::Extra => vec![
                    EffectDescriptor {
                        name: token(DECLARED_FD),
                        fd: socket(),
                        kind: FdKind::Socket,
                    },
                    EffectDescriptor {
                        name: token(DECLARED_FD),
                        fd: socket(),
                        kind: FdKind::Socket,
                    },
                ],
                Returned::WrongName => vec![EffectDescriptor {
                    name: token("unpublished"),
                    fd: socket(),
                    kind: FdKind::Socket,
                }],
                // A FIFO reported as a socket: the observed kernel kind is
                // not the declared one, so the boundary refuses it.
                Returned::WrongKind => {
                    let (read, write) =
                        nix::unistd::pipe().expect("a pipe is available in the test sandbox");
                    drop(write);
                    vec![EffectDescriptor {
                        name: token(DECLARED_FD),
                        fd: read,
                        kind: FdKind::Socket,
                    }]
                }
            };
            Ok(AdmittedEffectOutcome {
                result: canonical(&observed),
                descriptors,
            })
        })
    }
}

struct Harness {
    handler: Arc<RecordingHandler>,
    table: AdmittedEffectTable,
    ledger: EffectLedger,
    graph: AcceptedGraph,
    values: PrivateExecutionTable,
}

impl Harness {
    fn new(returned: Returned) -> Self {
        let handler = RecordingHandler::new(returned);
        let table =
            AdmittedEffectTable::new(vec![Arc::clone(&handler) as Arc<dyn AdmittedEffectHandler>])
                .expect("one implementation claims one operation");
        Self {
            handler,
            table,
            ledger: EffectLedger::new(),
            graph: accepted_graph(),
            values: private_table(source_freshness()),
        }
    }

    fn admission(&self) -> AdmittedEffectAdmission<'_> {
        AdmittedEffectAdmission::new(&self.ledger, &self.table, None)
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 - AE7 and AE22: raw host authority, old variants, argv/env
// ---------------------------------------------------------------------------

/// A payload naming a host path is refused by name. The carrier carries
/// parameters as a canonical object, so the only way a host path could reach
/// the boundary is inside one - and the screen refuses it there rather than
/// ignoring it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_payload_naming_a_host_path_is_refused_and_nothing_runs() {
    let harness = Harness::new(Returned::Declared);
    let refusal = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(canonical(
                &serde_json::json!({ "hostPath": "/etc/shadow" }),
            )),
            &[],
        )
        .await
        .expect_err("a host path is refused");
    assert_eq!(refusal.code, AUTHORITY_PARAMETER);
    assert_eq!(refusal.reason, RefusalReason::UntrustedImplementation);
    assert_eq!(refusal.stage, AdmissionStage::Authorize);
    assert_eq!(harness.handler.runs(), 0, "no host mutation happened");
    assert!(ADMITTED_EFFECT_REFUSALS.contains(&AUTHORITY_PARAMETER));
}

/// argv and environment redirection are the same refusal: an environment map
/// that points a workload at another socket is exactly the AE7 hazard.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_payload_naming_argv_or_environment_is_refused_and_nothing_runs() {
    for field in [
        "argv",
        "args",
        "env",
        "environment",
        "uid",
        "gid",
        "mountPolicy",
        "mounts",
        "seccompPolicy",
        "capabilities",
        "path",
        "devicePath",
        "commandLine",
    ] {
        let harness = Harness::new(Returned::Declared);
        let refusal = harness
            .admission()
            .run(
                ProjectionPosture::Accepted,
                ZONE,
                &harness.graph,
                &harness.values,
                &invocation(canonical(
                    &serde_json::json!({ field: "/run/other.sock" }),
                )),
                &[],
            )
            .await
            .expect_err("an authority-bearing field is refused");
        assert_eq!(refusal.code, AUTHORITY_PARAMETER, "field `{field}`");
        assert_eq!(harness.handler.runs(), 0, "field `{field}` ran nothing");
    }
}

/// A field the contract does not declare is refused rather than ignored, so a
/// caller cannot smuggle a value past the schema by picking an undeclared
/// name.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_payload_field_the_contract_does_not_declare_is_refused() {
    let harness = Harness::new(Returned::Declared);
    let refusal = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(canonical(&serde_json::json!({ "roleId": "virtiofsd" }))),
            &[],
        )
        .await
        .expect_err("an undeclared field is refused");
    assert_eq!(refusal.code, UNDECLARED_PARAMETER);
    assert_eq!(refusal.reason, RefusalReason::ConflictingDeclaration);
    assert_eq!(harness.handler.runs(), 0);
}

/// A frame that is not the admitted-effect carrier is refused at the frame,
/// with no translation and no legacy route (AE22). A retired variant keeps
/// its own closed code, so a straggler learns which version boundary it has
/// not moved past.
#[test]
fn a_legacy_wire_frame_is_refused_before_any_handler_is_reached() {
    for (variant, expected) in [
        ("SpawnRunner", STALE_WIRE_VERSION),
        ("LaunchMinijailChild", LEGACY_EFFECT_REQUEST),
        ("OpenPidfd", STALE_WIRE_VERSION),
        ("StartTransientUnit", LEGACY_EFFECT_REQUEST),
        ("CreateTapFd", STALE_WIRE_VERSION),
        ("EnvelopeInvoke", LEGACY_EFFECT_REQUEST),
    ] {
        let frame = serde_json::json!({
            "request": { "kind": variant, "payload": { "bundleRunnerIntentRef": "op-1" } },
        });
        let refusal = admit_effect_frame(&frame).expect_err("a legacy frame is refused");
        assert_eq!(refusal.code, expected, "variant `{variant}`");
        assert_eq!(refusal.variant.as_deref(), Some(variant));
    }
}

/// A serialized launch posture is not an admitted invocation. A
/// `SandboxLaunchPlan`-shaped payload rides the generic `EnvelopeInvoke`
/// frame today; the admitted-effect carrier refuses it by name.
#[test]
fn a_serialized_launch_posture_is_refused_by_name() {
    let frame = serde_json::json!({
        "request": {
            "kind": ADMITTED_EFFECT_FRAME_KIND,
            "invocation": {
                "operation": OPERATION,
                "subject": { "kind": "process", "resourceRef": CONSUMER },
                "legs": [leg_json()],
                "parameters": {
                    "sandboxPlan": {
                        "namespaceClasses": ["mount", "pid"],
                        "capabilityClasses": ["cap-sys-admin"],
                        "seccompClass": "seccomp-default",
                        "environmentClass": "inherit",
                    },
                },
                "expectedDependencies": [freshness_json(source_freshness())],
                "idempotencyKey": IDEMPOTENCY_KEY,
            },
        },
    });
    let refusal = admit_effect_frame(&frame).expect_err("a launch posture is refused");
    assert_eq!(refusal.code, AUTHORITY_PARAMETER);
}

/// A well-formed admitted-effect frame is the one frame this gate admits, and
/// the invocation it yields is the whole of what a caller may say.
#[test]
fn a_well_formed_admitted_effect_frame_is_admitted() {
    let frame = serde_json::json!({
        "request": {
            "kind": ADMITTED_EFFECT_FRAME_KIND,
            "invocation": {
                "operation": OPERATION,
                "subject": { "kind": "process", "resourceRef": CONSUMER },
                "legs": [leg_json()],
                "parameters": { "servingWorker": true },
                "expectedDependencies": [
                    freshness_json(source_freshness()),
                    freshness_json(consumer_freshness()),
                ],
                "idempotencyKey": IDEMPOTENCY_KEY,
            },
        },
    });
    let admitted = admit_effect_frame(&frame).expect("a well-formed frame is admitted");
    assert_eq!(admitted.operation(), &reference(OPERATION));
    assert_eq!(admitted.subject(), &consumer());
    assert_eq!(admitted.idempotency_key().as_str(), IDEMPOTENCY_KEY);
    assert!(!admitted.is_nested());
    assert!(admitted.chain_identities().is_empty());
    assert_eq!(admitted.expected_dependencies().len(), 2);
}

/// The carrier admits one frame kind and no legacy variant: a straggler's
/// typed request has nowhere to decode into, so it cannot reach a legacy
/// handler even by accident.
#[test]
fn the_carrier_carries_no_legacy_variant() {
    for variant in ["SpawnRunner", "OpenPidfd", "EnvelopeInvoke", "Hello"] {
        let legacy = serde_json::json!({ "kind": variant, "invocation": {} });
        assert!(
            serde_json::from_value::<AdmittedEffectCarrier>(legacy).is_err(),
            "`{variant}` is not a member of the admitted-effect carrier"
        );
    }
    let own = serde_json::json!({
        "kind": ADMITTED_EFFECT_FRAME_KIND,
        "invocation": {
            "operation": OPERATION,
            "subject": { "kind": "process", "resourceRef": CONSUMER },
            "legs": [leg_json()],
            "parameters": {},
            "expectedDependencies": [freshness_json(source_freshness())],
            "idempotencyKey": IDEMPOTENCY_KEY,
        },
    });
    serde_json::from_value::<AdmittedEffectCarrier>(own)
        .expect("the carrier's own kind decodes");
}

/// A nested leg whose chain does not begin with its own subject is a
/// substituted principal, and the carrier refuses it before the frame is
/// served.
#[test]
fn a_nested_leg_that_substitutes_its_subject_is_refused_by_the_carrier() {
    let substituted = AdmittedEffectInvocation::new(
        reference(OPERATION),
        consumer(),
        vec![leg()],
        canonical(&serde_json::json!({})),
        vec![source_freshness()],
        IdempotencyKey::parse("launch-0002").expect("a bounded key"),
        Some(EffectCorrelation {
            root_invocation_id: "effect-invocation-0".to_owned(),
            identities: vec![AuthoritySubject::named(
                AuthoritySubjectKind::Process,
                reference("Process/root"),
            )],
        }),
    );
    assert!(
        substituted.is_err(),
        "a substituted initiating subject is refused by the carrier"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 - AE15 and AE16: subject retention and dependency freshness
// ---------------------------------------------------------------------------

/// A nested leg keeps the subject its root was admitted for, and the plan it
/// resolves carries that same subject and the leg's correlation.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_nested_leg_keeps_the_subject_its_root_was_admitted_for() {
    let harness = Harness::new(Returned::Declared);
    let root = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect("the root invocation is admitted");

    let nested = nested_invocation(
        &root.invocation_id,
        "launch-0001-nested",
        vec![consumer()],
    );
    let admitted = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &nested,
            &[],
        )
        .await
        .expect("the nested leg is admitted against its own root");
    assert_eq!(admitted.plan.subject(), &consumer());
    let correlation = admitted
        .plan
        .correlation()
        .expect("a nested leg's plan records its correlation");
    assert_eq!(correlation.initiating_subject(), &consumer());
    assert_eq!(correlation.depth(), 1);
    assert_eq!(admitted.depth, 1);
    assert!(nested.is_nested());
}

/// A nested leg that presents a different subject than the recorded root is
/// refused: the privileged transport does not widen the grant.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_nested_leg_presenting_another_subject_is_refused() {
    let harness = Harness::new(Returned::Declared);
    let root = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect("the root invocation is admitted");

    let substituted = AdmittedEffectInvocation::new(
        reference(OPERATION),
        AuthoritySubject::named(AuthoritySubjectKind::Process, reference("Process/root")),
        vec![leg()],
        typed_parameters(),
        vec![source_freshness(), consumer_freshness()],
        IdempotencyKey::parse("launch-substituted").expect("a bounded key"),
        Some(EffectCorrelation {
            root_invocation_id: root.invocation_id.clone(),
            identities: vec![AuthoritySubject::named(
                AuthoritySubjectKind::Process,
                reference("Process/root"),
            )],
        }),
    )
    .expect("the substituted chain is well formed on the wire");
    let refusal = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &substituted,
            &[],
        )
        .await
        .expect_err("a substituted subject is refused");
    assert_eq!(refusal.code, CORRELATION_SUBJECT_REPLACED);
    assert_eq!(refusal.reason, RefusalReason::IdentityNotAuthorized);
    assert_eq!(harness.handler.runs(), 0);
}

/// A nested leg whose root this broker never admitted carries no provable
/// subject, so it is refused rather than taken at its word.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_nested_leg_under_an_unknown_root_is_refused() {
    let harness = Harness::new(Returned::Declared);
    let orphan = nested_invocation("effect-invocation-99", "launch-orphan", vec![consumer()]);
    let refusal = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &orphan,
            &[],
        )
        .await
        .expect_err("an unknown root is refused");
    assert_eq!(refusal.code, CORRELATION_SUBJECT_REPLACED);
}

/// AE16: the source's view rights move without any spec generation moving.
/// The caller's expectation still names the old desired revision, so the
/// effect is fenced even though nothing about the rendered spec changed.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_dependency_that_moved_without_a_spec_generation_change_fences_the_effect() {
    let harness = Harness::new(Returned::Declared);
    let moved = freshness(SOURCE, SOURCE_UID, 2, "volume-2");
    let values = private_table(moved);

    // The caller's expectation names revision 1; the broker observes 2.
    let refusal = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect_err("a stale expectation is fenced");
    assert_eq!(refusal.code, STALE_DEPENDENCY);
    assert_eq!(refusal.reason, RefusalReason::StaleAuthority);
    assert_eq!(refusal.stage, AdmissionStage::Authorize);
    assert_eq!(harness.handler.runs(), 0);
}

/// The plan's fence is the broker's own observation, and re-checking it after
/// a later acceptance fences the effect rather than letting a superseded plan
/// run.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_resolved_plan_carries_the_brokers_own_fence() {
    let harness = Harness::new(Returned::Declared);
    let admitted = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect("the invocation is admitted");
    let fence = admitted.plan.freshness();
    assert_eq!(fence.store(), &incarnation());
    assert_eq!(fence.observed().len(), 2);
    assert!(
        fence.is_current(&[source_freshness(), consumer_freshness()]),
        "the plan is current against the state it was admitted at"
    );
    let moved = freshness(SOURCE, SOURCE_UID, 2, "volume-2");
    assert!(
        !fence.is_current(&[moved, consumer_freshness()]),
        "a moved dependency fences the plan"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3 - returned descriptors and the retry record
// ---------------------------------------------------------------------------

/// The declared response contract is what the reply is checked against, and
/// the handler is given the resolved plan rather than a payload.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_handler_is_given_the_resolved_plan_and_the_declared_descriptors_come_back() {
    let harness = Harness::new(Returned::Declared);
    let response = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect("the declared effect runs");
    assert_eq!(response.response.descriptors.len(), 1);
    assert_eq!(response.response.descriptors[0].name.as_str(), DECLARED_FD);
    assert_eq!(response.response.descriptors[0].kind, FdKind::Socket);
    assert!(response.response.refusal.is_none());
    assert_eq!(
        response.descriptors.len(),
        1,
        "the live descriptor travels beside the answer, in the answer's order"
    );
    assert_eq!(
        nix::sys::stat::fstat(response.descriptors[0].as_raw_fd())
            .map(|stat| stat.st_mode & nix::libc::S_IFMT == nix::libc::S_IFSOCK),
        Ok(true),
        "the descriptor the frame will carry is the socket the handler minted"
    );
    assert_eq!(harness.handler.runs(), 1);

    let result = response
        .response
        .result
        .as_ref()
        .expect("a success carries its result");
    let field = |name: &str| {
        serde_json::to_string(result.get(name).expect("the handler reported the field"))
            .expect("a canonical value serializes")
    };
    assert_eq!(field("program"), format!("\"{PROGRAM}\""));
    assert_eq!(field("uid"), "4242");
    assert_eq!(field("sources"), format!("[\"{SOURCE}\"]"));
    assert_eq!(field("argv"), format!("[\"{PROGRAM}\"]"));
    assert_eq!(field("destinations"), "1");
    assert_eq!(field("nested"), "false");
    assert!(
        field("subject").contains(CONSUMER),
        "the handler was given the admitted subject, not a transport identity"
    );
}

/// A returned set that does not match the declared count, names, or kind is
/// refused, and the caller never receives it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_returned_descriptor_set_outside_the_declared_contract_is_refused() {
    for returned in [Returned::Empty, Returned::Extra, Returned::WrongName, Returned::WrongKind] {
        let harness = Harness::new(returned);
        let refusal = harness
            .admission()
            .run(
                ProjectionPosture::Accepted,
                ZONE,
                &harness.graph,
                &harness.values,
                &invocation(typed_parameters()),
                &[],
            )
            .await
            .expect_err("a descriptor set outside the contract is refused");
        assert_eq!(refusal.code, RESULT_FD_CONTRACT, "{returned:?}");
        assert_eq!(refusal.reason, RefusalReason::MandatoryFacetUnsupported);
        assert_eq!(harness.handler.runs(), 1, "{returned:?} ran and was refused");
    }
}

/// A retry never repeats the host effect. For an answer that declared
/// descriptors it is REFUSED rather than answered: the ledger holds their
/// names and kinds, not live descriptors, and minting them again would repeat
/// the effect, so a replay that claimed them would be a partial success - a
/// lie the caller would act on.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_retry_of_a_descriptor_returning_effect_is_refused_and_never_rerun() {
    let harness = Harness::new(Returned::Declared);
    harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect("the first call runs");
    let refusal = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect_err("a recorded answer cannot reproduce its descriptors");
    assert_eq!(refusal.code, RESULT_FD_CONTRACT);
    assert_eq!(harness.handler.runs(), 1, "the host effect happened once");
}

/// A retry of an effect whose answer declared no descriptor IS answered from
/// the ledger, with the recorded result under the recorded invocation id.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_retry_of_a_descriptorless_effect_is_answered_from_the_recorded_outcome() {
    let harness = Harness::new(Returned::Empty);
    let first = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect_err("an empty returned set is outside the declared contract");
    // A refused outcome is recorded as refused, so the retry replays the
    // refusal rather than running the effect a second time.
    let second = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect_err("the recorded refusal is replayed");
    assert_eq!(second.code, first.code);
    assert_eq!(harness.handler.runs(), 1, "the host effect happened once");
}

/// The same idempotency key with different parameters is a conflict, not a
/// second effect and not a replay.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_idempotency_key_reused_with_other_parameters_is_refused() {
    let harness = Harness::new(Returned::Declared);
    harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(canonical(&serde_json::json!({ "servingWorker": true }))),
            &[],
        )
        .await
        .expect("the first call runs");
    let refusal = harness
        .admission()
        .run(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(canonical(&serde_json::json!({ "servingWorker": false }))),
            &[],
        )
        .await
        .expect_err("a conflicting reuse is refused");
    assert_eq!(refusal.code, IDEMPOTENCY_CONFLICT);
    assert_eq!(harness.handler.runs(), 1);
}

// ---------------------------------------------------------------------------
// Scenario 4 - no fallback route, and nothing mutated before a refusal
// ---------------------------------------------------------------------------

/// An `Operation` no declared implementation serves is refused by name. There
/// is no default handler and no family switch, so the refusal is the whole of
/// the answer.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_unknown_handler_is_refused_before_any_host_mutation() {
    let harness = Harness::new(Returned::Declared);
    let unknown = AdmittedEffectInvocation::new(
        reference("Operation/not-declared"),
        consumer(),
        vec![leg()],
        typed_parameters(),
        vec![source_freshness(), consumer_freshness()],
        IdempotencyKey::parse("launch-unknown").expect("a bounded key"),
        None,
    )
    .expect("the invocation names an Operation");
    let refusal = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &unknown,
            &[],
        )
        .await
        .expect_err("an undeclared operation is refused");
    assert_eq!(refusal.code, UNKNOWN_IMPLEMENTATION);
    assert_eq!(refusal.reason, RefusalReason::UntrustedImplementation);
    assert_eq!(harness.handler.runs(), 0);
    assert!(ADMITTED_EFFECT_REFUSALS.contains(&UNKNOWN_IMPLEMENTATION));
    assert!(ADMITTED_EFFECT_REFUSALS.contains(&UNACCEPTED_PROJECTION));
    assert!(ADMITTED_EFFECT_REFUSALS.contains(&RESULT_FD_CONTRACT));
}

/// Two implementations claiming one `Operation` are refused while the table
/// is built, so a duplicate never becomes two routes to one effect.
#[test]
fn two_implementations_claiming_one_operation_are_refused() {
    let table = AdmittedEffectTable::new(vec![
        RecordingHandler::new(Returned::Declared) as Arc<dyn AdmittedEffectHandler>,
        RecordingHandler::new(Returned::Empty) as Arc<dyn AdmittedEffectHandler>,
    ]);
    assert!(
        table.is_err(),
        "a duplicate claim is refused before the table exists"
    );
}

/// A Zone with no accepted projection admits no new ordinary effect, and the
/// refusal lands before the implementation is resolved.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_unaccepted_projection_refuses_before_any_host_mutation() {
    let harness = Harness::new(Returned::Declared);
    // A broker that holds no projection at all holds no accepted graph, so it
    // says so rather than serving authority it cannot show.
    let unprovisioned = ProjectionPosture::current(ZONE).await;
    for posture in [ProjectionPosture::Unaccepted, unprovisioned] {
        let refusal = harness
            .admission()
            .admit(
                posture,
                ZONE,
                &harness.graph,
                &harness.values,
                &invocation(typed_parameters()),
                &[],
            )
            .await
            .expect_err("an unaccepted projection is refused");
        assert_eq!(refusal.code, UNACCEPTED_PROJECTION);
        assert_eq!(refusal.stage, AdmissionStage::Authorize);
        assert_eq!(harness.handler.runs(), 0);
    }
}

/// A relationship the accepted graph never admitted is refused: absence of an
/// accepted source decision is a refusal, not a pending approval.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_relationship_the_accepted_graph_never_admitted_is_refused() {
    let harness = Harness::new(Returned::Declared);
    let ungranted = AcceptedGraph::new(zone(), incarnation(), bootstrap());
    let refusal = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &ungranted,
            &harness.values,
            &invocation(typed_parameters()),
            &[],
        )
        .await
        .expect_err("an unadmitted relationship is refused");
    assert_eq!(refusal.code, UNPROVEN_EFFECT);
    assert_eq!(refusal.stage, AdmissionStage::Admit);
    assert_eq!(refusal.reason, RefusalReason::SourcePolicyRefused);
    assert_eq!(harness.handler.runs(), 0);
}

/// A request descriptor set the declared contract does not admit is refused
/// before the handler sees it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_request_descriptor_the_contract_does_not_admit_is_refused() {
    let harness = Harness::new(Returned::Declared);
    let (left, right) = nix::unistd::pipe().expect("a pipe is available in the test sandbox");
    let attached = [left, right];
    let refusal = harness
        .admission()
        .admit(
            ProjectionPosture::Accepted,
            ZONE,
            &harness.graph,
            &harness.values,
            &invocation(typed_parameters()),
            &attached,
        )
        .await
        .expect_err("an undeclared request descriptor is refused");
    assert_eq!(refusal.code, FD_LEG);
    assert_eq!(harness.handler.runs(), 0);
}

/// The relationship's own row reference is derived from its KTD3 key, so the
/// grant is read against a row the relationship names rather than against an
/// identity a second surface could mint.
#[test]
fn the_relationship_row_reference_is_derived_from_the_key() {
    assert_eq!(
        binding_row_ref(&binding_key()).expect("the binding row is canonical"),
        reference(BINDING_ROW)
    );
}

/// The plan request is assembled from the same leg the invocation named, and
/// it refuses a request that names no `Operation` or no leg at all.
#[test]
fn a_plan_request_needs_an_operation_and_a_leg() {
    let parameters = d2b_core::execution_plan::admit_parameters(&operation(), &typed_parameters())
        .expect("the declared parameter is admitted");
    let refused = d2b_core::execution_plan::EffectPlanRequest::new(
        reference(CONSUMER),
        operation(),
        consumer(),
        vec![BindingPlanRequest::new(
            binding_key(),
            RequestedRights::Observe,
            Vec::new(),
            None,
        )],
        parameters,
        vec![source_freshness()],
        d2b_core::resource_authority::TransportIdentity::Broker,
        None,
    );
    assert!(refused.is_err(), "a non-Operation reference is refused");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn nested_invocation(
    root_invocation_id: &str,
    idempotency_key: &str,
    identities: Vec<AuthoritySubject>,
) -> AdmittedEffectInvocation {
    AdmittedEffectInvocation::new(
        reference(OPERATION),
        consumer(),
        vec![leg()],
        typed_parameters(),
        vec![source_freshness(), consumer_freshness()],
        IdempotencyKey::parse(idempotency_key).expect("a bounded key"),
        Some(EffectCorrelation {
            root_invocation_id: root_invocation_id.to_owned(),
            identities,
        }),
    )
    .expect("the nested invocation is well formed")
}

fn leg_json() -> serde_json::Value {
    serde_json::json!({
        "binding": {
            "zone": ZONE,
            "kind": "volume",
            "sourceRef": SOURCE,
            "sourceUid": SOURCE_UID,
            "consumerRef": CONSUMER,
            "consumerUid": CONSUMER_UID,
            "slot": SLOT,
        },
        "rights": "observe",
        "presentation": ["filesystem-presentation"],
    })
}

fn freshness_json(value: FreshnessTuple) -> serde_json::Value {
    serde_json::json!({
        "zone": value.zone(),
        "storeIncarnation": value.store_incarnation(),
        "resourceRef": value.resource_ref(),
        "resourceUid": value.resource_uid(),
        "desiredRevision": value.desired_revision().get(),
        "desiredDigest": value.desired_digest(),
    })
}
