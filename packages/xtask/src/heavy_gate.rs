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
//!   escalates to `SIGKILL` after [`TERMINATION_GRACE`], observes the leader's
//!   exit *without reaping it*, unconditionally sweeps the group with
//!   `SIGKILL` while the leader is still an unreaped zombie, and only then
//!   reaps the leader. Sweeping before reaping keeps the leader's pid and pgid
//!   pinned by the zombie, so the numeric pgid cannot be recycled onto an
//!   unrelated process group between the exit and the `SIGKILL`. A `Ctrl-C`,
//!   an external timeout, or a signal that races the leader's exit therefore
//!   cannot orphan a descendant that still holds the slot, nor can the sweep
//!   ever reach a stranger's group.
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
//! invocations reuse the slot already held by the outer wrapper rather than
//! acquiring a second one, which would deadlock a two-slot semaphore against
//! itself. Nesting is never taken on trust: the mere presence of
//! [`GATE_ACTIVE_ENV`] proves nothing, because any process can export it. A
//! child is treated as nested only after the inherited slot descriptor
//! ([`SLOT_FD_ENV`]) is verified to be an open handle on the real,
//! currently-locked per-uid slot file it names ([`SLOT_INDEX_ENV`]); a forged,
//! stale, closed, or unlocked marker is ignored and a real slot is acquired
//! instead.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread::sleep;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::libc;
use nix::sys::signal::{SigSet, Signal, killpg};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::sys::stat::fstat;
use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
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

/// Attempt a whole-file exclusive open file description lock without blocking,
/// operating directly on a raw descriptor.
///
/// Applying `F_WRLCK` through a descriptor whose open file description already
/// holds the lock is idempotent and succeeds; applying it when a *different*
/// description holds the lock fails with `EAGAIN`/`EACCES`. That distinction is
/// what makes an `F_OFD_SETLK` through an inherited descriptor an atomic proof
/// of which description owns a slot.
fn try_lock_fd(fd: RawFd) -> std::result::Result<(), Errno> {
    let lock = flock_for(libc::F_WRLCK as libc::c_short);
    fcntl(fd, FcntlArg::F_OFD_SETLK(&lock)).map(drop)
}

/// Attempt a whole-file exclusive open file description lock without blocking.
fn try_lock(file: &File) -> std::result::Result<(), Errno> {
    try_lock_fd(file.as_raw_fd())
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

/// Whether the shared `d2b-heavy-gates` parent can be trusted not to let a
/// hostile peer rename another uid's slot directory out from under it.
///
/// The shared parent is the one directory that must tolerate several uids.
/// Only three shapes are safe:
///
/// * owned by us and not group- or world-writable, so no peer can create or
///   rename entries in it at all;
/// * owned by root and not group- or world-writable (a locked-down shared
///   parent an administrator provisioned); or
/// * owned by root and sticky (like `/tmp`), so peers may create their own
///   `uid-<uid>` entry but cannot rename ours.
///
/// A parent owned by a non-root peer is never trusted: as its owner that peer
/// could rename our slot directory even with the sticky bit set, which is
/// exactly the escape the two-slot limit exists to prevent. A group- or
/// world-writable parent we own is also untrusted here - but [`GateDir::prepare`]
/// first normalises an owned parent's mode to `0700`, so this predicate only
/// rejects it when it could not be locked down. Kept pure so the whole matrix
/// is directly testable.
pub fn shared_parent_is_trusted(owner_uid: u32, mode: u32, current_uid: u32) -> bool {
    let group_or_world_writable = mode & 0o022 != 0;
    let sticky = mode & 0o1000 != 0;
    if owner_uid == current_uid {
        return !group_or_world_writable;
    }
    if owner_uid == 0 {
        return !group_or_world_writable || sticky;
    }
    false
}

/// A prepared, ownership-checked per-uid slot directory anchored to a verified
/// open directory descriptor.
#[derive(Debug)]
pub struct GateDir {
    path: PathBuf,
    dir: File,
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
    /// The selected root (`XDG_RUNTIME_DIR`, else `TMPDIR`, else `/tmp`) is
    /// opened and `fstat`ed *first*. A root a non-root peer owns, or an
    /// owned-but-loosely-permissioned root, is refused: otherwise that peer
    /// could rename the whole verified `d2b-heavy-gates` directory between
    /// invocations, so a later invocation would create a *second* semaphore
    /// namespace and both would run two lanes each. Anchoring within a single
    /// invocation (via `/proc/self/fd`) protects that invocation's inodes but
    /// cannot preserve one namespace across invocations; only trusting the root
    /// can. A root is trusted when it is ours and not group- or world-writable
    /// (a per-user runtime directory), or root-owned and either locked down or
    /// sticky (like `/tmp`) - exactly [`shared_parent_is_trusted`].
    ///
    /// The shared `d2b-heavy-gates` parent is then created *relative to that
    /// anchored root descriptor* and private to us (mode `0700`) rather than
    /// sticky and world-writable, so a peer cannot win the create race, own the
    /// directory, and then rename our slot directory to let us mint fresh slot
    /// inodes past the two-lane limit. If the parent already exists it is
    /// accepted only when [`shared_parent_is_trusted`] holds - ours-and-private,
    /// or a root-owned parent an administrator provisioned. Every subsequent
    /// slot operation is anchored to the directory descriptor opened here, so
    /// renaming the path components after preparation cannot switch the
    /// semaphore namespace mid-run.
    pub fn prepare(root: &Path, uid: u32) -> Result<Self> {
        let path = gate_dir_path(root, uid);
        let shared = path
            .parent()
            .expect("the per-uid slot directory always has a parent")
            .to_path_buf();

        // Anchor to the root itself before creating anything beneath it. A
        // peer-owned or owned-but-loose root is refused so the shared directory
        // cannot be renamed out from under a future invocation.
        let root_dir = open_directory(root)?;
        let root_stat = fstat(root_dir.as_raw_fd()).map_err(|errno| {
            GateError::environment(format!("cannot stat {}: {errno}", root.display()))
        })?;
        if !shared_parent_is_trusted(root_stat.st_uid, root_stat.st_mode as u32, uid) {
            return Err(GateError::environment(format!(
                "the heavy-gate root {} is owned by uid {} with mode {:o}; refusing to create a \
                 semaphore namespace under a directory a peer could rename. Point \
                 XDG_RUNTIME_DIR at a per-user runtime directory, or set TMPDIR to a directory \
                 you own privately (mode 0700) or a root-owned sticky directory.",
                root.display(),
                root_stat.st_uid,
                root_stat.st_mode as u32 & 0o7777,
            )));
        }

        // Create the shared parent private to us, relative to the anchored
        // root descriptor. If it already exists and we own it, normalise its
        // mode to 0700 rather than reject: a world- or group-writable directory
        // lets any peer rename our entries even when we own it (and non-sticky
        // world-writable dirs let anyone rename any child), so locking it down
        // is the actual remedy - and it repairs a stale loose-moded directory
        // left by an older run. A parent owned by someone else is never
        // normalised; it is verified and, unless it is a trusted root-owned
        // parent, refused.
        let shared_anchor = anchored_path(&root_dir, GATE_DIR_NAME);
        create_dir_with_mode(&shared_anchor, 0o700)?;
        let shared_dir = open_directory(&shared_anchor)?;
        let mut shared_stat = fstat(shared_dir.as_raw_fd()).map_err(|errno| {
            GateError::environment(format!("cannot stat {}: {errno}", shared.display()))
        })?;
        if shared_stat.st_uid == uid && (shared_stat.st_mode as u32 & 0o7777) != 0o700 {
            let self_anchor = PathBuf::from(format!("/proc/self/fd/{}", shared_dir.as_raw_fd()));
            fs::set_permissions(&self_anchor, fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    GateError::environment(format!(
                        "cannot lock down the heavy-gate parent {}: {error}",
                        shared.display()
                    ))
                },
            )?;
            shared_stat = fstat(shared_dir.as_raw_fd()).map_err(|errno| {
                GateError::environment(format!("cannot stat {}: {errno}", shared.display()))
            })?;
        }
        if !shared_parent_is_trusted(shared_stat.st_uid, shared_stat.st_mode as u32, uid) {
            return Err(GateError::environment(format!(
                "the heavy-gate parent {} is owned by uid {} with mode {:o}; refusing to \
                 share a semaphore namespace a peer could rename. Point XDG_RUNTIME_DIR at a \
                 per-user runtime directory, or have an administrator provision a root-owned \
                 (optionally sticky) {GATE_DIR_NAME} directory.",
                shared.display(),
                shared_stat.st_uid,
                shared_stat.st_mode as u32 & 0o7777,
            )));
        }

        // Create and open the per-uid directory beneath the verified shared
        // descriptor so a swap of the shared path cannot redirect us.
        let uid_name = format!("uid-{uid}");
        let uid_anchor = anchored_path(&shared_dir, &uid_name);
        create_dir_with_mode(&uid_anchor, 0o700)?;
        let dir = open_directory(&uid_anchor)?;
        let metadata = fstat(dir.as_raw_fd()).map_err(|errno| {
            GateError::environment(format!("cannot stat {}: {errno}", path.display()))
        })?;
        if metadata.st_uid != uid {
            return Err(GateError::environment(format!(
                "heavy-gate slot directory {} is owned by uid {}, not uid {}; \
                 refusing to share a semaphore across uids",
                path.display(),
                metadata.st_uid,
                uid
            )));
        }
        if metadata.st_mode as u32 & 0o077 != 0 {
            return Err(GateError::environment(format!(
                "heavy-gate slot directory {} has mode {:o}; it must not be \
                 group- or world-accessible. Remove it and rerun.",
                path.display(),
                metadata.st_mode as u32 & 0o7777
            )));
        }
        Ok(Self { path, dir })
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.path.join(format!("slot-{index}"))
    }

    /// The `/proc/self/fd`-anchored path to `slot-<index>`.
    ///
    /// Opening through the directory descriptor keeps every slot operation
    /// bound to the inode verified in [`prepare`], even if the path components
    /// are renamed afterwards.
    fn slot_anchor(&self, index: usize) -> PathBuf {
        anchored_path(&self.dir, &format!("slot-{index}"))
    }

    /// Verify that this filesystem really implements `F_OFD_SETLK`.
    ///
    /// Uses a process-private probe file so a concurrent lane can never make
    /// the probe look like a failure. Any errno that means "unsupported"
    /// fails closed here, before a single slot is touched.
    pub fn probe_ofd_support(&self) -> Result<()> {
        let probe_name = format!(".ofd-probe-{}", std::process::id());
        let probe_path = self.path.join(&probe_name);
        let probe_anchor = anchored_path(&self.dir, &probe_name);
        let _ = fs::remove_file(&probe_anchor);
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&probe_anchor)
            .map_err(|error| {
                GateError::environment(format!(
                    "cannot create heavy-gate probe file {}: {error}",
                    probe_path.display()
                ))
            })?;

        let result = self.evaluate_probe(&probe, &probe_path);
        drop(probe);
        let _ = fs::remove_file(&probe_anchor);
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
            .open(self.slot_anchor(index))
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

    /// Whether `fd` is genuine evidence that this process runs inside a slot an
    /// ancestor wrapper already holds, claiming the slot atomically as part of
    /// the proof.
    ///
    /// Returns `true` only when every one of the following holds, so a forged
    /// or stale [`GATE_ACTIVE_ENV`] marker can never skip acquisition:
    ///
    /// * `index` names a real slot (`index < SLOT_COUNT`);
    /// * `fd` is an open descriptor (`fstat` succeeds) on a regular file owned
    ///   by this uid;
    /// * that descriptor's device and inode match the canonical
    ///   `slot-<index>` file in this verified, uid-private gate directory, so
    ///   it cannot be an attacker-controlled file elsewhere; and
    /// * a nonblocking `F_OFD_SETLK` write lock issued *through the inherited
    ///   descriptor itself* succeeds.
    ///
    /// That final step is the atomic ownership proof. Placing the lock through
    /// `fd` (not a fresh handle) means:
    ///
    /// * if `fd`'s open file description already holds the slot lock - the only
    ///   way a genuine ancestor handoff looks - the call is idempotent and
    ///   succeeds, and the lock stays held through the exec shim because the
    ///   descriptor stays open;
    /// * if the slot is unheld, the lock is safely *acquired* on this same
    ///   description, so the nested run holds a real slot rather than running a
    ///   third lane unsynchronised; and
    /// * if any *other* open file description holds the slot, the call fails
    ///   with `EAGAIN`/`EACCES` and nesting is rejected.
    ///
    /// The previous form issued `F_OFD_GETLK` on a *fresh* handle, which proved
    /// only that *some* description held the slot - not that the advertised
    /// descriptor did - so a forged unlocked descriptor passed whenever any
    /// unrelated lane happened to hold that slot, and the lock could be dropped
    /// between the query and its use. Claiming through `fd` removes both the
    /// impersonation and the TOCTOU window.
    pub fn descriptor_is_locked_slot(&self, index: usize, fd: RawFd) -> bool {
        if index >= SLOT_COUNT || fd < 0 {
            return false;
        }
        let uid = getuid().as_raw();
        let Ok(inherited) = fstat(fd) else {
            return false;
        };
        if (inherited.st_mode & libc::S_IFMT) != libc::S_IFREG {
            return false;
        }
        if inherited.st_uid != uid {
            return false;
        }
        let Ok(slot) = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.slot_anchor(index))
        else {
            return false;
        };
        let Ok(slot_stat) = fstat(slot.as_raw_fd()) else {
            return false;
        };
        if slot_stat.st_dev != inherited.st_dev || slot_stat.st_ino != inherited.st_ino {
            return false;
        }
        // Atomic ownership proof: claim the slot lock through the inherited
        // descriptor itself. Success means this description now holds the slot
        // (idempotently, if it already did); contention means another
        // description owns it and nesting must be rejected. There is no window
        // between checking and using the lock because the check *is* the claim.
        match try_lock_fd(fd) {
            Ok(()) => true,
            Err(_) => false,
        }
    }
}

fn create_dir_with_mode(path: &Path, mode: u32) -> Result<()> {
    match fs::DirBuilder::new().mode(mode).create(path) {
        Ok(()) => {
            // `mkdir` masks the requested mode with the umask; force it back
            // so a restrictive umask cannot loosen the directory we just made.
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

/// Open `path` as a directory, refusing a symlinked final component.
///
/// `O_DIRECTORY` rejects a non-directory (`ENOTDIR`) and `O_NOFOLLOW` rejects a
/// symlink (`ELOOP`) so the returned descriptor is always a real directory,
/// and every later operation anchored to it stays bound to that inode.
fn open_directory(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(errno) if errno == libc::ELOOP => GateError::environment(format!(
                "{} is a symlink; refusing to use it as a heavy-gate directory",
                path.display()
            )),
            Some(errno) if errno == libc::ENOTDIR => {
                GateError::environment(format!("{} is not a directory", path.display()))
            }
            _ => GateError::environment(format!(
                "cannot open the heavy-gate directory {}: {error}",
                path.display()
            )),
        })
}

/// A `/proc/self/fd`-anchored path to `name` beneath the open directory `dir`.
///
/// Resolving through the descriptor pins the operation to the exact inode the
/// descriptor already refers to, so a rename of the path components cannot
/// redirect it to a different directory.
fn anchored_path(dir: &File, name: &str) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}/{name}", dir.as_raw_fd()))
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
     usage: cargo xtask heavy-gate verify-slot\n\
     \n\
     Runs <command> under the sole two-slot per-UID heavy-lane semaphore.\n\
     Every Layer-2, host-integration, hardware, live, and perf-heavy command\n\
     must be started this way; the `heavy-*` Makefile targets do it for you.\n\
     \n\
     The command inherits a duplicated handle on the locked slot (its number\n\
     is exported as D2B_HEAVY_GATE_SLOT_FD) and runs in its own process\n\
     group, which the wrapper signals and reaps.\n\
     \n\
     `verify-slot` takes no command: it exits 0 only when this process\n\
     genuinely holds a slot (proved via the inherited descriptor's inode,\n\
     ownership, and an atomic F_OFD_SETLK), and exits 3 otherwise. Shell and\n\
     Make guards use it so a bare, forgeable D2B_HEAVY_GATE cannot bypass the\n\
     semaphore.\n\
     \n\
     exit codes: 64 usage, 69 open file description locks unsupported,\n\
     71 cannot start the command, 72 gate directory unusable,\n\
     75 no slot within the wait ceiling, 3 verify-slot found no held slot.\n\
     Any other code is the command's own.";

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

/// How the inherited environment classifies this invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NestingMarker {
    /// No [`GATE_ACTIVE_ENV`] marker; this is a top-level invocation.
    TopLevel,
    /// A marker is present and the inherited descriptor was verified to be an
    /// open handle on the real, currently-locked slot it names.
    VerifiedSlot,
    /// A marker is present but does not refer to a genuinely held slot: it is
    /// forged, stale, closed, or names an unlocked slot.
    Unverifiable,
}

/// Read the inherited slot index and descriptor from the environment.
fn read_nesting_env() -> Option<(usize, RawFd)> {
    let index = std::env::var_os(SLOT_INDEX_ENV)?
        .to_str()?
        .parse::<usize>()
        .ok()?;
    let fd = std::env::var_os(SLOT_FD_ENV)?
        .to_str()?
        .parse::<RawFd>()
        .ok()?;
    Some((index, fd))
}

/// Classify this invocation against `dir`, verifying any nesting marker rather
/// than trusting its mere presence.
fn classify_nesting(dir: &GateDir) -> NestingMarker {
    if std::env::var_os(GATE_ACTIVE_ENV).is_none() {
        return NestingMarker::TopLevel;
    }
    match read_nesting_env() {
        Some((index, fd)) if dir.descriptor_is_locked_slot(index, fd) => {
            NestingMarker::VerifiedSlot
        }
        _ => NestingMarker::Unverifiable,
    }
}

/// Marker sub-operation selecting the slot-verification check.
///
/// Unlike the wrapped-command form, this takes no command: it inspects the
/// inherited environment, confirms the caller genuinely holds a heavy-gate
/// slot (matching inode, ownership, and an atomic `F_OFD_SETLK` ownership
/// proof through the inherited descriptor), and reports the verdict purely
/// through its exit status. Shell and Make guards use it so they cannot be
/// fooled by a bare, forgeable `D2B_HEAVY_GATE` marker.
pub const VERIFY_SLOT_OP: &str = "verify-slot";

/// Exit code when [`VERIFY_SLOT_OP`] confirms a genuinely held slot.
pub const VERIFY_SLOT_HELD: u8 = 0;

/// Exit code when [`VERIFY_SLOT_OP`] finds no genuinely held slot. Distinct
/// from every [`GateErrorKind`] code so a caller can tell "not in a slot" (an
/// expected branch that should acquire) apart from a gate malfunction.
pub const VERIFY_SLOT_UNHELD: u8 = 3;

/// Answer "does this process genuinely hold a heavy-gate slot?" for the shell
/// and Make guards.
///
/// Returns [`VERIFY_SLOT_HELD`] only when the inherited [`GATE_ACTIVE_ENV`]
/// marker is backed by a descriptor that passes the same inode, ownership, and
/// atomic `F_OFD_SETLK` ownership proof the nesting check uses. A top-level
/// invocation, a forged or stale marker, or a marker naming an unlocked or
/// foreign descriptor all yield [`VERIFY_SLOT_UNHELD`], so a guard that merely
/// exported `D2B_HEAVY_GATE` is told to acquire a real slot instead of running
/// heavy work. Resolving the gate directory itself failing is a real
/// environment error and propagates as such.
fn verify_slot() -> Result<u8> {
    let dir = GateDir::resolve()?;
    match classify_nesting(&dir) {
        NestingMarker::VerifiedSlot => Ok(VERIFY_SLOT_HELD),
        _ => Ok(VERIFY_SLOT_UNHELD),
    }
}

fn execute(args: &[String]) -> Result<u8> {
    // The internal re-exec shim resolves and verifies its inherited slot; a
    // forged marker cannot authorize it into running unsynchronized.
    if args.first().is_some_and(|first| first == EXEC_SHIM_FLAG) {
        let dir = GateDir::resolve()?;
        return exec_wrapped_command(&args[1..], classify_nesting(&dir));
    }

    // The slot-verification sub-operation reuses the atomic ownership proof
    // and reports its verdict through the exit status alone.
    if args.first().is_some_and(|first| first == VERIFY_SLOT_OP) {
        if args.len() != 1 {
            return Err(GateError::usage(format!(
                "{USAGE}\n\n`{VERIFY_SLOT_OP}` takes no arguments"
            )));
        }
        return verify_slot();
    }

    // Parse before touching the filesystem so `--help` works even where the
    // gate directory is unusable.
    let Some(request) = Request::parse(args)? else {
        println!("{USAGE}");
        return Ok(0);
    };

    let dir = GateDir::resolve()?;
    match classify_nesting(&dir) {
        NestingMarker::VerifiedSlot => {
            eprintln!(
                "heavy-gate: reusing the verified inherited slot instead of \
                 acquiring a second one"
            );
            supervise(&request, None)
        }
        marker => {
            if marker == NestingMarker::Unverifiable {
                eprintln!(
                    "heavy-gate: the inherited D2B_HEAVY_GATE marker does not refer to a \
                     genuinely held slot; acquiring a real slot rather than trusting it"
                );
            }
            dir.probe_ofd_support()?;
            let guard = acquire_slot(&dir, AcquirePolicy::default(), &mut StderrProgress)?;
            supervise(&request, Some(&guard))
        }
    }
}

/// The one-shot re-exec shim.
///
/// Runs in the already-forked child, in its own process group, holding the
/// inherited slot descriptor. It clears the signal mask the wrapper needed for
/// its `signalfd` and then replaces itself with the real command, so the
/// command keeps this pid and process group but starts with default signal
/// dispositions and an empty mask. It only returns on failure.
fn exec_wrapped_command(args: &[String], nesting: NestingMarker) -> Result<u8> {
    // The shim runs no slot acquisition of its own, so it must only ever be
    // reachable from a wrapper whose held slot we have actually verified.
    if nesting != NestingMarker::VerifiedSlot {
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
    let leader = group;

    let outcome = supervise_loop(
        // Observe the leader's exit *without reaping it*: `WNOWAIT` leaves the
        // zombie in place so its pid and pgid stay reserved until the sweep
        // has run. Reaping here (as `try_wait` would) frees the pgid and opens
        // a window in which the kernel could recycle it onto an unrelated
        // group before the unconditional `SIGKILL` sweep fires.
        || observe_exit(leader),
        || relay.drain(),
        |signal| {
            eprintln!("heavy-gate: forwarding {signal} to the heavy lane process group");
            let _ = killpg(group, signal);
        },
        || {
            let _ = killpg(group, Signal::SIGKILL);
        },
        // Reap the leader only after the final sweep has returned, so the
        // zombie pinned the pgid across the whole teardown. `Child::wait`
        // reaps the zombie `observe_exit` deliberately left behind.
        || {
            let _ = child.wait();
        },
        || sleep(POLL_INTERVAL),
        Instant::now,
    )?;

    Ok(resolve_exit_code(
        outcome.code,
        outcome.signal,
        outcome.interrupt.map(|signal| signal as i32),
    ))
}

/// Observe the process leader's terminal disposition without reaping it.
///
/// `waitid` with `WEXITED | WNOHANG | WNOWAIT` reports a terminated child but
/// leaves it as an unreaped zombie, which keeps its pid and process-group id
/// reserved. That reservation is what lets the caller sweep the group with
/// `SIGKILL` before reaping without risking the pgid being recycled onto an
/// unrelated group in between. `WNOHANG` keeps the poll nonblocking.
fn observe_exit(leader: Pid) -> Result<Option<ChildExit>> {
    let flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT;
    match waitid(Id::Pid(leader), flags) {
        Ok(WaitStatus::Exited(_, code)) => Ok(Some(ChildExit {
            code: Some(code),
            signal: None,
        })),
        Ok(WaitStatus::Signaled(_, signal, _)) => Ok(Some(ChildExit {
            code: None,
            signal: Some(signal as i32),
        })),
        Ok(_) => Ok(None),
        Err(errno) => Err(GateError::of(
            GateErrorKind::Spawn,
            format!("cannot observe the heavy-lane child: {errno}"),
        )),
    }
}

/// The child's terminal disposition, abstracted so the supervision loop can be
/// driven deterministically in tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChildExit {
    code: Option<i32>,
    signal: Option<i32>,
}

/// What the supervision loop observed over the wrapped lane's lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SuperviseOutcome {
    code: Option<i32>,
    signal: Option<i32>,
    interrupt: Option<Signal>,
}

/// The core supervision loop, factored out so its signal-drain ordering is
/// unit-testable with injected effects.
///
/// The ordering is load-bearing for slot release:
///
/// * Drain pending signals *before* every exit check, so a terminating signal
///   that arrives while the leader is exiting is forwarded, not lost.
/// * Once the child is observed exited, drain *again* while the forwarded
///   signal mask is still blocked, catching a signal delivered between the
///   last drain and the exit.
/// * Unconditionally sweep the whole process group with `SIGKILL` after the
///   leader exits, so no descendant can outlive the wrapper still holding the
///   inherited slot descriptor - even if a signal is pending that will kill
///   the wrapper the moment the mask is restored.
/// * Reap the leader only *after* that final sweep. The leader is observed as
///   an unreaped zombie, so its pid and pgid stay reserved across the sweep; a
///   pgid recycled onto an unrelated group between the exit and the `SIGKILL`
///   is therefore impossible.
fn supervise_loop<Poll, Drain, Forward, Sweep, Reap, Nap, Now>(
    mut poll_exit: Poll,
    mut drain: Drain,
    mut forward: Forward,
    mut sweep: Sweep,
    mut reap: Reap,
    mut nap: Nap,
    mut now: Now,
) -> Result<SuperviseOutcome>
where
    Poll: FnMut() -> Result<Option<ChildExit>>,
    Drain: FnMut() -> Vec<Signal>,
    Forward: FnMut(Signal),
    Sweep: FnMut(),
    Reap: FnMut(),
    Nap: FnMut(),
    Now: FnMut() -> Instant,
{
    let mut interrupt: Option<Signal> = None;
    let mut escalate_at: Option<Instant> = None;

    let exit = loop {
        for signal in drain() {
            forward(signal);
            if interrupt.is_none() {
                interrupt = Some(signal);
            }
            if escalate_at.is_none() {
                escalate_at = Some(now() + TERMINATION_GRACE);
            }
        }

        if let Some(exit) = poll_exit()? {
            break exit;
        }

        if let Some(deadline) = escalate_at
            && now() >= deadline
        {
            eprintln!(
                "heavy-gate: heavy lane still running {}s after the first signal; \
                 sending SIGKILL to its process group",
                TERMINATION_GRACE.as_secs()
            );
            sweep();
            escalate_at = None;
        }

        nap();
    };

    // The child has exited, but the forwarded-signal mask is still blocked.
    // Drain once more so a termination signal that landed between the final
    // drain and the exit is still recorded as an interruption for the exit
    // code.
    for signal in drain() {
        if interrupt.is_none() {
            interrupt = Some(signal);
        }
    }

    // Unconditionally sweep the whole process group with SIGKILL before the
    // caller restores the forwarded-signal mask. An earlier form swept only
    // when an interrupt had been *observed*, which left a gap: a signal
    // arriving after this final drain but before a conditional sweep stays
    // pending, and `SignalRelay::drop` then unblocks it and kills the wrapper
    // while descendants that inherited the slot descriptor are still alive,
    // orphaning the slot. Sweeping unconditionally guarantees the process
    // group is gone before the mask is restored, so a late pending signal can
    // only terminate an already-childless wrapper, which releases the slot as
    // it exits. The sweep runs while the leader is still an unreaped zombie so
    // its pid and pgid cannot be recycled onto an unrelated group; the caller
    // reaps the leader only after the sweep returns.
    sweep();

    // Reap the leader now that the group is gone. Doing this after the sweep
    // keeps the leader's zombie pinning the pgid for the whole teardown.
    reap();

    Ok(SuperviseOutcome {
        code: exit.code,
        signal: exit.signal,
        interrupt,
    })
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
            // Lock the scratch root down to 0700 so the gate's root-trust
            // check is deterministic regardless of the runner's umask: an
            // owned-but-group-writable root is (correctly) refused by prepare.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("scratch root mode is settable");
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
        // Neither a plain top-level invocation nor an unverifiable (forged or
        // stale) nesting marker may authorise the shim; only a verified slot
        // may. Otherwise a lane could start unsynchronised by naming it.
        for marker in [NestingMarker::TopLevel, NestingMarker::Unverifiable] {
            let error = exec_wrapped_command(&["--".into(), "true".into()], marker)
                .expect_err("the shim is refused without a verified slot");
            assert_eq!(error.kind(), GateErrorKind::Usage);
            assert!(
                !error.message().contains(EXEC_SHIM_FLAG),
                "the internal marker must stay out of operator-visible text: {}",
                error.message()
            );
        }
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
        // Opening the shared parent through the anchored root descriptor with
        // O_DIRECTORY|O_NOFOLLOW refuses a symlinked final component: depending
        // on how the kernel resolves it under /proc/self/fd this surfaces as
        // ELOOP ("symlink") or ENOTDIR ("not a directory"). Either way the
        // symlink is never followed into a decoy directory.
        assert!(
            error.message().contains("symlink") || error.message().contains("not a directory"),
            "a symlinked shared parent is refused without being followed: {}",
            error.message()
        );
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
    fn prepare_rejects_a_group_writable_root() {
        // A root we own but that is group- or world-writable is refused: a peer
        // in that directory could rename the whole `d2b-heavy-gates` tree out
        // from under a later invocation, splitting the semaphore into a second
        // namespace. Only a private (0700) or root-owned sticky root is trusted.
        let scratch = Scratch::new("loose-root");
        fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o777)).unwrap();

        let error = GateDir::prepare(scratch.path(), getuid().as_raw()).unwrap_err();
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(
            error.message().contains("heavy-gate root"),
            "the failure names the untrusted root: {}",
            error.message()
        );
        // Restore a sane mode so Scratch teardown can remove the tree.
        let _ = fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o700));
    }

    #[test]
    fn prepare_accepts_a_private_root() {
        // The common, safe case: a per-user runtime directory we own privately.
        let scratch = Scratch::new("private-root");
        let dir = GateDir::prepare(scratch.path(), getuid().as_raw())
            .expect("a 0700 owned root is trusted");
        assert!(dir.path().ends_with(format!("uid-{}", getuid().as_raw())));
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

    // ---- nesting is verified, not trusted ------------------------------

    #[test]
    fn shared_parent_trust_matrix_admits_only_unrenameable_namespaces() {
        let us = 1000;
        let peer = 1001;
        // Ours and private is fine; ours but group- or world-writable lets a
        // peer rename our slot directory, so it is refused.
        assert!(shared_parent_is_trusted(us, 0o700, us));
        assert!(shared_parent_is_trusted(us, 0o755, us));
        assert!(!shared_parent_is_trusted(us, 0o720, us));
        assert!(!shared_parent_is_trusted(us, 0o707, us));
        assert!(!shared_parent_is_trusted(us, 0o777, us));
        // Root-owned is fine when locked down, or when sticky (like /tmp) so a
        // peer can only add its own entry, never rename ours.
        assert!(shared_parent_is_trusted(0, 0o755, us));
        assert!(shared_parent_is_trusted(0, 0o1777, us));
        assert!(!shared_parent_is_trusted(0, 0o777, us));
        // A non-root peer owner is never trusted, even sticky: as owner it can
        // rename our slot directory regardless of the sticky bit.
        assert!(!shared_parent_is_trusted(peer, 0o700, us));
        assert!(!shared_parent_is_trusted(peer, 0o755, us));
        assert!(!shared_parent_is_trusted(peer, 0o1777, us));
    }

    #[test]
    fn prepare_locks_down_an_owned_world_writable_shared_parent() {
        // A stale or hostile-but-owned shared parent left world-writable lets
        // any peer rename our uid directory (non-sticky world-writable dirs
        // permit renaming any child). Because we own it, the remedy is to lock
        // it down to 0700 rather than fail, which also repairs a directory an
        // older build created world-writable. (A genuinely peer-*owned* parent
        // needs a second uid to set up, which requires privilege; that half of
        // the check is covered by
        // `shared_parent_trust_matrix_admits_only_unrenameable_namespaces`.)
        let scratch = Scratch::new("owned-loose-parent");
        let shared = scratch.path().join(GATE_DIR_NAME);
        fs::create_dir_all(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();

        let dir = GateDir::prepare(scratch.path(), getuid().as_raw())
            .expect("an owned parent is normalised, not rejected");
        drop(dir);

        let mode = fs::metadata(&shared).unwrap().permissions().mode() & 0o7777;
        assert_eq!(
            mode, 0o700,
            "the world-writable parent was locked down so no peer can rename our slot directory"
        );
    }

    #[test]
    fn operations_stay_anchored_across_a_rename_of_the_gate_directory() {
        let _serial = exclusive();
        let scratch = Scratch::new("rename-anchor");
        let uid = getuid().as_raw();
        let dir = gate_dir_under(scratch.path());

        // Move the prepared directory aside and drop a fresh decoy in its old
        // place. Because every slot operation is anchored to the descriptor
        // opened in `prepare`, acquisition must act on the moved inode, never
        // the decoy at the original path.
        let original = gate_dir_path(scratch.path(), uid);
        let moved = scratch.path().join(GATE_DIR_NAME).join("uid-moved");
        fs::rename(&original, &moved).unwrap();
        create_dir_with_mode(&original, 0o700).unwrap();

        let guard = dir
            .try_acquire(0)
            .unwrap()
            .expect("acquisition still works through the anchored descriptor");
        assert!(
            moved.join("slot-0").exists(),
            "the slot file follows the anchored inode into its new path"
        );
        assert!(
            !original.join("slot-0").exists(),
            "a pathname swap cannot redirect the semaphore to a decoy directory"
        );
        drop(guard);
    }

    #[test]
    fn a_forged_or_stale_nesting_marker_is_never_treated_as_a_held_slot() {
        let _serial = exclusive();
        let scratch = Scratch::new("forged-marker");
        let dir = gate_dir_under(scratch.path());

        // An index past the real slot count is rejected outright.
        assert!(!dir.descriptor_is_locked_slot(SLOT_COUNT, 0));
        // A negative descriptor is rejected.
        assert!(!dir.descriptor_is_locked_slot(0, -1));
        // A descriptor number that is not open (nothing was opened at it)
        // fails the `fstat`, so a forged D2B_HEAVY_GATE_SLOT_FD is worthless.
        assert!(!dir.descriptor_is_locked_slot(0, 4096));
    }

    #[test]
    fn a_closed_descriptor_marker_is_rejected() {
        let _serial = exclusive();
        let scratch = Scratch::new("closed-fd");
        let dir = gate_dir_under(scratch.path());

        // Open the real slot, capture its descriptor number, then close it.
        // A marker naming a now-closed descriptor must not count as a slot.
        let slot = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.slot_anchor(0))
            .expect("slot opens");
        let fd = slot.as_raw_fd();
        drop(slot);
        assert!(
            !dir.descriptor_is_locked_slot(0, fd),
            "a closed descriptor fails fstat and is not a held slot"
        );
    }

    #[test]
    fn an_uncontended_unlocked_descriptor_is_claimed_atomically() {
        let _serial = exclusive();
        let scratch = Scratch::new("unlocked-slot");
        let dir = gate_dir_under(scratch.path());

        // A live O_RDWR descriptor on the real slot inode with no other
        // description holding the lock. The atomic proof claims the lock
        // through this very descriptor, so verification accepts it and the
        // slot is now genuinely held on this description - the nested run
        // therefore occupies a real slot rather than running unsynchronised.
        let slot = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.slot_anchor(0))
            .expect("slot opens");
        assert!(
            dir.descriptor_is_locked_slot(0, slot.as_raw_fd()),
            "an uncontended descriptor is accepted and the slot is claimed atomically"
        );
        // Prove the lock is now genuinely held on that description: a separate
        // acquisition attempt on the same slot must now see it busy.
        assert!(
            dir.try_acquire(0).unwrap().is_none(),
            "the slot lock is held by the claimed descriptor, so a fresh acquisition is busy"
        );
        drop(slot);
    }

    #[test]
    fn a_separate_open_is_rejected_while_another_description_holds_the_slot() {
        let _serial = exclusive();
        let scratch = Scratch::new("separate-open");
        let dir = gate_dir_under(scratch.path());

        // A genuine lane holds slot 0 through one open file description.
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");

        // A forged marker supplies an independent O_RDWR open on the very same
        // slot inode - correct device and inode, current uid, regular file -
        // but from a *separate* open file description that does not hold the
        // lock. An earlier GETLK-on-a-fresh-handle form accepted this because
        // *some* description (the guard's) held the slot; the atomic
        // SETLK-through-the-inherited-descriptor form rejects it because the
        // claim conflicts with the guard's lock.
        let separate = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.slot_anchor(0))
            .expect("separate open succeeds");
        let separate_stat = fstat(separate.as_raw_fd()).expect("fstat succeeds");
        let slot_stat = fstat(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(dir.slot_anchor(0))
                .expect("slot reopens")
                .as_raw_fd(),
        )
        .expect("fstat succeeds");
        assert_eq!(
            (separate_stat.st_dev, separate_stat.st_ino),
            (slot_stat.st_dev, slot_stat.st_ino),
            "the separate open really does name the canonical slot inode, so only the \
             lock-ownership check can reject it"
        );
        assert!(
            !dir.descriptor_is_locked_slot(0, separate.as_raw_fd()),
            "a descriptor from a separate open is rejected while another description holds \
             the slot, so a forged marker cannot smuggle in a third lane"
        );
        drop(separate);
        drop(guard);
    }

    #[test]
    fn a_marker_naming_a_genuinely_locked_slot_is_accepted() {
        let _serial = exclusive();
        let scratch = Scratch::new("locked-slot");
        let dir = gate_dir_under(scratch.path());

        // Hold the slot the way the wrapper does, then hand a duplicate to a
        // notional child. The duplicate names the real, locked slot inode, so
        // verification must accept it.
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");
        let child_handle = guard.duplicate_for_child().expect("duplicate succeeds");
        assert!(
            dir.descriptor_is_locked_slot(0, child_handle.as_raw_fd()),
            "a live descriptor on the genuinely locked slot is accepted"
        );
        // A marker that points at the locked slot but names the *other* index
        // must not verify, because its inode will not match.
        assert!(
            !dir.descriptor_is_locked_slot(1, child_handle.as_raw_fd()),
            "the descriptor must match the slot index it claims"
        );
        drop(child_handle);
        drop(guard);
    }

    // ---- verify-slot through the real binary (the shell-guard path) ----
    //
    // These drive the actual `xtask heavy-gate verify-slot` binary the shell
    // and Make guards call, not the internal Rust helper, so they prove the
    // guard cannot be fooled by a bare `D2B_HEAVY_GATE` export. A slot
    // descriptor is handed to the child exactly as the gate does it: an open
    // handle with close-on-exec cleared so it survives `execve` at the same
    // fd number the environment advertises.

    /// Path to the built `xtask` binary next to this unit-test binary
    /// (`<target>/debug/xtask`), or `None` when only the library tests were
    /// built so the binary is absent.
    fn xtask_binary() -> Option<PathBuf> {
        let test_exe = std::env::current_exe().ok()?;
        let candidate = test_exe.parent()?.parent()?.join("xtask");
        candidate.is_file().then_some(candidate)
    }

    /// Clear close-on-exec so a handle is inherited by the child at the same
    /// fd number, mirroring [`SlotGuard::duplicate_for_child`].
    fn clear_cloexec(file: &File) {
        fcntl(
            file.as_raw_fd(),
            FcntlArg::F_SETFD(FdFlag::from_bits_truncate(0)),
        )
        .expect("close-on-exec is clearable");
    }

    /// Run `xtask heavy-gate verify-slot` with a controlled environment and
    /// return its exit code. `slot` is `Some((index, fd))` to advertise a slot
    /// marker, or `None` to export only the bare, forgeable `D2B_HEAVY_GATE`.
    fn run_verify_slot(
        xtask: &Path,
        root: &Path,
        marker: bool,
        slot: Option<(usize, RawFd)>,
    ) -> i32 {
        let mut command = Command::new(xtask);
        command
            .args(["heavy-gate", VERIFY_SLOT_OP])
            .env("XDG_RUNTIME_DIR", root)
            .env_remove("TMPDIR")
            .env_remove(GATE_ACTIVE_ENV)
            .env_remove(SLOT_INDEX_ENV)
            .env_remove(SLOT_FD_ENV)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if marker {
            command.env(GATE_ACTIVE_ENV, "1");
        }
        if let Some((index, fd)) = slot {
            command.env(SLOT_INDEX_ENV, index.to_string());
            command.env(SLOT_FD_ENV, fd.to_string());
        }
        command
            .status()
            .expect("the verify-slot binary runs")
            .code()
            .expect("verify-slot exits normally")
    }

    #[test]
    fn verify_slot_rejects_a_bare_marker_through_the_binary() {
        let _serial = exclusive();
        let Some(xtask) = xtask_binary() else {
            eprintln!("skipping: xtask binary not built next to the test binary");
            return;
        };
        let scratch = Scratch::new("verify-bare");

        // Exactly the headline bypass: export the forgeable marker with no
        // real slot descriptor. verify-slot must report no held slot so the
        // shell guard acquires a real slot instead of running heavy work.
        let code = run_verify_slot(&xtask, scratch.path(), true, None);
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a bare D2B_HEAVY_GATE export is not a held slot"
        );
    }

    #[test]
    fn verify_slot_rejects_a_forged_foreign_descriptor_through_the_binary() {
        let _serial = exclusive();
        let Some(xtask) = xtask_binary() else {
            return;
        };
        let scratch = Scratch::new("verify-forged");
        let _dir = gate_dir_under(scratch.path());

        // A descriptor on a file the caller controls that is NOT the canonical
        // slot inode. Correct uid, regular file, but the inode will not match
        // slot-0, so verify-slot rejects it.
        let bogus_path = scratch.path().join("not-a-slot");
        let bogus = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&bogus_path)
            .expect("the bogus file opens");
        clear_cloexec(&bogus);

        let code = run_verify_slot(&xtask, scratch.path(), true, Some((0, bogus.as_raw_fd())));
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a descriptor on a foreign file is not a held slot"
        );
        drop(bogus);
    }

    #[test]
    fn verify_slot_rejects_a_closed_descriptor_through_the_binary() {
        let _serial = exclusive();
        let Some(xtask) = xtask_binary() else {
            return;
        };
        let scratch = Scratch::new("verify-closed");
        let dir = gate_dir_under(scratch.path());

        // Capture a real slot fd number, then close it. A marker naming a
        // now-closed descriptor fails fstat in the child.
        let slot = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.slot_anchor(0))
            .expect("slot opens");
        let fd = slot.as_raw_fd();
        drop(slot);

        let code = run_verify_slot(&xtask, scratch.path(), true, Some((0, fd)));
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a closed descriptor is not a held slot"
        );
    }

    #[test]
    fn verify_slot_rejects_a_separate_open_while_another_lane_holds_it() {
        let _serial = exclusive();
        let Some(xtask) = xtask_binary() else {
            return;
        };
        let scratch = Scratch::new("verify-contended");
        let dir = gate_dir_under(scratch.path());

        // A genuine lane holds slot 0 through one open file description.
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");

        // The caller presents a *separate* open on the same slot inode that
        // does not hold the lock. The atomic ownership proof through that
        // descriptor conflicts with the guard's lock, so verify-slot rejects
        // it: a forged marker cannot smuggle in a third lane.
        let separate = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.slot_anchor(0))
            .expect("separate open succeeds");
        clear_cloexec(&separate);

        let code = run_verify_slot(
            &xtask,
            scratch.path(),
            true,
            Some((0, separate.as_raw_fd())),
        );
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a separate open is rejected while another description holds the slot"
        );
        drop(separate);
        drop(guard);
    }

    #[test]
    fn verify_slot_accepts_a_genuinely_held_slot_through_the_binary() {
        let _serial = exclusive();
        let Some(xtask) = xtask_binary() else {
            return;
        };
        let scratch = Scratch::new("verify-held");
        let dir = gate_dir_under(scratch.path());

        // Hold the slot the way the wrapper does, hand the child an inherited
        // duplicate on the *same* locked description. The atomic proof through
        // that descriptor is idempotent, so verify-slot confirms a held slot.
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");
        let child_handle = guard.duplicate_for_child().expect("duplicate succeeds");

        let code = run_verify_slot(
            &xtask,
            scratch.path(),
            true,
            Some((0, child_handle.as_raw_fd())),
        );
        assert_eq!(
            code, VERIFY_SLOT_HELD as i32,
            "a descriptor on the genuinely locked slot is a held slot"
        );
        drop(child_handle);
        drop(guard);
    }

    // ---- supervision drains signals around the leader's exit -----------

    #[test]
    fn supervise_loop_sweeps_the_group_when_a_signal_races_the_leader_exit() {
        use std::cell::Cell;

        let poll_calls = Cell::new(0u32);
        let drain_calls = Cell::new(0u32);
        let forwarded = Cell::new(0u32);
        let sweeps = Cell::new(0u32);
        let reap_after_sweep = Cell::new(false);

        let outcome = supervise_loop(
            || {
                let n = poll_calls.get();
                poll_calls.set(n + 1);
                if n == 0 {
                    Ok(None)
                } else {
                    Ok(Some(ChildExit {
                        code: Some(0),
                        signal: None,
                    }))
                }
            },
            || {
                let n = drain_calls.get();
                drain_calls.set(n + 1);
                // The terminating signal only becomes visible on the
                // post-exit drain: exactly the race that previously orphaned
                // a slot holder.
                if n == 2 {
                    vec![Signal::SIGTERM]
                } else {
                    Vec::new()
                }
            },
            |_signal| forwarded.set(forwarded.get() + 1),
            || sweeps.set(sweeps.get() + 1),
            || reap_after_sweep.set(sweeps.get() == 1),
            || {},
            Instant::now,
        )
        .expect("the loop completes");

        assert!(
            reap_after_sweep.get(),
            "the leader is reaped only after the group is swept, so its zombie pins the pgid"
        );
        assert_eq!(
            outcome.interrupt,
            Some(Signal::SIGTERM),
            "a signal pending at leader exit is still recorded as an interruption"
        );
        assert_eq!(outcome.code, Some(0));
        assert_eq!(
            forwarded.get(),
            0,
            "a signal observed only after exit is not forwarded to a dead group"
        );
        assert_eq!(
            sweeps.get(),
            1,
            "the group is swept once so no descendant keeps holding the slot"
        );
        assert_eq!(poll_calls.get(), 2);
        assert_eq!(
            drain_calls.get(),
            3,
            "drained before each poll and once after exit"
        );
    }

    #[test]
    fn supervise_loop_forwards_and_sweeps_a_signal_seen_while_running() {
        use std::cell::Cell;

        let poll_calls = Cell::new(0u32);
        let drain_calls = Cell::new(0u32);
        let forwarded = Cell::new(0u32);
        let sweeps = Cell::new(0u32);

        let outcome = supervise_loop(
            || {
                let n = poll_calls.get();
                poll_calls.set(n + 1);
                if n == 0 {
                    Ok(None)
                } else {
                    Ok(Some(ChildExit {
                        code: Some(0),
                        signal: None,
                    }))
                }
            },
            || {
                let n = drain_calls.get();
                drain_calls.set(n + 1);
                if n == 0 {
                    vec![Signal::SIGINT]
                } else {
                    Vec::new()
                }
            },
            |_signal| forwarded.set(forwarded.get() + 1),
            || sweeps.set(sweeps.get() + 1),
            || {},
            || {},
            Instant::now,
        )
        .expect("the loop completes");

        assert_eq!(outcome.interrupt, Some(Signal::SIGINT));
        assert_eq!(
            forwarded.get(),
            1,
            "a signal seen while the lane runs is forwarded to its group"
        );
        assert_eq!(
            sweeps.get(),
            1,
            "the interrupted run sweeps the group exactly once"
        );
    }

    #[test]
    fn supervise_loop_sweeps_the_group_even_after_a_clean_exit() {
        use std::cell::Cell;

        let sweeps = Cell::new(0u32);
        let reaps = Cell::new(0u32);
        let reap_after_sweep = Cell::new(false);
        let outcome = supervise_loop(
            || {
                Ok(Some(ChildExit {
                    code: Some(3),
                    signal: None,
                }))
            },
            Vec::new,
            |_signal| panic!("no signal should be forwarded on a clean exit"),
            || sweeps.set(sweeps.get() + 1),
            || {
                reaps.set(reaps.get() + 1);
                reap_after_sweep.set(sweeps.get() == 1);
            },
            || {},
            Instant::now,
        )
        .expect("the loop completes");

        assert_eq!(outcome.interrupt, None);
        assert_eq!(outcome.code, Some(3));
        assert_eq!(reaps.get(), 1, "the leader is reaped exactly once");
        assert!(
            reap_after_sweep.get(),
            "the leader is reaped after the sweep, so the zombie pins its pid and pgid across \
             teardown and the pgid cannot be recycled onto an unrelated group"
        );
        assert_eq!(
            sweeps.get(),
            1,
            "the group is swept once even on a clean exit, so a pending signal that kills the \
             wrapper after the mask is restored cannot leave a slot-holding descendant behind"
        );
    }

    /// Extract the recipe body (the tab-indented command lines) for a Makefile
    /// rule so the inventory guard can assert what it routes through. Returns
    /// `None` when no such rule exists.
    fn makefile_recipe(makefile: &str, target: &str) -> Option<String> {
        let mut in_recipe = false;
        let mut recipe = String::new();
        for line in makefile.lines() {
            if !in_recipe {
                if line.starts_with('\t') {
                    continue;
                }
                if let Some((lhs, _)) = line.split_once(':')
                    && lhs.split_whitespace().any(|t| t == target)
                {
                    in_recipe = true;
                }
                continue;
            }
            if let Some(command) = line.strip_prefix('\t') {
                recipe.push_str(command);
                recipe.push('\n');
            } else {
                break;
            }
        }
        if in_recipe { Some(recipe) } else { None }
    }

    /// True when `line` opens a Makefile rule (`target[s]: [prereqs]`) rather
    /// than a variable assignment (`X := ...`, `X = ...`, `X ?= ...`,
    /// `X += ...`), a recipe body line, a comment, or a blank line.
    fn makefile_rule_parts(line: &str) -> Option<(&str, &str)> {
        if line.starts_with('\t') || line.starts_with('#') || line.trim().is_empty() {
            return None;
        }
        let (lhs, rhs) = line.split_once(':')?;
        // `:=` immediate assignment: the ':' we split on is the assignment
        // operator, so the right side starts with '='.
        if rhs.starts_with('=') {
            return None;
        }
        // `=`, `?=`, `+=` assignments keep the operator on the left of any ':'.
        if lhs.contains('=') {
            return None;
        }
        Some((lhs, rhs))
    }

    /// Every rule target name declared in the Makefile.
    fn makefile_targets(makefile: &str) -> Vec<String> {
        let mut targets = Vec::new();
        for line in makefile.lines() {
            if let Some((lhs, _)) = makefile_rule_parts(line) {
                for tok in lhs.split_whitespace() {
                    targets.push(tok.to_string());
                }
            }
        }
        targets
    }

    /// The prerequisite tokens declared for `target`, or `None` when the
    /// Makefile has no rule for it.
    fn makefile_prereqs(makefile: &str, target: &str) -> Option<Vec<String>> {
        for line in makefile.lines() {
            if let Some((lhs, rhs)) = makefile_rule_parts(line)
                && lhs.split_whitespace().any(|t| t == target)
            {
                return Some(rhs.split_whitespace().map(str::to_string).collect());
            }
        }
        None
    }

    /// Recursively collect every heavy-entrypoint candidate under `dir`: a
    /// shell script (`.sh`) or any executable regular file. A data file that is
    /// neither is ignored, so a non-`.sh` executable entrypoint cannot slip in
    /// unguarded. Returns an empty vec when `dir` is absent (optional lanes).
    fn collect_heavy_entrypoints(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries {
            let path = entry.expect("a readable dir entry").path();
            let meta = fs::symlink_metadata(&path)
                .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));
            if meta.file_type().is_dir() {
                out.extend(collect_heavy_entrypoints(&path));
                continue;
            }
            if !meta.file_type().is_file() {
                continue;
            }
            let is_sh = path.extension() == Some(OsStr::new("sh"));
            let is_exec = meta.permissions().mode() & 0o111 != 0;
            if is_sh || is_exec {
                out.push(path);
            }
        }
        out.sort();
        out
    }

    /// Inventory guard for the sole-use invariant: every live, hardware,
    /// benchmark, cloud, and performance entrypoint must route through the
    /// heavy-gate semaphore, so a future lane cannot be added that silently
    /// bypasses it. This is a CLOSED-WORLD guard - it walks the on-disk
    /// entrypoints recursively and parses the Makefile for every heavy lane
    /// rather than checking a hand-maintained list. Adding a new heavy
    /// entrypoint (a nested or non-`.sh` script, a new aggregating runner, a
    /// new `heavy-lane-*` make target, or a new public delegation) fails this
    /// test until it is gated.
    #[test]
    fn every_live_and_heavy_entrypoint_routes_through_the_gate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("packages/xtask resolves to a repo root");

        // The shared self-guard the shell entrypoints call. Its presence proves
        // the script asks the wrapper to verify a genuinely-held slot rather
        // than trusting the mere presence of the D2B_HEAVY_GATE variable.
        let self_guard_token = "d2b_heavy_gate_reexec";

        // 1. Filesystem entrypoints. Walk every heavy-lane directory
        //    recursively and require an executable self-guard on each script.
        //    performance-budgets.sh lives outside those directories, so it is
        //    named explicitly. Optional directories (benchmark, cloud) are
        //    walked when present and simply contribute nothing when absent.
        let mut entrypoints: Vec<PathBuf> = Vec::new();
        for dir in [
            "tests/integration/live",
            "tests/host-integration/hardware",
            "tests/benchmark",
            "tests/integration/cloud",
        ] {
            entrypoints.extend(collect_heavy_entrypoints(&root.join(dir)));
        }
        let perf = root.join("tests/unit/gates/performance-budgets.sh");
        assert!(
            perf.is_file(),
            "expected the performance-budgets entrypoint at {}",
            perf.display()
        );
        entrypoints.push(perf.clone());
        entrypoints.sort();
        entrypoints.dedup();

        let live_count = entrypoints
            .iter()
            .filter(|p| p.starts_with(root.join("tests/integration/live")))
            .count();
        assert!(
            live_count >= 7,
            "expected the known live-lane scripts to be discovered; found {live_count}"
        );

        for script in &entrypoints {
            let meta = fs::symlink_metadata(script)
                .unwrap_or_else(|e| panic!("cannot stat {}: {e}", script.display()));
            assert!(
                meta.permissions().mode() & 0o111 != 0,
                "heavy entrypoint {} is not executable; a non-executable entrypoint invites a \
                 caller to run it with a bare `bash <script>` that skips any wrapper",
                script.display()
            );
            let body = fs::read_to_string(script)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", script.display()));
            assert!(
                body.contains(self_guard_token),
                "heavy entrypoint {} does not call `{self_guard_token}`; add the shared \
                 self-guard so a direct invocation verifies a genuinely-held slot and re-execs \
                 through the gate instead of running heavy work unguarded",
                script.display()
            );
        }

        // 2. The aggregating runner and the layer dispatcher (which drives the
        //    hardware and perf lanes) must also route through the same
        //    verifying self-guard.
        for relative in ["tests/runner.sh", "tests/tools/run-layer.sh"] {
            let path = root.join(relative);
            let body = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            assert!(
                body.contains(self_guard_token),
                "{relative} does not route through the verifying heavy-gate self-guard"
            );
        }

        // 3. The shared self-guard itself must verify a held slot, not trust
        //    the bare marker. This is what closed the relocated bypass.
        let helper = root.join("tests/tools/heavy-gate-reexec.sh");
        let helper_body = fs::read_to_string(&helper)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", helper.display()));
        assert!(
            helper_body.contains("heavy-gate verify-slot"),
            "the shared self-guard must call `heavy-gate verify-slot`; without the ownership \
             proof it would fall back to trusting the forgeable D2B_HEAVY_GATE marker"
        );

        // 4. Makefile closed-world. Enumerate every `heavy-lane-*` target from
        //    the Makefile (not a hardcoded list). The guard target itself is
        //    the gate; every other heavy lane must (a) depend on it and (b) be
        //    reachable only through a public `$(HEAVY_GATE) $(MAKE) <lane>`
        //    delegation. A new raw heavy lane therefore fails this test until
        //    it is both guarded and delegated.
        let makefile =
            fs::read_to_string(root.join("Makefile")).expect("the repo Makefile is readable");

        let mut heavy_lanes: Vec<String> = makefile_targets(&makefile)
            .into_iter()
            .filter(|t| t.starts_with("heavy-lane-") && t != "heavy-lane-guard")
            .collect();
        heavy_lanes.sort();
        heavy_lanes.dedup();
        assert!(
            !heavy_lanes.is_empty(),
            "expected at least one heavy-lane-* work target in the Makefile"
        );

        for lane in &heavy_lanes {
            let prereqs = makefile_prereqs(&makefile, lane)
                .unwrap_or_else(|| panic!("the Makefile has no rule for `{lane}`"));
            assert!(
                prereqs.iter().any(|p| p == "heavy-lane-guard"),
                "heavy lane `{lane}` does not list `heavy-lane-guard` as a prerequisite, so its \
                 raw work could run without the slot-ownership check"
            );
            let delegation = format!("$(HEAVY_GATE) $(MAKE) {lane}");
            assert!(
                makefile.contains(&delegation),
                "heavy lane `{lane}` has no public `{delegation}` delegation; a raw lane with no \
                 gate-acquiring public entrypoint can only be run by bypassing the semaphore"
            );
        }

        // 5. The guard target's recipe must enforce ownership by calling
        //    `heavy-gate verify-slot` (the exclusive gating recipe), not by
        //    testing the bare D2B_HEAVY_GATE variable.
        let guard_recipe = makefile_recipe(&makefile, "heavy-lane-guard")
            .expect("the Makefile defines heavy-lane-guard");
        assert!(
            guard_recipe.contains("heavy-gate verify-slot"),
            "heavy-lane-guard must verify a genuinely-held slot via `heavy-gate verify-slot`; \
             testing D2B_HEAVY_GATE alone is the forgeable-marker bypass:\n{guard_recipe}"
        );

        // 6. static.sh routing. static.sh invokes performance-budgets.sh
        //    directly; that is safe only because the perf script self-guards
        //    (asserted in step 1). Require the invocation to be present so the
        //    routing stays wired, and require perf to be in the guarded set.
        let static_sh =
            fs::read_to_string(root.join("tests/static.sh")).expect("tests/static.sh is readable");
        assert!(
            static_sh.contains("performance-budgets.sh"),
            "tests/static.sh no longer references performance-budgets.sh; if the perf canary moved \
             its new entrypoint must remain in this inventory guard"
        );
        assert!(
            entrypoints.contains(&perf),
            "the performance-budgets entrypoint must be in the guarded set so static.sh's direct \
             invocation cannot bypass the semaphore"
        );
    }
}
