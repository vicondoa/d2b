//! W3 CLI-contract integration test, migrated from
//! tests/cli-rust-native-list.sh.
//!
//! Spawns the real `d2b` binary with the rendered fixture-smoke environment
//! (D2B_FIXTURES) and exercises the v3 `guest list` Zone resource command.
//! With the supplied socket deliberately unavailable, the command must fail
//! closed with the strict v3 JSON error envelope rather than parse as the
//! retired static VM inventory command.
//!
//! Requires D2B_FIXTURES (the fixture-smoke output dir), delivered by the
//! dedicated CLI-contract step in tests/tools/rust-workspace-checks.sh. When unset
//! (e.g. the plain `cargo test --workspace` pass that has no Nix sandbox) the
//! test skips; the gate step always sets D2B_FIXTURES, so the contract cannot
//! be silently disabled there.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const V3_ERROR_KEYS: &[&str] = &["errorClass", "message", "ok", "schemaVersion", "zoneRef"];

/// The fixture-smoke output dir, or `None` when D2B_FIXTURES is unset (plain
/// non-gated `cargo test` runs). The gated rust-workspace-checks.sh step always
/// sets it.
fn fixtures_dir() -> Option<String> {
    std::env::var("D2B_FIXTURES").ok()
}

fn assert_zone_unavailable_envelope(value: &Value, missing_socket: &Path) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("Zone error must be a JSON object, got: {value}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, V3_ERROR_KEYS,
        "Zone-unavailable envelope key set must stay closed"
    );
    assert_eq!(value["ok"], false);
    assert_eq!(value["zoneRef"], "Zone/local-root");
    assert_eq!(value["errorClass"], "zone-unavailable");
    assert_eq!(value["message"], "Zone runtime is unavailable");
    assert_eq!(value["schemaVersion"], 1);
    assert!(
        !value
            .to_string()
            .contains(&missing_socket.display().to_string()),
        "Zone-unavailable envelope must redact the missing socket path: {value}"
    );
}

fn run_with_missing_socket(args: &[&str], missing_socket: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(args)
        .env("D2B_PUBLIC_SOCKET", missing_socket)
        .output()
        .unwrap_or_else(|error| panic!("spawn d2b {args:?}: {error}"))
}

#[test]
fn list_json_matches_smoke_inventory_and_schema() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let fixtures = Path::new(&fixtures);
    for artifact in ["manifest.json", "bundle.json"] {
        assert!(
            fixtures.join(artifact).is_file(),
            "D2B_FIXTURES is missing the required {artifact} artifact"
        );
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let missing_public = tmp.path().join("public.sock");
    let missing_broker = tmp.path().join("priv.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["guest", "list", "--json"])
        // Keep the rendered fixture paths in the environment while proving
        // ModernCli does not fall back to their retired static inventory.
        .env("D2B_MANIFEST_PATH", fixtures.join("manifest.json"))
        .env("D2B_BUNDLE_PATH", fixtures.join("bundle.json"))
        .env("D2B_PUBLIC_SOCKET", &missing_public)
        .env("D2B_BROKER_SOCKET", &missing_broker)
        .output()
        .expect("spawn d2b guest list --json");

    assert_eq!(
        out.status.code(),
        Some(1),
        "`d2b guest list --json` must fail closed when the Zone is unavailable; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "v3 JSON errors must stay on stdout; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "guest list --json did not match the v3 error envelope: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_zone_unavailable_envelope(&envelope, &missing_public);
}

#[test]
fn v3_mutations_fail_closed_with_the_zone_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing_public = tmp.path().join("public.sock");
    let commands = [
        (
            "guest start",
            &["guest", "start", "work", "--apply", "--json"][..],
        ),
        (
            "guest stop",
            &["guest", "stop", "work", "--apply", "--json"][..],
        ),
        (
            "guest restart",
            &["guest", "restart", "work", "--apply", "--json"][..],
        ),
        (
            "activation build",
            &["activation", "build", "Guest/work", "--json"][..],
        ),
        (
            "activation switch",
            &["activation", "switch", "Guest/work", "--apply", "--json"][..],
        ),
        (
            "host prepare",
            &["host", "prepare", "--apply", "--json"][..],
        ),
        (
            "host destroy",
            &["host", "destroy", "--apply", "--json"][..],
        ),
        (
            "host install",
            &["host", "install", "--apply", "--json"][..],
        ),
        (
            "host reconcile",
            &["host", "reconcile", "--network", "--apply", "--json"][..],
        ),
    ];

    for (label, args) in commands {
        let out = run_with_missing_socket(args, &missing_public);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{label} must fail with Zone-unavailable, stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stderr.is_empty(),
            "{label} JSON errors must stay on stdout; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
            panic!(
                "{label} did not emit a v3 JSON envelope: {error}\nstdout:\n{}",
                String::from_utf8_lossy(&out.stdout)
            )
        });
        assert_zone_unavailable_envelope(&envelope, &missing_public);
    }
}

#[test]
fn retired_cli_namespaces_are_rejected_without_legacy_routing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing_public = tmp.path().join("public.sock");
    let commands = [
        ("vm", &["vm", "start", "work", "--json"][..]),
        ("realm", &["realm", "list", "--json"][..]),
        ("up", &["up", "work", "--json"][..]),
        ("keys", &["keys", "list", "--json"][..]),
        ("usb", &["usb", "probe", "--json"][..]),
    ];

    for (label, args) in commands {
        let out = run_with_missing_socket(args, &missing_public);
        assert_eq!(
            out.status.code(),
            Some(2),
            "retired {label} command must be a clap usage error; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stdout.is_empty(),
            "retired {label} command must not emit a success or v3 envelope"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("bash") && !stderr.contains("d2b-legacy"),
            "retired {label} command exposed a legacy route:\n{stderr}"
        );
    }
}

#[test]
fn retired_vm_lifecycle_does_not_expose_the_legacy_timeout_exit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing_public = tmp.path().join("public.sock");
    let out = run_with_missing_socket(
        &["vm", "start", "work", "--apply", "--json"],
        &missing_public,
    );

    assert_eq!(
        out.status.code(),
        Some(2),
        "retired vm lifecycle must stop at v3 parsing, not reach the old timeout path"
    );
    assert_ne!(
        out.status.code(),
        Some(33),
        "legacy API-ready timeout exit must not be reachable from a retired command"
    );
    assert!(out.stdout.is_empty());
}
