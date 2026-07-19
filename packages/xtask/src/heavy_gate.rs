use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use command_fds::{CommandFdExt, FdMapping};
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, fcntl},
    libc,
};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use signal_hook::{
    consts::{SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM},
    flag,
    iterator::Signals,
};

const SLOT_COUNT: usize = 2;
const SLOT_NAMES: [&str; SLOT_COUNT] = ["slot-0.lock", "slot-1.lock"];
const GATE_DIRECTORY_MODE: u32 = 0o700;
const SLOT_MODE: u32 = 0o600;
const GATE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const GATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);
const GROUP_OBSERVATION_FAILURE_LIMIT: usize = 5;
const HEAVY_GATE_CHILD_FD: i32 = 198;
pub const HEAVY_GATE_FD_ENV: &str = "D2B_HEAVY_GATE_FD";

#[derive(Debug)]
struct HeavyGateError(String);

impl HeavyGateError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for HeavyGateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<std::io::Error> for HeavyGateError {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("I/O error: {error}"))
    }
}

struct VerifiedGateDirectory {
    parent: VerifiedParentDirectory,
    fd: OwnedFd,
    name: OsString,
    identity: FileIdentity,
}

struct VerifiedParentDirectory {
    fd: OwnedFd,
    path: PathBuf,
    identity: FileIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: libc::dev_t,
    inode: libc::ino_t,
}

struct VerifiedSlot {
    fd: OwnedFd,
    name: &'static str,
    identity: FileIdentity,
}

struct HeavyGatePermit {
    directory: VerifiedGateDirectory,
    fd: OwnedFd,
    name: &'static str,
    identity: FileIdentity,
    slot: usize,
}

impl HeavyGatePermit {
    fn duplicate_for_child(&self) -> Result<OwnedFd, HeavyGateError> {
        rustix::io::fcntl_dupfd_cloexec(&self.fd, 0)
            .map_err(|error| HeavyGateError::new(format!("cannot duplicate gate slot FD: {error}")))
    }

    fn verify_namespace(&self) -> Result<(), HeavyGateError> {
        self.directory.verify_anchor()?;
        verify_slot_anchor(&self.directory, self.name, &self.fd, self.identity)
    }
}

enum LockAttempt {
    Acquired,
    Contended,
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(status) => exit_code(status),
        Err(error) => {
            eprintln!("heavy gate failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<ExitStatus, HeavyGateError> {
    let [separator, program, command_args @ ..] = args else {
        return Err(HeavyGateError::new(
            "usage: cargo xtask heavy-gate -- <command> [args...]",
        ));
    };
    if separator != "--" || program.is_empty() {
        return Err(HeavyGateError::new(
            "usage: cargo xtask heavy-gate -- <command> [args...]",
        ));
    }

    let _sigchld_handler =
        flag::register(SIGCHLD, Arc::new(AtomicBool::new(false))).map_err(|error| {
            HeavyGateError::new(format!("cannot make gate children waitable: {error}"))
        })?;
    let mut signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM])
        .map_err(|error| HeavyGateError::new(format!("cannot install signal handlers: {error}")))?;
    let gate_directory = gate_directory_from_environment()?;
    let permit = acquire_permit(&gate_directory, GATE_TIMEOUT, GATE_POLL_INTERVAL, || {
        signals.pending().next()
    })?;
    eprintln!("heavy gate: acquired slot {}", permit.slot);

    let mapped_fd = permit.duplicate_for_child()?;
    let mut command = Command::new(program);
    command
        .args(command_args)
        .process_group(0)
        .env(HEAVY_GATE_FD_ENV, HEAVY_GATE_CHILD_FD.to_string())
        .fd_mappings(vec![FdMapping {
            parent_fd: mapped_fd,
            child_fd: HEAVY_GATE_CHILD_FD,
        }])
        .map_err(|error| HeavyGateError::new(format!("cannot map gate slot FD: {error}")))?;

    let mut child = command
        .spawn()
        .map_err(|error| HeavyGateError::new(format!("cannot execute {program}: {error}")))?;
    drop(command);
    let leader_pid = child_pid(&child)?;
    let pidfd = match rustix::process::pidfd_open(leader_pid, rustix::process::PidfdFlags::empty())
    {
        Ok(pidfd) => pidfd,
        Err(error) => {
            let failure = HeavyGateError::new(format!(
                "cannot obtain race-free gate child authority: {error}"
            ));
            return Err(terminate_after_failure(
                &mut child, leader_pid, permit, failure,
            ));
        }
    };

    wait_for_process_group(&mut child, leader_pid, &pidfd, &mut signals, permit)
}

fn gate_directory_from_environment() -> Result<PathBuf, HeavyGateError> {
    let path = match env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        Some(runtime) => PathBuf::from(runtime).join("d2b-heavy-gates"),
        None => {
            let parent = env::var_os("TMPDIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            parent.join(format!(
                "d2b-heavy-gates-{}",
                rustix::process::geteuid().as_raw()
            ))
        }
    };
    if !path.is_absolute() {
        return Err(HeavyGateError::new(
            "heavy gate directory parent must be absolute",
        ));
    }
    Ok(path)
}

fn acquire_permit(
    path: &Path,
    timeout: Duration,
    poll_interval: Duration,
    mut pending_signal: impl FnMut() -> Option<i32>,
) -> Result<HeavyGatePermit, HeavyGateError> {
    if timeout.is_zero() || poll_interval.is_zero() {
        return Err(HeavyGateError::new(
            "heavy gate timeout and poll interval must be nonzero",
        ));
    }
    let directory = open_verified_gate_directory(path)?;
    let started = Instant::now();
    let mut attempted = false;
    loop {
        if let Some(signal) = pending_signal() {
            return Err(HeavyGateError::new(format!(
                "interrupted by signal {signal} while waiting for a heavy gate slot"
            )));
        }
        if attempted && started.elapsed() >= timeout {
            return Err(HeavyGateError::new(format!(
                "timed out after {} seconds waiting for a heavy gate slot",
                timeout.as_secs()
            )));
        }
        attempted = true;
        for (slot, name) in SLOT_NAMES.iter().enumerate() {
            let verified_slot = open_verified_slot(&directory, name)?;
            match try_ofd_lock(&verified_slot.fd)? {
                LockAttempt::Acquired => {
                    verified_slot.verify_anchor(&directory)?;
                    return Ok(HeavyGatePermit {
                        directory,
                        fd: verified_slot.fd,
                        name: verified_slot.name,
                        identity: verified_slot.identity,
                        slot,
                    });
                }
                LockAttempt::Contended => {}
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Err(HeavyGateError::new(format!(
                "timed out after {} seconds waiting for a heavy gate slot",
                timeout.as_secs()
            )));
        }
        thread::sleep(poll_interval.min(timeout - elapsed));
    }
}

fn open_verified_gate_directory(path: &Path) -> Result<VerifiedGateDirectory, HeavyGateError> {
    let parent_path = path.parent().ok_or_else(|| {
        HeavyGateError::new("heavy gate path must have an absolute parent directory")
    })?;
    let name = path.file_name().ok_or_else(|| {
        HeavyGateError::new("heavy gate path must name a directory below its parent")
    })?;
    let parent = open_verified_parent_directory(parent_path)?;
    let created =
        match rustix::fs::mkdirat(&parent.fd, name, Mode::from_raw_mode(GATE_DIRECTORY_MODE)) {
            Ok(()) => true,
            Err(rustix::io::Errno::EXIST) => false,
            Err(error) => {
                return Err(HeavyGateError::new(format!(
                    "cannot create heavy gate directory {}: {error}",
                    path.display()
                )));
            }
        };
    let fd = rustix::fs::openat(
        &parent.fd,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        HeavyGateError::new(format!(
            "cannot open heavy gate directory {}: {error}",
            path.display()
        ))
    })?;
    if created {
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(GATE_DIRECTORY_MODE)).map_err(|error| {
            HeavyGateError::new(format!("cannot set heavy gate directory mode: {error}"))
        })?;
    }
    let after = rustix::fs::fstat(&fd).map_err(|error| {
        HeavyGateError::new(format!("cannot stat heavy gate directory: {error}"))
    })?;
    let uid = rustix::process::geteuid().as_raw();
    if !gate_directory_metadata_is_safe(&after, uid) {
        return Err(HeavyGateError::new(
            "heavy gate directory has unsafe ownership, type, or mode",
        ));
    }
    verify_cloexec(&fd, "heavy gate directory")?;
    let directory = VerifiedGateDirectory {
        parent,
        fd,
        name: name.to_os_string(),
        identity: file_identity(&after),
    };
    directory.verify_anchor()?;
    Ok(directory)
}

fn open_verified_slot(
    directory: &VerifiedGateDirectory,
    name: &'static str,
) -> Result<VerifiedSlot, HeavyGateError> {
    directory.verify_anchor()?;
    let common = OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let (fd, created) = match rustix::fs::openat(
        &directory.fd,
        name,
        common | OFlags::EXCL,
        Mode::from_raw_mode(SLOT_MODE),
    ) {
        Ok(fd) => (fd, true),
        Err(rustix::io::Errno::EXIST) => (
            rustix::fs::openat(&directory.fd, name, common, Mode::from_raw_mode(SLOT_MODE))
                .map_err(|error| {
                    HeavyGateError::new(format!("cannot open heavy gate slot {name}: {error}"))
                })?,
            false,
        ),
        Err(error) => {
            return Err(HeavyGateError::new(format!(
                "cannot create heavy gate slot {name}: {error}"
            )));
        }
    };
    if created {
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(SLOT_MODE)).map_err(|error| {
            HeavyGateError::new(format!("cannot set heavy gate slot {name} mode: {error}"))
        })?;
    }
    let stat = rustix::fs::fstat(&fd)
        .map_err(|error| HeavyGateError::new(format!("cannot stat gate slot {name}: {error}")))?;
    let uid = rustix::process::geteuid().as_raw();
    if !slot_metadata_is_safe(&stat, uid) {
        return Err(HeavyGateError::new(format!(
            "heavy gate slot {name} has unsafe ownership, type, mode, or link count"
        )));
    }
    verify_cloexec(&fd, "heavy gate slot")?;
    let slot = VerifiedSlot {
        fd,
        name,
        identity: file_identity(&stat),
    };
    slot.verify_anchor(directory)?;
    Ok(slot)
}

impl VerifiedParentDirectory {
    fn verify_anchor(&self) -> Result<(), HeavyGateError> {
        let pinned = rustix::fs::fstat(&self.fd).map_err(|error| {
            HeavyGateError::new(format!("cannot restat heavy gate parent: {error}"))
        })?;
        let uid = rustix::process::geteuid().as_raw();
        if !parent_directory_metadata_is_safe(&pinned, uid)
            || file_identity(&pinned) != self.identity
        {
            return Err(HeavyGateError::new(
                "heavy gate parent changed ownership, type, mode, or identity",
            ));
        }

        let current = open_parent_directory_fd(&self.path)?;
        let current_stat = rustix::fs::fstat(&current).map_err(|error| {
            HeavyGateError::new(format!("cannot inspect heavy gate parent path: {error}"))
        })?;
        if !parent_directory_metadata_is_safe(&current_stat, uid)
            || file_identity(&current_stat) != self.identity
        {
            return Err(HeavyGateError::new(
                "heavy gate parent path was renamed or replaced",
            ));
        }
        Ok(())
    }
}

impl VerifiedGateDirectory {
    fn verify_anchor(&self) -> Result<(), HeavyGateError> {
        self.parent.verify_anchor()?;
        let pinned = rustix::fs::fstat(&self.fd).map_err(|error| {
            HeavyGateError::new(format!("cannot restat heavy gate directory: {error}"))
        })?;
        let uid = rustix::process::geteuid().as_raw();
        if !gate_directory_metadata_is_safe(&pinned, uid) || file_identity(&pinned) != self.identity
        {
            return Err(HeavyGateError::new(
                "heavy gate directory changed ownership, type, mode, or identity",
            ));
        }
        let named = rustix::fs::statat(&self.parent.fd, &self.name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| {
                HeavyGateError::new(format!(
                    "cannot verify pinned heavy gate directory name: {error}"
                ))
            })?;
        if !gate_directory_metadata_is_safe(&named, uid) || file_identity(&named) != self.identity {
            return Err(HeavyGateError::new(
                "heavy gate directory name was renamed or replaced",
            ));
        }
        Ok(())
    }
}

impl VerifiedSlot {
    fn verify_anchor(&self, directory: &VerifiedGateDirectory) -> Result<(), HeavyGateError> {
        verify_slot_anchor(directory, self.name, &self.fd, self.identity)
    }
}

fn open_verified_parent_directory(path: &Path) -> Result<VerifiedParentDirectory, HeavyGateError> {
    if !path.is_absolute() {
        return Err(HeavyGateError::new(
            "heavy gate directory parent must be absolute",
        ));
    }
    let fd = open_parent_directory_fd(path)?;
    let stat = rustix::fs::fstat(&fd)
        .map_err(|error| HeavyGateError::new(format!("cannot stat heavy gate parent: {error}")))?;
    let uid = rustix::process::geteuid().as_raw();
    if !parent_directory_metadata_is_safe(&stat, uid) {
        return Err(HeavyGateError::new(
            "heavy gate parent must be an invoking-UID-owned non-writable directory or a root-owned sticky world-writable directory",
        ));
    }
    verify_cloexec(&fd, "heavy gate parent")?;
    let parent = VerifiedParentDirectory {
        fd,
        path: path.to_path_buf(),
        identity: file_identity(&stat),
    };
    parent.verify_anchor()?;
    Ok(parent)
}

fn open_parent_directory_fd(path: &Path) -> Result<OwnedFd, HeavyGateError> {
    rustix::fs::open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        HeavyGateError::new(format!(
            "cannot open heavy gate parent {} without following symlinks: {error}",
            path.display()
        ))
    })
}

fn parent_directory_metadata_is_safe(stat: &rustix::fs::Stat, uid: u32) -> bool {
    parent_directory_values_are_safe(
        FileType::from_raw_mode(stat.st_mode),
        stat.st_uid,
        stat.st_mode,
        uid,
    )
}

fn parent_directory_values_are_safe(file_type: FileType, owner: u32, mode: u32, uid: u32) -> bool {
    if file_type != FileType::Directory {
        return false;
    }
    let mode = mode & 0o7777;
    let invoking_uid_owned = owner == uid && mode & 0o022 == 0;
    let root_tmp_style = owner == 0 && mode & 0o1000 != 0 && mode & 0o002 != 0;
    invoking_uid_owned || root_tmp_style
}

fn gate_directory_metadata_is_safe(stat: &rustix::fs::Stat, uid: u32) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == uid
        && stat.st_mode & 0o7777 == GATE_DIRECTORY_MODE
}

fn slot_metadata_is_safe(stat: &rustix::fs::Stat, uid: u32) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
        && stat.st_uid == uid
        && stat.st_nlink == 1
        && stat.st_mode & 0o7777 == SLOT_MODE
}

fn file_identity(stat: &rustix::fs::Stat) -> FileIdentity {
    FileIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    }
}

fn verify_slot_anchor(
    directory: &VerifiedGateDirectory,
    name: &str,
    fd: &OwnedFd,
    identity: FileIdentity,
) -> Result<(), HeavyGateError> {
    directory.verify_anchor()?;
    let uid = rustix::process::geteuid().as_raw();
    let pinned = rustix::fs::fstat(fd)
        .map_err(|error| HeavyGateError::new(format!("cannot restat gate slot {name}: {error}")))?;
    let named =
        rustix::fs::statat(&directory.fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
            HeavyGateError::new(format!("cannot verify pinned gate slot {name}: {error}"))
        })?;
    if !slot_metadata_is_safe(&pinned, uid)
        || !slot_metadata_is_safe(&named, uid)
        || file_identity(&pinned) != identity
        || file_identity(&named) != identity
    {
        return Err(HeavyGateError::new(format!(
            "heavy gate slot {name} was renamed, replaced, or made unsafe"
        )));
    }
    Ok(())
}

fn verify_cloexec(fd: &OwnedFd, label: &str) -> Result<(), HeavyGateError> {
    let flags = rustix::fs::fcntl_getfd(fd).map_err(|error| {
        HeavyGateError::new(format!("cannot inspect {label} FD flags: {error}"))
    })?;
    if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
        return Err(HeavyGateError::new(format!(
            "{label} FD is not close-on-exec"
        )));
    }
    Ok(())
}

fn try_ofd_lock(fd: &OwnedFd) -> Result<LockAttempt, HeavyGateError> {
    let lock = libc::flock {
        l_type: libc::F_WRLCK as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    match fcntl(fd.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock)) {
        Ok(_) => Ok(LockAttempt::Acquired),
        Err(Errno::EAGAIN | Errno::EACCES) => Ok(LockAttempt::Contended),
        Err(error) => Err(HeavyGateError::new(format!(
            "OFD locking is unavailable for the heavy gate slot: {error}"
        ))),
    }
}

fn wait_for_process_group(
    child: &mut Child,
    leader_pid: rustix::process::Pid,
    pidfd: &OwnedFd,
    signals: &mut Signals,
    permit: HeavyGatePermit,
) -> Result<ExitStatus, HeavyGateError> {
    wait_for_process_group_in(
        child,
        leader_pid,
        pidfd,
        signals,
        Path::new("/proc"),
        permit,
    )
}

fn wait_for_process_group_in(
    child: &mut Child,
    leader_pid: rustix::process::Pid,
    pidfd: &OwnedFd,
    signals: &mut Signals,
    proc_root: &Path,
    permit: HeavyGatePermit,
) -> Result<ExitStatus, HeavyGateError> {
    let observed = observe_process_group(child, leader_pid, pidfd, signals, proc_root, &permit);
    match observed {
        Ok(status) => {
            drop(permit);
            Ok(status)
        }
        Err(failure) => Err(terminate_after_failure(child, leader_pid, permit, failure)),
    }
}

fn observe_process_group(
    child: &mut Child,
    leader_pid: rustix::process::Pid,
    pidfd: &OwnedFd,
    signals: &mut Signals,
    proc_root: &Path,
    permit: &HeavyGatePermit,
) -> Result<ExitStatus, HeavyGateError> {
    loop {
        permit.verify_namespace()?;
        forward_pending_signals(signals, leader_pid)?;
        let exited = rustix::process::waitid(
            rustix::process::WaitId::PidFd(pidfd.as_fd()),
            rustix::process::WaitidOptions::EXITED
                | rustix::process::WaitidOptions::NOHANG
                | rustix::process::WaitidOptions::NOWAIT,
        )
        .map_err(|error| HeavyGateError::new(format!("cannot observe gate child: {error}")))?
        .is_some();
        if exited {
            break;
        }
        thread::sleep(CHILD_POLL_INTERVAL);
    }

    while process_group_has_nonleader_members(leader_pid, proc_root)? {
        permit.verify_namespace()?;
        forward_pending_signals(signals, leader_pid)?;
        thread::sleep(CHILD_POLL_INTERVAL);
    }
    permit.verify_namespace()?;
    child
        .wait()
        .map_err(|error| HeavyGateError::new(format!("cannot reap gate child: {error}")))
}

fn forward_pending_signals(
    signals: &mut Signals,
    process_group: rustix::process::Pid,
) -> Result<(), HeavyGateError> {
    for signal in signals.pending() {
        if let Some(signal) = rustix_signal(signal) {
            match rustix::process::kill_process_group(process_group, signal) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => {
                    return Err(HeavyGateError::new(format!(
                        "cannot forward signal to gate process group: {error}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn rustix_signal(signal: i32) -> Option<rustix::process::Signal> {
    match signal {
        SIGHUP => Some(rustix::process::Signal::Hup),
        SIGINT => Some(rustix::process::Signal::Int),
        SIGQUIT => Some(rustix::process::Signal::Quit),
        SIGTERM => Some(rustix::process::Signal::Term),
        _ => None,
    }
}

fn process_group_has_nonleader_members(
    leader_pid: rustix::process::Pid,
    proc_root: &Path,
) -> Result<bool, HeavyGateError> {
    let leader = leader_pid.as_raw_nonzero().get();
    for entry in fs::read_dir(proc_root).map_err(|error| {
        HeavyGateError::new(format!("cannot inspect child process group: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            HeavyGateError::new(format!("cannot inspect child process group entry: {error}"))
        })?;
        let file_name = entry.file_name();
        let Some(pid) = parse_ascii_i32(file_name.as_os_str()) else {
            continue;
        };
        if pid == leader {
            continue;
        }
        let stat = match fs::read(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(HeavyGateError::new(format!(
                    "cannot inspect process-group member: {error}"
                )));
            }
        };
        let process_group = process_group_from_stat(&stat)
            .ok_or_else(|| HeavyGateError::new("cannot parse process-group member stat record"))?;
        if process_group == leader {
            return Ok(true);
        }
    }
    Ok(false)
}

fn process_group_from_stat(stat: &[u8]) -> Option<i32> {
    let command_end = stat.iter().rposition(|byte| *byte == b')')?;
    let process_group = stat
        .get(command_end + 1..)?
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
        .nth(2)?;
    parse_ascii_i32_bytes(process_group)
}

fn parse_ascii_i32(value: &OsStr) -> Option<i32> {
    parse_ascii_i32_bytes(value.as_bytes())
}

fn parse_ascii_i32_bytes(value: &[u8]) -> Option<i32> {
    if value.is_empty() {
        return None;
    }
    value.iter().try_fold(0_i32, |result, byte| {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit <= 9)?;
        result.checked_mul(10)?.checked_add(i32::from(digit))
    })
}

fn child_pid(child: &Child) -> Result<rustix::process::Pid, HeavyGateError> {
    i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| HeavyGateError::new("gate child PID is invalid"))
}

fn terminate_after_failure(
    child: &mut Child,
    process_group: rustix::process::Pid,
    permit: HeavyGatePermit,
    failure: HeavyGateError,
) -> HeavyGateError {
    let cleanup = terminate_group_and_reap(child, process_group);
    drop(permit);
    match cleanup {
        Ok(()) => failure,
        Err(cleanup_error) => HeavyGateError::new(format!(
            "{failure}; gate process-group cleanup also failed: {cleanup_error}"
        )),
    }
}

fn terminate_group_and_reap(
    child: &mut Child,
    process_group: rustix::process::Pid,
) -> Result<(), HeavyGateError> {
    let mut cleanup = OsProcessGroupCleanup {
        child,
        process_group,
    };
    terminate_group_with_anchor(&mut cleanup)
}

trait ProcessGroupCleanup {
    fn signal_group(&mut self) -> Result<(), HeavyGateError>;
    fn wait_for_leader_exit(&mut self) -> Result<(), HeavyGateError>;
    fn has_nonleader_members(&mut self) -> Result<bool, HeavyGateError>;
    fn reap_leader(&mut self) -> Result<(), HeavyGateError>;
    fn pause(&mut self);
}

struct OsProcessGroupCleanup<'a> {
    child: &'a mut Child,
    process_group: rustix::process::Pid,
}

impl ProcessGroupCleanup for OsProcessGroupCleanup<'_> {
    fn signal_group(&mut self) -> Result<(), HeavyGateError> {
        match rustix::process::kill_process_group(self.process_group, rustix::process::Signal::Kill)
        {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(HeavyGateError::new(format!(
                "cannot terminate gate process group: {error}"
            ))),
        }
    }

    fn wait_for_leader_exit(&mut self) -> Result<(), HeavyGateError> {
        loop {
            match rustix::process::waitid(
                rustix::process::WaitId::Pid(self.process_group),
                rustix::process::WaitidOptions::EXITED | rustix::process::WaitidOptions::NOWAIT,
            ) {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => {
                    return Err(HeavyGateError::new(
                        "gate process-group leader exit was not observable",
                    ));
                }
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => {
                    return Err(HeavyGateError::new(format!(
                        "cannot preserve gate process-group leader identity: {error}"
                    )));
                }
            }
        }
    }

    fn has_nonleader_members(&mut self) -> Result<bool, HeavyGateError> {
        process_group_has_nonleader_members(self.process_group, Path::new("/proc"))
    }

    fn reap_leader(&mut self) -> Result<(), HeavyGateError> {
        self.child.wait().map(|_| ()).map_err(|error| {
            HeavyGateError::new(format!("cannot reap gate child after termination: {error}"))
        })
    }

    fn pause(&mut self) {
        thread::sleep(CHILD_POLL_INTERVAL);
    }
}

fn terminate_group_with_anchor(
    cleanup: &mut impl ProcessGroupCleanup,
) -> Result<(), HeavyGateError> {
    let mut first_error = None;
    if let Err(error) = cleanup.signal_group() {
        remember_cleanup_error(&mut first_error, error.to_string());
    }
    if let Err(error) = cleanup.wait_for_leader_exit() {
        remember_cleanup_error(&mut first_error, error.to_string());
        if let Err(error) = cleanup.reap_leader() {
            remember_cleanup_error(&mut first_error, error.to_string());
        }
        return Err(HeavyGateError::new(
            first_error.expect("leader-anchor failure was recorded"),
        ));
    }

    let mut observation_failures = 0;
    loop {
        match cleanup.has_nonleader_members() {
            Ok(false) => break,
            Ok(true) => {
                observation_failures = 0;
                if let Err(error) = cleanup.signal_group() {
                    remember_cleanup_error(&mut first_error, error.to_string());
                }
            }
            Err(error) => {
                remember_cleanup_error(&mut first_error, error.to_string());
                observation_failures += 1;
                if observation_failures >= GROUP_OBSERVATION_FAILURE_LIMIT {
                    if let Err(error) = cleanup.signal_group() {
                        remember_cleanup_error(&mut first_error, error.to_string());
                    }
                    break;
                }
            }
        }
        cleanup.pause();
    }

    // Reaping releases PID/PGID reuse protection. No group operation may follow.
    if let Err(error) = cleanup.reap_leader() {
        remember_cleanup_error(&mut first_error, error.to_string());
    }

    match first_error {
        Some(error) => Err(HeavyGateError::new(error)),
        None => Ok(()),
    }
}

fn remember_cleanup_error(target: &mut Option<String>, error: String) {
    if target.is_none() {
        *target = Some(error);
    }
}

fn exit_code(status: ExitStatus) -> ExitCode {
    let code = status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
        .clamp(0, 255) as u8;
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        os::unix::fs::{PermissionsExt, symlink},
        process::Stdio,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);
    const GROUP_TEST_ROLE: &str = "D2B_HEAVY_GATE_UNIT_GROUP_ROLE";
    const GROUP_TEST_DESCENDANT: &str = "D2B_HEAVY_GATE_UNIT_GROUP_DESCENDANT";

    #[derive(Debug, Eq, PartialEq)]
    enum CleanupEvent {
        Signal,
        WaitForLeaderExit,
        InspectMembers,
        ReapLeader,
    }

    enum MemberObservation {
        Present,
        Empty,
        Failure,
    }

    struct ReuseAwareCleanup {
        events: Vec<CleanupEvent>,
        member_observations: VecDeque<MemberObservation>,
        leader_reaped: bool,
        touched_reused_group: bool,
    }

    impl ReuseAwareCleanup {
        fn group_operation(&mut self, event: CleanupEvent) {
            if self.leader_reaped {
                self.touched_reused_group = true;
            }
            self.events.push(event);
        }
    }

    impl ProcessGroupCleanup for ReuseAwareCleanup {
        fn signal_group(&mut self) -> Result<(), HeavyGateError> {
            self.group_operation(CleanupEvent::Signal);
            Ok(())
        }

        fn wait_for_leader_exit(&mut self) -> Result<(), HeavyGateError> {
            self.group_operation(CleanupEvent::WaitForLeaderExit);
            Ok(())
        }

        fn has_nonleader_members(&mut self) -> Result<bool, HeavyGateError> {
            self.group_operation(CleanupEvent::InspectMembers);
            match self
                .member_observations
                .pop_front()
                .unwrap_or(MemberObservation::Empty)
            {
                MemberObservation::Present => Ok(true),
                MemberObservation::Empty => Ok(false),
                MemberObservation::Failure => {
                    Err(HeavyGateError::new("injected process-table failure"))
                }
            }
        }

        fn reap_leader(&mut self) -> Result<(), HeavyGateError> {
            self.events.push(CleanupEvent::ReapLeader);
            self.leader_reaped = true;
            Ok(())
        }

        fn pause(&mut self) {}
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::var_os("D2B_VALIDATION_OUTPUT_DIR")
                .map(PathBuf::from)
                .map(|root| root.join("rust-test-scratch/xtask-heavy-gate"))
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-scratch")
                });
            fs::create_dir_all(&root).expect("scratch root");
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .expect("scratch root mode");
            let path = root.join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("scratch");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("scratch mode");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn production_gate_shape_is_exact() {
        assert_eq!(SLOT_NAMES, ["slot-0.lock", "slot-1.lock"]);
        assert_eq!(GATE_POLL_INTERVAL, Duration::from_millis(250));
        assert_eq!(GATE_TIMEOUT, Duration::from_secs(30 * 60));
        assert_eq!(GATE_DIRECTORY_MODE, 0o700);
        assert_eq!(SLOT_MODE, 0o600);
        assert_eq!(GROUP_OBSERVATION_FAILURE_LIMIT, 5);
        assert!(parent_directory_values_are_safe(
            FileType::Directory,
            1000,
            0o700,
            1000
        ));
        assert!(parent_directory_values_are_safe(
            FileType::Directory,
            1000,
            0o755,
            1000
        ));
        assert!(parent_directory_values_are_safe(
            FileType::Directory,
            0,
            0o1777,
            1000
        ));
        assert!(!parent_directory_values_are_safe(
            FileType::Directory,
            1000,
            0o770,
            1000
        ));
        assert!(!parent_directory_values_are_safe(
            FileType::Directory,
            0,
            0o0777,
            1000
        ));
        assert!(!parent_directory_values_are_safe(
            FileType::Directory,
            0,
            0o1770,
            1000
        ));
    }

    #[test]
    fn two_slots_are_ofd_locked_and_original_fds_are_cloexec() {
        let scratch = Scratch::new("slots");
        let gate = scratch.0.join("gate");
        let first = acquire_permit(
            &gate,
            Duration::from_secs(1),
            Duration::from_millis(5),
            || None,
        )
        .expect("first permit");
        let second = acquire_permit(
            &gate,
            Duration::from_secs(1),
            Duration::from_millis(5),
            || None,
        )
        .expect("second permit");
        assert_ne!(first.slot, second.slot);
        verify_cloexec(&first.fd, "first permit").expect("CLOEXEC");
        verify_cloexec(&second.fd, "second permit").expect("CLOEXEC");
        let error = acquire_permit(
            &gate,
            Duration::from_millis(25),
            Duration::from_millis(5),
            || None,
        )
        .err()
        .expect("third permit must time out");
        assert!(error.to_string().contains("timed out"));
        drop(first);
        acquire_permit(
            &gate,
            Duration::from_secs(1),
            Duration::from_millis(5),
            || None,
        )
        .expect("released slot");
    }

    #[test]
    fn unsafe_directory_and_slot_metadata_fail_closed() {
        let scratch = Scratch::new("metadata");
        let wrong_mode = scratch.0.join("wrong-mode");
        fs::create_dir(&wrong_mode).expect("directory");
        fs::set_permissions(&wrong_mode, fs::Permissions::from_mode(0o750)).expect("mode");
        assert!(
            acquire_permit(
                &wrong_mode,
                Duration::from_millis(20),
                Duration::from_millis(5),
                || None
            )
            .is_err()
        );

        let target = scratch.0.join("target");
        fs::create_dir(&target).expect("target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).expect("target mode");
        let link = scratch.0.join("link");
        symlink(&target, &link).expect("symlink");
        assert!(
            acquire_permit(
                &link,
                Duration::from_millis(20),
                Duration::from_millis(5),
                || None
            )
            .is_err()
        );

        let hardlink_gate = scratch.0.join("hardlink-gate");
        fs::create_dir(&hardlink_gate).expect("gate");
        fs::set_permissions(
            &hardlink_gate,
            fs::Permissions::from_mode(GATE_DIRECTORY_MODE),
        )
        .expect("gate mode");
        fs::write(hardlink_gate.join(SLOT_NAMES[0]), b"").expect("slot");
        fs::set_permissions(
            hardlink_gate.join(SLOT_NAMES[0]),
            fs::Permissions::from_mode(SLOT_MODE),
        )
        .expect("slot mode");
        fs::hard_link(
            hardlink_gate.join(SLOT_NAMES[0]),
            hardlink_gate.join("alias.lock"),
        )
        .expect("hard link");
        assert!(
            acquire_permit(
                &hardlink_gate,
                Duration::from_millis(20),
                Duration::from_millis(5),
                || None
            )
            .is_err()
        );
    }

    #[test]
    fn unsafe_parent_modes_and_symlinks_fail_closed() {
        let scratch = Scratch::new("unsafe-parent");
        let unsafe_parent = scratch.0.join("unsafe");
        fs::create_dir(&unsafe_parent).expect("unsafe parent");
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o770))
            .expect("unsafe parent mode");
        assert!(
            acquire_permit(
                &unsafe_parent.join("gate"),
                Duration::from_millis(20),
                Duration::from_millis(5),
                || None
            )
            .is_err()
        );

        let target = scratch.0.join("parent-target");
        fs::create_dir(&target).expect("parent target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
            .expect("parent target mode");
        let parent_link = scratch.0.join("parent-link");
        symlink(&target, &parent_link).expect("parent symlink");
        assert!(
            acquire_permit(
                &parent_link.join("gate"),
                Duration::from_millis(20),
                Duration::from_millis(5),
                || None
            )
            .is_err()
        );
    }

    #[test]
    fn pinned_parent_gate_and_slot_reject_namespace_replacement() {
        let scratch = Scratch::new("namespace-replacement");

        let parent = scratch.0.join("parent");
        fs::create_dir(&parent).expect("parent");
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).expect("parent mode");
        let gate = parent.join("gate");
        let pinned_parent = open_verified_gate_directory(&gate).expect("pinned parent");
        let moved_parent = scratch.0.join("parent-moved");
        fs::rename(&parent, &moved_parent).expect("rename parent");
        fs::create_dir(&parent).expect("replacement parent");
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
            .expect("replacement parent mode");
        assert!(
            open_verified_slot(&pinned_parent, SLOT_NAMES[0]).is_err(),
            "a renamed parent must not create a second lock namespace"
        );

        let gate_parent = scratch.0.join("gate-parent");
        fs::create_dir(&gate_parent).expect("gate parent");
        fs::set_permissions(&gate_parent, fs::Permissions::from_mode(0o700))
            .expect("gate parent mode");
        let gate = gate_parent.join("gate");
        let pinned_gate = open_verified_gate_directory(&gate).expect("pinned gate");
        fs::rename(&gate, gate_parent.join("gate-moved")).expect("rename gate");
        fs::create_dir(&gate).expect("replacement gate");
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o700))
            .expect("replacement gate mode");
        assert!(
            open_verified_slot(&pinned_gate, SLOT_NAMES[0]).is_err(),
            "a renamed gate must not create a second lock namespace"
        );

        let slot_parent = scratch.0.join("slot-parent");
        fs::create_dir(&slot_parent).expect("slot parent");
        fs::set_permissions(&slot_parent, fs::Permissions::from_mode(0o700))
            .expect("slot parent mode");
        let slot_gate_path = slot_parent.join("gate");
        let slot_gate =
            open_verified_gate_directory(&slot_gate_path).expect("slot replacement gate");
        let slot = open_verified_slot(&slot_gate, SLOT_NAMES[0]).expect("pinned slot");
        let slot_path = slot_gate_path.join(SLOT_NAMES[0]);
        fs::rename(&slot_path, slot_gate_path.join("slot-moved")).expect("rename slot");
        fs::write(&slot_path, b"").expect("replacement slot");
        fs::set_permissions(&slot_path, fs::Permissions::from_mode(SLOT_MODE))
            .expect("replacement slot mode");
        assert!(
            slot.verify_anchor(&slot_gate).is_err(),
            "a renamed slot must not create a second lock namespace"
        );
    }

    #[test]
    fn proc_stat_parser_handles_non_utf8_spaces_and_parentheses() {
        assert_eq!(
            process_group_from_stat(b"123 (a command) S 1 77 77 0 -1"),
            Some(77)
        );
        assert_eq!(
            process_group_from_stat(b"123 (a ) command) S 1 88 88 0 -1"),
            Some(88)
        );
        assert_eq!(
            process_group_from_stat(b"123 (a \xff ) command) S 1 99 99 0 -1"),
            Some(99)
        );
    }

    #[test]
    fn cleanup_never_touches_reused_pgid_after_leader_reap() {
        let mut cleanup = ReuseAwareCleanup {
            events: Vec::new(),
            member_observations: VecDeque::from([
                MemberObservation::Present,
                MemberObservation::Empty,
            ]),
            leader_reaped: false,
            touched_reused_group: false,
        };

        terminate_group_with_anchor(&mut cleanup).expect("anchored cleanup");

        assert_eq!(
            cleanup.events,
            [
                CleanupEvent::Signal,
                CleanupEvent::WaitForLeaderExit,
                CleanupEvent::InspectMembers,
                CleanupEvent::Signal,
                CleanupEvent::InspectMembers,
                CleanupEvent::ReapLeader,
            ]
        );
        assert!(cleanup.leader_reaped);
        assert!(
            !cleanup.touched_reused_group,
            "a bare PGID operation ran after the simulated immediate reuse"
        );
    }

    #[test]
    fn persistent_proc_errors_end_with_anchored_kill_before_reap() {
        let mut cleanup = ReuseAwareCleanup {
            events: Vec::new(),
            member_observations: (0..GROUP_OBSERVATION_FAILURE_LIMIT)
                .map(|_| MemberObservation::Failure)
                .collect(),
            leader_reaped: false,
            touched_reused_group: false,
        };

        let error =
            terminate_group_with_anchor(&mut cleanup).expect_err("persistent proc failures");

        assert!(error.to_string().contains("injected process-table failure"));
        assert_eq!(
            cleanup
                .events
                .iter()
                .filter(|event| **event == CleanupEvent::InspectMembers)
                .count(),
            GROUP_OBSERVATION_FAILURE_LIMIT
        );
        assert_eq!(
            cleanup.events.first(),
            Some(&CleanupEvent::Signal),
            "cleanup must begin with an anchored kill"
        );
        assert_eq!(
            cleanup.events[cleanup.events.len() - 2..],
            [CleanupEvent::Signal, CleanupEvent::ReapLeader],
            "persistent observation failure needs a final anchored kill before reap"
        );
        assert!(cleanup.leader_reaped);
        assert!(
            !cleanup.touched_reused_group,
            "cleanup touched the simulated reused PGID after reaping"
        );
    }

    #[test]
    fn exited_leader_anchor_cleans_descendant_before_reap() {
        let scratch = Scratch::new("exited-leader-cleanup");
        let (mut child, leader, pidfd, descendant) =
            spawn_supervision_group(&scratch, "leader-exit");
        wait_for_test_leader_exit(&pidfd);
        assert!(
            rustix::process::test_kill_process(descendant).is_ok(),
            "descendant must still be live before cleanup"
        );

        terminate_group_and_reap(&mut child, leader).expect("anchored group cleanup");

        assert_group_was_reaped(&mut child, descendant);
    }

    #[test]
    fn waitid_observation_error_terminates_and_reaps_group() {
        let scratch = Scratch::new("waitid-error");
        let (mut child, leader, _pidfd, descendant) =
            spawn_supervision_group(&scratch, "leader-live");
        let invalid_pidfd_path = scratch.0.join("not-a-pidfd");
        fs::write(&invalid_pidfd_path, b"not a pidfd").expect("invalid pidfd fixture");
        let invalid_pidfd = rustix::fs::open(
            &invalid_pidfd_path,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("invalid pidfd fixture FD");
        let permit = acquire_permit(
            &scratch.0.join("gate"),
            Duration::from_secs(1),
            Duration::from_millis(5),
            || None,
        )
        .expect("permit");
        let mut signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM]).expect("signals");

        let error = wait_for_process_group_in(
            &mut child,
            leader,
            &invalid_pidfd,
            &mut signals,
            Path::new("/proc"),
            permit,
        )
        .expect_err("waitid failure must fail the gate");
        assert!(error.to_string().contains("cannot observe gate child"));
        assert_group_was_reaped(&mut child, descendant);
    }

    #[test]
    fn proc_observation_error_with_exited_leader_reaps_descendant_group() {
        let scratch = Scratch::new("proc-error");
        let (mut child, leader, pidfd, descendant) =
            spawn_supervision_group(&scratch, "leader-exit");
        let invalid_proc = scratch.0.join("not-proc");
        fs::write(&invalid_proc, b"not a directory").expect("invalid proc fixture");
        let permit = acquire_permit(
            &scratch.0.join("gate"),
            Duration::from_secs(1),
            Duration::from_millis(5),
            || None,
        )
        .expect("permit");
        let mut signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM]).expect("signals");

        let error = wait_for_process_group_in(
            &mut child,
            leader,
            &pidfd,
            &mut signals,
            &invalid_proc,
            permit,
        )
        .expect_err("proc failure must fail the gate");
        assert!(
            error
                .to_string()
                .contains("cannot inspect child process group")
        );
        assert_group_was_reaped(&mut child, descendant);
    }

    #[test]
    fn process_group_test_child() {
        let Some(role) = env::var_os(GROUP_TEST_ROLE) else {
            return;
        };
        match role.to_str().expect("UTF-8 test role") {
            "leader-live" | "leader-exit" => {
                let descendant = Command::new(env::current_exe().expect("test executable"))
                    .args([
                        "heavy_gate::tests::process_group_test_child",
                        "--exact",
                        "--nocapture",
                    ])
                    .env(GROUP_TEST_ROLE, "descendant")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("spawn group descendant");
                fs::write(
                    env::var_os(GROUP_TEST_DESCENDANT).expect("descendant marker"),
                    descendant.id().to_string(),
                )
                .expect("write descendant marker");
                drop(descendant);
                if role == "leader-live" {
                    thread::sleep(Duration::from_secs(30));
                }
            }
            "descendant" => thread::sleep(Duration::from_secs(30)),
            other => panic!("unknown process-group test role {other}"),
        }
    }

    fn spawn_supervision_group(
        scratch: &Scratch,
        role: &str,
    ) -> (Child, rustix::process::Pid, OwnedFd, rustix::process::Pid) {
        let descendant_marker = scratch.0.join("descendant-pid");
        let child = Command::new(env::current_exe().expect("test executable"))
            .args([
                "heavy_gate::tests::process_group_test_child",
                "--exact",
                "--nocapture",
            ])
            .env(GROUP_TEST_ROLE, role)
            .env(GROUP_TEST_DESCENDANT, &descendant_marker)
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn group leader");
        let leader = child_pid(&child).expect("leader PID");
        let pidfd = rustix::process::pidfd_open(leader, rustix::process::PidfdFlags::empty())
            .expect("leader pidfd");
        wait_for_test_path(&descendant_marker);
        let descendant = fs::read_to_string(descendant_marker)
            .expect("descendant PID")
            .parse::<i32>()
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .expect("valid descendant PID");
        (child, leader, pidfd, descendant)
    }

    fn wait_for_test_path(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_test_leader_exit(pidfd: &OwnedFd) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let exited = rustix::process::waitid(
                rustix::process::WaitId::PidFd(pidfd.as_fd()),
                rustix::process::WaitidOptions::EXITED
                    | rustix::process::WaitidOptions::NOHANG
                    | rustix::process::WaitidOptions::NOWAIT,
            )
            .expect("observe test leader")
            .is_some();
            if exited {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for test leader exit"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_group_was_reaped(child: &mut Child, descendant: rustix::process::Pid) {
        assert!(
            child.try_wait().expect("reaped leader status").is_some(),
            "group leader was not reaped"
        );
        assert!(
            matches!(
                rustix::process::test_kill_process(descendant),
                Err(rustix::io::Errno::SRCH)
            ),
            "group descendant survived observation failure"
        );
    }
}
