//! The policy resources as executable authority (U39; R8, R25-R33).
//!
//! Three things are proven here, and each one is a property of the canonical
//! contracts plus the one shared evaluator, not of a test-local imitation of
//! either:
//!
//! 1. An `Operation` row's implementation and the execution-policy selection
//!    behind it resolve through the prior accepted graph. A row's syntactic
//!    validity is not permission: the policy is admitted only for a selection
//!    the accepted `Role`/`RoleBinding` rows grant, and a backend that cannot
//!    enforce one of the policy's own mandatory facets is refused instead of
//!    launched without it.
//! 2. A candidate `RoleBinding` cannot authorize its own creation, because a
//!    candidate that would introduce its own grant is absent from the prior
//!    state the decision reads.
//! 3. The retired `RolePosture` and the broad `SeccompProfile` facets fail
//!    new-contract decoding. They are refused, not decoded with their
//!    authority quietly dropped: a row carrying a mount, a namespace set, a
//!    cgroup set, or a device-node bind never becomes a policy row at all.
//!
//! The foundation seed publishes posture and Command-materialized rows no
//! longer; the retired Role facets are asserted to refuse decoding rather
//! than to be silently dropped.

use d2b_contracts_resource::v3::{
    ALL_CONFINEMENT_FACETS, AdmissionStage, AdmissionDecision, AuthoritySubject,
    AuthoritySubjectKind, AuditJoin, AuditMode, BackendSupport, BudgetCeiling, BudgetRequest,
    CallableOperation, CanonicalJsonObject, CapabilityClass, ConfinementFacet, ExecutionInstance,
    ExecutionInstanceKind, ExecutionPolicySpec, ExecutionRequirements, NamespaceClass,
    OperationAudit, OperationAuthority, OperationBounds, OperationContractError,
    OperationDomain, OperationFds, OperationImplementation, OperationSurface, PayloadProvenance,
    PolicyAuthorization, PolicyCapabilities, PolicyIdentity, PolicyNamespaces, PolicyRoot,
    PolicySeccomp, RefusalReason, ResourceRef, ResourceTypeName, SecretAccess, StoreIncarnation,
    SyscallDefaultAction, SyscallFilter, ZoneId, admit_execution, canonical_json_bytes,
    seccomp_profile::SeccompProfileSpec,
};
use d2b_contracts_resource::v3::execution_policy::{BoundedText, BoundedToken};
use d2b_contracts_resource::v3::PayloadSchema;
use d2b_contracts_zone_session::v3::role::AuthorizedRole;
use d2b_contracts_zone_session::v3::{RoleBindingSpec, RoleResourceVerb, RoleRule};
use d2b_core::resource_authority::{
    AcceptedGraph, GraphAuthority, GraphMutation, MutationKind, MutationSubjectEvidence,
    ProjectionRow, TransportIdentity,
};
use serde_json::json;

const ZONE: &str = "policy-authority";
const STORE: &str = "store-generation-1";
const OPERATION: &str = "Operation/admit-execution";
const ROLE: &str = "Role/operation-caller";
const BINDING: &str = "RoleBinding/operators";

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("the fixture zone is canonical")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture reference is canonical")
}

fn store() -> StoreIncarnation {
    StoreIncarnation::parse(STORE).expect("the fixture incarnation is a bounded token")
}

fn bootstrap() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

fn operator() -> AuthoritySubject {
    AuthoritySubject::named(AuthoritySubjectKind::User, reference("User/operator"))
}

/// The canonical admitted bytes of one row, as the broker's projection stores
/// them.
fn canonical(value: &impl serde::Serialize) -> CanonicalJsonObject {
    CanonicalJsonObject::parse(&serde_json::to_vec(value).expect("the fixture row serializes"))
        .expect("the fixture row is a canonical JSON object")
}

fn rule(resource_type: &str, verbs: Vec<RoleResourceVerb>) -> RoleRule {
    RoleRule::new(
        vec![ResourceTypeName::parse(resource_type).expect("a registered resource type")],
        verbs,
        Vec::new(),
        Vec::new(),
        vec![zone()],
        Vec::new(),
        Vec::new(),
    )
    .expect("the role rule validates")
}

/// The accepted `Role` that lets the operator call exactly one operation.
fn calling_role() -> AuthorizedRole {
    AuthorizedRole::new(
        vec![rule("Operation", vec![RoleResourceVerb::Create])],
        vec![reference(OPERATION)],
    )
    .expect("the authorization-only role validates")
}

/// The accepted `RoleBinding` that carries that grant to the operator.
fn calling_binding() -> RoleBindingSpec {
    RoleBindingSpec::new(
        reference(ROLE),
        vec![reference("User/operator")],
        None,
        None,
    )
    .expect("the role binding validates")
}

/// The prior accepted graph, read from the canonical bytes a projection would
/// hold, so the decision is made against committed rows rather than against
/// values this test assembled by hand.
fn accepted_graph(role: AuthorizedRole, binding: RoleBindingSpec) -> AcceptedGraph {
    let role_bytes = canonical(&role);
    let binding_bytes = canonical(&binding);
    AcceptedGraph::from_canonical_rows(
        zone(),
        store(),
        bootstrap(),
        [
            ProjectionRow::new(&reference(ROLE), &role_bytes),
            ProjectionRow::new(&reference(BINDING), &binding_bytes),
        ],
    )
    .expect("the projection rows are the graph's authority rows")
}

/// The prior accepted graph with no accepted `RoleBinding` at all: the
/// candidate row exists only as the request under evaluation.
fn graph_without_any_binding(role: AuthorizedRole) -> AcceptedGraph {
    AcceptedGraph::new(zone(), store(), bootstrap()).with_role(reference(ROLE), role)
}

/// The authorization evidence one policy selection carries, derived from the
/// prior accepted graph: a selection is authorized exactly when an accepted
/// binding names the subject and an accepted `Role` names the operation that
/// selects the policy.
fn policy_authorization(
    accepted: &AcceptedGraph,
    subject: &AuthoritySubject,
    selecting_operation: &ResourceRef,
) -> PolicyAuthorization {
    let Some(initiating) = subject.resource_ref() else {
        return PolicyAuthorization::absent();
    };
    let granted = accepted.role_bindings().any(|(_, binding)| {
        binding.subjects().iter().any(|named| named == initiating)
            && accepted
                .role(binding.role_ref())
                .is_some_and(|role| role.operation_refs().contains(selecting_operation))
    });
    if granted {
        PolicyAuthorization::granted()
    } else {
        PolicyAuthorization::absent()
    }
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn text(value: &str) -> BoundedText {
    BoundedText::parse(value).expect("bounded text")
}

fn payload() -> PayloadSchema {
    PayloadSchema::parse(json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["instance"],
        "properties": {
            "instance": { "type": "string" },
            "token": { "type": "string", "writeOnly": true }
        }
    }))
    .expect("the payload schema validates")
}

fn audit() -> OperationAudit {
    OperationAudit::new(
        true,
        AuditMode::Yes,
        vec![],
        vec![text("token")],
        token("opaque-target"),
    )
    .expect("the audit facet validates")
}

fn authority() -> OperationAuthority {
    OperationAuthority::new(
        OperationSurface::Broker,
        OperationDomain::Host,
        text("host-operator"),
        d2b_contracts_resource::v3::BrokerRequirement::Yes,
    )
}

/// The committed `Operation` row: the payload contract plus the trusted
/// implementation that answers it.
fn operation_row() -> Vec<u8> {
    let operation = CallableOperation::new(
        OperationImplementation::provider_method(
            reference("Provider/system-minijail"),
            token("execution-policy"),
            token("admit"),
        )
        .expect("a declared provider method"),
        payload(),
        None,
        true,
        SecretAccess::ReadWrite,
        audit(),
        Some(AuditJoin::new(vec![text("instance")]).expect("the audit join validates")),
        authority(),
        OperationFds::default(),
        OperationBounds::default(),
        PayloadProvenance::Request,
    )
    .expect("the committed operation validates");
    canonical_json_bytes(&operation).expect("the row renders canonically")
}

/// The committed `ExecutionPolicy` row: reusable confinement, and no host
/// path, mount, device node, or resource reference of its own beyond the
/// identity and syscall-filter rows it selects.
fn policy_row() -> ExecutionPolicySpec {
    ExecutionPolicySpec::new(
        PolicyNamespaces::new(vec![NamespaceClass::User, NamespaceClass::Mount])
            .expect("the namespace set validates"),
        PolicyCapabilities::new(vec![CapabilityClass::NetworkBind])
            .expect("the capability ceiling validates"),
        true,
        PolicyIdentity::new(None, false).expect("the identity rules validate"),
        PolicyRoot::new(true, true),
        PolicySeccomp::new(Some(reference("SeccompProfile/desktop")))
            .expect("the syscall-filter selection validates"),
        Some(0o077),
    )
    .expect("the policy spec validates")
}

fn instance() -> ExecutionInstance {
    ExecutionInstance::new(
        ExecutionInstanceKind::LongRunning,
        None,
        BudgetRequest::new(500, 1024 * 1024, 32, 64).expect("the budget request validates"),
    )
    .expect("the execution instance validates")
}

fn requirements() -> ExecutionRequirements {
    ExecutionRequirements::new(
        vec![NamespaceClass::User, NamespaceClass::Mount],
        Vec::new(),
        true,
        Some(reference("SeccompProfile/desktop")),
        Vec::new(),
    )
    .expect("the execution requirements validate")
}

fn budget_ceiling() -> BudgetCeiling {
    BudgetCeiling::new(1_000, 4 * 1024 * 1024, 128, 256)
}

// ---------------------------------------------------------------------------
// 1. Operation implementation and ExecutionPolicy selection through the graph
// ---------------------------------------------------------------------------

/// The operation's implementation resolves to a declared provider method, and
/// the policy selection behind it is admitted only for the operation the
/// accepted graph grants. A subject the accepted graph does not grant, and a
/// backend that cannot enforce one of the policy's own mandatory facets, are
/// both refused.
#[test]
fn operation_implementation_and_policy_selection_resolve_through_the_admitted_graph() {
    let operation: CallableOperation =
        serde_json::from_slice(&operation_row()).expect("the committed operation decodes");
    let implementation = operation.implementation();
    assert!(implementation.is_provider_method());
    assert_eq!(
        implementation.provider().to_canonical_string(),
        "Provider/system-minijail",
        "an operation's implementation names the declaring provider"
    );
    assert_eq!(
        OperationImplementation::provider_method(
            reference(ROLE),
            token("execution-policy"),
            token("admit"),
        ),
        Err(OperationContractError::UntrustedImplementation),
        "executable declaration is provider-owned contract data, not a resource relationship"
    );

    let policy = policy_row();
    let instance = instance();
    let requirements = requirements();
    let ceiling = budget_ceiling();
    let support = BackendSupport::new(ALL_CONFINEMENT_FACETS.to_vec())
        .expect("the backend support set validates");
    let selecting = reference(OPERATION);

    let granted = accepted_graph(calling_role(), calling_binding());
    assert_eq!(
        policy_authorization(&granted, &operator(), &selecting),
        PolicyAuthorization::granted(),
        "the accepted Role names the operation that selects the policy"
    );
    let admitted = admit_execution(
        &instance,
        &requirements,
        &policy,
        &policy_authorization(&granted, &operator(), &selecting),
        &support,
        &ceiling,
    )
    .expect("an authorized selection under a fully capable backend is admitted");
    assert!(admitted.no_new_privileges());
    assert_eq!(admitted.seccomp_profile_ref(), Some(&reference("SeccompProfile/desktop")));
    assert_eq!(admitted.umask(), Some(0o077));

    // A subject the accepted graph does not grant reaches the same policy and
    // is refused at the authorizing stage, before any confinement facet is
    // compared: a syntactically valid row is not permission.
    let stranger = AuthoritySubject::named(AuthoritySubjectKind::User, reference("User/stranger"));
    let error = admit_execution(
        &instance,
        &requirements,
        &policy,
        &policy_authorization(&granted, &stranger, &selecting),
        &support,
        &ceiling,
    )
    .expect_err("an ungranted subject selects nothing");
    assert_eq!(error.stage(), AdmissionStage::Authorize);
    assert_eq!(error.reason(), RefusalReason::PolicySelectionNotAuthorized);

    // A role that exists but names a different operation grants this one
    // nothing: the grant is per operation, not per provider.
    let other_operation = AuthorizedRole::new(
        vec![rule("Operation", vec![RoleResourceVerb::Create])],
        vec![reference("Operation/start-volume")],
    )
    .expect("the role validates");
    let other = accepted_graph(other_operation, calling_binding());
    assert_eq!(
        policy_authorization(&other, &operator(), &selecting),
        PolicyAuthorization::absent(),
        "the accepted Role names a different operation"
    );

    // A backend missing one of the policy's own mandatory facets is refused
    // rather than launched without it.
    let mut facets = ALL_CONFINEMENT_FACETS.to_vec();
    facets.retain(|facet| *facet != ConfinementFacet::MountNamespace);
    let partial = BackendSupport::new(facets).expect("the backend support set validates");
    let error = admit_execution(
        &instance,
        &requirements,
        &policy,
        &PolicyAuthorization::granted(),
        &partial,
        &ceiling,
    )
    .expect_err("a backend without the mount namespace cannot run this policy");
    assert_eq!(error.stage(), AdmissionStage::Admit);
    assert_eq!(error.reason(), RefusalReason::MandatoryFacetUnsupported);
}

// ---------------------------------------------------------------------------
// 2. A candidate RoleBinding cannot authorize its own creation
// ---------------------------------------------------------------------------

/// The grant a candidate `RoleBinding` would introduce is exactly the authority
/// its own creation needs, and the creation is refused: the candidate is
/// absent from the prior accepted state the decision reads. Once that same
/// binding has been accepted, the identical mutation is admitted, which is
/// what makes the refusal about the prior state and nothing else.
#[test]
fn a_candidate_role_binding_cannot_authorize_its_own_creation() {
    let admin_role = AuthorizedRole::new(
        vec![rule(
            "RoleBinding",
            vec![RoleResourceVerb::Create, RoleResourceVerb::Delete],
        )],
        Vec::new(),
    )
    .expect("the role validates");
    let candidate = RoleBindingSpec::new(
        reference("Role/role-binding-admin"),
        vec![reference("User/operator")],
        None,
        None,
    )
    .expect("the candidate binding validates");

    let create = |target: &str, accepted: &AcceptedGraph| {
        GraphAuthority::admit_mutation(
            &GraphMutation::new(
                zone(),
                MutationSubjectEvidence::new(operator(), TransportIdentity::ComponentSession),
                MutationKind::Create,
                reference(target),
            ),
            accepted,
        )
    };

    let without = graph_without_any_binding(admin_role.clone());
    assert_eq!(
        create("RoleBinding/role-binding-admin", &without),
        AdmissionDecision::refuse(AdmissionStage::Authorize, RefusalReason::IdentityNotAuthorized),
        "a candidate binding is not in the prior state that would decide for it"
    );

    // The candidate that names the operator as its subject grants that
    // operator the very create it was refused, and still cannot be written.
    assert!(
        candidate.subjects().iter().any(|s| *s == reference("User/operator")),
        "the candidate is the grant that would authorize its own creation"
    );

    let with = AcceptedGraph::new(zone(), store(), bootstrap())
        .with_role(reference("Role/role-binding-admin"), admin_role)
        .with_role_binding(reference("RoleBinding/role-binding-admin"), candidate);
    assert_eq!(
        create("RoleBinding/role-binding-admin", &with),
        AdmissionDecision::Admitted,
        "the identical mutation is admitted once that binding is prior accepted state"
    );
}

// ---------------------------------------------------------------------------
// 3. The retired facets fail new-contract decoding
// ---------------------------------------------------------------------------

/// A row carrying a `RolePosture`, a mount grant, or a command reference does
/// not decode as the authorization-only `Role`: the independent execution and
/// access authority those facets carried is refused, not dropped.
#[test]
fn a_role_row_carrying_a_posture_or_a_command_reference_does_not_decode() {
    let role = calling_role();
    let bytes = canonical_json_bytes(&role).expect("the row renders canonically");
    serde_json::from_slice::<AuthorizedRole>(&bytes).expect("the canonical row decodes");

    let canonical: serde_json::Value =
        serde_json::from_slice(&bytes).expect("the canonical row parses as JSON");
    for (field, carried) in [
        (
            "posture",
            json!({
                "seccompRef": "SeccompProfile/desktop",
                "principalRef": "Principal/d2b-zonert",
                "capabilities": [],
                "namespaces": {},
                "mounts": [{ "path": "/var/lib/d2b", "writable": true }],
                "umask": null,
                "userNs": false
            }),
        ),
        ("commandRefs", json!(["Command/virtiofsd-worker"])),
        ("mounts", json!([{ "path": "/var/lib/d2b", "writable": true }])),
    ] {
        let mut retired = canonical.clone();
        retired
            .as_object_mut()
            .expect("the row is an object")
            .insert(field.to_owned(), carried);
        let error = serde_json::from_slice::<AuthorizedRole>(
            &serde_json::to_vec(&retired).expect("the row serializes"),
        )
        .expect_err("a retired Role facet must not decode");
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

/// A `SeccompProfile` row carrying a namespace set, a cgroup set, or a
/// device-node bind does not decode: a syscall filter is a syscall filter.
#[test]
fn a_seccomp_profile_row_carrying_access_authority_does_not_decode() {
    let profile = SeccompProfileSpec::new(
        SyscallFilter::new(
            SyscallDefaultAction::Errno,
            vec![token("read"), token("write")],
        )
        .expect("the syscall filter validates"),
    );
    let bytes = canonical_json_bytes(&profile).expect("the row renders canonically");
    assert_eq!(
        serde_json::from_slice::<SeccompProfileSpec>(&bytes).expect("the canonical row decodes"),
        profile
    );

    for (field, carried) in [
        ("namespaces", json!({ "mount": true, "pid": true })),
        ("cgroups", json!({ "cpu": 0.5 })),
        (
            "deviceBinds",
            json!([{ "path": "/dev/null", "kind": "char", "major": 1, "minor": 3, "access": "read-write" }]),
        ),
        ("mounts", json!([{ "path": "/var/lib/d2b", "writable": true }])),
    ] {
        let mut row: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the canonical row parses as JSON");
        row.as_object_mut()
            .expect("the row is an object")
            .insert(field.to_owned(), carried);
        let error = serde_json::from_slice::<SeccompProfileSpec>(
            &serde_json::to_vec(&row).expect("the row serializes"),
        )
        .expect_err("a retired SeccompProfile facet must not decode");
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

/// An `Operation` row carrying the retired command owner reference or an
/// inherited wire discriminant does not decode: an operation's implementation
/// is the provider-owned identity, and a materialized spawn row is gone.
#[test]
fn an_operation_row_carrying_a_command_owner_reference_does_not_decode() {
    let bytes = operation_row();
    serde_json::from_slice::<CallableOperation>(&bytes).expect("the canonical row decodes");

    for (field, carried) in [
        ("ownerRef", json!("Command/virtiofsd-worker")),
        ("wireTag", json!(7)),
        ("commandRefs", json!(["Command/virtiofsd-worker"])),
    ] {
        let mut row: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the canonical row parses as JSON");
        row.as_object_mut()
            .expect("the row is an object")
            .insert(field.to_owned(), carried);
        serde_json::from_slice::<CallableOperation>(&serde_json::to_vec(&row).expect("serializes"))
            .expect_err("a retired Operation facet must not decode");
    }
}
