//! Descriptor-bound Provider lifecycle effects.
//!
//! The daemon owns the lifecycle dispatcher, but it does not own a second
//! broker protocol.  A caller supplies a typed effect port and this module
//! performs only the Zone, caller-role, and idempotency admission that belongs
//! at the Provider boundary.  The production port is implemented by `d2bd`
//! with the existing typed broker dispatch functions.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    v3::{ResourceRef, identity::ZoneId},
};

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
}

impl GuestLifecycleOperation {
    /// Stable operation token used in idempotency diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

/// A v3 Guest lifecycle request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLifecycleRequest {
    zone: ZoneId,
    guest: ResourceRef,
    operation: GuestLifecycleOperation,
    idempotency_key: String,
}

impl GuestLifecycleRequest {
    /// Construct a request addressed by a same-Zone Guest ResourceRef.
    pub fn new(
        zone: ZoneId,
        guest: ResourceRef,
        operation: GuestLifecycleOperation,
        idempotency_key: impl Into<String>,
    ) -> Result<Self, ProviderEffectError> {
        if guest.resource_type().as_str() != "Guest" {
            return Err(ProviderEffectError::GuestRefInvalid);
        }
        let idempotency_key = idempotency_key.into();
        if idempotency_key.is_empty() || idempotency_key.len() > 128 {
            return Err(ProviderEffectError::IdempotencyKeyInvalid);
        }
        Ok(Self {
            zone,
            guest,
            operation,
            idempotency_key,
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
    /// The configured Provider registry is unavailable.
    RegistryUnavailable,
    /// No registered Provider owns the requested Guest route.
    ProviderNotRegistered,
    /// The selected Provider does not publish the requested lifecycle method.
    ProviderCapabilityDenied,
    /// The typed effect port refused or failed the mutation.
    EffectRejected,
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
            Self::RegistryUnavailable => "provider-effect-registry-unavailable",
            Self::ProviderNotRegistered => "provider-effect-provider-not-registered",
            Self::ProviderCapabilityDenied => "provider-effect-provider-capability-denied",
            Self::EffectRejected => "provider-effect-rejected",
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
    /// The exact request was already accepted under this idempotency key.
    Duplicate,
}

/// Result of invoking a typed lifecycle effect.
#[derive(Debug, PartialEq, Eq)]
pub enum EffectDispatch<T> {
    /// The effect port ran and returned its typed output.
    Dispatched(T),
    /// The exact request was already accepted and the effect was not invoked.
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

    /// Apply one already-admitted lifecycle request.
    fn apply(&self, request: &GuestLifecycleRequest) -> Result<Self::Output, ProviderEffectError>;
}

/// Effect-free lifecycle dispatcher with bounded idempotency tracking.
#[derive(Debug)]
pub struct ProviderLifecycleDispatch {
    zone: ZoneId,
    mutations: Mutex<BTreeMap<String, LifecycleMutation>>,
    state_path: Option<PathBuf>,
}

#[derive(Debug)]
struct LifecycleMutation {
    guest: ResourceRef,
    operation: GuestLifecycleOperation,
    admitted_at_ms: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedLifecycleMutation {
    key: String,
    guest: String,
    operation: String,
    admitted_at_ms: u64,
}

impl ProviderLifecycleDispatch {
    /// Construct a dispatcher for one Zone.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            mutations: Mutex::new(BTreeMap::new()),
            state_path: None,
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
        match fs::read(&state_path) {
            Ok(bytes) => {
                let persisted = serde_json::from_slice::<Vec<PersistedLifecycleMutation>>(&bytes)
                    .map_err(|_| ProviderEffectError::StateUnavailable)?;
                for entry in persisted {
                    if entry.key.is_empty() || entry.key.len() > 128 {
                        return Err(ProviderEffectError::StateUnavailable);
                    }
                    let guest = ResourceRef::parse(&entry.guest)
                        .map_err(|_| ProviderEffectError::StateUnavailable)?;
                    let operation = match entry.operation.as_str() {
                        "start" => GuestLifecycleOperation::Start,
                        "stop" => GuestLifecycleOperation::Stop,
                        _ => return Err(ProviderEffectError::StateUnavailable),
                    };
                    if mutations
                        .insert(
                            entry.key,
                            LifecycleMutation {
                                guest,
                                operation,
                                admitted_at_ms: entry.admitted_at_ms,
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
        if !caller_is_authorized(caller) {
            return Err(ProviderEffectError::CallerRoleDenied);
        }
        if request.zone() != &self.zone {
            return Err(ProviderEffectError::ZoneMismatch);
        }
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        let now = now_ms();
        mutations.retain(|_, mutation| {
            now.saturating_sub(mutation.admitted_at_ms)
                < LIFECYCLE_IDEMPOTENCY_TTL.as_millis() as u64
        });
        if let Some(mutation) = mutations.get(request.idempotency_key()) {
            if mutation.guest == *request.guest() && mutation.operation == request.operation() {
                return Ok(LifecycleDispatch::Duplicate);
            }
            return Err(ProviderEffectError::IdempotencyConflict);
        }
        if mutations.len() >= MAX_TRACKED_LIFECYCLE_MUTATIONS {
            return Err(ProviderEffectError::MutationTableFull);
        }
        mutations.insert(
            request.idempotency_key().to_owned(),
            LifecycleMutation {
                guest: request.guest().clone(),
                operation: request.operation(),
                admitted_at_ms: now,
            },
        );
        if let Err(error) = self.persist_locked(&mutations) {
            mutations.remove(request.idempotency_key());
            return Err(error);
        }
        Ok(LifecycleDispatch::Dispatch)
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
        match self.admit(caller, request)? {
            LifecycleDispatch::Duplicate => Ok(EffectDispatch::Duplicate),
            LifecycleDispatch::Dispatch => match effect.apply(request) {
                Ok(output) => {
                    if request.operation() == GuestLifecycleOperation::Stop {
                        self.retire_completed_start(request)?;
                    }
                    Ok(EffectDispatch::Dispatched(output))
                }
                Err(error) => {
                    self.remove(request)?;
                    Err(error)
                }
            },
        }
    }

    fn remove(&self, request: &GuestLifecycleRequest) -> Result<(), ProviderEffectError> {
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        mutations.remove(request.idempotency_key());
        self.persist_locked(&mutations)
    }

    fn retire_completed_start(
        &self,
        request: &GuestLifecycleRequest,
    ) -> Result<(), ProviderEffectError> {
        let mut mutations = self
            .mutations
            .lock()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        mutations.retain(|_, mutation| {
            mutation.guest != *request.guest()
                || mutation.operation != GuestLifecycleOperation::Start
        });
        self.persist_locked(&mutations)
    }

    fn retain_live(&self, mutations: &mut BTreeMap<String, LifecycleMutation>) {
        let now = now_ms();
        mutations.retain(|_, mutation| {
            now.saturating_sub(mutation.admitted_at_ms)
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
            .map(|(key, mutation)| PersistedLifecycleMutation {
                key: key.clone(),
                guest: mutation.guest.to_canonical_string(),
                operation: mutation.operation.as_str().to_owned(),
                admitted_at_ms: mutation.admitted_at_ms,
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
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

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

    fn request(
        zone: &ZoneId,
        operation: GuestLifecycleOperation,
        key: &str,
    ) -> GuestLifecycleRequest {
        GuestLifecycleRequest::new(
            zone.clone(),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            operation,
            key,
        )
        .expect("lifecycle request")
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
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
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
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_start_stop_start_accepts_distinct_operation_ids() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
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
            (GuestLifecycleOperation::Start, "start-1"),
            (GuestLifecycleOperation::Stop, "stop-1"),
            (GuestLifecycleOperation::Start, "start-2"),
        ] {
            let request = request(&zone, operation, key);
            assert!(matches!(
                dispatch.dispatch(&caller, &request, &effect),
                Ok(EffectDispatch::Dispatched(_))
            ));
        }
        assert_eq!(calls.load(Ordering::Acquire), 3);
        let _ = std::fs::remove_dir_all(root);
    }
}
