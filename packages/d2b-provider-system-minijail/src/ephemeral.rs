//! Bounded EphemeralProcess lifecycle for system-minijail.

use d2b_process_conformance::{ProcessExitClass, ProcessOutcome};

/// One-shot process state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EphemeralProcessState {
    ttl_remaining: Option<u64>,
    terminal: Option<ProcessExitClass>,
    incident_hold: bool,
}

impl EphemeralProcessState {
    /// Construct a one-shot state.
    pub const fn new(incident_hold: bool) -> Self {
        Self {
            ttl_remaining: None,
            terminal: None,
            incident_hold,
        }
    }

    /// Consume a typed broker terminal result.
    pub fn observe(&mut self, outcome: ProcessOutcome, ttl: u64) {
        self.terminal = Some(outcome.exit_class);
        self.ttl_remaining = Some(ttl);
    }

    /// Advance the deterministic TTL clock.
    pub fn tick(&mut self, amount: u64) {
        if let Some(remaining) = &mut self.ttl_remaining {
            *remaining = remaining.saturating_sub(amount);
        }
    }

    /// Whether the core cleanup handler may delete the row.
    pub const fn cleanup_eligible(&self) -> bool {
        self.terminal.is_some() && !self.incident_hold && matches!(self.ttl_remaining, Some(0))
    }
}
