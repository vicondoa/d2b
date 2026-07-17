//! Tests for the static security/policy invariants, migrated from
//! `tests/static-invariant-{world-readable-leak,opaque-key-ids,broad-caps,
//! writable-paths}.sh`.
//!
//! Two layers:
//!   * **synthetic** — the original positive/negative `jq` fixtures, now driving
//!     the typed `d2b_core::static_invariants` validators directly. These
//!     live here (not as `d2b-core` `#[cfg(test)]` unit tests) because
//!     `d2b-core` sets `[lib] test = false` (its libtest surface is the
//!     custom smoke harness), so in-crate unit tests would not run in the gate.
//!     The contract crate IS gated (the D2B_FIXTURES step in
//!     `tests/tools/rust-workspace-checks.sh`), and these synthetic cases need no
//!     fixture, so they run there unconditionally.
//!   * **rendered** — the validators run against the REAL rendered fixture-smoke
//!     artifacts (`manifest.json` / bundle profiles), a strictly stronger
//!     guarantee than the bash grep over a synthetic fixture.

use d2b_contract_tests::{load_bundle_resolver_from_env, load_manifest_value_from_env};
use d2b_core::static_invariants::{
    is_broad_cap_violation, path_bearing_key_violations, undeclared_writable_paths,
    world_readable_field_leaks,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Synthetic logic cases (ported positive/negative fixtures).
// ---------------------------------------------------------------------------

fn caps(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn world_readable_positive_fixture_accepted() {
    let manifest = json!({
        "_manifest": { "manifestVersion": 4 },
        "corp-vm": {
            "name": "corp-vm", "env": "work", "index": 10, "sshUser": "alice",
            "sshPort": 22, "ipv4": "10.20.0.10", "mac": "02:00:00:00:00:0a",
            "isNetVm": false
        }
    });
    assert!(world_readable_field_leaks(&manifest).is_empty());
}

#[test]
fn world_readable_negative_fixture_rejected() {
    let manifest = json!({
        "corp-vm": { "name": "corp-vm", "privateKeyPath": "/var/lib/d2b/vms/corp-vm/id_ed25519" }
    });
    assert_eq!(
        world_readable_field_leaks(&manifest),
        vec!["corp-vm.privateKeyPath".to_owned()]
    );
}

#[test]
fn world_readable_exempts_manifest_and_observability_blocks() {
    let manifest = json!({
        "_manifest": { "manifestVersion": 6, "anythingInHere": "is-fine" },
        "_observability": { "enabled": false, "obsVsockCid": 1000 }
    });
    assert!(world_readable_field_leaks(&manifest).is_empty());
}

#[test]
fn opaque_key_ids_positive_fixture_accepted() {
    let manifest = json!({
        "keys": { "ssh": { "key_id": "corp-vm-host-key" } },
        "secrets": [ { "secret_id": "api-token" } ]
    });
    assert!(path_bearing_key_violations(&manifest).is_empty());
}

#[test]
fn opaque_key_ids_negative_fixture_rejected() {
    let manifest = json!({
        "keys": { "ssh": { "privateKeyPath": "/var/lib/d2b/vms/corp-vm/id_ed25519" } },
        "secret_path": "/run/secrets/token"
    });
    let mut violations = path_bearing_key_violations(&manifest);
    violations.sort();
    assert_eq!(
        violations,
        vec![
            "keys.ssh.privateKeyPath=/var/lib/d2b/vms/corp-vm/id_ed25519".to_owned(),
            "secret_path=/run/secrets/token".to_owned(),
        ]
    );
}

#[test]
fn opaque_key_ids_ignores_path_keys_without_slash_value() {
    // A keyPath-suffixed field whose value is an opaque id (no slash) is OK.
    let manifest = json!({ "keys": { "ssh": { "keyPath": "corp-vm-host-key" } } });
    assert!(path_bearing_key_violations(&manifest).is_empty());
}

#[test]
fn broad_caps_positive_fixture_accepted() {
    // tap-broker holds CAP_NET_ADMIN but carries an ADR carve-out.
    assert!(!is_broad_cap_violation(
        &caps(&["CAP_NET_ADMIN"]),
        Some("ADR 0004")
    ));
}

#[test]
fn broad_caps_negative_fixture_rejected() {
    // runner holds CAP_SYS_ADMIN with no carve-out.
    assert!(is_broad_cap_violation(&caps(&["CAP_SYS_ADMIN"]), None));
}

#[test]
fn broad_caps_empty_carve_out_is_no_carve_out() {
    assert!(is_broad_cap_violation(
        &caps(&["CAP_NET_ADMIN"]),
        Some("   ")
    ));
}

#[test]
fn broad_caps_narrow_caps_need_no_carve_out() {
    assert!(!is_broad_cap_violation(&caps(&["CAP_SETUID"]), None));
}

#[test]
fn writable_paths_positive_fixture_accepted() {
    let declared = ["/var/lib/d2b/vms/corp-vm"];
    let used = ["/var/lib/d2b/vms/corp-vm"];
    assert!(undeclared_writable_paths(declared, used).is_empty());
}

#[test]
fn writable_paths_negative_fixture_rejected() {
    let declared = ["/var/lib/d2b/vms/corp-vm"];
    let used = ["/run/secrets"];
    assert_eq!(
        undeclared_writable_paths(declared, used),
        vec!["/run/secrets".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// Rendered-artifact contract cases (run the validators on the REAL fixture).
// ---------------------------------------------------------------------------

#[test]
fn rendered_manifest_exposes_only_public_safe_fields() {
    let manifest = load_manifest_value_from_env();
    let leaks = world_readable_field_leaks(&manifest);
    assert!(
        leaks.is_empty(),
        "rendered vms.json exposes non-allowlisted fields: {leaks:?}"
    );
}

#[test]
fn rendered_manifest_has_no_path_bearing_key_or_secret_fields() {
    let manifest = load_manifest_value_from_env();
    let violations = path_bearing_key_violations(&manifest);
    assert!(
        violations.is_empty(),
        "rendered vms.json leaks path-bearing key/secret fields: {violations:?}"
    );
}

#[test]
fn rendered_bundle_and_manifest_preserve_v12_v7_canonical_projection() {
    let resolver = load_bundle_resolver_from_env();
    assert_eq!(resolver.bundle.bundle_version, 12);
    assert_eq!(resolver.bundle.schema_version, "v2");
    assert_eq!(resolver.manifest.manifest.manifest_version, 7);
    assert!(
        !resolver.manifest.vms.is_empty(),
        "fixture manifest must exercise at least one realm workload"
    );
    for (workload_id, workload) in &resolver.manifest.vms {
        assert_eq!(
            workload_id, &workload.name,
            "manifest dynamic keys must remain canonical workload IDs"
        );
    }
}

#[test]
fn rendered_profiles_with_broad_caps_carry_an_adr_carve_out() {
    let resolver = load_bundle_resolver_from_env();
    let mut violations = Vec::new();
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            let p = &node.profile;
            if is_broad_cap_violation(&p.caps, p.adr_carve_out.as_deref()) {
                violations.push(format!("{} (vm {})", p.profile_id, dag.vm));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "process roles request broad capabilities without an ADR carve-out: {violations:?}"
    );
}
