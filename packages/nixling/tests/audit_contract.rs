//! CLI-contract integration test, migrated from tests/cli-rust-native-audit.sh.
//!
//! Covers the `nixling audit` machine + human contract:
//!   * daemon-down typed envelope when nixlingd is unreachable, with NO bash
//!     fallback even when the (removed) `NIXLING_LEGACY_*` escape hatches are
//!     set (a poison-pill that would `exit 99` if ever exec'd);
//!   * `audit --strict` returns the frozen not-yet-implemented envelope (78);
//!   * a daemon `auditResponse` frame is relayed verbatim to stdout (driven by
//!     an in-process SOCK_SEQPACKET mock daemon — replaces the bash gate's
//!     python mock);
//!   * a real, KVM-free `nixlingd serve --once` rejects a launcher-role peer
//!     with `authz-audit-requires-admin` (32) and NO bash fallback.
//!
//! The last case needs the daemon-spawn harness (NIXLING_TEST_NIXLINGD_BIN);
//! it skips cleanly when unavailable (plain `cargo test --workspace`).

mod common;

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use common::{TestPeer, spawn_nixlingd_once};

/// Write a non-executable / `exit 99` poison-pill the CLI must never exec.
fn write_poison_pill(dir: &Path) -> std::path::PathBuf {
    let p = dir.join("legacy-poison.sh");
    let mut f = std::fs::File::create(&p).expect("create poison");
    f.write_all(
        b"#!/usr/bin/env bash\necho 'FAIL: rust CLI exec'\\''d legacy bash' >&2\nexit 99\n",
    )
    .expect("write poison");
    let mut perms = std::fs::metadata(&p).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&p, perms).unwrap();
    p
}

#[test]
fn audit_reports_daemon_down_without_bash_fallback() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let poison = write_poison_pill(tmp.path());
    let missing = tmp.path().join("missing.sock");

    let run = |fmt: &str| {
        Command::new(env!("CARGO_BIN_EXE_nixling"))
            .args(["audit", fmt])
            .env("NIXLING_LEGACY_CLI", &poison)
            .env("NIXLING_LEGACY_CLI_PATH", &poison)
            .env("NIXLING_LEGACY_BASH_OPT_IN", "1")
            .env("NIXLING_PUBLIC_SOCKET", &missing)
            .env("NIXLING_AUDIT_TESTMODE_KVM_MODE", "660")
            .output()
            .expect("spawn nixling audit")
    };

    let human = run("--human");
    let json = run("--json");

    // No bash fallback: the poison-pill exit code (99) must never surface.
    assert_ne!(
        human.status.code(),
        Some(99),
        "audit exec'd the bash poison-pill"
    );
    assert_ne!(
        json.status.code(),
        Some(99),
        "audit exec'd the bash poison-pill"
    );

    assert_eq!(
        human.status.code(),
        Some(1),
        "audit --human daemon-down exits 1"
    );
    assert!(
        String::from_utf8_lossy(&human.stderr).contains("daemon-down"),
        "audit --human stderr should name daemon-down; got:\n{}",
        String::from_utf8_lossy(&human.stderr)
    );

    assert_eq!(
        json.status.code(),
        Some(1),
        "audit --json daemon-down exits 1"
    );
    let envelope: Value = serde_json::from_slice(&json.stdout).unwrap_or_else(|e| {
        panic!(
            "audit --json envelope: {e}\n{}",
            String::from_utf8_lossy(&json.stdout)
        )
    });
    assert_eq!(envelope["code"], "daemon-down");
    assert_eq!(envelope["exitCode"], 1);
}

#[test]
fn audit_strict_returns_not_yet_implemented_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("strict.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(["audit", "--strict", "--json"])
        .env("NIXLING_PUBLIC_SOCKET", &missing)
        .output()
        .expect("spawn nixling audit --strict");

    assert_eq!(out.status.code(), Some(78), "audit --strict exits 78");
    assert!(
        out.stderr.is_empty(),
        "audit --strict stderr should be empty"
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "strict envelope: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(envelope["code"], "not-yet-implemented");
    assert_eq!(envelope["exitCode"], 78);
}

#[test]
fn audit_relays_daemon_auditresponse_frames() {
    // In-process SOCK_SEQPACKET mock daemon: hello -> helloOk -> audit ->
    // auditResponse{lines}. The CLI relays the lines to stdout verbatim.
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock = tmp.path().join("mock.sock");
    let handle = spawn_audit_mock_daemon(&sock);

    let out = Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(["audit", "--human"])
        .env("NIXLING_PUBLIC_SOCKET", &sock)
        .output()
        .expect("spawn nixling audit --human (mock daemon)");

    handle.join().expect("mock daemon thread");

    assert!(
        out.status.success(),
        "audit against mock daemon should succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "audit mock-daemon stderr should be empty"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "broker audit line 1\nbroker audit line 2\n",
        "audit should relay the daemon auditResponse lines verbatim"
    );
}

#[test]
fn audit_admin_rejected_against_live_daemon_without_fallback() {
    let Some(daemon) = spawn_nixlingd_once(&TestPeer::launcher()) else {
        eprintln!("SKIP: NIXLING_TEST_NIXLINGD_BIN unset (daemon-spawn harness unavailable)");
        return;
    };

    let out = Command::new(env!("CARGO_BIN_EXE_nixling"))
        .args(["audit", "--json"])
        .env("NIXLING_PUBLIC_SOCKET", &daemon.socket_path)
        .output()
        .expect("spawn nixling audit --json (live daemon)");

    // --once daemon exits after serving this one request.
    let _ = daemon.wait();

    assert_eq!(
        out.status.code(),
        Some(32),
        "launcher peer is denied audit (admin-only) with exit 32; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("authz-audit-requires-admin"),
        "audit authz rejection should name authz-audit-requires-admin; got:\n{stderr}"
    );
    assert!(
        !stderr.contains("daemon-down"),
        "a reachable daemon must not surface daemon-down"
    );
    assert!(
        out.stdout.is_empty(),
        "rejected audit must not print a body"
    );
}

// --- in-process SOCK_SEQPACKET mock daemon ---------------------------------

/// Spawn a one-shot mock daemon that performs the audit handshake and returns
/// an `auditResponse` with two lines. Returns the joinable server thread.
fn spawn_audit_mock_daemon(path: &Path) -> std::thread::JoinHandle<()> {
    use nix::sys::socket::{
        AddressFamily, Backlog, SockFlag, SockType, UnixAddr, accept, bind, listen, socket,
    };

    let _ = std::fs::remove_file(path);
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::empty(),
        None,
    )
    .expect("seqpacket socket");
    let addr = UnixAddr::new(path.as_os_str().as_bytes()).expect("unix addr");
    bind(listener.as_raw_fd(), &addr).expect("bind mock sock");
    listen(&listener, Backlog::new(1).unwrap()).expect("listen mock sock");

    std::thread::spawn(move || {
        let conn = accept(listener.as_raw_fd()).expect("accept");
        // hello -> helloOk
        let hello = recv_frame(conn);
        assert_eq!(hello["type"], "hello", "expected hello frame, got {hello}");
        send_frame(
            conn,
            &serde_json::json!({
                "type": "helloOk",
                "serverVersion": "0.4.0",
                "selectedVersion": "0.4.0",
                "capabilities": ["typed-errors", "export-broker-audit"],
            }),
        );
        // audit -> auditResponse
        let req = recv_frame(conn);
        assert_eq!(req["type"], "audit", "expected audit frame, got {req}");
        send_frame(
            conn,
            &serde_json::json!({
                "type": "auditResponse",
                "lines": ["broker audit line 1", "broker audit line 2"],
            }),
        );
        let _ = nix::unistd::close(conn);
    })
}

fn recv_frame(fd: std::os::fd::RawFd) -> Value {
    let mut buf = vec![0u8; 1 << 20];
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
