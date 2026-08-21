use d2b_contract_tests::read_repo_file;

#[test]
fn usbip_proxy_listener_is_per_env_host_uplink_only() {
    let processes = read_repo_file("nixos-modules/processes-json.nix");
    assert!(
        processes
            .contains(r#"TCP-LISTEN:3240,bind=${m.hostUplinkIp},fork,max-children=4,reuseaddr"#),
        "USBIP proxy must bind the per-env host uplink IP, not a wildcard or shared listener"
    );
    for forbidden in [
        "TCP-LISTEN:3240,fork",
        "TCP-LISTEN:3240,reuseaddr",
        "bind=0.0.0.0",
        "bind=::",
    ] {
        assert!(
            !processes.contains(forbidden),
            "USBIP proxy listener must not contain {forbidden:?}"
        );
    }
}

#[test]
fn usbip_firewall_carveout_uses_host_visible_env_identity() {
    let resolver = read_repo_file("packages/d2b-core/src/bundle_resolver.rs");
    assert!(
        resolver.contains("fn scoped_usbip_proxy_rule_body"),
        "USBIP firewall intent builder must centralize scoped rule validation"
    );
    assert!(
        resolver.contains("ip saddr {net_uplink_ip} ip daddr {host_uplink_ip}"),
        "USBIP firewall carve-out must key on the host-visible net-VM source and host bridge destination"
    );
    for required in [
        "!uplink_flags.isolated",
        "!uplink_flags.neigh_suppress",
        "uplink_flags.resolved_learning()",
        "uplink_flags.resolved_unicast_flood()",
    ] {
        assert!(
            resolver.contains(required),
            "USBIP firewall must fail closed when uplink anti-spoofing validation is absent: missing {required}"
        );
    }
}
