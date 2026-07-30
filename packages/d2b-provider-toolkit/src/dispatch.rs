//! Bounded dispatch accounting for a Provider agent.
//!
//! A Provider agent serves at most [`MAX_DISPATCH_IN_FLIGHT`] concurrent
//! method dispatches. The ceiling is frozen rather than configured, so a
//! caller cannot make one agent hold unbounded work, and saturation is a
//! typed refusal rather than an unbounded queue.
//!
//! This is accounting only. It performs no dispatch, spawns nothing, and
//! knows no transport: a permit is proof that a slot was reserved, never
//! authority to call anything.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::ProviderToolkitError;

/// The frozen maximum number of concurrent in-flight dispatches.
pub const MAX_DISPATCH_IN_FLIGHT: usize = 64;

/// Bounded in-flight accounting for one Provider agent.
#[derive(Debug, Clone)]
pub struct DispatchLimiter {
    in_flight: Arc<AtomicUsize>,
    limit: usize,
}

impl DispatchLimiter {
    /// Build a limiter at the frozen ceiling.
    pub fn new() -> Self {
        Self::with_limit(MAX_DISPATCH_IN_FLIGHT).expect("the frozen ceiling is in range")
    }

    /// Build a limiter at an explicit limit.
    ///
    /// The limit is closed: zero and anything above
    /// [`MAX_DISPATCH_IN_FLIGHT`] are rejected, so no caller can widen the
    /// ceiling or disable admission.
    pub fn with_limit(limit: usize) -> Result<Self, ProviderToolkitError> {
        if limit == 0 || limit > MAX_DISPATCH_IN_FLIGHT {
            return Err(ProviderToolkitError::CapacityOutOfRange);
        }
        Ok(Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            limit,
        })
    }

    /// Reserve one dispatch slot, or refuse.
    pub fn acquire(&self) -> Result<DispatchPermit, ProviderToolkitError> {
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return Err(ProviderToolkitError::DispatchSaturated);
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(DispatchPermit {
                        in_flight: Arc::clone(&self.in_flight),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Return the frozen limit.
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Return the number of currently reserved slots.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }
}

impl Default for DispatchLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// One reserved dispatch slot, released when dropped.
///
/// The permit is not `Clone`: cloning it would release one slot twice and
/// let the agent exceed its frozen ceiling.
#[derive(Debug)]
pub struct DispatchPermit {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for DispatchPermit {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_closed_and_bounded() {
        assert_eq!(
            DispatchLimiter::with_limit(0).unwrap_err(),
            ProviderToolkitError::CapacityOutOfRange
        );
        assert_eq!(
            DispatchLimiter::with_limit(MAX_DISPATCH_IN_FLIGHT + 1).unwrap_err(),
            ProviderToolkitError::CapacityOutOfRange
        );
        assert_eq!(DispatchLimiter::new().limit(), MAX_DISPATCH_IN_FLIGHT);
    }

    #[test]
    fn saturation_refuses_and_a_released_permit_restores_the_slot() {
        let limiter = DispatchLimiter::with_limit(2).expect("valid limit");
        let first = limiter.acquire().expect("first slot");
        let second = limiter.acquire().expect("second slot");
        assert_eq!(limiter.in_flight(), 2);
        assert_eq!(
            limiter.acquire().unwrap_err(),
            ProviderToolkitError::DispatchSaturated
        );
        drop(second);
        assert_eq!(limiter.in_flight(), 1);
        let third = limiter.acquire().expect("reclaimed slot");
        assert_eq!(limiter.in_flight(), 2);
        drop(first);
        drop(third);
        assert_eq!(limiter.in_flight(), 0);
    }
}
