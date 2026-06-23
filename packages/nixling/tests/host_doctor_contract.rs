//! W3 CLI-contract integration test, migrated from
//! tests/cli-rust-native-host-doctor.sh.
//!
//! Spawns the real `nixling` binary and exercises `host doctor
//! --read-only --json` against on-disk daemon-state report fixtures
//! written into a sandboxed scratch directory. Every probe is
//! redirected via env knobs (`NIXLING_BROKER_SOCKET`,
//! `NIXLING_PUBLIC_SOCKET`, `NIXLING_DAEMON_STATE_DIR`,
//! `NIXLING_METRICS_URL`, `NIXLING_MANIFEST_PATH`) so the test never
//! touches real `/run` or `/var/lib` state and is fully hermetic.
//!
//! Unlike the `list` contract test, this gate needs no NL_FIXTURES
//! bundle/manifest: `host doctor --read-only` reads its probe inputs
//! from the env-pointed scratch paths, so the test always runs.
//!
//! The doctor JSON envelope is built by `doctor::render_summary` as a
//! free-form `serde_json::Value` (there is no single public strict
//! `*OutputV2` DTO for it), so the assertions below validate the
//! exact fields the bash gate checked: `command` / `mode` /
//! `broker_ready` / per-check `status` + `data` / `summary` /
//! `exitCode`, plus the process exit code captured from
//! `out.status.code()`.

use std::path::PathBuf;
use std::process::Command;

use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, listen, socket,
};
use serde_json::Value;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;

// Pinned manifest with observability disabled so the `signoz-ui-endpoint`
// probe returns early on every host (without this, the probe reads the
// host's real manifest and the baseline check-name set drifts). Copied
// verbatim from the bash gate's `manifest_path` heredoc.
const MANIFEST_JSON: &str = r#"{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318}}"#;

const PIDFD_TABLE_JSON: &str = r#"{
  "entries": [
    { "vm": "obs-net",  "role": "otel-host-bridge",   "pid": 1001, "startTimeTicks": 5 },
    { "vm": "corp-net", "role": "usbip",              "pid": 1002, "startTimeTicks": 5 },
    { "vm": "work-net", "role": "usbip",              "pid": 1003, "startTimeTicks": 5 },
    { "vm": "corp-vm",  "role": "cloud-hypervisor",   "pid": 1004, "startTimeTicks": 5 }
  ]
}"#;

const PIDFD_TABLE_PRIMARY_VMS_JSON: &str = r#"{
  "entries": [
    { "vm": "corp-vm",   "role": "ch-runner",  "pid": 2001, "startTimeTicks": 7 },
    { "vm": "media-vm",  "role": "qemu-media", "pid": 2002, "startTimeTicks": 8 },
    { "vm": "media-vm",  "role": "virtiofsd",  "pid": 2003, "startTimeTicks": 8 }
  ]
}"#;

const KERNEL_MODULE_CLEAN_JSON: &str = r#"{
  "required": ["kvm_intel"],
  "present": ["kvm_intel"],
  "missing_required": [],
  "optional_missing": []
}"#;

const KERNEL_MODULE_MISSING_JSON: &str = r#"{
  "required": ["kvm_intel"],
  "present": [],
  "missing_required": ["kvm_intel"],
  "optional_missing": []
}"#;

const AUTOSTART_FAILED_JSON: &str = r#"{
  "outcomes": [
    { "vm": "obs-net",  "env": "obs",  "is_net_vm": true,  "outcome": { "kind": "started" } },
    { "vm": "corp-vm",  "env": "corp", "is_net_vm": false, "outcome": { "kind": "failed", "reason": "broker refused" } }
  ]
}"#;

const AUTOSTART_DEGRADED_JSON: &str = r#"{
  "outcomes": [
    { "vm": "obs-net",  "env": "obs",  "is_net_vm": true,  "outcome": { "kind": "started" } },
    { "vm": "work-vm",  "env": "work", "is_net_vm": false, "outcome": { "kind": "degraded", "reason": "net-vm down" } }
  ]
}"#;

const SHUTDOWN_DEGRADED_WARN_JSON: &str = r#"{
  "schemaVersion": 1,
  "markers": [
    {
      "vm": "corp-vm",
      "outcome": "api_unavailable",
      "severity": "warn",
      "remediation": "provider shutdown API was unavailable; verify provider socket health before the next stop",
      "elapsedMs": 250
    }
  ]
}"#;

const SHUTDOWN_DEGRADED_FAIL_JSON: &str = r#"{
  "schemaVersion": 1,
  "markers": [
    {
      "vm": "corp-vm",
      "outcome": "timeout_exceeded",
      "severity": "fail",
      "remediation": "fix the in-guest shutdown path and retry nixling vm stop",
      "elapsedMs": 90000
    },
    {
      "vm": "media-vm",
      "outcome": "force_requested",
      "severity": "warn",
      "remediation": "explicit force stop requested by operator",
      "elapsedMs": 100
    }
  ]
}"#;

const STORAGE_LIFECYCLE_CLEAN_JSON: &str = r#"{
  "schemaVersion": "v2",
  "storageContractPresent": true,
  "syncContractPresent": true,
  "pathCount": 12,
  "restartPolicyCount": 4,
  "lockCount": 3,
  "issues": []
}"#;

/// Hermetic sandbox: a scratch tempdir holding the daemon-state report
/// JSONs + a pinned manifest, with closed loopback/socket paths so the
/// broker, daemon, and metrics probes surface predictable failures.
struct Sandbox {
    _tmp: tempfile::TempDir,
    state_dir: PathBuf,
    broker_socket: PathBuf,
    public_socket: PathBuf,
    manifest_path: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state_dir = tmp.path().join("daemon-state");
        std::fs::create_dir_all(&state_dir).expect("mk daemon-state dir");
        let manifest_path = tmp.path().join("vms.json");
        std::fs::write(&manifest_path, MANIFEST_JSON).expect("write manifest fixture");
        let broker_socket = tmp.path().join("broker.sock");
        let public_socket = tmp.path().join("public.sock");
        Self {
            _tmp: tmp,
            state_dir,
            broker_socket,
            public_socket,
            manifest_path,
        }
    }

    fn write_state(&self, name: &str, contents: &str) {
        std::fs::write(self.state_dir.join(name), contents)
            .unwrap_or_else(|err| panic!("write {name}: {err}"));
    }

    fn doctor_command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixling"));
        cmd.env("NIXLING_BROKER_SOCKET", &self.broker_socket)
            .env("NIXLING_PUBLIC_SOCKET", &self.public_socket)
            .env("NIXLING_DAEMON_STATE_DIR", &self.state_dir)
            // Closed loopback port -> predictable "unreachable" metrics probe.
            .env("NIXLING_METRICS_URL", "http://127.0.0.1:1/metrics")
            .env("NIXLING_MANIFEST_PATH", &self.manifest_path);
        cmd
    }

    /// Run `host doctor --read-only --json`, returning the captured
    /// process exit code and the parsed JSON envelope.
    fn run_doctor_json(&self) -> (i32, Value) {
        let out = self
            .doctor_command()
            .args(["host", "doctor", "--read-only", "--json"])
            .output()
            .expect("spawn host doctor --read-only --json");
        let code = out
            .status
            .code()
            .expect("host doctor terminated by signal, no exit code");
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
            panic!(
                "host doctor --json did not emit valid JSON: {err}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (code, value)
    }
}

/// Find the `checks[]` row with the given `name`, panicking with the
/// full envelope if it is absent.
fn check<'a>(envelope: &'a Value, name: &str) -> &'a Value {
    envelope["checks"]
        .as_array()
        .expect("checks[] is an array")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("check {name:?} missing; envelope:\n{envelope:#}"))
}

/// Bind + listen a `SOCK_SEQPACKET` `AF_UNIX` socket at `path`. The
/// doctor only `connect()`s (no handshake), so a queued connection in
/// the listen backlog is enough to report the probe as reachable. The
/// returned fd must stay alive for the duration of the probe.
fn listen_seqpacket(path: &std::path::Path) -> OwnedFd {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .expect("create seqpacket socket");
    let addr = UnixAddr::new(path).expect("seqpacket socket address");
    bind(fd.as_raw_fd(), &addr).expect("bind seqpacket listener");
    listen(&fd, Backlog::new(8).expect("listener backlog")).expect("listen seqpacket");
    fd
}

// --- 1. usage gate: missing --read-only must exit 78 ----------------

#[test]
fn host_doctor_without_read_only_exits_78_usage_envelope() {
    let sandbox = Sandbox::new();
    let out = sandbox
        .doctor_command()
        .args(["host", "doctor", "--json"])
        .output()
        .expect("spawn host doctor --json (no --read-only)");
    let code = out.status.code().expect("host doctor terminated by signal");
    assert_eq!(
        code,
        78,
        "host doctor without --read-only must exit 78; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value =
        serde_json::from_slice(&out.stdout).expect("usage refusal must emit a JSON error envelope");
    assert_eq!(
        envelope["code"], "--read-only-required",
        "usage envelope must carry the --read-only-required code; got:\n{envelope:#}"
    );
}

// --- 2. baseline (no state present) → broker fail, exit 2 -----------

#[test]
fn host_doctor_baseline_no_state_reports_broker_fail_exit_2() {
    let sandbox = Sandbox::new();
    let (code, env) = sandbox.run_doctor_json();

    assert_eq!(
        code, 2,
        "baseline doctor (no broker, no state) should exit 2 (broker fail); envelope:\n{env:#}"
    );
    assert_eq!(env["command"], "host doctor", "envelope command drift");
    assert_eq!(env["mode"], "read-only", "envelope mode drift");
    assert_eq!(
        env["broker_ready"],
        Value::Bool(false),
        "baseline must preserve top-level broker_ready=false"
    );

    let mut names: Vec<&str> = env["checks"]
        .as_array()
        .expect("checks[] array")
        .iter()
        .map(|c| c["name"].as_str().expect("check name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "autostart-status",
            "bridge-ipv6-sysctl",
            "broker-ready",
            "broker-reap-health",
            "daemon-ready",
            "graceful-shutdown-status",
            "kernel-module-matrix",
            "metrics-endpoint",
            "otel-host-bridge-runner",
            "pre-ns-posture",
            "seccomp-bpf-loaded",
            "storage-lifecycle-report",
            "usbipd-runners",
        ],
        "baseline doctor checks[] name set drift (signoz-ui-endpoint must be \
         suppressed by the observability-disabled manifest)"
    );

    let summary = &env["summary"];
    assert!(
        summary["fail"].as_u64().expect("summary.fail") >= 1,
        "baseline must have >=1 fail (broker unreachable); summary:\n{summary:#}"
    );
    assert!(
        summary["warn"].as_u64().expect("summary.warn") >= 5,
        "baseline must warn on >=5 missing-state probes; summary:\n{summary:#}"
    );
}

// --- 3. pidfd-table.json with bridge + usbipd → both runners pass ---

#[test]
fn host_doctor_pidfd_table_reports_otel_and_usbipd_runners() {
    let sandbox = Sandbox::new();
    sandbox.write_state("pidfd-table.json", PIDFD_TABLE_JSON);
    let (_code, env) = sandbox.run_doctor_json();

    let otel = check(&env, "otel-host-bridge-runner");
    assert_eq!(otel["status"], "pass", "OtelHostBridge runner not pass");
    assert_eq!(
        otel["data"]["count"].as_u64(),
        Some(1),
        "exactly one OtelHostBridge runner expected"
    );

    let usbipd = check(&env, "usbipd-runners");
    assert_eq!(usbipd["status"], "pass", "usbipd runners not pass");
    assert_eq!(
        usbipd["data"]["count"].as_u64(),
        Some(2),
        "two per-env usbipd runners expected"
    );
}

#[test]
fn host_doctor_graceful_shutdown_reports_live_primary_vmm_inventory() {
    let sandbox = Sandbox::new();
    sandbox.write_state("pidfd-table.json", PIDFD_TABLE_PRIMARY_VMS_JSON);

    let (_code, env) = sandbox.run_doctor_json();
    let graceful = check(&env, "graceful-shutdown-status");
    assert_eq!(
        graceful["status"], "pass",
        "absent degraded marker should still be pass while reporting live primary VMMs"
    );
    assert_eq!(
        graceful["data"]["livePrimaryVmmCount"], 2,
        "graceful-shutdown-status must inspect pidfd-table primary VMM state"
    );
    let live_vms = graceful["data"]["livePrimaryVms"]
        .as_array()
        .expect("livePrimaryVms array");
    assert!(
        live_vms.iter().any(|entry| entry["vm"] == "corp-vm"
            && entry["role"] == "ch-runner"
            && entry["pid"] == 2001),
        "livePrimaryVms missing ch-runner entry: {live_vms:#?}"
    );
    assert!(
        live_vms.iter().any(|entry| entry["vm"] == "media-vm"
            && entry["role"] == "qemu-media"
            && entry["pid"] == 2002),
        "livePrimaryVms missing qemu-media entry: {live_vms:#?}"
    );
    assert!(
        live_vms.iter().all(|entry| entry["role"] != "virtiofsd"),
        "livePrimaryVms must not include sidecars: {live_vms:#?}"
    );
}

#[test]
fn host_doctor_graceful_shutdown_warn_marker_reports_warn() {
    let sandbox = Sandbox::new();
    sandbox.write_state("shutdown-degraded.json", SHUTDOWN_DEGRADED_WARN_JSON);

    let (_code, env) = sandbox.run_doctor_json();
    let graceful = check(&env, "graceful-shutdown-status");
    assert_eq!(
        graceful["status"], "warn",
        "warn-only shutdown marker must report graceful-shutdown-status=warn"
    );
    assert_eq!(graceful["data"]["warn"], 1);
    assert_eq!(graceful["data"]["fail"], 0);
    assert_eq!(
        graceful["data"]["markers"][0]["outcome"], "api_unavailable",
        "marker outcome must be preserved for remediation"
    );
}

#[test]
fn host_doctor_graceful_shutdown_fail_marker_reports_fail() {
    let sandbox = Sandbox::new();
    sandbox.write_state("shutdown-degraded.json", SHUTDOWN_DEGRADED_FAIL_JSON);

    let (code, env) = sandbox.run_doctor_json();
    let graceful = check(&env, "graceful-shutdown-status");
    assert_eq!(
        graceful["status"], "fail",
        "any fail shutdown marker must report graceful-shutdown-status=fail"
    );
    assert_eq!(graceful["data"]["fail"], 1);
    assert_eq!(graceful["data"]["warn"], 1);
    assert!(
        graceful["detail"]
            .as_str()
            .expect("detail")
            .contains("corp-vm, media-vm"),
        "detail should name affected VMs: {graceful:#}"
    );
    assert_eq!(
        code, 2,
        "fail marker should contribute to host doctor exit code 2"
    );
}

#[test]
fn host_doctor_graceful_shutdown_fail_marker_wins_over_pidfd_parse_error() {
    let sandbox = Sandbox::new();
    sandbox.write_state("pidfd-table.json", "{not json");
    sandbox.write_state("shutdown-degraded.json", SHUTDOWN_DEGRADED_FAIL_JSON);

    let (_code, env) = sandbox.run_doctor_json();
    let graceful = check(&env, "graceful-shutdown-status");
    assert_eq!(
        graceful["status"], "fail",
        "pidfd-table parse errors must not hide fail shutdown markers"
    );
    assert_eq!(graceful["data"]["fail"], 1);
    assert!(
        graceful["data"]["pidfdInspectionError"]
            .as_str()
            .expect("pidfd inspection error")
            .contains("parse"),
        "pidfd inspection error should remain visible: {graceful:#}"
    );
}

// --- 4a. kernel-module-report.json clean → pass --------------------

#[test]
fn host_doctor_clean_kernel_module_report_passes() {
    let sandbox = Sandbox::new();
    sandbox.write_state("kernel-module-report.json", KERNEL_MODULE_CLEAN_JSON);
    let (_code, env) = sandbox.run_doctor_json();
    assert_eq!(
        check(&env, "kernel-module-matrix")["status"],
        "pass",
        "clean kernel-module-report must yield kernel-module-matrix=pass"
    );
}

// --- 4b. kernel-module-report.json required-missing → fail, exit 2 --

#[test]
fn host_doctor_missing_required_kernel_module_fails_exit_2() {
    let sandbox = Sandbox::new();
    sandbox.write_state("kernel-module-report.json", KERNEL_MODULE_MISSING_JSON);
    let (code, env) = sandbox.run_doctor_json();
    assert_eq!(code, 2, "missing required kernel module should exit 2");
    assert_eq!(
        check(&env, "kernel-module-matrix")["status"],
        "fail",
        "missing required module must yield kernel-module-matrix=fail"
    );
    assert_eq!(
        env["exitCode"].as_i64(),
        Some(2),
        "envelope exitCode must agree with the process exit code"
    );
}

// --- 4c. storage-lifecycle-report.json clean → pass -----------------

#[test]
fn host_doctor_clean_storage_lifecycle_report_passes() {
    let sandbox = Sandbox::new();
    sandbox.write_state(
        "storage-lifecycle-report.json",
        STORAGE_LIFECYCLE_CLEAN_JSON,
    );
    let (_code, env) = sandbox.run_doctor_json();
    let check = check(&env, "storage-lifecycle-report");
    assert_eq!(
        check["status"], "pass",
        "clean storage-lifecycle-report must yield storage-lifecycle-report=pass"
    );
    assert_eq!(
        check["data"]["issueCount"].as_u64(),
        Some(0),
        "clean storage lifecycle report should have no issues"
    );
    assert!(
        check["data"].get("remediation").is_none(),
        "clean storage lifecycle report must not carry remediation"
    );
}

// --- 5. autostart-report.json with Failed outcome → fail, exit 2 ----

#[test]
fn host_doctor_autostart_failed_outcome_fails_exit_2() {
    let sandbox = Sandbox::new();
    sandbox.write_state("autostart-report.json", AUTOSTART_FAILED_JSON);
    let (code, env) = sandbox.run_doctor_json();
    assert_eq!(code, 2, "autostart Failed outcome should exit 2");

    let autostart = check(&env, "autostart-status");
    assert_eq!(autostart["status"], "fail", "autostart-status must be fail");
    assert_eq!(
        autostart["data"]["failed"].as_u64(),
        Some(1),
        "exactly one failed autostart outcome expected"
    );
    assert_eq!(
        autostart["data"]["degradedTotal"].as_u64(),
        Some(1),
        "degradedTotal counts failed+degraded"
    );
}

// --- 6. autostart Degraded only → warn -----------------------------

#[test]
fn host_doctor_autostart_degraded_outcome_warns() {
    let sandbox = Sandbox::new();
    sandbox.write_state("autostart-report.json", AUTOSTART_DEGRADED_JSON);
    let (_code, env) = sandbox.run_doctor_json();

    let autostart = check(&env, "autostart-status");
    assert_eq!(autostart["status"], "warn", "degraded-only must be warn");
    assert_eq!(
        autostart["data"]["degraded"].as_u64(),
        Some(1),
        "exactly one degraded autostart outcome expected"
    );
}

// --- 7. metrics endpoint is optional when not serving ----------------

#[test]
fn host_doctor_metrics_endpoint_unreachable_passes_as_optional() {
    let sandbox = Sandbox::new();
    let (_code, env) = sandbox.run_doctor_json();
    let metrics = check(&env, "metrics-endpoint");
    assert_eq!(
        metrics["status"], "pass",
        "closed-port metrics probe should pass because metrics are optional"
    );
    assert!(
        metrics["detail"]
            .as_str()
            .expect("metrics detail")
            .contains("not serving metrics"),
        "metrics-endpoint detail must mention optional not-serving posture; got {:?}",
        metrics["detail"]
    );
}

// --- 8. human renderer surfaces summary line + per-check markers ----

#[test]
fn host_doctor_human_renderer_emits_summary_and_markers() {
    let sandbox = Sandbox::new();
    // Guarantee at least one [PASS] marker by seeding passing probes.
    sandbox.write_state("pidfd-table.json", PIDFD_TABLE_JSON);
    sandbox.write_state("kernel-module-report.json", KERNEL_MODULE_CLEAN_JSON);

    let out = sandbox
        .doctor_command()
        .args(["host", "doctor", "--read-only", "--human"])
        .output()
        .expect("spawn host doctor --read-only --human");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("host doctor --read-only: summary pass="),
        "human renderer missing summary line; output:\n{stdout}"
    );
    assert!(
        stdout.contains("[PASS]"),
        "human renderer missing [PASS] markers; output:\n{stdout}"
    );
}

// --- 9. live sockets → broker-ready + daemon-ready pass -------------

#[test]
fn host_doctor_live_sockets_report_broker_and_daemon_ready() {
    let sandbox = Sandbox::new();
    // Keep both listener fds alive across the probe.
    let _broker = listen_seqpacket(&sandbox.broker_socket);
    let _public = listen_seqpacket(&sandbox.public_socket);

    let (_code, env) = sandbox.run_doctor_json();
    assert_eq!(
        check(&env, "broker-ready")["status"],
        "pass",
        "reachable broker socket must report broker-ready=pass"
    );
    assert_eq!(
        check(&env, "daemon-ready")["status"],
        "pass",
        "reachable daemon socket must report daemon-ready=pass"
    );
    assert_eq!(
        env["broker_ready"],
        Value::Bool(true),
        "top-level broker_ready must be true when the broker socket is reachable"
    );
}

#[test]
fn host_doctor_private_broker_socket_denial_is_pass() {
    let sandbox = Sandbox::new();
    let _broker = listen_seqpacket(&sandbox.broker_socket);
    let mut perms = std::fs::metadata(&sandbox.broker_socket)
        .expect("stat broker socket")
        .permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&sandbox.broker_socket, perms).expect("chmod broker socket");

    let (_code, env) = sandbox.run_doctor_json();

    let broker = check(&env, "broker-ready");
    assert_eq!(
        broker["status"], "pass",
        "permission-denied private broker socket is the expected unprivileged posture"
    );
    assert!(
        broker["detail"]
            .as_str()
            .expect("broker detail")
            .contains("correctly denies direct unprivileged access")
    );
    assert_eq!(
        env["broker_ready"],
        Value::Bool(true),
        "top-level broker_ready remains true when private socket is present"
    );
}

#[test]
fn host_doctor_inaccessible_broker_parent_is_fail() {
    let mut sandbox = Sandbox::new();
    let broker_dir = sandbox._tmp.path().join("private-broker-dir");
    std::fs::create_dir(&broker_dir).expect("create private broker dir");
    sandbox.broker_socket = broker_dir.join("broker.sock");
    let _broker = listen_seqpacket(&sandbox.broker_socket);
    let mut perms = std::fs::metadata(&broker_dir)
        .expect("stat broker dir")
        .permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&broker_dir, perms).expect("chmod broker dir");

    let (_code, env) = sandbox.run_doctor_json();

    let mut restore = std::fs::metadata(&broker_dir)
        .expect("stat broker dir for restore")
        .permissions();
    restore.set_mode(0o700);
    let _ = std::fs::set_permissions(&broker_dir, restore);

    assert_eq!(
        check(&env, "broker-ready")["status"],
        "fail",
        "permission denied from an inaccessible parent is not a healthy private broker socket"
    );
}
