//! Authenticated Provider service-loop boundary.
//!
//! A Provider process may publish readiness only after its ComponentSession
//! route has been admitted and its generated service loop has a live,
//! authenticated session.  This module keeps the session authority in
//! `d2b-session`; it only consumes the redacted route binding and dispatches
//! frames through the bounded Provider adapter.

use d2b_contracts::v3::{
    CanonicalJsonObject, ResourceRef,
    component_session::RequestId,
    execution_policy::BoundedToken,
    zone_routing::{ZoneLabelId, ZonePath},
};
use d2b_session::{
    AuthenticatedComponentSession, AuthenticatedSessionRouteBinding, Cancellation,
    SessionAuthorizationRequest,
};

use crate::{
    ProviderAgentAdapter, ProviderService, ProviderToolkitError,
    runtime::{ProviderEntrypoint, ProviderRuntimeError, ProviderSessionAdmission},
};

/// A decoded Provider request whose authorization request is still owned by
/// the authenticated session.
pub struct AuthenticatedProviderRequest {
    request_id: RequestId,
    authorization: SessionAuthorizationRequest,
    zone: ZonePath,
    provider_ref: ResourceRef,
    method: BoundedToken,
    payload: CanonicalJsonObject,
}

impl AuthenticatedProviderRequest {
    /// Construct a request from generated bindings and the authenticated
    /// route. Generated bindings must derive `zone` and `provider_ref` from
    /// the supplied route rather than from caller payload fields.
    pub fn new(
        request_id: RequestId,
        authorization: SessionAuthorizationRequest,
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        payload: CanonicalJsonObject,
    ) -> Self {
        Self {
            request_id,
            authorization,
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

    /// Consume the exact authorization request.
    pub fn authorization(self) -> SessionAuthorizationRequest {
        self.authorization
    }

    /// Borrow the route-bound Zone.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// Borrow the route-bound Provider reference.
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

impl std::fmt::Debug for AuthenticatedProviderRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuthenticatedProviderRequest(<redacted>)")
    }
}

/// Generated Provider bindings for one authenticated ComponentSession.
pub trait AuthenticatedProviderFrameCodec: Send + Sync {
    /// Decode one protected frame using the route binding as the identity
    /// source.
    fn decode_request(
        &self,
        frame: &[u8],
        route: &AuthenticatedSessionRouteBinding,
    ) -> Result<AuthenticatedProviderRequest, ProviderToolkitError>;

    /// Encode a response for the authenticated request correlation.
    fn encode_response(
        &self,
        request_id: &RequestId,
        payload: &CanonicalJsonObject,
    ) -> Result<Vec<u8>, ProviderToolkitError>;
}

/// Serve one authenticated Provider session until cancellation or disconnect.
pub async fn serve_authenticated_component_session<S, C, Cap>(
    adapter: &ProviderAgentAdapter<S>,
    session: &mut AuthenticatedComponentSession<Cap>,
    codec: &C,
    cancellation: Cancellation,
    now_tick: impl Fn() -> u64 + Send + Sync + Copy,
) -> Result<(), ProviderToolkitError>
where
    S: ProviderService,
    C: AuthenticatedProviderFrameCodec,
{
    loop {
        if cancellation.is_cancelled() {
            return Ok(());
        }
        let frame = session
            .receive_ttrpc()
            .await
            .map_err(|_| ProviderToolkitError::SessionClosed)?;
        let route = session.route_binding();
        let request = codec.decode_request(&frame, &route)?;
        let expected_zone = ZonePath::new(vec![
            ZoneLabelId::parse(route.zone().as_str())
                .map_err(|_| ProviderToolkitError::SessionUnauthenticated)?,
        ])
        .map_err(|_| ProviderToolkitError::SessionUnauthenticated)?;
        let expected_provider = route
            .provider_ref()
            .ok_or(ProviderToolkitError::SessionUnauthenticated)?;
        // The codec may decode target metadata for wire diagnostics, but the
        // dispatch target is still derived from the live authenticated route.
        // A frame that tries to retarget another Zone or Provider is refused
        // before authorization or service dispatch.
        if request.zone() != &expected_zone || request.provider_ref() != expected_provider {
            return Err(ProviderToolkitError::SessionUnauthenticated);
        }
        let AuthenticatedProviderRequest {
            request_id,
            authorization,
            zone,
            provider_ref,
            method,
            payload,
        } = request;
        if authorization.target_zone() != route.zone()
            || authorization.target() != Some(expected_provider)
            || authorization.operation() != method.as_str()
        {
            return Err(ProviderToolkitError::AuthorizationDenied);
        }
        let response = session
            .authorize(authorization, now_tick())
            .await
            .map_err(|_| ProviderToolkitError::AuthorizationDenied)?;
        let response_payload = adapter.dispatch(zone, provider_ref, method, payload)?;
        let encoded = codec.encode_response(&request_id, &response_payload)?;
        session
            .send_authorized_ttrpc(response, encoded, now_tick())
            .await
            .map_err(|_| ProviderToolkitError::SessionClosed)?;
    }
}

/// Run one authenticated Provider service after readiness admission.
#[expect(
    clippy::too_many_arguments,
    reason = "the authenticated runtime boundary keeps every authority input explicit"
)]
pub async fn run_authenticated_provider<S, C, Cap>(
    entrypoint: ProviderEntrypoint,
    registration: crate::ProviderAdmission,
    session_admission: ProviderSessionAdmission,
    session: &mut AuthenticatedComponentSession<Cap>,
    service: S,
    codec: &C,
    cancellation: Cancellation,
    now_tick: impl Fn() -> u64 + Send + Sync + Copy,
) -> Result<(), ProviderRuntimeError>
where
    S: ProviderService,
    C: AuthenticatedProviderFrameCodec,
{
    entrypoint
        .publish_authenticated_ready(&registration, session_admission, session)
        .map_err(|_| ProviderRuntimeError::NotAccepting)?;
    let adapter = ProviderAgentAdapter::new(service);
    serve_authenticated_component_session(&adapter, session, codec, cancellation, now_tick)
        .await
        .map_err(|_| ProviderRuntimeError::SessionLoopFailed)
}

/// Route-bound identity check used by generated Provider startup glue.
pub fn validate_provider_route(
    route: &AuthenticatedSessionRouteBinding,
    provider_ref: &ResourceRef,
    service: &str,
) -> Result<(), ProviderRuntimeError> {
    if route.provider_ref() != Some(provider_ref)
        || route.service().as_str() != service
        || route.reconnect_generation().get() == 0
    {
        return Err(ProviderRuntimeError::SessionUnauthenticated);
    }
    Ok(())
}
