//! Per-session terminal supervisor contracts.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
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

    fn admits_session(&self, session: &ShellSession) -> bool {
        self.name == session.pool_name()
            && self.zone == session.zone()
            && self.execution_target == *session.execution_target()
            && self.workload_user == session.workload_user()
            && self.login_shell_ref == session.login_shell_ref()
            && session.output_ring_capacity() <= self.output_ring_capacity
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
    /// Construct a capability from a daemon authority response.
    ///
    /// Callers must not mint capabilities locally. Production implementations
    /// of [`ShellAuthorityPort`] use this only to decode an authenticated
    /// authority response.
    pub fn from_authority(id: u64, generation: u64, session_name: impl Into<String>) -> Self {
        Self {
            id,
            generation,
            session_name: session_name.into(),
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

impl Attachment {
    /// Construct an attachment from a daemon authority response.
    pub fn from_authority(
        id: u64,
        session_name: impl Into<String>,
        generation: u64,
    ) -> Self {
        Self {
            id,
            session_name: session_name.into(),
            generation,
        }
    }
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Attachment(<redacted>)")
    }
}

/// The daemon authority's atomic session-generation and capability grant.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionGrant {
    generation: u64,
    capability: SessionCapability,
}

impl SessionGrant {
    /// Construct a grant from an authority response.
    pub fn from_authority(generation: u64, capability: SessionCapability) -> Self {
        Self {
            generation,
            capability,
        }
    }

    /// Return the authority-selected generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Return the one-shot capability in this grant.
    pub fn capability(&self) -> SessionCapability {
        self.capability.clone()
    }
}

impl std::fmt::Debug for SessionGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionGrant")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

struct SessionEntry {
    fingerprint: SessionFingerprint,
    generation: u64,
    capabilities: BTreeSet<u64>,
    supervisor_identity: Option<SupervisorIdentity>,
}

struct PoolEntry {
    fingerprint: PoolFingerprint,
    retained_attachments: usize,
    entries: BTreeSet<Attachment>,
    next_attachment: u64,
}

#[derive(Default)]
struct AuthorityState {
    pools: BTreeMap<String, PoolEntry>,
    sessions: BTreeMap<String, SessionEntry>,
    next_capability: u64,
}

/// Daemon-owned authority operations required by controller and supervisor processes.
///
/// Production implementations must forward every operation to one durable
/// daemon authority owner. The `Arc` held by provider objects is only a
/// transport client; it must not contain the authoritative session generation,
/// capability census, or attachment quota state.
pub trait ShellAuthorityPort: Send + Sync {
    /// Reconcile one pool's authoritative attachment census.
    fn restore_pool(
        &self,
        pool: &ShellPool,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError>;

    /// Atomically create one exact session authority and mint its grant.
    fn open_session(&self, session: &ShellSession) -> Result<SessionGrant, ShellTerminalError>;

    /// Verify that a recovered session remains the exact authoritative incumbent.
    fn verify_recovery(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<bool, ShellTerminalError>;

    /// Advance a session after its prior supervisor retires and mint its grant.
    fn advance_session(
        &self,
        session: &ShellSession,
        retired_identity: Option<&SupervisorIdentity>,
    ) -> Result<SessionGrant, ShellTerminalError>;

    /// Bind one verified supervisor identity to the current session generation.
    fn claim_supervisor(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<(), ShellTerminalError>;

    /// Verify one supervisor request against the current session generation.
    fn validate_session(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<(), ShellTerminalError>;

    /// Consume a one-shot capability for one exact supervisor identity.
    fn consume_capability(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
        capability: &SessionCapability,
    ) -> Result<(), ShellTerminalError>;

    /// Reserve one pool-wide attachment slot.
    fn reserve_attachment(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<Attachment, ShellTerminalError>;

    /// Release one exact attachment after its owning session's named stream closes.
    fn release_attachment(
        &self,
        session: &ShellSession,
        attachment: &Attachment,
    ) -> Result<(), ShellTerminalError>;

    /// Reconcile a pool's observed attachment total without discarding live handles.
    fn reconcile_pool_attachments(
        &self,
        pool: &ShellPool,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError>;

    /// Retire only handles proved stale by the daemon's authoritative stream census.
    fn retire_proven_stale(
        &self,
        pool: &ShellPool,
        stale_attachments: &[Attachment],
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError>;

    /// Retire one exact session after its supervisor is stopped and streams close.
    fn finalize_session(
        &self,
        session: &ShellSession,
        identity: Option<&SupervisorIdentity>,
    ) -> Result<(), ShellTerminalError>;
}

/// Hermetic daemon-authority model for tests and integration fakes.
///
/// This type deliberately models a single daemon owner. Production callers
/// must implement [`ShellAuthorityPort`] with the daemon's durable authority
/// service and construct one client in each controller or supervisor process.
#[derive(Default)]
pub struct InMemoryShellAuthority {
    state: Mutex<AuthorityState>,
}

impl InMemoryShellAuthority {
    /// Construct an empty test authority owner.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AuthorityState>, ShellTerminalError> {
        self.state
            .lock()
            .map_err(|_| ShellTerminalError::CapacityExceeded)
    }

    fn pool_mut<'a>(
        state: &'a mut AuthorityState,
        pool: &ShellPool,
    ) -> Result<&'a mut PoolEntry, ShellTerminalError> {
        let entry = state
            .pools
            .get_mut(pool.name())
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        if entry.fingerprint != PoolFingerprint::from_pool(pool) {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        Ok(entry)
    }

    fn session_mut<'a>(
        state: &'a mut AuthorityState,
        session: &ShellSession,
    ) -> Result<&'a mut SessionEntry, ShellTerminalError> {
        let entry = state
            .sessions
            .get_mut(session.name())
            .ok_or(ShellTerminalError::StaleSessionGeneration)?;
        if entry.fingerprint != SessionFingerprint::from_session(session) {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        Ok(entry)
    }

    fn reconcile_pool(
        entry: &mut PoolEntry,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        let attached_streams = attached_streams as usize;
        let capacity = entry.fingerprint.max_attached as usize;
        if attached_streams > capacity || attached_streams < entry.entries.len() {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        entry.retained_attachments = attached_streams - entry.entries.len();
        Ok(())
    }
}

impl std::fmt::Debug for InMemoryShellAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InMemoryShellAuthority(<test-only>)")
    }
}

impl ShellAuthorityPort for InMemoryShellAuthority {
    fn restore_pool(
        &self,
        pool: &ShellPool,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        let mut state = self.lock()?;
        if let Some(entry) = state.pools.get_mut(pool.name()) {
            if entry.fingerprint != PoolFingerprint::from_pool(pool) {
                return Err(ShellTerminalError::CapacityExceeded);
            }
            return Self::reconcile_pool(entry, attached_streams);
        }
        let capacity = pool.spec().max_attached() as usize;
        if attached_streams as usize > capacity {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        state.pools.insert(
            pool.name().to_owned(),
            PoolEntry {
                fingerprint: PoolFingerprint::from_pool(pool),
                retained_attachments: attached_streams as usize,
                entries: BTreeSet::new(),
                next_attachment: 0,
            },
        );
        Ok(())
    }

    fn open_session(&self, session: &ShellSession) -> Result<SessionGrant, ShellTerminalError> {
        let mut state = self.lock()?;
        let pool = state
            .pools
            .get(session.pool_name())
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        let session_count = state
            .sessions
            .values()
            .filter(|entry| entry.fingerprint.pool_name == session.pool_name())
            .count();
        if !pool.fingerprint.admits_session(session)
            || session_count >= pool.fingerprint.max_sessions as usize
            || state.sessions.contains_key(session.name())
        {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        let capability_id = state
            .next_capability
            .checked_add(1)
            .ok_or(ShellTerminalError::CapabilityReused)?;
        state.next_capability = capability_id;
        state.sessions.insert(
            session.name().to_owned(),
            SessionEntry {
                fingerprint: SessionFingerprint::from_session(session),
                generation: 1,
                capabilities: BTreeSet::from([capability_id]),
                supervisor_identity: None,
            },
        );
        Ok(SessionGrant::from_authority(
            1,
            SessionCapability::from_authority(
                capability_id,
                1,
                session.name().to_owned(),
            ),
        ))
    }

    fn verify_recovery(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<bool, ShellTerminalError> {
        let state = self.lock()?;
        let Some(entry) = state.sessions.get(session.name()) else {
            return Ok(false);
        };
        Ok(
            entry.fingerprint == SessionFingerprint::from_session(session)
                && entry.generation == identity.generation()
                && entry.supervisor_identity.as_ref() == Some(identity),
        )
    }

    fn advance_session(
        &self,
        session: &ShellSession,
        retired_identity: Option<&SupervisorIdentity>,
    ) -> Result<SessionGrant, ShellTerminalError> {
        let mut state = self.lock()?;
        let current_identity = Self::session_mut(&mut state, session)?
            .supervisor_identity
            .clone();
        if current_identity.as_ref() != retired_identity {
            return Err(ShellTerminalError::SupervisorAmbiguous);
        }
        let capability_id = state
            .next_capability
            .checked_add(1)
            .ok_or(ShellTerminalError::CapabilityReused)?;
        state.next_capability = capability_id;
        let entry = Self::session_mut(&mut state, session)?;
        entry.generation = entry
            .generation
            .checked_add(1)
            .ok_or(ShellTerminalError::StaleSessionGeneration)?;
        entry.capabilities.clear();
        entry.supervisor_identity = None;
        let generation = entry.generation;
        entry.capabilities.insert(capability_id);
        Ok(SessionGrant::from_authority(
            generation,
            SessionCapability::from_authority(
                capability_id,
                generation,
                session.name().to_owned(),
            ),
        ))
    }

    fn claim_supervisor(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<(), ShellTerminalError> {
        let mut state = self.lock()?;
        let entry = Self::session_mut(&mut state, session)?;
        if entry.generation != identity.generation() || entry.supervisor_identity.is_some() {
            return Err(ShellTerminalError::SupervisorAmbiguous);
        }
        entry.supervisor_identity = Some(identity.clone());
        Ok(())
    }

    fn validate_session(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<(), ShellTerminalError> {
        let mut state = self.lock()?;
        let entry = Self::session_mut(&mut state, session)?;
        if entry.generation != identity.generation()
            || entry.supervisor_identity.as_ref() != Some(identity)
        {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        Ok(())
    }

    fn consume_capability(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
        capability: &SessionCapability,
    ) -> Result<(), ShellTerminalError> {
        if capability.generation != identity.generation() {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        if capability.session_name != session.name() {
            return Err(ShellTerminalError::CapabilitySessionMismatch);
        }
        let mut state = self.lock()?;
        let entry = Self::session_mut(&mut state, session)?;
        if entry.generation != identity.generation()
            || entry.supervisor_identity.as_ref() != Some(identity)
        {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        if !entry.capabilities.remove(&capability.id) {
            return Err(ShellTerminalError::CapabilityReused);
        }
        Ok(())
    }

    fn reserve_attachment(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<Attachment, ShellTerminalError> {
        let mut state = self.lock()?;
        let session_entry = Self::session_mut(&mut state, session)?;
        if session_entry.generation != identity.generation()
            || session_entry.supervisor_identity.as_ref() != Some(identity)
        {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        let pool = state
            .pools
            .get_mut(session.pool_name())
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        if pool.fingerprint.name != session.pool_name()
            || pool.retained_attachments.saturating_add(pool.entries.len())
                >= pool.fingerprint.max_attached as usize
        {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        pool.next_attachment = pool
            .next_attachment
            .checked_add(1)
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        let attachment = Attachment {
            id: pool.next_attachment,
            session_name: session.name().to_owned(),
            generation: identity.generation(),
        };
        pool.entries.insert(attachment.clone());
        Ok(attachment)
    }

    fn release_attachment(
        &self,
        session: &ShellSession,
        attachment: &Attachment,
    ) -> Result<(), ShellTerminalError> {
        if attachment.session_name != session.name() {
            return Err(ShellTerminalError::AttachmentUnknown);
        }
        let mut state = self.lock()?;
        let pool = state
            .pools
            .get_mut(session.pool_name())
            .ok_or(ShellTerminalError::AttachmentUnknown)?;
        if pool.entries.remove(attachment) {
            Ok(())
        } else {
            Err(ShellTerminalError::AttachmentUnknown)
        }
    }

    fn reconcile_pool_attachments(
        &self,
        pool: &ShellPool,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        let mut state = self.lock()?;
        Self::reconcile_pool(Self::pool_mut(&mut state, pool)?, attached_streams)
    }

    fn retire_proven_stale(
        &self,
        pool: &ShellPool,
        stale_attachments: &[Attachment],
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        let mut state = self.lock()?;
        let pool = Self::pool_mut(&mut state, pool)?;
        let distinct_stale: BTreeSet<_> = stale_attachments
            .iter()
            .filter(|attachment| pool.entries.contains(*attachment))
            .collect();
        let remaining_entries = pool.entries.len().saturating_sub(distinct_stale.len());
        let attached_streams = attached_streams as usize;
        if attached_streams > pool.fingerprint.max_attached as usize
            || attached_streams < remaining_entries
        {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        for attachment in distinct_stale {
            pool.entries.remove(attachment);
        }
        pool.retained_attachments = attached_streams - remaining_entries;
        Ok(())
    }

    fn finalize_session(
        &self,
        session: &ShellSession,
        identity: Option<&SupervisorIdentity>,
    ) -> Result<(), ShellTerminalError> {
        let mut state = self.lock()?;
        let entry = state
            .sessions
            .get(session.name())
            .ok_or(ShellTerminalError::SupervisorAmbiguous)?;
        if entry.fingerprint != SessionFingerprint::from_session(session)
            || entry.supervisor_identity.as_ref() != identity
        {
            return Err(ShellTerminalError::SupervisorAmbiguous);
        }
        let pool = state
            .pools
            .get(session.pool_name())
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        if pool
            .entries
            .iter()
            .any(|attachment| attachment.session_name == session.name())
        {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        state.sessions.remove(session.name());
        Ok(())
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
    authority: std::sync::Arc<dyn ShellAuthorityPort>,
}

impl SessionSupervisor {
    /// Construct a supervisor from process-adapter identity evidence.
    pub(super) fn new(
        session: ShellSession,
        identity: SupervisorIdentity,
        authority: std::sync::Arc<dyn ShellAuthorityPort>,
    ) -> Self {
        let ring = OutputRing::new(session.output_ring_capacity() as usize)
            .expect("a validated session has a valid output ring capacity");
        Self {
            session,
            identity,
            ring,
            authority,
        }
    }

    /// Authorize and attach a direct named terminal stream.
    pub fn attach(
        &mut self,
        subject: &Subject,
        request: AttachRequest,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        self.authorize(subject)?;
        let generation = self.identity.generation();
        if request.expected_generation != generation {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        let attachment = self
            .authority
            .reserve_attachment(&self.session, &self.identity)?;
        Ok(AttachReceipt {
            generation,
            replay: self.ring.tail(request.tail_bytes as usize),
            attachment,
        })
    }

    /// Consume a one-shot capability after rechecking the current request authority.
    pub fn attach_with_capability(
        &mut self,
        subject: &Subject,
        capability: SessionCapability,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        self.authorize(subject)?;
        let generation = self.identity.generation();
        self.authority
            .consume_capability(&self.session, &self.identity, &capability)?;
        let attachment = self
            .authority
            .reserve_attachment(&self.session, &self.identity)?;
        Ok(AttachReceipt {
            generation,
            replay: self.ring.tail(0),
            attachment,
        })
    }

    /// Release an authenticated named-terminal attachment after stream closure.
    pub fn detach(
        &mut self,
        subject: &Subject,
        attachment: Attachment,
    ) -> Result<(), ShellTerminalError> {
        self.authorize(subject)?;
        self.authority
            .release_attachment(&self.session, &attachment)
    }

    /// Append bytes emitted by this supervisor-owned PTY to its bounded replay ring.
    pub fn record_pty_output(&mut self, bytes: &[u8]) {
        if self
            .authority
            .validate_session(&self.session, &self.identity)
            .is_ok()
        {
            self.ring.append(bytes);
        }
    }

    fn authorize(&self, subject: &Subject) -> Result<(), ShellTerminalError> {
        Authorizer::authorize_target(
            subject,
            self.session.zone(),
            self.session.execution_target(),
        )
    }
}

impl std::fmt::Debug for SessionSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionSupervisor")
            .field("generation", &self.identity.generation())
            .field("ring", &self.ring)
            .finish()
    }
}
