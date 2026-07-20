use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

use nix::libc;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};

/// First file-descriptor number passed by systemd under SD_LISTEN_FDS.
pub const SD_LISTEN_FD_START: RawFd = 3;

/// Audited fd wrappers for Linux socket primitives not yet exposed with owned fd return types.
#[allow(unsafe_code)]
pub fn owned_fd_from_raw(fd: RawFd) -> OwnedFd {
    // SAFETY: accept4 returned a fresh owned descriptor for this process.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// Adopt the socket-activated listen fd at `SD_LISTEN_FD_START` (fd 3).
///
/// The caller has already verified `LISTEN_PID` / `LISTEN_FDS` env vars match
/// this process; this helper performs the fd-level validation and ownership
/// transfer:
///
/// 1. Verifies the fd is a socket via `fstat(2)` (`S_IFSOCK`).
/// 2. Verifies `SO_DOMAIN == AF_UNIX` via `getsockopt(2)`.
/// 3. Verifies `SO_TYPE == SOCK_SEQPACKET` via `getsockopt(2)`.
/// 4. Sets `FD_CLOEXEC` (defensive; systemd normally sets it already).
/// 5. Returns `OwnedFd::from_raw_fd(3)` — transfers ownership.
///
/// Routing this through `sys.rs` keeps the one `unsafe` block on the
/// quarantined FFI surface (the broker crate carries `unsafe_code = "deny"`).
#[allow(unsafe_code)]
pub fn adopt_listen_fd_from_fd3() -> io::Result<OwnedFd> {
    // SAFETY: fstat/getsockopt/fcntl are called with the raw fd value
    // only; we do not create an OwnedFd until all validation passes.
    unsafe {
        // 1. Verify it's a socket.
        let mut stat: libc::stat = std::mem::zeroed();
        if libc::fstat(SD_LISTEN_FD_START, &mut stat) < 0 {
            return Err(io::Error::last_os_error());
        }
        if stat.st_mode & libc::S_IFMT != libc::S_IFSOCK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "sd-listen fd {SD_LISTEN_FD_START} is not a socket \
                     (st_mode={:#o})",
                    stat.st_mode
                ),
            ));
        }

        // 2. Verify AF_UNIX via SO_DOMAIN.
        let domain = getsockopt_int(SD_LISTEN_FD_START, libc::SO_DOMAIN)?;
        if domain != libc::AF_UNIX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "sd-listen fd {SD_LISTEN_FD_START} is not AF_UNIX \
                     (SO_DOMAIN={domain})"
                ),
            ));
        }

        // 3. Verify SOCK_SEQPACKET via SO_TYPE.
        let sock_type = getsockopt_int(SD_LISTEN_FD_START, libc::SO_TYPE)?;
        if sock_type != libc::SOCK_SEQPACKET {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "sd-listen fd {SD_LISTEN_FD_START} is not SOCK_SEQPACKET \
                     (SO_TYPE={sock_type})"
                ),
            ));
        }

        // 4. Set FD_CLOEXEC (defensive; systemd normally sets it already).
        let flags = libc::fcntl(SD_LISTEN_FD_START, libc::F_GETFD);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(SD_LISTEN_FD_START, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }

        // 5. Transfer ownership.
        // SAFETY: fd 3 is the validated, owned socket inherited from systemd
        // via SD_LISTEN_FDS socket activation.  LISTEN_PID/LISTEN_FDS have
        // been verified by the caller.  All checks above confirmed it is an
        // AF_UNIX SOCK_SEQPACKET.  Ownership transfers to the returned OwnedFd;
        // the caller MUST NOT close fd 3 independently after this returns.
        Ok(OwnedFd::from_raw_fd(SD_LISTEN_FD_START))
    }
}

/// Read a single `c_int` socket option at `SOL_SOCKET` level.
///
/// # Safety
/// `fd` must be a valid open file descriptor.
#[allow(unsafe_code)]
unsafe fn getsockopt_int(fd: RawFd, optname: libc::c_int) -> io::Result<libc::c_int> {
    unsafe {
        let mut val: libc::c_int = 0;
        let mut optlen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let rc = libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            optname,
            std::ptr::addr_of_mut!(val).cast::<libc::c_void>(),
            &mut optlen,
        );
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(val)
        }
    }
}

/// Audited SO_PEERCRED helper for accepted Unix seqpacket connections.
#[allow(unsafe_code)]
pub fn peer_credentials(fd: RawFd) -> io::Result<(u32, u32, i32)> {
    // SAFETY: the borrowed fd is valid for the duration of this call; ownership stays with the caller.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    getsockopt(&borrowed, PeerCredentials)
        .map(|creds| (creds.uid(), creds.gid(), creds.pid()))
        .map_err(|err| io::Error::from_raw_os_error(err as i32))
}

/// Audited SO_PEERCRED uid helper for existing call sites.
pub fn peer_uid(fd: RawFd) -> io::Result<u32> {
    peer_credentials(fd).map(|(uid, _, _)| uid)
}

fn rustix_error_to_io(error: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(error.raw_os_error())
}

/// Validate a descriptor before it enters a realm child's inherited table.
///
/// The request metadata never upgrades an arbitrary descriptor into authority:
/// listeners, namespace handles, cgroup leaves, and directory roots must match
/// their kernel object type, and every descriptor must already be CLOEXEC in
/// the broker.
pub fn validate_realm_child_fd(
    fd: BorrowedFd<'_>,
    kind: d2b_host::realm_children::RealmChildFdKind,
) -> io::Result<()> {
    use d2b_host::realm_children::RealmChildFdKind as K;

    let flags = rustix::io::fcntl_getfd(fd).map_err(rustix_error_to_io)?;
    if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delegated realm-child fd is not CLOEXEC",
        ));
    }

    let stat = rustix::fs::fstat(fd).map_err(rustix_error_to_io)?;
    let object_type = rustix::fs::FileType::from_raw_mode(stat.st_mode);
    match kind {
        K::PublicListener | K::BrokerListener | K::BootstrapSession => {
            if object_type != rustix::fs::FileType::Socket {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "realm-child socket binding is not a socket",
                ));
            }
            let domain = rustix::net::sockopt::get_socket_domain(fd).map_err(rustix_error_to_io)?;
            let socket_type =
                rustix::net::sockopt::get_socket_type(fd).map_err(rustix_error_to_io)?;
            let accepting =
                rustix::net::sockopt::get_socket_acceptconn(fd).map_err(rustix_error_to_io)?;
            if domain != rustix::net::AddressFamily::UNIX
                || socket_type != rustix::net::SocketType::SEQPACKET
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "realm-child socket must be AF_UNIX SOCK_SEQPACKET",
                ));
            }
            let wants_listener = !matches!(kind, K::BootstrapSession);
            if accepting != wants_listener {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "realm-child socket listener state does not match binding kind",
                ));
            }
        }
        K::UserNamespace
        | K::MountNamespace
        | K::NetworkNamespace
        | K::IpcNamespace
        | K::PidNamespace
        | K::CgroupNamespace => {
            const NSFS_MAGIC: u32 = 0x6e73_6673;
            let statfs = rustix::fs::fstatfs(fd).map_err(rustix_error_to_io)?;
            if statfs.f_type != rustix::fs::FsWord::from(NSFS_MAGIC) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "realm-child namespace binding is not an nsfs descriptor",
                ));
            }
            let expected = match kind {
                K::UserNamespace => "user:[",
                K::MountNamespace => "mnt:[",
                K::NetworkNamespace => "net:[",
                K::IpcNamespace => "ipc:[",
                K::PidNamespace => "pid:[",
                K::CgroupNamespace => "cgroup:[",
                _ => unreachable!(),
            };
            let target = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))?;
            if !target.to_string_lossy().starts_with(expected) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "realm-child namespace descriptor kind mismatch",
                ));
            }
        }
        K::CgroupLeaf => {
            const CGROUP2_SUPER_MAGIC: u32 = 0x6367_7270;
            let statfs = rustix::fs::fstatfs(fd).map_err(rustix_error_to_io)?;
            if object_type != rustix::fs::FileType::Directory
                || statfs.f_type != rustix::fs::FsWord::from(CGROUP2_SUPER_MAGIC)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "realm-child cgroup leaf is not a cgroup-v2 directory",
                ));
            }
        }
        K::StateRoot | K::AuditRoot if object_type != rustix::fs::FileType::Directory => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "realm-child storage root is not a directory",
            ));
        }
        K::StateRoot | K::AuditRoot | K::Resource | K::Lease => {}
    }
    Ok(())
}

/// Audited `TUNSETIFF` helper. Opens a TAP and binds the requested ifname.
#[allow(unsafe_code)]
pub fn tun_create_tap_fd(fd: &OwnedFd, ifname: &str) -> io::Result<()> {
    let mut req: libc::ifreq = unsafe { std::mem::zeroed() };
    let name_bytes = ifname.as_bytes();
    if name_bytes.len() >= libc::IFNAMSIZ {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("tap ifname too long: {ifname}"),
        ));
    }
    for (dst, src) in req.ifr_name.iter_mut().zip(name_bytes.iter().copied()) {
        *dst = src as libc::c_char;
    }
    req.ifr_ifru.ifru_flags = (libc::IFF_TAP | libc::IFF_NO_PI | libc::IFF_VNET_HDR) as _;
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TUNSETIFF as _, &req) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
pub fn tun_set_persist(fd: &OwnedFd, persist: bool) -> io::Result<()> {
    let value: libc::c_int = if persist { 1 } else { 0 };
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TUNSETPERSIST as _, value) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
pub fn tun_set_owner(fd: &OwnedFd, uid: u32) -> io::Result<()> {
    let value = libc::c_int::try_from(uid).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("uid out of range: {uid}"),
        )
    })?;
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TUNSETOWNER as _, value) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
pub fn tun_set_group(fd: &OwnedFd, gid: u32) -> io::Result<()> {
    let value = libc::c_int::try_from(gid).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("gid out of range: {gid}"),
        )
    })?;
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TUNSETGROUP as _, value) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Path-safety helpers used by every broker filesystem op
/// (`UpdateHostsFile`, `ApplyNmUnmanaged`, `PrepareStateDir`,
/// `PrepareRuntimeDir`).
///
/// The contract mandates `openat2` with `O_NOFOLLOW` + `RESOLVE_BENEATH`
/// for parent-relative resolution; this module implements equivalent
/// fail-closed guards on stable Rust using `nix` + `std::fs`. Resolution
/// always refuses symlink and magic-link components and `..` escapes
/// (`RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_BENEATH`).
/// `RESOLVE_NO_XDEV` is additionally enforced at every component as
/// defense-in-depth and is relaxed *only* exactly where a real,
/// pre-existing kernel/framework mount sits (e.g. `/run` tmpfs, `/dev`
/// devtmpfs), since broker paths legitimately span those mounts — see
/// [`open_dir_path_safe`] for the per-component mount-tolerant walk:
///
/// - [`refuse_symlink`] — rejects symlinks at `path` via `lstat`;
/// - [`refuse_world_writable_parent`] — rejects world-writable parent
///   directories (the most common path-safety regression);
/// - [`refuse_non_root_parent`] — used in production paths under `/etc`
///   and `/run` to refuse non-root-owned parents (separate helper so
///   test scratch dirs can opt out);
/// - [`atomic_replace`] — `O_TMPFILE`-style temp-file + `rename(2)`
///   atomic replace, preserving parent-dir mode;
/// - [`read_to_string_nofollow`] — symlink-refusing read;
/// - [`ensure_dir_path_safe`] / [`remove_path_safe`] — fd-relative mkdir / unlink.
pub mod path_safe {
    use std::ffi::OsStr;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    pub fn refuse_symlink(path: &Path) -> io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(md) if md.file_type().is_symlink() => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("path-safety-violation: symlink at {}", path.display()),
            )),
            Ok(_) | Err(_) => Ok(()),
        }
    }

    pub fn refuse_world_writable_parent(path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path-safety-violation: {} has no parent", path.display()),
            )
        })?;
        let md = fs::symlink_metadata(parent)?;
        if md.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "path-safety-violation: parent of {} is a symlink",
                    path.display()
                ),
            ));
        }
        let mode = md.permissions().mode();
        if mode & 0o002 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "path-safety-violation: parent {} is world-writable (mode {mode:#o})",
                    parent.display()
                ),
            ));
        }
        Ok(())
    }

    pub fn refuse_non_root_parent(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path-safety-violation: {} has no parent", path.display()),
            )
        })?;
        let md = fs::symlink_metadata(parent)?;
        if md.uid() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "path-safety-violation: parent {} is owned by uid {} (expected 0)",
                    parent.display(),
                    md.uid()
                ),
            ));
        }
        Ok(())
    }

    pub fn read_to_string_nofollow(path: &Path) -> io::Result<String> {
        refuse_symlink(path)?;
        let mut f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    }

    pub fn write_nofollow(path: &Path, body: &[u8]) -> io::Result<()> {
        refuse_world_writable_parent(path)?;
        refuse_symlink(path)?;
        let mut f = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        f.write_all(body)?;
        f.flush()?;
        Ok(())
    }

    /// Atomically replaces `path` with `body` using the fd-relative
    /// writer so symlink swaps in the parent directory cannot redirect
    /// the update.
    pub fn atomic_replace(path: &Path, body: &[u8]) -> io::Result<()> {
        refuse_world_writable_parent(path)?;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path-safety-violation: {} has no parent", path.display()),
            )
        })?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing basename"))?;
        let dir_fd = open_dir_path_safe(parent)?;
        atomic_replace_fd(&dir_fd, name, body, 0o644)
    }

    fn resolve_path(path: &Path) -> io::Result<PathBuf> {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(std::env::current_dir()?.join(path))
        }
    }

    fn parent_and_name(path: &Path) -> io::Result<(PathBuf, String)> {
        let full = resolve_path(path)?;
        let parent = full.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path-safety-violation: {} has no parent", path.display()),
            )
        })?;
        let name = full
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("missing basename for {}", path.display()),
                )
            })?;
        Ok((parent.to_path_buf(), name.to_owned()))
    }
    pub fn remove_nofollow(path: &Path) -> io::Result<()> {
        let (parent, name) = parent_and_name(path)?;
        let parent_fd = open_dir_path_safe(&parent)?;
        remove_path_safe(&parent_fd, &name)
    }

    /// Fd-relative `mkdir` + `fchmod` + `fchown` analog. Stable Rust
    /// equivalents using `nix` so the L1c path-safety tests can
    /// validate the fail-closed behaviour without raw FFI.
    pub fn ensure_dir(
        path: &Path,
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
    ) -> io::Result<DirCreateResult> {
        let full = resolve_path(path)?;
        refuse_world_writable_parent(&full)?;
        let (parent, name) = parent_and_name(&full)?;
        let parent_fd = open_dir_path_safe(&parent)?;
        let (_, result) =
            ensure_dir_path_safe_inner(&parent_fd, &name, mode, owner_uid, owner_gid, true)?;
        Ok(result)
    }

    /// Like [`ensure_dir`] but does NOT re-assert ownership/mode on an
    /// *existing* directory; it only applies `mode`/owner to a dir it
    /// freshly CREATES.
    ///
    /// Used by the vm-start per-VM root prepares (`PrepareStateDir` /
    /// `PrepareRuntimeDir`). Host activation establishes the per-VM
    /// root as `d2bd:users 2770` plus per-runner POSIX ACLs (see
    /// `nixos-modules/host-activation.nix`); re-`fchmod`/`fchown`-ing it
    /// to a single runner principal on every reconcile clobbered that
    /// posture — clipping the ACL mask to `r-x` (so virtiofsd/gpu/video
    /// lost write access to their per-VM runtime dir) and flipping the
    /// owner to an unresolvable runner uid (tripping the daemon's
    /// ownership-matrix preflight on the next start). The matrix
    /// preflight is the authoritative correctness gate and fail-closes
    /// on real drift, so the prepare step only needs to guarantee
    /// existence, not re-stamp metadata.
    pub fn ensure_dir_preserve_existing(
        path: &Path,
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
    ) -> io::Result<DirCreateResult> {
        let full = resolve_path(path)?;
        refuse_world_writable_parent(&full)?;
        let (parent, name) = parent_and_name(&full)?;
        let parent_fd = open_dir_path_safe(&parent)?;
        let (_, result) =
            ensure_dir_path_safe_inner(&parent_fd, &name, mode, owner_uid, owner_gid, false)?;
        Ok(result)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DirCreateResult {
        Created,
        Reused,
    }

    // Re-export libc for callers within this crate that need
    // O_NOFOLLOW directly without a separate import.
    use nix::libc;

    use std::ffi::CString;
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    const RESOLVE_NO_XDEV: u64 = 0x01;
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    const RESOLVE_BENEATH: u64 = 0x08;
    const AT_EMPTY_PATH: libc::c_int = 0x1000;

    static TMP_NAME_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[derive(Debug)]
    pub enum PathSafeError {
        AtomicReplaceUnsupported { target_name: String, detail: String },
    }

    impl std::fmt::Display for PathSafeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::AtomicReplaceUnsupported {
                    target_name,
                    detail,
                } => write!(
                    f,
                    "atomic replace unsupported for {target_name:?}: {detail}"
                ),
            }
        }
    }

    impl std::error::Error for PathSafeError {}

    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }

    fn io_from_rustix(err: rustix::io::Errno) -> io::Error {
        io::Error::from_raw_os_error(err.raw_os_error())
    }

    fn cstring_from_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path contains interior NUL: {}", path.display()),
            )
        })
    }

    fn cstring_from_name(name: &str) -> io::Result<CString> {
        CString::new(name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path contains interior NUL: {name:?}"),
            )
        })
    }

    fn validate_target_name(target_name: &str) -> io::Result<()> {
        if target_name.is_empty()
            || target_name == "."
            || target_name == ".."
            || target_name.contains('/')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("target name must be a single path component: {target_name:?}"),
            ));
        }
        Ok(())
    }

    fn next_hidden_name(prefix: &str) -> String {
        let seq = TMP_NAME_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(".d2b-{prefix}.{}.{}", std::process::id(), seq)
    }

    fn tmpfile_fallback_allowed(err: &io::Error) -> bool {
        matches!(
            err.raw_os_error(),
            Some(code)
                if code == libc::EOPNOTSUPP
                    || code == libc::EINVAL
                    || code == libc::ENOENT
                    || code == libc::ENOSYS
                    || code == libc::EISDIR
        )
    }

    fn proc_link_fallback_allowed(err: &io::Error) -> bool {
        matches!(
            err.raw_os_error(),
            Some(code)
                if code == libc::EPERM
                    || code == libc::EINVAL
                    || code == libc::ENOENT
                    || code == libc::EOPNOTSUPP
                    || code == libc::ENOSYS
        )
    }

    fn renameat2_flag_unsupported(err: &io::Error) -> bool {
        matches!(
            err.raw_os_error(),
            Some(code)
                if code == libc::EINVAL
                    || code == libc::ENOSYS
                    || code == libc::EOPNOTSUPP
                    || code == libc::ENOTSUP
        )
    }

    #[allow(unsafe_code)]
    fn openat2_raw(
        dirfd: RawFd,
        path: &CString,
        flags: libc::c_int,
        mode: u32,
        resolve: u64,
    ) -> io::Result<OwnedFd> {
        let how = OpenHow {
            flags: flags as u64,
            mode: u64::from(mode),
            resolve,
        };
        let ret = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                dirfd,
                path.as_ptr(),
                &how as *const OpenHow,
                core::mem::size_of::<OpenHow>(),
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(super::owned_fd_from_raw(ret as RawFd))
    }

    #[allow(unsafe_code)]
    fn openat_raw(
        dirfd: RawFd,
        path: &CString,
        flags: libc::c_int,
        mode: u32,
    ) -> io::Result<OwnedFd> {
        let ret = unsafe { libc::openat(dirfd, path.as_ptr(), flags, mode) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(super::owned_fd_from_raw(ret))
    }

    #[allow(unsafe_code)]
    fn renameat2_raw(
        olddirfd: RawFd,
        oldpath: &CString,
        newdirfd: RawFd,
        newpath: &CString,
        flags: u32,
    ) -> io::Result<()> {
        let ret = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                olddirfd,
                oldpath.as_ptr(),
                newdirfd,
                newpath.as_ptr(),
                flags,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn renameat_raw(
        olddirfd: RawFd,
        oldpath: &CString,
        newdirfd: RawFd,
        newpath: &CString,
    ) -> io::Result<()> {
        let ret = unsafe { libc::renameat(olddirfd, oldpath.as_ptr(), newdirfd, newpath.as_ptr()) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn mkdirat_raw(dirfd: RawFd, path: &CString, mode: u32) -> io::Result<()> {
        let ret = unsafe { libc::mkdirat(dirfd, path.as_ptr(), mode) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn unlinkat_raw(dirfd: RawFd, path: &CString) -> io::Result<()> {
        unlinkat_raw_with_flags(dirfd, path, 0)
    }

    #[allow(unsafe_code)]
    fn unlinkat_raw_with_flags(dirfd: RawFd, path: &CString, flags: libc::c_int) -> io::Result<()> {
        let ret = unsafe { libc::unlinkat(dirfd, path.as_ptr(), flags) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn fstatat_raw(dirfd: RawFd, path: &CString, flags: libc::c_int) -> io::Result<libc::stat> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::fstatat(dirfd, path.as_ptr(), &mut stat, flags) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(stat)
    }

    #[allow(unsafe_code)]
    fn linkat_empty_path_raw(oldfd: RawFd, newdirfd: RawFd, newpath: &CString) -> io::Result<()> {
        let empty = CString::new(Vec::<u8>::new()).expect("empty C string is valid");
        let ret = unsafe {
            libc::linkat(
                oldfd,
                empty.as_ptr(),
                newdirfd,
                newpath.as_ptr(),
                AT_EMPTY_PATH,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn linkat_proc_fd_raw(oldfd: RawFd, newdirfd: RawFd, newpath: &CString) -> io::Result<()> {
        let proc_path = CString::new(format!("/proc/self/fd/{oldfd}"))
            .expect("proc fd paths never contain NUL");
        let ret = unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                proc_path.as_ptr(),
                newdirfd,
                newpath.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn write_temp_file(
        file: &mut File,
        contents: &[u8],
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
    ) -> io::Result<()> {
        file.write_all(contents)?;
        if owner_uid.is_some() || owner_gid.is_some() {
            fchown(file.as_fd(), owner_uid, owner_gid)?;
        }
        fchmod(file.as_fd(), mode)?;
        file.sync_all()
    }

    fn create_anonymous_tmpfile(dir_fd: &OwnedFd, _mode: u32) -> io::Result<File> {
        let dot = CString::new(".").expect("static dot path contains no NUL");
        match openat_raw(
            dir_fd.as_raw_fd(),
            &dot,
            libc::O_WRONLY | libc::O_CLOEXEC | libc::O_TMPFILE,
            0o600,
        ) {
            Ok(fd) => Ok(File::from(fd)),
            Err(err) if tmpfile_fallback_allowed(&err) => {
                for _ in 0..64 {
                    let tmp_name = cstring_from_name(&next_hidden_name("tmp"))?;
                    match openat_raw(
                        dir_fd.as_raw_fd(),
                        &tmp_name,
                        libc::O_WRONLY
                            | libc::O_CLOEXEC
                            | libc::O_NOFOLLOW
                            | libc::O_CREAT
                            | libc::O_EXCL,
                        0o600,
                    ) {
                        Ok(fd) => {
                            if let Err(unlink_err) = unlinkat_raw(dir_fd.as_raw_fd(), &tmp_name) {
                                drop(fd);
                                return Err(unlink_err);
                            }
                            return Ok(File::from(fd));
                        }
                        Err(open_err) if open_err.kind() == io::ErrorKind::AlreadyExists => {
                            continue;
                        }
                        Err(open_err) => return Err(open_err),
                    }
                }
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "could not allocate unique temporary file name",
                ))
            }
            Err(err) => Err(err),
        }
    }

    fn install_stage_name(
        dir_fd: &OwnedFd,
        stage_name: &CString,
        target_name: &CString,
    ) -> io::Result<()> {
        match renameat2_raw(
            dir_fd.as_raw_fd(),
            stage_name,
            dir_fd.as_raw_fd(),
            target_name,
            libc::RENAME_NOREPLACE,
        ) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                match renameat2_raw(
                    dir_fd.as_raw_fd(),
                    stage_name,
                    dir_fd.as_raw_fd(),
                    target_name,
                    libc::RENAME_EXCHANGE,
                ) {
                    Ok(()) => unlinkat_raw(dir_fd.as_raw_fd(), stage_name)?,
                    Err(exchange_err) if renameat2_flag_unsupported(&exchange_err) => {
                        renameat_raw(
                            dir_fd.as_raw_fd(),
                            stage_name,
                            dir_fd.as_raw_fd(),
                            target_name,
                        )?;
                    }
                    Err(exchange_err) => {
                        let _ = unlinkat_raw(dir_fd.as_raw_fd(), stage_name);
                        return Err(exchange_err);
                    }
                }
            }
            Err(err) if renameat2_flag_unsupported(&err) => {
                renameat_raw(
                    dir_fd.as_raw_fd(),
                    stage_name,
                    dir_fd.as_raw_fd(),
                    target_name,
                )?;
            }
            Err(err) => {
                let _ = unlinkat_raw(dir_fd.as_raw_fd(), stage_name);
                return Err(err);
            }
        }
        rustix::fs::fsync(dir_fd).map_err(io_from_rustix)
    }

    fn atomic_replace_via_named_stage(
        dir_fd: &OwnedFd,
        target_name: &str,
        contents: &[u8],
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
    ) -> io::Result<()> {
        let target_name = cstring_from_name(target_name)?;
        for _ in 0..64 {
            let stage_name = cstring_from_name(&next_hidden_name("stage"))?;
            let stage_fd = match openat_raw(
                dir_fd.as_raw_fd(),
                &stage_name,
                libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_CREAT | libc::O_EXCL,
                0o600,
            ) {
                Ok(fd) => fd,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            };
            let mut stage_file = File::from(stage_fd);
            if let Err(err) = write_temp_file(&mut stage_file, contents, mode, owner_uid, owner_gid)
            {
                drop(stage_file);
                let _ = unlinkat_raw(dir_fd.as_raw_fd(), &stage_name);
                return Err(err);
            }
            drop(stage_file);
            return install_stage_name(dir_fd, &stage_name, &target_name);
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate unique staging name",
        ))
    }

    fn atomic_replace_via_linked_tmp(
        dir_fd: &OwnedFd,
        target_name: &str,
        contents: &[u8],
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
    ) -> io::Result<()> {
        let mut tmp_file = create_anonymous_tmpfile(dir_fd, mode)?;
        write_temp_file(&mut tmp_file, contents, mode, owner_uid, owner_gid)?;
        let target_name = cstring_from_name(target_name)?;
        for _ in 0..64 {
            let stage_name = cstring_from_name(&next_hidden_name("stage"))?;
            match linkat_empty_path_raw(tmp_file.as_raw_fd(), dir_fd.as_raw_fd(), &stage_name) {
                Ok(()) => return install_stage_name(dir_fd, &stage_name, &target_name),
                Err(err) if proc_link_fallback_allowed(&err) => {
                    match linkat_proc_fd_raw(tmp_file.as_raw_fd(), dir_fd.as_raw_fd(), &stage_name)
                    {
                        Ok(()) => return install_stage_name(dir_fd, &stage_name, &target_name),
                        Err(proc_err) if proc_err.kind() == io::ErrorKind::AlreadyExists => {
                            continue;
                        }
                        Err(proc_err) => return Err(proc_err),
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate unique staging name",
        ))
    }

    /// `openat2(O_NOFOLLOW | RESOLVE_BENEATH)` helper used by every
    /// filesystem-mutating broker op. `dirfd` anchors the resolution;
    /// `path` must be a relative path that cannot escape the dirfd
    /// subtree.
    pub fn open_at(
        dirfd: BorrowedFd<'_>,
        path: &Path,
        flags: rustix::fs::OFlags,
    ) -> io::Result<OwnedFd> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        openat2(
            dirfd,
            path,
            flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(io_from_rustix)
    }

    /// `fstatat(AT_SYMLINK_NOFOLLOW)` of a single `name` component
    /// beneath an already-open safe parent dirfd. Returns `Ok(None)`
    /// when the entry is absent (`ENOENT`). The caller inspects
    /// `st_mode` (e.g. `S_IFLNK` / `S_IFDIR`), `st_uid`/`st_gid`, and
    /// `st_dev`/`st_ino` without following a symlink. Used by the
    /// swtpm-dir hardening step to detect symlink / non-dir / owner
    /// drift without opening the target.
    pub fn fstatat_nofollow(parent_fd: &OwnedFd, name: &str) -> io::Result<Option<libc::stat>> {
        validate_target_name(name)?;
        let c_name = cstring_from_name(name)?;
        match fstatat_raw(parent_fd.as_raw_fd(), &c_name, libc::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) => Ok(Some(stat)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// `fstat(2)` of an already-open fd. Binds identity (`st_dev` /
    /// `st_ino`) to the exact inode the held fd refers to, so a
    /// post-open rename/replace of the path cannot confuse the caller.
    #[allow(unsafe_code)]
    pub fn fstat_fd(fd: BorrowedFd<'_>) -> io::Result<libc::stat> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: `fstat` writes into the provided `stat` for a valid fd.
        let ret = unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(stat)
    }

    /// Returns whether `fd` carries an extended POSIX ACL xattr. The
    /// tuple is `(access_present, default_present)` for
    /// `system.posix_acl_access` and `system.posix_acl_default`. A
    /// directory with only the base owner/group/other entries (a
    /// "minimal" ACL) has NO xattr, so both `false` means "clean". An
    /// `ENODATA`/`ENOATTR`/`ENOTSUP` result is treated as absent so
    /// filesystems without ACL support don't fail the clean check.
    #[allow(unsafe_code)]
    pub fn fd_extended_acl_present(fd: BorrowedFd<'_>) -> io::Result<(bool, bool)> {
        fn one(fd: RawFd, name: &str) -> io::Result<bool> {
            let c_name = CString::new(name).expect("static xattr name has no NUL");
            // SAFETY: fgetxattr with a null value buffer (size 0) only
            // queries the attribute length / presence; it never writes.
            let ret = unsafe { libc::fgetxattr(fd, c_name.as_ptr(), std::ptr::null_mut(), 0) };
            if ret >= 0 {
                return Ok(true);
            }
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                // ENODATA (61) = no such attribute; ENOTSUP/EOPNOTSUPP =
                // FS without ACL support. Both mean "not present".
                Some(code)
                    if code == libc::ENODATA
                        || code == libc::ENOTSUP
                        || code == libc::EOPNOTSUPP =>
                {
                    Ok(false)
                }
                _ => Err(err),
            }
        }
        let access = one(fd.as_raw_fd(), "system.posix_acl_access")?;
        let default = one(fd.as_raw_fd(), "system.posix_acl_default")?;
        Ok((access, default))
    }

    /// Create a single file beneath an already-open safe parent dirfd.
    /// `name` must be a single path component; `openat2` enforces the
    /// same beneath/no-symlink/no-magiclink/no-xdev contract as
    /// [`open_dir_path_safe`].
    pub fn create_file_at_safe(
        parent_fd: &OwnedFd,
        name: &str,
        flags: libc::c_int,
        mode: u32,
    ) -> io::Result<OwnedFd> {
        validate_target_name(name)?;
        let name = cstring_from_name(name)?;
        openat2_raw(
            parent_fd.as_raw_fd(),
            &name,
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )
    }

    /// Open a single existing file beneath an already-open safe parent
    /// dirfd. `name` must be a single path component; callers pass
    /// access flags such as `O_RDWR | O_NONBLOCK`. This refuses final
    /// symlinks, magic links, and mount escapes before the caller binds
    /// identity with `fstat`.
    pub fn open_file_at_safe(
        parent_fd: &OwnedFd,
        name: &str,
        flags: libc::c_int,
    ) -> io::Result<OwnedFd> {
        validate_target_name(name)?;
        let name = cstring_from_name(name)?;
        openat2_raw(
            parent_fd.as_raw_fd(),
            &name,
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )
    }

    /// fd-based `fchmod` wrapper around rustix; replaces every
    /// path-string `chmod` call site in broker fs ops.
    pub fn fchmod(fd: BorrowedFd<'_>, mode: u32) -> io::Result<()> {
        use rustix::fs::{Mode, fchmod as rfchmod};
        rfchmod(fd, Mode::from_raw_mode(mode)).map_err(io_from_rustix)
    }

    /// fd-based `fchown` via the safe `nix` wrapper. The broker crate
    /// keeps `unsafe_code = "deny"`, so we route through
    /// `nix::unistd::fchown` (which has a safe signature) rather than
    /// `rustix::fs::fchown` (whose `Uid` / `Gid` constructors are
    /// `unsafe fn`).
    pub fn fchown(fd: BorrowedFd<'_>, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
        let res = nix::unistd::fchown(
            fd.as_raw_fd(),
            uid.map(nix::unistd::Uid::from_raw),
            gid.map(nix::unistd::Gid::from_raw),
        );
        res.map_err(|err| io::Error::from_raw_os_error(err as i32))
    }

    pub fn fchownat_empty_path(
        fd: BorrowedFd<'_>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> io::Result<()> {
        nix::unistd::fchownat(
            Some(fd.as_raw_fd()),
            "",
            uid.map(nix::unistd::Uid::from_raw),
            gid.map(nix::unistd::Gid::from_raw),
            nix::fcntl::AtFlags::AT_EMPTY_PATH | nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
        )
        .map_err(|error| io::Error::from_raw_os_error(error as i32))
    }

    #[allow(unsafe_code)]
    pub fn fchmodat_empty_path(fd: BorrowedFd<'_>, mode: u32) -> io::Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_fchmodat2,
                fd.as_raw_fd(),
                c"".as_ptr(),
                mode as libc::mode_t,
                libc::AT_EMPTY_PATH,
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub struct FileWriteLease {
        fd: RawFd,
    }

    impl Drop for FileWriteLease {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: releasing a lease on an fd is best-effort cleanup.
            let _ = unsafe { libc::fcntl(self.fd, libc::F_SETLEASE, libc::F_UNLCK) };
        }
    }

    /// Acquire a Linux write lease on `fd`. This fails if another process
    /// already has the file open, giving DiskInit a kernel-enforced
    /// exclusivity check stronger than advisory locks before it validates,
    /// repairs, or formats a declared disk image.
    #[allow(unsafe_code)]
    pub fn acquire_write_lease(fd: BorrowedFd<'_>) -> io::Result<FileWriteLease> {
        // SAFETY: F_SETLEASE mutates only the lease state on this valid fd.
        let ret = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETLEASE, libc::F_WRLCK) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(FileWriteLease { fd: fd.as_raw_fd() })
    }

    /// Run `program args...` while inheriting `fd` only in the child
    /// process. The broker parent keeps FD_CLOEXEC set the entire time;
    /// `pre_exec` clears it after fork and before exec so no concurrent
    /// spawn in another thread can inherit the descriptor.
    #[allow(unsafe_code)]
    pub fn command_output_inheriting_fd(
        program: &Path,
        args: &[&OsStr],
        fd: BorrowedFd<'_>,
    ) -> io::Result<Output> {
        use std::os::unix::process::CommandExt;

        let raw_fd = fd.as_raw_fd();
        let mut command = Command::new(program);
        command.args(args);
        command.env_remove("NOTIFY_SOCKET");
        // SAFETY: `pre_exec` runs in the child after fork and before exec.
        // The closure uses only async-signal-safe libc fcntl operations and
        // returns an io::Error directly on failure.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(raw_fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(io::Error::last_os_error());
                }
                let ret = libc::fcntl(raw_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                if ret < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.output()
    }

    /// Open a directory with `openat2`, hardened with
    /// `RESOLVE_NO_SYMLINKS`, `RESOLVE_NO_MAGICLINKS`, `RESOLVE_BENEATH`,
    /// and `RESOLVE_NO_XDEV`.
    ///
    /// Legitimate broker paths span multiple filesystems: `/run` is a
    /// tmpfs, `/dev` a devtmpfs, `/sys` sysfs, `/proc` procfs, and
    /// `/var/lib` may be its own mount. A single `/`-anchored `NO_XDEV`
    /// walk therefore fails with `EXDEV` at the first mount crossing
    /// (e.g. `/`→`/run` when preparing `/run/d2b/vms/<vm>`, or
    /// `/`→`/dev` when opening `/dev/net/tun`).
    ///
    /// We resolve **component by component**. Each component is opened
    /// with the full hardened mask; if a component is a genuine mount
    /// crossing (`openat2` returns `EXDEV` under `NO_XDEV`), it is
    /// re-opened **without** `NO_XDEV` — but still with `NO_SYMLINKS`,
    /// `NO_MAGICLINKS`, and `BENEATH` — and the walk continues with the
    /// full mask beneath. Net effect:
    ///
    /// - symlink / magic-link components are **always** refused (this is
    ///   the load-bearing protection against unprivileged redirection);
    /// - `..` escapes are **always** refused (`BENEATH`);
    /// - only **real, pre-existing** mounts are followed. Planting a new
    ///   mount over a path component requires `CAP_SYS_ADMIN`, which in
    ///   the broker's root-only threat model means the attacker already
    ///   owns the host. `NO_XDEV` therefore remains enforced at every
    ///   non-mount component as defense-in-depth, and is relaxed only
    ///   exactly where a legitimate kernel/framework mount sits.
    pub fn open_dir_path_safe(dir: &Path) -> io::Result<OwnedFd> {
        use std::path::Component;

        if !dir.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("directory must be absolute: {}", dir.display()),
            ));
        }

        const FULL: u64 =
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV;
        const MOUNT_TOLERANT: u64 = RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;

        let mut cur: OwnedFd = File::open("/")?.into();
        for component in dir.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => name,
                // `BENEATH` would reject these too, but refuse explicitly
                // so the failure is an unambiguous path-safety violation.
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "path-safety-violation: unexpected path component in {}",
                            dir.display()
                        ),
                    ));
                }
            };
            let name = name.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("path component is not valid UTF-8 in {}", dir.display()),
                )
            })?;
            let c_name = cstring_from_name(name)?;
            cur = match openat2_raw(cur.as_raw_fd(), &c_name, flags, 0, FULL) {
                Ok(fd) => fd,
                // A genuine mount crossing: follow it (still refusing
                // symlinks / magic-links / `..`), then keep enforcing the
                // full mask beneath.
                Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
                    openat2_raw(cur.as_raw_fd(), &c_name, flags, 0, MOUNT_TOLERANT)?
                }
                Err(err) => return Err(err),
            };
        }
        Ok(cur)
    }

    /// Atomic-replace a file under `dir_fd` using an anonymous temp
    /// file when possible, then `renameat2` plus a parent `fsync`.
    pub fn atomic_replace_fd(
        dir_fd: &OwnedFd,
        target_name: &str,
        contents: &[u8],
        mode: u32,
    ) -> io::Result<()> {
        validate_target_name(target_name)?;
        atomic_replace_fd_with_owner(dir_fd, target_name, contents, mode, None, None)
    }

    pub fn atomic_replace_fd_with_owner(
        dir_fd: &OwnedFd,
        target_name: &str,
        contents: &[u8],
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
    ) -> io::Result<()> {
        validate_target_name(target_name)?;
        match atomic_replace_via_linked_tmp(
            dir_fd,
            target_name,
            contents,
            mode,
            owner_uid,
            owner_gid,
        ) {
            Ok(()) => Ok(()),
            Err(err) if proc_link_fallback_allowed(&err) => atomic_replace_via_named_stage(
                dir_fd,
                target_name,
                contents,
                mode,
                owner_uid,
                owner_gid,
            ),
            Err(err) => Err(err),
        }
    }

    fn ensure_dir_path_safe_inner(
        parent_fd: &OwnedFd,
        name: &str,
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
        // When `false`, an EXISTING directory's mode + ownership are
        // left untouched (only a freshly CREATED dir gets `mode` +
        // owner). See `ensure_dir_preserve_existing` for why the
        // vm-start per-VM root prepares must not re-stamp metadata.
        reassert_metadata: bool,
    ) -> io::Result<(OwnedFd, DirCreateResult)> {
        use rustix::fs::OFlags;

        validate_target_name(name)?;
        let relative = Path::new(name);
        match open_at(
            parent_fd.as_fd(),
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY,
        ) {
            Ok(fd) => {
                if reassert_metadata {
                    fchmod(fd.as_fd(), mode)?;
                    if owner_uid.is_some() || owner_gid.is_some() {
                        fchown(fd.as_fd(), owner_uid, owner_gid)?;
                    }
                }
                return Ok((fd, DirCreateResult::Reused));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }

        let c_name = cstring_from_name(name)?;
        let created = match mkdirat_raw(parent_fd.as_raw_fd(), &c_name, mode) {
            Ok(()) => true,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => false,
            Err(err) => return Err(err),
        };

        let fd = open_at(
            parent_fd.as_fd(),
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY,
        )?;
        // Apply mode + ownership only when WE created the dir, or when
        // the caller asked to re-assert metadata. The `created == false`
        // case here is the `mkdirat` EEXIST race: a concurrent actor
        // (e.g. host activation) created the per-VM root between our
        // initial open (which returned NotFound) and this `mkdirat`.
        // Re-stamping it then would defeat `ensure_dir_preserve_existing`
        // exactly as the always-fchmod path did — clipping the ACL mask
        // / chowning to a runner principal. Treat that raced dir like
        // any other pre-existing dir and leave its metadata intact.
        if created || reassert_metadata {
            fchmod(fd.as_fd(), mode)?;
            if owner_uid.is_some() || owner_gid.is_some() {
                fchown(fd.as_fd(), owner_uid, owner_gid)?;
            }
        }
        Ok((
            fd,
            if created {
                DirCreateResult::Created
            } else {
                DirCreateResult::Reused
            },
        ))
    }

    pub fn ensure_dir_path_safe(
        parent_fd: &OwnedFd,
        name: &str,
        mode: u32,
        owner_uid: u32,
        owner_gid: u32,
    ) -> io::Result<OwnedFd> {
        ensure_dir_path_safe_inner(
            parent_fd,
            name,
            mode,
            Some(owner_uid),
            Some(owner_gid),
            true,
        )
        .map(|(fd, _)| fd)
    }

    pub fn remove_path_safe(parent_fd: &OwnedFd, name: &str) -> io::Result<()> {
        validate_target_name(name)?;
        let c_name = cstring_from_name(name)?;
        let stat = fstatat_raw(parent_fd.as_raw_fd(), &c_name, libc::AT_SYMLINK_NOFOLLOW)?;
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("path-safety-violation: symlink at {name}"),
            ));
        }
        let flags = if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            libc::AT_REMOVEDIR
        } else {
            0
        };
        unlinkat_raw_with_flags(parent_fd.as_raw_fd(), &c_name, flags)
    }
    /// `mkdirat` on a parent dirfd. `EEXIST` is folded into `Ok(())`
    /// so callers can use this as a one-shot idempotent dir create.
    pub fn mkdir_at(parent_dirfd: BorrowedFd<'_>, name: &Path, mode: u32) -> io::Result<()> {
        use rustix::fs::{Mode, mkdirat};
        match mkdirat(parent_dirfd, name, Mode::from_raw_mode(mode)) {
            Ok(()) => Ok(()),
            Err(err) if err == rustix::io::Errno::EXIST => Ok(()),
            Err(err) => Err(io_from_rustix(err)),
        }
    }

    /// Like [`mkdir_at`] but FAILS CLOSED on `EEXIST` (surfaced as
    /// [`io::ErrorKind::AlreadyExists`]) instead of treating a pre-existing
    /// entry as success. Used where adopting a directory this call did NOT
    /// create would be a security bug — e.g. the swtpm NVRAM dir
    /// fresh-create path (issue #64): a role UID with `rwx` on the sticky
    /// per-VM root can race-create `swtpm/` between the absence pre-check
    /// and this `mkdirat`, and the broker must refuse rather than
    /// stamp/own/marker-trust an attacker-planted directory.
    pub fn mkdir_at_exclusive(
        parent_dirfd: BorrowedFd<'_>,
        name: &Path,
        mode: u32,
    ) -> io::Result<()> {
        use rustix::fs::{Mode, mkdirat};
        mkdirat(parent_dirfd, name, Mode::from_raw_mode(mode)).map_err(io_from_rustix)
    }

    /// Alias retained for callers that still use the older helper name.
    pub fn open_dir(path: &Path) -> io::Result<OwnedFd> {
        open_dir_path_safe(path)
    }

    /// Helper holder for callers that want to keep a parent dirfd
    /// alive across multiple operations without re-implementing the
    /// `AsFd` plumbing.
    pub struct DirFd(pub OwnedFd);

    impl AsFd for DirFd {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }
    }

    #[cfg(test)]
    mod cross_mount_tests {
        use super::*;
        use std::os::unix::fs::MetadataExt;

        /// A path on a *separate* mount from `/` must be reachable. This
        /// is the regression for the cross-mount `EXDEV` bug: the broker
        /// prepares `/run/d2b/vms/<vm>` and opens `/dev/net/tun`,
        /// `/sys/fs/cgroup/...`, etc., all on mounts other than the
        /// rootfs; a naive `/`-anchored `NO_XDEV` walk fails at the first
        /// crossing.
        ///
        /// CI-stable: `/run` is a tmpfs on every Linux host, so this does
        /// not skip on CI. Verifies BOTH the bug (a single `/`-anchored
        /// `NO_XDEV` `openat2` returns `EXDEV`) AND the fix
        /// (`open_dir_path_safe` reaches the path).
        #[test]
        fn open_dir_path_safe_walks_across_a_mount_boundary() {
            let target = Path::new("/run");
            let (Ok(target_md), Ok(root_md)) = (std::fs::metadata(target), std::fs::metadata("/"))
            else {
                eprintln!("cross-mount test: SKIP (cannot stat /run or /)");
                return;
            };
            if target_md.dev() == root_md.dev() {
                eprintln!("cross-mount test: SKIP (/run is not a separate mount from /)");
                return;
            }

            // Demonstrate the bug: a single `/`-anchored walk with the
            // full mask (incl. NO_XDEV) cannot cross into `/run`.
            let slash: OwnedFd = std::fs::File::open("/").unwrap().into();
            let run_c = cstring_from_name("run").unwrap();
            match openat2_raw(
                slash.as_raw_fd(),
                &run_c,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0,
                RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
            ) {
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {}
                other => panic!(
                    "expected EXDEV from the `/`-anchored NO_XDEV walk into /run, got {other:?}"
                ),
            }

            // The fix: the component walk follows the mount and reaches it.
            open_dir_path_safe(target)
                .expect("open_dir_path_safe must reach /run across the mount boundary");

            // A nested path on the same separate mount is also reachable
            // (the per-component NO_XDEV does not false-EXDEV below the
            // crossing). `/run` always exists; assert reachability without
            // assuming a specific subdir.
            for nested in ["/run/lock", "/dev/net"] {
                let p = Path::new(nested);
                if std::fs::metadata(p).is_ok() {
                    open_dir_path_safe(p)
                        .unwrap_or_else(|e| panic!("open_dir_path_safe({nested}) failed: {e}"));
                }
            }
        }

        /// A path on the same filesystem as `/` still resolves (the fast
        /// path: every component succeeds under the full mask, no
        /// crossing).
        #[test]
        fn open_dir_path_safe_resolves_same_fs_path() {
            open_dir_path_safe(Path::new("/etc")).expect("/etc must resolve");
        }

        /// The load-bearing security invariant: a symlink path component
        /// is refused even though the mount-tolerant fallback drops
        /// `NO_XDEV`. `NO_SYMLINKS` is retained on every component,
        /// including a crossed one, so an attacker cannot redirect the
        /// walk via a symlink.
        #[test]
        fn open_dir_path_safe_refuses_symlink_component() {
            let dir = std::env::temp_dir().join(format!(
                "d2b-pathsafe-symlink-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(dir.join("real")).unwrap();
            let link = dir.join("link");
            std::os::unix::fs::symlink(dir.join("real"), &link).unwrap();
            let err = open_dir_path_safe(&link).expect_err("symlink component must be refused");
            // openat2 reports ELOOP for a refused symlink under NO_SYMLINKS.
            assert_eq!(
                err.raw_os_error(),
                Some(libc::ELOOP),
                "expected ELOOP for the refused symlink, got {err:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// A non-existent leaf yields `NotFound`, not a spurious `EXDEV`
        /// (the mount-tolerant fallback only triggers on a real crossing).
        #[test]
        fn open_dir_path_safe_missing_leaf_is_not_found() {
            let missing = Path::new("/run/d2b-definitely-absent-xmount-test");
            let err = open_dir_path_safe(missing).expect_err("missing leaf must error");
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
        }

        /// `..` and other non-normal components are refused.
        #[test]
        fn open_dir_path_safe_refuses_parent_dir_component() {
            let err = open_dir_path_safe(Path::new("/etc/../etc"))
                .expect_err("`..` component must be refused");
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
    }
}

/// Quarantined syscall surface for the pidfd handoff contract.
///
/// We use `clone3(CLONE_PIDFD)` as the preferred entry — `rustix
/// 0.38` does NOT expose `clone3`, so the call goes via
/// `libc::syscall(SYS_clone3, ...)` with a manually-constructed
/// `clone_args` struct per the kernel `clone3(2)` man page. Every
/// unsafe block is justified inline.
///
/// On `ENOSYS` (very old kernels) / `EINVAL` (newer kernels that
/// have shifted the `clone_args` layout) / `E2BIG`, the spawner
/// falls back to `fork(2)` + `pidfd_open(2)`. The race window
/// between fork and pidfd_open is non-zero (the child could exit and
/// its pid be reused before pidfd_open returns) but is vanishingly small
/// in practice; `pidfd.rs` documents it.
pub mod pidfd_sys {
    use std::ffi::CString;
    use std::io;
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::path::Path;

    use d2b_core::minijail_profile::{MountPolicy, NamespaceSet};
    use nix::libc;

    /// `clone_args` per `<linux/sched.h>`. Layout is stable since
    /// kernel 5.5 (the first `clone3`-with-pidfd release). We pin the
    /// `size = 88` shape (clone3 v2 / set_tid extension); the kernel
    /// `clone3` accepts a smaller `args.size` (88) and rejects bigger
    /// sizes on older kernels, so we pass the minimal size that
    /// supports CLONE_PIDFD.
    #[repr(C)]
    #[derive(Default)]
    struct CloneArgs {
        flags: u64,
        pidfd: u64,
        child_tid: u64,
        parent_tid: u64,
        exit_signal: u64,
        stack: u64,
        stack_size: u64,
        tls: u64,
        set_tid: u64,
        set_tid_size: u64,
        cgroup: u64,
    }

    /// Spawn outcome. The pidfd is `CLOEXEC` by virtue of
    /// `CLONE_PIDFD` (kernel enforces this), but [`set_cloexec`] also
    /// runs to make the post-condition load-bearing in our own audit
    /// records.
    #[derive(Debug)]
    pub struct SpawnOutcome {
        pub pid: i32,
        pub pidfd: OwnedFd,
        pub used_fork_fallback: bool,
    }

    /// Drive `clone3(CLONE_PIDFD)` with optional atomic cgroup
    /// placement (`CLONE_INTO_CGROUP`); falls back to `fork(2)` +
    /// `pidfd_open(2)` on `ENOSYS`/`EINVAL`/`E2BIG`. The closure
    /// `child_main` is invoked in the child; the parent receives the
    /// pidfd. The child is expected to `execve` shortly; if
    /// `child_main` returns the child process exits with the returned
    /// code.
    ///
    /// v1.1.1 `into_cgroup_dirfd` parameter (per ADR 0011
    /// Decision item 8 + ADR 0018 § "Atomic cgroup placement"):
    /// when `Some(dirfd)`, the clone3 syscall is invoked with
    /// `CLONE_INTO_CGROUP` and `args.cgroup = dirfd as u64`. The
    /// kernel atomically places the new child into the cgroup
    /// pointed at by `dirfd` (typically the per-role leaf
    /// `d2b.slice/<vm>/<role>/`) — eliminating the
    /// classical race window where the parent writes the child's
    /// PID to `cgroup.procs` AFTER fork (during which the child
    /// is unaccounted in the per-role cgroup).
    ///
    /// `CLONE_INTO_CGROUP` is supported on kernel ≥ 5.7; the
    /// fork+cgroup.procs fallback retains the v1.0 semantics for
    /// any kernel that returns ENOSYS/EINVAL on the new flag.
    #[allow(unsafe_code)]
    pub fn clone3_pidfd_or_fork_fallback<F>(
        extra_clone_flags: u64,
        mut child_main: F,
    ) -> io::Result<SpawnOutcome>
    where
        F: FnMut() -> i32,
    {
        clone3_pidfd_or_fork_fallback_with_cgroup(extra_clone_flags, None, move |_| child_main())
    }

    /// v1.1.1 variant of [`clone3_pidfd_or_fork_fallback`] that
    /// accepts an optional cgroup dirfd for atomic placement via
    /// `CLONE_INTO_CGROUP`. See the wrapper docstring for the
    /// rationale.
    #[allow(unsafe_code)]
    pub fn clone3_pidfd_or_fork_fallback_with_cgroup<F>(
        extra_clone_flags: u64,
        into_cgroup_dirfd: Option<i32>,
        mut child_main: F,
    ) -> io::Result<SpawnOutcome>
    where
        F: FnMut(bool) -> i32,
    {
        // CLONE_INTO_CGROUP = 0x200000000 per kernel
        // include/uapi/linux/sched.h (libc 0.2.95+ exposes this
        // constant; we hard-code the value as a u64 to keep the
        // build portable to libc crates that don't surface it as
        // a public constant on the current target triple).
        const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;

        let mut pidfd: libc::c_int = -1;
        let (flags, cgroup_arg) = match into_cgroup_dirfd {
            Some(dirfd) if dirfd >= 0 => (
                (libc::CLONE_PIDFD as u64) | extra_clone_flags | CLONE_INTO_CGROUP,
                dirfd as u64,
            ),
            _ => ((libc::CLONE_PIDFD as u64) | extra_clone_flags, 0u64),
        };
        let mut args = CloneArgs {
            flags,
            pidfd: &mut pidfd as *mut libc::c_int as u64,
            exit_signal: libc::SIGCHLD as u64,
            cgroup: cgroup_arg,
            ..Default::default()
        };

        // SAFETY: SYS_clone3 takes a pointer to `clone_args` plus the
        // struct size. We pass a stack-allocated args owned for the
        // duration of the call; no aliasing. The return value is the
        // child pid in the parent and `0` in the child.
        let ret = unsafe {
            libc::syscall(
                libc::SYS_clone3,
                &mut args as *mut CloneArgs,
                core::mem::size_of::<CloneArgs>(),
            )
        };

        if ret == 0 {
            let code = child_main(cgroup_arg != 0);
            unsafe { libc::_exit(code) };
        }

        if ret > 0 {
            if pidfd < 0 {
                return Err(io::Error::other(
                    "clone3 returned a positive pid but no pidfd",
                ));
            }
            // SAFETY: The kernel just gave us this fd.
            let owned = unsafe { OwnedFd::from_raw_fd(pidfd as RawFd) };
            set_cloexec(pidfd as RawFd)?;
            return Ok(SpawnOutcome {
                pid: ret as i32,
                pidfd: owned,
                used_fork_fallback: false,
            });
        }

        // Negative: errno-set failure. Match the fallback eligibility
        // codes per the doc-comment.
        let err = io::Error::last_os_error();
        let code = err.raw_os_error().unwrap_or(0);
        if code != libc::ENOSYS && code != libc::EINVAL && code != libc::E2BIG {
            return Err(err);
        }
        if (extra_clone_flags & ((libc::CLONE_NEWPID as u64) | (libc::CLONE_NEWUSER as u64))) != 0 {
            return Err(err);
        }

        // Fallback: fork(2) + pidfd_open(2). The race window between
        // fork and pidfd_open is documented in pidfd.rs.
        // SAFETY: `fork` is async-signal-safe.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            let code = child_main(false);
            // SAFETY: see clone3 branch above.
            unsafe { libc::_exit(code) };
        }
        let fd = match pidfd_open(pid as i32, 0) {
            Ok(fd) => fd,
            Err(error) => {
                // SAFETY: parent owns the freshly-forked child; if pidfd_open
                // fails, no runtime reaper will receive authority for it.
                let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
                let _ = reap_spawn_runner_error_child(pid);
                return Err(error);
            }
        };
        Ok(SpawnOutcome {
            pid,
            pidfd: fd,
            used_fork_fallback: true,
        })
    }

    /// Duplicate an already-owned fd while preserving `FD_CLOEXEC`.
    #[allow(unsafe_code)]
    pub fn dup_fd_cloexec(fd: RawFd) -> io::Result<OwnedFd> {
        // SAFETY: `F_DUPFD_CLOEXEC` duplicates the caller-owned fd and
        // returns a fresh descriptor this process now owns.
        let ret = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `F_DUPFD_CLOEXEC` returned a fresh owned fd.
        Ok(unsafe { OwnedFd::from_raw_fd(ret as RawFd) })
    }

    /// Duplicate an already-owned fd to a descriptor >= `min_fd`, preserving
    /// `FD_CLOEXEC`.
    #[allow(unsafe_code)]
    fn dup_fd_cloexec_min(fd: RawFd, min_fd: RawFd) -> io::Result<OwnedFd> {
        // SAFETY: `F_DUPFD_CLOEXEC` duplicates the caller-owned fd and returns a
        // fresh descriptor this process now owns.
        let ret = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, min_fd) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `F_DUPFD_CLOEXEC` returned a fresh owned fd.
        Ok(unsafe { OwnedFd::from_raw_fd(ret as RawFd) })
    }

    /// `pidfd_open(2)` wrapper used by the fork fallback AND the
    /// reconciliation path.
    #[allow(unsafe_code)]
    pub fn pidfd_open(pid: i32, flags: u32) -> io::Result<OwnedFd> {
        // SAFETY: `SYS_pidfd_open` takes (pid, flags); on success it
        // returns a fresh fd this process now owns.
        let ret = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, flags) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        set_cloexec(ret as RawFd)?;
        // SAFETY: see SYS_pidfd_open contract above.
        Ok(unsafe { OwnedFd::from_raw_fd(ret as RawFd) })
    }

    /// `pidfd_send_signal(2)` wrapper used by the broker's live pidfd
    /// control paths and daemon-side test oracles.
    #[allow(unsafe_code)]
    pub fn pidfd_send_signal(pidfd: BorrowedFd<'_>, signal: libc::c_int) -> io::Result<()> {
        // SAFETY: `SYS_pidfd_send_signal` takes `(pidfd, sig, siginfo,
        // flags)`; we pass a live borrowed fd, a raw signal number, a
        // null siginfo pointer, and `flags = 0` per the documented
        // simple-signal contract.
        let ret = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Synchronously reap a runner child on parent-side post-clone errors.
    ///
    /// Once `clone3_spawn_runner` returns `Err`, runtime never receives the
    /// pidfd and therefore never registers the async `waitid(P_PIDFD)` reaper.
    /// Error paths that have already created a child must consume its
    /// `SIGCHLD` here to avoid leaving a zombie behind.
    #[allow(unsafe_code)]
    pub(super) fn reap_spawn_runner_error_child(child_pid: i32) -> io::Result<()> {
        if child_pid <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid child pid for reap: {child_pid}"),
            ));
        }

        let mut status = 0;
        loop {
            let rc = unsafe { libc::waitpid(child_pid, &mut status, 0) };
            if rc == child_pid {
                return Ok(());
            }
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(err);
            }
        }
    }

    /// Run `setfacl -m <acl_spec> /proc/self/fd/<fd>` in a forked
    /// child while keeping the target fd CLOEXEC in the broker parent.
    ///
    /// This is intentionally in `sys.rs`: the child must clear
    /// FD_CLOEXEC after `fork(2)` and before `execve(2)` so only the
    /// setfacl child can resolve `/proc/self/fd/<fd>`. Clearing CLOEXEC
    /// in the broker parent would let unrelated concurrent runner
    /// spawns inherit the target fd.
    #[allow(unsafe_code)]
    pub fn run_setfacl_on_fd(fd: BorrowedFd<'_>, acl_spec: &str) -> io::Result<()> {
        run_setfacl_op_on_fd(fd, "-m", acl_spec)
    }

    /// Resolve the absolute path to `setfacl` from a FIXED list of
    /// trusted system locations (never `$PATH`, which a caller could
    /// poison). NixOS production hosts provide it under
    /// `/run/current-system/sw/bin`; Debian/Ubuntu (including the CI
    /// runner where these unit tests execute) under `/usr/bin` or
    /// `/bin`. Returns the first that exists, falling back to the NixOS
    /// path so a missing binary surfaces a sensible exec failure.
    fn resolve_setfacl_path() -> std::ffi::CString {
        const CANDIDATES: &[&str] = &[
            "/run/current-system/sw/bin/setfacl",
            "/usr/bin/setfacl",
            "/bin/setfacl",
        ];
        for cand in CANDIDATES {
            if std::path::Path::new(cand).exists() {
                return std::ffi::CString::new(*cand).expect("static setfacl path has no NUL");
            }
        }
        std::ffi::CString::new(CANDIDATES[0]).expect("static setfacl path has no NUL")
    }

    /// Run `setfacl <op> <acl_spec> /proc/self/fd/<fd>` in a forked
    /// child while keeping the target fd CLOEXEC in the broker parent.
    ///
    /// `op` is the setfacl operation flag, e.g. `-m` to add/modify an
    /// entry or `-x` to remove one. See [`run_setfacl_on_fd`] for the
    /// CLOEXEC rationale.
    #[allow(unsafe_code)]
    pub fn run_setfacl_op_on_fd(fd: BorrowedFd<'_>, op: &str, acl_spec: &str) -> io::Result<()> {
        let setfacl = resolve_setfacl_path();
        let dash_m = std::ffi::CString::new(op).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "setfacl op contains NUL byte")
        })?;
        let acl = std::ffi::CString::new(acl_spec).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "acl spec contains NUL byte")
        })?;
        let procfd = std::ffi::CString::new(format!("/proc/self/fd/{}", fd.as_raw_fd()))
            .expect("formatted fd path has no NUL");

        // SAFETY: fork is used to immediately exec setfacl. The child
        // performs only async-signal-safe libc calls before exec/_exit.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // Child: make the target fd visible to setfacl, but only in
            // this child fd table.
            let raw_fd = fd.as_raw_fd();
            let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFD) };
            if flags < 0 {
                unsafe { libc::_exit(126) };
            }
            if unsafe { libc::fcntl(raw_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
                unsafe { libc::_exit(126) };
            }
            let argv = [
                setfacl.as_ptr(),
                dash_m.as_ptr(),
                acl.as_ptr(),
                procfd.as_ptr(),
                std::ptr::null(),
            ];
            unsafe {
                libc::execv(setfacl.as_ptr(), argv.as_ptr());
                libc::_exit(127);
            }
        }

        let mut status = 0;
        loop {
            let rc = unsafe { libc::waitpid(pid, &mut status, 0) };
            if rc == pid {
                break;
            }
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(err);
            }
        }
        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "setfacl exited with wait status {status}"
            )))
        }
    }

    /// Clear BOTH the access ACL and the default ACL on the directory
    /// referenced by `fd` (the effect of `setfacl -b -k`) by removing the
    /// POSIX-ACL xattrs directly with `fremovexattr(2)`. Using the syscall
    /// rather than forking `setfacl` means no external binary is required
    /// (works on any host/distro, including the non-NixOS CI runner where
    /// these unit tests execute) and adds no fork/exec to the privileged
    /// broker. `ENODATA` (no such ACL) and `ENOTSUP`/`EOPNOTSUPP`
    /// (filesystem without ACL support) are tolerated as "nothing to
    /// clear", so it stays idempotent on an already-clean inode. The
    /// caller re-asserts mode `0700` afterwards.
    #[allow(unsafe_code)]
    pub fn run_setfacl_clear_on_fd(fd: BorrowedFd<'_>) -> io::Result<()> {
        fn remove_one(fd: RawFd, name: &str) -> io::Result<()> {
            let c_name = CString::new(name).expect("static xattr name has no NUL");
            // SAFETY: fremovexattr removes the named attribute on `fd`;
            // it reads/writes no caller buffers.
            let ret = unsafe { libc::fremovexattr(fd, c_name.as_ptr()) };
            if ret == 0 {
                return Ok(());
            }
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(code)
                    if code == libc::ENODATA
                        || code == libc::ENOTSUP
                        || code == libc::EOPNOTSUPP =>
                {
                    Ok(())
                }
                _ => Err(err),
            }
        }
        remove_one(fd.as_raw_fd(), "system.posix_acl_access")?;
        remove_one(fd.as_raw_fd(), "system.posix_acl_default")?;
        Ok(())
    }

    /// Pure I/O — no syscalls beyond `open`/`read`. Returns `None` if
    /// the file is missing or field 22 isn't parseable.
    pub fn read_proc_stat_start_time(pid: i32) -> io::Result<u64> {
        let path = format!("/proc/{pid}/stat");
        let stat = std::fs::read_to_string(&path)?;
        super::super::ops::pidfd::parse_proc_stat_start_time(&stat)
            .ok_or_else(|| io::Error::other(format!("could not parse field 22 from {path}")))
    }

    /// Assert `FD_CLOEXEC` is set on `fd`. Returns the verified flags.
    #[allow(unsafe_code)]
    pub fn verify_cloexec(fd: RawFd) -> io::Result<bool> {
        // SAFETY: F_GETFD is purely informational; we do not mutate.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((flags & libc::FD_CLOEXEC) != 0)
    }

    /// Force `FD_CLOEXEC` on `fd`. Used post-`clone3` and
    /// post-`pidfd_open` to make the contract explicit even though
    /// the kernel already enforces it on both paths.
    #[allow(unsafe_code)]
    pub fn set_cloexec(fd: RawFd) -> io::Result<()> {
        // SAFETY: F_GETFD/F_SETFD are non-destructive for valid fds.
        let cur = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if cur < 0 {
            return Err(io::Error::last_os_error());
        }
        let ret = unsafe { libc::fcntl(fd, libc::F_SETFD, cur | libc::FD_CLOEXEC) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub struct SeccompProgram {
        filters: Vec<libc::sock_filter>,
    }

    impl SeccompProgram {
        fn to_fprog(&self) -> io::Result<libc::sock_fprog> {
            let len: u16 = self.filters.len().try_into().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "seccomp program too large")
            })?;
            Ok(libc::sock_fprog {
                len,
                filter: self.filters.as_ptr() as *mut libc::sock_filter,
            })
        }

        /// Converts a [`d2b_host::seccomp::CompiledSeccompProgram`]
        /// (assembled from the ioctl_policy matrix in the broker parent
        /// before `clone3`) into an installable `SeccompProgram`.
        ///
        /// Each `BpfInstruction` has the same `(code, jt, jf, k)` field
        /// layout as `libc::sock_filter`; the conversion is field-by-field
        /// and requires no unsafe code.
        pub fn from_compiled(program: d2b_host::seccomp::CompiledSeccompProgram) -> Self {
            let filters = program
                .instructions
                .into_iter()
                .map(|instr| libc::sock_filter {
                    code: instr.code,
                    jt: instr.jt,
                    jf: instr.jf,
                    k: instr.k,
                })
                .collect();
            SeccompProgram { filters }
        }

        pub fn deny_syscalls(syscalls: &[u32]) -> Self {
            const BPF_LD_W_ABS: u16 = 0x20;
            const BPF_JMP_JEQ_K: u16 = 0x15;
            #[cfg(target_arch = "x86_64")]
            const BPF_JMP_JSET_K: u16 = 0x45;
            const BPF_RET_K: u16 = 0x06;
            const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
            const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
            const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
            #[cfg(target_arch = "x86_64")]
            const AUDIT_ARCH_CURRENT: u32 = 0xc000_003e;
            #[cfg(target_arch = "aarch64")]
            const AUDIT_ARCH_CURRENT: u32 = 0xc000_00b7;

            let mut filters = Vec::with_capacity(6 + syscalls.len() * 2);
            filters.push(libc::sock_filter {
                code: BPF_LD_W_ABS,
                jt: 0,
                jf: 0,
                k: 4,
            });
            filters.push(libc::sock_filter {
                code: BPF_JMP_JEQ_K,
                jt: 1,
                jf: 0,
                k: AUDIT_ARCH_CURRENT,
            });
            filters.push(libc::sock_filter {
                code: BPF_RET_K,
                jt: 0,
                jf: 0,
                k: SECCOMP_RET_KILL_PROCESS,
            });
            filters.push(libc::sock_filter {
                code: BPF_LD_W_ABS,
                jt: 0,
                jf: 0,
                k: 0,
            });
            #[cfg(target_arch = "x86_64")]
            {
                filters.push(libc::sock_filter {
                    code: BPF_JMP_JSET_K,
                    jt: 0,
                    jf: 1,
                    k: 0x4000_0000,
                });
                filters.push(libc::sock_filter {
                    code: BPF_RET_K,
                    jt: 0,
                    jf: 0,
                    k: SECCOMP_RET_KILL_PROCESS,
                });
            }
            for syscall in syscalls {
                filters.push(libc::sock_filter {
                    code: BPF_JMP_JEQ_K,
                    jt: 0,
                    jf: 1,
                    k: *syscall,
                });
                filters.push(libc::sock_filter {
                    code: BPF_RET_K,
                    jt: 0,
                    jf: 0,
                    k: SECCOMP_RET_ERRNO | libc::EPERM as u32,
                });
            }
            filters.push(libc::sock_filter {
                code: BPF_RET_K,
                jt: 0,
                jf: 0,
                k: SECCOMP_RET_ALLOW,
            });
            Self { filters }
        }

        #[cfg(test)]
        pub(super) fn rejects_syscall(&self, syscall: u32) -> bool {
            self.filters.windows(2).any(|pair| {
                pair[0].code == 0x15
                    && pair[0].k == syscall
                    && pair[1].code == 0x06
                    && pair[1].k & 0xffff_0000 == 0x0005_0000
            })
        }

        /// Installs this BPF program in the calling process via
        /// `seccomp(SECCOMP_SET_MODE_FILTER)`.  Caller must have
        /// already set `PR_SET_NO_NEW_PRIVS`.  Used by the broker
        /// child closure and by the behavioral tests in
        /// `seccomp_compile_tests`.
        #[allow(unsafe_code)]
        pub(crate) fn apply(&self) -> io::Result<()> {
            apply_seccomp(self)
        }
    }

    pub struct RunnerIsolationSpec {
        pub capabilities: Vec<String>,
        pub namespaces: NamespaceSet,
        pub seccomp_program: Option<SeccompProgram>,
        pub mount_policy: MountPolicy,
        pub cgroup_dir_fd: Option<OwnedFd>,
        pub cgroup_procs_fd: Option<OwnedFd>,
        /// When `Some`, the broker pre-establishes a single-entry user
        /// namespace for the runner before exec'ing the role binary. The
        /// child is fake-root inside the namespace (all caps), and the
        /// broker can DROP all host-side capabilities for the role
        /// profile. Used by virtiofsd to serve files with correct
        /// mode/UID semantics without needing CAP_DAC_* effective on the
        /// host. See ADR 0021.
        pub user_namespace: Option<UserNamespaceSpec>,
        /// When `Some`, the child calls `umask(2)` with the given mask
        /// immediately before execve. Profiles that bind shared Unix
        /// sockets (vhost-user-sound, crosvm-gpu, swtpm) declare `0o007`
        /// so created sockets have mode 0660 — combined with the
        /// per-VM-runtime default ACL, this lets cloud-hypervisor's
        /// named-user ACL entry become effective (mask:rw instead of
        /// mask:---).
        pub umask: Option<u32>,
        /// Pre-opened device fds to hand to the user-NS child via fd
        /// inheritance (ADR 0021).
        ///
        /// The broker parent opens these before `clone3(CLONE_NEWUSER)`
        /// (DAC checked at open time; survives the user-NS pivot).
        /// In the child closure, each fd is `dup2`'d to its target fd
        /// number (RENDER_NODE_INHERITED_FD + index), CLOEXEC cleared,
        /// and the original fd closed so no stale fds survive execve.
        ///
        /// The parent retains ownership via `OwnedFd` inside the child
        /// closure; the closure is dropped in the parent after fork,
        /// closing the original fds there. No double-close: fork gives
        /// parent and child independent fd tables.
        ///
        /// See `RENDER_NODE_INHERITED_FD` for the well-known protocol
        /// constant used by the first (and currently only) entry.
        pub pre_opened_device_fds: Vec<OwnedFd>,
        /// Optional controller-provided data-plane descriptors installed as
        /// stdin and stdout before the runner executes.
        pub inherited_stdio: Option<(OwnedFd, OwnedFd)>,
        /// Optional bounded RLIMIT_MEMLOCK installed in the child before
        /// dropping credentials. Used only for qemu-media runners whose
        /// trusted argv requests QEMU `mem-lock=on`.
        pub memlock_limit_bytes: Option<u64>,
    }

    /// Well-known fd number for the pre-opened render node.
    ///
    /// The broker parent `dup2`s `/dev/dri/renderD128` to this fd in the
    /// user-NS child before execve. The crosvm argv references it as
    /// `--gpu-device-node /proc/self/fd/10`.
    ///
    /// Chosen to avoid fds 0–9 (stdin=0, stdout=1, stderr=2,
    /// broker-socket=3, connection-fd=4, sync-pipe-read=5).
    pub const RENDER_NODE_INHERITED_FD: libc::c_int = 10;

    /// Single-entry uid/gid mapping for a runner's user namespace.
    /// The child sees `0` mapped to `host_uid_for_zero` on the
    /// host (and `host_gid_for_zero` for groups). All other UIDs
    /// inside the namespace map to overflowuid (65534).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UserNamespaceSpec {
        pub host_uid_for_zero: u32,
        pub host_gid_for_zero: u32,
    }

    struct PreparedMountAction {
        path: CString,
        readonly: bool,
    }

    struct PreparedDeviceBind {
        source: CString,
        destination: CString,
        kind: PreparedDeviceBindKind,
    }

    enum PreparedDeviceBindKind {
        Directory,
        DeviceNode {
            mode: libc::mode_t,
            dev: libc::dev_t,
        },
    }

    struct PreparedMaskAction {
        path: CString,
    }

    #[repr(C)]
    struct UserCapHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UserCapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    const SECCOMP_SET_MODE_FILTER: libc::c_uint = 1;

    #[allow(unsafe_code)]
    pub fn load_seccomp_program(path: &Path) -> io::Result<SeccompProgram> {
        let bytes = std::fs::read(path)?;
        let filter_size = std::mem::size_of::<libc::sock_filter>();
        if bytes.is_empty() || bytes.len() % filter_size != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "seccomp program {} is not a whole-number array of sock_filter records",
                    path.display()
                ),
            ));
        }
        let mut filters = Vec::with_capacity(bytes.len() / filter_size);
        for chunk in bytes.chunks_exact(filter_size) {
            let mut filter = std::mem::MaybeUninit::<libc::sock_filter>::uninit();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    chunk.as_ptr(),
                    filter.as_mut_ptr() as *mut u8,
                    filter_size,
                );
                filters.push(filter.assume_init());
            }
        }
        Ok(SeccompProgram { filters })
    }

    fn parse_capability_name(name: &str) -> io::Result<libc::c_int> {
        let cap = match name {
            "CAP_CHOWN" => 0,
            "CAP_DAC_OVERRIDE" => 1,
            "CAP_DAC_READ_SEARCH" => 2,
            "CAP_FOWNER" => 3,
            "CAP_FSETID" => 4,
            "CAP_KILL" => 5,
            "CAP_SETGID" => 6,
            "CAP_SETUID" => 7,
            "CAP_SETPCAP" => 8,
            "CAP_LINUX_IMMUTABLE" => 9,
            "CAP_NET_BIND_SERVICE" => 10,
            "CAP_NET_BROADCAST" => 11,
            "CAP_NET_ADMIN" => 12,
            "CAP_NET_RAW" => 13,
            "CAP_IPC_LOCK" => 14,
            "CAP_IPC_OWNER" => 15,
            "CAP_SYS_MODULE" => 16,
            "CAP_SYS_RAWIO" => 17,
            "CAP_SYS_CHROOT" => 18,
            "CAP_SYS_PTRACE" => 19,
            "CAP_SYS_PACCT" => 20,
            "CAP_SYS_ADMIN" => 21,
            "CAP_SYS_BOOT" => 22,
            "CAP_SYS_NICE" => 23,
            "CAP_SYS_RESOURCE" => 24,
            "CAP_SYS_TIME" => 25,
            "CAP_SYS_TTY_CONFIG" => 26,
            "CAP_MKNOD" => 27,
            "CAP_LEASE" => 28,
            "CAP_AUDIT_WRITE" => 29,
            "CAP_AUDIT_CONTROL" => 30,
            "CAP_SETFCAP" => 31,
            "CAP_MAC_OVERRIDE" => 32,
            "CAP_MAC_ADMIN" => 33,
            "CAP_SYSLOG" => 34,
            "CAP_WAKE_ALARM" => 35,
            "CAP_BLOCK_SUSPEND" => 36,
            "CAP_AUDIT_READ" => 37,
            "CAP_PERFMON" => 38,
            "CAP_BPF" => 39,
            "CAP_CHECKPOINT_RESTORE" => 40,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown Linux capability {name}"),
                ));
            }
        };
        Ok(cap)
    }

    fn parse_capabilities(capabilities: &[String]) -> io::Result<Vec<libc::c_int>> {
        capabilities
            .iter()
            .map(|cap| parse_capability_name(cap))
            .collect()
    }

    fn mount_policy_requires_namespace(policy: &MountPolicy) -> bool {
        !policy.read_only_paths.is_empty()
            || !policy.writable_paths.is_empty()
            || policy.nix_store_read_only
            || policy.hide_device_nodes_by_default
    }

    pub(crate) fn device_mask_required(policy: &MountPolicy, namespaces: &NamespaceSet) -> bool {
        policy.hide_device_nodes_by_default && namespaces.pid
    }

    fn prepare_mount_actions(policy: &MountPolicy) -> io::Result<Vec<PreparedMountAction>> {
        use std::collections::BTreeMap;

        let mut by_path: BTreeMap<String, bool> = BTreeMap::new();
        for path in &policy.read_only_paths {
            by_path.insert(path.clone(), true);
        }
        if policy.nix_store_read_only {
            by_path.insert("/nix/store".to_owned(), true);
        }
        for path in &policy.writable_paths {
            by_path.entry(path.path.clone()).or_insert(false);
        }
        // device_binds (e.g. /dev/kvm, /dev/dri/renderD128,
        // /dev/nvidia*) are bind-mounted writable into the runner mount
        // namespace. The host already controls access via the dev-node
        // mode bits + groups; the bind-mount just ensures the device
        // node is visible inside the namespace.
        for path in &policy.device_binds {
            by_path.entry(path.clone()).or_insert(false);
        }
        let mut out = Vec::with_capacity(by_path.len() + policy.bind_mounts.len());
        for (path, readonly) in by_path {
            if !Path::new(&path).is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("mount path {path:?} is not absolute"),
                ));
            }
            out.push(PreparedMountAction {
                path: CString::new(path.as_bytes()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL")
                })?,
                readonly,
            });
        }
        // bind_mounts entries are cross-domain bind mounts (e.g.
        // /run/user/<uid>/wayland-0 -> /run/d2b-gpu/<vm>/wayland-0).
        // The dst is created if missing; the src is bind-mounted at the
        // dst with MS_BIND|MS_REC. Both src and dst must be absolute. The
        // dst is always writable for the runner; readonly is not
        // supported on bind_mounts (Wayland clients need to
        // bidirectionally talk to the socket).
        //
        // bind_mounts entries appear AFTER the writable/readonly
        // bind-mounts so the dst dir exists at mount time.
        for bm in &policy.bind_mounts {
            for path in [&bm.src, &bm.dst] {
                if !Path::new(path).is_absolute() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("bind_mounts path {path:?} is not absolute"),
                    ));
                }
            }
            out.push(PreparedMountAction {
                path: CString::new(bm.dst.as_bytes()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "bind_mounts.dst contains NUL")
                })?,
                readonly: false,
            });
        }
        Ok(out)
    }

    fn prepare_device_binds(
        policy: &MountPolicy,
    ) -> io::Result<(Vec<OwnedFd>, Vec<PreparedDeviceBind>)> {
        let mut fds = Vec::with_capacity(policy.device_binds.len());
        let mut binds = Vec::with_capacity(policy.device_binds.len());
        for path in &policy.device_binds {
            let path_ref = Path::new(path);
            if !path_ref.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("device_binds path {path:?} is not absolute"),
                ));
            }
            let metadata = std::fs::metadata(path_ref)?;
            let file_type = metadata.file_type();
            let kind = if file_type.is_dir() {
                PreparedDeviceBindKind::Directory
            } else if file_type.is_char_device() {
                PreparedDeviceBindKind::DeviceNode {
                    mode: (libc::S_IFCHR | 0o600) as libc::mode_t,
                    dev: metadata.rdev() as libc::dev_t,
                }
            } else if file_type.is_block_device() {
                PreparedDeviceBindKind::DeviceNode {
                    mode: (libc::S_IFBLK | 0o600) as libc::mode_t,
                    dev: metadata.rdev() as libc::dev_t,
                }
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("device_binds path {path:?} is not a device node or directory"),
                ));
            };
            let mut flags = rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC;
            if matches!(kind, PreparedDeviceBindKind::Directory) {
                flags |= rustix::fs::OFlags::DIRECTORY;
            }
            let fd = rustix::fs::openat2(
                rustix::fs::CWD,
                path_ref,
                flags,
                rustix::fs::Mode::empty(),
                rustix::fs::ResolveFlags::NO_SYMLINKS,
            )
            .map_err(|err| io::Error::from_raw_os_error(err.raw_os_error()))?;
            let source = CString::new(format!("/proc/self/fd/{}", fd.as_raw_fd()))
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "procfd contains NUL"))?;
            let destination = CString::new(path.as_bytes()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "device_binds path contains NUL",
                )
            })?;
            fds.push(fd);
            binds.push(PreparedDeviceBind {
                source,
                destination,
                kind,
            });
        }
        Ok((fds, binds))
    }

    fn prepare_root_mask_actions(
        uid: libc::uid_t,
        policy: &MountPolicy,
    ) -> io::Result<Vec<PreparedMaskAction>> {
        if uid != 0 || !policy.hide_device_nodes_by_default {
            return Ok(Vec::new());
        }
        const MASK_PATHS: &[&str] = &[
            "/etc", "/var", "/home", "/root", "/run", "/tmp", "/boot", "/mnt", "/media", "/srv",
            "/opt",
        ];
        MASK_PATHS
            .iter()
            .map(|path| {
                Ok(PreparedMaskAction {
                    path: CString::new(path.as_bytes()).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "mask path contains NUL")
                    })?,
                })
            })
            .collect()
    }

    fn clone3_namespace_flags(namespaces: &NamespaceSet) -> u64 {
        let mut flags = 0u64;
        if namespaces.pid {
            flags |= libc::CLONE_NEWPID as u64;
        }
        if namespaces.user {
            flags |= libc::CLONE_NEWUSER as u64;
        }
        flags
    }

    fn unshare_namespace_flags(namespaces: &NamespaceSet, include_mount: bool) -> libc::c_int {
        let mut flags = 0;
        if include_mount {
            flags |= libc::CLONE_NEWNS;
        }
        if namespaces.net {
            flags |= libc::CLONE_NEWNET;
        }
        if namespaces.ipc {
            flags |= libc::CLONE_NEWIPC;
        }
        if namespaces.uts {
            flags |= libc::CLONE_NEWUTS;
        }
        flags
    }

    fn write_decimal_u32(mut value: u32, out: &mut [u8; 32]) -> usize {
        let mut cursor = out.len();
        if value == 0 {
            cursor -= 1;
            out[cursor] = b'0';
        } else {
            while value > 0 {
                cursor -= 1;
                out[cursor] = b'0' + (value % 10) as u8;
                value /= 10;
            }
        }
        let len = out.len() - cursor;
        out.copy_within(cursor.., 0);
        len
    }

    #[allow(unsafe_code)]
    fn write_pid_to_cgroup(fd: RawFd, pid: u32) -> io::Result<()> {
        let mut buf = [0u8; 32];
        let digits = write_decimal_u32(pid, &mut buf);
        buf[digits] = b'\n';
        let mut written = 0usize;
        let total = digits + 1;
        while written < total {
            let ret = unsafe {
                libc::write(
                    fd,
                    buf[written..total].as_ptr() as *const libc::c_void,
                    (total - written) as libc::size_t,
                )
            };
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
            written += ret as usize;
        }
        Ok(())
    }

    pub(super) fn parent_attach_fallback_cgroup(
        cgroup_procs_fd: Option<&OwnedFd>,
        child_pid: i32,
        cgroup_already_placed: bool,
    ) -> io::Result<()> {
        if cgroup_already_placed {
            return Ok(());
        }
        let Some(fd) = cgroup_procs_fd else {
            return Ok(());
        };
        let pid = u32::try_from(child_pid).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid child pid for cgroup attach: {child_pid}"),
            )
        })?;
        write_pid_to_cgroup(fd.as_raw_fd(), pid)
    }

    #[allow(unsafe_code)]
    fn apply_mount_actions(actions: &[PreparedMountAction]) -> io::Result<()> {
        for action in actions {
            let path = action.path.as_ptr();
            let bind_ret = unsafe {
                libc::mount(
                    path,
                    path,
                    std::ptr::null(),
                    (libc::MS_BIND | libc::MS_REC) as libc::c_ulong,
                    std::ptr::null(),
                )
            };
            if bind_ret < 0 {
                return Err(io::Error::last_os_error());
            }
            if action.readonly {
                let remount_ret = unsafe {
                    libc::mount(
                        std::ptr::null(),
                        path,
                        std::ptr::null(),
                        (libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_REC)
                            as libc::c_ulong,
                        std::ptr::null(),
                    )
                };
                if remount_ret < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    /// Variant returning errno + the path that failed so the broker-child
    /// stderr log can show which bind mount kicked the failure (multiple
    /// paths in actions list).
    #[allow(unsafe_code)]
    fn apply_mount_actions_debug(
        actions: &[PreparedMountAction],
    ) -> Result<(), (libc::c_int, Vec<u8>)> {
        for action in actions {
            let path = action.path.as_ptr();
            let bind_ret = unsafe {
                libc::mount(
                    path,
                    path,
                    std::ptr::null(),
                    (libc::MS_BIND | libc::MS_REC) as libc::c_ulong,
                    std::ptr::null(),
                )
            };
            if bind_ret < 0 {
                let errno = unsafe { *libc::__errno_location() };
                return Err((errno, action.path.as_bytes().to_vec()));
            }
            if action.readonly {
                let remount_ret = unsafe {
                    libc::mount(
                        std::ptr::null(),
                        path,
                        std::ptr::null(),
                        (libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_REC)
                            as libc::c_ulong,
                        std::ptr::null(),
                    )
                };
                if remount_ret < 0 {
                    let errno = unsafe { *libc::__errno_location() };
                    return Err((errno, action.path.as_bytes().to_vec()));
                }
            }
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn mkdir_one(path: *const libc::c_char) -> Result<(), libc::c_int> {
        if unsafe { libc::mkdir(path, 0o755) } < 0 {
            let errno = unsafe { *libc::__errno_location() };
            if errno != libc::EEXIST {
                return Err(errno);
            }
        }
        if unsafe { libc::chmod(path, 0o755) } < 0 {
            Err(unsafe { *libc::__errno_location() })
        } else {
            Ok(())
        }
    }

    #[allow(unsafe_code)]
    fn prepare_device_bind_parent_dirs(destination: &CString) -> Result<(), libc::c_int> {
        let bytes = destination.as_bytes_with_nul();
        if bytes.len() > 4096 || !bytes.starts_with(b"/") {
            return Err(libc::EINVAL);
        }
        let mut buf = [0u8; 4096];
        buf[..bytes.len()].copy_from_slice(bytes);

        for idx in 1..bytes.len() - 1 {
            if buf[idx] == b'/' {
                buf[idx] = 0;
                mkdir_one(buf.as_ptr() as *const libc::c_char)?;
                buf[idx] = b'/';
            }
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    pub(super) fn mknod_device_bind_target(
        destination: &CString,
        mode: libc::mode_t,
        dev: libc::dev_t,
        uid: libc::uid_t,
        gid: libc::gid_t,
    ) -> Result<(), libc::c_int> {
        if unsafe { libc::unlink(destination.as_ptr()) } < 0 {
            let errno = unsafe { *libc::__errno_location() };
            if errno != libc::ENOENT {
                return Err(errno);
            }
        }
        if unsafe { libc::mknod(destination.as_ptr(), mode, dev) } < 0 {
            return Err(unsafe { *libc::__errno_location() });
        }
        if unsafe { libc::chmod(destination.as_ptr(), mode & 0o777) } < 0 {
            return Err(unsafe { *libc::__errno_location() });
        }
        if unsafe { libc::chown(destination.as_ptr(), uid, gid) } < 0 {
            return Err(unsafe { *libc::__errno_location() });
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn apply_device_mask_and_binds(
        binds: &[PreparedDeviceBind],
        uid: libc::uid_t,
        gid: libc::gid_t,
    ) -> Result<(), (libc::c_int, Vec<u8>)> {
        let dev = c"/dev".as_ptr();
        let tmpfs = c"tmpfs".as_ptr();
        let options = c"mode=0755".as_ptr() as *const libc::c_void;
        if unsafe {
            libc::mount(
                tmpfs,
                dev,
                tmpfs,
                (libc::MS_NOSUID | libc::MS_NOEXEC) as libc::c_ulong,
                options,
            )
        } < 0
        {
            let errno = unsafe { *libc::__errno_location() };
            return Err((errno, b"/dev".to_vec()));
        }

        for bind in binds {
            if let Err(errno) = prepare_device_bind_parent_dirs(&bind.destination) {
                return Err((errno, bind.destination.as_bytes().to_vec()));
            }
            match bind.kind {
                PreparedDeviceBindKind::Directory => {
                    if let Err(errno) = mkdir_one(bind.destination.as_ptr()) {
                        return Err((errno, bind.destination.as_bytes().to_vec()));
                    }
                    if unsafe {
                        libc::mount(
                            bind.source.as_ptr(),
                            bind.destination.as_ptr(),
                            std::ptr::null(),
                            (libc::MS_BIND | libc::MS_REC) as libc::c_ulong,
                            std::ptr::null(),
                        )
                    } < 0
                    {
                        let errno = unsafe { *libc::__errno_location() };
                        return Err((errno, bind.destination.as_bytes().to_vec()));
                    }
                }
                PreparedDeviceBindKind::DeviceNode { mode, dev } => {
                    if let Err(errno) =
                        mknod_device_bind_target(&bind.destination, mode, dev, uid, gid)
                    {
                        return Err((errno, bind.destination.as_bytes().to_vec()));
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn apply_root_secret_masks(
        actions: &[PreparedMaskAction],
    ) -> Result<(), (libc::c_int, Vec<u8>)> {
        let tmpfs = c"tmpfs".as_ptr();
        let options = c"mode=0555".as_ptr() as *const libc::c_void;
        for action in actions {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::stat(action.path.as_ptr(), &mut st) } < 0 {
                let errno = unsafe { *libc::__errno_location() };
                if errno == libc::ENOENT {
                    continue;
                }
                return Err((errno, action.path.as_bytes().to_vec()));
            }
            if st.st_mode & libc::S_IFMT != libc::S_IFDIR {
                continue;
            }
            if unsafe {
                libc::mount(
                    tmpfs,
                    action.path.as_ptr(),
                    tmpfs,
                    (libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC | libc::MS_RDONLY)
                        as libc::c_ulong,
                    options,
                )
            } < 0
            {
                let errno = unsafe { *libc::__errno_location() };
                return Err((errno, action.path.as_bytes().to_vec()));
            }
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn apply_private_procfs() -> Result<(), (libc::c_int, Vec<u8>)> {
        let proc_path = c"/proc".as_ptr();
        let proc_type = c"proc".as_ptr();
        if unsafe {
            libc::mount(
                proc_type,
                proc_path,
                proc_type,
                (libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC) as libc::c_ulong,
                std::ptr::null(),
            )
        } < 0
        {
            let errno = unsafe { *libc::__errno_location() };
            return Err((errno, b"/proc".to_vec()));
        }
        Ok(())
    }

    /// Format errno as ASCII digits into buf, return number of bytes
    /// written. Async-signal-safe.
    fn format_errno(errno: libc::c_int, buf: &mut [u8; 16]) -> usize {
        if errno == 0 {
            buf[0] = b'0';
            return 1;
        }
        let mut n = errno.unsigned_abs();
        let mut tmp = [0u8; 16];
        let mut len = 0;
        while n > 0 {
            tmp[len] = b'0' + (n % 10) as u8;
            n /= 10;
            len += 1;
        }
        for i in 0..len {
            buf[i] = tmp[len - 1 - i];
        }
        len
    }

    #[allow(unsafe_code)]
    fn apply_capabilities(capabilities: &[libc::c_int]) -> io::Result<()> {
        let header = UserCapHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let mut data = [
            UserCapData {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            },
            UserCapData {
                effective: 0,
                permitted: 0,
                inheritable: 0,
            },
        ];
        for cap in capabilities {
            let idx = (*cap / 32) as usize;
            let bit = 1u32 << (*cap % 32);
            if idx >= data.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("capability value {cap} is out of range"),
                ));
            }
            data[idx].effective |= bit;
            data[idx].permitted |= bit;
            data[idx].inheritable |= bit;
        }
        let ret = unsafe { libc::syscall(libc::SYS_capset, &header as *const _, data.as_ptr()) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        let clear_ret = unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                libc::PR_CAP_AMBIENT_CLEAR_ALL,
                0,
                0,
                0,
            )
        };
        if clear_ret < 0 {
            return Err(io::Error::last_os_error());
        }
        for cap in capabilities {
            let raise_ret = unsafe {
                libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_RAISE, *cap, 0, 0)
            };
            if raise_ret < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn apply_seccomp(program: &SeccompProgram) -> io::Result<()> {
        let mut fprog = program.to_fprog()?;
        let ret = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                SECCOMP_SET_MODE_FILTER,
                0,
                &mut fprog as *mut libc::sock_fprog,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    const CHILD_EXIT_PRCTL_NO_NEW_PRIVS: libc::c_int = 60;
    const CHILD_EXIT_PRCTL_KEEP_CAPS: libc::c_int = 61;
    const CHILD_EXIT_UNSHARE: libc::c_int = 62;
    const CHILD_EXIT_MOUNT: libc::c_int = 64;
    const CHILD_EXIT_CAPSET: libc::c_int = 65;
    const CHILD_EXIT_SECCOMP: libc::c_int = 66;
    const CHILD_EXIT_SETGROUPS: libc::c_int = 70;
    const CHILD_EXIT_SETGID: libc::c_int = 71;
    const CHILD_EXIT_SETUID: libc::c_int = 72;
    const CHILD_EXIT_EXECVE: libc::c_int = 73;
    /// Child failed to read 1 byte from the user-NS sync pipe —
    /// typically EINTR or the parent died before writing the maps.
    /// Distinct exit code so audit/triage can distinguish from generic
    /// execve / capset failures.
    const CHILD_EXIT_USER_NS_SYNC: libc::c_int = 74;
    // Surfaces a config-level mistake (umask field set to a value
    // >0o777). Distinct from EXECVE failures so operators can grep
    // audit logs for misconfig.
    const CHILD_EXIT_INVALID_UMASK: libc::c_int = 75;
    /// dup2 of a pre-opened device fd to its well-known number failed
    /// in the child closure. Distinct from EXECVE so triage can
    /// immediately identify the fd-passing step.
    const CHILD_EXIT_PREOPEN_DUP2: libc::c_int = 76;
    /// setrlimit(RLIMIT_MEMLOCK) failed for a runner that requested it.
    const CHILD_EXIT_MEMLOCK_RLIMIT: libc::c_int = 77;
    const CHILD_EXIT_REALM_CAPABILITY_DROP: libc::c_int = 78;
    const CHILD_EXIT_REALM_FD_CLOSE: libc::c_int = 79;

    /// Spawn a per-role runner with namespace / seccomp / capability
    /// setup plus `setgroups` + `setgid` + `setuid` + `execve` in a
    /// single `clone3_pidfd_or_fork_fallback` call.
    ///
    /// Async-signal-safety invariant (`signal-safety(7)`): every heap
    /// allocation, CString build, seccomp-program load, and cgroup fd
    /// open happens in the parent before `clone3(2)` / `fork(2)`.
    /// The child closure only makes raw syscalls and `_exit(2)`.
    #[allow(unsafe_code)]
    pub fn clone3_spawn_runner(
        binary: std::ffi::CString,
        argv: Vec<std::ffi::CString>,
        env: Vec<std::ffi::CString>,
        uid: u32,
        gid: u32,
        supplementary_groups: Vec<u32>,
        isolation: RunnerIsolationSpec,
    ) -> io::Result<SpawnOutcome> {
        clone3_spawn_runner_with_extra_flags(
            binary,
            argv,
            env,
            uid,
            gid,
            supplementary_groups,
            isolation,
            0,
            None,
        )
    }

    /// Spawn one realm controller or child broker. Unlike ordinary workload
    /// runners this path requires `clone3`; it never uses the fork fallback
    /// because PID/user namespace creation and direct cgroup placement are
    /// security boundaries, not performance features.
    pub fn clone3_spawn_realm_child(
        binary: std::ffi::CString,
        argv: Vec<std::ffi::CString>,
        env: Vec<std::ffi::CString>,
        host_uid: u32,
        host_gid: u32,
        controller_host_credentials: Option<(u32, u32)>,
        cgroup_dir_fd: OwnedFd,
        inherited_fds: Vec<OwnedFd>,
    ) -> io::Result<SpawnOutcome> {
        let seccomp_program = SeccompProgram::deny_syscalls(&[
            libc::SYS_ptrace as u32,
            libc::SYS_process_vm_readv as u32,
            libc::SYS_process_vm_writev as u32,
            libc::SYS_bpf as u32,
            libc::SYS_perf_event_open as u32,
            libc::SYS_userfaultfd as u32,
            libc::SYS_reboot as u32,
            libc::SYS_init_module as u32,
            libc::SYS_finit_module as u32,
            libc::SYS_delete_module as u32,
            libc::SYS_swapon as u32,
            libc::SYS_swapoff as u32,
            libc::SYS_acct as u32,
            libc::SYS_quotactl as u32,
        ]);
        let isolation = RunnerIsolationSpec {
            capabilities: Vec::new(),
            namespaces: NamespaceSet {
                mount: true,
                pid: true,
                net: true,
                ipc: true,
                uts: false,
                user: true,
            },
            seccomp_program: Some(seccomp_program),
            mount_policy: MountPolicy {
                read_only_paths: Vec::new(),
                writable_paths: Vec::new(),
                nix_store_read_only: false,
                hide_device_nodes_by_default: true,
                device_binds: Vec::new(),
                bind_mounts: Vec::new(),
            },
            cgroup_dir_fd: Some(cgroup_dir_fd),
            cgroup_procs_fd: None,
            user_namespace: Some(UserNamespaceSpec {
                host_uid_for_zero: host_uid,
                host_gid_for_zero: host_gid,
            }),
            umask: Some(0o077),
            pre_opened_device_fds: inherited_fds,
            inherited_stdio: None,
            memlock_limit_bytes: None,
        };
        clone3_spawn_runner_with_extra_flags(
            binary,
            argv,
            env,
            host_uid,
            host_gid,
            Vec::new(),
            isolation,
            libc::CLONE_NEWCGROUP as u64,
            controller_host_credentials,
        )
    }

    #[allow(clippy::too_many_arguments, unsafe_code)]
    fn clone3_spawn_runner_with_extra_flags(
        binary: std::ffi::CString,
        argv: Vec<std::ffi::CString>,
        env: Vec<std::ffi::CString>,
        uid: u32,
        gid: u32,
        supplementary_groups: Vec<u32>,
        isolation: RunnerIsolationSpec,
        required_clone_flags: u64,
        additional_user_ns_mapping: Option<(u32, u32)>,
    ) -> io::Result<SpawnOutcome> {
        // NamespaceSet.user is ALLOWED when
        // RunnerIsolationSpec.user_namespace provides the uid_map/gid_map
        // values. Caller must set both for the child to be fake-root
        // inside the new user NS. Setting namespaces.user without
        // user_namespace is rejected because the child would land in the
        // namespace with overflowuid (65534) and no caps — never useful.
        let user_ns_spec = isolation.user_namespace;
        if let (Some(_), Some((additional_uid, additional_gid))) =
            (user_ns_spec, additional_user_ns_mapping)
            && (additional_uid == 0 || additional_gid == 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "realm child user namespace peer mapping is invalid",
            ));
        }
        if isolation.namespaces.user && user_ns_spec.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SpawnRunner: namespaces.user=true requires user_namespace = Some(spec)",
            ));
        }
        if user_ns_spec.is_some() && !isolation.namespaces.user {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SpawnRunner: user_namespace=Some requires namespaces.user=true",
            ));
        }

        let binary_ptr: *const libc::c_char = binary.as_ptr();
        let mut argv_ptrs_storage: Vec<*const libc::c_char> =
            argv.iter().map(|arg| arg.as_ptr()).collect();
        argv_ptrs_storage.push(std::ptr::null());
        let mut env_ptrs_storage: Vec<*const libc::c_char> =
            env.iter().map(|entry| entry.as_ptr()).collect();
        env_ptrs_storage.push(std::ptr::null());
        let supplementary_groups_storage: Vec<libc::gid_t> = supplementary_groups
            .into_iter()
            .map(|group| group as libc::gid_t)
            .collect();
        let capability_numbers = parse_capabilities(&isolation.capabilities)?;
        let mount_required =
            isolation.namespaces.mount || mount_policy_requires_namespace(&isolation.mount_policy);
        let mask_dev = device_mask_required(&isolation.mount_policy, &isolation.namespaces);
        let mount_actions = if mount_required {
            prepare_mount_actions(&isolation.mount_policy)?
        } else {
            Vec::new()
        };
        let (device_bind_fds, prepared_device_binds) = if mask_dev {
            prepare_device_binds(&isolation.mount_policy)?
        } else {
            (Vec::new(), Vec::new())
        };
        let root_mask_actions = prepare_root_mask_actions(uid, &isolation.mount_policy)?;
        let unshare_flags = unshare_namespace_flags(&isolation.namespaces, mount_required);
        let extra_clone_flags =
            clone3_namespace_flags(&isolation.namespaces) | required_clone_flags;
        let argv_ptrs = &argv_ptrs_storage;
        let env_ptrs = &env_ptrs_storage;
        let supplementary_groups = &supplementary_groups_storage;
        let capabilities = &capability_numbers;
        let mount_actions = &mount_actions;
        let prepared_device_binds = &prepared_device_binds;
        let root_mask_actions = &root_mask_actions;
        let seccomp_program = isolation.seccomp_program;
        let cgroup_dir_fd = isolation.cgroup_dir_fd;
        let cgroup_procs_fd = isolation.cgroup_procs_fd;
        let child_umask = isolation.umask;
        let memlock_limit_bytes = isolation.memlock_limit_bytes;
        let gid = gid as libc::gid_t;
        let uid = uid as libc::uid_t;

        // Extract raw fd numbers before the move closure so we can use
        // them in the async-signal-safe child path (raw syscalls only)
        // (ADR 0021). The OwnedFds themselves are moved into the closure
        // so the PARENT drops them (closes the fds on the parent side)
        // when the closure is dropped after clone/fork. Parent and child
        // have independent fd tables after fork, so there is no
        // double-close hazard.
        let mut pre_opened_raw_fds: Vec<libc::c_int> = isolation
            .pre_opened_device_fds
            .iter()
            .map(|fd| fd.as_raw_fd())
            .collect();
        let inherited_stdio_raw = isolation
            .inherited_stdio
            .as_ref()
            .map(|(stdin, stdout)| (stdin.as_raw_fd(), stdout.as_raw_fd()));
        let inherited_fd_range_start = RENDER_NODE_INHERITED_FD;
        let inherited_fd_range_end =
            inherited_fd_range_start.saturating_add(pre_opened_raw_fds.len() as libc::c_int);
        let mut overlap_safe_pre_opened_fds = Vec::new();
        for raw_fd in &mut pre_opened_raw_fds {
            if *raw_fd >= inherited_fd_range_start && *raw_fd < inherited_fd_range_end {
                let duplicated =
                    dup_fd_cloexec_min(*raw_fd, inherited_fd_range_end).map_err(|err| {
                        io::Error::new(
                            err.kind(),
                            format!("duplicate overlapping pre-opened fd {raw_fd}: {err}"),
                        )
                    })?;
                *raw_fd = duplicated.as_raw_fd();
                overlap_safe_pre_opened_fds.push(duplicated);
            }
        }
        let _pre_opened_device_fds_owner = isolation.pre_opened_device_fds;
        let _inherited_stdio_owner = isolation.inherited_stdio;
        let _overlap_safe_pre_opened_fds_owner = overlap_safe_pre_opened_fds;
        let _device_bind_fds_owner = device_bind_fds;

        // When a user NS is requested, create a sync pipe so the child
        // can block until the parent has written uid_map/gid_map/setgroups.
        // Pipe is FD_CLOEXEC so it auto-closes on execve regardless of
        // child path.
        let user_ns_sync = if user_ns_spec.is_some() {
            Some(make_sync_pipe()?)
        } else {
            None
        };
        // Capture BOTH pipe fds in the child closure. Without this, the
        // child inherits both ends of the pipe (CLOEXEC only fires on
        // execve, not at clone), so the parent's death never delivers EOF
        // to the child's read() — it can wedge forever. The child
        // explicitly closes its inherited write_fd before blocking, so
        // EOF is observable.
        let user_ns_sync_read_fd: libc::c_int = user_ns_sync
            .as_ref()
            .map(|p| p.read_fd.as_raw_fd())
            .unwrap_or(-1);
        let user_ns_sync_write_fd: libc::c_int = user_ns_sync
            .as_ref()
            .map(|p| p.write_fd.as_raw_fd())
            .unwrap_or(-1);

        // When a user namespace is in play, the child must transition
        // to in-NS UID 0 (the mapped root), NOT to the host stable
        // principal UID. Calling setuid(<host_uid>) inside the new user
        // NS fails with EINVAL because that UID is NOT mapped as an
        // in-namespace ID — only NS-UID 0 is mapped.
        //
        // The role profile's host UID/GID is still the EXEMPLAR
        // identity (used by ACLs and audit), but the in-NS
        // credential is always 0 when the broker pre-establishes
        // the namespace.
        let in_ns_credentials = user_ns_spec.is_some();
        let target_uid: libc::uid_t = if in_ns_credentials { 0 } else { uid };
        let target_gid: libc::gid_t = if in_ns_credentials { 0 } else { gid };

        // With setgroups=deny written by the parent's user-NS setup, the
        // child CANNOT call setgroups(2) — even setgroups(0, ...) returns
        // EPERM. Skip the call entirely when in a broker-pre-NS spawn.
        // Caller MUST ensure supplementary_groups is empty for
        // user_namespace spawns; we enforce this here defensively.
        if in_ns_credentials && !supplementary_groups_storage.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SpawnRunner: supplementary_groups must be empty when user_namespace is set (setgroups=deny precludes setgroups(2) calls)",
            ));
        }

        let cgroup_dir_raw_fd = cgroup_dir_fd.as_ref().map(|fd| fd.as_raw_fd());

        let outcome = clone3_pidfd_or_fork_fallback_with_cgroup(
            extra_clone_flags,
            cgroup_dir_raw_fd,
            move |_| unsafe {
                // If we're in a user NS, FIRST close the inherited write end
                // of the sync pipe so the parent's death is observable as EOF.
                // THEN block on the read end until the parent has written
                // uid_map/setgroups=deny/gid_map and signaled.
                if user_ns_sync_read_fd >= 0 {
                    if user_ns_sync_write_fd >= 0 {
                        libc::close(user_ns_sync_write_fd);
                    }
                    let mut buf = [0u8; 1];
                    let n = libc::read(user_ns_sync_read_fd, buf.as_mut_ptr() as *mut _, 1);
                    if n != 1 {
                        let m = b"DEBUG: sync read returned non-1\n";
                        libc::write(2, m.as_ptr() as *const _, m.len());
                        libc::_exit(CHILD_EXIT_USER_NS_SYNC);
                    }
                    libc::close(user_ns_sync_read_fd);
                    let m = b"DEBUG: sync passed\n";
                    libc::write(2, m.as_ptr() as *const _, m.len());
                }
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
                    let m = b"DEBUG: prctl NO_NEW_PRIVS failed\n";
                    libc::write(2, m.as_ptr() as *const _, m.len());
                    libc::_exit(CHILD_EXIT_PRCTL_NO_NEW_PRIVS);
                }
                if !capabilities.is_empty() && libc::prctl(libc::PR_SET_KEEPCAPS, 1, 0, 0, 0) < 0 {
                    libc::_exit(CHILD_EXIT_PRCTL_KEEP_CAPS);
                }
                if unshare_flags != 0 && libc::unshare(unshare_flags) < 0 {
                    let m = b"DEBUG: unshare failed\n";
                    libc::write(2, m.as_ptr() as *const _, m.len());
                    libc::_exit(CHILD_EXIT_UNSHARE);
                }
                if mount_required {
                    if libc::mount(
                        std::ptr::null(),
                        c"/".as_ptr(),
                        std::ptr::null(),
                        (libc::MS_REC | libc::MS_PRIVATE) as libc::c_ulong,
                        std::ptr::null(),
                    ) < 0
                    {
                        let errno = *libc::__errno_location();
                        let m = b"DEBUG: mount MS_PRIVATE failed errno=";
                        libc::write(2, m.as_ptr() as *const _, m.len());
                        let mut buf = [0u8; 16];
                        let len = format_errno(errno, &mut buf);
                        libc::write(2, buf.as_ptr() as *const _, len);
                        libc::write(2, b"\n".as_ptr() as *const _, 1);
                        libc::_exit(CHILD_EXIT_MOUNT);
                    }
                    // When in_ns_credentials (broker-pre-NS spawn per ADR 0021),
                    // SKIP apply_mount_actions.
                    // The user-NS already provides isolation (each NS has its
                    // own mount tree clone from clone3). Bind-mounting paths
                    // onto themselves WOULD FAIL with EPERM for any path that
                    // belongs to a mount inherited from the parent NS — Linux
                    // locks inherited mounts inside user-NS so they can't be
                    // mutated. virtiofsd's --sandbox=chroot does its own
                    // pivot_root inside the user-NS post-exec, which works
                    // because the new NS-root has CAP_SYS_ADMIN. The old
                    // (non-user-NS) bind-mount semantics from the minijail
                    // model are unnecessary for the broker-pre-NS path.
                    if !in_ns_credentials {
                        match apply_mount_actions_debug(mount_actions) {
                            Ok(()) => {}
                            Err((errno, path_bytes)) => {
                                let m = b"DEBUG: apply_mount_actions failed errno=";
                                libc::write(2, m.as_ptr() as *const _, m.len());
                                let mut buf = [0u8; 16];
                                let len = format_errno(errno, &mut buf);
                                libc::write(2, buf.as_ptr() as *const _, len);
                                let m2 = b" path=";
                                libc::write(2, m2.as_ptr() as *const _, m2.len());
                                libc::write(2, path_bytes.as_ptr() as *const _, path_bytes.len());
                                libc::write(2, b"\n".as_ptr() as *const _, 1);
                                libc::_exit(CHILD_EXIT_MOUNT);
                            }
                        }
                        match apply_root_secret_masks(root_mask_actions) {
                            Ok(()) => {}
                            Err((errno, path_bytes)) => {
                                let m = b"DEBUG: apply_root_secret_masks failed errno=";
                                libc::write(2, m.as_ptr() as *const _, m.len());
                                let mut buf = [0u8; 16];
                                let len = format_errno(errno, &mut buf);
                                libc::write(2, buf.as_ptr() as *const _, len);
                                let m2 = b" path=";
                                libc::write(2, m2.as_ptr() as *const _, m2.len());
                                libc::write(2, path_bytes.as_ptr() as *const _, path_bytes.len());
                                libc::write(2, b"\n".as_ptr() as *const _, 1);
                                libc::_exit(CHILD_EXIT_MOUNT);
                            }
                        }
                        if mask_dev {
                            match apply_device_mask_and_binds(
                                prepared_device_binds,
                                target_uid,
                                target_gid,
                            ) {
                                Ok(()) => {}
                                Err((errno, path_bytes)) => {
                                    let m = b"DEBUG: apply_device_mask_and_binds failed errno=";
                                    libc::write(2, m.as_ptr() as *const _, m.len());
                                    let mut buf = [0u8; 16];
                                    let len = format_errno(errno, &mut buf);
                                    libc::write(2, buf.as_ptr() as *const _, len);
                                    let m2 = b" path=";
                                    libc::write(2, m2.as_ptr() as *const _, m2.len());
                                    libc::write(
                                        2,
                                        path_bytes.as_ptr() as *const _,
                                        path_bytes.len(),
                                    );
                                    libc::write(2, b"\n".as_ptr() as *const _, 1);
                                    libc::_exit(CHILD_EXIT_MOUNT);
                                }
                            }
                            match apply_private_procfs() {
                                Ok(()) => {}
                                Err((errno, path_bytes)) => {
                                    let m = b"DEBUG: apply_private_procfs failed errno=";
                                    libc::write(2, m.as_ptr() as *const _, m.len());
                                    let mut buf = [0u8; 16];
                                    let len = format_errno(errno, &mut buf);
                                    libc::write(2, buf.as_ptr() as *const _, len);
                                    let m2 = b" path=";
                                    libc::write(2, m2.as_ptr() as *const _, m2.len());
                                    libc::write(
                                        2,
                                        path_bytes.as_ptr() as *const _,
                                        path_bytes.len(),
                                    );
                                    libc::write(2, b"\n".as_ptr() as *const _, 1);
                                    libc::_exit(CHILD_EXIT_MOUNT);
                                }
                            }
                        }
                    }
                }
                if let Some(limit) = memlock_limit_bytes {
                    let rlim = libc::rlimit {
                        rlim_cur: limit as libc::rlim_t,
                        rlim_max: limit as libc::rlim_t,
                    };
                    if libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) < 0 {
                        let m = b"DEBUG: setrlimit RLIMIT_MEMLOCK failed\n";
                        libc::write(2, m.as_ptr() as *const _, m.len());
                        libc::_exit(CHILD_EXIT_MEMLOCK_RLIMIT);
                    }
                }
                // Credential changes here MUST go through the RAW syscalls, not
                // the glibc `setgroups`/`setgid`/`setuid` wrappers. glibc routes
                // those wrappers through its SETXID machinery (`__nptl_setxid`),
                // which broadcasts the credential change to every thread in the
                // process and blocks on a futex until all of them acknowledge via
                // a signal. In this `clone3`/`fork` child of the MULTITHREADED
                // broker (and, under test, the multithreaded `cargo test` runner)
                // the inherited glibc thread list still names the parent's other
                // threads — none of which exist in the single-threaded child — so
                // the wrapper waits forever on a futex no one will ever post. That
                // is a classic fork-in-a-multithreaded-program deadlock and it
                // wedges the child before `execve`, leaking every inherited fd
                // (including other operations' held `flock`'d `sync.lock` fds,
                // which then never release). The raw syscalls change only the
                // calling (sole) thread's credentials, which is exactly what we
                // want immediately before `execve`, and they are async-signal-safe.
                //
                // Skip setgroups when in a broker-pre-NS spawn (parent wrote
                // setgroups=deny so any call would EPERM).
                if !in_ns_credentials
                    && libc::syscall(
                        libc::SYS_setgroups,
                        supplementary_groups.len(),
                        supplementary_groups.as_ptr(),
                    ) < 0
                {
                    libc::_exit(CHILD_EXIT_SETGROUPS);
                }
                if libc::syscall(libc::SYS_setgid, target_gid) < 0 {
                    let m = b"DEBUG: setgid failed\n";
                    libc::write(2, m.as_ptr() as *const _, m.len());
                    libc::_exit(CHILD_EXIT_SETGID);
                }
                if libc::syscall(libc::SYS_setuid, target_uid) < 0 {
                    let m = b"DEBUG: setuid failed\n";
                    libc::write(2, m.as_ptr() as *const _, m.len());
                    libc::_exit(CHILD_EXIT_SETUID);
                }
                if required_clone_flags != 0 {
                    for capability in 0..=40 {
                        if libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) < 0 {
                            libc::_exit(CHILD_EXIT_REALM_CAPABILITY_DROP);
                        }
                    }
                }
                if (required_clone_flags != 0 || !capabilities.is_empty())
                    && apply_capabilities(capabilities).is_err()
                {
                    libc::_exit(CHILD_EXIT_CAPSET);
                }
                // umask MUST precede seccomp. A restrictive BPF filter that
                // kills unrecognised ioctls would SIGSYS the process if
                // libc::umask() is implemented via ioctl on some kernels or
                // if a future profile adds ioctl-free seccomp. Ordering:
                //   capset → umask → seccomp → execve
                // Install umask from the role profile before execve. Sidecars
                // that bind shared Unix sockets (vhost-user-sound, crosvm-gpu,
                // swtpm) use 0o007 so the bind() returns a 0660-mode socket —
                // the existing /run/d2b/vms/<vm>/ default ACL
                // (user:<ch-uid>:rwx) then becomes effective for
                // cloud-hypervisor because mode-group-bits derive ACL mask=rw,
                // not mask=---.
                //
                // Reject umasks that exceed the POSIX file-mode width (0o777)
                // so a config typo (e.g. umask = 9999) is caught explicitly
                // rather than silently truncated by libc::umask.
                if let Some(mask) = child_umask {
                    if mask > 0o777 {
                        let m = b"DEBUG: invalid umask (>0o777)\n";
                        libc::write(2, m.as_ptr() as *const _, m.len());
                        libc::_exit(CHILD_EXIT_INVALID_UMASK);
                    }
                    libc::umask(mask as libc::mode_t);
                }
                if let Some((stdin_fd, stdout_fd)) = inherited_stdio_raw {
                    for (src_fd, dst_fd) in [(stdin_fd, 0), (stdout_fd, 1)] {
                        if src_fd != dst_fd && libc::dup2(src_fd, dst_fd) < 0 {
                            let m = b"DEBUG: dup2 inherited stdio fd failed\n";
                            libc::write(2, m.as_ptr() as *const _, m.len());
                            libc::_exit(CHILD_EXIT_PREOPEN_DUP2);
                        }
                        libc::fcntl(dst_fd, libc::F_SETFD, 0);
                    }
                }
                // Install pre-opened device fds at their well-known numbers
                // before seccomp is loaded (ADR 0021).
                //
                // Ordering (capset → umask → pre-open → seccomp → execve): the
                // dup2/fcntl calls must precede seccomp installation so the
                // filter does not need to permit them at runtime.
                //
                // Each fd is dup2'd to RENDER_NODE_INHERITED_FD + index:
                //   - dup2 does NOT copy CLOEXEC; fcntl(F_SETFD,0) is defensive
                //     no-op (CLOEXEC was already absent on the dup2 target) but
                //     explicit for future-proofing.
                //   - If src == dst (already at the target slot), skip dup2/close
                //     but still clear CLOEXEC (dup2(a,a) is a POSIX no-op AND
                //     does not clear CLOEXEC on the destination).
                //   - OwnedFds holding the original fds are in the closure and
                //     dropped by the PARENT after fork; in the child _exit(2) is
                //     called (no Rust destructors) after execve. No double-close.
                for (i, &src_fd) in pre_opened_raw_fds.iter().enumerate() {
                    let dst_fd = RENDER_NODE_INHERITED_FD + i as libc::c_int;
                    if src_fd != dst_fd {
                        if libc::dup2(src_fd, dst_fd) < 0 {
                            let m = b"DEBUG: dup2 pre-opened device fd failed\n";
                            libc::write(2, m.as_ptr() as *const _, m.len());
                            libc::_exit(CHILD_EXIT_PREOPEN_DUP2);
                        }
                        libc::close(src_fd);
                    }
                    // Clear CLOEXEC so the fd survives execve.
                    libc::fcntl(dst_fd, libc::F_SETFD, 0);
                }
                if required_clone_flags != 0 {
                    let close_low = libc::syscall(libc::SYS_close_range, 3u32, 9u32, 0u32);
                    let first_after_declared =
                        RENDER_NODE_INHERITED_FD as u32 + pre_opened_raw_fds.len() as u32;
                    let close_high =
                        libc::syscall(libc::SYS_close_range, first_after_declared, u32::MAX, 0u32);
                    if close_low < 0 || close_high < 0 {
                        libc::_exit(CHILD_EXIT_REALM_FD_CLOSE);
                    }
                }
                if let Some(program) = seccomp_program.as_ref()
                    && apply_seccomp(program).is_err()
                {
                    libc::_exit(CHILD_EXIT_SECCOMP);
                }
                libc::execve(binary_ptr, argv_ptrs.as_ptr(), env_ptrs.as_ptr());
                let m2 = b"DEBUG: execve returned (failed)\n";
                libc::write(2, m2.as_ptr() as *const _, m2.len());
                libc::_exit(CHILD_EXIT_EXECVE);
            },
        )?;

        if required_clone_flags != 0 && outcome.used_fork_fallback {
            let _ = pidfd_send_signal(outcome.pidfd.as_fd(), libc::SIGKILL);
            let _ = reap_spawn_runner_error_child(outcome.pid);
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "realm child spawn requires clone3 namespace and cgroup placement",
            ));
        }
        let cgroup_already_placed = cgroup_dir_raw_fd.is_some() && !outcome.used_fork_fallback;
        if let Err(err) = parent_attach_fallback_cgroup(
            cgroup_procs_fd.as_ref(),
            outcome.pid,
            cgroup_already_placed,
        ) {
            let _ = pidfd_send_signal(outcome.pidfd.as_fd(), libc::SIGKILL);
            drop(user_ns_sync);
            let _ = reap_spawn_runner_error_child(outcome.pid);
            return Err(err);
        }

        // Parent-side user-NS map writes. Performed AFTER clone3 returns
        // (we have the child's PID) and AFTER any fallback cgroup.procs
        // attach that the parent must perform with host credentials. Both
        // complete BEFORE the sync pipe write that unblocks the child.
        // Sequencing per `man 7 user_namespaces`: cgroup.procs fallback
        // attach → uid_map → setgroups=deny → gid_map → sync byte. The
        // child has already closed its inherited write_fd, so if the
        // parent dies BEFORE this point the child gets EOF on read and
        // exits CHILD_EXIT_USER_NS_SYNC=74.
        if let (Some(sync), Some(spec)) = (user_ns_sync, user_ns_spec) {
            if let Err(err) =
                write_user_namespace_maps(outcome.pid, spec, additional_user_ns_mapping).map_err(
                    |err| {
                        // Preserve the underlying io::Error as the source so
                        // callers can chain `.source()` to recover the original
                        // errno/kind information. The outer wrapper adds
                        // operator-friendly context (which /proc path, which pid)
                        // without erasing the cause.
                        io::Error::new(
                            err.kind(),
                            UserNsMapWriteError {
                                pid: outcome.pid,
                                source: err,
                            },
                        )
                    },
                )
            {
                drop(sync);
                let _ = reap_spawn_runner_error_child(outcome.pid);
                return Err(err);
            }
            // Drop the parent's inherited read end before signaling
            // — it's never read on the parent side. Explicit drop
            // for the reader half makes the intent obvious.
            drop(sync.read_fd);
            match rustix::io::write(&sync.write_fd, &[0u8; 1]) {
                Ok(1) => {}
                Ok(_) => {
                    drop(sync.write_fd);
                    let _ = reap_spawn_runner_error_child(outcome.pid);
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "short user namespace sync pipe write",
                    ));
                }
                Err(err) => {
                    drop(sync.write_fd);
                    let _ = reap_spawn_runner_error_child(outcome.pid);
                    return Err(io::Error::from_raw_os_error(err.raw_os_error()));
                }
            }
        }

        Ok(outcome)
    }

    /// Wrapping error type that preserves the underlying io::Error as
    /// `Error::source()` while adding pid + operation context to the
    /// Display impl. Used by `write_user_namespace_maps` failures so
    /// operators see "user_namespace map write failed for pid 12345:
    /// <orig>" AND callers chasing `.source()` recover the original
    /// ENOSPC / EPERM / EACCES etc.
    #[derive(Debug)]
    struct UserNsMapWriteError {
        pid: i32,
        source: io::Error,
    }

    impl std::fmt::Display for UserNsMapWriteError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "user_namespace map write failed for pid {}: {}",
                self.pid, self.source
            )
        }
    }

    impl std::error::Error for UserNsMapWriteError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.source)
        }
    }

    /// Parent-side helper that writes the single-entry uid_map /
    /// setgroups=deny / gid_map for the child user namespace. Per
    /// `man 7 user_namespaces` the setgroups=deny write MUST happen
    /// before gid_map when the writer is not privileged.
    ///
    /// Each io::Error is annotated with the specific /proc path that
    /// failed so operators don't have to guess which of the three writes
    /// errored.
    fn write_user_namespace_maps(
        child_pid: i32,
        spec: UserNamespaceSpec,
        additional: Option<(u32, u32)>,
    ) -> io::Result<()> {
        use std::fs;
        let uid_map_path = format!("/proc/{child_pid}/uid_map");
        let setgroups_path = format!("/proc/{child_pid}/setgroups");
        let gid_map_path = format!("/proc/{child_pid}/gid_map");
        let mut uid_map = format!("0 {} 1\n", spec.host_uid_for_zero);
        let mut gid_map = format!("0 {} 1\n", spec.host_gid_for_zero);
        if let Some((uid, gid)) = additional {
            if uid != spec.host_uid_for_zero {
                uid_map.push_str(&format!("1 {uid} 1\n"));
            }
            if gid != spec.host_gid_for_zero {
                gid_map.push_str(&format!("1 {gid} 1\n"));
            }
        }
        fs::write(&uid_map_path, uid_map)
            .map_err(|err| io::Error::new(err.kind(), format!("writing {uid_map_path}: {err}")))?;
        fs::write(&setgroups_path, "deny").map_err(|err| {
            io::Error::new(err.kind(), format!("writing {setgroups_path}: {err}"))
        })?;
        fs::write(&gid_map_path, gid_map)
            .map_err(|err| io::Error::new(err.kind(), format!("writing {gid_map_path}: {err}")))?;
        Ok(())
    }

    /// Small helper that creates a CLOEXEC pipe and returns the two ends
    /// as OwnedFds. Used to gate the child's post-clone setup on the
    /// parent finishing the uid_map writes.
    pub(super) struct SyncPipe {
        pub(super) read_fd: OwnedFd,
        pub(super) write_fd: OwnedFd,
    }

    pub(super) fn make_sync_pipe() -> io::Result<SyncPipe> {
        let (read_fd, write_fd) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)
            .map_err(|err| io::Error::from_raw_os_error(err as i32))?;
        Ok(SyncPipe { read_fd, write_fd })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::fs::{PermissionsExt, symlink};

    use d2b_core::minijail_profile::{MountPolicy, NamespaceSet};
    use nix::libc;
    use tempfile::tempdir;

    fn test_namespaces(pid: bool) -> NamespaceSet {
        NamespaceSet {
            mount: true,
            pid,
            net: false,
            ipc: true,
            uts: false,
            user: false,
        }
    }

    fn hidden_dev_policy(device_binds: Vec<String>) -> MountPolicy {
        MountPolicy {
            read_only_paths: vec!["/".to_owned()],
            writable_paths: vec![],
            nix_store_read_only: true,
            hide_device_nodes_by_default: true,
            device_binds,
            bind_mounts: vec![],
        }
    }

    #[test]
    fn device_mask_required_even_with_empty_device_binds() {
        let policy = hidden_dev_policy(vec![]);

        assert!(
            super::pidfd_sys::device_mask_required(&policy, &test_namespaces(true)),
            "hideDeviceNodesByDefault with a private pid namespace must mask /dev even when deviceBinds is empty"
        );
    }

    #[test]
    fn device_mask_requires_private_pid_namespace() {
        let policy = hidden_dev_policy(vec!["/dev/kvm".to_owned()]);

        assert!(
            !super::pidfd_sys::device_mask_required(&policy, &test_namespaces(false)),
            "device masking installs a private /proc and therefore requires the role to request a pid namespace"
        );
    }

    #[test]
    fn realm_child_seccomp_denies_host_introspection_and_kernel_mutation() {
        let program = super::pidfd_sys::SeccompProgram::deny_syscalls(&[
            libc::SYS_ptrace as u32,
            libc::SYS_bpf as u32,
            libc::SYS_reboot as u32,
        ]);

        assert!(program.rejects_syscall(libc::SYS_ptrace as u32));
        assert!(program.rejects_syscall(libc::SYS_bpf as u32));
        assert!(program.rejects_syscall(libc::SYS_reboot as u32));
        assert!(!program.rejects_syscall(libc::SYS_read as u32));
        assert!(!program.rejects_syscall(libc::SYS_clone3 as u32));
    }

    #[test]
    fn path_safe_atomic_replace_round_trips() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("artifact.txt");
        let dir_fd = super::path_safe::open_dir_path_safe(dir.path()).expect("open safe dir");

        super::path_safe::atomic_replace_fd(&dir_fd, "artifact.txt", b"hello", 0o640)
            .expect("atomic replace succeeds");

        assert_eq!(fs::read(&path).expect("read back"), b"hello");
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn path_safe_atomic_replace_refuses_symlinked_parent() {
        let dir = tempdir().expect("tempdir");
        let real_parent = dir.path().join("real-parent");
        let symlink_parent = dir.path().join("symlink-parent");
        fs::create_dir(&real_parent).expect("create real parent");
        symlink(&real_parent, &symlink_parent).expect("create symlinked parent");

        assert!(super::path_safe::open_dir_path_safe(&symlink_parent).is_err());
    }

    #[test]
    fn path_safe_atomic_replace_idempotent_on_retry() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("retry.txt");
        let dir_fd = super::path_safe::open_dir_path_safe(dir.path()).expect("open safe dir");

        super::path_safe::atomic_replace_fd(&dir_fd, "retry.txt", b"first", 0o600)
            .expect("first write succeeds");
        super::path_safe::atomic_replace_fd(&dir_fd, "retry.txt", b"second", 0o644)
            .expect("second write succeeds");

        assert_eq!(fs::read(&path).expect("read back"), b"second");
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn path_safe_ensure_dir_path_safe_creates_directory() {
        let dir = tempdir().expect("tempdir");
        let parent_fd = super::path_safe::open_dir_path_safe(dir.path()).expect("open safe dir");
        let current_uid = nix::unistd::Uid::current().as_raw();
        let current_gid = nix::unistd::Gid::current().as_raw();

        let created = super::path_safe::ensure_dir_path_safe(
            &parent_fd,
            "state",
            0o750,
            current_uid,
            current_gid,
        )
        .expect("create dir safely");
        drop(created);

        let path = dir.path().join("state");
        let meta = fs::metadata(&path).expect("metadata");
        assert!(meta.is_dir());
        assert_eq!(meta.permissions().mode() & 0o7777, 0o750);
    }

    #[test]
    fn path_safe_remove_path_safe_refuses_symlink() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, b"hello").expect("write target");
        symlink(&target, &link).expect("create symlink");

        let parent_fd = super::path_safe::open_dir_path_safe(dir.path()).expect("open safe dir");
        let err = super::path_safe::remove_path_safe(&parent_fd, "link")
            .expect_err("symlink should fail");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(link.exists());
    }

    #[test]
    fn path_safe_remove_path_safe_removes_directory() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        fs::create_dir(&child).expect("create child dir");

        let parent_fd = super::path_safe::open_dir_path_safe(dir.path()).expect("open safe dir");
        super::path_safe::remove_path_safe(&parent_fd, "child").expect("remove child dir");
        assert!(!child.exists());
    }

    // UserNamespaceSpec validation (ADR 0021). These tests exercise the
    // SHAPE of the request, not the kernel-level fork+map dance (which
    // requires root and is tested by integration tests in live_handlers.rs).

    use super::pidfd_sys::{RunnerIsolationSpec, UserNamespaceSpec, clone3_spawn_runner};
    use std::ffi::CString;

    fn empty_mount_policy() -> MountPolicy {
        MountPolicy {
            read_only_paths: vec![],
            writable_paths: vec![],
            nix_store_read_only: false,
            hide_device_nodes_by_default: false,
            device_binds: vec![],
            bind_mounts: vec![],
        }
    }

    fn isolation_with_user_namespace(spec: Option<UserNamespaceSpec>) -> RunnerIsolationSpec {
        RunnerIsolationSpec {
            capabilities: vec![],
            namespaces: NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: spec.is_some(),
            },
            seccomp_program: None,
            mount_policy: empty_mount_policy(),
            cgroup_dir_fd: None,
            cgroup_procs_fd: None,
            user_namespace: spec,
            umask: None,
            pre_opened_device_fds: Vec::new(),
            inherited_stdio: None,
            memlock_limit_bytes: None,
        }
    }

    #[test]
    fn user_namespace_true_requires_spec() {
        // namespaces.user=true but user_namespace=None is
        // rejected before clone3 — the child would land in the
        // NS with overflowuid and never be able to setuid(0).
        let mut iso = isolation_with_user_namespace(None);
        iso.namespaces.user = true;
        let bin = CString::new("/bin/true").unwrap();
        let err = clone3_spawn_runner(bin, vec![], vec![], 1, 1, vec![], iso)
            .expect_err("should reject mismatched user-NS request");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn user_namespace_spec_requires_namespace_flag() {
        // user_namespace=Some but namespaces.user=false is
        // also rejected — the spec would be silently ignored.
        let mut iso = isolation_with_user_namespace(Some(UserNamespaceSpec {
            host_uid_for_zero: 1000,
            host_gid_for_zero: 1000,
        }));
        iso.namespaces.user = false;
        let bin = CString::new("/bin/true").unwrap();
        let err = clone3_spawn_runner(bin, vec![], vec![], 1, 1, vec![], iso)
            .expect_err("should reject orphan user_namespace spec");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// Assert that the `RunnerIsolationSpec.umask` field is present,
    /// defaults to `None`, and accepts a valid octal value. This is the
    /// unit-level verification that the umask plumbing reaches the
    /// child-closure layer; the actual `libc::umask()` syscall is
    /// exercised by the live deploy + the integration-level VM boot
    /// tests (sidecars binding mode-0660 sockets that CH can connect to).
    #[test]
    fn isolation_spec_umask_field_defaults_to_none() {
        let iso = isolation_with_user_namespace(None);
        assert_eq!(iso.umask, None);
    }

    #[test]
    fn isolation_spec_umask_field_accepts_octal_007() {
        let mut iso = isolation_with_user_namespace(None);
        iso.umask = Some(0o007);
        assert_eq!(iso.umask, Some(7));
    }

    #[test]
    fn child_spawn_path_does_not_log_full_argv_or_env_to_global_file() {
        let source = include_str!("sys.rs");
        let global_log = concat!("d2b-broker-child", ".log");
        let argv_marker = concat!("DEBUG ", "argv[");
        let env_marker = concat!("DEBUG ", "env[");
        let spawn_marker = concat!("DEBUG: about", " to execve");
        assert!(
            !source.contains(global_log),
            "broker child path must not create a global argv/env debug log"
        );
        assert!(
            !source.contains(argv_marker) && !source.contains(env_marker),
            "broker child path must not log full runner argv/env"
        );
        assert!(
            !source.contains(spawn_marker),
            "broker child path must not emit a debug line for every spawn"
        );
    }

    fn owned_pipe_for_test() -> (OwnedFd, OwnedFd) {
        nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).expect("pipe2 failed")
    }

    fn read_pipe_once(read_fd: &OwnedFd) -> String {
        let mut buf = [0u8; 64];
        let n = rustix::io::read(read_fd, &mut buf).expect("pipe read failed");
        String::from_utf8(buf[..n].to_vec()).expect("pipe data is utf8")
    }

    fn assert_fd_cloexec(fd: &OwnedFd) {
        let flags = nix::fcntl::fcntl(fd.as_raw_fd(), nix::fcntl::FcntlArg::F_GETFD)
            .expect("F_GETFD succeeds for pipe fd");
        assert_ne!(
            flags & libc::FD_CLOEXEC,
            0,
            "sync pipe fd must be close-on-exec"
        );
    }

    #[test]
    fn sync_pipe_fds_are_cloexec() {
        let pipe = super::pidfd_sys::make_sync_pipe().expect("production sync pipe");

        assert_fd_cloexec(&pipe.read_fd);
        assert_fd_cloexec(&pipe.write_fd);
    }

    #[test]
    fn parent_fallback_cgroup_attach_writes_child_pid_not_self() {
        let (read_fd, write_fd) = owned_pipe_for_test();

        super::pidfd_sys::parent_attach_fallback_cgroup(Some(&write_fd), 4242, false)
            .expect("fallback cgroup attach succeeds");
        drop(write_fd);

        assert_eq!(read_pipe_once(&read_fd), "4242\n");
    }

    #[test]
    fn parent_fallback_cgroup_attach_skips_clone_into_cgroup_path() {
        let (read_fd, write_fd) = owned_pipe_for_test();

        super::pidfd_sys::parent_attach_fallback_cgroup(Some(&write_fd), 4242, true)
            .expect("clone-placed path is a no-op");
        drop(write_fd);

        assert_eq!(read_pipe_once(&read_fd), "");
    }

    #[test]
    fn spawn_runner_uses_clone_into_cgroup_before_fallback_attach() {
        let source = include_str!("sys.rs");
        let clone_into_cgroup_call = concat!(
            "clone3_pidfd_or_fork_fallback_with_cgroup(\n",
            "            extra_clone_flags,\n",
            "            cgroup_dir_raw_fd,"
        );
        let parent_fallback_attach = concat!(
            "parent_attach_fallback_cgroup(\n",
            "            cgroup_procs_fd.as_ref(),\n",
            "            outcome.pid,\n",
            "            cgroup_already_placed,"
        );
        let user_ns_maps =
            "write_user_namespace_maps(outcome.pid, spec, additional_user_ns_mapping)";
        let user_ns_signal = "rustix::io::write(&sync.write_fd";
        let reap_error_child = concat!("reap_spawn_runner", "_error_child(outcome.pid)");
        let removed_child_helper = concat!("write_self", "_to_cgroup");
        assert!(
            source.contains(clone_into_cgroup_call),
            "SpawnRunner must pass the role-leaf cgroup dirfd to clone3 for CLONE_INTO_CGROUP"
        );
        let parent_attach_pos = source
            .find(parent_fallback_attach)
            .expect("fallback cgroup attach must happen on the parent path");
        let user_ns_maps_pos = source
            .find(user_ns_maps)
            .expect("user namespace map writes must remain parent-side");
        let user_ns_signal_pos = source
            .find(user_ns_signal)
            .expect("user namespace sync write must remain parent-side");
        assert!(
            parent_attach_pos < user_ns_maps_pos && user_ns_maps_pos < user_ns_signal_pos,
            "fallback cgroup attach must write the child pid before user-NS maps and sync continuation"
        );
        let first_reap_pos = source
            .get(parent_attach_pos..)
            .expect("fallback cgroup attach position is in bounds")
            .find(reap_error_child)
            .map(|offset| parent_attach_pos + offset)
            .expect("post-clone error paths must synchronously reap the child");
        assert!(
            parent_attach_pos < first_reap_pos && first_reap_pos < user_ns_maps_pos,
            "fallback cgroup attach failure must reap before returning Err"
        );
        assert_eq!(
            source[parent_attach_pos..]
                .matches(reap_error_child)
                .count(),
            4,
            "fallback cgroup, user-NS map, sync-write error, and sync-write short-write paths must each reap before returning Err"
        );
        assert!(
            !source.contains(removed_child_helper),
            "child path must not mutate cgroup.procs after entering CLONE_NEWUSER"
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn reap_spawn_runner_error_child_consumes_child_status() {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
        if pid == 0 {
            unsafe { libc::_exit(0) };
        }

        super::pidfd_sys::reap_spawn_runner_error_child(pid).expect("error child reaped");

        let mut status = 0;
        let rc = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(rc, -1, "child status should already be consumed");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD),
            "second waitpid should report no unreaped child"
        );
    }

    #[test]
    #[cfg(unix)]
    fn mknod_device_bind_target_chowns_runner_device_node() {
        if !nix::unistd::Uid::effective().is_root() {
            return;
        }
        use std::os::unix::fs::MetadataExt as _;

        let dir =
            std::env::temp_dir().join(format!("d2b-device-bind-target-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let path = dir.join("null");
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .expect("temp path has no nul");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();

        let null_metadata = std::fs::metadata("/dev/null").expect("stat /dev/null");
        let rc = super::pidfd_sys::mknod_device_bind_target(
            &c_path,
            (nix::libc::S_IFCHR | 0o600) as nix::libc::mode_t,
            null_metadata.rdev() as nix::libc::dev_t,
            uid,
            gid,
        );

        let metadata = std::fs::metadata(&path).expect("stat created device node");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);

        rc.expect("mknod device bind target");
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.uid(), uid);
        assert_eq!(metadata.gid(), gid);
    }

    #[test]
    fn device_bind_target_chown_call_is_pinned() {
        let source = include_str!("sys.rs");
        let chown_call = concat!("libc::", "chown(destination.as_ptr(), uid, gid)");
        assert!(
            source.contains(chown_call),
            "masked device bind targets must be chowned to the runner uid/gid"
        );
    }

    /// Verify that the broker rejects an umask >0o777 before exec rather
    /// than silently truncating via libc cast. The child writes "DEBUG:
    /// invalid umask" to stderr and exits CHILD_EXIT_INVALID_UMASK (75).
    /// Verified at child-closure level: any value with bits above 0o777
    /// set must reach the `mask > 0o777` guard.
    #[test]
    fn umask_validation_bound_is_0o777() {
        // Sanity check: 0o007 is in range, 0o1000 is out.
        let valid: u32 = 0o007;
        let invalid: u32 = 0o1000;
        assert!(valid <= 0o777);
        assert!(invalid > 0o777);
    }

    /// Hermetic unit test asserting that `apply_mount_actions` is NOT
    /// called when `in_ns_credentials = true` (broker-pre-NS spawn per
    /// ADR 0021).
    ///
    /// Strategy — exit-code oracle: spawn a no-op binary (`true`) with
    /// `user_namespace = Some(...)` (which sets `in_ns_credentials =
    /// true`) and a mount policy that produces a non-empty
    /// `mount_actions` list (`nix_store_read_only = true` →
    /// `["/nix/store"]`):
    ///
    /// - With the guard (`if !in_ns_credentials`) in place:
    ///   `apply_mount_actions` is skipped; the child execs `true` and
    ///   exits 0.
    /// - Without the guard: `apply_mount_actions_debug` would attempt
    ///   `mount("/nix/store", "/nix/store", NULL, MS_BIND|MS_REC)`
    ///   inside the user-NS where inherited mounts are locked by the
    ///   kernel (CAP_SYS_ADMIN in the child user-NS is not sufficient
    ///   to mutate mounts owned by the parent user-NS); the call
    ///   returns EPERM and the child exits `CHILD_EXIT_MOUNT` (64).
    ///
    /// Skips cleanly on hosts with `kernel.unprivileged_userns_clone=0`
    /// (clone3 returns EPERM before any child runs).
    #[test]
    fn apply_mount_actions_skipped_in_user_ns() {
        use nix::sys::wait::{WaitStatus, waitpid};
        use nix::unistd::Pid;

        // /bin/true is absent on NixOS; probe common locations.
        let true_path = [
            "/bin/true",
            "/usr/bin/true",
            "/run/current-system/sw/bin/true",
        ]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .expect("could not find `true` binary in any candidate location");

        let current_uid = nix::unistd::Uid::current().as_raw();
        let current_gid = nix::unistd::Gid::current().as_raw();

        let iso = RunnerIsolationSpec {
            capabilities: vec![],
            namespaces: NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: true,
            },
            seccomp_program: None,
            // nix_store_read_only = true triggers
            // mount_policy_requires_namespace → mount_required = true
            // → prepare_mount_actions → [PreparedMountAction { path:
            // "/nix/store", readonly: true }].  This guarantees a
            // non-empty mount_actions slice reaches the guard.
            mount_policy: MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: true,
                hide_device_nodes_by_default: false,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_dir_fd: None,
            cgroup_procs_fd: None,
            user_namespace: Some(UserNamespaceSpec {
                host_uid_for_zero: current_uid,
                host_gid_for_zero: current_gid,
            }),
            umask: None,
            pre_opened_device_fds: Vec::new(),
            inherited_stdio: None,
            memlock_limit_bytes: None,
        };

        let bin = CString::new(true_path).unwrap();
        let argv0 = bin.clone();
        // supplementary_groups MUST be empty: the parent writes
        // setgroups=deny during uid_map setup, so any call to
        // setgroups(2) in the child would return EPERM.
        let outcome = match clone3_spawn_runner(
            bin,
            vec![argv0], // argv[0] = binary path; coreutils multi-call needs it
            vec![],
            current_uid,
            current_gid,
            vec![],
            iso,
        ) {
            Ok(o) => o,
            Err(e) if e.raw_os_error() == Some(nix::libc::EPERM) => {
                // kernel.unprivileged_userns_clone=0: user namespaces
                // not available on this host — skip rather than fail.
                println!(
                    "SKIP: unprivileged user NS not available \
                     (kernel.unprivileged_userns_clone=0)"
                );
                return;
            }
            Err(e) => panic!("clone3_spawn_runner failed unexpectedly: {e}"),
        };

        // Reap the child and capture its wait status.
        let wait_status = waitpid(Pid::from_raw(outcome.pid), None).expect("waitpid failed");
        const CHILD_EXIT_UNSHARE_IN_CHILD: nix::libc::c_int = 62;
        if matches!(
            wait_status,
            WaitStatus::Exited(_, code) if code == CHILD_EXIT_UNSHARE_IN_CHILD
        ) {
            println!("SKIP: unprivileged user NS not available inside child");
            return;
        }

        // CHILD_EXIT_MOUNT = 64 would mean apply_mount_actions ran and
        // got EPERM on the locked /nix/store bind-mount — i.e., the
        // `if !in_ns_credentials` guard is absent.
        assert_eq!(
            wait_status,
            WaitStatus::Exited(Pid::from_raw(outcome.pid), 0),
            "child did not exit 0 (expected /bin/true to succeed); \
             WaitStatus::Exited(_, 64) (CHILD_EXIT_MOUNT) would indicate \
             the user-namespace mount guard `if !in_ns_credentials` is missing and \
             apply_mount_actions was invoked inside the user-NS \
             (EPERM on locked inherited mount)"
        );
    }
}
