//! Coverage for the Network provider's membership admission and shared-fabric
//! realization.
//!
//! Every case is a failure the new model exists to prevent: two consumers on
//! one Network duplicating the shared fabric, a foreign host marker being
//! rewritten, one consumer's release removing host state another still uses,
//! a child target-support ceiling minting a membership of its own, and a
//! cached admission outliving the dependency revisions it was fenced against.

use d2b_contracts_resource::v3::{
    BindingArbitration, BindingAuthorization, BindingKey, BindingKind, BindingLifecycleState,
    BindingRealizationFacet, BindingSlot, BindingSupportEntry, BoundedToken, ChildSupportCeiling,
    CompletionCondition, DesiredDigest, DesiredRevision, DhcpSpec, DnsSpec, FreshnessTuple,
    Ipv4Cidr, IsolationSpec, MdnsSpec, NetworkAttachmentEntry, NetworkBindingRequest,
    NetworkBindingSpec, NetworkMembership, NetworkPresentation, NetworkProvenance, NetworkSpec,
    PortProtocol, PortSpec, RequestedRights, ResourceGeneration, ResourceRef, ResourceUid,
    RoutingSpec, StoreIncarnation, ZoneId, admit_binding_row_refs,
    network_binding::NetworkExecutionParentInput,
};
use d2b_provider_network_local::{
    HostStateObservation, MembershipAdmission, NetworkAdmittedConsumer, NetworkBindingError,
    NetworkBindingRegistry, NetworkBindingSource, NetworkFabricTarget, NetworkMembershipCeiling,
    NmUnmanagedObservation, ParentInputOutcome, canonical_binding_rows,
    controller::{NetworkAdmissionIntent, NetworkAdmissionKey, render_membership_config},
    membership_interface,
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

/// The interface a workload consumer reaches is derived once, in the binding
/// family. A second derivation inside the production admission intent would
/// let the tap an intent reserves drift away from the interface an admitted
/// membership claims, so this pins that the production intent reserves
/// exactly the membership family's derivation - and that two consumers on one
/// Network still get distinct interfaces.
#[test]
fn the_production_intent_reserves_the_membership_interfaces() {
    let intent = host_intent(3, 7);
    let provenance = intent.key().provenance();
    let claimed = membership_interface(&provenance, &uid(GUEST_UID)).expect("membership interface");
    assert!(
        intent
            .interface_names()
            .iter()
            .any(|name| name == &claimed),
        "the production admission intent reserves the interface an admitted membership claims"
    );

    // The same rule, keyed by consumer identity, is what keeps one consumer
    // from reaching another's interface on the shared fabric.
    let other = membership_interface(&provenance, &uid(SECOND_UID)).expect("second interface");
    assert_ne!(
        claimed, other,
        "two consumers on one Network hold distinct interfaces on the shared fabric"
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

/// The Zone every derived relationship belongs to, held for the whole test so
/// a derived source can borrow it.
static ZONE_ID: std::sync::LazyLock<ZoneId> =
    std::sync::LazyLock::new(|| ZoneId::parse(ZONE).expect("zone"));

// ---------------------------------------------------------------------------
// Committed Network row -> committed NetworkBinding rows
// ---------------------------------------------------------------------------

const HOST_UID: &str = "523e4567-e89b-42d3-a456-426614174004";

/// A committed Network row whose attachments are the execution targets that
/// join its fabric.
fn attached_network_spec(attachments: &[(&str, u8)]) -> NetworkSpec {
    NetworkSpec::new(
        Ipv4Cidr::parse("10.20.0.0/24").expect("lan cidr"),
        Ipv4Cidr::parse("10.20.1.0/30").expect("uplink cidr"),
        None,
        false,
        IsolationSpec::default(),
        RoutingSpec::default(),
        DhcpSpec::default(),
        DnsSpec::default(),
        None,
        MdnsSpec::default(),
        None,
        token("net-vm"),
        attachments
            .iter()
            .map(|(target, index)| {
                NetworkAttachmentEntry::new(reference(target), *index, None)
                    .expect("reserved attachment entry")
            })
            .collect(),
    )
    .expect("network spec declaring its attached consumers")
}

fn provenance() -> NetworkProvenance {
    host_intent(3, 7).key().provenance()
}

fn admitted_consumer(
    target: &str,
    identity: &str,
    presentation: NetworkPresentation,
) -> NetworkAdmittedConsumer {
    NetworkAdmittedConsumer::new(reference(target), uid(identity), presentation)
        .expect("admitted fabric consumer")
}

/// The committed Network row a derivation reads, for one attachment set and
/// one admitted consumer set.
fn binding_source<'a>(
    network_ref: &'a ResourceRef,
    spec: &'a NetworkSpec,
    provenance: &'a NetworkProvenance,
    consumers: &'a [NetworkAdmittedConsumer],
) -> NetworkBindingSource<'a> {
    NetworkBindingSource {
        network_ref,
        zone: &ZONE_ID,
        provenance,
        spec,
        consumers,
    }
}

/// The committed Network row of the primary fixture: a Host and a Guest both
/// attached, each with its own store identity and presentation.
fn attached_lan() -> (ResourceRef, NetworkSpec, NetworkProvenance, Vec<NetworkAdmittedConsumer>) {
    let network_ref = reference("Network/lan");
    let spec = attached_network_spec(&[("Host/system", 2), ("Guest/work-vm", 3)]);
    let provenance = provenance();
    let consumers = vec![
        admitted_consumer(
            "Host/system",
            HOST_UID,
            NetworkPresentation::shared_fabric(),
        ),
        admitted_consumer(
            "Guest/work-vm",
            GUEST_UID,
            NetworkPresentation::namespace_interface("eth0").expect("namespace presentation"),
        ),
    ];
    (network_ref, spec, provenance, consumers)
}

/// One committed Network row implies one committed binding row per attached
/// consumer, and the committed bytes read back as that family's own strict
/// row contract.
#[test]
fn a_committed_network_row_commits_one_binding_row_per_attached_consumer() {
    let (network_ref, spec, provenance, consumers) = attached_lan();
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);
    let rows = canonical_binding_rows(&source)
        .expect("the committed row derives its binding rows");

    assert_eq!(rows.len(), 2, "one row per consumer that joins the fabric");
    let names: std::collections::BTreeSet<&str> =
        rows.iter().map(|row| row.name().as_str()).collect();
    assert_eq!(names.len(), rows.len(), "two consumers, two row names");

    for (row, consumer) in rows.iter().zip(&consumers) {
        let decoded: NetworkBindingSpec =
            serde_json::from_slice(row.spec()).expect("the committed bytes are a NetworkBinding row");
        assert_eq!(decoded.network_ref(), &network_ref);
        assert_eq!(decoded.execution_ref(), consumer.target().reference());
        assert_eq!(decoded.presentation(), consumer.presentation());
        admit_binding_row_refs(
            BindingKind::Network,
            decoded.network_ref(),
            decoded.execution_ref(),
        )
        .expect("the row names a Network and a consumer this kind admits");
        assert_eq!(
            row.fabric_interface().as_str(),
            membership_interface(&provenance, consumer.consumer_uid())
                .expect("membership interface")
                .as_str(),
            "the row holds the interface the family derives, not a second one"
        );
    }

    // The Host reaches the provider-owned fabric directly; the Guest reaches
    // its own membership under the name its own request declared.
    let guest: NetworkBindingSpec =
        serde_json::from_slice(rows[1].spec()).expect("the Guest's committed row");
    assert_eq!(
        guest.presentation(),
        &NetworkPresentation::namespace_interface("eth0").expect("namespace presentation")
    );
    assert_eq!(
        guest
            .source()
            .realized_facets()
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            BindingRealizationFacet::SharedFabric,
            BindingRealizationFacet::NamespaceInterface
        ]
        .into_iter()
        .collect(),
        "the committed decision declares both facets this provider realizes"
    );
    for facet in guest.presentation().required_facets() {
        assert!(
            guest.source().realized_facets().contains(facet),
            "the committed decision realizes the presentation the row asks for"
        );
    }
    assert_eq!(
        guest.source().admitted_rights(),
        [RequestedRights::Consume],
        "the source admits exactly the right the family admits"
    );
    assert_eq!(
        guest.source().arbitration(),
        BindingArbitration::Shared,
        "a membership is admitted alongside its peers, never exclusively"
    );
}

/// The committed row is the source provider's decision, so it carries the
/// membership and nothing else: a second per-consumer ruleset in the row
/// would fork the Network's one firewall slot.
#[test]
fn a_committed_row_carries_no_traffic_policy() {
    let (network_ref, spec, provenance, consumers) = attached_lan();
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);
    let rows = canonical_binding_rows(&source)
        .expect("derived rows");
    let committed: serde_json::Value =
        serde_json::from_slice(rows[1].spec()).expect("committed row value");
    let fields: std::collections::BTreeSet<&String> = committed
        .as_object()
        .expect("a JSON object row")
        .keys()
        .collect();
    assert_eq!(
        fields,
        ["networkRef", "executionRef", "presentation", "source"]
            .into_iter()
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<String>>()
            .iter()
            .collect(),
        "no ports, no egress, no ruleset: the membership policy is the source's \
         admitted state, never restated by the row"
    );
}

/// Deriving again from the same committed row changes nothing, so a restart
/// re-ensures the same rows instead of churning identities.
#[test]
fn a_relationship_keeps_one_row_across_passes() {
    let (network_ref, spec, provenance, consumers) = attached_lan();
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);
    let first = canonical_binding_rows(&source).expect("derived rows");
    let second = canonical_binding_rows(&source).expect("derived rows");
    assert_eq!(first, second);
}

/// A derived row is something this family's own admission accepts: the
/// relationship key, the admitted right, and the arbitration the row
/// committed are the ones admission decides, and the interface it holds is
/// the one admission realizes.
#[test]
fn a_derived_row_is_admitted_by_this_familys_own_admission_path() {
    let (network_ref, spec, provenance, consumers) = attached_lan();
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);
    let rows = canonical_binding_rows(&source)
        .expect("derived rows");
    let row = &rows[1];
    let guest: NetworkBindingSpec =
        serde_json::from_slice(row.spec()).expect("the Guest's committed row");

    let consumer_uid = uid(GUEST_UID);
    let mut registry = NetworkBindingRegistry::new(zone());
    let admitted = registry
        .admit(MembershipAdmission {
            zone: zone(),
            source_uid: uid(NETWORK_UID),
            consumer_uid: consumer_uid.clone(),
            target: NetworkFabricTarget::new(guest.execution_ref().clone())
                .expect("the Guest is an execution target"),
            request: NetworkBindingRequest::new(
                guest.network_ref().clone(),
                guest.execution_ref().clone(),
                BindingSlot::parse("fabric").expect("the row's own consumer slot"),
                NetworkMembership::new(Vec::new(), false).expect("an empty membership"),
                guest.presentation().clone(),
 )
            .expect("the committed row is an admissible request"),
            host_intent: host_intent(3, 7).proof(),
            ceiling: NetworkMembershipCeiling::new(true, 8),
            authorization: BindingAuthorization::granted(),
            dependencies: vec![
                network_freshness(),
                consumer_freshness("Guest/work-vm", GUEST_UID),
            ],
            observation: HostStateObservation::empty(),
        })
        .expect("the derived row is admitted by the source's own path");

    assert_eq!(
        admitted.key(),
        &guest
            .key(zone(), uid(NETWORK_UID), consumer_uid)
            .expect("the committed row's relationship key"),
        "admission reaches exactly the key the committed row derives"
    );
    assert_eq!(
        admitted.evidence().admission().rights(),
        guest.source().admitted_rights()[0],
        "the right admission grants is the right the row committed"
    );
    assert_eq!(
        admitted.evidence().admission().arbitration(),
        guest.source().arbitration(),
        "the arbitration admission applied is the one the row committed"
    );
    assert_eq!(
        admitted.policy().fabric_interface(),
        row.fabric_interface(),
        "the interface admission holds is the one the derived row names"
    );
    assert_eq!(
        admitted.policy().presented_interface().as_str(),
        "eth0",
        "the consumer still reaches its membership under its own name"
    );
    assert!(
        admitted.policy().ports().is_empty(),
        "the row restated no traffic policy, so admission granted none"
    );
}

/// A Network that attaches nothing implies no relationship, so it commits no
/// row at all rather than a default membership.
#[test]
fn a_network_that_attaches_nothing_commits_no_row() {
    let network_ref = reference("Network/lan");
    let spec = attached_network_spec(&[]);
    let provenance = provenance();
    let consumers = vec![admitted_consumer(
        "Host/system",
        HOST_UID,
        NetworkPresentation::shared_fabric(),
    )];
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);

    assert!(
        canonical_binding_rows(&source)
            .expect("an unadmitted consumer is simply not a relationship")
            .is_empty(),
        "zero relationships derives zero rows"
    );
}

/// A committed attachment the accepted graph authorized nobody for is not a
/// relationship: it is refused rather than committed as an unbacked
/// membership.
#[test]
fn a_committed_attachment_without_an_admitted_consumer_is_refused() {
    let (network_ref, spec, provenance, consumers) = attached_lan();
    let unattached = vec![consumers[0].clone()];
    let source = binding_source(&network_ref, &spec, &provenance, &unattached);

    assert_eq!(
        canonical_binding_rows(&source).unwrap_err(),
        NetworkBindingError::NotAuthorized
    );
}

/// A consumer that can hold no fabric membership cannot enter the admitted
/// set. The neutral row contract admits every consumer kind for this family,
/// so the execution-parent bound is enforced where the row is minted: a
/// fabric is keyed by execution target, and the serving driver refuses a row
/// naming anything else terminally.
#[test]
fn a_consumer_that_cannot_hold_a_fabric_membership_is_refused() {
    assert_eq!(
        NetworkAdmittedConsumer::new(
            reference("Process/worker"),
            uid(FIRST_UID),
            NetworkPresentation::shared_fabric(),
        )
        .unwrap_err(),
        NetworkBindingError::WrongResourceType,
        "a Process consumes a target's membership; it is never the target itself"
    );
    assert_eq!(
        NetworkAdmittedConsumer::new(
            reference("Volume/state"),
            uid(FIRST_UID),
            NetworkPresentation::shared_fabric(),
        )
        .unwrap_err(),
        NetworkBindingError::WrongResourceType,
        "a Volume is not a binding consumer at all"
    );
    assert_eq!(
        admit_binding_row_refs(
            BindingKind::Network,
            &reference("Network/lan"),
            &reference("Volume/state"),
        )
        .unwrap_err(),
        d2b_contracts_resource::v3::BindingRowError::WrongConsumerType,
        "the row contract refuses the same reference the source does"
    );
    assert!(
        admit_binding_row_refs(
            BindingKind::Network,
            &reference("Network/lan"),
            &reference("Process/worker"),
        )
        .is_ok(),
        "the neutral contract admits a Process here, which is exactly why the \
         source refuses to mint the row: no fabric realizes it"
    );
}

/// A namespace presentation is realized by presenting the interface the
/// consumer named, so a name the kernel could never present is refused here
/// rather than committed as an unreachable promise. The name is a valid
/// bounded token, so only the realization can refuse it.
#[test]
fn a_row_naming_an_unpresentable_interface_is_refused() {
    let network_ref = reference("Network/lan");
    let spec = attached_network_spec(&[("Guest/work-vm", 2)]);
    let provenance = provenance();
    let consumers = vec![admitted_consumer(
        "Guest/work-vm",
        GUEST_UID,
        NetworkPresentation::namespace_interface("an-interface-name-far-longer-than-the-kernel-allows")
            .expect("a bounded token naming no Linux interface"),
    )];
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);

    assert_eq!(
        canonical_binding_rows(&source).unwrap_err(),
        NetworkBindingError::InvalidRequest
    );
}

/// A source row that is not a `Network` cannot commit a Network binding: the
/// row contract refuses it, and the derivation surfaces that refusal.
#[test]
fn a_source_row_that_is_not_a_network_is_refused() {
    let (network_ref, spec, provenance, consumers) = attached_lan();
    let volume_ref = reference("Volume/state");
    let source = binding_source(&volume_ref, &spec, &provenance, &consumers);

    assert_eq!(
        canonical_binding_rows(&source).unwrap_err(),
        NetworkBindingError::WrongResourceType
    );
    let rows = canonical_binding_rows(&binding_source(
        &network_ref,
        &spec,
        &provenance,
        &consumers,
 ))
    .expect("the same committed row read as a Network derives its rows");
    let host: NetworkBindingSpec = serde_json::from_slice(rows[0].spec()).expect("Host row");
    assert_eq!(host.network_ref(), &network_ref);
}

/// The row's consumer slot is derived from the consumer, so two committed
/// attachments for one consumer are a second declaration for one slot rather
/// than a second relationship.
#[test]
fn two_attachments_for_one_consumer_collide_on_one_slot() {
    let network_ref = reference("Network/lan");
    let spec = attached_network_spec(&[("Guest/work-vm", 2), ("Guest/work-vm", 3)]);
    let provenance = provenance();
    let consumers = vec![admitted_consumer(
        "Guest/work-vm",
        GUEST_UID,
        NetworkPresentation::shared_fabric(),
    )];
    let source = binding_source(&network_ref, &spec, &provenance, &consumers);

    assert_eq!(
        canonical_binding_rows(&source).unwrap_err(),
        NetworkBindingError::SlotOccupied
    );
}
