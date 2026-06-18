//! Non-mutating runner-liveness probe for the readiness wait path.
//!
//! The per-VM DAG executor's `LongLived` readiness wait (and the
//! split-readiness `probe_api_ready` path) used to loop on a readiness
//! socket until a deadline. When the spawned runner died before its
//! readiness socket ever appeared, that loop blocked all the way to the
//! readiness budget (default `max(api_timeout, 300) = 300s`) — the
//! observed `tpm.enable` first-run wedge.
//!
//! This module adds an **observe-only** liveness probe that the
//! readiness loop consults each iteration. It:
//!
//! - dups the daemon-held pidfd for the node's `(vm, role)` and polls it
//!   for `POLLIN` (the authoritative, reap-independent exit signal — the
//!   kernel marks a pidfd readable once the referenced process exits),
//! - PEEKs (does NOT consume) the `BrokerReapLog` matched by
//!   `runner_id`/`pid` for a buffered broker-reaped exit status,
//! - performs a READ-ONLY `/proc/<pid>/stat` start-time read to
//!   distinguish a still-our-process from PID reuse.
//!
//! It NEVER calls the mutating helpers (`wait_terminated`) and NEVER
//! deregisters. All deregistration (pidfd_table + snapshot + broker
//! registry) stays in the fail-fast rollback path.

use nixling_ipc::broker_wire::ChildExitStatus;

use crate::supervisor::pidfd_table::{BrokerReapLog, PidfdTable, read_proc_start_time_pub};

/// Classified liveness of a spawned long-lived runner, observed without
/// mutating any table or `/proc` reaping state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerLiveness {
    /// The registered process is still running (not yet exited).
    Alive,
    /// The registered process has terminated. Carries the bounded broker
    /// exit status when one was buffered, else `None`.
    Exited(Option<ChildExitStatus>),
    /// The registered PID now belongs to a different process
    /// (start-time drift) — our runner is gone and the PID was reused.
    Reused,
    /// Liveness could not be determined this iteration (entry already
    /// gone via rollback, unreadable `/proc`, dup failure). The caller
    /// keeps polling until the readiness deadline.
    Unknown,
}

/// Read-only `/proc/<pid>/stat` start-time observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartTimeObs {
    /// `/proc/<pid>/stat` start-time matches the registered value — this
    /// is still our process (possibly a zombie awaiting reap).
    Match,
    /// `/proc/<pid>/stat` start-time differs — the PID was reused.
    Drift,
    /// `/proc/<pid>/stat` is absent — the process is gone (reaped).
    Gone,
    /// `/proc/<pid>/stat` could not be read/parsed this iteration.
    Unreadable,
}

/// Pure inputs to [`classify`], so the classification decision table has
/// hermetic unit-test coverage independent of `/proc` and pidfds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessInputs {
    /// `false` when the `(vm, role)` entry is no longer registered
    /// (e.g. rollback already removed it).
    pub entry_present: bool,
    /// `Some(true)` when the dup'd pidfd polled `POLLIN` (exited);
    /// `Some(false)` when not readable; `None` when no pidfd was
    /// available to poll.
    pub pidfd_readable: Option<bool>,
    /// Read-only start-time observation.
    pub start_time: StartTimeObs,
    /// Buffered broker-reaped exit status (peeked, not consumed).
    pub reap_status: Option<ChildExitStatus>,
}

/// Pure classification of [`LivenessInputs`] into a [`RunnerLiveness`].
///
/// Priority:
/// 1. No entry → `Unknown` (rollback already owns it; don't fast-fail).
/// 2. start-time `Drift` → `Reused` (PID belongs to a different process).
/// 3. start-time `Gone` → `Exited` (process reaped/gone).
/// 4. pidfd `POLLIN` (readable) → `Exited` (terminated; possibly zombie).
/// 5. a buffered broker reap status → `Exited`.
/// 6. otherwise → `Alive` (start-time matches, pidfd not readable), or
///    `Unknown` when start-time was unreadable and nothing else fired.
pub fn classify(inputs: &LivenessInputs) -> RunnerLiveness {
    if !inputs.entry_present {
        return RunnerLiveness::Unknown;
    }
    match inputs.start_time {
        StartTimeObs::Drift => RunnerLiveness::Reused,
        StartTimeObs::Gone => RunnerLiveness::Exited(inputs.reap_status.clone()),
        StartTimeObs::Match => {
            if inputs.pidfd_readable == Some(true) || inputs.reap_status.is_some() {
                RunnerLiveness::Exited(inputs.reap_status.clone())
            } else {
                RunnerLiveness::Alive
            }
        }
        StartTimeObs::Unreadable => {
            if inputs.pidfd_readable == Some(true) || inputs.reap_status.is_some() {
                RunnerLiveness::Exited(inputs.reap_status.clone())
            } else {
                RunnerLiveness::Unknown
            }
        }
    }
}

/// Observe-only liveness probe. The readiness wait loop calls
/// [`LivenessProbe::probe`] each iteration; a fake implementation drives
/// the hermetic readiness-loop tests through the same call sites as
/// production.
pub trait LivenessProbe: Send + Sync {
    fn probe(&self) -> RunnerLiveness;
}

/// Production probe bound to one runner `(vm, role)`. Reads from the live
/// `PidfdTable` + `BrokerReapLog` without mutating either.
pub struct PidfdLivenessProbe<'a> {
    pidfd_table: &'a PidfdTable,
    reap_log: &'a BrokerReapLog,
    vm: String,
    role: String,
}

impl<'a> PidfdLivenessProbe<'a> {
    pub fn new(
        pidfd_table: &'a PidfdTable,
        reap_log: &'a BrokerReapLog,
        vm: impl Into<String>,
        role: impl Into<String>,
    ) -> Self {
        Self {
            pidfd_table,
            reap_log,
            vm: vm.into(),
            role: role.into(),
        }
    }

    fn gather(&self) -> LivenessInputs {
        // PEEK the buffered broker exit status (never consumes).
        let reap_status = self
            .reap_log
            .peek_for(&self.vm, &self.role)
            .map(|notif| notif.exit_status);

        // Dup the daemon-held pidfd for an authoritative, reap-independent
        // POLLIN poll. Absent entry => rollback already removed it.
        let Some((pidfd, pid, registered_start_time)) =
            self.pidfd_table.dup_pidfd_for(&self.vm, &self.role)
        else {
            return LivenessInputs {
                entry_present: false,
                pidfd_readable: None,
                start_time: StartTimeObs::Unreadable,
                reap_status,
            };
        };

        let pidfd_readable = poll_pollin(&pidfd);

        let start_time = match read_proc_start_time_pub(pid) {
            Ok(Some(observed)) if observed == registered_start_time => StartTimeObs::Match,
            Ok(Some(_)) => StartTimeObs::Drift,
            Ok(None) => StartTimeObs::Gone,
            Err(_) => StartTimeObs::Unreadable,
        };

        LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(pidfd_readable),
            start_time,
            reap_status,
        }
    }
}

impl LivenessProbe for PidfdLivenessProbe<'_> {
    fn probe(&self) -> RunnerLiveness {
        classify(&self.gather())
    }
}

/// Non-blocking `POLLIN` poll of a pidfd. Returns `true` when the fd is
/// readable (the referenced process has terminated). A poll error is
/// treated as not-readable (the start-time read disambiguates).
fn poll_pollin(fd: &impl std::os::fd::AsFd) -> bool {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    let mut fds = [PollFd::new(fd.as_fd(), PollFlags::POLLIN)];
    match poll(&mut fds, PollTimeout::ZERO) {
        Ok(n) if n > 0 => fds[0]
            .revents()
            .map(|revents| revents.contains(PollFlags::POLLIN))
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_ipc::broker_wire::ChildExitKind;

    fn status(code: i32) -> ChildExitStatus {
        ChildExitStatus {
            kind: ChildExitKind::Exited,
            code: Some(code),
            signal: None,
        }
    }

    #[test]
    fn no_entry_is_unknown() {
        let inputs = LivenessInputs {
            entry_present: false,
            pidfd_readable: Some(true),
            start_time: StartTimeObs::Gone,
            reap_status: Some(status(1)),
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Unknown);
    }

    #[test]
    fn start_time_drift_is_reused() {
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(false),
            start_time: StartTimeObs::Drift,
            reap_status: None,
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Reused);
    }

    #[test]
    fn gone_is_exited_with_reap_status() {
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(false),
            start_time: StartTimeObs::Gone,
            reap_status: Some(status(2)),
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Exited(Some(status(2))));
    }

    #[test]
    fn pollin_with_matching_start_time_is_exited() {
        // Zombie awaiting reap: /proc still shows our start-time but the
        // pidfd is readable.
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(true),
            start_time: StartTimeObs::Match,
            reap_status: None,
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Exited(None));
    }

    #[test]
    fn alive_when_match_and_not_readable() {
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(false),
            start_time: StartTimeObs::Match,
            reap_status: None,
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Alive);
    }

    #[test]
    fn reap_status_alone_marks_exited_even_if_proc_lags() {
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(false),
            start_time: StartTimeObs::Match,
            reap_status: Some(status(0)),
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Exited(Some(status(0))));
    }

    #[test]
    fn unreadable_proc_with_no_other_signal_is_unknown() {
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: None,
            start_time: StartTimeObs::Unreadable,
            reap_status: None,
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Unknown);
    }

    #[test]
    fn unreadable_proc_with_pollin_is_exited() {
        let inputs = LivenessInputs {
            entry_present: true,
            pidfd_readable: Some(true),
            start_time: StartTimeObs::Unreadable,
            reap_status: None,
        };
        assert_eq!(classify(&inputs), RunnerLiveness::Exited(None));
    }
}
