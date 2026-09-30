//! The shared Process Provider conformance suite.
//!
//! Both `system-systemd` and `system-minijail` run this suite over the same
//! scripted effect port. Every assertion is a neutral obligation from
//! `ADR-046-components-processes-and-sandbox`; anything provider-specific
//! is read from the Provider's declared
//! [`ProcessProviderProfile`](crate::ProcessProviderProfile) rather than
//! branched on by name.

#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeSet;

#[cfg(any(test, feature = "test-support"))]
use d2b_contracts_resource::v3::ResourceRef;
#[cfg(any(test, feature = "test-support"))]
use d2b_contracts_resource::v3::execution_policy::ExecutionDomain;

use crate::error::ProcessConformanceError;
use crate::identity::WaitReapOwner;
#[cfg(any(test, feature = "test-support"))]
use crate::identity::IdentityBinding;
#[cfg(any(test, feature = "test-support"))]
use crate::provider::{AdoptionOutcome, ProcessProvider};
use crate::sandbox::{StopProof, validate_stop_proof};
#[cfg(any(test, feature = "test-support"))]
use crate::status::{AdoptionCondition, ProcessPhaseClass};
#[cfg(any(test, feature = "test-support"))]
use crate::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use crate::ticket::LaunchTicket;

/// Field or value fragments that must never appear in public status.
#[cfg(any(test, feature = "test-support"))]
const FORBIDDEN_STATUS_FRAGMENTS: [&str; 12] = [
    "pid",
    "pidfd",
    "unit",
    "invocation",
    "cgroup",
    "path",
    "argv",
    "command",
    "binary",
    "env",
    "uid",
    "gid",
];

/// Build the two execution fixtures every Provider must handle
/// identically: a physical Host and a VM Guest.
#[cfg(any(test, feature = "test-support"))]
fn execution_refs() -> [ResourceRef; 2] {
    [
        ResourceRef::parse("Host/host-system").expect("valid fixture ref"),
        ResourceRef::parse("Guest/dev-vm").expect("valid fixture ref"),
    ]
}

/// A launch on a Host and on a Guest produces identical conformant status.
///
/// The ResourceType and its status projection do not change with locality.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_launch_is_locality_neutral<P: ProcessProvider>(provider: &P, provider_name: &str) {
    let profile = provider.profile();
    let bindings: Vec<IdentityBinding> = profile
        .required_identity_bindings()
        .iter()
        .copied()
        .collect();
    for execution_ref in execution_refs() {
        let port_owner = profile.wait_reap_owner();
        let ticket = fixtures::ticket_builder()
            .execution_ref(execution_ref.clone())
            .selected_provider(provider_name)
            .expected_identity(bindings.clone())
            .build()
            .expect("conformant fixture ticket");
        let report = block_on(provider.launch(&ticket)).expect("launch succeeds");
        assert_eq!(report.provider.as_str(), provider_name);
        assert_eq!(report.wait_reap_owner, port_owner);
        assert_eq!(report.execution_ref, execution_ref);
        assert_eq!(report.domain, ticket.domain());
        assert_eq!(report.user_ref.as_ref(), ticket.user_ref());
        assert_eq!(report.digests, *ticket.digests());
        assert!(!report.identity.is_zero());
        assert_eq!(report.phase, ProcessPhaseClass::Running);
        assert_eq!(report.adoption, AdoptionCondition::NotApplicable);
        assert!(report.last_exit.is_none());
    }
}

/// A ticket selecting a different Process Provider is rejected.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_foreign_provider_selection_is_rejected<P: ProcessProvider>(provider: &P) {
    let bindings: Vec<IdentityBinding> = provider
        .profile()
        .required_identity_bindings()
        .iter()
        .copied()
        .collect();
    let ticket = fixtures::ticket_builder()
        .selected_provider("some-other-provider")
        .expected_identity(bindings)
        .build()
        .expect("conformant fixture ticket");
    assert_eq!(
        block_on(provider.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::ProviderMismatch
    );
}

/// Every domain outside the Provider's declared support set is rejected,
/// and a user-domain launch the Provider does support carries the exact
/// `userRef` through to status.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_domain_support_matches_the_profile<P: ProcessProvider>(
    provider: &P,
    provider_name: &str,
) {
    let profile = provider.profile();
    let bindings: Vec<IdentityBinding> = profile
        .required_identity_bindings()
        .iter()
        .copied()
        .collect();
    let supported = profile.supported_domains().clone();
    let user_ref = ResourceRef::parse("User/alice").expect("valid fixture ref");

    for domain in [ExecutionDomain::System, ExecutionDomain::User] {
        let ticket = fixtures::ticket_builder()
            .selected_provider(provider_name)
            .expected_identity(bindings.clone())
            .domain(domain)
            .user_ref((domain == ExecutionDomain::User).then(|| user_ref.clone()))
            .build()
            .expect("conformant fixture ticket");
        let outcome = block_on(provider.launch(&ticket));
        if supported.contains(&domain) {
            let report = outcome.expect("supported domain launches");
            assert_eq!(report.domain, domain);
            if domain == ExecutionDomain::User {
                assert_eq!(report.user_ref.as_ref(), Some(&user_ref));
            } else {
                assert!(report.user_ref.is_none());
            }
        } else {
            assert_eq!(
                outcome.unwrap_err(),
                ProcessConformanceError::DomainNotSupported
            );
        }
    }
}

/// A launch that establishes fewer identity bindings than the Provider
/// requires fails closed and is never reported as running.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_incomplete_launch_identity_fails_closed<P, F>(build: F, provider_name: &str)
where
    P: ProcessProvider,
    F: Fn(ScriptedEffectPort) -> P,
{
    let probe = build(ScriptedEffectPort::launching(
        [],
        crate::identity::WaitReapOwner::Local,
    ));
    let bindings: Vec<IdentityBinding> = probe
        .profile()
        .required_identity_bindings()
        .iter()
        .copied()
        .collect();
    let owner = probe.profile().wait_reap_owner();
    drop(probe);

    let provider = build(ScriptedEffectPort::launching([], owner));
    let ticket = fixtures::ticket_builder()
        .selected_provider(provider_name)
        .expected_identity(bindings)
        .build()
        .expect("conformant fixture ticket");
    assert_eq!(
        block_on(provider.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::IdentityUnverified
    );
}

/// Adoption verifies every required identity binding *before* a pidfd is
/// opened, and ambiguity quarantines instead of adopting.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_adoption_verifies_identity_before_opening_a_pidfd<P, F>(build: F, provider_name: &str)
where
    P: ProcessProvider,
    F: Fn(ScriptedEffectPort) -> P,
{
    let probe = build(ScriptedEffectPort::launching(
        [],
        crate::identity::WaitReapOwner::Local,
    ));
    let required: BTreeSet<IdentityBinding> = probe.profile().required_identity_bindings().clone();
    let owner = probe.profile().wait_reap_owner();
    drop(probe);
    let bindings: Vec<IdentityBinding> = required.iter().copied().collect();

    // Nothing running: no candidate, no pidfd.
    let port = ScriptedEffectPort::launching(bindings.clone(), owner);
    let provider = build(port);
    let ticket = fixtures::ticket_builder()
        .selected_provider(provider_name)
        .expected_identity(bindings.clone())
        .build()
        .expect("conformant fixture ticket");
    assert_eq!(
        block_on(provider.adopt(&ticket)).expect("absent adoption"),
        AdoptionOutcome::Absent
    );

    // Fully verified candidate: adopted, and the pidfd is opened only
    // after the observation.
    let full = build(
        ScriptedEffectPort::launching(bindings.clone(), owner)
            .with_candidate(bindings.clone(), owner),
    );
    let adopted = block_on(full.adopt(&ticket)).expect("verified adoption");
    match adopted {
        AdoptionOutcome::Adopted(report) => {
            assert_eq!(report.adoption, AdoptionCondition::Adopted);
            assert_eq!(report.phase, ProcessPhaseClass::Running);
        }
        AdoptionOutcome::Stale { .. } => panic!("incomplete fixture is not a stale executable"),
        other => panic!("expected adoption, observed {other:?}"),
    }

    // Ambiguous candidate: quarantined, and no pidfd is ever opened.
    let partial: Vec<IdentityBinding> = bindings.iter().copied().skip(1).collect();
    let ambiguous_port =
        ScriptedEffectPort::launching(bindings.clone(), owner).with_candidate(partial, owner);
    let ambiguous = build(ambiguous_port);
    match block_on(ambiguous.adopt(&ticket)).expect("ambiguous adoption reports") {
        AdoptionOutcome::Quarantined(report) => {
            assert_eq!(report.adoption, AdoptionCondition::Quarantined);
            assert_eq!(report.phase, ProcessPhaseClass::Unknown);
        }
        AdoptionOutcome::Stale { .. } => panic!("partial identity is not a stale executable"),
        other => panic!("expected quarantine, observed {other:?}"),
    }
}

/// The pidfd is opened only after identity verification, proven from the
/// recorded effect-port call order.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_pidfd_open_follows_verification(port_calls: &[PortCall]) {
    let observe = port_calls
        .iter()
        .position(|call| *call == PortCall::Observe);
    let open = port_calls
        .iter()
        .position(|call| *call == PortCall::OpenPidfd);
    if let Some(open) = open {
        let observe = observe.expect("a pidfd was opened without an observation");
        assert!(
            observe < open,
            "pidfd opened before identity was observed: {port_calls:?}"
        );
    }
}

/// A fresh target-local controller launch is not an assignment.
pub fn assert_controller_launch_has_no_resource_client(ticket: &LaunchTicket) {
    assert!(
        ticket.validate_controller_launch().is_ok(),
        "controller launch must carry only target-side launch evidence"
    );
    assert!(ticket.provider_generation().is_some());
    assert!(!ticket.has_assignment_binding());
    assert!(ticket.resource_client_binding().is_none());
}

/// An assigned controller or child ticket is fenced to one session and epoch.
pub fn assert_assignment_is_session_fenced(
    ticket: &LaunchTicket,
    session_generation: u64,
    assignment_epoch: u64,
) {
    assert!(ticket.validate_assignment().is_ok());
    assert_eq!(
        ticket
            .session_generation()
            .map(|generation| generation.get()),
        Some(session_generation)
    );
    assert_eq!(ticket.assignment_epoch(), Some(assignment_epoch));
    assert!(ticket.resource_client_binding().is_some());
}

/// Finalizer release requires the exact stop and owner-specific terminal
/// evidence; an ambiguous child cannot be treated as cleaned up.
pub fn assert_finalizer_requires_verified_stop(owner: WaitReapOwner) {
    assert_eq!(
        validate_stop_proof(owner, StopProof::default()),
        Err(ProcessConformanceError::StopProofMissing)
    );
    let complete = StopProof {
        exact_main_signaled: true,
        broker_reaped: owner == WaitReapOwner::Local,
        cgroup_empty: true,
        manager_terminal: owner == WaitReapOwner::ServiceManager,
    };
    assert!(validate_stop_proof(owner, complete).is_ok());
}

/// Public status carries no PID, pidfd, unit name, cgroup, path, argv,
/// environment, or numeric identity.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_status_is_redacted<P: ProcessProvider>(provider: &P, provider_name: &str) {
    let bindings: Vec<IdentityBinding> = provider
        .profile()
        .required_identity_bindings()
        .iter()
        .copied()
        .collect();
    let ticket = fixtures::ticket_builder()
        .selected_provider(provider_name)
        .expected_identity(bindings)
        .build()
        .expect("conformant fixture ticket");
    let report = block_on(provider.launch(&ticket)).expect("launch succeeds");
    let rendered = serde_json::to_value(&report).expect("status serializes");
    let object = rendered.as_object().expect("status is an object");
    for key in object.keys() {
        let lowered = key.to_ascii_lowercase();
        for fragment in FORBIDDEN_STATUS_FRAGMENTS {
            assert!(
                !lowered.contains(fragment),
                "public status key {key} carries the forbidden fragment {fragment}"
            );
        }
    }
    assert_eq!(
        format!("{:?}", report.identity),
        "ProcessIdentityDigest(<redacted>)"
    );
}


/// Both Process lifetimes resolve one plan through one policy path.
///
/// AE20 and AE28: the binding prepares against the committed consumer identity
/// before the consumer runs, and a long-running and a run-to-completion
/// instance differ only in the kind the admitted execution records.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_one_policy_path_for_both_lifetimes() {
    use d2b_contracts_resource::v3::ResourceRef;
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ExecutionInstanceKind, PolicyAuthorization,
    };

    use crate::plan::{BindingPreparation, ProcessPlanRequest};
    use crate::testing::plan_fixtures as fixtures;

    let build = |reference: &str, kind: ExecutionInstanceKind| {
        let consumer = ResourceRef::parse(reference).expect("a canonical fixture reference");
        ProcessPlanRequest::new(
            fixtures::subject(reference),
            BoundedToken::parse("process").expect("a canonical token"),
            fixtures::instance(kind),
            fixtures::requirements(),
            fixtures::policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&consumer)],
            vec![fixtures::prepared_binding(&consumer, BindingPreparation::Prepared)],
            Vec::new(),
        )
        .expect("the fixture request is well formed")
    };

    // A one-shot and a long-running instance go through the same function with
    // the same argument shape; the only difference the plan records is the
    // lifetime the instance declared.
    let one_shot = build("EphemeralProcess/flush", ExecutionInstanceKind::OneShot);
    let long_running = build("Process/worker", ExecutionInstanceKind::LongRunning);
    assert_eq!(
        one_shot
            .admit_execution()
            .expect("the one policy path admits a one-shot instance")
            .kind(),
        ExecutionInstanceKind::OneShot
    );
    assert_eq!(
        long_running
            .admit_execution()
            .expect("the same policy path admits a long-running instance")
            .kind(),
        ExecutionInstanceKind::LongRunning
    );
}

/// A plan is refused unless its source side is prepared, so a consumer never
/// starts against access that does not exist yet.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_preparation_completes_before_the_consumer_runs() {
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ExecutionInstanceKind, PolicyAuthorization,
    };
    use crate::plan::{BindingPreparation, ProcessPlanRequest, ProcessPlanValues, resolve_process_plan};
    use crate::testing::plan_fixtures as fixtures;

    let consumer = fixtures::consumer();
    let values = ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
    let incomplete = ProcessPlanRequest::new(
        fixtures::subject("Process/worker"),
        BoundedToken::parse("process").expect("a canonical token"),
        fixtures::instance(ExecutionInstanceKind::LongRunning),
        fixtures::requirements(),
        fixtures::policy(),
        PolicyAuthorization::granted(),
        fixtures::backend_support(),
        fixtures::ceiling(),
        vec![fixtures::volume_claim(&consumer)],
        vec![fixtures::prepared_binding(&consumer, BindingPreparation::Incomplete)],
        Vec::new(),
    )
    .expect("the fixture request is well formed");
    assert!(
        resolve_process_plan(&incomplete, &values).is_err(),
        "an unprepared source side may not start a consumer"
    );

    let prepared = ProcessPlanRequest::new(
        fixtures::subject("Process/worker"),
        BoundedToken::parse("process").expect("a canonical token"),
        fixtures::instance(ExecutionInstanceKind::LongRunning),
        fixtures::requirements(),
        fixtures::policy(),
        PolicyAuthorization::granted(),
        fixtures::backend_support(),
        fixtures::ceiling(),
        vec![fixtures::volume_claim(&consumer)],
        vec![fixtures::prepared_binding(&consumer, BindingPreparation::Prepared)],
        Vec::new(),
    )
    .expect("the fixture request is well formed");
    let plan = resolve_process_plan(&prepared, &values).expect("a prepared plan resolves");
    assert!(plan.admits_start());
}

/// A restart or an adoption matches all five independent evidence facts, and a
/// candidate that diverges in any one of them is refused by name.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_adoption_matches_every_launch_evidence_fact() {
    use d2b_contracts_resource::v3::ResourceRef;
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ExecutionInstanceKind, PolicyAuthorization,
    };
    use crate::plan::{BindingPreparation, ProcessPlanRequest, ProcessPlanValues, resolve_process_plan};
    use crate::testing::plan_fixtures as fixtures;

    let build = |reference: &str, provider: &str| {
        let consumer = ResourceRef::parse(reference).expect("a canonical fixture reference");
        let request = ProcessPlanRequest::new(
            fixtures::subject(reference),
            BoundedToken::parse(provider).expect("a canonical token"),
            fixtures::instance(ExecutionInstanceKind::LongRunning),
            fixtures::requirements(),
            fixtures::policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&consumer)],
            vec![fixtures::prepared_binding(&consumer, BindingPreparation::Prepared)],
            Vec::new(),
        )
        .expect("the fixture request is well formed");
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        resolve_process_plan(&request, &values).expect("the fixture plan resolves")
    };

    let first = build("Process/worker", "process");
    first
        .evidence()
        .admit_candidate(first.evidence())
        .expect("the same evidence is the same launch");

    // A different assigned Provider is a different launch, even though
    // nothing else about it moved.
    assert!(first
        .evidence()
        .admit_candidate(build("Process/worker", "other").evidence())
        .is_err());
}

/// A supplied launch argument cannot replace a binding-selected source, and
/// the refusal - not a silent drop - is what the caller is told.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_supplied_arguments_cannot_redirect_a_source() {
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ExecutionInstanceKind, PolicyAuthorization,
    };
    use crate::plan::{BindingPreparation, ProcessPlanRequest, ProcessPlanValues, resolve_process_plan};
    use crate::testing::plan_fixtures as fixtures;

    let consumer = fixtures::consumer();
    let values = ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
    let request = |arguments: Vec<String>| {
        ProcessPlanRequest::new(
            fixtures::subject("Process/worker"),
            BoundedToken::parse("process").expect("a canonical token"),
            fixtures::instance(ExecutionInstanceKind::LongRunning),
            fixtures::requirements(),
            fixtures::policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&consumer)],
            vec![fixtures::prepared_binding(&consumer, BindingPreparation::Prepared)],
            arguments,
        )
        .expect("the fixture request is well formed")
    };

    // The destination the broker resolved, and the source it resolved behind
    // it, are both unreachable by name from a supplied argument.
    for hostile in [
        fixtures::DESTINATION_PATH.to_owned(),
        format!("--root={}", fixtures::DESTINATION_PATH),
        fixtures::SOURCE_PATH.to_owned(),
    ] {
        assert!(
            resolve_process_plan(&request(vec![hostile.clone()]), &values).is_err(),
            "argument {hostile:?} must not redirect a source"
        );
    }

    // An argument that names nothing the plan resolved is still admitted: the
    // screen matches resolved paths, not substrings, so a legitimate template
    // value is not caught by the redirect refusal.
    let admitted = resolve_process_plan(&request(vec!["--serve".to_owned()]), &values)
        .expect("an argument naming no resolved source is admitted");
    assert_eq!(admitted.arguments().values(), ["--serve"]);
}

/// A failed launch gives back only the relationships it prepared and refuses
/// to stop a runner it did not start.
#[cfg(any(test, feature = "test-support"))]
pub fn assert_failed_launch_releases_only_its_own_effects() {
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ExecutionInstanceKind, PolicyAuthorization,
    };
    use crate::identity::ProcessIdentityDigest;
    use crate::plan::{BindingPreparation, ProcessPlanRequest, ProcessPlanValues, resolve_process_plan};
    use crate::testing::plan_fixtures as fixtures;

    let consumer = fixtures::consumer();
    let values = ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
    let request = ProcessPlanRequest::new(
        fixtures::subject("Process/worker"),
        BoundedToken::parse("process").expect("a canonical token"),
        fixtures::instance(ExecutionInstanceKind::LongRunning),
        fixtures::requirements(),
        fixtures::policy(),
        PolicyAuthorization::granted(),
        fixtures::backend_support(),
        fixtures::ceiling(),
        vec![fixtures::volume_claim(&consumer)],
        vec![fixtures::prepared_binding(&consumer, BindingPreparation::Prepared)],
        Vec::new(),
    )
    .expect("the fixture request is well formed");
    let plan = resolve_process_plan(&request, &values).expect("the fixture plan resolves");

    let mut scope = plan.launch_scope();
    let own = ProcessIdentityDigest::from_bytes([0x11; 32]);
    scope.record_runner(own).expect("the launch recorded its own runner");
    assert_eq!(scope.release_for(&own).expect("its own runner").slots().len(), 1);
    assert!(
        scope
            .release_for(&ProcessIdentityDigest::from_bytes([0x22; 32]))
            .is_err(),
        "an existing runner is never released by a failed launch"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::ConfigurationDigest;
    use d2b_contracts_resource::v3::{ResourceGeneration, identity::ReconnectGeneration};

    #[test]
    fn controller_launch_and_assignment_authority_are_separate() {
        let launch = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_resource_revision(d2b_contracts_resource::v3::ZoneRevision::new(1))
            .unwrap()
            .with_controller_launch_binding(
                ResourceGeneration::new(2).unwrap(),
                ReconnectGeneration::new(4).unwrap(),
                ConfigurationDigest::from_bytes([1; 32]),
                ConfigurationDigest::from_bytes([2; 32]),
            )
            .unwrap();
        assert_controller_launch_has_no_resource_client(&launch);

        let assignment = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_resource_revision(d2b_contracts_resource::v3::ZoneRevision::new(1))
            .unwrap()
            .with_assignment_binding(
                ResourceGeneration::new(3).unwrap(),
                ReconnectGeneration::new(4).unwrap(),
                9,
                ConfigurationDigest::from_bytes([3; 32]),
            )
            .unwrap();
        assert_assignment_is_session_fenced(&assignment, 4, 9);
        assert_ne!(launch, assignment);
    }

    #[test]
    fn reconnect_and_finalizer_proofs_fail_closed_on_stale_evidence() {
        let first = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_resource_revision(d2b_contracts_resource::v3::ZoneRevision::new(1))
            .unwrap()
            .with_assignment_binding(
                ResourceGeneration::new(1).unwrap(),
                ReconnectGeneration::new(1).unwrap(),
                1,
                ConfigurationDigest::from_bytes([4; 32]),
            )
            .unwrap();
        let reconnect = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_resource_revision(d2b_contracts_resource::v3::ZoneRevision::new(2))
            .unwrap()
            .with_assignment_binding(
                ResourceGeneration::new(1).unwrap(),
                ReconnectGeneration::new(2).unwrap(),
                2,
                ConfigurationDigest::from_bytes([5; 32]),
            )
            .unwrap();
        assert_ne!(first.session_generation(), reconnect.session_generation());
        assert_ne!(first.assignment_epoch(), reconnect.assignment_epoch());
        assert_finalizer_requires_verified_stop(WaitReapOwner::Local);
        assert_finalizer_requires_verified_stop(WaitReapOwner::ServiceManager);
}
}
