//! CLI-contract integration coverage for the v3 ModernCli USB surface.
//!
//! The former tests exercised the retired top-level `usb` command and its
//! static USBIP goldens. The v3 replacement is the typed `device usb` resource
//! command. These tests keep the historical libtest names because the
//! migration census pins them, but assert the v3 command and rejection
//! contracts instead of restoring an alias or comparing retired goldens.
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

struct UsbEnv {
    _tmp: TempDir,
    manifest: PathBuf,
    bundle: PathBuf,
    missing_public: PathBuf,
    missing_broker: PathBuf,
    legacy_cli: PathBuf,
    tool_path: PathBuf,
}

impl UsbEnv {
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

fn assert_no_retired_transport(output: &Output, env: &UsbEnv, label: &str) {
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

fn assert_zone_unavailable(output: &Output, env: &UsbEnv, label: &str, zone_ref: &str, mode: &str) {
    assert_no_retired_transport(output, env, label);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label}: unexpected exit code\n{}",
        rendered_output(output)
    );

    if mode == "json" {
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

fn assert_zone_unavailable_modes(env: &UsbEnv, args: &[&str], label: &str) {
    let mut json_args = args.to_vec();
    json_args.extend_from_slice(&["--zone", "work", "--json"]);
    let json = env.run(&json_args);
    assert_zone_unavailable(&json, env, label, "Zone/work", "json");

    let mut human_args = args.to_vec();
    human_args.extend_from_slice(&["--zone", "work", "--human"]);
    let human = env.run(&human_args);
    assert_zone_unavailable(&human, env, label, "Zone/work", "human");
}

fn assert_usage_rejection(output: &Output, env: &UsbEnv, label: &str, marker: &str) {
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

fn assert_success(output: &Output, label: &str) {
    assert!(
        output.status.success(),
        "{label}: expected success; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn usb_help_matches_golden() {
    // Historical test id retained for the migration pin. The typed v3 help
    // output is the replacement for the retired top-level usb golden.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbEnv::new(&fixtures);

    let modern = env.run(&["device", "usb", "--help"]);
    assert_success(&modern, "d2b device usb --help");
    assert_no_retired_transport(&modern, &env, "d2b device usb --help");
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

    let retired = env.run(&["usb", "--help"]);
    assert_usage_rejection(
        &retired,
        &env,
        "retired d2b usb --help",
        "unrecognized subcommand 'usb'",
    );
}

#[test]
fn usb_attach_dry_run_matches_golden() {
    // Historical test id retained for the migration pin. USB mutation now
    // addresses a typed Device resource and fails closed without a Zone.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbEnv::new(&fixtures);
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

    let retired = env.run(&["usb", "attach", "corp-vm", "1-2", "--dry-run", "--json"]);
    assert_usage_rejection(
        &retired,
        &env,
        "retired d2b usb attach",
        "unrecognized subcommand 'usb'",
    );
}

#[test]
fn usb_detach_dry_run_matches_golden() {
    // Historical test id retained for the migration pin. Keep the v3 Device
    // resource syntax and the same strict no-fallback behavior as attach.
    let Some(fixtures) = fixtures_dir() else {
        return;
    };
    let env = UsbEnv::new(&fixtures);
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
}
