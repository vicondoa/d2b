//! Host-side Cloud Hypervisor CONNECT helper for guest-control transport.
//!
//! This module only opens the transport stream. A successful CONNECT is not a
//! guest health result and must not be used as VM readiness.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path};
use std::time::Duration;

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
    ConnectIo { kind: String },
    WriteIo { kind: String },
    AckIo { kind: String },
    AckTimeout,
    AckEof,
    AckTooLong,
    AckMalformed,
}

pub struct GuestControlConnectedStream {
    stream: UnixStream,
    ack_token: String,
}

impl GuestControlConnectedStream {
    pub fn into_stream(self) -> UnixStream {
        self.stream
    }

    pub fn ack_token(&self) -> &str {
        &self.ack_token
    }
}

pub enum GuestControlTransportProbeResult {
    Connected(GuestControlConnectedStream),
    Failed(GuestControlTransportFailure),
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
    setup_timeout: Duration,
) -> GuestControlTransportProbeResult {
    match connect_guest_control_vsock_inner(
        socket_path.as_ref(),
        state_root.as_ref(),
        setup_timeout,
    ) {
        Ok(connected) => GuestControlTransportProbeResult::Connected(connected),
        Err(failure) => GuestControlTransportProbeResult::Failed(failure),
    }
}

fn connect_guest_control_vsock_inner(
    socket_path: &Path,
    state_root: &Path,
    setup_timeout: Duration,
) -> Result<GuestControlConnectedStream, GuestControlTransportFailure> {
    validate_socket_path(socket_path, state_root)?;

    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        GuestControlTransportFailure::ConnectIo {
            kind: error.kind().to_string(),
        }
    })?;
    stream
        .set_read_timeout(Some(setup_timeout))
        .map_err(io_failure(|kind| GuestControlTransportFailure::AckIo {
            kind,
        }))?;
    stream
        .set_write_timeout(Some(setup_timeout))
        .map_err(io_failure(|kind| GuestControlTransportFailure::WriteIo {
            kind,
        }))?;
    stream
        .write_all(GUEST_CONTROL_CONNECT_LINE)
        .map_err(io_failure(|kind| GuestControlTransportFailure::WriteIo {
            kind,
        }))?;
    let ack_token = read_ack_token(&mut stream)?;
    stream.set_read_timeout(None).map_err(io_failure(|kind| {
        GuestControlTransportFailure::AckIo { kind }
    }))?;
    stream.set_write_timeout(None).map_err(io_failure(|kind| {
        GuestControlTransportFailure::WriteIo { kind }
    }))?;
    Ok(GuestControlConnectedStream { stream, ack_token })
}

fn validate_socket_path(
    socket_path: &Path,
    state_root: &Path,
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
    let parent = socket_path
        .parent()
        .ok_or(GuestControlTransportFailure::SocketOutsideStateRoot)?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|_| GuestControlTransportFailure::SocketMissing)?;
    if !canonical_parent.starts_with(&canonical_root) {
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
    Ok(())
}

fn has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn read_ack_token(stream: &mut UnixStream) -> Result<String, GuestControlTransportFailure> {
    let mut ack = Vec::with_capacity(MAX_ACK_BYTES);
    let mut byte = [0_u8; 1];
    loop {
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
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
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
        connect_guest_control_vsock(path, root, Duration::from_millis(100))
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
