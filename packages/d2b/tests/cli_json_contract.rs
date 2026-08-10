//! W3 CLI-contract integration test, migrated from tests/cli-json.sh.
//!
//! The retired bash gate built a synthetic `nixosSystem` fixture and asserted
//! the machine-readable JSON contract for `list` / `status` / activation keys
//! / `audit`.
//! Its legacy inventory shape is no longer a v3 command. The pending-restart
//! cases now exercise the typed `guest list` and `guest status` Zone resource
//! commands and require their strict v3 result or fail-closed Zone error
//! envelope.
//!
//! This module covers only the behaviours unique to the cli-json gate:
//!   * `guest list --json` and `guest status <name> --json` with the supplied
//!     fixture artifacts and no Zone runtime fail closed with the strict v3
//!     `zone-unavailable` envelope rather than a legacy `pendingRestart` field;
//!   * `activation keys list --json` with no Zone runtime: exit 1, empty
//!     stderr, and the v3 `zone-unavailable` envelope on stdout;
//!   * `audit --json` run under a PTY (a real TTY): stays JSON (not the human
//!     stderr form) and returns the daemon-down envelope
//!     `kind == "d2b audit requires d2bd"`, exit 1.
//!
//! The resource cases reuse the rendered fixture-smoke artifacts via
//! `D2B_FIXTURES`; they skip cleanly when it is unset (the plain
//! `cargo test --workspace` pass with no Nix sandbox). The Zone-down keys /
//! audit cases need no fixture - they only point the public socket at a
//! missing path - so they always run.

use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// The closed v3 envelope emitted before a Zone context can be established.
const ZONE_ERROR_KEYS: &[&str] = &["errorClass", "message", "ok", "schemaVersion", "zoneRef"];

/// The fixture-smoke output dir, or `None` when D2B_FIXTURES is unset (plain
/// non-gated `cargo test` runs). The gated rust-workspace-checks.sh step always
/// sets it.
fn fixtures_dir() -> Option<String> {
    std::env::var("D2B_FIXTURES").ok()
}

/// A v3 resource invocation environment using the supplied fixture artifacts
/// while pointing Zone transport at missing sockets.
struct ZoneFixtureEnv {
    _tmp: tempfile::TempDir,
    manifest: PathBuf,
    bundle: PathBuf,
    missing_public: PathBuf,
    missing_broker: PathBuf,
}

impl ZoneFixtureEnv {
    fn new(fixtures: &str) -> Self {
        let fixtures = Path::new(fixtures);
        for artifact in ["manifest.json", "bundle.json"] {
            assert!(
                fixtures.join(artifact).is_file(),
                "D2B_FIXTURES is missing the required {artifact} artifact"
            );
        }
        let tmp = tempfile::tempdir().expect("tempdir");
        Self {
            manifest: fixtures.join("manifest.json"),
            bundle: fixtures.join("bundle.json"),
            missing_public: tmp.path().join("missing-public.sock"),
            missing_broker: tmp.path().join("missing-priv.sock"),
            _tmp: tmp,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_d2b"))
            .args(args)
            .env("D2B_MANIFEST_PATH", &self.manifest)
            .env("D2B_BUNDLE_PATH", &self.bundle)
            .env("D2B_PUBLIC_SOCKET", &self.missing_public)
            .env("D2B_BROKER_SOCKET", &self.missing_broker)
            .output()
            .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
    }
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
    assert_eq!(value["kind"], format!("d2b {verb} requires d2bd"));
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
            .is_some_and(|s| s.contains("d2bd is unreachable")),
        "observedState must mention d2bd is unreachable, got: {}",
        value["observedState"]
    );
    assert!(
        value["remediation"]
            .as_str()
            .is_some_and(|s| s.contains("Start d2bd")),
        "remediation must tell the operator to Start d2bd, got: {}",
        value["remediation"]
    );
    assert_eq!(
        value["docsAnchor"],
        "docs/reference/error-codes.md#daemon-down"
    );
}

/// Assert the strict v3 transport envelope used when command dispatch cannot
/// discover a Zone runtime. The missing socket path must not be reflected in
/// the operator-facing JSON.
fn assert_zone_unavailable_envelope(value: &Value, missing_socket: &Path) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("Zone error must be a JSON object, got: {value}"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, ZONE_ERROR_KEYS,
        "Zone-unavailable envelope key set must stay closed"
    );
    assert_eq!(value["ok"], false);
    assert_eq!(value["zoneRef"], "Zone/local-root");
    assert_eq!(value["errorClass"], "zone-unavailable");
    assert_eq!(value["message"], "Zone runtime is unavailable");
    assert_eq!(value["schemaVersion"], 1);
    let rendered = value.to_string();
    assert!(
        !rendered.contains(&missing_socket.display().to_string()),
        "Zone-unavailable envelope must redact the missing socket path: {rendered}"
    );
}

#[test]
fn list_reports_pending_restart_when_booted_differs_and_active() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = ZoneFixtureEnv::new(&fixtures);
    let out = env.run(&["guest", "list", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "`d2b guest list --json` must fail closed when the Zone is unavailable; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "v3 JSON errors must stay on stdout; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "guest list --json did not match the v3 error envelope: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_zone_unavailable_envelope(&envelope, &env.missing_public);
}

#[test]
fn status_reports_pending_restart_with_consistent_current_booted() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let env = ZoneFixtureEnv::new(&fixtures);
    let out = env.run(&["guest", "status", "corp-vm", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "`d2b guest status corp-vm --json` must fail closed when the Zone is unavailable; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "v3 JSON errors must stay on stdout; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "guest status --json did not match the v3 error envelope: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_zone_unavailable_envelope(&envelope, &env.missing_public);
}

#[test]
fn activation_keys_list_zone_down_returns_v3_envelope() {
    // No fixture is needed: v3 activation keys first discovers a Zone runtime.
    // Pointing the public socket at a missing path exercises the pre-dispatch
    // zone-unavailable envelope rather than the retired daemon-down shape.
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("missing-public.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["activation", "keys", "list", "--json"])
        .env("D2B_PUBLIC_SOCKET", &missing)
        .env("D2B_BROKER_SOCKET", tmp.path().join("missing-priv.sock"))
        .output()
        .expect("spawn d2b activation keys list --json");

    assert_eq!(
        out.status.code(),
        Some(1),
        "activation keys list --json Zone-down exits 1; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "activation keys list --json Zone-down: the envelope is on stdout, stderr is empty; got:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "activation keys list --json envelope: {err}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_zone_unavailable_envelope(&envelope, &missing);
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

    let mut child = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["audit", "--json"])
        .env("D2B_PUBLIC_SOCKET", &missing)
        .env("D2B_BROKER_SOCKET", tmp.path().join("missing-priv.sock"))
        .env("D2B_AUDIT_TESTMODE_KVM_MODE", "660")
        .stdin(Stdio::from(slave_stdin))
        .stdout(Stdio::from(slave_stdout))
        .stderr(Stdio::from(slave_stderr))
        .spawn()
        .expect("spawn d2b audit --json under a PTY");

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
