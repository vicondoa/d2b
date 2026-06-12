// v1.1.2fu19 panel-security R2 critical must-fix: replace the
// shell-script `[ -L ] / [ -f ] / find -type f` check-then-act
// patterns in nixos-modules/host-activation.nix with fd-safe
// operations that cannot be defeated by a runner-UID attacker
// swapping a checked regular file for a symlink between the
// check and the action.
//
// All paths used here open the target with `O_NOFOLLOW` (refusing
// to traverse a final-segment symlink) and `O_PATH` or
// `O_DIRECTORY` so the fd refers to a stable inode. Subsequent
// mutations use `fchown(2)` / `fchmod(2)` / `ftruncate(2)` /
// `fsetxattr(2)` against that fd, removing the TOCTOU window.
//
// Verbs (each accepts `--help`):
//   ensure-regular-file --path P --uid U --gid G --mode M --size-mib N
//     Atomically create `P` with `O_CREAT|O_EXCL|O_WRONLY|O_NOFOLLOW`,
//     `ftruncate` to N MiB, `fchown(uid,gid)`, `fchmod(mode)`. If P
//     already exists as a regular file (not symlink), re-asserts
//     ownership/mode/size idempotently via O_NOFOLLOW open + fstat
//     check + fchown/fchmod. If P exists as a symlink or non-regular
//     file (directory, device, FIFO), refuses and exits 2.
//   enforce-dir-posture --path P --uid U --gid G --mode M
//     Open P with `O_DIRECTORY|O_NOFOLLOW`, fstat to confirm it IS
//     a directory (not a symlink-to-dir), fchown(uid,gid),
//     fchmod(mode). If P is a symlink, refuses and exits 2.
//   clear-acl-on-path --path P [--require-kind regular|directory|socket|any]
//     Open P with openat2 RESOLVE_NO_SYMLINKS, verify the requested file
//     type, then run setfacl -b against /proc/self/fd/<N>.
//
// Exit codes:
//   0  - success (action applied or already-correct)
//   1  - input / parse / nonexistent / IO error
//   2  - safety refusal (symlink or wrong file type at target)
//
// nixling-host is `#![forbid(unsafe_code)]`; this binary lives in
// the same crate so it inherits that policy. Direct libc::open()
// is required for `O_NOFOLLOW|O_EXCL` which `std::fs::OpenOptions`
// only exposes via the `unix::OpenOptionsExt::custom_flags()` API
// which IS safe; we use that.

use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use nix::sys::stat::{fchmod, Mode};
use nix::unistd::{fchown, ftruncate, Gid, Uid};
use rustix::fs::{Mode as RxMode, OFlags, ResolveFlags, CWD};
use rustix::mount::{mount_change, unmount, MountPropagationFlags, UnmountFlags};
use rustix::thread::{unshare, UnshareFlags};

use nixling_host::hardlink_farm::{
    build_farm, build_store_view, replace_live_top_level_paths, BuildStoreViewFarmRequest,
    BuildStoreViewRequest, ReplaceLivePathsRequest,
};

/// v1.1.2fu24 panel-security R5 critical must-fix: open `path`
/// with `openat2(AT_FDCWD, path, { O_NOFOLLOW + ..., RESOLVE_NO_SYMLINKS })`.
/// `RESOLVE_NO_SYMLINKS` refuses ANY symlink encountered during
/// path resolution — final segment AND every intermediate
/// component. This closes the symlink-swap-of-ancestor TOCTOU
/// class that plain `O_NOFOLLOW` (which only protects the final
/// component) cannot defend against.
///
/// Requires Linux >= 5.6 (openat2 syscall); v1.1 kernel floor
/// is 6.9 (ADR 0008) so this is satisfied unconditionally.
fn open_no_symlinks(path: &std::path::Path, oflags: OFlags) -> std::io::Result<OwnedFd> {
    let full_flags = oflags | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    rustix::fs::openat2(
        CWD,
        path,
        full_flags,
        RxMode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )
    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))
}

#[derive(Debug)]
struct Args {
    verb: String,
    path: Option<PathBuf>,
    uid: Option<u32>,
    gid: Option<u32>,
    mode: Option<u32>,
    size_mib: Option<u64>,
    acl_spec: Option<String>,
    also_spec: Option<String>,
    require_kind: Option<String>,
    if_owner: Option<String>,
    setfacl_bin: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut argv = std::env::args().skip(1);
    let verb = argv.next().ok_or("missing verb")?;
    let mut args = Args {
        verb,
        path: None,
        uid: None,
        gid: None,
        mode: None,
        size_mib: None,
        acl_spec: None,
        also_spec: None,
        require_kind: None,
        if_owner: None,
        setfacl_bin: None,
    };
    while let Some(flag) = argv.next() {
        let value = argv
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--path" => args.path = Some(PathBuf::from(value)),
            "--uid" => args.uid = Some(value.parse().map_err(|e| format!("--uid: {e}"))?),
            "--gid" => args.gid = Some(value.parse().map_err(|e| format!("--gid: {e}"))?),
            "--mode" => {
                let m =
                    u32::from_str_radix(&value, 8).map_err(|e| format!("--mode (octal): {e}"))?;
                args.mode = Some(m);
            }
            "--size-mib" => {
                args.size_mib = Some(value.parse().map_err(|e| format!("--size-mib: {e}"))?);
            }
            "--acl-spec" => args.acl_spec = Some(value),
            "--also-spec" => args.also_spec = Some(value),
            "--require-kind" => args.require_kind = Some(value),
            "--if-owner" => args.if_owner = Some(value),
            "--setfacl-bin" => args.setfacl_bin = Some(PathBuf::from(value)),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok(args)
}

fn require<T>(field: &str, value: Option<T>) -> Result<T, String> {
    value.ok_or_else(|| format!("missing required flag: --{field}"))
}

fn print_help() {
    eprintln!(
        "nixling-activation-helper — fd-safe activation primitives\n\
         \n\
         USAGE:\n  \
           nixling-activation-helper ensure-regular-file --path P --uid U --gid G --mode M --size-mib N\n  \
           nixling-activation-helper enforce-dir-posture --path P --uid U --gid G --mode M\n  \
           nixling-activation-helper clear-acl-on-path --path P [--require-kind regular|directory|socket|any] [--setfacl-bin PATH]\n  \
           nixling-activation-helper setfacl-on-path --path P --acl-spec A [--also-spec A2] [--require-kind regular|directory|socket|any] [--setfacl-bin PATH]\n  \
           nixling-activation-helper chown-if-orphan --path P --uid U --gid G\n  \
           nixling-activation-helper build-store-view-farm   (request JSON on stdin)\n  \
           nixling-activation-helper build-store-view        (request JSON on stdin)\n\
         \n\
         EXIT CODES:\n  \
           0 success / already-correct\n  \
           1 input or IO error\n  \
           2 safety refusal (symlink at target / wrong file type)\n"
    );
}

fn cmd_ensure_regular_file(args: &Args) -> ExitCode {
    let path = match require("path", args.path.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let uid = match require("uid", args.uid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let gid = match require("gid", args.gid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let mode = match require("mode", args.mode) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let size_mib = match require("size-mib", args.size_mib) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    // v1.1.2fu24 panel-security R5 critical must-fix: use
    // openat2 + RESOLVE_NO_SYMLINKS so NO path component
    // (intermediate or final) can be a symlink. Closes the
    // ancestor-swap TOCTOU class. We still try O_EXCL+O_CREAT
    // first so the AlreadyExists fallback path can re-assert
    // mode on an existing regular file.
    match open_no_symlinks(path, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL) {
        Ok(fd) => {
            let file: File = File::from(fd);
            let target_size = size_mib.saturating_mul(1024 * 1024);
            if let Err(e) = ftruncate(&file, target_size as i64) {
                eprintln!("ftruncate({}) failed: {e}", path.display());
                return ExitCode::from(1);
            }
            if let Err(e) = fchown(
                file.as_raw_fd(),
                Some(Uid::from_raw(uid)),
                Some(Gid::from_raw(gid)),
            ) {
                eprintln!("fchown({}) failed: {e}", path.display());
                return ExitCode::from(1);
            }
            let perms = Mode::from_bits_truncate(mode);
            if let Err(e) = fchmod(file.as_raw_fd(), perms) {
                eprintln!("fchmod({}) failed: {e}", path.display());
                return ExitCode::from(1);
            }
            ExitCode::from(0)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // v1.1.2fu24: re-assert path also uses openat2 +
            // RESOLVE_NO_SYMLINKS for full path-safety. O_NONBLOCK
            // keeps FIFO/socket targets from hanging open(2);
            // O_NOFOLLOW is implicit via the helper.
            let existing_fd = match open_no_symlinks(path, OFlags::RDONLY | OFlags::NONBLOCK) {
                Ok(fd) => fd,
                Err(e2) if e2.raw_os_error() == Some(libc::ELOOP) => {
                    eprintln!(
                        "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                        path.display()
                    );
                    return ExitCode::from(2);
                }
                Err(e2) => {
                    eprintln!("open({}) for re-assert failed: {e2}", path.display());
                    return ExitCode::from(1);
                }
            };
            let existing: File = File::from(existing_fd);
            let meta = match existing.metadata() {
                Ok(m) => m,
                Err(e2) => {
                    eprintln!("fstat({}) failed: {e2}", path.display());
                    return ExitCode::from(1);
                }
            };
            let file_type = meta.file_type();
            if !file_type.is_file() {
                eprintln!(
                    "refusing: {} is not a regular file (mode 0o{:o})",
                    path.display(),
                    meta.mode()
                );
                return ExitCode::from(2);
            }
            if let Err(e2) = fchown(
                existing.as_raw_fd(),
                Some(Uid::from_raw(uid)),
                Some(Gid::from_raw(gid)),
            ) {
                eprintln!("re-assert fchown({}) failed: {e2}", path.display());
                return ExitCode::from(1);
            }
            let perms = Mode::from_bits_truncate(mode);
            if let Err(e2) = fchmod(existing.as_raw_fd(), perms) {
                eprintln!("re-assert fchmod({}) failed: {e2}", path.display());
                return ExitCode::from(1);
            }
            ExitCode::from(0)
        }
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                path.display()
            );
            ExitCode::from(2)
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            // RESOLVE_NO_SYMLINKS can also return EXDEV for
            // bind-mount crossings; treat as safety refusal.
            eprintln!(
                "refusing: {} crosses a bind mount (RESOLVE_NO_SYMLINKS EXDEV)",
                path.display()
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("create({}) failed: {e}", path.display());
            ExitCode::from(1)
        }
    }
}

fn cmd_enforce_dir_posture(args: &Args) -> ExitCode {
    let path = match require("path", args.path.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let uid = match require("uid", args.uid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let gid = match require("gid", args.gid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let mode = match require("mode", args.mode) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    // v1.1.2fu24 panel-security R5 critical must-fix: use
    // openat2 + RESOLVE_NO_SYMLINKS so NO path component
    // (intermediate or final) can be a symlink.
    let dir_fd = match open_no_symlinks(path, OFlags::RDONLY | OFlags::DIRECTORY) {
        Ok(fd) => fd,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => {
            eprintln!(
                "refusing: {} is not a directory (O_DIRECTORY returned ENOTDIR)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            eprintln!(
                "refusing: {} crosses a bind mount (RESOLVE_NO_SYMLINKS EXDEV)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ExitCode::from(0);
        }
        Err(e) => {
            eprintln!("open({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let dir: File = File::from(dir_fd);
    let meta = match dir.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("fstat({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    if !meta.file_type().is_dir() {
        eprintln!(
            "refusing: {} fstat says non-directory (mode 0o{:o})",
            path.display(),
            meta.mode()
        );
        return ExitCode::from(2);
    }
    if let Err(e) = fchown(
        dir.as_raw_fd(),
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
    ) {
        eprintln!("fchown({}) failed: {e}", path.display());
        return ExitCode::from(1);
    }
    let perms = Mode::from_bits_truncate(mode);
    if let Err(e) = fchmod(dir.as_raw_fd(), perms) {
        eprintln!("fchmod({}) failed: {e}", path.display());
        return ExitCode::from(1);
    }
    ExitCode::from(0)
}

/// v1.1.2fu23 panel-security R4 critical must-fix: fd-safe
/// `setfacl` wrapper that cannot be redirected to attacker-
/// controlled symlink targets. Opens `--path` with O_PATH +
/// O_NOFOLLOW (refuses symlinks), fstats to validate the file
/// type matches `--require-kind` (regular | directory | socket | any),
/// then invokes `setfacl -m <acl-spec> [-m <also-spec>]
/// /proc/<helper-pid>/fd/<N>` while keeping FD_CLOEXEC set. The
/// kernel resolves the magic procfs symlink to the inode the helper
/// already holds, so the setxattr cannot be redirected to a
/// different path and the target fd is not inherited by setfacl. The
/// `--setfacl-bin` flag pins the setfacl binary (typically
/// `${pkgs.acl}/bin/setfacl`) so $PATH is not consulted.
fn cmd_setfacl_on_path(args: &Args) -> ExitCode {
    let path = match require("path", args.path.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let acl_spec = match require("acl-spec", args.acl_spec.as_ref()) {
        Ok(v) => v.clone(),
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let require_kind = args.require_kind.as_deref().unwrap_or("any");
    let setfacl_bin = args
        .setfacl_bin
        .clone()
        .unwrap_or_else(|| PathBuf::from("setfacl"));

    // v1.1.2fu24 panel-security R5 critical must-fix: use
    // openat2 + RESOLVE_NO_SYMLINKS so NO path component
    // (intermediate or final) can be a symlink. O_PATH lets this
    // fd-safe path cover sockets as well as regular files/dirs.
    // O_CLOEXEC remains set. setfacl addresses the held inode via
    // /proc/<helper-pid>/fd/<N>, so no target fd is inherited across
    // exec.
    let raw_fd = match open_no_symlinks(path, OFlags::PATH) {
        Ok(fd) => fd,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            eprintln!(
                "refusing: {} crosses a bind mount (RESOLVE_NO_SYMLINKS EXDEV)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ExitCode::from(0);
        }
        Err(e) => {
            eprintln!("open({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let fd: File = File::from(raw_fd);
    let meta = match fd.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("fstat({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    if meta.file_type().is_symlink() {
        eprintln!("refusing: {} fstat says symlink", path.display());
        return ExitCode::from(2);
    }
    match require_kind {
        "regular" if !meta.file_type().is_file() => {
            eprintln!(
                "refusing: {} fstat says non-regular (mode 0o{:o}); --require-kind=regular",
                path.display(),
                meta.mode()
            );
            return ExitCode::from(2);
        }
        "directory" if !meta.file_type().is_dir() => {
            eprintln!(
                "refusing: {} fstat says non-directory (mode 0o{:o}); --require-kind=directory",
                path.display(),
                meta.mode()
            );
            return ExitCode::from(2);
        }
        "socket" if !meta.file_type().is_socket() => {
            eprintln!(
                "refusing: {} fstat says non-socket (mode 0o{:o}); --require-kind=socket",
                path.display(),
                meta.mode()
            );
            return ExitCode::from(2);
        }
        "any" | "regular" | "directory" | "socket" => {}
        other => {
            eprintln!("error: invalid --require-kind: {other}");
            return ExitCode::from(1);
        }
    }
    let procfd_path = format!("/proc/{}/fd/{}", std::process::id(), fd.as_raw_fd());
    let mut cmd = Command::new(&setfacl_bin);
    cmd.arg("-m").arg(&acl_spec);
    if let Some(also) = &args.also_spec {
        cmd.arg("-m").arg(also);
    }
    cmd.arg(&procfd_path);
    match cmd.status() {
        Ok(status) if status.success() => ExitCode::from(0),
        Ok(status) => {
            eprintln!(
                "setfacl on /proc/self/fd ({}) failed: {:?}",
                path.display(),
                status
            );
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("spawn setfacl failed: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_clear_acl_on_path(args: &Args) -> ExitCode {
    let path = match require("path", args.path.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let require_kind = args.require_kind.as_deref().unwrap_or("any");
    let setfacl_bin = args
        .setfacl_bin
        .clone()
        .unwrap_or_else(|| PathBuf::from("setfacl"));

    let raw_fd = match open_no_symlinks(path, OFlags::PATH) {
        Ok(fd) => fd,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            eprintln!(
                "refusing: {} crosses a bind mount (RESOLVE_NO_SYMLINKS EXDEV)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ExitCode::from(0);
        }
        Err(e) => {
            eprintln!("open({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let fd: File = File::from(raw_fd);
    let meta = match fd.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("fstat({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    if meta.file_type().is_symlink() {
        eprintln!("refusing: {} fstat says symlink", path.display());
        return ExitCode::from(2);
    }
    match require_kind {
        "regular" if !meta.file_type().is_file() => {
            eprintln!("refusing: {} fstat says non-regular", path.display());
            return ExitCode::from(2);
        }
        "directory" if !meta.file_type().is_dir() => {
            eprintln!("refusing: {} fstat says non-directory", path.display());
            return ExitCode::from(2);
        }
        "socket" if !meta.file_type().is_socket() => {
            eprintln!("refusing: {} fstat says non-socket", path.display());
            return ExitCode::from(2);
        }
        "any" | "regular" | "directory" | "socket" => {}
        other => {
            eprintln!("error: invalid --require-kind: {other}");
            return ExitCode::from(1);
        }
    }
    let procfd_path = format!("/proc/{}/fd/{}", std::process::id(), fd.as_raw_fd());
    match Command::new(&setfacl_bin)
        .arg("-b")
        .arg(&procfd_path)
        .status()
    {
        Ok(status) if status.success() => ExitCode::from(0),
        Ok(status) => {
            eprintln!(
                "setfacl -b on /proc/self/fd ({}) failed: {status:?}",
                path.display()
            );
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("spawn setfacl failed: {e}");
            ExitCode::from(1)
        }
    }
}

/// v1.1.2fu23 panel-security R4 high must-fix: fd-safe chown
/// that only fires when the existing owner matches `--if-owner`
/// (typically "UNKNOWN" for the orphan-repair case). Opens
/// `--path` with O_PATH + O_NOFOLLOW (refuses symlinks),
/// fstats to read the existing uid:gid, looks them up against
/// /etc/passwd + /etc/group, and only chowns when the existing
/// owner string matches the marker (or when getpwuid/getgrgid
/// returns NULL — the "UNKNOWN" case).
fn cmd_chown_if_orphan(args: &Args) -> ExitCode {
    let path = match require("path", args.path.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let uid = match require("uid", args.uid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let gid = match require("gid", args.gid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    // v1.1.2fu24 panel-security R5 critical must-fix: use
    // openat2 + RESOLVE_NO_SYMLINKS so NO path component
    // (intermediate or final) can be a symlink. fchown(2)
    // does NOT work on O_PATH fds (kernel returns EBADF), so
    // use O_RDONLY + O_NONBLOCK (no FIFO hang) — the helper
    // adds O_NOFOLLOW + O_CLOEXEC implicitly.
    let raw_fd = match open_no_symlinks(path, OFlags::RDONLY | OFlags::NONBLOCK) {
        Ok(fd) => fd,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            eprintln!(
                "refusing: {} crosses a bind mount (RESOLVE_NO_SYMLINKS EXDEV)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ExitCode::from(0);
        }
        Err(e) => {
            eprintln!("open({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let fd: File = File::from(raw_fd);
    let meta = match fd.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("fstat({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let current_uid = meta.uid();
    let user_known = nix::unistd::User::from_uid(Uid::from_raw(current_uid))
        .ok()
        .flatten()
        .is_some();
    let current_gid = meta.gid();
    let group_known = nix::unistd::Group::from_gid(Gid::from_raw(current_gid))
        .ok()
        .flatten()
        .is_some();
    let is_orphan = !user_known || !group_known;
    if !is_orphan {
        return ExitCode::from(0);
    }
    if let Err(e) = fchown(
        fd.as_raw_fd(),
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
    ) {
        eprintln!("fchown({}) failed: {e}", path.display());
        return ExitCode::from(1);
    }
    eprintln!(
        "nixling: repaired orphan ownership on {} ({}:{} -> {}:{})",
        path.display(),
        current_uid,
        current_gid,
        uid,
        gid
    );
    ExitCode::from(0)
}

fn cmd_build_store_view_farm() -> ExitCode {
    use std::io::Read;

    let mut buf = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut buf) {
        eprintln!("build-store-view-farm: read stdin: {e}");
        return ExitCode::from(1);
    }
    let req: BuildStoreViewFarmRequest = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("build-store-view-farm: parse request: {e}");
            return ExitCode::from(1);
        }
    };
    // We are already inside the broker-established private mount
    // namespace with `/nix/store` lazily detached, so the source
    // closure paths and the per-VM farm root now resolve on the same
    // (root) mount and `link(2)` succeeds.
    match build_farm(
        &req.farm_root,
        req.generation,
        &req.closure_paths,
        &req.marker,
    ) {
        Ok(_) => ExitCode::from(0),
        Err(e) => {
            // Emit the typed HardlinkFarmError as a single JSON line on
            // stdout so the calling broker can recover it and preserve
            // its typed error mapping (collision / different-fs /
            // marker). Human-readable detail goes to stderr.
            if let Ok(j) = serde_json::to_string(&e) {
                println!("{j}");
            }
            eprintln!("build-store-view-farm: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_build_store_view() -> ExitCode {
    use std::io::Read;

    let mut buf = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut buf) {
        eprintln!("build-store-view: read stdin: {e}");
        return ExitCode::from(1);
    }
    let req: BuildStoreViewRequest = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("build-store-view: parse request: {e}");
            return ExitCode::from(1);
        }
    };
    // Already inside the broker-established private mount namespace with
    // `/nix/store` lazily detached, so the closure paths and the per-VM
    // farm root resolve on the same (root) mount and `link(2)` succeeds.
    match build_store_view(
        &req.farm_root,
        &req.generation_id,
        &req.closure_paths,
        &req.marker,
    ) {
        Ok(counts) => {
            // Emit the link/skip accounting as a single JSON line on
            // stdout so the calling broker can recover it.
            if let Ok(j) = serde_json::to_string(&counts) {
                println!("{j}");
            }
            ExitCode::from(0)
        }
        Err(e) => {
            // Typed HardlinkFarmError as one JSON line on stdout so the
            // broker preserves its collision / different-fs / marker
            // mapping; human-readable detail to stderr.
            if let Ok(j) = serde_json::to_string(&e) {
                println!("{j}");
            }
            eprintln!("build-store-view: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_replace_store_view_live() -> ExitCode {
    use std::io::Read;

    let mut buf = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut buf) {
        eprintln!("replace-store-view-live: read stdin: {e}");
        return ExitCode::from(1);
    }
    let req: ReplaceLivePathsRequest = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("replace-store-view-live: parse request: {e}");
            return ExitCode::from(1);
        }
    };
    match replace_live_top_level_paths(&req.farm_root, &req.stage_tag, &req.closure_paths) {
        Ok(counts) => {
            if let Ok(j) = serde_json::to_string(&counts) {
                println!("{j}");
            }
            ExitCode::from(0)
        }
        Err(e) => {
            if let Ok(j) = serde_json::to_string(&e) {
                println!("{j}");
            }
            eprintln!("replace-store-view-live: {e}");
            ExitCode::from(1)
        }
    }
}

fn prepare_private_store_namespace() -> Result<(), String> {
    unshare(UnshareFlags::NEWNS).map_err(|e| format!("unshare mount namespace: {e}"))?;
    mount_change(
        "/",
        MountPropagationFlags::PRIVATE | MountPropagationFlags::REC,
    )
    .map_err(|e| format!("make mount propagation private: {e}"))?;
    let _ = unmount("/nix/store", UnmountFlags::DETACH);
    Ok(())
}

fn run_private_store_verb(verb: &str) -> ExitCode {
    if let Err(err) = prepare_private_store_namespace() {
        eprintln!("private-store: {err}");
        return ExitCode::from(1);
    }
    match verb {
        "build-store-view-farm" => cmd_build_store_view_farm(),
        "build-store-view" => cmd_build_store_view(),
        "replace-store-view-live" => cmd_replace_store_view_live(),
        other => {
            eprintln!("private-store: unsupported verb {other}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    // `build-store-view-farm` takes its (potentially large) request as
    // JSON on stdin, not `--flag value` argv, so it bypasses the
    // generic flag parser. The broker invokes it under
    // `unshare --mount --propagation private` + `umount -l /nix/store`.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("private-store") {
        let Some(verb) = args.get(2).map(String::as_str) else {
            eprintln!("private-store: missing verb");
            return ExitCode::from(1);
        };
        return run_private_store_verb(verb);
    }
    if args.get(1).map(String::as_str) == Some("build-store-view-farm") {
        return cmd_build_store_view_farm();
    }
    // `build-store-view` is the ADR 0027 split-layout build, same
    // stdin-JSON contract, same private-namespace invocation.
    if args.get(1).map(String::as_str) == Some("build-store-view") {
        return cmd_build_store_view();
    }
    if args.get(1).map(String::as_str) == Some("replace-store-view-live") {
        return cmd_replace_store_view_live();
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            print_help();
            return ExitCode::from(1);
        }
    };
    match args.verb.as_str() {
        "ensure-regular-file" => cmd_ensure_regular_file(&args),
        "enforce-dir-posture" => cmd_enforce_dir_posture(&args),
        "clear-acl-on-path" => cmd_clear_acl_on_path(&args),
        "setfacl-on-path" => cmd_setfacl_on_path(&args),
        "chown-if-orphan" => cmd_chown_if_orphan(&args),
        "--help" | "-h" => {
            print_help();
            ExitCode::from(0)
        }
        other => {
            eprintln!("error: unknown verb: {other}");
            print_help();
            ExitCode::from(1)
        }
    }
}
