//! The attempt driver: deadline arithmetic, retry classification, and
//! cancellation forwarding.
//!
//! This module owns the call-policy state machine and nothing else. It performs
//! no I/O and holds no session, so the session driver landing in `d2b-bus` can
//! wrap it without this crate depending on the session contract. The retry
//! classification is carried over unchanged from the ADR45 client; only the
//! remote-error mapping is new, because v3 replaces the ADR45 remote retry
//! taxonomy with the canonical `ResourceError` retry class.

use core::time::Duration;
use std::{sync::Arc, time::Instant};

use d2b_contracts::v3::{ResourceError, RetryClass};

use crate::{
    CallOptions, CancellationToken, ClientError, MAX_REQUEST_LIFETIME_MS, ResolvedTarget,
    WallClock, ZoneServiceKind,
};

/// How one attempt ended, as observed by the session layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFailure {
    /// The attempt never reached the peer.
    BeforeDispatch,
    /// The attempt failed in a way the peer can safely see again.
    Retryable,
    /// The session dropped.
    Disconnected,
    /// It is unknown whether the peer applied the request.
    Ambiguous,
    /// The attempt exceeded its deadline.
    Deadline,
    /// The attempt was cancelled.
    Cancelled,
    /// The peer violated the wire contract.
    Protocol,
}

impl SessionFailure {
    /// The terminal client refusal this failure maps to.
    pub const fn terminal(self) -> ClientError {
        match self {
            Self::BeforeDispatch | Self::Retryable => ClientError::TransportFailed,
            Self::Ambiguous => ClientError::AmbiguousMutation,
            Self::Disconnected => ClientError::SessionLost,
            Self::Deadline => ClientError::DeadlineExpired,
            Self::Cancelled => ClientError::Cancelled,
            Self::Protocol => ClientError::ContractViolation,
        }
    }

    const fn is_retryable(self, mutating: bool, has_idempotency: bool) -> bool {
        match self {
            Self::BeforeDispatch | Self::Retryable | Self::Disconnected => {
                !mutating || has_idempotency
            }
            Self::Ambiguous => !mutating,
            Self::Deadline | Self::Cancelled | Self::Protocol => false,
        }
    }
}

/// The immutable call profile of one service method.
///
/// The ADR45 client read these from a generated `MethodSpec`. The v3 generated
/// service inventory does not exist yet, so the caller supplies the profile and
/// this crate enforces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodProfile {
    service: ZoneServiceKind,
    mutating: bool,
    requires_idempotency: bool,
    max_lifetime_ms: u32,
}

impl MethodProfile {
    /// Declare one method profile.
    ///
    /// A method that requires an idempotency key but is not mutating, and a
    /// lifetime outside `1..=MAX_REQUEST_LIFETIME_MS`, are both refused.
    pub const fn new(
        service: ZoneServiceKind,
        mutating: bool,
        requires_idempotency: bool,
        max_lifetime_ms: u32,
    ) -> Result<Self, ClientError> {
        if requires_idempotency && !mutating {
            return Err(ClientError::InvalidMethod);
        }
        if max_lifetime_ms == 0 || max_lifetime_ms as u64 > MAX_REQUEST_LIFETIME_MS {
            return Err(ClientError::InvalidMethod);
        }
        Ok(Self {
            service,
            mutating,
            requires_idempotency,
            max_lifetime_ms,
        })
    }

    /// The service this method belongs to.
    pub const fn service(self) -> ZoneServiceKind {
        self.service
    }

    /// Whether the method mutates peer state.
    pub const fn mutating(self) -> bool {
        self.mutating
    }

    /// Whether the method demands an idempotency key.
    pub const fn requires_idempotency(self) -> bool {
        self.requires_idempotency
    }

    /// The method's own lifetime ceiling in milliseconds.
    pub const fn max_lifetime_ms(self) -> u32 {
        self.max_lifetime_ms
    }
}

/// One admitted attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptTicket {
    attempt: u8,
    relative_timeout_nanos: u64,
}

impl AttemptTicket {
    /// The one-based attempt ordinal.
    pub const fn attempt(self) -> u8 {
        self.attempt
    }

    /// The remaining budget for this attempt, in nanoseconds.
    pub const fn relative_timeout_nanos(self) -> u64 {
        self.relative_timeout_nanos
    }
}

/// What the driver decided after one attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptDisposition {
    /// Begin another attempt immediately.
    RetryNow,
    /// Begin another attempt after at least this many milliseconds.
    ///
    /// The driver never sleeps; the caller owns the wait.
    RetryAfterMs(u32),
    /// Stop with this terminal refusal.
    Fail(ClientError),
}

/// The per-call retry, deadline, and cancellation state machine.
///
/// A driver is single-use and consumed by one call. It admits an attempt only
/// while the retry budget, the deadline, and the cancellation token all allow
/// one, so every stopping condition is an explicit typed refusal.
#[derive(Debug)]
pub struct CallDriver<W> {
    profile: MethodProfile,
    options: CallOptions,
    has_attachments: bool,
    attempt: u8,
    monotonic_deadline: Instant,
    clock: Arc<W>,
}

impl<W: WallClock> CallDriver<W> {
    /// Admit one call against a resolved target.
    ///
    /// The resolved service must match the method's service, a method that
    /// requires an idempotency key must have been given one, and the deadline
    /// must still be in the future against `clock`.
    pub fn new(
        target: &ResolvedTarget,
        profile: MethodProfile,
        options: CallOptions,
        has_attachments: bool,
        clock: Arc<W>,
    ) -> Result<Self, ClientError> {
        if target.service() != profile.service() {
            return Err(ClientError::InvalidMethod);
        }
        if profile.requires_idempotency() && !options.metadata.has_idempotency_key() {
            return Err(ClientError::IdempotencyRequired);
        }
        options.metadata.validate_lifetime()?;
        let remaining_ms = options
            .metadata
            .expires_at_unix_ms()
            .checked_sub(clock.now_unix_ms())
            .ok_or(ClientError::DeadlineExpired)?
            .min(u64::from(profile.max_lifetime_ms()));
        if remaining_ms == 0 {
            return Err(ClientError::DeadlineExpired);
        }
        let monotonic_deadline = Instant::now()
            .checked_add(Duration::from_millis(remaining_ms))
            .ok_or(ClientError::InvalidMetadata)?;
        Ok(Self {
            profile,
            options,
            has_attachments,
            attempt: 0,
            monotonic_deadline,
            clock,
        })
    }

    /// The method profile this driver enforces.
    pub const fn profile(&self) -> MethodProfile {
        self.profile
    }

    /// The number of attempts already admitted.
    pub const fn attempts_made(&self) -> u8 {
        self.attempt
    }

    /// Admit the next attempt, or refuse with a typed reason.
    pub fn begin_attempt(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<AttemptTicket, ClientError> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if self.attempt >= self.options.retry.max_attempts() {
            return Err(ClientError::RetryLimitExceeded);
        }
        let relative_timeout_nanos = self.relative_timeout()?;
        self.attempt = self.attempt.saturating_add(1);
        Ok(AttemptTicket {
            attempt: self.attempt,
            relative_timeout_nanos,
        })
    }

    /// Classify a session-level attempt failure.
    pub fn record_session_failure(&self, failure: SessionFailure) -> AttemptDisposition {
        let has_idempotency = self.options.metadata.has_idempotency_key();
        let mutating = self.profile.mutating();
        if failure.is_retryable(mutating, has_idempotency) {
            if !self.has_attachments && self.can_retry() {
                return AttemptDisposition::RetryNow;
            }
            if self.attempt >= self.options.retry.max_attempts() {
                return AttemptDisposition::Fail(ClientError::RetryLimitExceeded);
            }
        }
        if failure == SessionFailure::Ambiguous && mutating {
            return AttemptDisposition::Fail(ClientError::AmbiguousMutation);
        }
        AttemptDisposition::Fail(failure.terminal())
    }

    /// Classify a peer-reported refusal.
    ///
    /// The peer's retry class is an input, not a suggestion the client may
    /// widen: a `Never` or `Reauthorize` verdict is terminal here regardless of
    /// the remaining retry budget, and an authorization verdict is never
    /// retried into a second decision.
    pub fn record_remote_error(&self, error: &ResourceError) -> AttemptDisposition {
        let terminal = AttemptDisposition::Fail(ClientError::Remote {
            kind: error.kind(),
            retry: error.retry_class(),
        });
        if self.has_attachments || !self.can_retry() {
            return terminal;
        }
        match error.retry_class() {
            RetryClass::Immediate => AttemptDisposition::RetryNow,
            RetryClass::AfterDelay => match error.retry_after_ms() {
                Some(delay) => AttemptDisposition::RetryAfterMs(delay),
                None => terminal,
            },
            RetryClass::Never | RetryClass::Reauthorize => terminal,
        }
    }

    const fn can_retry(&self) -> bool {
        self.attempt < self.options.retry.max_attempts()
            && (!self.profile.mutating() || self.options.metadata.has_idempotency_key())
    }

    fn relative_timeout(&self) -> Result<u64, ClientError> {
        let wall_ms = self
            .options
            .metadata
            .expires_at_unix_ms()
            .checked_sub(self.clock.now_unix_ms())
            .ok_or(ClientError::DeadlineExpired)?;
        let monotonic = self
            .monotonic_deadline
            .saturating_duration_since(Instant::now());
        let relative = monotonic.min(Duration::from_millis(wall_ms));
        if relative.is_zero() {
            return Err(ClientError::DeadlineExpired);
        }
        Ok(relative.as_nanos().try_into().unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MetadataInput, REQUEST_ID_BYTES, RetryPolicy, RouteRecord, RouteTable, ServiceOwner,
        TargetInput, TargetResolver, TransportKind, TransportSelection, target::fixtures::zone,
    };
    use d2b_contracts::v3::{ResourceErrorKind, ResourceErrorReason};

    const ISSUED: u64 = 1_000;

    #[derive(Debug)]
    struct FixedClock(u64);

    impl WallClock for FixedClock {
        fn now_unix_ms(&self) -> u64 {
            self.0
        }
    }

    fn resolved(service: ZoneServiceKind) -> ResolvedTarget {
        let k1 = zone(&["k1", "k0"]);
        RouteTable::new(vec![RouteRecord::new(
            ServiceOwner::Zone(k1.clone()),
            TransportKind::ZoneLink,
        )])
        .resolve(
            &TargetInput::ZoneService(k1, service),
            service,
            TransportSelection::exact(TransportKind::ZoneLink),
        )
        .expect("route")
    }

    fn options(attempts: u8, idempotent: bool) -> CallOptions {
        let mut metadata =
            MetadataInput::new([1; REQUEST_ID_BYTES], ISSUED, ISSUED + 60_000).unwrap();
        if idempotent {
            metadata = metadata.with_idempotency(vec![1, 2, 3]).unwrap();
        }
        CallOptions {
            metadata,
            retry: RetryPolicy::new(attempts).unwrap(),
        }
    }

    fn driver(
        profile: MethodProfile,
        attempts: u8,
        idempotent: bool,
        has_attachments: bool,
    ) -> CallDriver<FixedClock> {
        CallDriver::new(
            &resolved(profile.service()),
            profile,
            options(attempts, idempotent),
            has_attachments,
            Arc::new(FixedClock(ISSUED)),
        )
        .expect("driver")
    }

    fn read_profile() -> MethodProfile {
        MethodProfile::new(ZoneServiceKind::Resource, false, false, 30_000).unwrap()
    }

    fn write_profile() -> MethodProfile {
        MethodProfile::new(ZoneServiceKind::Resource, true, true, 30_000).unwrap()
    }

    fn remote(kind: ResourceErrorKind, retry: RetryClass, after: Option<u32>) -> ResourceError {
        ResourceError::new(
            kind,
            None,
            after,
            retry,
            ResourceErrorReason::parse("refused").unwrap(),
        )
        .expect("resource error")
    }

    #[test]
    fn method_profiles_fail_closed_on_contradictory_declarations() {
        assert_eq!(
            MethodProfile::new(ZoneServiceKind::Resource, false, true, 1_000).unwrap_err(),
            ClientError::InvalidMethod
        );
        assert_eq!(
            MethodProfile::new(ZoneServiceKind::Resource, true, true, 0).unwrap_err(),
            ClientError::InvalidMethod
        );
        assert_eq!(
            MethodProfile::new(
                ZoneServiceKind::Resource,
                true,
                true,
                MAX_REQUEST_LIFETIME_MS as u32 + 1
            )
            .unwrap_err(),
            ClientError::InvalidMethod
        );
        let profile = write_profile();
        assert!(profile.mutating());
        assert!(profile.requires_idempotency());
        assert_eq!(profile.max_lifetime_ms(), 30_000);
        assert_eq!(profile.service(), ZoneServiceKind::Resource);
    }

    #[test]
    fn admission_requires_a_matching_service_an_idempotency_key_and_a_live_deadline() {
        // The resolved service must match the method's service.
        assert_eq!(
            CallDriver::new(
                &resolved(ZoneServiceKind::Zone),
                read_profile(),
                options(1, false),
                false,
                Arc::new(FixedClock(ISSUED)),
            )
            .unwrap_err(),
            ClientError::InvalidMethod
        );
        // A mutating method without a key is refused before any attempt.
        assert_eq!(
            CallDriver::new(
                &resolved(ZoneServiceKind::Resource),
                write_profile(),
                options(1, false),
                false,
                Arc::new(FixedClock(ISSUED)),
            )
            .unwrap_err(),
            ClientError::IdempotencyRequired
        );
        // An already expired deadline is refused.
        assert_eq!(
            CallDriver::new(
                &resolved(ZoneServiceKind::Resource),
                read_profile(),
                options(1, false),
                false,
                Arc::new(FixedClock(ISSUED + 60_000)),
            )
            .unwrap_err(),
            ClientError::DeadlineExpired
        );
    }

    #[test]
    fn the_attempt_budget_is_exact_and_exhaustion_is_typed() {
        let mut driver = driver(read_profile(), 2, false, false);
        let token = CancellationToken::default();
        assert_eq!(driver.attempts_made(), 0);
        assert_eq!(driver.begin_attempt(&token).unwrap().attempt(), 1);
        assert_eq!(driver.begin_attempt(&token).unwrap().attempt(), 2);
        assert_eq!(
            driver.begin_attempt(&token).unwrap_err(),
            ClientError::RetryLimitExceeded
        );
        assert_eq!(driver.attempts_made(), 2);
        assert_eq!(driver.profile(), read_profile());
    }

    #[test]
    fn cancellation_is_forwarded_before_every_attempt() {
        let mut driver = driver(read_profile(), 4, false, false);
        let token = CancellationToken::default();
        let ticket = driver.begin_attempt(&token).expect("first attempt");
        assert!(ticket.relative_timeout_nanos() > 0);
        // A retryable failure would otherwise admit a second attempt.
        assert_eq!(
            driver.record_session_failure(SessionFailure::Retryable),
            AttemptDisposition::RetryNow
        );
        token.cancel();
        assert_eq!(
            driver.begin_attempt(&token).unwrap_err(),
            ClientError::Cancelled
        );
        assert_eq!(driver.attempts_made(), 1);
        // A cancelled attempt is terminal rather than retried.
        assert_eq!(
            driver.record_session_failure(SessionFailure::Cancelled),
            AttemptDisposition::Fail(ClientError::Cancelled)
        );
    }

    #[test]
    fn session_failure_classification_matches_the_carried_over_policy() {
        let read = || driver(read_profile(), 4, false, false);
        for failure in [
            SessionFailure::BeforeDispatch,
            SessionFailure::Retryable,
            SessionFailure::Disconnected,
            SessionFailure::Ambiguous,
        ] {
            assert_eq!(
                read().record_session_failure(failure),
                AttemptDisposition::RetryNow,
                "{failure:?}"
            );
        }
        for (failure, expected) in [
            (SessionFailure::Deadline, ClientError::DeadlineExpired),
            (SessionFailure::Cancelled, ClientError::Cancelled),
            (SessionFailure::Protocol, ClientError::ContractViolation),
        ] {
            assert_eq!(
                read().record_session_failure(failure),
                AttemptDisposition::Fail(expected)
            );
        }

        // A mutating call without a key never retries, and an ambiguous
        // mutating outcome is its own terminal refusal.
        let mutating_no_key =
            MethodProfile::new(ZoneServiceKind::Resource, true, false, 30_000).unwrap();
        let unkeyed = || driver(mutating_no_key, 4, false, false);
        assert_eq!(
            unkeyed().record_session_failure(SessionFailure::Retryable),
            AttemptDisposition::Fail(ClientError::TransportFailed)
        );
        assert_eq!(
            unkeyed().record_session_failure(SessionFailure::Ambiguous),
            AttemptDisposition::Fail(ClientError::AmbiguousMutation)
        );
        // A keyed mutating call may retry a pre-dispatch failure.
        assert_eq!(
            driver(write_profile(), 4, true, false)
                .record_session_failure(SessionFailure::BeforeDispatch),
            AttemptDisposition::RetryNow
        );
        // An ambiguous mutating outcome stays terminal even with a key.
        assert_eq!(
            driver(write_profile(), 4, true, false)
                .record_session_failure(SessionFailure::Ambiguous),
            AttemptDisposition::Fail(ClientError::AmbiguousMutation)
        );
        // An attempt carrying attachments is never replayed.
        assert_eq!(
            driver(read_profile(), 4, false, true)
                .record_session_failure(SessionFailure::Retryable),
            AttemptDisposition::Fail(ClientError::TransportFailed)
        );
    }

    #[test]
    fn an_exhausted_budget_reports_the_retry_limit_rather_than_the_last_failure() {
        let mut driver = driver(read_profile(), 1, false, false);
        let token = CancellationToken::default();
        driver.begin_attempt(&token).expect("only attempt");
        assert_eq!(
            driver.record_session_failure(SessionFailure::Retryable),
            AttemptDisposition::Fail(ClientError::RetryLimitExceeded)
        );
    }

    #[test]
    fn a_peer_verdict_is_an_input_and_is_never_widened() {
        let read = || driver(read_profile(), 4, false, false);
        assert_eq!(
            read().record_remote_error(&remote(
                ResourceErrorKind::Backpressure,
                RetryClass::Immediate,
                None
            )),
            AttemptDisposition::RetryNow
        );
        assert_eq!(
            read().record_remote_error(&remote(
                ResourceErrorKind::Backpressure,
                RetryClass::AfterDelay,
                Some(250)
            )),
            AttemptDisposition::RetryAfterMs(250)
        );
        for retry in [RetryClass::Never, RetryClass::Reauthorize] {
            let error = remote(ResourceErrorKind::AuthorizationDenied, retry, None);
            assert_eq!(
                read().record_remote_error(&error),
                AttemptDisposition::Fail(ClientError::Remote {
                    kind: ResourceErrorKind::AuthorizationDenied,
                    retry,
                })
            );
        }
        // Attachments and an exhausted budget both suppress the retry.
        assert_eq!(
            driver(read_profile(), 4, false, true).record_remote_error(&remote(
                ResourceErrorKind::Backpressure,
                RetryClass::Immediate,
                None
            )),
            AttemptDisposition::Fail(ClientError::Remote {
                kind: ResourceErrorKind::Backpressure,
                retry: RetryClass::Immediate,
            })
        );
        let mut exhausted = driver(read_profile(), 1, false, false);
        exhausted
            .begin_attempt(&CancellationToken::default())
            .expect("only attempt");
        assert_eq!(
            exhausted.record_remote_error(&remote(
                ResourceErrorKind::Backpressure,
                RetryClass::Immediate,
                None
            )),
            AttemptDisposition::Fail(ClientError::Remote {
                kind: ResourceErrorKind::Backpressure,
                retry: RetryClass::Immediate,
            })
        );
    }

    #[test]
    fn the_attempt_timeout_never_exceeds_the_method_ceiling() {
        let profile = MethodProfile::new(ZoneServiceKind::Resource, false, false, 5).unwrap();
        let mut driver = driver(profile, 2, false, false);
        let ticket = driver
            .begin_attempt(&CancellationToken::default())
            .expect("attempt");
        assert!(ticket.relative_timeout_nanos() <= 5 * 1_000_000);
    }
}
