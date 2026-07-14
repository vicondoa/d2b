use std::{fmt, time::Instant};

use d2b_contracts::{
    v2_component_session::{
        CorrelationId, IdempotencyKey, RequestEnvelope, RequestId, SessionErrorCode, TraceId,
    },
    v2_services::{admit_metadata, common},
};

use crate::{Result, SessionError};

pub struct DeadlineBudget {
    envelope: RequestEnvelope,
    service_max_lifetime_ms: u64,
    monotonic_deadline: Instant,
}

impl DeadlineBudget {
    pub fn admit_metadata(
        metadata: &common::RequestMetadata,
        expected_session_generation: u64,
        requires_idempotency: bool,
        local_wall_clock_ms: u64,
        now: Instant,
        service_max_lifetime_ms: u64,
        peer_ttrpc_timeout_nanos: Option<u64>,
    ) -> Result<Self> {
        if metadata.session_generation != expected_session_generation {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        admit_metadata(
            metadata,
            requires_idempotency,
            local_wall_clock_ms,
            service_max_lifetime_ms,
            None,
            peer_ttrpc_timeout_nanos,
        )
        .map_err(|_| SessionError::new(SessionErrorCode::DeadlineInvalid))?;
        let envelope = RequestEnvelope {
            request_id: RequestId::new(metadata.request_id.clone())?,
            correlation_id: if metadata.correlation_id.is_empty() {
                None
            } else {
                Some(CorrelationId::new(
                    metadata.correlation_id.as_bytes().to_vec(),
                )?)
            },
            trace_id: if metadata.trace_id.is_empty() {
                None
            } else {
                Some(TraceId::new(metadata.trace_id.clone())?)
            },
            idempotency_key: if metadata.idempotency_key.is_empty() {
                None
            } else {
                Some(IdempotencyKey::new(metadata.idempotency_key.clone())?)
            },
            issued_at_unix_ms: metadata.issued_at_unix_ms,
            expires_at_unix_ms: metadata.expires_at_unix_ms,
        };
        Self::admit(
            envelope,
            local_wall_clock_ms,
            now,
            service_max_lifetime_ms,
            peer_ttrpc_timeout_nanos,
        )
    }

    pub fn admit(
        envelope: RequestEnvelope,
        local_wall_clock_ms: u64,
        now: Instant,
        service_max_lifetime_ms: u64,
        peer_ttrpc_timeout_nanos: Option<u64>,
    ) -> Result<Self> {
        let admitted = envelope.admit(
            local_wall_clock_ms,
            service_max_lifetime_ms,
            None,
            peer_ttrpc_timeout_nanos,
        )?;
        let monotonic_deadline = now
            .checked_add(std::time::Duration::from_nanos(admitted.remaining_nanos))
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        Ok(Self {
            envelope,
            service_max_lifetime_ms,
            monotonic_deadline,
        })
    }

    pub fn absolute_expiry_unix_ms(&self) -> u64 {
        self.envelope.expires_at_unix_ms
    }

    pub fn remaining_nanos(
        &self,
        local_wall_clock_ms: u64,
        now: Instant,
        peer_ttrpc_timeout_nanos: Option<u64>,
    ) -> Result<u64> {
        let monotonic = self
            .monotonic_deadline
            .checked_duration_since(now)
            .ok_or_else(|| SessionError::new(SessionErrorCode::DeadlineExpired))?
            .as_nanos();
        let monotonic = u64::try_from(monotonic).unwrap_or(u64::MAX);
        self.envelope
            .admit(
                local_wall_clock_ms,
                self.service_max_lifetime_ms,
                Some(monotonic),
                peer_ttrpc_timeout_nanos,
            )
            .map(|admitted| admitted.remaining_nanos)
            .map_err(|_| SessionError::new(SessionErrorCode::DeadlineExpired))
    }

    pub fn ttrpc_context(
        &self,
        local_wall_clock_ms: u64,
        now: Instant,
        peer_ttrpc_timeout_nanos: Option<u64>,
    ) -> Result<ttrpc::context::Context> {
        let remaining = self.remaining_nanos(local_wall_clock_ms, now, peer_ttrpc_timeout_nanos)?;
        Ok(ttrpc::context::with_timeout(
            remaining.min(i64::MAX as u64) as i64
        ))
    }

    pub fn peer_timeout(timeout_nano: i64) -> Option<u64> {
        u64::try_from(timeout_nano).ok().filter(|value| *value != 0)
    }
}

impl fmt::Debug for DeadlineBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeadlineBudget")
            .field("absolute_expiry", &"<redacted>")
            .field("request_id", &"<redacted>")
            .finish_non_exhaustive()
    }
}
