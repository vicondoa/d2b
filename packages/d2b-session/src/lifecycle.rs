use std::{fmt, time::Instant};

use d2b_contracts::v2_component_session::{
    CloseReason, CloseRecord, KeepaliveRecord, LimitProfile, Remediation, SessionErrorCode,
};

use crate::{Result, SessionError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Established,
    Disconnected,
    Reconnecting,
    Closing,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeepaliveAction {
    None,
    SendPing(KeepaliveRecord),
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

    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn on_activity(&mut self, now: Instant) {
        if self.phase == SessionPhase::Established {
            self.last_activity = now;
        }
    }

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

    pub fn disconnect(&mut self, now: Instant) {
        self.phase = SessionPhase::Disconnected;
        self.pending_ping = None;
        self.disconnected_at = Some(now);
        self.reconnect_attempts = 0;
    }

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
