//! W3 CLI-contract integration test, migrated from
//! tests/cli-vm-verbs-eval.sh.
//!
//! Spawns the real `nixling` binary against a synthetic inline manifest and a
//! deliberately-missing public socket, and asserts that the Rust CLI's
//! mutating verbs (`up`/`down`/`restart` plus their `vm start`/`vm stop`/
//! `vm restart` aliases), `list`/`vm list`, and the guest-control surfaces
//! (`vm konsole`, `vm exec`) are fully daemon-native with NO bash fallback:
//!
//!   1. With nixlingd's public socket missing, every mutating verb surfaces
//!      the typed `daemon-down` envelope (`code == "daemon-down"`,
//!      `exitCode == 1`) and exits 1 — even when the removed
//!      `NIXLING_LEGACY_BASH_OPT_IN=1` escape hatch is set.
//!   2. The `NIXLING_LEGACY_CLI_PATH` / `NIXLING_LEGACY_CLI` poison-pill is
//!      NEVER invoked: it is wired to an executable sentinel that would
//!      `exit 99` if ever exec'd, so any exit code of 99 fails the assertion.
//!   3. `vm list` / `list` return the rust-native JSON envelopes without
//!      touching bash.
//!   4. `vm konsole --dry-run` emits the guest-control transport shape (no
//!      retired SSH fields) and `vm exec` reaches `cmd_vm_exec` through real
//!      clap parsing + dispatch.
//!
//! Layer 1: no live daemon, no microvm spawn, no NL_FIXTURES. Self-contained —
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
// (a single workload VM). Pointed at via NIXLING_MANIFEST_PATH; NOT a Nix eval.
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
    "stateDir": "/var/lib/nixling/vms/test-vm",
    "bridge": "nl-work",
    "sshUser": null
  }
}"#;

// Synthetic manifest for the guest-control (konsole / exec) surfaces.
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
    "stateDir": "/var/lib/nixling/vms/konsole-vm",
    "bridge": "nl-work",
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
    socket: PathBuf,
    poison: PathBuf,
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

    let poison = tmp.path().join("legacy-poison.sh");
    std::fs::write(&poison, POISON_PILL).expect("write poison-pill");
    let mut perms = std::fs::metadata(&poison)
        .expect("stat poison-pill")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&poison, perms).expect("chmod poison-pill");

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
            socket,
            poison,
            system_state,
            daemon_state,
        },
    )
}

/// Spawn the real `nixling` binary with the missing socket, the poison-pill
/// legacy-CLI env (and the removed `NIXLING_LEGACY_BASH_OPT_IN` opt-in) set, and
/// the hermetic state fixtures. Every invocation therefore doubles as a proof
/// that no verb falls back to the poison-pill.
fn run_cli(p: &ScratchPaths, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(args)
        .env("NIXLING_MANIFEST_PATH", &p.manifest)
        .env("NIXLING_PUBLIC_SOCKET", &p.socket)
        .env("NIXLING_LEGACY_CLI_PATH", &p.poison)
        .env("NIXLING_LEGACY_CLI", &p.poison)
        .env("NIXLING_LEGACY_BASH_OPT_IN", "1")
        .env("NIXLING_SUPPRESS_LEGACY_BASH_WARNING", "1")
        .env("NIXLING_TEST_SYSTEM_STATE_JSON", &p.system_state)
        .env("NIXLING_DAEMON_STATE_DIR", &p.daemon_state)
        .output()
        .expect("spawn nixling")
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
         (NIXLING_LEGACY_BASH_OPT_IN must NOT be honoured)\nstderr:\n{}",
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
    // Spot-check: NIXLING_LEGACY_BASH_OPT_IN=1 + a poison-pill legacy path must
    // never reach the poison-pill. The escape hatch is removed.
    let (_guard, paths) = scratch(VM_MANIFEST_JSON);
    let out = run_cli(&paths, &["up", "test-vm", "--apply", "--json"]);
    assert_ne!(
        out.status.code(),
        Some(99),
        "NIXLING_LEGACY_BASH_OPT_IN was honoured — the escape hatch must be removed\nstderr:\n{}",
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
fn vm_list_is_daemon_native_json() {
    let (_guard, paths) = scratch(VM_MANIFEST_JSON);
    let out = run_cli(&paths, &["vm", "list", "--json"]);
    assert_ne!(
        out.status.code(),
        Some(99),
        "vm list exec'd the legacy bash poison-pill\nstderr:\n{}",
        stderr_of(&out),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "vm list expected exit 0\nstderr:\n{}",
        stderr_of(&out),
    );
    let envelope = stdout_json(&out, "vm list");
    assert_eq!(
        envelope.get("command").and_then(Value::as_str),
        Some("vm list"),
        "vm list did not emit the rust-native JSON envelope; got {envelope}",
    );
}

#[test]
fn top_level_list_is_daemon_native_json() {
    // `nixling list` is the native manifest view; re-assert with the same
    // poison-pill setup to keep the no-bash-fallback contract honest.
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
        Some(0),
        "list expected exit 0\nstderr:\n{}",
        stderr_of(&out),
    );
    let inventory = stdout_json(&out, "list");
    let items = inventory.as_array().unwrap_or_else(|| {
        panic!("list --json must emit the native manifest array; got {inventory}")
    });
    assert!(
        items
            .iter()
            .any(|i| i.get("name").and_then(Value::as_str) == Some("test-vm")),
        "list --json must include the synthetic test-vm; got {inventory}",
    );
}

#[test]
fn vm_konsole_dry_run_emits_guest_control_shape() {
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let out = run_cli(
        &paths,
        &["vm", "konsole", "konsole-vm", "--dry-run", "--json"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "vm konsole --dry-run --json should exit 0\nstderr:\n{}",
        stderr_of(&out),
    );
    let envelope = stdout_json(&out, "vm konsole");
    assert_eq!(
        envelope.get("command").and_then(Value::as_str),
        Some("vm konsole"),
    );
    assert_eq!(
        envelope.get("mode").and_then(Value::as_str),
        Some("dry-run")
    );
    assert_eq!(
        envelope.get("vm").and_then(Value::as_str),
        Some("konsole-vm")
    );
    assert_eq!(
        envelope.get("terminal").and_then(Value::as_str),
        Some("konsole"),
        "default terminal is konsole",
    );
    assert_eq!(
        envelope.get("transport").and_then(Value::as_str),
        Some("guest-control"),
    );

    let argv: Vec<&str> = envelope
        .get("argv")
        .and_then(Value::as_array)
        .expect("argv array")
        .iter()
        .map(|v| v.as_str().expect("argv element is a string"))
        .collect();
    assert_eq!(
        argv.first().copied(),
        Some("konsole"),
        "argv[0] is the terminal"
    );
    // konsole hosts `nixling vm exec -it <vm> -- bash -l` over guest-control.
    let joined = argv.join(" ");
    assert!(
        joined.ends_with("vm exec -it konsole-vm -- /run/current-system/sw/bin/bash -l"),
        "vm konsole .argv must host `vm exec -it konsole-vm -- /run/current-system/sw/bin/bash -l`; got {argv:?}",
    );

    // The retired SSH fields must be absent from the JSON entirely.
    assert!(
        envelope.get("host").is_none(),
        "vm konsole must not emit SSH .host"
    );
    assert!(
        envelope.get("user").is_none(),
        "vm konsole must not emit SSH .user"
    );
    assert!(
        envelope.get("key").is_none(),
        "vm konsole must not emit SSH .key"
    );
}

#[test]
fn vm_konsole_terminal_override_reflected() {
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let out = run_cli(
        &paths,
        &[
            "vm",
            "konsole",
            "konsole-vm",
            "--dry-run",
            "--json",
            "--terminal",
            "xterm",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "vm konsole --terminal override should exit 0\nstderr:\n{}",
        stderr_of(&out),
    );
    let envelope = stdout_json(&out, "vm konsole --terminal");
    assert_eq!(
        envelope.get("terminal").and_then(Value::as_str),
        Some("xterm"),
        "--terminal override not reflected",
    );
    let argv0 = envelope
        .get("argv")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str);
    assert_eq!(
        argv0,
        Some("xterm"),
        "--terminal override not reflected in argv[0]"
    );
}

#[test]
fn vm_konsole_rejects_retired_ssh_flags() {
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let retired: &[(&str, &str)] = &[
        ("--host", "192.0.2.44"),
        ("--user", "bob"),
        ("--key", "/custom/key"),
    ];
    for (flag, value) in retired {
        let out = run_cli(
            &paths,
            &[
                "vm",
                "konsole",
                "konsole-vm",
                "--dry-run",
                "--json",
                flag,
                value,
            ],
        );
        assert_ne!(
            out.status.code(),
            Some(0),
            "vm konsole must reject retired {flag} (exited 0)\nstderr:\n{}",
            stderr_of(&out),
        );
    }
}

#[test]
fn vm_konsole_rejects_unknown_vm() {
    let (_guard, paths) = scratch(KONSOLE_MANIFEST_JSON);
    let out = run_cli(
        &paths,
        &["vm", "konsole", "missing-vm", "--dry-run", "--json"],
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "vm konsole on an unknown vm should exit non-zero\nstderr:\n{}",
        stderr_of(&out),
    );
}

#[test]
fn vm_exec_missing_command_emits_cli_usage_envelope() {
    // Exercises `nixling vm exec` through real clap parsing + dispatch: a
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
