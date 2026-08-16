//! Native guest-relay lifecycle and restart adoption.

use crate::{GuestIdentity, ReadySession};
use async_trait::async_trait;
use std::fmt;

/// Exact Guest-bound relay identity.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayBinding {
    guest: GuestIdentity,
    session_token: [u8; 16],
}

impl RelayBinding {
    /// Construct a binding at the authenticated core boundary.
    pub const fn new(guest: GuestIdentity, session_token: [u8; 16]) -> Self {
        Self {
            guest,
            session_token,
        }
    }

    /// Borrow the Guest identity.
    pub const fn guest(&self) -> &GuestIdentity {
        &self.guest
    }
}

impl fmt::Debug for RelayBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelayBinding(<redacted>)")
    }
}

/// Restart observation supplied by the Core relay adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayObservation<L, R> {
    /// Binding proof attached to the observed listener and relay.
    pub binding: RelayBinding,
    /// Matching listener handle.
    pub listener: L,
    /// Matching relay process handle.
    pub process: R,
}

impl<L, R> fmt::Debug for RelayObservation<L, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayObservation")
            .field("binding", &self.binding)
            .field("listener", &"<redacted>")
            .field("process", &"<redacted>")
            .finish()
    }
}

/// Stable relay effect failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayEffectError {
    /// Another relay owns the CID authority.
    CidAuthorityConflict,
    /// The listener could not be acquired.
    ListenerUnavailable,
    /// The relay process could not be started.
    ProcessUnavailable,
    /// The observed listener or process does not match the binding.
    RestartMismatch,
    /// The effect can be retried.
    Transient,
    /// Closure was not confirmed.
    CloseUnconfirmed,
}

impl RelayEffectError {
    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::CidAuthorityConflict => "vsock-cid-authority-conflict",
            Self::ListenerUnavailable => "vsock-listener-unavailable",
            Self::ProcessUnavailable => "vsock-relay-process-unavailable",
            Self::RestartMismatch => "vsock-relay-restart-mismatch",
            Self::Transient => "vsock-relay-transient",
            Self::CloseUnconfirmed => "vsock-relay-close-unconfirmed",
        }
    }
}

impl fmt::Display for RelayEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for RelayEffectError {}

/// Effect boundary for the native guest relay.
#[async_trait]
pub trait RelayEffectPort: Send + Sync + 'static {
    /// Opaque CID authority reservation.
    type CidReservation: Send + 'static;
    /// Opaque listener handle.
    type Listener: Send + 'static;
    /// Opaque relay process identity.
    type RelayProcess: Send + 'static;

    /// Reserve the exact Host-global CID before any effect starts.
    async fn reserve_cid(
        &self,
        binding: &RelayBinding,
    ) -> Result<Self::CidReservation, RelayEffectError>;

    /// Bind the matching listener while retaining CID authority.
    async fn bind_listener(
        &self,
        binding: &RelayBinding,
        reservation: &Self::CidReservation,
    ) -> Result<Self::Listener, RelayEffectError>;

    /// Start the native relay process.
    async fn spawn_relay(
        &self,
        binding: &RelayBinding,
        listener: &Self::Listener,
        reservation: &Self::CidReservation,
    ) -> Result<Self::RelayProcess, RelayEffectError>;

    /// Close the relay process.
    async fn close_relay(&self, process: &Self::RelayProcess) -> Result<(), RelayEffectError>;

    /// Close the listener.
    async fn close_listener(&self, listener: &Self::Listener) -> Result<(), RelayEffectError>;

    /// Release CID authority after listener and relay closure.
    async fn release_cid(&self, reservation: &Self::CidReservation)
    -> Result<(), RelayEffectError>;

    /// Find a matching listener and relay during restart adoption.
    async fn observe(
        &self,
        binding: &RelayBinding,
    ) -> Result<Option<RelayObservation<Self::Listener, Self::RelayProcess>>, RelayEffectError>;
}

/// Relay lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayPhase {
    /// No CID, listener, or relay is held.
    Idle,
    /// CID and listener are being acquired.
    Starting,
    /// The matching listener and relay are ready.
    Ready,
    /// Restart adoption found ambiguity or a stale identity.
    Degraded,
    /// Closure is in progress; authority remains held.
    Finalizing,
    /// All effects closed and CID authority released.
    Closed,
}

/// Native relay controller. It owns no raw socket or process identity.
pub struct NativeGuestRelay<P>
where
    P: RelayEffectPort,
{
    port: P,
    binding: RelayBinding,
    reservation: Option<P::CidReservation>,
    listener: Option<P::Listener>,
    process: Option<P::RelayProcess>,
    phase: RelayPhase,
}

impl<P> NativeGuestRelay<P>
where
    P: RelayEffectPort,
{
    /// Construct an idle relay controller.
    pub fn new(port: P, binding: RelayBinding) -> Self {
        Self {
            port,
            binding,
            reservation: None,
            listener: None,
            process: None,
            phase: RelayPhase::Idle,
        }
    }

    /// Return the current relay lifecycle phase.
    pub const fn phase(&self) -> RelayPhase {
        self.phase
    }

    /// Borrow the exact relay binding.
    pub const fn binding(&self) -> &RelayBinding {
        &self.binding
    }

    /// Acquire CID authority, bind the listener, and start the native relay.
    pub async fn start(&mut self, session: &ReadySession) -> Result<(), RelayEffectError> {
        if !session.matches(self.binding.guest()) {
            self.phase = RelayPhase::Degraded;
            return Err(RelayEffectError::RestartMismatch);
        }
        if !matches!(self.phase, RelayPhase::Idle | RelayPhase::Closed) {
            return Err(RelayEffectError::Transient);
        }
        self.phase = RelayPhase::Starting;
        let reservation = match self.port.reserve_cid(&self.binding).await {
            Ok(reservation) => reservation,
            Err(error) => {
                self.phase = RelayPhase::Idle;
                return Err(error);
            }
        };
        self.reservation = Some(reservation);
        let listener = match self
            .port
            .bind_listener(
                &self.binding,
                self.reservation.as_ref().expect("reservation"),
            )
            .await
        {
            Ok(listener) => listener,
            Err(error) => {
                if self
                    .port
                    .release_cid(self.reservation.as_ref().expect("reservation"))
                    .await
                    .is_ok()
                {
                    self.reservation = None;
                    self.phase = RelayPhase::Idle;
                } else {
                    self.phase = RelayPhase::Degraded;
                }
                return Err(error);
            }
        };
        self.listener = Some(listener);
        let process = match self
            .port
            .spawn_relay(
                &self.binding,
                self.listener.as_ref().expect("listener"),
                self.reservation.as_ref().expect("reservation"),
            )
            .await
        {
            Ok(process) => process,
            Err(error) => {
                let listener_closed = self
                    .port
                    .close_listener(self.listener.as_ref().expect("listener"))
                    .await
                    .is_ok();
                if listener_closed {
                    self.listener = None;
                }
                let reservation_released = if listener_closed {
                    self.port
                        .release_cid(self.reservation.as_ref().expect("reservation"))
                        .await
                        .is_ok()
                } else {
                    false
                };
                if reservation_released {
                    self.reservation = None;
                }
                self.phase = if listener_closed && reservation_released {
                    RelayPhase::Idle
                } else {
                    RelayPhase::Degraded
                };
                return Err(error);
            }
        };
        self.process = Some(process);
        self.phase = RelayPhase::Ready;
        Ok(())
    }

    /// Adopt only the exact matching listener and relay after restart.
    pub async fn adopt(&mut self, reservation: P::CidReservation) -> Result<(), RelayEffectError> {
        if !matches!(self.phase, RelayPhase::Idle | RelayPhase::Closed) {
            return Err(RelayEffectError::Transient);
        }
        self.reservation = Some(reservation);
        let observation = match self.port.observe(&self.binding).await {
            Ok(Some(observation)) => observation,
            Ok(None) => {
                self.phase = RelayPhase::Degraded;
                return Err(RelayEffectError::RestartMismatch);
            }
            Err(error) => {
                self.phase = RelayPhase::Degraded;
                return Err(error);
            }
        };
        if observation.binding != self.binding {
            self.phase = RelayPhase::Degraded;
            return Err(RelayEffectError::RestartMismatch);
        }
        self.listener = Some(observation.listener);
        self.process = Some(observation.process);
        self.phase = RelayPhase::Ready;
        Ok(())
    }

    /// Close the relay, then listener, then release CID authority.
    pub async fn finalize(&mut self) -> Result<(), RelayEffectError> {
        if self.phase == RelayPhase::Closed {
            return Ok(());
        }
        self.phase = RelayPhase::Finalizing;
        if let Some(process) = self.process.as_ref() {
            self.port.close_relay(process).await?;
            self.process = None;
        }
        if let Some(listener) = self.listener.as_ref() {
            self.port.close_listener(listener).await?;
            self.listener = None;
        }
        if let Some(reservation) = self.reservation.as_ref() {
            self.port.release_cid(reservation).await?;
            self.reservation = None;
        }
        self.phase = RelayPhase::Closed;
        Ok(())
    }
}

impl<P> fmt::Debug for NativeGuestRelay<P>
where
    P: RelayEffectPort,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeGuestRelay")
            .field("phase", &self.phase)
            .field("has_reservation", &self.reservation.is_some())
            .field("has_listener", &self.listener.is_some())
            .field("has_process", &self.process.is_some())
            .finish()
    }
}
