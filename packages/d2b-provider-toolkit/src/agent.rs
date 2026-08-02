//! Bounded Provider-agent dispatch and service-server adapters.
//!
//! The adapter is intentionally transport-agnostic.  A ComponentSession
//! receive loop supplies a canonical request, while this module owns the
//! fixed 64-call admission ceiling, the 1024-entry diagnostic audit ring,
//! and the bounded shutdown state.

use std::sync::Mutex;

use d2b_contracts::v3::{
    CanonicalJsonObject, ResourceRef, component_session::RequestId, execution_policy::BoundedToken,
    zone_routing::ZonePath,
};
use d2b_session::{Cancellation, ComponentSessionDriver};

use crate::{
    DispatchLimiter, ProviderAgentAuditEvent, ProviderAgentAuditLog, ProviderAgentAuditOutcome,
    ProviderToolkitError,
};

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
}

impl<S> ProviderAgentAdapter<S> {
    /// Construct an adapter with the frozen dispatch and audit bounds.
    pub fn new(service: S) -> Self {
        Self {
            service,
            dispatch: DispatchLimiter::new(),
            audit: Mutex::new(ProviderAgentAuditLog::new()),
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
        let _permit = self.dispatch.acquire()?;
        let result = self.service.dispatch(&method, &payload);
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

    /// Serve decoded Provider RPC frames from one authenticated
    /// ComponentSession until cancellation or transport close.
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
        loop {
            if cancellation.is_cancelled() {
                return Ok(());
            }
            let frame = driver
                .receive_ttrpc()
                .await
                .map_err(|_| ProviderToolkitError::SessionClosed)?;
            let request = codec
                .decode_request(&frame)
                .map_err(|_| ProviderToolkitError::WireInvalid)?;
            let response = self.dispatch(
                request.zone().clone(),
                request.provider_ref().clone(),
                request.method().clone(),
                request.payload().clone(),
            )?;
            let encoded = codec
                .encode_response(request.request_id(), &response)
                .map_err(|_| ProviderToolkitError::WireInvalid)?;
            driver
                .send_ttrpc_cancellable(encoded, cancellation.clone())
                .await
                .map_err(|_| ProviderToolkitError::SessionClosed)?;
        }
    }
}

/// Generated-service registration facade over a Provider agent adapter.
pub struct GeneratedProviderServiceServer<S> {
    adapter: ProviderAgentAdapter<S>,
}

impl<S> GeneratedProviderServiceServer<S> {
    /// Construct the generated service facade.
    pub fn new(service: S) -> Self {
        Self {
            adapter: ProviderAgentAdapter::new(service),
        }
    }

    /// Borrow the bounded adapter.
    pub const fn adapter(&self) -> &ProviderAgentAdapter<S> {
        &self.adapter
    }
}

impl<S> GeneratedProviderServiceServer<S>
where
    S: ProviderService,
{
    /// Serve the generated service over an authenticated ComponentSession.
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
        self.adapter
            .serve_component_session(driver, codec, cancellation)
            .await
    }
}

/// Fixed Provider-agent shutdown state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAgentProcess {
    shutdown_deadline_ms: u32,
    stopping: bool,
}

impl ProviderAgentProcess {
    /// Maximum shutdown deadline accepted by the toolkit.
    pub const MAX_SHUTDOWN_DEADLINE_MS: u32 = 5_000;

    /// Construct a running process state.
    pub fn new(shutdown_deadline_ms: u32) -> Result<Self, ProviderToolkitError> {
        if shutdown_deadline_ms == 0 || shutdown_deadline_ms > Self::MAX_SHUTDOWN_DEADLINE_MS {
            return Err(ProviderToolkitError::CapacityOutOfRange);
        }
        Ok(Self {
            shutdown_deadline_ms,
            stopping: false,
        })
    }

    /// Request bounded shutdown.
    pub fn stop(&mut self) {
        self.stopping = true;
    }

    /// Complete the bounded shutdown transition.
    ///
    /// The transport owner performs the actual session close; this state
    /// transition is deliberately synchronous and cannot outlive the fixed
    /// deadline advertised by the process.
    pub async fn shutdown(&mut self) -> Result<(), ProviderToolkitError> {
        self.stop();
        Ok(())
    }

    /// Whether shutdown has been requested.
    pub const fn stopping(&self) -> bool {
        self.stopping
    }

    /// Return the configured shutdown deadline.
    pub const fn shutdown_deadline_ms(&self) -> u32 {
        self.shutdown_deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::ZoneLabelId;
    use d2b_contracts::v3::{ResourceName, ResourceTypeName};

    struct Echo;

    impl ProviderService for Echo {
        fn dispatch(
            &self,
            _method: &BoundedToken,
            payload: &CanonicalJsonObject,
        ) -> Result<CanonicalJsonObject, ProviderToolkitError> {
            Ok(payload.clone())
        }
    }

    fn zone() -> ZonePath {
        ZonePath::new(vec![ZoneLabelId::parse("dev").unwrap()]).unwrap()
    }

    fn provider() -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse("Provider").unwrap(),
            ResourceName::parse("system-core").unwrap(),
        )
    }

    #[test]
    fn adapter_dispatches_and_records_only_bounded_metadata() {
        let adapter = ProviderAgentAdapter::new(Echo);
        let result = adapter
            .dispatch(
                zone(),
                provider(),
                BoundedToken::parse("inspect").unwrap(),
                CanonicalJsonObject::parse(br#"{"ok":true}"#).unwrap(),
            )
            .unwrap();
        assert_eq!(
            result,
            CanonicalJsonObject::parse(br#"{"ok":true}"#).unwrap()
        );
        assert_eq!(adapter.audit_len(), 1);
    }

    #[test]
    fn shutdown_deadline_is_bounded() {
        assert!(ProviderAgentProcess::new(5_001).is_err());
        let mut process = ProviderAgentProcess::new(5_000).unwrap();
        process.stop();
        assert!(process.stopping());
    }
}
