use d2b_contract_tests::read_repo_file;

#[test]
fn usbip_provider_binding_is_realm_and_resource_scoped() {
    let provider = read_repo_file("nixos-modules/provider-registry-v2-extensions/device.nix");
    let devices = read_repo_file("nixos-modules/realm-device-rows.nix");

    for required in [
        r#"axis = "local-device";"#,
        r#"deviceResourceIds = lib.sort lib.lessThan"#,
        r#"row.realmId == provider.realmId"#,
        r#"row.providerId == provider.providerId"#,
        r#"placement = {"#,
        r#"inherit realmId controllerRole;"#,
    ] {
        assert!(
            provider.contains(required),
            "device provider registry missing scoped binding {required:?}"
        );
    }
    assert!(
        devices.contains(r#"broker = "realm-local";"#)
            && devices.contains(r#"source = {"#)
            && devices.contains(r#"kind = "realm-broker";"#),
        "USBIP allocation must remain under the owning realm broker"
    );
}

#[test]
fn usbip_host_firewall_builder_remains_fail_closed() {
    let resolver = read_repo_file("packages/d2b-core/src/bundle_resolver.rs");
    assert!(resolver.contains("fn scoped_usbip_proxy_rule_body"));
    assert!(resolver.contains("ip saddr {net_uplink_ip} ip daddr {host_uplink_ip}"));
    for required in [
        "!uplink_flags.isolated",
        "!uplink_flags.neigh_suppress",
        "uplink_flags.resolved_learning()",
        "uplink_flags.resolved_unicast_flood()",
    ] {
        assert!(
            resolver.contains(required),
            "host USBIP firewall validation missing {required}"
        );
    }
}
