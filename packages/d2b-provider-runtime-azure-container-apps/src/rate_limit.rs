//! Provider-local circuit breaking for the ACA data-plane effect port.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::error::{ProviderError, RetryHint};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub default_open_for: Duration,
    pub probe_timeout: Duration,
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
enum State {
    Closed,
    Open { until: Instant },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inner {
    state: State,
    failures: u32,
    last_hint: Option<RetryHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerSnapshot {
    pub state: &'static str,
    pub failures: u32,
    pub remaining: Option<Duration>,
    pub retry_hint: Option<RetryHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitPermit {
    epoch: u64,
}

impl CircuitPermit {
    pub fn is_probe(self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct ProviderCircuitBreaker {
    config: CircuitBreakerConfig,
    inner: RwLock<(Inner, u64)>,
}

impl ProviderCircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            inner: RwLock::new((
                Inner {
                    state: State::Closed,
                    failures: 0,
                    last_hint: None,
                },
                0,
            )),
        }
    }

    pub fn before_request(&self, now: Instant) -> Result<CircuitPermit, ProviderError> {
        let mut guard = self.inner.write().expect("circuit lock poisoned");
        if let State::Open { until } = guard.0.state {
            if now < until {
                let remaining = until.saturating_duration_since(now);
                let hint = RetryHint::bounded(remaining, Duration::ZERO, self.config.max_open_for);
                return Err(ProviderError::rate_limited(
                    format!(
                        "provider circuit breaker open; retry after {} ms",
                        hint.applied_backoff().as_millis()
                    ),
                    hint,
                ));
            }
            guard.0.state = State::Closed;
            guard.0.failures = 0;
            guard.0.last_hint = None;
            guard.1 = guard.1.saturating_add(1);
        }
        Ok(CircuitPermit { epoch: guard.1 })
    }

    pub fn record_success(&self, permit: CircuitPermit) {
        let mut guard = self.inner.write().expect("circuit lock poisoned");
        if permit.epoch != guard.1 {
            return;
        }
        guard.0.state = State::Closed;
        guard.0.failures = 0;
        guard.0.last_hint = None;
    }

    pub fn record_rate_limited(&self, now: Instant, hint: RetryHint, permit: CircuitPermit) {
        let mut guard = self.inner.write().expect("circuit lock poisoned");
        if permit.epoch != guard.1 {
            return;
        }
        guard.0.state = State::Open {
            until: now + hint.applied_backoff(),
        };
        guard.0.failures = guard.0.failures.saturating_add(1);
        guard.0.last_hint = Some(hint);
    }

    pub fn record_transient_failure(&self, now: Instant, permit: CircuitPermit) {
        let mut guard = self.inner.write().expect("circuit lock poisoned");
        if permit.epoch != guard.1 {
            return;
        }
        guard.0.failures = guard.0.failures.saturating_add(1);
        if guard.0.failures >= self.config.failure_threshold {
            let hint = RetryHint::bounded(
                self.config.default_open_for,
                Duration::ZERO,
                self.config.max_open_for,
            );
            guard.0.state = State::Open {
                until: now + hint.applied_backoff(),
            };
            guard.0.last_hint = Some(hint);
        }
    }

    pub fn record_cancellation(&self, now: Instant, permit: CircuitPermit) {
        self.record_transient_failure(now, permit);
    }

    pub fn snapshot(&self, now: Instant) -> CircuitBreakerSnapshot {
        let guard = self.inner.read().expect("circuit lock poisoned");
        let (state, remaining) = match guard.0.state {
            State::Closed => ("closed", None),
            State::Open { until } => ("open", Some(until.saturating_duration_since(now))),
        };
        CircuitBreakerSnapshot {
            state,
            failures: guard.0.failures,
            remaining,
            retry_hint: guard.0.last_hint,
        }
    }
}

impl Default for ProviderCircuitBreaker {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}
