//! The shared Process Provider conformance suite.
//!
//! Both `system-systemd` and `system-minijail` run this suite over the same
//! scripted effect port. Every assertion is a neutral obligation from
//! `ADR-046-components-processes-and-sandbox`; anything provider-specific
//! is read from the Provider's declared
//! [`ProcessProviderProfile`](crate::ProcessProviderProfile) rather than
//! branched on by name.

use std::collections::BTreeSet;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::ExecutionDomain;

use crate::error::ProcessConformanceError;
use crate::identity::IdentityBinding;
use crate::provider::{AdoptionOutcome, ProcessProvider};
use crate::status::{AdoptionCondition, ProcessPhaseClass};
use crate::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};

/// Field or value fragments that must never appear in public status.
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
fn execution_refs() -> [ResourceRef; 2] {
    [
        ResourceRef::parse("Host/host-system").expect("valid fixture ref"),
        ResourceRef::parse("Guest/dev-vm").expect("valid fixture ref"),
    ]
}

/// A launch on a Host and on a Guest produces identical conformant status.
///
/// The ResourceType and its status projection do not change with locality.
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
        assert_eq!(report.phase, ProcessPhaseClass::Running);
        assert_eq!(report.adoption, AdoptionCondition::NotApplicable);
        assert!(report.last_exit.is_none());
    }
}

/// A ticket selecting a different Process Provider is rejected.
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
        other => panic!("expected quarantine, observed {other:?}"),
    }
}

/// The pidfd is opened only after identity verification, proven from the
/// recorded effect-port call order.
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

/// Public status carries no PID, pidfd, unit name, cgroup, path, argv,
/// environment, or numeric identity.
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
