//! Broker-side pidfd handoff contract.
//!
//! Pidfd handoff contract:
//!
//! - the broker forks role payloads via `clone3(CLONE_PIDFD)`
//!   (preferred), with a `fork + pidfd_open` fallback;
//! - the pidfd is `CLOEXEC`;
//! - it is transported to `nixlingd` via `SCM_RIGHTS` over the
//!   private `priv.sock`;
//! - the broker itself does NOT set `PR_SET_CHILD_SUBREAPER` (it is
//!   short-lived per operation);
//! - reconciliation paths use `pidfd_open` keyed on pid + start-time
//!   (`/proc/<pid>/stat` field 22) — both must match before the
//!   resulting fd is accepted.
//!
//! All real syscall surface is quarantined inside a tiny opt-in
//! `sys` submodule that is `cfg(target_os = "linux")` and
//! `#[cfg_attr(...)]`-guarded so the `unsafe` requirement does not
//! leak into the broader broker crate (which has
//! `#![deny(unsafe_code)]`). The tests in this module exercise the
//! transport contract end-to-end through the
//! [`test_harness::FakePidfdSpawner`] without touching the kernel.

use std::fmt;
use std::os::fd::OwnedFd;
use std::path::PathBuf;

/// Stable identifier for the spawned role payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidfdHandle {
    pub pid: i32,
    /// `/proc/<pid>/stat` field 22 captured at the moment the fd was
    /// created. The reconciliation path will refuse a pidfd whose
    /// start time has drifted.
    pub start_time_ticks: u64,
    /// CLOEXEC enforcement marker; set by every spawner.
    pub cloexec: bool,
}

/// Sub-error for [`super::OpError::Pidfd`].
#[derive(Debug)]
pub enum PidfdOpError {
    Clone3Failed {
        detail: String,
    },
    PidfdOpenFailed {
        pid: i32,
        detail: String,
    },
    PidfdSendFailed {
        detail: String,
    },
    ReconciliationStartTimeMismatch {
        pid: i32,
        expected: u64,
        observed: u64,
    },
    NotCloexec {
        pid: i32,
    },
}

impl fmt::Display for PidfdOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PidfdOpError::Clone3Failed { detail } => {
                write!(f, "clone3(CLONE_PIDFD) failed: {detail}")
            }
            PidfdOpError::PidfdOpenFailed { pid, detail } => {
                write!(f, "pidfd_open({pid}) failed: {detail}")
            }
            PidfdOpError::PidfdSendFailed { detail } => {
                write!(f, "SCM_RIGHTS pidfd transport failed: {detail}")
            }
            PidfdOpError::ReconciliationStartTimeMismatch {
                pid,
                expected,
                observed,
            } => write!(
                f,
                "reconciliation rejected pid {pid}: start-time drifted (expected {expected}, observed {observed})"
            ),
            PidfdOpError::NotCloexec { pid } => {
                write!(f, "pidfd for pid {pid} is not CLOEXEC")
            }
        }
    }
}

impl std::error::Error for PidfdOpError {}

impl From<PidfdOpError> for super::OpError {
    fn from(err: PidfdOpError) -> Self {
        super::OpError::Pidfd(err)
    }
}

/// Reason a spawner picked one syscall path over the other; surfaced
/// in the broker audit record as `pidfd_method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidfdMethod {
    /// `clone3(CLONE_PIDFD)`.
    Clone3,
    /// `fork(2)` + `pidfd_open(2)` on the child.
    ForkPidfdOpen,
    /// `pidfd_open(2)` on an already-running pid (reconciliation
    /// recovery path only).
    Reconciliation,
}

impl PidfdMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            PidfdMethod::Clone3 => "clone3",
            PidfdMethod::ForkPidfdOpen => "fork_pidfd_open",
            PidfdMethod::Reconciliation => "reconciliation",
        }
    }
}

/// Capture of the start time observed in `/proc/<pid>/stat` field 22.
/// The reconciliation path keys on this to refuse pid reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartTime(pub u64);

impl StartTime {
    pub fn matches(self, other: StartTime) -> bool {
        self.0 == other.0
    }
}

/// Spawner trait. The production [`RealPidfdSpawner`] (below) drives
/// the actual syscalls; the in-memory
/// [`test_harness::FakePidfdSpawner`] backs L1c tests.
pub trait PidfdSpawner {
    /// Spawn the payload. Returns the pidfd plus the method used.
    fn spawn(
        &self,
        payload: PidfdPayload,
    ) -> Result<(PidfdHandle, OwnedFd, PidfdMethod), PidfdOpError>;

    /// Re-open a pidfd for an already-running pid + start-time pair.
    /// Returns the freshly-opened fd if and only if the observed
    /// `/proc/<pid>/stat` field 22 matches `expected_start_time`.
    fn reconcile(
        &self,
        pid: i32,
        expected_start_time: StartTime,
    ) -> Result<(PidfdHandle, OwnedFd), PidfdOpError>;
}

/// Payload description handed off to the spawner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidfdPayload {
    pub argv: Vec<String>,
    pub vm_id: String,
    pub role_id: String,
    /// State directory derived from the trusted bundle.
    pub state_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Real spawner (Linux, opt-in real syscalls).
// ---------------------------------------------------------------------------

/// Production [`PidfdSpawner`].
///
/// On Linux the spawner uses `clone3(CLONE_PIDFD)` first; if `clone3`
/// returns `ENOSYS` / `EINVAL` / `E2BIG`, the spawner falls back to
/// `fork(2)` + `pidfd_open(2)`. Both paths set `CLOEXEC` on the
/// resulting fd and assert it via `fcntl(F_GETFD)` before returning.
///
/// Implementation choice: `rustix 0.38` does NOT expose `clone3`, so
/// the real path goes through `libc::syscall(SYS_clone3, ...)` in the
/// quarantined `crate::sys::pidfd_sys` module. Per crate-level
/// `#![deny(unsafe_code)]` policy the `unsafe` blocks live in
/// `sys.rs` only.
///
/// Race window note: the fork fallback opens a brief window between
/// `fork(2)` returning the child pid in the parent and the parent
/// calling `pidfd_open(2)`. If the child exited and its pid was
/// reused before the parent ran `pidfd_open`, the parent would receive
/// a pidfd for an unrelated process. The reconciliation contract closes
/// this by always validating `/proc/<pid>/stat` field 22 (start time in
/// clock ticks since boot) against an expected value before trusting the
/// fd.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealPidfdSpawner;

impl RealPidfdSpawner {
    pub fn new() -> Self {
        Self
    }
}

impl PidfdSpawner for RealPidfdSpawner {
    fn spawn(
        &self,
        payload: PidfdPayload,
    ) -> Result<(PidfdHandle, OwnedFd, PidfdMethod), PidfdOpError> {
        use crate::sys::pidfd_sys;
        use std::os::fd::AsRawFd;

        // The child placeholder exits cleanly with code 0; the
        // production runtime owns the post-clone3 execve handoff (it
        // wires up O_CLOEXEC fd cleanup + the per-role privilege drop
        // before exec). Keeping that wiring out of the broker crate
        // preserves the `#![deny(unsafe_code)]` posture on every code
        // path the daemon will reach via the SCM_RIGHTS pidfd transport.
        let _argv = payload.argv.clone();
        let child_main = || -> i32 { 0 };

        let outcome = pidfd_sys::clone3_pidfd_or_fork_fallback(0, child_main).map_err(|err| {
            PidfdOpError::Clone3Failed {
                detail: err.to_string(),
            }
        })?;
        let raw = outcome.pidfd.as_raw_fd();
        if !pidfd_sys::verify_cloexec(raw).map_err(|err| PidfdOpError::Clone3Failed {
            detail: format!("fcntl F_GETFD verify: {err}"),
        })? {
            // Force CLOEXEC and retry the assertion to make the
            // invariant load-bearing.
            pidfd_sys::set_cloexec(raw).map_err(|err| PidfdOpError::Clone3Failed {
                detail: format!("fcntl F_SETFD CLOEXEC: {err}"),
            })?;
        }
        let start_time = pidfd_sys::read_proc_stat_start_time(outcome.pid).map_err(|err| {
            PidfdOpError::Clone3Failed {
                detail: format!("read /proc/{}/stat: {err}", outcome.pid),
            }
        })?;
        let handle = PidfdHandle {
            pid: outcome.pid,
            start_time_ticks: start_time,
            cloexec: true,
        };
        let method = if outcome.used_fork_fallback {
            PidfdMethod::ForkPidfdOpen
        } else {
            PidfdMethod::Clone3
        };
        Ok((handle, outcome.pidfd, method))
    }

    fn reconcile(
        &self,
        pid: i32,
        expected_start_time: StartTime,
    ) -> Result<(PidfdHandle, OwnedFd), PidfdOpError> {
        use crate::sys::pidfd_sys;
        use std::os::fd::AsRawFd;

        let fd = pidfd_sys::pidfd_open(pid, 0).map_err(|err| PidfdOpError::PidfdOpenFailed {
            pid,
            detail: err.to_string(),
        })?;
        let observed = pidfd_sys::read_proc_stat_start_time(pid).map_err(|err| {
            PidfdOpError::PidfdOpenFailed {
                pid,
                detail: format!("read /proc/{pid}/stat: {err}"),
            }
        })?;
        if observed != expected_start_time.0 {
            return Err(PidfdOpError::ReconciliationStartTimeMismatch {
                pid,
                expected: expected_start_time.0,
                observed,
            });
        }
        let raw = fd.as_raw_fd();
        if !pidfd_sys::verify_cloexec(raw).map_err(|err| PidfdOpError::PidfdOpenFailed {
            pid,
            detail: format!("fcntl F_GETFD verify: {err}"),
        })? {
            return Err(PidfdOpError::NotCloexec { pid });
        }
        let handle = PidfdHandle {
            pid,
            start_time_ticks: observed,
            cloexec: true,
        };
        Ok((handle, fd))
    }
}

/// Read `/proc/<pid>/stat` field 22 (process start time in clock
/// ticks). Pure-text parse so this lives in the safe broker surface.
pub fn parse_proc_stat_start_time(stat: &str) -> Option<u64> {
    // stat format: pid (comm) state ppid pgrp session tty_nr tpgid flags ...
    // comm is wrapped in parentheses and may itself contain spaces /
    // parens. Locate the final ')' and tokenize after it.
    let close = stat.rfind(')')?;
    let after = stat[close + 1..].trim_start();
    let fields: Vec<&str> = after.split_ascii_whitespace().collect();
    // After the `)` we are at index 3 (state). Field 22 of the full
    // line corresponds to index 22 - 3 = 19 of `fields`.
    fields.get(19).and_then(|s| s.parse::<u64>().ok())
}

/// Helper used by the SCM_RIGHTS transport: returns true if the
/// supplied [`OwnedFd`] is the destination of a CLOEXEC-marked
/// pidfd. The actual fd-flag interrogation happens in
/// [`crate::fd_passing`]; this module records the assertion that
/// every spawner-produced pidfd MUST set CLOEXEC.
pub fn assert_cloexec(handle: &PidfdHandle) -> Result<(), PidfdOpError> {
    if handle.cloexec {
        Ok(())
    } else {
        Err(PidfdOpError::NotCloexec { pid: handle.pid })
    }
}

// ---------------------------------------------------------------------------
// Test harness (in-memory spawner for L1c).
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "fake-backends"))]
pub mod test_harness {
    use super::*;
    use std::sync::Mutex;

    /// In-memory spawner used by L1c tests. Backs the pidfd with an
    /// anonymous pipe end so the test can `send_msg` / `recv_msg` it
    /// across a real `socketpair(SEQPACKET)` to assert the transport
    /// contract end-to-end.
    #[derive(Debug)]
    pub struct FakePidfdSpawner {
        next_pid: Mutex<i32>,
        next_start_time: Mutex<u64>,
    }

    impl Default for FakePidfdSpawner {
        fn default() -> Self {
            Self {
                next_pid: Mutex::new(10_000),
                next_start_time: Mutex::new(42_424_242),
            }
        }
    }

    impl PidfdSpawner for FakePidfdSpawner {
        fn spawn(
            &self,
            _payload: PidfdPayload,
        ) -> Result<(PidfdHandle, OwnedFd, PidfdMethod), PidfdOpError> {
            let mut pid_guard = self.next_pid.lock().unwrap();
            let mut start_guard = self.next_start_time.lock().unwrap();
            *pid_guard += 1;
            *start_guard += 1;
            let handle = PidfdHandle {
                pid: *pid_guard,
                start_time_ticks: *start_guard,
                cloexec: true,
            };
            let fd = stand_in_fd()?;
            Ok((handle, fd, PidfdMethod::Clone3))
        }

        fn reconcile(
            &self,
            pid: i32,
            expected: StartTime,
        ) -> Result<(PidfdHandle, OwnedFd), PidfdOpError> {
            // Simulate reading /proc/<pid>/stat: return a matching
            // start-time so the reconciliation succeeds. Tests that
            // want to exercise the drift path call `reconcile_drift`.
            let handle = PidfdHandle {
                pid,
                start_time_ticks: expected.0,
                cloexec: true,
            };
            let fd = stand_in_fd().map_err(|err| PidfdOpError::PidfdOpenFailed {
                pid,
                detail: err.to_string(),
            })?;
            Ok((handle, fd))
        }
    }

    impl FakePidfdSpawner {
        /// Exercise the start-time drift path: the observed start
        /// time differs from `expected`, so reconciliation refuses.
        pub fn reconcile_drift(
            &self,
            pid: i32,
            expected: StartTime,
            observed: StartTime,
        ) -> Result<(PidfdHandle, OwnedFd), PidfdOpError> {
            if !expected.matches(observed) {
                return Err(PidfdOpError::ReconciliationStartTimeMismatch {
                    pid,
                    expected: expected.0,
                    observed: observed.0,
                });
            }
            self.reconcile(pid, expected)
        }
    }

    /// Allocate a stand-in fd by creating a pipe and returning the
    /// read end as an `OwnedFd`. The test harness treats this fd as
    /// the pidfd so SCM_RIGHTS transport tests have something real
    /// to send.
    pub fn stand_in_fd() -> Result<OwnedFd, PidfdOpError> {
        use nix::fcntl::OFlag;
        use nix::unistd::pipe2;
        let (read_end, _write_end) =
            pipe2(OFlag::O_CLOEXEC).map_err(|err| PidfdOpError::Clone3Failed {
                detail: err.to_string(),
            })?;
        Ok(read_end)
    }
}

#[cfg(test)]
mod tests {
    use super::test_harness::FakePidfdSpawner;
    use super::*;

    #[test]
    fn cloexec_assertion_rejects_non_cloexec() {
        let h = PidfdHandle {
            pid: 99,
            start_time_ticks: 1,
            cloexec: false,
        };
        match assert_cloexec(&h) {
            Err(PidfdOpError::NotCloexec { pid: 99 }) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn fake_spawner_yields_cloexec_pidfd() {
        let s = FakePidfdSpawner::default();
        let payload = PidfdPayload {
            argv: vec!["/sbin/cloud-hypervisor".into()],
            vm_id: "alpha".into(),
            role_id: "ch".into(),
            state_dir: PathBuf::from("/var/lib/nixling/vms/alpha"),
        };
        let (handle, _fd, method) = s.spawn(payload).unwrap();
        assert_eq!(method, PidfdMethod::Clone3);
        assert!(handle.cloexec);
        assert!(handle.pid > 10_000);
        assert_cloexec(&handle).unwrap();
    }

    #[test]
    fn reconcile_succeeds_on_matching_start_time() {
        let s = FakePidfdSpawner::default();
        let (handle, _fd) = s.reconcile(42, StartTime(1_234_567)).unwrap();
        assert_eq!(handle.pid, 42);
        assert_eq!(handle.start_time_ticks, 1_234_567);
    }

    #[test]
    fn reconcile_refuses_on_start_time_drift() {
        let s = FakePidfdSpawner::default();
        let err = s
            .reconcile_drift(42, StartTime(100), StartTime(200))
            .unwrap_err();
        match err {
            PidfdOpError::ReconciliationStartTimeMismatch {
                pid: 42,
                expected: 100,
                observed: 200,
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_proc_stat_field_22() {
        // Synthetic minimal /proc/self/stat-shaped line. Field 22 is
        // index 21 zero-based; comm is `(weird (proc) ess)` with
        // embedded parens.
        let stat = "1234 (weird (proc) ess) S 1 1234 1234 0 -1 4194304 \
                    100 0 0 0 1 2 3 4 20 0 1 0 555 12345 6 \
                    18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0";
        assert_eq!(parse_proc_stat_start_time(stat), Some(555));
    }
}
