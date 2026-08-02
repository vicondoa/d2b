//! Single-session security-key lease state machine.

use core::fmt;
use d2b_contracts::v3::ResourceUid;

use crate::authority::{
    PhysicalAuthorityLease, PhysicalUsbBackingClaim, RelayLaunchTicket, SecurityKeyEffectError,
    SecurityKeyEffectPort, SecurityKeyOpenIntent,
};

/// Opaque security-key session identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecurityKeySessionId([u8; 16]);

impl SecurityKeySessionId {
    /// Construct an ID at the relay/Core boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for SecurityKeySessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecurityKeySessionId(<redacted>)")
    }
}

/// Lease lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    /// No session owns the Device.
    Idle,
    /// A Guest request is waiting for the authority.
    AwaitingLease,
    /// The relay has the active physical lease.
    Active,
    /// The session ended normally.
    Completed,
    /// The session was cancelled.
    Cancelled,
    /// The bounded timeout expired.
    Expired,
}

/// Closed lease failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyLeaseError {
    /// The controller already has an active or waiting session.
    SessionConflict,
    /// The physical backing authority rejected the request.
    Effect(SecurityKeyEffectError),
    /// A transition was requested from the wrong state.
    InvalidTransition,
}

impl SecurityKeyLeaseError {
    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::SessionConflict => "device-claim-conflict",
            Self::Effect(error) => error.code(),
            Self::InvalidTransition => "device-session-invalid-transition",
        }
    }
}

impl fmt::Display for SecurityKeyLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SecurityKeyLeaseError {}

/// Active lease state held by one Device controller.
pub struct SecurityKeyLease {
    holder: ResourceUid,
    backing: PhysicalUsbBackingClaim,
    state: LeaseState,
    session: Option<SecurityKeySessionId>,
    authority_lease: Option<PhysicalAuthorityLease>,
    relay_ticket: Option<RelayLaunchTicket>,
}

impl SecurityKeyLease {
    /// Construct an idle Device lease.
    pub fn new(holder: ResourceUid, backing: PhysicalUsbBackingClaim) -> Self {
        Self {
            holder,
            backing,
            state: LeaseState::Idle,
            session: None,
            authority_lease: None,
            relay_ticket: None,
        }
    }

    /// Return the current lifecycle state.
    pub const fn state(&self) -> LeaseState {
        self.state
    }

    /// Borrow the opaque holder identity.
    pub const fn holder(&self) -> &ResourceUid {
        &self.holder
    }

    /// Borrow the active session ID, if present.
    pub const fn session(&self) -> Option<&SecurityKeySessionId> {
        self.session.as_ref()
    }

    /// Start a session, claiming physical authority before opening hidraw.
    pub fn acquire<P: SecurityKeyEffectPort>(
        &mut self,
        session: SecurityKeySessionId,
        device_uid: ResourceUid,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        if !matches!(
            self.state,
            LeaseState::Idle | LeaseState::Completed | LeaseState::Cancelled | LeaseState::Expired
        ) || self.session.is_some()
        {
            return Err(SecurityKeyLeaseError::SessionConflict);
        }
        self.state = LeaseState::AwaitingLease;
        let authority_lease = match port.claim_physical_backing(&self.backing) {
            Ok(lease) => lease,
            Err(error) => {
                self.state = LeaseState::Idle;
                return Err(SecurityKeyLeaseError::Effect(error));
            }
        };
        let intent = SecurityKeyOpenIntent::from_core(device_uid, session, self.backing.clone());
        let relay_ticket = match port.open_hidraw(&intent) {
            Ok(ticket) => ticket,
            Err(error) => {
                let _ = port.release_physical_backing(authority_lease);
                self.state = LeaseState::Idle;
                return Err(SecurityKeyLeaseError::Effect(error));
            }
        };
        self.session = Some(session);
        self.authority_lease = Some(authority_lease);
        self.relay_ticket = Some(relay_ticket);
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Complete the active session and release its authority.
    pub fn complete<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish(LeaseState::Completed, port)
    }

    /// Cancel the active session and release its authority.
    pub fn cancel<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish(LeaseState::Cancelled, port)
    }

    /// Expire the active session and release its authority.
    pub fn expire<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish(LeaseState::Expired, port)
    }

    fn finish<P: SecurityKeyEffectPort>(
        &mut self,
        terminal: LeaseState,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        if self.state != LeaseState::Active {
            return Err(SecurityKeyLeaseError::InvalidTransition);
        }
        let authority = self
            .authority_lease
            .take()
            .ok_or(SecurityKeyLeaseError::InvalidTransition)?;
        port.release_physical_backing(authority)
            .map_err(SecurityKeyLeaseError::Effect)?;
        self.relay_ticket = None;
        self.session = None;
        self.state = terminal;
        Ok(())
    }
}

impl fmt::Debug for SecurityKeyLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurityKeyLease")
            .field("holder", &"<redacted>")
            .field("backing", &self.backing)
            .field("state", &self.state)
            .field("session", &self.session)
            .finish()
    }
}
