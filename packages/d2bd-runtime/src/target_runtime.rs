//! Provider-neutral target runtime and bounded admission contracts.
//!
//! The composition crate selects concrete Providers and effect adapters. This
//! module owns the authority-neutral lifecycle that both Host and Guest
//! instances use: fixed process-start mode, bounded admission, target-scoped
//! controller assignments, and revocation on session loss.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use d2b_contracts_broker::broker_wire::BrokerProfile;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};

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
}

impl ProviderDeployment {
    pub fn new(mode: DaemonMode, limits: AdmissionLimits) -> Result<Self, AdmissionError> {
        Ok(Self {
            mode,
            admission: AdmissionBudget::new(limits)?,
            assignments: Arc::new(Mutex::new(BTreeMap::new())),
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
            }
        }
        Ok(keys.len())
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
        Ok(keys.len())
    }

    pub fn active_assignments(&self) -> Result<usize, DeploymentError> {
        let assignments = self
            .assignments
            .lock()
            .map_err(|_| DeploymentError::StateUnavailable)?;
        Ok(assignments
            .values()
            .filter(|state| state.active.load(Ordering::Acquire))
            .count())
    }
}

/// ProviderDeployment refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentError {
    Admission(AdmissionError),
    GenerationZero,
    TargetWrongKind,
    ProviderWrongKind,
    AssignmentAlreadyActive,
    StateUnavailable,
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
}
