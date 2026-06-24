//! W3 CLI-contract integration test, migrated from
//! tests/cli-rust-native-list.sh.
//!
//! Spawns the real `nixling` binary against the rendered fixture-smoke bundle
//! (NL_FIXTURES) + a synthetic systemd/bridge state fixture, and asserts that
//! `list --json`:
//!   * deserializes strictly into `nixling_contracts::cli_output::ListOutputV2`
//!     (`deny_unknown_fields` on `ListItemOutputV2` makes this the schema
//!     check the bash gate did via docs/reference/cli-output/list.schema.json);
//!   * returns the expected smoke inventory (2 VMs; corp-vm is the workload
//!     VM, sys-work-net is the running auto-declared net VM, both
//!     runner-parity-OK against their committed closures).
//!
//! Requires NL_FIXTURES (the fixture-smoke output dir), delivered by the
//! dedicated CLI-contract step in tests/tools/rust-workspace-checks.sh. When unset
//! (e.g. the plain `cargo test --workspace` pass that has no Nix sandbox) the
//! test skips; the gate step always sets NL_FIXTURES, so the contract cannot be
//! silently disabled there.

use std::process::Command;

use nixling_contracts::cli_output::ListOutputV2;

// Mirrors tests/cli-rust-native-common.sh nl_write_system_state_fixture, but
// also pins nixlingd.service (the bash helper omitted it, so the CLI fell back
// to the real host's `systemctl is-active nixlingd.service` — non-hermetic; see
// tests/README.md). corp-vm: all units inactive + an empty daemon-state dir
// (pidfd-table.json absent -> ch-runner "stopped") -> status "stopped".
// sys-work-net: net VM -> always "running".
const SYSTEM_STATE_JSON: &str = r#"{
  "units": {
    "nixlingd.service": "inactive",
    "nixling@corp-vm.service": "inactive",
    "microvm@corp-vm.service": "inactive",
    "microvm-virtiofsd@corp-vm.service": "inactive",
    "nixling@sys-work-net.service": "active",
    "microvm@sys-work-net.service": "active",
    "microvm-virtiofsd@sys-work-net.service": "active"
  },
  "bridges": {
    "br-work-lan": { "state": "UP", "admin": "up", "expectedCarrier": "NO-CARRIER", "result": "ok" },
    "br-work-up":  { "state": "UP", "admin": "up", "expectedCarrier": "UP", "result": "ok" }
  }
}"#;

/// The fixture-smoke output dir, or `None` when NL_FIXTURES is unset (plain
/// non-gated `cargo test` runs). The gated rust-workspace-checks.sh step always
/// sets it.
fn fixtures_dir() -> Option<String> {
    std::env::var("NL_FIXTURES").ok()
}

#[test]
fn list_json_matches_smoke_inventory_and_schema() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let sys = tmp.path().join("system-state.json");
    std::fs::write(&sys, SYSTEM_STATE_JSON).expect("write system-state fixture");
    // Sandbox the daemon-state dir to an empty dir so pidfd-table.json is absent
    // (-> per-role "stopped") instead of reading the real host's
    // /var/lib/nixling/daemon-state — the hermeticity fix over the bash gate.
    let daemon_state = tmp.path().join("daemon-state");
    std::fs::create_dir_all(&daemon_state).expect("mk daemon-state dir");
    // nixlingd's public socket is preferred for live VM status (d098dfca: "report
    // live public VM status from pidfd table"). Point it (and the broker socket)
    // at non-existent paths so `list` cannot reach the real host daemon and falls
    // back to the static fixture inventory — the hermeticity fix for that change.
    let missing_public = tmp.path().join("public.sock");
    let missing_broker = tmp.path().join("priv.sock");

    let out = Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(["list", "--json"])
        .env("NIXLING_MANIFEST_PATH", format!("{fixtures}/manifest.json"))
        .env("NIXLING_BUNDLE_PATH", format!("{fixtures}/bundle.json"))
        .env("NIXLING_TEST_SYSTEM_STATE_JSON", &sys)
        .env("NIXLING_DAEMON_STATE_DIR", &daemon_state)
        .env("NIXLING_PUBLIC_SOCKET", &missing_public)
        .env("NIXLING_BROKER_SOCKET", &missing_broker)
        .output()
        .expect("spawn nixling list --json");

    assert!(
        out.status.success(),
        "`nixling list --json` exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    // Strict schema validation: ListItemOutputV2 is deny_unknown_fields, so a
    // successful typed deserialize is equivalent to validating against
    // docs/reference/cli-output/list.schema.json.
    let list: ListOutputV2 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "list --json did not match the ListOutputV2 schema: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });

    let items = &list.0;
    assert_eq!(
        items.len(),
        2,
        "expected exactly the 2 smoke VMs, got {items:?}"
    );
    let corp = items
        .iter()
        .find(|i| i.name == "corp-vm")
        .expect("corp-vm in inventory");
    assert_eq!(corp.env.as_deref(), Some("work"));
    assert!(!corp.is_net_vm, "corp-vm is a workload VM");
    assert_eq!(
        corp.status, "stopped",
        "corp-vm: all units inactive + empty daemon-state -> stopped"
    );
    assert_eq!(
        corp.runner_parity_ok,
        Some(true),
        "corp-vm runner parity must be OK against its committed closure"
    );
    let net = items
        .iter()
        .find(|i| i.name == "sys-work-net")
        .expect("sys-work-net in inventory");
    assert!(net.is_net_vm, "sys-work-net is the auto-declared net VM");
    assert_eq!(net.status, "running", "the active net VM is running");
    assert_eq!(
        net.runner_parity_ok,
        Some(true),
        "sys-work-net runner parity must be OK against its committed closure"
    );
}
