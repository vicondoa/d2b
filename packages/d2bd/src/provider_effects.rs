//! Descriptor-bound Provider lifecycle effects.
//!
//! The daemon owns the lifecycle dispatcher, but it does not own a second
//! broker protocol.  A caller supplies a typed effect port and this module
//! performs only the Zone, caller-role, and idempotency admission that belongs
//! at the Provider boundary.  The production port is implemented by `d2bd`
//! with the existing typed broker dispatch functions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_broker::broker_wire::{BrokerRequest, BrokerResponse};
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid, ZoneId};
use d2b_process_conformance::LaunchTicket;
use d2b_resource_api::AuthorizationLease;
use d2b_resource_store::AdmittedVerb;
use d2bd_runtime::{
    broker_transport::{ModeBoundBrokerAdapter, ModeBoundBrokerError},
    target_runtime::DaemonMode,
};

/// Provider-neutral EffectPort classes wired by the static d2bd composition
/// root. These are capability classes, not request names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderEffectClass {
    Runtime,
    ProcessLaunch,
    ProcessObserve,
    ProcessSignal,
}

impl ProviderEffectClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::ProcessLaunch => "process-launch",
            Self::ProcessObserve => "process-observe",
            Self::ProcessSignal => "process-signal",
        }
    }
}

/// Static, mode-bound Provider effect adapter.
#[derive(Debug, Clone)]
pub struct FixedEffectAdapter {
    mode: DaemonMode,
    broker: ModeBoundBrokerAdapter,
}

impl FixedEffectAdapter {
    /// Construct the adapter from one closed daemon mode.
    pub fn for_mode(mode: DaemonMode, socket_path: PathBuf, daemon_uid: u32) -> Self {
        match mode {
            DaemonMode::Host => Self::host(socket_path, daemon_uid),
            DaemonMode::Guest => Self::guest(socket_path, daemon_uid),
        }
    }

    pub fn host(socket_path: PathBuf, daemon_uid: u32) -> Self {
        Self {
            mode: DaemonMode::Host,
            broker: ModeBoundBrokerAdapter::host(socket_path, daemon_uid),
        }
    }

    pub fn guest(socket_path: PathBuf, daemon_uid: u32) -> Self {
        Self {
            mode: DaemonMode::Guest,
            broker: ModeBoundBrokerAdapter::guest(socket_path, daemon_uid),
        }
    }

    pub const fn mode(&self) -> DaemonMode {
        self.mode
    }

    pub const fn broker_profile(&self) -> d2b_contracts_broker::broker_wire::BrokerProfile {
        self.broker.profile()
    }

    /// Validate the fixed socket instance before any ticket is delivered.
    pub fn validate_instance(&self) -> Result<(), FixedEffectError> {
        self.broker
            .validate_instance()
            .map_err(FixedEffectError::Broker)
    }

    pub fn allows(&self, class: ProviderEffectClass) -> bool {
        match self.mode {
            DaemonMode::Host => true,
            DaemonMode::Guest => matches!(
                class,
                ProviderEffectClass::Runtime
                    | ProviderEffectClass::ProcessLaunch
                    | ProviderEffectClass::ProcessObserve
                    | ProviderEffectClass::ProcessSignal
            ),
        }
    }

    pub fn dispatch(
        &self,
        class: ProviderEffectClass,
        request: BrokerRequest,
        timeout: Option<Duration>,
    ) -> Result<BrokerResponse, FixedEffectError> {
        if !self.allows(class) {
            return Err(FixedEffectError::EffectClassDenied(class));
        }
        self.broker
            .dispatch(request, timeout)
            .map_err(FixedEffectError::Broker)
    }

    /// Validate a signed target-local controller LaunchTicket against this
    /// adapter's fixed daemon mode before it reaches a Process Provider.
    ///
    /// The ticket carries target-session evidence only. ResourceClient
    /// assignment is admitted separately by ProviderDeployment after the
    /// controller authenticates and publishes readiness.
    pub fn validate_controller_ticket(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<(), FixedEffectError> {
        self.validate_instance()?;
        ticket
            .validate_controller_launch()
            .map_err(|_| FixedEffectError::ControllerTicketInvalid)?;
        let expected_target = match self.mode {
            DaemonMode::Host => "Host",
            DaemonMode::Guest => "Guest",
        };
        if ticket.execution_ref().resource_type().as_str() != expected_target {
            return Err(FixedEffectError::ControllerTargetDenied);
        }
        if !self.allows(ProviderEffectClass::ProcessLaunch) {
            return Err(FixedEffectError::EffectClassDenied(
                ProviderEffectClass::ProcessLaunch,
            ));
        }
        Ok(())
    }
}

/// Refusal at the fixed EffectPort boundary.
#[derive(Debug)]
pub enum FixedEffectError {
    EffectClassDenied(ProviderEffectClass),
    ControllerTicketInvalid,
    ControllerTargetDenied,
    Broker(ModeBoundBrokerError),
}

impl std::fmt::Display for FixedEffectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EffectClassDenied(class) => {
                write!(formatter, "provider-effect-class-denied:{}", class.as_str())
            }
            Self::ControllerTicketInvalid => {
                formatter.write_str("provider-controller-ticket-invalid")
            }
            Self::ControllerTargetDenied => {
                formatter.write_str("provider-controller-target-denied")
            }
            Self::Broker(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FixedEffectError {}

/// Maximum retained lifecycle mutation keys.
pub const MAX_TRACKED_LIFECYCLE_MUTATIONS: usize = 256;
/// Idempotency entries are retained only long enough to cover one bounded
/// lifecycle retry window.
pub const LIFECYCLE_IDEMPOTENCY_TTL: Duration = Duration::from_secs(300);

/// Closed caller roles allowed to request a Provider lifecycle effect.
///
/// The daemon reuses the broker wire role rather than defining a second
/// caller-role enum.  `NotAuthorized` is the only refusal state; the broker
/// has already classified every other variant from its authenticated peer.
fn caller_is_authorized(caller: &BrokerCallerRole) -> bool {
    !matches!(caller, BrokerCallerRole::NotAuthorized)
}

/// Guest lifecycle operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLifecycleOperation {
    /// Start the Guest runtime.
    Start,
    /// Stop the Guest runtime.
    Stop,
    /// Fence a stop followed by a fresh start.
    Restart,
}

impl GuestLifecycleOperation {
    /// Stable operation token used in idempotency diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }
}

/// The immutable identity and authorization evidence carried by one Guest
/// lifecycle request.
#[derive(Clone, PartialEq, Eq)]
pub struct LifecycleAuthorization {
    subject_uid: Option<ResourceUid>,
    zone_uid: ResourceUid,
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    guest_generation: ResourceGeneration,
    provider_assignment_generation: ResourceGeneration,
    policy_revision: u64,
    operation_id: String,
    stop_only: bool,
}

impl core::fmt::Debug for LifecycleAuthorization {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LifecycleAuthorization")
            .field("has_subject_uid", &self.subject_uid.is_some())
            .field("has_zone_uid", &true)
            .field("has_guest_uid", &true)
            .field("guest_generation", &"<redacted>")
            .field("provider_assignment_generation", &"<redacted>")
            .field("policy_revision", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("stop_only", &self.stop_only)
            .finish()
    }
}

impl LifecycleAuthorization {
    /// Consume the Resource API's sealed authorization lease at the Provider
    /// boundary after checking it against the current Guest identity.
    pub fn from_lease(
        lease: AuthorizationLease,
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        guest_generation: ResourceGeneration,
        provider_assignment_generation: ResourceGeneration,
    ) -> Result<Self, ProviderEffectError> {
        if lease.operation() != AdmittedVerb::UpdateSpec
            || guest_ref.resource_type().as_str() != "Guest"
            || lease.object_uid() != Some(&guest_uid)
            || lease.object_generation() != Some(guest_generation)
            || lease.provider_assignment_generation() != Some(provider_assignment_generation)
            || lease.policy_revision() == 0
            || lease.operation_id().is_empty()
            || lease.operation_id().len() > 128
            || lease.operation_id().chars().any(char::is_control)
        {
            return Err(ProviderEffectError::AuthorizationLeaseInvalid);
        }
        Ok(Self {
            subject_uid: Some(lease.subject_uid().clone()),
            zone_uid: lease.zone_uid().clone(),
            guest_ref,
            guest_uid,
            guest_generation,
            provider_assignment_generation,
            policy_revision: lease.policy_revision(),
            operation_id: lease.operation_id().to_owned(),
            stop_only: false,
        })
    }

    /// Construct the distinct, non-transferable host-shutdown capability.
    ///
    /// This constructor is crate-visible so only the daemon's guarded
    /// shutdown hook can create it; it is never represented as an admin
    /// authorization.
    pub(crate) fn host_shutdown(
        zone_uid: ResourceUid,
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        guest_generation: ResourceGeneration,
        provider_assignment_generation: ResourceGeneration,
        policy_revision: u64,
        operation_id: String,
    ) -> Result<Self, ProviderEffectError> {
        if policy_revision == 0
            || operation_id.is_empty()
            || operation_id.len() > 128
            || operation_id.chars().any(char::is_control)
        {
            return Err(ProviderEffectError::AuthorizationLeaseInvalid);
        }
        Ok(Self {
            subject_uid: None,
            zone_uid,
            guest_ref,
            guest_uid,
            guest_generation,
            provider_assignment_generation,
            policy_revision,
            operation_id,
            stop_only: true,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        zone_uid: &str,
        guest_uid: &str,
        guest_generation: u64,
        provider_assignment_generation: u64,
        policy_revision: u64,
        operation_id: &str,
    ) -> Self {
        Self::for_test_with_guest(
            "Guest/workstation",
            zone_uid,
            guest_uid,
            guest_generation,
            provider_assignment_generation,
            policy_revision,
            operation_id,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_with_guest(
        guest_ref: &str,
        zone_uid: &str,
        guest_uid: &str,
        guest_generation: u64,
        provider_assignment_generation: u64,
        policy_revision: u64,
        operation_id: &str,
    ) -> Self {
        Self {
            subject_uid: Some(
                ResourceUid::parse("33333333-3333-4333-8333-333333333333")
                    .expect("test subject UID"),
            ),
            zone_uid: ResourceUid::parse(zone_uid).expect("test Zone UID"),
            guest_ref: ResourceRef::parse(guest_ref).expect("test Guest ref"),
            guest_uid: ResourceUid::parse(guest_uid).expect("test Guest UID"),
            guest_generation: ResourceGeneration::new(guest_generation)
                .expect("test Guest generation"),
            provider_assignment_generation: ResourceGeneration::new(provider_assignment_generation)
                .expect("test Provider assignment generation"),
            policy_revision,
            operation_id: operation_id.to_owned(),
            stop_only: false,
        }
    }

    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    pub const fn guest_generation(&self) -> ResourceGeneration {
        self.guest_generation
    }

    pub const fn provider_assignment_generation(&self) -> ResourceGeneration {
        self.provider_assignment_generation
    }

    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub const fn subject_uid(&self) -> Option<&ResourceUid> {
        self.subject_uid.as_ref()
    }

    pub const fn is_stop_only(&self) -> bool {
        self.stop_only
    }
}

/// A v3 Guest lifecycle request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLifecycleRequest {
    zone: ZoneId,
    guest: ResourceRef,
    operation: GuestLifecycleOperation,
    idempotency_key: String,
    authorization: LifecycleAuthorization,
}

impl GuestLifecycleRequest {
    /// Construct a request addressed by a same-Zone Guest ResourceRef.
    pub fn new(
        zone: ZoneId,
        guest: ResourceRef,
        operation: GuestLifecycleOperation,
        idempotency_key: impl Into<String>,
        authorization: LifecycleAuthorization,
    ) -> Result<Self, ProviderEffectError> {
        if guest.resource_type().as_str() != "Guest" {
            return Err(ProviderEffectError::GuestRefInvalid);
        }
        let idempotency_key = idempotency_key.into();
        if idempotency_key.is_empty() || idempotency_key.len() > 128 {
            return Err(ProviderEffectError::IdempotencyKeyInvalid);
        }
        if authorization.guest_uid.as_str().is_empty()
            || authorization.guest_ref.resource_type().as_str() != "Guest"
            || authorization.guest_ref != guest
            || authorization.guest_generation.get() == 0
            || authorization.provider_assignment_generation.get() == 0
            || authorization.policy_revision == 0
            || authorization.operation_id.is_empty()
            || authorization.operation_id.len() > 128
            || authorization.operation_id != idempotency_key
        {
            return Err(ProviderEffectError::AuthorizationLeaseInvalid);
        }
        if authorization.stop_only && operation != GuestLifecycleOperation::Stop {
            return Err(ProviderEffectError::StopOnlyLease);
        }
        Ok(Self {
            zone,
            guest,
            operation,
            idempotency_key,
            authorization,
        })
    }

    /// Borrow the Zone context.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the Guest ResourceRef.
    pub const fn guest(&self) -> &ResourceRef {
        &self.guest
    }

    /// Return the requested operation.
    pub const fn operation(&self) -> GuestLifecycleOperation {
        self.operation
    }

    /// Borrow the deduplication key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Borrow the consumed lifecycle authorization identity.
    pub const fn authorization(&self) -> &LifecycleAuthorization {
        &self.authorization
    }
}

/// Provider lifecycle effect-port failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderEffectError {
    /// The caller role cannot dispatch lifecycle effects.
    CallerRoleDenied,
    /// The request did not target a Guest.
    GuestRefInvalid,
    /// The Zone context does not match the request's resource route.
    ZoneMismatch,
    /// The key is empty or over its fixed bound.
    IdempotencyKeyInvalid,
    /// The bounded deduplication table is full.
    MutationTableFull,
    /// The same key was used for a different request.
    IdempotencyConflict,
    /// The dispatcher state could not be read or updated.
    StateUnavailable,
    /// The request cannot claim lifecycle ownership because another request
    /// for the Guest is executing or a newer desired generation is pending.
    MutationPending,
    /// The configured Provider registry is unavailable.
    RegistryUnavailable,
    /// No registered Provider owns the requested Guest route.
    ProviderNotRegistered,
    /// The selected Provider does not publish the requested lifecycle method.
    ProviderCapabilityDenied,
    /// The typed effect port refused or failed the mutation.
    EffectRejected,
    /// The request did not carry valid sealed authorization evidence.
    AuthorizationLeaseInvalid,
    /// Durable state belongs to a different immutable Guest identity.
    IdentityMismatch,
    /// A stop-only HostShutdown capability attempted a broader operation.
    StopOnlyLease,
}

impl ProviderEffectError {
    /// Stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::CallerRoleDenied => "provider-effect-caller-role-denied",
            Self::GuestRefInvalid => "provider-effect-guest-ref-invalid",
            Self::ZoneMismatch => "provider-effect-zone-mismatch",
            Self::IdempotencyKeyInvalid => "provider-effect-idempotency-key-invalid",
            Self::MutationTableFull => "provider-effect-mutation-table-full",
            Self::IdempotencyConflict => "provider-effect-idempotency-conflict",
            Self::StateUnavailable => "provider-effect-state-unavailable",
            Self::MutationPending => "provider-effect-mutation-pending",
            Self::RegistryUnavailable => "provider-effect-registry-unavailable",
            Self::ProviderNotRegistered => "provider-effect-provider-not-registered",
            Self::ProviderCapabilityDenied => "provider-effect-provider-capability-denied",
            Self::EffectRejected => "provider-effect-rejected",
            Self::AuthorizationLeaseInvalid => "provider-effect-authorization-lease-invalid",
            Self::IdentityMismatch => "provider-effect-identity-mismatch",
            Self::StopOnlyLease => "provider-effect-stop-only-lease",
        }
    }
}

impl core::fmt::Display for ProviderEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderEffectError {}

/// Result of admitting a lifecycle request without invoking an effect port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleDispatch {
    /// The request was newly admitted and may invoke its effect port.
    Dispatch,
    /// A durable pending request must be reconciled with the actual lifecycle
    /// state before invoking its effect port.
    Reconcile,
    /// Another in-process execution owns the Guest or a newer generation is
    /// pending.
    Pending,
    /// The exact request was already applied under this idempotency key.
    Duplicate,
}

/// The authoritative lifecycle state observed by a Provider effect port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLifecycleState {
    /// The Guest runtime is started.
    Started,
    /// The Guest runtime is stopped.
    Stopped,
}

impl GuestLifecycleState {
    fn satisfies(self, operation: GuestLifecycleOperation) -> bool {
        matches!(
            (self, operation),
            (Self::Started, GuestLifecycleOperation::Start)
                | (Self::Stopped, GuestLifecycleOperation::Stop)
        )
    }
}

/// Result of invoking a typed lifecycle effect.
#[derive(Debug, PartialEq, Eq)]
pub enum EffectDispatch<T> {
    /// The effect port ran and returned its typed output.
    Dispatched(T),
    /// The exact request was already applied, including after reconciliation,
    /// and the effect was not invoked.
    Duplicate,
}

/// Typed Provider lifecycle effect port supplied by the daemon composition
/// layer.
///
/// Implementations must route the request through an existing typed effect
/// adapter.  This trait intentionally has no socket, path, argv, or raw
/// broker payload surface.
pub trait ProviderLifecycleEffectPort {
    /// The successful effect result.
    type Output;

    /// Observe the authoritative lifecycle state for one request.
    ///
    /// The default refuses reconciliation rather than treating an unknown
    /// state as applied.  Production ports must source this from the real
    /// lifecycle boundary or a durable downstream idempotency result.
    fn actual_state(
        &self,
        _request: &GuestLifecycleRequest,
    ) -> Result<GuestLifecycleState, ProviderEffectError> {
        Err(ProviderEffectError::StateUnavailable)
    }

    /// Apply one already-admitted lifecycle request.
    fn apply(&self, request: &GuestLifecycleRequest) -> Result<Self::Output, ProviderEffectError>;
}

/// Effect-free lifecycle dispatcher with bounded idempotency tracking,
/// immutable Guest-identity ownership, and monotonic desired-state
/// generations.
#[derive(Debug)]
pub struct ProviderLifecycleDispatch {
    zone: ZoneId,
    mutations: Mutex<BTreeMap<String, LifecycleMutation>>,
    state_path: Option<PathBuf>,
    next_desired_generation: AtomicU64,
}

#[derive(Debug, Clone)]
struct LifecycleMutation {
    guest: ResourceRef,
    operation: GuestLifecycleOperation,
    authorization: Option<LifecycleAuthorization>,
    admitted_at_ms: u64,
    desired_generation: u64,
    status: LifecycleMutationStatus,
    executing: bool,
    quarantined: bool,
    quarantine_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LifecycleIdentityKey {
    zone_uid: ResourceUid,
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    guest_generation: ResourceGeneration,
    provider_assignment_generation: ResourceGeneration,
    policy_revision: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum LifecycleMutationStatus {
    #[default]
    Pending,
    Applied,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedLifecycleMutation {
    key: String,
    guest: String,
    operation: String,
    #[serde(alias = "admitted_at_ms")]
    admitted_at_ms: u64,
    /// Monotonic desired-state generation for this lifecycle admission.
    #[serde(default, alias = "desired_generation")]
    desired_generation: u64,
    /// Missing status is treated as pending for migration from the original
    /// admission-only persistence format.
    #[serde(default)]
    status: LifecycleMutationStatus,
    /// Complete immutable authorization identity. Missing fields identify
    /// pre-U10 rows, which are retained as quarantine records.
    #[serde(default)]
    subject_uid: Option<String>,
    #[serde(default)]
    zone_uid: Option<String>,
    #[serde(default)]
    guest_uid: Option<String>,
    #[serde(default)]
    guest_generation: Option<u64>,
    #[serde(default)]
    provider_assignment_generation: Option<u64>,
    #[serde(default)]
    policy_revision: Option<u64>,
    #[serde(default)]
    operation_id: Option<String>,
    #[serde(default)]
    stop_only: Option<bool>,
    #[serde(default)]
    quarantined: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quarantine_reason: Option<String>,
}

impl ProviderLifecycleDispatch {
    /// Construct a dispatcher for one Zone.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            mutations: Mutex::new(BTreeMap::new()),
            state_path: None,
            next_desired_generation: AtomicU64::new(0),
        }
    }

    /// Construct a dispatcher backed by a daemon-owned durable state file.
    pub fn new_persistent(
        zone: ZoneId,
        state_path: impl Into<PathBuf>,
    ) -> Result<Self, ProviderEffectError> {
        let state_path = state_path.into();
        if !state_path.is_absolute() {
            return Err(ProviderEffectError::StateUnavailable);
        }
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).map_err(|_| ProviderEffectError::StateUnavailable)?;
        }
        let mut mutations = BTreeMap::new();
        let mut next_desired_generation = 0_u64;
        match fs::read(&state_path) {
            Ok(bytes) => {
                let mut persisted =
                    serde_json::from_slice::<Vec<PersistedLifecycleMutation>>(&bytes)
                        .map_err(|_| ProviderEffectError::StateUnavailable)?;
                let mut keys = BTreeSet::new();
                for entry in &mut persisted {
                    if entry.key.is_empty() || entry.key.len() > 128 {
                        return Err(ProviderEffectError::StateUnavailable);
                    }
                    if !keys.insert(entry.key.clone()) {
                        return Err(ProviderEffectError::StateUnavailable);
                    }
                    let guest = ResourceRef::parse(&entry.guest)
                        .map_err(|_| ProviderEffectError::StateUnavailable)?;
                    match entry.operation.as_str() {
                        "start" | "stop" | "restart" => {}
                        _ => return Err(ProviderEffectError::StateUnavailable),
                    }
                    entry.guest = guest.to_canonical_string();
                }
                next_desired_generation = migrate_legacy_generations(&mut persisted)?;
                for entry in persisted {
                    let guest = ResourceRef::parse(&entry.guest)
                        .map_err(|_| ProviderEffectError::StateUnavailable)?;
                    let operation = match entry.operation.as_str() {
                        "start" => GuestLifecycleOperation::Start,
                        "stop" => GuestLifecycleOperation::Stop,
                        "restart" => GuestLifecycleOperation::Restart,
                        _ => return Err(ProviderEffectError::StateUnavailable),
                    };
                    let authorization = persisted_authorization(&entry);
                    let quarantined = entry.quarantined || authorization.is_none();
                    let quarantine_reason = if quarantined {
                        entry
                            .quarantine_reason
                            .or_else(|| Some("missing-lifecycle-identity".to_owned()))
                    } else {
                        None
                    };
                    if mutations
                        .insert(
                            entry.key,
                            LifecycleMutation {
                                guest,
                                operation,
                                authorization,
                                admitted_at_ms: entry.admitted_at_ms,
                                desired_generation: entry.desired_generation,
                                status: entry.status,
                                executing: false,
                                quarantined,
                                quarantine_reason,
                            },
                        )
                        .is_some()
                    {
                        return Err(ProviderEffectError::StateUnavailable);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ProviderEffectError::StateUnavailable),
        }
        let dispatcher = Self {
            zone,
            mutations: Mutex::new(mutations),
            state_path: Some(state_path),
            next_desired_generation: AtomicU64::new(next_desired_generation),
        };
        if let Ok(mut state) = dispatcher.mutations.lock() {
            dispatcher.retain_live(&mut state);
            dispatcher.persist_locked(&state)?;
        } else {
            return Err(ProviderEffectError::StateUnavailable);
        }
        Ok(dispatcher)
    }

    /// Admit one request after checking caller role, Zone, and deduplication.
    pub fn admit(
        &self,
        caller: &BrokerCallerRole,
        request: &GuestLifecycleRequest,
    ) -> Result<LifecycleDispatch, ProviderEffectError> {
        self.admit_internal(caller, request, false)
    }

    fn admit_internal(
        &self,
        caller: &BrokerCallerRole,
        request: &GuestLifecycleRequest,
        claim_pending: bool,
    ) -> Result<LifecycleDispatch, ProviderEffectError> {
        if !caller_is_authorized(caller) {
            return Err(ProviderEffectError::CallerRoleDenied);
        }
        if request.zone() != &self.zone {
            return Err(ProviderEffectError::ZoneMismatch);
        }
        validate_authorization(request)?;
        if matches!(caller, BrokerCallerRole::HostShutdownUid { .. })
            != request.authorization().is_stop_only()
            || (matches!(caller, BrokerCallerRole::HostShutdownUid { .. })
                && request.operation() != GuestLifecycleOperation::Stop)
        {
            return Err(ProviderEffectError::StopOnlyLease);
        }
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        let now = now_ms();
        self.retain_live(&mut mutations);
        let latest_generation = latest_generation_for_identity(&mutations, request.authorization());
        let guest_executing = guest_has_execution(&mutations, request.authorization());
        if let Some(mutation) = mutations.get_mut(request.idempotency_key()) {
            if mutation.quarantined || mutation.authorization.is_none() {
                return Err(ProviderEffectError::IdentityMismatch);
            }
            if !mutation_has_identity(mutation, request.authorization())
                || mutation.operation != request.operation()
                || mutation.authorization.as_ref() != Some(request.authorization())
            {
                mutation.quarantined = true;
                mutation.executing = false;
                mutation.quarantine_reason = Some("lifecycle-identity-mismatch".to_owned());
                self.persist_locked(&mutations)?;
                return Err(ProviderEffectError::IdentityMismatch);
            }
            let is_latest = latest_generation == Some(mutation.desired_generation);
            return Ok(match mutation.status {
                LifecycleMutationStatus::Applied if is_latest => LifecycleDispatch::Duplicate,
                LifecycleMutationStatus::Applied => LifecycleDispatch::Pending,
                LifecycleMutationStatus::Pending if !is_latest => LifecycleDispatch::Pending,
                LifecycleMutationStatus::Pending if claim_pending => {
                    if mutation.executing || guest_executing {
                        LifecycleDispatch::Pending
                    } else {
                        mutation.executing = true;
                        LifecycleDispatch::Reconcile
                    }
                }
                LifecycleMutationStatus::Pending => LifecycleDispatch::Pending,
            });
        }
        let active_mutations = mutations
            .values()
            .filter(|mutation| !mutation.quarantined)
            .count();
        if active_mutations >= MAX_TRACKED_LIFECYCLE_MUTATIONS {
            return Err(ProviderEffectError::MutationTableFull);
        }
        let desired_generation = self.allocate_desired_generation()?;
        let executing = claim_pending && !guest_executing;
        mutations.insert(
            request.idempotency_key().to_owned(),
            LifecycleMutation {
                guest: request.guest().clone(),
                operation: request.operation(),
                authorization: Some(request.authorization().clone()),
                admitted_at_ms: now,
                desired_generation,
                status: LifecycleMutationStatus::Pending,
                executing,
                quarantined: false,
                quarantine_reason: None,
            },
        );
        if let Err(error) = self.persist_locked(&mutations) {
            mutations.remove(request.idempotency_key());
            return Err(error);
        }
        Ok(if claim_pending && guest_executing {
            LifecycleDispatch::Pending
        } else {
            LifecycleDispatch::Dispatch
        })
    }

    /// Admit a request and invoke its typed effect exactly once.
    ///
    /// A refused effect is removed from the bounded table so an operator can
    /// retry with a fresh broker round trip.  No fallback effect is attempted.
    pub fn dispatch<P: ProviderLifecycleEffectPort>(
        &self,
        caller: &BrokerCallerRole,
        request: &GuestLifecycleRequest,
        effect: &P,
    ) -> Result<EffectDispatch<P::Output>, ProviderEffectError> {
        match self.admit_internal(caller, request, true)? {
            LifecycleDispatch::Duplicate => Ok(EffectDispatch::Duplicate),
            LifecycleDispatch::Pending => Err(ProviderEffectError::MutationPending),
            LifecycleDispatch::Reconcile => {
                let actual_state = match effect.actual_state(request) {
                    Ok(actual_state) => actual_state,
                    Err(error) => {
                        self.release_execution(request);
                        return Err(error);
                    }
                };
                if actual_state.satisfies(request.operation()) {
                    match self.complete_applied(request) {
                        Ok(true) => Ok(EffectDispatch::Duplicate),
                        Ok(false) => Err(ProviderEffectError::MutationPending),
                        Err(error) => {
                            self.release_execution(request);
                            Err(error)
                        }
                    }
                } else {
                    self.apply_admitted(request, effect)
                }
            }
            LifecycleDispatch::Dispatch => self.apply_admitted(request, effect),
        }
    }

    fn apply_admitted<P: ProviderLifecycleEffectPort>(
        &self,
        request: &GuestLifecycleRequest,
        effect: &P,
    ) -> Result<EffectDispatch<P::Output>, ProviderEffectError> {
        if !self.execution_is_current(request)? {
            return Err(ProviderEffectError::MutationPending);
        }
        match effect.apply(request) {
            Ok(output) => match self.complete_applied(request) {
                Ok(_) => Ok(EffectDispatch::Dispatched(output)),
                Err(error) => {
                    self.release_execution(request);
                    Err(error)
                }
            },
            Err(error) => match self.remove(request) {
                Ok(()) => Err(error),
                Err(persist_error) => {
                    self.release_execution(request);
                    Err(persist_error)
                }
            },
        }
    }

    fn remove(&self, request: &GuestLifecycleRequest) -> Result<(), ProviderEffectError> {
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        let previous = mutations.clone();
        let (authorization, operation, desired_generation, status) = mutations
            .get(request.idempotency_key())
            .map(|mutation| {
                (
                    mutation.authorization.clone(),
                    mutation.operation,
                    mutation.desired_generation,
                    mutation.status,
                )
            })
            .ok_or(ProviderEffectError::StateUnavailable)?;
        if authorization.as_ref() != Some(request.authorization())
            || operation != request.operation()
            || status != LifecycleMutationStatus::Pending
        {
            return Err(ProviderEffectError::StateUnavailable);
        }
        if latest_generation_for_identity(&mutations, request.authorization())
            != Some(desired_generation)
        {
            if let Some(mutation) = mutations.get_mut(request.idempotency_key()) {
                mutation.executing = false;
            }
            return Ok(());
        }
        mutations.remove(request.idempotency_key());
        if let Err(error) = self.persist_locked(&mutations) {
            *mutations = previous;
            restore_pending_execution(&mut mutations, request);
            return Err(error);
        }
        Ok(())
    }

    fn complete_applied(
        &self,
        request: &GuestLifecycleRequest,
    ) -> Result<bool, ProviderEffectError> {
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        let previous = mutations.clone();
        let (authorization, operation, desired_generation, status) = mutations
            .get_mut(request.idempotency_key())
            .map(|mutation| {
                (
                    mutation.authorization.clone(),
                    mutation.operation,
                    mutation.desired_generation,
                    mutation.status,
                )
            })
            .ok_or(ProviderEffectError::StateUnavailable)?;
        if authorization.as_ref() != Some(request.authorization())
            || operation != request.operation()
            || status != LifecycleMutationStatus::Pending
        {
            return Err(ProviderEffectError::StateUnavailable);
        }
        if latest_generation_for_identity(&mutations, request.authorization())
            != Some(desired_generation)
        {
            if let Some(mutation) = mutations.get_mut(request.idempotency_key()) {
                mutation.executing = false;
            }
            return Ok(false);
        }
        let mutation = mutations
            .get_mut(request.idempotency_key())
            .ok_or(ProviderEffectError::StateUnavailable)?;
        mutation.status = LifecycleMutationStatus::Applied;
        mutation.executing = false;
        mutations.retain(|_, mutation| {
            mutation.quarantined
                || !mutation_has_identity(mutation, request.authorization())
                || mutation.desired_generation >= desired_generation
                || mutation.executing
                || mutation.status == LifecycleMutationStatus::Pending
        });
        if let Err(error) = self.persist_locked(&mutations) {
            *mutations = previous;
            restore_pending_execution(&mut mutations, request);
            return Err(error);
        }
        Ok(true)
    }

    fn allocate_desired_generation(&self) -> Result<u64, ProviderEffectError> {
        self.next_desired_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            })
            .map(|previous| previous.saturating_add(1))
            .map_err(|_| ProviderEffectError::StateUnavailable)
    }

    fn execution_is_current(
        &self,
        request: &GuestLifecycleRequest,
    ) -> Result<bool, ProviderEffectError> {
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        let latest_generation = latest_generation_for_identity(&mutations, request.authorization());
        let Some(mutation) = mutations.get_mut(request.idempotency_key()) else {
            return Err(ProviderEffectError::StateUnavailable);
        };
        let current = mutation_has_identity(mutation, request.authorization())
            && mutation.operation == request.operation()
            && mutation.status == LifecycleMutationStatus::Pending
            && mutation.executing
            && latest_generation == Some(mutation.desired_generation);
        if !current {
            mutation.executing = false;
        }
        Ok(current)
    }

    fn release_execution(&self, request: &GuestLifecycleRequest) {
        if let Ok(mut mutations) = self.mutations.lock()
            && let Some(mutation) = mutations.get_mut(request.idempotency_key())
            && mutation_has_identity(mutation, request.authorization())
            && mutation.operation == request.operation()
            && mutation.status == LifecycleMutationStatus::Pending
        {
            mutation.executing = false;
        }
    }

    /// Check whether an already-applied lifecycle admission is still the
    /// latest request for the same immutable Guest identity.
    pub fn is_latest(
        &self,
        caller: &BrokerCallerRole,
        request: &GuestLifecycleRequest,
    ) -> Result<bool, ProviderEffectError> {
        if !caller_is_authorized(caller) {
            return Err(ProviderEffectError::CallerRoleDenied);
        }
        if request.zone() != &self.zone {
            return Err(ProviderEffectError::ZoneMismatch);
        }
        validate_authorization(request)?;
        if matches!(caller, BrokerCallerRole::HostShutdownUid { .. })
            != request.authorization().is_stop_only()
            || (matches!(caller, BrokerCallerRole::HostShutdownUid { .. })
                && request.operation() != GuestLifecycleOperation::Stop)
        {
            return Err(ProviderEffectError::StopOnlyLease);
        }
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        self.retain_live(&mut mutations);
        let Some(mutation) = mutations.get(request.idempotency_key()) else {
            return Ok(false);
        };
        Ok(!mutation.quarantined
            && mutation_has_identity(mutation, request.authorization())
            && mutation.operation == request.operation()
            && mutation.status == LifecycleMutationStatus::Applied
            && latest_generation_for_identity(&mutations, request.authorization())
                == Some(mutation.desired_generation))
    }

    /// Return the latest operation for one exact persisted Guest identity.
    pub(crate) fn latest_operation_for_identity(
        &self,
        zone_uid: &ResourceUid,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
        guest_generation: ResourceGeneration,
        provider_assignment_generation: ResourceGeneration,
        policy_revision: u64,
    ) -> Result<Option<GuestLifecycleOperation>, ProviderEffectError> {
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        self.retain_live(&mut mutations);
        let latest = mutations
            .values()
            .filter(|mutation| !mutation.quarantined)
            .filter(|mutation| {
                let Some(authorization) = mutation.authorization.as_ref() else {
                    return false;
                };
                authorization.zone_uid() == zone_uid
                    && authorization.guest_ref() == guest_ref
                    && authorization.guest_uid() == guest_uid
                    && authorization.guest_generation() == guest_generation
                    && authorization.provider_assignment_generation()
                        == provider_assignment_generation
                    && authorization.policy_revision() == policy_revision
            })
            .max_by_key(|mutation| mutation.desired_generation);
        Ok(latest.map(|mutation| mutation.operation))
    }

    fn retain_live(&self, mutations: &mut BTreeMap<String, LifecycleMutation>) {
        let now = now_ms();
        let durable_latest = mutations
            .iter()
            .filter_map(|(key, mutation)| {
                if mutation.quarantined || mutation.status != LifecycleMutationStatus::Applied {
                    return None;
                }
                let identity = lifecycle_identity_key(mutation)?;
                Some((identity, (mutation.desired_generation, key.clone())))
            })
            .fold(
                BTreeMap::<LifecycleIdentityKey, (u64, String)>::new(),
                |mut latest, (identity, candidate)| {
                    if latest
                        .get(&identity)
                        .is_none_or(|current| candidate.0 > current.0)
                    {
                        latest.insert(identity, candidate);
                    }
                    latest
                },
            );
        mutations.retain(|_, mutation| {
            // An executing row is live until its effect returns. Pending and
            // settled history gets bounded recovery/retention. The latest
            // applied row for each exact Guest identity is also retained as
            // durable desired lifecycle state.
            let durable = lifecycle_identity_key(mutation).and_then(|identity| {
                durable_latest
                    .get(&identity)
                    .filter(|(generation, _)| *generation == mutation.desired_generation)
            });
            durable.is_some()
                || mutation.quarantined
                || mutation.executing
                || now.saturating_sub(mutation.admitted_at_ms)
                    < LIFECYCLE_IDEMPOTENCY_TTL.as_millis() as u64
        });
    }

    fn persist_locked(
        &self,
        mutations: &BTreeMap<String, LifecycleMutation>,
    ) -> Result<(), ProviderEffectError> {
        let Some(path) = self.state_path.as_deref() else {
            return Ok(());
        };
        let entries = mutations
            .iter()
            .map(|(key, mutation)| {
                let authorization = mutation.authorization.as_ref();
                PersistedLifecycleMutation {
                    key: key.clone(),
                    guest: mutation.guest.to_canonical_string(),
                    operation: mutation.operation.as_str().to_owned(),
                    admitted_at_ms: mutation.admitted_at_ms,
                    desired_generation: mutation.desired_generation,
                    status: mutation.status,
                    subject_uid: authorization.and_then(|value| {
                        value
                            .subject_uid
                            .as_ref()
                            .map(|uid| uid.as_str().to_owned())
                    }),
                    zone_uid: authorization.map(|value| value.zone_uid.as_str().to_owned()),
                    guest_uid: authorization.map(|value| value.guest_uid.as_str().to_owned()),
                    guest_generation: authorization.map(|value| value.guest_generation.get()),
                    provider_assignment_generation: authorization
                        .map(|value| value.provider_assignment_generation.get()),
                    policy_revision: authorization.map(|value| value.policy_revision),
                    operation_id: authorization.map(|value| value.operation_id.clone()),
                    stop_only: authorization.map(|value| value.stop_only),
                    quarantined: mutation.quarantined,
                    quarantine_reason: mutation.quarantine_reason.clone(),
                }
            })
            .collect::<Vec<_>>();
        let bytes =
            serde_json::to_vec(&entries).map_err(|_| ProviderEffectError::StateUnavailable)?;
        let next = path.with_extension("json.next");
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&next)
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        fs::rename(&next, path).map_err(|_| ProviderEffectError::StateUnavailable)?;
        if let Some(parent) = path.parent() {
            OpenOptions::new()
                .read(true)
                .open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| ProviderEffectError::StateUnavailable)?;
        }
        Ok(())
    }
}

fn lifecycle_identity_key(mutation: &LifecycleMutation) -> Option<LifecycleIdentityKey> {
    let authorization = mutation.authorization.as_ref()?;
    Some(LifecycleIdentityKey {
        zone_uid: authorization.zone_uid().clone(),
        guest_ref: authorization.guest_ref().clone(),
        guest_uid: authorization.guest_uid().clone(),
        guest_generation: authorization.guest_generation(),
        provider_assignment_generation: authorization.provider_assignment_generation(),
        policy_revision: authorization.policy_revision(),
    })
}

fn validate_authorization(request: &GuestLifecycleRequest) -> Result<(), ProviderEffectError> {
    let authorization = request.authorization();
    if authorization.guest_uid.as_str().is_empty()
        || authorization.guest_generation.get() == 0
        || authorization.provider_assignment_generation.get() == 0
        || authorization.policy_revision == 0
        || authorization.operation_id.is_empty()
        || authorization.operation_id.len() > 128
    {
        return Err(ProviderEffectError::AuthorizationLeaseInvalid);
    }
    if authorization.stop_only && request.operation() != GuestLifecycleOperation::Stop {
        return Err(ProviderEffectError::StopOnlyLease);
    }
    Ok(())
}

fn persisted_authorization(entry: &PersistedLifecycleMutation) -> Option<LifecycleAuthorization> {
    let zone_uid = ResourceUid::parse(entry.zone_uid.as_ref()?).ok()?;
    let guest_ref = ResourceRef::parse(&entry.guest).ok()?;
    if guest_ref.resource_type().as_str() != "Guest" {
        return None;
    }
    let guest_uid = ResourceUid::parse(entry.guest_uid.as_ref()?).ok()?;
    let guest_generation = ResourceGeneration::new(entry.guest_generation?).ok()?;
    let provider_assignment_generation =
        ResourceGeneration::new(entry.provider_assignment_generation?).ok()?;
    let policy_revision = entry.policy_revision?;
    let operation_id = entry.operation_id.as_ref()?.clone();
    let stop_only = entry.stop_only.unwrap_or(false);
    let subject_uid = match (stop_only, entry.subject_uid.as_ref()) {
        (true, None) => None,
        (false, Some(value)) => Some(ResourceUid::parse(value).ok()?),
        _ => return None,
    };
    if policy_revision == 0
        || operation_id.is_empty()
        || operation_id.len() > 128
        || operation_id.chars().any(char::is_control)
        || operation_id != entry.key
        || (stop_only && entry.operation != "stop")
    {
        return None;
    }
    Some(LifecycleAuthorization {
        subject_uid,
        zone_uid,
        guest_ref,
        guest_uid,
        guest_generation,
        provider_assignment_generation,
        policy_revision,
        operation_id,
        stop_only,
    })
}

fn migrate_legacy_generations(
    persisted: &mut [PersistedLifecycleMutation],
) -> Result<u64, ProviderEffectError> {
    let mut used_generations = BTreeSet::new();
    let mut next_generation = 0_u64;
    let mut zero_generation_by_guest = BTreeMap::<String, Vec<usize>>::new();

    for (index, entry) in persisted.iter().enumerate() {
        if entry.quarantined || persisted_authorization(entry).is_none() {
            continue;
        }
        if entry.desired_generation == 0 {
            zero_generation_by_guest
                .entry(entry.guest.clone())
                .or_default()
                .push(index);
        } else {
            used_generations.insert(entry.desired_generation);
            next_generation = next_generation.max(entry.desired_generation);
        }
    }

    for indexes in zero_generation_by_guest.values_mut() {
        indexes.sort_by(|left, right| {
            persisted[*left]
                .admitted_at_ms
                .cmp(&persisted[*right].admitted_at_ms)
                .then_with(|| persisted[*left].key.cmp(&persisted[*right].key))
        });

        let mut group_start = 0;
        while group_start < indexes.len() {
            let timestamp = persisted[indexes[group_start]].admitted_at_ms;
            let group_end = indexes[group_start..]
                .iter()
                .position(|index| persisted[*index].admitted_at_ms != timestamp)
                .map_or(indexes.len(), |offset| group_start + offset);
            let operation = &persisted[indexes[group_start]].operation;
            if indexes[group_start..group_end]
                .iter()
                .any(|index| persisted[*index].operation != *operation)
            {
                return Err(ProviderEffectError::StateUnavailable);
            }
            group_start = group_end;
        }

        for index in indexes {
            loop {
                next_generation = next_generation
                    .checked_add(1)
                    .ok_or(ProviderEffectError::StateUnavailable)?;
                if used_generations.insert(next_generation) {
                    break;
                }
            }
            persisted[*index].desired_generation = next_generation;
        }
    }

    Ok(next_generation)
}

fn mutation_has_identity(
    mutation: &LifecycleMutation,
    authorization: &LifecycleAuthorization,
) -> bool {
    !mutation.quarantined
        && mutation.authorization.as_ref().is_some_and(|current| {
            current.zone_uid == authorization.zone_uid
                && current.guest_ref == authorization.guest_ref
                && current.guest_uid == authorization.guest_uid
                && current.guest_generation == authorization.guest_generation
                && current.provider_assignment_generation
                    == authorization.provider_assignment_generation
                && current.policy_revision == authorization.policy_revision
        })
}

fn latest_generation_for_identity(
    mutations: &BTreeMap<String, LifecycleMutation>,
    authorization: &LifecycleAuthorization,
) -> Option<u64> {
    mutations
        .values()
        .filter(|mutation| mutation_has_identity(mutation, authorization))
        .map(|mutation| mutation.desired_generation)
        .max()
}

fn guest_has_execution(
    mutations: &BTreeMap<String, LifecycleMutation>,
    authorization: &LifecycleAuthorization,
) -> bool {
    mutations
        .values()
        .any(|mutation| mutation_has_identity(mutation, authorization) && mutation.executing)
}

fn restore_pending_execution(
    mutations: &mut BTreeMap<String, LifecycleMutation>,
    request: &GuestLifecycleRequest,
) {
    if let Some(mutation) = mutations.get_mut(request.idempotency_key())
        && mutation_has_identity(mutation, request.authorization())
        && mutation.operation == request.operation()
        && mutation.status == LifecycleMutationStatus::Pending
    {
        mutation.executing = false;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        ResourceGeneration, ResourceRef, ZoneRevision, identity::ReconnectGeneration,
    };
    use d2b_process_conformance::ConfigurationDigest;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Sender, channel},
    };

    #[test]
    fn fixed_effect_adapter_binds_controller_ticket_to_its_mode() {
        let ticket = d2b_process_conformance::testing::fixtures::ticket_builder()
            .execution_ref(ResourceRef::parse("Guest/dev-vm").unwrap())
            .build()
            .unwrap()
            .with_resource_revision(ZoneRevision::new(1))
            .unwrap()
            .with_controller_launch_binding(
                ResourceGeneration::new(2).unwrap(),
                ReconnectGeneration::new(3).unwrap(),
                ConfigurationDigest::from_bytes([1; 32]),
                ConfigurationDigest::from_bytes([2; 32]),
            )
            .unwrap();
        let guest = FixedEffectAdapter::guest("/run/d2b/guest-broker.sock".into(), 997);
        assert!(guest.validate_controller_ticket(&ticket).is_ok());
        let host = FixedEffectAdapter::host("/run/d2b/broker.sock".into(), 997);
        assert!(matches!(
            host.validate_controller_ticket(&ticket),
            Err(FixedEffectError::ControllerTargetDenied)
        ));
    }

    #[test]
    fn malformed_controller_ticket_is_rejected_before_broker_io() {
        let ticket = d2b_process_conformance::testing::fixtures::ticket_builder()
            .execution_ref(ResourceRef::parse("Guest/dev-vm").unwrap())
            .build()
            .unwrap();
        let guest = FixedEffectAdapter::guest("/run/d2b/guest-broker.sock".into(), 997);
        assert!(matches!(
            guest.validate_controller_ticket(&ticket),
            Err(FixedEffectError::ControllerTicketInvalid)
        ));
    }

    struct RecordingEffect {
        calls: Arc<AtomicUsize>,
        reject: AtomicBool,
    }

    impl ProviderLifecycleEffectPort for RecordingEffect {
        type Output = usize;

        fn apply(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            if self.reject.load(Ordering::Acquire) {
                return Err(ProviderEffectError::EffectRejected);
            }
            Ok(self.calls.fetch_add(1, Ordering::AcqRel) + 1)
        }
    }

    struct ReconciliationEffect {
        calls: Arc<AtomicUsize>,
        reached: AtomicBool,
    }

    impl ProviderLifecycleEffectPort for ReconciliationEffect {
        type Output = usize;

        fn actual_state(
            &self,
            request: &GuestLifecycleRequest,
        ) -> Result<GuestLifecycleState, ProviderEffectError> {
            let reached = self.reached.load(Ordering::Acquire);
            Ok(match (request.operation(), reached) {
                (GuestLifecycleOperation::Start, true) => GuestLifecycleState::Started,
                (GuestLifecycleOperation::Start, false) => GuestLifecycleState::Stopped,
                (GuestLifecycleOperation::Stop, true) => GuestLifecycleState::Stopped,
                (GuestLifecycleOperation::Stop, false) => GuestLifecycleState::Started,
                (GuestLifecycleOperation::Restart, _) => GuestLifecycleState::Stopped,
            })
        }

        fn apply(
            &self,
            request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            self.reached.store(true, Ordering::Release);
            let _ = request;
            Ok(self.calls.fetch_add(1, Ordering::AcqRel) + 1)
        }
    }

    struct UnavailableReconciliationEffect {
        calls: Arc<AtomicUsize>,
    }

    impl ProviderLifecycleEffectPort for UnavailableReconciliationEffect {
        type Output = usize;

        fn actual_state(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<GuestLifecycleState, ProviderEffectError> {
            Err(ProviderEffectError::StateUnavailable)
        }

        fn apply(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            Ok(self.calls.fetch_add(1, Ordering::AcqRel) + 1)
        }
    }

    #[derive(Clone)]
    struct BlockingEffect {
        entered: Sender<()>,
        release: Arc<Barrier>,
        calls: Arc<AtomicUsize>,
    }

    impl ProviderLifecycleEffectPort for BlockingEffect {
        type Output = usize;

        fn apply(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            self.entered.send(()).expect("effect entered");
            self.release.wait();
            Ok(call)
        }
    }

    struct FailThenReconcileEffect {
        calls: Arc<AtomicUsize>,
        fail_first: AtomicBool,
        reached: AtomicBool,
    }

    impl ProviderLifecycleEffectPort for FailThenReconcileEffect {
        type Output = usize;

        fn actual_state(
            &self,
            request: &GuestLifecycleRequest,
        ) -> Result<GuestLifecycleState, ProviderEffectError> {
            Ok(
                match (request.operation(), self.reached.load(Ordering::Acquire)) {
                    (GuestLifecycleOperation::Start, true) => GuestLifecycleState::Started,
                    (GuestLifecycleOperation::Start, false) => GuestLifecycleState::Stopped,
                    (GuestLifecycleOperation::Stop, true) => GuestLifecycleState::Stopped,
                    (GuestLifecycleOperation::Stop, false) => GuestLifecycleState::Started,
                    (GuestLifecycleOperation::Restart, _) => GuestLifecycleState::Stopped,
                },
            )
        }

        fn apply(
            &self,
            request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            if self.fail_first.swap(false, Ordering::AcqRel) {
                return Err(ProviderEffectError::EffectRejected);
            }
            self.reached.store(true, Ordering::Release);
            let _ = request;
            Ok(call)
        }
    }

    #[derive(Clone)]
    struct LongRunningEffect {
        entered: Sender<()>,
        release: Arc<Barrier>,
        calls: Arc<AtomicUsize>,
    }

    impl ProviderLifecycleEffectPort for LongRunningEffect {
        type Output = usize;

        fn apply(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            if call == 1 {
                self.entered.send(()).expect("effect entered");
                self.release.wait();
            }
            Ok(call)
        }
    }

    #[derive(Clone)]
    struct StatefulEffect {
        entered: Sender<()>,
        release: Arc<Barrier>,
        state: Arc<Mutex<GuestLifecycleState>>,
        calls: Arc<AtomicUsize>,
    }

    impl ProviderLifecycleEffectPort for StatefulEffect {
        type Output = usize;

        fn actual_state(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<GuestLifecycleState, ProviderEffectError> {
            Ok(*self.state.lock().expect("state lock"))
        }

        fn apply(
            &self,
            request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            if call == 1 {
                self.entered.send(()).expect("effect entered");
                self.release.wait();
            }
            *self.state.lock().expect("state lock") = match request.operation() {
                GuestLifecycleOperation::Start => GuestLifecycleState::Started,
                GuestLifecycleOperation::Stop => GuestLifecycleState::Stopped,
                GuestLifecycleOperation::Restart => GuestLifecycleState::Started,
            };
            Ok(call)
        }
    }

    fn request(
        zone: &ZoneId,
        operation: GuestLifecycleOperation,
        key: &str,
    ) -> GuestLifecycleRequest {
        request_with_guest(zone, operation, key, "22222222-2222-4222-8222-222222222222")
    }

    fn request_with_guest(
        zone: &ZoneId,
        operation: GuestLifecycleOperation,
        key: &str,
        guest_uid: &str,
    ) -> GuestLifecycleRequest {
        GuestLifecycleRequest::new(
            zone.clone(),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            operation,
            key,
            LifecycleAuthorization::for_test_with_guest(
                "Guest/workstation",
                "11111111-1111-4111-8111-111111111111",
                guest_uid,
                1,
                1,
                1,
                key,
            ),
        )
        .expect("lifecycle request")
    }

    fn persisted_row(
        key: &str,
        guest: &str,
        operation: &str,
        admitted_at_ms: u64,
        desired_generation: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "key": key,
            "guest": guest,
            "operation": operation,
            "admitted_at_ms": admitted_at_ms,
            "desired_generation": desired_generation,
            "status": "pending",
            "subjectUid": "33333333-3333-4333-8333-333333333333",
            "zoneUid": "11111111-1111-4111-8111-111111111111",
            "guestUid": "22222222-2222-4222-8222-222222222222",
            "guestGeneration": 1,
            "providerAssignmentGeneration": 1,
            "policyRevision": 1,
            "operationId": key,
            "stopOnly": false,
        })
    }

    #[test]
    fn dispatch_invokes_typed_effect_once_and_deduplicates_reachably() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls: Arc::clone(&calls),
            reject: AtomicBool::new(false),
        };
        let request = request(&zone, GuestLifecycleOperation::Start, "k1");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };

        assert_eq!(
            dispatch.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(
            dispatch.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Duplicate)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn unauthorized_or_mismatched_requests_fail_closed_before_effect() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls: Arc::clone(&calls),
            reject: AtomicBool::new(false),
        };
        let lifecycle_request = request(&zone, GuestLifecycleOperation::Stop, "k2");

        assert_eq!(
            dispatch.dispatch(
                &BrokerCallerRole::NotAuthorized,
                &lifecycle_request,
                &effect
            ),
            Err(ProviderEffectError::CallerRoleDenied)
        );
        let other_zone = ZoneId::parse("personal").expect("Zone");
        let other_request = request(&other_zone, GuestLifecycleOperation::Stop, "k3");
        assert_eq!(
            dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &other_request,
                &effect
            ),
            Err(ProviderEffectError::ZoneMismatch)
        );
        assert_eq!(calls.load(Ordering::Acquire), 0);
    }

    #[test]
    fn rejected_effect_is_not_replaced_by_a_fallback() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls,
            reject: AtomicBool::new(true),
        };
        let request = request(&zone, GuestLifecycleOperation::Start, "k4");
        assert_eq!(
            dispatch.dispatch(
                &BrokerCallerRole::LauncherUid { uid: 1000 },
                &request,
                &effect
            ),
            Err(ProviderEffectError::EffectRejected)
        );
    }

    #[test]
    fn completed_stop_retires_prior_start_without_releasing_failed_retry() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls: Arc::clone(&calls),
            reject: AtomicBool::new(false),
        };
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let start = request(&zone, GuestLifecycleOperation::Start, "start-1");
        let stop = request(&zone, GuestLifecycleOperation::Stop, "stop-1");
        assert!(matches!(
            dispatch.dispatch(&caller, &start, &effect),
            Ok(EffectDispatch::Dispatched(_))
        ));
        assert!(matches!(
            dispatch.dispatch(&caller, &stop, &effect),
            Ok(EffectDispatch::Dispatched(_))
        ));
        let next_start = request(&zone, GuestLifecycleOperation::Start, "start-2");
        assert!(matches!(
            dispatch.dispatch(&caller, &next_start, &effect),
            Ok(EffectDispatch::Dispatched(_))
        ));
        assert_eq!(calls.load(Ordering::Acquire), 3);
    }

    #[test]
    fn persistent_admission_deduplicates_the_same_request_after_restart() {
        let root = crate::test_scratch_root()
            .join(format!("provider-lifecycle-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let request = request(&zone, GuestLifecycleOperation::Start, "admitted-start");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls: Arc::clone(&calls),
            reject: AtomicBool::new(false),
        };
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            dispatch.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        drop(dispatch);

        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone, &path).expect("restore state");
        assert_eq!(
            restarted.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Duplicate)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read lifecycle state"))
                .expect("parse lifecycle state");
        assert_eq!(persisted[0]["status"], "applied");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn applied_stop_intent_survives_idempotency_ttl_and_restart() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-stop-intent-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = request(&zone, GuestLifecycleOperation::Stop, "durable-stop");
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            dispatch.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        {
            let mut mutations = dispatch.mutations.lock().expect("mutation lock");
            mutations
                .get_mut("durable-stop")
                .expect("applied stop")
                .admitted_at_ms =
                now_ms().saturating_sub(LIFECYCLE_IDEMPOTENCY_TTL.as_millis() as u64 + 1);
        }
        assert_eq!(
            dispatch
                .latest_operation_for_identity(
                    request.authorization().zone_uid(),
                    request.authorization().guest_ref(),
                    request.authorization().guest_uid(),
                    request.authorization().guest_generation(),
                    request.authorization().provider_assignment_generation(),
                    request.authorization().policy_revision(),
                )
                .expect("latest durable intent"),
            Some(GuestLifecycleOperation::Stop)
        );
        drop(dispatch);
        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone, &path).expect("restart state");
        assert_eq!(
            restarted
                .latest_operation_for_identity(
                    request.authorization().zone_uid(),
                    request.authorization().guest_ref(),
                    request.authorization().guest_uid(),
                    request.authorization().guest_generation(),
                    request.authorization().provider_assignment_generation(),
                    request.authorization().policy_revision(),
                )
                .expect("latest durable intent after restart"),
            Some(GuestLifecycleOperation::Stop)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_generation_migration_preserves_temporal_latest_after_restart() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-legacy-temporal-migration-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let now = now_ms();
        let rows = vec![
            persisted_row(
                "a-later",
                "Guest/workstation",
                "stop",
                now.saturating_sub(100),
                0,
            ),
            persisted_row(
                "z-earlier",
                "Guest/workstation",
                "start",
                now.saturating_sub(200),
                0,
            ),
        ];
        std::fs::create_dir_all(&root).expect("create state directory");
        std::fs::write(
            &path,
            serde_json::to_vec(&rows).expect("serialize legacy state"),
        )
        .expect("write legacy state");

        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("restart");
        let persisted: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).expect("read migrated state"))
                .expect("parse migrated state");
        assert_eq!(persisted[0]["key"], "a-later");
        assert_eq!(persisted[0]["desiredGeneration"], 2);
        assert_eq!(persisted[1]["key"], "z-earlier");
        assert_eq!(persisted[1]["desiredGeneration"], 1);

        let calls = Arc::new(AtomicUsize::new(0));
        let effect = ReconciliationEffect {
            calls: Arc::clone(&calls),
            reached: AtomicBool::new(false),
        };
        let later = request(&zone, GuestLifecycleOperation::Stop, "a-later");
        let earlier = request(&zone, GuestLifecycleOperation::Start, "z-earlier");
        assert_eq!(
            restarted.dispatch(&caller, &later, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(
            restarted.dispatch(&caller, &earlier, &effect),
            Err(ProviderEffectError::MutationPending)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);

        let migrated_bytes = std::fs::read(&path).expect("read stable migrated state");
        drop(restarted);
        let restarted_again =
            ProviderLifecycleDispatch::new_persistent(zone, &path).expect("second restart");
        assert_eq!(
            std::fs::read(&path).expect("read second migrated state"),
            migrated_bytes
        );
        drop(restarted_again);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_generation_migration_rejects_conflicting_same_guest_timestamp_tie() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-legacy-tie-conflict-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let timestamp = now_ms().saturating_sub(100);
        let rows = vec![
            persisted_row("a-start", "Guest/workstation", "start", timestamp, 0),
            persisted_row("b-stop", "Guest/workstation", "stop", timestamp, 0),
        ];
        std::fs::create_dir_all(&root).expect("create state directory");
        let original = serde_json::to_vec(&rows).expect("serialize conflicting legacy state");
        std::fs::write(&path, &original).expect("write conflicting legacy state");

        let zone = ZoneId::parse("work").expect("Zone");
        assert!(matches!(
            ProviderLifecycleDispatch::new_persistent(zone, &path),
            Err(ProviderEffectError::StateUnavailable)
        ));
        assert_eq!(std::fs::read(&path).expect("read refused state"), original);
        assert!(!path.with_extension("json.next").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_generation_migration_orders_identical_timestamp_duplicates_deterministically() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-legacy-tie-duplicates-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let timestamp = now_ms().saturating_sub(100);
        let rows = vec![
            persisted_row("b-start", "Guest/workstation", "start", timestamp, 0),
            persisted_row("a-start", "Guest/workstation", "start", timestamp, 0),
        ];
        std::fs::create_dir_all(&root).expect("create state directory");
        std::fs::write(
            &path,
            serde_json::to_vec(&rows).expect("serialize duplicate legacy state"),
        )
        .expect("write duplicate legacy state");

        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("restart");
        let persisted: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).expect("read migrated state"))
                .expect("parse migrated state");
        assert_eq!(persisted[0]["key"], "a-start");
        assert_eq!(persisted[0]["desiredGeneration"], 1);
        assert_eq!(persisted[1]["key"], "b-start");
        assert_eq!(persisted[1]["desiredGeneration"], 2);
        let migrated_bytes = std::fs::read(&path).expect("read stable migrated state");
        drop(dispatch);

        let restarted_again =
            ProviderLifecycleDispatch::new_persistent(zone, &path).expect("second restart");
        assert_eq!(
            std::fs::read(&path).expect("read second migrated state"),
            migrated_bytes
        );
        drop(restarted_again);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_generation_migration_assigns_unique_values_above_mixed_explicit_state() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-legacy-mixed-generations-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let now = now_ms();
        let rows = vec![
            persisted_row(
                "explicit-alpha",
                "Guest/alpha",
                "start",
                now.saturating_sub(400),
                4,
            ),
            persisted_row(
                "zero-beta-early",
                "Guest/beta",
                "start",
                now.saturating_sub(300),
                0,
            ),
            persisted_row(
                "zero-alpha",
                "Guest/alpha",
                "stop",
                now.saturating_sub(200),
                0,
            ),
            persisted_row(
                "explicit-gamma",
                "Guest/gamma",
                "stop",
                now.saturating_sub(100),
                9,
            ),
            persisted_row(
                "zero-beta-late",
                "Guest/beta",
                "stop",
                now.saturating_sub(50),
                0,
            ),
        ];
        std::fs::create_dir_all(&root).expect("create state directory");
        std::fs::write(
            &path,
            serde_json::to_vec(&rows).expect("serialize mixed legacy state"),
        )
        .expect("write mixed legacy state");

        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("restart");
        let persisted: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).expect("read migrated state"))
                .expect("parse migrated state");
        let mut generations = persisted
            .iter()
            .map(|entry| {
                (
                    entry["key"].as_str().expect("key").to_owned(),
                    entry["desiredGeneration"].as_u64().expect("generation"),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(generations.remove("explicit-alpha"), Some(4));
        assert_eq!(generations.remove("explicit-gamma"), Some(9));
        assert_eq!(generations.remove("zero-alpha"), Some(10));
        assert_eq!(generations.remove("zero-beta-early"), Some(11));
        assert_eq!(generations.remove("zero-beta-late"), Some(12));
        assert!(generations.is_empty());
        let assigned = persisted
            .iter()
            .map(|entry| entry["desiredGeneration"].as_u64().expect("generation"))
            .collect::<Vec<_>>();
        let unique = assigned
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), assigned.len());

        let new_request = GuestLifecycleRequest::new(
            zone,
            ResourceRef::parse("Guest/delta").expect("Guest ref"),
            GuestLifecycleOperation::Start,
            "new-admission",
            LifecycleAuthorization::for_test_with_guest(
                "Guest/delta",
                "11111111-1111-4111-8111-111111111111",
                "44444444-4444-4444-8444-444444444444",
                1,
                1,
                1,
                "new-admission",
            ),
        )
        .expect("new request");
        assert_eq!(
            dispatch.admit(&BrokerCallerRole::AdminUid { uid: 1000 }, &new_request),
            Ok(LifecycleDispatch::Dispatch)
        );
        assert_eq!(
            dispatch
                .mutations
                .lock()
                .expect("mutation lock")
                .get("new-admission")
                .map(|mutation| mutation.desired_generation),
            Some(13)
        );
        drop(dispatch);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_admission_reexecutes_pending_when_actual_state_is_not_reached() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-pending-retry-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = request(&zone, GuestLifecycleOperation::Start, "pending-start");
        let admitted =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            admitted.admit(&caller, &request),
            Ok(LifecycleDispatch::Dispatch)
        );
        let pending: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read pending state"))
                .expect("parse pending state");
        assert_eq!(pending[0]["status"], "pending");
        drop(admitted);

        let calls = Arc::new(AtomicUsize::new(0));
        let effect = ReconciliationEffect {
            calls: Arc::clone(&calls),
            reached: AtomicBool::new(false),
        };
        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone, &path).expect("restore state");
        assert_eq!(
            restarted.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_pending_reconciles_without_effect_when_actual_state_is_reached() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-pending-reconcile-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = request(&zone, GuestLifecycleOperation::Stop, "pending-stop");
        let admitted =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            admitted.admit(&caller, &request),
            Ok(LifecycleDispatch::Dispatch)
        );
        drop(admitted);

        let calls = Arc::new(AtomicUsize::new(0));
        let effect = ReconciliationEffect {
            calls: Arc::clone(&calls),
            reached: AtomicBool::new(true),
        };
        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone, &path).expect("restore state");
        assert_eq!(
            restarted.dispatch(&caller, &request, &effect),
            Ok(EffectDispatch::Duplicate)
        );
        assert_eq!(calls.load(Ordering::Acquire), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_pending_state_unavailable_keeps_retryable_admission() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-pending-unavailable-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = request(&zone, GuestLifecycleOperation::Start, "pending-unavailable");
        let admitted =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            admitted.admit(&caller, &request),
            Ok(LifecycleDispatch::Dispatch)
        );
        drop(admitted);

        let calls = Arc::new(AtomicUsize::new(0));
        let unavailable = UnavailableReconciliationEffect {
            calls: Arc::clone(&calls),
        };
        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("restore state");
        assert_eq!(
            restarted.dispatch(&caller, &request, &unavailable),
            Err(ProviderEffectError::StateUnavailable)
        );
        assert_eq!(calls.load(Ordering::Acquire), 0);
        let pending: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read pending state"))
                .expect("parse pending state");
        assert_eq!(pending[0]["status"], "pending");

        let retry = ReconciliationEffect {
            calls: Arc::clone(&calls),
            reached: AtomicBool::new(false),
        };
        assert_eq!(
            restarted.dispatch(&caller, &request, &retry),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completing_operation_preserves_pending_opposite_admission() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let start = request(&zone, GuestLifecycleOperation::Start, "pending-start");
        let stop = request(&zone, GuestLifecycleOperation::Stop, "completed-stop");
        assert_eq!(
            dispatch.admit(&caller, &start),
            Ok(LifecycleDispatch::Dispatch)
        );
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        assert!(matches!(
            dispatch.dispatch(&caller, &stop, &effect),
            Ok(EffectDispatch::Dispatched(_))
        ));
        assert_eq!(
            dispatch.admit(&caller, &start),
            Ok(LifecycleDispatch::Pending)
        );
    }

    #[test]
    fn concurrent_opposite_requests_keep_in_flight_rows_until_both_complete() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-concurrent-opposite-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = Arc::new(
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path)
                .expect("open persistent state"),
        );
        let state = Arc::new(Mutex::new(GuestLifecycleState::Stopped));
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered, entered_rx) = channel();
        let effect = StatefulEffect {
            entered,
            release: Arc::new(Barrier::new(2)),
            state,
            calls: Arc::clone(&calls),
        };
        let start_dispatch = Arc::clone(&dispatch);
        let start_effect = effect.clone();
        let start = request(&zone, GuestLifecycleOperation::Start, "concurrent-start");
        let start_thread = std::thread::spawn(move || {
            start_dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &start,
                &start_effect,
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("start effect entered");
        let stop = request(&zone, GuestLifecycleOperation::Stop, "concurrent-stop");
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &stop, &effect),
            Err(ProviderEffectError::MutationPending)
        );
        let pending: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read pending state"))
                .expect("parse pending state");
        assert_eq!(
            pending.as_array().map(|entries| entries.len()),
            Some(2),
            "the newer opposite admission must remain durable while the older effect runs"
        );
        effect.release.wait();
        assert_eq!(
            start_thread.join().expect("start thread"),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &stop, &effect),
            Ok(EffectDispatch::Dispatched(2))
        );
        assert_eq!(calls.load(Ordering::Acquire), 2);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read settled state"))
                .expect("parse settled state");
        assert_eq!(persisted[0]["status"], "pending");
        assert_eq!(persisted[1]["status"], "applied");
        assert_eq!(persisted[1]["desiredGeneration"], 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn same_operation_concurrency_still_runs_one_effect() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = Arc::new(ProviderLifecycleDispatch::new(zone.clone()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered, entered_rx) = channel();
        let effect = BlockingEffect {
            entered,
            release: Arc::new(Barrier::new(2)),
            calls: Arc::clone(&calls),
        };
        let first_dispatch = Arc::clone(&dispatch);
        let first_effect = effect.clone();
        let first = request(&zone, GuestLifecycleOperation::Start, "same-operation");
        let first_thread = std::thread::spawn(move || {
            first_dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &first,
                &first_effect,
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first effect entered");

        let second = request(&zone, GuestLifecycleOperation::Start, "same-operation");
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &second, &effect,),
            Err(ProviderEffectError::MutationPending)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);

        effect.release.wait();
        assert_eq!(
            first_thread.join().expect("first thread"),
            Ok(EffectDispatch::Dispatched(1))
        );
    }

    #[test]
    fn executing_rows_survive_ttl_while_the_effect_is_running() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = Arc::new(ProviderLifecycleDispatch::new(zone.clone()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered, entered_rx) = channel();
        let effect = LongRunningEffect {
            entered,
            release: Arc::new(Barrier::new(2)),
            calls: Arc::clone(&calls),
        };
        let first_dispatch = Arc::clone(&dispatch);
        let first_effect = effect.clone();
        let lifecycle_request = request(&zone, GuestLifecycleOperation::Start, "ttl-running");
        let first_thread = std::thread::spawn(move || {
            first_dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &lifecycle_request,
                &first_effect,
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("long-running effect entered");

        {
            let mut mutations = dispatch.mutations.lock().expect("mutation lock");
            let mutation = mutations
                .get_mut("ttl-running")
                .expect("executing mutation");
            mutation.admitted_at_ms =
                now_ms().saturating_sub(LIFECYCLE_IDEMPOTENCY_TTL.as_millis() as u64 + 1);
        }
        let retry = request(&zone, GuestLifecycleOperation::Start, "ttl-running");
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &retry, &effect,),
            Err(ProviderEffectError::MutationPending)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);

        effect.release.wait();
        assert_eq!(
            first_thread.join().expect("first thread"),
            Ok(EffectDispatch::Dispatched(1))
        );
    }

    #[test]
    fn completion_persist_failure_releases_execution_for_reconciliation() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-completion-persist-failure-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let lifecycle_request =
            request(&zone, GuestLifecycleOperation::Start, "completion-failure");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            dispatch.admit(&caller, &lifecycle_request),
            Ok(LifecycleDispatch::Dispatch)
        );
        std::fs::create_dir(path.with_extension("json.next")).expect("block next state");

        let calls = Arc::new(AtomicUsize::new(0));
        let effect = ReconciliationEffect {
            calls: Arc::clone(&calls),
            reached: AtomicBool::new(false),
        };
        assert_eq!(
            dispatch.dispatch(&caller, &lifecycle_request, &effect),
            Err(ProviderEffectError::StateUnavailable)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        std::fs::remove_dir(path.with_extension("json.next")).expect("unblock next state");

        assert_eq!(
            dispatch.dispatch(&caller, &lifecycle_request, &effect),
            Ok(EffectDispatch::Duplicate)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removal_persist_failure_releases_execution_for_retry() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-removal-persist-failure-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let lifecycle_request = request(&zone, GuestLifecycleOperation::Start, "removal-failure");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            dispatch.admit(&caller, &lifecycle_request),
            Ok(LifecycleDispatch::Dispatch)
        );
        std::fs::create_dir(path.with_extension("json.next")).expect("block next state");

        let calls = Arc::new(AtomicUsize::new(0));
        let effect = FailThenReconcileEffect {
            calls: Arc::clone(&calls),
            fail_first: AtomicBool::new(true),
            reached: AtomicBool::new(false),
        };
        assert_eq!(
            dispatch.dispatch(&caller, &lifecycle_request, &effect),
            Err(ProviderEffectError::StateUnavailable)
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        std::fs::remove_dir(path.with_extension("json.next")).expect("unblock next state");

        assert_eq!(
            dispatch.dispatch(&caller, &lifecycle_request, &effect),
            Ok(EffectDispatch::Dispatched(2))
        );
        assert_eq!(calls.load(Ordering::Acquire), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opposite_admissions_are_serialized_by_latest_desired_generation() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-opposite-generation-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = Arc::new(
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state"),
        );
        let state = Arc::new(Mutex::new(GuestLifecycleState::Stopped));
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered, entered_rx) = channel();
        let effect = StatefulEffect {
            entered,
            release: Arc::new(Barrier::new(2)),
            state: Arc::clone(&state),
            calls: Arc::clone(&calls),
        };
        let start = request(&zone, GuestLifecycleOperation::Start, "generation-start");
        let start_dispatch = Arc::clone(&dispatch);
        let start_effect = effect.clone();
        let start_thread = std::thread::spawn(move || {
            start_dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &start,
                &start_effect,
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("start effect entered");

        let stop = request(&zone, GuestLifecycleOperation::Stop, "generation-stop");
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &stop, &effect),
            Err(ProviderEffectError::MutationPending)
        );
        effect.release.wait();
        assert_eq!(
            start_thread.join().expect("start thread"),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(
            *state.lock().expect("state lock"),
            GuestLifecycleState::Started
        );

        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &stop, &effect),
            Ok(EffectDispatch::Dispatched(2))
        );
        assert_eq!(
            *state.lock().expect("state lock"),
            GuestLifecycleState::Stopped
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read generation state"))
                .expect("parse generation state");
        assert_eq!(persisted.as_array().map(|entries| entries.len()), Some(2));
        assert_eq!(persisted[0]["key"], "generation-start");
        assert_eq!(persisted[0]["status"], "pending");
        assert_eq!(persisted[0]["desiredGeneration"], 1);
        assert_eq!(persisted[1]["key"], "generation-stop");
        assert_eq!(persisted[1]["desiredGeneration"], 2);
        assert_eq!(calls.load(Ordering::Acquire), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restart_preserves_latest_desired_generation() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-generation-restart-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        let start = request(&zone, GuestLifecycleOperation::Start, "restart-start");
        let stop = request(&zone, GuestLifecycleOperation::Stop, "restart-stop");
        assert_eq!(
            dispatch.admit(&caller, &start),
            Ok(LifecycleDispatch::Dispatch)
        );
        assert_eq!(
            dispatch.admit(&caller, &stop),
            Ok(LifecycleDispatch::Dispatch)
        );
        let before_restart: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read pre-restart state"))
                .expect("parse pre-restart state");
        assert_eq!(before_restart[0]["desiredGeneration"], 1);
        assert_eq!(before_restart[1]["desiredGeneration"], 2);
        drop(dispatch);

        let restarted =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("restart");
        let (entered, _entered_rx) = channel();
        let effect = StatefulEffect {
            entered,
            release: Arc::new(Barrier::new(1)),
            state: Arc::new(Mutex::new(GuestLifecycleState::Started)),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        assert_eq!(
            restarted.dispatch(&caller, &start, &effect),
            Err(ProviderEffectError::MutationPending)
        );
        assert_eq!(
            restarted.dispatch(&caller, &stop, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read restarted state"))
                .expect("parse restarted state");
        assert_eq!(persisted.as_array().map(|entries| entries.len()), Some(2));
        assert_eq!(persisted[0]["key"], "restart-start");
        assert_eq!(persisted[0]["status"], "pending");
        assert_eq!(persisted[0]["desiredGeneration"], 1);
        assert_eq!(persisted[1]["key"], "restart-stop");
        assert_eq!(persisted[1]["desiredGeneration"], 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restart_prunes_expired_unowned_pending_rows_for_bounded_recovery() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-expired-recovery-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let lifecycle_request = request(&zone, GuestLifecycleOperation::Start, "expired-recovery");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        assert_eq!(
            dispatch.admit(&caller, &lifecycle_request),
            Ok(LifecycleDispatch::Dispatch)
        );
        drop(dispatch);

        let mut persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read pending state"))
                .expect("parse pending state");
        persisted[0]["admittedAtMs"] = serde_json::Value::from(0_u64);
        std::fs::write(
            &path,
            serde_json::to_vec(&persisted).expect("serialize expired state"),
        )
        .expect("write expired state");

        let restarted = ProviderLifecycleDispatch::new_persistent(zone, &path).expect("restart");
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls: Arc::clone(&calls),
            reject: AtomicBool::new(false),
        };
        assert_eq!(
            restarted.dispatch(&caller, &lifecycle_request, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_start_stop_start_stop_retires_opposite_applied_entries() {
        let root = crate::test_scratch_root()
            .join(format!("provider-lifecycle-cycles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let calls = Arc::new(AtomicUsize::new(0));
        let effect = RecordingEffect {
            calls: Arc::clone(&calls),
            reject: AtomicBool::new(false),
        };
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), root.join("lifecycle.json"))
                .expect("open state");
        for (operation, key) in [
            (GuestLifecycleOperation::Start, "stable-start"),
            (GuestLifecycleOperation::Stop, "stable-stop"),
            (GuestLifecycleOperation::Start, "stable-start"),
            (GuestLifecycleOperation::Stop, "stable-stop"),
        ] {
            let request = request(&zone, operation, key);
            assert!(matches!(
                dispatch.dispatch(&caller, &request, &effect),
                Ok(EffectDispatch::Dispatched(_))
            ));
        }
        assert_eq!(calls.load(Ordering::Acquire), 4);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lifecycle_state_persists_the_complete_guest_identity() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-identity-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let authorization = LifecycleAuthorization::for_test(
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            4,
            9,
            7,
            "identity-key",
        );
        let request = GuestLifecycleRequest::new(
            zone.clone(),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            GuestLifecycleOperation::Start,
            "identity-key",
            authorization,
        )
        .expect("request");
        let dispatch = ProviderLifecycleDispatch::new_persistent(zone, &path).expect("open state");
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &request, &effect,),
            Ok(EffectDispatch::Dispatched(1))
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read identity state"))
                .expect("parse identity state");
        assert_eq!(
            persisted[0]["zoneUid"],
            "11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(
            persisted[0]["guestUid"],
            "22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(persisted[0]["guestGeneration"], 4);
        assert_eq!(persisted[0]["providerAssignmentGeneration"], 9);
        assert_eq!(persisted[0]["policyRevision"], 7);
        assert_eq!(persisted[0]["operationId"], "identity-key");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stale_lifecycle_identity_is_quarantined_without_replay_or_deletion() {
        let root = crate::test_scratch_root().join(format!(
            "provider-lifecycle-stale-identity-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        let zone = ZoneId::parse("work").expect("Zone");
        let original = GuestLifecycleRequest::new(
            zone.clone(),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            GuestLifecycleOperation::Start,
            "stale-key",
            LifecycleAuthorization::for_test(
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                4,
                9,
                7,
                "stale-key",
            ),
        )
        .expect("original request");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(zone.clone(), &path).expect("open state");
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        assert_eq!(
            dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &original,
                &effect,
            ),
            Ok(EffectDispatch::Dispatched(1))
        );
        drop(dispatch);

        let replacement = GuestLifecycleRequest::new(
            zone,
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            GuestLifecycleOperation::Start,
            "stale-key",
            LifecycleAuthorization::for_test(
                "11111111-1111-4111-8111-111111111111",
                "33333333-3333-4333-8333-333333333333",
                4,
                9,
                7,
                "stale-key",
            ),
        )
        .expect("replacement request");
        let restarted =
            ProviderLifecycleDispatch::new_persistent(ZoneId::parse("work").expect("Zone"), &path)
                .expect("restore state");
        assert_eq!(
            restarted.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &replacement,
                &effect,
            ),
            Err(ProviderEffectError::IdentityMismatch)
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read quarantined state"))
                .expect("parse quarantined state");
        assert_eq!(persisted[0]["quarantined"], true);
        assert_eq!(
            persisted[0]["guestUid"],
            "22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(effect.calls.load(Ordering::Acquire), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn host_shutdown_authorization_is_stop_only() {
        let zone = ZoneId::parse("work").expect("Zone");
        let authorization = LifecycleAuthorization::host_shutdown(
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("Zone UID"),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("Guest UID"),
            ResourceGeneration::new(1).expect("Guest generation"),
            ResourceGeneration::new(1).expect("Provider assignment generation"),
            1,
            "shutdown-start".to_owned(),
        )
        .expect("HostShutdown lease");
        let error = GuestLifecycleRequest::new(
            zone,
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            GuestLifecycleOperation::Start,
            "shutdown-start",
            authorization,
        )
        .expect_err("HostShutdown must not authorize start");
        assert_eq!(error, ProviderEffectError::StopOnlyLease);
    }

    #[test]
    fn lifecycle_target_substitution_is_rejected_before_effect_admission() {
        let authorization = LifecycleAuthorization::for_test_with_guest(
            "Guest/other",
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            1,
            1,
            1,
            "target-substitution",
        );
        let error = GuestLifecycleRequest::new(
            ZoneId::parse("work").expect("Zone"),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            GuestLifecycleOperation::Start,
            "target-substitution",
            authorization,
        )
        .expect_err("target substitution must fail closed");
        assert_eq!(error, ProviderEffectError::AuthorizationLeaseInvalid);
    }

    #[test]
    fn uidless_lifecycle_row_is_quarantined_and_retained() {
        let root = crate::test_scratch_root()
            .join(format!("provider-lifecycle-uidless-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lifecycle.json");
        std::fs::create_dir_all(&root).expect("create state directory");
        std::fs::write(
            &path,
            serde_json::to_vec(&[serde_json::json!({
                "key": "uidless",
                "guest": "Guest/workstation",
                "operation": "start",
                "admitted_at_ms": now_ms(),
                "desired_generation": 1,
                "status": "pending",
            })])
            .expect("serialize uidless row"),
        )
        .expect("write uidless row");
        let dispatch =
            ProviderLifecycleDispatch::new_persistent(ZoneId::parse("work").unwrap(), &path)
                .expect("open state");
        let uidless_request = request(
            &ZoneId::parse("work").unwrap(),
            GuestLifecycleOperation::Start,
            "uidless",
        );
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        assert_eq!(
            dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &uidless_request,
                &effect,
            ),
            Err(ProviderEffectError::IdentityMismatch)
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read retained row"))
                .expect("parse retained row");
        assert_eq!(persisted[0]["quarantined"], true);
        assert_eq!(persisted[0]["guest"], "Guest/workstation");
        assert_eq!(effect.calls.load(Ordering::Acquire), 0);

        let fresh = request(
            &ZoneId::parse("work").unwrap(),
            GuestLifecycleOperation::Start,
            "fresh-after-quarantine",
        );
        assert_eq!(
            dispatch.dispatch(&BrokerCallerRole::AdminUid { uid: 1000 }, &fresh, &effect,),
            Ok(EffectDispatch::Dispatched(1))
        );
        let retained: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read retained quarantine"))
                .expect("parse retained quarantine");
        assert_eq!(retained.as_array().map(|entries| entries.len()), Some(2));
        assert_eq!(
            retained
                .as_array()
                .and_then(|entries| entries.iter().find(|entry| entry["key"] == "uidless"))
                .and_then(|entry| entry["quarantined"].as_bool()),
            Some(true)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn same_name_recreated_guest_does_not_block_new_identity() {
        let zone = ZoneId::parse("work").expect("Zone");
        let dispatch = Arc::new(ProviderLifecycleDispatch::new(zone.clone()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered, entered_rx) = channel();
        let effect = LongRunningEffect {
            entered,
            release: Arc::new(Barrier::new(2)),
            calls: Arc::clone(&calls),
        };
        let old_request = request(&zone, GuestLifecycleOperation::Start, "old-guest");
        let old_dispatch = Arc::clone(&dispatch);
        let old_effect = effect.clone();
        let old_thread = std::thread::spawn(move || {
            old_dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &old_request,
                &old_effect,
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("old Guest effect entered");

        let recreated = request_with_guest(
            &zone,
            GuestLifecycleOperation::Start,
            "new-guest",
            "33333333-3333-4333-8333-333333333333",
        );
        assert_eq!(
            dispatch.dispatch(
                &BrokerCallerRole::AdminUid { uid: 1000 },
                &recreated,
                &effect,
            ),
            Ok(EffectDispatch::Dispatched(2))
        );

        effect.release.wait();
        assert_eq!(
            old_thread.join().expect("old Guest thread"),
            Ok(EffectDispatch::Dispatched(1))
        );
        assert_eq!(calls.load(Ordering::Acquire), 2);
    }

    #[test]
    fn lifecycle_identity_fence_includes_generation_assignment_and_policy() {
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        let original = request(&zone, GuestLifecycleOperation::Start, "original");
        assert_eq!(
            dispatch.dispatch(&caller, &original, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );

        let replacement = GuestLifecycleRequest::new(
            zone,
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            GuestLifecycleOperation::Stop,
            "replacement",
            LifecycleAuthorization::for_test_with_guest(
                "Guest/workstation",
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                2,
                3,
                4,
                "replacement",
            ),
        )
        .expect("replacement request");
        assert_eq!(
            dispatch.dispatch(&caller, &replacement, &effect),
            Ok(EffectDispatch::Dispatched(2))
        );
        assert!(
            dispatch
                .is_latest(&caller, &original)
                .expect("original identity remains independently latest"),
            "a new Guest generation or Provider assignment must not supersede the old identity"
        );
    }

    #[test]
    fn superseded_stop_admission_cannot_authorize_restart_start() {
        let zone = ZoneId::parse("work").expect("Zone");
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let dispatch = ProviderLifecycleDispatch::new(zone.clone());
        let effect = RecordingEffect {
            calls: Arc::new(AtomicUsize::new(0)),
            reject: AtomicBool::new(false),
        };
        let stop = request(&zone, GuestLifecycleOperation::Stop, "restart-stop");
        assert_eq!(
            dispatch.dispatch(&caller, &stop, &effect),
            Ok(EffectDispatch::Dispatched(1))
        );

        let newer_start = request(&zone, GuestLifecycleOperation::Start, "superseding-start");
        assert_eq!(
            dispatch.admit(&caller, &newer_start),
            Ok(LifecycleDispatch::Dispatch)
        );
        assert!(
            !dispatch
                .is_latest(&caller, &stop)
                .expect("stop admission state"),
            "restart must refuse to start after its stop admission is superseded"
        );
    }
}
