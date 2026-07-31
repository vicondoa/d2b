use d2b_contracts::v3::ResourceName;
use d2b_provider_network_local::ifname::{
    IfName, IfNameError, IfNameMapping, NetworkIfRole, derive_ifname, detect_collisions,
};

fn name(value: &str) -> ResourceName {
    ResourceName::parse(value).unwrap()
}

#[test]
fn ifname_too_long_rejected() {
    assert_eq!(IfName::parse("abcdefghijklmnop"), Err(IfNameError::TooLong));
}

#[test]
fn ifname_invalid_character_rejected() {
    assert_eq!(IfName::parse("br work"), Err(IfNameError::InvalidCharacter));
}

#[test]
fn derivation_is_deterministic() {
    let first = derive_ifname("work", NetworkIfRole::WorkloadGuestTap, Some("vm"), None).unwrap();
    let second = derive_ifname("work", NetworkIfRole::WorkloadGuestTap, Some("vm"), None).unwrap();
    assert_eq!(first, second);
}

#[test]
fn role_distinguishes_bridge_vs_tap() {
    let bridge = derive_ifname("work", NetworkIfRole::LanBridge, None, None).unwrap();
    let tap = derive_ifname("work", NetworkIfRole::NetVmLanTap, None, None).unwrap();
    assert_ne!(bridge, tap);
}

#[test]
fn vm_changes_derivation() {
    let first = derive_ifname("work", NetworkIfRole::WorkloadGuestTap, Some("vm-a"), None).unwrap();
    let second =
        derive_ifname("work", NetworkIfRole::WorkloadGuestTap, Some("vm-b"), None).unwrap();
    assert_ne!(first, second);
}

#[test]
fn detect_collisions_flags_duplicate_bridge() {
    let duplicate = IfName::parse("d2b-bAAAAAAAA").unwrap();
    let mappings = [
        IfNameMapping::new(
            name("work"),
            None,
            NetworkIfRole::LanBridge,
            duplicate.clone(),
        ),
        IfNameMapping::new(name("personal"), None, NetworkIfRole::LanBridge, duplicate),
    ];
    assert_eq!(detect_collisions(&mappings), Err(IfNameError::Collision));
}

#[test]
fn detect_collisions_flags_bridge_vs_tap() {
    let duplicate = IfName::parse("d2b-bAAAAAAAA").unwrap();
    let mappings = [
        IfNameMapping::new(
            name("work"),
            None,
            NetworkIfRole::LanBridge,
            duplicate.clone(),
        ),
        IfNameMapping::new(
            name("personal"),
            Some(name("vm")),
            NetworkIfRole::WorkloadGuestTap,
            duplicate,
        ),
    ];
    assert_eq!(detect_collisions(&mappings), Err(IfNameError::Collision));
}

#[test]
fn detect_collisions_passes_unique_set() {
    let mappings = [
        IfNameMapping::new(
            name("work"),
            None,
            NetworkIfRole::LanBridge,
            derive_ifname("work", NetworkIfRole::LanBridge, None, None).unwrap(),
        ),
        IfNameMapping::new(
            name("personal"),
            None,
            NetworkIfRole::LanBridge,
            derive_ifname("personal", NetworkIfRole::LanBridge, None, None).unwrap(),
        ),
    ];
    detect_collisions(&mappings).unwrap();
}
