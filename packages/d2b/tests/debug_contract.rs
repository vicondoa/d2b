//! Behavior-level contract tests for `d2b debug`.
//!
//! These drive the real binary, which is where the refusal outcomes are
//! observable: an unreachable zone refuses with the zone-unavailable class
//! and prints no report, and a zone disagreement is a usage error refused
//! before any request.
//!
//! The rest of the exit contract - the absent named row, the empty zone, and
//! the per-type degradation - shares this read path but needs a fake daemon
//! that speaks a whole session, which would test the fake rather than the
//! command. Those outcomes are covered by the module tests in
//! `src/debug.rs` and by the live lane, where
//! `tests/host-integration/resource-operator-activation.nix` exercises the
//! command against a real daemon.

use std::{
    env,
    os::fd::AsRawFd,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
};

use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, UnixAddr, accept, bind, listen, socket,
};

fn socket_path(name: &str) -> PathBuf {
    let path = env::temp_dir().join(format!(
        "d2b-debug-contract-{name}-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

/// A live listening socket that never answers, so the zone reads as present
/// but not serving. The thread is detached on drop; a test can legitimately
/// end while it is still waiting for a connection.
fn unanswering_socket(path: &Path) -> std::thread::JoinHandle<()> {
    let _ = std::fs::remove_file(path);
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(
        listener.as_raw_fd(),
        &UnixAddr::new(path.as_os_str().as_bytes()).unwrap(),
    )
    .unwrap();
    listen(&listener, Backlog::new(8).unwrap()).unwrap();
    std::thread::spawn(move || loop {
        if accept(listener.as_raw_fd()).is_err() {
            return;
        }
    })
}

fn run_debug(socket: &Path, args: &[&str]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .env("D2B_PUBLIC_SOCKET", socket)
        .args(args)
        .output()
        .expect("run d2b");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn an_unreachable_zone_refuses_without_rendering_a_report() {
    let socket = socket_path("unreachable");
    let (code, stdout) = run_debug(&socket, &["debug", "work"]);

    assert_eq!(code, 1, "an unanswering zone is a refusal, not a report");
    assert!(
        stdout.contains("zone-unavailable"),
        "the refusal names the class: {stdout}"
    );
    assert!(
        !stdout.contains("\"roots\""),
        "no partial tree is printed: {stdout}"
    );
}

#[test]
fn an_explicit_zone_that_disagrees_with_the_positional_zone_is_refused() {
    // R22 and AE9: the disagreement is refused before any connection, so this
    // holds even with no daemon at all.
    let socket = socket_path("mismatch");
    let (code, stdout) = run_debug(&socket, &["--zone", "dev", "debug", "prod"]);

    assert_eq!(code, 2, "a zone disagreement is a usage error: {stdout}");
    assert!(
        stdout.contains("ref-invalid"),
        "the refusal names the class: {stdout}"
    );
    assert!(
        !stdout.contains("\"roots\""),
        "no tree is printed: {stdout}"
    );
}

#[test]
fn a_present_but_unanswering_zone_is_not_reported_as_a_healthy_empty_zone() {
    let socket = socket_path("silent");
    let _server = unanswering_socket(&socket);
    let (code, stdout) = run_debug(&socket, &["debug", "work"]);

    assert_ne!(
        code, 0,
        "a zone that never answers is not a successful empty report: {stdout}"
    );
    assert!(
        !stdout.contains("\"roots\": []"),
        "an empty tree must never stand in for an unanswering zone: {stdout}"
    );
}
