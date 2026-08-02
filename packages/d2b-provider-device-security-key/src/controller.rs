//! Security-key Device controller facade.

use core::fmt;
use d2b_contracts::v3::ResourceUid;

use crate::{
    PhysicalUsbBackingClaim, SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyLease,
    SecurityKeyLeaseError, SecurityKeySessionId, SessionRecord, SessionResult, SessionRing,
};

/// Controller-level failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyControllerError {
    /// Lease state rejected the requested operation.
    Lease(SecurityKeyLeaseError),
    /// Session ring could not be created.
    RingCapacity,
    /// An effect failed while recording a terminal session.
    Effect(SecurityKeyEffectError),
}

impl fmt::Display for SecurityKeyControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Lease(error) => error.code(),
            Self::RingCapacity => "security-key-session-ring-capacity-out-of-range",
            Self::Effect(error) => error.code(),
        })
    }
}

impl std::error::Error for SecurityKeyControllerError {}

/// Combined reconcile outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyReconcileOutcome {
    /// The lease and relay are active.
    Active,
    /// The terminal session was recorded and authority released.
    Completed,
}

/// Device-security-key controller state.
pub struct SecurityKeyController {
    lease: SecurityKeyLease,
    ring: SessionRing,
}

impl SecurityKeyController {
    /// Construct a controller with a bounded session ring.
    pub fn new(
        holder: ResourceUid,
        backing: PhysicalUsbBackingClaim,
        ring_capacity: usize,
    ) -> Result<Self, SecurityKeyControllerError> {
        Ok(Self {
            lease: SecurityKeyLease::new(holder, backing),
            ring: SessionRing::new(ring_capacity)
                .map_err(|_| SecurityKeyControllerError::RingCapacity)?,
        })
    }

    /// Borrow the underlying lease state.
    pub const fn lease(&self) -> &SecurityKeyLease {
        &self.lease
    }

    /// Start a session through the authority-before-open sequence.
    pub fn acquire<P: SecurityKeyEffectPort>(
        &mut self,
        session: SecurityKeySessionId,
        device_uid: ResourceUid,
        port: &mut P,
    ) -> Result<SecurityKeyReconcileOutcome, SecurityKeyControllerError> {
        self.lease
            .acquire(session, device_uid, port)
            .map_err(SecurityKeyControllerError::Lease)?;
        self.ring
            .push(SessionRecord::new(session, SessionResult::InProgress));
        Ok(SecurityKeyReconcileOutcome::Active)
    }

    /// Complete and record the active session.
    pub fn complete<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<SecurityKeyReconcileOutcome, SecurityKeyControllerError> {
        let session = self
            .lease
            .session()
            .copied()
            .ok_or(SecurityKeyControllerError::Lease(
                SecurityKeyLeaseError::InvalidTransition,
            ))?;
        self.lease
            .complete(port)
            .map_err(SecurityKeyControllerError::Lease)?;
        self.ring
            .push(SessionRecord::new(session, SessionResult::Success));
        Ok(SecurityKeyReconcileOutcome::Completed)
    }
}

impl fmt::Debug for SecurityKeyController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurityKeyController")
            .field("lease", &self.lease)
            .field("ring", &self.ring)
            .finish()
    }
}
