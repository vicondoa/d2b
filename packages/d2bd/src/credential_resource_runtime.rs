//! Provider-side Credential session admission and scoped delivery.
//!
//! The Credential resource driver lives in [`crate::credential_driver`].
//! This module keeps the pieces that are not the reconciler: the typed
//! Provider session handoff registry the ProviderSupervisor populates, the
//! exact non-secret revocation request whose delete path binds one session
//! generation (R28), and the same-Zone ResourceService gate the relay
//! delivery path forwards through.
//!
//! The old runner-backed `CredentialResourceReconciler` lived here; the U12
//! conversion deleted it and its behavior now lives in `CredentialDriver`.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_provider::v3::{
    credential::{
        CREDENTIAL_SERVICE_NAME, CredentialMethod, CredentialOutcomeCode, CredentialRequest,
        MetadataResponse, decode_outer, encode_outer,
    },
    credential_controller::{CredentialIdempotencyKey, CredentialProviderKind},
};
use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, canonical_digest,
    identity::ReconnectGeneration,
};
use d2b_provider_transport_azure_relay::{
    RelayCredentialError, RelayCredentialLease, ScopedCredentialClient, ScopedCredentialRequest,
};
use d2b_session::{ComponentSessionDriver, SessionTtrpcClient};

const CREDENTIAL_RESOURCE_TYPE: &str = "Credential";

/// Stable failures from the Credential resource adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialResourceRuntimeError {
    /// A resource or identity failed closed validation.
    InvalidResource,
    /// A typed Credential session refused or could not confirm revocation.
    Revocation,
}

impl core::fmt::Display for CredentialResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "credential-resource-invalid",
            Self::Revocation => "credential-revocation-unconfirmed",
        })
    }
}

impl std::error::Error for CredentialResourceRuntimeError {}

/// Exact non-secret inputs for one provider-side RevokeToken call.
///
/// The driver fills these from the durable row, the Provider facts, and the
/// live session; [`CredentialRevocationRequest::new`] owns the closed
/// validation, so an incomplete identity can never reach a Provider.
pub(crate) struct CredentialRevocationInputs {
    pub(crate) zone: ZoneId,
    pub(crate) credential_ref: ResourceRef,
    pub(crate) credential_uid: ResourceUid,
    pub(crate) credential_generation: ResourceGeneration,
    pub(crate) user_ref: Option<ResourceRef>,
    pub(crate) provider_ref: ResourceRef,
    pub(crate) provider_generation: ResourceGeneration,
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) session_generation: ReconnectGeneration,
    /// `credential.rotationGeneration`; a missing or zero value keeps the
    /// old status read's default of 1.
    pub(crate) rotation_generation: u64,
}

/// Exact non-secret input for one provider-side RevokeToken call.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CredentialRevocationRequest {
    zone: ZoneId,
    credential_ref: ResourceRef,
    credential_uid: ResourceUid,
    credential_generation: ResourceGeneration,
    user_ref: Option<ResourceRef>,
    provider_ref: ResourceRef,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    session_generation: ReconnectGeneration,
    operation_id: String,
    idempotency_key: String,
    deadline_ms: u64,
}

impl core::fmt::Debug for CredentialRevocationRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialRevocationRequest")
            .field("credential_ref", &"<redacted>")
            .field("credential_uid", &"<redacted>")
            .field("credential_generation", &self.credential_generation)
            .field("user_ref", &"<redacted>")
            .field("provider_ref", &"<redacted>")
            .field("provider_generation", &self.provider_generation)
            .field("controller_generation", &self.controller_generation)
            .field("session_generation", &self.session_generation)
            .field("operation_id", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("deadline_ms", &self.deadline_ms)
            .finish()
    }
}

impl CredentialRevocationRequest {
    /// Validate and bind one revocation identity. The request is only
    /// constructible for a credential Provider with a live (non-zero)
    /// session generation; everything else fails closed as
    /// [`CredentialResourceRuntimeError::InvalidResource`] (R28).
    pub(crate) fn new(
        inputs: CredentialRevocationInputs,
    ) -> Result<Self, CredentialResourceRuntimeError> {
        if inputs.session_generation.get() == 0
            || inputs.rotation_generation == 0
            || credential_provider_kind(&inputs.provider_ref).is_none()
        {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        let operation_id = credential_revoke_operation_id(&inputs);
        let idempotency_key = CredentialIdempotencyKey::derive(
            &inputs.credential_uid,
            inputs.rotation_generation,
            CredentialMethod::RevokeToken,
        )
        .map_err(|_| CredentialResourceRuntimeError::Revocation)?
        .request_value();
        Ok(Self {
            zone: inputs.zone,
            credential_ref: inputs.credential_ref,
            credential_uid: inputs.credential_uid,
            credential_generation: inputs.credential_generation,
            user_ref: inputs.user_ref,
            provider_ref: inputs.provider_ref,
            provider_generation: inputs.provider_generation,
            controller_generation: inputs.controller_generation,
            session_generation: inputs.session_generation,
            operation_id,
            idempotency_key,
            deadline_ms: 10_000,
        })
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub(crate) fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }
}

/// The durable revocation identity the old reconciler derived: the same
/// zone/uid/generation/Provider/generation tuple always yields the same
/// operation id, so a rejoined daemon re-issues the identical request.
fn credential_revoke_operation_id(inputs: &CredentialRevocationInputs) -> String {
    let preimage = format!(
        "{}:{}:{}:{}:{}:{}",
        inputs.zone.as_str(),
        inputs.credential_uid.as_str(),
        inputs.credential_generation.get(),
        inputs.provider_ref.to_canonical_string(),
        inputs.provider_generation.get(),
        inputs.controller_generation.get(),
    );
    format!(
        "credential-revoke-{}",
        canonical_digest("d2b:credential-revoke/v1", preimage.as_bytes())
    )
}

/// Provider-side revocation result. Only confirmed outcomes may unblock
/// Process cleanup and finalizer release.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialRevocationOutcome {
    Revoked,
    AlreadyRevoked,
    Uncertain,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CredentialRevocationEvidence {
    operation_id: String,
    outcome: CredentialRevocationOutcome,
    session_generation: ReconnectGeneration,
}

impl CredentialRevocationEvidence {
    pub(crate) fn confirmed(
        request: &CredentialRevocationRequest,
        outcome: CredentialRevocationOutcome,
    ) -> Self {
        Self {
            operation_id: request.operation_id.clone(),
            outcome,
            session_generation: request.session_generation,
        }
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub(crate) fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    pub(crate) fn outcome_code(&self) -> &'static str {
        match self.outcome {
            CredentialRevocationOutcome::Revoked => "revoked",
            CredentialRevocationOutcome::AlreadyRevoked => "already-revoked",
            CredentialRevocationOutcome::Uncertain => "uncertain",
        }
    }
}

impl core::fmt::Debug for CredentialRevocationEvidence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialRevocationEvidence")
            .field("operation_id", &"<redacted>")
            .field("outcome", &self.outcome)
            .field("session_generation", &self.session_generation)
            .finish()
    }
}

/// Typed Credential session used by the resource controller's cleanup effect.
#[async_trait]
pub(crate) trait CredentialSession: Send + Sync {
    fn session_generation(&self) -> Option<ReconnectGeneration>;

    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError>;
}

/// One authenticated Provider ComponentSession used for typed Credential calls.
///
/// The session driver is supplied by the daemon's ProviderSupervisor handoff.
/// This adapter owns only the generated ttrpc client and a single-flight gate;
/// Resource status remains the durable evidence owner.
pub(crate) struct ComponentCredentialSession {
    route: d2b_session::AuthenticatedSessionRouteBinding,
    driver: Arc<dyn ComponentSessionDriver>,
    client: Arc<SessionTtrpcClient>,
    gate: tokio::sync::Mutex<()>,
}

impl ComponentCredentialSession {
    pub(crate) fn new(
        route: d2b_session::AuthenticatedSessionRouteBinding,
        driver: Arc<dyn ComponentSessionDriver>,
    ) -> Result<Self, CredentialResourceRuntimeError> {
        if route.provider_ref().is_none()
            || route.reconnect_generation().get() == 0
            || driver.generation() != route.reconnect_generation().get()
            || !route.liveness().is_live()
            || route.service().as_str() != CREDENTIAL_SERVICE_NAME
        {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        let client_driver = Arc::clone(&driver);
        Ok(Self {
            route,
            driver: client_driver,
            client: Arc::new(SessionTtrpcClient::new(driver)),
            gate: tokio::sync::Mutex::new(()),
        })
    }
}

#[async_trait]
impl CredentialSession for ComponentCredentialSession {
    fn session_generation(&self) -> Option<ReconnectGeneration> {
        self.route
            .liveness()
            .is_live()
            .then_some(self.route.reconnect_generation())
    }

    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
        let Some(provider_ref) = self.route.provider_ref() else {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        };
        if request.credential_ref.resource_type().as_str() != CREDENTIAL_RESOURCE_TYPE
            || &request.zone != self.route.zone()
            || &request.provider_ref != provider_ref
            || self.route.provider_generation() != Some(request.provider_generation)
            || self.route.controller_generation() != Some(request.controller_generation)
        {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        if request.session_generation != self.route.reconnect_generation() {
            return Ok(CredentialRevocationOutcome::Uncertain);
        }
        let _gate = self.gate.lock().await;
        if !self.route.liveness().is_live()
            || self.driver.generation() != self.route.reconnect_generation().get()
        {
            return Ok(CredentialRevocationOutcome::Uncertain);
        }
        let now_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| CredentialResourceRuntimeError::Revocation)?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let expiry = now_unix_ms.saturating_add(request.deadline_ms);
        if expiry <= now_unix_ms {
            return Ok(CredentialRevocationOutcome::Uncertain);
        }
        let typed = CredentialRequest::new(
            request.credential_ref.clone(),
            request.operation_id.clone(),
            request.idempotency_key.clone(),
            expiry,
            expiry,
        )
        .map_err(|_| CredentialResourceRuntimeError::Revocation)?;
        let mut rpc = ttrpc::proto::Request::new();
        rpc.set_service("d2b.credential.v3.CredentialService".to_owned());
        rpc.set_method("RevokeToken".to_owned());
        rpc.timeout_nano =
            i64::try_from(request.deadline_ms.saturating_mul(1_000_000)).unwrap_or(i64::MAX);
        rpc.metadata = vec![
            ttrpc::proto::KeyValue {
                key: "d2b.credential.zone".to_owned(),
                value: request.zone.as_str().to_owned(),
                ..Default::default()
            },
            ttrpc::proto::KeyValue {
                key: "d2b.credential.provider".to_owned(),
                value: request.provider_ref.to_canonical_string(),
                ..Default::default()
            },
            ttrpc::proto::KeyValue {
                key: "d2b.credential.uid".to_owned(),
                value: request.credential_uid.as_str().to_owned(),
                ..Default::default()
            },
            ttrpc::proto::KeyValue {
                key: "d2b.credential.generation".to_owned(),
                value: request.credential_generation.get().to_string(),
                ..Default::default()
            },
            ttrpc::proto::KeyValue {
                key: "d2b.credential.provider-generation".to_owned(),
                value: request.provider_generation.get().to_string(),
                ..Default::default()
            },
            ttrpc::proto::KeyValue {
                key: "d2b.credential.controller-generation".to_owned(),
                value: request.controller_generation.get().to_string(),
                ..Default::default()
            },
            ttrpc::proto::KeyValue {
                key: "d2b.credential.session-generation".to_owned(),
                value: request.session_generation.get().to_string(),
                ..Default::default()
            },
        ];
        if let Some(user_ref) = request.user_ref.as_ref() {
            rpc.metadata.push(ttrpc::proto::KeyValue {
                key: "d2b.credential.user-ref".to_owned(),
                value: user_ref.to_canonical_string(),
                ..Default::default()
            });
        }
        rpc.payload =
            encode_outer(&typed).map_err(|_| CredentialResourceRuntimeError::Revocation)?;
        let response = match self.client.client().request(rpc).await {
            Ok(response) => response,
            Err(_) => return Ok(CredentialRevocationOutcome::Uncertain),
        };
        let metadata: MetadataResponse = decode_outer(&response.payload)
            .map_err(|_| CredentialResourceRuntimeError::Revocation)?;
        if metadata.metadata.state
            != d2b_contracts_provider::v3::credential::CredentialLeaseState::Revoked
        {
            return Ok(CredentialRevocationOutcome::Uncertain);
        }
        match metadata.metadata.outcome {
            CredentialOutcomeCode::Revoked => Ok(CredentialRevocationOutcome::Revoked),
            CredentialOutcomeCode::AlreadyRevoked => {
                Ok(CredentialRevocationOutcome::AlreadyRevoked)
            }
            CredentialOutcomeCode::Success => Ok(CredentialRevocationOutcome::Uncertain),
        }
    }
}

/// Registry populated by authenticated ProviderSupervisor session handoffs.
#[derive(Clone, Default)]
pub(crate) struct CredentialSessionRegistry {
    sessions: Arc<
        std::sync::Mutex<
            std::collections::BTreeMap<
                ResourceRef,
                (ReconnectGeneration, Arc<dyn CredentialSession>),
            >,
        >,
    >,
}

impl CredentialSessionRegistry {
    pub(crate) fn register(
        &self,
        provider_ref: ResourceRef,
        session_generation: ReconnectGeneration,
        session: Arc<dyn CredentialSession>,
    ) -> Result<(), CredentialResourceRuntimeError> {
        if provider_ref.resource_type().as_str() != "Provider" || session_generation.get() == 0 {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| CredentialResourceRuntimeError::Revocation)?;
        if sessions
            .get(&provider_ref)
            .is_some_and(|(generation, _)| *generation > session_generation)
        {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        sessions.insert(provider_ref, (session_generation, session));
        Ok(())
    }

    pub(crate) fn remove(
        &self,
        provider_ref: &ResourceRef,
        session_generation: ReconnectGeneration,
    ) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if sessions
                .get(provider_ref)
                .is_some_and(|(generation, _)| *generation == session_generation)
            {
                sessions.remove(provider_ref);
            }
        }
    }

    pub(crate) fn for_provider(&self, provider_ref: ResourceRef) -> Arc<dyn CredentialSession> {
        Arc::new(RegistryCredentialSession {
            provider_ref,
            sessions: Arc::clone(&self.sessions),
        })
    }
}

struct RegistryCredentialSession {
    provider_ref: ResourceRef,
    sessions: Arc<
        std::sync::Mutex<
            std::collections::BTreeMap<
                ResourceRef,
                (ReconnectGeneration, Arc<dyn CredentialSession>),
            >,
        >,
    >,
}

#[async_trait]
impl CredentialSession for RegistryCredentialSession {
    fn session_generation(&self) -> Option<ReconnectGeneration> {
        self.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(&self.provider_ref)
                .map(|(generation, _)| *generation)
        })
    }

    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
        if request.provider_ref != self.provider_ref {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        let current = self
            .sessions
            .lock()
            .map_err(|_| CredentialResourceRuntimeError::Revocation)?
            .get(&self.provider_ref)
            .map(|(generation, session)| (*generation, Arc::clone(session)));
        match current {
            Some((generation, session)) if generation == request.session_generation => {
                session.revoke_credential(request).await
            }
            Some(_) | None => Ok(CredentialRevocationOutcome::Uncertain),
        }
    }
}

pub(crate) fn credential_provider_kind(
    provider_ref: &ResourceRef,
) -> Option<CredentialProviderKind> {
    match provider_ref.to_canonical_string().as_str() {
        "Provider/credential-secret-service" => Some(CredentialProviderKind::SecretService),
        "Provider/credential-entra" => Some(CredentialProviderKind::Entra),
        "Provider/credential-managed-identity" => Some(CredentialProviderKind::ManagedIdentity),
        _ => None,
    }
}

pub(crate) fn is_credential_provider_ref(provider_ref: &ResourceRef) -> bool {
    credential_provider_kind(provider_ref).is_some()
}

/// A same-Zone ResourceService gate around a typed Credential ComponentSession.
///
/// The delegate owns the sensitive delivery channel. This adapter only verifies
/// the current Credential row and exact Guest scope before forwarding the
/// already-authorized request.
pub(crate) struct SameZoneScopedCredentialClient {
    zone: ZoneId,
    route: d2b_session::AuthenticatedSessionRouteBinding,
    execution_ref: ResourceRef,
    resource: Arc<dyn CredentialResourceReader>,
    delegate: Arc<dyn ScopedCredentialClient>,
}

#[async_trait]
trait CredentialResourceReader: Send + Sync {
    async fn get(&self, request: wire::GetRequest) -> wire::GetResponse;
}

struct ComponentSessionCredentialResourceReader {
    client: d2b_resource_api::generated::d2b_resource_v3_ttrpc::ResourceServiceClient,
}

#[async_trait]
impl CredentialResourceReader for ComponentSessionCredentialResourceReader {
    async fn get(&self, request: wire::GetRequest) -> wire::GetResponse {
        self.client
            .get(ttrpc::context::Context::default(), &request)
            .await
            .unwrap_or_else(|_| {
                let mut response = wire::GetResponse::new();
                response.error = protobuf::MessageField::some(wire::ResourceError {
                    kind: protobuf::EnumOrUnknown::new(
                        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_INTERNAL_INTEGRITY_FAILURE,
                    ),
                    ..wire::ResourceError::new()
                });
                response
            })
    }
}

impl core::fmt::Debug for SameZoneScopedCredentialClient {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SameZoneScopedCredentialClient(<redacted>)")
    }
}

impl SameZoneScopedCredentialClient {
    pub(crate) fn with_component_session(
        zone: ZoneId,
        session: &d2bd_runtime::guest_component_session::GuestComponentSessionClient,
        delegate: Arc<dyn ScopedCredentialClient>,
    ) -> Result<Self, RelayCredentialError> {
        let route = session.route_binding();
        if session.identity().zone() != &zone
            || session.generation() == 0
            || !route.liveness().is_live()
            || session.identity().validate_route(&route).is_err()
            || route.context().execution_ref() != Some(session.identity().guest_ref())
        {
            return Err(RelayCredentialError::InvalidScope);
        }
        Ok(Self::with_resource_reader(
            zone,
            route,
            session.identity().guest_ref().clone(),
            Arc::new(ComponentSessionCredentialResourceReader {
                client: session.resource_service_client(),
            }),
            delegate,
        ))
    }

    fn with_resource_reader(
        zone: ZoneId,
        route: d2b_session::AuthenticatedSessionRouteBinding,
        execution_ref: ResourceRef,
        resource: Arc<dyn CredentialResourceReader>,
        delegate: Arc<dyn ScopedCredentialClient>,
    ) -> Self {
        Self {
            zone,
            route,
            execution_ref,
            resource,
            delegate,
        }
    }

    fn validate_request_scope(
        request: &ScopedCredentialRequest,
        expected_zone: &ZoneId,
    ) -> Result<(), RelayCredentialError> {
        if request.zone() != expected_zone
            || request.execution_ref().resource_type().as_str() != "Guest"
            || request.credential_ref().resource_type().as_str() != CREDENTIAL_RESOURCE_TYPE
            || request.binding().zone() != Some(expected_zone)
        {
            return Err(RelayCredentialError::InvalidScope);
        }
        Ok(())
    }
}

#[async_trait]
impl ScopedCredentialClient for SameZoneScopedCredentialClient {
    async fn read_credential(
        &self,
        request: &ScopedCredentialRequest,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Self::validate_request_scope(request, &self.zone)?;
        if !self.route.liveness().is_live()
            || request.execution_ref() != &self.execution_ref
            || request.binding().reconnect_generation() != self.route.reconnect_generation().get()
        {
            return Err(RelayCredentialError::InvalidScope);
        }
        let response = self
            .resource
            .get({
                let mut get = wire::GetRequest::new();
                get.meta = protobuf::MessageField::some(
                    d2bd_runtime::resource_runtime_support::public_request_meta(
                        "credential-scoped-read",
                    ),
                );
                get.target = protobuf::MessageField::some(scoped_identity(request));
                let mut projection = wire::Projection::new();
                projection.kind =
                    protobuf::EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
                get.projection = protobuf::MessageField::some(projection);
                get
            })
            .await;
        let Some(resource) = response.resource.as_ref() else {
            return Err(RelayCredentialError::Unavailable);
        };
        let value = serde_json::from_slice::<serde_json::Value>(&resource.canonical_json)
            .map_err(|_| RelayCredentialError::Unavailable)?;
        if response.error.is_some()
            || value
                .pointer("/metadata/zone")
                .and_then(serde_json::Value::as_str)
                != Some(request.zone().as_str())
            || value
                .pointer("/spec/scope/executionRef")
                .and_then(serde_json::Value::as_str)
                != Some(request.execution_ref().to_canonical_string().as_str())
            || !value
                .pointer("/spec/allowedOperations")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|operations| {
                    operations
                        .iter()
                        .any(|operation| operation.as_str() == Some("acquire-token"))
                })
            || value
                .pointer("/status/phase")
                .and_then(serde_json::Value::as_str)
                != Some("Ready")
            || value
                .pointer("/metadata/deletionRequestedAt")
                .is_some_and(|deletion| !deletion.is_null())
        {
            return Err(RelayCredentialError::InvalidScope);
        }
        let lease = self.delegate.read_credential(request).await?;
        if lease.binding() != Some(request.binding()) || lease.role() != request.role() {
            return Err(RelayCredentialError::BindingMismatch);
        }
        Ok(lease)
    }

    async fn revoke_credential(
        &self,
        lease: RelayCredentialLease,
    ) -> Result<(), RelayCredentialError> {
        self.delegate.revoke_credential(lease).await
    }
}

fn scoped_identity(request: &ScopedCredentialRequest) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = request.zone().as_str().to_owned();
    identity.resource_type = request.credential_ref().resource_type().as_str().to_owned();
    identity.name = request.credential_ref().name().as_str().to_owned();
    identity
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_provider::v3::credential::CredentialWire;
    use d2b_provider_transport_azure_relay::{RelayCredentialBinding, RelayCredentialRole};
    use ttrpc::proto::Codec;

    const MI_PROVIDER: &str = "Provider/credential-managed-identity";

    fn revocation_inputs(session_generation: u64) -> CredentialRevocationInputs {
        CredentialRevocationInputs {
            zone: ZoneId::parse("dev").unwrap(),
            credential_ref: ResourceRef::parse("Credential/relay").unwrap(),
            credential_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            credential_generation: ResourceGeneration::new(1).unwrap(),
            user_ref: None,
            provider_ref: ResourceRef::parse(MI_PROVIDER).unwrap(),
            provider_generation: ResourceGeneration::new(1).unwrap(),
            controller_generation: ControllerGeneration::new(1).unwrap(),
            session_generation: ReconnectGeneration::new(session_generation).unwrap(),
            rotation_generation: 1,
        }
    }

    fn revocation_request(session_generation: u64) -> CredentialRevocationRequest {
        CredentialRevocationRequest::new(revocation_inputs(session_generation))
            .expect("revocation request")
    }

    fn component_session(
        provider_ref: &ResourceRef,
        generation: u64,
        driver: Arc<FakeCredentialDriver>,
    ) -> Arc<ComponentCredentialSession> {
        let route = d2b_session::AuthenticatedSessionRouteBinding::for_test(
            Some(provider_ref.clone()),
            CREDENTIAL_SERVICE_NAME,
            generation,
            Some(1),
            Some(1),
        );
        Arc::new(ComponentCredentialSession::new(route, driver).unwrap())
    }

    #[tokio::test]
    async fn revoke_session_deduplicates_the_fenced_operation_identity() {
        let session = RecordingCredentialSession::default();
        let request = revocation_request(7);
        assert_eq!(
            session.revoke_credential(&request).await.unwrap(),
            CredentialRevocationOutcome::Revoked
        );
        assert_eq!(
            session.revoke_credential(&request).await.unwrap(),
            CredentialRevocationOutcome::AlreadyRevoked
        );
        assert_eq!(session.operations.lock().unwrap().len(), 1);
        let debug = format!("{request:?}");
        assert!(!debug.contains("Credential/relay"));
        assert!(!debug.contains("123e4567-e89b-42d3-a456-426614174000"));
        let rejoined = revocation_request(8);
        assert_eq!(request.operation_id, rejoined.operation_id);
        assert_eq!(request.idempotency_key, rejoined.idempotency_key);
        assert_ne!(request.session_generation, rejoined.session_generation);
    }

    #[tokio::test]
    async fn provider_route_generation_mismatch_is_uncertain() {
        let provider_ref = ResourceRef::parse(MI_PROVIDER).unwrap();
        let driver = Arc::new(FakeCredentialDriver::new(9));
        let route = d2b_session::AuthenticatedSessionRouteBinding::for_test(
            Some(provider_ref),
            CREDENTIAL_SERVICE_NAME,
            9,
            Some(1),
            Some(1),
        );
        let session = ComponentCredentialSession::new(route, driver).unwrap();
        assert_eq!(
            session
                .revoke_credential(&revocation_request(8))
                .await
                .unwrap(),
            CredentialRevocationOutcome::Uncertain
        );
    }

    #[tokio::test]
    async fn component_session_revocation_rebinds_on_rejoin_with_the_same_durable_identity() {
        let provider_ref = ResourceRef::parse(MI_PROVIDER).unwrap();
        let registry = CredentialSessionRegistry::default();
        let driver = Arc::new(FakeCredentialDriver::new(9));
        let session = component_session(&provider_ref, 9, Arc::clone(&driver));
        registry
            .register(
                provider_ref.clone(),
                ReconnectGeneration::new(9).unwrap(),
                session,
            )
            .unwrap();
        let request = revocation_request(9);
        let scoped = registry.for_provider(provider_ref.clone());
        assert_eq!(
            scoped.session_generation(),
            Some(ReconnectGeneration::new(9).unwrap())
        );
        assert_eq!(
            scoped.revoke_credential(&request).await.unwrap(),
            CredentialRevocationOutcome::Revoked
        );
        assert_eq!(driver.requests.lock().unwrap().len(), 1);

        // A rejoin registers the new generation; the retired generation can
        // no longer revoke (R28), and the same durable request re-issued
        // through the new session keeps the identical operation id.
        let rejoined_driver = Arc::new(FakeCredentialDriver::new(10));
        let rejoined_session = component_session(&provider_ref, 10, Arc::clone(&rejoined_driver));
        registry
            .register(
                provider_ref.clone(),
                ReconnectGeneration::new(10).unwrap(),
                rejoined_session,
            )
            .unwrap();
        assert_eq!(
            registry
                .for_provider(provider_ref.clone())
                .revoke_credential(&revocation_request(9))
                .await
                .unwrap(),
            CredentialRevocationOutcome::Uncertain
        );
        assert_eq!(
            registry
                .for_provider(provider_ref.clone())
                .revoke_credential(&revocation_request(10))
                .await
                .unwrap(),
            CredentialRevocationOutcome::Revoked
        );
        let rejoined = rejoined_driver.requests.lock().unwrap();
        assert_eq!(rejoined.len(), 1);
        assert_eq!(rejoined[0].0, request.operation_id);
        assert_eq!(rejoined[0].2, 10);
    }

    #[test]
    fn revocation_request_rejects_zero_rotation_or_foreign_providers() {
        let mut inputs = revocation_inputs(7);
        inputs.rotation_generation = 0;
        assert_eq!(
            CredentialRevocationRequest::new(inputs).unwrap_err(),
            CredentialResourceRuntimeError::InvalidResource
        );

        let mut inputs = revocation_inputs(7);
        inputs.provider_ref = ResourceRef::parse("Provider/volume-virtiofs").unwrap();
        assert_eq!(
            CredentialRevocationRequest::new(inputs).unwrap_err(),
            CredentialResourceRuntimeError::InvalidResource
        );
    }

    #[test]
    fn confirmed_revocation_evidence_is_redacted_and_bounded() {
        let request = revocation_request(7);
        let evidence =
            CredentialRevocationEvidence::confirmed(&request, CredentialRevocationOutcome::Revoked);
        assert_eq!(evidence.operation_id(), request.operation_id.as_str());
        assert_eq!(evidence.session_generation(), request.session_generation);
        assert_eq!(evidence.outcome_code(), "revoked");
        let debug = format!("{evidence:?}");
        assert!(debug.contains("session_generation"));
        assert!(!debug.contains("credential-revoke-"));
        assert!(!debug.contains("123e4567-e89b-42d3-a456-426614174000"));
    }

    #[derive(Clone, Default)]
    struct RecordingCredentialSession {
        operations:
            Arc<std::sync::Mutex<std::collections::BTreeMap<String, CredentialRevocationOutcome>>>,
        attempts: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl CredentialSession for RecordingCredentialSession {
        fn session_generation(&self) -> Option<ReconnectGeneration> {
            Some(ReconnectGeneration::new(7).expect("recording session generation"))
        }

        async fn revoke_credential(
            &self,
            request: &CredentialRevocationRequest,
        ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
            self.attempts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut operations = self.operations.lock().unwrap();
            if operations
                .insert(
                    request.operation_id.clone(),
                    CredentialRevocationOutcome::Revoked,
                )
                .is_some()
            {
                Ok(CredentialRevocationOutcome::AlreadyRevoked)
            } else {
                Ok(CredentialRevocationOutcome::Revoked)
            }
        }
    }

    struct UncertainCredentialSession;

    #[async_trait]
    impl CredentialSession for UncertainCredentialSession {
        fn session_generation(&self) -> Option<ReconnectGeneration> {
            Some(ReconnectGeneration::new(7).expect("uncertain session generation"))
        }

        async fn revoke_credential(
            &self,
            _request: &CredentialRevocationRequest,
        ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
            Ok(CredentialRevocationOutcome::Uncertain)
        }
    }

    #[tokio::test]
    async fn uncertain_revocation_never_unblocks_cleanup() {
        // A live session that cannot confirm revocation must keep the
        // credential row alive (R28).
        let session = UncertainCredentialSession;
        assert_eq!(
            session
                .revoke_credential(&revocation_request(7))
                .await
                .unwrap(),
            CredentialRevocationOutcome::Uncertain
        );
    }

    #[test]
    fn scoped_resource_client_rejects_wrong_zone_or_execution_before_session() {
        let request = ScopedCredentialRequest::new(
            ZoneId::parse("other").unwrap(),
            ResourceRef::parse("Credential/relay").unwrap(),
            ResourceRef::parse("Guest/gateway").unwrap(),
            RelayCredentialRole::Send,
            RelayCredentialBinding::new_scoped(
                ZoneId::parse("other").unwrap(),
                "link",
                "session",
                1,
            )
            .unwrap(),
            1_000,
        )
        .unwrap();
        assert!(
            SameZoneScopedCredentialClient::validate_request_scope(
                &request,
                &ZoneId::parse("dev").unwrap()
            )
            .is_err()
        );
    }

    struct NeverCredentialResourceReader;

    #[async_trait]
    impl CredentialResourceReader for NeverCredentialResourceReader {
        async fn get(&self, _request: wire::GetRequest) -> wire::GetResponse {
            panic!("invalid scoped request reached ResourceService")
        }
    }

    struct NeverScopedCredentialDelegate;

    #[async_trait]
    impl ScopedCredentialClient for NeverScopedCredentialDelegate {
        async fn read_credential(
            &self,
            _request: &ScopedCredentialRequest,
        ) -> Result<RelayCredentialLease, RelayCredentialError> {
            Err(RelayCredentialError::Unavailable)
        }

        async fn revoke_credential(
            &self,
            _lease: RelayCredentialLease,
        ) -> Result<(), RelayCredentialError> {
            Err(RelayCredentialError::Unavailable)
        }
    }

    #[tokio::test]
    async fn scoped_resource_client_rejects_wrong_guest_or_reconnect_before_resource_read() {
        let zone = ZoneId::parse("dev").unwrap();
        let route = d2b_session::AuthenticatedSessionRouteBinding::for_test(
            Some(ResourceRef::parse("Provider/relay").unwrap()),
            "d2b.resource.v3",
            7,
            Some(1),
            Some(1),
        );
        let client = SameZoneScopedCredentialClient::with_resource_reader(
            zone.clone(),
            route,
            ResourceRef::parse("Guest/gateway").unwrap(),
            Arc::new(NeverCredentialResourceReader),
            Arc::new(NeverScopedCredentialDelegate),
        );
        let wrong_guest = ScopedCredentialRequest::new(
            zone.clone(),
            ResourceRef::parse("Credential/relay").unwrap(),
            ResourceRef::parse("Guest/other").unwrap(),
            RelayCredentialRole::Send,
            RelayCredentialBinding::new_scoped(zone.clone(), "link", "session", 7).unwrap(),
            1_000,
        )
        .unwrap();
        assert_eq!(
            client.read_credential(&wrong_guest).await.unwrap_err(),
            RelayCredentialError::InvalidScope
        );
        let stale_session = ScopedCredentialRequest::new(
            zone.clone(),
            ResourceRef::parse("Credential/relay").unwrap(),
            ResourceRef::parse("Guest/gateway").unwrap(),
            RelayCredentialRole::Send,
            RelayCredentialBinding::new_scoped(zone, "link", "session", 6).unwrap(),
            1_000,
        )
        .unwrap();
        assert_eq!(
            client.read_credential(&stale_session).await.unwrap_err(),
            RelayCredentialError::InvalidScope
        );
    }

    struct FakeCredentialDriver {
        generation: u64,
        responses: std::sync::Arc<(
            tokio::sync::Mutex<std::collections::VecDeque<Vec<u8>>>,
            tokio::sync::Notify,
        )>,
        requests: std::sync::Arc<std::sync::Mutex<Vec<(String, String, u64)>>>,
    }

    impl FakeCredentialDriver {
        fn new(generation: u64) -> Self {
            Self {
                generation,
                responses: std::sync::Arc::new((
                    tokio::sync::Mutex::new(std::collections::VecDeque::new()),
                    tokio::sync::Notify::new(),
                )),
                requests: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl d2b_session::ComponentSessionDriver for FakeCredentialDriver {
        fn generation(&self) -> u64 {
            self.generation
        }

        async fn start_ttrpc(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
            frame: Vec<u8>,
        ) -> d2b_session::Result<()> {
            let header = ttrpc::proto::MessageHeader::from(&frame);
            let request = ttrpc::Request::decode(&frame[ttrpc::proto::MESSAGE_HEADER_LENGTH..])
                .map_err(|_| d2b_session::SessionError::new(
                    d2b_contracts_zone_session::v3::component_session::SessionErrorCode::RecordMalformed,
                ))?;
            let typed = CredentialRequest::decode_wire(&request.payload).map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_contracts_zone_session::v3::component_session::SessionErrorCode::RecordMalformed,
                )
            })?;
            let session_generation = request
                .metadata
                .iter()
                .find(|value| value.key == "d2b.credential.session-generation")
                .and_then(|value| value.value.parse::<u64>().ok())
                .expect("provider route session generation");
            self.requests.lock().unwrap().push((
                typed.operation_id().to_owned(),
                typed.idempotency_key().to_owned(),
                session_generation,
            ));
            let metadata = MetadataResponse {
                metadata: d2b_contracts_provider::v3::credential::CredentialMetadata {
                    lease_handle:
                        d2b_contracts_provider::v3::credential::CredentialLeaseHandle::parse(
                            "provider-lease",
                        )
                        .unwrap(),
                    rotation_generation: 1,
                    source_version:
                        d2b_contracts_provider::v3::credential::CredentialSourceVersion::parse(
                            "provider-source",
                        )
                        .unwrap(),
                    expires_at_unix_ms: u64::MAX,
                    state: d2b_contracts_provider::v3::credential::CredentialLeaseState::Revoked,
                    outcome: CredentialOutcomeCode::Revoked,
                },
            };
            let response_payload = encode_outer(&metadata).map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_contracts_zone_session::v3::component_session::SessionErrorCode::RecordMalformed,
                )
            })?;
            let mut response = ttrpc::Response::new();
            response.set_status(ttrpc::get_status(ttrpc::Code::OK, ""));
            response.payload = response_payload;
            let encoded = response.encode().map_err(|_| {
                d2b_session::SessionError::new(
                    d2b_contracts_zone_session::v3::component_session::SessionErrorCode::RecordMalformed,
                )
            })?;
            let mut response_frame = Vec::from(ttrpc::proto::MessageHeader::new_response(
                header.stream_id,
                encoded.len() as u32,
            ));
            response_frame.extend(encoded);
            let (queue, notify) = &*self.responses;
            queue.lock().await.push_back(response_frame);
            notify.notify_one();
            Ok(())
        }

        async fn complete_ttrpc(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn cancel(
            &self,
            _generation: u64,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            loop {
                let (queue, notify) = &*self.responses;
                if let Some(frame) = queue.lock().await.pop_front() {
                    return Ok(frame);
                }
                notify.notified().await;
            }
        }

        async fn register_inbound_call(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<d2b_session::Cancellation> {
            Err(d2b_session::SessionError::new(
                d2b_contracts_zone_session::v3::component_session::SessionErrorCode::Cancelled,
            ))
        }

        async fn mark_inbound_dispatched(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn complete_inbound_call(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn remove_inbound_call(
            &self,
            _request_id: d2b_contracts_zone_session::v3::component_session::RequestId,
        ) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn send_attachments(
            &self,
            _attachments: Vec<d2b_session::OwnedAttachment>,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_attachments(
            &self,
        ) -> d2b_session::Result<Vec<d2b_session::OwnedAttachment>> {
            Ok(Vec::new())
        }

        async fn open_named_stream(
            &self,
            _stream: d2b_session::StreamId,
            _send_credit: u32,
            _receive_credit: u32,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_named_stream(
            &self,
            _stream: d2b_session::StreamId,
            _bytes: Vec<u8>,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<d2b_session::StreamEvent> {
            Err(d2b_session::SessionError::new(
                d2b_contracts_zone_session::v3::component_session::SessionErrorCode::Cancelled,
            ))
        }

        async fn grant_named_stream_credit(
            &self,
            _stream: d2b_session::StreamId,
            _bytes: u32,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn close_named_stream(
            &self,
            _stream: d2b_session::StreamId,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn reset_named_stream(
            &self,
            _stream: d2b_session::StreamId,
        ) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn drive_keepalive(&self, _now: std::time::Instant) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> d2b_session::Result<d2b_session::SessionEvent> {
            Err(d2b_session::SessionError::new(
                d2b_contracts_zone_session::v3::component_session::SessionErrorCode::Cancelled,
            ))
        }

        async fn close(
            &self,
            _reason: d2b_contracts_zone_session::v3::component_session::CloseReason,
            _remediation: d2b_contracts_zone_session::v3::component_session::Remediation,
        ) -> d2b_session::Result<()> {
            Ok(())
        }
    }
}
