//! W3 CLI-contract integration test, migrated from
//! tests/cli-rust-native-status.sh.
//!
//! Spawns the real `nixling` binary against the rendered fixture-smoke bundle
//! (NL_FIXTURES) + a synthetic systemd/bridge state fixture, and asserts that
//! `status`:
//!   * `--vm <name>` and the positional `<name>` form produce byte-identical
//!     `--json` output;
//!   * `--json` deserializes strictly into `nixling::StatusVmOutputV2`
//!     (`deny_unknown_fields` makes this the schema check the bash gate did via
//!     docs/reference/cli-output/status.schema.json) and is classified as the
//!     `StatusOutputV2::Vm` untagged variant;
//!   * corp-vm exposes its declared/static state: `declaredRoles` contains
//!     `cloud-hypervisor-runner` and `runnerParity.runnerParityOk == true`
//!     (closures are emitted in fixture-smoke, so parity resolves);
//!   * `--check-bridges --json` returns the frozen not-yet-implemented envelope
//!     (`StatusOutputV2::CheckBridges`, mode `check-bridges`);
//!   * `--human` renders the runner-parity and bridge-health sections.
//!
//! Requires NL_FIXTURES (the fixture-smoke output dir), delivered by the
//! dedicated CLI-contract step in tests/tools/rust-workspace-checks.sh. When unset
//! (e.g. the plain `cargo test --workspace` pass that has no Nix sandbox) the
//! test skips; the gate step always sets NL_FIXTURES, so the contract cannot be
//! silently disabled there.
//!
//! Hermeticity vs the bash gate (see tests/README.md): the bash gate pointed
//! the CLI at fixture-smoke's bundle.json, whose `processesPath`/`hostPath` are
//! the *absolute* `/etc/nixling/{processes,host}.json`. On a deployed nixling
//! host those files exist, so `read_bundle_json` reads the host's deployed
//! manifest instead of the fixture (the fixture's corp-vm vanishes ->
//! declaredRoles empty). This test copies the fixture artifacts into a temp
//! bundle and rewrites those two paths to relative basenames so the CLI reads
//! the COPIED fixture, regardless of host state. It also pins `nixlingd.service`
//! in the system-state fixture and sandboxes `NIXLING_DAEMON_STATE_DIR` to an
//! empty dir (pidfd-table.json absent -> per-role "stopped").

use std::path::{Path, PathBuf};
use std::process::Command;

use nixling::{StatusBridgeCheckOutputV2, StatusOutputV2, StatusVmOutputV2};

// corp-vm: all units inactive + an empty daemon-state dir (pidfd-table.json
// absent -> ch-runner / virtiofsd "stopped"). nixlingd.service is pinned
// inactive (the bash helper omitted it, so the CLI fell back to the real
// host's `systemctl is-active nixlingd.service` — non-hermetic). The bridges
// drive the `--human` "Bridge health" section.
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

/// A hermetic `status` invocation environment: a temp bundle copied from the
/// fixture-smoke output (with absolute `/etc/nixling` artifact paths rewritten
/// to relative basenames), a synthetic system-state fixture, and an empty
/// daemon-state dir. Built once, reused across multiple `run`s so the
/// flag/positional equivalence check compares against an identical bundle.
struct StatusEnv {
    _tmp: tempfile::TempDir,
    manifest: String,
    bundle: PathBuf,
    sys: PathBuf,
    daemon_state: PathBuf,
}

impl StatusEnv {
    fn new(fixtures: &str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bundle = build_hermetic_bundle(fixtures, tmp.path());
        let sys = tmp.path().join("system-state.json");
        std::fs::write(&sys, SYSTEM_STATE_JSON).expect("write system-state fixture");
        let daemon_state = tmp.path().join("daemon-state");
        std::fs::create_dir_all(&daemon_state).expect("mk daemon-state dir");
        Self {
            _tmp: tmp,
            manifest: format!("{fixtures}/manifest.json"),
            bundle,
            sys,
            daemon_state,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_nixling"))
            .args(args)
            .env("NIXLING_MANIFEST_PATH", &self.manifest)
            .env("NIXLING_BUNDLE_PATH", &self.bundle)
            .env("NIXLING_TEST_SYSTEM_STATE_JSON", &self.sys)
            .env("NIXLING_DAEMON_STATE_DIR", &self.daemon_state)
            .output()
            .unwrap_or_else(|err| panic!("spawn nixling {}: {err}", args.join(" ")))
    }
}

/// Copy the fixture-smoke bundle artifacts into `tmp/bundle` and rewrite the
/// absolute `processesPath`/`hostPath` to relative basenames so the bundle
/// context resolves the COPIED fixture, never the host's deployed
/// `/etc/nixling/*.json`. Returns the temp bundle.json path.
fn build_hermetic_bundle(fixtures: &str, tmp: &Path) -> PathBuf {
    let bundle_dir = tmp.join("bundle");
    let closures_dir = bundle_dir.join("closures");
    std::fs::create_dir_all(&closures_dir).expect("mk bundle dir");

    std::fs::copy(
        format!("{fixtures}/processes.json"),
        bundle_dir.join("processes.json"),
    )
    .expect("copy processes.json");
    std::fs::copy(
        format!("{fixtures}/host.json"),
        bundle_dir.join("host.json"),
    )
    .expect("copy host.json");
    for entry in std::fs::read_dir(format!("{fixtures}/closures")).expect("read closures dir") {
        let entry = entry.expect("closure dirent");
        std::fs::copy(entry.path(), closures_dir.join(entry.file_name())).expect("copy closure");
    }

    let raw = std::fs::read(format!("{fixtures}/bundle.json")).expect("read bundle.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&raw).expect("parse fixture bundle.json");
    bundle["processesPath"] = serde_json::Value::String("processes.json".to_owned());
    bundle["hostPath"] = serde_json::Value::String("host.json".to_owned());
    let bundle_path = bundle_dir.join("bundle.json");
    std::fs::write(
        &bundle_path,
        serde_json::to_vec(&bundle).expect("serialize rewritten bundle"),
    )
    .expect("write rewritten bundle.json");
    bundle_path
}

fn assert_success(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "`nixling {what}` exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn status_vm_json_matches_schema_and_static_state() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = StatusEnv::new(&fixtures);
    let out = env.run(&["status", "--vm", "corp-vm", "--json"]);
    assert_success(&out, "status --vm corp-vm --json");

    // The untagged StatusOutputV2 classifies this payload as the Vm variant
    // (not Inventory / CheckBridges).
    let classified: StatusOutputV2 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "status --json did not match the StatusOutputV2 schema: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert!(
        matches!(classified, StatusOutputV2::Vm(_)),
        "status --vm output must classify as the Vm variant, got {classified:?}"
    );

    // Strict schema validation: StatusVmOutputV2 is deny_unknown_fields, so a
    // successful direct deserialize is equivalent to validating against
    // docs/reference/cli-output/status.schema.json. (serde's untagged path does
    // not enforce deny_unknown_fields, so the direct deserialize is the strict
    // check.)
    let vm: StatusVmOutputV2 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "status --json did not strictly match StatusVmOutputV2: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });

    assert_eq!(vm.name, "corp-vm");
    assert_eq!(vm.env.as_deref(), Some("work"));
    assert!(
        !vm.pending_restart,
        "corp-vm: all units inactive -> not pending-restart"
    );
    // RUNTIME_UNKNOWN is the bare "unknown" sentinel.
    assert_eq!(vm.runtime, "unknown");
    assert_eq!(
        vm.services.nixling, "inactive",
        "nixlingd.service pinned inactive in the system-state fixture"
    );
    assert_eq!(
        vm.services.microvm, "stopped",
        "empty daemon-state (pidfd-table.json absent) -> ch-runner stopped"
    );
    assert_eq!(
        vm.services.virtiofsd, "stopped",
        "empty daemon-state (pidfd-table.json absent) -> virtiofsd stopped"
    );
    assert!(
        vm.declared_roles
            .iter()
            .any(|r| r == "cloud-hypervisor-runner"),
        "corp-vm declaredRoles must include cloud-hypervisor-runner, got {:?}",
        vm.declared_roles
    );
    let parity = vm
        .runner_parity
        .as_ref()
        .expect("corp-vm runner parity must be present (closure emitted in fixture-smoke)");
    assert!(
        parity.runner_parity_ok,
        "corp-vm runner parity must be OK against its committed closure"
    );
}

#[test]
fn status_vm_flag_and_positional_json_are_equivalent() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = StatusEnv::new(&fixtures);
    let flag = env.run(&["status", "--vm", "corp-vm", "--json"]);
    assert_success(&flag, "status --vm corp-vm --json");
    let positional = env.run(&["status", "corp-vm", "--json"]);
    assert_success(&positional, "status corp-vm --json");
    assert_eq!(
        flag.stdout,
        positional.stdout,
        "`status --vm <name>` and `status <name>` must stay byte-equivalent;\nflag:\n{}\npositional:\n{}",
        String::from_utf8_lossy(&flag.stdout),
        String::from_utf8_lossy(&positional.stdout),
    );
}

#[test]
fn status_check_bridges_returns_frozen_not_yet_implemented_envelope() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = StatusEnv::new(&fixtures);
    let out = env.run(&["status", "--check-bridges", "--json"]);
    assert_success(&out, "status --check-bridges --json");

    let classified: StatusOutputV2 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "status --check-bridges --json did not match StatusOutputV2: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert!(
        matches!(classified, StatusOutputV2::CheckBridges(_)),
        "check-bridges output must classify as the CheckBridges variant, got {classified:?}"
    );

    // Strict deny_unknown_fields schema check on the concrete envelope.
    let envelope: StatusBridgeCheckOutputV2 =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
            panic!(
                "status --check-bridges --json did not strictly match StatusBridgeCheckOutputV2: {err}\noutput:\n{}",
                String::from_utf8_lossy(&out.stdout)
            )
        });
    assert_eq!(envelope.mode, "check-bridges");
    assert_eq!(envelope.status, "not-yet-implemented");
}

#[test]
fn status_human_renders_runner_parity_and_bridge_sections() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = StatusEnv::new(&fixtures);
    let out = env.run(&["status", "--vm", "corp-vm", "--human"]);
    assert_success(&out, "status --vm corp-vm --human");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("runner parity: ok"),
        "status --human must render the runner-parity section:\n{stdout}"
    );
    assert!(
        stdout.contains("Bridge health"),
        "status --human must render the bridge-health section:\n{stdout}"
    );
}
