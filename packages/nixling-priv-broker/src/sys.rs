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

/// Path-safety helpers used by every W3 broker filesystem op
/// (`UpdateHostsFile`, `ApplyNmUnmanaged`, `PrepareStateDir`,
/// `PrepareRuntimeDir`). See plan.md §"W3 filesystem path-safety
/// tests".
///
/// The plan mandates `openat2` with `O_NOFOLLOW` + `RESOLVE_BENEATH`
/// for parent-relative resolution; this module implements equivalent
/// fail-closed guards on stable Rust using `nix` + `std::fs`, including
/// `RESOLVE_NO_XDEV` so mount-point crossings are rejected:
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
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

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
            ensure_dir_path_safe_inner(&parent_fd, &name, mode, owner_uid, owner_gid)?;
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
        format!(".nixling-{prefix}.{}.{}", std::process::id(), seq)
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

    fn atomic_replace_unsupported(target_name: &CString, detail: impl Into<String>) -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            PathSafeError::AtomicReplaceUnsupported {
                target_name: target_name.to_string_lossy().into_owned(),
                detail: detail.into(),
            },
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

    fn write_temp_file(file: &mut File, contents: &[u8], mode: u32) -> io::Result<()> {
        file.write_all(contents)?;
        fchmod(file.as_fd(), mode)?;
        file.sync_all()
    }

    fn create_anonymous_tmpfile(dir_fd: &OwnedFd, mode: u32) -> io::Result<File> {
        let dot = CString::new(".").expect("static dot path contains no NUL");
        match openat_raw(
            dir_fd.as_raw_fd(),
            &dot,
            libc::O_WRONLY | libc::O_CLOEXEC | libc::O_TMPFILE,
            mode,
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
                        mode,
                    ) {
                        Ok(fd) => {
                            if let Err(unlink_err) = unlinkat_raw(dir_fd.as_raw_fd(), &tmp_name) {
                                drop(fd);
                                return Err(unlink_err);
                            }
                            return Ok(File::from(fd));
                        }
                        Err(open_err) if open_err.kind() == io::ErrorKind::AlreadyExists => {
                            continue
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
                    Ok(()) => {
                        if let Err(unlink_err) = unlinkat_raw(dir_fd.as_raw_fd(), stage_name) {
                            return Err(unlink_err);
                        }
                    }
                    Err(exchange_err) if renameat2_flag_unsupported(&exchange_err) => {
                        let _ = unlinkat_raw(dir_fd.as_raw_fd(), stage_name);
                        return Err(atomic_replace_unsupported(
                            target_name,
                            format!(
                                "renameat2(RENAME_EXCHANGE) unsupported for existing target: {exchange_err}"
                            ),
                        ));
                    }
                    Err(exchange_err) => {
                        let _ = unlinkat_raw(dir_fd.as_raw_fd(), stage_name);
                        return Err(exchange_err);
                    }
                }
            }
            Err(err) if renameat2_flag_unsupported(&err) => {
                let _ = unlinkat_raw(dir_fd.as_raw_fd(), stage_name);
                return Err(atomic_replace_unsupported(
                    target_name,
                    format!("renameat2(RENAME_NOREPLACE) unsupported for safe install: {err}"),
                ));
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
    ) -> io::Result<()> {
        let target_name = cstring_from_name(target_name)?;
        for _ in 0..64 {
            let stage_name = cstring_from_name(&next_hidden_name("stage"))?;
            let stage_fd = match openat_raw(
                dir_fd.as_raw_fd(),
                &stage_name,
                libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_CREAT | libc::O_EXCL,
                mode,
            ) {
                Ok(fd) => fd,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            };
            let mut stage_file = File::from(stage_fd);
            if let Err(err) = write_temp_file(&mut stage_file, contents, mode) {
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
    ) -> io::Result<()> {
        let mut tmp_file = create_anonymous_tmpfile(dir_fd, mode)?;
        write_temp_file(&mut tmp_file, contents, mode)?;
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
                            continue
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

    /// W3fu1 H1 (security-3): `openat2(O_NOFOLLOW | RESOLVE_BENEATH)`
    /// helper used by every W3 filesystem-mutating broker op per
    /// plan.md §"W3 filesystem path-safety tests" ("must use
    /// fd-relative openat2 with O_NOFOLLOW + RESOLVE_BENEATH").
    /// `dirfd` anchors the resolution; `path` must be a relative
    /// path that cannot escape the dirfd subtree.
    pub fn open_at(
        dirfd: BorrowedFd<'_>,
        path: &Path,
        flags: rustix::fs::OFlags,
    ) -> io::Result<OwnedFd> {
        use rustix::fs::{openat2, Mode, OFlags, ResolveFlags};
        openat2(
            dirfd,
            path,
            flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(io_from_rustix)
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
            RESOLVE_BENEATH
                | RESOLVE_NO_SYMLINKS
                | RESOLVE_NO_MAGICLINKS
                | RESOLVE_NO_XDEV,
        )
    }

    /// W3fu1 H1 (security-3): fd-based `fchmod` wrapper around
    /// rustix; replaces every path-string `chmod` call site in
    /// W3 broker fs ops.
    pub fn fchmod(fd: BorrowedFd<'_>, mode: u32) -> io::Result<()> {
        use rustix::fs::{fchmod as rfchmod, Mode};
        rfchmod(fd, Mode::from_raw_mode(mode)).map_err(io_from_rustix)
    }

    /// W3fu1 H1 (security-3): fd-based `fchown` via the safe `nix`
    /// wrapper. The broker crate keeps `unsafe_code = "deny"`, so we
    /// route through `nix::unistd::fchown` (which has a safe
    /// signature) rather than `rustix::fs::fchown` (whose `Uid` /
    /// `Gid` constructors are `unsafe fn`).
    pub fn fchown(fd: BorrowedFd<'_>, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
        let res = nix::unistd::fchown(
            fd.as_raw_fd(),
            uid.map(nix::unistd::Uid::from_raw),
            gid.map(nix::unistd::Gid::from_raw),
        );
        res.map_err(|err| io::Error::from_raw_os_error(err as i32))
    }

    /// Open a directory with `openat2` anchored at `/` and the full
    /// hardening mask:
    ///
    /// - `RESOLVE_BENEATH` keeps resolution anchored below `/` and
    ///   rejects `..` escapes;
    /// - `RESOLVE_NO_SYMLINKS` rejects symlink path components;
    /// - `RESOLVE_NO_MAGICLINKS` rejects procfs-style magic links; and
    /// - `RESOLVE_NO_XDEV` rejects mount-point crossings so a bind-mount
    ///   swap cannot redirect the walk into a different filesystem.
    pub fn open_dir_path_safe(dir: &Path) -> io::Result<OwnedFd> {
        if !dir.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("directory must be absolute: {}", dir.display()),
            ));
        }

        let root_fd: OwnedFd = File::open("/")?.into();
        let relative = dir.strip_prefix("/").map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("directory must be absolute: {}", dir.display()),
            )
        })?;
        if relative.as_os_str().is_empty() {
            return Ok(root_fd);
        }
        let relative = cstring_from_path(relative)?;
        openat2_raw(
            root_fd.as_raw_fd(),
            &relative,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0,
            RESOLVE_BENEATH
                | RESOLVE_NO_SYMLINKS
                | RESOLVE_NO_MAGICLINKS
                | RESOLVE_NO_XDEV,
        )
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
        match atomic_replace_via_linked_tmp(dir_fd, target_name, contents, mode) {
            Ok(()) => Ok(()),
            Err(err) if proc_link_fallback_allowed(&err) => {
                atomic_replace_via_named_stage(dir_fd, target_name, contents, mode)
            }
            Err(err) => Err(err),
        }
    }

    fn ensure_dir_path_safe_inner(
        parent_fd: &OwnedFd,
        name: &str,
        mode: u32,
        owner_uid: Option<u32>,
        owner_gid: Option<u32>,
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
                fchmod(fd.as_fd(), mode)?;
                if owner_uid.is_some() || owner_gid.is_some() {
                    fchown(fd.as_fd(), owner_uid, owner_gid)?;
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
        fchmod(fd.as_fd(), mode)?;
        if owner_uid.is_some() || owner_gid.is_some() {
            fchown(fd.as_fd(), owner_uid, owner_gid)?;
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
        ensure_dir_path_safe_inner(parent_fd, name, mode, Some(owner_uid), Some(owner_gid))
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
    /// W3fu1 H1 (security-3): `mkdirat` on a parent dirfd.
    /// `EEXIST` is folded into `Ok(())` so callers can use this as
    /// a one-shot idempotent dir create.
    pub fn mkdir_at(parent_dirfd: BorrowedFd<'_>, name: &Path, mode: u32) -> io::Result<()> {
        use rustix::fs::{mkdirat, Mode};
        match mkdirat(parent_dirfd, name, Mode::from_raw_mode(mode)) {
            Ok(()) => Ok(()),
            Err(err) if err == rustix::io::Errno::EXIST => Ok(()),
            Err(err) => Err(io_from_rustix(err)),
        }
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
}

/// Quarantined syscall surface for the W3 pidfd handoff contract
/// (plan.md §"W3 pidfd handoff contract").
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
/// between fork and pidfd_open is non-zero (the child could exit
/// and its pid be reused before pidfd_open returns) but is
/// vanishingly small in practice; W3 documents it in the
/// `pidfd.rs` doc-comment.
pub mod pidfd_sys {
    use std::ffi::CString;
    use std::io;
    use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
    use std::path::Path;

    use nix::libc;
    use nixling_core::minijail_profile::{MountPolicy, NamespaceSet};

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
    pub struct SpawnOutcome {
        pub pid: i32,
        pub pidfd: OwnedFd,
        pub used_fork_fallback: bool,
    }

    /// Drive `clone3(CLONE_PIDFD)`; falls back to `fork(2)` +
    /// `pidfd_open(2)` on `ENOSYS`/`EINVAL`/`E2BIG`. The closure
    /// `child_main` is invoked in the child; the parent receives the
    /// pidfd. The child is expected to `execve` shortly; if
    /// `child_main` returns the child process exits with the returned
    /// code.
    #[allow(unsafe_code)]
    pub fn clone3_pidfd_or_fork_fallback<F>(
        extra_clone_flags: u64,
        mut child_main: F,
    ) -> io::Result<SpawnOutcome>
    where
        F: FnMut() -> i32,
    {
        let mut pidfd: libc::c_int = -1;
        let mut args = CloneArgs {
            flags: (libc::CLONE_PIDFD as u64) | extra_clone_flags,
            // The kernel writes the pidfd into the i32 pointed to by
            // `pidfd` (treated as a u64 in clone_args).
            pidfd: &mut pidfd as *mut libc::c_int as u64,
            exit_signal: libc::SIGCHLD as u64,
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
            // Child.
            let code = child_main();
            // SAFETY: `_exit` is async-signal-safe; we have not
            // touched any stdlib state that requires `exit(3)`.
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
            let code = child_main();
            // SAFETY: see clone3 branch above.
            unsafe { libc::_exit(code) };
        }
        let fd = pidfd_open(pid as i32, 0)?;
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

    /// Read `/proc/<pid>/stat` field 22 (start time in clock ticks).
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
    }

    pub struct RunnerIsolationSpec {
        pub capabilities: Vec<String>,
        pub namespaces: NamespaceSet,
        pub seccomp_program: Option<SeccompProgram>,
        pub mount_policy: MountPolicy,
        pub cgroup_procs_fd: Option<OwnedFd>,
    }

    struct PreparedMountAction {
        path: CString,
        readonly: bool,
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
                ))
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
        let mut out = Vec::with_capacity(by_path.len());
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
        Ok(out)
    }

    fn clone3_namespace_flags(namespaces: &NamespaceSet) -> u64 {
        let mut flags = 0u64;
        if namespaces.pid {
            flags |= libc::CLONE_NEWPID as u64;
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
    fn write_self_to_cgroup(fd: RawFd) -> io::Result<()> {
        let pid = unsafe { libc::getpid() } as u32;
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
                        (libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY) as libc::c_ulong,
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
    const CHILD_EXIT_CGROUP: libc::c_int = 63;
    const CHILD_EXIT_MOUNT: libc::c_int = 64;
    const CHILD_EXIT_CAPSET: libc::c_int = 65;
    const CHILD_EXIT_SECCOMP: libc::c_int = 66;
    const CHILD_EXIT_SETGROUPS: libc::c_int = 70;
    const CHILD_EXIT_SETGID: libc::c_int = 71;
    const CHILD_EXIT_SETUID: libc::c_int = 72;
    const CHILD_EXIT_EXECVE: libc::c_int = 73;

    /// W4-fu: spawn a per-role runner with namespace / seccomp /
    /// capability setup plus `setgroups` + `setgid` + `setuid` +
    /// `execve` in a single `clone3_pidfd_or_fork_fallback` call.
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
        if isolation.namespaces.user {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "SpawnRunner user namespaces require uid/gid map wiring and are not yet supported",
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
        let mount_actions = if mount_required {
            prepare_mount_actions(&isolation.mount_policy)?
        } else {
            Vec::new()
        };
        let unshare_flags = unshare_namespace_flags(&isolation.namespaces, mount_required);
        let extra_clone_flags = clone3_namespace_flags(&isolation.namespaces);
        let argv_ptrs = &argv_ptrs_storage;
        let env_ptrs = &env_ptrs_storage;
        let supplementary_groups = &supplementary_groups_storage;
        let capabilities = &capability_numbers;
        let mount_actions = &mount_actions;
        let seccomp_program = isolation.seccomp_program;
        let cgroup_procs_fd = isolation.cgroup_procs_fd;
        let gid = gid as libc::gid_t;
        let uid = uid as libc::uid_t;

        clone3_pidfd_or_fork_fallback(extra_clone_flags, move || unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
                libc::_exit(CHILD_EXIT_PRCTL_NO_NEW_PRIVS);
            }
            if !capabilities.is_empty() && libc::prctl(libc::PR_SET_KEEPCAPS, 1, 0, 0, 0) < 0 {
                libc::_exit(CHILD_EXIT_PRCTL_KEEP_CAPS);
            }
            if unshare_flags != 0 && libc::unshare(unshare_flags) < 0 {
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
                    libc::_exit(CHILD_EXIT_MOUNT);
                }
                if apply_mount_actions(mount_actions).is_err() {
                    libc::_exit(CHILD_EXIT_MOUNT);
                }
            }
            if let Some(fd) = cgroup_procs_fd.as_ref() {
                if write_self_to_cgroup(fd.as_raw_fd()).is_err() {
                    libc::_exit(CHILD_EXIT_CGROUP);
                }
            }
            if libc::setgroups(supplementary_groups.len(), supplementary_groups.as_ptr()) < 0 {
                libc::_exit(CHILD_EXIT_SETGROUPS);
            }
            if libc::setgid(gid) < 0 {
                libc::_exit(CHILD_EXIT_SETGID);
            }
            if libc::setuid(uid) < 0 {
                libc::_exit(CHILD_EXIT_SETUID);
            }
            if !capabilities.is_empty() && apply_capabilities(capabilities).is_err() {
                libc::_exit(CHILD_EXIT_CAPSET);
            }
            if let Some(program) = seccomp_program.as_ref() {
                if apply_seccomp(program).is_err() {
                    libc::_exit(CHILD_EXIT_SECCOMP);
                }
            }
            libc::execve(binary_ptr, argv_ptrs.as_ptr(), env_ptrs.as_ptr());
            libc::_exit(CHILD_EXIT_EXECVE);
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};

    use tempfile::tempdir;

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
}
