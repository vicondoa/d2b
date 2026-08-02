//! Per-call metadata, retry policy, and cancellation.
//!
//! The metadata bounds here are carried verbatim from the ADR45 client so a v3
//! request cannot outlive the protocol ceiling or omit a required idempotency
//! key. They are declared locally because the v3 session contract module
//! (`d2b_contracts::v3::zone_session`) does not yet publish them; see the crate
//! documentation for the reconciliation obligation.

use core::{
    fmt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::ClientError;

/// The protocol ceiling on one request's lifetime.
pub const MAX_REQUEST_LIFETIME_MS: u64 = 15 * 60 * 1_000;

/// The exact request-identifier width.
pub const REQUEST_ID_BYTES: usize = 16;

/// The exact trace-identifier width.
pub const TRACE_ID_BYTES: usize = 16;

/// The inclusive correlation-identifier byte bound.
pub const MAX_CORRELATION_ID_BYTES: usize = 64;

/// The inclusive idempotency-key byte bound.
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 64;

/// The inclusive retry-attempt bound.
pub const MAX_RETRY_ATTEMPTS: u8 = 8;

/// A source of wall-clock time in Unix milliseconds.
pub trait WallClock: Send + Sync {
    /// The current Unix time in milliseconds.
    fn now_unix_ms(&self) -> u64;
}

/// The production wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl WallClock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

/// Validated per-request metadata.
///
/// The identifiers are held as opaque bytes and are never rendered by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct MetadataInput {
    request_id: [u8; REQUEST_ID_BYTES],
    correlation_id: Option<Vec<u8>>,
    trace_id: Option<[u8; TRACE_ID_BYTES]>,
    idempotency_key: Option<Vec<u8>>,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
}

impl fmt::Debug for MetadataInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetadataInput")
            .field("has_correlation", &self.correlation_id.is_some())
            .field("has_trace", &self.trace_id.is_some())
            .field("has_idempotency", &self.idempotency_key.is_some())
            .field("issued_at_unix_ms", &self.issued_at_unix_ms)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

impl MetadataInput {
    /// Build validated metadata for one request.
    pub fn new(
        request_id: [u8; REQUEST_ID_BYTES],
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, ClientError> {
        let input = Self {
            request_id,
            correlation_id: None,
            trace_id: None,
            idempotency_key: None,
            issued_at_unix_ms,
            expires_at_unix_ms,
        };
        input.validate_lifetime()?;
        Ok(input)
    }

    /// Attach a bounded ASCII correlation identifier.
    pub fn with_correlation(mut self, value: impl Into<String>) -> Result<Self, ClientError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_CORRELATION_ID_BYTES || !value.is_ascii() {
            return Err(ClientError::InvalidMetadata);
        }
        self.correlation_id = Some(value.into_bytes());
        Ok(self)
    }

    /// Attach a trace identifier.
    pub const fn with_trace(mut self, value: [u8; TRACE_ID_BYTES]) -> Self {
        self.trace_id = Some(value);
        self
    }

    /// Attach a bounded idempotency key.
    pub fn with_idempotency(mut self, value: Vec<u8>) -> Result<Self, ClientError> {
        if value.is_empty() || value.len() > MAX_IDEMPOTENCY_KEY_BYTES {
            return Err(ClientError::InvalidMetadata);
        }
        self.idempotency_key = Some(value);
        Ok(self)
    }

    /// Whether an idempotency key is present.
    pub const fn has_idempotency_key(&self) -> bool {
        self.idempotency_key.is_some()
    }

    /// The request identifier bytes.
    pub const fn request_id(&self) -> &[u8; REQUEST_ID_BYTES] {
        &self.request_id
    }

    /// The correlation identifier bytes, when present.
    pub fn correlation_id(&self) -> Option<&[u8]> {
        self.correlation_id.as_deref()
    }

    /// The trace identifier bytes, when present.
    pub const fn trace_id(&self) -> Option<&[u8; TRACE_ID_BYTES]> {
        self.trace_id.as_ref()
    }

    /// The idempotency key bytes, when present.
    pub fn idempotency_key(&self) -> Option<&[u8]> {
        self.idempotency_key.as_deref()
    }

    /// The issue time in Unix milliseconds.
    pub const fn issued_at_unix_ms(&self) -> u64 {
        self.issued_at_unix_ms
    }

    /// The expiry time in Unix milliseconds.
    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }

    /// Re-check the lifetime bound.
    pub const fn validate_lifetime(&self) -> Result<(), ClientError> {
        let Some(lifetime) = self.expires_at_unix_ms.checked_sub(self.issued_at_unix_ms) else {
            return Err(ClientError::InvalidMetadata);
        };
        if self.issued_at_unix_ms == 0 || lifetime == 0 || lifetime > MAX_REQUEST_LIFETIME_MS {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(())
    }
}

/// The bounded retry budget for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    max_attempts: u8,
}

impl RetryPolicy {
    /// Allow up to `max_attempts` total attempts.
    pub const fn new(max_attempts: u8) -> Result<Self, ClientError> {
        if max_attempts == 0 || max_attempts > MAX_RETRY_ATTEMPTS {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(Self { max_attempts })
    }

    /// Allow exactly one attempt.
    pub const fn no_retry() -> Self {
        Self { max_attempts: 1 }
    }

    /// The total attempt budget.
    pub const fn max_attempts(self) -> u8 {
        self.max_attempts
    }
}

/// The per-call options a caller supplies.
#[derive(Debug, Clone)]
pub struct CallOptions {
    /// Validated request metadata.
    pub metadata: MetadataInput,
    /// The retry budget.
    pub retry: RetryPolicy,
}

/// A cooperative cancellation signal shared by a caller and a call driver.
///
/// Cancellation is observed, never inferred: a driver checks the token before
/// each attempt and refuses with [`ClientError::Cancelled`] rather than racing
/// an in-flight attempt to a second answer.
#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    next_waiter: AtomicUsize,
    waiters: Mutex<Vec<(usize, Waker)>>,
}

/// A future that completes when its cancellation token is cancelled.
///
/// The future is allocation-free after the token has been created and does
/// not depend on the session runtime's executor primitives. That keeps the
/// cancellation signal usable by both the local Zone client and the session
/// adapter that eventually forwards the cancel record.
pub struct CancellationFuture {
    state: Arc<CancellationState>,
    registered: Option<usize>,
}

impl Future for CancellationFuture {
    type Output = ();

    fn poll(mut self: core::pin::Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.state.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        let state = Arc::clone(&self.state);
        let mut waiters = state.waiters.lock().unwrap();
        if state.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        if let Some(registered) = self.registered {
            if let Some((_, waiter)) = waiters.iter_mut().find(|(id, _)| *id == registered) {
                if !waiter.will_wake(context.waker()) {
                    *waiter = context.waker().clone();
                }
            } else {
                waiters.push((registered, context.waker().clone()));
            }
        } else {
            let registered = state.next_waiter.fetch_add(1, Ordering::Relaxed);
            self.registered = Some(registered);
            waiters.push((registered, context.waker().clone()));
        }
        Poll::Pending
    }
}

impl Drop for CancellationFuture {
    fn drop(&mut self) {
        let Some(registered) = self.registered.take() else {
            return;
        };
        let mut waiters = self.state.waiters.lock().unwrap();
        waiters.retain(|(id, _)| *id != registered);
    }
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    /// Request cancellation. Idempotent.
    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            let waiters = std::mem::take(&mut *self.state.waiters.lock().unwrap());
            for (_, waiter) in waiters {
                waiter.wake();
            }
        }
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// Wait until cancellation has been requested.
    pub fn cancelled(&self) -> CancellationFuture {
        CancellationFuture {
            state: Arc::clone(&self.state),
            registered: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const ISSUED: u64 = 1_000;

    #[test]
    fn metadata_lifetime_bounds_fail_closed() {
        assert!(MetadataInput::new([1; REQUEST_ID_BYTES], ISSUED, ISSUED + 1).is_ok());
        for (issued, expires) in [
            (0, 1),
            (ISSUED, ISSUED),
            (ISSUED + 1, ISSUED),
            (ISSUED, ISSUED + MAX_REQUEST_LIFETIME_MS + 1),
        ] {
            assert_eq!(
                MetadataInput::new([1; REQUEST_ID_BYTES], issued, expires).unwrap_err(),
                ClientError::InvalidMetadata
            );
        }
        assert!(
            MetadataInput::new(
                [1; REQUEST_ID_BYTES],
                ISSUED,
                ISSUED + MAX_REQUEST_LIFETIME_MS
            )
            .is_ok()
        );
    }

    #[test]
    fn optional_metadata_fields_are_bounded() {
        let base = MetadataInput::new([1; REQUEST_ID_BYTES], ISSUED, ISSUED + 1_000).unwrap();
        assert!(!base.has_idempotency_key());
        assert_eq!(base.request_id(), &[1; REQUEST_ID_BYTES]);
        assert_eq!(base.issued_at_unix_ms(), ISSUED);
        assert_eq!(base.expires_at_unix_ms(), ISSUED + 1_000);

        assert!(base.clone().with_correlation("").is_err());
        assert!(base.clone().with_correlation("caf\u{e9}").is_err());
        assert!(
            base.clone()
                .with_correlation("c".repeat(MAX_CORRELATION_ID_BYTES + 1))
                .is_err()
        );
        let full = base
            .clone()
            .with_correlation("c".repeat(MAX_CORRELATION_ID_BYTES))
            .unwrap()
            .with_trace([7; TRACE_ID_BYTES])
            .with_idempotency(vec![9; MAX_IDEMPOTENCY_KEY_BYTES])
            .unwrap();
        assert!(full.has_idempotency_key());
        assert_eq!(full.trace_id(), Some(&[7; TRACE_ID_BYTES]));
        assert_eq!(full.correlation_id().map(<[u8]>::len), Some(64));
        assert_eq!(full.idempotency_key().map(<[u8]>::len), Some(64));

        assert!(base.clone().with_idempotency(Vec::new()).is_err());
        assert!(
            base.with_idempotency(vec![0; MAX_IDEMPOTENCY_KEY_BYTES + 1])
                .is_err()
        );
    }

    #[test]
    fn retry_policy_bounds_are_exact() {
        assert_eq!(
            RetryPolicy::new(0).unwrap_err(),
            ClientError::InvalidMetadata
        );
        assert_eq!(
            RetryPolicy::new(MAX_RETRY_ATTEMPTS + 1).unwrap_err(),
            ClientError::InvalidMetadata
        );
        assert_eq!(RetryPolicy::new(1).unwrap().max_attempts(), 1);
        assert_eq!(
            RetryPolicy::new(MAX_RETRY_ATTEMPTS).unwrap().max_attempts(),
            MAX_RETRY_ATTEMPTS
        );
        assert_eq!(RetryPolicy::no_retry().max_attempts(), 1);
    }

    #[test]
    fn cancellation_is_shared_idempotent_and_never_renders_state_as_identity() {
        let token = CancellationToken::default();
        let observer = token.clone();
        assert!(!token.is_cancelled());
        token.cancel();
        token.cancel();
        assert!(observer.is_cancelled());
        assert_eq!(
            format!("{observer:?}"),
            "CancellationToken { cancelled: true }"
        );
    }

    #[test]
    fn metadata_debug_never_echoes_an_identifier() {
        let marker = format!("marker{:x}", std::process::id());
        let metadata = MetadataInput::new([0xAB; REQUEST_ID_BYTES], ISSUED, ISSUED + 1_000)
            .unwrap()
            .with_correlation(marker.clone())
            .unwrap()
            .with_trace([0xCD; TRACE_ID_BYTES])
            .with_idempotency(marker.clone().into_bytes())
            .unwrap();
        let rendered = format!("{metadata:?}");
        assert!(!rendered.contains(&marker), "{rendered}");
        assert!(!rendered.contains("171"), "{rendered}");
        assert!(rendered.contains("has_correlation: true"), "{rendered}");
    }

    #[test]
    fn the_system_clock_reports_a_plausible_unix_millisecond() {
        // 2020-01-01T00:00:00Z in Unix milliseconds.
        assert!(SystemClock.now_unix_ms() > 1_577_836_800_000);
    }
}
