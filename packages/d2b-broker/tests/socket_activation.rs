//! Socket-activation integration test.
//!
//! Spawns the broker binary with `LISTEN_FDS=1 LISTEN_FDNAMES=priv.sock`
//! and fd 3 = a bound `AF_UNIX SOCK_SEQPACKET` listen socket, then asserts
//! that the broker:
//!
//! 1. Adopts the inherited fd rather than binding the socket itself.
//! 2. Starts serving requests on it (verified by a round-trip Hello).
//!
//! # How LISTEN_PID is set correctly
//!
//! Rust's `Command::spawn` captures the environment from the PARENT process
//! before fork; `setenv` calls in `pre_exec` do not affect the env array
//! passed to `execve`.  To work around this we launch a POSIX `sh` one-liner:
//!
//! ```sh
//! LISTEN_PID=$$ LISTEN_FDS=1 LISTEN_FDNAMES=priv.sock exec 3>&<fd> broker
//! ```
//!
//! `$$` expands to the shell's PID; after `exec`, the broker runs in the same
//! process (same PID), so `LISTEN_PID == broker's std::process::id()`.
//! The `3>&<fd>` redirect duplicates the inherited listen socket to fd 3;
//! `dup2` (internally used by the shell) clears FD_CLOEXEC on fd 3.

#![cfg(not(feature = "layer1-bootstrap"))]

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use d2b_broker::protocol::connect_seqpacket_bounded;
use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse, HelloRequest,
};
use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, listen, socket,
};
use tempfile::TempDir;

/// Path to the broker binary under test; set by cargo when building
/// integration tests for this package.
fn broker_bin() -> std::path::PathBuf {
    if let Some(bin) = std::env::var_os("CARGO_BIN_EXE_d2b-broker") {
        return std::path::PathBuf::from(bin);
    }
    // Cargo integration tests run with the package manifest dir as cwd;
    // resolve the workspace target dir from there at RUNTIME (a compile-time
    // env!("CARGO_MANIFEST_DIR") would embed an absolute sandbox path, which
    // the bazel build forbids).
    let candidate = std::env::current_dir()
        .expect("test cwd")
        .join("../../target/debug/d2b-broker");
    assert!(
        candidate.exists(),
        "the d2b-broker binary is not built; run `cargo build -p d2b-broker-composition --bin d2b-broker` first (expected at {})",
        candidate.display()
    );
    candidate
}

fn scratch_dir() -> TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn scrub_shell_environment_path() -> PathBuf {
    if let Some(path) = std::env::var_os("D2B_TEST_SCRUB_SHELL_ENVIRONMENT") {
        return PathBuf::from(path);
    }

    let mut root = std::env::current_dir().expect("resolve current directory");
    loop {
        let candidate = root.join("tests/tools/scrub-shell-environment");
        if candidate.is_file() {
            return candidate;
        }
        if !root.pop() {
            break;
        }
    }
    panic!("tests/tools/scrub-shell-environment is not discoverable");
}

/// Create a bound, listening `AF_UNIX SOCK_SEQPACKET` socket at `path`.
fn bind_seqpacket_listen(path: &std::path::Path) -> io::Result<OwnedFd> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let addr = UnixAddr::new(path).map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    bind(fd.as_raw_fd(), &addr).map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    listen(
        &fd,
        Backlog::new(8).map_err(|e| io::Error::from_raw_os_error(e as i32))?,
    )
    .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    Ok(fd)
}

/// Verify the broker adopts the socket-activated fd and handles a Hello.
///
/// We launch the broker via a POSIX `sh` one-liner so that `LISTEN_PID=$$`
/// expands to the shell's PID, which becomes the broker's PID after `exec`.
/// The listen socket is passed into the shell via its inherited fd number;
/// the shell redirects it to fd 3 before exec-ing the broker.
#[test]
#[allow(unsafe_code)]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn broker_adopts_socket_activated_fd_and_serves_hello() {
    let scratch = scratch_dir();
    let sock_path = scratch.path().join("priv.sock");
    let audit_dir = scratch.path().join("audit");
    std::fs::create_dir_all(&audit_dir).expect("create audit dir");

    // Create the listening socket in the parent.
    let listen_fd = bind_seqpacket_listen(&sock_path).expect("bind seqpacket");
    let listen_fd_raw = listen_fd.as_raw_fd();

    // Clear FD_CLOEXEC so the fd survives the fork+exec into `sh`.
    // The shell's `3>&<fd_raw>` redirect then dup2s it to fd 3; dup2 itself
    // clears FD_CLOEXEC on the new fd, so the broker receives a non-cloexec fd 3.
    // SAFETY: fcntl with F_GETFD/F_SETFD is always safe for a valid open fd.
    unsafe {
        let flags = libc::fcntl(listen_fd_raw, libc::F_GETFD);
        assert!(flags >= 0, "F_GETFD failed: {}", io::Error::last_os_error());
        let rc = libc::fcntl(listen_fd_raw, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        assert!(rc >= 0, "F_SETFD failed: {}", io::Error::last_os_error());
    }

    let broker_bin = broker_bin();
    assert!(
        broker_bin.exists(),
        "broker binary not found at {}: set CARGO_BIN_EXE_d2b-broker",
        broker_bin.display()
    );

    let current_uid = nix::unistd::Uid::current().as_raw();
    let current_gid = nix::unistd::Gid::current().as_raw();
    let audit_dir_str = audit_dir.to_str().expect("audit dir utf8").to_owned();
    let broker_bin_str = broker_bin.to_str().expect("broker bin utf8").to_owned();

    // Shell one-liner:
    //   LISTEN_PID=$$       → shell's PID; after `exec`, broker's PID matches.
    //   LISTEN_FDS=1        → one socket fd follows.
    //   LISTEN_FDNAMES=priv.sock → matches the broker's fd-name expectation.
    //   exec 3>&<fd> broker → redirect listen socket to fd 3, then exec broker.
    //
    // Variable references ($BROKER etc.) protect against path quoting issues.
    let shell_cmd = format!(
        "LISTEN_PID=$$ LISTEN_FDS=1 LISTEN_FDNAMES=priv.sock \
         exec 3>&{listen_fd_raw} \
         \"$BROKER\" host \
         --test-mode \
         --d2bd-uid \"$UID\" \
         --d2bd-gid \"$GID\" \
         --audit-dir \"$AUDIT_DIR\" \
         --bundle-path /nonexistent/bundle.json"
    );

    let scrubber = scrub_shell_environment_path();
    let mut broker_proc = Command::new(scrubber)
        .args(["-c", &shell_cmd])
        .env("BROKER", &broker_bin_str)
        .env("UID", current_uid.to_string())
        .env("GID", current_gid.to_string())
        .env("AUDIT_DIR", &audit_dir_str)
        .env("RUST_LOG", "off")
        .spawn()
        .expect("spawn sh → broker");

    // Parent's copy is no longer needed; the child has inherited it.
    drop(listen_fd);

    // Wait until the broker's accept loop is actually serving: retry the
    // whole Hello round trip until it completes, fail fast if the broker
    // exits, and keep a generous deadline only as a last-resort guard
    // against a wedged broker. The wait is progress-based - each attempt
    // either completes or is retried, so wall-clock load cannot trip it.
    let response = {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("socket-activation test runtime");
        runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = broker_proc.try_wait().expect("try_wait") {
                    panic!(
                        "broker exited prematurely with status {status}; \
                         socket_path={}",
                        sock_path.display()
                    );
                }
                let attempt = async {
                    let client =
                        connect_seqpacket_bounded(&sock_path, Duration::from_millis(500)).await?;
                    let envelope = BrokerRequestEnvelope {
                        request: BrokerRequest::Hello(HelloRequest {
                            client_version: "test-0".to_owned(),
                            supported_features: vec![],
                        }),
                        caller_role: BrokerCallerRole::default(),
                        audit_join: None,
                    };
                    client.send_json_frame(&envelope).await?;
                    client.recv_json_frame::<BrokerResponse>().await
                };
                match tokio::time::timeout(Duration::from_millis(500), attempt).await {
                    Ok(Ok(Some(response))) => break response,
                    Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "broker never served Hello within 30 s; socket_path={}",
                            sock_path.display()
                        );
                    }
                }
            }
        })
    };

    // Kill the broker before asserting so cleanup runs even on assertion failure.
    let _ = broker_proc.kill();
    let _ = broker_proc.wait();

    match response {
        BrokerResponse::Hello(ref hello_resp) => {
            assert!(
                !hello_resp.server_version.is_empty(),
                "HelloResponse.server_version should be non-empty"
            );
        }
        other => {
            panic!("expected BrokerResponse::Hello, got {:?}", other);
        }
    }
}

use nix::libc;
