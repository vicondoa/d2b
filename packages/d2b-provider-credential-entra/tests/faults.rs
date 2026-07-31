mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialMethod, CredentialRequest, CredentialServiceErrorCode, PlacementBinding,
};
use d2b_provider_credential_entra::{
    EntraClientError, EntraClientState, EntraConfig, EntraCredentialClient,
    EntraCredentialProviderFactory, EntraFuture, EntraLeaseGrant, EntraLeaseInspection,
    EntraLeaseRef, EntraLeaseRenewal, EntraLeaseRequest, EntraLeaseRevocation, EntraPlacement,
};

use common::{Admission, ProviderHarness, admitted, request, setup};

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
