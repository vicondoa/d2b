//! Shared provider rate-limit and circuit-breaker primitives.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::error::{ProviderError, RetryHint};

/// Circuit breaker configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerConfig {
    /// Consecutive transient failures before opening the circuit.
    pub failure_threshold: u32,
    /// Default open duration when a failure has no retry hint.
    pub default_open_for: Duration,
    /// Maximum retained retry/backoff duration.
    pub max_open_for: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            default_open_for: Duration::from_secs(10),
            max_open_for: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    Closed,
    Open { until: Instant },
    ProbeInFlight { started: Instant },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CircuitInner {
    state: CircuitState,
    failures: u32,
    last_hint: Option<RetryHint>,
}

/// Snapshot safe for diagnostics and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerSnapshot {
    /// Stable state label (`closed`, `open`, or `half-open`).
    pub state: &'static str,
    /// Consecutive failure count.
    pub failures: u32,
    /// Remaining open duration, if open.
    pub remaining: Option<Duration>,
    /// Last structured retry hint, if any.
    pub retry_hint: Option<RetryHint>,
}

/// Concurrency-safe provider circuit breaker. It stores only state labels,
/// counters, timestamps, and bounded retry hints.
#[derive(Debug)]
pub struct ProviderCircuitBreaker {
    config: CircuitBreakerConfig,
    inner: RwLock<CircuitInner>,
}

impl ProviderCircuitBreaker {
    /// Create a circuit breaker with `config`.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            inner: RwLock::new(CircuitInner {
                state: CircuitState::Closed,
                failures: 0,
                last_hint: None,
            }),
        }
    }

    /// Check whether a request may be issued at `now`.
    pub fn before_request(&self, now: Instant) -> Result<(), ProviderError> {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        match inner.state {
            CircuitState::Closed => Ok(()),
            CircuitState::ProbeInFlight { started }
                if now.duration_since(started) >= self.config.default_open_for =>
            {
                let hint = RetryHint::bounded(
                    self.config.default_open_for,
                    Duration::ZERO,
                    self.config.max_open_for,
                );
                inner.state = CircuitState::Open {
                    until: now + hint.applied_backoff(),
                };
                inner.last_hint = Some(hint);
                Err(ProviderError::rate_limited(
                    format!(
                        "provider circuit breaker probe timed out; retry after {} ms",
                        hint.applied_backoff().as_millis()
                    ),
                    hint,
                ))
            }
            CircuitState::ProbeInFlight { .. } => {
                let hint = RetryHint::bounded(
                    self.config.default_open_for,
                    Duration::ZERO,
                    self.config.max_open_for,
                );
                Err(ProviderError::rate_limited(
                    "provider circuit breaker half-open probe already in flight",
                    hint,
                ))
            }
            CircuitState::Open { until } if now >= until => {
                inner.state = CircuitState::ProbeInFlight { started: now };
                tracing::info!(
                    event = "provider-circuit-transition",
                    state = "half-open",
                    "provider circuit half-open probe started"
                );
                Ok(())
            }
            CircuitState::Open { until } => {
                let remaining = until.saturating_duration_since(now);
                let hint = RetryHint::bounded(remaining, Duration::ZERO, self.config.max_open_for);
                Err(ProviderError::rate_limited(
                    format!(
                        "provider circuit breaker open; retry after {} ms",
                        hint.applied_backoff().as_millis()
                    ),
                    hint,
                ))
            }
        }
    }

    /// Record a successful provider request.
    pub fn record_success(&self) {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        inner.state = CircuitState::Closed;
        inner.failures = 0;
        inner.last_hint = None;
        tracing::info!(
            event = "provider-circuit-transition",
            state = "closed",
            "provider circuit closed"
        );
    }

    /// Record a rate-limit response and open the circuit immediately.
    pub fn record_rate_limited(&self, now: Instant, hint: RetryHint) {
        self.open(now, hint);
        tracing::info!(
            event = "provider-rate-limited",
            retry_after_ms = millis_u64(hint.retry_after()),
            applied_backoff_ms = millis_u64(hint.applied_backoff()),
            "provider rate limited"
        );
    }

    /// Record a transient provider failure. Opens after the configured
    /// threshold.
    pub fn record_transient_failure(&self, now: Instant) {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        inner.failures = inner.failures.saturating_add(1);
        if inner.failures >= self.config.failure_threshold {
            let hint = RetryHint::bounded(
                self.config.default_open_for,
                Duration::ZERO,
                self.config.max_open_for,
            );
            inner.state = CircuitState::Open {
                until: now + hint.applied_backoff(),
            };
            inner.last_hint = Some(hint);
            tracing::info!(
                event = "provider-circuit-transition",
                state = "open",
                reason = "transient-failure",
                applied_backoff_ms = millis_u64(hint.applied_backoff()),
                "provider circuit opened"
            );
        }
    }

    /// Snapshot state at `now`.
    pub fn snapshot(&self, now: Instant) -> CircuitBreakerSnapshot {
        let inner = self.inner.read().expect("circuit lock poisoned");
        let (state, remaining) = match inner.state {
            CircuitState::Closed => ("closed", None),
            CircuitState::ProbeInFlight { .. } => ("probe-in-flight", None),
            CircuitState::Open { until } => ("open", Some(until.saturating_duration_since(now))),
        };
        CircuitBreakerSnapshot {
            state,
            failures: inner.failures,
            remaining,
            retry_hint: inner.last_hint,
        }
    }

    fn open(&self, now: Instant, hint: RetryHint) {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        inner.state = CircuitState::Open {
            until: now + hint.applied_backoff(),
        };
        inner.failures = inner.failures.saturating_add(1);
        inner.last_hint = Some(hint);
        tracing::info!(
            event = "provider-circuit-transition",
            state = "open",
            reason = "rate-limited",
            applied_backoff_ms = millis_u64(hint.applied_backoff()),
            "provider circuit opened"
        );
    }
}

impl Default for ProviderCircuitBreaker {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

fn _assert_send_sync<T: Send + Sync>() {}

fn millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::ErrorKind;

    #[test]
    fn circuit_breaker_is_send_sync() {
        _assert_send_sync::<ProviderCircuitBreaker>();
    }

    #[test]
    fn rate_limit_opens_and_half_opens_with_injected_time() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        let hint = RetryHint::bounded(
            Duration::from_secs(2),
            Duration::from_millis(250),
            Duration::from_secs(10),
        );
        breaker.record_rate_limited(now, hint);
        let err = breaker
            .before_request(now + Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert_eq!(
            err.retry_hint().unwrap().applied_backoff(),
            Duration::from_millis(1250)
        );
        assert!(breaker.before_request(now + Duration::from_secs(3)).is_ok());
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(3)).state,
            "probe-in-flight"
        );
        breaker.record_success();
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(3)).state,
            "closed"
        );
    }

    #[test]
    fn half_open_allows_only_one_probe() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        let hint = RetryHint::bounded(
            Duration::from_secs(1),
            Duration::ZERO,
            Duration::from_secs(5),
        );
        breaker.record_rate_limited(now, hint);
        assert!(breaker.before_request(now + Duration::from_secs(2)).is_ok());
        let err = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(2)).state,
            "probe-in-flight"
        );
    }

    #[test]
    fn stale_probe_reopens_circuit_instead_of_wedging() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            default_open_for: Duration::from_secs(2),
            max_open_for: Duration::from_secs(10),
        });
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
        );
        assert!(breaker.before_request(now + Duration::from_secs(2)).is_ok());
        let err = breaker
            .before_request(now + Duration::from_secs(5))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert_eq!(breaker.snapshot(now + Duration::from_secs(5)).state, "open");
    }

    #[test]
    fn transient_failures_open_after_threshold() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            default_open_for: Duration::from_secs(5),
            max_open_for: Duration::from_secs(20),
        });
        breaker.record_transient_failure(now);
        assert!(breaker.before_request(now).is_ok());
        breaker.record_transient_failure(now);
        assert!(breaker.before_request(now).is_err());
    }
}
