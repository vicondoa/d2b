//! Deterministic Relay reconnect backoff.

/// Backoff decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectDecision {
    /// Open immediately.
    OpenNow,
    /// Wait for a bounded delay.
    RetryAfter(u32),
    /// The session is terminally closed.
    Closed,
}

/// Bounded exponential backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectBackoff {
    attempt: u8,
    current_ms: u32,
    max_ms: u32,
    stable_reset_ms: u32,
    closed: bool,
}

impl ReconnectBackoff {
    /// Construct a backoff with bounded values.
    pub const fn new(max_ms: u32, stable_reset_ms: u32) -> Self {
        Self {
            attempt: 0,
            current_ms: 0,
            max_ms,
            stable_reset_ms,
            closed: false,
        }
    }

    /// Record a failed open.
    pub fn failed(&mut self) -> ReconnectDecision {
        if self.closed {
            return ReconnectDecision::Closed;
        }
        self.attempt = self.attempt.saturating_add(1);
        self.current_ms = if self.current_ms == 0 {
            1_000.min(self.max_ms)
        } else {
            self.current_ms.saturating_mul(2).min(self.max_ms)
        };
        ReconnectDecision::RetryAfter(self.current_ms)
    }

    /// Record a stable connection.
    pub fn stable(&mut self, elapsed_ms: u32) {
        if elapsed_ms >= self.stable_reset_ms {
            self.attempt = 0;
            self.current_ms = 0;
        }
    }

    /// Close permanently.
    pub const fn close(&mut self) {
        self.closed = true;
    }

    /// Return the failure count.
    pub const fn attempts(&self) -> u8 {
        self.attempt
    }
}
