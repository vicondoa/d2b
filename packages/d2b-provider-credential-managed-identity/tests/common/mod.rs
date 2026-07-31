#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use d2b_contracts::v3::credential::{
    AudienceToken, CredentialLeaseHandle, CredentialLeaseState, CredentialSourceVersion,
    PlacementBinding,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ResourceUid};
use d2b_credential_service::{
    CredentialAdmission, CredentialAuthorization, CredentialMethod, CredentialRequest,
    CredentialServiceError, DeliveryRouteDigest, DeliverySessionParams,
};
use d2b_provider_credential_managed_identity::{
    ManagedIdentityClientConfig, ManagedIdentityClientError, ManagedIdentityClientState,
    ManagedIdentityCredentialClient, ManagedIdentityCredentialProvider,
    ManagedIdentityCredentialProviderFactory, ManagedIdentityFuture, ManagedIdentityLeaseGrant,
    ManagedIdentityLeaseInspection, ManagedIdentityLeaseRef, ManagedIdentityLeaseRenewal,
    ManagedIdentityLeaseRequest, ManagedIdentityLeaseRevocation, ManagedIdentityPlacement,
};

pub const EXPIRY: u64 = 20_000;

pub struct FakeClient {
    pub state: Mutex<ManagedIdentityClientState>,
    pub inspection: Mutex<Option<ManagedIdentityLeaseInspection>>,
    pub issue_calls: AtomicUsize,
    pub inspect_calls: AtomicUsize,
    pub refresh_calls: AtomicUsize,
    pub revoke_calls: AtomicUsize,
    pub issue_error: Mutex<Option<ManagedIdentityClientError>>,
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
        let token = self.token_canary.clone();
        let endpoint = self.endpoint_canary.clone();
        let response = self.response_canary.clone();
        let inspection = &self.inspection;
        Box::pin(async move {
            if state == ManagedIdentityClientState::Unavailable {
                return Err(ManagedIdentityClientError::Unavailable);
            }
            if let Some(error) = error {
                return Err(error);
            }
            assert!(!token.is_empty() && !endpoint.is_empty() && !response.is_empty());
            let grant = ManagedIdentityLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("managed-identity-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("managed-identity-source-1")
                    .unwrap(),
                rotation_generation: 1,
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
        let inspection = &self.inspection;
        Box::pin(async move {
            let grant = ManagedIdentityLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("managed-identity-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("managed-identity-source-2")
                    .unwrap(),
                rotation_generation: 2,
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
    DeliverySessionParams::new(
        ResourceRef::parse("Credential/aca-relay-mi").unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        AudienceToken::parse("azure-resource-manager").unwrap(),
        method.operation_class(),
        EXPIRY,
        15_000,
        DeliveryRouteDigest::parse(format!("sha256:{}", "d".repeat(64))).unwrap(),
        4_096,
        sequence,
    )
    .unwrap()
}

#[derive(Clone)]
pub struct Admission {
    pub authenticated_consumer: ResourceRef,
}

impl CredentialAdmission for Admission {
    fn authorize(
        &self,
        method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        if self.authenticated_consumer
            != ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap()
        {
            return Err(CredentialServiceError::new(
                d2b_credential_service::CredentialServiceErrorCode::OperationDenied,
            ));
        }
        CredentialAuthorization::new(
            method,
            method.requires_delivery().then(|| delivery(method, 1)),
        )
    }
}

pub fn admitted() -> Admission {
    Admission {
        authenticated_consumer: ResourceRef::parse("Provider/runtime-azure-container-apps")
            .unwrap(),
    }
}
