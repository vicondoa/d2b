//! Per-session terminal supervisor contracts.

use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::Mutex;

use d2b_contracts_resource::v3::{
    ResourceRef,
    execution_policy::{BoundedToken, ExecutionDomain},
    identity::ReconnectGeneration,
    process::{ExecutionSpec, ProcessClass, ProcessSpec},
};

use crate::{
    Authorizer, ExecutionTarget, ShellPool, ShellSession, ShellTerminalError, Subject,
    session::{OutputRing, RingReplay, SupervisorIdentity},
};
use tracing::{debug, warn};

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
    ///
    /// # Errors
    ///
    /// Returns [`ShellTerminalError::CapacityOutOfRange`] when the tail
    /// byte budget exceeds the ring ceiling.
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

/// The admitted `EndpointBinding` relationship one terminal stream rides.
///
/// The binding is what the graph committed for a session's interactive
/// stream: the exact `Process` that consumes it, the exact `Endpoint` it is
/// attached to, and the reconnect generation below which the admission no
/// longer speaks. Nothing beside the binding may widen it - the endpoint is
/// named here rather than beside the binding, so a stream cannot be granted
/// a host directory because its socket happens to live in one (R23).
#[derive(Clone, PartialEq, Eq)]
pub struct TerminalStreamBinding {
    consumer: ResourceRef,
    endpoint: ResourceRef,
    minimum_reconnect: ReconnectGeneration,
}

impl TerminalStreamBinding {
    /// Construct the admitted relationship for one session's stream.
    ///
    /// The consumer is the session's own supervisor `Process`; the endpoint
    /// is the one exact stream endpoint the graph bound to it.
    pub const fn admitted(
        consumer: ResourceRef,
        endpoint: ResourceRef,
        minimum_reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            consumer,
            endpoint,
            minimum_reconnect,
        }
    }

    /// Borrow the `Process` row that consumes the stream.
    pub const fn consumer(&self) -> &ResourceRef {
        &self.consumer
    }

    /// Borrow the exact admitted stream endpoint.
    pub const fn endpoint(&self) -> &ResourceRef {
        &self.endpoint
    }

    /// Return the lowest reconnect generation this admission still accepts.
    pub const fn minimum_reconnect(&self) -> ReconnectGeneration {
        self.minimum_reconnect
    }

    /// Measure one incoming observation against the admitted relationship.
    ///
    /// # Errors
    ///
    /// Returns [`ShellTerminalError::EndpointBindingMismatch`] when the
    /// observation names a different consumer or endpoint, and
    /// [`ShellTerminalError::StaleReconnect`] when it was made in a
    /// reconnect generation this relationship no longer admits.
    pub fn admit(
        &self,
        evidence: &TerminalAttachEvidence,
    ) -> Result<(), ShellTerminalError> {
        if evidence.consumer() != &self.consumer || evidence.endpoint() != &self.endpoint {
            return Err(ShellTerminalError::EndpointBindingMismatch);
        }
        if evidence.reconnect() < self.minimum_reconnect {
            return Err(ShellTerminalError::StaleReconnect);
        }
        Ok(())
    }
}

impl std::fmt::Debug for TerminalStreamBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TerminalStreamBinding(<redacted>)")
    }
}

/// The live, non-secret observations one incoming terminal stream presents.
///
/// Evidence is measured against the admitted binding and never merges with
/// it: a connecting stream cannot move the fence by presenting evidence, and
/// the fence cannot adopt a stream's claims without the graph having
/// committed them.
#[derive(Clone, PartialEq, Eq)]
pub struct TerminalAttachEvidence {
    consumer: ResourceRef,
    endpoint: ResourceRef,
    reconnect: ReconnectGeneration,
}

impl TerminalAttachEvidence {
    /// Construct the evidence one attach attempt presents.
    pub const fn presented(
        consumer: ResourceRef,
        endpoint: ResourceRef,
        reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            consumer,
            endpoint,
            reconnect,
        }
    }

    /// Borrow the `Process` row the stream claims to ride.
    pub const fn consumer(&self) -> &ResourceRef {
        &self.consumer
    }

    /// Borrow the stream endpoint the stream claims to be attached to.
    pub const fn endpoint(&self) -> &ResourceRef {
        &self.endpoint
    }

    /// Return the reconnect generation this attempt is.
    pub const fn reconnect(&self) -> ReconnectGeneration {
        self.reconnect
    }
}

impl std::fmt::Debug for TerminalAttachEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TerminalAttachEvidence(<redacted>)")
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
    pub fn from_authority(id: u64, session_name: impl Into<String>, generation: u64) -> Self {
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


/// The Process provider that realizes every shell supervisor.
///
/// This is provider contract data, not per-session configuration: the shell
/// Provider names one Process implementation and the graph admits which
/// backend realizes it on the selected target. No session, pool, or caller
/// chooses it.
pub const SUPERVISOR_PROCESS_PROVIDER_REF: &str = "Provider/system-systemd";

/// The trusted executable template the shell Provider's supervisor runs from.
///
/// The template is the provider's own declared executable (R31); it is a
/// declared method's target, not an argv a caller composes.
pub const SUPERVISOR_PROCESS_TEMPLATE: &str = "shell-supervisor-main";

/// The execution fields one shell supervisor runs under.
///
/// The execution target and workload user are the exact admitted graph
/// references; the template is the provider's declared executable. There is
/// no argument through which a caller could place a different identity,
/// executable, or argv, so a user-domain supervisor can only ever run as the
/// `User` its session was admitted for.
///
/// # Errors
///
/// Propagates the typed execution-spec refusal when the supplied references
/// cannot describe a user-domain service.
pub fn supervisor_execution_spec(
    execution_ref: ResourceRef,
    user_ref: ResourceRef,
) -> Result<ExecutionSpec, ShellTerminalError> {
    ExecutionSpec::new(
        execution_ref,
        Some(ExecutionDomain::User),
        Some(user_ref),
        ProcessClass::Service,
        BoundedToken::parse(SUPERVISOR_PROCESS_TEMPLATE)
            .map_err(|_| ShellTerminalError::SupervisorAmbiguous)?,
        None,
        Vec::new(),
        Vec::new(),
        Default::default(),
        Default::default(),
        None,
        Vec::new(),
        Default::default(),
    )
    .map_err(|_| ShellTerminalError::SupervisorAmbiguous)
}

/// The authenticated `User` identity a supervisor launch actually ran under.
///
/// The process adapter proves this from the launched unit, so it is evidence
/// about the running process rather than a claim carried beside the request.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkloadIdentity {
    user_ref: ResourceRef,
}

impl WorkloadIdentity {
    /// Construct the proven workload identity from the observed `User` row.
    pub const fn proven(user_ref: ResourceRef) -> Self {
        Self { user_ref }
    }

    /// Borrow the `User` the supervisor was proved to run as.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }
}

impl std::fmt::Debug for WorkloadIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkloadIdentity(<redacted>)")
    }
}
/// The target-local Process resource created for one ShellSession supervisor.
#[derive(Clone, PartialEq, Eq)]
pub struct SupervisorProcessResource {
    resource_ref: ResourceRef,
    owner_ref: ResourceRef,
    spec: ProcessSpec,
}

impl SupervisorProcessResource {
    /// Build the target-local Process intent owned by one ShellSession.
    pub fn for_session(session: &ShellSession) -> Self {
        let resource_ref = session.supervisor_process_ref().clone();
        let owner_ref =
            ResourceRef::parse(&format!("{}/{}", session.resource_type(), session.name()))
                .expect("validated ShellSession resource reference");
        let execution = supervisor_execution_spec(
            session.supervisor_execution_ref().clone(),
            session.supervisor_user_ref().clone(),
        )
        .expect("validated supervisor Process execution spec");
        Self {
            resource_ref,
            owner_ref,
            spec: ProcessSpec::minimal(execution),
        }
    }

    /// Build the Process intent only for the exact admitted workload user.
    ///
    /// # Errors
    ///
    /// Returns [`ShellTerminalError::WorkloadIdentityMismatch`] when the
    /// identity the launch actually ran under is not the `User` this
    /// session's supervisor was admitted for. A user-domain supervisor is
    /// never selectable onto another user's identity, and the refusal lands
    /// before any row is realized rather than after it has launched.
    pub fn admitted_for_session(
        session: &ShellSession,
        workload: &WorkloadIdentity,
    ) -> Result<Self, ShellTerminalError> {
        if workload.user_ref() != session.supervisor_user_ref() {
            return Err(ShellTerminalError::WorkloadIdentityMismatch);
        }
        Ok(Self::for_session(session))
    }

    /// Borrow the target-local Process identity.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the ShellSession owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the Process resource spec.
    pub const fn spec(&self) -> &ProcessSpec {
        &self.spec
    }
}

impl SupervisorProcessResource {
    fn from_session(session: &ShellSession) -> Self {
        Self::for_session(session)
    }
}

impl std::fmt::Debug for SupervisorProcessResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SupervisorProcessResource")
            .field("resource_ref", &"<redacted>")
            .field("owner_ref", &"<redacted>")
            .finish_non_exhaustive()
    }
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

    /// Create or adopt the session's target-local supervisor Process child.
    ///
    /// This is a resource-intent operation only. The controller never gets a
    /// process handle and never spawns the child itself.
    fn ensure_supervisor_process(&self, session: &ShellSession) -> Result<(), ShellTerminalError>;

    /// Delete the session's supervisor Process child after its streams close.
    fn remove_supervisor_process(&self, session: &ShellSession) -> Result<(), ShellTerminalError>;

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

    /// Consume a one-shot capability and reserve an attachment in one admission.
    ///
    /// A retryable capacity refusal must not consume the capability.
    fn admit_capability_attachment(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
        capability: &SessionCapability,
    ) -> Result<Attachment, ShellTerminalError>;

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

/// Daemon-owned session-generation, capability, and attachment ledger.
///
/// The ledger deliberately contains no resource-store or process authority.
/// A production daemon composes it with its authenticated Resource API adapter,
/// while the public [`InMemoryShellAuthority`] wrapper uses it for hermetic
/// tests.
#[derive(Default)]
pub struct ShellAuthorityLedger {
    state: Mutex<AuthorityState>,
}

impl ShellAuthorityLedger {
    /// Construct an empty daemon authority ledger.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<tokio::sync::MutexGuard<'_, AuthorityState>, ShellTerminalError> {
        // The `ShellAuthorityPort` contract is synchronous, so this sync
        // surface uses `try_lock` fail-closed per U4: a briefly contended
        // ledger reports capacity-exceeded rather than parking the caller.
        self.state.try_lock().map_err(|_| {
            warn!(
                provider = "shell-terminal",
                "authority state lock poisoned; reporting capacity-exceeded"
            );
            ShellTerminalError::CapacityExceeded
        })
    }

    /// Validate that a Provider-reconstructed session still matches the
    /// authority ledger without touching its test-only Process map.
    ///
    /// # Errors
    ///
    /// Returns [`ShellTerminalError::SupervisorAmbiguous`] when the
    /// session is not projected and
    /// [`ShellTerminalError::StaleSessionGeneration`] when the fingerprint
    /// no longer matches.
    pub fn validate_session(&self, session: &ShellSession) -> Result<(), ShellTerminalError> {
        let state = self.lock()?;
        let Some(entry) = state.sessions.get(session.name()) else {
            return Err(ShellTerminalError::SupervisorAmbiguous);
        };
        if entry.fingerprint != SessionFingerprint::from_session(session) {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        Ok(())
    }

    /// Return whether any authenticated attachment still owns this session.
    pub fn has_active_attachments(
        &self,
        session: &ShellSession,
    ) -> Result<bool, ShellTerminalError> {
        let state = self.lock()?;
        let Some(entry) = state.sessions.get(session.name()) else {
            return Err(ShellTerminalError::SupervisorAmbiguous);
        };
        if entry.fingerprint != SessionFingerprint::from_session(session) {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        Ok(state.pools.get(session.pool_name()).is_some_and(|pool| {
            pool.entries
                .iter()
                .any(|attachment| attachment.session_name == session.name())
        }))
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

impl std::fmt::Debug for ShellAuthorityLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ShellAuthorityLedger(<daemon-owned>)")
    }
}

impl ShellAuthorityPort for ShellAuthorityLedger {
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
            SessionCapability::from_authority(capability_id, 1, session.name().to_owned()),
        ))
    }

    fn ensure_supervisor_process(&self, session: &ShellSession) -> Result<(), ShellTerminalError> {
        self.validate_session(session)
    }

    fn remove_supervisor_process(&self, session: &ShellSession) -> Result<(), ShellTerminalError> {
        self.validate_session(session)
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
        {
            let current_identity = Self::session_mut(&mut state, session)?
                .supervisor_identity
                .as_ref();
            if current_identity != retired_identity {
                return Err(ShellTerminalError::SupervisorAmbiguous);
            }
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
            SessionCapability::from_authority(capability_id, generation, session.name().to_owned()),
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

    fn admit_capability_attachment(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
        capability: &SessionCapability,
    ) -> Result<Attachment, ShellTerminalError> {
        if capability.generation != identity.generation() {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        if capability.session_name != session.name() {
            return Err(ShellTerminalError::CapabilitySessionMismatch);
        }
        let mut state = self.lock()?;
        {
            let entry = Self::session_mut(&mut state, session)?;
            if entry.generation != identity.generation()
                || entry.supervisor_identity.as_ref() != Some(identity)
            {
                return Err(ShellTerminalError::StaleSessionGeneration);
            }
            if !entry.capabilities.contains(&capability.id) {
                return Err(ShellTerminalError::CapabilityReused);
            }
        }
        let attachment = {
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
            attachment
        };
        if !Self::session_mut(&mut state, session)?
            .capabilities
            .remove(&capability.id)
        {
            if let Some(pool) = state.pools.get_mut(session.pool_name()) {
                pool.entries.remove(&attachment);
            }
            return Err(ShellTerminalError::CapabilityReused);
        }
        Ok(attachment)
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

/// Hermetic authority wrapper for provider unit and integration tests.
///
/// Production composition uses [`ShellAuthorityLedger`] directly and supplies
/// the Resource API process adapter at the daemon boundary.
#[derive(Default)]
pub struct InMemoryShellAuthority {
    ledger: ShellAuthorityLedger,
    supervisor_processes: Mutex<BTreeMap<String, SupervisorProcessResource>>,
    user_processes: Mutex<BTreeMap<String, UserDomainProcess>>,
}

impl InMemoryShellAuthority {
    /// Declare a `Process` row this Provider does not own.
    ///
    /// A real user domain holds far more than this Provider's supervisors.
    /// Placing those rows in the census is what makes a removal's scope
    /// observable: a teardown that matched on the workload user rather than
    /// on the owning session would delete them, and the provider's own
    /// removal scenario fails.
    pub fn declare_user_process(&self, process: UserDomainProcess) {
        if let Ok(mut processes) = self.user_processes.try_lock() {
            processes.insert(process.resource_ref().to_canonical_string(), process);
        }
    }

    /// Return one row of the user-domain census.
    pub fn user_process(&self, resource_ref: &str) -> Option<UserDomainProcess> {
        self.user_processes
            .try_lock()
            .ok()
            .and_then(|processes| processes.get(resource_ref).cloned())
    }

    /// Return the `Process` rows still running for one workload user.
    pub fn user_process_names_for(&self, user_ref: &ResourceRef) -> Vec<String> {
        self.user_processes
            .try_lock()
            .map(|processes| {
                processes
                    .values()
                    .filter(|process| process.user_ref() == user_ref)
                    .map(|process| process.resource_ref().to_canonical_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// One `Process` row of the user-domain census a removal path must respect.
///
/// The census exists so a removal can be observed to be scoped rather than
/// broad: a shell session owns exactly one row, and every other row - another
/// session of the same pool, a workload process of the same `User`, or a
/// process of a different user entirely - belongs to somebody else and stays.
#[derive(Clone, PartialEq, Eq)]
pub struct UserDomainProcess {
    resource_ref: ResourceRef,
    user_ref: ResourceRef,
    owned_by: Option<String>,
}

impl UserDomainProcess {
    /// Declare a `Process` row the shell Provider does not own.
    pub fn unowned(resource_ref: ResourceRef, user_ref: ResourceRef) -> Self {
        Self {
            resource_ref,
            user_ref,
            owned_by: None,
        }
    }

    /// Borrow the `Process` row identity.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the `User` the row runs as.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }

    /// Borrow the owning session name, when this row is a shell supervisor.
    pub fn owned_by(&self) -> Option<&str> {
        self.owned_by.as_deref()
    }

    fn for_session(session: &ShellSession) -> Self {
        Self {
            resource_ref: session.supervisor_process_ref().clone(),
            user_ref: session.supervisor_user_ref().clone(),
            owned_by: Some(session.name().to_owned()),
        }
    }
}

impl std::fmt::Debug for UserDomainProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserDomainProcess")
            .field("owned_by", &self.owned_by)
            .finish_non_exhaustive()
    }
}

impl InMemoryShellAuthority {
    /// Construct an empty test authority owner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the currently declared supervisor Process resource.
    pub fn supervisor_process_resource(
        &self,
        session_name: &str,
    ) -> Option<SupervisorProcessResource> {
        self.supervisor_processes
            .try_lock()
            .ok()
            .and_then(|processes| processes.get(session_name).cloned())
    }

    fn ensure_supervisor_process_state(
        &self,
        session: &ShellSession,
    ) -> Result<(), ShellTerminalError> {
        self.ledger.validate_session(session)?;
        let value = SupervisorProcessResource::from_session(session);
        let owned = UserDomainProcess::for_session(session);
        let mut processes = self
            .supervisor_processes
            .try_lock()
            .map_err(|_| ShellTerminalError::SupervisorAmbiguous)?;
        if let Some(existing) = processes.get(session.name()) {
            if existing != &value {
                return Err(ShellTerminalError::SupervisorAmbiguous);
            }
        } else {
            processes.insert(session.name().to_owned(), value);
        }
        drop(processes);
        if let Ok(mut census) = self.user_processes.try_lock() {
            census.insert(owned.resource_ref().to_canonical_string(), owned);
        }
        Ok(())
    }

    fn remove_supervisor_process_state(
        &self,
        session: &ShellSession,
    ) -> Result<(), ShellTerminalError> {
        self.ledger.validate_session(session)?;
        if self.ledger.has_active_attachments(session)? {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        self.supervisor_processes
            .try_lock()
            .map_err(|_| ShellTerminalError::SupervisorAmbiguous)?
            .remove(session.name());
        // Removal is scoped to the one row this session owns. Matching on the
        // workload `User` instead would drain every process that user runs,
        // including rows this Provider never created and rows another session
        // still depends on.
        if let Ok(mut census) = self.user_processes.try_lock() {
            census.remove(&session.supervisor_process_ref().to_canonical_string());
        }
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
        self.ledger.restore_pool(pool, attached_streams)
    }

    fn open_session(&self, session: &ShellSession) -> Result<SessionGrant, ShellTerminalError> {
        self.ledger.open_session(session)
    }

    fn ensure_supervisor_process(&self, session: &ShellSession) -> Result<(), ShellTerminalError> {
        self.ensure_supervisor_process_state(session)
    }

    fn remove_supervisor_process(&self, session: &ShellSession) -> Result<(), ShellTerminalError> {
        self.remove_supervisor_process_state(session)
    }

    fn verify_recovery(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<bool, ShellTerminalError> {
        self.ledger.verify_recovery(session, identity)
    }

    fn advance_session(
        &self,
        session: &ShellSession,
        retired_identity: Option<&SupervisorIdentity>,
    ) -> Result<SessionGrant, ShellTerminalError> {
        self.ledger.advance_session(session, retired_identity)
    }

    fn claim_supervisor(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<(), ShellTerminalError> {
        self.ledger.claim_supervisor(session, identity)
    }

    fn validate_session(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<(), ShellTerminalError> {
        ShellAuthorityPort::validate_session(&self.ledger, session, identity)
    }

    fn consume_capability(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
        capability: &SessionCapability,
    ) -> Result<(), ShellTerminalError> {
        self.ledger
            .consume_capability(session, identity, capability)
    }

    fn admit_capability_attachment(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
        capability: &SessionCapability,
    ) -> Result<Attachment, ShellTerminalError> {
        self.ledger
            .admit_capability_attachment(session, identity, capability)
    }

    fn reserve_attachment(
        &self,
        session: &ShellSession,
        identity: &SupervisorIdentity,
    ) -> Result<Attachment, ShellTerminalError> {
        self.ledger.reserve_attachment(session, identity)
    }

    fn release_attachment(
        &self,
        session: &ShellSession,
        attachment: &Attachment,
    ) -> Result<(), ShellTerminalError> {
        self.ledger.release_attachment(session, attachment)
    }

    fn reconcile_pool_attachments(
        &self,
        pool: &ShellPool,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        self.ledger
            .reconcile_pool_attachments(pool, attached_streams)
    }

    fn retire_proven_stale(
        &self,
        pool: &ShellPool,
        stale_attachments: &[Attachment],
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        self.ledger
            .retire_proven_stale(pool, stale_attachments, attached_streams)
    }

    fn finalize_session(
        &self,
        session: &ShellSession,
        identity: Option<&SupervisorIdentity>,
    ) -> Result<(), ShellTerminalError> {
        self.ledger.finalize_session(session, identity)
    }
}

/// A successful attachment response with redacted terminal replay bytes.
pub struct AttachReceipt {
    generation: u64,
    replay: RingReplay,
    attachment: Attachment,
    stream_name: &'static str,
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

    /// Return the sole authenticated ComponentSession stream name.
    pub const fn stream_name(&self) -> &'static str {
        self.stream_name
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

    /// Authorize and attach a terminal stream measured against the admitted
    /// `EndpointBinding` relationship.
    ///
    /// This is the graph-bound attach: the bounded replay stays a data-plane
    /// payload carried by the receipt, while the control decision is only
    /// "does this observation still speak for the relationship the graph
    /// committed". A stream carrying older evidence, a different consumer
    /// `Process`, or a different endpoint never reaches the attachment
    /// census, so it cannot consume a slot or read another session's ring.
    ///
    /// # Errors
    ///
    /// Returns the authorization refusal, the stale-generation refusal, the
    /// [`TerminalStreamBinding::admit`] refusal, or the authority's refusal.
    pub fn attach_admitted(
        &mut self,
        subject: &Subject,
        request: AttachRequest,
        stream: &TerminalStreamBinding,
        evidence: &TerminalAttachEvidence,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        stream.admit(evidence).inspect_err(|error| {
            warn!(
                provider = "shell-terminal",
                session = self.session.name(),
                error = ?error,
                "attach rejected: observation is outside the admitted terminal binding"
            );
        })?;
        if stream.consumer() != self.session.supervisor_process_ref() {
            warn!(
                provider = "shell-terminal",
                session = self.session.name(),
                "attach rejected: admitted binding belongs to another supervisor Process"
            );
            return Err(ShellTerminalError::EndpointBindingMismatch);
        }
        self.attach(subject, request)
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
            warn!(
                provider = "shell-terminal",
                session = self.session.name(),
                expected = request.expected_generation,
                actual = generation,
                "attach rejected: stale session generation"
            );
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        let attachment = self
            .authority
            .reserve_attachment(&self.session, &self.identity)
            .inspect_err(|error| {
                warn!(
                    provider = "shell-terminal",
                    session = self.session.name(),
                    error = ?error,
                    "authority refused attachment reservation"
                );
            })?;
        Ok(AttachReceipt {
            generation,
            replay: self.ring.tail(request.tail_bytes as usize),
            attachment,
            stream_name: super::TERMINAL_STREAM,
        })
    }

    /// Borrow the Process child resource represented by this supervisor.
    pub const fn process_ref(&self) -> &ResourceRef {
        self.session.supervisor_process_ref()
    }

    /// Consume a one-shot capability after rechecking the current request authority.
    pub fn attach_with_capability(
        &mut self,
        subject: &Subject,
        capability: SessionCapability,
    ) -> Result<AttachReceipt, ShellTerminalError> {
        self.authorize(subject)?;
        let generation = self.identity.generation();
        let attachment = self
            .authority
            .admit_capability_attachment(&self.session, &self.identity, &capability)
            .inspect_err(|error| {
                warn!(
                    provider = "shell-terminal",
                    session = self.session.name(),
                    error = ?error,
                    "authority refused capability attachment"
                );
            })?;
        Ok(AttachReceipt {
            generation,
            replay: self.ring.tail(0),
            attachment,
            stream_name: super::TERMINAL_STREAM,
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
            .inspect_err(|error| {
                warn!(
                    provider = "shell-terminal",
                    session = self.session.name(),
                    error = ?error,
                    "authority refused attachment release"
                );
            })
    }

    /// Append bytes emitted by this supervisor-owned PTY to its bounded replay ring.
    pub fn record_pty_output(&mut self, bytes: &[u8]) {
        if self
            .authority
            .validate_session(&self.session, &self.identity)
            .is_ok()
        {
            self.ring.append(bytes);
        } else {
            debug!(
                provider = "shell-terminal",
                session = self.session.name(),
                "pty output dropped for session whose authority validation failed"
            );
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
