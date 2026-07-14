//! Legacy host-terminal launch helpers.
//!
//! This path is intentionally per-invocation: it creates a randomized
//! Wayland socket under an identity-scoped 0700 directory and points the foreground
//! WezTerm child at a randomized mux socket so an identity-bound terminal never
//! reuses the operator's global WezTerm daemon.

use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
};

use rustix::{
    fd::OwnedFd,
    process::{Pid, PidfdFlags, pidfd_open},
};

const TERMINAL_RUNTIME_DIR: &str = "d2b-wayland-proxy";
const SOCKET_MODE: u32 = 0o600;
const DIR_MODE: u32 = 0o700;
const MAX_UNIX_SOCKET_PATH_BYTES: usize = 107;

#[derive(Debug)]
pub struct TerminalRuntime {
    xdg_runtime_dir: PathBuf,
    root: PathBuf,
    listen_socket: PathBuf,
    mux_socket: PathBuf,
}

impl TerminalRuntime {
    pub fn prepare(identity_component: &str) -> io::Result<Self> {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR is required for host-terminal proxy mode",
            )
        })?;
        Self::prepare_in(Path::new(&runtime_dir), identity_component)
    }

    pub fn prepare_in(runtime_dir: &Path, identity_component: &str) -> io::Result<Self> {
        let token = random_token()?;
        Self::prepare_in_with_token(runtime_dir, identity_component, &token)
    }

    pub fn listen_socket(&self) -> &Path {
        &self.listen_socket
    }

    pub fn mux_socket(&self) -> &Path {
        &self.mux_socket
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.xdg_runtime_dir
    }

    pub fn wayland_display_value(&self) -> OsString {
        self.listen_socket.as_os_str().to_owned()
    }

    fn prepare_in_with_token(
        runtime_dir: &Path,
        identity_component: &str,
        token: &str,
    ) -> io::Result<Self> {
        validate_identity_component(identity_component)?;
        validate_token(token)?;
        ensure_runtime_parent(runtime_dir)?;
        let (root, listen_socket, mux_socket) =
            terminal_runtime_paths(runtime_dir, identity_component, token)?;
        ensure_private_dir(&root)?;

        unlink_stale_socket(&listen_socket)?;
        unlink_stale_socket(&mux_socket)?;

        Ok(Self {
            xdg_runtime_dir: runtime_dir.to_owned(),
            root,
            listen_socket,
            mux_socket,
        })
    }
}

fn terminal_runtime_paths(
    runtime_dir: &Path,
    identity_component: &str,
    token: &str,
) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    let root = runtime_dir
        .join(TERMINAL_RUNTIME_DIR)
        .join(identity_component);
    let listen_socket = root.join(format!("w-{token}"));
    let mux_socket = root.join(format!("m-{token}"));
    for path in [&listen_socket, &mux_socket] {
        if path.as_os_str().as_encoded_bytes().len() > MAX_UNIX_SOCKET_PATH_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal runtime socket path exceeds the Unix socket limit",
            ));
        }
    }
    Ok((root, listen_socket, mux_socket))
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.listen_socket);
        let _ = fs::remove_file(&self.mux_socket);
        let _ = fs::remove_dir(&self.root);
    }
}

pub fn chmod_socket_strict(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "proxy listen path is not a Unix socket",
        ));
    }

    fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_MODE))?;
    let mode = fs::symlink_metadata(path)?.permissions().mode() & 0o777;
    if mode != SOCKET_MODE {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("proxy listen socket mode must be 0600, got {mode:o}"),
        ));
    }
    Ok(())
}

pub fn unlink_stale_socket_path(path: &Path) -> io::Result<()> {
    unlink_stale_socket(path)
}

#[derive(Debug)]
pub struct TerminalChild {
    child: Child,
    pidfd: OwnedFd,
}

impl TerminalChild {
    pub fn spawn(
        program: &OsStr,
        args: &[OsString],
        runtime: &TerminalRuntime,
    ) -> io::Result<Self> {
        let mut command = terminal_command(program, args, runtime);
        let mut child = command.spawn()?;
        let pidfd = pidfd_open(Pid::from_child(&child), PidfdFlags::empty()).map_err(|err| {
            let _ = child.kill();
            let _ = child.wait();
            io::Error::other(format!("pidfd_open failed: {err}"))
        })?;
        Ok(Self { child, pidfd })
    }

    pub fn poll_fd(&self) -> &OwnedFd {
        &self.pidfd
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub fn terminate(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn child_exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(128)
}

pub fn terminal_command(program: &OsStr, args: &[OsString], runtime: &TerminalRuntime) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime.runtime_dir())
        .env("WAYLAND_DISPLAY", runtime.wayland_display_value())
        .env_remove("DISPLAY")
        .env("WEZTERM_UNIX_SOCKET", runtime.mux_socket())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
}

fn ensure_runtime_parent(runtime_dir: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(runtime_dir)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "XDG_RUNTIME_DIR must be a real directory",
        ));
    }
    if metadata.permissions().mode() & 0o007 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "XDG_RUNTIME_DIR must not be world accessible",
        ));
    }
    Ok(())
}

fn ensure_private_dir(path: &Path) -> io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "terminal runtime path exists but is not a directory",
        ));
    }
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(DIR_MODE))?;
    let mode = fs::symlink_metadata(path)?.permissions().mode() & 0o777;
    if mode != DIR_MODE {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("terminal runtime directory mode must be 0700, got {mode:o}"),
        ));
    }
    Ok(())
}

fn unlink_stale_socket(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "refusing to replace non-socket terminal runtime path",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_identity_component(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid identity component for terminal runtime path",
        ));
    }
    Ok(())
}

fn validate_token(token: &str) -> io::Result<()> {
    if token.is_empty()
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid terminal runtime token",
        ));
    }
    Ok(())
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|err| io::Error::other(err.to_string()))?;
    let mut out = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn test_root(name: &str) -> PathBuf {
        let root = PathBuf::from("target")
            .join("d2b-wayland-proxy-terminal-tests")
            .join(format!("{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("chmod root");
        root
    }

    #[test]
    fn terminal_runtime_creates_private_identity_dir_and_random_socket_paths() {
        let root = test_root("private-dir");
        let runtime =
            TerminalRuntime::prepare_in_with_token(&root, "work", "abc123").expect("runtime");

        assert_eq!(
            fs::symlink_metadata(root.join(TERMINAL_RUNTIME_DIR).join("work"))
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            runtime.listen_socket(),
            root.join(TERMINAL_RUNTIME_DIR)
                .join("work")
                .join("w-abc123")
        );
        assert_eq!(
            runtime.mux_socket(),
            root.join(TERMINAL_RUNTIME_DIR)
                .join("work")
                .join("m-abc123")
        );
        drop(runtime);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_runtime_bounds_canonical_target_socket_paths() {
        let (_, listen_socket, mux_socket) = terminal_runtime_paths(
            Path::new("/run/user/1000"),
            "endpoint-fc002cd9909aab17c2232e85",
            "00112233445566778899aabbccddeeff",
        )
        .expect("canonical target paths");
        assert!(listen_socket.as_os_str().as_encoded_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES);
        assert!(mux_socket.as_os_str().as_encoded_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES);
    }

    #[test]
    fn terminal_runtime_rejects_socket_paths_beyond_sun_path() {
        let runtime_dir = PathBuf::from("/").join("r".repeat(MAX_UNIX_SOCKET_PATH_BYTES));
        let error = terminal_runtime_paths(&runtime_dir, "work", "abc123")
            .expect_err("oversized socket path");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn terminal_runtime_unlinks_stale_socket_but_not_regular_file() {
        let root = test_root("stale");
        let first =
            TerminalRuntime::prepare_in_with_token(&root, "work", "abc123").expect("runtime");
        let stale_path = first.listen_socket().to_owned();
        UnixListener::bind(&stale_path).expect("bind stale socket");
        std::mem::forget(first);

        let second =
            TerminalRuntime::prepare_in_with_token(&root, "work", "abc123").expect("reprepare");
        assert!(!second.listen_socket().exists());
        fs::write(second.mux_socket(), b"not a socket").expect("regular file");
        let err = TerminalRuntime::prepare_in_with_token(&root, "work", "abc123")
            .expect_err("regular file refused");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        drop(second);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn chmod_socket_strict_enforces_0600_socket_mode() {
        let root = test_root("socket-mode");
        let path = root.join("wayland.sock");
        let _listener = UnixListener::bind(&path).expect("bind socket");

        chmod_socket_strict(&path).expect("chmod socket");

        assert_eq!(
            fs::symlink_metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_child_env_points_to_proxy_and_per_launch_mux() {
        let root = test_root("env");
        let runtime =
            TerminalRuntime::prepare_in_with_token(&root, "work", "abc123").expect("runtime");
        let command = terminal_command(
            OsStr::new("wezterm"),
            &[
                OsString::from("start"),
                OsString::from("--always-new-process"),
            ],
            &runtime,
        );
        let env = command.get_envs().collect::<Vec<_>>();

        assert!(env.iter().any(|(key, value)| {
            *key == OsStr::new("XDG_RUNTIME_DIR")
                && value == &Some(runtime.runtime_dir().as_os_str())
        }));
        assert!(env.iter().any(|(key, value)| {
            *key == OsStr::new("WAYLAND_DISPLAY")
                && value == &Some(runtime.listen_socket().as_os_str())
        }));
        assert!(
            env.iter()
                .any(|(key, value)| { *key == OsStr::new("DISPLAY") && value.is_none() })
        );
        assert!(env.iter().any(|(key, value)| {
            *key == OsStr::new("WEZTERM_UNIX_SOCKET")
                && value == &Some(runtime.mux_socket().as_os_str())
        }));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("start"), OsStr::new("--always-new-process")]
        );
        drop(runtime);
        let _ = fs::remove_dir_all(root);
    }
}
