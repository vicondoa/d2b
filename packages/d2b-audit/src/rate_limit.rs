//! Audit durability classes and bounded write admission.

use std::time::{Duration, Instant};

/// Audit write class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditWriteClass {
    /// Must be durable before the operation returns.
    Privileged,
    /// Durable within a bounded window.
    Standard,
    /// Best-effort informational record.
    BestEffort,
}

impl AuditWriteClass {
    /// Stable class label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Privileged => "privileged",
            Self::Standard => "standard",
            Self::BestEffort => "best-effort",
        }
    }
}

/// Result of write-rate admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// The write may proceed.
    Allowed,
    /// A non-privileged write was rate limited.
    Limited,
}

/// Fixed-window limiter with privileged-never-dropped semantics.
#[derive(Debug)]
pub struct AuditRateLimiter {
    max_writes_per_second: u32,
    window_started: Instant,
    writes: u32,
}

impl AuditRateLimiter {
    /// Construct a limiter. A zero rate is rejected by clamping to one for
    /// non-privileged records; privileged records remain unlimited.
    pub fn new(max_writes_per_second: u32) -> Self {
        Self {
            max_writes_per_second: max_writes_per_second.max(1),
            window_started: Instant::now(),
            writes: 0,
        }
    }

    /// Admit one write.
    pub fn admit(&mut self, class: AuditWriteClass) -> RateDecision {
        if class == AuditWriteClass::Privileged {
            return RateDecision::Allowed;
        }
        if self.window_started.elapsed() >= Duration::from_secs(1) {
            self.window_started = Instant::now();
            self.writes = 0;
        }
        if self.writes >= self.max_writes_per_second {
            return RateDecision::Limited;
        }
        self.writes += 1;
        RateDecision::Allowed
    }

    /// Return the configured non-privileged limit.
    pub const fn max_writes_per_second(&self) -> u32 {
        self.max_writes_per_second
    }
}

/// Default rate from the legacy broker writer.
pub const DEFAULT_AUDIT_WRITES_PER_SECOND: u32 = 4096;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileged_records_are_never_limited() {
        let mut limiter = AuditRateLimiter::new(1);
        for _ in 0..128 {
            assert_eq!(
                limiter.admit(AuditWriteClass::Privileged),
                RateDecision::Allowed
            );
        }
    }

    #[test]
    fn standard_records_are_limited() {
        let mut limiter = AuditRateLimiter::new(1);
        assert_eq!(
            limiter.admit(AuditWriteClass::Standard),
            RateDecision::Allowed
        );
        assert_eq!(
            limiter.admit(AuditWriteClass::Standard),
            RateDecision::Limited
        );
    }
}
