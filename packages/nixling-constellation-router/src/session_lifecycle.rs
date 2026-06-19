//! The gateway-owned workload/display **session lifecycle** state machine
//! (ADR 0032). Pure, synchronous, transport-neutral: the gateway drives it
//! as a provider session is allocated, wired, run, and torn down, and uses
//! it to reconcile orphans after a restart.
//!
//! The P0 design panel (software, service-architect) required a concrete
//! lifecycle with explicit states, rollback on partial failure, and an
//! idempotent stop — so a half-allocated ACA session can never be leaked or
//! double-stopped. The phases are strictly ordered:
//!
//! ```text
//! Allocating → TokenMinting → RelayConnecting → DisplayOpening → Running
//!      └────────────── (failure at any phase) ──────────────┐
//!                                                            ▼
//!                                          Stopping → Stopped
//! ```
//!
//! A failure in any *active* phase routes through `Stopping` (so whatever was
//! already allocated — the ACA session, a minted token lease, a relay
//! connection, an open display — is cleaned up) and then `Stopped`. `stop()`
//! is idempotent: calling it on a session that is already `Stopping`/`Stopped`
//! is a no-op success, so a retry or a reconcile pass cannot wedge or
//! double-free.

use std::fmt;

/// The ordered phases of a workload/display session. `Failed` records the
/// phase the failure occurred in (for audit) and always transitions onward
/// to `Stopping`/`Stopped` so nothing is leaked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionPhase {
    /// Allocating the provider workload (e.g. an ACA sandbox).
    Allocating,
    /// Minting the short-lived per-session credential / token lease.
    TokenMinting,
    /// Establishing the relay (reachability) connection.
    RelayConnecting,
    /// Opening the authorized display stream.
    DisplayOpening,
    /// Fully established and serving.
    Running,
    /// Tearing down (releasing whatever was allocated).
    Stopping,
    /// Fully torn down (terminal).
    Stopped,
}

impl SessionPhase {
    /// Whether this phase still owns allocated resources that a teardown
    /// must release.
    fn is_active(self) -> bool {
        !matches!(self, SessionPhase::Stopping | SessionPhase::Stopped)
    }

    /// The next phase in the forward establishment sequence, if any.
    fn forward_next(self) -> Option<SessionPhase> {
        match self {
            SessionPhase::Allocating => Some(SessionPhase::TokenMinting),
            SessionPhase::TokenMinting => Some(SessionPhase::RelayConnecting),
            SessionPhase::RelayConnecting => Some(SessionPhase::DisplayOpening),
            SessionPhase::DisplayOpening => Some(SessionPhase::Running),
            _ => None,
        }
    }
}

/// Why a lifecycle transition was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleError {
    /// `advance()` was called from a phase with no forward step (it is
    /// already `Running`, or it is tearing down).
    NotAdvanceable,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LifecycleError::NotAdvanceable => {
                write!(f, "session phase cannot be advanced from its current state")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

/// The gateway-owned lifecycle of one workload/display session.
#[derive(Debug, Clone)]
pub struct SessionLifecycle {
    phase: SessionPhase,
    /// The phase a failure was observed in, if the session is tearing down
    /// because of one (for audit/reconciliation). `None` for an orderly stop.
    failed_in: Option<SessionPhase>,
}

impl Default for SessionLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionLifecycle {
    /// A fresh session at the first phase (`Allocating`).
    pub fn new() -> Self {
        Self {
            phase: SessionPhase::Allocating,
            failed_in: None,
        }
    }

    /// The current phase.
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Whether the session is fully torn down (terminal).
    pub fn is_stopped(&self) -> bool {
        self.phase == SessionPhase::Stopped
    }

    /// Whether the session is fully established and serving.
    pub fn is_running(&self) -> bool {
        self.phase == SessionPhase::Running
    }

    /// The phase a failure was observed in, if any (for audit).
    pub fn failed_in(&self) -> Option<SessionPhase> {
        self.failed_in
    }

    /// Advance one phase along the forward establishment sequence. Rejected
    /// (fail-closed) once `Running` or while tearing down.
    pub fn advance(&mut self) -> Result<SessionPhase, LifecycleError> {
        match self.phase.forward_next() {
            Some(next) => {
                self.phase = next;
                Ok(next)
            }
            None => Err(LifecycleError::NotAdvanceable),
        }
    }

    /// Record a failure in the current phase and roll the session into
    /// teardown. If the current phase still owns resources it moves to
    /// `Stopping` (cleanup); a failure observed while already tearing down
    /// is preserved without resurrecting the session. Idempotent.
    pub fn fail(&mut self) {
        if self.failed_in.is_none() {
            self.failed_in = Some(self.phase);
        }
        if self.phase.is_active() {
            self.phase = SessionPhase::Stopping;
        }
    }

    /// Begin an orderly teardown. Idempotent: from an active phase it moves
    /// to `Stopping`; from `Stopping`/`Stopped` it is a no-op success so a
    /// retry or reconcile pass can never double-free or wedge.
    pub fn stop(&mut self) {
        if self.phase.is_active() {
            self.phase = SessionPhase::Stopping;
        }
    }

    /// Confirm teardown is complete (resources released). Idempotent: only
    /// `Stopping`/`Stopped` may finish, so a `finish_stop()` that races a
    /// still-running session is a no-op (the caller must `stop()` first).
    pub fn finish_stop(&mut self) {
        if matches!(self.phase, SessionPhase::Stopping | SessionPhase::Stopped) {
            self.phase = SessionPhase::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_sequence_reaches_running_then_refuses_to_advance() {
        let mut s = SessionLifecycle::new();
        assert_eq!(s.phase(), SessionPhase::Allocating);
        assert_eq!(s.advance(), Ok(SessionPhase::TokenMinting));
        assert_eq!(s.advance(), Ok(SessionPhase::RelayConnecting));
        assert_eq!(s.advance(), Ok(SessionPhase::DisplayOpening));
        assert_eq!(s.advance(), Ok(SessionPhase::Running));
        assert!(s.is_running());
        // Running is terminal for forward progress.
        assert_eq!(s.advance(), Err(LifecycleError::NotAdvanceable));
    }

    #[test]
    fn failure_mid_establishment_rolls_into_teardown_and_records_phase() {
        let mut s = SessionLifecycle::new();
        s.advance().unwrap(); // TokenMinting
        s.advance().unwrap(); // RelayConnecting
        s.fail();
        assert_eq!(s.failed_in(), Some(SessionPhase::RelayConnecting));
        assert_eq!(s.phase(), SessionPhase::Stopping);
        s.finish_stop();
        assert!(s.is_stopped());
        // A second failure during teardown does not resurrect or relabel.
        s.fail();
        assert_eq!(s.failed_in(), Some(SessionPhase::RelayConnecting));
        assert!(s.is_stopped());
    }

    #[test]
    fn stop_is_idempotent() {
        let mut s = SessionLifecycle::new();
        s.advance().unwrap();
        s.stop();
        assert_eq!(s.phase(), SessionPhase::Stopping);
        // Repeated stop / finish are no-op successes.
        s.stop();
        s.finish_stop();
        assert!(s.is_stopped());
        s.stop();
        s.finish_stop();
        assert!(s.is_stopped());
        // No failure was recorded for an orderly stop.
        assert_eq!(s.failed_in(), None);
    }

    #[test]
    fn finish_stop_without_stopping_is_a_no_op() {
        let mut s = SessionLifecycle::new();
        s.advance().unwrap(); // TokenMinting, still active
        // Racing finish before stop must not terminalize a live session.
        s.finish_stop();
        assert_eq!(s.phase(), SessionPhase::TokenMinting);
        assert!(!s.is_stopped());
    }
}
