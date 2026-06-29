//! CLI-output golden contract migrated from `tests/cli-json-drift.sh`.
//!
//! The schema-generation half of that shell gate now lives in
//! `tests/unit/gates/drift-check.sh`; this test owns the committed
//! `tests/golden/cli-output/*.golden` output contract that remained.  Existing
//! CLI-contract tests already cover the daemon-only runtime semantics for the
//! list/status/audit/host-check/auth/usb surfaces; this module adds golden
//! checks for the unique dry-run renderer matrix and keeps the committed v0.4.0
//! bash subset goldens tied to the Rust JSON shape.  A few retired-process
//! phrases in old goldens are normalized to the committed daemon-only wording
//! before runtime comparison; those spec corrections are documented in the
//! migration commit.

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use d2b_contracts::cli_output::UsbProbeOutputV1;
use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, UnixAddr, accept, bind, listen, socket,
};
use serde_json::{Value, json};

const SYSTEM_STATE_JSON: &str = r#"{
  "units": {
    "d2bd.service": "inactive",
    "d2b@corp-vm.service": "inactive",
    "microvm@corp-vm.service": "inactive",
    "d2b@sys-work-net.service": "active",
    "microvm@sys-work-net.service": "active"
  },
  "bridges": {
    "br-work-lan": {
      "state": "UP",
      "admin": "up",
      "expectedCarrier": "NO-CARRIER",
      "result": "ok"
    },
    "br-work-up": {
      "state": "UP",
      "admin": "up",
      "expectedCarrier": "UP",
      "result": "ok"
    }
  }
}"#;

const AUTH_LAUNCHER_JSON: &str = r#"{
  "publicReachable": true,
  "publicVersion": "0.4.0-test",
  "brokerReachable": false,
  "brokerVersion": null
}"#;

struct FixtureEnv {
    _tmp: tempfile::TempDir,
    tree: PathBuf,
    system_state: PathBuf,
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

        let system_state = tmp.path().join("system-state.json");
        fs::write(&system_state, SYSTEM_STATE_JSON).expect("write system-state fixture");

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
            system_state,
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
            // Read-only verbs (list/status/...) prefer d2bd's public socket
            // for live VM status (d098dfca); point it + the broker socket at
            // non-existent paths so spawns fall back to the static fixture
            // inventory instead of the operator's live daemon.
            .env("D2B_PUBLIC_SOCKET", self.tree.join("public.sock"))
            .env("D2B_BROKER_SOCKET", self.tree.join("priv.sock"));
        for (key, value) in envs {
            cmd.env(key, value);
        }
        cmd.output()
            .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
    }

    fn run_with_shape(&self, shape: &str, args: &[&str]) -> Output {
        let mut cmd = base_command(args, &self.home, &self.runtime);
        cmd.env("D2B_MANIFEST_PATH", self.tree.join("manifest.json"))
            .env("D2B_BUNDLE_PATH", self.tree.join("bundle.json"))
            .env("D2B_DAEMON_STATE_DIR", &self.daemon_state)
            .env("D2B_TEST_DEPLOYMENT_SHAPE", shape);
        cmd.output()
            .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
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

fn normalized_runtime_golden(name: &str) -> String {
    let expected = golden(name);
    expected
        .replace(
            "uses the daemon-first bridge and falls back to the legacy bash path only when needed.",
            "routes through d2bd → broker.",
        )
        .replace(
            "W8 keys rotate --dry-run: planned operation. --apply uses the daemon-first bridge and broker audit when the daemon handles the request.",
            "d2b keys rotate --dry-run: planned operation. --apply routes through d2bd → broker RunKeysRotate with broker audit.",
        )
        .replace(
            "W8 keys trust --dry-run: planned operation. --apply uses the daemon-first bridge and broker audit when the daemon handles the request.",
            "d2b keys trust --dry-run: planned operation. --apply routes through d2bd → broker RunKeysRotate with broker audit.",
        )
        .replace(
            "W8 keys rotate-known-host --dry-run: planned operation. --apply uses the daemon-first bridge and broker audit when the daemon handles the request.",
            "d2b keys rotate-known-host --dry-run: planned operation. --apply routes through d2bd → broker RunKeysRotate with broker audit.",
        )
        .replace("W3 host-prepare", "host-prepare")
        .replace("v1.1-P2", "v1.1")
        .replace("W15: dry-run preview retained;", "dry-run preview;")
        .replace("with the W3 socket ACLs", "with socket ACLs")
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

fn list_v04_subset(value: &Value) -> Value {
    Value::Array(
        value
            .as_array()
            .expect("list output is an array")
            .iter()
            .map(|item| {
                json!({
                    "name": item["name"].clone(),
                    "env": item["env"].clone(),
                    "graphics": item["graphics"].clone(),
                    "tpm": item["tpm"].clone(),
                    "usbip": item["usbip"].clone(),
                    "staticIp": item["staticIp"].clone(),
                    "status": item["status"].clone(),
                    "isNetVm": item["isNetVm"].clone(),
                })
            })
            .collect(),
    )
}

fn status_v04_subset(value: &Value) -> Value {
    json!({
        "name": value["name"].clone(),
        "services": value["services"].clone(),
        "current": value["current"].clone(),
        "booted": value["booted"].clone(),
        "pendingRestart": value["pendingRestart"].clone(),
    })
}

#[test]
fn list_output_matches_cli_json_drift_goldens() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    let human = env.run(
        &["list", "--human"],
        &[("D2B_TEST_SYSTEM_STATE_JSON", &env.system_state)],
    );
    assert_matches_golden(&human, "list-human.golden", "list --human");

    let json = env.run(
        &["list", "--json"],
        &[("D2B_TEST_SYSTEM_STATE_JSON", &env.system_state)],
    );
    assert_matches_golden(&json, "list-json.golden", "list --json");

    let rust_subset = list_v04_subset(&json_value(&json.stdout, "list --json"));
    let bash_subset = json_value(
        golden("list.v04bash.golden").as_bytes(),
        "list.v04bash.golden",
    );
    assert_eq!(
        rust_subset, bash_subset,
        "list rust JSON stays equivalent to the v0.4.0 bash subset"
    );
}

#[test]
fn status_goldens_preserve_v04_bash_subset() {
    let status = json_value(
        golden("status-json.golden").as_bytes(),
        "status-json.golden",
    );
    let rust_subset = status_v04_subset(&status);
    let bash_subset = json_value(
        golden("status.v04bash.golden").as_bytes(),
        "status.v04bash.golden",
    );
    assert_eq!(
        rust_subset, bash_subset,
        "status rust JSON golden stays equivalent to the v0.4.0 bash subset"
    );
    assert!(
        golden("status-human.golden").contains("=== corp-vm ==="),
        "status-human.golden remains the committed corp-vm human contract"
    );
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
            &["auth", "status", "--test-uid", "1000", "--human"][..],
            vec![("D2B_AUTH_STATUS_FIXTURE", env.auth_status.as_path())],
            "auth-status-human.golden",
            "auth status --test-uid 1000 --human",
        ),
        (
            &["auth", "status", "--test-uid", "1000", "--json"][..],
            vec![("D2B_AUTH_STATUS_FIXTURE", env.auth_status.as_path())],
            "auth-status-json.golden",
            "auth status --test-uid 1000 --json",
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
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for (args, golden_name, label) in [
        (
            &["vm", "start", "corp-vm", "--dry-run", "--human"][..],
            "vm-start-dry-run-human.golden",
            "vm start corp-vm --dry-run --human",
        ),
        (
            &["vm", "start", "corp-vm", "--dry-run", "--json"][..],
            "vm-start-dry-run-json.golden",
            "vm start corp-vm --dry-run --json",
        ),
        (
            &["vm", "stop", "corp-vm", "--dry-run", "--human"][..],
            "vm-stop-dry-run-human.golden",
            "vm stop corp-vm --dry-run --human",
        ),
        (
            &["vm", "stop", "corp-vm", "--dry-run", "--json"][..],
            "vm-stop-dry-run-json.golden",
            "vm stop corp-vm --dry-run --json",
        ),
        (
            &["vm", "restart", "corp-vm", "--dry-run", "--human"][..],
            "vm-restart-dry-run-human.golden",
            "vm restart corp-vm --dry-run --human",
        ),
        (
            &["vm", "restart", "corp-vm", "--dry-run", "--json"][..],
            "vm-restart-dry-run-json.golden",
            "vm restart corp-vm --dry-run --json",
        ),
    ] {
        let out = env.run(args, &[]);
        assert_matches_golden(&out, golden_name, label);
    }
}

#[test]
fn top_level_lifecycle_dry_run_outputs_match_goldens() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for (args, golden_name, label) in [
        (
            &["switch", "corp-vm", "--dry-run", "--human"][..],
            "switch-dry-run-human.golden",
            "switch corp-vm --dry-run --human",
        ),
        (
            &["switch", "corp-vm", "--dry-run", "--json"][..],
            "switch-dry-run-json.golden",
            "switch corp-vm --dry-run --json",
        ),
        (
            &["boot", "corp-vm", "--dry-run", "--human"][..],
            "boot-dry-run-human.golden",
            "boot corp-vm --dry-run --human",
        ),
        (
            &["boot", "corp-vm", "--dry-run", "--json"][..],
            "boot-dry-run-json.golden",
            "boot corp-vm --dry-run --json",
        ),
        (
            &["test", "corp-vm", "--dry-run", "--human"][..],
            "test-dry-run-human.golden",
            "test corp-vm --dry-run --human",
        ),
        (
            &["test", "corp-vm", "--dry-run", "--json"][..],
            "test-dry-run-json.golden",
            "test corp-vm --dry-run --json",
        ),
        (
            &["rollback", "corp-vm", "--dry-run", "--human"][..],
            "rollback-dry-run-human.golden",
            "rollback corp-vm --dry-run --human",
        ),
        (
            &["rollback", "corp-vm", "--dry-run", "--json"][..],
            "rollback-dry-run-json.golden",
            "rollback corp-vm --dry-run --json",
        ),
        (
            &["gc", "--dry-run", "--human"][..],
            "gc-dry-run-human.golden",
            "gc --dry-run --human",
        ),
        (
            &["gc", "--dry-run", "--json"][..],
            "gc-dry-run-json.golden",
            "gc --dry-run --json",
        ),
        (
            &["keys", "rotate", "corp-vm", "--dry-run", "--human"][..],
            "keys-rotate-dry-run-human.golden",
            "keys rotate corp-vm --dry-run --human",
        ),
        (
            &["keys", "rotate", "corp-vm", "--dry-run", "--json"][..],
            "keys-rotate-dry-run-json.golden",
            "keys rotate corp-vm --dry-run --json",
        ),
        (
            &["trust", "corp-vm", "--dry-run", "--human"][..],
            "trust-dry-run-human.golden",
            "trust corp-vm --dry-run --human",
        ),
        (
            &["trust", "corp-vm", "--dry-run", "--json"][..],
            "trust-dry-run-json.golden",
            "trust corp-vm --dry-run --json",
        ),
        (
            &["rotate-known-host", "corp-vm", "--dry-run", "--human"][..],
            "rotate-known-host-dry-run-human.golden",
            "rotate-known-host corp-vm --dry-run --human",
        ),
        (
            &["rotate-known-host", "corp-vm", "--dry-run", "--json"][..],
            "rotate-known-host-dry-run-json.golden",
            "rotate-known-host corp-vm --dry-run --json",
        ),
    ] {
        let out = env.run(args, &[]);
        assert_matches_golden(&out, golden_name, label);
    }
}

#[test]
fn host_lifecycle_dry_run_outputs_match_goldens() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for (args, golden_name, label) in [
        (
            &["host", "prepare", "--dry-run", "--human"][..],
            "host-prepare-dry-run-human.golden",
            "host prepare --dry-run --human",
        ),
        (
            &["host", "prepare", "--dry-run", "--json"][..],
            "host-prepare-dry-run-json.golden",
            "host prepare --dry-run --json",
        ),
        (
            &["host", "destroy", "--dry-run", "--human"][..],
            "host-destroy-dry-run-human.golden",
            "host destroy --dry-run --human",
        ),
        (
            &["host", "destroy", "--dry-run", "--json"][..],
            "host-destroy-dry-run-json.golden",
            "host destroy --dry-run --json",
        ),
        (
            &["migrate", "--dry-run", "--human"][..],
            "migrate-dry-run-human.golden",
            "migrate --dry-run --human",
        ),
        (
            &["migrate", "--dry-run", "--json"][..],
            "migrate-dry-run-json.golden",
            "migrate --dry-run --json",
        ),
    ] {
        let out = env.run_with_shape("all-daemon", args);
        assert_matches_golden(&out, golden_name, label);
    }

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
fn host_migrate_storage_dry_run_json_reports_checkpoint_and_rollback() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    let out = env.run(&["host", "migrate-storage", "--dry-run", "--json"], &[]);
    assert_success(&out, "host migrate-storage --dry-run --json");
    let value: Value = serde_json::from_slice(&out.stdout).expect("parse migrate-storage JSON");
    assert_eq!(value["command"], "host migrate-storage");
    assert_eq!(value["mode"], "dry-run");
    let checkpoint = value["checkpointId"].as_str().expect("checkpointId string");
    assert!(checkpoint.starts_with("storage-cutover-"));
    assert!(
        value["rollbackCommand"]
            .as_str()
            .expect("rollback command string")
            .contains(checkpoint)
    );
    assert_eq!(value["applyStatus"], "not-implemented-in-this-build");
    assert!(
        value["preserve"]
            .as_array()
            .expect("preserve array")
            .iter()
            .any(|entry| entry.as_str().is_some_and(|s| s.contains("swtpm NVRAM")))
    );
    assert!(
        value["preflightRequirements"]
            .as_array()
            .expect("preflight array")
            .iter()
            .any(|entry| entry
                .as_str()
                .is_some_and(|s| s.contains("net VMs stopped")))
    );
    assert!(
        value["failClosedHazards"]
            .as_array()
            .expect("hazards array")
            .iter()
            .any(|entry| entry
                .as_str()
                .is_some_and(|s| s.contains("unlink lock files")))
    );
}

#[test]
fn host_migrate_storage_apply_fails_closed_without_mutation() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    let out = env.run(&["host", "migrate-storage", "--apply", "--json"], &[]);
    assert!(
        !out.status.success(),
        "apply should fail closed until broker support lands"
    );
    let value: Value = serde_json::from_slice(&out.stdout).expect("parse apply refusal JSON");
    assert_eq!(value["code"], "storage-migration-apply-not-implemented");
    assert_eq!(value["exitCode"], 78);
    assert!(
        value["remediation"]
            .as_str()
            .expect("remediation")
            .contains("host migrate-storage --dry-run")
    );
}

#[test]
fn host_migrate_storage_rollback_fails_closed_without_mutation() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    let out = env.run(
        &[
            "host",
            "migrate-storage",
            "--rollback",
            "--from-checkpoint",
            "storage-cutover-test",
            "--json",
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "rollback should fail closed until broker support lands"
    );
    let value: Value = serde_json::from_slice(&out.stdout).expect("parse rollback refusal JSON");
    assert_eq!(value["code"], "storage-migration-rollback-not-implemented");
    assert_eq!(value["exitCode"], 78);
    assert!(
        value["observedState"]
            .as_str()
            .expect("observed state")
            .contains("storage-cutover-test")
    );
}

#[test]
fn usb_dry_run_outputs_match_goldens() {
    let Some(env) = FixtureEnv::new() else {
        return;
    };
    for (args, golden_name, label) in [
        (
            &["usb", "attach", "corp-vm", "1-2", "--dry-run", "--human"][..],
            "usb-attach-dry-run-human.golden",
            "usb attach corp-vm 1-2 --dry-run --human",
        ),
        (
            &["usb", "attach", "corp-vm", "1-2", "--dry-run", "--json"][..],
            "usb-attach-dry-run-json.golden",
            "usb attach corp-vm 1-2 --dry-run --json",
        ),
        (
            &["usb", "detach", "corp-vm", "1-2", "--dry-run", "--human"][..],
            "usb-detach-dry-run-human.golden",
            "usb detach corp-vm 1-2 --dry-run --human",
        ),
        (
            &["usb", "detach", "corp-vm", "1-2", "--dry-run", "--json"][..],
            "usb-detach-dry-run-json.golden",
            "usb detach corp-vm 1-2 --dry-run --json",
        ),
    ] {
        let out = env.run(args, &[]);
        assert_matches_golden(&out, golden_name, label);
    }
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
    let out = base_command(&["usb", "probe", "--json"], &home, &runtime)
        .env("D2B_PUBLIC_SOCKET", &socket_path)
        .output()
        .expect("spawn d2b usb probe --json");
    server.join().expect("usb probe mock daemon");
    assert_success(&out, "usb probe --json");

    let parsed: UsbProbeOutputV1 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "usb probe --json did not match UsbProbeOutputV1: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(parsed.command, "usb probe");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].vm, "corp-vm");
    assert_eq!(parsed.entries[0].bus_id, "1-2");
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
            req["type"], "usbipProbe",
            "expected usbipProbe frame, got {req}"
        );
        send_frame(
            conn,
            &json!({
                "type": "usbipProbeResponse",
                "entries": [
                    {
                        "vm": "corp-vm",
                        "env": "work",
                        "busId": "1-2",
                        "lockPath": "/run/d2b/locks/usbip/1-2",
                        "status": "degraded",
                        "ownerVm": "corp-vm",
                        "durableClaim": {
                            "state": "held-by-desired-owner",
                            "ownerVm": "corp-vm"
                        },
                        "host": {
                            "bind": "unknown",
                            "carrier": "unknown",
                            "proxy": "unknown"
                        },
                        "guest": {
                            "import": "detached"
                        },
                        "topologyPolicy": {
                            "topology": "unknown",
                            "policy": "allowed"
                        },
                        "degradedReasons": [
                            {
                                "code": "guest-import-unavailable",
                                "summary": "the guest USBIP import has not converged",
                                "remediation": "Run `d2b usb attach corp-vm 1-2 --apply` after the VM is running."
                            }
                        ],
                        "remediationCommands": [
                            "d2b usb attach corp-vm 1-2 --apply"
                        ]
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
