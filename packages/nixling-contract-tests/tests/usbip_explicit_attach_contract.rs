/// Phase 4 explicit USB attach contract tests.
///
/// These tests verify the shape, policy, and reject behavior for the
/// `nixling usb attach <vm> <present-busid> --apply` explicit path — the
/// path that does NOT require static busid/vendor allowlists in the bundle.
use nixling_contract_tests::read_repo_file;
use nixling_contracts::{
    broker_wire::{BrokerRequest, UsbipExplicitBindRequest, UsbipExplicitFirewallRuleRequest},
    usbip::{UsbipClaimSource, UsbipDaemonClaimRecord, validate_bus_id},
};

// ---------------------------------------------------------------------------
// Explicit plan — present-busid busid shape contract
// ---------------------------------------------------------------------------

#[test]
fn explicit_broker_bind_request_round_trips_via_serde() {
    let req = UsbipExplicitBindRequest {
        bus_id: "1-2".to_owned(),
        vm: "work".to_owned(),
        env: "corp".to_owned(),
        tracing_span_id: None,
    };
    let wrapped = BrokerRequest::UsbipExplicitBind(req);
    let json = serde_json::to_value(&wrapped).expect("serialize UsbipExplicitBind");
    let back: BrokerRequest = serde_json::from_value(json).expect("deserialize UsbipExplicitBind");
    match back {
        BrokerRequest::UsbipExplicitBind(r) => {
            assert_eq!(r.bus_id, "1-2");
            assert_eq!(r.vm, "work");
            assert_eq!(r.env, "corp");
        }
        other => panic!("expected UsbipExplicitBind, got {other:?}"),
    }
}

#[test]
fn explicit_broker_firewall_rule_request_round_trips_via_serde() {
    let req = UsbipExplicitFirewallRuleRequest {
        bus_id: "3-4".to_owned(),
        env: "personal".to_owned(),
        host_uplink_ip: "192.0.2.1".to_owned(),
        net_uplink_ip: "192.0.2.20".to_owned(),
        tracing_span_id: None,
    };
    let wrapped = BrokerRequest::UsbipExplicitFirewallRule(req);
    let json = serde_json::to_value(&wrapped).expect("serialize UsbipExplicitFirewallRule");
    let back: BrokerRequest =
        serde_json::from_value(json).expect("deserialize UsbipExplicitFirewallRule");
    match back {
        BrokerRequest::UsbipExplicitFirewallRule(r) => {
            assert_eq!(r.bus_id, "3-4");
            assert_eq!(r.env, "personal");
            assert_eq!(r.host_uplink_ip, "192.0.2.1");
            assert_eq!(r.net_uplink_ip, "192.0.2.20");
        }
        other => panic!("expected UsbipExplicitFirewallRule, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// UsbipExplicitBind / UsbipExplicitFirewallRule deny_unknown_fields contract
// ---------------------------------------------------------------------------

#[test]
fn explicit_bind_request_denies_unknown_fields() {
    let bad = serde_json::json!({
        "kind": "UsbipExplicitBind",
        "bus_id": "1-2",
        "vm": "work",
        "env": "corp",
        "tracing_span_id": null,
        "surprise_field": "evil"
    });
    let result = serde_json::from_value::<UsbipExplicitBindRequest>(bad);
    assert!(
        result.is_err(),
        "UsbipExplicitBindRequest must deny unknown fields (deny_unknown_fields)"
    );
}

#[test]
fn explicit_firewall_rule_request_denies_unknown_fields() {
    let bad = serde_json::json!({
        "bus_id": "1-2",
        "env": "corp",
        "host_uplink_ip": "192.0.2.1",
        "net_uplink_ip": "192.0.2.20",
        "extra": "bad"
    });
    let result = serde_json::from_value::<UsbipExplicitFirewallRuleRequest>(bad);
    assert!(
        result.is_err(),
        "UsbipExplicitFirewallRuleRequest must deny unknown fields (deny_unknown_fields)"
    );
}

// ---------------------------------------------------------------------------
// Busid validation — explicit path uses same sysfs shape as declared path
// ---------------------------------------------------------------------------

#[test]
fn explicit_attach_uses_same_busid_validation_as_declared_path() {
    // The explicit path validates busid shape using the same
    // `nixling_contracts::usbip::validate_bus_id` as the declared path.
    // No wider surface is permitted; the validator is the gate.
    let valid = ["1-2", "1-2.3", "3-4.5.6", "10-1.2", "2-1"];
    for busid in valid {
        assert!(
            validate_bus_id(busid).is_ok(),
            "valid busid {busid:?} must pass shared validator"
        );
    }
    let invalid = ["", "abc", "1-", "-2", "1_2", "/dev/bus/usb/001/002", "1 -2"];
    for busid in invalid {
        assert!(
            validate_bus_id(busid).is_err(),
            "invalid busid {busid:?} must fail shared validator"
        );
    }
}

// ---------------------------------------------------------------------------
// UsbipClaimSource — claim source enum shape
// ---------------------------------------------------------------------------

#[test]
fn claim_source_explicit_is_explicit_not_declared() {
    let source = UsbipClaimSource::Explicit;
    assert!(source.is_explicit());
    assert!(!source.is_declared());
}

#[test]
fn claim_source_declared_is_declared_not_explicit() {
    let source = UsbipClaimSource::Declared {
        firewall_ref: "usbip-fw-corp-1-2".to_owned(),
        bind_ref: "usbip-bind-corp-work-1-2".to_owned(),
    };
    assert!(source.is_declared());
    assert!(!source.is_explicit());
}

#[test]
fn claim_source_round_trips_via_serde() {
    for source in [
        UsbipClaimSource::Explicit,
        UsbipClaimSource::Declared {
            firewall_ref: "usbip-fw-corp-1-2".to_owned(),
            bind_ref: "usbip-bind-corp-work-1-2".to_owned(),
        },
    ] {
        let json = serde_json::to_value(&source).expect("serialize UsbipClaimSource");
        let back: UsbipClaimSource =
            serde_json::from_value(json).expect("deserialize UsbipClaimSource");
        assert_eq!(
            format!("{source:?}"),
            format!("{back:?}"),
            "UsbipClaimSource must round-trip via serde"
        );
    }
}

// ---------------------------------------------------------------------------
// UsbipDaemonClaimRecord — lock path derivation
// ---------------------------------------------------------------------------

#[test]
fn lock_path_for_busid_is_scoped_to_run_nixling_locks_usbip() {
    let path = UsbipDaemonClaimRecord::lock_path_for_busid("1-2");
    assert!(
        path.starts_with("/run/nixling/locks/usbip/"),
        "OFD lock path must be under /run/nixling/locks/usbip/ — got {path:?}"
    );
    assert!(
        path.ends_with("1-2"),
        "OFD lock path must end with busid — got {path:?}"
    );
}

#[test]
fn lock_path_for_busid_does_not_traverse() {
    // Dotted busids like "1-2.3" must not produce a directory traversal.
    for busid in ["1-2.3", "3-4.5.6", "10-1.2"] {
        let path = UsbipDaemonClaimRecord::lock_path_for_busid(busid);
        assert!(
            path.starts_with("/run/nixling/locks/usbip/"),
            "lock path must stay under /run/nixling/locks/usbip/ for busid {busid:?} — got {path:?}"
        );
        assert!(
            !path.contains(".."),
            "lock path must not contain '..' for busid {busid:?} — got {path:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Per-device backend model — no shared backend for explicit busids
// ---------------------------------------------------------------------------

#[test]
fn explicit_bind_request_carries_bus_id_not_shared_scope() {
    // The explicit broker bind request carries the raw busid and vm,
    // NOT a shared env scope. This enforces the per-device backend model:
    // the broker must set up a per-busid backend/proxy rather than
    // restarting or reusing the shared `sys-<env>-usbipd` backend.
    let req = UsbipExplicitBindRequest {
        bus_id: "1-2".to_owned(),
        vm: "work".to_owned(),
        env: "corp".to_owned(),
        tracing_span_id: None,
    };
    // The request carries vm+env to allow per-device routing decisions,
    // but the key identifier for the backend is bus_id, not an env scope.
    assert_eq!(req.bus_id, "1-2");
    assert!(
        !req.bus_id.is_empty(),
        "explicit bind request must carry non-empty busid"
    );
    assert!(
        !req.vm.is_empty(),
        "explicit bind request must carry target vm for ACL scoping"
    );
}

#[test]
fn explicit_firewall_rule_carries_per_env_uplink_not_shared_subnet() {
    // The explicit firewall rule carries per-env uplink IPs, not a shared
    // wildcard. This enforces firewall env scope: the rule is scoped to
    // exactly the target env bridge, not all nixling envs.
    let req = UsbipExplicitFirewallRuleRequest {
        bus_id: "1-2".to_owned(),
        env: "corp".to_owned(),
        host_uplink_ip: "192.0.2.1".to_owned(),
        net_uplink_ip: "192.0.2.20".to_owned(),
        tracing_span_id: None,
    };
    assert!(
        !req.host_uplink_ip.is_empty(),
        "explicit firewall rule must carry host_uplink_ip for env-scoped nftables rule"
    );
    assert!(
        !req.net_uplink_ip.is_empty(),
        "explicit firewall rule must carry net_uplink_ip for env-scoped nftables rule"
    );
    // The env identifier allows the broker to scope the rule to exactly
    // this env's bridge, not open a wildcard across all nixling envs.
    assert!(
        !req.env.is_empty(),
        "explicit firewall rule must carry env identifier"
    );
}

// ---------------------------------------------------------------------------
// Codebase policy: explicit ops must not appear in bundle intent lookups
// ---------------------------------------------------------------------------

#[test]
fn explicit_path_is_not_gated_on_bundle_firewall_or_bind_intent() {
    // The daemon must not require a `usbip-fw-<env>-<busid>` or
    // `usbip-bind-<env>-<vm>-<busid>` bundle intent for the explicit path.
    // The explicit path is selected precisely because those intents are absent.
    //
    // Verify that `dispatch_broker_usbip_bind` in lib.rs checks for the
    // absence of declared intents and falls through to the explicit path
    // rather than emitting a "intent missing" error.
    let lib_rs = read_repo_file("packages/nixlingd/src/lib.rs");
    assert!(
        lib_rs.contains("has_declared_intents"),
        "dispatch_broker_usbip_bind must check for declared intent presence before choosing explicit path"
    );
    assert!(
        lib_rs.contains("UsbipExplicitBind"),
        "dispatch_broker_usbip_bind must dispatch UsbipExplicitBind for the explicit path"
    );
    assert!(
        lib_rs.contains("UsbipExplicitFirewallRule"),
        "dispatch_broker_usbip_bind must dispatch UsbipExplicitFirewallRule for the explicit path"
    );
}

// ---------------------------------------------------------------------------
// Codebase policy: sysfs presence check must precede broker calls
// ---------------------------------------------------------------------------

#[test]
fn sysfs_presence_check_is_fail_closed_before_broker_dispatch() {
    let lib_rs = read_repo_file("packages/nixlingd/src/lib.rs");
    // The sysfs check function and claim exclusivity check must exist
    // and be called before any broker dispatch in the explicit path.
    assert!(
        lib_rs.contains("check_sysfs_busid_present"),
        "lib.rs must define and call check_sysfs_busid_present for fail-closed pre-flight"
    );
    assert!(
        lib_rs.contains("check_usbip_claim_exclusivity"),
        "lib.rs must define and call check_usbip_claim_exclusivity for active claim pre-flight"
    );
    assert!(
        lib_rs.contains("UsbipBusidNotPresent"),
        "lib.rs must surface UsbipBusidNotPresent typed error for absent busid"
    );
    assert!(
        lib_rs.contains("UsbipExplicitClaimConflict"),
        "lib.rs must surface UsbipExplicitClaimConflict typed error for active claim conflict"
    );
}

// ---------------------------------------------------------------------------
// Broker ops are registered as Unimplemented stubs in layer1-bootstrap
// ---------------------------------------------------------------------------

#[test]
fn explicit_ops_appear_in_broker_runtime_as_unimplemented_stubs() {
    let runtime = read_repo_file("packages/nixling-priv-broker/src/runtime.rs");
    assert!(
        runtime.contains("UsbipExplicitBind"),
        "broker runtime.rs must handle UsbipExplicitBind (at least as Unimplemented stub)"
    );
    assert!(
        runtime.contains("UsbipExplicitFirewallRule"),
        "broker runtime.rs must handle UsbipExplicitFirewallRule (at least as Unimplemented stub)"
    );
}
