use d2b_contracts::unsafe_local_wire::HelperSupervisorId;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use nix::sys::stat::{Mode as NixMode, umask};
use std::fmt;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_SOCKET_PATH_BYTES: usize = 107;
static SOCKET_BIND_UMASK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellSocketError {
    RuntimeDirectoryInvalid,
    PathInvalid,
    AlreadyExists,
    BindFailed,
    OwnershipMismatch,
    ConnectFailed,
}

pub(crate) fn validate_runtime_directory(
    directory: &Path,
    expected_uid: u32,
) -> Result<(), ShellSocketError> {
    if !directory.is_absolute() {
        return Err(ShellSocketError::RuntimeDirectoryInvalid);
    }
    let metadata =
        fs::symlink_metadata(directory).map_err(|_| ShellSocketError::RuntimeDirectoryInvalid)?;
    let mode = metadata.permissions().mode() & 0o7777;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != expected_uid
        || mode & 0o700 != 0o700
        || mode & 0o027 != 0
        || mode & 0o7000 != 0
    {
        return Err(ShellSocketError::RuntimeDirectoryInvalid);
    }
    Ok(())
}

pub(crate) fn supervisor_socket_path(
    runtime_directory: &Path,
    supervisor_id: &HelperSupervisorId,
) -> Result<PathBuf, ShellSocketError> {
    let path = runtime_directory.join(format!(".d2b-shell-{}.sock", supervisor_id.as_str()));
    if path.as_os_str().as_bytes().len() > MAX_SOCKET_PATH_BYTES {
        return Err(ShellSocketError::PathInvalid);
    }
    Ok(path)
}

pub(crate) struct OwnedShellListener {
    listener: UnixListener,
    path: PathBuf,
    owner_uid: u32,
    device: u64,
    inode: u64,
}

impl fmt::Debug for OwnedShellListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedShellListener")
            .field("path", &"<redacted>")
            .field("owner_uid", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl OwnedShellListener {
    pub(crate) fn bind(
        runtime_directory: &Path,
        supervisor_id: &HelperSupervisorId,
        expected_uid: u32,
    ) -> Result<Self, ShellSocketError> {
        validate_runtime_directory(runtime_directory, expected_uid)?;
        let path = supervisor_socket_path(runtime_directory, supervisor_id)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(ShellSocketError::AlreadyExists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ShellSocketError::BindFailed),
        }

        let bind_guard = SOCKET_BIND_UMASK
            .lock()
            .map_err(|_| ShellSocketError::BindFailed)?;
        let previous_umask = umask(NixMode::from_bits_truncate(0o177));
        let listener = UnixListener::bind(&path);
        umask(previous_umask);
        drop(bind_guard);
        let listener = listener.map_err(|_| ShellSocketError::BindFailed)?;
        let result = (|| {
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| ShellSocketError::OwnershipMismatch)?;
            if !metadata.file_type().is_socket()
                || metadata.uid() != expected_uid
                || metadata.permissions().mode() & 0o7777 != 0o600
            {
                return Err(ShellSocketError::OwnershipMismatch);
            }
            Ok((metadata.dev(), metadata.ino()))
        })();
        let (device, inode) = match result {
            Ok(identity) => identity,
            Err(error) => {
                remove_if_exact_socket(&path, expected_uid, None);
                return Err(error);
            }
        };
        Ok(Self {
            listener,
            path,
            owner_uid: expected_uid,
            device,
            inode,
        })
    }

    pub(crate) fn listener(&self) -> &UnixListener {
        &self.listener
    }
}

impl Drop for OwnedShellListener {
    fn drop(&mut self) {
        remove_if_exact_socket(&self.path, self.owner_uid, Some((self.device, self.inode)));
    }
}

pub(crate) fn connect_owned_stream(
    runtime_directory: &Path,
    supervisor_id: &HelperSupervisorId,
    expected_uid: u32,
) -> Result<UnixStream, ShellSocketError> {
    validate_runtime_directory(runtime_directory, expected_uid)?;
    let path = supervisor_socket_path(runtime_directory, supervisor_id)?;
    let before = fs::symlink_metadata(&path).map_err(|_| ShellSocketError::ConnectFailed)?;
    if !before.file_type().is_socket()
        || before.uid() != expected_uid
        || before.permissions().mode() & 0o7777 != 0o600
    {
        return Err(ShellSocketError::OwnershipMismatch);
    }
    let stream = UnixStream::connect(&path).map_err(|_| ShellSocketError::ConnectFailed)?;
    let peer =
        getsockopt(&stream, PeerCredentials).map_err(|_| ShellSocketError::OwnershipMismatch)?;
    if peer.uid() as u32 != expected_uid {
        return Err(ShellSocketError::OwnershipMismatch);
    }
    let after = fs::symlink_metadata(&path).map_err(|_| ShellSocketError::OwnershipMismatch)?;
    if !after.file_type().is_socket()
        || after.uid() != expected_uid
        || before.dev() != after.dev()
        || before.ino() != after.ino()
    {
        return Err(ShellSocketError::OwnershipMismatch);
    }
    Ok(stream)
}

fn remove_if_exact_socket(path: &Path, owner_uid: u32, identity: Option<(u64, u64)>) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    let identity_matches = identity
        .map(|(device, inode)| metadata.dev() == device && metadata.ino() == inode)
        .unwrap_or(true);
    if metadata.file_type().is_socket() && metadata.uid() == owner_uid && identity_matches {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::Uid;
    use std::os::fd::{AsFd, AsRawFd};

    fn scratch() -> PathBuf {
        let mut random = [0u8; 4];
        getrandom::getrandom(&mut random).unwrap();
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let root = crate::test_scratch_root();
        let path = root.join(format!(".d2bt-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn listener_is_private_cloexec_and_cleans_only_owned_inode() {
        let directory = scratch();
        let uid = Uid::current().as_raw();
        let id = HelperSupervisorId::new("socket-test").unwrap();
        let path = supervisor_socket_path(&directory, &id).unwrap();
        {
            let owned = OwnedShellListener::bind(&directory, &id, uid).unwrap();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(metadata.file_type().is_socket());
            assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
            assert!(owned.listener().as_fd().as_raw_fd() >= 0);
            assert!(
                rustix::io::fcntl_getfd(owned.listener())
                    .unwrap()
                    .contains(rustix::io::FdFlags::CLOEXEC)
            );
        }
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn runtime_directory_allows_acl_traversal_without_group_write() {
        let directory = scratch();
        let uid = Uid::current().as_raw();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(validate_runtime_directory(&directory, uid), Ok(()));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn runtime_directory_rejects_symlink_and_permissive_mode() {
        let directory = scratch();
        let uid = Uid::current().as_raw();
        for mode in [0o770, 0o755, 0o1750] {
            fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                validate_runtime_directory(&directory, uid),
                Err(ShellSocketError::RuntimeDirectoryInvalid),
                "{mode:o}"
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn supervisor_socket_path_rejects_paths_beyond_sun_path() {
        let directory = PathBuf::from("/").join("r".repeat(MAX_SOCKET_PATH_BYTES));
        let id = HelperSupervisorId::new("path-boundary").unwrap();
        assert_eq!(
            supervisor_socket_path(&directory, &id),
            Err(ShellSocketError::PathInvalid)
        );
    }

    #[test]
    fn bind_never_follows_preexisting_symlink() {
        use std::os::unix::fs::symlink;

        let directory = scratch();
        let uid = Uid::current().as_raw();
        let id = HelperSupervisorId::new("symlink-test").unwrap();
        let path = supervisor_socket_path(&directory, &id).unwrap();
        let target = directory.join("target");
        fs::write(&target, b"unchanged").unwrap();
        symlink(&target, &path).unwrap();
        assert_eq!(
            OwnedShellListener::bind(&directory, &id, uid).unwrap_err(),
            ShellSocketError::AlreadyExists
        );
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cleanup_preserves_replacement_socket_inode() {
        let directory = scratch();
        let uid = Uid::current().as_raw();
        let id = HelperSupervisorId::new("replacement-test").unwrap();
        let path = supervisor_socket_path(&directory, &id).unwrap();
        let owned = OwnedShellListener::bind(&directory, &id, uid).unwrap();
        fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        drop(owned);
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
        drop(replacement);
        fs::remove_file(path).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
