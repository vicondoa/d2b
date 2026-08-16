#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use d2b_contracts::v3::credential::{
    AudienceToken, CredentialAuthorization, CredentialLeaseHandle, CredentialLeaseState,
    CredentialMethod, CredentialProvider, CredentialRequest, CredentialResponse,
    CredentialServiceError, CredentialServiceErrorCode, CredentialSessionBinding,
    CredentialSourceVersion, DeliveryRouteDigest, DeliverySessionParams, PlacementBinding,
    dispatch_authorized_provider,
};
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding,
    SessionPurpose, TranscriptHash, TransportBinding,
};
use d2b_provider_credential_managed_identity::{
    ManagedIdentityClientConfig, ManagedIdentityClientError, ManagedIdentityClientState,
    ManagedIdentityCredentialClient, ManagedIdentityCredentialProvider,
    ManagedIdentityCredentialProviderFactory, ManagedIdentityFuture, ManagedIdentityLeaseGrant,
    ManagedIdentityLeaseInspection, ManagedIdentityLeaseRef, ManagedIdentityLeaseRenewal,
    ManagedIdentityLeaseRequest, ManagedIdentityLeaseRevocation, ManagedIdentityPlacement,
};

pub const EXPIRY: u64 = 20_000;

pub fn session_expiry() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
        + 20_000
}

pub struct FakeClient {
    pub state: Mutex<ManagedIdentityClientState>,
    pub inspection: Mutex<Option<ManagedIdentityLeaseInspection>>,
    pub issue_calls: AtomicUsize,
    pub inspect_calls: AtomicUsize,
    pub refresh_calls: AtomicUsize,
    pub revoke_calls: AtomicUsize,
    pub issue_error: Mutex<Option<ManagedIdentityClientError>>,
    pub observed_request: Mutex<Option<(String, String, String)>>,
    pub token_canary: String,
    pub endpoint_canary: String,
    pub response_canary: String,
}

impl FakeClient {
    pub fn new() -> Self {
        let nonce = format!("{:x}", std::process::id());
        Self {
            state: Mutex::new(ManagedIdentityClientState::Ready),
            inspection: Mutex::new(None),
            issue_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            issue_error: Mutex::new(None),
            observed_request: Mutex::new(None),
            token_canary: format!("managed-identity-token-canary-{nonce}"),
            endpoint_canary: format!("managed-identity-endpoint-canary-{nonce}"),
            response_canary: format!("managed-identity-response-canary-{nonce}"),
        }
    }
}

impl ManagedIdentityCredentialClient for FakeClient {
    fn state(&self) -> ManagedIdentityFuture<'_, ManagedIdentityClientState> {
        let state = *self.state.lock().unwrap();
        Box::pin(async move { Ok(state) })
    }

    fn issue_lease(
        &self,
        request: &ManagedIdentityLeaseRequest,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        let error = *self.issue_error.lock().unwrap();
        let state = *self.state.lock().unwrap();
        let expiry = request.requested_expiry_unix_ms();
        let rotation_generation = request.rotation_generation();
        let token = self.token_canary.clone();
        let endpoint = self.endpoint_canary.clone();
        *self.observed_request.lock().unwrap() = Some((
            request.credential_ref().to_canonical_string(),
            request.operation_id().to_owned(),
            request.idempotency_key().to_owned(),
        ));
        let inspection = &self.inspection;
        Box::pin(async move {
            if state == ManagedIdentityClientState::Unavailable {
                return Err(ManagedIdentityClientError::Unavailable);
            }
            if let Some(error) = error {
                return Err(error);
            }
            let grant = ManagedIdentityLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse(&token).unwrap(),
                source_version: CredentialSourceVersion::parse(&endpoint).unwrap(),
                rotation_generation,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().unwrap() = Some(ManagedIdentityLeaseInspection {
                state: CredentialLeaseState::Active,
                source_version: grant.source_version.clone(),
                rotation_generation: grant.rotation_generation,
                expires_at_unix_ms: grant.expires_at_unix_ms,
            });
            Ok(grant)
        })
    }

    fn inspect_lease(
        &self,
        _lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseInspection> {
        self.inspect_calls.fetch_add(1, Ordering::SeqCst);
        let inspection = self.inspection.lock().unwrap().clone().unwrap();
        Box::pin(async move { Ok(inspection) })
    }

    fn refresh_lease(
        &self,
        lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseRenewal> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let expiry = lease.metadata().expires_at_unix_ms;
        let rotation_generation = lease.metadata().rotation_generation;
        let inspection = &self.inspection;
        Box::pin(async move {
            let grant = ManagedIdentityLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("managed-identity-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("managed-identity-source-2")
                    .unwrap(),
                rotation_generation: rotation_generation + 1,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().unwrap() = Some(ManagedIdentityLeaseInspection {
                state: CredentialLeaseState::Active,
                source_version: grant.source_version.clone(),
                rotation_generation: grant.rotation_generation,
                expires_at_unix_ms: grant.expires_at_unix_ms,
            });
            Ok(grant)
        })
    }

    fn revoke_lease(
        &self,
        _lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseRevocation> {
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(ManagedIdentityLeaseRevocation::Revoked) })
    }
}

pub fn setup() -> (ManagedIdentityCredentialProvider, Arc<FakeClient>) {
    let client = Arc::new(FakeClient::new());
    let config = ManagedIdentityClientConfig::new("client-1234", "azure-imds-aca", 64).unwrap();
    let placement = ManagedIdentityPlacement::new(
        PlacementBinding::GuestAgent,
        ResourceRef::parse("Guest/aca-sandbox").unwrap(),
        ResourceRef::parse("Zone/dev").unwrap(),
    )
    .unwrap();
    let factory = ManagedIdentityCredentialProviderFactory::new(
        config,
        placement,
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        client.clone(),
    )
    .unwrap();
    (factory.construct(), client)
}

pub fn request(idempotency: &str) -> CredentialRequest {
    CredentialRequest::new(
        ResourceRef::parse("Credential/aca-relay-mi").unwrap(),
        "operation-1",
        idempotency,
        EXPIRY,
        15_000,
    )
    .unwrap()
}

pub fn delivery(method: CredentialMethod, sequence: u64) -> DeliverySessionParams {
    delivery_for(
        method,
        sequence,
        ResourceRef::parse("Credential/aca-relay-mi").unwrap(),
    )
}

pub fn delivery_for(
    method: CredentialMethod,
    sequence: u64,
    credential_ref: ResourceRef,
) -> DeliverySessionParams {
    delivery_for_timing(method, sequence, credential_ref, EXPIRY, 15_000)
}

pub fn delivery_for_timing(
    method: CredentialMethod,
    sequence: u64,
    credential_ref: ResourceRef,
    expiry_unix_ms: u64,
    deadline_unix_ms: u64,
) -> DeliverySessionParams {
    DeliverySessionParams::new(
        credential_ref,
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        AudienceToken::parse("azure-resource-manager").unwrap(),
        method.operation_class(),
        expiry_unix_ms,
        deadline_unix_ms,
        DeliveryRouteDigest::parse(format!("sha256:{}", "d".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
}

pub fn authenticated_session(
    subject_ref: &str,
    zone_ref: &str,
    execution_ref: &str,
    provider_ref: &str,
    provider_generation: u64,
    reconnect_generation: u64,
) -> CredentialSessionBinding {
    authenticated_session_with_expiry(
        subject_ref,
        zone_ref,
        execution_ref,
        provider_ref,
        provider_generation,
        reconnect_generation,
        session_expiry(),
    )
}

pub fn authenticated_session_with_expiry(
    subject_ref: &str,
    zone_ref: &str,
    execution_ref: &str,
    provider_ref: &str,
    provider_generation: u64,
    reconnect_generation: u64,
    expires_at_unix_ms: u64,
) -> CredentialSessionBinding {
    let subject = AuthenticatedSubjectContext::new(
        ResourceRef::parse(subject_ref).unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceRef::parse(zone_ref).unwrap(),
        EvidenceClass::EnrolledKk,
        SessionPurpose::parse("credential-delivery").unwrap(),
        ServiceName::parse("d2b.credential.v3").unwrap(),
        SessionBinding::new(
            SchemaFingerprint::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            TransportBinding::new(
                Locality::Local,
                BindingDigest::parse(format!("sha256:{}", "b".repeat(64))).unwrap(),
            ),
            ReconnectGeneration::new(reconnect_generation).unwrap(),
            TranscriptHash::from_bytes([7; 32]),
        ),
    )
    .with_execution_ref(ResourceRef::parse(execution_ref).unwrap())
    .with_provider_ref(ResourceRef::parse(provider_ref).unwrap())
    .with_process_ref(ResourceRef::parse("Process/mi-agent-aca-relay-mi").unwrap())
    .with_provider_generation(ResourceGeneration::new(provider_generation).unwrap());
    CredentialSessionBinding::new(subject, expires_at_unix_ms).unwrap()
}

pub fn authorization(
    method: CredentialMethod,
    session: CredentialSessionBinding,
) -> Result<CredentialAuthorization, CredentialServiceError> {
    authorization_for(
        method,
        session,
        ResourceRef::parse("Credential/aca-relay-mi").unwrap(),
    )
}

pub fn authorization_for(
    method: CredentialMethod,
    session: CredentialSessionBinding,
    credential_ref: ResourceRef,
) -> Result<CredentialAuthorization, CredentialServiceError> {
    CredentialAuthorization::new(
        method,
        method
            .requires_delivery()
            .then(|| delivery_for(method, 1, credential_ref)),
    )?
    .with_authenticated_session(session)
}

#[derive(Clone)]
pub struct Admission {
    pub authenticated_consumer: ResourceRef,
}

pub trait TestAdmission {
    fn authorize(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError>;
}

impl TestAdmission for Admission {
    fn authorize(
        &self,
        method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        if self.authenticated_consumer
            != ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap()
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::OperationDenied,
            ));
        }
        authorization_for(
            method,
            authenticated_session(
                "Provider/runtime-azure-container-apps",
                "Zone/dev",
                "Guest/aca-sandbox",
                "Provider/runtime-azure-container-apps",
                1,
                1,
            ),
            _request.credential_ref().clone(),
        )
    }
}

pub struct ProviderHarness<P, A> {
    provider: P,
    admission: A,
}

impl<P, A> ProviderHarness<P, A>
where
    P: CredentialProvider,
    A: TestAdmission,
{
    pub const fn new(provider: P, admission: A) -> Self {
        Self {
            provider,
            admission,
        }
    }

    pub fn call(
        &self,
        method: CredentialMethod,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let authorization = self.admission.authorize(method, &request)?;
        dispatch_authorized_provider(&self.provider, method, &request, &authorization)
    }
}

pub fn admitted() -> Admission {
    Admission {
        authenticated_consumer: ResourceRef::parse("Provider/runtime-azure-container-apps")
            .unwrap(),
    }
}
