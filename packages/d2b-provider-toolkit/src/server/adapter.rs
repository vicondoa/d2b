//! Bounded Provider-agent dispatch: the adapter, the generated frame codec
//! it decodes through, and the frozen in-flight accounting behind it.
//!
//! The adapter is intentionally transport-agnostic. A ComponentSession
//! receive loop supplies a canonical request, while this module owns the
//! fixed 64-call admission ceiling and the 1024-entry diagnostic audit ring.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ResourceRef, execution_policy::BoundedToken,
};
use d2b_contracts_zone_session::v3::{
    component_session::RequestId,
    zone_routing::{ZoneLabelId, ZonePath},
};
use d2b_session::{AuthenticatedSessionRouteBinding, Cancellation, ComponentSessionDriver};

use crate::audit::{ProviderAgentAuditEvent, ProviderAgentAuditLog, ProviderAgentAuditOutcome};
use crate::base::error::ProviderToolkitError;
use crate::base::runtime::same_controller_identity;
use tracing::warn;

/// Validate the strict attachment-index sequence carried by a Provider
/// adapter.  Descriptors are numbered from zero and may not repeat, reorder,
/// or skip an index; rejecting before dispatch prevents an adapter from
/// confusing a stale attachment with a current one.
pub fn validate_attachment_indexes(indexes: &[u32]) -> Result<(), ProviderToolkitError> {
    for (expected, observed) in indexes.iter().enumerate() {
        if *observed != expected as u32 {
            return Err(ProviderToolkitError::NonMonotoneAttachmentIndexes);
        }
    }
    Ok(())
}

/// Provider-specific service implementation behind the generic adapter.
pub trait ProviderService: Send + Sync {
    /// Dispatch one canonical method payload.
    fn dispatch(
        &self,
        method: &BoundedToken,
        payload: &CanonicalJsonObject,
    ) -> Result<CanonicalJsonObject, ProviderToolkitError>;
}

/// One decoded request carried by an authenticated ComponentSession.
pub struct ProviderRequest {
    request_id: RequestId,
    zone: ZonePath,
    provider_ref: ResourceRef,
    method: BoundedToken,
    payload: CanonicalJsonObject,
}

impl ProviderRequest {
    /// Build a decoded request after binding all routing identity locally.
    pub fn new(
        request_id: RequestId,
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        payload: CanonicalJsonObject,
    ) -> Self {
        Self {
            request_id,
            zone,
            provider_ref,
            method,
            payload,
        }
    }

    /// Borrow the authenticated request correlation.
    pub const fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// Borrow the Zone routing identity.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// Borrow the Provider resource identity.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the method.
    pub const fn method(&self) -> &BoundedToken {
        &self.method
    }

    /// Borrow the canonical payload.
    pub const fn payload(&self) -> &CanonicalJsonObject {
        &self.payload
    }
}

impl std::fmt::Debug for ProviderRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProviderRequest(<redacted>)")
    }
}

/// Codec owned by generated v3 service bindings.
pub trait ProviderFrameCodec: Send + Sync {
    /// Decode one authenticated request frame.
    fn decode_request(&self, frame: &[u8]) -> Result<ProviderRequest, ProviderToolkitError>;

    /// Encode one response frame for the request correlation.
    fn encode_response(
        &self,
        request_id: &RequestId,
        payload: &CanonicalJsonObject,
    ) -> Result<Vec<u8>, ProviderToolkitError>;
}

/// Bounded Provider-agent adapter.
pub struct ProviderAgentAdapter<S> {
    service: S,
    dispatch: DispatchLimiter,
    audit: Mutex<ProviderAgentAuditLog>,
    authenticated_route: Mutex<Option<AuthenticatedSessionRouteBinding>>,
}

impl<S> ProviderAgentAdapter<S> {
    /// Construct an adapter with the frozen dispatch and audit bounds.
    pub fn new(service: S) -> Self {
        Self {
            service,
            dispatch: DispatchLimiter::new(),
            audit: Mutex::new(ProviderAgentAuditLog::new()),
            authenticated_route: Mutex::new(None),
        }
    }

    /// Borrow the service implementation.
    pub const fn service(&self) -> &S {
        &self.service
    }

    /// Borrow dispatch accounting.
    pub const fn dispatch_limiter(&self) -> &DispatchLimiter {
        &self.dispatch
    }

    /// Snapshot retained audit events.
    pub fn audit_len(&self) -> usize {
        self.audit
            .lock()
            .map(|audit| audit.len())
            .unwrap_or_default()
    }

    /// Bind the adapter to one authenticated controller route.
    ///
    /// A route is admission evidence only. It does not grant a ResourceClient
    /// or effect capability, and a second route cannot replace the first one
    /// while this adapter is alive.
    pub fn bind_authenticated_route(
        &self,
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<(), ProviderToolkitError> {
        if route.provider_ref().is_none()
            || route.provider_generation().is_none()
            || route.controller_generation().is_none()
            || route.reconnect_generation().get() == 0
        {
            warn!(provider = ?route.provider_ref(), "authenticated route bind refused: route is missing provider, generation, or reconnect identity");
            return Err(ProviderToolkitError::SessionUnauthenticated);
        }
        let mut bound = self.authenticated_route.lock().map_err(|_| {
            warn!(provider = ?route.provider_ref(), "authenticated route bind failed: adapter route state lock poisoned");
            ProviderToolkitError::SessionUnauthenticated
        })?;
        match bound.as_ref() {
            None => {
                *bound = Some(route);
                Ok(())
            }
            Some(existing) if existing == &route => Ok(()),
            Some(existing)
                if same_controller_identity(existing, &route)
                    && route.reconnect_generation() > existing.reconnect_generation() =>
            {
                *bound = Some(route);
                Ok(())
            }
            Some(_) => {
                warn!(provider = ?route.provider_ref(), "authenticated route bind refused: route conflicts with an already-bound controller route");
                Err(ProviderToolkitError::SessionUnauthenticated)
            }
        }
    }

    /// Whether an authenticated controller route has been bound.
    pub fn has_authenticated_route(&self) -> bool {
        self.authenticated_route
            .lock()
            .ok()
            .is_some_and(|route| route.is_some())
    }
}

impl<S> ProviderAgentAdapter<S>
where
    S: ProviderService,
{
    /// Dispatch one request under bounded admission and record its outcome.
    pub fn dispatch(
        &self,
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        payload: CanonicalJsonObject,
    ) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let bound = self.authenticated_route.lock().map_err(|_| {
            warn!(zone = ?zone, provider = %provider_ref, method = ?method, "dispatch refused: adapter route state lock poisoned");
            ProviderToolkitError::SessionUnauthenticated
        })?;
        if let Some(route) = bound.as_ref() {
            Self::validate_bound_request(route, &zone, &provider_ref)?;
        }
        self.dispatch_inner(zone, provider_ref, method, payload)
    }

    fn dispatch_for_route(
        &self,
        route: &AuthenticatedSessionRouteBinding,
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        payload: CanonicalJsonObject,
    ) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let bound = self.authenticated_route.lock().map_err(|_| {
            warn!(zone = ?zone, provider = %provider_ref, method = ?method, "dispatch refused: adapter route state lock poisoned");
            ProviderToolkitError::SessionUnauthenticated
        })?;
        let current_route = bound.as_ref().ok_or_else(|| {
            warn!(zone = ?zone, provider = %provider_ref, method = ?method, "dispatch refused: no controller route is bound to the adapter");
            ProviderToolkitError::SessionUnauthenticated
        })?;
        if current_route != route {
            warn!(zone = ?zone, provider = %provider_ref, method = ?method, "dispatch refused: request route no longer matches the bound controller route");
            return Err(ProviderToolkitError::SessionUnauthenticated);
        }
        Self::validate_bound_request(route, &zone, &provider_ref)?;
        self.dispatch_inner(zone, provider_ref, method, payload)
    }

    fn dispatch_inner(
        &self,
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        payload: CanonicalJsonObject,
    ) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let _permit = self.dispatch.acquire().map_err(|e| {
            warn!(zone = ?zone, provider = %provider_ref, method = ?method, reason = %e, "dispatch refused: dispatch admission ceiling saturated");
            e
        })?;
        let result = self.service.dispatch(&method, &payload);
        if let Err(e) = result.as_ref() {
            warn!(zone = ?zone, provider = %provider_ref, method = ?method, reason = %e, "provider service dispatch failed");
        }
        let outcome = if result.is_ok() {
            ProviderAgentAuditOutcome::Accepted
        } else {
            ProviderAgentAuditOutcome::Failed
        };
        if let Ok(mut audit) = self.audit.lock() {
            audit.record(ProviderAgentAuditEvent::new(
                zone,
                provider_ref,
                method,
                outcome,
            ));
        }
        result
    }

    /// Serve decoded Provider frames from one authenticated
    /// ComponentSession until cancellation or transport close.
    ///
    /// Callers must bind the authenticated route before entering this loop.
    /// Every decoded target is checked against that binding.
    ///
    /// Session authentication, generation binding, attachment policy, and
    /// stream fairness remain owned by `d2b-session`; this loop only bridges
    /// the generated frame codec to the bounded Provider service adapter.
    pub async fn serve_component_session<D, C>(
        &self,
        driver: &D,
        codec: &C,
        cancellation: Cancellation,
    ) -> Result<(), ProviderToolkitError>
    where
        D: ComponentSessionDriver,
        C: ProviderFrameCodec,
    {
        let route = self
            .authenticated_route
            .lock()
            .map_err(|_| {
                warn!("provider session refused: adapter route state lock poisoned");
                ProviderToolkitError::SessionUnauthenticated
            })?
            .clone()
            .ok_or(ProviderToolkitError::SessionUnauthenticated)?;
        loop {
            if cancellation.is_cancelled() {
                return Ok(());
            }
            let current_route = self
                .authenticated_route
                .lock()
                .map_err(|_| {
                    warn!("provider session loop aborted: adapter route state lock poisoned");
                    ProviderToolkitError::SessionUnauthenticated
                })?
                .clone()
                .ok_or(ProviderToolkitError::SessionUnauthenticated)?;
            if current_route != route {
                warn!(zone = ?route.zone(), "provider session loop aborted: bound controller route changed mid-session");
                return Err(ProviderToolkitError::SessionUnauthenticated);
            }
            let frame = driver.receive_ttrpc().await.map_err(|_| {
                warn!(zone = ?route.zone(), "provider session receive failed; closing session");
                ProviderToolkitError::SessionClosed
            })?;
            let request = codec.decode_request(&frame).map_err(|e| {
                warn!(zone = ?route.zone(), reason = %e, "provider frame decode failed; closing session");
                e
            })?;
            let response = self.dispatch_for_route(
                &route,
                request.zone().clone(),
                request.provider_ref().clone(),
                request.method().clone(),
                request.payload().clone(),
            )?;
            let encoded = codec
                .encode_response(request.request_id(), &response)
                .map_err(|e| {
                    warn!(zone = ?route.zone(), reason = %e, "provider response encode failed; closing session");
                    e
                })?;
            driver
                .send_ttrpc_cancellable(encoded, cancellation.clone())
                .await
                .map_err(|_| {
                    warn!(zone = ?route.zone(), "provider session send failed; closing session");
                    ProviderToolkitError::SessionClosed
                })?;
        }
    }

    fn validate_bound_request(
        route: &AuthenticatedSessionRouteBinding,
        zone: &ZonePath,
        provider_ref: &ResourceRef,
    ) -> Result<(), ProviderToolkitError> {
        let expected_zone = ZonePath::new(vec![ZoneLabelId::parse(route.zone().as_str()).map_err(
            |_| {
                warn!(zone = ?route.zone(), "dispatch refused: bound route zone label is not a valid zone root");
                ProviderToolkitError::SessionUnauthenticated
            },
        )?])
        .map_err(|_| {
            warn!(zone = ?route.zone(), "dispatch refused: bound route zone is not a valid zone root");
            ProviderToolkitError::SessionUnauthenticated
        })?;
        let expected_provider = route.provider_ref().ok_or_else(|| {
            warn!(zone = ?route.zone(), "dispatch refused: bound route has no provider reference");
            ProviderToolkitError::SessionUnauthenticated
        })?;
        if zone != &expected_zone || provider_ref != expected_provider {
            warn!(zone = ?zone, provider = %provider_ref, "dispatch refused: request target retargets a different zone or provider than the bound route");
            return Err(ProviderToolkitError::SessionUnauthenticated);
        }
        Ok(())
    }
}

/// The frozen maximum number of concurrent in-flight dispatches.
pub const MAX_DISPATCH_IN_FLIGHT: usize = 64;

/// Bounded in-flight accounting for one Provider agent.
///
/// The ceiling is frozen rather than configured, so a caller cannot make one
/// agent hold unbounded work, and saturation is a typed refusal rather than
/// an unbounded queue.
#[derive(Debug, Clone)]
pub struct DispatchLimiter {
    in_flight: Arc<AtomicUsize>,
    limit: usize,
}

impl DispatchLimiter {
    /// Build a limiter at the frozen ceiling.
    pub fn new() -> Self {
        Self::with_limit(MAX_DISPATCH_IN_FLIGHT).expect("the frozen ceiling is in range")
    }

    /// Build a limiter at an explicit limit.
    ///
    /// The limit is closed: zero and anything above
    /// [`MAX_DISPATCH_IN_FLIGHT`] are rejected, so no caller can widen the
    /// ceiling or disable admission.
    pub fn with_limit(limit: usize) -> Result<Self, ProviderToolkitError> {
        if limit == 0 || limit > MAX_DISPATCH_IN_FLIGHT {
            return Err(ProviderToolkitError::CapacityOutOfRange);
        }
        Ok(Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            limit,
        })
    }

    /// Reserve one dispatch slot, or refuse.
    pub fn acquire(&self) -> Result<DispatchPermit, ProviderToolkitError> {
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return Err(ProviderToolkitError::DispatchSaturated);
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(DispatchPermit {
                        in_flight: Arc::clone(&self.in_flight),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Return the frozen limit.
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Return the number of currently reserved slots.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }
}

impl Default for DispatchLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// One reserved dispatch slot, released when dropped.
///
/// The permit is not `Clone`: cloning it would release one slot twice and
/// let the agent exceed its frozen ceiling.
#[derive(Debug)]
pub struct DispatchPermit {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for DispatchPermit {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_closed_and_bounded() {
        assert_eq!(
            DispatchLimiter::with_limit(0).unwrap_err(),
            ProviderToolkitError::CapacityOutOfRange
        );
        assert_eq!(
            DispatchLimiter::with_limit(MAX_DISPATCH_IN_FLIGHT + 1).unwrap_err(),
            ProviderToolkitError::CapacityOutOfRange
        );
        assert_eq!(DispatchLimiter::new().limit(), MAX_DISPATCH_IN_FLIGHT);
    }

    #[test]
    fn saturation_refuses_and_a_released_permit_restores_the_slot() {
        let limiter = DispatchLimiter::with_limit(2).expect("valid limit");
        let first = limiter.acquire().expect("first slot");
        let second = limiter.acquire().expect("second slot");
        assert_eq!(limiter.in_flight(), 2);
        assert_eq!(
            limiter.acquire().unwrap_err(),
            ProviderToolkitError::DispatchSaturated
        );
        drop(second);
        assert_eq!(limiter.in_flight(), 1);
        let third = limiter.acquire().expect("reclaimed slot");
        assert_eq!(limiter.in_flight(), 2);
        drop(first);
        drop(third);
        assert_eq!(limiter.in_flight(), 0);
    }
}
