//! W3 CLI-contract integration test, migrated from tests/cli-json.sh.
//!
//! This module covers the remaining behaviours unique to the cli-json gate:
//!   * `keys list --json` with no daemon: exit 1, empty stderr, and the
//!     structured daemon-down envelope on stdout with
//!     `kind == "d2b keys list requires d2bd"`;
//!   * `audit --json` run under a PTY (a real TTY): stays JSON (not the human
//!     stderr form) and returns the daemon-down envelope
//!     `kind == "d2b audit requires d2bd"`, exit 1.

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

#[test]
fn keys_list_daemon_down_returns_structured_envelope() {
    // No fixture needed: keys list connects straight to the public socket and
    // never loads the manifest. Pointing it at a missing socket surfaces the
    // daemon-down envelope.
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("missing-public.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["keys", "list", "--json"])
        .env("D2B_PUBLIC_SOCKET", &missing)
        .env("D2B_BROKER_SOCKET", tmp.path().join("missing-priv.sock"))
        .output()
        .expect("spawn d2b keys list --json");

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
