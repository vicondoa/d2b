//! Coverage for the Network provider's membership admission and shared-fabric
//! realization.
//!
//! Every case is a failure the new model exists to prevent: two consumers on
//! one Network duplicating the shared fabric, a foreign host marker being
//! rewritten, one consumer's release removing host state another still uses,
//! a child target-support ceiling minting a membership of its own, and a
//! cached admission outliving the dependency revisions it was fenced against.

use d2b_contracts_resource::v3::{
    BindingAuthorization, BindingKey, BindingKind, BindingLifecycleState, BindingSlot,
    BindingSupportEntry, BoundedToken, ChildSupportCeiling, CompletionCondition, DesiredDigest,
    DesiredRevision, FreshnessTuple, Ipv4Cidr, NetworkBindingRequest, NetworkMembership,
    NetworkPresentation, NetworkSpec, PortProtocol, PortSpec, RequestedRights, ResourceGeneration,
    ResourceRef, ResourceUid, StoreIncarnation, ZoneId, network_binding::NetworkExecutionParentInput,
};
use d2b_provider_network_local::{
    HostStateObservation, MembershipAdmission, NetworkBindingError, NetworkBindingRegistry,
    NetworkFabricTarget, NetworkMembershipCeiling, NmUnmanagedObservation, ParentInputOutcome,
    controller::{NetworkAdmissionIntent, NetworkAdmissionKey, render_membership_config},
    nftables::{SharedNftTable, SharedTableEntry},
    observe::HostNetworkOccupancy,
};

const ZONE: &str = "work";
const NETWORK_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const FIRST_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const SECOND_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const GUEST_UID: &str = "423e4567-e89b-42d3-a456-426614174003";
const INSTALLED_GENERATION: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).expect("bounded slot token")
}

fn host_target() -> NetworkFabricTarget {
    NetworkFabricTarget::new(reference("Host/system")).expect("host execution target")
}

fn network_spec() -> NetworkSpec {
    NetworkSpec::minimal(
        Ipv4Cidr::parse("10.20.0.0/24").expect("lan cidr"),
        Ipv4Cidr::parse("10.20.1.0/30").expect("uplink cidr"),
        token("net-vm"),
    )
    .expect("minimal network spec")
}

fn host_intent(
    network_generation: u64,
    attachment_generation: u64,
) -> NetworkAdmissionIntent {
    NetworkAdmissionIntent::new(
        NetworkAdmissionKey::new(
            uid("523e4567-e89b-42d3-a456-426614174004"),
            uid(NETWORK_UID),
            ResourceGeneration::new(network_generation).expect("network generation"),
            ResourceGeneration::new(attachment_generation).expect("attachment generation"),
            d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(INSTALLED_GENERATION)
                .expect("installed generation"),
        ),
        network_spec(),
        vec![uid(GUEST_UID)],
    )
    .expect("root-owned host intent")
}

fn freshness(resource: &str, identity: &str, revision: DesiredRevision, payload: &str) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("store-one").expect("store incarnation"),
        reference(resource),
        uid(identity),
        revision,
        DesiredDigest::of(payload.as_bytes()),
    )
}

fn network_freshness() -> FreshnessTuple {
    freshness(
        "Network/lan",
        NETWORK_UID,
        DesiredRevision::INITIAL,
        "network",
    )
}

fn consumer_freshness(consumer: &str, identity: &str) -> FreshnessTuple {
    freshness(consumer, identity, DesiredRevision::INITIAL, "consumer")
}

fn dependencies() -> Vec<FreshnessTuple> {
    vec![network_freshness(), consumer_freshness("Process/frontend", FIRST_UID)]
}

fn port(number: u16, protocol: PortProtocol, purpose: &str) -> PortSpec {
    PortSpec::new(number, protocol, purpose).expect("declared port")
}

fn request(
    consumer: &str,
    slot_name: &str,
    ports: Vec<PortSpec>,
    allow_egress: bool,
) -> Result<NetworkBindingRequest, d2b_contracts_resource::v3::BindingContractError> {
    NetworkBindingRequest::new(
        reference("Network/lan"),
        reference(consumer),
        slot(slot_name),
        NetworkMembership::new(ports, allow_egress)?,
        NetworkPresentation::shared_fabric(),
    )
}

fn admission(
    consumer: &str,
    consumer_uid: &str,
    slot_name: &str,
    ports: Vec<PortSpec>,
    allow_egress: bool,
) -> MembershipAdmission {
    let fence = vec![network_freshness(), consumer_freshness(consumer, consumer_uid)];
    MembershipAdmission {
        zone: zone(),
        source_uid: uid(NETWORK_UID),
        consumer_uid: uid(consumer_uid),
        target: host_target(),
        request: request(consumer, slot_name, ports, allow_egress).expect("typed request"),
        host_intent: host_intent(3, 7).proof(),
        ceiling: NetworkMembershipCeiling::new(true, 8),
        authorization: BindingAuthorization::granted(),
        dependencies: fence,
        observation: HostStateObservation::empty(),
    }
}

/// AE9: two processes on one Network share the fabric and keep their own
/// egress and port policy.
#[test]
fn two_consumers_share_one_fabric_with_distinct_traffic_policy() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let frontend = registry
        .admit(admission(
            "Process/frontend",
            FIRST_UID,
            "lan",
            vec![port(443, PortProtocol::Tcp, "https")],
            false,
        ))
        .expect("the first consumer is admitted");
    let shared_digest = registry
        .fabric(frontend.fabric())
        .expect("the fabric is realized")
        .realization()
        .digest();

    let backend = registry
        .admit(admission(
            "Process/backend",
            SECOND_UID,
            "lan",
            vec![
                port(8443, PortProtocol::Tcp, "admin"),
                port(53, PortProtocol::Udp, "resolver"),
            ],
            true,
        ))
        .expect("the second consumer shares the same fabric");

    assert_eq!(registry.fabric_count(), 1, "one Network, one target, one fabric");
    assert_eq!(
        frontend.fabric(),
        backend.fabric(),
        "both consumers resolve to the same shared fabric identity"
    );
    let fabric = registry
        .fabric(frontend.fabric())
        .expect("the fabric is realized");
    assert_eq!(fabric.member_count(), 2, "both memberships sit on it");
    assert_eq!(
        fabric.realization().digest(),
        shared_digest,
        "admitting a second consumer does not re-derive the shared realization"
    );

    let frontend_policy = frontend.policy();
    let backend_policy = backend.policy();
    assert!(
        frontend_policy.admits_inbound(PortProtocol::Tcp, 443)
            && !frontend_policy.admits_inbound(PortProtocol::Tcp, 8443),
        "the frontend receives only the port it asked for"
    );
    assert!(
        backend_policy.admits_inbound(PortProtocol::Tcp, 8443)
            && backend_policy.admits_inbound(PortProtocol::Udp, 53)
            && !backend_policy.admits_inbound(PortProtocol::Tcp, 443),
        "the backend's ports stay its own"
    );
    assert!(
        !frontend_policy.allow_egress() && backend_policy.allow_egress(),
        "egress is a per-consumer decision, not a fabric-wide one"
    );
    assert_ne!(
        frontend_policy.digest(),
        backend_policy.digest(),
        "the two policies are distinguishable without a second fabric"
    );
    assert_ne!(
        frontend_policy.fabric_interface(),
        backend_policy.fabric_interface(),
        "each membership holds its own interface on the shared fabric"
    );

    let provenance = fabric.realization().provenance();
    let frontend_config =
        render_membership_config(&network_spec(), provenance, frontend_policy).expect("config");
    let backend_config =
        render_membership_config(&network_spec(), provenance, backend_policy).expect("config");
    assert_eq!(
        frontend_config.nftables, backend_config.nftables,
        "the shared firewall projection is rendered once"
    );
    assert_eq!(frontend_config.routing, backend_config.routing);
    assert_ne!(
        frontend_config.dnsmasq, backend_config.dnsmasq,
        "the per-consumer reservation differs"
    );
    assert_ne!(
        frontend_config.digest(),
        backend_config.digest(),
        "each consumer's config is separately reloadable"
    );
}

/// A foreign nftables marker in the Network's own slot refuses the mutation
/// and the observed bytes survive untouched.
#[test]
fn a_foreign_nftables_marker_refuses_the_membership() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let foreign = b"someone else's chain".to_vec();
    let table = SharedNftTable::new(vec![SharedTableEntry::foreign_in_network_slot(
        uid(NETWORK_UID),
        foreign.clone(),
    )]);
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    pending.observation = HostStateObservation::new(
        table.clone(),
        HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
        NmUnmanagedObservation::default(),
    );

    assert_eq!(
        registry.admit(pending).unwrap_err(),
        NetworkBindingError::ForeignHostState
    );
    assert_eq!(registry.fabric_count(), 0, "no fabric was realized");
    assert_eq!(
        table.entries()[0].bytes(),
        foreign.as_slice(),
        "the foreign chain is preserved byte for byte"
    );
}

/// A foreign ownership marker on a derived host interface refuses the
/// membership.
#[test]
fn a_foreign_host_interface_marker_refuses_the_membership() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let intent = host_intent(3, 7);
    let occupied = intent.interface_names()[0].clone();
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    pending.observation = HostStateObservation::new(
        SharedNftTable::new(Vec::new()),
        HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new())
            .with_interface_ownership([(occupied.clone(), "operator:uplink".to_owned())]),
        NmUnmanagedObservation::default(),
    );
    let observed = pending.observation.occupancy().clone();

    assert_eq!(
        registry.admit(pending).unwrap_err(),
        NetworkBindingError::ForeignHostState
    );
    assert_eq!(registry.fabric_count(), 0);
    assert_eq!(
        observed.interface_ownership_marker(&occupied),
        Some("operator:uplink"),
        "the operator's marker is preserved"
    );
}

/// A NetworkManager unmanaged configuration somebody else installed refuses
/// the membership.
#[test]
fn a_foreign_network_manager_configuration_refuses_the_membership() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    let unmanaged = NmUnmanagedObservation::new(vec!["ens3".to_owned()], Vec::new());
    let installed = unmanaged.devices().to_vec();
    pending.observation = HostStateObservation::new(
        SharedNftTable::new(Vec::new()),
        HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
        unmanaged,
    );

    assert_eq!(
        registry.admit(pending).unwrap_err(),
        NetworkBindingError::ForeignHostState
    );
    assert_eq!(registry.fabric_count(), 0);
    assert_eq!(
        installed,
        ["ens3".to_owned()],
        "the existing unmanaged devices are preserved"
    );
}

/// One consumer leaving never removes the fabric another consumer still uses.
#[test]
fn releasing_one_consumer_keeps_the_shared_fabric() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let frontend = registry
        .admit(admission(
            "Process/frontend",
            FIRST_UID,
            "lan",
            vec![port(443, PortProtocol::Tcp, "https")],
            false,
        ))
        .expect("the first consumer is admitted");
    let backend = registry
        .admit(admission(
            "Process/backend",
            SECOND_UID,
            "lan",
            vec![port(8443, PortProtocol::Tcp, "admin")],
            false,
        ))
        .expect("the second consumer is admitted");
    registry
        .prepare(frontend.key())
        .expect("source preparation");
    registry
        .complete(frontend.key())
        .expect("consumer completion");
    registry
        .prepare(backend.key())
        .expect("source preparation");
    registry
        .complete(backend.key())
        .expect("consumer completion");

    let first = registry
        .release(frontend.key())
        .expect("the first consumer releases");
    assert_eq!(first.remaining_members(), 1);
    assert!(
        first.fabric_retained(),
        "a live member keeps the shared fabric"
    );
    assert_eq!(registry.fabric_count(), 1);
    assert!(registry.fabric_in_use(backend.fabric()));
    assert_eq!(
        registry
            .readiness(backend.key())
            .expect("the surviving membership is still tracked")
            .state(),
        BindingLifecycleState::Active,
        "the surviving consumer is untouched"
    );

    let second = registry
        .release(backend.key())
        .expect("the last consumer releases");
    assert_eq!(second.remaining_members(), 0);
    assert!(
        !second.fabric_retained(),
        "the last release retires the fabric entry"
    );
    assert_eq!(registry.fabric_count(), 0);
    assert!(registry.membership(backend.key()).is_none());
}

/// AE31: a child target-support ceiling constrains child admission and creates
/// no membership, no reservation, and no fabric of its own.
#[test]
fn a_target_support_ceiling_creates_no_membership() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let ceiling = ChildSupportCeiling::new(vec![
        BindingSupportEntry::new(BindingKind::Network, vec![RequestedRights::Consume])
            .expect("network support entry"),
    ])
    .expect("child support ceiling");
    let target = host_target();

    let outcome = registry
        .classify_parent_input(
            target.clone(),
            &NetworkExecutionParentInput::ChildSupportCeiling(ceiling),
            None,
        )
        .expect("the ceiling is recorded");
    assert!(
        matches!(outcome, ParentInputOutcome::Ceiling { admits: true }),
        "the ceiling admits network membership for a child"
    );
    assert_eq!(
        registry.fabric_count(),
        0,
        "a ceiling realizes no fabric and mints no membership"
    );
    assert!(registry.child_ceiling(&target).is_some());

    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    pending.observation = HostStateObservation::empty();
    let admitted = registry.admit(pending).expect("the ceiling admits the child");
    assert_eq!(
        admitted.evidence().reservation().source_uid(),
        &uid(NETWORK_UID),
        "the reservation belongs to the Network, never to the ceiling"
    );
}

/// A ceiling that does not cover network membership refuses a child request
/// instead of admitting it by default.
#[test]
fn a_ceiling_without_network_support_refuses_a_child_request() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let ceiling = ChildSupportCeiling::new(vec![
        BindingSupportEntry::new(BindingKind::Volume, vec![RequestedRights::Observe])
            .expect("volume support entry"),
    ])
    .expect("child support ceiling");
    let target = host_target();
    let outcome = registry
        .classify_parent_input(
            target.clone(),
            &NetworkExecutionParentInput::ChildSupportCeiling(ceiling),
            None,
        )
        .expect("the ceiling is recorded");
    assert!(matches!(
        outcome,
        ParentInputOutcome::Ceiling { admits: false }
    ));

    assert_eq!(
        registry
            .admit(admission(
                "Process/frontend",
                FIRST_UID,
                "lan",
                vec![port(443, PortProtocol::Tcp, "https")],
                false,
            ))
            .unwrap_err(),
        NetworkBindingError::TargetSupportMissing
    );
    assert_eq!(registry.fabric_count(), 0);
}

/// A parent's child defaults shape one child's request and grant the parent
/// nothing.
#[test]
fn child_defaults_create_no_membership() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let defaults = d2b_contracts_resource::v3::ChildRequestDefaults::new(
        reference("Process/frontend"),
        d2b_contracts_resource::v3::DefaultedSource::new(
            BindingKind::Network,
            reference("Network/lan"),
            None,
        )
        .expect("defaulted source"),
    )
    .expect("child defaults");

    let outcome = registry
        .classify_parent_input(
            host_target(),
            &NetworkExecutionParentInput::ChildRequestDefaults(defaults),
            None,
        )
        .expect("the defaults are classified");
    assert!(matches!(
        outcome,
        ParentInputOutcome::ChildDefault { ref child_ref } if child_ref == &reference("Process/frontend")
    ));
    assert_eq!(registry.fabric_count(), 0);
}

/// A parent use is a membership whose consumer is the parent itself.
#[test]
fn a_parent_use_becomes_a_membership() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let target = host_target();
    let pending = admission(
        "Host/system",
        FIRST_UID,
        "lan",
        vec![port(22, PortProtocol::Tcp, "admin")],
        false,
    );
    let request = pending.request.clone();

    let outcome = registry
        .classify_parent_input(
            target,
            &NetworkExecutionParentInput::ParentUse(request),
            Some(pending),
        )
        .expect("the parent use is admitted");
    let ParentInputOutcome::Membership(membership) = outcome else {
        panic!("a parent use yields the parent's own membership");
    };
    assert_eq!(
        membership.policy().consumer_ref(),
        &reference("Host/system"),
        "the membership belongs to the parent, not to a child"
    );
    assert_eq!(registry.fabric_count(), 1);
}

/// A Network with no external attachment carries no outbound traffic, so a
/// consumer asking for egress is refused by the source's own policy.
#[test]
fn the_source_refuses_egress_the_network_cannot_carry() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        Vec::new(),
        true,
    );
    pending.ceiling = NetworkMembershipCeiling::from_spec(&network_spec());

    assert_eq!(
        registry.admit(pending).unwrap_err(),
        NetworkBindingError::SourcePolicyRefused
    );
    assert_eq!(registry.fabric_count(), 0);
}

/// The same request is admitted once the Network's own ceiling allows egress.
#[test]
fn the_source_admits_egress_within_its_ceiling() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let admitted = registry
        .admit(admission(
            "Process/frontend",
            FIRST_UID,
            "lan",
            Vec::new(),
            true,
        ))
        .expect("egress within the ceiling is admitted");
    assert!(admitted.policy().allow_egress());
    assert_eq!(registry.fabric_count(), 1);
}

/// A second, differently shaped declaration for one live consumer slot is
/// refused before any mutation.
#[test]
fn a_conflicting_declaration_cannot_replace_a_live_slot() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let first = registry
        .admit(admission(
            "Process/frontend",
            FIRST_UID,
            "lan",
            vec![port(443, PortProtocol::Tcp, "https")],
            false,
        ))
        .expect("the first declaration is admitted");

    assert_eq!(
        registry
            .admit(admission(
                "Process/frontend",
                FIRST_UID,
                "lan",
                vec![port(22, PortProtocol::Tcp, "admin")],
                false,
            ))
            .unwrap_err(),
        NetworkBindingError::SlotOccupied
    );
    assert_eq!(
        registry
            .membership(first.key())
            .expect("the original declaration still holds the slot")
            .policy()
            .ports()[0]
            .port(),
        443,
        "the refused declaration did not change the live policy"
    );
}

/// Source preparation and consumer-side completion stay separate, so a
/// consumer never starts before its pre-start conditions hold.
#[test]
fn preparation_and_consumer_completion_are_separate() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let admitted = registry
        .admit(admission(
            "Process/frontend",
            FIRST_UID,
            "lan",
            vec![port(443, PortProtocol::Tcp, "https")],
            false,
        ))
        .expect("the consumer is admitted");

    assert_eq!(
        registry
            .complete(admitted.key())
            .unwrap_err(),
        NetworkBindingError::UnexpectedState,
        "consumer completion waits for source preparation"
    );
    let prepared = registry.prepare(admitted.key()).expect("preparation");
    assert_eq!(prepared.state(), BindingLifecycleState::Prepared);
    assert_eq!(prepared.prepare(), CompletionCondition::Complete);
    assert_eq!(
        prepared.consumer_completion(),
        CompletionCondition::Pending,
        "the consumer has not started yet"
    );

    let active = registry.complete(admitted.key()).expect("completion");
    assert_eq!(active.state(), BindingLifecycleState::Active);
    assert_eq!(active.consumer_completion(), CompletionCondition::Complete);
}

/// Cached readiness cannot remint access: an admission whose dependency
/// revisions changed is refused rather than reported as granted use.
#[test]
fn a_changed_dependency_refuses_recovery() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let admitted = registry
        .admit(admission(
            "Process/frontend",
            FIRST_UID,
            "lan",
            vec![port(443, PortProtocol::Tcp, "https")],
            false,
        ))
        .expect("the consumer is admitted");
    registry.prepare(admitted.key()).expect("preparation");
    registry.complete(admitted.key()).expect("completion");

    let observed = dependencies();
    assert_eq!(
        registry.recover(admitted.key(), &observed).expect("recovered").state(),
        BindingLifecycleState::Active,
        "matching evidence recovers the recorded effect"
    );

    let stale = vec![
        network_freshness(),
        freshness("Process/frontend", FIRST_UID, DesiredRevision::INITIAL, "edited"),
    ];
    assert_eq!(
        registry.recover(admitted.key(), &stale).unwrap_err(),
        NetworkBindingError::StaleAuthority
    );
}

/// An admission with no authorization grant is refused at the authorize stage.
#[test]
fn a_request_without_a_grant_is_refused() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    pending.authorization = BindingAuthorization::absent();

    assert_eq!(
        registry.admit(pending).unwrap_err(),
        NetworkBindingError::NotAuthorized
    );
    assert_eq!(registry.fabric_count(), 0);
}

/// A fence that omits the consumer row cannot keep an admission alive across a
/// change to the row that carried it.
#[test]
fn an_incomplete_dependency_fence_is_refused() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    pending.dependencies.truncate(1);

    assert_eq!(
        registry.admit(pending).unwrap_err(),
        NetworkBindingError::StaleAuthority
    );
    assert_eq!(registry.fabric_count(), 0);
}

/// A namespace presentation reaches the consumer under its own interface name
/// while the fabric interface stays provider-owned.
#[test]
fn a_namespace_presentation_names_the_consumers_interface() {
    let mut registry = NetworkBindingRegistry::new(zone());
    let mut pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    pending.request = NetworkBindingRequest::new(
        reference("Network/lan"),
        reference("Process/frontend"),
        slot("lan"),
        NetworkMembership::new(vec![port(443, PortProtocol::Tcp, "https")], false)
            .expect("membership"),
        NetworkPresentation::namespace_interface("eth0").expect("namespace presentation"),
    )
    .expect("typed request");

    let admitted = registry.admit(pending).expect("the namespace request is admitted");
    assert_eq!(
        admitted.policy().presented_interface().as_str(),
        "eth0",
        "the consumer sees the interface it asked for"
    );
    assert_ne!(
        admitted.policy().fabric_interface(),
        admitted.policy().presented_interface(),
        "the host-side fabric interface stays provider-derived"
    );
}

/// A slot the daemon is about to commit can be checked without mutating the
/// registry.
#[test]
fn a_slot_can_be_checked_before_the_row_is_committed() {
    let registry = NetworkBindingRegistry::new(zone());
    let pending = admission(
        "Process/frontend",
        FIRST_UID,
        "lan",
        vec![port(443, PortProtocol::Tcp, "https")],
        false,
    );
    let key: BindingKey = pending
        .request
        .key(zone(), uid(NETWORK_UID), uid(FIRST_UID))
        .expect("relationship key");

    assert!(registry.check_slot(&key, &pending.request.fingerprint()).is_ok());
    assert_eq!(
        registry.check_slot(&key, &pending.request.fingerprint()),
        Ok(d2b_contracts_resource::v3::BindingSlotDecision::Claimed),
        "checking does not record, so the same candidate still reads as free"
    );
    assert_eq!(registry.fabric_count(), 0);
}
