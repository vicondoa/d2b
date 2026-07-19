//! W3 CLI-contract integration test, migrated from
//! tests/cli-vm-verbs-eval.sh.
//!
//! Spawns the real `d2b` binary against a synthetic inline manifest and a
//! deliberately-missing public socket, and asserts that the Rust CLI's
//! mutating verbs (`up`/`down`/`restart` plus their `vm start`/`vm stop`/
//! `vm restart` aliases), `list`/`vm list`, and the guest-control surface
//! (`vm exec`) are fully daemon-native with NO bash fallback:
//!
//!   1. With d2bd's public socket missing, every mutating verb surfaces
//!      the typed `daemon-down` envelope (`code == "daemon-down"`,
//!      `exitCode == 1`) and exits 1 — even when the removed
//!      `D2B_LEGACY_BASH_OPT_IN=1` escape hatch is set.
//!   2. The `D2B_LEGACY_CLI_PATH` / `D2B_LEGACY_CLI` poison-pill is
//!      NEVER invoked: it is wired to an executable sentinel that would
//!      `exit 99` if ever exec'd, so any exit code of 99 fails the assertion.
//!   3. `vm list` and top-level `list` fail closed when their authenticated
//!      ComponentSession is unavailable.
//!   4. `vm exec` reaches `cmd_vm_exec` through real clap parsing + dispatch.
//!
//! Layer 1: no live daemon, no microvm spawn, no D2B_FIXTURES. Self-contained —
//! always runs. Runs in seconds.
//!
//! The `daemon-down` envelope is the private `HostErrorEnvelope` DTO (not part
//! of the crate's public surface), so the JSON is asserted over
//! `serde_json::Value` rather than a typed deserialize.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

// Synthetic inline manifest reproduced verbatim from the bash gate's heredoc
// (a single workload VM). Pointed at via D2B_MANIFEST_PATH; NOT a Nix eval.
const VM_MANIFEST_JSON: &str = r#"{
  "test-vm": {
    "name": "test-vm",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "audio": false,
    "audioService": "none",
    "usbipYubikey": false,
    "staticIp": null,
    "isNetVm": false,
    "stateDir": "/var/lib/d2b/vms/test-vm",
    "bridge": "d2b-work",
    "sshUser": null
  }
}"#;

// Synthetic manifest for the guest-control (`vm exec`) surface.
const KONSOLE_MANIFEST_JSON: &str = r#"{
  "konsole-vm": {
    "name": "konsole-vm",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "audio": false,
    "audioService": "none",
    "usbipYubikey": false,
    "staticIp": "10.30.0.99",
    "isNetVm": false,
    "stateDir": "/var/lib/d2b/vms/konsole-vm",
    "bridge": "d2b-work",
    "sshUser": "alice"
  }
}"#;

// A poison-pill "legacy bash CLI": if the rust CLI ever exec's it the process
// exits 99, a distinctive code no native verb path returns.
const POISON_PILL: &str = "#!/usr/bin/env bash\n\
echo \"FAIL: rust CLI exec'd the legacy bash poison-pill with args: $*\" >&2\n\
exit 99\n";

// Minimal hermetic system-state so `list` does not probe the real host's
// systemctl / daemon-state (which would make the gate non-hermetic).
const SYSTEM_STATE_JSON: &str = r#"{ "units": {}, "bridges": {} }"#;

struct ScratchPaths {
    manifest: PathBuf,
    bundle: PathBuf,
    socket: PathBuf,
    poison: PathBuf,
    realm_entrypoints: PathBuf,
    system_state: PathBuf,
    daemon_state: PathBuf,
}

/// Build a scratch sandbox: the synthetic manifest, a never-bound public-socket
/// path, the exit-99 poison-pill, a minimal system-state fixture, and an empty
/// daemon-state dir. The returned `TempDir` guard must be kept alive for the
/// duration of the test.
fn scratch(manifest_json: &str) -> (TempDir, ScratchPaths) {
    let tmp = tempfile::tempdir().expect("tempdir");

    let manifest = tmp.path().join("vms.json");
    std::fs::write(&manifest, manifest_json).expect("write manifest");
    let bundle = tmp.path().join("missing-bundle.json");

    let poison = tmp.path().join("legacy-poison.sh");
    std::fs::write(&poison, POISON_PILL).expect("write poison-pill");
    let mut perms = std::fs::metadata(&poison)
        .expect("stat poison-pill")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&poison, perms).expect("chmod poison-pill");

    let realm_entrypoints = tmp.path().join("missing-realm-entrypoints.json");

    let system_state = tmp.path().join("system-state.json");
    std::fs::write(&system_state, SYSTEM_STATE_JSON).expect("write system-state");

    let daemon_state = tmp.path().join("daemon-state");
    std::fs::create_dir_all(&daemon_state).expect("mk daemon-state dir");

    // A path under the tempdir that is never created -> the public socket is
    // missing, so every daemon-backed verb fails the connectivity check.
    let socket = tmp.path().join("never-bound.sock");
    let _ = std::fs::remove_file(&socket);

    (
        tmp,
        ScratchPaths {
            manifest,
            bundle,
            socket,
            poison,
            realm_entrypoints,
            system_state,
            daemon_state,
        },
    )
}

/// Spawn the real `d2b` binary with the missing socket, the poison-pill
/// legacy-CLI env (and the removed `D2B_LEGACY_BASH_OPT_IN` opt-in) set, and
/// the hermetic state fixtures. Every invocation therefore doubles as a proof
/// that no verb falls back to the poison-pill.
fn run_cli(p: &ScratchPaths, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(args)
        .env("D2B_MANIFEST_PATH", &p.manifest)
        .env("D2B_BUNDLE_PATH", &p.bundle)
        .env("D2B_PUBLIC_SOCKET", &p.socket)
        .env("D2B_LEGACY_CLI_PATH", &p.poison)
        .env("D2B_LEGACY_CLI", &p.poison)
        .env("D2B_LEGACY_BASH_OPT_IN", "1")
        .env("D2B_SUPPRESS_LEGACY_BASH_WARNING", "1")
        .env("D2B_REALM_ENTRYPOINTS_PATH", &p.realm_entrypoints)
        .env("D2B_TEST_SYSTEM_STATE_JSON", &p.system_state)
        .env("D2B_DAEMON_STATE_DIR", &p.daemon_state)
        .output()
        .expect("spawn d2b")
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout_json(out: &Output, label: &str) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "{label}: stdout was not valid JSON: {err}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&out.stdout),
            stderr_of(out),
        )
    })
}

/// Assert the typed `daemon-down` envelope on stdout with exit 1, and that the
/// poison-pill was never exec'd (exit code is never 99).
fn assert_daemon_down(out: &Output, label: &str) {
    let code = out.status.code();
    assert_ne!(
        code,
        Some(99),
        "{label}: rust CLI exec'd the legacy bash poison-pill \
         (D2B_LEGACY_BASH_OPT_IN must NOT be honoured)\nstderr:\n{}",
        stderr_of(out),
    );
    assert_eq!(
        code,
        Some(1),
        "{label}: expected the daemon-down typed envelope (exit 1)\nstderr:\n{}",
        stderr_of(out),
    );
    let envelope = stdout_json(out, label);
    assert_eq!(
        envelope.get("code").and_then(Value::as_str),
        Some("daemon-down"),
        "{label}: expected .code == \"daemon-down\"; got {envelope}",
    );
    assert_eq!(
        envelope.get("exitCode").and_then(Value::as_i64),
        Some(1),
        "{label}: expected .exitCode == 1; got {envelope}",
    );
}

#[test]
fn mutating_verbs_emit_daemon_down_without_bash_fallback() {
    let (_guard, paths) = scratch(VM_MANIFEST_JSON);

    let verbs: &[(&str, &[&str])] = &[
        ("up", &["up", "test-vm", "--apply", "--json"]),
        ("down", &["down", "test-vm", "--apply", "--json"]),
        ("restart", &["restart", "test-vm", "--apply", "--json"]),
        ("vm-start", &["vm", "start", "test-vm", "--apply", "--json"]),
        ("vm-stop", &["vm", "stop", "test-vm", "--apply", "--json"]),
        (
            "vm-restart",
            &["vm", "restart", "test-vm", "--apply", "--json"],
        ),
    ];

    for (label, args) in verbs {
        let out = run_cli(&paths, args);
        assert_daemon_down(&out, label);
    }
}

#[test]
fn legacy_bash_opt_in_is_a_no_op() {
    // Spot-check: D2B_LEGACY_BASH_OPT_IN=1 + a poison-pill legacy path must
    // never reach the poison-pill. The escape hatch is removed.
    let (_guard, paths) = scratch(VM_MANIFEST_JSON);
    let out = run_cli(&paths, &["up", "test-vm", "--apply", "--json"]);
    assert_ne!(
        out.status.code(),
        Some(99),
        "D2B_LEGACY_BASH_OPT_IN was honoured — the escape hatch must be removed\nstderr:\n{}",
        stderr_of(&out),
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "opt-in spot-check: expected daemon-down exit 1\nstderr:\n{}",
        stderr_of(&out),
    );
}

#[test]
fn vm_list_requires_authenticated_daemon_session() {
    let (_guard, paths) = scratch(VM_MANIFEST_JSON);
    let out = run_cli(&paths, &["vm", "list", "--json"]);
    assert_ne!(
        out.status.code(),
        Some(99),
        "vm list exec'd the legacy bash poison-pill\nstderr:\n{}",
        stderr_of(&out),
    );
    assert_eq!(out.status.code(), Some(69));
    assert!(out.stdout.is_empty());
    assert!(stderr_of(&out).contains("client-connect-failed"));
}

#[test]
fn top_level_list_requires_authenticated_daemon_session() {
    let (_guard, paths) = scratch(VM_MANIFEST_JSON);
    let out = run_cli(&paths, &["list", "--json"]);
    assert_ne!(
        out.status.code(),
        Some(99),
        "top-level list exec'd the legacy bash poison-pill\nstderr:\n{}",
        stderr_of(&out),
    );
    assert_eq!(
        out.status.code(),
        Some(69),
        "list expected ComponentSession transport exit 69\nstderr:\n{}",
        stderr_of(&out),
    );
    assert!(
        out.stdout.is_empty(),
        "list must not emit a static fallback document"
    );
    assert!(stderr_of(&out).contains("client-connect-failed"));
}

#[test]
fn vm_exec_missing_command_emits_cli_usage_envelope() {
    // Exercises `d2b vm exec` through real clap parsing + dispatch: a
    // missing command surfaces the cli/usage envelope (exit 2).
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let out = run_cli(&paths, &["vm", "exec", "konsole-vm", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "vm exec (no command) should exit 2\nstderr:\n{}",
        stderr_of(&out),
    );
    let envelope = stdout_json(&out, "vm exec usage");
    assert_eq!(
        envelope.get("command").and_then(Value::as_str),
        Some("vm exec")
    );
    assert_eq!(envelope.get("source").and_then(Value::as_str), Some("cli"));
    assert_eq!(
        envelope.get("reason").and_then(Value::as_str),
        Some("usage")
    );
}

#[test]
fn vm_exec_no_daemon_emits_transport_unavailable_envelope() {
    // With the daemon socket absent, `vm exec` surfaces the guest-control
    // transport-unavailable envelope (proving it reaches cmd_vm_exec and never
    // falls back to SSH/bash).
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let out = run_cli(
        &paths,
        &["vm", "exec", "konsole-vm", "--json", "--", "/bin/true"],
    );
    assert_ne!(
        out.status.code(),
        Some(99),
        "vm exec exec'd the legacy bash poison-pill\nstderr:\n{}",
        stderr_of(&out),
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "vm exec with no daemon should exit non-zero\nstderr:\n{}",
        stderr_of(&out),
    );
    let envelope = stdout_json(&out, "vm exec transport");
    assert_eq!(
        envelope.get("command").and_then(Value::as_str),
        Some("vm exec")
    );
    assert_eq!(
        envelope.get("source").and_then(Value::as_str),
        Some("transport"),
    );
    assert_eq!(
        envelope.get("reason").and_then(Value::as_str),
        Some("guest-control-transport-unavailable"),
    );
}

#[test]
fn vm_exec_rejects_interactive_without_tty() {
    // -i/--interactive without -t/--tty must fail-fast with a usage error
    // (guestd forwards stdin only in PTY mode).
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let out = run_cli(
        &paths,
        &["vm", "exec", "konsole-vm", "-i", "--", "/bin/true"],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "vm exec -i without -t should exit 2\nstderr:\n{}",
        stderr_of(&out),
    );
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("-t/--tty") || stderr.to_lowercase().contains("requires -t"),
        "vm exec -i without -t error must cite the -t/--tty requirement; got:\n{stderr}",
    );
}
