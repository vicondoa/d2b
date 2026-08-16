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
        let frame = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(()),
            frame = session.receive_ttrpc() => {
                frame.map_err(|_| ProviderToolkitError::SessionClosed)?
            }
        };
        let route = session.route_binding();
        let request = codec.decode_request(&frame, &route)?;
        validate_authenticated_provider_request(
            &route,
            request.zone(),
            request.provider_ref(),
            &request.authorization,
            request.method(),
        )?;
        let AuthenticatedProviderRequest {
            request_id,
            authorization,
            zone,
            provider_ref,
            method,
            payload,
        } = request;
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

fn validate_authenticated_provider_request(
    route: &AuthenticatedSessionRouteBinding,
    request_zone: &ZonePath,
    request_provider: &ResourceRef,
    authorization: &SessionAuthorizationRequest,
    method: &BoundedToken,
) -> Result<(), ProviderToolkitError> {
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
    if request_zone != &expected_zone || request_provider != expected_provider {
        return Err(ProviderToolkitError::SessionUnauthenticated);
    }
    if authorization.service() != route.service()
        || authorization.target_zone() != route.zone()
        || authorization.target() != Some(expected_provider)
        // Session authorization carries the canonical Service/Member spelling;
        // the Provider adapter receives only the bounded member token.
        || authorization
            .operation()
            .rsplit_once('/')
            .map(|(_, member)| member)
            != Some(method.as_str())
    {
        return Err(ProviderToolkitError::AuthorizationDenied);
    }
    Ok(())
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
        || route.provider_generation().is_none()
        || route.reconnect_generation().get() == 0
    {
        return Err(ProviderRuntimeError::SessionUnauthenticated);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::ServiceName;
    use d2b_session::SessionVerb;

    fn route() -> (AuthenticatedSessionRouteBinding, ResourceRef) {
        let provider_ref = ResourceRef::parse("Provider/display").expect("valid Provider ref");
        let route = AuthenticatedSessionRouteBinding::for_test(
            Some(provider_ref.clone()),
            "d2b.provider.v3",
            1,
            Some(2),
            Some(3),
        );
        (route, provider_ref)
    }

    fn request_parts(
        route: &AuthenticatedSessionRouteBinding,
        provider_ref: &ResourceRef,
    ) -> (
        ZonePath,
        ResourceRef,
        SessionAuthorizationRequest,
        BoundedToken,
    ) {
        let zone = ZonePath::new(vec![ZoneLabelId::parse("dev").expect("valid Zone label")])
            .expect("valid Zone path");
        let method = BoundedToken::parse("render").expect("valid method");
        let authorization = SessionAuthorizationRequest::new(
            SessionVerb::Invoke,
            ServiceName::parse(route.service().as_str()).expect("valid service"),
            format!("Provider/{}", method.as_str()),
            route.zone().clone(),
            Some(provider_ref.clone()),
        )
        .expect("valid authorization request");
        (zone, provider_ref.clone(), authorization, method)
    }

    #[test]
    fn authenticated_request_accepts_route_bound_identity_and_authorization() {
        let (route, provider_ref) = route();
        let (zone, provider_ref, authorization, method) = request_parts(&route, &provider_ref);

        assert_eq!(
            validate_authenticated_provider_request(
                &route,
                &zone,
                &provider_ref,
                &authorization,
                &method,
            ),
            Ok(())
        );
    }

    #[test]
    fn authenticated_request_rejects_retargeted_zone_or_provider() {
        let (route, provider_ref) = route();
        let (_, provider_ref, authorization, method) = request_parts(&route, &provider_ref);
        let wrong_zone =
            ZonePath::new(vec![ZoneLabelId::parse("other").expect("valid Zone label")])
                .expect("valid Zone path");

        assert_eq!(
            validate_authenticated_provider_request(
                &route,
                &wrong_zone,
                &provider_ref,
                &authorization,
                &method,
            ),
            Err(ProviderToolkitError::SessionUnauthenticated)
        );

        let forged_provider = ResourceRef::parse("Provider/other").expect("valid Provider ref");
        let (zone, _, authorization, method) = request_parts(&route, &provider_ref);
        assert_eq!(
            validate_authenticated_provider_request(
                &route,
                &zone,
                &forged_provider,
                &authorization,
                &method,
            ),
            Err(ProviderToolkitError::SessionUnauthenticated)
        );
    }

    #[test]
    fn authenticated_request_rejects_mismatched_authorization_before_dispatch() {
        let (route, provider_ref) = route();
        let (zone, provider_ref, _, method) = request_parts(&route, &provider_ref);
        let authorization = SessionAuthorizationRequest::new(
            SessionVerb::Invoke,
            ServiceName::parse("d2b.other.v3").expect("valid service"),
            format!("Provider/{}", method.as_str()),
            route.zone().clone(),
            Some(provider_ref.clone()),
        )
        .expect("valid authorization request");

        assert_eq!(
            validate_authenticated_provider_request(
                &route,
                &zone,
                &provider_ref,
                &authorization,
                &method,
            ),
            Err(ProviderToolkitError::AuthorizationDenied)
        );

        let (zone, provider_ref, _, method) = request_parts(&route, &provider_ref);
        let authorization = SessionAuthorizationRequest::new(
            SessionVerb::Invoke,
            route.service().clone(),
            "Provider/other",
            route.zone().clone(),
            Some(provider_ref.clone()),
        )
        .expect("valid authorization request");
        assert_eq!(
            validate_authenticated_provider_request(
                &route,
                &zone,
                &provider_ref,
                &authorization,
                &method,
            ),
            Err(ProviderToolkitError::AuthorizationDenied)
        );
    }

    #[test]
    fn authenticated_request_rejects_missing_provider_route() {
        let route =
            AuthenticatedSessionRouteBinding::for_test(None, "d2b.provider.v3", 1, None, None);
        let provider_ref = ResourceRef::parse("Provider/display").expect("valid Provider ref");
        let (zone, provider_ref, authorization, method) = request_parts(
            &AuthenticatedSessionRouteBinding::for_test(
                Some(provider_ref.clone()),
                "d2b.provider.v3",
                1,
                Some(2),
                Some(3),
            ),
            &provider_ref,
        );

        assert_eq!(
            validate_authenticated_provider_request(
                &route,
                &zone,
                &provider_ref,
                &authorization,
                &method,
            ),
            Err(ProviderToolkitError::SessionUnauthenticated)
        );
    }

    #[test]
    fn provider_route_validation_rejects_identity_service_and_generation_mismatches() {
        let (route, provider_ref) = route();
        assert_eq!(
            validate_provider_route(&route, &provider_ref, "d2b.provider.v3"),
            Ok(())
        );

        let other_provider = ResourceRef::parse("Provider/other").expect("valid Provider ref");
        assert_eq!(
            validate_provider_route(&route, &other_provider, "d2b.provider.v3"),
            Err(ProviderRuntimeError::SessionUnauthenticated)
        );
        assert_eq!(
            validate_provider_route(&route, &provider_ref, "d2b.other.v3"),
            Err(ProviderRuntimeError::SessionUnauthenticated)
        );

        let no_generation = AuthenticatedSessionRouteBinding::for_test(
            Some(provider_ref.clone()),
            "d2b.provider.v3",
            1,
            None,
            Some(3),
        );
        assert_eq!(
            validate_provider_route(&no_generation, &provider_ref, "d2b.provider.v3"),
            Err(ProviderRuntimeError::SessionUnauthenticated)
        );
    }
}
