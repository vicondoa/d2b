use std::{fmt, time::Instant};

use d2b_contracts_zone_session::v3::component_session::{
    CloseReason, CloseRecord, KeepaliveRecord, LimitProfile, Remediation, SessionErrorCode,
};

use crate::{Result, SessionError};

/// Phase of a component session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// The session is established and exchanging records.
    Established,
    /// The session lost its transport and is not yet reconnecting.
    Disconnected,
    /// A reconnect attempt is in progress.
    Reconnecting,
    /// The session is closing.
    Closing,
    /// The session is closed.
    Closed,
}

/// Action a keepalive poll asks the caller to take.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeepaliveAction {
    /// No action is due.
    None,
    /// Send a keepalive ping carrying the given record.
    SendPing(KeepaliveRecord),
    /// Close the session with the given record.
    Close(CloseRecord),
}

impl fmt::Debug for KeepaliveAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("KeepaliveAction::None"),
            Self::SendPing(_) => formatter.write_str("KeepaliveAction::SendPing(<redacted>)"),
            Self::Close(record) => formatter
                .debug_tuple("KeepaliveAction::Close")
                .field(&record.reason.as_str())
                .finish(),
        }
    }
}

/// Tracks session phase, generation, keepalive state, and reconnect budget.
pub struct SessionLifecycle {
    phase: SessionPhase,
    generation: u64,
    limits: LimitProfile,
    last_activity: Instant,
    pending_ping: Option<(u64, Instant)>,
    next_ping_nonce: u64,
    disconnected_at: Option<Instant>,
    reconnect_attempts: u16,
}

impl SessionLifecycle {
    /// Construct an established lifecycle for one generation.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::GenerationMismatch`] when `generation` is zero,
    /// and the limit-profile validation error when `limits` is invalid.
    pub fn new(generation: u64, limits: LimitProfile, now: Instant) -> Result<Self> {
        limits.validate()?;
        if generation == 0 {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        Ok(Self {
            phase: SessionPhase::Established,
            generation,
            limits,
            last_activity: now,
            pending_ping: None,
            next_ping_nonce: 0,
            disconnected_at: None,
            reconnect_attempts: 0,
        })
    }

    /// Return the current phase.
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Return the current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Record transport activity while established, refreshing the keepalive idle clock.
    pub fn on_activity(&mut self, now: Instant) {
        if self.phase == SessionPhase::Established {
            self.last_activity = now;
        }
    }

    /// Decide the keepalive action due at `now`.
    ///
    /// Returns `SendPing` when the keepalive interval elapsed without a pending
    /// ping, `Close` when the ping timeout elapsed or ping nonces are exhausted,
    /// and `None` otherwise.
    pub fn poll_keepalive(&mut self, now: Instant) -> KeepaliveAction {
        if self.phase != SessionPhase::Established {
            return KeepaliveAction::None;
        }
        if let Some((_, sent_at)) = self.pending_ping {
            if now
                .checked_duration_since(sent_at)
                .unwrap_or_default()
                .as_millis()
                >= u128::from(self.limits.keepalive_timeout_ms)
            {
                self.phase = SessionPhase::Closing;
                return KeepaliveAction::Close(CloseRecord {
                    reconnect_generation: self.generation,
                    reason: CloseReason::KeepaliveTimeout,
                    remediation: Remediation::RetryBounded,
                });
            }
            return KeepaliveAction::None;
        }
        if now
            .checked_duration_since(self.last_activity)
            .unwrap_or_default()
            .as_millis()
            < u128::from(self.limits.keepalive_interval_ms)
        {
            return KeepaliveAction::None;
        }
        if self.next_ping_nonce == u64::MAX {
            self.phase = SessionPhase::Closing;
            return KeepaliveAction::Close(CloseRecord {
                reconnect_generation: self.generation,
                reason: CloseReason::NonceExhausted,
                remediation: Remediation::ReplaceGeneration,
            });
        }
        let nonce = self.next_ping_nonce;
        self.next_ping_nonce += 1;
        self.pending_ping = Some((nonce, now));
        KeepaliveAction::SendPing(KeepaliveRecord {
            reconnect_generation: self.generation,
            nonce,
        })
    }

    /// Accept a pong for the pending ping.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::GenerationMismatch`] when the pong generation
    /// differs, and [`SessionErrorCode::UnknownControl`] when the pong does not
    /// match the pending ping.
    pub fn receive_pong(&mut self, pong: KeepaliveRecord, now: Instant) -> Result<()> {
        if pong.reconnect_generation != self.generation {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        match self.pending_ping {
            Some((nonce, _)) if nonce == pong.nonce => {
                self.pending_ping = None;
                self.last_activity = now;
                Ok(())
            }
            _ => Err(SessionError::new(SessionErrorCode::UnknownControl)),
        }
    }

    /// Mark the session disconnected, resetting the reconnect budget.
    pub fn disconnect(&mut self, now: Instant) {
        self.phase = SessionPhase::Disconnected;
        self.pending_ping = None;
        self.disconnected_at = Some(now);
        self.reconnect_attempts = 0;
    }

    /// Begin a reconnect attempt, advancing the generation.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::InternalInvariant`] when the phase is not
    /// `Disconnected` or `Reconnecting`, [`SessionErrorCode::SessionDisconnected`]
    /// when the reconnect budget or window is exhausted, and
    /// [`SessionErrorCode::NonceExhausted`] when the generation overflows.
    pub fn begin_reconnect(&mut self, now: Instant) -> Result<u64> {
        if !matches!(
            self.phase,
            SessionPhase::Disconnected | SessionPhase::Reconnecting
        ) {
            return Err(SessionError::new(SessionErrorCode::InternalInvariant));
        }
        let disconnected_at = self
            .disconnected_at
            .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
        if self.reconnect_attempts >= self.limits.reconnect_attempts
            || now
                .checked_duration_since(disconnected_at)
                .unwrap_or_default()
                .as_millis()
                >= u128::from(self.limits.reconnect_window_ms)
        {
            self.phase = SessionPhase::Closed;
            return Err(SessionError::new(SessionErrorCode::SessionDisconnected));
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| SessionError::new(SessionErrorCode::NonceExhausted))?;
        self.reconnect_attempts += 1;
        self.phase = SessionPhase::Reconnecting;
        Ok(self.generation)
    }

    /// Mark a reconnect attempt established.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::InternalInvariant`] when the phase is not
    /// `Reconnecting`.
    pub fn reconnect_established(&mut self, now: Instant) -> Result<()> {
        if self.phase != SessionPhase::Reconnecting {
            return Err(SessionError::new(SessionErrorCode::InternalInvariant));
        }
        self.phase = SessionPhase::Established;
        self.last_activity = now;
        self.pending_ping = None;
        self.disconnected_at = None;
        self.reconnect_attempts = 0;
        Ok(())
    }

    /// Close the session, returning the close record to emit.
    pub fn close(&mut self, reason: CloseReason, remediation: Remediation) -> CloseRecord {
        self.phase = SessionPhase::Closed;
        CloseRecord {
            reconnect_generation: self.generation,
            reason,
            remediation,
        }
    }
}

impl fmt::Debug for SessionLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionLifecycle")
            .field("phase", &self.phase)
            .field("generation", &"<redacted>")
            .field("pending_ping", &self.pending_ping.is_some())
            .field("reconnect_attempts", &self.reconnect_attempts)
            .finish()
    }
}
