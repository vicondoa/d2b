//! The sole heavy-lane semaphore for d2b validation.
//!
//! `cargo xtask heavy-gate -- <command> [args...]` is the only place a
//! Layer-2, host-integration, hardware, live, or perf-heavy command may be
//! started. It is a two-slot, per-UID semaphore built on open file
//! description locks so concurrent heavy validation cannot oversubscribe the
//! shared Nix store, cargo target directory, or KVM device.
//!
//! Contract, as specified by `docs/specs/ADR-046-validation-and-delivery.md`
//! section 11:
//!
//! * Exactly [`SLOT_COUNT`] slots, scoped to the invoking uid, living under
//!   `${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/d2b-heavy-gates/uid-<uid>/`.
//! * Acquisition is nonblocking (`F_OFD_SETLK`), retried every
//!   [`RETRY_INTERVAL`] for at most [`ACQUIRE_TIMEOUT`].
//! * Fail-closed: if the platform or filesystem does not support open file
//!   description locking, the gate refuses to run. There is deliberately no
//!   `flock` fallback and no unsynchronized degraded mode, because a
//!   fail-open gate would let two multi-hour lanes stampede the shared host
//!   resources it exists to protect.
//! * The child receives a duplicated handle on the *same* open file
//!   description, with `FD_CLOEXEC` cleared, so the slot stays held for the
//!   child's whole life. An open file description lock is released only when
//!   the last descriptor referring to that description is closed, so the
//!   wrapper never releases the slot early.
//! * The wrapper owns the child's process group: it starts the child as a
//!   process-group leader, forwards terminating signals to the whole group,
//!   escalates to `SIGKILL` after [`TERMINATION_GRACE`], reaps the child, and
//!   sweeps the group when the run was interrupted. A `Ctrl-C` or an external
//!   timeout therefore cannot orphan a running heavy lane that still holds a
//!   slot.
//!
//! The wrapper receives those signals through a `signalfd`, which requires
//! them to be blocked. A blocked signal mask survives `execve`, and a wrapped
//! command that inherited a blocked `SIGTERM` would ignore the wrapper's
//! graceful teardown entirely and only die at the `SIGKILL` escalation. The
//! wrapper therefore starts the command through a one-shot re-exec of itself
//! ([`EXEC_SHIM_FLAG`]) that clears the mask and immediately `execve`s the
//! real command in the same process. That keeps the pid, the process group,
//! and the inherited slot descriptor identical while guaranteeing the command
//! starts with a clean signal mask.
//!
//! "Sole use" also means: no crate, wave, or reviewer role may add a second
//! lock file, sleep-and-retry loop, or per-crate heavy-lane guard. Nested
//! invocations reuse the slot already held by the outer wrapper (signalled by
//! [`GATE_ACTIVE_ENV`]) rather than acquiring a second one, which would
//! deadlock a two-slot semaphore against itself.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread::sleep;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::libc;
use nix::sys::signal::{SigSet, Signal, killpg};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::unistd::{Pid, getuid};

/// Number of concurrent heavy lanes allowed per uid.
pub const SLOT_COUNT: usize = 2;

/// Delay between nonblocking acquisition attempts.
pub const RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// Ceiling on how long a lane waits for a slot before failing closed.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Grace period between forwarding a terminating signal to the child's
/// process group and escalating to `SIGKILL`.
pub const TERMINATION_GRACE: Duration = Duration::from_secs(10);

/// How often the supervisor loop wakes to reap the child and drain signals.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How often a waiting lane repeats its "still waiting" diagnostic.
const WAIT_NOTICE_INTERVAL: Duration = Duration::from_secs(60);

/// Directory name holding every uid's slot directory.
const GATE_DIR_NAME: &str = "d2b-heavy-gates";

/// Set in the child environment so nested lanes reuse the held slot instead
/// of deadlocking against the same two slots.
pub const GATE_ACTIVE_ENV: &str = "D2B_HEAVY_GATE";

/// Slot index the child is running under.
pub const SLOT_INDEX_ENV: &str = "D2B_HEAVY_GATE_SLOT";

/// Descriptor number of the child's inherited handle on the locked slot.
pub const SLOT_FD_ENV: &str = "D2B_HEAVY_GATE_SLOT_FD";

/// Internal marker selecting the one-shot re-exec shim. Not an operator
/// surface: the wrapper passes it to itself so the wrapped command starts
/// with a cleared signal mask.
pub const EXEC_SHIM_FLAG: &str = "--exec-child";

/// Signals the wrapper forwards to the child's process group.
const FORWARDED_SIGNALS: [Signal; 4] = [
    Signal::SIGINT,
    Signal::SIGTERM,
    Signal::SIGHUP,
    Signal::SIGQUIT,
];

pub type Result<T> = std::result::Result<T, GateError>;

/// Failure classes, each with a distinct exit code drawn from the `sysexits`
/// range so a gate failure is never confused with the wrapped command's own
/// exit status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateErrorKind {
    /// The invocation itself was malformed.
    Usage,
    /// Open file description locking is unavailable. Always fails closed.
    Unsupported,
    /// No slot became free within [`ACQUIRE_TIMEOUT`].
    Timeout,
    /// The gate directory or slot file could not be prepared safely.
    Environment,
    /// The wrapped command could not be started or supervised.
    Spawn,
}

impl GateErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Unsupported => "unsupported",
            Self::Timeout => "timeout",
            Self::Environment => "environment",
            Self::Spawn => "spawn",
        }
    }

    /// Process exit code for this failure class. Never zero.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Usage => 64,
            Self::Unsupported => 69,
            Self::Environment => 72,
            Self::Spawn => 71,
            Self::Timeout => 75,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateError {
    kind: GateErrorKind,
    message: String,
}

impl GateError {
    pub fn of(kind: GateErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::of(GateErrorKind::Usage, message)
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self::of(GateErrorKind::Environment, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::of(GateErrorKind::Unsupported, message)
    }

    pub fn kind(&self) -> GateErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for GateError {}

/// How a nonblocking lock attempt resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockOutcome {
    /// The slot is held by another lane; retry.
    Busy,
    /// Open file description locking is not available here; fail closed.
    Unsupported,
    /// Something else went wrong with the slot file.
    Environment,
}

/// Classify an `F_OFD_SETLK` failure.
///
/// `EAGAIN`/`EACCES` are the only retryable outcomes. Everything that means
/// "this kernel or filesystem cannot do open file description locks" maps to
/// [`LockOutcome::Unsupported`] so the gate refuses to run rather than
/// silently degrading to unsynchronized execution.
pub fn classify_lock_errno(errno: Errno) -> LockOutcome {
    match errno {
        Errno::EAGAIN | Errno::EACCES => LockOutcome::Busy,
        Errno::EINVAL | Errno::ENOLCK | Errno::ENOSYS | Errno::EOPNOTSUPP => {
            LockOutcome::Unsupported
        }
        _ => LockOutcome::Environment,
    }
}

fn flock_for(kind: libc::c_short) -> libc::flock {
    libc::flock {
        l_type: kind,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    }
}

/// Attempt a whole-file exclusive open file description lock without blocking.
fn try_lock(file: &File) -> std::result::Result<(), Errno> {
    let lock = flock_for(libc::F_WRLCK as libc::c_short);
    fcntl(file.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock)).map(drop)
}

/// Release a whole-file open file description lock.
fn unlock(file: &File) -> std::result::Result<(), Errno> {
    let lock = flock_for(libc::F_UNLCK as libc::c_short);
    fcntl(file.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock)).map(drop)
}

/// Resolve the directory the gate directory is created under.
///
/// Kept pure so the precedence rule is directly testable.
pub fn gate_root_from(xdg_runtime_dir: Option<&Path>, tmpdir: Option<&Path>) -> PathBuf {
    xdg_runtime_dir
        .or(tmpdir)
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf()
}

/// Per-uid slot directory under `root`.
pub fn gate_dir_path(root: &Path, uid: u32) -> PathBuf {
    root.join(GATE_DIR_NAME).join(format!("uid-{uid}"))
}

/// A prepared, ownership-checked per-uid slot directory.
#[derive(Clone, Debug)]
pub struct GateDir {
    path: PathBuf,
}

impl GateDir {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve and prepare the gate directory from the process environment.
    pub fn resolve() -> Result<Self> {
        let xdg = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty());
        let tmpdir = std::env::var_os("TMPDIR").filter(|value| !value.is_empty());
        let root = gate_root_from(xdg.as_ref().map(Path::new), tmpdir.as_ref().map(Path::new));
        Self::prepare(&root, getuid().as_raw())
    }

    /// Create (if needed) and validate `<root>/d2b-heavy-gates/uid-<uid>`.
    ///
    /// The shared parent is created sticky and world-writable, exactly like
    /// `/tmp`, so an unprivileged peer on a multi-user host cannot deny
    /// service by winning the create race; the sticky bit stops it from
    /// removing or renaming another uid's slot directory. The per-uid
    /// directory itself is the real boundary: it must be a real directory
    /// owned by this uid with no group or other access.
    pub fn prepare(root: &Path, uid: u32) -> Result<Self> {
        let path = gate_dir_path(root, uid);
        let shared = path
            .parent()
            .expect("the per-uid slot directory always has a parent")
            .to_path_buf();
        create_dir_with_mode(&shared, 0o1777)?;
        require_directory(&shared)?;

        create_dir_with_mode(&path, 0o700)?;
        let metadata = require_directory(&path)?;
        if metadata.uid() != uid {
            return Err(GateError::environment(format!(
                "heavy-gate slot directory {} is owned by uid {}, not uid {}; \
                 refusing to share a semaphore across uids",
                path.display(),
                metadata.uid(),
                uid
            )));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(GateError::environment(format!(
                "heavy-gate slot directory {} has mode {:o}; it must not be \
                 group- or world-accessible. Remove it and rerun.",
                path.display(),
                metadata.mode() & 0o7777
            )));
        }
        Ok(Self { path })
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.path.join(format!("slot-{index}"))
    }

    /// Verify that this filesystem really implements `F_OFD_SETLK`.
    ///
    /// Uses a process-private probe file so a concurrent lane can never make
    /// the probe look like a failure. Any errno that means "unsupported"
    /// fails closed here, before a single slot is touched.
    pub fn probe_ofd_support(&self) -> Result<()> {
        let probe_path = self.path.join(format!(".ofd-probe-{}", std::process::id()));
        let _ = fs::remove_file(&probe_path);
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&probe_path)
            .map_err(|error| {
                GateError::environment(format!(
                    "cannot create heavy-gate probe file {}: {error}",
                    probe_path.display()
                ))
            })?;

        let result = self.evaluate_probe(&probe, &probe_path);
        drop(probe);
        let _ = fs::remove_file(&probe_path);
        result
    }

    fn evaluate_probe(&self, probe: &File, probe_path: &Path) -> Result<()> {
        match try_lock(probe) {
            Ok(()) => unlock(probe).map_err(|errno| self.probe_error(errno, probe_path, "release")),
            // A contended probe still proves the mechanism works.
            Err(Errno::EAGAIN | Errno::EACCES) => Ok(()),
            Err(errno) => Err(self.probe_error(errno, probe_path, "acquire")),
        }
    }

    fn probe_error(&self, errno: Errno, probe_path: &Path, phase: &str) -> GateError {
        match classify_lock_errno(errno) {
            LockOutcome::Unsupported => self.unsupported_error(errno),
            _ => GateError::environment(format!(
                "cannot {phase} the heavy-gate probe lock on {}: {errno}",
                probe_path.display()
            )),
        }
    }

    fn unsupported_error(&self, errno: Errno) -> GateError {
        GateError::unsupported(format!(
            "open file description locks (F_OFD_SETLK) are unavailable on {} ({errno}). \
             The heavy gate fails closed rather than falling back to flock or running \
             unsynchronized; point XDG_RUNTIME_DIR or TMPDIR at a filesystem that \
             supports them.",
            self.path.display()
        ))
    }

    /// One nonblocking attempt at `index`.
    ///
    /// Returns `Ok(None)` when the slot is held by another lane.
    pub fn try_acquire(&self, index: usize) -> Result<Option<SlotGuard>> {
        assert!(index < SLOT_COUNT, "slot index out of range");
        let path = self.slot_path(index);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|error| {
                GateError::environment(format!(
                    "cannot open heavy-gate slot file {}: {error}",
                    path.display()
                ))
            })?;
        let metadata = file.metadata().map_err(|error| {
            GateError::environment(format!(
                "cannot stat heavy-gate slot file {}: {error}",
                path.display()
            ))
        })?;
        if !metadata.is_file() {
            return Err(GateError::environment(format!(
                "heavy-gate slot path {} is not a regular file",
                path.display()
            )));
        }
        if metadata.uid() != getuid().as_raw() {
            return Err(GateError::environment(format!(
                "heavy-gate slot file {} is owned by uid {}, not uid {}",
                path.display(),
                metadata.uid(),
                getuid().as_raw()
            )));
        }

        match try_lock(&file) {
            Ok(()) => Ok(Some(SlotGuard { index, file })),
            Err(errno) => match classify_lock_errno(errno) {
                LockOutcome::Busy => Ok(None),
                LockOutcome::Unsupported => Err(self.unsupported_error(errno)),
                LockOutcome::Environment => Err(GateError::environment(format!(
                    "cannot lock heavy-gate slot file {}: {errno}",
                    path.display()
                ))),
            },
        }
    }
}

fn create_dir_with_mode(path: &Path, mode: u32) -> Result<()> {
    match fs::DirBuilder::new().mode(mode).create(path) {
        Ok(()) => {
            // `mkdir` masks the requested mode with the umask; force it back
            // so a restrictive umask cannot turn the shared parent into a
            // single-uid directory.
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
                GateError::environment(format!(
                    "cannot set mode {mode:o} on {}: {error}",
                    path.display()
                ))
            })
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(GateError::environment(format!(
            "cannot create {}: {error}",
            path.display()
        ))),
    }
}

fn require_directory(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        GateError::environment(format!("cannot stat {}: {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(GateError::environment(format!(
            "{} is a symlink; refusing to use it as a heavy-gate directory",
            path.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(GateError::environment(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    Ok(metadata)
}

/// A held slot. Dropping it closes this descriptor; the underlying open file
/// description lock survives as long as any duplicate handed to a child is
/// still open.
#[derive(Debug)]
pub struct SlotGuard {
    index: usize,
    file: File,
}

impl SlotGuard {
    pub fn index(&self) -> usize {
        self.index
    }

    /// Duplicate the locked descriptor for the child.
    ///
    /// `File::try_clone` duplicates the descriptor without duplicating the
    /// open file description, which is precisely what makes the child share
    /// this slot's lock. Clearing `FD_CLOEXEC` lets the duplicate survive the
    /// child's `execve`, so the slot stays held until the child and every
    /// process that inherited the descriptor is gone.
    pub fn duplicate_for_child(&self) -> Result<File> {
        let duplicate = self.file.try_clone().map_err(|error| {
            GateError::environment(format!("cannot duplicate heavy-gate slot handle: {error}"))
        })?;
        fcntl(
            duplicate.as_raw_fd(),
            FcntlArg::F_SETFD(FdFlag::from_bits_truncate(0)),
        )
        .map_err(|errno| {
            GateError::environment(format!(
                "cannot clear close-on-exec for the heavy-gate slot handle: {errno}"
            ))
        })?;
        Ok(duplicate)
    }
}

/// Retry policy for slot acquisition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcquirePolicy {
    pub retry_interval: Duration,
    pub timeout: Duration,
}

impl Default for AcquirePolicy {
    fn default() -> Self {
        Self {
            retry_interval: RETRY_INTERVAL,
            timeout: ACQUIRE_TIMEOUT,
        }
    }
}

/// Diagnostics emitted while waiting for a slot.
pub trait Progress {
    fn waiting(&mut self, waited: Duration);
    fn acquired(&mut self, index: usize, waited: Duration);
}

/// Human-facing progress reporting on stderr.
pub struct StderrProgress;

impl Progress for StderrProgress {
    fn waiting(&mut self, waited: Duration) {
        eprintln!(
            "heavy-gate: all {SLOT_COUNT} slots busy after {:.0}s; retrying every {}ms \
             (ceiling {}m)",
            waited.as_secs_f64(),
            RETRY_INTERVAL.as_millis(),
            ACQUIRE_TIMEOUT.as_secs() / 60
        );
    }

    fn acquired(&mut self, index: usize, waited: Duration) {
        eprintln!(
            "heavy-gate: acquired slot {index} of {SLOT_COUNT} after {:.1}s",
            waited.as_secs_f64()
        );
    }
}

/// Progress sink that says nothing. Test scaffolding only: production lanes
/// always report their wait through [`StderrProgress`].
#[cfg(test)]
pub struct SilentProgress;

#[cfg(test)]
impl Progress for SilentProgress {
    fn waiting(&mut self, _waited: Duration) {}
    fn acquired(&mut self, _index: usize, _waited: Duration) {}
}

/// Acquire one of the [`SLOT_COUNT`] slots, or fail closed.
pub fn acquire_slot(
    dir: &GateDir,
    policy: AcquirePolicy,
    progress: &mut dyn Progress,
) -> Result<SlotGuard> {
    let started = Instant::now();
    // Offset the scan so concurrent lanes do not all pile onto slot 0.
    let offset = std::process::id() as usize % SLOT_COUNT;
    let mut next_notice = WAIT_NOTICE_INTERVAL;
    loop {
        for step in 0..SLOT_COUNT {
            let index = (offset + step) % SLOT_COUNT;
            if let Some(guard) = dir.try_acquire(index)? {
                progress.acquired(index, started.elapsed());
                return Ok(guard);
            }
        }
        let waited = started.elapsed();
        if waited >= policy.timeout {
            return Err(GateError::of(
                GateErrorKind::Timeout,
                format!(
                    "no heavy-gate slot became free within {}s; failing closed rather than \
                     running unsynchronized. Another heavy lane is still holding both slots \
                     under {}.",
                    policy.timeout.as_secs(),
                    dir.path().display()
                ),
            ));
        }
        if waited >= next_notice {
            progress.waiting(waited);
            next_notice = waited + WAIT_NOTICE_INTERVAL;
        }
        sleep(
            policy
                .retry_interval
                .min(policy.timeout.saturating_sub(waited)),
        );
    }
}

/// A parsed `heavy-gate` invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl Request {
    /// Parse `heavy-gate`'s arguments.
    ///
    /// The documented form is `heavy-gate -- <command> [args...]`; the
    /// separator is optional when the command name is not option-shaped.
    pub fn parse(args: &[String]) -> Result<Option<Self>> {
        if args.is_empty() {
            return Err(GateError::usage(format!("{USAGE}\n\nno command given")));
        }
        if matches!(args[0].as_str(), "-h" | "--help" | "help") {
            return Ok(None);
        }
        let rest = if args[0] == "--" { &args[1..] } else { args };
        let Some((program, arguments)) = rest.split_first() else {
            return Err(GateError::usage(format!(
                "{USAGE}\n\nno command follows the `--` separator"
            )));
        };
        if program.starts_with('-') {
            return Err(GateError::usage(format!(
                "{USAGE}\n\nunknown option `{program}`; heavy-gate takes no options"
            )));
        }
        Ok(Some(Self {
            program: OsString::from(program),
            args: arguments.iter().map(OsString::from).collect(),
        }))
    }
}

const USAGE: &str = "usage: cargo xtask heavy-gate -- <command> [args...]\n\
     \n\
     Runs <command> under the sole two-slot per-UID heavy-lane semaphore.\n\
     Every Layer-2, host-integration, hardware, live, and perf-heavy command\n\
     must be started this way; the `heavy-*` Makefile targets do it for you.\n\
     \n\
     The command inherits a duplicated handle on the locked slot (its number\n\
     is exported as D2B_HEAVY_GATE_SLOT_FD) and runs in its own process\n\
     group, which the wrapper signals and reaps.\n\
     \n\
     exit codes: 64 usage, 69 open file description locks unsupported,\n\
     71 cannot start the command, 72 gate directory unusable,\n\
     75 no slot within the wait ceiling. Any other code is the command's own.";

/// Entry point for `cargo xtask heavy-gate`.
pub fn run(args: &[String]) -> ExitCode {
    match execute(args) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!(
                "heavy-gate failed [{}]: {}",
                error.kind().as_str(),
                error.message()
            );
            ExitCode::from(error.kind().exit_code())
        }
    }
}

fn execute(args: &[String]) -> Result<u8> {
    let inside_slot = std::env::var_os(GATE_ACTIVE_ENV).is_some();

    if args.first().is_some_and(|first| first == EXEC_SHIM_FLAG) {
        return exec_wrapped_command(&args[1..], inside_slot);
    }

    let Some(request) = Request::parse(args)? else {
        println!("{USAGE}");
        return Ok(0);
    };

    if inside_slot {
        eprintln!(
            "heavy-gate: already inside a heavy-gate slot; reusing it instead of \
             acquiring a second one"
        );
        return supervise(&request, None);
    }

    let dir = GateDir::resolve()?;
    dir.probe_ofd_support()?;
    let guard = acquire_slot(&dir, AcquirePolicy::default(), &mut StderrProgress)?;
    supervise(&request, Some(&guard))
}

/// The one-shot re-exec shim.
///
/// Runs in the already-forked child, in its own process group, holding the
/// inherited slot descriptor. It clears the signal mask the wrapper needed for
/// its `signalfd` and then replaces itself with the real command, so the
/// command keeps this pid and process group but starts with default signal
/// dispositions and an empty mask. It only returns on failure.
fn exec_wrapped_command(args: &[String], inside_slot: bool) -> Result<u8> {
    // The shim runs no slot acquisition of its own, so it must only ever be
    // reachable from a wrapper that already holds one. That is exactly the
    // condition the nesting rule above already treats as slot-covered.
    if !inside_slot {
        return Err(GateError::usage(format!(
            "{USAGE}\n\nheavy-gate takes no options"
        )));
    }
    let Some(request) = Request::parse(args)? else {
        return Err(GateError::usage(format!("{USAGE}\n\nno command given")));
    };
    exec_shim(&request)
}

/// Clear the wrapper's signal mask and become the wrapped command.
fn exec_shim(request: &Request) -> Result<u8> {
    let mut mask = SigSet::empty();
    for signal in FORWARDED_SIGNALS {
        mask.add(signal);
    }
    mask.thread_unblock().map_err(|errno| {
        GateError::of(
            GateErrorKind::Spawn,
            format!("cannot restore the heavy lane's signal mask: {errno}"),
        )
    })?;
    let error = Command::new(&request.program).args(&request.args).exec();
    Err(GateError::of(
        GateErrorKind::Spawn,
        format!(
            "cannot start `{}`: {error}",
            request.program.to_string_lossy()
        ),
    ))
}

/// Blocked-signal relay. Restores the caller's mask on drop.
struct SignalRelay {
    mask: SigSet,
    fd: SignalFd,
}

impl SignalRelay {
    fn install() -> Result<Self> {
        let mut mask = SigSet::empty();
        for signal in FORWARDED_SIGNALS {
            mask.add(signal);
        }
        mask.thread_block().map_err(|errno| {
            GateError::of(
                GateErrorKind::Spawn,
                format!("cannot block terminating signals: {errno}"),
            )
        })?;
        let fd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
            .map_err(|errno| {
                GateError::of(
                    GateErrorKind::Spawn,
                    format!("cannot create the heavy-gate signalfd: {errno}"),
                )
            })?;
        Ok(Self { mask, fd })
    }

    fn drain(&self) -> Vec<Signal> {
        let mut received = Vec::new();
        while let Ok(Some(info)) = self.fd.read_signal() {
            if let Ok(signal) = Signal::try_from(info.ssi_signo as i32) {
                received.push(signal);
            }
        }
        received
    }
}

impl Drop for SignalRelay {
    fn drop(&mut self) {
        let _ = self.mask.thread_unblock();
    }
}

/// Build the command that starts the wrapped lane.
///
/// The lane is started through a one-shot re-exec of this binary so the real
/// command runs with a cleared signal mask; see the module documentation.
fn shim_command(request: &Request) -> Result<Command> {
    #[cfg(test)]
    if let Some(command) = tests::test_shim_command(request) {
        return Ok(command);
    }
    let shim = std::env::current_exe().map_err(|error| {
        GateError::of(
            GateErrorKind::Spawn,
            format!("cannot locate the heavy-gate binary to re-exec: {error}"),
        )
    })?;
    let mut command = Command::new(shim);
    command.arg("heavy-gate");
    command.arg(EXEC_SHIM_FLAG);
    command.arg("--");
    command.arg(&request.program);
    command.args(&request.args);
    Ok(command)
}

fn supervise(request: &Request, guard: Option<&SlotGuard>) -> Result<u8> {
    let relay = SignalRelay::install()?;

    let mut command = shim_command(request)?;
    // The child leads its own process group so the wrapper can signal the
    // whole lane, and so a terminal `Ctrl-C` reaches the wrapper first.
    command.process_group(0);
    command.env(GATE_ACTIVE_ENV, "1");

    let handoff = match guard {
        Some(slot) => {
            let duplicate = slot.duplicate_for_child()?;
            command.env(SLOT_INDEX_ENV, slot.index().to_string());
            command.env(SLOT_FD_ENV, duplicate.as_raw_fd().to_string());
            Some(duplicate)
        }
        None => None,
    };

    let mut child = command.spawn().map_err(|error| {
        GateError::of(
            GateErrorKind::Spawn,
            format!(
                "cannot start the heavy lane for `{}`: {error}",
                request.program.to_string_lossy()
            ),
        )
    })?;
    // The child now has its own descriptor for the same open file
    // description, so the wrapper's duplicate is no longer needed. The slot
    // stays locked: the wrapper still holds `guard`, and the child holds an
    // inherited handle for its whole life.
    drop(handoff);

    let group = Pid::from_raw(child.id() as i32);
    let mut interrupt: Option<Signal> = None;
    let mut escalate_at: Option<Instant> = None;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                return Err(GateError::of(
                    GateErrorKind::Spawn,
                    format!("cannot reap the heavy-lane child: {error}"),
                ));
            }
        }

        for signal in relay.drain() {
            eprintln!("heavy-gate: forwarding {signal} to the heavy lane process group");
            let _ = killpg(group, signal);
            if interrupt.is_none() {
                interrupt = Some(signal);
            }
            if escalate_at.is_none() {
                escalate_at = Some(Instant::now() + TERMINATION_GRACE);
            }
        }

        if let Some(deadline) = escalate_at
            && Instant::now() >= deadline
        {
            eprintln!(
                "heavy-gate: heavy lane still running {}s after the first signal; \
                 sending SIGKILL to its process group",
                TERMINATION_GRACE.as_secs()
            );
            let _ = killpg(group, Signal::SIGKILL);
            escalate_at = None;
        }

        sleep(POLL_INTERVAL);
    };

    if interrupt.is_some() {
        // The run was interrupted, so anything still alive in the lane's
        // process group is an orphan that would keep holding this slot.
        let _ = killpg(group, Signal::SIGKILL);
    }

    Ok(resolve_exit_code(
        status.code(),
        status.signal(),
        interrupt.map(|signal| signal as i32),
    ))
}

/// Map the child's disposition onto the wrapper's exit code.
///
/// A signalled child reports `128 + signal`, matching shell convention. When
/// the wrapper itself was interrupted but the child still exited normally,
/// the interruption wins so a `Ctrl-C`-ed lane never looks successful.
fn resolve_exit_code(code: Option<i32>, signal: Option<i32>, interrupt: Option<i32>) -> u8 {
    if let Some(signal) = signal.or(interrupt) {
        return (128 + signal).clamp(1, 255) as u8;
    }
    match code {
        Some(code) => (code & 0xff) as u8,
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::io::Write;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Serialises every test that either asserts exact open file description
    /// lock state or spawns a child process.
    ///
    /// `fork` copies the whole descriptor table, and close-on-exec descriptors
    /// stay open in the child until it reaches `execve`. A spawn on one test
    /// thread can therefore keep another test's slot descriptor alive for a
    /// moment and make a released slot still look held. Production never sees
    /// this: one wrapper process spawns exactly one child.
    static LOCK_STATE: Mutex<()> = Mutex::new(());

    fn exclusive() -> MutexGuard<'static, ()> {
        LOCK_STATE.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Env var selecting the in-test slot-holder child mode.
    const HOLD_ROOT_ENV: &str = "D2B_HEAVY_GATE_TEST_HOLD_ROOT";
    /// Env var selecting the in-test full-wrapper child mode.
    const WRAP_ROOT_ENV: &str = "D2B_HEAVY_GATE_TEST_WRAP_ROOT";

    const HOLDER_TEST: &str = "heavy_gate::tests::slot_holder_child_mode";
    const WRAPPER_TEST: &str = "heavy_gate::tests::full_wrapper_child_mode";
    const SHIM_TEST: &str = "heavy_gate::tests::exec_shim_child_mode";

    /// Env vars carrying the wrapped command into the in-test shim.
    const SHIM_PROGRAM_ENV: &str = "D2B_HEAVY_GATE_TEST_SHIM_PROGRAM";
    const SHIM_ARGS_ENV: &str = "D2B_HEAVY_GATE_TEST_SHIM_ARGS";

    /// Separator for the encoded argument vector; a unit separator cannot
    /// appear in the arguments these tests pass.
    const ARG_SEPARATOR: u8 = 0x1f;

    fn encode_args(args: &[OsString]) -> OsString {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let mut bytes = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            if index > 0 {
                bytes.push(ARG_SEPARATOR);
            }
            bytes.extend_from_slice(arg.as_bytes());
        }
        OsString::from_vec(bytes)
    }

    fn decode_args(value: &OsStr) -> Vec<OsString> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return Vec::new();
        }
        bytes
            .split(|byte| *byte == ARG_SEPARATOR)
            .map(|chunk| OsString::from_vec(chunk.to_vec()))
            .collect()
    }

    /// In-test stand-in for the production re-exec shim.
    ///
    /// The test binary is `libtest`, not the `xtask` dispatcher, so it cannot
    /// re-invoke itself as `heavy-gate --exec-child`. Tests reach exactly the
    /// same shim code through a dedicated child-mode test instead.
    pub(super) fn test_shim_command(request: &Request) -> Option<Command> {
        let exe = std::env::current_exe().ok()?;
        let mut command = Command::new(exe);
        command.args([SHIM_TEST, "--exact", "--nocapture", "--test-threads=1"]);
        command.env(SHIM_PROGRAM_ENV, &request.program);
        command.env(SHIM_ARGS_ENV, encode_args(&request.args));
        Some(command)
    }

    static SCRATCH_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    /// Self-cleaning scratch directory under the cargo target directory, so
    /// no test writes into the repository tree.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            let target = match std::env::var_os("CARGO_TARGET_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("xtask lives inside the workspace root")
                    .join("target"),
            };
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = target
                .join("heavy-gate-tests")
                .join(format!("{label}-{}-{sequence}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch directory is creatable");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn gate_dir_under(root: &Path) -> GateDir {
        GateDir::prepare(root, getuid().as_raw()).expect("gate directory is preparable")
    }

    fn wait_for(deadline: Duration, mut ready: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < deadline {
            if ready() {
                return true;
            }
            sleep(Duration::from_millis(10));
        }
        ready()
    }

    // ---- pure contract -------------------------------------------------

    #[test]
    fn gate_root_precedence_is_xdg_then_tmpdir_then_tmp() {
        assert_eq!(
            gate_root_from(
                Some(Path::new("/run/user/1000")),
                Some(Path::new("/scratch"))
            ),
            PathBuf::from("/run/user/1000")
        );
        assert_eq!(
            gate_root_from(None, Some(Path::new("/scratch"))),
            PathBuf::from("/scratch")
        );
        assert_eq!(gate_root_from(None, None), PathBuf::from("/tmp"));
    }

    #[test]
    fn gate_directory_is_scoped_per_uid() {
        let first = gate_dir_path(Path::new("/run/user/1000"), 1000);
        let second = gate_dir_path(Path::new("/run/user/1000"), 1001);
        assert!(first.ends_with("d2b-heavy-gates/uid-1000"));
        assert!(second.ends_with("d2b-heavy-gates/uid-1001"));
        assert_ne!(first, second);
    }

    #[test]
    fn unsupported_locking_is_never_treated_as_retryable() {
        for errno in [
            Errno::EINVAL,
            Errno::ENOLCK,
            Errno::ENOSYS,
            Errno::EOPNOTSUPP,
        ] {
            assert_eq!(
                classify_lock_errno(errno),
                LockOutcome::Unsupported,
                "{errno} must fail closed, never fall back"
            );
        }
        for errno in [Errno::EAGAIN, Errno::EACCES] {
            assert_eq!(classify_lock_errno(errno), LockOutcome::Busy);
        }
        assert_eq!(classify_lock_errno(Errno::EIO), LockOutcome::Environment);
    }

    #[test]
    fn every_failure_class_has_a_distinct_nonzero_exit_code() {
        let kinds = [
            GateErrorKind::Usage,
            GateErrorKind::Unsupported,
            GateErrorKind::Timeout,
            GateErrorKind::Environment,
            GateErrorKind::Spawn,
        ];
        let mut codes: Vec<u8> = kinds.iter().map(|kind| kind.exit_code()).collect();
        assert!(codes.iter().all(|code| *code != 0));
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), kinds.len(), "exit codes must be distinct");
    }

    #[test]
    fn unsupported_error_message_refuses_a_fallback() {
        let scratch = Scratch::new("unsupported-message");
        let dir = gate_dir_under(scratch.path());
        let error = dir.unsupported_error(Errno::EINVAL);
        assert_eq!(error.kind(), GateErrorKind::Unsupported);
        assert!(error.message().contains("F_OFD_SETLK"));
        assert!(error.message().contains("fails closed"));
        assert!(error.message().contains("flock"));
    }

    #[test]
    fn exit_code_reports_signals_and_interruptions() {
        assert_eq!(resolve_exit_code(Some(0), None, None), 0);
        assert_eq!(resolve_exit_code(Some(7), None, None), 7);
        assert_eq!(
            resolve_exit_code(None, Some(Signal::SIGKILL as i32), None),
            128 + 9
        );
        assert_eq!(
            resolve_exit_code(Some(0), None, Some(Signal::SIGINT as i32)),
            128 + 2
        );
        assert_eq!(resolve_exit_code(None, None, None), 1);
    }

    #[test]
    fn parse_accepts_the_documented_and_bare_forms() {
        let separated = Request::parse(&["--".into(), "make".into(), "check".into()])
            .expect("parses")
            .expect("is not help");
        let bare = Request::parse(&["make".into(), "check".into()])
            .expect("parses")
            .expect("is not help");
        assert_eq!(separated, bare);
        assert_eq!(separated.program, OsString::from("make"));
        assert_eq!(separated.args, vec![OsString::from("check")]);
    }

    #[test]
    fn parse_rejects_malformed_invocations() {
        assert_eq!(
            Request::parse(&[]).unwrap_err().kind(),
            GateErrorKind::Usage
        );
        assert_eq!(
            Request::parse(&["--".into()]).unwrap_err().kind(),
            GateErrorKind::Usage
        );
        assert_eq!(
            Request::parse(&["--jobs".into()]).unwrap_err().kind(),
            GateErrorKind::Usage
        );
        assert!(
            Request::parse(&["--help".into()])
                .expect("help parses")
                .is_none()
        );
    }

    #[test]
    fn the_internal_exec_shim_is_not_an_operator_entry_point() {
        // Outside a wrapper that already holds a slot the shim marker must be
        // rejected like any other unknown option, so no lane can start
        // unsynchronised by naming it.
        let error = exec_wrapped_command(&["--".into(), "true".into()], false)
            .expect_err("the shim is refused");
        assert_eq!(error.kind(), GateErrorKind::Usage);
        assert!(
            !error.message().contains(EXEC_SHIM_FLAG),
            "the internal marker must stay out of operator-visible text: {}",
            error.message()
        );
    }

    // ---- slot mechanics ------------------------------------------------

    #[test]
    fn ofd_locking_is_available_on_the_scratch_filesystem() {
        let _serial = exclusive();
        let scratch = Scratch::new("probe");
        let dir = gate_dir_under(scratch.path());
        dir.probe_ofd_support().expect("probe succeeds");
        assert!(
            !dir.path()
                .join(format!(".ofd-probe-{}", std::process::id()))
                .exists(),
            "the probe file is removed"
        );
    }

    #[test]
    fn exactly_two_slots_are_available_and_release_frees_them() {
        let _serial = exclusive();
        let scratch = Scratch::new("two-slots");
        let dir = gate_dir_under(scratch.path());

        // Separate `open` calls create separate open file descriptions, so
        // these contend exactly as two processes would.
        let first = dir.try_acquire(0).unwrap().expect("slot 0 is free");
        let second = dir.try_acquire(1).unwrap().expect("slot 1 is free");
        assert!(dir.try_acquire(0).unwrap().is_none(), "slot 0 stays held");
        assert!(dir.try_acquire(1).unwrap().is_none(), "slot 1 stays held");

        let index = second.index();
        drop(second);
        let reacquired = dir
            .try_acquire(index)
            .unwrap()
            .expect("the released slot is free again");
        assert_eq!(reacquired.index(), index);
        drop(reacquired);
        drop(first);
    }

    #[test]
    fn acquire_fails_closed_when_both_slots_stay_held() {
        let _serial = exclusive();
        let scratch = Scratch::new("timeout");
        let dir = gate_dir_under(scratch.path());
        let _held: Vec<SlotGuard> = (0..SLOT_COUNT)
            .map(|index| dir.try_acquire(index).unwrap().expect("slot is free"))
            .collect();

        let policy = AcquirePolicy {
            retry_interval: Duration::from_millis(5),
            timeout: Duration::from_millis(60),
        };
        let error = acquire_slot(&dir, policy, &mut SilentProgress).unwrap_err();
        assert_eq!(error.kind(), GateErrorKind::Timeout);
        assert!(error.message().contains("failing closed"));
    }

    #[test]
    fn prepare_rejects_a_symlinked_gate_directory() {
        let scratch = Scratch::new("symlink");
        let decoy = scratch.path().join("decoy");
        fs::create_dir_all(&decoy).unwrap();
        std::os::unix::fs::symlink(&decoy, scratch.path().join(GATE_DIR_NAME)).unwrap();

        let error = GateDir::prepare(scratch.path(), getuid().as_raw()).unwrap_err();
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(error.message().contains("symlink"));
    }

    #[test]
    fn prepare_rejects_a_group_accessible_slot_directory() {
        let scratch = Scratch::new("loose-mode");
        let path = gate_dir_path(scratch.path(), getuid().as_raw());
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o770)).unwrap();

        let error = GateDir::prepare(scratch.path(), getuid().as_raw()).unwrap_err();
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(error.message().contains("group- or world-accessible"));
    }

    #[test]
    fn duplicated_handle_survives_exec_and_keeps_the_slot_locked() {
        let _serial = exclusive();
        let scratch = Scratch::new("handoff");
        let dir = gate_dir_under(scratch.path());
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");
        let duplicate = guard.duplicate_for_child().expect("duplicate succeeds");

        let flags = fcntl(duplicate.as_raw_fd(), FcntlArg::F_GETFD).expect("F_GETFD works");
        assert_eq!(
            flags & libc::FD_CLOEXEC,
            0,
            "the child's handle must survive execve"
        );

        // Dropping the wrapper's own guard while the duplicate is still open
        // must NOT release the open file description lock.
        drop(guard);
        assert!(
            dir.try_acquire(0).unwrap().is_none(),
            "the slot stays held while the child's duplicate is open"
        );
        drop(duplicate);
        assert!(
            dir.try_acquire(0).unwrap().is_some(),
            "the slot frees once every handle is closed"
        );
    }

    // ---- multi-process behaviour --------------------------------------
    //
    // `xtask` is a binary-only crate, so there is no `tests/` tree to put a
    // real integration test in. These tests re-execute the test binary with
    // an env-selected mode to get genuinely separate processes.

    /// Child mode: hold one slot until the parent creates `release`.
    ///
    /// A no-op in ordinary runs; it only does work when the parent selects it.
    #[test]
    fn slot_holder_child_mode() {
        let Some(root) = std::env::var_os(HOLD_ROOT_ENV) else {
            return;
        };
        let root = PathBuf::from(root);
        let dir = GateDir::resolve().expect("the child resolves the same gate directory");
        assert_eq!(
            dir.path(),
            gate_dir_path(&root, getuid().as_raw()),
            "the child must resolve XDG_RUNTIME_DIR to the parent's scratch gate"
        );
        dir.probe_ofd_support().expect("probe succeeds");
        let guard = acquire_slot(&dir, AcquirePolicy::default(), &mut SilentProgress)
            .expect("the child acquires a slot");

        let ready = root.join(format!("ready-{}", std::process::id()));
        File::create(&ready).unwrap().write_all(b"held").unwrap();

        let release = root.join("release");
        assert!(
            wait_for(Duration::from_secs(30), || release.exists()),
            "the parent released the holder"
        );
        drop(guard);
    }

    /// Child mode: run the whole wrapper over a probe command.
    #[test]
    fn full_wrapper_child_mode() {
        if std::env::var_os(WRAP_ROOT_ENV).is_none() {
            return;
        }
        // The probe asserts the inherited slot descriptor is real and still
        // open in the grandchild, that the command starts with none of the
        // wrapper's forwarded signals blocked, and then exits with a
        // distinctive code.
        let blocked_mask = 0x4007u32; // SIGHUP | SIGINT | SIGQUIT | SIGTERM
        let script = format!(
            "test -n \"${SLOT_FD_ENV}\" || exit 40; \
             test -e /proc/self/fd/${SLOT_FD_ENV} || exit 41; \
             test -n \"${GATE_ACTIVE_ENV}\" || exit 42; \
             blocked=$(sed -n 's/^SigBlk:[[:space:]]*//p' /proc/self/status); \
             test $(( 0x$blocked & {blocked_mask} )) -eq 0 || exit 43; \
             exit 7"
        );
        let code = execute(&["--".into(), "sh".into(), "-c".into(), script])
            .expect("the wrapper runs the probe");
        std::process::exit(i32::from(code));
    }

    /// Child mode: the in-test re-exec shim.
    ///
    /// Selected by [`test_shim_command`]; a no-op in ordinary runs.
    #[test]
    fn exec_shim_child_mode() {
        let Some(program) = std::env::var_os(SHIM_PROGRAM_ENV) else {
            return;
        };
        let args = decode_args(&std::env::var_os(SHIM_ARGS_ENV).unwrap_or_default());
        let request = Request { program, args };
        let error = exec_shim(&request).expect_err("exec only returns on failure");
        eprintln!("{}", error.message());
        std::process::exit(i32::from(error.kind().exit_code()));
    }

    fn spawn_child_mode(test_name: &str, root: &Path, env_key: &str) -> std::process::Child {
        Command::new(std::env::current_exe().expect("the test binary path is known"))
            .args([test_name, "--exact", "--nocapture", "--test-threads=1"])
            .env(env_key, root)
            .env("XDG_RUNTIME_DIR", root)
            .env_remove("TMPDIR")
            .env_remove(GATE_ACTIVE_ENV)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the child mode process starts")
    }

    #[test]
    fn a_third_lane_blocks_until_a_concurrent_lane_releases_its_slot() {
        let _serial = exclusive();
        let scratch = Scratch::new("concurrency");
        let root = scratch.path().to_path_buf();
        let dir = gate_dir_under(&root);

        let mut holders: Vec<std::process::Child> = (0..SLOT_COUNT)
            .map(|_| spawn_child_mode(HOLDER_TEST, &root, HOLD_ROOT_ENV))
            .collect();

        let ready_count = || {
            fs::read_dir(&root)
                .map(|entries| {
                    entries
                        .filter_map(std::result::Result::ok)
                        .filter(|entry| entry.file_name().to_string_lossy().starts_with("ready-"))
                        .count()
                })
                .unwrap_or(0)
        };
        assert!(
            wait_for(Duration::from_secs(30), || ready_count() >= SLOT_COUNT),
            "both holder processes acquired a slot (saw {})",
            ready_count()
        );

        // Both slots are held by other processes: a nonblocking attempt from
        // this process must fail on every slot.
        for index in 0..SLOT_COUNT {
            assert!(
                dir.try_acquire(index).unwrap().is_none(),
                "slot {index} is held by a concurrent process"
            );
        }

        // Release after a measurable delay and prove the waiter blocked for
        // at least that long before it got in.
        let release_path = root.join("release");
        let releaser = std::thread::spawn(move || {
            sleep(Duration::from_millis(400));
            File::create(&release_path).unwrap();
        });

        let started = Instant::now();
        let policy = AcquirePolicy {
            retry_interval: Duration::from_millis(25),
            timeout: Duration::from_secs(30),
        };
        let guard = acquire_slot(&dir, policy, &mut SilentProgress)
            .expect("the waiter acquires once a holder releases");
        let waited = started.elapsed();
        releaser.join().expect("the releaser thread finishes");

        assert!(
            waited >= Duration::from_millis(300),
            "the waiter blocked until a slot was released (waited {waited:?})"
        );
        drop(guard);

        for holder in &mut holders {
            let status = holder.wait().expect("the holder process is reaped");
            assert!(status.success(), "holder exited cleanly: {status:?}");
        }
    }

    #[test]
    fn the_wrapper_hands_the_locked_descriptor_to_the_child_and_propagates_its_status() {
        let _serial = exclusive();
        let scratch = Scratch::new("wrapper");
        let root = scratch.path().to_path_buf();
        let mut child = spawn_child_mode(WRAPPER_TEST, &root, WRAP_ROOT_ENV);
        let status = child.wait().expect("the wrapper child is reaped");
        assert_eq!(
            status.code(),
            Some(7),
            "the wrapper propagates the command's exit code and the command \
             saw a live D2B_HEAVY_GATE_SLOT_FD"
        );
    }
}
