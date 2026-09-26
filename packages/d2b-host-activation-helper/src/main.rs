use std::collections::BTreeSet;
use std::env;
use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process;

#[derive(Debug)]
struct Config {
    root: PathBuf,
    legacy_gids: BTreeSet<libc::gid_t>,
    target_gid: libc::gid_t,
    skip_while_lock_held: Option<PathBuf>,
    fail_closed: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: d2b-host-activation-helper chgrp-by-numeric-gid --root PATH --legacy-gids GID[,GID...] --target-gid GID [--skip-while-lock-held PATH] [--fail-closed]"
    );
    process::exit(64);
}

fn parse_gid(s: &str) -> Result<libc::gid_t, String> {
    let value: u64 = s.parse().map_err(|_| format!("invalid gid: {s}"))?;
    if value > libc::gid_t::MAX as u64 {
        return Err(format!("gid out of range: {s}"));
    }
    Ok(value as libc::gid_t)
}

fn parse_args() -> Result<Config, String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("chgrp-by-numeric-gid") => {}
        _ => usage(),
    }

    let mut root = None;
    let mut legacy_gids = BTreeSet::new();
    let mut target_gid = None;
    let mut skip_while_lock_held = None;
    let mut fail_closed = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--legacy-gids" => {
                let raw = args.next().ok_or("--legacy-gids requires a value")?;
                for part in raw.split(',').filter(|p| !p.is_empty()) {
                    legacy_gids.insert(parse_gid(part)?);
                }
            }
            "--target-gid" => {
                target_gid = Some(parse_gid(
                    &args.next().ok_or("--target-gid requires a value")?,
                )?)
            }
            "--skip-while-lock-held" => skip_while_lock_held = args.next().map(PathBuf::from),
            "--fail-closed" => fail_closed = true,
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    let root = root.ok_or("--root is required")?;
    let target_gid = target_gid.ok_or("--target-gid is required")?;
    if legacy_gids.is_empty() {
        return Err("--legacy-gids must name at least one gid".to_string());
    }
    Ok(Config {
        root,
        legacy_gids,
        target_gid,
        skip_while_lock_held,
        fail_closed,
    })
}

fn cstring_path(path: &std::path::Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

fn cstring_name(name: &[u8]) -> io::Result<CString> {
    CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry contains NUL"))
}

fn last_errno() -> io::Error {
    io::Error::last_os_error()
}

fn lock_is_held(path: &std::path::Path) -> io::Result<bool> {
    let c_path = cstring_path(path)?;
    // SAFETY: `c_path` is a `CString` whose pointer is NUL-terminated and valid for the call; the returned fd is checked for < 0 before any use.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        let err = last_errno();
        return if err.kind() == io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(err)
        };
    }
    let mut flock = libc::flock {
        l_type: libc::F_RDLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    // SAFETY: fd is a valid open descriptor (checked >= 0 above); `flock` is fully initialized before the call, and the result is checked before use.
    let rc = unsafe { libc::fcntl(fd, libc::F_OFD_SETLK, &mut flock) };
    let saved = last_errno();
    // SAFETY: fd is a valid open descriptor from the open above; it is closed exactly once here after the fcntl result was captured.
    unsafe { libc::close(fd) };
    if rc == 0 {
        Ok(false)
    } else if matches!(
        saved.raw_os_error(),
        Some(libc::EAGAIN) | Some(libc::EACCES)
    ) {
        Ok(true)
    } else {
        Err(saved)
    }
}

fn open_root(path: &std::path::Path) -> io::Result<libc::c_int> {
    let c_path = cstring_path(path)?;
    // SAFETY: `c_path` is a `CString` whose pointer is NUL-terminated and valid for the call; the returned fd is checked for < 0 before use, and the caller owns it.
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 { Err(last_errno()) } else { Ok(fd) }
}

fn gid_for_fd(fd: libc::c_int) -> io::Result<libc::gid_t> {
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fd is a valid open descriptor in the caller's ownership (checked when it was opened); `st` is only read via `assume_init` after the return value is checked for success below.
    let rc = unsafe { libc::fstat(fd, st.as_mut_ptr()) };
    if rc != 0 {
        return Err(last_errno());
    }
    // SAFETY: the preceding fstat returned 0, so `st` was initialized by the kernel before `assume_init()` reads it.
    Ok(unsafe { st.assume_init() }.st_gid)
}

fn entry_stat(parent_fd: libc::c_int, name: &CStr) -> io::Result<libc::stat> {
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: parent_fd is a valid open directory fd; `name` is a `CStr` (NUL-terminated, valid for the call); `st` is written by the kernel and only read via `assume_init` after the return value is checked for success.
    let rc = unsafe {
        libc::fstatat(
            parent_fd,
            name.as_ptr(),
            st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        return Err(last_errno());
    }
    // SAFETY: the preceding fstatat returned 0, so `st` was initialized by the kernel before `assume_init()` reads it.
    Ok(unsafe { st.assume_init() })
}

fn is_dir(mode: libc::mode_t) -> bool {
    (mode & libc::S_IFMT) == libc::S_IFDIR
}
fn is_symlink(mode: libc::mode_t) -> bool {
    (mode & libc::S_IFMT) == libc::S_IFLNK
}

fn format_chgrp_log_line(path: &Path, old_gid: libc::gid_t, new_gid: libc::gid_t) -> String {
    format!(
        "d2b-group-migration: chgrp path={} old_gid={} new_gid={}",
        path.display(),
        old_gid,
        new_gid
    )
}

fn log_chgrp(path: &Path, old_gid: libc::gid_t, new_gid: libc::gid_t) {
    eprintln!("{}", format_chgrp_log_line(path, old_gid, new_gid));
}

fn walk_dir(
    fd: libc::c_int,
    path: &Path,
    cfg: &Config,
    migrate: bool,
    leftovers: &mut u64,
) -> io::Result<()> {
    // SAFETY: fd is a valid open descriptor in the caller's ownership (checked when it was opened); `dup` only aliases it for the duration of the call, and its own result is checked below before use.
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return Err(last_errno());
    }
    // SAFETY: dup_fd was checked >= 0 above; fdopendir takes ownership of dup_fd only on success (on failure we close it below, and the returned DIR* is checked for null before use.
    let dir = unsafe { libc::fdopendir(dup_fd) };
    if dir.is_null() {
        let err = last_errno();
        // SAFETY: dup_fd was checked >= 0 above; fdopendir failed, so the fd is still owned by us, and must be closed to avoid a leak.
        unsafe { libc::close(dup_fd) };
        return Err(err);
    }

    loop {
        errno_clear();
        // SAFETY: dir is a valid non-null DIR* from fdopendir above; errno was cleared before the call so a null result can be distinguished from end-of-stream, and the result is checked for null before dereference.
        let ent = unsafe { libc::readdir(dir) };
        if ent.is_null() {
            let err = last_errno();
            // SAFETY: dir is a valid DIR* from fdopendir, and we stop reading before closing it, so no entry is dereferenced after the close.
            unsafe { libc::closedir(dir) };
            return if err.raw_os_error() == Some(0) {
                Ok(())
            } else {
                Err(err)
            };
        }
        // SAFETY: ent is non-null (checked above); d_name is a NUL-terminated char array inside a valid dirent, guaranteed by readdir's contract; the CStr is used only within this iteration, before the next readdir or closedir.
        let name_c = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
        let name = name_c.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let name_owned = cstring_name(name)?;
        let st = match entry_stat(fd, &name_owned) {
            Ok(st) => st,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if is_symlink(st.st_mode) {
            continue;
        }
        if cfg.legacy_gids.contains(&st.st_gid) {
            if migrate {
                let entry_path = path.join(std::ffi::OsStr::from_bytes(name));
                // SAFETY: fd is a valid open directory descriptor; `name_owned` is a NUL-terminated `CString` valid for the call; the return value is checked for error before use.
                let rc = unsafe {
                    libc::fchownat(
                        fd,
                        name_owned.as_ptr(),
                        libc::uid_t::MAX,
                        cfg.target_gid,
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                };
                if rc != 0 {
                    return Err(last_errno());
                }
                log_chgrp(&entry_path, st.st_gid, cfg.target_gid);
            } else {
                *leftovers += 1;
            }
        }
        if is_dir(st.st_mode) {
            // SAFETY: fd is a valid open directory descriptor; `name_owned` is a NUL-terminated `CString`; the returned fd is checked for < 0 before use, and later closed exactly once here.
            let child_fd = unsafe {
                libc::openat(
                    fd,
                    name_owned.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if child_fd < 0 {
                let err = last_errno();
                if err.kind() == io::ErrorKind::NotFound {
                    continue;
                }
                return Err(err);
            }
            let child_path = path.join(std::ffi::OsStr::from_bytes(name));
            let result = walk_dir(child_fd, &child_path, cfg, migrate, leftovers);
            // SAFETY: child_fd was checked >= 0 when opened above; walk_dir dups rather than consumes its descriptors, so it is still owned here, and closed exactly once after the recursive walk completes.
            unsafe { libc::close(child_fd) };
            result?;
        }
    }
}

fn errno_clear() {
    // SAFETY: __errno_location() returns a valid pointer to the calling thread's errno, which is a plain int; writing 0 to it is always sound.
    unsafe {
        *libc::__errno_location() = 0;
    }
}

fn run(cfg: Config) -> io::Result<i32> {
    if let Some(lock) = &cfg.skip_while_lock_held
        && lock_is_held(lock)?
    {
        eprintln!("d2b-group-migration: {lock:?} is locked; skipping legacy gid migration");
        if !cfg.fail_closed {
            return Ok(0);
        }
        let mut leftovers = 0;
        scan_for_leftovers(&cfg, &mut leftovers)?;
        return Ok(if leftovers > 0 {
            eprintln!(
                "d2b-group-migration: {leftovers} entries still have a legacy gid under {:?}",
                cfg.root
            );
            1
        } else {
            0
        });
    }

    let root_fd = open_root(&cfg.root)?;
    if cfg.legacy_gids.contains(&gid_for_fd(root_fd)?) {
        let old_gid = gid_for_fd(root_fd)?;
        // SAFETY: root_fd is a valid open directory fd from open_root (checked >= 0); the return value is checked for error before proceeding.
        let rc = unsafe { libc::fchown(root_fd, libc::uid_t::MAX, cfg.target_gid) };
        if rc != 0 {
            let err = last_errno();
            // SAFETY: root_fd is a valid open descriptor (checked >= 0 at open_root), and is still owned by us on this error path, so it must be closed to avoid a leak.
            unsafe { libc::close(root_fd) };
            return Err(err);
        }
        log_chgrp(&cfg.root, old_gid, cfg.target_gid);
    }
    let mut migration_leftovers = 0;
    let result = walk_dir(root_fd, &cfg.root, &cfg, true, &mut migration_leftovers);
    // SAFETY: root_fd is a valid open descriptor from open_root; walk_dir dups rather than closes its arguments, so it is still owned here, and closed exactly once after the walk completes.
    unsafe { libc::close(root_fd) };
    result?;

    let mut leftovers = 0;
    if cfg.fail_closed {
        scan_for_leftovers(&cfg, &mut leftovers)?;
    }
    if cfg.fail_closed && leftovers > 0 {
        eprintln!(
            "d2b-group-migration: {leftovers} entries still have a legacy gid under {:?}",
            cfg.root
        );
        Ok(1)
    } else {
        Ok(0)
    }
}

fn scan_for_leftovers(cfg: &Config, leftovers: &mut u64) -> io::Result<()> {
    let root_fd = open_root(&cfg.root)?;
    let result = (|| {
        if cfg.legacy_gids.contains(&gid_for_fd(root_fd)?) {
            *leftovers += 1;
        }
        walk_dir(root_fd, &cfg.root, cfg, false, leftovers)
    })();
    // SAFETY: root_fd is a valid open descriptor from open_root; walk_dir dups rather than closes its arguments, so it is still owned here, and closed exactly once after the walk completes.
    unsafe { libc::close(root_fd) };
    result
}

fn main() {
    let cfg = parse_args().unwrap_or_else(|err| {
        eprintln!("d2b-host-activation-helper: {err}");
        usage();
    });
    match run(cfg) {
        Ok(code) => process::exit(code),
        Err(err) => {
            eprintln!("d2b-host-activation-helper: {err}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::AsRawFd;
    use tempfile::tempdir;

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn migrate_then_fail_closed_scan_reopens_root_for_full_walk() {
        let dir = tempdir().expect("test dir");
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("legacy"), b"legacy").expect("legacy file");

        let current_gid =
            fs::metadata(nested.join("legacy")).expect("metadata").gid() as libc::gid_t;
        let cfg = Config {
            root: dir.path().to_path_buf(),
            legacy_gids: BTreeSet::from([current_gid]),
            target_gid: current_gid,
            skip_while_lock_held: None,
            fail_closed: false,
        };

        let migrate_fd = open_root(&cfg.root).expect("open migrate root");
        let mut migrate_leftovers = 0;
        walk_dir(migrate_fd, &cfg.root, &cfg, true, &mut migrate_leftovers)
            .expect("migration walk");
        unsafe { libc::close(migrate_fd) };

        let mut scan_leftovers = 0;
        scan_for_leftovers(&cfg, &mut scan_leftovers).expect("fail-closed scan");

        assert_eq!(migrate_leftovers, 0);
        assert_eq!(scan_leftovers, 3);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn held_lock_fail_closed_runs_postscan_and_exits_nonzero() {
        let dir = tempdir().expect("test dir");
        let legacy_path = dir.path().join("legacy");
        fs::write(&legacy_path, b"legacy").expect("legacy file");
        let lock_path = dir.path().join("migration.lock");
        let lock_file = fs::File::create(&lock_path).expect("lock file");
        let mut flock = libc::flock {
            l_type: libc::F_WRLCK as libc::c_short,
            l_whence: libc::SEEK_SET as libc::c_short,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };
        let rc = unsafe { libc::fcntl(lock_file.as_raw_fd(), libc::F_OFD_SETLK, &mut flock) };
        assert_eq!(rc, 0, "hold migration lock");

        let current_gid = fs::metadata(&legacy_path).expect("metadata").gid() as libc::gid_t;
        let target_gid = if current_gid == 0 { 1 } else { 0 };
        let cfg = Config {
            root: dir.path().to_path_buf(),
            legacy_gids: BTreeSet::from([current_gid]),
            target_gid,
            skip_while_lock_held: Some(lock_path),
            fail_closed: true,
        };

        assert!(
            lock_is_held(
                cfg.skip_while_lock_held
                    .as_ref()
                    .expect("configured lock path")
            )
            .expect("lock probe")
        );
        let exit_code = run(cfg).expect("run migration");

        assert_ne!(exit_code, 0);
    }

    #[test]
    fn cstring_helpers_reject_embedded_nul_bytes() {
        assert!(cstring_path(Path::new("/etc/hosts")).is_ok());
        let err = cstring_path(Path::new("/etc/hosts\0evil")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(cstring_name(b"entry").is_ok());
        let err = cstring_name(b"entry\0evil").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn parse_gid_refuses_non_numeric_and_out_of_range_inputs() {
        assert_eq!(parse_gid("0"), Ok(0));
        assert_eq!(parse_gid("65534"), Ok(65534));
        assert!(parse_gid("abc").is_err());
        assert!(parse_gid("4294967296").is_err());
        assert!(parse_gid("-1").is_err());
    }

}
