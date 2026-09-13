//! The standard authenticated service loop.
//!
//! One provider process serves one authenticated ComponentSession route. This
//! module owns that loop - the bounded dispatch adapter behind it, the
//! generated frame codec in front of it, the readiness handshake that
//! publishes the marker only once the typed service is live, and the drain
//! that waits for the last in-flight call.
//!
//! Session authentication, generation binding, attachment policy, and stream
//! fairness stay in `d2b-session`: everything here consumes the redacted
//! route binding rather than re-deriving authority from it.

mod adapter;
mod credential;
mod service;
mod session;

pub use adapter::{
    DispatchLimiter, DispatchPermit, MAX_DISPATCH_IN_FLIGHT, ProviderAgentAdapter,
    ProviderFrameCodec, ProviderRequest, ProviderService, validate_attachment_indexes,
};
pub use credential::{
    CredentialAuthorizationSource, CredentialRequestMetadata, RouteCredentialAuthorization,
    credential_service, run_authenticated_credential_provider,
};
pub use service::{
    GeneratedProviderServiceServer, GeneratedServiceDescriptor, MAX_SERVER_IN_FLIGHT, ServerError,
    ServerRequestPermit,
};
pub use session::{
    AuthenticatedProviderFrameCodec, AuthenticatedProviderRequest, run_authenticated_provider,
    serve_authenticated_component_session, validate_provider_route,
};

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_zone_session::v3::component_session::{CloseReason, Remediation};
use d2b_session::{AuthenticatedSessionRouteBinding, ComponentSessionDriver, StreamId};

use crate::base::{
    ProviderAdmission, ProviderEntrypoint, ProviderRuntimeError, ProviderSessionAdmission,
};

/// Named stream carrying the post-admission Provider readiness receipt.
pub const PROVIDER_READY_STREAM_ID: u16 = 0x0103;
/// Initial bounded credit for the Provider readiness stream.
pub const PROVIDER_READY_STREAM_CREDIT: u32 = 256;
/// Protected readiness receipt sent only after the typed service is live.
pub const PROVIDER_READY_MARKER: &[u8] = b"d2b-provider-ready-v1";

/// The bounded drain budget the loop waits for after the session closes.
pub const SERVICE_LOOP_DRAIN_BUDGET: Duration = Duration::from_secs(5);

/// Serve one admitted authenticated route until the session loop ends.
///
/// The service map is the provider's generated surface. Readiness is
/// published only after the loop is live and the marker is written, so a
/// controller that observes readiness knows the typed service is answering.
pub async fn serve_authenticated_route(
    entrypoint: ProviderEntrypoint,
    registration: ProviderAdmission,
    session_admission: ProviderSessionAdmission,
    driver: Arc<dyn ComponentSessionDriver>,
    route: AuthenticatedSessionRouteBinding,
    services: crate::base::ServiceMethods,
) -> Result<(), ProviderRuntimeError> {
    let serving = tokio::spawn(d2b_session::serve_ttrpc_services(
        Arc::clone(&driver),
        services,
    ));
    tokio::task::yield_now().await;
    if serving.is_finished() {
        let _ = serving.await;
        return Err(ProviderRuntimeError::SessionLoopFailed);
    }
    if entrypoint
        .publish_authenticated_ready(&registration, session_admission, &route)
        .is_err()
    {
        serving.abort();
        let _ = serving.await;
        return Err(ProviderRuntimeError::SessionLoopFailed);
    }
    let ready_result = async {
        let stream = StreamId::new(PROVIDER_READY_STREAM_ID)
            .map_err(|_| ProviderRuntimeError::SessionLoopFailed)?;
        driver
            .open_named_stream(
                stream,
                PROVIDER_READY_STREAM_CREDIT,
                PROVIDER_READY_STREAM_CREDIT,
            )
            .await
            .map_err(|_| ProviderRuntimeError::SessionLoopFailed)?;
        driver
            .send_named_stream(stream, PROVIDER_READY_MARKER.to_vec())
            .await
            .map_err(|_| ProviderRuntimeError::SessionLoopFailed)?;
        driver
            .close_named_stream(stream)
            .await
            .map_err(|_| ProviderRuntimeError::SessionLoopFailed)
    }
    .await;
    if let Err(error) = ready_result {
        serving.abort();
        let _ = serving.await;
        return Err(error);
    }
    let result = serving
        .await
        .map_err(|_| ProviderRuntimeError::SessionLoopFailed)?
        .map_err(|_| ProviderRuntimeError::SessionLoopFailed);
    let _ = driver.close(CloseReason::Normal, Remediation::None).await;
    drop(registration);
    if !entrypoint.drain(SERVICE_LOOP_DRAIN_BUDGET) {
        return Err(ProviderRuntimeError::SessionLoopFailed);
    }
    result
}
