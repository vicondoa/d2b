//! CLI-contract integration coverage for the v3 ModernCli status surface.
//!
//! The former tests exercised the retired v2 static VM inventory and bridge
//! checks. The v3 status surface addresses Zone resources through either the
//! generic `status <ResourceType>/<name>` command or the typed
//! `guest status <name>` command. These tests keep the historical libtest names
//! because the migration census pins them, but assert only the v3 envelopes.
//!
//! The rendered fixture artifacts remain present in the child environment
//! while the Zone socket is deliberately unavailable. A v3 command must fail
//! closed with a strict envelope rather than reading the retired manifest,
//! invoking a legacy CLI or SSH fallback, or exposing a secret canary.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const V3_ERROR_KEYS: &[&str] = &["errorClass", "message", "ok", "schemaVersion", "zoneRef"];
const SECRET_CANARY: &str = "zone-private-canary";
const LEGACY_CLI_POISON: &str = "#!/bin/sh\nexit 99\n";
const SSH_POISON: &str = "#!/bin/sh\nexit 98\n";

struct StatusEnv {
    _tmp: TempDir,
    manifest: PathBuf,
    bundle: PathBuf,
    missing_public: PathBuf,
    missing_broker: PathBuf,
    legacy_cli: PathBuf,
    tool_path: PathBuf,
}

impl StatusEnv {
    fn new(fixtures: &str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let fixtures = Path::new(fixtures);
        let manifest = fixtures.join("manifest.json");
        let bundle = fixtures.join("bundle.json");
        assert!(manifest.is_file(), "D2B_FIXTURES is missing manifest.json");
        assert!(bundle.is_file(), "D2B_FIXTURES is missing bundle.json");

        let tool_path = tmp.path().join("tools");
        std::fs::create_dir(&tool_path).expect("mk tool fixture dir");
        write_executable(&tool_path.join("ssh"), SSH_POISON);
        write_executable(&tool_path.join("scp"), SSH_POISON);

        // Put the canary in a legacy executable path. If any retired route
        // tries to surface that path, the output must still remain redacted.
        let legacy_cli = tmp.path().join(SECRET_CANARY);
        write_executable(&legacy_cli, LEGACY_CLI_POISON);

        Self {
            missing_public: tmp.path().join("missing-public.sock"),
            missing_broker: tmp.path().join("missing-broker.sock"),
            legacy_cli,
            tool_path,
            _tmp: tmp,
            manifest,
            bundle,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_d2b"))
            .env_clear()
            .args(args)
            .env("PATH", &self.tool_path)
            .env("D2B_MANIFEST_PATH", &self.manifest)
            .env("D2B_BUNDLE_PATH", &self.bundle)
            .env("D2B_PUBLIC_SOCKET", &self.missing_public)
            .env("D2B_BROKER_SOCKET", &self.missing_broker)
            .env("D2B_LEGACY_CLI_PATH", &self.legacy_cli)
            .env("D2B_LEGACY_CLI", &self.legacy_cli)
            .env("D2B_LEGACY_BASH_OPT_IN", "1")
            .env("D2B_SUPPRESS_LEGACY_BASH_WARNING", "1")
            .env("D2B_SECRET_CANARY", SECRET_CANARY)
            .output()
            .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
    }
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write executable fixture");
    let mut permissions = std::fs::metadata(path)
        .expect("stat executable fixture")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod executable fixture");
}

fn fixtures_dir() -> Option<String> {
    std::env::var("D2B_FIXTURES").ok().or_else(|| {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        None
    })
}

fn rendered_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_no_retired_transport(output: &Output, env: &StatusEnv, label: &str) {
    assert_ne!(
        output.status.code(),
        Some(99),
        "{label}: executed the legacy CLI poison-pill\n{}",
        rendered_output(output)
    );
    assert_ne!(
        output.status.code(),
        Some(98),
        "{label}: executed the SSH/SCP poison-pill\n{}",
        rendered_output(output)
    );
    let rendered = rendered_output(output);
    assert!(
        !rendered.contains(SECRET_CANARY),
        "{label}: leaked the private canary\n{rendered}"
    );
    assert!(
        !rendered.to_ascii_lowercase().contains("bash"),
        "{label}: exposed a bash fallback\n{rendered}"
    );
    assert!(
        !rendered.to_ascii_lowercase().contains("ssh"),
        "{label}: exposed an SSH fallback\n{rendered}"
    );
    assert!(
        !rendered.contains(&env.missing_public.display().to_string()),
        "{label}: leaked the missing Zone socket path\n{rendered}"
    );
}

fn json_value(output: &Output, label: &str) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "{label}: stdout was not valid JSON: {err}\n{}",
            rendered_output(output)
        )
    })
}

fn assert_json_error(
    output: &Output,
    env: &StatusEnv,
    label: &str,
    exit_code: i32,
    zone_ref: &str,
    error_class: &str,
    message: &str,
) {
    assert_no_retired_transport(output, env, label);
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "{label}: unexpected exit code\n{}",
        rendered_output(output)
    );
    assert!(
        output.stderr.is_empty(),
        "{label}: JSON errors must stay on stdout; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value = json_value(output, label);
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{label}: error envelope is not an object: {value}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, V3_ERROR_KEYS,
        "{label}: v3 error envelope keys drifted: {value}"
    );
    assert_eq!(value["ok"], false, "{label}: expected ok=false");
    assert_eq!(
        value["zoneRef"], zone_ref,
        "{label}: unexpected Zone reference"
    );
    assert_eq!(
        value["errorClass"], error_class,
        "{label}: unexpected error class"
    );
    assert_eq!(value["message"], message, "{label}: unexpected message");
    assert_eq!(
        value["schemaVersion"], 1,
        "{label}: unexpected JSON schema version"
    );
}

fn assert_human_zone_unavailable(output: &Output, env: &StatusEnv, label: &str) {
    assert_no_retired_transport(output, env, label);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label}: unexpected exit code\n{}",
        rendered_output(output)
    );
    assert!(
        output.stdout.is_empty(),
        "{label}: human errors must stay on stderr; stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("zone-unavailable") && stderr.contains("Zone runtime is unavailable"),
        "{label}: unexpected human Zone error:\n{stderr}"
    );
}

fn assert_usage_rejection(output: &Output, env: &StatusEnv, label: &str, marker: &str) {
    assert_no_retired_transport(output, env, label);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{label}: expected ModernCli usage exit 2\n{}",
        rendered_output(output)
    );
    assert!(
        output.stdout.is_empty(),
        "{label}: usage rejection must not emit a JSON document"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(marker),
        "{label}: usage error missed {marker:?}; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("Usage: d2b"),
        "{label}: usage error did not identify d2b; stderr:\n{stderr}"
    );
}

#[test]
fn status_vm_json_matches_schema_and_static_state() {
    // Historical test id retained for the migration pin. The v3 generic and
    // typed Guest status commands must not recover the retired static state.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = StatusEnv::new(&fixtures);

    for args in [
        &["status", "Guest/corp-vm", "--zone", "work", "--json"][..],
        &["guest", "status", "corp-vm", "--zone", "work", "--json"][..],
    ] {
        let output = env.run(args);
        assert_json_error(
            &output,
            &env,
            &format!("d2b {}", args.join(" ")),
            1,
            "Zone/work",
            "zone-unavailable",
            "Zone runtime is unavailable",
        );
    }
}

#[test]
fn status_vm_flag_and_positional_json_are_equivalent() {
    // Modern generic and typed Guest status are equivalent only at the
    // v3 Zone envelope. The old bare VM positional form is a ref-invalid
    // resource reference, not a static VM lookup.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = StatusEnv::new(&fixtures);

    let generic = env.run(&["status", "Guest/corp-vm", "--zone", "work", "--json"]);
    let guest = env.run(&["guest", "status", "corp-vm", "--zone", "work", "--json"]);
    assert_json_error(
        &generic,
        &env,
        "d2b status Guest/corp-vm --zone work --json",
        1,
        "Zone/work",
        "zone-unavailable",
        "Zone runtime is unavailable",
    );
    assert_json_error(
        &guest,
        &env,
        "d2b guest status corp-vm --zone work --json",
        1,
        "Zone/work",
        "zone-unavailable",
        "Zone runtime is unavailable",
    );
    assert_eq!(
        generic.stdout, guest.stdout,
        "generic and Guest status must share the same v3 Zone envelope"
    );

    let old_positional = env.run(&["status", "corp-vm", "--zone", "work", "--json"]);
    assert_json_error(
        &old_positional,
        &env,
        "d2b status corp-vm --zone work --json",
        2,
        "Zone/work",
        "ref-invalid",
        "resource reference must use <ResourceType>/<name>",
    );
}

#[test]
fn status_check_bridges_returns_frozen_not_yet_implemented_envelope() {
    // Historical test id retained for the migration pin. Bridge checking was
    // removed from the v3 status command, so ModernCli must reject it instead
    // of reviving the retired static bridge path.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = StatusEnv::new(&fixtures);

    let bridges = env.run(&["status", "--check-bridges", "--json"]);
    assert_usage_rejection(
        &bridges,
        &env,
        "retired status --check-bridges",
        "unexpected argument '--check-bridges'",
    );

    let vm_flag = env.run(&["status", "--vm", "corp-vm", "--json"]);
    assert_usage_rejection(
        &vm_flag,
        &env,
        "retired status --vm",
        "unexpected argument '--vm'",
    );
}

#[test]
fn status_human_renders_runner_parity_and_bridge_sections() {
    // Historical test id retained for the migration pin. Human v3 status
    // reports the unavailable Zone and never renders retired static sections.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = StatusEnv::new(&fixtures);
    let output = env.run(&["guest", "status", "corp-vm", "--zone", "work", "--human"]);
    assert_human_zone_unavailable(
        &output,
        &env,
        "d2b guest status corp-vm --zone work --human",
    );

    let rendered = rendered_output(&output);
    assert!(
        !rendered.contains("runner parity") && !rendered.contains("Bridge health"),
        "v3 human status must not render retired static sections:\n{rendered}"
    );
}
