//! Provider-neutral target runtime and bounded admission contracts.
//!
//! The composition crate selects concrete Providers and effect adapters. This
//! module owns the authority-neutral lifecycle that both Host and Guest
//! instances use: fixed process-start mode, bounded admission, target-scoped
//! controller assignments, and revocation on session loss.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use d2b_contracts_broker::broker_wire::BrokerProfile;
use d2b_contracts_provider::v3::{
    ArtifactDigest, ComponentDescriptor, ComponentExecution, ComponentType,
    ControllerInstanceScope, ControllerTargetKind, EffectPortClass,
};
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceName, ResourceRef, ResourceTypeName,
    ResourceUid, SchemaFingerprint, ZoneId, ZoneRevision,
    identity::ReconnectGeneration,
    process::{ExecutionSpec, ProcessClass, ProcessSpec},
};
use sha2::{Digest, Sha256};

/// The only daemon modes. A mode is selected before the runtime starts and is
/// never read from a request or changed on a live instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DaemonMode {
    Host,
    Guest,
}

impl DaemonMode {
    /// Stable process-start spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Guest => "guest",
        }
    }

    /// Parse only the two closed process-start spellings.
    pub fn parse(value: &str) -> Result<Self, ModeParseError> {
        match value {
            "host" => Ok(Self::Host),
            "guest" => Ok(Self::Guest),
            _ => Err(ModeParseError::UnknownMode),
        }
    }

    /// The broker profile bound to this daemon mode.
    pub const fn broker_profile(self) -> BrokerProfile {
        match self {
            Self::Host => BrokerProfile::Host,
            Self::Guest => BrokerProfile::Guest,
        }
    }

    /// The execution target kind owned by this daemon mode.
    pub const fn target_kind(self) -> TargetKind {
        match self {
            Self::Host => TargetKind::Host,
            Self::Guest => TargetKind::Guest,
        }
    }

    /// Surfaces that may be initialized by the mode.
    pub const fn surfaces(self) -> ModeSurfaces {
        match self {
            Self::Host => ModeSurfaces {
                local_zone_store: true,
                public_operator_socket: true,
                realm_credentials: true,
                host_controller_authority: true,
                parent_component_session: false,
            },
            Self::Guest => ModeSurfaces {
                local_zone_store: false,
                public_operator_socket: false,
                realm_credentials: false,
                host_controller_authority: false,
                parent_component_session: true,
            },
        }
    }
}

impl std::fmt::Display for DaemonMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Closed mode parser failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeParseError {
    UnknownMode,
}

impl std::fmt::Display for ModeParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("daemon-mode-unknown")
    }
}

impl std::error::Error for ModeParseError {}

/// Target kinds accepted by the shared ProviderDeployment lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetKind {
    Host,
    Guest,
}

impl TargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Guest => "guest",
        }
    }
}

/// Authority-bearing surfaces a mode is allowed to initialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeSurfaces {
    pub local_zone_store: bool,
    pub public_operator_socket: bool,
    pub realm_credentials: bool,
    pub host_controller_authority: bool,
    pub parent_component_session: bool,
}

impl ModeSurfaces {
    pub const fn host() -> Self {
        DaemonMode::Host.surfaces()
    }

    pub const fn guest() -> Self {
        DaemonMode::Guest.surfaces()
    }
}

/// Bounded admission classes used before per-request state is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdmissionKind {
    Session,
    Reconnect,
    Controller,
    Watch,
    Stream,
}

impl AdmissionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Reconnect => "reconnect",
            Self::Controller => "controller",
            Self::Watch => "watch",
            Self::Stream => "stream",
        }
    }
}

/// Fixed admission budgets for a target runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionLimits {
    pub max_sessions: usize,
    pub max_reconnects_per_window: usize,
    pub max_controllers: usize,
    pub max_watches: usize,
    pub max_streams: usize,
    pub reconnect_window: Duration,
}

impl AdmissionLimits {
    /// Conservative Guest defaults. Host callers may use a larger explicit
    /// profile, but zero is never treated as unlimited.
    pub const fn guest_default() -> Self {
        Self {
            max_sessions: 4,
            max_reconnects_per_window: 8,
            max_controllers: 32,
            max_watches: 64,
            max_streams: 128,
            reconnect_window: Duration::from_secs(60),
        }
    }

    /// Host defaults share the same machinery while allowing the normal
    /// operator-facing concurrency envelope.
    pub const fn host_default() -> Self {
        Self {
            max_sessions: 64,
            max_reconnects_per_window: 64,
            max_controllers: 256,
            max_watches: 512,
            max_streams: 1024,
            reconnect_window: Duration::from_secs(60),
        }
    }

    fn validate(self) -> Result<Self, AdmissionError> {
        if self.max_sessions == 0
            || self.max_reconnects_per_window == 0
            || self.max_controllers == 0
            || self.max_watches == 0
            || self.max_streams == 0
            || self.reconnect_window.is_zero()
        {
            return Err(AdmissionError::InvalidLimits);
        }
        Ok(self)
    }
}

/// Bounded admission failure. The class is a closed label, never a caller
/// supplied diagnostic string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    InvalidLimits,
    LimitExceeded(AdmissionKind),
    ReconnectWindowUnavailable,
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("target-runtime-invalid-limits"),
            Self::LimitExceeded(kind) => {
                write!(formatter, "target-runtime-{}-limit", kind.as_str())
            }
            Self::ReconnectWindowUnavailable => {
                formatter.write_str("target-runtime-reconnect-window-unavailable")
            }
        }
    }
}

impl std::error::Error for AdmissionError {}

#[derive(Debug)]
struct AdmissionCounters {
    sessions: AtomicUsize,
    controllers: AtomicUsize,
    watches: AtomicUsize,
    streams: AtomicUsize,
    reconnects: AtomicUsize,
    reconnect_history: Mutex<VecDeque<Instant>>,
}

/// Shared admission state for one mode-bound runtime.
#[derive(Debug, Clone)]
pub struct AdmissionBudget {
    limits: AdmissionLimits,
    counters: Arc<AdmissionCounters>,
}

impl AdmissionBudget {
    pub fn new(limits: AdmissionLimits) -> Result<Self, AdmissionError> {
        Ok(Self {
            limits: limits.validate()?,
            counters: Arc::new(AdmissionCounters {
                sessions: AtomicUsize::new(0),
                controllers: AtomicUsize::new(0),
                watches: AtomicUsize::new(0),
                streams: AtomicUsize::new(0),
                reconnects: AtomicUsize::new(0),
                reconnect_history: Mutex::new(VecDeque::new()),
            }),
        })
    }

    pub const fn limits(&self) -> AdmissionLimits {
        self.limits
    }

    /// Reserve a non-reconnect class without allocating class state first.
    pub fn try_admit(&self, kind: AdmissionKind) -> Result<AdmissionPermit, AdmissionError> {
        let (counter, limit) = match kind {
            AdmissionKind::Session => (&self.counters.sessions, self.limits.max_sessions),
            AdmissionKind::Controller => (&self.counters.controllers, self.limits.max_controllers),
            AdmissionKind::Watch => (&self.counters.watches, self.limits.max_watches),
            AdmissionKind::Stream => (&self.counters.streams, self.limits.max_streams),
            AdmissionKind::Reconnect => return self.try_admit_reconnect(Instant::now()),
        };
        reserve(counter, limit, kind).map(|_| permit(kind, Arc::clone(&self.counters)))
    }

    /// Reserve one reconnect attempt inside the fixed sliding window.
    pub fn try_admit_reconnect(&self, now: Instant) -> Result<AdmissionPermit, AdmissionError> {
        let mut history = self
            .counters
            .reconnect_history
            .lock()
            .map_err(|_| AdmissionError::ReconnectWindowUnavailable)?;
        while history.front().is_some_and(|started| {
            now.saturating_duration_since(*started) >= self.limits.reconnect_window
        }) {
            history.pop_front();
        }
        if history.len() >= self.limits.max_reconnects_per_window {
            return Err(AdmissionError::LimitExceeded(AdmissionKind::Reconnect));
        }
        reserve(
            &self.counters.reconnects,
            self.limits.max_reconnects_per_window,
            AdmissionKind::Reconnect,
        )?;
        history.push_back(now);
        Ok(permit(AdmissionKind::Reconnect, Arc::clone(&self.counters)))
    }

    pub fn active(&self, kind: AdmissionKind) -> usize {
        match kind {
            AdmissionKind::Session => self.counters.sessions.load(Ordering::Acquire),
            AdmissionKind::Reconnect => self.counters.reconnects.load(Ordering::Acquire),
            AdmissionKind::Controller => self.counters.controllers.load(Ordering::Acquire),
            AdmissionKind::Watch => self.counters.watches.load(Ordering::Acquire),
            AdmissionKind::Stream => self.counters.streams.load(Ordering::Acquire),
        }
    }
}

fn reserve(counter: &AtomicUsize, limit: usize, kind: AdmissionKind) -> Result<(), AdmissionError> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= limit {
            return Err(AdmissionError::LimitExceeded(kind));
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(()),
            Err(observed) => current = observed,
        }
    }
}

/// RAII reservation released when the admitted operation is closed.
#[derive(Debug)]
struct PermitState {
    kind: AdmissionKind,
    counters: Arc<AdmissionCounters>,
    released: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct AdmissionPermit {
    inner: Arc<PermitState>,
}

impl AdmissionPermit {
    pub fn kind(&self) -> AdmissionKind {
        self.inner.kind
    }

    pub fn release(&self) {
        if !self.inner.released.swap(true, Ordering::AcqRel) {
            let counter = match self.inner.kind {
                AdmissionKind::Session => &self.inner.counters.sessions,
                AdmissionKind::Reconnect => &self.inner.counters.reconnects,
                AdmissionKind::Controller => &self.inner.counters.controllers,
                AdmissionKind::Watch => &self.inner.counters.watches,
                AdmissionKind::Stream => &self.inner.counters.streams,
            };
            counter.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        self.release();
    }
}

fn permit(kind: AdmissionKind, counters: Arc<AdmissionCounters>) -> AdmissionPermit {
    AdmissionPermit {
        inner: Arc::new(PermitState {
            kind,
            counters,
            released: AtomicBool::new(false),
        }),
    }
}

/// Finalizer retained on every target-local controller Process until the
/// controller's children have been adopted or quarantined and terminally
/// accounted for.
pub const CONTROLLER_PROCESS_FINALIZER: &str = "provider-controller.d2bus.org/children";

/// Resource verbs a controller ResourceClient may use. Spec writes and
/// arbitrary deletes remain outside the controller capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ControllerResourceVerb {
    Get,
    List,
    Watch,
    Create,
    UpdateStatus,
    UpdateFinalizers,
}

/// Closed mutation/read set granted to every admitted controller client.
pub const CONTROLLER_ALLOWED_RESOURCE_VERBS: &[ControllerResourceVerb; 6] = &[
    ControllerResourceVerb::Get,
    ControllerResourceVerb::List,
    ControllerResourceVerb::Watch,
    ControllerResourceVerb::Create,
    ControllerResourceVerb::UpdateStatus,
    ControllerResourceVerb::UpdateFinalizers,
];

/// Lifecycle of a signed target-local controller Process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ControllerProcessPhase {
    Pending,
    Launching,
    Running,
    Ready,
    Assigned,
    Draining,
    Revoked,
    Quarantined,
    Released,
}

/// The immutable Process resource Core creates for one signed controller
/// component and one exact execution target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerProcessResource {
    process_ref: ResourceRef,
    controller_role_ref: ResourceRef,
    uid: ResourceUid,
    zone: ZoneId,
    provider_ref: ResourceRef,
    component_id: d2b_contracts_resource::v3::execution_policy::BoundedToken,
    target: ResourceRef,
    process_provider_ref: ResourceRef,
    process_spec: ProcessSpec,
    resource_generation: ResourceGeneration,
    resource_revision: ZoneRevision,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    target_session_generation: ReconnectGeneration,
    signed_descriptor_digest: ArtifactDigest,
    artifact_digest: ArtifactDigest,
    required_effect_classes: BTreeSet<EffectPortClass>,
    owned_resource_types: BTreeSet<ResourceTypeName>,
    repair_owner: ResourceRef,
}

impl ControllerProcessResource {
    /// Borrow the generated Process resource reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the signed controller role reference represented by this
    /// Process instance.
    pub const fn controller_role_ref(&self) -> &ResourceRef {
        &self.controller_role_ref
    }

    /// Borrow the generated Process resource UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Borrow the containing Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the semantic Provider that owns this controller role.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the signed component identifier.
    pub const fn component_id(
        &self,
    ) -> &d2b_contracts_resource::v3::execution_policy::BoundedToken {
        &self.component_id
    }

    /// Borrow the exact Host or Guest execution target.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Borrow the selected fixed Process Provider.
    pub const fn process_provider_ref(&self) -> &ResourceRef {
        &self.process_provider_ref
    }

    /// Borrow the generated controller Process spec.
    pub const fn process_spec(&self) -> &ProcessSpec {
        &self.process_spec
    }

    /// Return the Process resource generation.
    pub const fn resource_generation(&self) -> ResourceGeneration {
        self.resource_generation
    }

    /// Return the committed Process resource revision.
    pub const fn resource_revision(&self) -> ZoneRevision {
        self.resource_revision
    }

    /// Return the installed semantic Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the signed controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Return the parent target ComponentSession generation.
    pub const fn target_session_generation(&self) -> ReconnectGeneration {
        self.target_session_generation
    }

    /// Borrow the signed descriptor commitment.
    pub const fn signed_descriptor_digest(&self) -> &ArtifactDigest {
        &self.signed_descriptor_digest
    }

    /// Borrow the concrete target artifact commitment.
    pub const fn artifact_digest(&self) -> &ArtifactDigest {
        &self.artifact_digest
    }

    /// Borrow the signed EffectPort class set.
    pub fn required_effect_classes(&self) -> &BTreeSet<EffectPortClass> {
        &self.required_effect_classes
    }

    /// Borrow the ResourceTypes owned by this signed controller role.
    pub fn owned_resource_types(&self) -> &BTreeSet<ResourceTypeName> {
        &self.owned_resource_types
    }

    /// Return the finalizer Core retains while this controller owns children.
    pub const fn finalizer(&self) -> &'static str {
        CONTROLLER_PROCESS_FINALIZER
    }

    /// Borrow the single repair owner for this Process resource.
    pub const fn repair_owner(&self) -> &ResourceRef {
        &self.repair_owner
    }
}

/// Authenticated readiness evidence for a controller's own ComponentSession.
///
/// This is separate from the target session generation carried by the static
/// launch ticket. It is the only evidence accepted before a ResourceClient
/// assignment is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerSessionBinding {
    process_ref: ResourceRef,
    zone: ZoneId,
    provider_ref: ResourceRef,
    target: ResourceRef,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    target_session_generation: ReconnectGeneration,
    session_generation: ReconnectGeneration,
    readiness_digest: SchemaFingerprint,
}

impl ControllerSessionBinding {
    /// Construct authenticated controller-session evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        process_ref: ResourceRef,
        zone: ZoneId,
        provider_ref: ResourceRef,
        target: ResourceRef,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        target_session_generation: ReconnectGeneration,
        session_generation: ReconnectGeneration,
        readiness_digest: SchemaFingerprint,
    ) -> Self {
        Self {
            process_ref,
            zone,
            provider_ref,
            target,
            provider_generation,
            controller_generation,
            target_session_generation,
            session_generation,
            readiness_digest,
        }
    }

    /// Borrow the controller Process reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the semantic Provider identity.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the exact execution target.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Return the Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Return the target session generation.
    pub const fn target_session_generation(&self) -> ReconnectGeneration {
        self.target_session_generation
    }

    /// Return the controller's authenticated session generation.
    pub const fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    /// Borrow the controller readiness commitment.
    pub const fn readiness_digest(&self) -> &SchemaFingerprint {
        &self.readiness_digest
    }
}

/// A ResourceClient assignment request after controller readiness succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerAssignmentRequest {
    process_ref: ResourceRef,
    resource_ref: ResourceRef,
    resource_uid: ResourceUid,
    resource_generation: ResourceGeneration,
    resource_revision: ZoneRevision,
    provider_ref: ResourceRef,
    target: ResourceRef,
    session_generation: ReconnectGeneration,
}

impl ControllerAssignmentRequest {
    /// Construct one exact resource-to-controller assignment request.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        process_ref: ResourceRef,
        resource_ref: ResourceRef,
        resource_uid: ResourceUid,
        resource_generation: ResourceGeneration,
        resource_revision: ZoneRevision,
        provider_ref: ResourceRef,
        target: ResourceRef,
        session_generation: ReconnectGeneration,
    ) -> Self {
        Self {
            process_ref,
            resource_ref,
            resource_uid,
            resource_generation,
            resource_revision,
            provider_ref,
            target,
            session_generation,
        }
    }
}

/// The exact identity bound to a controller's scoped ResourceClient.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControllerAssignmentIdentity {
    process_ref: ResourceRef,
    resource_ref: ResourceRef,
    resource_uid: ResourceUid,
    resource_generation: ResourceGeneration,
    resource_revision: ZoneRevision,
    provider_ref: ResourceRef,
    target: ResourceRef,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    session_generation: ReconnectGeneration,
    assignment_epoch: u64,
}

impl ControllerAssignmentIdentity {
    /// Borrow the controller Process identity.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the assigned resource.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the assigned resource UID.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    /// Return the assigned resource generation.
    pub const fn resource_generation(&self) -> ResourceGeneration {
        self.resource_generation
    }

    /// Return the assigned resource revision.
    pub const fn resource_revision(&self) -> ZoneRevision {
        self.resource_revision
    }

    /// Borrow the semantic Provider identity.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the exact execution target.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Return the Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Return the authenticated controller session generation.
    pub const fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    /// Return the monotonically increasing assignment epoch.
    pub const fn assignment_epoch(&self) -> u64 {
        self.assignment_epoch
    }
}

/// A controller child observed during restart adoption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerChildObservation {
    child_ref: ResourceRef,
    uid: ResourceUid,
    identity_verified: bool,
}

impl ControllerChildObservation {
    /// Construct a fully verified child observation.
    pub fn verified(child_ref: ResourceRef, uid: ResourceUid) -> Self {
        Self {
            child_ref,
            uid,
            identity_verified: true,
        }
    }

    /// Construct an ambiguous observation that must be quarantined.
    pub fn quarantined(child_ref: ResourceRef, uid: ResourceUid) -> Self {
        Self {
            child_ref,
            uid,
            identity_verified: false,
        }
    }
}

/// Result of adopting or quarantining the observed controller children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerChildAdoption {
    pub adopted: usize,
    pub quarantined: usize,
}

/// Context passed from ProviderDeployment to the fixed Process adapter for a
/// static controller launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerLaunchContext {
    resource: ControllerProcessResource,
    target_readiness_digest: SchemaFingerprint,
}

impl ControllerLaunchContext {
    /// Borrow the generated Process resource.
    pub const fn resource(&self) -> &ControllerProcessResource {
        &self.resource
    }

    /// Borrow the target readiness commitment bound into the LaunchTicket.
    pub const fn target_readiness_digest(&self) -> &SchemaFingerprint {
        &self.target_readiness_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildDisposition {
    Adopted,
    Quarantined,
}

#[derive(Debug)]
struct ControllerRecord {
    resource: ControllerProcessResource,
    phase: ControllerProcessPhase,
    controller_permit: Option<AdmissionPermit>,
    target_ready: bool,
    launch_identity: Option<[u8; 32]>,
    target_readiness_digest: Option<SchemaFingerprint>,
    ready_session: Option<ControllerSessionBinding>,
    last_session_generation: Option<ReconnectGeneration>,
    assignments: BTreeMap<ControllerAssignmentIdentity, Arc<AssignmentState>>,
    children: BTreeMap<ResourceRef, (ResourceUid, ChildDisposition)>,
    finalizer_held: bool,
}

/// The immutable identity of one controller assignment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControllerAssignmentKey {
    pub zone: ZoneId,
    pub provider: ResourceRef,
    pub target: ResourceRef,
    pub provider_generation: u64,
    pub controller_generation: u64,
    pub session_generation: u64,
    pub assignment_epoch: u64,
}

impl ControllerAssignmentKey {
    pub fn validate(&self, mode: DaemonMode) -> Result<(), DeploymentError> {
        if self.provider_generation == 0
            || self.controller_generation == 0
            || self.session_generation == 0
            || self.assignment_epoch == 0
        {
            return Err(DeploymentError::GenerationZero);
        }
        let expected = mode.target_kind();
        let actual = match self.target.resource_type().as_str() {
            "Host" => TargetKind::Host,
            "Guest" => TargetKind::Guest,
            _ => return Err(DeploymentError::TargetWrongKind),
        };
        if actual != expected {
            return Err(DeploymentError::TargetWrongKind);
        }
        if self.provider.resource_type().as_str() != "Provider" {
            return Err(DeploymentError::ProviderWrongKind);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct AssignmentState {
    active: AtomicBool,
    permit: Mutex<Option<AdmissionPermit>>,
}

impl AssignmentState {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(true),
            permit: Mutex::new(None),
        }
    }

    fn install_permit(&self, permit: AdmissionPermit) -> Result<(), DeploymentError> {
        let mut slot = self
            .permit
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        *slot = Some(permit);
        Ok(())
    }

    fn revoke(&self) {
        self.active.store(false, Ordering::Release);
        if let Ok(mut slot) = self.permit.lock() {
            if let Some(permit) = slot.take() {
                permit.release();
            }
        }
    }
}

/// A revocable controller assignment lease.
#[derive(Debug)]
pub struct AssignmentLease {
    key: ControllerAssignmentKey,
    state: Arc<AssignmentState>,
    registry: Arc<Mutex<BTreeMap<ControllerAssignmentKey, Arc<AssignmentState>>>>,
    _controller_permit: AdmissionPermit,
}

impl AssignmentLease {
    pub fn key(&self) -> &ControllerAssignmentKey {
        &self.key
    }

    pub fn is_active(&self) -> bool {
        self.state.active.load(Ordering::Acquire)
    }

    pub fn revoke(&self) {
        self.state.revoke();
        unregister_assignment(&self.registry, &self.key, &self.state);
    }
}

impl Drop for AssignmentLease {
    fn drop(&mut self) {
        self.state.revoke();
        unregister_assignment(&self.registry, &self.key, &self.state);
    }
}

/// A live authenticated controller session.
///
/// Dropping the lease revokes every assignment for this exact controller
/// session generation. A later reconnect must authenticate again and receives
/// a new assignment epoch.
#[derive(Debug)]
pub struct ControllerSessionLease {
    process_ref: ResourceRef,
    binding: ControllerSessionBinding,
    deployment: ProviderDeployment,
}

impl ControllerSessionLease {
    /// Borrow the authenticated session binding.
    pub const fn binding(&self) -> &ControllerSessionBinding {
        &self.binding
    }

    /// Return the authenticated controller session generation.
    pub const fn generation(&self) -> ReconnectGeneration {
        self.binding.session_generation
    }

    /// Borrow the controller Process reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Return whether this exact controller session still admits work.
    pub fn is_active(&self) -> bool {
        self.deployment
            .controller_session_is_active(&self.process_ref, self.binding.session_generation())
    }
}

impl Drop for ControllerSessionLease {
    fn drop(&mut self) {
        let _ = self
            .deployment
            .revoke_controller_session(&self.process_ref, self.binding.session_generation);
    }
}

/// A scoped ResourceClient assignment held by one ready controller session.
#[derive(Debug)]
pub struct ControllerAssignmentLease {
    identity: ControllerAssignmentIdentity,
    state: Arc<AssignmentState>,
    assignments: Arc<Mutex<BTreeMap<ControllerAssignmentIdentity, Arc<AssignmentState>>>>,
    controllers: Arc<Mutex<BTreeMap<ResourceRef, Arc<Mutex<ControllerRecord>>>>>,
    resource_types: BTreeSet<ResourceTypeName>,
    _controller_permit: AdmissionPermit,
}

impl ControllerAssignmentLease {
    /// Borrow the complete assignment identity.
    pub const fn identity(&self) -> &ControllerAssignmentIdentity {
        &self.identity
    }

    /// Return whether this assignment may still mutate or observe.
    pub fn is_active(&self) -> bool {
        self.state.active.load(Ordering::Acquire)
    }

    /// Borrow the ResourceTypes this controller client may query.
    pub fn resource_types(&self) -> &BTreeSet<ResourceTypeName> {
        &self.resource_types
    }

    /// Return whether a closed controller verb is in scope for this client.
    pub fn allows(&self, verb: ControllerResourceVerb) -> bool {
        self.is_active() && CONTROLLER_ALLOWED_RESOURCE_VERBS.contains(&verb)
    }

    /// Revoke this exact assignment.
    pub fn revoke(&self) {
        self.state.revoke();
        unregister_controller_assignment(
            &self.assignments,
            &self.controllers,
            &self.identity,
            &self.state,
        );
    }
}

impl Drop for ControllerAssignmentLease {
    fn drop(&mut self) {
        self.state.revoke();
        unregister_controller_assignment(
            &self.assignments,
            &self.controllers,
            &self.identity,
            &self.state,
        );
    }
}

fn unregister_controller_assignment(
    assignments: &Mutex<BTreeMap<ControllerAssignmentIdentity, Arc<AssignmentState>>>,
    controllers: &Mutex<BTreeMap<ResourceRef, Arc<Mutex<ControllerRecord>>>>,
    identity: &ControllerAssignmentIdentity,
    state: &Arc<AssignmentState>,
) {
    if let Ok(mut current) = assignments.lock()
        && current
            .get(identity)
            .is_some_and(|candidate| Arc::ptr_eq(candidate, state))
    {
        current.remove(identity);
    }
    if let Ok(current) = controllers.lock()
        && let Some(record) = current.get(identity.process_ref())
        && let Ok(mut record) = record.lock()
    {
        record.assignments.retain(|candidate, candidate_state| {
            candidate != identity || !Arc::ptr_eq(candidate_state, state)
        });
        if record.assignments.is_empty() && record.phase == ControllerProcessPhase::Assigned {
            record.phase = if record.ready_session.is_some() {
                ControllerProcessPhase::Ready
            } else {
                ControllerProcessPhase::Revoked
            };
        }
    }
}

fn unregister_assignment(
    registry: &Mutex<BTreeMap<ControllerAssignmentKey, Arc<AssignmentState>>>,
    key: &ControllerAssignmentKey,
    state: &Arc<AssignmentState>,
) {
    if let Ok(mut assignments) = registry.lock()
        && assignments
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, state))
    {
        assignments.remove(key);
    }
}

/// Shared ProviderDeployment behavior used by Host and Guest compositions.
#[derive(Debug, Clone)]
pub struct ProviderDeployment {
    mode: DaemonMode,
    admission: AdmissionBudget,
    assignments: Arc<Mutex<BTreeMap<ControllerAssignmentKey, Arc<AssignmentState>>>>,
    controllers: Arc<Mutex<BTreeMap<ResourceRef, Arc<Mutex<ControllerRecord>>>>>,
    controller_assignments:
        Arc<Mutex<BTreeMap<ControllerAssignmentIdentity, Arc<AssignmentState>>>>,
    next_assignment_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl ProviderDeployment {
    pub fn new(mode: DaemonMode, limits: AdmissionLimits) -> Result<Self, AdmissionError> {
        Ok(Self {
            mode,
            admission: AdmissionBudget::new(limits)?,
            assignments: Arc::new(Mutex::new(BTreeMap::new())),
            controllers: Arc::new(Mutex::new(BTreeMap::new())),
            controller_assignments: Arc::new(Mutex::new(BTreeMap::new())),
            next_assignment_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    pub const fn mode(&self) -> DaemonMode {
        self.mode
    }

    pub const fn target_kind(&self) -> TargetKind {
        self.mode.target_kind()
    }

    pub fn admission(&self) -> &AdmissionBudget {
        &self.admission
    }

    /// Admit exactly one target-scoped controller assignment.
    ///
    /// The controller reservation is acquired before inserting any assignment
    /// record, so a flood cannot allocate unbounded per-controller state.
    pub fn admit_assignment(
        &self,
        key: ControllerAssignmentKey,
    ) -> Result<AssignmentLease, DeploymentError> {
        key.validate(self.mode)?;
        let permit = self
            .admission
            .try_admit(AdmissionKind::Controller)
            .map_err(DeploymentError::Admission)?;
        let mut assignments = self
            .assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if assignments.contains_key(&key) {
            return Err(DeploymentError::AssignmentAlreadyActive);
        }
        let state = Arc::new(AssignmentState::new());
        state.install_permit(permit.clone())?;
        assignments.insert(key.clone(), Arc::clone(&state));
        Ok(AssignmentLease {
            key,
            state,
            registry: Arc::clone(&self.assignments),
            _controller_permit: permit,
        })
    }

    /// Revoke every assignment bound to one ComponentSession generation.
    pub fn revoke_session(&self, session_generation: u64) -> Result<usize, DeploymentError> {
        if session_generation == 0 {
            return Err(DeploymentError::GenerationZero);
        }
        let mut count = 0usize;
        let mut assignments = self
            .assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        let keys = assignments
            .keys()
            .filter(|key| key.session_generation == session_generation)
            .cloned()
            .collect::<Vec<_>>();
        for key in &keys {
            if let Some(state) = assignments.remove(key) {
                state.revoke();
                count = count.saturating_add(1);
            }
        }
        drop(assignments);
        let records = self
            .controllers
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for record in records {
            let mut record = record
                .lock()
                .map_err(|_| DeploymentError::StateUnavailable)?;
            let matches = record
                .ready_session
                .as_ref()
                .is_some_and(|binding| binding.session_generation().get() == session_generation);
            if matches {
                count = count.saturating_add(revoke_record_assignments(
                    &self.controller_assignments,
                    &mut record,
                    ReconnectGeneration::new(session_generation)
                        .map_err(|_| DeploymentError::GenerationZero)?,
                )?);
                record.ready_session = None;
                if record.phase != ControllerProcessPhase::Released {
                    record.phase = ControllerProcessPhase::Revoked;
                }
            }
        }
        Ok(count)
    }

    /// Revoke every assignment before replacing the target's generation.
    pub fn revoke_target(&self, target: &ResourceRef) -> Result<usize, DeploymentError> {
        let mut assignments = self
            .assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        let keys = assignments
            .keys()
            .filter(|key| &key.target == target)
            .cloned()
            .collect::<Vec<_>>();
        for key in &keys {
            if let Some(state) = assignments.remove(key) {
                state.revoke();
            }
        }
        let mut count = keys.len();
        drop(assignments);
        let records = self
            .controllers
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for record in records {
            let mut record = record
                .lock()
                .map_err(|_| DeploymentError::StateUnavailable)?;
            if record.resource.target() == target {
                count = count.saturating_add(revoke_record_assignments_all(
                    &self.controller_assignments,
                    &mut record,
                ));
                record.ready_session = None;
                if record.phase != ControllerProcessPhase::Released {
                    record.phase = ControllerProcessPhase::Revoked;
                }
            }
        }
        Ok(count)
    }

    /// Revoke every assignment and controller instance for one Provider
    /// generation before a Provider replacement is admitted.
    pub fn revoke_provider(
        &self,
        provider: &ResourceRef,
        provider_generation: u64,
    ) -> Result<usize, DeploymentError> {
        if provider_generation == 0 {
            return Err(DeploymentError::GenerationZero);
        }
        let mut assignments = self
            .assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        let keys = assignments
            .keys()
            .filter(|key| {
                &key.provider == provider && key.provider_generation == provider_generation
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut count = keys.len();
        for key in keys {
            if let Some(state) = assignments.remove(&key) {
                state.revoke();
            }
        }
        drop(assignments);
        let records = self
            .controllers
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for record in records {
            let mut record = record
                .lock()
                .map_err(|_| DeploymentError::StateUnavailable)?;
            if &record.resource.provider_ref == provider
                && record.resource.provider_generation.get() == provider_generation
            {
                count = count.saturating_add(revoke_record_assignments_all(
                    &self.controller_assignments,
                    &mut record,
                ));
                record.ready_session = None;
                if record.phase != ControllerProcessPhase::Released {
                    record.phase = ControllerProcessPhase::Revoked;
                }
            }
        }
        Ok(count)
    }

    pub fn active_assignments(&self) -> Result<usize, DeploymentError> {
        let assignments = self
            .assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        let legacy = assignments
            .values()
            .filter(|state| state.active.load(Ordering::Acquire))
            .count();
        drop(assignments);
        Ok(legacy.saturating_add(self.active_controller_assignments()?))
    }

    /// Return the number of active controller ResourceClient assignments.
    pub fn active_controller_assignments(&self) -> Result<usize, DeploymentError> {
        let assignments = self
            .controller_assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        Ok(assignments
            .values()
            .filter(|state| state.active.load(Ordering::Acquire))
            .count())
    }

    /// Create one signed target-local controller Process resource.
    ///
    /// The returned resource is only an intent. No executable is resolved and
    /// no process is spawned until [`Self::begin_controller_launch`] is
    /// admitted by the fixed Process adapter.
    #[allow(clippy::too_many_arguments)]
    pub fn create_controller_process(
        &self,
        zone: ZoneId,
        provider_ref: ResourceRef,
        descriptor: &ComponentDescriptor,
        resource_generation: ResourceGeneration,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        target_session_generation: ReconnectGeneration,
        resource_revision: ZoneRevision,
        target: ResourceRef,
        process_provider_ref: ResourceRef,
        target_ready: bool,
    ) -> Result<ControllerProcessResource, DeploymentError> {
        validate_controller_descriptor(
            self.mode,
            &provider_ref,
            descriptor,
            &target,
            &process_provider_ref,
            resource_generation,
            provider_generation,
            controller_generation,
            target_session_generation,
            resource_revision,
        )?;
        let process_name = controller_process_name(
            &zone,
            &provider_ref,
            descriptor.component_id(),
            &target,
            provider_generation,
            controller_generation,
            target_session_generation,
        );
        let process_ref = ResourceRef::new(
            ResourceTypeName::parse("Process")
                .map_err(|_| DeploymentError::ControllerDescriptorInvalid)?,
            ResourceName::parse(process_name)
                .map_err(|_| DeploymentError::ControllerDescriptorInvalid)?,
        );
        let controller_role_ref = ResourceRef::new(
            ResourceTypeName::parse("Process")
                .map_err(|_| DeploymentError::ControllerDescriptorInvalid)?,
            ResourceName::parse(descriptor.component_id().as_str())
                .map_err(|_| DeploymentError::ControllerDescriptorInvalid)?,
        );
        let uid = controller_process_uid(
            &zone,
            &provider_ref,
            descriptor.component_id(),
            &target,
            provider_generation,
            controller_generation,
            target_session_generation,
        )?;
        let process_spec = ProcessSpec::minimal(
            ExecutionSpec::minimal(
                target.clone(),
                ProcessClass::Controller,
                descriptor.component_id().clone(),
            )
            .map_err(|_| DeploymentError::ControllerDescriptorInvalid)?,
        );
        let target_kind = controller_target_kind(&target)?;
        let capability = descriptor
            .target_capability(target_kind)
            .ok_or(DeploymentError::ControllerDescriptorInvalid)?;
        let resource = ControllerProcessResource {
            process_ref: process_ref.clone(),
            controller_role_ref,
            uid,
            zone,
            provider_ref,
            component_id: descriptor.component_id().clone(),
            target,
            process_provider_ref,
            process_spec,
            resource_generation,
            resource_revision,
            provider_generation,
            controller_generation,
            target_session_generation,
            signed_descriptor_digest: descriptor.config_digest().clone(),
            artifact_digest: capability.artifact_digest().clone(),
            required_effect_classes: capability.required_effect_classes().clone(),
            owned_resource_types: descriptor.exported_resource_types().clone(),
            repair_owner: process_ref.clone(),
        };
        let controller_permit = self
            .admission
            .try_admit(AdmissionKind::Controller)
            .map_err(DeploymentError::Admission)?;
        let mut controllers = self
            .controllers
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if controllers.contains_key(&resource.process_ref) {
            return Err(DeploymentError::ControllerAlreadyDeployed);
        }
        controllers.insert(
            resource.process_ref.clone(),
            Arc::new(Mutex::new(ControllerRecord {
                resource: resource.clone(),
                phase: ControllerProcessPhase::Pending,
                controller_permit: Some(controller_permit),
                target_ready,
                launch_identity: None,
                target_readiness_digest: None,
                ready_session: None,
                last_session_generation: None,
                assignments: BTreeMap::new(),
                children: BTreeMap::new(),
                finalizer_held: true,
            })),
        );
        Ok(resource)
    }

    /// Return the current lifecycle phase for one controller Process.
    pub fn controller_phase(&self, process_ref: &ResourceRef) -> Option<ControllerProcessPhase> {
        self.controllers
            .lock()
            .ok()
            .and_then(|controllers| controllers.get(process_ref).cloned())
            .and_then(|record| record.lock().ok().map(|record| record.phase))
    }

    fn controller_session_is_active(
        &self,
        process_ref: &ResourceRef,
        session_generation: ReconnectGeneration,
    ) -> bool {
        self.controllers
            .lock()
            .ok()
            .and_then(|controllers| controllers.get(process_ref).cloned())
            .and_then(|record| {
                record.lock().ok().map(|record| {
                    matches!(
                        record.phase,
                        ControllerProcessPhase::Ready | ControllerProcessPhase::Assigned
                    ) && record
                        .ready_session
                        .as_ref()
                        .is_some_and(|binding| binding.session_generation() == session_generation)
                })
            })
            .unwrap_or(false)
    }

    /// Return the signed target-local controller Process resources currently
    /// owned by this deployment.
    pub fn controller_processes(&self) -> Result<Vec<ControllerProcessResource>, DeploymentError> {
        let records = self
            .controllers
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        records
            .into_iter()
            .map(|record| {
                record
                    .lock()
                    .map_err(|_| DeploymentError::StateUnavailable)
                    .map(|record| record.resource.clone())
            })
            .collect()
    }

    /// Borrow one signed target-local controller Process resource.
    pub fn controller_process(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<ControllerProcessResource, DeploymentError> {
        self.controller_record(process_ref)?
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)
            .map(|record| record.resource.clone())
    }

    /// Begin one target-local controller launch after target readiness passed.
    pub fn begin_controller_launch(
        &self,
        process_ref: &ResourceRef,
        target_readiness_digest: SchemaFingerprint,
    ) -> Result<ControllerLaunchContext, DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if !record.target_ready {
            return Err(DeploymentError::ControllerTargetNotReady);
        }
        if digest_is_zero(target_readiness_digest.as_str()) {
            return Err(DeploymentError::ControllerIdentityInvalid);
        }
        if record.phase != ControllerProcessPhase::Pending {
            return Err(DeploymentError::ControllerLaunchInvalid);
        }
        record.phase = ControllerProcessPhase::Launching;
        record.target_readiness_digest = Some(target_readiness_digest.clone());
        Ok(ControllerLaunchContext {
            resource: record.resource.clone(),
            target_readiness_digest,
        })
    }

    /// Record a fixed Process adapter launch receipt.
    pub fn controller_launch_succeeded(
        &self,
        process_ref: &ResourceRef,
        identity: [u8; 32],
    ) -> Result<(), DeploymentError> {
        if identity == [0; 32] {
            return Err(DeploymentError::ControllerIdentityInvalid);
        }
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if record.phase != ControllerProcessPhase::Launching {
            return Err(DeploymentError::ControllerLaunchInvalid);
        }
        if record
            .launch_identity
            .is_some_and(|existing| existing != identity)
        {
            return Err(DeploymentError::ControllerIdentityChanged);
        }
        record.launch_identity = Some(identity);
        record.phase = ControllerProcessPhase::Running;
        Ok(())
    }

    /// Record a restart adoption receipt for an existing controller Process.
    pub fn controller_adopted(
        &self,
        process_ref: &ResourceRef,
        identity: [u8; 32],
    ) -> Result<(), DeploymentError> {
        if identity == [0; 32] {
            return Err(DeploymentError::ControllerIdentityInvalid);
        }
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if !matches!(
            record.phase,
            ControllerProcessPhase::Pending
                | ControllerProcessPhase::Launching
                | ControllerProcessPhase::Running
                | ControllerProcessPhase::Revoked
        ) {
            return Err(DeploymentError::ControllerLaunchInvalid);
        }
        if record
            .launch_identity
            .is_some_and(|existing| existing != identity)
        {
            return Err(DeploymentError::ControllerIdentityChanged);
        }
        record.launch_identity = Some(identity);
        record.phase = ControllerProcessPhase::Running;
        Ok(())
    }

    /// Return a failed controller launch to Pending, or quarantine it when
    /// the Process adapter cannot establish an unambiguous identity.
    pub fn controller_launch_failed(
        &self,
        process_ref: &ResourceRef,
        quarantine: bool,
    ) -> Result<(), DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if record.phase != ControllerProcessPhase::Launching {
            return Err(DeploymentError::ControllerLaunchInvalid);
        }
        record.phase = if quarantine {
            ControllerProcessPhase::Quarantined
        } else {
            ControllerProcessPhase::Pending
        };
        if !quarantine {
            record.target_readiness_digest = None;
        }
        Ok(())
    }

    /// Quarantine a controller whose restart adoption found ambiguous
    /// process identity. Quarantine retains the finalizer and blocks cleanup.
    pub fn quarantine_controller(&self, process_ref: &ResourceRef) -> Result<(), DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        record.phase = ControllerProcessPhase::Quarantined;
        record.ready_session = None;
        revoke_record_assignments_all(&self.controller_assignments, &mut record);
        Ok(())
    }

    /// Whether the controller Process finalizer is still retained.
    pub fn controller_finalizer_held(&self, process_ref: &ResourceRef) -> Option<bool> {
        self.controllers
            .lock()
            .ok()
            .and_then(|controllers| controllers.get(process_ref).cloned())
            .and_then(|record| record.lock().ok().map(|record| record.finalizer_held))
    }

    /// Admit a controller's separate authenticated ComponentSession.
    pub fn admit_controller_session(
        &self,
        binding: ControllerSessionBinding,
    ) -> Result<ControllerSessionLease, DeploymentError> {
        let record = self.controller_record(binding.process_ref())?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        validate_controller_session(&record.resource, &binding)?;
        if record
            .target_readiness_digest
            .as_ref()
            .is_some_and(|expected| expected != binding.readiness_digest())
        {
            return Err(DeploymentError::ControllerSessionMismatch);
        }
        if record.phase == ControllerProcessPhase::Released
            || record.phase == ControllerProcessPhase::Quarantined
            || record.phase == ControllerProcessPhase::Launching
            || record.phase == ControllerProcessPhase::Pending
        {
            return Err(DeploymentError::ControllerNotReady);
        }
        if let Some(previous) = record.ready_session.as_ref() {
            if binding.session_generation() <= previous.session_generation() {
                return Err(DeploymentError::ControllerSessionStale);
            }
            let previous_generation = previous.session_generation();
            revoke_record_assignments(
                &self.controller_assignments,
                &mut record,
                previous_generation,
            )?;
            record.phase = ControllerProcessPhase::Revoked;
        } else if let Some(previous) = record.last_session_generation
            && binding.session_generation() <= previous
        {
            return Err(DeploymentError::ControllerSessionStale);
        }
        record.last_session_generation = Some(binding.session_generation());
        record.ready_session = Some(binding.clone());
        record.phase = if record.assignments.is_empty() {
            ControllerProcessPhase::Ready
        } else {
            ControllerProcessPhase::Assigned
        };
        Ok(ControllerSessionLease {
            process_ref: binding.process_ref().clone(),
            binding,
            deployment: self.clone(),
        })
    }

    /// Admit one ResourceClient only after the controller session is ready.
    pub fn admit_controller_assignment(
        &self,
        request: ControllerAssignmentRequest,
    ) -> Result<ControllerAssignmentLease, DeploymentError> {
        let record = self.controller_record(&request.process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        let ready = record
            .ready_session
            .as_ref()
            .ok_or(DeploymentError::ControllerNotReady)?;
        if record.phase != ControllerProcessPhase::Ready
            && record.phase != ControllerProcessPhase::Assigned
        {
            return Err(DeploymentError::ControllerNotReady);
        }
        if request.provider_ref != record.resource.provider_ref
            || request.target != record.resource.target
            || request.session_generation != ready.session_generation()
            || request.session_generation
                != record
                    .last_session_generation
                    .ok_or(DeploymentError::ControllerSessionMismatch)?
            || request.resource_ref.resource_type().as_str() == "Provider"
        {
            return Err(DeploymentError::ControllerSessionMismatch);
        }
        if !record
            .resource
            .owned_resource_types
            .contains(request.resource_ref.resource_type())
        {
            return Err(DeploymentError::ControllerResourceTypeUnowned);
        }
        if request.resource_generation.get() == 0
            || request.resource_revision.get() == 0
            || request.resource_uid.as_str().is_empty()
        {
            return Err(DeploymentError::ControllerAssignmentInvalid);
        }
        if record.assignments.keys().any(|identity| {
            identity.resource_ref == request.resource_ref
                && identity.resource_uid == request.resource_uid
                && identity.resource_generation == request.resource_generation
        }) {
            return Err(DeploymentError::ControllerAssignmentConflict);
        }
        let permit = self
            .admission
            .try_admit(AdmissionKind::Controller)
            .map_err(DeploymentError::Admission)?;
        let epoch = self
            .next_assignment_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| DeploymentError::AssignmentEpochExhausted)?
            .saturating_add(1);
        let identity = ControllerAssignmentIdentity {
            process_ref: request.process_ref,
            resource_ref: request.resource_ref,
            resource_uid: request.resource_uid,
            resource_generation: request.resource_generation,
            resource_revision: request.resource_revision,
            provider_ref: request.provider_ref,
            target: request.target,
            provider_generation: record.resource.provider_generation,
            controller_generation: record.resource.controller_generation,
            session_generation: request.session_generation,
            assignment_epoch: epoch,
        };
        let state = Arc::new(AssignmentState::new());
        let mut assignments = self
            .controller_assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if assignments.contains_key(&identity) {
            return Err(DeploymentError::ControllerAssignmentConflict);
        }
        state.install_permit(permit.clone())?;
        assignments.insert(identity.clone(), Arc::clone(&state));
        record
            .assignments
            .insert(identity.clone(), Arc::clone(&state));
        record.phase = ControllerProcessPhase::Assigned;
        Ok(ControllerAssignmentLease {
            identity,
            state,
            assignments: Arc::clone(&self.controller_assignments),
            controllers: Arc::clone(&self.controllers),
            resource_types: record.resource.owned_resource_types.clone(),
            _controller_permit: permit,
        })
    }

    /// Revoke every controller assignment and session bound to one session.
    pub fn revoke_controller_session(
        &self,
        process_ref: &ResourceRef,
        session_generation: ReconnectGeneration,
    ) -> Result<usize, DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if record
            .ready_session
            .as_ref()
            .is_none_or(|session| session.session_generation() != session_generation)
        {
            return Ok(0);
        }
        let count = revoke_record_assignments(
            &self.controller_assignments,
            &mut record,
            session_generation,
        )?;
        record.ready_session = None;
        if record.phase != ControllerProcessPhase::Released {
            record.phase = ControllerProcessPhase::Revoked;
        }
        Ok(count)
    }

    /// Record one child resource under the controller's finalizer owner.
    pub fn record_controller_child(
        &self,
        process_ref: &ResourceRef,
        child_ref: ResourceRef,
        child_uid: ResourceUid,
    ) -> Result<(), DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if !record.finalizer_held {
            return Err(DeploymentError::ControllerReleased);
        }
        if child_ref == *process_ref {
            return Err(DeploymentError::ControllerChildAmbiguous);
        }
        if let Some((existing_uid, _)) = record.children.get(&child_ref)
            && existing_uid != &child_uid
        {
            record.phase = ControllerProcessPhase::Quarantined;
            return Err(DeploymentError::ControllerChildAmbiguous);
        }
        record
            .children
            .insert(child_ref, (child_uid, ChildDisposition::Quarantined));
        Ok(())
    }

    /// Adopt verified children or quarantine ambiguous observations.
    pub fn adopt_controller_children(
        &self,
        process_ref: &ResourceRef,
        observations: impl IntoIterator<Item = ControllerChildObservation>,
    ) -> Result<ControllerChildAdoption, DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if !record.finalizer_held {
            return Err(DeploymentError::ControllerReleased);
        }
        let mut result = ControllerChildAdoption {
            adopted: 0,
            quarantined: 0,
        };
        for observation in observations {
            let Some((expected_uid, disposition)) = record.children.get_mut(&observation.child_ref)
            else {
                record.phase = ControllerProcessPhase::Quarantined;
                return Err(DeploymentError::ControllerChildAmbiguous);
            };
            if expected_uid != &observation.uid || !observation.identity_verified {
                *disposition = ChildDisposition::Quarantined;
                result.quarantined = result.quarantined.saturating_add(1);
                record.phase = ControllerProcessPhase::Quarantined;
            } else {
                *disposition = ChildDisposition::Adopted;
                result.adopted = result.adopted.saturating_add(1);
            }
        }
        Ok(result)
    }

    /// Remove one child only after exact adopted identity and terminal state.
    pub fn remove_controller_child(
        &self,
        process_ref: &ResourceRef,
        child_ref: ResourceRef,
        child_uid: &ResourceUid,
    ) -> Result<(), DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        let Some((expected_uid, disposition)) = record.children.get(&child_ref) else {
            return Err(DeploymentError::ControllerChildMissing);
        };
        if expected_uid != child_uid {
            return Err(DeploymentError::ControllerChildAmbiguous);
        }
        if *disposition != ChildDisposition::Adopted {
            return Err(DeploymentError::ControllerChildQuarantined);
        }
        record.children.remove(&child_ref);
        Ok(())
    }

    /// Prepare controller cleanup without releasing its finalizer.
    pub fn prepare_controller_cleanup(
        &self,
        process_ref: &ResourceRef,
        repair_owner: &ResourceRef,
    ) -> Result<(), DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if repair_owner != &record.resource.repair_owner {
            return Err(DeploymentError::ControllerRepairOwnerMismatch);
        }
        if !record.finalizer_held {
            return Err(DeploymentError::ControllerFinalizerMissing);
        }
        if !record.assignments.is_empty() {
            return Err(DeploymentError::ControllerAssignmentActive);
        }
        if !record.children.is_empty() {
            return Err(DeploymentError::ControllerChildrenUnresolved);
        }
        if record.phase == ControllerProcessPhase::Quarantined {
            return Err(DeploymentError::ControllerChildQuarantined);
        }
        if !matches!(
            record.phase,
            ControllerProcessPhase::Revoked | ControllerProcessPhase::Draining
        ) {
            return Err(DeploymentError::ControllerCleanupInvalid);
        }
        record.phase = ControllerProcessPhase::Draining;
        Ok(())
    }

    /// Complete cleanup and clear the finalizer only for the named owner.
    pub fn complete_controller_cleanup(
        &self,
        process_ref: &ResourceRef,
        repair_owner: &ResourceRef,
    ) -> Result<(), DeploymentError> {
        let record = self.controller_record(process_ref)?;
        let mut record = record
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        if repair_owner != &record.resource.repair_owner {
            return Err(DeploymentError::ControllerRepairOwnerMismatch);
        }
        if record.phase != ControllerProcessPhase::Draining
            || !record.children.is_empty()
            || !record.assignments.is_empty()
        {
            return Err(DeploymentError::ControllerCleanupInvalid);
        }
        record.finalizer_held = false;
        if let Some(permit) = record.controller_permit.take() {
            permit.release();
        }
        record.phase = ControllerProcessPhase::Released;
        Ok(())
    }

    fn controller_record(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<Arc<Mutex<ControllerRecord>>, DeploymentError> {
        self.controllers
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?
            .get(process_ref)
            .cloned()
            .ok_or(DeploymentError::ControllerNotFound)
    }
}

fn controller_target_kind(target: &ResourceRef) -> Result<ControllerTargetKind, DeploymentError> {
    match target.resource_type().as_str() {
        "Host" => Ok(ControllerTargetKind::Host),
        "Guest" => Ok(ControllerTargetKind::Guest),
        _ => Err(DeploymentError::TargetWrongKind),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_controller_descriptor(
    mode: DaemonMode,
    provider_ref: &ResourceRef,
    descriptor: &ComponentDescriptor,
    target: &ResourceRef,
    process_provider_ref: &ResourceRef,
    resource_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    target_session_generation: ReconnectGeneration,
    resource_revision: ZoneRevision,
) -> Result<(), DeploymentError> {
    if provider_ref.resource_type().as_str() != "Provider"
        || process_provider_ref.resource_type().as_str() != "Provider"
        || !matches!(
            descriptor.execution(),
            ComponentExecution::Launchable { .. }
        )
        || descriptor.component_type() != ComponentType::Controller
        || resource_generation.get() == 0
        || provider_generation.get() == 0
        || controller_generation.get() == 0
        || target_session_generation.get() == 0
        || resource_revision.get() == 0
    {
        return Err(DeploymentError::ControllerDescriptorInvalid);
    }
    let target_kind = controller_target_kind(target)?;
    let expected_target = match mode {
        DaemonMode::Host => ControllerTargetKind::Host,
        DaemonMode::Guest => ControllerTargetKind::Guest,
    };
    if target_kind != expected_target
        || !descriptor.supported_target_kinds().contains(&target_kind)
        || !matches!(
            descriptor.instance_scope(),
            Some(
                ControllerInstanceScope::FixedExecutionTarget
                    | ControllerInstanceScope::PerResourceTarget
            )
        )
    {
        return Err(DeploymentError::TargetWrongKind);
    }
    let Some(capability) = descriptor.target_capability(target_kind) else {
        return Err(DeploymentError::ControllerDescriptorInvalid);
    };
    if digest_is_zero(capability.artifact_digest().as_str())
        || digest_is_zero(descriptor.config_digest().as_str())
        || !capability
            .required_effect_classes()
            .contains(&EffectPortClass::Process)
    {
        return Err(DeploymentError::ControllerDescriptorInvalid);
    }
    Ok(())
}

fn controller_process_name(
    zone: &ZoneId,
    provider_ref: &ResourceRef,
    component_id: &d2b_contracts_resource::v3::execution_policy::BoundedToken,
    target: &ResourceRef,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    target_session_generation: ReconnectGeneration,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"d2b-controller-process-v1\0");
    digest.update(zone.as_str().as_bytes());
    digest.update([0]);
    digest.update(provider_ref.to_canonical_string().as_bytes());
    digest.update([0]);
    digest.update(component_id.as_str().as_bytes());
    digest.update([0]);
    digest.update(target.to_canonical_string().as_bytes());
    digest.update(provider_generation.get().to_be_bytes());
    digest.update(controller_generation.get().to_be_bytes());
    digest.update(target_session_generation.get().to_be_bytes());
    let digest = digest.finalize();
    format!(
        "controller-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]
    )
}

fn controller_process_uid(
    zone: &ZoneId,
    provider_ref: &ResourceRef,
    component_id: &d2b_contracts_resource::v3::execution_policy::BoundedToken,
    target: &ResourceRef,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    target_session_generation: ReconnectGeneration,
) -> Result<ResourceUid, DeploymentError> {
    let mut digest = Sha256::new();
    digest.update(b"d2b-controller-process-uid-v1\0");
    digest.update(zone.as_str().as_bytes());
    digest.update([0]);
    digest.update(provider_ref.to_canonical_string().as_bytes());
    digest.update([0]);
    digest.update(component_id.as_str().as_bytes());
    digest.update([0]);
    digest.update(target.to_canonical_string().as_bytes());
    digest.update(provider_generation.get().to_be_bytes());
    digest.update(controller_generation.get().to_be_bytes());
    digest.update(target_session_generation.get().to_be_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let rendered = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    ResourceUid::parse(rendered).map_err(|_| DeploymentError::ControllerDescriptorInvalid)
}

fn validate_controller_session(
    resource: &ControllerProcessResource,
    binding: &ControllerSessionBinding,
) -> Result<(), DeploymentError> {
    if binding.process_ref() != resource.process_ref()
        || binding.zone() != resource.zone()
        || binding.provider_ref() != resource.provider_ref()
        || binding.target() != resource.target()
        || binding.provider_generation() != resource.provider_generation()
        || binding.controller_generation() != resource.controller_generation()
        || binding.target_session_generation() != resource.target_session_generation()
        || binding.session_generation().get() == 0
        || digest_is_zero(binding.readiness_digest().as_str())
    {
        return Err(DeploymentError::ControllerSessionMismatch);
    }
    Ok(())
}

fn digest_is_zero(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|digest| digest.bytes().all(|byte| byte == b'0'))
}

fn revoke_record_assignments(
    assignments: &Mutex<BTreeMap<ControllerAssignmentIdentity, Arc<AssignmentState>>>,
    record: &mut ControllerRecord,
    session_generation: ReconnectGeneration,
) -> Result<usize, DeploymentError> {
    let keys = record
        .assignments
        .keys()
        .filter(|identity| identity.session_generation == session_generation)
        .cloned()
        .collect::<Vec<_>>();
    let count = keys.len();
    for key in &keys {
        if let Some(state) = record.assignments.get(key) {
            state.revoke();
        }
    }
    let mut assignments = assignments
        .lock()
        .map_err(|_| DeploymentError::StateUnavailable)?;
    for key in keys {
        assignments.remove(&key);
        record.assignments.remove(&key);
    }
    Ok(count)
}

fn revoke_record_assignments_all(
    assignments: &Mutex<BTreeMap<ControllerAssignmentIdentity, Arc<AssignmentState>>>,
    record: &mut ControllerRecord,
) -> usize {
    let keys = record.assignments.keys().cloned().collect::<Vec<_>>();
    let count = keys.len();
    for key in &keys {
        if let Some(state) = record.assignments.get(key) {
            state.revoke();
        }
    }
    if let Ok(mut assignments) = assignments.lock() {
        for key in &keys {
            assignments.remove(key);
        }
    }
    for key in keys {
        record.assignments.remove(&key);
    }
    count
}

/// ProviderDeployment refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentError {
    Admission(AdmissionError),
    GenerationZero,
    TargetWrongKind,
    ProviderWrongKind,
    AssignmentAlreadyActive,
    StateUnavailable,
    ControllerDescriptorInvalid,
    ControllerTargetNotReady,
    ControllerAlreadyDeployed,
    ControllerLaunchInvalid,
    ControllerIdentityInvalid,
    ControllerIdentityChanged,
    ControllerNotFound,
    ControllerNotReady,
    ControllerSessionMismatch,
    ControllerSessionStale,
    ControllerAssignmentInvalid,
    ControllerResourceTypeUnowned,
    ControllerAssignmentConflict,
    ControllerAssignmentActive,
    AssignmentEpochExhausted,
    ControllerReleased,
    ControllerChildAmbiguous,
    ControllerChildMissing,
    ControllerChildQuarantined,
    ControllerChildrenUnresolved,
    ControllerRepairOwnerMismatch,
    ControllerFinalizerMissing,
    ControllerCleanupInvalid,
}

impl std::fmt::Display for DeploymentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Admission(error) => return error.fmt(formatter),
            Self::GenerationZero => "provider-deployment-generation-zero",
            Self::TargetWrongKind => "provider-deployment-target-wrong-kind",
            Self::ProviderWrongKind => "provider-deployment-provider-wrong-kind",
            Self::AssignmentAlreadyActive => "provider-deployment-assignment-active",
            Self::StateUnavailable => "provider-deployment-state-unavailable",
            Self::ControllerDescriptorInvalid => {
                "provider-deployment-controller-descriptor-invalid"
            }
            Self::ControllerTargetNotReady => "provider-deployment-controller-target-not-ready",
            Self::ControllerAlreadyDeployed => "provider-deployment-controller-already-deployed",
            Self::ControllerLaunchInvalid => "provider-deployment-controller-launch-invalid",
            Self::ControllerIdentityInvalid => "provider-deployment-controller-identity-invalid",
            Self::ControllerIdentityChanged => "provider-deployment-controller-identity-changed",
            Self::ControllerNotFound => "provider-deployment-controller-not-found",
            Self::ControllerNotReady => "provider-deployment-controller-not-ready",
            Self::ControllerSessionMismatch => "provider-deployment-controller-session-mismatch",
            Self::ControllerSessionStale => "provider-deployment-controller-session-stale",
            Self::ControllerAssignmentInvalid => {
                "provider-deployment-controller-assignment-invalid"
            }
            Self::ControllerResourceTypeUnowned => {
                "provider-deployment-controller-resource-type-unowned"
            }
            Self::ControllerAssignmentConflict => {
                "provider-deployment-controller-assignment-conflict"
            }
            Self::ControllerAssignmentActive => "provider-deployment-controller-assignment-active",
            Self::AssignmentEpochExhausted => "provider-deployment-assignment-epoch-exhausted",
            Self::ControllerReleased => "provider-deployment-controller-released",
            Self::ControllerChildAmbiguous => "provider-deployment-controller-child-ambiguous",
            Self::ControllerChildMissing => "provider-deployment-controller-child-missing",
            Self::ControllerChildQuarantined => "provider-deployment-controller-child-quarantined",
            Self::ControllerChildrenUnresolved => {
                "provider-deployment-controller-children-unresolved"
            }
            Self::ControllerRepairOwnerMismatch => {
                "provider-deployment-controller-repair-owner-mismatch"
            }
            Self::ControllerFinalizerMissing => "provider-deployment-controller-finalizer-missing",
            Self::ControllerCleanupInvalid => "provider-deployment-controller-cleanup-invalid",
        })
    }
}

impl std::error::Error for DeploymentError {}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{ResourceName, ResourceTypeName};

    fn resource(kind: &str, name: &str) -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse(kind).expect("resource type"),
            ResourceName::parse(name).expect("resource name"),
        )
    }

    fn assignment(session_generation: u64) -> ControllerAssignmentKey {
        ControllerAssignmentKey {
            zone: ZoneId::parse("work").expect("zone"),
            provider: resource("Provider", "system-systemd"),
            target: resource("Guest", "workload"),
            provider_generation: 1,
            controller_generation: 1,
            session_generation,
            assignment_epoch: 1,
        }
    }

    #[test]
    fn mode_is_closed_and_surfaces_are_not_widenable() {
        assert_eq!(DaemonMode::parse("host"), Ok(DaemonMode::Host));
        assert_eq!(DaemonMode::parse("guest"), Ok(DaemonMode::Guest));
        assert!(DaemonMode::parse("both").is_err());
        assert!(!DaemonMode::Guest.surfaces().local_zone_store);
        assert!(!DaemonMode::Guest.surfaces().public_operator_socket);
        assert!(!DaemonMode::Guest.surfaces().realm_credentials);
        assert!(DaemonMode::Guest.surfaces().parent_component_session);
    }

    #[test]
    fn admission_refuses_before_controller_state_is_allocated() {
        let budget = AdmissionBudget::new(AdmissionLimits {
            max_sessions: 1,
            max_reconnects_per_window: 1,
            max_controllers: 1,
            max_watches: 1,
            max_streams: 1,
            reconnect_window: Duration::from_secs(60),
        })
        .expect("valid limits");
        let first = budget
            .try_admit(AdmissionKind::Controller)
            .expect("first controller");
        assert!(matches!(
            budget.try_admit(AdmissionKind::Controller),
            Err(AdmissionError::LimitExceeded(AdmissionKind::Controller))
        ));
        drop(first);
        assert!(budget.try_admit(AdmissionKind::Controller).is_ok());
    }

    #[test]
    fn disconnect_revokes_all_assignments_for_the_session_generation() {
        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let lease = deployment
            .admit_assignment(assignment(7))
            .expect("assignment");
        assert!(lease.is_active());
        assert_eq!(deployment.revoke_session(7).expect("revoke"), 1);
        assert!(!lease.is_active());
        assert_eq!(deployment.active_assignments().expect("count"), 0);
    }

    #[test]
    fn assignment_drop_releases_controller_slot_and_key() {
        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let lease = deployment
            .admit_assignment(assignment(7))
            .expect("assignment");
        drop(lease);
        assert_eq!(deployment.active_assignments().expect("count"), 0);
        assert!(deployment.admit_assignment(assignment(7)).is_ok());
    }

    #[test]
    fn guest_cannot_admit_a_host_target_or_non_provider_controller() {
        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let mut host = assignment(1);
        host.target = resource("Host", "host-system");
        assert!(matches!(
            deployment.admit_assignment(host),
            Err(DeploymentError::TargetWrongKind)
        ));

        let mut wrong_provider = assignment(1);
        wrong_provider.provider = resource("Process", "controller");
        assert!(matches!(
            deployment.admit_assignment(wrong_provider),
            Err(DeploymentError::ProviderWrongKind)
        ));
    }

    fn signed_controller_descriptor() -> d2b_contracts_provider::v3::ComponentDescriptor {
        use d2b_contracts_provider::v3::{
            ArtifactDigest, BinaryRef, ComponentTargetCapability, ComponentType,
            ControllerInstanceScope, ControllerTargetKind, EffectPortClass,
        };
        use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};

        let digest = ArtifactDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap();
        ComponentDescriptor::new(
            BoundedToken::parse("process-controller").unwrap(),
            ComponentType::Controller,
            [ResourceTypeName::parse("Process").unwrap()],
            [BoundedToken::parse("reconcile").unwrap()],
            [ExecutionDomain::System],
            8,
            digest.clone(),
            [],
            false,
        )
        .unwrap()
        .with_execution(d2b_contracts_provider::v3::ComponentExecution::Launchable {
            binary_ref: BinaryRef::parse("process-controller").unwrap(),
        })
        .with_controller_placement(
            ControllerInstanceScope::PerResourceTarget,
            [ControllerTargetKind::Host, ControllerTargetKind::Guest],
        )
        .unwrap()
        .with_target_capabilities([ComponentTargetCapability::new(
            ControllerTargetKind::Guest,
            digest,
            [EffectPortClass::Process],
        )
        .unwrap()])
        .unwrap()
    }

    #[test]
    fn target_local_controller_launch_requires_ready_session_before_assignment() {
        use d2b_contracts_resource::v3::{
            ControllerGeneration, ResourceGeneration, ResourceUid, ZoneRevision,
            identity::ReconnectGeneration,
        };

        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let provider = resource("Provider", "runtime");
        let process_provider = resource("Provider", "system-systemd");
        let target = resource("Guest", "workload");
        let process = deployment
            .create_controller_process(
                ZoneId::parse("work").unwrap(),
                provider.clone(),
                &signed_controller_descriptor(),
                ResourceGeneration::new(3).unwrap(),
                ResourceGeneration::new(7).unwrap(),
                ControllerGeneration::new(4).unwrap(),
                ReconnectGeneration::new(2).unwrap(),
                ZoneRevision::new(11),
                target.clone(),
                process_provider,
                true,
            )
            .expect("controller process");

        assert_eq!(
            process.process_spec().execution().process_class(),
            ProcessClass::Controller
        );
        assert_eq!(
            process.controller_role_ref(),
            &resource("Process", "process-controller")
        );
        let readiness = d2b_contracts_resource::v3::SchemaFingerprint::parse(format!(
            "sha256:{}",
            "b".repeat(64)
        ))
        .unwrap();
        let launch = deployment
            .begin_controller_launch(process.process_ref(), readiness.clone())
            .expect("launch admission");
        assert_eq!(launch.resource().process_ref(), process.process_ref());
        assert!(matches!(
            deployment.admit_controller_assignment(ControllerAssignmentRequest::new(
                process.process_ref().clone(),
                resource("Process", "owned"),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ZoneRevision::new(12),
                provider.clone(),
                target.clone(),
                ReconnectGeneration::new(2).unwrap(),
            )),
            Err(DeploymentError::ControllerNotReady)
        ));

        deployment
            .controller_launch_succeeded(process.process_ref(), [9; 32])
            .expect("launch success");
        let session = deployment
            .admit_controller_session(ControllerSessionBinding::new(
                process.process_ref().clone(),
                process.zone().clone(),
                provider.clone(),
                target.clone(),
                process.provider_generation(),
                process.controller_generation(),
                process.target_session_generation(),
                ReconnectGeneration::new(5).unwrap(),
                readiness,
            ))
            .expect("controller session");
        let assignment = deployment
            .admit_controller_assignment(ControllerAssignmentRequest::new(
                process.process_ref().clone(),
                resource("Process", "owned"),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ZoneRevision::new(12),
                provider,
                target,
                session.generation(),
            ))
            .expect("scoped assignment");
        assert!(assignment.is_active());
        assert_eq!(assignment.identity().resource_generation().get(), 1);
        assert_eq!(assignment.identity().assignment_epoch(), 1);
        assert_eq!(deployment.active_controller_assignments().unwrap(), 1);
        drop(session);
        assert!(!assignment.is_active());
        assert_eq!(deployment.active_controller_assignments().unwrap(), 0);
        let reconnect = deployment
            .admit_controller_session(ControllerSessionBinding::new(
                process.process_ref().clone(),
                process.zone().clone(),
                process.provider_ref().clone(),
                process.target().clone(),
                process.provider_generation(),
                process.controller_generation(),
                process.target_session_generation(),
                ReconnectGeneration::new(6).unwrap(),
                d2b_contracts_resource::v3::SchemaFingerprint::parse(format!(
                    "sha256:{}",
                    "b".repeat(64)
                ))
                .unwrap(),
            ))
            .expect("reconnect");
        let replacement = deployment
            .admit_controller_assignment(ControllerAssignmentRequest::new(
                process.process_ref().clone(),
                resource("Process", "owned"),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ZoneRevision::new(12),
                process.provider_ref().clone(),
                process.target().clone(),
                reconnect.generation(),
            ))
            .expect("reconnected assignment");
        assert_eq!(replacement.identity().assignment_epoch(), 2);
        assert!(replacement.is_active());
        drop(reconnect);
        assert!(!replacement.is_active());
    }

    #[test]
    fn controller_cleanup_requires_verified_children_and_exact_repair_owner() {
        use d2b_contracts_resource::v3::{
            ControllerGeneration, ResourceGeneration, ResourceUid, ZoneRevision,
            identity::ReconnectGeneration,
        };

        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let process = deployment
            .create_controller_process(
                ZoneId::parse("work").unwrap(),
                resource("Provider", "runtime"),
                &signed_controller_descriptor(),
                ResourceGeneration::new(3).unwrap(),
                ResourceGeneration::new(7).unwrap(),
                ControllerGeneration::new(4).unwrap(),
                ReconnectGeneration::new(2).unwrap(),
                ZoneRevision::new(11),
                resource("Guest", "workload"),
                resource("Provider", "system-systemd"),
                true,
            )
            .expect("controller process");
        deployment
            .begin_controller_launch(
                process.process_ref(),
                d2b_contracts_resource::v3::SchemaFingerprint::parse(format!(
                    "sha256:{}",
                    "b".repeat(64)
                ))
                .unwrap(),
            )
            .unwrap();
        deployment
            .controller_launch_succeeded(process.process_ref(), [9; 32])
            .unwrap();
        let session = deployment
            .admit_controller_session(ControllerSessionBinding::new(
                process.process_ref().clone(),
                process.zone().clone(),
                process.provider_ref().clone(),
                process.target().clone(),
                process.provider_generation(),
                process.controller_generation(),
                process.target_session_generation(),
                ReconnectGeneration::new(5).unwrap(),
                d2b_contracts_resource::v3::SchemaFingerprint::parse(format!(
                    "sha256:{}",
                    "b".repeat(64)
                ))
                .unwrap(),
            ))
            .unwrap();
        let child_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        deployment
            .record_controller_child(
                process.process_ref(),
                resource("Process", "child"),
                child_uid.clone(),
            )
            .unwrap();
        deployment
            .revoke_session(session.generation().get())
            .expect("revoke session");
        assert!(
            deployment
                .prepare_controller_cleanup(process.process_ref(), process.process_ref())
                .is_err()
        );
        deployment
            .adopt_controller_children(
                process.process_ref(),
                [ControllerChildObservation::verified(
                    resource("Process", "child"),
                    child_uid.clone(),
                )],
            )
            .unwrap();
        deployment
            .remove_controller_child(
                process.process_ref(),
                resource("Process", "child"),
                &child_uid,
            )
            .unwrap();
        deployment
            .prepare_controller_cleanup(process.process_ref(), process.process_ref())
            .expect("cleanup owner");
        deployment
            .complete_controller_cleanup(process.process_ref(), process.process_ref())
            .expect("cleanup complete");
        assert_eq!(
            deployment.controller_phase(process.process_ref()),
            Some(ControllerProcessPhase::Released)
        );
    }

    #[test]
    fn ambiguous_controller_child_stays_quarantined_and_keeps_finalizer() {
        use d2b_contracts_resource::v3::{
            ControllerGeneration, ResourceGeneration, ResourceUid, ZoneRevision,
            identity::ReconnectGeneration,
        };

        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let process = deployment
            .create_controller_process(
                ZoneId::parse("work").unwrap(),
                resource("Provider", "runtime"),
                &signed_controller_descriptor(),
                ResourceGeneration::new(3).unwrap(),
                ResourceGeneration::new(7).unwrap(),
                ControllerGeneration::new(4).unwrap(),
                ReconnectGeneration::new(2).unwrap(),
                ZoneRevision::new(11),
                resource("Guest", "workload"),
                resource("Provider", "system-systemd"),
                true,
            )
            .expect("controller process");
        deployment
            .record_controller_child(
                process.process_ref(),
                resource("Process", "child"),
                ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            )
            .unwrap();
        deployment
            .adopt_controller_children(
                process.process_ref(),
                [ControllerChildObservation::quarantined(
                    resource("Process", "child"),
                    ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
                )],
            )
            .unwrap();
        assert_eq!(
            deployment.controller_phase(process.process_ref()),
            Some(ControllerProcessPhase::Quarantined)
        );
        assert!(matches!(
            deployment.remove_controller_child(
                process.process_ref(),
                resource("Process", "child"),
                &ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            ),
            Err(DeploymentError::ControllerChildQuarantined)
        ));
        assert!(
            deployment
                .controller_finalizer_held(process.process_ref())
                .unwrap()
        );
    }

    #[test]
    fn target_session_reconnect_gets_a_new_controller_process_identity() {
        use d2b_contracts_resource::v3::{
            ControllerGeneration, ResourceGeneration, ZoneRevision, identity::ReconnectGeneration,
        };

        let deployment =
            ProviderDeployment::new(DaemonMode::Guest, AdmissionLimits::guest_default())
                .expect("deployment");
        let provider = resource("Provider", "runtime");
        let target = resource("Guest", "workload");
        let first = deployment
            .create_controller_process(
                ZoneId::parse("work").unwrap(),
                provider.clone(),
                &signed_controller_descriptor(),
                ResourceGeneration::new(1).unwrap(),
                ResourceGeneration::new(2).unwrap(),
                ControllerGeneration::new(3).unwrap(),
                ReconnectGeneration::new(4).unwrap(),
                ZoneRevision::new(5),
                target.clone(),
                resource("Provider", "system-systemd"),
                true,
            )
            .unwrap();
        let second = deployment
            .create_controller_process(
                ZoneId::parse("work").unwrap(),
                provider,
                &signed_controller_descriptor(),
                ResourceGeneration::new(1).unwrap(),
                ResourceGeneration::new(2).unwrap(),
                ControllerGeneration::new(3).unwrap(),
                ReconnectGeneration::new(5).unwrap(),
                ZoneRevision::new(6),
                target,
                resource("Provider", "system-systemd"),
                true,
            )
            .unwrap();
        assert_ne!(first.process_ref(), second.process_ref());
        assert_ne!(first.uid(), second.uid());
        assert_eq!(first.controller_role_ref(), second.controller_role_ref());
    }
}
