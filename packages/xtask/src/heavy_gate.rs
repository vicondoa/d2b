use std::{
    env, fs,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::{
            fs::{DirBuilderExt, MetadataExt},
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    thread,
    time::{Duration, Instant},
};

use command_fds::{CommandFdExt, FdMapping};
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, fcntl},
    libc,
};
use rustix::fs::{FileType, Mode, OFlags};
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM},
    iterator::Signals,
};

const SLOT_COUNT: usize = 2;
const SLOT_NAMES: [&str; SLOT_COUNT] = ["slot-0.lock", "slot-1.lock"];
const GATE_DIRECTORY_MODE: u32 = 0o700;
const SLOT_MODE: u32 = 0o600;
const GATE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const GATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);
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
    fd: OwnedFd,
}

struct HeavyGatePermit {
    fd: OwnedFd,
    slot: usize,
}

impl HeavyGatePermit {
    fn duplicate_for_child(&self) -> Result<OwnedFd, HeavyGateError> {
        rustix::io::fcntl_dupfd_cloexec(&self.fd, 0)
            .map_err(|error| HeavyGateError::new(format!("cannot duplicate gate slot FD: {error}")))
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
            terminate_group_and_reap(&mut child, leader_pid);
            return Err(HeavyGateError::new(format!(
                "cannot obtain race-free gate child authority: {error}"
            )));
        }
    };

    let result = wait_for_process_group(&mut child, leader_pid, &pidfd, &mut signals);
    drop(permit);
    result
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
    loop {
        if let Some(signal) = pending_signal() {
            return Err(HeavyGateError::new(format!(
                "interrupted by signal {signal} while waiting for a heavy gate slot"
            )));
        }
        for slot in 0..SLOT_COUNT {
            let fd = open_verified_slot(&directory, SLOT_NAMES[slot])?;
            match try_ofd_lock(&fd)? {
                LockAttempt::Acquired => return Ok(HeavyGatePermit { fd, slot }),
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
    let created = match fs::DirBuilder::new().mode(GATE_DIRECTORY_MODE).create(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => {
            return Err(HeavyGateError::new(format!(
                "cannot create heavy gate directory {}: {error}",
                path.display()
            )));
        }
    };
    let before = fs::symlink_metadata(path).map_err(|error| {
        HeavyGateError::new(format!(
            "cannot inspect heavy gate directory {}: {error}",
            path.display()
        ))
    })?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err(HeavyGateError::new(
            "heavy gate path is not a non-symlink directory",
        ));
    }
    let fd = rustix::fs::open(
        path,
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
    if FileType::from_raw_mode(after.st_mode) != FileType::Directory
        || before.dev() != after.st_dev
        || before.ino() != after.st_ino
        || before.uid() != uid
        || after.st_uid != uid
        || after.st_mode & 0o7777 != GATE_DIRECTORY_MODE
        || before.mode() & 0o7777 != GATE_DIRECTORY_MODE
    {
        return Err(HeavyGateError::new(
            "heavy gate directory has unsafe ownership, type, mode, or identity",
        ));
    }
    verify_cloexec(&fd, "heavy gate directory")?;
    Ok(VerifiedGateDirectory { fd })
}

fn open_verified_slot(
    directory: &VerifiedGateDirectory,
    name: &str,
) -> Result<OwnedFd, HeavyGateError> {
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
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != uid
        || stat.st_nlink != 1
        || stat.st_mode & 0o7777 != SLOT_MODE
    {
        return Err(HeavyGateError::new(format!(
            "heavy gate slot {name} has unsafe ownership, type, mode, or link count"
        )));
    }
    verify_cloexec(&fd, "heavy gate slot")?;
    Ok(fd)
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
) -> Result<ExitStatus, HeavyGateError> {
    loop {
        forward_pending_signals(signals, leader_pid);
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

    while process_group_has_nonleader_members(leader_pid)? {
        forward_pending_signals(signals, leader_pid);
        thread::sleep(CHILD_POLL_INTERVAL);
    }
    child
        .wait()
        .map_err(|error| HeavyGateError::new(format!("cannot reap gate child: {error}")))
}

fn forward_pending_signals(signals: &mut Signals, process_group: rustix::process::Pid) {
    for signal in signals.pending() {
        if let Some(signal) = rustix_signal(signal) {
            let _ = rustix::process::kill_process_group(process_group, signal);
        }
    }
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
) -> Result<bool, HeavyGateError> {
    let leader = leader_pid.as_raw_nonzero().get();
    for entry in fs::read_dir("/proc").map_err(|error| {
        HeavyGateError::new(format!("cannot inspect child process group: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            HeavyGateError::new(format!("cannot inspect child process group entry: {error}"))
        })?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if pid == leader {
            continue;
        }
        let stat = match fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(HeavyGateError::new(format!(
                    "cannot inspect process-group member: {error}"
                )));
            }
        };
        if process_group_from_stat(&stat) == Some(leader) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn process_group_from_stat(stat: &str) -> Option<i32> {
    let command_end = stat.rfind(')')?;
    stat.get(command_end + 1..)?
        .split_ascii_whitespace()
        .nth(2)?
        .parse()
        .ok()
}

fn child_pid(child: &Child) -> Result<rustix::process::Pid, HeavyGateError> {
    i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| HeavyGateError::new("gate child PID is invalid"))
}

fn terminate_group_and_reap(child: &mut Child, process_group: rustix::process::Pid) {
    let _ = rustix::process::kill_process_group(process_group, rustix::process::Signal::Kill);
    let _ = child.wait();
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
        os::unix::fs::{PermissionsExt, symlink},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("repository root")
                .to_path_buf();
            let path = root.join(format!(
                ".d2b-heavy-gate-{label}-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
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
    fn proc_stat_parser_handles_spaces_and_parentheses() {
        assert_eq!(
            process_group_from_stat("123 (a command) S 1 77 77 0 -1"),
            Some(77)
        );
        assert_eq!(
            process_group_from_stat("123 (a ) command) S 1 88 88 0 -1"),
            Some(88)
        );
    }
}
