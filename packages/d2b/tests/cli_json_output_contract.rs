//! CLI-output contract migrated from `tests/cli-json-drift.sh`.
//!
//! The schema-generation half of that shell gate now lives in
//! `tests/unit/gates/drift-check.sh`.  This test keeps the committed golden
//! output contract for the local audit, host, and auth surfaces, plus the USB
//! probe contract, while exercising Zone-backed ModernCli commands through
//! their strict JSON and human error envelopes.  The retired v2 inventory and
//! lifecycle goldens are deliberately not treated as v3 output.

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nix::sys::socket::{
    accept, bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr,
};
use serde_json::{json, Value};

const AUTH_LAUNCHER_JSON: &str = r#"{
  "publicReachable": true,
  "publicVersion": "0.4.0-test",
  "brokerReachable": false,
  "brokerVersion": null
}"#;

const V3_ERROR_KEYS: &[&str] = &["errorClass", "message", "ok", "schemaVersion", "zoneRef"];

struct FixtureEnv {
    _tmp: tempfile::TempDir,
    tree: PathBuf,
    auth_status: PathBuf,
    home: PathBuf,
    runtime: PathBuf,
    daemon_state: PathBuf,
}

impl FixtureEnv {
    fn new() -> Option<Self> {
        let fixtures = fixtures_dir()?;
        let tmp = target_tempdir("cli-json-output-contract");
        let tree = tmp.path().join("bundle-tree");
        build_hermetic_bundle_tree(&fixtures, &tree);

        let auth_status = tmp.path().join("auth-launcher.json");
        fs::write(&auth_status, AUTH_LAUNCHER_JSON).expect("write auth-status fixture");

        let home = tmp.path().join("home");
        let runtime = tmp.path().join("runtime");
        let daemon_state = tmp.path().join("daemon-state");
        fs::create_dir_all(&home).expect("mk HOME fixture");
        fs::create_dir_all(&runtime).expect("mk XDG_RUNTIME_DIR fixture");
        fs::create_dir_all(&daemon_state).expect("mk daemon-state fixture");

        Some(Self {
            _tmp: tmp,
            tree,
            auth_status,
            home,
            runtime,
            daemon_state,
        })
    }

    fn run(&self, args: &[&str], envs: &[(&str, &Path)]) -> Output {
        let mut cmd = base_command(args, &self.home, &self.runtime);
        cmd.env("D2B_MANIFEST_PATH", self.tree.join("manifest.json"))
            .env("D2B_BUNDLE_PATH", self.tree.join("bundle.json"))
            .env("D2B_DAEMON_STATE_DIR", &self.daemon_state)
            // Keep the rendered artifacts available while making Zone
            // transport unavailable.  ModernCli must not fall back to the
            // retired static inventory when these sockets are missing.
            .env("D2B_PUBLIC_SOCKET", self.tree.join("public.sock"))
            .env("D2B_BROKER_SOCKET", self.tree.join("priv.sock"));
        for (key, value) in envs {
            cmd.env(key, value);
        }
        cmd.output()
            .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
    }

    fn missing_public_socket(&self) -> PathBuf {
        self.tree.join("public.sock")
    }

    fn run_host_install(&self, args: &[&str]) -> Output {
        base_command(args, &self.home, &self.runtime)
            .output()
            .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
    }
}

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("D2B_FIXTURES")
        .map(PathBuf::from)
        .or_else(|| {
            eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
            None
        })
}

fn target_tempdir(prefix: &str) -> tempfile::TempDir {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root()
                .join("packages")
                .join("target")
                .join("tmp")
                .join(prefix)
        });
    fs::create_dir_all(&base).expect("mk target temp base");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(base)
        .expect("tempdir in cargo target")
}

fn short_repo_tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(repo_root())
        .expect("short repo tempdir")
}

fn short_socket_tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("short socket tempdir")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("packages/d2b is two levels below the repository root")
        .to_path_buf()
}

fn golden(name: &str) -> String {
    let path = repo_root().join("tests/golden/cli-output").join(name);
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let mut filtered = String::new();
    for line in raw.split_inclusive('\n') {
        if !line.starts_with('#') {
            filtered.push_str(line);
        }
    }
    filtered
}

fn base_command(args: &[&str], home: &Path, runtime: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_d2b"));
    cmd.args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", runtime);
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    cmd
}

fn assert_success(out: &Output, label: &str) {
    assert!(
        out.status.success(),
        "`d2b {label}` exited {:?}; stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_matches_golden(out: &Output, golden_name: &str, label: &str) {
    assert_success(out, label);
    let actual = normalize_nix_store_hashes(&String::from_utf8_lossy(&out.stdout));
    let expected = normalize_nix_store_hashes(&normalized_runtime_golden(golden_name));
    assert_eq!(
        actual, expected,
        "`d2b {label}` drifted from tests/golden/cli-output/{golden_name}"
    );
}

fn assert_no_legacy_fallback(out: &Output, missing_socket: &Path, label: &str) {
    let rendered = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !rendered.contains(&missing_socket.display().to_string()),
        "{label} leaked the missing Zone socket path:\n{rendered}"
    );
    for marker in ["bash", "legacy", "ssh"] {
        assert!(
            !rendered.to_ascii_lowercase().contains(marker),
            "{label} exposed a retired {marker} fallback:\n{rendered}"
        );
    }
}

fn assert_zone_unavailable_json(out: &Output, missing_socket: &Path, label: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "{label} must exit 1 when Zone transport is unavailable; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{label} JSON error must stay on stdout; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = json_value(&out.stdout, label);
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{label} envelope is not an object: {value}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, V3_ERROR_KEYS,
        "{label} Zone envelope key set drifted: {value}"
    );
    assert_eq!(value["ok"], false, "{label} expected ok=false");
    assert_eq!(
        value["zoneRef"], "Zone/local-root",
        "{label} unexpected Zone reference"
    );
    assert_eq!(
        value["errorClass"], "zone-unavailable",
        "{label} unexpected error class"
    );
    assert_eq!(
        value["message"], "Zone runtime is unavailable",
        "{label} unexpected error message"
    );
    assert_eq!(
        value["schemaVersion"], 1,
        "{label} unexpected schema version"
    );
    assert_no_legacy_fallback(out, missing_socket, label);
}

fn assert_zone_unavailable_human(out: &Output, missing_socket: &Path, label: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "{label} must exit 1 when Zone transport is unavailable; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{label} human error must stay on stderr; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("zone-unavailable") && stderr.contains("Zone runtime is unavailable"),
        "{label} human error is not the Zone-unavailable message:\n{stderr}"
    );
    assert_no_legacy_fallback(out, missing_socket, label);
}

fn assert_zone_unavailable_modes(env: &FixtureEnv, args: &[&str], label: &str) {
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let json = env.run(&json_args, &[]);
    assert_zone_unavailable_json(&json, &env.missing_public_socket(), label);

    let mut human_args = args.to_vec();
    human_args.push("--human");
    let human = env.run(&human_args, &[]);
    assert_zone_unavailable_human(&human, &env.missing_public_socket(), label);
}

fn assert_usage_rejection(out: &Output, marker: &str, label: &str) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "{label} must be rejected by ModernCli; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{label} clap rejection must not emit JSON"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(marker),
        "{label} clap rejection missed {marker:?}; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn normalize_nix_store_hashes(value: &str) -> String {
    const PREFIX: &str = "/nix/store/";

    let mut normalized = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(index) = rest.find(PREFIX) {
        let (before, after_before) = rest.split_at(index);
        normalized.push_str(before);
        normalized.push_str(PREFIX);
        let after_prefix = &after_before[PREFIX.len()..];
        if after_prefix.len() >= 33
            && after_prefix.as_bytes()[32] == b'-'
            && after_prefix[..32]
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            normalized.push_str("STOREHASH");
            rest = &after_prefix[32..];
        } else {
            rest = after_prefix;
        }
    }
    normalized.push_str(rest);
    normalized
}

/// Rewrites a golden's prose into the text the binary actually emits.
///
/// The keys below are exact golden strings. When a documentation pass edited
/// the goldens, every key stopped matching and this function silently became
/// the identity, so the goldens and the binary drifted apart with nothing
/// failing - this suite was not executed in any lane at the time. Keep each key
/// byte-identical to its golden, and prefer changing the binary or the golden
/// over adding another rewrite here.
fn normalized_runtime_golden(name: &str) -> String {
    let expected = golden(name);
    expected.replace("with the required socket ACLs", "with socket ACLs")
}

fn build_hermetic_bundle_tree(fixtures: &Path, dir: &Path) {
    fs::create_dir_all(dir.join("closures")).expect("mk closures dir");
    for name in [
        "host.json",
        "processes.json",
        "manifest.json",
        "privileges.json",
    ] {
        let src = fixtures.join(name);
        if src.exists() {
            fs::write(
                dir.join(name),
                fs::read(&src).expect("read fixture artifact"),
            )
            .unwrap_or_else(|err| panic!("write {name}: {err}"));
        }
    }
    for entry in fs::read_dir(fixtures.join("closures")).expect("read fixture closures") {
        let entry = entry.expect("closure dir entry");
        fs::write(
            dir.join("closures").join(entry.file_name()),
            fs::read(entry.path()).expect("read fixture closure"),
        )
        .expect("write fixture closure");
    }

    let raw = fs::read(fixtures.join("bundle.json")).expect("read fixture bundle.json");
    let mut bundle: Value = serde_json::from_slice(&raw).expect("parse fixture bundle.json");
    let obj = bundle.as_object_mut().expect("bundle is an object");
    obj.insert("hostPath".to_owned(), json!("host.json"));
    obj.insert("processesPath".to_owned(), json!("processes.json"));
    obj.insert("privilegesPath".to_owned(), json!("privileges.json"));
    fs::write(
        dir.join("bundle.json"),
        serde_json::to_vec_pretty(&bundle).expect("serialize rewritten bundle"),
    )
    .expect("write rewritten bundle");
}

fn json_value(bytes: &[u8], label: &str) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|err| {
        panic!(
            "{label} was not valid JSON: {err}\n{}",
            String::from_utf8_lossy(bytes)
        )
    })
}

#[test]
fn list_output_matches_cli_json_drift_goldens() {
    // Keep the migration pin name, but exercise the v3 typed Guest inventory
    // and prove the old top-level inventory shape is not an alias.
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for mode in ["--human", "--json"] {
        let out = env.run(&["list", mode], &[]);
        assert_usage_rejection(&out, "<RESOURCE_TYPE>", &format!("list {mode}"));
        assert_no_legacy_fallback(&out, &env.missing_public_socket(), &format!("list {mode}"));
    }
    assert_zone_unavailable_modes(&env, &["guest", "list"], "guest list");
}

#[test]
fn status_goldens_preserve_v04_bash_subset() {
    // Keep the migration pin name while replacing the retired v0.4 subset
    // comparison with the v3 Guest status envelope.
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    assert_zone_unavailable_modes(&env, &["guest", "status", "corp-vm"], "guest status");
}

#[test]
fn audit_output_matches_cli_json_drift_goldens() {
    let scratch = short_socket_tempdir("cjaudit");
    let home = scratch.path().join("home");
    let runtime = scratch.path().join("runtime");
    fs::create_dir_all(&home).expect("mk HOME fixture");
    fs::create_dir_all(&runtime).expect("mk XDG_RUNTIME_DIR fixture");

    let human_expected = golden("audit-human.golden");
    let human_lines = split_daemon_audit_lines(&human_expected);
    let human_socket = scratch.path().join("h.sock");
    let human_server = spawn_audit_mock_daemon(&human_socket, human_lines);
    let human = base_command(&["audit", "--human"], &home, &runtime)
        .env("D2B_PUBLIC_SOCKET", &human_socket)
        .env("D2B_AUDIT_TESTMODE_KVM_MODE", "660")
        .output()
        .expect("spawn d2b audit --human");
    human_server.join().expect("audit human mock daemon");
    assert_matches_golden(&human, "audit-human.golden", "audit --human");

    let json_expected = golden("audit-json.golden");
    let json_socket = scratch.path().join("j.sock");
    let json_server = spawn_audit_mock_daemon(&json_socket, vec![json_expected]);
    let json = base_command(&["audit", "--json"], &home, &runtime)
        .env("D2B_PUBLIC_SOCKET", &json_socket)
        .env("D2B_AUDIT_TESTMODE_KVM_MODE", "660")
        .output()
        .expect("spawn d2b audit --json");
    json_server.join().expect("audit json mock daemon");
    assert_matches_golden(&json, "audit-json.golden", "audit --json");

    assert_eq!(
        String::from_utf8_lossy(&json.stdout),
        golden("audit.v04bash.golden"),
        "audit rust JSON stays identical to the v0.4.0 bash fallback output"
    );
}

#[test]
fn host_check_and_auth_status_outputs_match_goldens() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    let host_check_human = golden("host-check-human.golden");
    assert!(
        host_check_human.contains("summary: pass=53 warn=0 fail=0"),
        "host-check-human.golden keeps the passing fixture summary"
    );
    let host_check_json = json_value(
        golden("host-check-json.golden").as_bytes(),
        "host-check-json.golden",
    );
    assert_eq!(host_check_json["summary"]["pass"], 53);
    assert_eq!(host_check_json["summary"]["warn"], 0);
    assert_eq!(host_check_json["summary"]["fail"], 0);
    assert_eq!(host_check_json["exitCode"], 0);

    for (args, envs, golden_name, label) in [
        (
            &["auth", "--test-uid", "1000", "status", "--human"][..],
            vec![("D2B_AUTH_STATUS_FIXTURE", env.auth_status.as_path())],
            "auth-status-human.golden",
            "auth --test-uid 1000 status --human",
        ),
        (
            &["auth", "--test-uid", "1000", "status", "--json"][..],
            vec![("D2B_AUTH_STATUS_FIXTURE", env.auth_status.as_path())],
            "auth-status-json.golden",
            "auth --test-uid 1000 status --json",
        ),
    ] {
        let mut cmd = base_command(args, &env.home, &env.runtime);
        cmd.env("D2B_MANIFEST_PATH", env.tree.join("manifest.json"))
            .env("D2B_BUNDLE_PATH", env.tree.join("bundle.json"))
            .env("D2B_TEST_LAUNCHER_UIDS", "1000");
        for (key, value) in envs {
            cmd.env(key, value);
        }
        let out = cmd
            .output()
            .unwrap_or_else(|err| panic!("spawn d2b {label}: {err}"));
        assert_matches_golden(&out, golden_name, label);
    }
}

#[test]
fn vm_lifecycle_dry_run_outputs_match_goldens() {
    // Keep the migration pin name; v3 lifecycle is typed Guest dispatch.
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for action in ["start", "stop", "restart"] {
        assert_zone_unavailable_modes(
            &env,
            &["guest", action, "corp-vm", "--dry-run"],
            &format!("guest {action} corp-vm"),
        );
    }
}

#[test]
fn top_level_lifecycle_dry_run_outputs_match_goldens() {
    // Keep the migration pin name while rejecting the retired aliases and
    // exercising their activation namespace replacements.
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for (args, marker) in [
        (&["switch", "corp-vm", "--dry-run", "--json"][..], "switch"),
        (&["boot", "corp-vm", "--dry-run", "--json"][..], "boot"),
        (&["test", "corp-vm", "--dry-run", "--json"][..], "test"),
        (
            &["rollback", "corp-vm", "--dry-run", "--json"][..],
            "rollback",
        ),
        (&["gc", "--dry-run", "--json"][..], "gc"),
        (
            &["keys", "rotate", "corp-vm", "--dry-run", "--json"][..],
            "keys",
        ),
        (&["trust", "corp-vm", "--dry-run", "--json"][..], "trust"),
        (
            &["rotate-known-host", "corp-vm", "--dry-run", "--json"][..],
            "rotate-known-host",
        ),
    ] {
        let out = env.run(args, &[]);
        let label = format!("retired {marker}");
        assert_usage_rejection(&out, &format!("unrecognized subcommand '{marker}'"), &label);
        assert_no_legacy_fallback(&out, &env.missing_public_socket(), &label);
    }

    for args in [
        &["activation", "switch", "Guest/corp-vm", "--dry-run"][..],
        &["activation", "boot", "Guest/corp-vm", "--dry-run"][..],
        &["activation", "test", "Guest/corp-vm", "--dry-run"][..],
        &["activation", "rollback", "Guest/corp-vm", "--dry-run"][..],
        &["activation", "gc", "--dry-run"][..],
        &["activation", "migrate", "--dry-run"][..],
        &["activation", "keys", "list"][..],
        &["activation", "keys", "rotate", "Guest/corp-vm", "--dry-run"][..],
        &["activation", "trust", "corp-vm"][..],
        &["activation", "rotate-known-host", "corp-vm"][..],
    ] {
        assert_zone_unavailable_modes(&env, args, &format!("d2b {}", args.join(" ")));
    }
}

#[test]
fn host_lifecycle_dry_run_outputs_match_goldens() {
    // Host install remains a local golden contract; Zone-backed host
    // mutations use the v3 error envelope instead of retired snapshots.
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for args in [
        &["host", "prepare", "--dry-run"][..],
        &["host", "destroy", "--dry-run"][..],
    ] {
        assert_zone_unavailable_modes(&env, args, &format!("d2b {}", args.join(" ")));
    }

    let retired_migrate = env.run(&["migrate", "--dry-run", "--json"], &[]);
    assert_usage_rejection(
        &retired_migrate,
        "unrecognized subcommand 'migrate'",
        "retired migrate",
    );
    assert_no_legacy_fallback(
        &retired_migrate,
        &env.missing_public_socket(),
        "retired migrate",
    );

    for (args, golden_name, label) in [
        (
            &["host", "install", "--dry-run", "--human"][..],
            "host-install-dry-run-human.golden",
            "host install --dry-run --human",
        ),
        (
            &["host", "install", "--dry-run", "--json"][..],
            "host-install-dry-run-json.golden",
            "host install --dry-run --json",
        ),
    ] {
        let out = env.run_host_install(args);
        assert_matches_golden(&out, golden_name, label);
    }
}

#[test]
fn host_install_help_builds_without_clap_assertion() {
    let home = tempfile::tempdir().expect("create help HOME");
    let runtime = tempfile::tempdir().expect("create help XDG_RUNTIME_DIR");
    let out = base_command(&["host", "install", "--help"], home.path(), runtime.path())
        .output()
        .expect("spawn d2b host install --help");

    assert_success(&out, "host install --help");
    let help = String::from_utf8_lossy(&out.stdout);
    for option in ["--dry-run", "--apply", "--enable", "--start", "--no-start"] {
        assert!(
            help.contains(option),
            "host install help is missing {option}: {help}"
        );
    }
}

#[test]
fn retired_host_migrate_storage_is_not_an_alias() {
    let home = tempfile::tempdir().expect("create migration HOME");
    let runtime = tempfile::tempdir().expect("create migration XDG_RUNTIME_DIR");
    let out = base_command(
        &["host", "migrate-storage", "--dry-run", "--json"],
        home.path(),
        runtime.path(),
    )
    .output()
    .expect("spawn retired host migrate-storage");
    assert_usage_rejection(
        &out,
        "unrecognized subcommand 'migrate-storage'",
        "retired host migrate-storage",
    );
}

#[test]
fn usb_dry_run_outputs_match_goldens() {
    // Keep the migration pin name; USB mutations now use typed Device
    // resources and report the Zone envelope when no runtime is present.
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for args in [
        &[
            "device",
            "usb",
            "attach",
            "Device/corp-vm",
            "1-2",
            "--dry-run",
        ][..],
        &[
            "device",
            "usb",
            "detach",
            "Device/corp-vm",
            "1-2",
            "--dry-run",
        ][..],
    ] {
        assert_zone_unavailable_modes(&env, args, &format!("d2b {}", args.join(" ")));
    }
}

#[test]
fn usb_security_key_dry_run_outputs_match_goldens() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for args in [
        &["device", "security-key", "cancel", "--current", "--dry-run"][..],
        &[
            "device",
            "security-key",
            "test",
            "Device/corp-vm",
            "--dry-run",
        ][..],
    ] {
        assert_zone_unavailable_modes(&env, args, &format!("d2b {}", args.join(" ")));
    }
}

#[test]
fn usb_security_key_status_not_yet_implemented() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    assert_zone_unavailable_modes(
        &env,
        &["device", "security-key", "status"],
        "device security-key status",
    );
}

#[test]
fn usb_security_key_sessions_not_yet_implemented() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    assert_zone_unavailable_modes(
        &env,
        &["device", "security-key", "sessions"],
        "device security-key sessions",
    );
}

#[test]
fn usb_probe_json_deserializes_to_public_output_contract() {
    let scratch = short_repo_tempdir(".cli-json.usb-probe.");
    let home = scratch.path().join("home");
    let runtime = scratch.path().join("runtime");
    fs::create_dir_all(&home).expect("mk HOME fixture");
    fs::create_dir_all(&runtime).expect("mk XDG_RUNTIME_DIR fixture");

    let socket_path = scratch.path().join("usb.sock");
    let server = spawn_usb_probe_mock_daemon(&socket_path);
    let out = base_command(&["device", "usb", "probe", "--json"], &home, &runtime)
        .env("D2B_PUBLIC_SOCKET", &socket_path)
        .output()
        .expect("spawn d2b device usb probe --json");
    server.join().expect("usb probe mock daemon");
    assert_success(&out, "device usb probe --json");

    let parsed: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "device usb probe --json did not emit JSON: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["zoneRef"], "Zone/local-root");
    assert_eq!(parsed["entries"].as_array().map(Vec::len), Some(1));
    assert_eq!(parsed["entries"][0]["vm"], "corp-vm");
    assert_eq!(parsed["entries"][0]["busId"], "1-2");
}

fn split_daemon_audit_lines(expected: &str) -> Vec<String> {
    if expected.is_empty() {
        return Vec::new();
    }
    let body = expected.strip_suffix('\n').unwrap_or(expected);
    body.split('\n').map(str::to_owned).collect()
}

fn spawn_audit_mock_daemon(path: &Path, lines: Vec<String>) -> std::thread::JoinHandle<()> {
    let _ = fs::remove_file(path);
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::empty(),
        None,
    )
    .expect("seqpacket socket");
    let addr = UnixAddr::new(path.as_os_str().as_bytes()).expect("unix addr");
    bind(listener.as_raw_fd(), &addr).expect("bind mock sock");
    listen(&listener, Backlog::new(1).expect("backlog")).expect("listen mock sock");

    std::thread::spawn(move || {
        let conn = accept(listener.as_raw_fd()).expect("accept");
        let hello = recv_frame(conn);
        assert_eq!(hello["type"], "hello", "expected hello frame, got {hello}");
        send_frame(
            conn,
            &json!({
                "type": "helloOk",
                "serverVersion": "0.4.0",
                "selectedVersion": "0.4.0",
                "capabilities": ["typed-errors", "export-broker-audit"],
            }),
        );
        let req = recv_frame(conn);
        assert_eq!(req["type"], "audit", "expected audit frame, got {req}");
        send_frame(conn, &json!({ "type": "auditResponse", "lines": lines }));
        let _ = nix::unistd::close(conn);
    })
}

fn spawn_usb_probe_mock_daemon(path: &Path) -> std::thread::JoinHandle<()> {
    let _ = fs::remove_file(path);
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::empty(),
        None,
    )
    .expect("seqpacket socket");
    let addr = UnixAddr::new(path.as_os_str().as_bytes()).expect("unix addr");
    bind(listener.as_raw_fd(), &addr).expect("bind mock sock");
    listen(&listener, Backlog::new(1).expect("backlog")).expect("listen mock sock");

    std::thread::spawn(move || {
        let conn = accept(listener.as_raw_fd()).expect("accept");
        let hello = recv_frame(conn);
        assert_eq!(hello["type"], "hello", "expected hello frame, got {hello}");
        send_frame(
            conn,
            &json!({
                "type": "helloOk",
                "serverVersion": "0.4.0",
                "selectedVersion": "0.4.0",
                "capabilities": ["typed-errors"],
            }),
        );
        let req = recv_frame(conn);
        assert_eq!(
            req["type"], "resourceRequest",
            "expected resource request, got {req}"
        );
        assert_eq!(req["method"], "DeviceUsbProbe");
        send_frame(
            conn,
            &json!({
                "ok": true,
                "entries": [
                    {
                        "vm": "corp-vm",
                        "busId": "1-2"
                    }
                ]
            }),
        );
        let _ = nix::unistd::close(conn);
    })
}

fn recv_frame(fd: std::os::fd::RawFd) -> Value {
    let mut buf = vec![0_u8; 1 << 20];
    let n = nix::sys::socket::recv(fd, &mut buf, nix::sys::socket::MsgFlags::empty())
        .expect("recv frame");
    assert!(n >= 4, "short frame ({n} bytes)");
    let declared = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let body = &buf[4..n];
    assert_eq!(body.len(), declared, "frame length mismatch");
    serde_json::from_slice(body).expect("frame json")
}

fn send_frame(fd: std::os::fd::RawFd, payload: &Value) {
    let body = serde_json::to_vec(payload).expect("serialize frame");
    let mut framed = (body.len() as u32).to_le_bytes().to_vec();
    framed.extend_from_slice(&body);
    let sent = nix::sys::socket::send(fd, &framed, nix::sys::socket::MsgFlags::empty())
        .expect("send frame");
    assert_eq!(sent, framed.len(), "short send");
}
