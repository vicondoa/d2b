//! Contract coverage for the confinement, syscall-filter, and callable
//! contracts introduced with the unified resource graph.
//!
//! The suite walks the ExecutionPolicy compatibility table row by row. Every
//! row is a composition rule with its own refusal, and a row that silently
//! intersected, dropped a requirement, or accepted a broader value would
//! produce a launch that looks admitted and is not confined - which is the
//! failure this contract exists to remove.

use d2b_contracts_resource::v3::{
    ALL_CONFINEMENT_FACETS, AdmittedExecution, AdmissionStage, AuditJoin, AuditMode,
    BackendSupport, BudgetCeiling, BudgetRequest, BoundedText, BoundedToken, BrokerRequirement,
    CallableOperation, CapabilityClass, ConfinementFacet, ExecutionInstance, ExecutionInstanceKind,
    ExecutionPolicyFingerprint, ExecutionPolicySpec, ExecutionRequirements, NamespaceClass,
    OperationAuthority, OperationBounds,
    OperationContractError, OperationDomain, OperationFds, OperationImplementation, OperationSurface,
    PayloadProvenance, PolicyAuthorization, PolicyCapabilities, PolicyIdentity, PolicyNamespaces,
    PolicyRefusal, PolicyRoot, PolicySeccomp, RefusalReason, SecretAccess, SeccompProfileSpec,
    SyscallDefaultAction, SyscallFilter, admit_execution,
};
use d2b_contracts_resource::v3::payload_schema::PayloadSchema;

const USER: &str = "User/alice";
const OTHER_USER: &str = "User/bob";
const PROFILE: &str = "SeccompProfile/desktop";
const OTHER_PROFILE: &str = "SeccompProfile/other";
const PROVIDER: &str = "Provider/volume-virtiofs";

fn user(reference: &str) -> d2b_contracts_resource::v3::ResourceRef {
    d2b_contracts_resource::v3::ResourceRef::parse(reference).expect("registered user reference")
}

fn profile(reference: &str) -> d2b_contracts_resource::v3::ResourceRef {
    d2b_contracts_resource::v3::ResourceRef::parse(reference).expect("registered profile reference")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn spec(
    namespaces: Vec<NamespaceClass>,
    capabilities: Vec<CapabilityClass>,
    identity: Option<d2b_contracts_resource::v3::ResourceRef>,
    no_new_privileges: bool,
    seccomp: Option<d2b_contracts_resource::v3::ResourceRef>,
) -> ExecutionPolicySpec {
    ExecutionPolicySpec::new(
        PolicyNamespaces::new(namespaces).expect("namespaces"),
        PolicyCapabilities::new(capabilities).expect("capabilities"),
        no_new_privileges,
        PolicyIdentity::new(identity, false).expect("identity"),
        PolicyRoot::new(true, true),
        PolicySeccomp::new(seccomp).expect("seccomp"),
        Some(0o077),
    )
    .expect("policy")
}

fn instance(kind: ExecutionInstanceKind, identity: Option<&str>, budget: BudgetRequest) -> ExecutionInstance {
    ExecutionInstance::new(
        kind,
        identity.map(user),
        budget,
    )
    .expect("instance")
}

fn requirements(
    namespaces: Vec<NamespaceClass>,
    capabilities: Vec<CapabilityClass>,
    mandatory_no_new_privileges: bool,
    seccomp: Option<d2b_contracts_resource::v3::ResourceRef>,
    facets: Vec<ConfinementFacet>,
) -> ExecutionRequirements {
    ExecutionRequirements::new(namespaces, capabilities, mandatory_no_new_privileges, seccomp, facets)
        .expect("requirements")
}

fn support() -> BackendSupport {
    BackendSupport::new(ALL_CONFINEMENT_FACETS.to_vec()).expect("support set")
}

fn ceiling() -> BudgetCeiling {
    BudgetCeiling::new(1_000, 4 * 1024 * 1024, 64, 128)
}

fn budget() -> BudgetRequest {
    BudgetRequest::new(500, 1024 * 1024, 32, 64).expect("budget")
}

fn admit(
    instance: &ExecutionInstance,
    requirements: &ExecutionRequirements,
    policy: &ExecutionPolicySpec,
    authorized: bool,
    support: &BackendSupport,
) -> Result<AdmittedExecution, PolicyRefusal> {
    admit_execution(
        instance,
        requirements,
        policy,
        &if authorized {
            PolicyAuthorization::granted()
        } else {
            PolicyAuthorization::absent()
        },
        support,
        &ceiling(),
    )
}

/// Row 1: required namespaces compose toward the stricter requirement, and a
/// required class the policy does not admit is refused rather than dropped.
#[test]
fn required_namespaces_compose_or_refuse() {
    let policy = spec(vec![NamespaceClass::User, NamespaceClass::Mount], Vec::new(), None, false, None);
    let support = support();

    let admitted = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(vec![NamespaceClass::User], Vec::new(), false, None, Vec::new()),
        &policy,
        true,
        &support,
    )
    .expect("the required class is inside the policy");
    assert_eq!(
        admitted.namespace_classes(),
        &[NamespaceClass::User, NamespaceClass::Mount],
        "composition takes the union, never the weaker set"
    );

    let refusal = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(vec![NamespaceClass::Network], Vec::new(), false, None, Vec::new()),
        &policy,
        true,
        &support,
    )
    .expect_err("a class outside the policy must be refused");
    assert_eq!(refusal.reason(), RefusalReason::RequiredNamespaceNotAdmitted);
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
}

/// Row 2: the required capability set must fit inside the policy ceiling, and
/// the admitted set is the requested subset rather than the whole ceiling.
#[test]
fn capabilities_must_fit_inside_the_ceiling() {
    let policy = spec(
        vec![NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind, CapabilityClass::SysTime],
        None,
        false,
        None,
    );
    let support = support();

    let refusal = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(
            vec![NamespaceClass::Mount],
            vec![CapabilityClass::NetworkAdmin],
            false,
            None,
            Vec::new(),
        ),
        &policy,
        true,
        &support,
    )
    .expect_err("a capability outside the ceiling must be refused");
    assert_eq!(
        refusal.reason(),
        RefusalReason::RequiredCapabilityOutsideCeiling
    );

    let admitted = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(
            vec![NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            false,
            None,
            Vec::new(),
        ),
        &policy,
        true,
        &support,
    )
    .expect("the required capability is inside the ceiling");
    assert_eq!(admitted.capability_classes(), &[CapabilityClass::NetworkBind]);
}

/// Row 3: an implementation that requires a mandatory restriction is refused
/// when the policy does not provide it, and instance input can never weaken
/// one the policy does provide.
#[test]
fn mandatory_restrictions_are_never_weakened() {
    let permissive = spec(
        vec![NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind],
        None,
        false,
        None,
    );
    let strict = spec(
        vec![NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind],
        None,
        true,
        None,
    );
    let support = support();
    let needs_restriction = requirements(
        vec![NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind],
        true,
        None,
        Vec::new(),
    );
    let plain = requirements(
        vec![NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind],
        false,
        None,
        Vec::new(),
    );

    assert_eq!(
        admit(
            &instance(ExecutionInstanceKind::LongRunning, None, budget()),
            &needs_restriction,
            &permissive,
            true,
            &support,
        )
        .expect_err("a policy that omits the required restriction is refused")
        .reason(),
        RefusalReason::RestrictionWeakened
    );
    assert!(
        admit(
            &instance(ExecutionInstanceKind::LongRunning, None, budget()),
            &plain,
            &strict,
            true,
            &support,
        )
        .expect("admitted")
        .no_new_privileges(),
        "instance input cannot relax a policy restriction"
    );
}

/// Row 4: identity resolves only through the authorized rule. There is no
/// numerical caller override to compare against and no default to fall back
/// to when the selection is unauthorized.
#[test]
fn identity_resolves_only_through_the_authorized_rule() {
    let bound = spec(
        vec![NamespaceClass::Mount],
        Vec::new(),
        Some(user(USER)),
        false,
        None,
    );
    let support = support();
    let plain = requirements(vec![NamespaceClass::Mount], Vec::new(), false, None, Vec::new());

    let admitted = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &plain,
        &bound,
        true,
        &support,
    )
    .expect("admitted");
    assert_eq!(admitted.user_ref(), Some(&user(USER)));

    let refusal = admit(
        &instance(ExecutionInstanceKind::LongRunning, Some(OTHER_USER), budget()),
        &plain,
        &bound,
        true,
        &support,
    )
    .expect_err("a different identity is refused");
    assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);

    let unbound = spec(vec![NamespaceClass::Mount], Vec::new(), None, false, None);
    assert_eq!(
        admit(
            &instance(ExecutionInstanceKind::LongRunning, Some(USER), budget()),
            &plain,
            &unbound,
            true,
            &support,
        )
        .expect_err("a policy that authorizes no identity admits no selection")
        .reason(),
        RefusalReason::IdentityNotAuthorized
    );
}

/// Row 5: mandatory root restrictions are enforced on the target, and a
/// target that cannot realize one is refused before any host effect.
#[test]
fn a_target_that_cannot_enforce_a_mandatory_facet_is_refused() {
    let policy = spec(
        vec![NamespaceClass::Mount],
        Vec::new(),
        None,
        true,
        None,
    );
    let partial = BackendSupport::new(vec![ConfinementFacet::MountNamespace, ConfinementFacet::ReadOnlyRoot])
        .expect("partial support");
    let refusal = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(vec![NamespaceClass::Mount], Vec::new(), false, None, Vec::new()),
        &policy,
        true,
        &partial,
    )
    .expect_err("an unenforceable mandatory facet is refused");
    assert_eq!(refusal.reason(), RefusalReason::MandatoryFacetUnsupported);
}

/// Row 6: the syscall filter must be one the implementation declared, and the
/// profile reference must be a profile rather than some other resource.
#[test]
fn an_incompatible_syscall_filter_is_refused() {
    let policy = spec(
        vec![NamespaceClass::Mount],
        Vec::new(),
        None,
        false,
        Some(profile(PROFILE)),
    );
    let support = BackendSupport::new(
        [
            ConfinementFacet::MountNamespace,
            ConfinementFacet::ReadOnlyRoot,
            ConfinementFacet::PrivateRoot,
            ConfinementFacet::SyscallFilter,
        ]
        .to_vec(),
    )
    .expect("support");

    let refusal = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(
            vec![NamespaceClass::Mount],
            Vec::new(),
            false,
            Some(profile(OTHER_PROFILE)),
            Vec::new(),
        ),
        &policy,
        true,
        &support,
    )
    .expect_err("a different filter is refused");
    assert_eq!(refusal.reason(), RefusalReason::SeccompIncompatible);

    let admitted = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(
            vec![NamespaceClass::Mount],
            Vec::new(),
            false,
            Some(profile(PROFILE)),
            Vec::new(),
        ),
        &policy,
        true,
        &support,
    )
    .expect("the declared filter is admitted");
    assert_eq!(admitted.seccomp_profile_ref(), Some(&profile(PROFILE)));
}

/// Row 7: the admitted limits come from the budget and Quota contracts, and a
/// request over the ceiling is refused rather than clamped.
#[test]
fn a_request_over_the_admitted_ceiling_is_refused() {
    let policy = spec(vec![NamespaceClass::Mount], Vec::new(), None, false, None);
    let refusal = admit(
        &instance(
            ExecutionInstanceKind::LongRunning,
            None,
            BudgetRequest::new(2_000, 1, 1, 1).expect("budget"),
        ),
        &requirements(vec![NamespaceClass::Mount], Vec::new(), false, None, Vec::new()),
        &policy,
        true,
        &support(),
    )
    .expect_err("an over-ceiling request is refused");
    assert_eq!(refusal.reason(), RefusalReason::LimitExceedsCeiling);
}

/// Row 8: selecting a policy is a request. Without authorization evidence it
/// is refused even when every other row would admit the instance.
#[test]
fn an_unauthorized_policy_selection_is_refused() {
    let policy = spec(vec![NamespaceClass::Mount], Vec::new(), None, false, None);
    let refusal = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(vec![NamespaceClass::Mount], Vec::new(), false, None, Vec::new()),
        &policy,
        false,
        &support(),
    )
    .expect_err("a policy reference is not a grant");
    assert_eq!(refusal.reason(), RefusalReason::PolicySelectionNotAuthorized);
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
}

/// A run-to-completion instance and a long-running one are the same contract
/// with the same outcome.
#[test]
fn both_execution_lifetimes_take_one_policy_path() {
    let policy = spec(
        vec![NamespaceClass::User, NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind],
        Some(user(USER)),
        true,
        Some(profile(PROFILE)),
    );
    let support = BackendSupport::new(ALL_CONFINEMENT_FACETS.to_vec()).expect("support");
    let requirements = requirements(
        vec![NamespaceClass::User, NamespaceClass::Mount],
        vec![CapabilityClass::NetworkBind],
        false,
        Some(profile(PROFILE)),
        Vec::new(),
    );

    let long_running = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements,
        &policy,
        true,
        &support,
    )
    .expect("admitted");
    let one_shot = admit(
        &instance(ExecutionInstanceKind::OneShot, None, budget()),
        &requirements,
        &policy,
        true,
        &support,
    )
    .expect("admitted");

    assert_eq!(long_running.kind(), ExecutionInstanceKind::LongRunning);
    assert_eq!(one_shot.kind(), ExecutionInstanceKind::OneShot);
    assert_eq!(long_running.namespace_classes(), one_shot.namespace_classes());
    assert_eq!(long_running.capability_classes(), one_shot.capability_classes());
    assert_eq!(long_running.no_new_privileges(), one_shot.no_new_privileges());
    assert_eq!(long_running.user_ref(), one_shot.user_ref());
    assert_eq!(long_running.umask(), one_shot.umask());
    assert_eq!(long_running.policy(), one_shot.policy());
}

/// An admitted execution records the digest of the policy it was admitted
/// under, so a later policy edit invalidates the admission even when nothing
/// else in the graph moved.
#[test]
fn an_admitted_execution_is_fenced_against_its_policy_digest() {
    let policy = spec(vec![NamespaceClass::Mount], Vec::new(), None, true, None);
    let relaxed = spec(vec![NamespaceClass::Mount], Vec::new(), None, false, None);
    let admitted = admit(
        &instance(ExecutionInstanceKind::LongRunning, None, budget()),
        &requirements(vec![NamespaceClass::Mount], Vec::new(), false, None, Vec::new()),
        &policy,
        true,
        &support(),
    )
    .expect("admitted");
    assert_eq!(admitted.policy().as_str(), ExecutionPolicyFingerprint::from_spec(&policy).as_str());
    assert_ne!(admitted.policy().as_str(), ExecutionPolicyFingerprint::from_spec(&relaxed).as_str());
}

/// The old flattened `Host`/`Guest` fragment and the new resource are
/// different contracts with different meaning; a round trip of one is never
/// mistaken for a round trip of the other.
#[test]
fn the_execution_parent_fragment_and_the_policy_resource_stay_distinct() {
    use d2b_contracts_resource::v3::ExecutionDomain;
    use d2b_contracts_resource::v3::execution_policy::{BudgetSpec, NetworkAttachment};

    let parent = d2b_contracts_resource::v3::ExecutionPolicy::new(
        ExecutionDomain::System,
        vec![ExecutionDomain::System],
        None,
        BudgetSpec::default(),
        vec![NetworkAttachment::new(
            d2b_contracts_resource::v3::ResourceRef::parse("Network/lan").expect("network"),
            true,
        )
        .expect("attachment")],
        Vec::new(),
        Vec::new(),
    )
    .expect("execution-parent facts");

    let policy = spec(vec![NamespaceClass::Network], Vec::new(), None, true, None);
    let parent_bytes =
        d2b_contracts_resource::v3::to_base_object(&parent).expect("parent object");
    let policy_bytes =
        d2b_contracts_resource::v3::to_base_object(&policy).expect("policy object");

    assert!(parent_bytes.get("networkAttachments").is_some());
    assert!(policy_bytes.get("networkAttachments").is_none());
    assert!(policy_bytes.get("namespaces").is_some());
    assert_eq!(
        serde_json::from_str::<ExecutionPolicySpec>(&serde_json::to_string(&policy_bytes).expect("policy json")).expect("policy row"),
        policy
    );
    assert!(
        serde_json::from_str::<ExecutionPolicySpec>(&serde_json::to_string(&parent_bytes).expect("parent json")).is_err(),
        "an execution-parent fragment is not a policy row"
    );
}

/// The profile carries syscall policy only, and a row that still carries the
/// old namespace, cgroup, or device authority is refused.
#[test]
fn a_profile_row_carries_syscall_policy_only() {
    let declared = SeccompProfileSpec::new(
        SyscallFilter::new(
            SyscallDefaultAction::Errno,
            vec![token("read"), token("write")],
        )
        .expect("filter"),
    );
    let bytes = d2b_contracts_resource::v3::canonical_json_bytes(&declared)
        .expect("canonical bytes");
    assert_eq!(
        serde_json::from_slice::<SeccompProfileSpec>(&bytes).expect("decode"),
        declared
    );
    for retired in [
        r#"{"filter":{"defaultAction":"errno","allowed":["read"]},"namespaces":{"mount":true}}"#,
        r#"{"filter":{"defaultAction":"errno","allowed":["read"]},"cgroups":{}}"#,
        r#"{"filter":{"defaultAction":"errno","allowed":["read"]},"deviceBinds":[]}"#,
    ] {
        assert!(serde_json::from_str::<SeccompProfileSpec>(retired).is_err());
    }
}

/// An operation binds one declared provider implementation, and a retired
/// command reference cannot express one.
#[test]
fn an_operation_binds_a_declared_implementation() {
    use serde_json::json;

    let payload = PayloadSchema::parse(json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["target"],
        "properties": { "target": { "type": "string" } }
    }))
    .expect("payload");
    let audit = d2b_contracts_resource::v3::OperationAudit::new(
        true,
        AuditMode::Yes,
        vec![BoundedText::parse("target").expect("field")],
        Vec::new(),
        token("opaque-target"),
    )
    .expect("audit");
    let authority = OperationAuthority::new(
        OperationSurface::Broker,
        OperationDomain::Host,
        BoundedText::parse("host-operator").expect("authority"),
        BrokerRequirement::Yes,
    );

    let declared = CallableOperation::new(
        OperationImplementation::provider_method(
            d2b_contracts_resource::v3::ResourceRef::parse(PROVIDER).expect("provider"),
            token("controller"),
            token("serve-view"),
        )
        .expect("declared implementation"),
        payload.clone(),
        None,
        false,
        SecretAccess::None,
        audit,
        Some(AuditJoin::new(vec![BoundedText::parse("target").expect("field")]).expect("join")),
        authority.clone(),
        OperationFds::default(),
        OperationBounds::default(),
        PayloadProvenance::Request,
    )
    .expect("operation");

    let bytes = d2b_contracts_resource::v3::canonical_json_bytes(&declared)
        .expect("canonical bytes");
    assert_eq!(
        serde_json::from_slice::<CallableOperation>(&bytes).expect("decode"),
        declared
    );

    assert_eq!(
        OperationImplementation::provider_method(
            d2b_contracts_resource::v3::ResourceRef::parse("Command/worker").expect("command"),
            token("controller"),
            token("serve-view"),
        ),
        Err(OperationContractError::UntrustedImplementation)
    );
    assert_eq!(
        OperationImplementation::trusted_executable_template(
            d2b_contracts_resource::v3::ResourceRef::parse("Process/web").expect("process"),
            token("shell"),
        ),
        Err(OperationContractError::UntrustedImplementation)
    );
}
