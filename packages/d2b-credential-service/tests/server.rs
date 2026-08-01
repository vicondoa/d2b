use std::sync::atomic::{AtomicUsize, Ordering};

use d2b_contracts::v3::credential::{
    AudienceToken, CredentialLeaseHandle, CredentialLeaseState, CredentialSourceVersion,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ResourceUid};
use d2b_credential_service::{
    CredentialAdmission, CredentialAuthorization, CredentialFailureState, CredentialMetadata,
    CredentialMethod, CredentialOutcomeCode, CredentialProvider, CredentialRequest,
    CredentialResponse, CredentialServer, CredentialServiceError, CredentialServiceErrorCode,
    CredentialTransport, DeliveryResponse, DeliveryRouteDigest, DeliverySessionParams,
    MetadataResponse, error_for_failure_state,
};

struct Admission {
    result: Result<CredentialAuthorization, CredentialServiceError>,
}

impl CredentialAdmission for Admission {
    fn authorize(
        &self,
        _method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        self.result.clone()
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
        _request: &CredentialRequest,
        _authorization: &CredentialAuthorization,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeliveryChange {
    CredentialRef,
    CredentialUid,
    CredentialGeneration,
    ConsumerRef,
    ConsumerGeneration,
    Audience,
    Operation,
    Expiry,
    Deadline,
    Route,
    MaxTokenBytes,
    Sequence,
}

fn delivery_params(consumer: &str) -> DeliverySessionParams {
    delivery_params_with_change(consumer, None)
}

fn delivery_params_with_change(
    consumer: &str,
    change: Option<DeliveryChange>,
) -> DeliverySessionParams {
    DeliverySessionParams::new(
        ResourceRef::parse(if change == Some(DeliveryChange::CredentialRef) {
            "Credential/personal-entra"
        } else {
            "Credential/work-entra"
        })
        .unwrap(),
        ResourceUid::parse(if change == Some(DeliveryChange::CredentialUid) {
            "123e4567-e89b-42d3-a456-426614174001"
        } else {
            "123e4567-e89b-42d3-a456-426614174000"
        })
        .unwrap(),
        ResourceGeneration::new(if change == Some(DeliveryChange::CredentialGeneration) {
            2
        } else {
            1
        })
        .unwrap(),
        ResourceRef::parse(consumer).unwrap(),
        ResourceGeneration::new(if change == Some(DeliveryChange::ConsumerGeneration) {
            2
        } else {
            1
        })
        .unwrap(),
        AudienceToken::parse(if change == Some(DeliveryChange::Audience) {
            "azure-key-vault"
        } else {
            "azure-resource-manager"
        })
        .unwrap(),
        if change == Some(DeliveryChange::Operation) {
            CredentialMethod::SignChallenge.operation_class()
        } else {
            CredentialMethod::AcquireToken.operation_class()
        },
        if change == Some(DeliveryChange::Expiry) {
            2_100
        } else {
            2_000
        },
        if change == Some(DeliveryChange::Deadline) {
            1_400
        } else {
            1_500
        },
        DeliveryRouteDigest::parse(format!(
            "sha256:{}",
            if change == Some(DeliveryChange::Route) {
                "b".repeat(64)
            } else {
                "a".repeat(64)
            }
        ))
        .unwrap(),
        if change == Some(DeliveryChange::MaxTokenBytes) {
            2_048
        } else {
            4_096
        },
        if change == Some(DeliveryChange::Sequence) {
            2
        } else {
            1
        },
    )
    .unwrap()
}

fn acquire_response(delivery_session_params: DeliverySessionParams) -> CredentialResponse {
    CredentialResponse::AcquireToken(DeliveryResponse {
        metadata: metadata(CredentialLeaseState::Active),
        delivery_session_params,
    })
}

fn metadata_authorization() -> CredentialAuthorization {
    CredentialAuthorization::new(CredentialMethod::InspectMetadata, None).unwrap()
}

fn acquire_authorization(delivery: DeliverySessionParams) -> CredentialAuthorization {
    CredentialAuthorization::new(CredentialMethod::AcquireToken, Some(delivery)).unwrap()
}

#[test]
fn denial_occurs_before_provider_dispatch() {
    let calls = AtomicUsize::new(0);
    let server = CredentialServer::new(
        Provider {
            calls: &calls,
            result: Ok(acquire_response(delivery_params(
                "Provider/display-wayland",
            ))),
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
    struct StateProvider<'a> {
        calls: &'a AtomicUsize,
        state: CredentialFailureState,
    }

    impl CredentialProvider for StateProvider<'_> {
        fn dispatch(
            &self,
            _method: CredentialMethod,
            _request: &CredentialRequest,
            _authorization: &CredentialAuthorization,
        ) -> Result<CredentialResponse, CredentialServiceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(error_for_failure_state(self.state))
        }
    }

    for (state, code) in [
        (
            CredentialFailureState::Locked,
            CredentialServiceErrorCode::ProviderUnavailable,
        ),
        (
            CredentialFailureState::Unavailable,
            CredentialServiceErrorCode::ProviderUnavailable,
        ),
        (
            CredentialFailureState::Denied,
            CredentialServiceErrorCode::OperationDenied,
        ),
        (
            CredentialFailureState::Expired,
            CredentialServiceErrorCode::LeaseExpired,
        ),
        (
            CredentialFailureState::Revoked,
            CredentialServiceErrorCode::LeaseRevoked,
        ),
    ] {
        let calls = AtomicUsize::new(0);
        let server = CredentialServer::new(
            StateProvider {
                calls: &calls,
                state,
            },
            Admission {
                result: Ok(metadata_authorization()),
            },
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
fn provider_cannot_replace_the_bus_authorized_delivery_binding() {
    let authorized = delivery_params("Provider/display-wayland");
    let matching_calls = AtomicUsize::new(0);
    let matching_server = CredentialServer::new(
        Provider {
            calls: &matching_calls,
            result: Ok(acquire_response(authorized.clone())),
        },
        Admission {
            result: Ok(acquire_authorization(authorized.clone())),
        },
    );
    assert!(
        matching_server
            .call(CredentialMethod::AcquireToken, request())
            .is_ok()
    );

    for change in [
        DeliveryChange::CredentialRef,
        DeliveryChange::CredentialUid,
        DeliveryChange::CredentialGeneration,
        DeliveryChange::ConsumerRef,
        DeliveryChange::ConsumerGeneration,
        DeliveryChange::Audience,
        DeliveryChange::Operation,
        DeliveryChange::Expiry,
        DeliveryChange::Deadline,
        DeliveryChange::Route,
        DeliveryChange::MaxTokenBytes,
        DeliveryChange::Sequence,
    ] {
        let calls = AtomicUsize::new(0);
        let consumer = if change == DeliveryChange::ConsumerRef {
            "Provider/notification-desktop"
        } else {
            "Provider/display-wayland"
        };
        let provider_selected = delivery_params_with_change(consumer, Some(change));
        let server = CredentialServer::new(
            Provider {
                calls: &calls,
                result: Ok(acquire_response(provider_selected)),
            },
            Admission {
                result: Ok(acquire_authorization(authorized.clone())),
            },
        );

        assert_eq!(
            server
                .call(CredentialMethod::AcquireToken, request())
                .unwrap_err()
                .code(),
            CredentialServiceErrorCode::InvariantFailure
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn service_errors_render_canonical_codes() {
    for (code, expected) in [
        (
            CredentialServiceErrorCode::Malformed,
            "credential-schema-invalid",
        ),
        (
            CredentialServiceErrorCode::Oversize,
            "credential-schema-invalid",
        ),
        (
            CredentialServiceErrorCode::DeadlineExceeded,
            "deadline-exceeded",
        ),
        (
            CredentialServiceErrorCode::OperationDenied,
            "credential-operation-denied",
        ),
        (
            CredentialServiceErrorCode::ProviderUnavailable,
            "credential-provider-unavailable",
        ),
        (
            CredentialServiceErrorCode::LeaseExpired,
            "credential-lease-expired",
        ),
        (
            CredentialServiceErrorCode::LeaseRevoked,
            "credential-lease-revoked",
        ),
        (
            CredentialServiceErrorCode::InvariantFailure,
            "credential-invariant-failure",
        ),
    ] {
        assert_eq!(CredentialServiceError::new(code).to_string(), expected);
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
        Admission {
            result: Ok(metadata_authorization()),
        },
    );
    assert!(
        server
            .call(CredentialMethod::InspectMetadata, request())
            .is_err()
    );
}
