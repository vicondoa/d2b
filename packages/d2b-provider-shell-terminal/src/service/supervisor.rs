//! Per-session terminal supervisor contracts.

use std::collections::BTreeSet;

use crate::{
    Authorizer, ShellPool, ShellSession, ShellTerminalError, Subject,
    session::{OutputRing, RingReplay, SupervisorIdentity},
};

/// A bounded direct-attach request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachRequest {
    expected_generation: u64,
    tail_bytes: u64,
}

impl AttachRequest {
    /// Construct an attach request for one exact supervisor generation.
    pub fn new(expected_generation: u64, tail_bytes: u64) -> Result<Self, ShellTerminalError> {
        if tail_bytes > 1024 * 1024 {
            return Err(ShellTerminalError::CapacityOutOfRange);
        }
        Ok(Self {
            expected_generation,
            tail_bytes,
        })
    }
}

/// A one-shot capability minted only by an already authorized `OpenSession`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionCapability {
    id: u64,
    generation: u64,
}

impl SessionCapability {
    pub(super) const fn new(id: u64, generation: u64) -> Self {
        Self { id, generation }
    }
}

/// A successful attachment response with redacted terminal replay bytes.
pub struct AttachReceipt {
    generation: u64,
    replay: RingReplay,
}

impl AttachReceipt {
    /// Return the generation admitted for the named terminal stream.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Borrow the replay prepared for the authenticated stream.
    pub const fn replay(&self) -> &RingReplay {
        &self.replay
    }
}

impl std::fmt::Debug for AttachReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachReceipt")
            .field("generation", &self.generation)
            .field("replay", &self.replay)
            .finish()
    }
}

/// One supervisor that owns exactly one PTY/ring model for one session.
pub struct SessionSupervisor {
    session: ShellSession,
    pool: ShellPool,
    identity: SupervisorIdentity,
    ring: OutputRing,
    attached: u32,
    consumed_capabilities: BTreeSet<u64>,
}

impl SessionSupervisor {
    /// Construct a supervisor from process-adapter identity evidence.
    pub fn new(session: ShellSession, pool: ShellPool, identity: SupervisorIdentity) -> Self {
        let ring = OutputRing::new(session.output_ring_capacity() as usize)
            .expect("a validated session has a valid output ring capacity");
        Self {
            session,
            pool,
            identity,
            ring,
            attached: 0,
            consumed_capabilities: BTreeSet::new(),
        }
    }

    /// Authorize and attach a direct named terminal stream.
    pub fn attach(
        &mut self,
        subject: &Subject,
        request: AttachRequest,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        self.authorize(subject)?;
        if request.expected_generation != self.identity.generation() {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        self.reserve_attachment()?;
        Ok(AttachReceipt {
            generation: self.identity.generation(),
            replay: self.ring.tail(request.tail_bytes as usize),
        })
    }

    /// Consume a one-shot capability after rechecking the current request authority.
    pub fn attach_with_capability(
        &mut self,
        subject: &Subject,
        capability: SessionCapability,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        self.authorize(subject)?;
        if capability.generation != self.identity.generation() {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        if !self.consumed_capabilities.insert(capability.id) {
            return Err(ShellTerminalError::CapabilityReused);
        }
        self.reserve_attachment()?;
        Ok(AttachReceipt {
            generation: self.identity.generation(),
            replay: self.ring.tail(0),
        })
    }

    fn authorize(&self, subject: &Subject) -> Result<(), ShellTerminalError> {
        Authorizer::authorize_target(
            subject,
            self.session.zone(),
            self.session.execution_target(),
        )
    }

    fn reserve_attachment(&mut self) -> Result<(), ShellTerminalError> {
        if self.attached >= self.pool.spec().max_attached() {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        self.attached = self.attached.saturating_add(1);
        Ok(())
    }
}

impl std::fmt::Debug for SessionSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionSupervisor")
            .field("generation", &self.identity.generation())
            .field("attached", &self.attached)
            .field("ring", &self.ring)
            .finish()
    }
}
