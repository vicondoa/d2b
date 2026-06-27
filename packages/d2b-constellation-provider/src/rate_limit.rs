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
    /// Maximum duration a half-open probe may stay in flight before reopening.
    pub probe_timeout: Duration,
    /// Maximum retained retry/backoff duration.
    pub max_open_for: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            default_open_for: Duration::from_secs(10),
            probe_timeout: Duration::from_secs(30),
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
    epoch: u64,
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

/// Permission returned by [`ProviderCircuitBreaker::before_request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitPermit {
    probe: bool,
    epoch: u64,
}

impl CircuitPermit {
    /// Whether this request is the single half-open probe.
    pub fn is_probe(self) -> bool {
        self.probe
    }

    /// State epoch this request was admitted under.
    pub fn epoch(self) -> u64 {
        self.epoch
    }
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
                epoch: 0,
                failures: 0,
                last_hint: None,
            }),
        }
    }

    /// Check whether a request may be issued at `now`.
    pub fn before_request(&self, now: Instant) -> Result<CircuitPermit, ProviderError> {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        match inner.state {
            CircuitState::Closed => Ok(CircuitPermit {
                probe: false,
                epoch: inner.epoch,
            }),
            CircuitState::ProbeInFlight { started }
                if now.saturating_duration_since(started) >= self.config.probe_timeout =>
            {
                let hint = retry_hint_for_failures(self.config, inner.failures.saturating_add(1));
                let failed_at = started + self.config.probe_timeout;
                let until = failed_at + hint.applied_backoff();
                if now >= until {
                    inner.failures = inner.failures.saturating_add(1);
                    inner.last_hint = Some(hint);
                    self.start_probe_locked(&mut inner, now, "probe-timeout-expired");
                    return Ok(CircuitPermit {
                        probe: true,
                        epoch: inner.epoch,
                    });
                }
                self.open_until_locked(&mut inner, until, hint, "probe-timeout");
                let remaining = until.saturating_duration_since(now);
                let error_hint =
                    RetryHint::bounded(remaining, Duration::ZERO, self.config.max_open_for);
                Err(ProviderError::rate_limited(
                    format!(
                        "provider circuit breaker probe timed out; retry after {} ms",
                        error_hint.applied_backoff().as_millis()
                    ),
                    error_hint,
                ))
            }
            CircuitState::ProbeInFlight { .. } => {
                let hint = RetryHint::bounded(
                    self.config.probe_timeout,
                    Duration::ZERO,
                    self.config.max_open_for,
                );
                Err(ProviderError::rate_limited(
                    format!(
                        "provider circuit breaker half-open probe already in flight; retry after {} ms",
                        hint.applied_backoff().as_millis()
                    ),
                    hint,
                ))
            }
            CircuitState::Open { until } if now >= until => {
                self.start_probe_locked(&mut inner, now, "open-window-elapsed");
                Ok(CircuitPermit {
                    probe: true,
                    epoch: inner.epoch,
                })
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
    pub fn record_success(&self, permit: CircuitPermit) {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        if permit.epoch != inner.epoch {
            return;
        }
        let previous = inner.state;
        if matches!(previous, CircuitState::Open { .. })
            || (matches!(previous, CircuitState::ProbeInFlight { .. }) && !permit.is_probe())
        {
            return;
        }
        if previous != CircuitState::Closed {
            inner.epoch = inner.epoch.saturating_add(1);
        }
        inner.state = CircuitState::Closed;
        inner.failures = 0;
        inner.last_hint = None;
        if previous != CircuitState::Closed {
            tracing::info!(
                event = "provider-circuit-transition",
                previous_state = state_label(previous),
                state = "closed",
                reason = "success",
                "provider circuit closed"
            );
        }
    }

    /// Record a rate-limit response and open the circuit immediately.
    pub fn record_rate_limited(&self, now: Instant, hint: RetryHint, permit: CircuitPermit) {
        {
            let mut inner = self.inner.write().expect("circuit lock poisoned");
            if permit.epoch != inner.epoch {
                if let CircuitState::Open { until } = &mut inner.state {
                    let requested_until = now + hint.applied_backoff();
                    if requested_until > *until {
                        *until = requested_until;
                        inner.last_hint = Some(hint);
                    }
                } else {
                    return;
                }
            } else {
                if matches!(inner.state, CircuitState::ProbeInFlight { .. }) && !permit.is_probe() {
                    return;
                }
                self.open_locked(&mut inner, now, hint, "rate-limited");
            }
        }
        tracing::info!(
            event = "provider-rate-limited",
            retry_after_ms = millis_u64(hint.retry_after()),
            applied_backoff_ms = millis_u64(hint.applied_backoff()),
            "provider rate limited"
        );
    }

    /// Record a transient provider failure. Opens after the configured
    /// threshold.
    pub fn record_transient_failure(&self, now: Instant, permit: CircuitPermit) {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        if permit.epoch != inner.epoch {
            return;
        }
        match inner.state {
            CircuitState::ProbeInFlight { .. } if permit.is_probe() => {
                let hint = retry_hint_for_failures(self.config, inner.failures.saturating_add(1));
                self.open_locked(&mut inner, now, hint, "probe-failure");
            }
            CircuitState::Closed => {
                let next_failures = inner.failures.saturating_add(1);
                if next_failures >= self.config.failure_threshold {
                    let hint = retry_hint_for_failures(self.config, next_failures);
                    self.open_locked(&mut inner, now, hint, "transient-failure");
                } else {
                    inner.failures = next_failures;
                }
            }
            CircuitState::Open { .. } | CircuitState::ProbeInFlight { .. } => {}
        }
    }

    /// Record cancellation of an in-flight half-open probe.
    pub fn record_cancellation(&self, now: Instant, permit: CircuitPermit) {
        let mut inner = self.inner.write().expect("circuit lock poisoned");
        if permit.is_probe()
            && permit.epoch == inner.epoch
            && matches!(inner.state, CircuitState::ProbeInFlight { .. })
        {
            let hint = retry_hint_for_failures(self.config, inner.failures.saturating_add(1));
            self.open_locked(&mut inner, now, hint, "probe-cancelled");
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

    fn start_probe_locked(&self, inner: &mut CircuitInner, now: Instant, reason: &'static str) {
        let previous = inner.state;
        inner.epoch = inner.epoch.saturating_add(1);
        inner.state = CircuitState::ProbeInFlight { started: now };
        tracing::info!(
            event = "provider-circuit-transition",
            previous_state = state_label(previous),
            state = "probe-in-flight",
            reason,
            "provider circuit probe started"
        );
    }

    fn open_locked(
        &self,
        inner: &mut CircuitInner,
        now: Instant,
        hint: RetryHint,
        reason: &'static str,
    ) {
        let previous = inner.state;
        let requested_until = now + hint.applied_backoff();
        debug_assert!(!matches!(previous, CircuitState::Open { .. }));
        self.open_until_locked(inner, requested_until, hint, reason);
    }

    fn open_until_locked(
        &self,
        inner: &mut CircuitInner,
        until: Instant,
        hint: RetryHint,
        reason: &'static str,
    ) {
        let previous = inner.state;
        debug_assert!(!matches!(previous, CircuitState::Open { .. }));
        inner.epoch = inner.epoch.saturating_add(1);
        inner.state = CircuitState::Open { until };
        inner.failures = inner.failures.saturating_add(1);
        inner.last_hint = Some(hint);
        tracing::info!(
            event = "provider-circuit-transition",
            previous_state = state_label(previous),
            state = "open",
            reason,
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

fn retry_hint_for_failures(config: CircuitBreakerConfig, failures: u32) -> RetryHint {
    let exponent = failures.saturating_sub(config.failure_threshold).min(8);
    let factor = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let base = config.default_open_for.saturating_mul(factor);
    let jitter = deterministic_jitter_ms(u64::from(failures));
    RetryHint::bounded(base, jitter, config.max_open_for)
}

fn deterministic_jitter_ms(seed: u64) -> Duration {
    Duration::from_millis(seed.wrapping_mul(97) % 501)
}

fn state_label(state: CircuitState) -> &'static str {
    match state {
        CircuitState::Closed => "closed",
        CircuitState::Open { .. } => "open",
        CircuitState::ProbeInFlight { .. } => "probe-in-flight",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_constellation_core::ErrorKind;

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
        breaker.record_rate_limited(
            now,
            hint,
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let err = breaker
            .before_request(now + Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert_eq!(
            err.retry_hint().unwrap().applied_backoff(),
            Duration::from_millis(1250)
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(3))
            .unwrap();
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(3)).state,
            "probe-in-flight"
        );
        breaker.record_success(permit);
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
        breaker.record_rate_limited(
            now,
            hint,
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());
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
            probe_timeout: Duration::from_secs(2),
            max_open_for: Duration::from_secs(10),
        });
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        assert!(breaker.before_request(now + Duration::from_secs(2)).is_ok());
        let err = breaker
            .before_request(now + Duration::from_secs(5))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert_eq!(breaker.snapshot(now + Duration::from_secs(5)).state, "open");
    }

    #[test]
    fn expired_probe_reopens_with_retry_hint_and_counts_failure() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            default_open_for: Duration::from_secs(2),
            probe_timeout: Duration::from_secs(2),
            max_open_for: Duration::from_secs(10),
        });
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());
        let err = breaker
            .before_request(now + Duration::from_secs(4))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        let snapshot = breaker.snapshot(now + Duration::from_secs(4));
        assert_eq!(snapshot.state, "open");
        assert_eq!(snapshot.failures, 2);
        assert!(snapshot.retry_hint.is_some());
    }

    #[test]
    fn late_probe_timeout_discovery_allows_next_probe_after_backoff_elapsed() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            default_open_for: Duration::from_secs(2),
            probe_timeout: Duration::from_secs(2),
            max_open_for: Duration::from_secs(10),
        });
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());
        let next = breaker
            .before_request(now + Duration::from_secs(20))
            .unwrap();
        assert!(next.is_probe());
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(20)).state,
            "probe-in-flight"
        );
    }

    #[test]
    fn reverse_time_during_probe_does_not_panic() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());
        let err = breaker
            .before_request(now + Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
    }

    #[test]
    fn probe_failure_reopens_immediately() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());
        breaker.record_transient_failure(now + Duration::from_secs(3), permit);
        assert_eq!(breaker.snapshot(now + Duration::from_secs(3)).state, "open");
    }

    #[test]
    fn transient_failures_open_after_threshold() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            default_open_for: Duration::from_secs(5),
            probe_timeout: Duration::from_secs(5),
            max_open_for: Duration::from_secs(20),
        });
        breaker.record_transient_failure(
            now,
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        assert!(breaker.before_request(now).is_ok());
        breaker.record_transient_failure(
            now,
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        assert!(breaker.before_request(now).is_err());
        assert_eq!(breaker.snapshot(now).failures, 2);
    }

    #[test]
    fn failure_while_open_does_not_increase_failures_or_shrink_deadline() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            default_open_for: Duration::from_secs(5),
            probe_timeout: Duration::from_secs(5),
            max_open_for: Duration::from_secs(20),
        });
        breaker.record_transient_failure(
            now,
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let before = breaker.snapshot(now);
        breaker.record_transient_failure(
            now + Duration::from_secs(1),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let after = breaker.snapshot(now + Duration::from_secs(1));
        assert_eq!(after.failures, before.failures);
        assert!(after.remaining.unwrap() >= Duration::from_secs(4));
    }

    #[test]
    fn concurrent_rate_limit_extends_open_deadline() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        let first_permit = breaker.before_request(now).unwrap();
        let second_permit = breaker.before_request(now).unwrap();
        breaker.record_rate_limited(
            now + Duration::from_secs(1),
            RetryHint::bounded(
                Duration::from_secs(2),
                Duration::ZERO,
                Duration::from_secs(20),
            ),
            first_permit,
        );
        breaker.record_rate_limited(
            now + Duration::from_secs(2),
            RetryHint::bounded(
                Duration::from_secs(10),
                Duration::ZERO,
                Duration::from_secs(20),
            ),
            second_permit,
        );
        assert!(
            breaker
                .snapshot(now + Duration::from_secs(2))
                .remaining
                .unwrap()
                >= Duration::from_secs(10)
        );
        assert_eq!(
            breaker
                .snapshot(now + Duration::from_secs(2))
                .retry_hint
                .unwrap()
                .retry_after(),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn late_success_while_open_does_not_close_circuit() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(5),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        breaker.record_success(CircuitPermit {
            probe: false,
            epoch: 0,
        });
        assert_eq!(breaker.snapshot(now).state, "open");
    }

    #[test]
    fn stale_epoch_success_and_failure_do_not_mutate_open_circuit() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        let first_permit = breaker.before_request(now).unwrap();
        let stale_permit = breaker.before_request(now).unwrap();
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(5),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            first_permit,
        );
        let before = breaker.snapshot(now);
        breaker.record_success(stale_permit);
        breaker.record_transient_failure(now + Duration::from_secs(1), stale_permit);
        let after = breaker.snapshot(now + Duration::from_secs(1));
        assert_eq!(after.state, "open");
        assert_eq!(after.failures, before.failures);
        assert_eq!(after.retry_hint, before.retry_hint);
    }

    #[test]
    fn cancellation_reopens_probe_in_flight() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());
        breaker.record_cancellation(now + Duration::from_secs(3), permit);
        assert_eq!(breaker.snapshot(now + Duration::from_secs(3)).state, "open");
    }

    #[test]
    fn stale_non_probe_observations_do_not_affect_probe() {
        let now = Instant::now();
        let breaker = ProviderCircuitBreaker::default();
        breaker.record_rate_limited(
            now,
            RetryHint::bounded(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(10),
            ),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        let permit = breaker
            .before_request(now + Duration::from_secs(2))
            .unwrap();
        assert!(permit.is_probe());

        breaker.record_success(CircuitPermit {
            probe: false,
            epoch: 0,
        });
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(2)).state,
            "probe-in-flight"
        );
        breaker.record_transient_failure(
            now + Duration::from_secs(2),
            CircuitPermit {
                probe: false,
                epoch: 0,
            },
        );
        assert_eq!(
            breaker.snapshot(now + Duration::from_secs(2)).state,
            "probe-in-flight"
        );
    }
}
