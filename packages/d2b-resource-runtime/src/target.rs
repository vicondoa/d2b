//! Explicit Host vs Guest execution-target layer (R19, spec section 23).
//!
//! Unit U4 established the coarse target handle every driver context exposes;
//! this module adds the directory that turns a declared execution reference
//! (`Host/<name>` or `Guest/<name>`) into a realization handle, records the
//! exact target of every resource the manager holds, and binds guest
//! assignments to the authenticated ComponentSession generation they were
//! minted under.
//!
//! Two rules are load-bearing and are enforced here rather than by callers:
//!
//! - **Target failure is not Zone failure** (R21). Losing a guest session
//!   marks the target unavailable and leaves every desired resource, every
//!   assignment, and every target-local realization in place. A handle minted
//!   under the lost session cannot realize, observe, or delete anything; the
//!   reconnect outcome names the assignments that must be re-adopted, and
//!   [`TargetDirectory::adopt`] is the one operation that re-binds them to the
//!   new session generation (F5).
//! - **ZoneLink is a consumer, never the owner** (R20). A ZoneLink resource
//!   targeting a guest is an ordinary assignment on this path; releasing it
//!   (on deletion) touches no other assignment, no other target-local
//!   realization, and not the guest session itself.

pub const MODULE_NAME: &str = "target";

use std::{collections::BTreeMap, collections::HashMap, fmt, sync::Arc};

use parking_lot::Mutex;

use crate::guest_target::{
    GuestAdoption, GuestRealizeRequest, GuestTargetControl, GuestTargetError, TargetControlAssignment,
    TargetResourceInstance,
};
use crate::identity::ResourceKey;

/// Where a resource's effects execute (spec section 23.1). A Host-zone
/// resource may realize inside a guest while remaining owned and visible in
/// the Host zone (R18); the handle carries only the execution target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetHandle {
    /// Effects run locally on the host.
    Host,
    /// Effects run inside a guest through the authenticated ComponentSession.
    Guest,
}

/// Closed execution-target kind named by a resource's execution reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetKind {
    Host,
    Guest,
}

impl TargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "Host",
            Self::Guest => "Guest",
        }
    }

    pub const fn is_host(self) -> bool {
        matches!(self, Self::Host)
    }

    pub const fn is_guest(self) -> bool {
        matches!(self, Self::Guest)
    }

    const fn handle(self) -> TargetHandle {
        match self {
            Self::Host => TargetHandle::Host,
            Self::Guest => TargetHandle::Guest,
        }
    }
}

impl fmt::Display for TargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Maximum bytes in one execution-target name.
pub const MAX_TARGET_NAME_BYTES: usize = 128;

/// One canonical execution reference: `Host/<name>` or `Guest/<name>`.
///
/// This is the reference a desired spec declares (`spec.executionRef`); it
/// names where effects run and never changes a resource's Zone identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetRef {
    kind: TargetKind,
    name: String,
}

impl TargetRef {
    /// A `Host/<name>` reference.
    pub fn host(name: impl AsRef<str>) -> Result<Self, TargetError> {
        Self::new(TargetKind::Host, name.as_ref())
    }

    /// A `Guest/<name>` reference.
    pub fn guest(name: impl AsRef<str>) -> Result<Self, TargetError> {
        Self::new(TargetKind::Guest, name.as_ref())
    }

    /// Parse one canonical `Host/<name>` or `Guest/<name>` reference.
    pub fn parse(value: &str) -> Result<Self, TargetError> {
        let (kind, name) = value.split_once('/').ok_or(TargetError::InvalidExecutionReference)?;
        let kind = match kind {
            "Host" => TargetKind::Host,
            "Guest" => TargetKind::Guest,
            _ => return Err(TargetError::InvalidExecutionReference),
        };
        Self::new(kind, name)
    }

    fn new(kind: TargetKind, name: &str) -> Result<Self, TargetError> {
        if name.is_empty()
            || name.len() > MAX_TARGET_NAME_BYTES
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(TargetError::InvalidExecutionReference);
        }
        Ok(Self { kind, name: name.to_owned() })
    }

    pub const fn kind(&self) -> TargetKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn is_host(&self) -> bool {
        self.kind.is_host()
    }

    pub const fn is_guest(&self) -> bool {
        self.kind.is_guest()
    }

    /// The canonical `Kind/name` form a spec carries.
    pub fn to_canonical_string(&self) -> String {
        format!("{}/{}", self.kind.as_str(), self.name)
    }
}

impl fmt::Display for TargetRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_canonical_string())
    }
}

/// Host realization handle: local provider effects, no session boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTargetHandle {
    reference: TargetRef,
}

impl HostTargetHandle {
    pub const fn reference(&self) -> &TargetRef {
        &self.reference
    }
}

/// Guest realization handle: one guest, bound to the authenticated
/// ComponentSession generation it was minted under.
///
/// The handle is a value, not a channel: it never carries the session's
/// control capability, so it cannot outlive the generation it names. A handle
/// whose generation is not the live one can only be re-bound by
/// [`TargetDirectory::adopt`], which performs target-local discovery instead
/// of inheriting the previous session's authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestTargetHandle {
    reference: TargetRef,
    session_generation: Option<u64>,
}

impl GuestTargetHandle {
    pub const fn reference(&self) -> &TargetRef {
        &self.reference
    }

    /// The authenticated session generation this assignment is bound to.
    /// `None` while no session has ever been registered for the guest.
    pub const fn session_generation(&self) -> Option<u64> {
        self.session_generation
    }

    pub const fn is_bound(&self) -> bool {
        self.session_generation.is_some()
    }
}

/// What the directory resolved for one resource's declared execution
/// reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget {
    Host(HostTargetHandle),
    Guest(GuestTargetHandle),
}

impl ResolvedTarget {
    pub const fn kind(&self) -> TargetKind {
        match self {
            Self::Host(_) => TargetKind::Host,
            Self::Guest(_) => TargetKind::Guest,
        }
    }

    /// The coarse handle a driver context exposes (U4).
    pub const fn handle(&self) -> TargetHandle {
        self.kind().handle()
    }

    pub const fn host(&self) -> Option<&HostTargetHandle> {
        match self {
            Self::Host(handle) => Some(handle),
            Self::Guest(_) => None,
        }
    }

    pub const fn guest(&self) -> Option<&GuestTargetHandle> {
        match self {
            Self::Guest(handle) => Some(handle),
            Self::Host(_) => None,
        }
    }

    /// The bound guest session generation, when this is a guest target.
    pub const fn session_generation(&self) -> Option<u64> {
        match self {
            Self::Host(_) => None,
            Self::Guest(handle) => handle.session_generation,
        }
    }
}

/// The directory's record binding one Host-zone resource to its exact target
/// (R18: the record lives with the resource's authority, never in the guest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAssignment {
    source: ResourceKey,
    uid: [u8; 16],
    desired_generation: u64,
    target: ResolvedTarget,
}

impl TargetAssignment {
    /// The owning Host-zone resource key.
    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The owning resource's stable uid.
    pub const fn uid(&self) -> &[u8; 16] {
        &self.uid
    }

    /// The desired generation this assignment was recorded for.
    pub const fn desired_generation(&self) -> u64 {
        self.desired_generation
    }

    /// The resolved target.
    pub const fn target(&self) -> &ResolvedTarget {
        &self.target
    }

    /// The coarse execution handle.
    pub const fn handle(&self) -> TargetHandle {
        self.target.handle()
    }

    /// The bound guest session generation, when this is a guest target.
    pub const fn session_generation(&self) -> Option<u64> {
        self.target.session_generation()
    }
}

/// Availability of one guest target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetAvailability {
    /// The directory has never heard of this guest target.
    Unknown,
    /// A session is live at this generation.
    Connected { session_generation: u64 },
    /// The guest is known and its assignments survive, but no session is
    /// live: target-dependent observed state is unavailable (R21).
    Unavailable { last_session_generation: u64 },
}

impl TargetAvailability {
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// Target-local observation of one resource's realization.
///
/// [`Self::Absent`] and [`Self::Unavailable`] are deliberately distinct: a
/// disconnected target never lets a caller conclude that a realization is
/// missing (R21, and the guest-mount readiness gate that consumes this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetObservation {
    /// The target answered and holds no realization for this resource.
    Absent,
    /// The target answered and the realization is converging.
    Realizing { session_generation: u64 },
    /// The target answered and the realization is serving.
    Ready { session_generation: u64 },
    /// The target could not answer: no guest session is live. Desired state
    /// is untouched.
    Unavailable,
}

impl TargetObservation {
    /// Whether the target answered at all.
    pub const fn is_observed(self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    /// Whether the realization is present on the target (ready or not).
    pub const fn is_present(self) -> bool {
        matches!(self, Self::Realizing { .. } | Self::Ready { .. })
    }
}

/// Outcome of registering one authenticated guest session generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestConnectOutcome {
    reference: TargetRef,
    session_generation: u64,
    pending_adoption: Vec<ResourceKey>,
}

impl GuestConnectOutcome {
    pub const fn reference(&self) -> &TargetRef {
        &self.reference
    }

    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// Assignments whose realization must be re-adopted under this session
    /// generation (F5). Adoption is explicit; connecting never inherits the
    /// previous session's authority.
    pub fn pending_adoption(&self) -> &[ResourceKey] {
        &self.pending_adoption
    }
}

/// Outcome of losing one guest session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestDisconnectOutcome {
    reference: TargetRef,
    session_generation: u64,
    affected: Vec<ResourceKey>,
}

impl GuestDisconnectOutcome {
    pub const fn reference(&self) -> &TargetRef {
        &self.reference
    }

    /// The session generation that was lost.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// Assignments whose target-dependent observed state is now unavailable.
    /// They are still assigned and still desired: nothing is deleted or moved
    /// (R21).
    pub fn affected(&self) -> &[ResourceKey] {
        &self.affected
    }
}

/// Outcome of re-adopting assignments after a reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAdoptionOutcome {
    handle: GuestTargetHandle,
    adopted: Vec<GuestAdoption>,
}

impl GuestAdoptionOutcome {
    /// The handle re-bound to the live session generation.
    pub const fn handle(&self) -> &GuestTargetHandle {
        &self.handle
    }

    /// Per-source discovery results, in the order the sources were given.
    pub fn adopted(&self) -> &[GuestAdoption] {
        &self.adopted
    }
}

/// Closed target-resolution and target-operation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    /// The declared execution reference is not a canonical `Host/<name>` or
    /// `Guest/<name>`.
    InvalidExecutionReference,
    /// No record exists for this guest target.
    UnknownGuest,
    /// No guest session is live: desired state stays, realization waits.
    GuestUnavailable,
    /// A session generation of zero is never a valid ComponentSession.
    SessionGenerationZero,
    /// The operation belongs to a session generation that is not the live one.
    StaleSessionGeneration,
    /// A reconnect carried a generation that is not newer than the live one.
    SessionGenerationRegression,
    /// The target-control peer answered a protocol this host does not speak.
    ProtocolMismatch,
    /// A session is already live for this guest generation.
    SessionAlreadyConnected,
    /// No assignment is recorded for this resource.
    NotAssigned,
    /// The resource is not realized through a guest target.
    NotGuestTarget,
    /// The guest target refused the operation.
    GuestRefused(GuestTargetError),
}

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExecutionReference => formatter.write_str("target-invalid-execution-ref"),
            Self::UnknownGuest => formatter.write_str("target-unknown-guest"),
            Self::GuestUnavailable => formatter.write_str("target-guest-unavailable"),
            Self::SessionGenerationZero => formatter.write_str("target-session-generation-zero"),
            Self::StaleSessionGeneration => formatter.write_str("target-stale-session-generation"),
            Self::SessionGenerationRegression => {
                formatter.write_str("target-session-generation-regression")
            }
            Self::ProtocolMismatch => formatter.write_str("target-protocol-mismatch"),
            Self::SessionAlreadyConnected => formatter.write_str("target-session-already-connected"),
            Self::NotAssigned => formatter.write_str("target-not-assigned"),
            Self::NotGuestTarget => formatter.write_str("target-not-a-guest-target"),
            Self::GuestRefused(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for TargetError {}

/// Resolves the execution target a stored desired spec declares.
///
/// One implementation per composition: it reads the same canonical
/// `spec.executionRef` the resource contracts resolve, and returns the
/// canonical `Host/<name>` or `Guest/<name>` reference. A row whose type has
/// no execution anchor, or a legacy row that carries none, returns `None` and
/// realizes on the Zone's Host target.
pub trait TargetResolver: Send + Sync + 'static {
    fn execution_ref(&self, key: &ResourceKey, spec: &[u8]) -> Option<String>;
}

/// One resolved, directory-backed target binding for a single resource.
///
/// This is what a resource actor holds and what a guest-realizing driver
/// works through: the directory owns the session generation, and every
/// operation re-validates the binding against the live session instead of
/// trusting a channel the caller kept.
#[derive(Debug, Clone)]
pub struct TargetBinding {
    directory: Arc<TargetDirectory>,
    assignment: TargetAssignment,
}

impl TargetBinding {
    pub fn new(directory: Arc<TargetDirectory>, assignment: TargetAssignment) -> Self {
        Self { directory, assignment }
    }

    /// The per-Zone directory this binding resolves through.
    pub fn directory(&self) -> &Arc<TargetDirectory> {
        &self.directory
    }

    /// The recorded assignment.
    pub const fn assignment(&self) -> &TargetAssignment {
        &self.assignment
    }

    /// The coarse handle a driver context exposes (U4).
    pub const fn handle(&self) -> TargetHandle {
        self.assignment.handle()
    }

    /// The guest handle, when this resource realizes on a guest target.
    pub fn guest(&self) -> Option<&GuestTargetHandle> {
        self.assignment.target().guest()
    }

    /// Realize (create or update) the target-local instance through the live
    /// guest session. The spec is this driver's target-local shape; the host
    /// resolved it and the target applies exactly it.
    pub async fn realize(
        &self,
        spec: Vec<u8>,
        spec_digest: &str,
        local_handle: &str,
    ) -> Result<TargetResourceInstance, TargetError> {
        let handle = self.guest().ok_or(TargetError::NotGuestTarget)?;
        self.directory
            .realize(handle, self.assignment.source(), spec, spec_digest, local_handle)
            .await
    }

    /// Observe the target-local instance. [`TargetObservation::Unavailable`]
    /// means the target could not answer - never that the realization is gone.
    pub async fn observe(&self) -> Result<TargetObservation, TargetError> {
        let handle = self.guest().ok_or(TargetError::NotGuestTarget)?;
        self.directory.observe(handle, self.assignment.source()).await
    }

    /// Delete the target-local instance through the live guest session.
    pub async fn delete(&self) -> Result<bool, TargetError> {
        let handle = self.guest().ok_or(TargetError::NotGuestTarget)?;
        self.directory.delete(handle, self.assignment.source()).await
    }

    /// Re-adopt the target-local realization after a reconnect (F5). Returns
    /// the binding re-bound to the live session generation.
    pub async fn adopt(&self) -> Result<(TargetBinding, GuestAdoptionOutcome), TargetError> {
        let handle = self.guest().ok_or(TargetError::NotGuestTarget)?;
        let outcome = self.directory.adopt(handle, &[self.assignment.source().clone()]).await?;
        let rebound = outcome.handle().clone();
        let mut assignment = self.assignment.clone();
        assignment.target = ResolvedTarget::Guest(rebound);
        Ok((TargetBinding { directory: Arc::clone(&self.directory), assignment }, outcome))
    }
}

#[derive(Debug)]
struct GuestRecord {
    live: Option<LiveGuestSession>,
    last_session_generation: Option<u64>,
    assignments: HashMap<ResourceKey, TargetAssignment>,
}

#[derive(Debug, Clone)]
struct LiveGuestSession {
    session_generation: u64,
    control: Arc<dyn GuestTargetControl>,
}

#[derive(Debug, Default)]
struct DirectoryState {
    guests: BTreeMap<TargetRef, GuestRecord>,
    host_assignments: HashMap<ResourceKey, TargetAssignment>,
}

/// The per-Zone target directory (KTD5: one manager per Zone, one directory
/// per manager).
///
/// It decides *where* effects occur and holds the session-generation binding
/// that decides *whose* authority they may use. It never changes a resource's
/// Zone identity, never synthesizes a desired resource in a guest namespace,
/// and never deletes desired state because a target went away.
#[derive(Debug, Clone)]
pub struct TargetDirectory {
    inner: Arc<Mutex<DirectoryState>>,
}

impl Default for TargetDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetDirectory {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(DirectoryState::default())) }
    }

    /// Resolve one declared execution reference without recording anything.
    pub fn resolve(&self, execution_ref: &str) -> Result<ResolvedTarget, TargetError> {
        let reference = TargetRef::parse(execution_ref)?;
        let state = self.inner.lock();
        Ok(Self::resolve_locked(&state, reference))
    }

    fn resolve_locked(state: &DirectoryState, reference: TargetRef) -> ResolvedTarget {
        match reference.kind {
            TargetKind::Host => ResolvedTarget::Host(HostTargetHandle { reference }),
            TargetKind::Guest => {
                let session_generation = state
                    .guests
                    .get(&reference)
                    .and_then(|record| record.live.as_ref())
                    .map(|live| live.session_generation);
                ResolvedTarget::Guest(GuestTargetHandle { reference, session_generation })
            }
        }
    }

    /// Record (or replace) the target assignment of one resource.
    ///
    /// A guest assignment is recorded even while the guest is unavailable:
    /// desired state is durable and target failure never drops it (R21). The
    /// returned assignment carries the session generation it is bound to, and
    /// stays unusable until the guest is connected and the assignment adopted.
    pub fn assign(
        &self,
        source: &ResourceKey,
        uid: &[u8; 16],
        desired_generation: u64,
        execution_ref: &str,
    ) -> Result<TargetAssignment, TargetError> {
        let reference = TargetRef::parse(execution_ref)?;
        let mut state = self.inner.lock();
        let target = Self::resolve_locked(&state, reference.clone());
        let assignment = TargetAssignment {
            source: source.clone(),
            uid: *uid,
            desired_generation,
            target,
        };
        state.host_assignments.remove(source);
        for record in state.guests.values_mut() {
            record.assignments.remove(source);
        }
        match &reference.kind {
            TargetKind::Host => {
                state.host_assignments.insert(source.clone(), assignment.clone());
            }
            TargetKind::Guest => {
                state
                    .guests
                    .entry(reference)
                    .or_insert_with(Self::new_guest_record)
                    .assignments
                    .insert(source.clone(), assignment.clone());
            }
        }
        Ok(assignment)
    }

    /// The recorded assignment of one resource.
    pub fn assignment(&self, source: &ResourceKey) -> Option<TargetAssignment> {
        let state = self.inner.lock();
        if let Some(assignment) = state.host_assignments.get(source) {
            return Some(assignment.clone());
        }
        state
            .guests
            .values()
            .find_map(|record| record.assignments.get(source).cloned())
    }

    /// Drop the directory record of one resource, after its target-local
    /// realization has been deleted (F3: delete the realization, then forget
    /// the assignment; the durable deleting mark owns the retry).
    ///
    /// It removes one assignment and nothing else. The guest session, every
    /// other assignment for the same guest, and every other target-local
    /// realization stay exactly as they were (R20: a deleted ZoneLink must
    /// not disturb unrelated resources targeting the same guest).
    pub fn release(&self, source: &ResourceKey) -> Option<TargetAssignment> {
        let mut state = self.inner.lock();
        if let Some(assignment) = state.host_assignments.remove(source) {
            return Some(assignment);
        }
        state
            .guests
            .values_mut()
            .find_map(|record| record.assignments.remove(source))
    }

    /// Every resource currently assigned to one guest target, in identity
    /// order.
    pub fn assignments_for(&self, guest: &TargetRef) -> Vec<ResourceKey> {
        let mut assigned: Vec<ResourceKey> = self
            .inner
            .lock()
            .guests
            .get(guest)
            .map(|record| record.assignments.keys().cloned().collect())
            .unwrap_or_default();
        assigned.sort_by(|left, right| identity_order(left).cmp(&identity_order(right)));
        assigned
    }

    /// Availability of one guest target.
    pub fn availability(&self, guest: &TargetRef) -> TargetAvailability {
        match self.inner.lock().guests.get(guest) {
            None => TargetAvailability::Unknown,
            Some(record) => match (&record.live, record.last_session_generation) {
                (Some(live), _) => TargetAvailability::Connected {
                    session_generation: live.session_generation,
                },
                (None, Some(last_session_generation)) => {
                    TargetAvailability::Unavailable { last_session_generation }
                }
                (None, None) => TargetAvailability::Unknown,
            },
        }
    }

    fn new_guest_record() -> GuestRecord {
        GuestRecord { live: None, last_session_generation: None, assignments: HashMap::new() }
    }

    /// Register one authenticated guest session generation.
    ///
    /// A generation older than the live one is refused, so a reconnect can
    /// never inherit a newer session's authority. Replacing a live session
    /// requires a strictly newer generation; assignments are never re-bound
    /// here - the outcome names them and [`Self::adopt`] re-binds them.
    pub fn connect_guest(
        &self,
        guest: &TargetRef,
        session_generation: u64,
        control: Arc<dyn GuestTargetControl>,
    ) -> Result<GuestConnectOutcome, TargetError> {
        if session_generation == 0 {
            return Err(TargetError::SessionGenerationZero);
        }
        let mut state = self.inner.lock();
        let record = state
            .guests
            .entry(guest.clone())
            .or_insert_with(Self::new_guest_record);
        match &record.live {
            Some(live) if session_generation < live.session_generation => {
                return Err(TargetError::SessionGenerationRegression);
            }
            Some(live) if session_generation == live.session_generation => {
                return Err(TargetError::SessionAlreadyConnected);
            }
            Some(_) => {}
            None => {
                if record
                    .last_session_generation
                    .is_some_and(|last| session_generation < last)
                {
                    return Err(TargetError::SessionGenerationRegression);
                }
            }
        }
        record.last_session_generation = Some(session_generation);
        record.live = Some(LiveGuestSession { session_generation, control });
        let pending_adoption =
            record.assignments.keys().cloned().collect();
        Ok(GuestConnectOutcome { reference: guest.clone(), session_generation, pending_adoption })
    }

    /// Mark one guest session generation lost.
    ///
    /// Desired resources and assignments are untouched: only availability
    /// changes (R21). A disconnect carrying a generation that is not the live
    /// one is refused, so a stale notification can never tear down a newer
    /// session.
    pub fn disconnect_guest(
        &self,
        guest: &TargetRef,
        session_generation: u64,
    ) -> Result<GuestDisconnectOutcome, TargetError> {
        let mut state = self.inner.lock();
        let record = state.guests.get_mut(guest).ok_or(TargetError::UnknownGuest)?;
        match &record.live {
            Some(live) if live.session_generation == session_generation => {
                record.live = None;
            }
            Some(_) => return Err(TargetError::StaleSessionGeneration),
            None if record.last_session_generation == Some(session_generation) => {}
            None => return Err(TargetError::StaleSessionGeneration),
        }
        Ok(GuestDisconnectOutcome {
            reference: guest.clone(),
            session_generation,
            affected: record.assignments.keys().cloned().collect(),
        })
    }

    /// Realize (create or update) the target-local instance of one assigned
    /// resource through its live guest session.
    ///
    /// The spec and its commitment are the owning driver's target-local shape:
    /// the target applies what the host resolved and never invents a
    /// realization of its own.
    pub async fn realize(
        &self,
        handle: &GuestTargetHandle,
        source: &ResourceKey,
        spec: Vec<u8>,
        spec_digest: &str,
        local_handle: &str,
    ) -> Result<TargetResourceInstance, TargetError> {
        let (control, assignment) = self.session_authority(handle, source)?;
        let session_generation = handle.session_generation().ok_or(TargetError::GuestUnavailable)?;
        let request = GuestRealizeRequest::new(
            TargetControlAssignment::new(
                assignment.source.clone(),
                *assignment.uid(),
                assignment.desired_generation(),
                session_generation,
            ),
            spec,
            spec_digest,
            local_handle,
        );
        control.realize(request).await.map_err(Self::map_guest_error)
    }

    /// Observe the target-local instance of one assigned resource.
    ///
    /// An unanswered target reports [`TargetObservation::Unavailable`], never
    /// [`TargetObservation::Absent`]: a caller that cannot see the target must
    /// fail closed instead of concluding the realization is gone. A handle
    /// bound to a generation that is no longer live is a different failure -
    /// the target can answer, but not through this authority - and reports
    /// [`TargetError::StaleSessionGeneration`].
    pub async fn observe(
        &self,
        handle: &GuestTargetHandle,
        source: &ResourceKey,
    ) -> Result<TargetObservation, TargetError> {
        let (control, assignment) = match self.session_authority(handle, source) {
            Ok(authority) => authority,
            Err(TargetError::GuestUnavailable) => return Ok(TargetObservation::Unavailable),
            Err(error) => return Err(error),
        };
        let session_generation = handle.session_generation().ok_or(TargetError::GuestUnavailable)?;
        match control.observe(&Self::control_assignment(&assignment, session_generation)).await {
            Ok(observation) => Ok(observation),
            Err(GuestTargetError::SessionUnavailable) => Ok(TargetObservation::Unavailable),
            Err(error) => Err(Self::map_guest_error(error)),
        }
    }

    /// Delete the target-local instance of one assigned resource. Reports
    /// whether the target held one.
    pub async fn delete(
        &self,
        handle: &GuestTargetHandle,
        source: &ResourceKey,
    ) -> Result<bool, TargetError> {
        let (control, assignment) = self.session_authority(handle, source)?;
        let session_generation = handle.session_generation().ok_or(TargetError::GuestUnavailable)?;
        control
            .delete(&Self::control_assignment(&assignment, session_generation))
            .await
            .map(|()| true)
            .map_err(Self::map_guest_error)
    }

    /// Re-adopt the target-local realizations of assigned resources after a
    /// reconnect (F5).
    ///
    /// The only operation that accepts a handle from an older generation: it
    /// re-binds the assignment and the returned handle to the live generation
    /// and asks the target for discovery. Nothing is created here - a
    /// realization that is present is adopted, and a missing one is reported
    /// so the owning actor realizes it again.
    pub async fn adopt(
        &self,
        handle: &GuestTargetHandle,
        sources: &[ResourceKey],
    ) -> Result<GuestAdoptionOutcome, TargetError> {
        let (control, session_generation) = {
            let state = self.inner.lock();
            for source in sources {
                self.assigned_to(&state, source, handle)?;
            }
            let record = state.guests.get(handle.reference()).ok_or(TargetError::UnknownGuest)?;
            let live = record.live.as_ref().ok_or(TargetError::GuestUnavailable)?;
            (Arc::clone(&live.control), live.session_generation)
        };
        let adopted = {
            let mut adopted = Vec::with_capacity(sources.len());
            for source in sources {
                let assignment = self
                    .assignment(source)
                    .ok_or(TargetError::NotAssigned)?;
                adopted.push(
                    control
                        .adopt(&Self::control_assignment(&assignment, session_generation))
                        .await
                        .map_err(Self::map_guest_error)?,
                );
            }
            adopted
        };
        let rebound = GuestTargetHandle {
            reference: handle.reference.clone(),
            session_generation: Some(session_generation),
        };
        let target = ResolvedTarget::Guest(rebound.clone());
        let mut state = self.inner.lock();
        if let Some(record) = state.guests.get_mut(handle.reference()) {
            for source in sources {
                if let Some(assignment) = record.assignments.get_mut(source) {
                    assignment.target = target.clone();
                }
            }
        }
        Ok(GuestAdoptionOutcome { handle: rebound, adopted })
    }

    /// The assignment of one resource, verified to belong to the handle's
    /// guest target: a resource realized locally, or assigned to a different
    /// guest, is never this handle's authority.
    fn assigned_to<'a>(
        &self,
        state: &'a DirectoryState,
        source: &ResourceKey,
        handle: &GuestTargetHandle,
    ) -> Result<&'a TargetAssignment, TargetError> {
        if state.host_assignments.contains_key(source) {
            return Err(TargetError::NotGuestTarget);
        }
        state
            .guests
            .get(handle.reference())
            .and_then(|record| record.assignments.get(source))
            .ok_or(TargetError::NotAssigned)
    }

    /// The live control channel of one assignment, or the reason it has no
    /// authority: no live session, or a handle bound to another generation.
    fn session_authority(
        &self,
        handle: &GuestTargetHandle,
        source: &ResourceKey,
    ) -> Result<(Arc<dyn GuestTargetControl>, TargetAssignment), TargetError> {
        let state = self.inner.lock();
        let assignment = self.assigned_to(&state, source, handle)?.clone();
        let record = state.guests.get(handle.reference()).ok_or(TargetError::UnknownGuest)?;
        let live = record.live.as_ref().ok_or(TargetError::GuestUnavailable)?;
        match handle.session_generation {
            Some(bound) if bound == live.session_generation => {
                Ok((Arc::clone(&live.control), assignment))
            }
            Some(_) => Err(TargetError::StaleSessionGeneration),
            None => Err(TargetError::GuestUnavailable),
        }
    }

    /// The wire assignment of one directory record for one session
    /// generation: the same identity the record carries, plus the generation
    /// the request belongs to.
    fn control_assignment(
        assignment: &TargetAssignment,
        session_generation: u64,
    ) -> TargetControlAssignment {
        TargetControlAssignment::new(
            assignment.source.clone(),
            *assignment.uid(),
            assignment.desired_generation(),
            session_generation,
        )
    }

    fn map_guest_error(error: GuestTargetError) -> TargetError {
        match error {
            GuestTargetError::SessionUnavailable => TargetError::GuestUnavailable,
            GuestTargetError::ProtocolMismatch => TargetError::ProtocolMismatch,
            GuestTargetError::StaleSessionGeneration => TargetError::StaleSessionGeneration,
            GuestTargetError::SessionGenerationRegression => TargetError::SessionGenerationRegression,
        }
    }
}

/// Stable ordering for resource identities inside the directory.
fn identity_order(key: &ResourceKey) -> (&str, &str, &str) {
    (&key.zone, &key.type_name, &key.name)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::guest_target::GuestTargetRuntime;

    fn guest() -> TargetRef {
        TargetRef::guest("work-vm").expect("guest ref")
    }

    /// Fixture target-local spec and its commitment.
    fn spec() -> Vec<u8> {
        br#"{"providerRef":"Provider/test"}"#.to_vec()
    }

    fn digest() -> &'static str {
        "sha256:0000000000000000000000000000000000000000000000000000000000000002"
    }

    fn key(type_name: &str, name: &str) -> ResourceKey {
        ResourceKey::new("work", type_name, name)
    }

    /// Register one live guest session generation for the test guest.
    fn connected(runtime: &Arc<GuestTargetRuntime>, directory: &TargetDirectory, generation: u64) {
        runtime.bind_session(generation).expect("bind session");
        let control = runtime.control(generation).expect("control");
        directory.connect_guest(&guest(), generation, control).expect("connect guest");
    }

    fn guest_handle(directory: &TargetDirectory, source: &ResourceKey) -> GuestTargetHandle {
        directory
            .assignment(source)
            .expect("assignment")
            .target()
            .guest()
            .expect("guest handle")
            .clone()
    }

    /// A guest that stops answering while the directory still believes its
    /// session is live.
    #[derive(Debug)]
    struct Vanished;

    #[async_trait]
    impl GuestTargetControl for Vanished {
        async fn realize(
            &self,
            _request: GuestRealizeRequest,
        ) -> Result<TargetResourceInstance, GuestTargetError> {
            Err(GuestTargetError::SessionUnavailable)
        }

        async fn observe(
            &self,
            _assignment: &TargetControlAssignment,
        ) -> Result<TargetObservation, GuestTargetError> {
            Err(GuestTargetError::SessionUnavailable)
        }

        async fn delete(
            &self,
            _assignment: &TargetControlAssignment,
        ) -> Result<(), GuestTargetError> {
            Err(GuestTargetError::SessionUnavailable)
        }

        async fn adopt(
            &self,
            _assignment: &TargetControlAssignment,
        ) -> Result<GuestAdoption, GuestTargetError> {
            Err(GuestTargetError::SessionUnavailable)
        }
    }

    #[test]
    fn host_targeting_host_realizes_locally_through_the_directory() {
        let directory = TargetDirectory::new();
        let source = key("Process", "hosted");

        let resolved = directory.resolve("Host/main-host").expect("resolve host");
        assert_eq!(resolved.kind(), TargetKind::Host);
        assert_eq!(resolved.handle(), TargetHandle::Host);
        assert!(resolved.host().is_some(), "a Host target resolves to a local handle");
        assert_eq!(resolved.session_generation(), None, "no session crosses a local target");

        let assignment =
            directory.assign(&source, &[1; 16], 3, "Host/main-host").expect("assign host");
        assert_eq!(assignment.handle(), TargetHandle::Host);
        assert_eq!(assignment.session_generation(), None);
        assert_eq!(assignment.target().kind(), TargetKind::Host);
        assert_eq!(directory.assignment(&source), Some(assignment));
    }

    #[tokio::test]
    async fn a_host_assignment_is_never_realizable_through_a_guest_target() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let source = key("Process", "hosted");
        directory.assign(&source, &[1; 16], 3, "Host/main-host").expect("assign host");
        let elsewhere = key("Service", "elsewhere");
        directory.assign(&elsewhere, &[2; 16], 1, "Guest/work-vm").expect("assign guest");
        let handle = guest_handle(&directory, &elsewhere);

        assert_eq!(
            directory.realize(&handle, &source, spec(), digest(), "/run/d2b/hosted.sock").await.err(),
            Some(TargetError::NotGuestTarget),
            "a locally realized resource carries no guest authority"
        );
        assert_eq!(directory.assignments_for(&guest()), vec![elsewhere]);
    }

    #[tokio::test]
    async fn guest_targeting_realizes_through_the_guest_path_without_a_duplicate() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let source = key("Process", "worker");

        let assignment =
            directory.assign(&source, &[7; 16], 4, "Guest/work-vm").expect("assign guest");
        assert_eq!(assignment.handle(), TargetHandle::Guest);
        assert_eq!(assignment.session_generation(), Some(1));
        let handle = assignment.target().guest().expect("guest handle").clone();

        let first =
            directory.realize(&handle, &source, spec(), digest(), "/run/d2b/worker.sock").await.expect("realize");
        let second = directory.realize(&handle, &source, spec(), digest(), "/run/d2b/worker.sock")
            .await
            .expect("realize again");

        assert_eq!(first.source(), second.source());
        assert_eq!(first.local_handle(), "/run/d2b/worker.sock");
        assert_eq!(
            runtime.instances().len(),
            1,
            "the guest holds one target-local realization, not a second desired resource"
        );
        assert_eq!(
            runtime.instances()[0].source(),
            &source,
            "the realization is keyed by the Host-zone identity"
        );
        assert_eq!(runtime.instances()[0].source().zone, "work");
        assert_eq!(directory.assignments_for(&guest()), vec![source.clone()]);
        assert_eq!(
            directory.assignment(&source).expect("assignment").handle(),
            TargetHandle::Guest,
            "realization authority stays with the Host-zone resource"
        );
        assert!(runtime.mark_ready(&source).is_some());
        assert_eq!(
            directory.observe(&handle, &source).await.expect("observe"),
            TargetObservation::Ready { session_generation: 1 }
        );
    }

    #[tokio::test]
    async fn guest_disconnect_keeps_desired_state_and_reports_observed_state_unavailable() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let source = key("Process", "worker");
        directory.assign(&source, &[7; 16], 4, "Guest/work-vm").expect("assign guest");
        let handle = guest_handle(&directory, &source);
        directory.realize(&handle, &source, spec(), digest(), "/run/d2b/worker.sock").await.expect("realize");

        let lost = directory.disconnect_guest(&guest(), 1).expect("disconnect");

        assert_eq!(lost.affected(), [source.clone()], "the actor learns what lost its target");
        assert_eq!(
            directory.availability(&guest()),
            TargetAvailability::Unavailable { last_session_generation: 1 }
        );
        assert_eq!(
            directory.assignment(&source).expect("assignment survives").session_generation(),
            Some(1),
            "a target failure never drops or moves desired state"
        );
        assert_eq!(
            directory.observe(&handle, &source).await.expect("observe"),
            TargetObservation::Unavailable,
            "an unanswered target never reports Absent"
        );
        assert_eq!(
            directory.realize(&handle, &source, spec(), digest(), "/run/d2b/worker.sock").await.err(),
            Some(TargetError::GuestUnavailable)
        );
        assert_eq!(
            directory.delete(&handle, &source).await.err(),
            Some(TargetError::GuestUnavailable)
        );
        assert_eq!(runtime.instances().len(), 1, "the target-local realization is untouched");
        assert_eq!(directory.assignments_for(&guest()), vec![source]);
    }

    #[tokio::test]
    async fn a_guest_that_stops_answering_reports_unavailable_never_absent() {
        let directory = TargetDirectory::new();
        directory.connect_guest(&guest(), 1, Arc::new(Vanished)).expect("connect guest");
        let source = key("Process", "worker");
        directory.assign(&source, &[7; 16], 4, "Guest/work-vm").expect("assign guest");
        let handle = guest_handle(&directory, &source);

        assert_eq!(
            directory.observe(&handle, &source).await.expect("observe"),
            TargetObservation::Unavailable,
            "a target that cannot answer is not a target that answered Absent"
        );
        assert_eq!(
            directory.realize(&handle, &source, spec(), digest(), "/run/d2b/worker.sock").await.err(),
            Some(TargetError::GuestUnavailable)
        );
    }

    #[tokio::test]
    async fn guest_reconnect_rebinds_the_assignment_and_adopts_the_realization() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let source = key("Process", "worker");
        directory.assign(&source, &[7; 16], 4, "Guest/work-vm").expect("assign guest");
        let stale = guest_handle(&directory, &source);
        directory.realize(&stale, &source, spec(), digest(), "/run/d2b/worker.sock").await.expect("realize");
        runtime.mark_ready(&source).expect("ready");
        directory.disconnect_guest(&guest(), 1).expect("disconnect");

        runtime.bind_session(2).expect("reconnect");
        let reconnected = directory
            .connect_guest(&guest(), 2, runtime.control(2).expect("control"))
            .expect("connect guest");
        assert_eq!(reconnected.session_generation(), 2);
        assert_eq!(
            reconnected.pending_adoption(),
            [source.clone()],
            "a reconnect names the assignments that must re-run discovery"
        );

        let adopted = directory.adopt(&stale, &[source.clone()]).await.expect("adopt");

        assert_eq!(adopted.handle().session_generation(), Some(2), "the handle is re-bound");
        match &adopted.adopted()[0] {
            GuestAdoption::Adopted(instance) => {
                assert_eq!(instance.session_generation(), 2);
                assert_eq!(instance.local_handle(), "/run/d2b/worker.sock");
            }
            GuestAdoption::Missing => panic!("the present realization is adopted, not recreated"),
        }
        assert_eq!(runtime.instances().len(), 1, "adoption never mints a duplicate");
        assert_eq!(
            directory.assignment(&source).expect("assignment").session_generation(),
            Some(2)
        );
        assert_eq!(
            directory.observe(adopted.handle(), &source).await.expect("observe"),
            TargetObservation::Ready { session_generation: 2 }
        );
    }

    #[tokio::test]
    async fn a_stale_session_generation_cannot_inherit_realization_authority() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let source = key("Process", "worker");
        directory.assign(&source, &[7; 16], 4, "Guest/work-vm").expect("assign guest");
        let stale = guest_handle(&directory, &source);
        directory.realize(&stale, &source, spec(), digest(), "/run/d2b/worker.sock").await.expect("realize");

        runtime.bind_session(2).expect("reconnect");
        let live = directory
            .connect_guest(&guest(), 2, runtime.control(2).expect("control"))
            .expect("connect guest");

        assert_eq!(
            directory.realize(&stale, &source, spec(), digest(), "/run/d2b/worker.sock").await.err(),
            Some(TargetError::StaleSessionGeneration),
            "the old session cannot realize for the new one"
        );
        assert_eq!(
            directory.delete(&stale, &source).await.err(),
            Some(TargetError::StaleSessionGeneration)
        );
        assert_eq!(
            directory.observe(&stale, &source).await.err(),
            Some(TargetError::StaleSessionGeneration)
        );
        assert_eq!(
            directory
                .adopt(&stale, &[source.clone()])
                .await
                .expect("adopt")
                .handle()
                .session_generation(),
            Some(2),
            "only adoption may cross the generation boundary"
        );
        assert!(live.pending_adoption().contains(&source));
    }

    #[test]
    fn stale_session_events_cannot_tear_down_or_replace_a_newer_session() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(2).expect("bind session");
        directory
            .connect_guest(&guest(), 2, runtime.control(2).expect("control"))
            .expect("connect guest");

        assert_eq!(
            directory.disconnect_guest(&guest(), 1).err(),
            Some(TargetError::StaleSessionGeneration),
            "a stale disconnect must not tear down the live session"
        );
        assert_eq!(
            directory.availability(&guest()),
            TargetAvailability::Connected { session_generation: 2 }
        );
        assert_eq!(
            directory.connect_guest(&guest(), 1, Arc::new(Vanished)).err(),
            Some(TargetError::SessionGenerationRegression),
            "a reconnect can never carry an older generation"
        );
        assert_eq!(
            directory.connect_guest(&guest(), 2, Arc::new(Vanished)).err(),
            Some(TargetError::SessionAlreadyConnected)
        );
        assert_eq!(
            directory.connect_guest(&guest(), 0, Arc::new(Vanished)).err(),
            Some(TargetError::SessionGenerationZero)
        );
        assert_eq!(
            directory.disconnect_guest(&guest(), 2).expect("disconnect").session_generation(),
            2
        );
        assert_eq!(
            directory.disconnect_guest(&guest(), 2).expect("idempotent").session_generation(),
            2
        );
    }

    #[tokio::test]
    async fn zone_link_deletion_leaves_unrelated_resources_targeting_the_same_guest_alone() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let link = key("ZoneLink", "work-edge");
        let worker = key("Process", "worker");
        directory.assign(&link, &[3; 16], 1, "Guest/work-vm").expect("assign link");
        directory.assign(&worker, &[7; 16], 4, "Guest/work-vm").expect("assign worker");
        let link_handle = guest_handle(&directory, &link);
        let worker_handle = guest_handle(&directory, &worker);
        directory.realize(&link_handle, &link, spec(), digest(), "/run/d2b/edge.sock").await.expect("realize link");
        directory.realize(&worker_handle, &worker, spec(), digest(), "/run/d2b/worker.sock")
            .await
            .expect("realize worker");

        assert!(
            directory.delete(&link_handle, &link).await.expect("delete link realization"),
            "the ZoneLink's own realization is the only one torn down"
        );
        let released = directory.release(&link).expect("release link assignment");
        assert_eq!(released.source(), &link);
        assert_eq!(
            directory.delete(&link_handle, &link).await.err(),
            Some(TargetError::NotAssigned),
            "a released assignment carries no lingering authority"
        );

        assert_eq!(
            directory.assignments_for(&guest()),
            vec![worker.clone()],
            "the unrelated assignment survives the ZoneLink deletion"
        );
        assert_eq!(
            directory.assignment(&worker).expect("worker assignment").session_generation(),
            Some(1)
        );
        assert_eq!(
            directory.availability(&guest()),
            TargetAvailability::Connected { session_generation: 1 },
            "deleting a ZoneLink resource never touches the guest session"
        );
        assert_eq!(runtime.instances().len(), 1);
        assert_eq!(runtime.instances()[0].source(), &worker);
        assert_eq!(
            directory.observe(&worker_handle, &worker).await.expect("observe"),
            TargetObservation::Realizing { session_generation: 1 }
        );
    }

    /// §36 targeting/ZoneLink separation (`ordinary Guest-targeted resource
    /// does not create or modify a ZoneLink`): realizing an ordinary resource
    /// on a Guest never mints a ZoneLink assignment and leaves the live
    /// link's own assignment and realization untouched.
    #[tokio::test]
    async fn ordinary_guest_realization_never_creates_or_moves_a_zone_link() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let link = key("ZoneLink", "work-edge");
        let worker = key("Process", "worker");
        let link_assignment =
            directory.assign(&link, &[3; 16], 1, "Guest/work-vm").expect("assign link");
        let link_handle = guest_handle(&directory, &link);
        directory
            .realize(&link_handle, &link, spec(), digest(), "/run/d2b/edge.sock")
            .await
            .expect("realize link");
        runtime.mark_ready(&link).expect("link ready");

        // The ordinary resource runs its whole target lifecycle alongside the
        // link: assigned, realized, observed, deleted, released.
        directory.assign(&worker, &[7; 16], 4, "Guest/work-vm").expect("assign worker");
        let worker_handle = guest_handle(&directory, &worker);
        directory
            .realize(&worker_handle, &worker, spec(), digest(), "/run/d2b/worker.sock")
            .await
            .expect("realize worker");
        assert_eq!(
            directory.observe(&worker_handle, &worker).await.expect("observe"),
            TargetObservation::Realizing { session_generation: 1 }
        );
        directory.delete(&worker_handle, &worker).await.expect("delete worker realization");
        directory.release(&worker).expect("release worker");

        assert_eq!(
            directory.assignment(&link),
            Some(link_assignment),
            "the ordinary resource's lifecycle never rewrites the ZoneLink assignment"
        );
        assert_eq!(
            directory.assignments_for(&guest()),
            vec![link.clone()],
            "no second ZoneLink (or any other) assignment is synthesized"
        );
        assert_eq!(
            directory.observe(&link_handle, &link).await.expect("observe"),
            TargetObservation::Ready { session_generation: 1 },
            "the link's realization is untouched"
        );
        assert_eq!(runtime.instances().len(), 1, "only the link remains realized");
        assert_eq!(runtime.instances()[0].source(), &link);
    }

    /// §36 targeting/ZoneLink separation (`ZoneLink topology state and Guest
    /// target availability can change independently`): a Guest session drop
    /// and reconnect move availability and adoption, never the link's
    /// assignment or realization; the link's own release never moves
    /// availability.
    #[tokio::test]
    async fn zone_link_topology_and_guest_availability_move_independently() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let link = key("ZoneLink", "work-edge");
        let link_assignment =
            directory.assign(&link, &[3; 16], 1, "Guest/work-vm").expect("assign link");
        let link_handle = guest_handle(&directory, &link);
        directory
            .realize(&link_handle, &link, spec(), digest(), "/run/d2b/edge.sock")
            .await
            .expect("realize link");

        // Availability drops; the link keeps its assignment and realization.
        directory.disconnect_guest(&guest(), 1).expect("disconnect");
        assert_eq!(
            directory.availability(&guest()),
            TargetAvailability::Unavailable { last_session_generation: 1 }
        );
        assert_eq!(directory.assignment(&link), Some(link_assignment));
        assert_eq!(runtime.instances().len(), 1, "the drop never deletes the link's realization");

        // Availability returns on a new generation; only adoption may cross it.
        runtime.bind_session(2).expect("reconnect");
        let reconnected = directory
            .connect_guest(&guest(), 2, runtime.control(2).expect("control"))
            .expect("connect guest");
        assert_eq!(
            directory.availability(&guest()),
            TargetAvailability::Connected { session_generation: 2 }
        );
        assert_eq!(reconnected.pending_adoption(), [link.clone()], "the link re-adopts");
        assert_eq!(
            directory.assignment(&link).expect("assignment").session_generation(),
            Some(1),
            "availability moved; the link assignment did not"
        );

        // The link's own lifecycle never moves availability.
        directory.release(&link).expect("release link");
        assert_eq!(
            directory.availability(&guest()),
            TargetAvailability::Connected { session_generation: 2 },
            "link topology state and target availability are independent"
        );
        assert_eq!(runtime.instances().len(), 1, "releasing the assignment keeps the realization");
    }

    #[tokio::test]
    async fn a_resource_carries_exactly_one_target_assignment() {
        let directory = TargetDirectory::new();
        let guest_a = TargetRef::guest("guest-a").expect("guest");
        let guest_b = TargetRef::guest("guest-b").expect("guest");
        let first = Arc::new(GuestTargetRuntime::new(guest_a.clone()));
        let second = Arc::new(GuestTargetRuntime::new(guest_b.clone()));
        let source = key("Process", "worker");
        for (runtime, reference) in [(&first, &guest_a), (&second, &guest_b)] {
            runtime.bind_session(1).expect("bind session");
            directory
                .connect_guest(reference, 1, runtime.control(1).expect("control"))
                .expect("connect guest");
        }
        directory.assign(&source, &[7; 16], 1, "Guest/guest-a").expect("assign first");
        directory.assign(&source, &[7; 16], 2, "Guest/guest-b").expect("assign second");

        assert!(
            directory.assignments_for(&guest_a).is_empty(),
            "the old guest loses the assignment: one assignment per resource"
        );
        assert_eq!(directory.assignments_for(&guest_b), vec![source.clone()]);
        let handle = guest_handle(&directory, &source);
        assert_eq!(handle.reference(), &guest_b);
        assert_eq!(
            directory.realize(&handle, &source, spec(), digest(), "/run/d2b/worker.sock")
                .await
                .expect("realize")
                .session_generation(),
            1
        );
        assert_eq!(first.instances().len(), 0, "the old target never realizes the moved resource");
        assert_eq!(second.instances().len(), 1);
    }

    #[tokio::test]
    async fn only_assigned_resources_have_target_authority() {
        let directory = TargetDirectory::new();
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        connected(&runtime, &directory, 1);
        let assigned = key("Process", "assigned");
        let unassigned = key("Process", "unassigned");
        directory.assign(&assigned, &[7; 16], 1, "Guest/work-vm").expect("assign guest");
        let handle = guest_handle(&directory, &assigned);

        assert_eq!(
            directory.realize(&handle, &unassigned, spec(), digest(), "/run/d2b/unassigned.sock").await.err(),
            Some(TargetError::NotAssigned)
        );
        assert_eq!(directory.assignments_for(&guest()), vec![assigned]);
        assert!(directory.release(&unassigned).is_none());
    }

    #[tokio::test]
    async fn a_target_binding_never_trusts_a_channel_it_kept() {
        let directory = Arc::new(TargetDirectory::new());
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        directory
            .connect_guest(&guest(), 1, runtime.control(1).expect("control"))
            .expect("connect guest");
        let source = key("Process", "worker");
        let assignment =
            directory.assign(&source, &[7; 16], 2, "Guest/work-vm").expect("assign guest");
        let binding = TargetBinding::new(Arc::clone(&directory), assignment);

        binding.realize(spec(), digest(), "/run/d2b/worker.sock").await.expect("realize");
        assert_eq!(
            binding.observe().await.expect("observe"),
            TargetObservation::Realizing { session_generation: 1 }
        );

        directory.disconnect_guest(&guest(), 1).expect("disconnect");
        assert_eq!(
            binding.observe().await.expect("observe"),
            TargetObservation::Unavailable,
            "the binding reflects the live session, not the one it was minted under"
        );
        assert_eq!(
            binding.realize(spec(), digest(), "/run/d2b/worker.sock").await.err(),
            Some(TargetError::GuestUnavailable)
        );

        runtime.bind_session(2).expect("reconnect");
        directory
            .connect_guest(&guest(), 2, runtime.control(2).expect("control"))
            .expect("connect guest");
        let (rebound, outcome) = binding.adopt().await.expect("adopt");
        assert_eq!(rebound.guest().expect("guest handle").session_generation(), Some(2));
        assert!(matches!(outcome.adopted()[0], GuestAdoption::Adopted(_)));
        assert_eq!(
            binding.realize(spec(), digest(), "/run/d2b/worker.sock").await.err(),
            Some(TargetError::StaleSessionGeneration),
            "the un-adopted binding cannot act for the new session"
        );
        rebound.realize(spec(), digest(), "/run/d2b/worker.sock").await.expect("realize through the adopted binding");
        assert_eq!(runtime.instances().len(), 1);
    }

    #[tokio::test]
    async fn a_host_binding_has_no_guest_realization_path() {
        let directory = Arc::new(TargetDirectory::new());
        let source = key("Process", "hosted");
        let assignment =
            directory.assign(&source, &[1; 16], 1, "Host/main-host").expect("assign host");
        let binding = TargetBinding::new(directory, assignment);

        assert_eq!(binding.handle(), TargetHandle::Host);
        assert!(binding.guest().is_none());
        assert_eq!(binding.realize(spec(), digest(), "/run/d2b/hosted.sock").await.err(), Some(TargetError::NotGuestTarget));
        assert_eq!(binding.observe().await.err(), Some(TargetError::NotGuestTarget));
        assert_eq!(binding.delete().await.err(), Some(TargetError::NotGuestTarget));
        assert_eq!(binding.adopt().await.err(), Some(TargetError::NotGuestTarget));
    }

    #[test]
    fn execution_references_are_canonical_or_refused() {
        assert_eq!(
            TargetRef::parse("Host/main-host").expect("host").to_canonical_string(),
            "Host/main-host"
        );
        assert_eq!(TargetRef::parse("Guest/work-vm").expect("guest").kind(), TargetKind::Guest);
        for invalid in
            ["", "main-host", "Zone/work", "Host/", "Host/a/b", "Host/../etc", "Host/host name"]
        {
            assert_eq!(
                TargetRef::parse(invalid).err(),
                Some(TargetError::InvalidExecutionReference),
                "{invalid} is not an execution reference"
            );
        }
    }
}
