use d2b_contracts::v3::{
    execution_policy::BoundedToken,
    network::{Ipv4Cidr, NetworkSpec},
};
use d2b_provider_network_local::{
    artifact::{ArtifactCatalogEntry, ArtifactKind, resolve_net_vm_system_artifact},
    controller::{config_volume_spec, guest_agent_process_spec, render_config},
};

fn spec(lan: &str, uplink: &str) -> NetworkSpec {
    NetworkSpec::minimal(
        Ipv4Cidr::parse(lan).unwrap(),
        Ipv4Cidr::parse(uplink).unwrap(),
        BoundedToken::parse("net-vm-base").unwrap(),
    )
    .unwrap()
}

#[test]
fn two_networks_share_system_artifact_and_diverge_only_in_volume_content() {
    let first = spec("10.20.0.0/24", "192.0.2.0/30");
    let second = spec("10.30.0.0/24", "198.51.100.0/30");
    let catalog = [ArtifactCatalogEntry::new(
        BoundedToken::parse("net-vm-base").unwrap(),
        ArtifactKind::NixosSystem,
    )];
    assert_eq!(
        resolve_net_vm_system_artifact(&first, &catalog).unwrap(),
        resolve_net_vm_system_artifact(&second, &catalog).unwrap()
    );
    assert_ne!(
        render_config(&first).unwrap(),
        render_config(&second).unwrap()
    );
}

#[test]
fn typed_volume_and_agent_shapes_match_the_contract() {
    let volume = config_volume_spec("host-system", None).unwrap();
    assert_eq!(volume.layout().len(), 5);
    assert!(volume.attachments().is_empty());
    assert_eq!(volume.quota().unwrap().max_bytes(), Some(4 * 1024 * 1024));
    assert_eq!(
        volume.views()["guest-readonly"].rights(),
        [
            d2b_contracts::v3::volume::ViewRight::Read,
            d2b_contracts::v3::volume::ViewRight::Traverse,
        ]
    );
    let agent = guest_agent_process_spec("net-vm").unwrap();
    assert_eq!(
        agent.execution().process_class(),
        d2b_contracts::v3::process::ProcessClass::Worker
    );
    assert_eq!(agent.execution().sandbox().namespace_classes(), []);
    assert_eq!(
        agent.execution().sandbox().capability_classes(),
        [
            d2b_contracts::v3::process::CapabilityClass::NetworkAdmin,
            d2b_contracts::v3::process::CapabilityClass::NetworkBind,
            d2b_contracts::v3::process::CapabilityClass::NetworkRaw,
        ]
    );
    assert_eq!(agent.execution().mounts().len(), 1);
    assert_eq!(
        agent.execution().mounts()[0].access(),
        d2b_contracts::v3::process::MountAccess::ReadOnly
    );
    assert!(agent.execution().mounts()[0].required());
}

#[test]
fn rendered_config_keeps_the_mandatory_host_blocklist() {
    let content = render_config(&spec("10.20.0.0/24", "192.0.2.0/30")).unwrap();
    let firewall = String::from_utf8(content.nftables).unwrap();
    for cidr in d2b_contracts::v3::network::DEFAULT_HOST_BLOCKLIST {
        assert!(firewall.contains(cidr));
    }
}

#[test]
fn generic_nix_module_preserves_boot_safety_and_excludes_network_desired_data() {
    let module = include_str!("../nix/net-vm.nix");
    assert!(module.contains("\"10-eth-dhcp\" = lib.mkForce"));
    assert!(module.matches("matchConfig.MACAddress").count() >= 5);
    assert!(!module.contains("uplinkMac = lib.mkOption"));
    assert!(!module.contains("lanMac = lib.mkOption"));
    assert!(module.contains("net.ipv6.conf.eth0.disable_ipv6"));
    assert!(module.contains("table ip6 filter"));
    for forbidden in [
        "services.dnsmasq",
        "services.avahi",
        "dhcp-host",
        "masquerade",
        "hostBlocklist",
        "portForward",
        "attachments.json",
    ] {
        assert!(
            !module.contains(forbidden),
            "generic module contains {forbidden}"
        );
    }
}
