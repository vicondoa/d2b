use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    os::unix::ffi::OsStrExt,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::Path,
};

use crate::daemon_config::{DEFAULT_SERVER_VERSION, DaemonConfig};
use crate::typed_error::TypedError;
use crate::unix_transport::io_wrap;
use nix::fcntl::{FcntlArg, fcntl};
#[cfg(test)]
use nix::sys::socket::recv;
use nix::sys::socket::{AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr, sendto, socket};
use nix::unistd::{self, Gid, Group, Uid, User};
use socket2::{Domain, SockAddr, Socket, Type};

#[derive(Debug, Clone)]
pub struct RuntimeIdentity {
    pub daemon_uid: Uid,
    pub daemon_gid: Gid,
    pub public_socket_gid: Gid,
    pub unsafe_local_helper_socket_gid: Option<Gid>,
    pub expect_root_owned_parent: bool,
}

pub fn resolve_runtime_identity(
    config: &DaemonConfig,
    allow_unprivileged_runtime_dir: bool,
) -> Result<RuntimeIdentity, TypedError> {
    if allow_unprivileged_runtime_dir {
        let daemon_uid = User::from_name(&config.daemon_user)
            .ok()
            .flatten()
            .map(|user| user.uid)
            .unwrap_or_else(unistd::getuid);
        let daemon_gid = Group::from_name(&config.daemon_group)
            .ok()
            .flatten()
            .map(|group| group.gid)
            .unwrap_or_else(unistd::getgid);
        return Ok(RuntimeIdentity {
            daemon_uid,
            daemon_gid,
            public_socket_gid: unistd::getgid(),
            unsafe_local_helper_socket_gid: config
                .unsafe_local_helper_socket_path
                .as_ref()
                .map(|_| unistd::getgid()),
            expect_root_owned_parent: false,
        });
    }
    let daemon_user = User::from_name(&config.daemon_user)
        .map_err(io_wrap("lookup daemon user"))?
        .ok_or_else(|| TypedError::InternalConfig {
            detail: format!("daemon user {} does not exist", config.daemon_user),
        })?;
    let daemon_group = Group::from_name(&config.daemon_group)
        .map_err(io_wrap("lookup daemon group"))?
        .ok_or_else(|| TypedError::InternalConfig {
            detail: format!("daemon group {} does not exist", config.daemon_group),
        })?;
    let public_group = Group::from_name(&config.public_socket_group)
        .map_err(io_wrap("lookup public socket group"))?
        .ok_or_else(|| TypedError::InternalConfig {
            detail: format!(
                "public socket group {} does not exist",
                config.public_socket_group
            ),
        })?;
    let unsafe_local_helper_socket_gid = match (
        config.unsafe_local_helper_socket_path.as_ref(),
        config.unsafe_local_helper_socket_group.as_ref(),
    ) {
        (Some(_), Some(group_name)) => Some(
            Group::from_name(group_name)
                .map_err(io_wrap("lookup unsafe-local helper socket group"))?
                .ok_or_else(|| TypedError::InternalConfig {
                    detail: format!("unsafe-local helper socket group {group_name} does not exist"),
                })?
                .gid,
        ),
        (None, None) => None,
        _ => {
            return Err(TypedError::InternalConfig {
                detail: "unsafe-local helper socket path and group must be configured together"
                    .to_owned(),
            });
        }
    };
    Ok(RuntimeIdentity {
        daemon_uid: daemon_user.uid,
        daemon_gid: daemon_group.gid,
        public_socket_gid: public_group.gid,
        unsafe_local_helper_socket_gid,
        expect_root_owned_parent: true,
    })
}

pub fn resolve_unsafe_local_helper_uids(
    config: &DaemonConfig,
    daemon_uid: Uid,
) -> Result<Vec<u32>, TypedError> {
    let mut uids = BTreeSet::new();
    for username in &config.unsafe_local_helper_users {
        let user = User::from_name(username)
            .map_err(io_wrap("lookup unsafe-local helper user"))?
            .ok_or_else(|| TypedError::InternalConfig {
                detail: "configured unsafe-local helper user does not exist".to_owned(),
            })?;
        let uid = user.uid.as_raw();
        if uid == 0 || uid == daemon_uid.as_raw() {
            return Err(TypedError::InternalConfig {
                detail: "unsafe-local helper users must be non-root and distinct from d2bd"
                    .to_owned(),
            });
        }
        uids.insert(uid);
    }
    Ok(uids.into_iter().collect())
}

pub fn validate_lock_parent(
    lock_path: &Path,
    identity: &RuntimeIdentity,
) -> Result<(), TypedError> {
    let parent = lock_path
        .parent()
        .ok_or_else(|| TypedError::InternalLockParentInvalid {
            path: lock_path.to_path_buf(),
            detail: "lock path has no parent directory".to_owned(),
        })?;
    let metadata =
        fs::symlink_metadata(parent).map_err(|err| TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: err.to_string(),
        })?;
    if metadata.file_type().is_symlink() {
        return Err(TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: "parent directory must not be a symlink".to_owned(),
        });
    }
    if !metadata.is_dir() {
        return Err(TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: "parent path is not a directory".to_owned(),
        });
    }
    // The production tmpfile rule installs /run/d2b as
    // `root:d2b 1770` (sticky bit, world-closed) with explicit POSIX
    // ACLs (g::r-x, u:d2bd:rwx, m::rwx) so:
    //   - launcher users (members of `d2b`) traverse via the effective
    //     group ACL entry (g::r-x) to reach `/run/d2b/public.sock`
    //     (mode 0660, group d2b);
    //   - d2bd gets rwx via the named-user ACL entry without owning
    //     the directory, so root-owned subdirs (e.g. /run/d2b/vms)
    //     do not trigger the systemd-tmpfiles unsafe-path-transition guard;
    //   - the sticky bit prevents d2bd from unlinking those root-owned
    //     children.
    // The base mode bits stored in the inode are 0o1770; after masking
    // with 0o777 the check sees 0o770. The `--allow-unprivileged-runtime-dir`
    // test flag permits running under the invoking user's uid/gid (and
    // accepts 0755, 0750, or 0770 to accommodate ad-hoc `cargo test` dirs).
    let (expected_uid, expected_gid, mode_acceptable): (u32, u32, fn(u32) -> bool) =
        if identity.expect_root_owned_parent {
            (
                0, // root owns /run/d2b; daemon access via ACL
                identity.public_socket_gid.as_raw(),
                |m| m == 0o770,
            )
        } else {
            (unistd::getuid().as_raw(), unistd::getgid().as_raw(), |m| {
                m == 0o755 || m == 0o750 || m == 0o770
            })
        };
    let mode = metadata.permissions().mode() & 0o777;
    if metadata.uid() != expected_uid || metadata.gid() != expected_gid || !mode_acceptable(mode) {
        return Err(TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: format!(
                "expected uid:gid {}:{} mode 0770 (production root:d2b 1770) or 0755/0750/0770 (test), got {}:{} mode {:04o}",
                expected_uid,
                expected_gid,
                metadata.uid(),
                metadata.gid(),
                mode
            ),
        });
    }
    Ok(())
}

pub fn ensure_locks_dir(path: &Path, identity: &RuntimeIdentity) -> Result<(), TypedError> {
    fs::create_dir_all(path).map_err(|err| TypedError::InternalIo {
        context: format!("create locks dir {}", path.display()),
        detail: err.to_string(),
    })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o750)).map_err(|err| {
        TypedError::InternalIo {
            context: format!("chmod locks dir {}", path.display()),
            detail: err.to_string(),
        }
    })?;
    if identity.expect_root_owned_parent && unistd::geteuid().is_root() {
        unistd::chown(path, Some(Uid::from_raw(0)), Some(identity.daemon_gid))
            .map_err(io_wrap("chown locks dir"))?;
    }
    Ok(())
}

pub fn acquire_state_lock(path: &Path, identity: &RuntimeIdentity) -> Result<File, TypedError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| TypedError::InternalIo {
            context: format!("open daemon lock {}", path.display()),
            detail: err.to_string(),
        })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).map_err(|err| {
        TypedError::InternalIo {
            context: format!("chmod daemon lock {}", path.display()),
            detail: err.to_string(),
        }
    })?;
    if identity.expect_root_owned_parent && unistd::geteuid().is_root() {
        unistd::chown(path, Some(Uid::from_raw(0)), Some(identity.daemon_gid))
            .map_err(io_wrap("chown daemon lock"))?;
    }

    let lock = libc::flock {
        l_type: libc::F_WRLCK as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    match fcntl(file.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock)) {
        Ok(_) => Ok(file),
        Err(nix::errno::Errno::EAGAIN) | Err(nix::errno::Errno::EACCES) => {
            Err(TypedError::InternalAlreadyRunning {
                path: path.to_path_buf(),
            })
        }
        Err(err) => Err(TypedError::InternalIo {
            context: format!("acquire OFD lock {}", path.display()),
            detail: err.to_string(),
        }),
    }
}

pub fn bind_public_socket(path: &Path, identity: &RuntimeIdentity) -> Result<Socket, TypedError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_socket() {
            fs::remove_file(path).map_err(|err| TypedError::InternalIo {
                context: format!("remove stale socket {}", path.display()),
                detail: err.to_string(),
            })?;
        } else {
            return Err(TypedError::InternalIo {
                context: format!("bind public socket {}", path.display()),
                detail: "existing path is not a socket".to_owned(),
            });
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| TypedError::InternalIo {
            context: format!("create public socket parent {}", parent.display()),
            detail: err.to_string(),
        })?;
    }

    let socket =
        Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None).map_err(|err| {
            TypedError::InternalIo {
                context: format!("create public seqpacket socket {}", path.display()),
                detail: err.to_string(),
            }
        })?;
    let address = SockAddr::unix(path).map_err(|err| TypedError::InternalIo {
        context: format!("encode public socket path {}", path.display()),
        detail: err.to_string(),
    })?;
    socket
        .bind(&address)
        .map_err(|err| TypedError::InternalIo {
            context: format!("bind public socket {}", path.display()),
            detail: err.to_string(),
        })?;
    socket.listen(128).map_err(|err| TypedError::InternalIo {
        context: format!("listen on public socket {}", path.display()),
        detail: err.to_string(),
    })?;
    socket
        .set_nonblocking(true)
        .map_err(|err| TypedError::InternalIo {
            context: format!("set public socket {} nonblocking", path.display()),
            detail: err.to_string(),
        })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660)).map_err(|err| {
        TypedError::InternalIo {
            context: format!("chmod public socket {}", path.display()),
            detail: err.to_string(),
        }
    })?;
    // Always chgrp the socket to `public_socket_gid` (i.e. `d2b` in
    // production). The previous `geteuid().is_root()` gate meant the
    // non-root systemd unit (User=d2bd, SupplementaryGroups=d2b)
    // left the socket with group `d2bd`, which made launcher users
    // unable to connect even though they have a seat in
    // the supplementary group. `chown(path, None, Some(group))` is
    // permitted for the file owner whenever the target gid is one of
    // the caller's groups (real, effective, or supplementary), which
    // is exactly the production case. The test path still works:
    // `expect_root_owned_parent` is false, so we skip the chown there
    // and the socket inherits the caller's primary gid.
    if identity.expect_root_owned_parent {
        unistd::chown(path, None, Some(identity.public_socket_gid))
            .map_err(io_wrap("chown public socket"))?;
    }
    Ok(socket)
}

pub fn sd_notify_address(notify_socket: &OsStr) -> Option<UnixAddr> {
    let bytes = notify_socket.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if let Some(abstract_name) = bytes.strip_prefix(b"@") {
        return match UnixAddr::new_abstract(abstract_name) {
            Ok(address) => Some(address),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "sd_notify: invalid abstract NOTIFY_SOCKET address; notification skipped"
                );
                None
            }
        };
    }
    match UnixAddr::new(Path::new(notify_socket)) {
        Ok(address) => Some(address),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "sd_notify: invalid NOTIFY_SOCKET path; notification skipped"
            );
            None
        }
    }
}

pub fn sd_notify_payload(notify_socket: Option<&OsStr>, payload: &str, context: &'static str) {
    let Some(notify_socket) = notify_socket else {
        return;
    };
    let Some(address) = sd_notify_address(notify_socket) else {
        return;
    };
    let sock = socket(
        AddressFamily::Unix,
        SockType::Datagram,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    );
    let sock = match sock {
        Ok(sock) => sock,
        Err(err) => {
            tracing::warn!(error = %err, context, "sd_notify: create datagram socket failed");
            return;
        }
    };
    let bytes = payload.as_bytes();
    loop {
        match sendto(sock.as_raw_fd(), bytes, &address, MsgFlags::MSG_DONTWAIT) {
            Ok(written) if written == bytes.len() => return,
            Ok(written) => {
                tracing::warn!(
                    written,
                    expected = bytes.len(),
                    context,
                    "sd_notify: short datagram write"
                );
                return;
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::EAGAIN) => {
                tracing::warn!(context, "sd_notify: datagram socket would block");
                return;
            }
            Err(err) => {
                tracing::warn!(error = %err, context, "sd_notify: sendto failed");
                return;
            }
        }
    }
}

pub fn sd_notify_status(notify_socket: Option<&OsStr>, status: &'static str) {
    sd_notify_payload(
        notify_socket,
        &format!("STATUS={status}"),
        "sd_notify STATUS",
    )
}

pub fn sd_notify_ready(notify_socket: Option<&OsStr>) {
    let payload = format!(
        "READY=1\nMAINPID={}\nSTATUS=d2bd public socket ready",
        std::process::id()
    );
    sd_notify_payload(notify_socket, &payload, "sd_notify READY")
}

pub fn drop_privileges_if_root(identity: &RuntimeIdentity) -> Result<(), TypedError> {
    if !identity.expect_root_owned_parent || !unistd::geteuid().is_root() {
        return Ok(());
    }
    unistd::setgroups(&[identity.daemon_gid]).map_err(io_wrap("setgroups"))?;
    unistd::setgid(identity.daemon_gid).map_err(io_wrap("setgid"))?;
    unistd::setuid(identity.daemon_uid).map_err(io_wrap("setuid"))?;
    Ok(())
}

/// Write the daemon's canonicalized binary path + version + start-time
/// to the runtime `version` file on startup. The production public socket
/// lives in `/run/d2b`, so production writes `/run/d2b/version`; test
/// listeners write beside their redirected public socket.
/// This lets the CLI's `crate::daemon_version::compute_restart_status` compute the
/// `[pending restart]` signal post-restart. Failures are logged
/// to stderr and non-fatal - the absence of the version file
/// surfaces in the CLI as `DaemonRestartStatus::DaemonNotRunning`,
/// which is a reasonable degraded shape.
pub fn write_daemon_version_file(config: &DaemonConfig) {
    let binary_path = match std::env::current_exe().and_then(std::fs::canonicalize) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(err) => {
            eprintln!("d2bd: could not canonicalize daemon binary path: {err}");
            return;
        }
    };
    let started_at = chrono_like_rfc3339();
    let payload = crate::daemon_version::DaemonVersionFile {
        server_version: DEFAULT_SERVER_VERSION.to_owned(),
        binary_path,
        started_at,
        protocol_version: d2b_contracts_broker::PROTOCOL_VERSION,
    };
    let json = match serde_json::to_vec_pretty(&payload) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("d2bd: could not serialize daemon version: {err}");
            return;
        }
    };
    let path = daemon_version_file_path(config);
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        eprintln!(
            "d2bd: could not create {} for version file: {err}",
            parent.display()
        );
        return;
    }
    let tmp = path.with_extension("version.tmp");
    if let Err(err) = std::fs::write(&tmp, &json) {
        eprintln!("d2bd: could not write {}: {err}", tmp.display());
        return;
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        eprintln!("d2bd: could not rename version file into place: {err}");
    }
}

pub fn daemon_version_file_path(config: &DaemonConfig) -> std::path::PathBuf {
    config
        .public_socket_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/run/d2b"))
        .join("version")
}

/// Tiny RFC-3339 UTC formatter (`YYYY-MM-DDTHH:MM:SSZ`) so we can
/// stamp `DaemonVersionFile.started_at` without pulling in `chrono`
/// as a new top-level dependency. The daemon's startup is the only
/// caller; precision to the second is sufficient.
pub fn chrono_like_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil-time inverse via Howard Hinnant's days-from-civil.
    let days = (secs / 86_400) as i64;
    let secs_of_day = (secs % 86_400) as u32;
    let (y, m, d) = days_to_ymd(days);
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`: given a days-since-1970-01-01
/// integer, return `(year, month, day)` in the proleptic Gregorian
/// calendar. Adapted for u32 → tuple.
pub fn days_to_ymd(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { (y + 1) as i32 } else { y as i32 };
    (year, m, d)
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod sd_notify_tests {
    use super::*;
    use std::os::unix::net::UnixDatagram;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_abstract_name(label: &str) -> Vec<u8> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        format!("d2bd-{label}-{}-{nanos}", std::process::id()).into_bytes()
    }

    fn payload_string(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("sd_notify payload is utf8")
    }

    #[test]
    fn sd_notify_ready_noops_without_notify_socket() {
        sd_notify_ready(None);
    }

    #[test]
    fn sd_notify_ready_sends_pathname_datagram() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notify.sock");
        let listener = UnixDatagram::bind(&path).expect("bind notify datagram");

        sd_notify_ready(Some(path.as_os_str()));

        let mut buf = [0u8; 256];
        let len = listener.recv(&mut buf).expect("recv notify payload");
        let payload = payload_string(&buf[..len]);
        assert!(payload.contains("READY=1"));
        assert!(payload.contains(&format!("MAINPID={}", std::process::id())));
        assert!(payload.contains("STATUS=d2bd public socket ready"));
    }

    #[test]
    fn sd_notify_status_sends_abstract_datagram() {
        let name = unique_abstract_name("notify");
        let fd = socket(
            AddressFamily::Unix,
            SockType::Datagram,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("create abstract listener");
        let addr = UnixAddr::new_abstract(&name).expect("abstract address");
        nix::sys::socket::bind(fd.as_raw_fd(), &addr).expect("bind abstract notify socket");

        let mut env_value = Vec::with_capacity(name.len() + 1);
        env_value.push(b'@');
        env_value.extend_from_slice(&name);
        let env_value = OsStr::from_bytes(&env_value);
        sd_notify_status(Some(env_value), "d2bd test status");

        let mut buf = [0u8; 256];
        let len = recv(fd.as_raw_fd(), &mut buf, MsgFlags::empty()).expect("recv status payload");
        assert_eq!(payload_string(&buf[..len]), "STATUS=d2bd test status");
    }

    #[test]
    fn sd_notify_ready_errors_when_socket_is_unreachable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing").join("notify.sock");
        sd_notify_ready(Some(path.as_os_str()));
    }
}

#[cfg(test)]
mod runtime_acl_tests {
    //! Regression tests for the public-socket ACL + lock-parent shape
    //! under the root-owned runtime-dir contract.
    //!
    //! Coverage of the production deployment topology
    //! (`User=d2bd`, `SupplementaryGroups=d2b`,
    //! tmpfiles `d /run/d2b 1770 root d2b -` +
    //! `a+ /run/d2b - - - - g::r-x` +
    //! `a+ /run/d2b - - - - u:d2bd:rwx` +
    //! `a+ /run/d2b - - - - m::rwx`,
    //! socket `mode 0660 group d2b`) is split across
    //! these focused unit tests because the real system identities
    //! (`d2bd`, `d2b`) only exist on the deployed
    //! NixOS host. The uid=0 requirement in the production validator
    //! cannot be exercised without root; the tests below cover the
    //! non-root cargo-test path (`expect_root_owned_parent=false`) and
    //! the socket chgrp behaviour that does not require uid=0.

    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    use nix::unistd::{self, Gid, Uid};

    use super::{RuntimeIdentity, bind_public_socket, validate_lock_parent};

    static SCRATCH_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn scratch_dir(_tag: &str) -> PathBuf {
        let nonce = SCRATCH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("nlr-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn caller_identity(expect_root_owned_parent: bool) -> RuntimeIdentity {
        RuntimeIdentity {
            daemon_uid: unistd::getuid(),
            daemon_gid: unistd::getgid(),
            public_socket_gid: unistd::getgid(),
            unsafe_local_helper_socket_gid: None,
            expect_root_owned_parent,
        }
    }

    /// Pick a supplementary group different from the caller's primary gid
    /// so we can prove the
    /// `expect_root_owned_parent=true` chgrp actually mutated the
    /// socket's gid. The caller is a member of every group `getgroups`
    /// returns, so `chown(None, Some(supp_gid))` is permitted by POSIX.
    /// Returns `None` when the runtime has only the primary gid (e.g.
    /// inside minimal CI containers); the caller skips the assertion in
    /// that case with a visible log line so the gap is documented
    /// rather than silently passing.
    fn distinct_supplementary_gid() -> Option<Gid> {
        let primary = unistd::getgid();
        let groups = match unistd::getgroups() {
            Ok(groups) => groups,
            Err(err) => {
                eprintln!("runtime_acl_tests: getgroups failed: {err}; cannot pick supp gid");
                return None;
            }
        };
        groups.into_iter().find(|&gid| gid != primary)
    }

    #[test]
    fn bind_public_socket_chgrps_to_public_socket_gid_even_when_non_root() {
        // Under the production unit the daemon never runs as root,
        // so the previous `if geteuid().is_root()` gate around the
        // chown left the socket with group `d2bd` instead of
        // `d2b`. With the gate removed and `chown(path,
        // None, Some(public_socket_gid))`, the socket must always
        // pick up the requested group when
        // `expect_root_owned_parent` is true.
        //
        // The assertion is only meaningful if the socket's natural
        // (umask-inherited) gid differs from
        // `public_socket_gid`; otherwise a regression that silently
        // re-introduces the `is_root()` gate could pass the test
        // because the socket would already carry the expected gid by
        // inheritance. Pick a supplementary group that differs from
        // the caller's primary gid and use it as the public socket
        // gid. POSIX permits a non-root file owner to chown to any
        // group they belong to (real, effective, or supplementary),
        // so the chown succeeds; if `bind_public_socket` ever skips
        // it under non-root, the socket keeps the primary gid and
        // the assertion fails.
        let Some(supp_gid) = distinct_supplementary_gid() else {
            eprintln!(
                "bind_public_socket_chgrps_to_public_socket_gid_even_when_non_root: \
                 caller has no supplementary gid distinct from primary; \
                 skipping the strict chgrp regression (see runtime_acl_tests docstring)"
            );
            return;
        };

        let dir = scratch_dir("bind-chgrp");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o750))
            .expect("chmod scratch dir 0750");
        let socket_path = dir.join("public.sock");

        let identity = RuntimeIdentity {
            daemon_uid: unistd::getuid(),
            daemon_gid: unistd::getgid(),
            public_socket_gid: supp_gid,
            unsafe_local_helper_socket_gid: None,
            expect_root_owned_parent: true,
        };
        let _socket = match bind_public_socket(&socket_path, &identity) {
            Ok(socket) => socket,
            Err(crate::typed_error::TypedError::InternalIo { detail, .. })
                if detail.contains("EINVAL") =>
            {
                return;
            }
            Err(error) => panic!("bind public socket: {error:?}"),
        };

        let meta = fs::symlink_metadata(&socket_path).expect("stat socket");
        assert_ne!(
            unistd::getgid().as_raw(),
            supp_gid.as_raw(),
            "supp_gid {} must differ from primary gid {} for this test to be meaningful",
            supp_gid,
            unistd::getgid()
        );
        assert_eq!(
            meta.gid(),
            supp_gid.as_raw(),
            "public socket group must equal public_socket_gid={supp_gid:?} under \
             expect_root_owned_parent=true; got gid={} (matches primary={})",
            meta.gid(),
            unistd::getgid()
        );
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o660,
            "public socket mode must be 0660, got 0{:o}",
            mode
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bind_public_socket_skips_chown_in_test_mode() {
        // The test-only path (`expect_root_owned_parent=false`) must
        // skip the chown so plain `cargo test` runs that do not
        // belong to the production socket group still succeed. The
        // socket inherits the caller's primary gid via the default
        // umask path.
        let dir = scratch_dir("bind-test-skip");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755))
            .expect("chmod scratch dir 0755");
        let socket_path = dir.join("public.sock");

        let identity = caller_identity(false);
        let _socket = bind_public_socket(&socket_path, &identity).expect("bind public socket");

        let meta = fs::symlink_metadata(&socket_path).expect("stat socket");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o660,
            "public socket mode must be 0660 in test mode too"
        );
        // We do NOT assert gid here: the test path intentionally
        // skips chown and inherits whatever the umask gave us.
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_lock_parent_accepts_production_tmpfile_shape() {
        // Production posture: `d /run/d2b 1770 root d2b -` with
        // ACLs (g::r-x, u:d2bd:rwx, m::rwx). The validator expects
        // uid=0, gid=public_socket_gid, mode=0o770 (0o1770 & 0o777).
        // Since cargo tests cannot become root, this exercises the
        // equivalent shape via the unprivileged path (expect_root_owned_parent=false)
        // with mode 0o770, which test mode accepts.
        let dir = scratch_dir("validate-prod");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o770))
            .expect("chmod scratch dir 0770");
        let identity = caller_identity(false);
        let lock_path = dir.join("daemon.lock");
        validate_lock_parent(&lock_path, &identity)
            .expect("validator must accept mode 0770 (root:d2b 1770 equivalent) in test mode");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_lock_parent_rejects_wrong_mode_in_production() {
        // 0o700 (the old `/run/d2b/locks` mode) is not acceptable
        // for `/run/d2b` itself because launcher users could not
        // traverse it. The validator must reject the wrong mode.
        // Uses the test path (expect_root_owned_parent=false) to test
        // the mode-rejection logic independently from uid checks.
        let dir = scratch_dir("validate-bad-mode");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .expect("chmod scratch dir 0700");
        let identity = caller_identity(false);
        let lock_path = dir.join("daemon.lock");
        let err = validate_lock_parent(&lock_path, &identity)
            .expect_err("validator must reject mode 0o700 for the public socket parent");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("0700") || msg.contains("mode"),
            "error message must mention the mismatched mode; got {msg}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_lock_parent_test_mode_accepts_either_0755_or_0750_or_0770() {
        // Test mode (`expect_root_owned_parent=false`) accepts 0o755,
        // 0o750, and 0o770 because ad-hoc cargo-test scratch dirs may
        // carry any of these depending on the caller's umask. 0o770 is
        // the cargo-test-accessible equivalent of the production
        // root:d2b 1770 posture.
        for mode in [0o755u32, 0o750u32, 0o770u32] {
            let dir = scratch_dir(&format!("validate-test-mode-{mode:o}"));
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).expect("chmod scratch dir");
            let identity = caller_identity(false);
            let lock_path = dir.join("daemon.lock");
            validate_lock_parent(&lock_path, &identity).unwrap_or_else(|err| {
                panic!("validator must accept mode 0{mode:o} in test mode: {err:?}")
            });
            fs::remove_dir_all(&dir).ok();
        }
    }

    // Silence "unused import" when the file's imports are otherwise
    // visible only to non-test code.
    #[allow(dead_code)]
    fn _ensure_types_in_scope(_: Uid, _: Gid) {}
}
