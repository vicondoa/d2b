//! Zone-scoped Provider lifecycle effect dispatch.
//!
//! Requests are addressed by `Zone/<zone>` plus `Guest/<name>` and are
//! deduplicated by an opaque idempotency key. The actual start/stop effect is
//! supplied by a descriptor-bound broker adapter; this planner never opens a
//! socket or mutates host state.

use std::collections::BTreeMap;

use d2b_contracts::v3::{ResourceRef, identity::ZoneId};

/// Maximum retained lifecycle mutation keys.
pub const MAX_TRACKED_LIFECYCLE_MUTATIONS: usize = 256;

/// Closed caller roles allowed to request a Provider lifecycle effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerCallerRole {
    /// The Zone controller.
    ZoneController,
    /// An explicitly authorized configuration controller.
    ConfigurationController,
    /// Any other caller, which is refused.
    Other,
}

/// Guest lifecycle operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLifecycleOperation {
    Start,
    Stop,
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
        }
    }
}

impl core::fmt::Display for ProviderEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderEffectError {}

/// Result of dispatching one lifecycle request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleDispatch {
    /// The caller may invoke the descriptor-bound effect port.
    Dispatch,
    /// The exact request was already accepted under this idempotency key.
    Duplicate,
}

/// Effect-free lifecycle dispatcher with bounded idempotency tracking.
#[derive(Debug)]
pub struct ProviderLifecycleDispatch {
    zone: ZoneId,
    mutations: BTreeMap<String, (ResourceRef, GuestLifecycleOperation)>,
}

impl ProviderLifecycleDispatch {
    /// Construct a dispatcher for one Zone.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            mutations: BTreeMap::new(),
        }
    }

    /// Admit one request after checking caller role, Zone, and deduplication.
    pub fn admit(
        &mut self,
        caller: BrokerCallerRole,
        request: &GuestLifecycleRequest,
    ) -> Result<LifecycleDispatch, ProviderEffectError> {
        if !matches!(
            caller,
            BrokerCallerRole::ZoneController | BrokerCallerRole::ConfigurationController
        ) {
            return Err(ProviderEffectError::CallerRoleDenied);
        }
        if request.zone() != &self.zone {
            return Err(ProviderEffectError::ZoneMismatch);
        }
        if let Some((guest, operation)) = self.mutations.get(request.idempotency_key()) {
            if guest == request.guest() && *operation == request.operation() {
                return Ok(LifecycleDispatch::Duplicate);
            }
            return Err(ProviderEffectError::IdempotencyConflict);
        }
        if self.mutations.len() >= MAX_TRACKED_LIFECYCLE_MUTATIONS {
            return Err(ProviderEffectError::MutationTableFull);
        }
        self.mutations.insert(
            request.idempotency_key().to_owned(),
            (request.guest().clone(), request.operation()),
        );
        Ok(LifecycleDispatch::Dispatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_resource_lifecycle_is_zone_scoped_and_idempotent() {
        let zone = ZoneId::parse("work").unwrap();
        let guest = ResourceRef::parse("Guest/workstation").unwrap();
        let request =
            GuestLifecycleRequest::new(zone.clone(), guest, GuestLifecycleOperation::Start, "k1")
                .unwrap();
        let mut dispatch = ProviderLifecycleDispatch::new(zone);
        assert_eq!(
            dispatch
                .admit(BrokerCallerRole::ZoneController, &request)
                .unwrap(),
            LifecycleDispatch::Dispatch
        );
        assert_eq!(
            dispatch
                .admit(BrokerCallerRole::ZoneController, &request)
                .unwrap(),
            LifecycleDispatch::Duplicate
        );
        assert_eq!(
            dispatch
                .admit(BrokerCallerRole::Other, &request)
                .unwrap_err(),
            ProviderEffectError::CallerRoleDenied
        );
    }
}
