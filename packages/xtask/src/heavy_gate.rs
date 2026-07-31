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
//! * Exactly [`SLOT_COUNT`] slots, scoped to the invoking uid, living under the
//!   system-provisioned `/run/d2b-heavy-gates/uid-<uid>/`. The root and per-uid
//!   directory are root-owned and non-writable by unprivileged users, and the
//!   two uid-owned slot files are provisioned in advance. No component that
//!   names a slot can therefore be squatted, unlinked, or renamed by a peer or
//!   by the invoking uid while another lane holds its inode. If this fixed
//!   namespace is absent or malformed, the gate fails closed; it never falls
//!   back to a user-owned runtime or temporary directory.
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
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
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
use rustix::fs::{Mode, OFlags};

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

/// The single, fixed, system-provisioned heavy-gate root.
///
/// `/run/d2b-heavy-gates` is chosen as a root-owned runtime namespace, not as a
/// user-writable place where this process creates another directory. Its parent
/// (`/run`), the root itself, and every `uid-<uid>` slot directory must be
/// root-owned and non-writable by group or other. The two `slot-*` files are
/// pre-created, owned by the target uid, and mode `0600`. Consequently neither a
/// foreign uid nor the target uid can replace a name while a lock remains on the
/// old inode.
///
/// The namespace deliberately has no fallback. `/run/user/<uid>` is not always
/// present and is owned by the uid, so selecting it conditionally would revive
/// the split-pool race and placing slot names inside it would let that uid rename
/// them. An absent provisioned root is therefore an environment error with a
/// bounded provisioning diagnostic, never permission to use a weaker location.
const CANONICAL_GATE_ROOT: &str = "/run/d2b-heavy-gates";

/// Semantic labels for the fixed heavy-gate namespace components.
///
/// Every operator- and CI-facing diagnostic names the directory or file by its
/// ROLE, never by its absolute path or the caller's uid. The gate paths embed
/// both the root and the uid (`/run/d2b-heavy-gates/uid-<uid>`), and these
/// diagnostics reach stderr and CI logs verbatim, so interpolating the resolved
/// path or the raw uid would leak them. Naming the role instead keeps the
/// message actionable without disclosing either. The remediation text uses the
/// shell-style `$UID` placeholder rather than the literal number.
mod gate_label {
    /// The single fixed, system-provisioned per-host root.
    pub const ROOT: &str = "the heavy-gate root directory";
    /// The per-uid `uid-$UID` slot directory.
    pub const SLOT_DIR: &str = "the per-uid heavy-gate slot directory";

    /// The `slot-<index>` file within the per-uid slot directory.
    pub fn slot(index: usize) -> String {
        format!("heavy-gate slot {index}")
    }
}

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

const PROVISIONING_ERROR_CODE: &str = "heavy-gate-provisioning-required";
const PROVISIONING_ACTION: &str = "make heavy-gate-provision";

/// Build the stable operator-facing diagnostic for an unavailable or malformed
/// protected namespace.
///
/// The observed state and remediation are intentionally path- and uid-free.
/// Detailed errors may name only the semantic labels in [`gate_label`].
fn provisioning_error(observed: &str, detail: impl fmt::Display) -> GateError {
    GateError::environment(format!(
        "code: {PROVISIONING_ERROR_CODE}; observed: {observed}; remediation: run \
         `{PROVISIONING_ACTION}`; detail: {detail}; no fallback namespace was used"
    ))
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

/// The verdict of proving whether an inherited descriptor genuinely holds a
/// heavy-gate slot.
///
/// This deliberately separates a legitimate "no slot is held" answer from a
/// verifier *malfunction*. A malfunction (a lock mechanism that is unsupported
/// here, or an environment error touching the slot file) is returned as an
/// [`Err`] so callers can fail closed instead of mistaking it for "unheld" and
/// re-execing forever against a broken environment. The two non-error verdicts
/// are:
///
/// * [`SlotProof::Held`] - the inherited descriptor is proven to hold the slot
///   lock, so the caller may proceed under it; and
/// * [`SlotProof::NotHeld`] - there is no genuinely held slot to inherit (no
///   marker, a forged or stale marker, an absent slot file, a foreign owner, a
///   mismatched inode, or a slot another open file description holds), so the
///   caller should acquire a real slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotProof {
    /// The inherited descriptor is proven to hold the slot lock.
    Held,
    /// No genuinely held slot backs the marker; the caller must acquire one.
    NotHeld,
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

/// The single canonical namespace root.
///
/// Non-overridable and not a function of runtime-directory existence: in a
/// released binary the root is the constant [`CANONICAL_GATE_ROOT`], never
/// `TMPDIR`, `XDG_RUNTIME_DIR`, `/run/user/<uid>`, or any other value that an
/// attacker - or a well-meaning operator wiring up two lanes - could set or
/// cause to differ between concurrent invocations. The fixed root and its
/// per-uid directories are provisioned by root, so the uid that owns the slot
/// files cannot rename a directory or slot name and mint fresh lock inodes.
///
/// `test_root` is an injectable seam used *exclusively* by this crate's own
/// tests, which is why the function stays pure (no environment or filesystem
/// access) and its precedence is directly unit-testable. The production caller
/// always passes `None` (see [`GateDir::resolve`]), so a released binary has
/// no code path that consults a caller-selected root.
#[cfg(test)]
pub fn gate_root_from(_uid: u32, test_root: Option<&Path>) -> PathBuf {
    match test_root {
        Some(root) => root.to_path_buf(),
        None => PathBuf::from(CANONICAL_GATE_ROOT),
    }
}

/// Per-uid slot directory under `root`.
#[cfg(test)]
pub fn gate_dir_path(root: &Path, uid: u32) -> PathBuf {
    root.join(format!("uid-{uid}"))
}

/// Whether a system-provisioned directory prevents every unprivileged uid,
/// including the slot owner, from replacing entries beneath it.
///
/// Sticky world-writable directories are deliberately rejected. Sticky
/// semantics stop one uid from renaming another uid's entry, but the owner of an
/// entry may still rename it; that is the exact inode-splitting attack the gate
/// must prevent. Only a root-owned directory with no group/other write bit is a
/// valid parent for the fixed namespace and slot names.
pub fn system_gate_parent_is_trusted(owner_uid: u32, mode: u32) -> bool {
    owner_uid == 0 && mode & 0o022 == 0
}

/// A prepared, ownership-checked per-uid slot directory anchored to a verified
/// open directory descriptor.
#[derive(Debug)]
pub struct GateDir {
    /// The resolved per-uid slot directory path. Retained only for test
    /// assertions on the filesystem layout; production diagnostics name the
    /// directory by role, never by path, so no operator- or CI-facing surface
    /// reads it.
    #[cfg(test)]
    path: PathBuf,
    dir: File,
    /// Test namespaces create their slot fixtures lazily. Released binaries
    /// always set this to false and require both slot files to be provisioned.
    create_slots: bool,
}

impl GateDir {
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve the gate directory from the single provisioned namespace root.
    ///
    /// A released binary accepts only [`CANONICAL_GATE_ROOT`] and never creates
    /// any namespace component. Missing or unsafe provisioning fails closed with
    /// no fallback. Unit-test child modes may inject a private scratch root; that
    /// seam is compiled out of released binaries.
    pub fn resolve() -> Result<Self> {
        let uid = getuid().as_raw();
        #[cfg(test)]
        if let Some(root) = Self::test_root_override() {
            return Self::prepare_test(&root, uid);
        }
        Self::open_provisioned(Path::new(CANONICAL_GATE_ROOT), uid)
    }

    /// The test-only root override.
    ///
    /// In the crate's test build the child-mode helpers re-exec this same
    /// binary with `XDG_RUNTIME_DIR` pointed at a per-test scratch directory,
    /// so tests never touch the real namespace. In a released binary this
    /// always returns `None`: the production root is the fixed constant
    /// [`CANONICAL_GATE_ROOT`] and no environment variable can redirect it.
    #[cfg(test)]
    fn test_root_override() -> Option<PathBuf> {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    /// Open a fully provisioned root and per-uid slot directory.
    ///
    /// Nothing is created or repaired here. Both directories must be root-owned
    /// and non-writable by unprivileged users, and both slot files must already
    /// exist as private regular files owned by `uid`. This makes every pathname
    /// in the lock identity immutable to the uid that runs heavy work.
    fn open_provisioned(root: &Path, uid: u32) -> Result<Self> {
        let root_dir = open_directory(root, gate_label::ROOT).map_err(|error| {
            provisioning_error(
                "the protected heavy-gate root is unavailable",
                error.message(),
            )
        })?;
        verify_system_directory(&root_dir, gate_label::ROOT)?;

        let uid_name = format!("uid-{uid}");
        let dir =
            open_directory_at(&root_dir, &uid_name, gate_label::SLOT_DIR).map_err(|error| {
                provisioning_error(
                    "the protected per-user slot directory is unavailable",
                    error.message(),
                )
            })?;
        verify_system_directory(&dir, gate_label::SLOT_DIR)?;

        let gate = Self {
            #[cfg(test)]
            path: gate_dir_path(root, uid),
            dir,
            create_slots: false,
        };
        for index in 0..SLOT_COUNT {
            gate.open_slot(index, false)?.ok_or_else(|| {
                provisioning_error(
                    "a provisioned slot is unavailable",
                    format!("{} is absent", gate_label::slot(index)),
                )
            })?;
        }
        Ok(gate)
    }

    /// Build a uid-owned scratch namespace for this module's tests.
    ///
    /// Released binaries never call this path. It exists only because an
    /// unprivileged test cannot synthesize the root-owned production layout.
    #[cfg(test)]
    fn prepare_test(root: &Path, uid: u32) -> Result<Self> {
        let root_dir = open_directory(root, gate_label::ROOT)?;
        let root_stat = fstat(root_dir.as_raw_fd()).map_err(|errno| {
            GateError::environment(format!("cannot stat {}: {errno}", gate_label::ROOT))
        })?;
        if root_stat.st_uid != uid || root_stat.st_mode as u32 & 0o022 != 0 {
            return Err(GateError::environment(format!(
                "{} is not an owned, non-writable test root",
                gate_label::ROOT
            )));
        }

        let uid_name = format!("uid-{uid}");
        let dir = open_or_create_directory_at(&root_dir, &uid_name, gate_label::SLOT_DIR, 0o700)?;
        let metadata = fstat(dir.as_raw_fd()).map_err(|errno| {
            GateError::environment(format!("cannot stat {}: {errno}", gate_label::SLOT_DIR))
        })?;
        if metadata.st_uid != uid || metadata.st_mode as u32 & 0o077 != 0 {
            return Err(GateError::environment(format!(
                "{} must be owned by the test uid and private (0700)",
                gate_label::SLOT_DIR
            )));
        }
        Ok(Self {
            path: gate_dir_path(root, uid),
            dir,
            create_slots: true,
        })
    }

    #[cfg(test)]
    fn slot_path(&self, index: usize) -> PathBuf {
        self.path.join(format!("slot-{index}"))
    }

    /// Open one slot directly beneath the pinned directory descriptor.
    ///
    /// Production never creates a slot: a missing file is unsafe provisioning.
    /// Unit-test scratch namespaces opt into `O_CREAT`, then pin and normalise
    /// the resulting descriptor without re-resolving a pathname.
    fn open_slot(&self, index: usize, missing_is_ok: bool) -> Result<Option<File>> {
        let name = format!("slot-{index}");
        let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        if self.create_slots {
            flags |= OFlags::CREATE;
        }
        let fd = match rustix::fs::openat(
            self.dir.as_fd(),
            name.as_str(),
            flags,
            Mode::from_bits_truncate(0o600),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) if missing_is_ok && self.create_slots => {
                return Ok(None);
            }
            Err(rustix::io::Errno::NOENT) => {
                return Err(provisioning_error(
                    "a provisioned slot is unavailable",
                    format!("{} is absent", gate_label::slot(index)),
                ));
            }
            Err(error) => {
                return Err(provisioning_error(
                    "a provisioned slot is unusable",
                    format!(
                        "cannot open {} relative to the pinned slot directory: {error}",
                        gate_label::slot(index)
                    ),
                ));
            }
        };
        if self.create_slots {
            rustix::fs::fchmod(fd.as_fd(), Mode::from_bits_truncate(0o600)).map_err(|error| {
                GateError::environment(format!(
                    "cannot set the private mode on {}: {error}",
                    gate_label::slot(index)
                ))
            })?;
        }
        let file = File::from(fd);
        verify_slot_file(&file, index, getuid().as_raw())?;
        Ok(Some(file))
    }

    /// Verify that this filesystem really implements `F_OFD_SETLK`.
    ///
    /// Probes through a canonical slot rather than creating a throwaway file.
    /// Contention proves the mechanism works; an uncontended probe is released
    /// immediately. The provisioned slot directory is intentionally immutable
    /// to the caller, so no probe leaf needs to be created or removed.
    pub fn probe_ofd_support(&self) -> Result<()> {
        let probe = self.open_slot(0, false)?.ok_or_else(|| {
            GateError::environment(format!(
                "{} is absent from the system-provisioned namespace",
                gate_label::slot(0)
            ))
        })?;
        self.evaluate_probe(&probe)
    }

    fn evaluate_probe(&self, probe: &File) -> Result<()> {
        match try_lock(probe) {
            Ok(()) => unlock(probe).map_err(|errno| self.probe_error(errno, "release")),
            // A contended probe still proves the mechanism works.
            Err(Errno::EAGAIN | Errno::EACCES) => Ok(()),
            Err(errno) => Err(self.probe_error(errno, "acquire")),
        }
    }

    fn probe_error(&self, errno: Errno, phase: &str) -> GateError {
        match classify_lock_errno(errno) {
            LockOutcome::Unsupported => self.unsupported_error(errno),
            _ => GateError::environment(format!(
                "cannot {phase} the lock on {}: {errno}",
                gate_label::slot(0)
            )),
        }
    }

    fn unsupported_error(&self, errno: Errno) -> GateError {
        GateError::unsupported(format!(
            "open file description locks (F_OFD_SETLK) are unavailable on the filesystem backing \
             {} ({errno}). The heavy gate fails closed rather than falling back to flock or \
             running unsynchronized; the system-provisioned heavy-gate root must live on a \
             filesystem that supports them.",
            gate_label::SLOT_DIR
        ))
    }

    /// One nonblocking attempt at `index`.
    ///
    /// Returns `Ok(None)` when the slot is held by another lane.
    pub fn try_acquire(&self, index: usize) -> Result<Option<SlotGuard>> {
        assert!(index < SLOT_COUNT, "slot index out of range");
        let file = self.open_slot(index, false)?.ok_or_else(|| {
            GateError::environment(format!(
                "{} is absent from the system-provisioned namespace",
                gate_label::slot(index)
            ))
        })?;

        match try_lock(&file) {
            Ok(()) => Ok(Some(SlotGuard { index, file })),
            Err(errno) => match classify_lock_errno(errno) {
                LockOutcome::Busy => Ok(None),
                LockOutcome::Unsupported => Err(self.unsupported_error(errno)),
                LockOutcome::Environment => Err(GateError::environment(format!(
                    "cannot lock {}: {errno}",
                    gate_label::slot(index)
                ))),
            },
        }
    }

    /// Prove whether `fd` is genuine evidence that this process runs inside a
    /// slot an ancestor wrapper already holds, claiming the slot atomically as
    /// part of the proof.
    ///
    /// Returns [`SlotProof::Held`] only when every one of the following holds,
    /// so a forged or stale [`GATE_ACTIVE_ENV`] marker can never skip
    /// acquisition:
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
    /// A verdict of [`SlotProof::NotHeld`] covers every *legitimate* "no slot
    /// is held" shape: a malformed marker, a closed (`EBADF`) or foreign
    /// descriptor, an absent slot file, a mismatched inode, or a slot a
    /// *different* description holds (`EAGAIN`/`EACCES`). A verifier
    /// *malfunction* - a slot file that exists but cannot be opened or
    /// stat'd, or a lock mechanism that is unsupported or errors for an
    /// environmental reason - is returned as an [`Err`] instead of being
    /// silently flattened into "unheld", so a caller fails closed on a broken
    /// environment rather than re-execing forever against it.
    ///
    /// The previous form issued `F_OFD_GETLK` on a *fresh* handle, which proved
    /// only that *some* description held the slot - not that the advertised
    /// descriptor did - so a forged unlocked descriptor passed whenever any
    /// unrelated lane happened to hold that slot, and the lock could be dropped
    /// between the query and its use. Claiming through `fd` removes both the
    /// impersonation and the TOCTOU window.
    pub fn descriptor_is_locked_slot(&self, index: usize, fd: RawFd) -> Result<SlotProof> {
        if index >= SLOT_COUNT || fd < 0 {
            return Ok(SlotProof::NotHeld);
        }
        let uid = getuid().as_raw();
        let inherited = match fstat(fd) {
            Ok(stat) => stat,
            // A closed or invalid inherited descriptor is a stale or forged
            // marker, not a gate malfunction: acquire a real slot instead.
            Err(Errno::EBADF) => return Ok(SlotProof::NotHeld),
            Err(errno) => {
                return Err(GateError::environment(format!(
                    "cannot stat the inherited heavy-gate slot descriptor: {errno}"
                )));
            }
        };
        if (inherited.st_mode & libc::S_IFMT) != libc::S_IFREG {
            return Ok(SlotProof::NotHeld);
        }
        if inherited.st_uid != uid {
            return Ok(SlotProof::NotHeld);
        }
        let Some(slot) = self.open_slot(index, true)? else {
            // A test scratch namespace may not have created this slot yet. In
            // production both slots were verified during `open_provisioned`, so
            // disappearance would be a root-level mutation rather than a
            // caller-controlled fallback.
            return Ok(SlotProof::NotHeld);
        };
        let slot_stat = fstat(slot.as_raw_fd()).map_err(|errno| {
            GateError::environment(format!("cannot stat {}: {errno}", gate_label::slot(index)))
        })?;
        if slot_stat.st_dev != inherited.st_dev || slot_stat.st_ino != inherited.st_ino {
            return Ok(SlotProof::NotHeld);
        }
        // Atomic ownership proof: claim the slot lock through the inherited
        // descriptor itself. Success means this description now holds the slot
        // (idempotently, if it already did); a Busy conflict means another
        // description owns it and nesting must be rejected. Anything else is a
        // verifier malfunction and fails closed. There is no window between
        // checking and using the lock because the check *is* the claim.
        match try_lock_fd(fd) {
            Ok(()) => Ok(SlotProof::Held),
            Err(errno) => match classify_lock_errno(errno) {
                LockOutcome::Busy => Ok(SlotProof::NotHeld),
                LockOutcome::Unsupported => Err(self.unsupported_error(errno)),
                LockOutcome::Environment => Err(GateError::environment(format!(
                    "cannot verify heavy-gate slot ownership through the inherited \
                     descriptor: {errno}"
                ))),
            },
        }
    }
}

/// Open `path` as a directory, refusing a symlinked final component.
///
/// `O_DIRECTORY` rejects a non-directory (`ENOTDIR`) and `O_NOFOLLOW` rejects a
/// symlink (`ELOOP`) so the returned descriptor is always a real directory,
/// and every later operation anchored to it stays bound to that inode.
fn open_directory(path: &Path, label: &str) -> Result<File> {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| match error {
        rustix::io::Errno::LOOP => GateError::environment(format!(
            "{label} is a symlink; refusing to use it as a heavy-gate directory"
        )),
        rustix::io::Errno::NOTDIR => GateError::environment(format!("{label} is not a directory")),
        _ => GateError::environment(format!("cannot open {label}: {error}")),
    })
}

/// Open a child directory directly relative to a pinned parent descriptor.
fn open_directory_at(parent: &File, name: &str, label: &str) -> Result<File> {
    rustix::fs::openat(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| match error {
        rustix::io::Errno::LOOP => GateError::environment(format!(
            "{label} is a symlink; refusing to use it as a heavy-gate directory"
        )),
        rustix::io::Errno::NOTDIR => GateError::environment(format!("{label} is not a directory")),
        _ => GateError::environment(format!("cannot open {label}: {error}")),
    })
}

/// Test-only fd-relative directory creation.
#[cfg(test)]
fn open_or_create_directory_at(parent: &File, name: &str, label: &str, mode: u32) -> Result<File> {
    if let Ok(dir) = open_directory_at(parent, name, label) {
        return Ok(dir);
    }
    match rustix::fs::mkdirat(parent.as_fd(), name, Mode::from_bits_truncate(mode)) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(error) => {
            return Err(GateError::environment(format!(
                "cannot create {label} relative to its pinned parent: {error}"
            )));
        }
    }
    let dir = open_directory_at(parent, name, label)?;
    rustix::fs::fchmod(dir.as_fd(), Mode::from_bits_truncate(mode)).map_err(|error| {
        GateError::environment(format!("cannot set mode {mode:o} on {label}: {error}"))
    })?;
    Ok(dir)
}

fn verify_system_directory(dir: &File, label: &str) -> Result<()> {
    let stat = fstat(dir.as_raw_fd())
        .map_err(|errno| GateError::environment(format!("cannot stat {label}: {errno}")))?;
    if !system_gate_parent_is_trusted(stat.st_uid, stat.st_mode as u32) {
        return Err(provisioning_error(
            "a protected heavy-gate directory has unsafe ownership or mode",
            format!(
                "{label} must be root-owned and non-writable by group or other; sticky \
                 world-writable and uid-owned directories are refused because their entries can \
                 still be renamed"
            ),
        ));
    }
    Ok(())
}

fn verify_slot_file(file: &File, index: usize, uid: u32) -> Result<()> {
    let stat = fstat(file.as_raw_fd()).map_err(|errno| {
        GateError::environment(format!("cannot stat {}: {errno}", gate_label::slot(index)))
    })?;
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(provisioning_error(
            "a provisioned slot has an unsafe file type",
            format!("{} is not a regular file", gate_label::slot(index)),
        ));
    }
    if stat.st_uid != uid {
        return Err(provisioning_error(
            "a provisioned slot has the wrong owner",
            format!(
                "{} is owned by a different user than the caller",
                gate_label::slot(index)
            ),
        ));
    }
    if stat.st_mode as u32 & 0o777 != 0o600 {
        return Err(provisioning_error(
            "a provisioned slot has the wrong mode",
            format!(
                "{} does not have the required provisioned mode 0600",
                gate_label::slot(index)
            ),
        ));
    }
    Ok(())
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
                    gate_label::SLOT_DIR
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
     If the protected namespace is unavailable, run `make\n\
     heavy-gate-provision`; the gate never falls back to a user-writable\n\
     namespace.\n\
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
///
/// A malfunction while proving the inherited descriptor (see
/// [`GateDir::descriptor_is_locked_slot`]) propagates as an [`Err`] so the
/// caller fails closed, rather than being flattened into an unverifiable
/// marker that would silently acquire a fresh slot against a broken
/// environment.
fn classify_nesting(dir: &GateDir) -> Result<NestingMarker> {
    if std::env::var_os(GATE_ACTIVE_ENV).is_none() {
        return Ok(NestingMarker::TopLevel);
    }
    match read_nesting_env() {
        Some((index, fd)) => match dir.descriptor_is_locked_slot(index, fd)? {
            SlotProof::Held => Ok(NestingMarker::VerifiedSlot),
            SlotProof::NotHeld => Ok(NestingMarker::Unverifiable),
        },
        None => Ok(NestingMarker::Unverifiable),
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
/// heavy work. A verifier *malfunction* - failing to resolve the gate
/// directory, or an unsupported/environment error while proving ownership -
/// propagates as an [`Err`] and exits with the corresponding
/// [`GateErrorKind`] code, never [`VERIFY_SLOT_UNHELD`], so a caller can tell a
/// genuine "not in a slot" apart from a broken environment and fail closed.
fn verify_slot() -> Result<u8> {
    let dir = GateDir::resolve()?;
    match classify_nesting(&dir)? {
        NestingMarker::VerifiedSlot => Ok(VERIFY_SLOT_HELD),
        _ => Ok(VERIFY_SLOT_UNHELD),
    }
}

fn execute(args: &[String]) -> Result<u8> {
    // The internal re-exec shim resolves and verifies its inherited slot; a
    // forged marker cannot authorize it into running unsynchronized.
    if args.first().is_some_and(|first| first == EXEC_SHIM_FLAG) {
        let dir = GateDir::resolve()?;
        return exec_wrapped_command(&args[1..], classify_nesting(&dir)?);
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
    match classify_nesting(&dir)? {
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
            crate::diagnostic_redaction::redact_path(Path::new(&request.program))
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
                crate::diagnostic_redaction::redact_path(Path::new(&request.program))
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
    use rustix::fs::AtFlags;
    use std::ffi::OsStr;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
    /// Env var selecting the in-test verify-slot child mode.
    const VERIFY_ROOT_ENV: &str = "D2B_HEAVY_GATE_TEST_VERIFY_ROOT";

    const HOLDER_TEST: &str = "heavy_gate::tests::slot_holder_child_mode";
    const WRAPPER_TEST: &str = "heavy_gate::tests::full_wrapper_child_mode";
    const SHIM_TEST: &str = "heavy_gate::tests::exec_shim_child_mode";
    const VERIFY_TEST: &str = "heavy_gate::tests::verify_slot_child_mode";

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
        GateDir::prepare_test(root, getuid().as_raw()).expect("gate directory is preparable")
    }

    /// Asserts a gate diagnostic names components by role only. Every message
    /// `run` prints to stderr and CI logs verbatim, so it must carry neither
    /// an absolute path (a supplied root or a resolved runtime path) nor the
    /// caller's numeric uid in any of the historical leak shapes. The redacted
    /// diagnostics use role labels and the shell-style `$UID` placeholder.
    fn assert_no_path_or_uid(message: &str, roots: &[&Path]) {
        for root in roots {
            let root = root.to_string_lossy();
            assert!(
                !message.contains(root.as_ref()),
                "a gate diagnostic must not leak {root}: {message}"
            );
        }
        assert!(
            !message.contains("/home") && !message.contains("/target/"),
            "a gate diagnostic must not leak HOME or a build path: {message}"
        );
        let uid = getuid().as_raw();
        for leak in [
            format!("uid {uid}"),
            format!("uid-{uid}"),
            format!("/run/user/{uid}"),
        ] {
            assert!(
                !message.contains(&leak),
                "a gate diagnostic must not leak the caller's uid as {leak:?}: {message}"
            );
        }
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
    fn gate_leaf_operations_remain_fd_relative_and_procfs_independent() {
        let source = include_str!("heavy_gate.rs");
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("the production portion precedes the test module");
        let proc_fd_leaf = ["/proc/self", "/fd/"].concat();
        let old_helper = ["anchored", "_path("].concat();

        assert!(
            !production.contains(&proc_fd_leaf),
            "production gate operations must not reconstruct paths through procfs"
        );
        assert!(
            !production.contains(&old_helper),
            "the pathname reconstruction helper must not return"
        );
        assert!(
            production.contains("rustix::fs::openat(")
                && production.contains("rustix::fs::mkdirat("),
            "slot opens and test-only fixture creation must stay fd-relative"
        );
    }

    #[test]
    fn gate_root_is_a_fixed_constant_independent_of_the_uid_and_runtime_dir() {
        // Production (no injected override) is always the single fixed root,
        // never a function of the uid and never a function of whether
        // /run/user/<uid> happens to exist. Two different uids resolve to the
        // same root; per-uid isolation comes from the uid-<uid> slot directory
        // beneath it, not from the root. This is what makes two lanes for the
        // same uid contend for one slot pool even if a login or logout races
        // their startup.
        assert_eq!(
            gate_root_from(1000, None),
            PathBuf::from("/run/d2b-heavy-gates")
        );
        assert_eq!(
            gate_root_from(1001, None),
            PathBuf::from("/run/d2b-heavy-gates")
        );
        // The injectable seam (tests only) wins when present, so the crate's
        // own tests can redirect the namespace to a scratch directory.
        assert_eq!(
            gate_root_from(1000, Some(Path::new("/scratch/gate"))),
            PathBuf::from("/scratch/gate")
        );
    }

    #[test]
    fn gate_directory_is_scoped_per_uid() {
        let first = gate_dir_path(Path::new("/run/d2b-heavy-gates"), 1000);
        let second = gate_dir_path(Path::new("/run/d2b-heavy-gates"), 1001);
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
    fn provisioning_diagnostic_is_stable_actionable_and_redacted() {
        let error = provisioning_error(
            "the protected heavy-gate root is unavailable",
            "the heavy-gate root directory cannot be opened",
        );
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(error.message().contains(
            "code: heavy-gate-provisioning-required; observed: the protected heavy-gate root is unavailable"
        ));
        assert!(
            error
                .message()
                .contains("remediation: run `make heavy-gate-provision`")
        );
        assert!(error.message().contains("no fallback namespace was used"));
        assert_no_path_or_uid(error.message(), &[]);
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
            dir.path().join("slot-0").is_file(),
            "the OFD probe uses the canonical slot instead of a removable probe leaf"
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
        std::os::unix::fs::symlink(&decoy, gate_dir_path(scratch.path(), getuid().as_raw()))
            .unwrap();

        let error = GateDir::prepare_test(scratch.path(), getuid().as_raw()).unwrap_err();
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(
            error.message().contains("symlink") || error.message().contains("not a directory"),
            "a symlinked slot directory is refused by fd-relative O_NOFOLLOW: {}",
            error.message()
        );
    }

    #[test]
    fn prepare_rejects_a_group_accessible_slot_directory() {
        let scratch = Scratch::new("loose-mode");
        let path = gate_dir_path(scratch.path(), getuid().as_raw());
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o770)).unwrap();

        let error = GateDir::prepare_test(scratch.path(), getuid().as_raw()).unwrap_err();
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(error.message().contains("private"));
    }

    #[test]
    fn prepare_rejects_a_group_writable_root() {
        // A root we own but that is group- or world-writable is refused: a peer
        // in that directory could rename the whole `d2b-heavy-gates` tree out
        // from under a later invocation, splitting the semaphore into a second
        // namespace. The test seam accepts only a private uid-owned root.
        let scratch = Scratch::new("loose-root");
        fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o777)).unwrap();

        let error = GateDir::prepare_test(scratch.path(), getuid().as_raw()).unwrap_err();
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
    fn refused_gate_directories_name_roles_never_paths_or_the_uid() {
        // Every GateDir preparation refusal is printed by `run` to stderr and CI
        // logs verbatim. Force the two representative refusals and assert each
        // names its component by role only - never the resolved runtime path
        // and never the caller's numeric uid. The leak escaped twice before
        // precisely because nothing asserted on this output.

        // A world- or group-writable root is untrusted (names the ROOT role).
        let root = Scratch::new("gate-redaction-root");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let error = GateDir::prepare_test(root.path(), getuid().as_raw())
            .expect_err("a world-writable root must be refused");
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert_no_path_or_uid(error.message(), &[root.path()]);
        let _ = fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700));

        // A pre-existing group-accessible per-uid slot directory is refused
        // (names the slot-directory role).
        let loose = Scratch::new("gate-redaction-slot");
        let slot_dir = gate_dir_path(loose.path(), getuid().as_raw());
        fs::create_dir_all(&slot_dir).unwrap();
        fs::set_permissions(&slot_dir, fs::Permissions::from_mode(0o770)).unwrap();
        let error = GateDir::prepare_test(loose.path(), getuid().as_raw())
            .expect_err("a group-accessible slot directory must be refused");
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert_no_path_or_uid(error.message(), &[loose.path(), &slot_dir]);
    }

    #[test]
    fn prepare_accepts_a_private_root() {
        // Test child modes need an unprivileged scratch namespace; production
        // never accepts this shape.
        let scratch = Scratch::new("private-root");
        let dir = GateDir::prepare_test(scratch.path(), getuid().as_raw())
            .expect("a 0700 owned test root is trusted");
        assert!(dir.path().ends_with(format!("uid-{}", getuid().as_raw())));
    }

    #[test]
    fn production_rejects_a_uid_owned_namespace_before_using_slots() {
        let scratch = Scratch::new("uid-owned-production-root");
        let uid = getuid().as_raw();
        let slot_dir = gate_dir_path(scratch.path(), uid);
        fs::create_dir_all(&slot_dir).unwrap();
        for index in 0..SLOT_COUNT {
            fs::write(slot_dir.join(format!("slot-{index}")), "").unwrap();
        }

        let error = GateDir::open_provisioned(scratch.path(), uid)
            .expect_err("a uid-owned namespace is renameable and must be refused");
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(
            error.message().contains("root-owned")
                && error.message().contains("sticky")
                && error.message().contains("renamed"),
            "the refusal clearly diagnoses the unsafe ownership: {}",
            error.message()
        );
        assert_no_path_or_uid(error.message(), &[scratch.path(), &slot_dir]);
    }

    #[test]
    fn missing_provisioned_root_fails_closed_without_a_fallback() {
        let scratch = Scratch::new("missing-production-root");
        let missing = scratch.path().join("absent");
        let error = GateDir::open_provisioned(&missing, getuid().as_raw())
            .expect_err("a missing provisioned root must fail closed");
        assert_eq!(error.kind(), GateErrorKind::Environment);
        assert!(error.message().contains("unavailable"));
        assert!(error.message().contains(PROVISIONING_ERROR_CODE));
        assert!(error.message().contains(PROVISIONING_ACTION));
        assert!(error.message().contains("observed:"));
        assert!(error.message().contains("no fallback"));
        assert!(
            !missing.exists(),
            "resolution never creates the missing root"
        );
        assert_no_path_or_uid(error.message(), &[scratch.path(), &missing]);
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

    #[test]
    fn a_spawn_failure_keeps_unambiguous_repository_context() {
        // A wrapped program that does not exist must fail closed while its
        // diagnostic keeps repository-relative context without exposing the
        // absolute checkout. `exec_shim` runs the exec in-process, so a missing
        // program returns the spawn diagnostic directly.
        let repo = crate::repo_root().unwrap();
        let program = repo.join(".scratch/redaction-sentinel-lane/does-not-exist");
        let request = Request {
            program: program.into_os_string(),
            args: Vec::new(),
        };
        let error = exec_shim(&request).expect_err("a missing program must fail to exec");
        let message = error.message();
        assert!(
            !message.contains(repo.to_str().unwrap()),
            "the spawn diagnostic leaked its absolute checkout: {message}"
        );
        assert!(
            message.contains("<repo>/.scratch/redaction-sentinel-lane/does-not-exist"),
            "the spawn diagnostic must preserve repository-relative context: {message}"
        );
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
    fn system_parent_trust_matrix_rejects_every_unprivileged_owner_and_sticky_root() {
        let us = 1000;
        let peer = 1001;
        // A uid-owned directory is renameable by that uid even at 0700. A peer
        // owner is equally unsafe. Production admits neither.
        assert!(!system_gate_parent_is_trusted(us, 0o700));
        assert!(!system_gate_parent_is_trusted(peer, 0o700));
        // Root ownership alone is insufficient: sticky 1777 still lets an
        // entry's owner rename that entry. This is why /tmp is not a safe root.
        assert!(!system_gate_parent_is_trusted(0, 0o1777));
        assert!(!system_gate_parent_is_trusted(0, 0o777));
        assert!(!system_gate_parent_is_trusted(0, 0o775));
        assert!(system_gate_parent_is_trusted(0, 0o755));
        assert!(system_gate_parent_is_trusted(0, 0o711));
    }

    #[test]
    fn canonical_namespace_parent_is_empirically_not_writable_by_this_uid() {
        if getuid().as_raw() == 0 {
            return;
        }
        let run = open_directory(Path::new("/run"), "the canonical namespace parent")
            .expect("/run is openable");
        verify_system_directory(&run, "the canonical namespace parent")
            .expect("/run is root-owned and non-writable");
        let probe = format!(".d2b-heavy-gate-squat-probe-{}", std::process::id());
        let outcome =
            rustix::fs::mkdirat(run.as_fd(), probe.as_str(), Mode::from_bits_truncate(0o700));
        if outcome.is_ok() {
            let _ = rustix::fs::unlinkat(run.as_fd(), probe.as_str(), AtFlags::REMOVEDIR);
        }
        assert!(
            matches!(
                outcome,
                Err(rustix::io::Errno::ACCESS | rustix::io::Errno::PERM | rustix::io::Errno::ROFS)
            ),
            "an unprivileged uid must not be able to squat a name under /run: {outcome:?}"
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
        // opened in `prepare_test`, acquisition must act on the moved inode, never
        // the decoy at the original path.
        let original = gate_dir_path(scratch.path(), uid);
        let moved = scratch.path().join("uid-moved");
        fs::rename(&original, &moved).unwrap();
        let root_dir = open_directory(scratch.path(), gate_label::ROOT).unwrap();
        let uid_name = format!("uid-{uid}");
        let decoy =
            open_or_create_directory_at(&root_dir, &uid_name, gate_label::SLOT_DIR, 0o700).unwrap();
        drop(decoy);

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
        assert_eq!(
            dir.descriptor_is_locked_slot(SLOT_COUNT, 0).unwrap(),
            SlotProof::NotHeld
        );
        // A negative descriptor is rejected.
        assert_eq!(
            dir.descriptor_is_locked_slot(0, -1).unwrap(),
            SlotProof::NotHeld
        );
        // A descriptor number that is not open (nothing was opened at it)
        // fails the `fstat` with EBADF, so a forged D2B_HEAVY_GATE_SLOT_FD is
        // a legitimate "not held", not a malfunction.
        assert_eq!(
            dir.descriptor_is_locked_slot(0, 4096).unwrap(),
            SlotProof::NotHeld
        );
    }

    #[test]
    fn a_closed_descriptor_marker_is_rejected() {
        let _serial = exclusive();
        let scratch = Scratch::new("closed-fd");
        let dir = gate_dir_under(scratch.path());

        // Open the real slot, capture its descriptor number, then close it.
        // A marker naming a now-closed descriptor must not count as a slot.
        let slot = dir.open_slot(0, false).unwrap().expect("slot opens");
        let fd = slot.as_raw_fd();
        drop(slot);
        assert_eq!(
            dir.descriptor_is_locked_slot(0, fd).unwrap(),
            SlotProof::NotHeld,
            "a closed descriptor fails fstat with EBADF and is not a held slot"
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
        let slot = dir.open_slot(0, false).unwrap().expect("slot opens");
        assert_eq!(
            dir.descriptor_is_locked_slot(0, slot.as_raw_fd()).unwrap(),
            SlotProof::Held,
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
        let separate = dir.open_slot(0, false).unwrap().expect("slot opens");
        let separate_stat = fstat(separate.as_raw_fd()).expect("fstat succeeds");
        let reopened = dir.open_slot(0, false).unwrap().expect("slot reopens");
        let slot_stat = fstat(reopened.as_raw_fd()).expect("fstat succeeds");
        assert_eq!(
            (separate_stat.st_dev, separate_stat.st_ino),
            (slot_stat.st_dev, slot_stat.st_ino),
            "the separate open really does name the canonical slot inode, so only the \
             lock-ownership check can reject it"
        );
        assert_eq!(
            dir.descriptor_is_locked_slot(0, separate.as_raw_fd())
                .unwrap(),
            SlotProof::NotHeld,
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
        assert_eq!(
            dir.descriptor_is_locked_slot(0, child_handle.as_raw_fd())
                .unwrap(),
            SlotProof::Held,
            "a live descriptor on the genuinely locked slot is accepted"
        );
        // A marker that points at the locked slot but names the *other* index
        // must not verify, because its inode will not match.
        assert_eq!(
            dir.descriptor_is_locked_slot(1, child_handle.as_raw_fd())
                .unwrap(),
            SlotProof::NotHeld,
            "the descriptor must match the slot index it claims"
        );
        drop(child_handle);
        drop(guard);
    }

    #[test]
    fn a_verifier_malfunction_propagates_as_an_error_not_unheld() {
        let _serial = exclusive();
        let scratch = Scratch::new("verifier-malfunction");
        let dir = gate_dir_under(scratch.path());

        // A live, uid-owned regular-file descriptor stands in for the inherited
        // slot marker so the check gets past the fstat/ownership screen and
        // reaches the canonical-slot open.
        let inherited = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(scratch.path().join("inherited-marker"))
            .expect("inherited marker opens");

        // Replace the canonical slot-0 path with a *directory*. Opening it
        // O_RDWR|O_NOFOLLOW then fails with EISDIR - not NotFound - which is a
        // genuine verifier malfunction, not a legitimate "no slot is held".
        fs::create_dir(dir.slot_path(0)).expect("slot path is replaceable with a directory");

        let outcome = dir.descriptor_is_locked_slot(0, inherited.as_raw_fd());
        assert!(
            outcome.is_err(),
            "a slot file that exists but cannot be opened as a regular file must fail closed \
             with an error, never collapse into SlotProof::NotHeld (exit 3)"
        );
        drop(inherited);
    }

    // ---- verify-slot through the CLI dispatch (the shell-guard path) ----
    //
    // These drive the real `heavy_gate::run` / `execute` CLI dispatch that the
    // shell and Make guards reach through `xtask heavy-gate verify-slot`, so
    // they prove the guard cannot be fooled by a bare `D2B_HEAVY_GATE` export.
    // They run it in a re-exec of *this* test binary rather than the separately
    // built `xtask` binary: the canonical per-uid namespace is non-overridable
    // in a released binary by design, so only the crate's own test build
    // honours the `XDG_RUNTIME_DIR` scratch-root seam. The exercised code path
    // (`execute` -> `verify_slot` -> `classify_nesting` ->
    // `descriptor_is_locked_slot`) is byte-for-byte the one the real binary
    // runs. A slot descriptor is handed to the child exactly as the gate does
    // it: an open handle with close-on-exec cleared so it survives `execve` at
    // the same fd number the environment advertises.

    /// Clear close-on-exec so a handle is inherited by the child at the same
    /// fd number, mirroring [`SlotGuard::duplicate_for_child`].
    fn clear_cloexec(file: &File) {
        fcntl(
            file.as_raw_fd(),
            FcntlArg::F_SETFD(FdFlag::from_bits_truncate(0)),
        )
        .expect("close-on-exec is clearable");
    }

    /// Child mode: run `verify-slot` through the real CLI dispatch against the
    /// parent's scratch namespace. Selected by [`VERIFY_ROOT_ENV`]; a no-op in
    /// ordinary runs. Exits with the exact code the `xtask heavy-gate
    /// verify-slot` binary would, mapping a gate error to its kind's exit code
    /// exactly as [`run`] does, so a verifier *malfunction* is distinguishable
    /// from a plain "unheld".
    #[test]
    fn verify_slot_child_mode() {
        if std::env::var_os(VERIFY_ROOT_ENV).is_none() {
            return;
        }
        let code = match execute(&[VERIFY_SLOT_OP.to_string()]) {
            Ok(code) => code,
            Err(error) => error.kind().exit_code(),
        };
        std::process::exit(i32::from(code));
    }

    /// Run `verify-slot` through a re-exec of this test binary with a
    /// controlled environment and return its exit code. `slot` is
    /// `Some((index, fd))` to advertise a slot marker, or `None` to export
    /// only the bare, forgeable `D2B_HEAVY_GATE`.
    fn run_verify_slot(root: &Path, marker: bool, slot: Option<(usize, RawFd)>) -> i32 {
        let mut command =
            Command::new(std::env::current_exe().expect("the test binary path is known"));
        command
            .args([VERIFY_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(VERIFY_ROOT_ENV, root)
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
            .expect("the verify-slot child runs")
            .code()
            .expect("verify-slot exits normally")
    }

    #[test]
    fn verify_slot_rejects_a_bare_marker_through_the_binary() {
        let _serial = exclusive();
        let scratch = Scratch::new("verify-bare");

        // Exactly the headline bypass: export the forgeable marker with no
        // real slot descriptor. verify-slot must report no held slot so the
        // shell guard acquires a real slot instead of running heavy work.
        let code = run_verify_slot(scratch.path(), true, None);
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a bare D2B_HEAVY_GATE export is not a held slot"
        );
    }

    #[test]
    fn verify_slot_rejects_a_forged_foreign_descriptor_through_the_binary() {
        let _serial = exclusive();
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

        let code = run_verify_slot(scratch.path(), true, Some((0, bogus.as_raw_fd())));
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a descriptor on a foreign file is not a held slot"
        );
        drop(bogus);
    }

    #[test]
    fn verify_slot_rejects_a_closed_descriptor_through_the_binary() {
        let _serial = exclusive();
        let scratch = Scratch::new("verify-closed");
        let dir = gate_dir_under(scratch.path());

        // Capture a real slot fd number, then close it. A marker naming a
        // now-closed descriptor fails fstat in the child.
        let slot = dir.open_slot(0, false).unwrap().expect("slot opens");
        let fd = slot.as_raw_fd();
        drop(slot);

        let code = run_verify_slot(scratch.path(), true, Some((0, fd)));
        assert_eq!(
            code, VERIFY_SLOT_UNHELD as i32,
            "a closed descriptor is not a held slot"
        );
    }

    #[test]
    fn verify_slot_rejects_a_separate_open_while_another_lane_holds_it() {
        let _serial = exclusive();
        let scratch = Scratch::new("verify-contended");
        let dir = gate_dir_under(scratch.path());

        // A genuine lane holds slot 0 through one open file description.
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");

        // The caller presents a *separate* open on the same slot inode that
        // does not hold the lock. The atomic ownership proof through that
        // descriptor conflicts with the guard's lock, so verify-slot rejects
        // it: a forged marker cannot smuggle in a third lane.
        let separate = dir.open_slot(0, false).unwrap().expect("slot opens");
        clear_cloexec(&separate);

        let code = run_verify_slot(scratch.path(), true, Some((0, separate.as_raw_fd())));
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
        let scratch = Scratch::new("verify-held");
        let dir = gate_dir_under(scratch.path());

        // Hold the slot the way the wrapper does, hand the child an inherited
        // duplicate on the *same* locked description. The atomic proof through
        // that descriptor is idempotent, so verify-slot confirms a held slot.
        let guard = dir.try_acquire(0).unwrap().expect("slot 0 is free");
        let child_handle = guard.duplicate_for_child().expect("duplicate succeeds");

        let code = run_verify_slot(scratch.path(), true, Some((0, child_handle.as_raw_fd())));
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

    /// True when a shell line is a `source`/`.` directive whose target basename
    /// is `name` (for example `. "$HERE/lib.sh"` or `source ./lib.sh`).
    fn line_sources_basename(line: &str, name: &str) -> bool {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix(". ")
            .or_else(|| trimmed.strip_prefix("source "))
        else {
            return false;
        };
        let Some(tok) = rest.split_whitespace().next() else {
            return false;
        };
        let tok = tok.trim_matches(|c| c == '"' || c == '\'');
        Path::new(tok).file_name().and_then(|n| n.to_str()) == Some(name)
    }

    /// Filesystem properties shared by the entrypoint collector and the
    /// sourced-library classifier. Keeping this one classifier prevents the
    /// two callers from drifting on what "regular", "shell", and "executable"
    /// mean.
    #[derive(Clone, Copy)]
    struct EntrypointProperties {
        is_shell: bool,
        is_executable: bool,
    }

    fn entrypoint_properties(path: &Path, meta: &fs::Metadata) -> Option<EntrypointProperties> {
        meta.file_type().is_file().then_some(EntrypointProperties {
            is_shell: path.extension() == Some(OsStr::new("sh")),
            is_executable: meta.permissions().mode() & 0o111 != 0,
        })
    }

    /// Whether `candidate` (a non-executable `.sh`) is a genuine shell
    /// *library*: an executable regular `.sh` entrypoint in the same directory
    /// pulls it in with a `source`/`.` directive. An inert text fixture, data
    /// file, directory, symlink, or non-executable shell file is not evidence
    /// that the candidate runs only behind a guarded entrypoint.
    ///
    /// Matching a same-directory entrypoint - the `. "$HERE/lib.sh"` shape
    /// every d2b lane uses - makes this a per-file classification rather than
    /// the old `basename == "lib.sh"` skip. Both this scan and
    /// [`heavy_entrypoint`] use [`entrypoint_properties`], so their filesystem
    /// definition cannot drift apart.
    fn is_sibling_sourced_library(candidate: &Path) -> bool {
        let Some(dir) = candidate.parent() else {
            return false;
        };
        let Some(name) = candidate.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        for entry in entries {
            let sibling = entry.expect("a readable dir entry").path();
            if sibling == candidate {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&sibling) else {
                continue;
            };
            let Some(properties) = entrypoint_properties(&sibling, &meta) else {
                continue;
            };
            if !properties.is_shell || !properties.is_executable {
                continue;
            }
            let Ok(text) = fs::read_to_string(&sibling) else {
                continue;
            };
            if text.lines().any(|line| line_sources_basename(line, name)) {
                return true;
            }
        }
        false
    }

    /// Classify a single regular file. Returns `Some(path)` when it is a heavy
    /// *entrypoint* (an executable regular file, or a non-executable `.sh` that
    /// no sibling sources), and `None` when it is inert data or a
    /// sibling-sourced shell library.
    ///
    /// This is the explicit per-file entrypoint-versus-library rule that
    /// replaces the old `basename == "lib.sh"` heuristic. Executability is the
    /// primary signal - an executable file is runnable as `./file` and is
    /// always an entrypoint, even if named `lib.sh` - and a non-executable
    /// `.sh` is a library only when a sibling actually sources it (otherwise it
    /// is still runnable as `bash file`, so it must be gated). A file that is
    /// neither `.sh` nor executable (a `.nix`, `.md`, `.txt`, ...) is inert
    /// data.
    fn heavy_entrypoint(path: &Path, meta: &fs::Metadata) -> Option<PathBuf> {
        let properties = entrypoint_properties(path, meta)?;
        if !properties.is_shell && !properties.is_executable {
            return None;
        }
        if !properties.is_executable && properties.is_shell && is_sibling_sourced_library(path) {
            return None;
        }
        Some(path.to_path_buf())
    }

    /// Markers of genuinely heavy work: build, container, VM, sudo, or device
    /// activity. A lane cannot be exempted from the gate (declared
    /// OUT_OF_SCOPE) while any file in it performs one of these; the exemption
    /// must be justified by this *checked property*, not by a free-text
    /// comment. This is what makes the census closed - an exemption the census
    /// cannot verify is refused - and it is exactly the gap that let the
    /// genuinely-heavy distro-matrix lane sit exempt behind a comment.
    const HEAVY_WORK_MARKERS: &[(&str, &str)] = &[
        ("cargo build", "build"),
        ("cargo test", "build"),
        ("cargo run", "build"),
        ("cargo clippy", "build"),
        ("nix build", "build"),
        ("nixos-rebuild", "build"),
        ("podman", "container"),
        ("docker", "container"),
        ("buildah", "container"),
        ("nerdctl", "container"),
        ("cloud-hypervisor", "VM"),
        ("qemu", "VM"),
        ("virtiofsd", "VM"),
        ("swtpm", "VM"),
        ("runNixOSTest", "VM"),
        ("d2b vm start", "VM"),
        ("sudo ", "sudo"),
        ("doas ", "sudo"),
        ("/dev/kvm", "device"),
        ("/dev/dri", "device"),
        ("/dev/vfio", "device"),
        ("/dev/nvidia", "device"),
        ("modprobe", "device"),
        ("usbip", "device"),
    ];

    /// Scan every regular file under `dir` for a [`HEAVY_WORK_MARKERS`] token,
    /// returning the first offending `(file, kind)`. Used to verify that a lane
    /// claimed OUT_OF_SCOPE really performs no heavy work, so the census refuses
    /// an exemption it cannot check.
    fn lane_heavy_work_marker(dir: &Path) -> Option<(PathBuf, &'static str)> {
        let mut stack = vec![dir.to_path_buf()];
        let mut hits: Vec<(PathBuf, &'static str)> = Vec::new();
        while let Some(current) = stack.pop() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("a readable dir entry").path();
                let meta = fs::symlink_metadata(&path)
                    .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));
                if meta.file_type().is_dir() {
                    stack.push(path);
                    continue;
                }
                if !meta.file_type().is_file() {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                if let Some((_, kind)) = HEAVY_WORK_MARKERS
                    .iter()
                    .find(|(marker, _)| text.contains(marker))
                {
                    hits.push((path, kind));
                }
            }
        }
        hits.sort();
        hits.into_iter().next()
    }

    /// Recursively collect every heavy-entrypoint candidate under `dir`. A
    /// sourced shell library is not an entrypoint - it is loaded by an
    /// entrypoint that already holds a slot - so it is excluded via
    /// [`heavy_entrypoint`]'s explicit classification and never required to
    /// self-guard. Returns an empty vec when `dir` is absent (optional lanes).
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
            if let Some(entrypoint) = heavy_entrypoint(&path, &meta) {
                out.push(entrypoint);
            }
        }
        out.sort();
        out
    }

    /// Inventory guard for the sole-use invariant: every live, hardware,
    /// benchmark, cloud, container, and performance entrypoint must route
    /// through the heavy-gate semaphore, so a future lane cannot be added that
    /// silently bypasses it. This is a CLOSED-WORLD guard - it walks the
    /// on-disk entrypoints recursively, censuses every lane directory against
    /// an explicit classification, and parses the Makefile for every heavy
    /// lane rather than checking a hand-maintained list. Adding a new heavy
    /// entrypoint (a nested or non-`.sh` script, a new aggregating runner, a
    /// new lane directory, a new `heavy-lane-*` make target, or a new public
    /// delegation) fails this test until it is gated.
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

        // Closed-world lane classification. Trusting a hand-maintained
        // directory list is exactly why the type-9 container lane went missing
        // for four rounds, so every lane directory on disk must be explicitly
        // classified. GATED lanes route through the heavy-gate and every
        // runnable entrypoint they contain must self-guard. OUT_OF_SCOPE lanes
        // must additionally prove they hold no slot by a *checked property* -
        // no file in them performs build, container, VM, sudo, or device work
        // (see HEAVY_WORK_MARKERS / lane_heavy_work_marker) - rather than by a
        // free-text comment, which is what let the genuinely-heavy distro-matrix
        // lane (sudo + /dev/kvm + cargo build --release --workspace) sit exempt.
        // GATED names may sit under different parents (benchmark lives directly
        // under tests/), so the walked set is spelled out and the census
        // cross-checks that every GATED directory on disk is actually walked.
        const GATED_LANE_DIRS: &[&str] = &[
            "live",
            "containers",
            "cloud",
            "hardware",
            "benchmark",
            "distro-matrix",
        ];
        // Every OUT_OF_SCOPE entry is verified against HEAVY_WORK_MARKERS below,
        // so an entry that performs heavy work is refused regardless of the
        // reason string. Empty today: the previously-exempt distro-matrix lane
        // is now GATED.
        const OUT_OF_SCOPE_LANE_DIRS: &[(&str, &str)] = &[];
        let walked_heavy_dirs = [
            "tests/integration/live",
            "tests/integration/containers",
            "tests/integration/cloud",
            "tests/host-integration/hardware",
            "tests/benchmark",
            "tests/integration/distro-matrix",
        ];

        // Census: every entry under the lane parents must be classified.
        // Subdirectories are classified as GATED (walked; every entrypoint must
        // self-guard) or OUT_OF_SCOPE (verified free of heavy work). Regular
        // files *directly* under a lane parent are classified too - an earlier
        // form only descended into subdirectories, so a heavy `.sh` dropped
        // straight into tests/integration/ escaped the census entirely. A loose
        // entrypoint is folded into the guarded set; loose data and
        // sibling-sourced libraries contribute nothing.
        let mut loose_entrypoints: Vec<PathBuf> = Vec::new();
        for parent_rel in ["tests/integration", "tests/host-integration"] {
            let parent = root.join(parent_rel);
            let Ok(entries) = fs::read_dir(&parent) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("a readable dir entry").path();
                let meta = fs::symlink_metadata(&path)
                    .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));
                if !meta.file_type().is_dir() {
                    // A file directly under the lane parent. Classify it with
                    // the same entrypoint-vs-library rule the lane walk uses; a
                    // heavy entrypoint here must still self-guard.
                    if let Some(entrypoint) = heavy_entrypoint(&path, &meta) {
                        loose_entrypoints.push(entrypoint);
                    }
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .expect("a utf-8 lane directory name")
                    .to_string();
                let gated = GATED_LANE_DIRS.contains(&name.as_str());
                let out_of_scope = OUT_OF_SCOPE_LANE_DIRS.iter().any(|(n, _)| *n == name);
                assert!(
                    gated ^ out_of_scope,
                    "lane directory {parent_rel}/{name} is not classified exactly once: classify \
                     it as GATED (add it to GATED_LANE_DIRS and to the walked heavy set so its \
                     entrypoints self-guard) or OUT_OF_SCOPE (add it to OUT_OF_SCOPE_LANE_DIRS \
                     and ensure it performs no build/container/VM/sudo/device work). A new heavy \
                     lane must not appear silently unguarded."
                );
                if gated {
                    let walked = walked_heavy_dirs.iter().any(|w| root.join(w) == path);
                    assert!(
                        walked,
                        "GATED lane directory {parent_rel}/{name} is classified but not in the \
                         walked heavy set; add it so its entrypoints are required to self-guard"
                    );
                }
                if out_of_scope {
                    // The exemption is only honoured if it is *checked*: a lane
                    // declared out of scope must actually perform no heavy work.
                    // A comment string is not enough - that is precisely how the
                    // heavy distro-matrix lane stayed exempt for a round.
                    if let Some((file, kind)) = lane_heavy_work_marker(&path) {
                        panic!(
                            "OUT_OF_SCOPE lane {parent_rel}/{name} is not actually out of scope: \
                             {} performs {kind} work. An exemption must be justified by a checked \
                             property, not a comment; gate the lane (move it to GATED_LANE_DIRS \
                             and the walked set) instead of exempting it.",
                            file.display()
                        );
                    }
                }
            }
        }

        // 1. Filesystem entrypoints. Walk every GATED heavy-lane directory
        //    recursively and require an executable self-guard on each script.
        //    performance-budgets.sh lives outside those directories, so it is
        //    named explicitly. The fixture-contract lane is eval-only and no
        //    longer belongs to the heavy inventory. Optional directories (benchmark, cloud) are walked
        //    when present and simply contribute nothing when absent.
        let mut entrypoints: Vec<PathBuf> = Vec::new();
        for dir in walked_heavy_dirs {
            entrypoints.extend(collect_heavy_entrypoints(&root.join(dir)));
        }
        // Loose entrypoints discovered directly under the lane parents (none
        // today) must be gated exactly like lane entrypoints.
        entrypoints.extend(loose_entrypoints);
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

        // The type-9 container lane must contribute at least its one runnable
        // entrypoint; its sourced lib.sh must NOT (it is not an entrypoint).
        let container_count = entrypoints
            .iter()
            .filter(|p| p.starts_with(root.join("tests/integration/containers")))
            .count();
        assert!(
            container_count >= 1,
            "expected the type-9 container lane entrypoints to be discovered and gated; found \
             {container_count}"
        );

        // The distro-matrix Tier-1 lane is genuinely heavy (sudo + /dev/kvm +
        // cargo build --release --workspace). It was exempt by comment for a
        // round; it must now be discovered and gated like any other lane.
        let distro_count = entrypoints
            .iter()
            .filter(|p| p.starts_with(root.join("tests/integration/distro-matrix")))
            .count();
        assert!(
            distro_count >= 1,
            "expected the distro-matrix Tier-1 entrypoint to be discovered and gated; found \
             {distro_count}"
        );
        assert!(
            !entrypoints
                .iter()
                .any(|p| p.file_name() == Some(OsStr::new("lib.sh"))),
            "a sourced lib.sh was collected as an entrypoint; sourced libraries must not be \
             required to hold a slot"
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

        // 2. The aggregating runners and the layer dispatcher (which drive the
        //    hardware, perf, and container lanes) must also route through the
        //    same verifying self-guard.
        for relative in [
            "tests/runner.sh",
            "tests/tools/run-layer.sh",
            "tests/test-integration.sh",
        ] {
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

    /// The OUT_OF_SCOPE escape hatch must be backed by a *checked property*, not
    /// a comment. Prove it has teeth: the census's heavy-work scanner would
    /// refuse to exempt the distro-matrix lane, because that lane genuinely
    /// performs sudo, device, and build work. If someone re-added distro-matrix
    /// to OUT_OF_SCOPE_LANE_DIRS, the census's out_of_scope branch would panic
    /// with exactly this detection.
    #[test]
    fn out_of_scope_exemption_is_refused_for_a_lane_that_does_heavy_work() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("packages/xtask resolves to a repo root");
        let distro = root.join("tests/integration/distro-matrix");
        let hit = lane_heavy_work_marker(&distro);
        assert!(
            hit.is_some(),
            "the distro-matrix lane must be detected as performing heavy work so it can never be \
             exempted by comment"
        );
        // A genuinely inert lane must scan clean, so the mechanism does not
        // reject every exemption outright - it rejects only lanes that actually
        // perform heavy work.
        let inert = Scratch::new("inert-lane");
        fs::write(inert.path().join("notes.md"), "just documentation\n").unwrap();
        fs::write(inert.path().join("data.txt"), "1 2 3\n").unwrap();
        assert!(
            lane_heavy_work_marker(inert.path()).is_none(),
            "an inert lane performs no heavy work and must scan clean"
        );
    }

    #[test]
    fn inert_source_text_cannot_hide_an_unguarded_heavy_script() {
        let scratch = Scratch::new("inert-source-decoy");
        let heavy = scratch.path().join("heavy-work.sh");
        fs::write(&heavy, "#!/usr/bin/env bash\ncargo build\n").unwrap();
        fs::set_permissions(&heavy, fs::Permissions::from_mode(0o644)).unwrap();

        // This is the bypass shape: inert sibling text claims to source the
        // non-executable heavy script. It is not itself a runnable shell
        // entrypoint, so its content cannot establish that the script runs only
        // behind a guarded caller.
        let decoy = scratch.path().join("notes.txt");
        fs::write(&decoy, "source heavy-work.sh\n").unwrap();
        fs::set_permissions(&decoy, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(
            !is_sibling_sourced_library(&heavy),
            "a non-executable text sibling is not evidence of a sourcing entrypoint"
        );
        assert_eq!(
            collect_heavy_entrypoints(scratch.path()),
            vec![heavy],
            "the census must retain the unguarded heavy script and demand its self-guard"
        );
    }

    /// The entrypoint-vs-library classification is per-file and content-based,
    /// not a `basename == "lib.sh"` skip. A sibling-sourced non-executable
    /// `lib.sh` is a library; an executable file is always an entrypoint even
    /// when named `lib.sh`; and a non-executable `.sh` that nobody sources is
    /// treated as an ungated entrypoint so it cannot bypass the census by name.
    #[test]
    fn entrypoint_and_library_are_classified_by_property_not_by_basename() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("packages/xtask resolves to a repo root");

        // The real container lib.sh: non-executable and sourced by a sibling.
        let container_lib = root.join("tests/integration/containers/lib.sh");
        let container_meta =
            fs::symlink_metadata(&container_lib).expect("the container lib.sh exists");
        assert!(
            heavy_entrypoint(&container_lib, &container_meta).is_none(),
            "a sibling-sourced, non-executable lib.sh must classify as a library, not an \
             entrypoint"
        );
        assert!(is_sibling_sourced_library(&container_lib));

        // The line-level source matcher recognises the `. \"$HERE/lib.sh\"`
        // shape and ignores unrelated lines and mere mentions.
        assert!(line_sources_basename("  . \"$HERE/lib.sh\"", "lib.sh"));
        assert!(line_sources_basename("source ./helpers.sh", "helpers.sh"));
        assert!(!line_sources_basename(
            "# mentions lib.sh in a comment",
            "lib.sh"
        ));
        assert!(!line_sources_basename("echo lib.sh", "lib.sh"));

        // A scratch tree lets us assert the executable-name and unsourced cases
        // without depending on fixtures that must not exist in the repo.
        let scratch = Scratch::new("classify-file");
        let exec_lib = scratch.path().join("lib.sh");
        fs::write(&exec_lib, "#!/usr/bin/env bash\n").unwrap();
        fs::set_permissions(&exec_lib, fs::Permissions::from_mode(0o755)).unwrap();
        let exec_meta = fs::symlink_metadata(&exec_lib).unwrap();
        assert!(
            heavy_entrypoint(&exec_lib, &exec_meta).is_some(),
            "an executable file named lib.sh is still an entrypoint and must be gated"
        );

        let orphan = scratch.path().join("orphan.sh");
        fs::write(&orphan, "#!/usr/bin/env bash\n").unwrap();
        fs::set_permissions(&orphan, fs::Permissions::from_mode(0o644)).unwrap();
        let orphan_meta = fs::symlink_metadata(&orphan).unwrap();
        assert!(
            heavy_entrypoint(&orphan, &orphan_meta).is_some(),
            "a non-executable .sh that no sibling sources is not a library; it must be treated as \
             an ungated entrypoint and flagged"
        );

        let data = scratch.path().join("fixture.txt");
        fs::write(&data, "inert\n").unwrap();
        let data_meta = fs::symlink_metadata(&data).unwrap();
        assert!(
            heavy_entrypoint(&data, &data_meta).is_none(),
            "an inert data file is neither an entrypoint nor a library"
        );
    }
}
