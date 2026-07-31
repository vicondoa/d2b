mod common;

use std::future::poll_fn;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Poll, Waker};
use std::thread;
use std::time::Duration;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialLeaseHandle, CredentialLeaseState, CredentialMethod, CredentialOutcomeCode,
    CredentialRequest, CredentialResponse, CredentialServiceErrorCode, CredentialSourceVersion,
    PlacementBinding,
};
use d2b_provider_credential_secret_service::{
    LockPolicy, Oo7SecretServicePort, SecretServiceConfig, SecretServiceCredentialProvider,
    SecretServiceCredentialProviderFactory, SecretServiceFuture, SecretServiceLeaseGrant,
    SecretServiceLeaseInspection, SecretServiceLeaseRef, SecretServiceLeaseRenewal,
    SecretServiceLeaseRequest, SecretServiceLeaseRevocation, SecretServicePlacement,
    SecretServiceState,
};

use common::{Admission, ProviderHarness, request, setup};

#[test]
fn acquire_refresh_revoke_and_inspect_use_the_injected_port() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    let acquired = server
        .call(CredentialMethod::AcquireToken, request("idem-acquire"))
        .unwrap();
    assert!(matches!(acquired, CredentialResponse::AcquireToken(_)));
    let refreshed = server
        .call(CredentialMethod::RefreshToken, request("idem-refresh"))
        .unwrap();
    assert!(matches!(refreshed, CredentialResponse::RefreshToken(_)));
    let inspected = server
        .call(CredentialMethod::InspectMetadata, request("idem-inspect"))
        .unwrap();
    let CredentialResponse::InspectMetadata(inspected) = inspected else {
        panic!("inspect response");
    };
    assert_eq!(inspected.metadata.state, CredentialLeaseState::Active);
    assert_eq!(inspected.metadata.rotation_generation, 2);
    let revoked = server
        .call(CredentialMethod::RevokeToken, request("idem-revoke"))
        .unwrap();
    let CredentialResponse::RevokeToken(revoked) = revoked else {
        panic!("revoke response");
    };
    assert_eq!(revoked.metadata.outcome, CredentialOutcomeCode::Revoked);
    let repeated = server
        .call(CredentialMethod::RevokeToken, request("idem-revoke-2"))
        .unwrap();
    let CredentialResponse::RevokeToken(repeated) = repeated else {
        panic!("revoke response");
    };
    assert_eq!(
        repeated.metadata.outcome,
        CredentialOutcomeCode::AlreadyRevoked
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
    assert_eq!(port.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_acquire_is_idempotent() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(CredentialMethod::AcquireToken, request("same-key"))
        .unwrap();
    server
        .call(CredentialMethod::AcquireToken, request("same-key"))
        .unwrap();
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_acquires_issue_once() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let port = Arc::new(BlockingPort::new(entered_tx));
    let server = Arc::new(ProviderHarness::new(
        provider_with_port(64, port.clone()),
        Admission,
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
    port.release();
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
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn revocation_releases_capacity() {
    let (provider, port) = setup(1);
    let server = ProviderHarness::new(provider, Admission);
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
                ResourceRef::parse("Credential/other-keyring").unwrap(),
                "operation-2",
                "idem-second",
                common::EXPIRY,
                15_000,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 2);
}

struct BlockingPort {
    entered: Mutex<Option<mpsc::Sender<()>>>,
    state: Mutex<BlockingState>,
    issue_calls: AtomicUsize,
}

struct BlockingState {
    released: bool,
    wakers: Vec<Waker>,
}

impl BlockingPort {
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

impl Oo7SecretServicePort for BlockingPort {
    fn state(&self) -> SecretServiceFuture<'_, SecretServiceState> {
        Box::pin(async { Ok(SecretServiceState::Unlocked) })
    }

    fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant> {
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
            Ok(SecretServiceLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("blocking-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("blocking-source").unwrap(),
                rotation_generation: 1,
                expires_at_unix_ms: expiry,
            })
        })
    }

    fn inspect_lease(
        &self,
        _lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseInspection> {
        Box::pin(async { panic!("unexpected inspect") })
    }

    fn refresh_lease(
        &self,
        _lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRenewal> {
        Box::pin(async { panic!("unexpected refresh") })
    }

    fn revoke_lease(
        &self,
        _lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRevocation> {
        Box::pin(async { panic!("unexpected revoke") })
    }
}

fn provider_with_port(
    max_leases: u32,
    port: Arc<dyn Oo7SecretServicePort>,
) -> SecretServiceCredentialProvider {
    SecretServiceCredentialProviderFactory::new(
        SecretServiceConfig::new("login collection", max_leases, LockPolicy::FailClosed).unwrap(),
        SecretServicePlacement::new(
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/workstation").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap(),
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port,
    )
    .unwrap()
    .construct()
}
