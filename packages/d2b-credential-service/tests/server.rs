use std::sync::atomic::{AtomicUsize, Ordering};

use d2b_contracts::v3::credential::{
    AudienceToken, CredentialLeaseHandle, CredentialLeaseState, CredentialSourceVersion,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ResourceUid};
use d2b_credential_service::{
    CredentialAdmission, CredentialMetadata, CredentialMethod, CredentialOutcomeCode,
    CredentialProvider, CredentialRequest, CredentialResponse, CredentialServer,
    CredentialServiceError, CredentialServiceErrorCode, CredentialTransport, DeliveryResponse,
    DeliveryRouteDigest, DeliverySessionParams, MetadataResponse,
};

struct Admission {
    result: Result<(), CredentialServiceError>,
}

impl CredentialAdmission for Admission {
    fn authorize(
        &self,
        _method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<(), CredentialServiceError> {
        self.result
    }
}

struct Provider<'a> {
    calls: &'a AtomicUsize,
    result: Result<CredentialResponse, CredentialServiceError>,
}

impl CredentialProvider for Provider<'_> {
    fn dispatch(
        &self,
        _method: CredentialMethod,
        _request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}

fn request() -> CredentialRequest {
    CredentialRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        "operation-1",
        "idempotency-1",
        2_000,
        1_500,
    )
    .unwrap()
}

fn metadata(state: CredentialLeaseState) -> CredentialMetadata {
    CredentialMetadata {
        lease_handle: CredentialLeaseHandle::parse("lease-1").unwrap(),
        rotation_generation: 1,
        source_version: CredentialSourceVersion::parse("source-1").unwrap(),
        expires_at_unix_ms: 2_000,
        state,
        outcome: CredentialOutcomeCode::Success,
    }
}

fn acquire_response() -> CredentialResponse {
    CredentialResponse::AcquireToken(DeliveryResponse {
        metadata: metadata(CredentialLeaseState::Active),
        delivery_session_params: DeliverySessionParams::new(
            ResourceRef::parse("Credential/work-entra").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ResourceRef::parse("Provider/display-wayland").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            AudienceToken::parse("azure-resource-manager").unwrap(),
            CredentialMethod::AcquireToken.operation_class(),
            2_000,
            1_500,
            DeliveryRouteDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            4_096,
            1,
        )
        .unwrap(),
    })
}

#[test]
fn denial_occurs_before_provider_dispatch() {
    let calls = AtomicUsize::new(0);
    let server = CredentialServer::new(
        Provider {
            calls: &calls,
            result: Ok(acquire_response()),
        },
        Admission {
            result: Err(CredentialServiceError::new(
                CredentialServiceErrorCode::OperationDenied,
            )),
        },
    );
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request())
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn locked_unavailable_and_expired_states_remain_closed_codes() {
    for (_scenario, code) in [
        ("locked", CredentialServiceErrorCode::ProviderUnavailable),
        (
            "unavailable",
            CredentialServiceErrorCode::ProviderUnavailable,
        ),
        ("denied", CredentialServiceErrorCode::OperationDenied),
        ("expired", CredentialServiceErrorCode::LeaseExpired),
        ("revoked", CredentialServiceErrorCode::LeaseRevoked),
    ] {
        let calls = AtomicUsize::new(0);
        let server = CredentialServer::new(
            Provider {
                calls: &calls,
                result: Err(CredentialServiceError::new(code)),
            },
            Admission { result: Ok(()) },
        );
        assert_eq!(
            server
                .call(CredentialMethod::InspectMetadata, request())
                .unwrap_err()
                .code(),
            code
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn non_delivery_methods_reject_a_delivery_shaped_response() {
    let calls = AtomicUsize::new(0);
    let server = CredentialServer::new(
        Provider {
            calls: &calls,
            result: Ok(CredentialResponse::RevokeToken(MetadataResponse {
                metadata: metadata(CredentialLeaseState::Revoked),
            })),
        },
        Admission { result: Ok(()) },
    );
    assert!(
        server
            .call(CredentialMethod::InspectMetadata, request())
            .is_err()
    );
}
