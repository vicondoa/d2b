#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use d2b_contracts_provider::v3::credential::{
    AdmittedCredentialDelivery, AudienceToken, CredentialAuthorization, CredentialDeliveryEvidence,
    CredentialLeaseHandle, CredentialLeaseState, CredentialMethod, CredentialProvider,
    CredentialRequest, CredentialResponse, CredentialServiceError, CredentialSessionBinding,
    CredentialSourceVersion, DeliveryIdentity, DeliveryRouteDigest, DeliverySessionParams,
    OperationClass, PlacementBinding, dispatch_authorized_provider,
};
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose, TranscriptHash,
    TransportBinding,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingRefusal, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    ZoneId,
};
use d2b_provider_credential_secret_service::{
    LockPolicy, Oo7SecretServicePort, SecretServiceConfig, SecretServiceCredentialProvider,
    SecretServiceCredentialProviderFactory, SecretServiceFuture, SecretServiceLeaseGrant,
    SecretServiceLeaseInspection, SecretServiceLeaseRef, SecretServiceLeaseRenewal,
    SecretServiceLeaseRequest, SecretServiceLeaseRevocation, SecretServicePlacement,
    SecretServicePortError, SecretServiceSessionCapability, SecretServiceState,
};

pub const EXPIRY: u64 = 20_000;

pub struct FakeOo7Port {
    pub state: Mutex<SecretServiceState>,
    pub inspection: Mutex<Option<SecretServiceLeaseInspection>>,
    pub issue_calls: AtomicUsize,
    pub inspect_calls: AtomicUsize,
    pub refresh_calls: AtomicUsize,
    pub revoke_calls: AtomicUsize,
    pub ambiguous_revoke_calls: AtomicUsize,
    pub ambiguous_refresh_revoke_calls: AtomicUsize,
    pub issue_error: Mutex<Option<SecretServicePortError>>,
    pub refresh_error: Mutex<Option<SecretServicePortError>>,
    pub issue_rotation_generation: Mutex<u64>,
    pub observed_request: Mutex<Option<(String, String, String)>>,
    pub credential_canary: String,
    pub object_path_canary: String,
}

impl FakeOo7Port {
    pub fn new() -> Self {
        let nonce = format!("{:x}", std::process::id());
        Self {
            state: Mutex::new(SecretServiceState::Unlocked),
            inspection: Mutex::new(None),
            issue_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            ambiguous_revoke_calls: AtomicUsize::new(0),
            ambiguous_refresh_revoke_calls: AtomicUsize::new(0),
            issue_error: Mutex::new(None),
            refresh_error: Mutex::new(None),
            issue_rotation_generation: Mutex::new(1),
            observed_request: Mutex::new(None),
            credential_canary: format!("secret-service-value-canary-{nonce}"),
            object_path_canary: format!("secret-service-object-path-canary-{nonce}"),
        }
    }
}

impl Oo7SecretServicePort for FakeOo7Port {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn state(&self) -> SecretServiceFuture<'_, SecretServiceState> {
        let state = *self.state.lock().unwrap();
        Box::pin(async move { Ok(state) })
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        let error = *self.issue_error.lock().unwrap();
        let expiry = request.requested_expiry_unix_ms();
        let rotation_generation = *self.issue_rotation_generation.lock().unwrap();
        let secret = self.credential_canary.clone();
        let object_path = self.object_path_canary.clone();
        *self.observed_request.lock().unwrap() = Some((
            request.credential_ref().to_canonical_string(),
            request.operation_id().to_owned(),
            request.idempotency_key().to_owned(),
        ));
        let inspection = &self.inspection;
        Box::pin(async move {
            if let Some(error) = error {
                return Err(error);
            }
            let grant = SecretServiceLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse(&secret).unwrap(),
                source_version: CredentialSourceVersion::parse(&object_path).unwrap(),
                rotation_generation,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
                state: CredentialLeaseState::Active,
                source_version: grant.source_version.clone(),
                rotation_generation: grant.rotation_generation,
                expires_at_unix_ms: grant.expires_at_unix_ms,
            });
            Ok(grant)
        })
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn inspect_lease(
        &self,
        _lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseInspection> {
        self.inspect_calls.fetch_add(1, Ordering::SeqCst);
        let inspection = self.inspection.lock().unwrap().clone().unwrap();
        Box::pin(async move { Ok(inspection) })
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn refresh_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRenewal> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        let error = *self.refresh_error.lock().unwrap();
        let expiry = lease.metadata().expires_at_unix_ms;
        let inspection = &self.inspection;
        Box::pin(async move {
            if let Some(error) = error {
                return Err(error);
            }
            let grant = SecretServiceLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("secret-service-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("secret-service-source-2").unwrap(),
                rotation_generation: 2,
                expires_at_unix_ms: expiry,
            };
            *inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
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
        _lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRevocation> {
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretServiceLeaseRevocation::Revoked) })
    }

    fn revoke_ambiguous_lease(
        &self,
        _request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRevocation> {
        self.ambiguous_revoke_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretServiceLeaseRevocation::Revoked) })
    }

    fn revoke_ambiguous_refresh(
        &self,
        _lease: &SecretServiceLeaseRef,
        _operation_id: &str,
        _idempotency_key: &str,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRevocation> {
        self.ambiguous_refresh_revoke_calls
            .fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretServiceLeaseRevocation::Revoked) })
    }
}

pub fn setup(max_leases: u32) -> (SecretServiceCredentialProvider, Arc<FakeOo7Port>) {
    let port = Arc::new(FakeOo7Port::new());
    let config =
        SecretServiceConfig::new("login collection", max_leases, LockPolicy::FailClosed).unwrap();
    let placement = SecretServicePlacement::new(
        ZoneId::parse("user-zone").unwrap(),
        PlacementBinding::UserAgent,
        ResourceRef::parse("Host/workstation").unwrap(),
        ResourceRef::parse("User/alice").unwrap(),
    )
    .unwrap();
    let factory = SecretServiceCredentialProviderFactory::new(
        config,
        placement,
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port.clone(),
    )
    .unwrap();
    (
        factory
            .construct()
            .expect("test provider authority must be constructible"),
        port,
    )
}

pub fn request(idempotency: &str) -> CredentialRequest {
    CredentialRequest::new(
        ResourceRef::parse("Credential/local-keyring").unwrap(),
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
        ResourceRef::parse("Credential/local-keyring").unwrap(),
    )
}

pub fn delivery_for(
    method: CredentialMethod,
    sequence: u64,
    credential_ref: ResourceRef,
) -> DeliverySessionParams {
    delivery_for_consumer(
        method,
        sequence,
        credential_ref,
        ResourceRef::parse("Provider/shell-terminal").unwrap(),
    )
}

pub fn delivery_for_consumer(
    method: CredentialMethod,
    sequence: u64,
    credential_ref: ResourceRef,
    consumer_ref: ResourceRef,
) -> DeliverySessionParams {
    DeliverySessionParams::new(
        credential_ref,
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        consumer_ref,
        ResourceGeneration::new(1).unwrap(),
        AudienceToken::parse("user-session").unwrap(),
        method.operation_class(),
        EXPIRY,
        15_000,
        DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
}

#[derive(Clone)]
pub struct Admission;

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
        let params = if method.requires_delivery() {
            Some(delivery_for_request(method, 1, request))
        } else {
            None
        };
        CredentialAuthorization::new(method, params)
    }
}

pub fn delivery_for_request(
    method: CredentialMethod,
    sequence: u64,
    request: &CredentialRequest,
) -> DeliverySessionParams {
    DeliverySessionParams::new(
        request.credential_ref().clone(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceRef::parse("Provider/shell-terminal").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        AudienceToken::parse("user-session").unwrap(),
        method.operation_class(),
        request.requested_expiry_unix_ms(),
        request.deadline_unix_ms(),
        DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
}

pub trait SessionCapabilitySource {
    fn test_session_capability(&self) -> SecretServiceSessionCapability;
}

impl SessionCapabilitySource for SecretServiceCredentialProvider {
    fn test_session_capability(&self) -> SecretServiceSessionCapability {
        self.issue_session_capability(ResourceGeneration::new(1).unwrap())
            .expect("test provider must issue its placement-bound capability")
    }
}

pub struct ProviderHarness<P, A> {
    provider: P,
    admission: A,
    capability: Arc<SecretServiceSessionCapability>,
}

impl<P, A> ProviderHarness<P, A>
where
    P: CredentialProvider + SessionCapabilitySource,
    A: TestAdmission,
{
    pub fn new(provider: P, admission: A) -> Self {
        Self {
            capability: Arc::new(provider.test_session_capability()),
            provider,
            admission,
        }
    }

    pub fn call(
        &self,
        method: CredentialMethod,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let authorization = self
            .admission
            .authorize(method, &request)?
            .with_shared_session_proof(self.capability.clone());
        dispatch_authorized_provider(&self.provider, method, &request, &authorization)
    }
}

pub fn operation_class(method: CredentialMethod) -> OperationClass {
    method.operation_class()
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
    /// Admit the relationship this crate's own delivery session matches.
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
                ResourceRef::parse("Provider/shell-terminal").unwrap(),
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
        ResourceRef::parse("Provider/shell-terminal").unwrap(),
        ResourceGeneration::new(component_generation).unwrap(),
        AudienceToken::parse(audience).unwrap(),
        class,
        request.requested_expiry_unix_ms(),
        request.deadline_unix_ms(),
        DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
}

/// The consumer's authenticated subject at one exact Provider generation.
pub fn subject_context(provider_generation: u64) -> AuthenticatedSubjectContext {
    let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    AuthenticatedSubjectContext::new(
        ResourceRef::parse("Provider/shell-terminal").unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceRef::parse("Zone/user-zone").unwrap(),
        EvidenceClass::UnixPeer,
        SessionPurpose::parse("credential").unwrap(),
        ServiceName::parse("d2b.credential.v3").unwrap(),
        SessionBinding::new(
            SchemaFingerprint::parse(digest).unwrap(),
            TransportBinding::new(
                Locality::Local,
                BindingDigest::parse(digest).unwrap(),
            ),
            ReconnectGeneration::new(1).unwrap(),
            TranscriptHash::from_bytes([9; 32]),
        ),
    )
    .with_execution_ref(ResourceRef::parse("Host/workstation").unwrap())
    .with_provider_ref(ResourceRef::parse("Provider/credential-secret-service").unwrap())
    .with_provider_generation(ResourceGeneration::new(provider_generation).unwrap())
}

/// Authorize one method with an exact delivery session, Provider session, and
/// placement-bound session capability.
#[derive(Clone)]
pub struct SessionAdmission {
    pub delivery: DeliverySessionParams,
    pub provider_generation: u64,
    pub capability: Arc<SecretServiceSessionCapability>,
}

impl TestAdmission for SessionAdmission {
    fn authorize(
        &self,
        method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        let context = subject_context(self.provider_generation);
        let subject = context.clone();
        let session = CredentialSessionBinding::new(context, EXPIRY)?;
        Ok(
            CredentialAuthorization::new_for_subject(method, Some(self.delivery.clone()), subject)?
                .with_authenticated_session(session)?
                .with_shared_session_proof(self.capability.clone()),
        )
    }
}
