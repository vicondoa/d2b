#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use d2b_contracts_provider::v3::credential::{
    AudienceToken, CredentialAuthorization, CredentialLeaseHandle, CredentialLeaseState,
    CredentialMethod, CredentialProvider, CredentialRequest, CredentialResponse,
    CredentialServiceError, CredentialServiceErrorCode, CredentialSessionBinding,
    CredentialSourceVersion, DeliveryRouteDigest, DeliverySessionParams, PlacementBinding,
    dispatch_authorized_provider,
};
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    ServiceName, SessionBinding, SessionPurpose, TranscriptHash, TransportBinding,
};
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint};
use d2b_provider_credential_entra::{
    EntraClientError, EntraClientState, EntraConfig, EntraCredentialClient,
    EntraCredentialProvider, EntraCredentialProviderFactory, EntraFuture, EntraLeaseGrant,
    EntraLeaseInspection, EntraLeaseRef, EntraLeaseRenewal, EntraLeaseRequest,
    EntraLeaseRevocation, EntraPlacement,
};

pub const EXPIRY: u64 = 20_000;

pub struct FakeEntraClient {
    pub state: Mutex<EntraClientState>,
    pub inspection: Mutex<Option<EntraLeaseInspection>>,
    pub issue_calls: AtomicUsize,
    pub inspect_calls: AtomicUsize,
    pub refresh_calls: AtomicUsize,
    pub revoke_calls: AtomicUsize,
    pub refresh_generation: Mutex<u64>,
    pub issue_generation: Mutex<Option<u64>>,
    pub issue_expiry: Mutex<Option<u64>>,
    pub issue_revoke_error: Mutex<Option<EntraClientError>>,
    pub issue_error: Mutex<Option<EntraClientError>>,
    pub refresh_error: Mutex<Option<EntraClientError>>,
    pub revoke_error: Mutex<Option<EntraClientError>>,
    pub observed_request: Mutex<Option<(String, String, String)>>,
    pub token_canary: String,
    pub endpoint_canary: String,
    pub cookie_canary: String,
}

impl FakeEntraClient {
    pub fn new() -> Self {
        let nonce = format!("{:x}", std::process::id());
        Self {
            state: Mutex::new(EntraClientState::Ready),
            inspection: Mutex::new(None),
            issue_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            refresh_generation: Mutex::new(2),
            issue_generation: Mutex::new(None),
            issue_expiry: Mutex::new(None),
            issue_revoke_error: Mutex::new(None),
            issue_error: Mutex::new(None),
            refresh_error: Mutex::new(None),
            revoke_error: Mutex::new(None),
            observed_request: Mutex::new(None),
            token_canary: format!("entra-token-canary-{nonce}"),
            endpoint_canary: format!("entra-endpoint-canary-{nonce}"),
            cookie_canary: format!("entra-cookie-canary-{nonce}"),
        }
    }
}

impl EntraCredentialClient for FakeEntraClient {
    fn state(&self) -> EntraFuture<'_, EntraClientState> {
        let state = *self.state.lock().unwrap();
        Box::pin(async move { Ok(state) })
    }

    fn issue_lease(&self, request: &EntraLeaseRequest) -> EntraFuture<'_, EntraLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        let error = *self.issue_error.lock().unwrap();
        let state = *self.state.lock().unwrap();
        let expiry = self
            .issue_expiry
            .lock()
            .unwrap()
            .unwrap_or(request.requested_expiry_unix_ms());
        let generation = self.issue_generation.lock().unwrap().unwrap_or(1);
        let issue_revoke_error = *self.issue_revoke_error.lock().unwrap();
        let token = self.token_canary.clone();
        let endpoint = self.endpoint_canary.clone();
        *self.observed_request.lock().unwrap() = Some((
            request.credential_ref().to_canonical_string(),
            request.operation_id().to_owned(),
            request.idempotency_key().to_owned(),
        ));
        let inspection = &self.inspection;
        let revoke_error_slot = &self.revoke_error;
        Box::pin(async move {
            if state == EntraClientState::InteractionRequired {
                return Err(EntraClientError::InteractionRequired);
            }
            if let Some(error) = error {
                return Err(error);
            }
            if let Some(error) = issue_revoke_error {
                *revoke_error_slot.lock().unwrap() = Some(error);
            }
            let grant = EntraLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse(&token).unwrap(),
                source_version: CredentialSourceVersion::parse(&endpoint).unwrap(),
                rotation_generation: generation,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().unwrap() = Some(EntraLeaseInspection {
                state: CredentialLeaseState::Active,
                source_version: grant.source_version.clone(),
                rotation_generation: grant.rotation_generation,
                expires_at_unix_ms: grant.expires_at_unix_ms,
            });
            Ok(grant)
        })
    }

    fn inspect_lease(&self, lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseInspection> {
        self.inspect_calls.fetch_add(1, Ordering::SeqCst);
        if lease.endpoint_generation() != 7 {
            return Box::pin(async { Err(EntraClientError::GenerationMismatch) });
        }
        let inspection = self.inspection.lock().unwrap().clone().unwrap();
        Box::pin(async move { Ok(inspection) })
    }

    fn refresh_lease(&self, lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRenewal> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let error = *self.refresh_error.lock().unwrap();
        let expiry = lease.metadata().expires_at_unix_ms;
        let generation = *self.refresh_generation.lock().unwrap();
        let inspection = &self.inspection;
        Box::pin(async move {
            if let Some(error) = error {
                return Err(error);
            }
            let grant = EntraLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("entra-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("entra-source-2").unwrap(),
                rotation_generation: generation,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().unwrap() = Some(EntraLeaseInspection {
                state: CredentialLeaseState::Active,
                source_version: grant.source_version.clone(),
                rotation_generation: grant.rotation_generation,
                expires_at_unix_ms: grant.expires_at_unix_ms,
            });
            Ok(grant)
        })
    }

    fn revoke_lease(&self, _lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRevocation> {
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        let error = *self.revoke_error.lock().unwrap();
        Box::pin(async move {
            if let Some(error) = error {
                return Err(error);
            }
            Ok(EntraLeaseRevocation::Revoked)
        })
    }
}

pub fn setup() -> (EntraCredentialProvider, Arc<FakeEntraClient>) {
    let client = Arc::new(FakeEntraClient::new());
    let config = EntraConfig::new("tenant-1234", 64).unwrap();
    let placement = EntraPlacement::new_in_zone(
        ResourceRef::parse("Zone/work").unwrap(),
        PlacementBinding::GuestAgent,
        ResourceRef::parse("Guest/consumer").unwrap(),
        ResourceRef::parse("Guest/identity").unwrap(),
        ResourceRef::parse("Endpoint/entra-login").unwrap(),
        7,
    )
    .unwrap();
    let factory = EntraCredentialProviderFactory::new(
        config,
        placement,
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        client.clone(),
    )
    .unwrap();
    (factory.construct(), client)
}

pub fn subject_context() -> AuthenticatedSubjectContext {
    subject_context_for(
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceRef::parse("Zone/work").unwrap(),
        Locality::Local,
    )
}

pub fn subject_context_for(
    subject_ref: ResourceRef,
    zone_ref: ResourceRef,
    locality: Locality,
) -> AuthenticatedSubjectContext {
    subject_context_with_bindings(
        subject_ref,
        zone_ref,
        locality,
        Some(ResourceRef::parse("Guest/consumer").unwrap()),
        Some(ResourceRef::parse("Provider/credential-entra").unwrap()),
    )
}

pub fn subject_context_with_bindings(
    subject_ref: ResourceRef,
    zone_ref: ResourceRef,
    locality: Locality,
    execution_ref: Option<ResourceRef>,
    provider_ref: Option<ResourceRef>,
) -> AuthenticatedSubjectContext {
    let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let context = AuthenticatedSubjectContext::new(
        subject_ref,
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        zone_ref,
        EvidenceClass::UnixPeer,
        SessionPurpose::parse("credential").unwrap(),
        ServiceName::parse("d2b.credential.v3").unwrap(),
        SessionBinding::new(
            SchemaFingerprint::parse(digest).unwrap(),
            TransportBinding::new(locality, BindingDigest::parse(digest).unwrap()),
            ReconnectGeneration::new(1).unwrap(),
            TranscriptHash::from_bytes([0x5a; 32]),
        ),
    );
    let mut context = context;
    if let Some(execution) = execution_ref {
        context = context.with_execution_ref(execution);
    }
    if let Some(provider) = provider_ref {
        context = context.with_provider_ref(provider);
    }
    context.with_provider_generation(ResourceGeneration::new(1).unwrap())
}

pub fn session_binding() -> CredentialSessionBinding {
    CredentialSessionBinding::new(subject_context(), EXPIRY).unwrap()
}

pub fn request(idempotency: &str) -> CredentialRequest {
    CredentialRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        "operation-1",
        idempotency,
        EXPIRY,
        15_000,
    )
    .unwrap()
}

pub fn delivery(method: CredentialMethod, sequence: u64) -> DeliverySessionParams {
    delivery_values(
        method,
        ResourceRef::parse("Credential/work-entra").unwrap(),
        EXPIRY,
        15_000,
        sequence,
        1,
    )
}

pub fn delivery_for_request(
    method: CredentialMethod,
    request: &CredentialRequest,
) -> DeliverySessionParams {
    delivery_values(
        method,
        request.credential_ref().clone(),
        request.requested_expiry_unix_ms(),
        request.deadline_unix_ms(),
        1,
        1,
    )
}

pub fn delivery_with_component_generation(
    method: CredentialMethod,
    sequence: u64,
    component_generation: u64,
) -> DeliverySessionParams {
    delivery_values(
        method,
        ResourceRef::parse("Credential/work-entra").unwrap(),
        EXPIRY,
        15_000,
        sequence,
        component_generation,
    )
}

fn delivery_values(
    method: CredentialMethod,
    credential_ref: ResourceRef,
    expiry_unix_ms: u64,
    deadline_unix_ms: u64,
    sequence: u64,
    component_generation: u64,
) -> DeliverySessionParams {
    DeliverySessionParams::new(
        credential_ref,
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceGeneration::new(component_generation).unwrap(),
        AudienceToken::parse("azure-resource-manager").unwrap(),
        method.operation_class(),
        expiry_unix_ms,
        deadline_unix_ms,
        DeliveryRouteDigest::parse(format!("sha256:{}", "b".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
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
        request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        if self.authenticated_consumer
            != ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap()
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::OperationDenied,
            ));
        }
        CredentialAuthorization::new_for_subject(
            method,
            method
                .requires_delivery()
                .then(|| delivery_for_request(method, request)),
            subject_context(),
        )
        .and_then(|authorization| authorization.with_authenticated_session(session_binding()))
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
