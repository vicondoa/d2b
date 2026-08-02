//! W5 CLI-contract integration coverage for the v3 `ModernCli` surface.
//!
//! The old test exercised the retired v2 `up`/`vm` verbs and the manifest
//! inventory shape.  v3 addresses Zone resources instead: lifecycle uses
//! `guest`, execution uses `exec`, and inventory uses generic
//! `list <ResourceType>`.  These tests keep the historical libtest names
//! because the migration census pins them, but their assertions are all over
//! the committed v3 parser and JSON envelope.
//!
//! Every runtime probe uses a missing Zone socket and missing legacy artifacts.
//! The v3 error envelope therefore proves that the command reached ModernCli
//! dispatch without contacting SSH, executing a legacy CLI, or leaking the
//! command's private canary.  Clap rejections intentionally remain plain usage
//! errors: they happen before a v3 JSON envelope exists.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const SECRET_CANARY: &str = "zone-private-canary";
const EXEC_SECRET_ENV: &str = "D2B_EXEC_SECRET=zone-private-canary";

// If any retired fallback path is reached, the distinctive status is visible
// to the parent test process.
const LEGACY_CLI_POISON: &str = "#!/bin/sh\nexit 99\n";
const SSH_POISON: &str = "#!/bin/sh\nexit 98\n";

const V3_ERROR_KEYS: &[&str] = &["errorClass", "message", "ok", "schemaVersion", "zoneRef"];

struct ScratchPaths {
    // These deliberately do not exist.  ModernCli must not recover by reading
    // the retired v2 manifest/bundle or a host runtime path.
    manifest: PathBuf,
    bundle: PathBuf,
    socket: PathBuf,
    legacy_cli: PathBuf,
    tool_path: PathBuf,
}

fn write_executable(path: &PathBuf, contents: &str) {
    std::fs::write(path, contents).expect("write executable fixture");
    let mut permissions = std::fs::metadata(path)
        .expect("stat executable fixture")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod executable fixture");
}

/// Build a hermetic v3 CLI sandbox.  The legacy and SSH paths are executable
/// sentinels, while the Zone socket and old manifest artifacts are absent.
fn scratch() -> (TempDir, ScratchPaths) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tool_path = tmp.path().join("tools");
    std::fs::create_dir(&tool_path).expect("mk tool fixture dir");

    let legacy_cli = tmp.path().join("legacy-cli-poison");
    write_executable(&legacy_cli, LEGACY_CLI_POISON);
    write_executable(&tool_path.join("ssh"), SSH_POISON);
    write_executable(&tool_path.join("scp"), SSH_POISON);

    let paths = ScratchPaths {
        manifest: tmp.path().join("retired-v2-manifest.json"),
        bundle: tmp.path().join("retired-v2-bundle.json"),
        socket: tmp.path().join("missing-zone.sock"),
        legacy_cli,
        tool_path,
    };
    (tmp, paths)
}

fn run_cli(paths: &ScratchPaths, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .env_clear()
        .args(args)
        .env("PATH", &paths.tool_path)
        .env("D2B_PUBLIC_SOCKET", &paths.socket)
        .env("D2B_MANIFEST_PATH", &paths.manifest)
        .env("D2B_BUNDLE_PATH", &paths.bundle)
        .env("D2B_LEGACY_CLI_PATH", &paths.legacy_cli)
        .env("D2B_LEGACY_CLI", &paths.legacy_cli)
        .env("D2B_LEGACY_BASH_OPT_IN", "1")
        .env("D2B_SUPPRESS_LEGACY_BASH_WARNING", "1")
        .output()
        .expect("spawn d2b")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr_of(output)
    )
}

fn stdout_json(output: &Output, label: &str) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{label}: stdout was not valid JSON: {error}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            stderr_of(output),
        )
    })
}

/// Assert that no retired execution route ran and no private test canary was
/// reflected in a user-facing response.
fn assert_no_retired_transport(output: &Output, paths: &ScratchPaths, label: &str) {
    assert_ne!(
        output.status.code(),
        Some(99),
        "{label}: executed the legacy CLI poison-pill\n{}",
        combined_output(output)
    );
    assert_ne!(
        output.status.code(),
        Some(98),
        "{label}: executed the SSH/SCP poison-pill\n{}",
        combined_output(output)
    );
    let rendered = combined_output(output);
    assert!(
        !rendered.contains(SECRET_CANARY),
        "{label}: leaked the private canary\n{rendered}"
    );
    assert!(
        !rendered.to_ascii_lowercase().contains("ssh"),
        "{label}: exposed an SSH fallback in its output\n{rendered}"
    );
    assert!(
        !rendered.to_ascii_lowercase().contains("bash"),
        "{label}: exposed a bash fallback in its output\n{rendered}"
    );
    assert!(
        !rendered.contains(&paths.socket.display().to_string()),
        "{label}: leaked the Zone socket path\n{rendered}"
    );
}

fn assert_v3_error(
    output: &Output,
    paths: &ScratchPaths,
    label: &str,
    exit_code: i32,
    error_class: &str,
    message: &str,
) {
    assert_no_retired_transport(output, paths, label);
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "{label}: unexpected exit code; stderr:\n{}",
        stderr_of(output)
    );
    assert!(
        output.stderr.is_empty(),
        "{label}: JSON errors must keep stderr empty; got:\n{}",
        stderr_of(output)
    );

    let envelope = stdout_json(output, label);
    let object = envelope
        .as_object()
        .unwrap_or_else(|| panic!("{label}: JSON error must be an object: {envelope}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, V3_ERROR_KEYS,
        "{label}: v3 error envelope keys drifted: {envelope}"
    );
    assert_eq!(envelope["ok"], false, "{label}: expected ok=false");
    assert_eq!(
        envelope["zoneRef"], "Zone/local-root",
        "{label}: unexpected Zone reference"
    );
    assert_eq!(
        envelope["schemaVersion"], 1,
        "{label}: unexpected JSON schema version"
    );
    assert_eq!(
        envelope["errorClass"], error_class,
        "{label}: unexpected error class"
    );
    assert_eq!(envelope["message"], message, "{label}: unexpected message");
}

fn assert_usage_rejection(output: &Output, paths: &ScratchPaths, label: &str, marker: &str) {
    assert_no_retired_transport(output, paths, label);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{label}: expected clap usage exit 2; stderr:\n{}",
        stderr_of(output)
    );
    assert!(
        output.stdout.is_empty(),
        "{label}: clap rejection must not emit a JSON document"
    );
    let stderr = stderr_of(output);
    assert!(
        stderr.contains(marker),
        "{label}: usage error did not contain {marker:?}; got:\n{stderr}"
    );
    assert!(
        stderr.contains("Usage: d2b"),
        "{label}: usage error did not identify d2b; got:\n{stderr}"
    );
}

fn assert_retired_subcommand(output: &Output, paths: &ScratchPaths, label: &str, command: &str) {
    assert_usage_rejection(
        output,
        paths,
        label,
        &format!("unrecognized subcommand '{command}'"),
    );
}

#[test]
fn mutating_verbs_emit_daemon_down_without_bash_fallback() {
    // Historical test id retained for the migration pin.  v3 lifecycle
    // operations are Guest resource updates, not daemon-down v2 verbs.
    let (_guard, paths) = scratch();

    for action in ["start", "stop", "restart"] {
        let args = ["guest", action, "corp-vm", "--apply", "--json"];
        let output = run_cli(&paths, &args);
        assert_v3_error(
            &output,
            &paths,
            &format!("guest {action}"),
            1,
            "zone-unavailable",
            "Zone runtime is unavailable",
        );
    }
}

#[test]
fn legacy_bash_opt_in_is_a_no_op() {
    // Both retired namespaces must fail at the ModernCli parser.  In
    // particular, --json does not manufacture a v3 envelope for clap errors.
    let (_guard, paths) = scratch();

    let up = run_cli(
        &paths,
        &["up", "corp-vm", "--apply", "--json", SECRET_CANARY],
    );
    assert_retired_subcommand(&up, &paths, "retired up", "up");

    let vm = run_cli(&paths, &["vm", "start", "corp-vm", "--apply", "--json"]);
    assert_retired_subcommand(&vm, &paths, "retired vm", "vm");
}

#[test]
fn vm_list_is_daemon_native_json() {
    // Historical test id retained for the migration pin.  Generic v3 list
    // takes a standard ResourceType and returns the Zone envelope on transport
    // failure.
    let (_guard, paths) = scratch();
    let output = run_cli(&paths, &["list", "Guest", "--phase", "Ready", "--json"]);
    assert_v3_error(
        &output,
        &paths,
        "list Guest",
        1,
        "zone-unavailable",
        "Zone runtime is unavailable",
    );
}

#[test]
fn top_level_list_is_daemon_native_json() {
    let (_guard, paths) = scratch();

    let output = run_cli(
        &paths,
        &[
            "list",
            "EphemeralProcess",
            "--execution-ref",
            "Guest/corp-vm",
            "--domain",
            "user",
            "--json",
        ],
    );
    assert_v3_error(
        &output,
        &paths,
        "list EphemeralProcess",
        1,
        "zone-unavailable",
        "Zone runtime is unavailable",
    );

    // The old top-level inventory shape is no longer a command: ResourceType
    // is required by ModernCli.
    let retired_shape = run_cli(&paths, &["list", "--json"]);
    assert_usage_rejection(
        &retired_shape,
        &paths,
        "retired list shape",
        "<RESOURCE_TYPE>",
    );
}

#[test]
fn vm_exec_missing_command_emits_cli_usage_envelope() {
    let (_guard, paths) = scratch();
    let output = run_cli(&paths, &["exec", "run", "Guest/corp-vm", "--json"]);

    // Missing command is rejected by clap before a Zone context can emit a
    // JSON envelope.
    assert_usage_rejection(&output, &paths, "exec run missing command", "<COMMAND>");
}

#[test]
fn vm_exec_no_daemon_emits_transport_unavailable_envelope() {
    let (_guard, paths) = scratch();
    let output = run_cli(
        &paths,
        &[
            "exec",
            "run",
            "Guest/corp-vm",
            "--env",
            EXEC_SECRET_ENV,
            "--json",
            "--",
            "/bin/true",
        ],
    );
    assert_v3_error(
        &output,
        &paths,
        "exec run Guest/corp-vm",
        1,
        "zone-unavailable",
        "Zone runtime is unavailable",
    );
}

#[test]
fn vm_exec_rejects_interactive_without_tty() {
    let (_guard, paths) = scratch();
    let output = run_cli(
        &paths,
        &[
            "exec",
            "attach",
            "EphemeralProcess/run-1",
            "--interactive",
            "--tty",
            "--json",
        ],
    );
    assert_v3_error(
        &output,
        &paths,
        "exec attach tty with JSON",
        2,
        "ref-invalid",
        "--tty is incompatible with --json",
    );
}
