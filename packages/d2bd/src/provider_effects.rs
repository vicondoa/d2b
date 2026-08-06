//! Descriptor-bound Provider lifecycle effects.
//!
//! The daemon owns the lifecycle dispatcher, but it does not own a second
//! broker protocol.  A caller supplies a typed effect port and this module
//! performs only the Zone, caller-role, and idempotency admission that belongs
//! at the Provider boundary.  The production port is implemented by `d2bd`
//! with the existing typed broker dispatch functions.

use std::{collections::BTreeMap, sync::Mutex};

use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    v3::{ResourceRef, identity::ZoneId},
};

/// Maximum retained lifecycle mutation keys.
pub const MAX_TRACKED_LIFECYCLE_MUTATIONS: usize = 256;

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
    mutations: Mutex<BTreeMap<String, (ResourceRef, GuestLifecycleOperation)>>,
}

impl ProviderLifecycleDispatch {
    /// Construct a dispatcher for one Zone.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            mutations: Mutex::new(BTreeMap::new()),
        }
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
        if let Some((guest, operation)) = mutations.get(request.idempotency_key()) {
            if guest == request.guest() && *operation == request.operation() {
                return Ok(LifecycleDispatch::Duplicate);
            }
            return Err(ProviderEffectError::IdempotencyConflict);
        }
        if mutations.len() >= MAX_TRACKED_LIFECYCLE_MUTATIONS {
            return Err(ProviderEffectError::MutationTableFull);
        }
        mutations.insert(
            request.idempotency_key().to_owned(),
            (request.guest().clone(), request.operation()),
        );
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
                Ok(output) => Ok(EffectDispatch::Dispatched(output)),
                Err(error) => {
                    self.remove(request);
                    Err(error)
                }
            },
        }
    }

    fn remove(&self, request: &GuestLifecycleRequest) {
        if let Ok(mut mutations) = self.mutations.lock() {
            mutations.remove(request.idempotency_key());
        }
    }
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
}
