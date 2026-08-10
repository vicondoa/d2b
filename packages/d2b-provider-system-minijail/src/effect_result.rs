//! Broker-owned minijail effect results.

use d2b_process_conformance::{
    BrokerTerminalResult, LaunchedProcess, ProcessConformanceError, ProcessOutcome,
};

/// The opaque result returned by a minijail spawn effect.
pub struct MinijailEffectResult {
    /// Verified launch identity and local pidfd evidence.
    pub launched: LaunchedProcess,
    terminal: Option<BrokerTerminalResult>,
}

impl std::fmt::Debug for MinijailEffectResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MinijailEffectResult(<redacted>)")
    }
}

impl MinijailEffectResult {
    /// Build a running result.
    pub const fn running(launched: LaunchedProcess) -> Self {
        Self {
            launched,
            terminal: None,
        }
    }

    /// Attach the one-shot broker terminal result.
    pub fn with_terminal(mut self, terminal: BrokerTerminalResult) -> Self {
        self.terminal = Some(terminal);
        self
    }

    /// Consume the broker result and relay it to the matching ticket.
    pub fn terminal_outcome(
        self,
        ticket: &d2b_process_conformance::LaunchTicket,
    ) -> Result<Option<ProcessOutcome>, ProcessConformanceError> {
        self.terminal.map(|result| result.relay(ticket)).transpose()
    }
}
