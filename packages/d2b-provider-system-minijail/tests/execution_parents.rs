//! Execution-parent and domain cells for `system-minijail`.
//!
//! `ADR-046-provider-model-and-packaging`, section "Provider/system-minijail",
//! implements the same ResourceTypes as `system-systemd`, so the same
//! obligation applies: the status a Process carries does not change with
//! its execution parent. These cells hold Host and Guest to one status
//! shape, and hold the descriptor-gated user domain to the same exact
//! identity requirement under both parents.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::ExecutionDomain;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceUid, ZoneRevision,
};
use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionCondition, AdoptionOutcome, ConfigurationDigest, GuestExecutionBinding,
    IdentityBinding, ProcessConformanceError, ProcessIdentityDigest, ProcessPhaseClass,
    ProcessProvider, ReadinessExpectation, WaitReapOwner,
};
use d2b_provider_system_minijail::{MinijailProcessProvider, PROVIDER_NAME};

fn required() -> Vec<IdentityBinding> {
    vec![
        IdentityBinding::Pid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Cgroup,
        IdentityBinding::Executable,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]
}

fn provider(port: ScriptedEffectPort) -> MinijailProcessProvider<ScriptedEffectPort> {
    MinijailProcessProvider::new(port)
}

fn guest_binding(provider: u64, session: u64, epoch: u64) -> GuestExecutionBinding {
    GuestExecutionBinding::new(
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ConfigurationDigest::from_bytes([8; 32]),
        ReconnectGeneration::new(session).unwrap(),
        epoch,
        ResourceGeneration::new(provider).unwrap(),
        ControllerGeneration::new(1).unwrap(),
    )
    .unwrap()
}

fn bind_guest(
    ticket: d2b_process_conformance::LaunchTicket,
    execution_ref: &ResourceRef,
    binding: GuestExecutionBinding,
) -> d2b_process_conformance::LaunchTicket {
    if execution_ref.resource_type().as_str() == "Guest" {
        ticket.with_guest_execution_binding(binding).unwrap()
    } else {
        ticket
    }
}

fn launching() -> MinijailProcessProvider<ScriptedEffectPort> {
    provider(ScriptedEffectPort::launching(
        required(),
        WaitReapOwner::Local,
    ))
}

fn host_ref() -> ResourceRef {
    ResourceRef::parse("Host/host-system").expect("valid reference")
}

fn guest_ref() -> ResourceRef {
    ResourceRef::parse("Guest/dev-vm").expect("valid reference")
}

fn user_ref() -> ResourceRef {
    ResourceRef::parse("User/alice").expect("valid reference")
}

#[test]
fn a_non_host_execution_parent_yields_the_same_status_shape() {
    let provider = launching();
    let build = |execution_ref: ResourceRef| {
        fixtures::ticket_builder()
            .selected_provider(PROVIDER_NAME)
            .expected_identity(required())
            .execution_ref(execution_ref)
            .build()
            .expect("conformant ticket")
    };
    let on_host = block_on(provider.launch(&build(host_ref()))).expect("host launch");
    let on_guest = block_on(provider.launch(&build(guest_ref()))).expect("guest launch");

    assert_eq!(on_host.execution_ref, host_ref());
    assert_eq!(on_guest.execution_ref, guest_ref());
    assert_eq!(on_host.provider, on_guest.provider);
    assert_eq!(on_host.phase, on_guest.phase);
    assert_eq!(on_host.adoption, on_guest.adoption);
    assert_eq!(on_host.wait_reap_owner, on_guest.wait_reap_owner);
    assert_eq!(on_host.domain, on_guest.domain);
    assert_eq!(on_host.user_ref, on_guest.user_ref);
    assert_eq!(on_host.digests, on_guest.digests);
    assert_eq!(on_host.identity, on_guest.identity);
}

#[test]
fn a_descriptor_admitted_user_domain_still_requires_an_exact_user() {
    // Admitting the user domain in the descriptor widens which tickets are
    // considered; it does not relax the identity the ticket must name. The
    // launch ticket refuses to be constructed without one, under a Host and
    // under a Guest alike, so the controller's own check is defence in
    // depth rather than the only guard.
    for execution_ref in [host_ref(), guest_ref()] {
        assert_eq!(
            fixtures::ticket_builder()
                .selected_provider(PROVIDER_NAME)
                .expected_identity(required())
                .execution_ref(execution_ref)
                .domain(ExecutionDomain::User)
                .user_ref(None)
                .build()
                .unwrap_err(),
            ProcessConformanceError::UserRefRequired
        );
    }
}

#[test]
fn a_user_domain_launch_on_a_guest_carries_the_exact_user_through() {
    let admitted = MinijailProcessProvider::with_user_domain(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local),
        true,
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .execution_ref(guest_ref())
        .domain(ExecutionDomain::User)
        .user_ref(Some(user_ref()))
        .build()
        .expect("conformant ticket");
    let report = block_on(admitted.launch(&ticket)).expect("guest user-domain launch");
    assert_eq!(report.domain, ExecutionDomain::User);
    assert_eq!(report.user_ref.as_ref(), Some(&user_ref()));
    assert_eq!(report.execution_ref, guest_ref());
}

#[test]
fn adoption_on_a_non_host_parent_verifies_identity_the_same_way() {
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local)
            .with_candidate(required(), WaitReapOwner::Local),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .execution_ref(guest_ref())
        .build()
        .expect("conformant ticket");
    match block_on(provider.adopt(&ticket)).expect("adoption reports") {
        AdoptionOutcome::Adopted(report) => {
            assert_eq!(report.adoption, AdoptionCondition::Adopted);
            assert_eq!(report.execution_ref, guest_ref());
        }
        other => panic!("expected adoption, observed {other:?}"),
    }
    assert_eq!(
        provider.port().calls(),
        vec![PortCall::Observe, PortCall::OpenPidfd]
    );
}

#[test]
fn a_candidate_whose_wait_owner_disagrees_is_quarantined() {
    // d2b owns wait and reap for every minijail-launched process. A
    // candidate the service manager owns is not this Provider's process,
    // however many identity bindings match, so it quarantines rather than
    // being adopted, signalled, or reused.
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local)
            .with_candidate(required(), WaitReapOwner::ServiceManager),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket");
    assert!(matches!(
        block_on(provider.adopt(&ticket)).expect("adoption reports"),
        AdoptionOutcome::Quarantined(_)
    ));
    assert!(!provider.port().calls().contains(&PortCall::OpenPidfd));
}

#[test]
fn readiness_and_identity_seals_are_parent_neutral() {
    for execution_ref in [host_ref(), guest_ref()] {
        let provider = provider(
            ScriptedEffectPort::launching(required(), WaitReapOwner::Local)
                .with_candidate(required(), WaitReapOwner::Local),
        );
        let ticket = fixtures::ticket_builder()
            .selected_provider(PROVIDER_NAME)
            .expected_identity(required())
            .execution_ref(execution_ref.clone())
            .build()
            .expect("conformant ticket")
            .with_readiness(ReadinessExpectation::condition(1_000).expect("bounded readiness"))
            .with_expected_identity_digest(ProcessIdentityDigest::from_bytes([0x11; 32]))
            .expect("identity seal");

        let report = block_on(provider.launch(&ticket)).expect("ready launch");
        assert_eq!(report.execution_ref, execution_ref);
        assert_eq!(report.phase, ProcessPhaseClass::Ready);
        assert_eq!(
            provider.port().calls(),
            vec![PortCall::Launch, PortCall::Observe]
        );
    }
}

#[test]
fn controller_launch_and_assignment_authority_remain_disjoint_on_each_parent() {
    for execution_ref in [host_ref(), guest_ref()] {
        let launch = bind_guest(
            fixtures::ticket_builder()
                .selected_provider(PROVIDER_NAME)
                .execution_ref(execution_ref.clone())
                .without_guest_execution_binding()
                .build()
                .expect("conformant ticket"),
            &execution_ref,
            guest_binding(2, 4, 1),
        )
        .with_resource_revision(ZoneRevision::new(1))
        .expect("revision binding")
        .with_controller_launch_binding(
            ResourceGeneration::new(2).expect("provider generation"),
            ReconnectGeneration::new(4).expect("session generation"),
            ConfigurationDigest::from_bytes([1; 32]),
            ConfigurationDigest::from_bytes([2; 32]),
        )
        .expect("controller launch binding");
        d2b_process_conformance::suite::assert_controller_launch_has_no_resource_client(&launch);

        let assignment = bind_guest(
            fixtures::ticket_builder()
                .selected_provider(PROVIDER_NAME)
                .execution_ref(execution_ref.clone())
                .without_guest_execution_binding()
                .build()
                .expect("conformant ticket"),
            &execution_ref,
            guest_binding(2, 4, 9),
        )
        .with_resource_revision(ZoneRevision::new(1))
        .expect("revision binding")
        .with_assignment_binding(
            ResourceGeneration::new(2).expect("provider generation"),
            ReconnectGeneration::new(4).expect("session generation"),
            9,
            ConfigurationDigest::from_bytes([3; 32]),
        )
        .expect("assignment binding");
        d2b_process_conformance::suite::assert_assignment_is_session_fenced(&assignment, 4, 9);
    }
}
