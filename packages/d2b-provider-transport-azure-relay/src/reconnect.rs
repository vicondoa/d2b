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
    elapsed_ms: u32,
    max_ms: u32,
    stable_reset_ms: u32,
    max_attempts: u8,
    max_window_ms: u32,
    closed: bool,
}

impl ReconnectBackoff {
    /// Default maximum number of reconnect attempts.
    pub const DEFAULT_MAX_ATTEMPTS: u8 = 8;
    /// Default reconnect window.
    pub const DEFAULT_MAX_WINDOW_MS: u32 = 30_000;

    /// Construct a backoff with bounded values.
    pub const fn new(max_ms: u32, stable_reset_ms: u32) -> Self {
        Self::with_limits(
            max_ms,
            stable_reset_ms,
            Self::DEFAULT_MAX_ATTEMPTS,
            Self::DEFAULT_MAX_WINDOW_MS,
        )
    }

    /// Construct a backoff with explicit finite attempt and time bounds.
    pub const fn with_limits(
        max_ms: u32,
        stable_reset_ms: u32,
        max_attempts: u8,
        max_window_ms: u32,
    ) -> Self {
        Self {
            attempt: 0,
            current_ms: 0,
            elapsed_ms: 0,
            max_ms,
            stable_reset_ms,
            max_attempts,
            max_window_ms,
            closed: false,
        }
    }

    /// Record a failed open.
    pub fn failed(&mut self) -> ReconnectDecision {
        if self.closed {
            return ReconnectDecision::Closed;
        }
        if self.attempt >= self.max_attempts {
            self.closed = true;
            return ReconnectDecision::Closed;
        }
        let delay = if self.current_ms == 0 {
            1_000.min(self.max_ms)
        } else {
            self.current_ms.saturating_mul(2).min(self.max_ms)
        };
        if self.elapsed_ms.saturating_add(delay) > self.max_window_ms {
            self.closed = true;
            return ReconnectDecision::Closed;
        }
        self.attempt = self.attempt.saturating_add(1);
        self.current_ms = delay;
        self.elapsed_ms = self.elapsed_ms.saturating_add(delay);
        ReconnectDecision::RetryAfter(delay)
    }

    /// Record a stable connection.
    pub fn stable(&mut self, elapsed_ms: u32) {
        if elapsed_ms >= self.stable_reset_ms {
            self.attempt = 0;
            self.current_ms = 0;
            self.elapsed_ms = 0;
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

    /// Return the accumulated reconnect delay.
    pub const fn elapsed_ms(&self) -> u32 {
        self.elapsed_ms
    }
}
