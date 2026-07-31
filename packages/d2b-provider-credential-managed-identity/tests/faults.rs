mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialMethod, CredentialRequest, CredentialServiceErrorCode, PlacementBinding,
};
use d2b_provider_credential_managed_identity::{
    ManagedIdentityClientConfig, ManagedIdentityClientState, ManagedIdentityCredentialClient,
    ManagedIdentityCredentialProviderFactory, ManagedIdentityFuture, ManagedIdentityLeaseGrant,
    ManagedIdentityLeaseInspection, ManagedIdentityLeaseRef, ManagedIdentityLeaseRenewal,
    ManagedIdentityLeaseRequest, ManagedIdentityLeaseRevocation, ManagedIdentityPlacement,
};

use common::{Admission, ProviderHarness, admitted, request, setup};

#[test]
fn unavailable_maps_to_provider_unavailable_without_fallback() {
    let (provider, client) = setup();
    *client.state.lock().unwrap() = ManagedIdentityClientState::Unavailable;
    let server = ProviderHarness::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-unavailable"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn exact_sdk_consumer_mismatch_denies_before_client_dispatch() {
    let (provider, client) = setup();
    assert!(!provider.authorizes_consumer(&ResourceRef::parse("Provider/other").unwrap()));
    let server = ProviderHarness::new(
        provider,
        Admission {
            authenticated_consumer: ResourceRef::parse("Provider/other").unwrap(),
        },
    );
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
fn sign_challenge_is_schema_invalid_before_client_use() {
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
    let provider = ManagedIdentityCredentialProviderFactory::new(
        ManagedIdentityClientConfig::new("client-1234", "azure-imds-aca", 64).unwrap(),
        ManagedIdentityPlacement::new(
            PlacementBinding::GuestAgent,
            ResourceRef::parse("Guest/aca-sandbox").unwrap(),
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
                        ResourceRef::parse("Credential/aca-relay-mi").unwrap(),
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

impl ManagedIdentityCredentialClient for NeverClient {
    fn state(&self) -> ManagedIdentityFuture<'_, ManagedIdentityClientState> {
        Box::pin(async { Ok(ManagedIdentityClientState::Ready) })
    }

    fn issue_lease(
        &self,
        _request: &ManagedIdentityLeaseRequest,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::pending())
    }

    fn inspect_lease(
        &self,
        _lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseInspection> {
        Box::pin(async { panic!("unexpected inspect") })
    }

    fn refresh_lease(
        &self,
        _lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseRenewal> {
        Box::pin(async { panic!("unexpected refresh") })
    }

    fn revoke_lease(
        &self,
        _lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseRevocation> {
        Box::pin(async { panic!("unexpected revoke") })
    }
}
