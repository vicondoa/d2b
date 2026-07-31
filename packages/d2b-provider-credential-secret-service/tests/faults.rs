mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialMethod, CredentialRequest, CredentialServiceErrorCode, PlacementBinding,
};
use d2b_provider_credential_secret_service::{
    LockPolicy, Oo7SecretServicePort, SecretServiceConfig, SecretServiceCredentialProviderFactory,
    SecretServiceFuture, SecretServiceLeaseGrant, SecretServiceLeaseInspection,
    SecretServiceLeaseRef, SecretServiceLeaseRenewal, SecretServiceLeaseRequest,
    SecretServiceLeaseRevocation, SecretServicePlacement, SecretServicePortError,
    SecretServiceState,
};

use common::{Admission, ProviderHarness, request, setup};

#[test]
fn locked_and_unavailable_map_to_provider_unavailable() {
    for failure in [
        SecretServicePortError::Locked,
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
        d2b_contracts::v3::ResourceRef::parse("Credential/other-keyring").unwrap(),
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
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/workstation").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap(),
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port.clone(),
    )
    .unwrap()
    .construct();
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

struct NeverPort {
    issue_calls: AtomicUsize,
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
