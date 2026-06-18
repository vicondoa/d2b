//! Host-side Cloud Hypervisor CONNECT helper for guest-control transport.
//!
//! This module only opens the transport stream. A successful CONNECT is not a
//! guest health result and must not be used as VM readiness.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use socket2::{Domain, SockAddr, Socket, Type};

pub const GUEST_CONTROL_CONNECT_PORT: u16 = 14_318;
pub const GUEST_CONTROL_CONNECT_LINE: &[u8] = b"CONNECT 14318\n";
const MAX_ACK_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestControlTransportFailure {
    SocketPathNotAbsolute,
    StateRootNotAbsolute,
    SocketPathTraversal,
    StateRootInvalid,
    SocketOutsideStateRoot,
    SocketMissing,
    SocketIsSymlink,
    SocketNotUnixSocket,
    SocketHardLinked,
    UnsafeDirectory,
    PeerCredentialIo { kind: String },
    PeerCredentialMismatch,
    ConnectIo { kind: String },
    WriteIo { kind: String },
    AckIo { kind: String },
    AckTimeout,
    AckEof,
    AckTooLong,
    AckMalformed,
}

pub struct GuestControlConnectedStream {
    socket: Socket,
    ack_token: String,
}

impl GuestControlConnectedStream {
    pub fn into_socket(self) -> Socket {
        self.socket
    }

    pub fn ack_token(&self) -> &str {
        &self.ack_token
    }
}

pub enum GuestControlTransportProbeResult {
    Connected(GuestControlConnectedStream),
    Failed(GuestControlTransportFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryPolicy {
    ProductionStateRoot {
        uid: u32,
        gid: u32,
    },
    #[cfg(test)]
    AllowTestTempDirs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerPolicy {
    Expected {
        uid: u32,
        gid: u32,
    },
    #[cfg(test)]
    CurrentProcess,
}

impl GuestControlTransportProbeResult {
    pub fn failure(&self) -> Option<&GuestControlTransportFailure> {
        match self {
            Self::Connected(_) => None,
            Self::Failed(failure) => Some(failure),
        }
    }
}

pub fn connect_guest_control_vsock(
    socket_path: impl AsRef<Path>,
    state_root: impl AsRef<Path>,
    expected_state_root_uid: u32,
    expected_state_root_gid: u32,
    expected_peer_uid: u32,
    expected_peer_gid: u32,
    setup_timeout: Duration,
) -> GuestControlTransportProbeResult {
    match connect_guest_control_vsock_inner(
        socket_path.as_ref(),
        state_root.as_ref(),
        setup_timeout,
        DirectoryPolicy::ProductionStateRoot {
            uid: expected_state_root_uid,
            gid: expected_state_root_gid,
        },
        PeerPolicy::Expected {
            uid: expected_peer_uid,
            gid: expected_peer_gid,
        },
    ) {
        Ok(connected) => GuestControlTransportProbeResult::Connected(connected),
        Err(failure) => GuestControlTransportProbeResult::Failed(failure),
    }
}

#[cfg(test)]
pub(crate) fn connect_guest_control_vsock_for_tests(
    socket_path: impl AsRef<Path>,
    state_root: impl AsRef<Path>,
    setup_timeout: Duration,
) -> GuestControlTransportProbeResult {
    match connect_guest_control_vsock_inner(
        socket_path.as_ref(),
        state_root.as_ref(),
        setup_timeout,
        DirectoryPolicy::AllowTestTempDirs,
        PeerPolicy::CurrentProcess,
    ) {
        Ok(connected) => GuestControlTransportProbeResult::Connected(connected),
        Err(failure) => GuestControlTransportProbeResult::Failed(failure),
    }
}

fn connect_guest_control_vsock_inner(
    socket_path: &Path,
    state_root: &Path,
    setup_timeout: Duration,
    directory_policy: DirectoryPolicy,
    peer_policy: PeerPolicy,
) -> Result<GuestControlConnectedStream, GuestControlTransportFailure> {
    validate_socket_path(socket_path, state_root, directory_policy)?;

    let deadline = Instant::now() + setup_timeout;
    let mut socket = connect_unix_socket_with_timeout(socket_path, remaining_setup_time(deadline)?)
        .map_err(|error| GuestControlTransportFailure::ConnectIo {
            kind: error.kind().to_string(),
        })?;
    validate_peer_credentials(&socket, peer_policy)?;
    let remaining = remaining_setup_time(deadline)?;
    socket
        .set_read_timeout(Some(remaining))
        .map_err(io_failure(|kind| GuestControlTransportFailure::AckIo {
            kind,
        }))?;
    socket
        .set_write_timeout(Some(remaining))
        .map_err(io_failure(|kind| GuestControlTransportFailure::WriteIo {
            kind,
        }))?;
    socket
        .write_all(GUEST_CONTROL_CONNECT_LINE)
        .map_err(io_failure(|kind| GuestControlTransportFailure::WriteIo {
            kind,
        }))?;
    let ack_token = read_ack_token(&mut socket, deadline)?;
    socket.set_read_timeout(None).map_err(io_failure(|kind| {
        GuestControlTransportFailure::AckIo { kind }
    }))?;
    socket.set_write_timeout(None).map_err(io_failure(|kind| {
        GuestControlTransportFailure::WriteIo { kind }
    }))?;
    Ok(GuestControlConnectedStream { socket, ack_token })
}

fn connect_unix_socket_with_timeout(
    socket_path: &Path,
    timeout: Duration,
) -> std::io::Result<Socket> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    let addr = SockAddr::unix(socket_path)?;
    socket.connect_timeout(&addr, timeout)?;
    Ok(socket)
}

fn validate_peer_credentials(
    socket: &Socket,
    peer_policy: PeerPolicy,
) -> Result<(), GuestControlTransportFailure> {
    let peer = getsockopt(socket, PeerCredentials).map_err(|error| {
        GuestControlTransportFailure::PeerCredentialIo {
            kind: error.to_string(),
        }
    })?;
    let (expected_uid, expected_gid) = match peer_policy {
        PeerPolicy::Expected { uid, gid } => (uid, gid),
        #[cfg(test)]
        PeerPolicy::CurrentProcess => (current_uid_for_tests(), current_gid_for_tests()),
    };
    if peer.uid() as u32 != expected_uid || peer.gid() as u32 != expected_gid {
        return Err(GuestControlTransportFailure::PeerCredentialMismatch);
    }
    Ok(())
}

#[cfg(test)]
fn current_uid_for_tests() -> u32 {
    nix::unistd::geteuid().as_raw()
}

#[cfg(test)]
fn current_gid_for_tests() -> u32 {
    nix::unistd::getgid().as_raw()
}

fn validate_socket_path(
    socket_path: &Path,
    state_root: &Path,
    directory_policy: DirectoryPolicy,
) -> Result<(), GuestControlTransportFailure> {
    if !socket_path.is_absolute() {
        return Err(GuestControlTransportFailure::SocketPathNotAbsolute);
    }
    if !state_root.is_absolute() {
        return Err(GuestControlTransportFailure::StateRootNotAbsolute);
    }
    if has_parent_dir(socket_path) || has_parent_dir(state_root) {
        return Err(GuestControlTransportFailure::SocketPathTraversal);
    }

    let canonical_root =
        fs::canonicalize(state_root).map_err(|_| GuestControlTransportFailure::StateRootInvalid)?;
    validate_root_ancestors(&canonical_root, directory_policy)?;
    validate_directory_chain(&canonical_root, &canonical_root, directory_policy)?;
    let parent = socket_path
        .parent()
        .ok_or(GuestControlTransportFailure::SocketOutsideStateRoot)?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|_| GuestControlTransportFailure::SocketMissing)?;
    if canonical_parent != canonical_root {
        return Err(GuestControlTransportFailure::SocketOutsideStateRoot);
    }

    let metadata = fs::symlink_metadata(socket_path)
        .map_err(|_| GuestControlTransportFailure::SocketMissing)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(GuestControlTransportFailure::SocketIsSymlink);
    }
    if !file_type.is_socket() {
        return Err(GuestControlTransportFailure::SocketNotUnixSocket);
    }
    if metadata.nlink() != 1 {
        return Err(GuestControlTransportFailure::SocketHardLinked);
    }
    Ok(())
}

fn validate_root_ancestors(
    root: &Path,
    _directory_policy: DirectoryPolicy,
) -> Result<(), GuestControlTransportFailure> {
    #[cfg(test)]
    if matches!(_directory_policy, DirectoryPolicy::AllowTestTempDirs) {
        return Ok(());
    }
    let mut ancestors = Vec::new();
    let mut current = root;
    while let Some(parent) = current.parent() {
        ancestors.push(parent.to_path_buf());
        current = parent;
    }
    ancestors.reverse();
    for ancestor in ancestors {
        validate_root_owned_directory(&ancestor)?;
    }
    Ok(())
}

fn validate_directory_chain(
    root: &Path,
    leaf: &Path,
    directory_policy: DirectoryPolicy,
) -> Result<(), GuestControlTransportFailure> {
    if !leaf.starts_with(root) {
        return Err(GuestControlTransportFailure::SocketOutsideStateRoot);
    }
    let mut current = root.to_path_buf();
    validate_directory_metadata(&current, directory_policy)?;
    let relative = leaf
        .strip_prefix(root)
        .map_err(|_| GuestControlTransportFailure::SocketOutsideStateRoot)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(GuestControlTransportFailure::SocketPathTraversal);
        };
        current.push(name);
        validate_directory_metadata(&current, directory_policy)?;
    }
    Ok(())
}

fn validate_directory_metadata(
    path: &Path,
    directory_policy: DirectoryPolicy,
) -> Result<(), GuestControlTransportFailure> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| GuestControlTransportFailure::StateRootInvalid)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(GuestControlTransportFailure::UnsafeDirectory);
    }
    match directory_policy {
        DirectoryPolicy::ProductionStateRoot { uid, gid } => {
            if metadata.uid() != uid || metadata.gid() != gid || (metadata.mode() & 0o002) != 0 {
                return Err(GuestControlTransportFailure::UnsafeDirectory);
            }
        }
        #[cfg(test)]
        DirectoryPolicy::AllowTestTempDirs => {}
    }
    Ok(())
}

fn validate_root_owned_directory(path: &Path) -> Result<(), GuestControlTransportFailure> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| GuestControlTransportFailure::StateRootInvalid)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || (metadata.mode() & 0o022) != 0
    {
        return Err(GuestControlTransportFailure::UnsafeDirectory);
    }
    Ok(())
}

fn has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn read_ack_token(
    stream: &mut Socket,
    deadline: Instant,
) -> Result<String, GuestControlTransportFailure> {
    let mut ack = Vec::with_capacity(MAX_ACK_BYTES);
    let mut byte = [0_u8; 1];
    loop {
        stream
            .set_read_timeout(Some(remaining_setup_time(deadline)?))
            .map_err(io_failure(|kind| GuestControlTransportFailure::AckIo {
                kind,
            }))?;
        match stream.read(&mut byte) {
            Ok(0) => return Err(GuestControlTransportFailure::AckEof),
            Ok(_) => {
                ack.push(byte[0]);
                if ack.len() > MAX_ACK_BYTES {
                    return Err(GuestControlTransportFailure::AckTooLong);
                }
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                return Err(GuestControlTransportFailure::AckTimeout);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(GuestControlTransportFailure::AckIo {
                    kind: error.kind().to_string(),
                });
            }
        }
    }

    let line = std::str::from_utf8(&ack).map_err(|_| GuestControlTransportFailure::AckMalformed)?;
    let token = line
        .strip_prefix("OK ")
        .and_then(|value| value.strip_suffix('\n'))
        .ok_or(GuestControlTransportFailure::AckMalformed)?;
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GuestControlTransportFailure::AckMalformed);
    }
    Ok(token.to_owned())
}

fn remaining_setup_time(deadline: Instant) -> Result<Duration, GuestControlTransportFailure> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(GuestControlTransportFailure::AckTimeout)
}

fn io_failure<F>(constructor: F) -> impl FnOnce(std::io::Error) -> GuestControlTransportFailure
where
    F: FnOnce(String) -> GuestControlTransportFailure,
{
    move |error| constructor(error.kind().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::{Read, Write};
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    fn state_root() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn socket_path(root: &TempDir) -> PathBuf {
        root.path().join("vsock.sock")
    }

    fn connect(path: &Path, root: &Path) -> GuestControlTransportProbeResult {
        connect_guest_control_vsock_for_tests(path, root, Duration::from_millis(100))
    }

    fn fake_ch<F>(path: &Path, responder: F) -> thread::JoinHandle<Vec<u8>>
    where
        F: FnOnce(&mut UnixStream) + Send + 'static,
    {
        let listener = UnixListener::bind(path).expect("bind fake ch socket");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fake ch client");
            let mut request = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                let read = stream.read(&mut byte).expect("read request byte");
                if read == 0 {
                    break;
                }
                request.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            responder(&mut stream);
            request
        })
    }

    #[test]
    fn connects_with_exact_handshake_and_ack_token() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |stream| {
            stream.write_all(b"OK 7\n").expect("write ack");
        });

        let result = connect(&socket, root.path());
        let request = handle.join().expect("fake ch thread joins");
        assert_eq!(request, GUEST_CONTROL_CONNECT_LINE);
        match result {
            GuestControlTransportProbeResult::Connected(stream) => {
                assert_eq!(stream.ack_token(), "7");
            }
            GuestControlTransportProbeResult::Failed(failure) => {
                panic!("unexpected failure: {failure:?}");
            }
        }
    }

    #[test]
    fn rejects_non_absolute_socket_path() {
        let root = state_root();
        assert_eq!(
            connect(Path::new("vsock.sock"), root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketPathNotAbsolute)
        );
    }

    #[test]
    fn rejects_non_absolute_state_root() {
        assert_eq!(
            connect(Path::new("/tmp/vsock.sock"), Path::new("relative-root")).failure(),
            Some(&GuestControlTransportFailure::StateRootNotAbsolute)
        );
    }

    #[test]
    fn production_directory_policy_rejects_wrong_owner_and_world_write() {
        let root = state_root();
        let metadata = fs::symlink_metadata(root.path()).expect("metadata");
        let policy = DirectoryPolicy::ProductionStateRoot {
            uid: metadata.uid(),
            gid: metadata.gid(),
        };
        assert_eq!(validate_directory_metadata(root.path(), policy), Ok(()));

        let wrong_uid_policy = DirectoryPolicy::ProductionStateRoot {
            uid: metadata.uid().wrapping_add(1),
            gid: metadata.gid(),
        };
        assert_eq!(
            validate_directory_metadata(root.path(), wrong_uid_policy),
            Err(GuestControlTransportFailure::UnsafeDirectory)
        );

        let mut permissions = metadata.permissions();
        permissions.set_mode(metadata.mode() | 0o002);
        fs::set_permissions(root.path(), permissions).expect("make world-writable");
        assert_eq!(
            validate_directory_metadata(root.path(), policy),
            Err(GuestControlTransportFailure::UnsafeDirectory)
        );
    }

    #[test]
    fn production_root_ancestor_policy_rejects_world_writable_tmp() {
        assert_eq!(
            validate_root_owned_directory(Path::new("/tmp")),
            Err(GuestControlTransportFailure::UnsafeDirectory)
        );
    }

    #[test]
    fn rejects_parent_dir_escape() {
        let root = state_root();
        let path = root.path().join("..").join("outside.sock");
        assert_eq!(
            connect(&path, root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketPathTraversal)
        );
    }

    #[test]
    fn rejects_missing_socket() {
        let root = state_root();
        assert_eq!(
            connect(&socket_path(&root), root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketMissing)
        );
    }

    #[test]
    fn rejects_regular_file_socket_path() {
        let root = state_root();
        let socket = socket_path(&root);
        File::create(&socket).expect("regular file");
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketNotUnixSocket)
        );
    }

    #[test]
    fn rejects_peer_credential_mismatch() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |stream| {
            stream.write_all(b"OK 7\n").expect("write ack");
        });
        let mismatched_uid = current_uid_for_tests().wrapping_add(1);
        let result = connect_guest_control_vsock_inner(
            &socket,
            root.path(),
            Duration::from_millis(100),
            DirectoryPolicy::AllowTestTempDirs,
            PeerPolicy::Expected {
                uid: mismatched_uid,
                gid: current_gid_for_tests(),
            },
        );
        match result {
            Err(failure) => {
                assert_eq!(
                    failure,
                    GuestControlTransportFailure::PeerCredentialMismatch
                );
            }
            Ok(_) => panic!("peer mismatch unexpectedly connected"),
        }
        let _ = handle.join();
    }

    #[test]
    fn rejects_hard_linked_socket_path() {
        let root = state_root();
        let socket = socket_path(&root);
        let linked = root.path().join("linked.sock");
        let _listener = UnixListener::bind(&socket).expect("bind socket");
        fs::hard_link(&socket, &linked).expect("hard-link socket");
        assert_eq!(
            connect(&linked, root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketHardLinked)
        );
    }

    #[test]
    fn rejects_socket_symlink_escape() {
        let root = state_root();
        let outside = state_root();
        let outside_socket = outside.path().join("outside.sock");
        let _listener = UnixListener::bind(&outside_socket).expect("outside socket");
        let socket = socket_path(&root);
        symlink(&outside_socket, &socket).expect("socket symlink");
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketIsSymlink)
        );
    }

    #[test]
    fn rejects_parent_symlink_escape() {
        let root = state_root();
        let outside = state_root();
        let outside_socket = outside.path().join("vsock.sock");
        let _listener = UnixListener::bind(&outside_socket).expect("outside socket");
        let link = root.path().join("link");
        symlink(outside.path(), &link).expect("parent symlink");
        let socket = link.join("vsock.sock");
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketOutsideStateRoot)
        );
    }

    #[test]
    fn rejects_malformed_ack() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |stream| {
            stream.write_all(b"NOPE 7\n").expect("write ack");
        });
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::AckMalformed)
        );
        let _ = handle.join();
    }

    #[test]
    fn rejects_non_numeric_ack_token() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |stream| {
            stream.write_all(b"OK token\n").expect("write ack");
        });
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::AckMalformed)
        );
        let _ = handle.join();
    }

    #[test]
    fn rejects_ack_eof() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |_stream| {});
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::AckEof)
        );
        let _ = handle.join();
    }

    #[test]
    fn rejects_overlong_ack() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |stream| {
            let mut ack = b"OK ".to_vec();
            ack.extend(std::iter::repeat_n(b'1', MAX_ACK_BYTES));
            stream.write_all(&ack).expect("write ack");
        });
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::AckTooLong)
        );
        let _ = handle.join();
    }

    #[test]
    fn rejects_ack_timeout() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |_stream| {
            thread::sleep(Duration::from_millis(250));
        });
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::AckTimeout)
        );
        let _ = handle.join();
    }

    #[test]
    fn slow_drip_ack_respects_single_setup_deadline() {
        let root = state_root();
        let socket = socket_path(&root);
        let handle = fake_ch(&socket, |stream| {
            let _ = stream.write_all(b"O");
            thread::sleep(Duration::from_millis(75));
            let _ = stream.write_all(b"K");
            thread::sleep(Duration::from_millis(75));
            let _ = stream.write_all(b" 1\n");
        });
        let result =
            connect_guest_control_vsock_for_tests(&socket, root.path(), Duration::from_millis(100));
        assert_eq!(
            result.failure(),
            Some(&GuestControlTransportFailure::AckTimeout)
        );
        let _ = handle.join();
    }

    #[test]
    fn uses_base_socket_not_port_suffixed_socket() {
        let root = state_root();
        let base = socket_path(&root);
        let suffixed = root.path().join("vsock.sock_14318");
        let handle = fake_ch(&base, |stream| {
            stream.write_all(b"OK 99\n").expect("write ack");
        });
        let result = connect(&base, root.path());
        let _ = handle.join();
        assert!(matches!(
            result,
            GuestControlTransportProbeResult::Connected(_)
        ));
        assert!(!suffixed.exists());
    }

    #[test]
    fn rejects_canonical_parent_outside_state_root() {
        let root = state_root();
        let outside = state_root();
        let nested = outside.path().join("nested");
        fs::create_dir(&nested).expect("nested outside dir");
        let socket = nested.join("vsock.sock");
        let _listener = UnixListener::bind(&socket).expect("outside socket");
        assert_eq!(
            connect(&socket, root.path()).failure(),
            Some(&GuestControlTransportFailure::SocketOutsideStateRoot)
        );
    }
}
