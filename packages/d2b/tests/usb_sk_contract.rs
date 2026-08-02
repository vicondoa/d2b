//! CLI-contract coverage for the v3 ModernCli USB resource surface.
//!
//! The former tests exercised the retired `usb security-key` command family
//! and compared its static goldens or not-yet-implemented envelopes. The v3
//! CLI deliberately has no top-level `usb` command. These tests keep the
//! historical libtest names because the migration census pins them, but assert
//! usage rejection for every retired invocation and the typed `device usb`
//! help and Zone error contracts instead.
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
const RETIRED_USB_MARKER: &str = "unrecognized subcommand 'usb'";

struct UsbSecurityKeyEnv {
    _tmp: TempDir,
    manifest: PathBuf,
    bundle: PathBuf,
    missing_public: PathBuf,
    missing_broker: PathBuf,
    legacy_cli: PathBuf,
    tool_path: PathBuf,
}

impl UsbSecurityKeyEnv {
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

fn assert_no_retired_transport(output: &Output, env: &UsbSecurityKeyEnv, label: &str) {
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
    let lowercase = rendered.to_ascii_lowercase();
    assert!(
        !rendered.contains(SECRET_CANARY),
        "{label}: leaked the private canary\n{rendered}"
    );
    for marker in ["bash", "ssh", "static", "fallback"] {
        assert!(
            !lowercase.contains(marker),
            "{label}: exposed a retired {marker} path\n{rendered}"
        );
    }
    for path in [
        &env.missing_public,
        &env.missing_broker,
        &env.manifest,
        &env.bundle,
        &env.legacy_cli,
    ] {
        assert!(
            !rendered.contains(&path.display().to_string()),
            "{label}: leaked private path {}\n{rendered}",
            path.display()
        );
    }
}

fn json_value(output: &Output, label: &str) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "{label}: stdout was not valid JSON: {err}\n{}",
            rendered_output(output)
        )
    })
}

fn assert_zone_unavailable(
    output: &Output,
    env: &UsbSecurityKeyEnv,
    label: &str,
    zone_ref: &str,
    json_mode: bool,
) {
    assert_no_retired_transport(output, env, label);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label}: unexpected exit code\n{}",
        rendered_output(output)
    );

    if json_mode {
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
            value["errorClass"], "zone-unavailable",
            "{label}: unexpected error class"
        );
        assert_eq!(
            value["message"], "Zone runtime is unavailable",
            "{label}: unexpected message"
        );
        assert_eq!(
            value["schemaVersion"], 1,
            "{label}: unexpected JSON schema version"
        );
    } else {
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
}

fn assert_zone_unavailable_modes(env: &UsbSecurityKeyEnv, args: &[&str], label: &str) {
    let mut json_args = args.to_vec();
    json_args.extend_from_slice(&["--zone", "work", "--json"]);
    let json = env.run(&json_args);
    assert_zone_unavailable(&json, env, label, "Zone/work", true);

    let mut human_args = args.to_vec();
    human_args.extend_from_slice(&["--zone", "work", "--human"]);
    let human = env.run(&human_args);
    assert_zone_unavailable(&human, env, label, "Zone/work", false);
}

fn assert_usage_rejection(output: &Output, env: &UsbSecurityKeyEnv, label: &str, marker: &str) {
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

fn assert_retired_security_key(env: &UsbSecurityKeyEnv, args: &[&str], label: &str) {
    let output = env.run(args);
    assert_usage_rejection(&output, env, label, RETIRED_USB_MARKER);
}

fn assert_success(output: &Output, label: &str) {
    assert!(
        output.status.success(),
        "{label}: expected success; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn usb_security_key_help_matches_golden() {
    // Historical test id retained for the migration pin. The typed v3 help
    // output replaces the retired security-key golden.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);

    let modern = env.run(&["device", "usb", "--help"]);
    assert_success(&modern, "d2b device usb --help");
    assert_no_retired_transport(&modern, &env, "d2b device usb --help");
    assert!(
        modern.stderr.is_empty(),
        "d2b device usb --help must not write stderr:\n{}",
        String::from_utf8_lossy(&modern.stderr)
    );
    let help = String::from_utf8_lossy(&modern.stdout);
    assert!(
        help.contains("Usage: d2b device usb"),
        "typed USB help must identify the v3 command:\n{help}"
    );
    for subcommand in ["attach", "detach", "probe"] {
        assert!(
            help.contains(subcommand),
            "typed USB help is missing {subcommand}:\n{help}"
        );
    }

    assert_retired_security_key(
        &env,
        &["usb", "security-key", "--help"],
        "retired d2b usb security-key --help",
    );
}

#[test]
fn usb_security_key_cancel_current_dry_run_matches_golden() {
    // The supported probe is Zone-backed and must report the typed unavailable
    // response in both output modes, not consult the retired static artifacts.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);
    assert_zone_unavailable_modes(&env, &["device", "usb", "probe"], "d2b device usb probe");

    assert_retired_security_key(
        &env,
        &["usb", "security-key", "cancel", "--current", "--dry-run"],
        "retired d2b usb security-key cancel --current --dry-run",
    );
}

#[test]
fn usb_security_key_test_dry_run_matches_golden() {
    // Keep a typed Device mutation probe alongside the retired rejection.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);
    assert_zone_unavailable_modes(
        &env,
        &[
            "device",
            "usb",
            "attach",
            "Device/corp-vm",
            "1-2",
            "--dry-run",
        ],
        "d2b device usb attach Device/corp-vm 1-2 --dry-run",
    );

    assert_retired_security_key(
        &env,
        &["usb", "security-key", "test", "corp-vm", "--dry-run"],
        "retired d2b usb security-key test corp-vm --dry-run",
    );
}

#[test]
fn usb_security_key_status_not_yet_implemented() {
    // The old status path is a parser rejection, not a not-yet-implemented
    // response. The modern detach command still uses the typed Zone envelope.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);
    assert_zone_unavailable_modes(
        &env,
        &[
            "device",
            "usb",
            "detach",
            "Device/corp-vm",
            "1-2",
            "--dry-run",
        ],
        "d2b device usb detach Device/corp-vm 1-2 --dry-run",
    );
    assert_retired_security_key(
        &env,
        &["usb", "security-key", "status", "--json"],
        "retired d2b usb security-key status --json",
    );
}

#[test]
fn usb_security_key_sessions_not_yet_implemented() {
    // The historical sessions assertion must not preserve a v2 envelope.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);
    assert_retired_security_key(
        &env,
        &["usb", "security-key", "sessions", "--json"],
        "retired d2b usb security-key sessions --json",
    );
}

#[test]
fn usb_security_key_test_apply_not_yet_implemented() {
    // `--apply` remains a typed mutation flag on the v3 USB resource surface.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);
    assert_zone_unavailable_modes(
        &env,
        &[
            "device",
            "usb",
            "attach",
            "Device/corp-vm",
            "1-2",
            "--apply",
        ],
        "d2b device usb attach Device/corp-vm 1-2 --apply",
    );
    assert_retired_security_key(
        &env,
        &["usb", "security-key", "test", "corp-vm", "--json"],
        "retired d2b usb security-key test corp-vm --json",
    );
}

#[test]
fn usb_security_key_cancel_apply_not_yet_implemented() {
    // A typed detach request also fails closed when the Zone is unavailable.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbSecurityKeyEnv::new(&fixtures);
    assert_zone_unavailable_modes(
        &env,
        &[
            "device",
            "usb",
            "detach",
            "Device/corp-vm",
            "1-2",
            "--apply",
        ],
        "d2b device usb detach Device/corp-vm 1-2 --apply",
    );
    assert_retired_security_key(
        &env,
        &[
            "usb",
            "security-key",
            "cancel",
            "--current",
            "--apply",
            "--json",
        ],
        "retired d2b usb security-key cancel --current --apply --json",
    );
}
