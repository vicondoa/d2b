mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use d2b_contracts_provider::v3::credential::{
    CredentialAuthorization, CredentialMethod, CredentialProvider, CredentialRequest,
    CredentialServiceErrorCode, PlacementBinding,
};
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ZoneId};
use d2b_provider_credential_secret_service::{
    LockPolicy, Oo7SecretServicePort, SecretServiceConfig, SecretServiceCredentialProvider,
    SecretServiceCredentialProviderFactory, SecretServiceFuture, SecretServiceLeaseGrant,
    SecretServiceLeaseInspection, SecretServiceLeaseRef, SecretServiceLeaseRenewal,
    SecretServiceLeaseRequest, SecretServiceLeaseRevocation, SecretServicePlacement,
    SecretServicePortError, SecretServiceState,
};

use common::{Admission, ProviderHarness, request, setup};

#[test]
fn locked_and_unavailable_map_to_provider_unavailable() {
    for failure in [
        SecretServicePortError::Locked,
        SecretServicePortError::Missing,
        SecretServicePortError::Unavailable,
    ] {
        let (provider, port) = setup(64);
        *port.issue_error.lock().unwrap() = Some(failure);
        let server = ProviderHarness::new(provider, Admission);
        assert_eq!(
            server
                .call(CredentialMethod::AcquireToken, request("idem-failure"))
                .unwrap_err()
                .code(),
            CredentialServiceErrorCode::ProviderUnavailable
        );
        assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn locked_state_is_checked_before_issuing_a_lease() {
    let (provider, port) = setup(64);
    *port.state.lock().unwrap() = SecretServiceState::Locked;
    let server = ProviderHarness::new(provider, Admission);

    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-locked-state"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn completion_unknown_is_not_replayed_with_the_same_idempotency_key() {
    let (provider, port) = setup(64);
    *port.issue_error.lock().unwrap() = Some(SecretServicePortError::CompletionUnknown);
    let server = ProviderHarness::new(provider, Admission);
    let request = request("idem-unknown");

    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request.clone())
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn remembered_ambiguous_acquire_consumes_capacity() {
    let (provider, port) = setup(1);
    let server = ProviderHarness::new(provider, Admission);
    *port.issue_error.lock().unwrap() = Some(SecretServicePortError::CompletionUnknown);
    assert_eq!(
        server
            .call(
                CredentialMethod::AcquireToken,
                request("idem-capacity-unknown")
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );

    *port.issue_error.lock().unwrap() = None;
    let other = CredentialRequest::new(
        ResourceRef::parse("Credential/other-keyring").unwrap(),
        "operation-capacity-other",
        "idem-capacity-other",
        common::EXPIRY,
        15_000,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, other)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn unknown_lease_record_consumes_capacity() {
    let (provider, port) = setup(1);
    let server = ProviderHarness::new(provider, Admission);
    *port.issue_rotation_generation.lock().unwrap() = 0;
    assert_eq!(
        server
            .call(
                CredentialMethod::AcquireToken,
                request("idem-capacity-record")
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );

    *port.issue_rotation_generation.lock().unwrap() = 1;
    let other = CredentialRequest::new(
        ResourceRef::parse("Credential/other-keyring").unwrap(),
        "operation-capacity-record-other",
        "idem-capacity-record-other",
        common::EXPIRY,
        15_000,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, other)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn ambiguous_refresh_uses_adapter_recovery_without_revoking_only_the_old_lease() {
    let (provider, port) = setup(64);
    let capability = Arc::new(
        provider
            .issue_session_capability(ResourceGeneration::new(1).unwrap())
            .unwrap(),
    );
    let acquire_authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(common::delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap()
    .with_shared_session_proof(capability.clone());
    let refresh_authorization = CredentialAuthorization::new(
        CredentialMethod::RefreshToken,
        Some(common::delivery(CredentialMethod::RefreshToken, 1)),
    )
    .unwrap()
    .with_shared_session_proof(capability);

    provider
        .dispatch(
            CredentialMethod::AcquireToken,
            &request("idem-refresh-recovery-acquire"),
            &acquire_authorization,
        )
        .unwrap();
    *port.refresh_error.lock().unwrap() = Some(SecretServicePortError::CompletionUnknown);
    assert_eq!(
        provider
            .dispatch(
                CredentialMethod::RefreshToken,
                &request("idem-refresh-recovery"),
                &refresh_authorization,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );

    provider.disconnect(&acquire_authorization).unwrap();
    assert_eq!(
        port.ambiguous_refresh_revoke_calls.load(Ordering::SeqCst),
        1
    );
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn disconnect_recovers_an_ambiguous_acquire_without_replaying_issue() {
    let (provider, port) = setup(64);
    *port.issue_error.lock().unwrap() = Some(SecretServicePortError::CompletionUnknown);
    let capability = provider
        .issue_session_capability(ResourceGeneration::new(1).unwrap())
        .unwrap();
    let authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(common::delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap()
    .with_session_proof(capability);

    assert_eq!(
        provider
            .dispatch(
                CredentialMethod::AcquireToken,
                &request("idem-ambiguous-disconnect"),
                &authorization,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    provider.disconnect(&authorization).unwrap();
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
    assert_eq!(port.ambiguous_revoke_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn unsupported_sign_challenge_never_calls_the_port() {
    let (provider, port) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    assert_eq!(
        server
            .call(CredentialMethod::SignChallenge, request("idem-sign"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::Malformed
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cardinality_is_enforced_before_a_second_port_call() {
    let (provider, port) = setup(1);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(CredentialMethod::AcquireToken, request("idem-first"))
        .unwrap();
    let other = CredentialRequest::new(
        d2b_contracts_resource::v3::ResourceRef::parse("Credential/other-keyring").unwrap(),
        "operation-2",
        "idem-second",
        common::EXPIRY,
        15_000,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, other)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn port_call_stops_at_request_deadline() {
    let port = Arc::new(NeverPort {
        issue_calls: AtomicUsize::new(0),
    });
    let provider = SecretServiceCredentialProviderFactory::new(
        SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).unwrap(),
        SecretServicePlacement::new(
            ZoneId::parse("user-zone").unwrap(),
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/workstation").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap(),
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port.clone(),
    )
    .unwrap()
    .construct()
    .expect("test provider authority must be constructible");
    let server = ProviderHarness::new(provider, Admission);
    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        result_tx
            .send(
                server.call(
                    CredentialMethod::AcquireToken,
                    CredentialRequest::new(
                        ResourceRef::parse("Credential/local-keyring").unwrap(),
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
            .expect("permanently pending port call ignored its request deadline")
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::DeadlineExceeded
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn deadline_fences_issue_retry() {
    let (server, port) = never_server();
    let first = CredentialRequest::new(
        ResourceRef::parse("Credential/local-keyring").unwrap(),
        "operation-deadline-first",
        "idem-deadline-first",
        common::EXPIRY,
        10,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, first)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::DeadlineExceeded
    );
    assert_eq!(
        server
            .call(
                CredentialMethod::AcquireToken,
                request("idem-deadline-second")
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn late_unlock_does_not_start_or_fence_issue() {
    let (server, port) = delayed_unlock_server(Duration::from_millis(40));
    let first = CredentialRequest::new(
        ResourceRef::parse("Credential/local-keyring").unwrap(),
        "operation-late-unlock-first",
        "idem-late-unlock-first",
        common::EXPIRY,
        10,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, first)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::DeadlineExceeded
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 0);

    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-late-unlock-second"),
        )
        .unwrap();
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}

struct DelayedUnlockPort {
    issue_calls: AtomicUsize,
    ready_at: std::time::Instant,
}

fn delayed_unlock_server(
    delay: Duration,
) -> (
    ProviderHarness<SecretServiceCredentialProvider, Admission>,
    Arc<DelayedUnlockPort>,
) {
    let port = Arc::new(DelayedUnlockPort {
        issue_calls: AtomicUsize::new(0),
        ready_at: std::time::Instant::now() + delay,
    });
    let provider = SecretServiceCredentialProviderFactory::new(
        SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).unwrap(),
        SecretServicePlacement::new(
            ZoneId::parse("user-zone").unwrap(),
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/workstation").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap(),
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port.clone(),
    )
    .unwrap()
    .construct()
    .expect("test provider authority must be constructible");
    (ProviderHarness::new(provider, Admission), port)
}

impl Oo7SecretServicePort for DelayedUnlockPort {
    fn state(&self) -> SecretServiceFuture<'_, SecretServiceState> {
        let ready_at = self.ready_at;
        Box::pin(std::future::poll_fn(move |context| {
            if std::time::Instant::now() >= ready_at {
                std::task::Poll::Ready(Ok(SecretServiceState::Unlocked))
            } else {
                context.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        }))
    }

    fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        let expiry = request.requested_expiry_unix_ms();
        Box::pin(async move {
            Ok(SecretServiceLeaseGrant {
                lease_handle: d2b_contracts_provider::v3::credential::CredentialLeaseHandle::parse(
                    "secret-service-lease",
                )
                .unwrap(),
                source_version:
                    d2b_contracts_provider::v3::credential::CredentialSourceVersion::parse(
                        "secret-service-source",
                    )
                    .unwrap(),
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

struct NeverPort {
    issue_calls: AtomicUsize,
}

fn never_server() -> (
    ProviderHarness<SecretServiceCredentialProvider, Admission>,
    Arc<NeverPort>,
) {
    let port = Arc::new(NeverPort {
        issue_calls: AtomicUsize::new(0),
    });
    let provider = SecretServiceCredentialProviderFactory::new(
        SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).unwrap(),
        SecretServicePlacement::new(
            ZoneId::parse("user-zone").unwrap(),
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/workstation").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap(),
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port.clone(),
    )
    .unwrap()
    .construct()
    .expect("test provider authority must be constructible");
    (ProviderHarness::new(provider, Admission), port)
}

impl Oo7SecretServicePort for NeverPort {
    fn state(&self) -> SecretServiceFuture<'_, SecretServiceState> {
        Box::pin(async { Ok(SecretServiceState::Unlocked) })
    }

    fn issue_lease(
        &self,
        _request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::pending())
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
