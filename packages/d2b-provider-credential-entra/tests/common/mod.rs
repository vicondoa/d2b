#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use d2b_contracts_provider::v3::credential::{
    AdmittedCredentialDelivery, AudienceToken, CredentialAuthorization, CredentialDeliveryEvidence,
    CredentialLeaseHandle, CredentialLeaseState, CredentialMethod, CredentialProvider,
    CredentialRequest, CredentialResponse, CredentialServiceError, CredentialServiceErrorCode,
    CredentialSessionBinding, CredentialSourceVersion, DeliveryIdentity, DeliveryRouteDigest,
    DeliverySessionParams, OperationClass, PlacementBinding, dispatch_authorized_provider,
};
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    ServiceName, SessionBinding, SessionPurpose, TranscriptHash, TransportBinding,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingRefusal, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    SchemaFingerprint,
};
use d2b_provider_credential_entra::{
    EntraClientError, EntraClientState, EntraConfig, EntraCredentialClient,
    EntraCredentialProvider, EntraCredentialProviderFactory, EntraFuture, EntraLeaseGrant,
    EntraLeaseInspection, EntraLeaseRef, EntraLeaseRenewal, EntraLeaseRequest,
    EntraLeaseRevocation, EntraPlacement,
};

pub const EXPIRY: u64 = 20_000;

pub struct FakeEntraClient {
    pub state: tokio::sync::Mutex<EntraClientState>,
    pub inspection: tokio::sync::Mutex<Option<EntraLeaseInspection>>,
    pub issue_calls: AtomicUsize,
    pub inspect_calls: AtomicUsize,
    pub refresh_calls: AtomicUsize,
    pub revoke_calls: AtomicUsize,
    pub refresh_generation: tokio::sync::Mutex<u64>,
    pub issue_generation: tokio::sync::Mutex<Option<u64>>,
    pub issue_expiry: tokio::sync::Mutex<Option<u64>>,
    pub issue_revoke_error: tokio::sync::Mutex<Option<EntraClientError>>,
    pub issue_error: tokio::sync::Mutex<Option<EntraClientError>>,
    pub refresh_error: tokio::sync::Mutex<Option<EntraClientError>>,
    pub revoke_error: tokio::sync::Mutex<Option<EntraClientError>>,
    pub observed_request: tokio::sync::Mutex<Option<(String, String, String)>>,
    pub token_canary: String,
    pub endpoint_canary: String,
    pub cookie_canary: String,
}

impl FakeEntraClient {
    pub fn new() -> Self {
        let nonce = format!("{:x}", std::process::id());
        Self {
            state: tokio::sync::Mutex::new(EntraClientState::Ready),
            inspection: tokio::sync::Mutex::new(None),
            issue_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            refresh_generation: tokio::sync::Mutex::new(2),
            issue_generation: tokio::sync::Mutex::new(None),
            issue_expiry: tokio::sync::Mutex::new(None),
            issue_revoke_error: tokio::sync::Mutex::new(None),
            issue_error: tokio::sync::Mutex::new(None),
            refresh_error: tokio::sync::Mutex::new(None),
            revoke_error: tokio::sync::Mutex::new(None),
            observed_request: tokio::sync::Mutex::new(None),
            token_canary: format!("entra-token-canary-{nonce}"),
            endpoint_canary: format!("entra-endpoint-canary-{nonce}"),
            cookie_canary: format!("entra-cookie-canary-{nonce}"),
        }
    }
}

impl EntraCredentialClient for FakeEntraClient {
    fn state(&self) -> EntraFuture<'_, EntraClientState> {
        let state = &self.state;
        Box::pin(async move { Ok(*state.lock().await) })
    }

    fn issue_lease(&self, request: &EntraLeaseRequest) -> EntraFuture<'_, EntraLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        let inspection = &self.inspection;
        let revoke_error_slot = &self.revoke_error;
        let token = self.token_canary.clone();
        let endpoint = self.endpoint_canary.clone();
        let credential = request.credential_ref().to_canonical_string();
        let operation_id = request.operation_id().to_owned();
        let idempotency_key = request.idempotency_key().to_owned();
        let requested_expiry = request.requested_expiry_unix_ms();
        Box::pin(async move {
            let error = *self.issue_error.lock().await;
            let state = *self.state.lock().await;
            let expiry = (*self.issue_expiry.lock().await).unwrap_or(requested_expiry);
            let generation = (*self.issue_generation.lock().await).unwrap_or(1);
            let issue_revoke_error = *self.issue_revoke_error.lock().await;
            *self.observed_request.lock().await = Some((
                credential,
                operation_id,
                idempotency_key,
            ));
            if state == EntraClientState::InteractionRequired {
                return Err(EntraClientError::InteractionRequired);
            }
            if let Some(error) = error {
                return Err(error);
            }
            if let Some(error) = issue_revoke_error {
                *revoke_error_slot.lock().await = Some(error);
            }
            let grant = EntraLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse(&token).unwrap(),
                source_version: CredentialSourceVersion::parse(&endpoint).unwrap(),
                rotation_generation: generation,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().await = Some(EntraLeaseInspection {
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
        let inspection = &self.inspection;
        Box::pin(async move { Ok((*inspection.lock().await).clone().unwrap()) })
    }

    fn refresh_lease(&self, lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRenewal> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let expiry = lease.metadata().expires_at_unix_ms;
        let inspection = &self.inspection;
        Box::pin(async move {
            let error = *self.refresh_error.lock().await;
            let generation = *self.refresh_generation.lock().await;
            if let Some(error) = error {
                return Err(error);
            }
            let grant = EntraLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("entra-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("entra-source-2").unwrap(),
                rotation_generation: generation,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().await = Some(EntraLeaseInspection {
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
        Box::pin(async move {
            let error = *self.revoke_error.lock().await;
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

/// The consumer's authenticated subject at one exact Provider generation.
pub fn subject_context_for_generation(provider_generation: u64) -> AuthenticatedSubjectContext {
    subject_context_for(
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceRef::parse("Zone/work").unwrap(),
        Locality::Local,
    )
    .with_provider_generation(ResourceGeneration::new(provider_generation).unwrap())
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

/// One admitted `CredentialBinding` relationship, as the shared delivery port
/// sees it.
///
/// The double holds what the source side commits - the audience, the granted
/// operation classes, the generations the fence is against, and the current
/// delivery session - and answers only from those. It never reads the session
/// being admitted, which is what makes a widened audience or an ungranted
/// operation fail here rather than inside the Provider.
pub struct AdmittedRelationship {
    audience: AudienceToken,
    granted: Vec<OperationClass>,
    consumer_component_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    current: DeliveryIdentity,
}

impl AdmittedRelationship {
    /// Admit the relationship the harness's own delivery session matches.
    pub fn new(
        request: &CredentialRequest,
        granted: &[OperationClass],
        audience: &str,
        sequence: u64,
    ) -> Self {
        Self {
            audience: AudienceToken::parse(audience).unwrap(),
            granted: granted.to_vec(),
            consumer_component_generation: ResourceGeneration::new(1).unwrap(),
            provider_generation: ResourceGeneration::new(1).unwrap(),
            current: DeliveryIdentity::new(
                request.credential_ref().clone(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                AudienceToken::parse(audience).unwrap(),
                OperationClass::AcquireToken,
                sequence,
            ),
        }
    }

    /// The relationship's current delivery session names `class`.
    pub fn delivering(mut self, class: OperationClass) -> Self {
        let current = self.current.clone();
        self.current = DeliveryIdentity::new(
            current.credential_ref().clone(),
            current.credential_uid().clone(),
            current.credential_generation(),
            current.consumer_provider_ref().clone(),
            current.consumer_component_generation(),
            current.audience().clone(),
            class,
            current.sequence(),
        );
        self
    }

    /// The relationship's fence is against a different Provider session.
    pub fn for_provider_generation(mut self, generation: u64) -> Self {
        self.provider_generation = ResourceGeneration::new(generation).unwrap();
        self
    }
}

impl AdmittedCredentialDelivery for AdmittedRelationship {
    fn audience(&self) -> &AudienceToken {
        &self.audience
    }

    fn grants(&self, class: OperationClass) -> bool {
        self.granted.contains(&class)
    }

    fn current_delivery(&self) -> DeliveryIdentity {
        self.current.clone()
    }

    fn fence_refusal(&self, evidence: &CredentialDeliveryEvidence) -> Option<BindingRefusal> {
        if evidence.consumer_component_generation() != self.consumer_component_generation
            || evidence.provider_generation() != self.provider_generation
        {
            return Some(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            ));
        }
        None
    }
}

/// A delivery session with an exact audience, sequence, and consumer
/// component generation.
pub fn delivery_session(
    request: &CredentialRequest,
    class: OperationClass,
    sequence: u64,
    audience: &str,
    component_generation: u64,
) -> DeliverySessionParams {
    DeliverySessionParams::new(
        request.credential_ref().clone(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceGeneration::new(component_generation).unwrap(),
        AudienceToken::parse(audience).unwrap(),
        class,
        request.requested_expiry_unix_ms(),
        request.deadline_unix_ms(),
        DeliveryRouteDigest::parse(format!("sha256:{}", "b".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
}

/// Authorize one method with an exact delivery session and Provider session.
#[derive(Clone)]
pub struct SessionAdmission {
    pub delivery: DeliverySessionParams,
    pub provider_generation: u64,
}

impl TestAdmission for SessionAdmission {
    fn authorize(
        &self,
        _method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        let context = subject_context_for_generation(self.provider_generation);
        let session = CredentialSessionBinding::new(context.clone(), EXPIRY).unwrap();
        CredentialAuthorization::new_for_subject(
            _method,
            Some(self.delivery.clone()),
            context,
        )
        .and_then(|authorization| authorization.with_authenticated_session(session))
    }
}
