mod common;

use std::future::poll_fn;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Poll, Waker};
use std::thread;
use std::time::Duration;

use d2b_contracts_provider::v3::{
    credential::{
    CredentialAuthorization, CredentialLeaseHandle, CredentialLeaseState, CredentialMethod,
    CredentialOutcomeCode, CredentialProvider, CredentialRequest, CredentialResponse,
    CredentialServiceErrorCode, CredentialSourceVersion, PlacementBinding,
},
};
use d2b_contracts_resource::v3::{
    ResourceRef,
    ZoneId,
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
fn acquire_with_a_new_key_does_not_orphan_the_existing_lease() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    let first = server
        .call(CredentialMethod::AcquireToken, request("first-key"))
        .unwrap();
    let second = server
        .call(CredentialMethod::AcquireToken, request("second-key"))
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
    server
        .call(CredentialMethod::RevokeToken, request("revoke-key"))
        .unwrap();
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_refresh_is_idempotent() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-acquire-refresh"),
        )
        .unwrap();

    let first = server
        .call(CredentialMethod::RefreshToken, request("same-refresh-key"))
        .unwrap();
    let second = server
        .call(CredentialMethod::RefreshToken, request("same-refresh-key"))
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(port.refresh_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn an_older_refresh_key_replays_its_original_result() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-acquire-old-refresh"),
        )
        .unwrap();

    let first = server
        .call(CredentialMethod::RefreshToken, request("first-refresh-key"))
        .unwrap();
    server
        .call(
            CredentialMethod::RefreshToken,
            request("second-refresh-key"),
        )
        .unwrap();
    let replay = server
        .call(CredentialMethod::RefreshToken, request("first-refresh-key"))
        .unwrap();

    assert_eq!(replay, first);
    assert_eq!(port.refresh_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn inspect_persists_terminal_state_for_later_revoke() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-inspect-terminal"),
        )
        .unwrap();
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Revoked,
        source_version: CredentialSourceVersion::parse("terminal-source").unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });

    let inspected = server
        .call(
            CredentialMethod::InspectMetadata,
            request("idem-inspect-terminal-read"),
        )
        .unwrap();
    let CredentialResponse::InspectMetadata(inspected) = inspected else {
        panic!("inspect response");
    };
    assert_eq!(inspected.metadata.state, CredentialLeaseState::Revoked);

    let revoked = server
        .call(
            CredentialMethod::RevokeToken,
            request("idem-inspect-terminal-revoke"),
        )
        .unwrap();
    let CredentialResponse::RevokeToken(revoked) = revoked else {
        panic!("revoke response");
    };
    assert_eq!(
        revoked.metadata.outcome,
        CredentialOutcomeCode::AlreadyRevoked
    );
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn inspect_unknown_state_is_fenced_and_cannot_restore_active_metadata() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-inspect-unknown"),
        )
        .unwrap();
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Unknown,
        source_version: CredentialSourceVersion::parse("unknown-source").unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });

    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-inspect-unknown-read"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Active,
        source_version: CredentialSourceVersion::parse("restored-source").unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });
    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-inspect-unknown-retry"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(port.inspect_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn inspect_cannot_restore_a_revoked_lease() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-inspect-revoked"),
        )
        .unwrap();
    server
        .call(
            CredentialMethod::RevokeToken,
            request("idem-inspect-revoked-revoke"),
        )
        .unwrap();

    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-inspect-revoked-read"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseRevoked
    );
    assert_eq!(port.inspect_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn refresh_preflight_terminal_state_is_sticky() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-refresh-terminal"),
        )
        .unwrap();
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Revoked,
        source_version: CredentialSourceVersion::parse("refresh-terminal-source").unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });

    assert_eq!(
        server
            .call(
                CredentialMethod::RefreshToken,
                request("idem-refresh-terminal-call"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseRevoked
    );
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Active,
        source_version: CredentialSourceVersion::parse("refresh-restored-source").unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });
    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-refresh-terminal-read"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseRevoked
    );
    assert_eq!(port.inspect_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn refresh_rotation_mismatch_fences_the_lease() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-refresh-generation"),
        )
        .unwrap();
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Active,
        source_version: CredentialSourceVersion::parse("mismatch-source").unwrap(),
        rotation_generation: 2,
        expires_at_unix_ms: common::EXPIRY,
    });

    assert_eq!(
        server
            .call(
                CredentialMethod::RefreshToken,
                request("idem-refresh-generation-call"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    *port.inspection.lock().unwrap() = Some(SecretServiceLeaseInspection {
        state: CredentialLeaseState::Active,
        source_version: CredentialSourceVersion::parse("mismatch-restored-source").unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });
    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-refresh-generation-read"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(port.inspect_calls.load(Ordering::SeqCst), 1);
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
fn disconnect_waits_for_inflight_acquire_and_fences_the_session() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let port = Arc::new(BlockingPort::new(entered_tx));
    let provider = Arc::new(provider_with_port(64, port.clone()));
    let capability = Arc::new(
        provider
            .issue_session_capability(
                d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            )
            .unwrap(),
    );
    let authorization = Arc::new(
        CredentialAuthorization::new(
            CredentialMethod::AcquireToken,
            Some(common::delivery(CredentialMethod::AcquireToken, 1)),
        )
        .unwrap()
        .with_shared_session_proof(capability),
    );

    let acquire_provider = provider.clone();
    let acquire_authorization = authorization.clone();
    let (acquire_tx, acquire_rx) = mpsc::channel();
    let acquire = thread::spawn(move || {
        acquire_tx
            .send(acquire_provider.dispatch(
                CredentialMethod::AcquireToken,
                &request("race-acquire"),
                &acquire_authorization,
            ))
            .unwrap();
    });
    entered_rx.recv().unwrap();

    let disconnect_provider = provider.clone();
    let disconnect_authorization = authorization.clone();
    let (disconnect_tx, disconnect_rx) = mpsc::channel();
    let disconnect = thread::spawn(move || {
        disconnect_tx
            .send(disconnect_provider.disconnect(&disconnect_authorization))
            .unwrap();
    });
    assert!(
        disconnect_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    port.release();
    assert!(
        acquire_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_ok()
    );
    assert!(
        disconnect_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_ok()
    );
    acquire.join().unwrap();
    disconnect.join().unwrap();
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider
            .dispatch(
                CredentialMethod::InspectMetadata,
                &request("after-race"),
                &authorization,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
}

#[test]
fn inspect_waits_on_the_lifecycle_gate_before_disconnect() {
    let (inspect_tx, inspect_rx) = mpsc::channel();
    let port = Arc::new(InspectBlockingPort::new(inspect_tx));
    let provider = Arc::new(provider_with_port(64, port.clone()));
    let capability = Arc::new(
        provider
            .issue_session_capability(
                d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            )
            .unwrap(),
    );
    let acquire_authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(common::delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap()
    .with_shared_session_proof(capability.clone());
    provider
        .dispatch(
            CredentialMethod::AcquireToken,
            &request("inspect-fence-acquire"),
            &acquire_authorization,
        )
        .unwrap();
    let inspect_authorization =
        CredentialAuthorization::new(CredentialMethod::InspectMetadata, None)
            .unwrap()
            .with_shared_session_proof(capability);

    let inspect_provider = provider.clone();
    let inspect_authorization_for_thread = inspect_authorization.clone();
    let (inspect_result_tx, inspect_result_rx) = mpsc::channel();
    let inspect = thread::spawn(move || {
        inspect_result_tx
            .send(inspect_provider.dispatch(
                CredentialMethod::InspectMetadata,
                &request("inspect-fence"),
                &inspect_authorization_for_thread,
            ))
            .unwrap();
    });
    inspect_rx.recv().unwrap();

    let disconnect_provider = provider.clone();
    let disconnect_authorization = acquire_authorization.clone();
    let (disconnect_tx, disconnect_rx) = mpsc::channel();
    let disconnect = thread::spawn(move || {
        disconnect_tx
            .send(disconnect_provider.disconnect(&disconnect_authorization))
            .unwrap();
    });
    assert!(
        disconnect_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    port.release();
    assert!(
        inspect_result_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_ok()
    );
    assert!(
        disconnect_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_ok()
    );
    inspect.join().unwrap();
    disconnect.join().unwrap();
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 1);
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
    revoke_calls: AtomicUsize,
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
            revoke_calls: AtomicUsize::new(0),
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
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretServiceLeaseRevocation::Revoked) })
    }
}

struct InspectBlockingPort {
    entered: Mutex<Option<mpsc::Sender<()>>>,
    state: Mutex<BlockingState>,
    revoke_calls: AtomicUsize,
}

impl InspectBlockingPort {
    fn new(entered: mpsc::Sender<()>) -> Self {
        Self {
            entered: Mutex::new(Some(entered)),
            state: Mutex::new(BlockingState {
                released: false,
                wakers: Vec::new(),
            }),
            revoke_calls: AtomicUsize::new(0),
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

impl Oo7SecretServicePort for InspectBlockingPort {
    fn state(&self) -> SecretServiceFuture<'_, SecretServiceState> {
        Box::pin(async { Ok(SecretServiceState::Unlocked) })
    }

    fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant> {
        let expiry = request.requested_expiry_unix_ms();
        Box::pin(async move {
            Ok(SecretServiceLeaseGrant {
                lease_handle: CredentialLeaseHandle::parse("inspect-lease").unwrap(),
                source_version: CredentialSourceVersion::parse("inspect-source").unwrap(),
                rotation_generation: 1,
                expires_at_unix_ms: expiry,
            })
        })
    }

    fn inspect_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseInspection> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
        }
        let metadata = lease.metadata().clone();
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
            Ok(SecretServiceLeaseInspection {
                state: CredentialLeaseState::Active,
                source_version: metadata.source_version,
                rotation_generation: metadata.rotation_generation,
                expires_at_unix_ms: metadata.expires_at_unix_ms,
            })
        })
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
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(SecretServiceLeaseRevocation::Revoked) })
    }
}

fn provider_with_port(
    max_leases: u32,
    port: Arc<dyn Oo7SecretServicePort>,
) -> SecretServiceCredentialProvider {
    SecretServiceCredentialProviderFactory::new(
        SecretServiceConfig::new("login collection", max_leases, LockPolicy::FailClosed).unwrap(),
        SecretServicePlacement::new(
            ZoneId::parse("user-zone").unwrap(),
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
    .expect("test provider authority must be constructible")
}
