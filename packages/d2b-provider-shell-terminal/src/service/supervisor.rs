//! Per-session terminal supervisor contracts.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use crate::{
    Authorizer, ExecutionTarget, ShellPool, ShellSession, ShellTerminalError, Subject,
    session::{OutputRing, RingReplay, SupervisorIdentity},
};

#[derive(PartialEq, Eq)]
struct SessionFingerprint {
    name: String,
    zone: String,
    pool_name: String,
    execution_target: ExecutionTarget,
    workload_user: String,
    login_shell_ref: String,
    output_ring_capacity: u64,
}

impl SessionFingerprint {
    fn from_session(session: &ShellSession) -> Self {
        Self {
            name: session.name().to_owned(),
            zone: session.zone().to_owned(),
            pool_name: session.pool_name().to_owned(),
            execution_target: session.execution_target().clone(),
            workload_user: session.workload_user().to_owned(),
            login_shell_ref: session.login_shell_ref().to_owned(),
            output_ring_capacity: session.output_ring_capacity(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct PoolFingerprint {
    name: String,
    zone: String,
    execution_target: ExecutionTarget,
    workload_user: String,
    login_shell_ref: String,
    max_sessions: u32,
    max_attached: u32,
    output_ring_capacity: u64,
}

impl PoolFingerprint {
    fn from_pool(pool: &ShellPool) -> Self {
        Self {
            name: pool.name().to_owned(),
            zone: pool.zone().to_owned(),
            execution_target: pool.execution_target().clone(),
            workload_user: pool.workload_user().to_owned(),
            login_shell_ref: pool.spec().login_shell_ref().to_owned(),
            max_sessions: pool.spec().max_sessions(),
            max_attached: pool.spec().max_attached(),
            output_ring_capacity: pool.spec().output_ring_capacity(),
        }
    }
}

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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionCapability {
    id: u64,
    generation: u64,
    session_name: String,
}

impl SessionCapability {
    pub(super) fn new(id: u64, generation: u64, session_name: String) -> Self {
        Self {
            id,
            generation,
            session_name,
        }
    }
}

impl std::fmt::Debug for SessionCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionCapability")
            .field("id", &"<redacted>")
            .field("generation", &self.generation)
            .finish()
    }
}

/// An opaque handle for one active stream attachment.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Attachment {
    id: u64,
    session_name: String,
    generation: u64,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Attachment(<redacted>)")
    }
}

/// The single-flight authority for one session supervisor generation.
pub struct SessionAuthority {
    fingerprint: SessionFingerprint,
    generation: Mutex<u64>,
}

impl std::fmt::Debug for SessionAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionAuthority(<redacted>)")
    }
}

impl SessionAuthority {
    pub(super) fn new(session: &ShellSession, generation: u64) -> Self {
        Self {
            fingerprint: SessionFingerprint::from_session(session),
            generation: Mutex::new(generation),
        }
    }

    pub(super) fn matches(
        &self,
        session: &ShellSession,
        generation: u64,
    ) -> Result<bool, ShellTerminalError> {
        Ok(*self
            .generation
            .lock()
            .map_err(|_| ShellTerminalError::StaleSessionGeneration)?
            == generation
            && self.fingerprint == SessionFingerprint::from_session(session))
    }

    pub(super) fn advance(&self) -> Result<u64, ShellTerminalError> {
        let mut generation = self
            .generation
            .lock()
            .map_err(|_| ShellTerminalError::StaleSessionGeneration)?;
        *generation = generation
            .checked_add(1)
            .ok_or(ShellTerminalError::StaleSessionGeneration)?;
        Ok(*generation)
    }

    fn with_current<T>(
        &self,
        session: &ShellSession,
        expected_generation: u64,
        operation: impl FnOnce() -> Result<T, ShellTerminalError>,
    ) -> Result<T, ShellTerminalError> {
        let current = self
            .generation
            .lock()
            .map_err(|_| ShellTerminalError::StaleSessionGeneration)?;
        if *current != expected_generation
            || self.fingerprint != SessionFingerprint::from_session(session)
        {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        operation()
    }
}

/// A synchronized attachment quota shared by all supervisors in one pool.
#[derive(Debug)]
pub(super) struct PoolAttachmentBudget {
    capacity: usize,
    retained_attachments: Mutex<usize>,
    entries: Mutex<BTreeSet<Attachment>>,
    next_id: Mutex<u64>,
}

impl PoolAttachmentBudget {
    pub(super) fn restored(
        capacity: u32,
        retained_attachments: u32,
    ) -> Result<Self, ShellTerminalError> {
        if retained_attachments > capacity {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        Ok(Self {
            capacity: capacity as usize,
            retained_attachments: Mutex::new(retained_attachments as usize),
            entries: Mutex::new(BTreeSet::new()),
            next_id: Mutex::new(0),
        })
    }

    fn reserve(
        &self,
        session_name: &str,
        generation: u64,
    ) -> Result<Attachment, ShellTerminalError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        let retained_attachments = self
            .retained_attachments
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        if retained_attachments.saturating_add(entries.len()) >= self.capacity {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        let mut next_id = self
            .next_id
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        *next_id = next_id
            .checked_add(1)
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        let attachment = Attachment {
            id: *next_id,
            session_name: session_name.to_owned(),
            generation,
        };
        entries.insert(attachment.clone());
        Ok(attachment)
    }

    fn release(&self, attachment: &Attachment) -> Result<(), ShellTerminalError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ShellTerminalError::AttachmentUnknown)?;
        if entries.remove(attachment) {
            Ok(())
        } else {
            Err(ShellTerminalError::AttachmentUnknown)
        }
    }

    pub(super) fn reconcile_retained_attachments(
        &self,
        retained_attachments: u32,
    ) -> Result<(), ShellTerminalError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        let mut retained = self
            .retained_attachments
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        if (retained_attachments as usize) > self.capacity
            || (retained_attachments as usize) < entries.len()
        {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        *retained = retained_attachments as usize - entries.len();
        Ok(())
    }

    fn retire_proven_stale(
        &self,
        stale_attachments: &[Attachment],
        remote_attachments: u32,
    ) -> Result<(), ShellTerminalError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        let mut retained = self
            .retained_attachments
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)?;
        for attachment in stale_attachments {
            entries.remove(attachment);
        }
        if entries.len().saturating_add(remote_attachments as usize) > self.capacity {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        *retained = remote_attachments as usize;
        Ok(())
    }
}

/// The shared attachment authority that survives controller restart adoption.
#[derive(Clone)]
pub struct PoolAttachmentAuthority {
    fingerprint: PoolFingerprint,
    budget: Arc<PoolAttachmentBudget>,
}

impl std::fmt::Debug for PoolAttachmentAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PoolAttachmentAuthority(<redacted>)")
    }
}

impl PoolAttachmentAuthority {
    pub(super) fn restored(
        pool: &ShellPool,
        capacity: u32,
        retained_attachments: u32,
    ) -> Result<Self, ShellTerminalError> {
        Ok(Self {
            fingerprint: PoolFingerprint::from_pool(pool),
            budget: Arc::new(PoolAttachmentBudget::restored(
                capacity,
                retained_attachments,
            )?),
        })
    }

    pub(super) fn matches_pool(&self, pool: &ShellPool) -> bool {
        self.fingerprint == PoolFingerprint::from_pool(pool)
            && self.budget.capacity == pool.spec().max_attached() as usize
    }

    fn reserve(
        &self,
        session_name: &str,
        generation: u64,
    ) -> Result<Attachment, ShellTerminalError> {
        self.budget.reserve(session_name, generation)
    }

    fn release(&self, attachment: &Attachment) -> Result<(), ShellTerminalError> {
        self.budget.release(attachment)
    }

    pub(super) fn reconcile_retained_attachments(
        &self,
        retained_attachments: u32,
    ) -> Result<(), ShellTerminalError> {
        self.budget
            .reconcile_retained_attachments(retained_attachments)
    }

    pub(super) fn retire_proven_stale(
        &self,
        stale_attachments: &[Attachment],
        remote_attachments: u32,
    ) -> Result<(), ShellTerminalError> {
        self.budget
            .retire_proven_stale(stale_attachments, remote_attachments)
    }
}

/// A successful attachment response with redacted terminal replay bytes.
pub struct AttachReceipt {
    generation: u64,
    replay: RingReplay,
    attachment: Attachment,
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

    /// Return the opaque handle that releases this attachment on disconnect.
    pub fn attachment(&self) -> Attachment {
        self.attachment.clone()
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
    identity: SupervisorIdentity,
    ring: OutputRing,
    attachment_authority: PoolAttachmentAuthority,
    authority: Arc<SessionAuthority>,
    attached: BTreeSet<Attachment>,
    consumed_capabilities: BTreeSet<u64>,
}

impl SessionSupervisor {
    /// Construct a supervisor from process-adapter identity evidence.
    pub(super) fn new(
        session: ShellSession,
        identity: SupervisorIdentity,
        attachment_authority: PoolAttachmentAuthority,
        authority: Arc<SessionAuthority>,
    ) -> Self {
        let ring = OutputRing::new(session.output_ring_capacity() as usize)
            .expect("a validated session has a valid output ring capacity");
        Self {
            session,
            identity,
            ring,
            attachment_authority,
            authority,
            attached: BTreeSet::new(),
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
        let authority = Arc::clone(&self.authority);
        let session = self.session.clone();
        let generation = self.identity.generation();
        authority.with_current(&session, generation, || {
            if request.expected_generation != generation {
                return Err(ShellTerminalError::StaleSessionGeneration);
            }
            let attachment = self.reserve_attachment()?;
            Ok(AttachReceipt {
                generation,
                replay: self.ring.tail(request.tail_bytes as usize),
                attachment,
            })
        })
    }

    /// Consume a one-shot capability after rechecking the current request authority.
    pub fn attach_with_capability(
        &mut self,
        subject: &Subject,
        capability: SessionCapability,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        self.authorize(subject)?;
        let authority = Arc::clone(&self.authority);
        let session = self.session.clone();
        let generation = self.identity.generation();
        authority.with_current(&session, generation, || {
            if capability.generation != generation {
                return Err(ShellTerminalError::StaleSessionGeneration);
            }
            if capability.session_name != self.session.name() {
                return Err(ShellTerminalError::CapabilitySessionMismatch);
            }
            if !self.consumed_capabilities.insert(capability.id) {
                return Err(ShellTerminalError::CapabilityReused);
            }
            let attachment = self.reserve_attachment()?;
            Ok(AttachReceipt {
                generation,
                replay: self.ring.tail(0),
                attachment,
            })
        })
    }

    /// Release an authenticated named-terminal attachment after stream closure.
    pub fn detach(
        &mut self,
        subject: &Subject,
        attachment: Attachment,
    ) -> Result<(), ShellTerminalError> {
        self.authorize(subject)?;
        if attachment.generation != self.identity.generation()
            || attachment.session_name != self.session.name()
            || !self.attached.contains(&attachment)
        {
            return Err(ShellTerminalError::AttachmentUnknown);
        }
        self.attachment_authority.release(&attachment)?;
        self.attached.remove(&attachment);
        Ok(())
    }

    /// Append bytes emitted by this supervisor-owned PTY to its bounded replay ring.
    pub fn record_pty_output(&mut self, bytes: &[u8]) {
        let authority = Arc::clone(&self.authority);
        let session = self.session.clone();
        let generation = self.identity.generation();
        let _ = authority.with_current(&session, generation, || {
            self.ring.append(bytes);
            Ok(())
        });
    }

    fn authorize(&self, subject: &Subject) -> Result<(), ShellTerminalError> {
        Authorizer::authorize_target(
            subject,
            self.session.zone(),
            self.session.execution_target(),
        )
    }

    fn reserve_attachment(&mut self) -> Result<Attachment, ShellTerminalError> {
        let attachment = self
            .attachment_authority
            .reserve(self.session.name(), self.identity.generation())?;
        self.attached.insert(attachment.clone());
        Ok(attachment)
    }
}

impl std::fmt::Debug for SessionSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionSupervisor")
            .field("generation", &self.identity.generation())
            .field("attached", &self.attached.len())
            .field("ring", &self.ring)
            .finish()
    }
}
