//! Socket-activation integration test.
//!
//! Spawns the broker binary with `LISTEN_FDS=1 LISTEN_FDNAMES=priv.sock`
//! and fd 3 = a bound `AF_UNIX SOCK_SEQPACKET` listen socket, then asserts
//! that the broker:
//!
//! 1. Adopts the inherited fd rather than binding the socket itself.
//! 2. Establishes an authenticated `d2b.broker.v2` ComponentSession.
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

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use d2b_contracts::v2_component_session::{AttachmentDescriptor, EndpointRole, Locality};
use d2b_priv_broker::service_v2::{BrokerPeerRole, broker_endpoint_policy};
use d2b_session::{HandshakeCredentials, SessionEngine};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, PeerIdentityPolicy, SeqpacketSocket, UnixSeqpacketTransport,
    UnixSessionError,
};
use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, connect, listen, socket,
};
/// Path to the broker binary under test; set by cargo when building
/// integration tests for this package.
const BROKER_BIN: &str = env!("CARGO_BIN_EXE_d2b-priv-broker");

struct ScratchDir(tempfile::TempDir);

impl ScratchDir {
    fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}

fn scratch_dir() -> ScratchDir {
    let Some(root) = std::env::var_os("D2B_VALIDATION_SOCKET_DIR").map(std::path::PathBuf::from)
    else {
        return ScratchDir(tempfile::tempdir().expect("create socket activation test tempdir"));
    };
    std::fs::create_dir_all(&root).expect("create socket activation test root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("harden socket activation test root");
    ScratchDir(tempfile::tempdir_in(root).expect("create socket activation tempdir"))
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

fn credit_scopes(limit: usize) -> CreditScopeSet {
    CreditScopeSet::new(
        CreditPool::new(limit).expect("packet credit"),
        CreditPool::new(limit).expect("request credit"),
        CreditPool::new(limit).expect("operation credit"),
        CreditPool::new(limit).expect("session credit"),
        CreditPool::new(limit).expect("process credit"),
        CreditPool::new(limit).expect("host credit"),
    )
}

fn connect_seqpacket(path: &std::path::Path) -> io::Result<OwnedFd> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    )
    .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    let address =
        UnixAddr::new(path).map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    connect(fd.as_raw_fd(), &address)
        .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    Ok(fd)
}

/// Verify the broker adopts the socket-activated fd and authenticates v2.
///
/// We launch the broker via a POSIX `sh` one-liner so that `LISTEN_PID=$$`
/// expands to the shell's PID, which becomes the broker's PID after `exec`.
/// The listen socket is passed into the shell via its inherited fd number;
/// the shell redirects it to fd 3 before exec-ing the broker.
#[tokio::test(flavor = "current_thread")]
#[allow(unsafe_code)]
async fn broker_adopts_socket_activated_fd_and_serves_component_session() {
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

    let broker_bin = PathBuf::from(BROKER_BIN);
    assert!(
        broker_bin.exists(),
        "broker binary not found at {}: set CARGO_BIN_EXE_d2b-priv-broker",
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
         \"$BROKER\" serve \
         --test-mode \
         --d2bd-uid \"$UID\" \
         --d2bd-gid \"$GID\" \
         --audit-dir \"$AUDIT_DIR\" \
         --bundle-path /nonexistent/bundle.json"
    );

    let mut broker_proc = Command::new("sh")
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

    // Allow time for the broker to initialise the accept loop.
    std::thread::sleep(Duration::from_millis(400));

    // Verify the broker is still running.
    if let Some(status) = broker_proc.try_wait().expect("try_wait") {
        panic!(
            "broker exited prematurely with status {status}; \
             socket_path={}",
            sock_path.display()
        );
    }

    // Connect and complete the authenticated broker-v2 handshake. No JSON
    // negotiation or compatibility probe is sent.
    let client_fd = connect_seqpacket(&sock_path).expect("connect to activated socket");
    let socket = SeqpacketSocket::from_owned(client_fd).expect("client seqpacket");
    let listener_owner_pid = std::process::id();
    let verifier = Arc::new(move |peer: &SeqpacketSocket| {
        let credentials = peer.acceptor_peer_credentials()?;
        if Some(credentials.pid())
            == rustix::process::Pid::from_raw(i32::try_from(listener_owner_pid).unwrap_or(-1))
            && credentials.uid().as_raw() == current_uid
            && credentials.gid().as_raw() == current_gid
        {
            Ok(())
        } else {
            Err(UnixSessionError::CredentialMismatch)
        }
    });
    let policy = broker_endpoint_policy(
        BrokerPeerRole::LocalRootController,
        EndpointRole::LocalRootBroker,
        1,
        d2b_priv_broker::service_v2::broker_channel_binding(
            current_uid,
            current_gid,
            EndpointRole::LocalRootBroker,
        ),
    )
    .expect("broker policy");
    let resolver = Arc::new(|_: &AttachmentDescriptor| Err(UnixSessionError::DescriptorMismatch));
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credit_scopes(usize::from(policy.attachment_policy.max_per_session)),
        resolver,
        PeerIdentityPolicy::pathname(verifier),
    )
    .expect("client transport");
    let session = SessionEngine::establish_initiator(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .expect("authenticated broker component session");
    drop(session);

    // Kill the broker before asserting so cleanup runs even on assertion failure.
    let _ = broker_proc.kill();
    let _ = broker_proc.wait();
}

use nix::libc;
