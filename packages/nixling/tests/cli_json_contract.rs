//! W3 CLI-contract integration test, migrated from tests/cli-json.sh.
//!
//! The retired bash gate built a synthetic `nixosSystem` fixture and asserted
//! the machine-readable JSON contract for `list` / `status` / `keys` / `audit`.
//! Its KEY-SET shape checks (the exact `keys | sort` jq assertions for `list`,
//! `status`, `services`, `runnerParity`, `livePoolIntegrity`) are now covered by
//! the strict `deny_unknown_fields` DTO deserializes in `cli_contract.rs` and
//! `status_contract.rs` — a successful typed deserialize into
//! `nixling_ipc::cli_output::{ListOutputV2, StatusVmOutputV2}` IS the exact-key-set check.
//!
//! This module covers only the behaviours unique to the cli-json gate:
//!   * `pending-restart`: when a VM's `booted != current` AND it counts as
//!     running, `list --json` reports `status == "pending-restart"` and
//!     `status <vm> --json` reports `pendingRestart == true` with running
//!     services and consistent `current`/`booted` (deserialized strictly into
//!     `nixling_ipc::cli_output::{ListOutputV2, StatusVmOutputV2}`);
//!   * `keys list --json` with no daemon: exit 1, empty stderr, and the
//!     structured daemon-down envelope on stdout with
//!     `kind == "nixling keys list requires nixlingd"`;
//!   * `audit --json` run under a PTY (a real TTY): stays JSON (not the human
//!     stderr form) and returns the daemon-down envelope
//!     `kind == "nixling audit requires nixlingd"`, exit 1.
//!
//! The pending-restart cases reuse the rendered fixture-smoke bundle via
//! `NL_FIXTURES` (the same artifact dir cli_contract.rs / status_contract.rs
//! consume); they skip cleanly when it is unset (the plain
//! `cargo test --workspace` pass with no Nix sandbox). The daemon-down keys /
//! audit cases need no fixture — they only point the public socket at a missing
//! path — so they always run.

use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nixling_ipc::cli_output::{ListOutputV2, StatusVmOutputV2};
use serde_json::Value;

/// The exact key set of the structured host-error (`daemon-down`) envelope,
/// matching the bash gate's `(keys | sort)` assertion.
const ENVELOPE_KEYS: &[&str] = &[
    "code",
    "docsAnchor",
    "exitCode",
    "kind",
    "observedState",
    "remediation",
    "whatWasChecked",
];

/// corp-vm's `current`/`booted` symlink targets are deliberately DIFFERENT
/// store-path strings so `is_pending_restart` sees `current != booted`.
const CURRENT_TARGET: &str = "/nix/store/nixling-current";
const BOOTED_TARGET: &str = "/nix/store/nixling-booted";

/// System-state fixture pinning `nixlingd.service` active. With the daemon
/// active, `vm_counts_as_running` is true, so a `current != booted` mismatch
/// resolves to `pending-restart`. (Mirrors the bash gate's
/// system-state-active.json.)
const SYSTEM_STATE_ACTIVE_JSON: &str = r#"{"units":{"nixlingd.service":"active"},"bridges":{}}"#;

/// pidfd-table marking corp-vm's ch-runner running, so `status`'s
/// `services.microvm` resolves to `running` (mirrors the bash gate).
const PIDFD_TABLE_JSON: &str = r#"{"entries":[{"vm":"corp-vm","role":"ch-runner","pid":12345}]}"#;

/// The fixture-smoke output dir, or `None` when NL_FIXTURES is unset (plain
/// non-gated `cargo test` runs). The gated rust-workspace-checks.sh step always
/// sets it.
fn fixtures_dir() -> Option<String> {
    std::env::var("NL_FIXTURES").ok()
}

/// Copy the fixture-smoke bundle artifacts into `tmp/bundle` and rewrite the
/// absolute `processesPath`/`hostPath` to relative basenames so the bundle
/// context resolves the COPIED fixture, never the host's deployed
/// `/etc/nixling/*.json` (this host IS a deployed nixling host). Returns the
/// temp bundle.json path. (Same hermeticity fix as status_contract.rs.)
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

/// A hermetic `list`/`status` invocation environment that forces corp-vm into
/// the `pending-restart` state: a temp bundle copied from the fixture-smoke
/// output, a per-VM state-root whose `current`/`booted` symlinks point at
/// DIFFERENT store paths, a daemon-state dir whose pidfd-table marks the
/// ch-runner running, and a system-state fixture pinning `nixlingd.service`
/// active. Built once, reused across the list/status assertions.
struct PendingRestartEnv {
    _tmp: tempfile::TempDir,
    manifest: String,
    bundle: PathBuf,
    state_root: PathBuf,
    daemon_state: PathBuf,
    sys: PathBuf,
    missing_public: PathBuf,
    missing_broker: PathBuf,
}

impl PendingRestartEnv {
    fn new(fixtures: &str) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bundle = build_hermetic_bundle(fixtures, tmp.path());

        let state_root = tmp.path().join("state");
        let vm_state = state_root.join("corp-vm");
        std::fs::create_dir_all(&vm_state).expect("mk corp-vm state dir");
        std::os::unix::fs::symlink(CURRENT_TARGET, vm_state.join("current"))
            .expect("symlink current");
        std::os::unix::fs::symlink(BOOTED_TARGET, vm_state.join("booted")).expect("symlink booted");

        let daemon_state = tmp.path().join("daemon-state");
        std::fs::create_dir_all(&daemon_state).expect("mk daemon-state dir");
        std::fs::write(daemon_state.join("pidfd-table.json"), PIDFD_TABLE_JSON)
            .expect("write pidfd-table");

        let sys = tmp.path().join("system-state-active.json");
        std::fs::write(&sys, SYSTEM_STATE_ACTIVE_JSON).expect("write system-state fixture");

        Self {
            manifest: format!("{fixtures}/manifest.json"),
            bundle,
            state_root,
            daemon_state,
            sys,
            missing_public: tmp.path().join("missing-public.sock"),
            missing_broker: tmp.path().join("missing-priv.sock"),
            _tmp: tmp,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_nixling"))
            .args(args)
            .env("NIXLING_MANIFEST_PATH", &self.manifest)
            .env("NIXLING_BUNDLE_PATH", &self.bundle)
            .env("NIXLING_STATE_ROOT", &self.state_root)
            .env("NIXLING_DAEMON_STATE_DIR", &self.daemon_state)
            .env("NIXLING_TEST_SYSTEM_STATE_JSON", &self.sys)
            .env("NIXLING_PUBLIC_SOCKET", &self.missing_public)
            .env("NIXLING_BROKER_SOCKET", &self.missing_broker)
            .output()
            .unwrap_or_else(|err| panic!("spawn nixling {}: {err}", args.join(" ")))
    }
}

fn assert_success(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "`nixling {what}` exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Assert `value` is the structured daemon-down envelope for `verb`: the exact
/// key set, `code == "daemon-down"`, `exitCode == 1`, the documented
/// what/observed/remediation substrings, and the error-codes docs anchor.
fn assert_daemon_down_envelope(value: &Value, verb: &str) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("envelope must be a JSON object, got: {value}"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, ENVELOPE_KEYS,
        "envelope key set must match the documented host-error shape"
    );
    assert_eq!(value["kind"], format!("nixling {verb} requires nixlingd"));
    assert_eq!(value["code"], "daemon-down");
    assert_eq!(value["exitCode"], 1);
    assert!(
        value["whatWasChecked"]
            .as_str()
            .is_some_and(|s| s.contains("Daemon connectivity")),
        "whatWasChecked must mention Daemon connectivity, got: {}",
        value["whatWasChecked"]
    );
    assert!(
        value["observedState"]
            .as_str()
            .is_some_and(|s| s.contains("nixlingd is unreachable")),
        "observedState must mention nixlingd is unreachable, got: {}",
        value["observedState"]
    );
    assert!(
        value["remediation"]
            .as_str()
            .is_some_and(|s| s.contains("Start nixlingd")),
        "remediation must tell the operator to Start nixlingd, got: {}",
        value["remediation"]
    );
    assert_eq!(
        value["docsAnchor"],
        "docs/reference/error-codes.md#daemon-down"
    );
}

#[test]
fn list_reports_pending_restart_when_booted_differs_and_active() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = PendingRestartEnv::new(&fixtures);
    let out = env.run(&["list", "--json"]);
    assert_success(&out, "list --json");

    // Strict schema validation: ListItemOutputV2 is deny_unknown_fields, so a
    // successful typed deserialize is the exact-key-set check the bash gate did
    // via jq.
    let list: ListOutputV2 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "list --json did not match the ListOutputV2 schema: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    let corp = list
        .0
        .iter()
        .find(|i| i.name == "corp-vm")
        .expect("corp-vm in inventory");
    assert_eq!(
        corp.status, "pending-restart",
        "corp-vm: current != booted + daemon active -> pending-restart"
    );
}

#[test]
fn status_reports_pending_restart_with_consistent_current_booted() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: NL_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = PendingRestartEnv::new(&fixtures);
    let out = env.run(&["status", "corp-vm", "--json"]);
    assert_success(&out, "status corp-vm --json");

    // Strict schema validation: StatusVmOutputV2 is deny_unknown_fields, so a
    // successful direct deserialize is the exact-key-set check (services,
    // runnerParity, livePoolIntegrity sub-shapes included) the bash gate did.
    let vm: StatusVmOutputV2 = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "status --json did not strictly match StatusVmOutputV2: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });

    assert_eq!(vm.name, "corp-vm");
    assert_eq!(vm.env.as_deref(), Some("work"));
    assert!(
        vm.pending_restart,
        "corp-vm: current != booted + daemon active -> pendingRestart true"
    );
    assert_eq!(vm.current.as_deref(), Some(CURRENT_TARGET));
    assert_eq!(vm.booted.as_deref(), Some(BOOTED_TARGET));
    assert_eq!(
        vm.services.nixling, "active",
        "nixlingd.service pinned active in the system-state fixture"
    );
    assert_eq!(
        vm.services.microvm, "running",
        "pidfd-table marks corp-vm ch-runner running"
    );
    assert_eq!(vm.runtime, "unknown");
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
fn keys_list_daemon_down_returns_structured_envelope() {
    // No fixture needed: keys list connects straight to the public socket and
    // never loads the manifest. Pointing it at a missing socket surfaces the
    // daemon-down envelope.
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("missing-public.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(["keys", "list", "--json"])
        .env("NIXLING_PUBLIC_SOCKET", &missing)
        .env(
            "NIXLING_BROKER_SOCKET",
            tmp.path().join("missing-priv.sock"),
        )
        .output()
        .expect("spawn nixling keys list --json");

    assert_eq!(
        out.status.code(),
        Some(1),
        "keys list --json daemon-down exits 1; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "keys list --json daemon-down: the envelope is on stdout, stderr is empty; got:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "keys list --json envelope: {err}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_daemon_down_envelope(&envelope, "keys list");
}

#[test]
fn audit_json_stays_json_on_tty_with_daemon_down_envelope() {
    // The bash gate used `script -q -e -c "$CLI audit --json" /dev/null` to give
    // the CLI a real PTY, proving `audit --json` stays the JSON envelope even on
    // a TTY (it does not fall back to the human-on-stderr form). Reproduce the
    // PTY with rustix's pty API (the `pty` feature is enabled on the workspace
    // rustix; no new dependency/feature added).
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("missing-public.sock");

    let (master, slave_path) = open_pty();
    let slave_stdin = open_pts_slave(&slave_path);
    let slave_stdout = open_pts_slave(&slave_path);
    let slave_stderr = open_pts_slave(&slave_path);

    let mut child = Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(["audit", "--json"])
        .env("NIXLING_PUBLIC_SOCKET", &missing)
        .env(
            "NIXLING_BROKER_SOCKET",
            tmp.path().join("missing-priv.sock"),
        )
        .env("NIXLING_AUDIT_TESTMODE_KVM_MODE", "660")
        .stdin(Stdio::from(slave_stdin))
        .stdout(Stdio::from(slave_stdout))
        .stderr(Stdio::from(slave_stderr))
        .spawn()
        .expect("spawn nixling audit --json under a PTY");

    // The slave fds were moved into the child; the parent must hold none of
    // them or the master read below would never see EOF/EIO.
    let raw = drain_pty_master(master);
    let status = child.wait().expect("wait audit child");

    assert_eq!(
        status.code(),
        Some(1),
        "audit --json daemon-down on a TTY exits 1; raw PTY output:\n{}",
        String::from_utf8_lossy(&raw)
    );

    // Strip the CRLF the PTY line discipline inserts (the bash gate did
    // `tr -d '\r'`).
    let cleaned: Vec<u8> = raw.into_iter().filter(|&b| b != b'\r').collect();
    let envelope: Value = serde_json::from_slice(&cleaned).unwrap_or_else(|err| {
        panic!(
            "audit --json on a TTY must stay JSON, not the human form: {err}\noutput:\n{}",
            String::from_utf8_lossy(&cleaned)
        )
    });
    assert_daemon_down_envelope(&envelope, "audit");
}

/// Allocate a pseudo-terminal: open the master (`/dev/ptmx`), grant + unlock
/// the slave, and return `(master, slave_path)`.
fn open_pty() -> (OwnedFd, PathBuf) {
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt master");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let name = ptsname(&master, Vec::new()).expect("ptsname");
    let slave_path = PathBuf::from(std::ffi::OsStr::from_bytes(name.as_bytes()));
    (master, slave_path)
}

/// Open the PTY slave (`/dev/pts/N`) read-write without acquiring it as the
/// controlling terminal.
fn open_pts_slave(path: &Path) -> OwnedFd {
    use rustix::fs::{Mode, OFlags, open};
    open(path, OFlags::RDWR | OFlags::NOCTTY, Mode::empty()).expect("open pts slave")
}

/// Read the PTY master to end-of-stream. When the child exits and closes its
/// slave fds, a Linux PTY master read returns `EIO` rather than a clean EOF;
/// treat that as the terminator. The audit envelope (~600 bytes) fits inside
/// the PTY buffer, so the child never blocks waiting for us to read.
fn drain_pty_master(master: OwnedFd) -> Vec<u8> {
    let mut file = std::fs::File::from(master);
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            // EIO (errno 5) is the PTY-master EOF after the slave side closes.
            Err(err) if err.raw_os_error() == Some(5) => break,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => panic!("read PTY master: {err}"),
        }
    }
    out
}
