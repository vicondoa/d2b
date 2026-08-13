//! CLI-contract integration test, migrated from tests/cli-rust-native-audit.sh.
//!
//! Covers the `d2b audit` machine + human contract:
//!   * daemon-down typed envelope when d2bd is unreachable, with NO bash
//!     fallback even when the (removed) `D2B_LEGACY_*` escape hatches are
//!     set (a poison-pill that would `exit 99` if ever exec'd);
//!   * `audit --strict` returns the frozen not-yet-implemented envelope (78);
//!   * a daemon `auditResponse` frame is relayed verbatim to stdout (driven by
//!     an in-process SOCK_SEQPACKET mock daemon - replaces the bash gate's
//!     python mock);
//!   * a real, KVM-free `d2bd serve --once` rejects a launcher-role peer
//!     with `authz-audit-requires-admin` (32) and NO bash fallback.
//!
//! The last case needs the daemon-spawn harness (D2B_TEST_D2BD_BIN);
//! it skips cleanly when unavailable (plain `cargo test --workspace`).

mod common;

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use common::{TestPeer, spawn_d2bd_once};

const V3_ERROR_KEYS: &[&str] = &["errorClass", "message", "ok", "schemaVersion", "zoneRef"];

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
        Command::new(env!("CARGO_BIN_EXE_d2b"))
            .args(["audit", fmt])
            .env("D2B_LEGACY_CLI", &poison)
            .env("D2B_LEGACY_CLI_PATH", &poison)
            .env("D2B_LEGACY_BASH_OPT_IN", "1")
            .env("D2B_PUBLIC_SOCKET", &missing)
            .env("D2B_AUDIT_TESTMODE_KVM_MODE", "660")
            .output()
            .expect("spawn d2b audit")
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
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["audit", "--strict", "--json"])
        .env("D2B_PUBLIC_SOCKET", &missing)
        .output()
        .expect("spawn d2b audit --strict");

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
    // auditResponse{entries}. The CLI relays the records to stdout verbatim.
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock = tmp.path().join("mock.sock");
    let handle = spawn_audit_mock_daemon(&sock);

    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["audit", "--human"])
        .env("D2B_PUBLIC_SOCKET", &sock)
        .output()
        .expect("spawn d2b audit --human (mock daemon)");

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
fn audit_rejects_legacy_lines_response() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock = tmp.path().join("mock-legacy-lines.sock");
    let handle = spawn_single_audit_response_mock(
        &sock,
        serde_json::json!({
            "type": "auditResponse",
            "lines": ["legacy audit line"],
            "complete": true,
        }),
    );

    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["audit", "--human"])
        .env("D2B_PUBLIC_SOCKET", &sock)
        .output()
        .expect("spawn d2b audit --human (legacy mock daemon)");

    handle.join().expect("legacy mock daemon thread");

    assert_eq!(
        out.status.code(),
        Some(1),
        "legacy audit lines must be rejected: stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("failed to decode auditResponse"),
        "legacy rejection should identify the audit response decode failure: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn audit_relays_multiple_paginated_auditresponse_frames() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock = tmp.path().join("mock-paginated.sock");
    let handle = spawn_paginated_audit_mock_daemon(&sock);

    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["audit", "--human"])
        .env("D2B_PUBLIC_SOCKET", &sock)
        .output()
        .expect("spawn d2b audit --human (paginated mock daemon)");

    handle.join().expect("paginated mock daemon thread");

    assert!(
        out.status.success(),
        "paginated audit should succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "broker audit page 1\nbroker audit page 2\n",
        "audit should concatenate all complete pages"
    );
}

#[test]
fn audit_admin_rejected_against_live_daemon_without_fallback() {
    let Some(daemon) = spawn_d2bd_once(&TestPeer::launcher()) else {
        eprintln!("SKIP: D2B_TEST_D2BD_BIN unset (daemon-spawn harness unavailable)");
        return;
    };

    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["audit", "--json"])
        .env("D2B_PUBLIC_SOCKET", &daemon.socket_path)
        .output()
        .expect("spawn d2b audit --json (live daemon)");

    // --once daemon exits after serving this one request.
    let _ = daemon.wait();

    assert_eq!(
        out.status.code(),
        Some(32),
        "launcher peer is denied audit (admin-only) with exit 32; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "ModernCli JSON errors must stay on stdout; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "audit authorization rejection must be a v3 JSON envelope: {err}\nstdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    let object = envelope
        .as_object()
        .unwrap_or_else(|| panic!("audit rejection must be a JSON object: {envelope}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, V3_ERROR_KEYS,
        "audit rejection must use the closed v3 error envelope"
    );
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["zoneRef"], "Zone/local-root");
    assert_eq!(envelope["schemaVersion"], 1);
    assert_eq!(envelope["errorClass"], "authz-audit-requires-admin");
    let message = envelope["message"]
        .as_str()
        .expect("audit rejection envelope message");
    assert!(
        message.contains("audit requires an admin role"),
        "audit authz rejection should describe the admin-only contract; got:\n{message}"
    );
    assert!(
        !out.stdout
            .windows("daemon-down".len())
            .any(|window| { window == b"daemon-down" }),
        "a reachable daemon must not surface daemon-down"
    );
    assert!(
        !out.stdout
            .windows("auditResponse".len())
            .any(|window| { window == b"auditResponse" }),
        "rejected audit must not print an audit body"
    );
}

// --- in-process SOCK_SEQPACKET mock daemon ---------------------------------

/// Spawn a one-shot mock daemon that performs the audit handshake and returns
/// an `auditResponse` with two lines. Returns the joinable server thread.
fn spawn_audit_mock_daemon(path: &Path) -> std::thread::JoinHandle<()> {
    spawn_single_audit_response_mock(
        path,
        serde_json::json!({
            "type": "auditResponse",
            "entries": [
                {"sequence": 0, "record": "broker audit line 1"},
                {"sequence": 1, "record": "broker audit line 2"},
            ],
            "complete": true,
        }),
    )
}

fn spawn_single_audit_response_mock(path: &Path, response: Value) -> std::thread::JoinHandle<()> {
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
        send_frame(conn, &response);
        let _ = nix::unistd::close(conn);
    })
}

fn spawn_paginated_audit_mock_daemon(path: &Path) -> std::thread::JoinHandle<()> {
    use d2b_contracts::broker_wire::AuditExportCursor;
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
    bind(listener.as_raw_fd(), &addr).expect("bind paginated mock sock");
    listen(&listener, Backlog::new(1).unwrap()).expect("listen paginated mock sock");

    std::thread::spawn(move || {
        let conn = accept(listener.as_raw_fd()).expect("accept");
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

        let first_request = recv_frame(conn);
        assert_eq!(
            first_request["type"], "audit",
            "expected first audit frame, got {first_request}"
        );
        let cursor = AuditExportCursor {
            day: "2026-08-13".to_owned(),
            line: 0,
            sequence: 0,
        };
        send_frame(
            conn,
            &serde_json::json!({
                "type": "auditResponse",
                "entries": [{
                    "sequence": 0,
                    "record": "broker audit page 1"
                }],
                "nextCursor": cursor,
                "complete": false,
            }),
        );

        let second_request = recv_frame_with_timeout(conn);
        assert_eq!(
            second_request["type"], "audit",
            "expected second audit frame, got {second_request}"
        );
        assert_eq!(
            second_request["cursor"]["line"], 0,
            "CLI must send the continuation cursor"
        );
        send_frame(
            conn,
            &serde_json::json!({
                "type": "auditResponse",
                "entries": [{
                    "sequence": 1,
                    "record": "broker audit page 2"
                }],
                "complete": true,
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

fn recv_frame_with_timeout(fd: std::os::fd::RawFd) -> Value {
    use nix::errno::Errno;
    use nix::sys::socket::{MsgFlags, recv};
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match recv(fd, &mut buf, MsgFlags::MSG_DONTWAIT) {
            Ok(n) => {
                assert!(n >= 4, "short frame ({n} bytes)");
                let declared = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                let body = &buf[4..n];
                assert_eq!(body.len(), declared, "frame length mismatch");
                return serde_json::from_slice(body).expect("frame json");
            }
            Err(error) if error == Errno::EAGAIN || error == Errno::EWOULDBLOCK => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for paginated audit request"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("receive paginated audit frame: {error}"),
        }
    }
}

fn send_frame(fd: std::os::fd::RawFd, payload: &Value) {
    let body = serde_json::to_vec(payload).expect("serialize frame");
    let mut framed = (body.len() as u32).to_le_bytes().to_vec();
    framed.extend_from_slice(&body);
    let sent = nix::sys::socket::send(fd, &framed, nix::sys::socket::MsgFlags::empty())
        .expect("send frame");
    assert_eq!(sent, framed.len(), "short send");
}
