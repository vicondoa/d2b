use d2b_contracts::{
    broker_wire::{
        ApplyNftablesProjectionRequest, CreateBridgeRequest, DeleteBridgeRequest,
        DeletePersistentTapRequest, NftablesProjectionAction,
    },
    v3::{ResourceName, ResourceUid},
};
use d2b_provider_network_local::{
    ExternalNicAdmissionError, ExternalNicClaim, MacvtapMode, SharingPolicy,
    admit_external_nic_claims,
    bridge_port::{BridgePortFlagSet, TapRole, validate_readback},
    ifname::{IfName, IfNameMapping, NetworkIfRole, derive_ifname, detect_collisions},
    netlink::{LinkKind, LinkSpec, NetlinkError},
    nftables::{
        NetworkNftProjection, NftablesError, SharedNftTable, SharedTableEntry, apply_projection,
    },
    routes::{LinkClass, RoutePreflightError, RouteRow},
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn adapted_ifname_derivation_is_deterministic_and_collision_checked() {
    let first = derive_ifname("work", NetworkIfRole::LanBridge, None, None).unwrap();
    let second = derive_ifname("work", NetworkIfRole::LanBridge, None, None).unwrap();
    assert_eq!(first, second);

    let mappings = [
        IfNameMapping::new(
            ResourceName::parse("work").unwrap(),
            None,
            NetworkIfRole::LanBridge,
            first.clone(),
        ),
        IfNameMapping::new(
            ResourceName::parse("personal").unwrap(),
            None,
            NetworkIfRole::LanBridge,
            first,
        ),
    ];
    assert!(detect_collisions(&mappings).is_err());
}

#[test]
fn bridge_port_readback_rejects_one_flag_of_drift() {
    let role = TapRole::WorkloadLanIsolated;
    let mut observed = BridgePortFlagSet::defaults_for(role);
    observed.isolated = false;
    assert!(validate_readback(role, observed).is_err());
}

#[test]
fn firewall_target_marker_conflict_preserves_original_bytes() {
    let owner = uid("123e4567-e89b-42d3-a456-426614174000");
    let bytes = b"foreign marker payload".to_vec();
    let snapshot = SharedNftTable::new(vec![SharedTableEntry::foreign_in_network_slot(
        owner.clone(),
        bytes.clone(),
    )]);
    assert_eq!(
        apply_projection(&snapshot, &NetworkNftProjection::empty(owner)).unwrap_err(),
        NftablesError::ForeignMarkerPreserved
    );
    assert_eq!(snapshot.entries()[0].bytes(), bytes);
}

#[test]
fn cross_zone_bridge_multiplex_is_rejected_before_any_provider_effect() {
    let work = uid("123e4567-e89b-42d3-a456-426614174000");
    let personal = uid("223e4567-e89b-42d3-a456-426614174001");
    let claims = [
        ExternalNicClaim::new(work, MacvtapMode::Bridge, SharingPolicy::Multiplexed),
        ExternalNicClaim::new(personal, MacvtapMode::Bridge, SharingPolicy::Multiplexed),
    ];
    assert_eq!(
        admit_external_nic_claims(&claims, 2),
        Err(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2)
    );
}

#[test]
fn required_closed_broker_operation_types_are_available() {
    use core::any::TypeId;

    let types = [
        TypeId::of::<CreateBridgeRequest>(),
        TypeId::of::<DeleteBridgeRequest>(),
        TypeId::of::<DeletePersistentTapRequest>(),
        TypeId::of::<ApplyNftablesProjectionRequest>(),
    ];
    assert_eq!(types.len(), 4);
    assert_ne!(
        NftablesProjectionAction::Apply,
        NftablesProjectionAction::Remove
    );
}

#[test]
fn diagnostics_redact_interface_address_uid_and_payload_canaries() {
    let interface = "secret-iface";
    let address = "198.51.100.77/32";
    let payload = "secret-rule-payload";
    let owner = uid("123e4567-e89b-42d3-a456-426614174000");
    let link = LinkSpec::new(IfName::parse(interface).unwrap(), LinkKind::Bridge, None);
    let route = RouteRow::new(address, LinkClass::HostLan, true);
    let entry = SharedTableEntry::foreign(payload.as_bytes().to_vec());
    let diagnostics = format!(
        "{link:?} {route:?} {entry:?} {owner:?} {:?} {:?}",
        NetlinkError::Backend,
        RoutePreflightError::ForeignDefaultRoute,
    );
    for canary in [interface, address, payload, owner.as_str()] {
        assert!(!diagnostics.contains(canary));
    }
}
