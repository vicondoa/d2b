use nixling_contract_tests::read_repo_file;

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
    let resolver = read_repo_file("packages/nixling-core/src/bundle_resolver.rs");
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

#[test]
fn usbip_proxy_sync_strategy_does_not_assume_busid_aware_l4_proxy() {
    let state = read_repo_file("packages/nixlingd/src/usbip_reconcile_state.rs");
    let component_doc = read_repo_file("docs/reference/components-usbip.md");
    let state_machine_doc = read_repo_file("docs/reference/usbip-state-machine.md");

    for required in [
        "UsbipProxySynchronizationPlan",
        "OptimisticBackendExportRefresh",
        "FailClosedRevocationNotIsolated",
        "PreserveSameEnvStreams",
        "AcquireExclusiveSocketLifecycleLock",
        "RebindProxyListenerFdRelative",
    ] {
        assert!(
            state.contains(required),
            "USBIP proxy synchronization strategy must encode {required}"
        );
    }
    assert!(
        component_doc.contains("generic L4 TCP forwarder")
            && state_machine_doc.contains("current generic L4 proxy strategy"),
        "USBIP docs must state that the current proxy is generic L4, not busid-aware"
    );
    for forbidden in [
        "stop other proxies",
        "steals the lock",
        "selectively close one busid stream by itself",
    ] {
        assert!(
            !component_doc.contains(forbidden),
            "USBIP component doc must not claim busid-aware proxy behaviour: {forbidden:?}"
        );
    }
}
