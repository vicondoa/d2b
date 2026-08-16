mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use d2b_contracts::v3::Locality;
use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialAuthorization, CredentialMethod, CredentialRequest, CredentialServiceErrorCode,
    PlacementBinding, dispatch_authorized_provider,
};
use d2b_provider_credential_entra::{
    EntraClientError, EntraClientState, EntraConfig, EntraCredentialClient,
    EntraCredentialProviderFactory, EntraFuture, EntraLeaseGrant, EntraLeaseInspection,
    EntraLeaseRef, EntraLeaseRenewal, EntraLeaseRequest, EntraLeaseRevocation, EntraPlacement,
};

use common::{Admission, ProviderHarness, admitted, delivery, request, setup, subject_context_for};

#[test]
fn interaction_required_is_unavailable_not_denied() {
    let (provider, client) = setup();
    *client.state.lock().unwrap() = EntraClientState::InteractionRequired;
    let server = ProviderHarness::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-interaction"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
}

#[test]
fn exact_consumer_mismatch_is_denied_before_client_dispatch() {
    let (provider, client) = setup();
    assert!(!provider.authorizes_consumer(&ResourceRef::parse("Provider/other").unwrap()));
    let admission = Admission {
        authenticated_consumer: ResourceRef::parse("Provider/other").unwrap(),
    };
    let server = ProviderHarness::new(provider, admission);
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-mismatch"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn unauthorized_and_relay_subjects_cannot_resolve_or_refresh() {
    let (provider, client) = setup();
    let unauthorized_request = request("idem-unauthorized");
    let missing_context = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap();
    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &unauthorized_request,
            &missing_context,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    let unauthorized = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
        subject_context_for(
            ResourceRef::parse("User/alice").unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            Locality::Local,
        ),
    )
    .unwrap();
    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &unauthorized_request,
            &unauthorized,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    ProviderHarness::new(&provider, admitted())
        .call(CredentialMethod::AcquireToken, request("idem-authorized"))
        .unwrap();
    let unauthorized_inspect = CredentialAuthorization::new_for_subject(
        CredentialMethod::InspectMetadata,
        None,
        subject_context_for(
            ResourceRef::parse("User/alice").unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            Locality::Local,
        ),
    )
    .unwrap();
    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::InspectMetadata,
            &request("idem-inspect-unauthorized"),
            &unauthorized_inspect,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    let unauthorized_refresh = CredentialAuthorization::new_for_subject(
        CredentialMethod::RefreshToken,
        Some(delivery(CredentialMethod::RefreshToken, 1)),
        subject_context_for(
            ResourceRef::parse("User/alice").unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            Locality::Local,
        ),
    )
    .unwrap();
    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::RefreshToken,
            &request("idem-refresh-unauthorized"),
            &unauthorized_refresh,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    let relay = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
        subject_context_for(
            ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            Locality::AdjacentZone,
        ),
    )
    .unwrap();
    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &unauthorized_request,
            &relay,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn generation_and_unsupported_operation_fail_closed() {
    let (provider, client) = setup();
    assert_eq!(
        provider.validate_endpoint_generation(8).unwrap_err().code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    *client.issue_error.lock().unwrap() = Some(EntraClientError::GenerationMismatch);
    let server = ProviderHarness::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-generation"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );

    let (provider, client) = setup();
    let server = ProviderHarness::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::SignChallenge, request("idem-sign"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::Malformed
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn refresh_failure_degrades_only_the_owning_resource_with_bounded_retry() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-refresh-failure"),
        )
        .unwrap();
    server
        .call(
            CredentialMethod::AcquireToken,
            CredentialRequest::new(
                ResourceRef::parse("Credential/other-entra").unwrap(),
                "operation-other",
                "idem-other",
                common::EXPIRY,
                15_000,
            )
            .unwrap(),
        )
        .unwrap();
    *client.refresh_error.lock().unwrap() = Some(EntraClientError::Unavailable);

    for attempt in 0..d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS {
        assert_eq!(
            server
                .call(
                    CredentialMethod::RefreshToken,
                    request(&format!("idem-refresh-failure-refresh-{attempt}"))
                )
                .unwrap_err()
                .code(),
            CredentialServiceErrorCode::ProviderUnavailable
        );
    }
    let refresh_calls = client.refresh_calls.load(Ordering::SeqCst);
    assert_eq!(
        server
            .call(
                CredentialMethod::RefreshToken,
                request("idem-refresh-failure-bounded")
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(
        client.refresh_calls.load(Ordering::SeqCst),
        refresh_calls,
        "refresh retry must stop at the bounded attempt ceiling"
    );
    assert_eq!(
        provider.resource_health(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some(d2b_provider_credential_entra::EntraResourceHealth::Degraded)
    );
    assert_eq!(
        provider.refresh_retry_state(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some((
            d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS,
            d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS,
        ))
    );
    assert_eq!(
        provider.resource_health(&ResourceRef::parse("Credential/other-entra").unwrap()),
        Some(d2b_provider_credential_entra::EntraResourceHealth::Ready)
    );
}

#[test]
fn client_call_stops_at_request_deadline() {
    let client = Arc::new(NeverClient {
        issue_calls: AtomicUsize::new(0),
    });
    let provider = EntraCredentialProviderFactory::new(
        EntraConfig::new("tenant-1234", 64).unwrap(),
        EntraPlacement::new(
            PlacementBinding::GuestAgent,
            ResourceRef::parse("Guest/consumer").unwrap(),
            ResourceRef::parse("Guest/identity").unwrap(),
            ResourceRef::parse("Endpoint/entra-login").unwrap(),
            7,
        )
        .unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        client.clone(),
    )
    .unwrap()
    .construct();
    let server = ProviderHarness::new(provider, admitted());
    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        result_tx
            .send(
                server.call(
                    CredentialMethod::AcquireToken,
                    CredentialRequest::new(
                        ResourceRef::parse("Credential/work-entra").unwrap(),
                        "operation-deadline",
                        "idem-deadline",
                        common::EXPIRY,
                        10,
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
    });
    assert_eq!(
        result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("permanently pending client call ignored its request deadline")
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::DeadlineExceeded
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
}

struct NeverClient {
    issue_calls: AtomicUsize,
}

impl EntraCredentialClient for NeverClient {
    fn state(&self) -> EntraFuture<'_, EntraClientState> {
        Box::pin(async { Ok(EntraClientState::Ready) })
    }

    fn issue_lease(&self, _request: &EntraLeaseRequest) -> EntraFuture<'_, EntraLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::pending())
    }

    fn inspect_lease(&self, _lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseInspection> {
        Box::pin(async { panic!("unexpected inspect") })
    }

    fn refresh_lease(&self, _lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRenewal> {
        Box::pin(async { panic!("unexpected refresh") })
    }

    fn revoke_lease(&self, _lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRevocation> {
        Box::pin(async { panic!("unexpected revoke") })
    }
}
