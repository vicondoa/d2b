mod common;

use std::future::poll_fn;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Poll, Waker};
use std::thread;
use std::time::Duration;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialLeaseHandle, CredentialMethod, CredentialRequest, CredentialResponse,
    CredentialServiceErrorCode, CredentialSourceVersion, PlacementBinding,
};
use d2b_provider_credential_managed_identity::{
    ManagedIdentityClientConfig, ManagedIdentityClientState, ManagedIdentityCredentialClient,
    ManagedIdentityCredentialProvider, ManagedIdentityCredentialProviderFactory,
    ManagedIdentityFuture, ManagedIdentityLeaseGrant, ManagedIdentityLeaseInspection,
    ManagedIdentityLeaseRef, ManagedIdentityLeaseRenewal, ManagedIdentityLeaseRequest,
    ManagedIdentityLeaseRevocation, ManagedIdentityPlacement,
};

use common::{ProviderHarness, admitted, request, setup};

#[test]
fn acquire_refresh_revoke_and_inspect_use_the_injected_client() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(provider, admitted());
    assert!(matches!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-acquire"))
            .unwrap(),
        CredentialResponse::AcquireToken(_)
    ));
    assert!(matches!(
        server
            .call(CredentialMethod::RefreshToken, request("idem-refresh"))
            .unwrap(),
        CredentialResponse::RefreshToken(_)
    ));
    assert!(matches!(
        server
            .call(CredentialMethod::InspectMetadata, request("idem-inspect"))
            .unwrap(),
        CredentialResponse::InspectMetadata(_)
    ));
    assert!(matches!(
        server
            .call(CredentialMethod::RevokeToken, request("idem-revoke"))
            .unwrap(),
        CredentialResponse::RevokeToken(_)
    ));
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.revoke_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_acquire_is_idempotent() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(provider, admitted());
    server
        .call(CredentialMethod::AcquireToken, request("idem-same"))
        .unwrap();
    server
        .call(CredentialMethod::AcquireToken, request("idem-same"))
        .unwrap();
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_acquires_issue_once() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let client = Arc::new(BlockingClient::new(entered_tx));
    let server = Arc::new(ProviderHarness::new(
        provider_with_client(64, client.clone()),
        admitted(),
    ));
    let (first_tx, first_rx) = mpsc::channel();
    let first_server = server.clone();
    let first = thread::spawn(move || {
        first_tx
            .send(first_server.call(CredentialMethod::AcquireToken, request("same-key")))
            .unwrap();
    });
    entered_rx.recv().unwrap();

    let (second_tx, second_rx) = mpsc::channel();
    let second_server = server.clone();
    let second = thread::spawn(move || {
        second_tx
            .send(second_server.call(CredentialMethod::AcquireToken, request("same-key")))
            .unwrap();
    });
    let second_result = second_rx.recv_timeout(Duration::from_secs(1));
    client.release();
    let first_result = first_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    first.join().unwrap();
    second.join().unwrap();

    first_result.unwrap();
    assert_eq!(
        second_result
            .expect("second acquire did not fail fast while issuance was pending")
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn revocation_releases_capacity() {
    let client = Arc::new(common::FakeClient::new());
    let server = ProviderHarness::new(provider_with_client(1, client.clone()), admitted());
    server
        .call(CredentialMethod::AcquireToken, request("idem-first"))
        .unwrap();
    server
        .call(CredentialMethod::RevokeToken, request("idem-revoke"))
        .unwrap();
    server
        .call(
            CredentialMethod::AcquireToken,
            CredentialRequest::new(
                ResourceRef::parse("Credential/other-managed-identity").unwrap(),
                "operation-2",
                "idem-second",
                common::EXPIRY,
                15_000,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 2);
}

struct BlockingClient {
    entered: Mutex<Option<mpsc::Sender<()>>>,
    state: Mutex<BlockingState>,
    issue_calls: AtomicUsize,
}

struct BlockingState {
    released: bool,
    wakers: Vec<Waker>,
}

impl BlockingClient {
    fn new(entered: mpsc::Sender<()>) -> Self {
        Self {
            entered: Mutex::new(Some(entered)),
            state: Mutex::new(BlockingState {
                released: false,
                wakers: Vec::new(),
            }),
            issue_calls: AtomicUsize::new(0),
        }
    }

    fn release(&self) {
        let wakers = {
            let mut state = self.state.lock().unwrap();
            state.released = true;
            std::mem::take(&mut state.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
    }
}

impl ManagedIdentityCredentialClient for BlockingClient {
    fn state(&self) -> ManagedIdentityFuture<'_, ManagedIdentityClientState> {
        Box::pin(async { Ok(ManagedIdentityClientState::Ready) })
    }

    fn issue_lease(
        &self,
        request: &ManagedIdentityLeaseRequest,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseGrant> {
        let call = self.issue_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            let entered = self.entered.lock().unwrap().take().unwrap();
            entered.send(()).unwrap();
        } else {
            self.release();
        }
        let expiry = request.requested_expiry_unix_ms();
        Box::pin(async move {
            poll_fn(|context| {
                let mut state = self.state.lock().unwrap();
                if state.released {
                    Poll::Ready(())
                } else {
                    state.wakers.push(context.waker().clone());
                    Poll::Pending
                }
            })
            .await;
            Ok(ManagedIdentityLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("blocking-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("blocking-source").unwrap(),
                rotation_generation: 1,
                expires_at_unix_ms: expiry,
            })
        })
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

fn provider_with_client(
    max_leases: u32,
    client: Arc<dyn ManagedIdentityCredentialClient>,
) -> ManagedIdentityCredentialProvider {
    ManagedIdentityCredentialProviderFactory::new(
        ManagedIdentityClientConfig::new("client-1234", "azure-imds-aca", max_leases).unwrap(),
        ManagedIdentityPlacement::new(
            PlacementBinding::GuestAgent,
            ResourceRef::parse("Guest/aca-sandbox").unwrap(),
        )
        .unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        client,
    )
    .unwrap()
    .construct()
}
