//! Generic v3 client and retry layer.
//!
//! The client carries only bounded metadata and delegates transport and
//! service resolution to injected implementations. It does not hold a store
//! handle or a Zone authority.

#![forbid(unsafe_code)]

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Maximum bytes in one metadata token.
pub const MAX_METADATA_TOKEN_BYTES: usize = 128;

/// Validated request metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataInput {
    trace: Option<d2b_telemetry::TraceContext>,
    correlation_id: String,
    idempotency_key: Option<String>,
    deadline: Option<Duration>,
}

impl MetadataInput {
    /// Construct bounded metadata.
    pub fn new(
        correlation_id: impl Into<String>,
        idempotency_key: Option<String>,
        deadline: Option<Duration>,
    ) -> Result<Self, ClientError> {
        let correlation_id = bounded_token(correlation_id.into())?;
        if let Some(key) = &idempotency_key {
            bounded_token(key.clone())?;
        }
        if deadline.is_some_and(|value| value.is_zero()) {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(Self {
            trace: None,
            correlation_id,
            idempotency_key,
            deadline,
        })
    }

    /// Attach a validated trace context.
    pub fn with_trace(mut self, trace: d2b_telemetry::TraceContext) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Borrow the trace context.
    pub const fn trace(&self) -> Option<&d2b_telemetry::TraceContext> {
        self.trace.as_ref()
    }

    /// Borrow the correlation identity.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    /// Borrow the optional idempotency key.
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// Return a bounded deadline.
    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }
}

/// Retry configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum attempts including the first request.
    pub max_attempts: u8,
    /// Initial retry delay.
    pub initial_delay: Duration,
    /// Maximum retry delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    /// Validate policy bounds.
    pub fn validate(self) -> Result<Self, ClientError> {
        if self.max_attempts == 0
            || self.initial_delay.is_zero()
            || self.max_delay < self.initial_delay
            || self.max_delay > Duration::from_secs(60)
        {
            return Err(ClientError::InvalidRetryPolicy);
        }
        Ok(self)
    }

    /// Return the bounded delay for a one-based retry number.
    pub fn delay_for(self, retry_number: u8) -> Duration {
        let shift = u32::from(retry_number.saturating_sub(1)).min(16);
        let factor = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
        self.initial_delay
            .checked_mul(factor)
            .unwrap_or(self.max_delay)
            .min(self.max_delay)
    }
}

/// Transport resolver abstraction.
pub trait Resolver: Send + Sync {
    /// Resolve a v3 service package to an opaque endpoint handle.
    fn resolve(&self, service: &str) -> Result<Box<dyn Connector>, ClientError>;
}

/// Transport connector abstraction.
pub trait Connector: Send + Sync {
    /// Send one request and return a transport response.
    fn call(
        &self,
        service: &str,
        method: &str,
        metadata: &MetadataInput,
    ) -> Result<Vec<u8>, ClientError>;
}

/// Clock abstraction for cancellation and retry tests.
pub trait Clock: Send + Sync {
    /// Current monotonic instant.
    fn now(&self) -> Instant;
}

/// Native monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Cancellation token.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    /// Construct a token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel all operations observing this token.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Generic v3 client.
#[derive(Debug)]
pub struct Client<R, C = MonotonicClock> {
    resolver: Arc<R>,
    clock: C,
    retry: RetryPolicy,
}

impl<R, C> Client<R, C>
where
    R: Resolver,
    C: Clock,
{
    /// Construct a client.
    pub fn new(resolver: Arc<R>, clock: C, retry: RetryPolicy) -> Result<Self, ClientError> {
        Ok(Self {
            resolver,
            clock,
            retry: retry.validate()?,
        })
    }

    /// Call a typed service package/method with bounded retry behavior.
    pub fn call(
        &self,
        service: &str,
        method: &str,
        metadata: &MetadataInput,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, ClientError> {
        if service.is_empty() || method.is_empty() {
            return Err(ClientError::InvalidRoute);
        }
        let connector = self.resolver.resolve(service)?;
        let started = self.clock.now();
        for attempt in 0..self.retry.max_attempts {
            if cancellation.is_cancelled() {
                return Err(ClientError::Cancelled);
            }
            if metadata
                .deadline()
                .is_some_and(|deadline| self.clock.now().duration_since(started) >= deadline)
            {
                return Err(ClientError::DeadlineExpired);
            }
            match connector.call(service, method, metadata) {
                Ok(response) => return Ok(response),
                Err(error) if error.retryable() && attempt + 1 < self.retry.max_attempts => {
                    let _delay = self.retry.delay_for(attempt + 1);
                }
                Err(error) => return Err(error),
            }
        }
        Err(ClientError::RetryExhausted)
    }
}

/// Daemon-local typed client facade.
pub type DaemonClient<R, C = MonotonicClock> = Client<R, C>;
/// Guest-side typed client facade.
pub type GuestClient<R, C = MonotonicClock> = Client<R, C>;

/// Client failures with stable, identity-free codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    /// Metadata was malformed.
    InvalidMetadata,
    /// Retry configuration was malformed.
    InvalidRetryPolicy,
    /// Service or method was not in the closed catalog.
    InvalidRoute,
    /// Connector could not be resolved.
    ServiceUnavailable,
    /// Request can be retried.
    TransportRetryable,
    /// Request failed permanently.
    TransportFailed,
    /// Request was cancelled.
    Cancelled,
    /// Request deadline elapsed.
    DeadlineExpired,
    /// All attempts were exhausted.
    RetryExhausted,
}

impl ClientError {
    /// Whether retrying may succeed.
    pub const fn retryable(self) -> bool {
        matches!(self, Self::TransportRetryable | Self::ServiceUnavailable)
    }
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidMetadata => "client-metadata-invalid",
            Self::InvalidRetryPolicy => "client-retry-policy-invalid",
            Self::InvalidRoute => "client-route-invalid",
            Self::ServiceUnavailable => "client-service-unavailable",
            Self::TransportRetryable => "client-transport-retryable",
            Self::TransportFailed => "client-transport-failed",
            Self::Cancelled => "client-cancelled",
            Self::DeadlineExpired => "client-deadline-expired",
            Self::RetryExhausted => "client-retry-exhausted",
        })
    }
}

impl std::error::Error for ClientError {}

fn bounded_token(value: String) -> Result<String, ClientError> {
    if value.is_empty()
        || value.len() > MAX_METADATA_TOKEN_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(ClientError::InvalidMetadata);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ResolverImpl;

    impl Resolver for ResolverImpl {
        fn resolve(&self, service: &str) -> Result<Box<dyn Connector>, ClientError> {
            if service == "d2b.resource.v3" {
                Ok(Box::new(ConnectorImpl))
            } else {
                Err(ClientError::ServiceUnavailable)
            }
        }
    }

    struct ConnectorImpl;

    impl Connector for ConnectorImpl {
        fn call(
            &self,
            _service: &str,
            _method: &str,
            _metadata: &MetadataInput,
        ) -> Result<Vec<u8>, ClientError> {
            Ok(b"ok".to_vec())
        }
    }

    #[test]
    fn trace_metadata_and_typed_route_are_bounded() {
        let metadata = MetadataInput::new("correlation", None, None)
            .unwrap()
            .with_trace(d2b_telemetry::TraceContext::new("trace", "span").unwrap());
        let client = Client::new(
            Arc::new(ResolverImpl),
            MonotonicClock,
            RetryPolicy::default(),
        )
        .unwrap();
        let result = client.call(
            "d2b.resource.v3",
            "Get",
            &metadata,
            &CancellationToken::new(),
        );
        assert_eq!(result.unwrap(), b"ok");
        assert!(MetadataInput::new("bad id", None, None).is_err());
    }
}
