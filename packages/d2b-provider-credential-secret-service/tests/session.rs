mod common;

use d2b_contracts::v3::credential::{
    CredentialAuthorization, CredentialMethod, CredentialProvider, CredentialServiceErrorCode,
    CredentialSessionAuthority, PlacementBinding, dispatch_authorized_provider,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ZoneId};
use d2b_provider_credential_secret_service::{
    LockPolicy, SecretServiceConfig, SecretServiceCredentialProvider,
    SecretServiceCredentialProviderFactory, SecretServicePlacement,
};

use common::{FakeOo7Port, delivery, request, setup};

#[test]
fn unauthenticated_authorization_cannot_reach_the_port() {
    let (provider, port) = setup(64);
    let authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap();
    assert_eq!(
        provider
            .dispatch(
                CredentialMethod::AcquireToken,
                &request("unauthenticated"),
                &authorization,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(
        port.issue_calls.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[test]
fn one_session_capability_rejects_clone_replay_and_disconnects_owned_lease() {
    let (provider, port) = setup(64);
    let capability = provider
        .issue_session_capability(ResourceGeneration::new(1).unwrap())
        .unwrap();
    let cloned_capability = capability.clone();
    let authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap()
    .with_session_capability(capability);
    dispatch_authorized_provider(
        &provider,
        CredentialMethod::AcquireToken,
        &request("session-owned"),
        &authorization,
    )
    .unwrap();
    dispatch_authorized_provider(
        &provider,
        CredentialMethod::AcquireToken,
        &d2b_contracts::v3::credential::CredentialRequest::new(
            ResourceRef::parse("Credential/other-keyring").unwrap(),
            "operation-2",
            "session-owned-2",
            common::EXPIRY,
            15_000,
        )
        .unwrap(),
        &authorization,
    )
    .unwrap();

    let cloned_authorization = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 2)),
    )
    .unwrap()
    .with_session_capability(cloned_capability);
    assert_eq!(
        provider
            .dispatch(
                CredentialMethod::AcquireToken,
                &request("clone-replay"),
                &cloned_authorization,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    provider.disconnect(&authorization).unwrap();
    assert_eq!(
        port.revoke_calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(
        provider
            .dispatch(
                CredentialMethod::InspectMetadata,
                &request("after-disconnect"),
                &authorization,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
}

#[test]
fn wrong_zone_and_workload_capabilities_are_refused() {
    let (provider, _) = setup(64);
    let authority = CredentialSessionAuthority::new();
    for (zone, workload, consumer, subject) in [
        (
            "other-zone",
            "Host/workstation",
            "Provider/shell-terminal",
            "User/alice",
        ),
        (
            "user-zone",
            "Host/other-workstation",
            "Provider/shell-terminal",
            "User/alice",
        ),
        (
            "user-zone",
            "Host/workstation",
            "Provider/other-consumer",
            "User/alice",
        ),
        (
            "user-zone",
            "Host/workstation",
            "Provider/shell-terminal",
            "User/bob",
        ),
    ] {
        let capability = authority
            .issue(
                ZoneId::parse(zone).unwrap(),
                ResourceRef::parse(workload).unwrap(),
                ResourceRef::parse(subject).unwrap(),
                ResourceRef::parse(consumer).unwrap(),
                ResourceGeneration::new(1).unwrap(),
            )
            .unwrap();
        let authorization = CredentialAuthorization::new(
            CredentialMethod::AcquireToken,
            Some(delivery(CredentialMethod::AcquireToken, 1)),
        )
        .unwrap()
        .with_session_capability(capability);
        assert_eq!(
            provider
                .dispatch(
                    CredentialMethod::AcquireToken,
                    &request("wrong-binding"),
                    &authorization,
                )
                .unwrap_err()
                .code(),
            CredentialServiceErrorCode::OperationDenied
        );
    }
}

#[test]
fn disconnect_revokes_only_the_owned_workload_leases() {
    let port = std::sync::Arc::new(FakeOo7Port::new());
    let first = provider_for(
        ZoneId::parse("user-zone").unwrap(),
        ResourceRef::parse("Host/workstation").unwrap(),
        port.clone(),
    );
    let second = provider_for(
        ZoneId::parse("other-zone").unwrap(),
        ResourceRef::parse("Host/other-workstation").unwrap(),
        port.clone(),
    );
    let first_capability = std::sync::Arc::new(
        first
            .issue_session_capability(ResourceGeneration::new(1).unwrap())
            .unwrap(),
    );
    let second_capability = std::sync::Arc::new(
        second
            .issue_session_capability(ResourceGeneration::new(1).unwrap())
            .unwrap(),
    );
    let first_auth = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap()
    .with_shared_session_capability(first_capability.clone());
    let second_auth = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
    )
    .unwrap()
    .with_shared_session_capability(second_capability.clone());

    dispatch_authorized_provider(
        &first,
        CredentialMethod::AcquireToken,
        &request("first-lease"),
        &first_auth,
    )
    .unwrap();
    dispatch_authorized_provider(
        &second,
        CredentialMethod::AcquireToken,
        &request("second-lease"),
        &second_auth,
    )
    .unwrap();

    first.disconnect(&first_auth).unwrap();
    let second_inspect_auth = CredentialAuthorization::new(CredentialMethod::InspectMetadata, None)
        .unwrap()
        .with_shared_session_capability(second_capability);
    dispatch_authorized_provider(
        &second,
        CredentialMethod::InspectMetadata,
        &request("second-inspect"),
        &second_inspect_auth,
    )
    .unwrap();
    assert_eq!(
        port.revoke_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    second.finalize_session(&second_auth).unwrap();
    assert_eq!(
        port.revoke_calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

fn provider_for(
    zone: ZoneId,
    workload: ResourceRef,
    port: std::sync::Arc<FakeOo7Port>,
) -> SecretServiceCredentialProvider {
    SecretServiceCredentialProviderFactory::new(
        SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).unwrap(),
        SecretServicePlacement::new(
            zone,
            PlacementBinding::UserAgent,
            workload,
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap(),
        Some(ResourceRef::parse("Provider/shell-terminal").unwrap()),
        port,
    )
    .unwrap()
    .construct()
}
