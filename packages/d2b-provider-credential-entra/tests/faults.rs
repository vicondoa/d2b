mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use d2b_contracts_zone_session::v3::Locality;
use d2b_contracts_zone_session::v3::ResourceRef;
use d2b_contracts_provider::v3::credential::{
    CredentialAuthorization, CredentialMethod, CredentialRequest, CredentialResponse,
    CredentialServiceErrorCode, CredentialSessionBinding, PlacementBinding,
    dispatch_authorized_provider,
};
use d2b_provider_credential_entra::{
    EntraClientError, EntraClientState, EntraConfig, EntraCredentialClient,
    EntraCredentialProviderFactory, EntraFuture, EntraLeaseGrant, EntraLeaseInspection,
    EntraLeaseRef, EntraLeaseRenewal, EntraLeaseRequest, EntraLeaseRevocation, EntraPlacement,
};

use common::{
    Admission, ProviderHarness, admitted, delivery, delivery_with_component_generation, request,
    session_binding, setup, subject_context, subject_context_for, subject_context_with_bindings,
};

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
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
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
fn missing_authenticated_session_is_denied_before_client_dispatch() {
    let (provider, client) = setup();
    let authorization = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
        subject_context(),
    )
    .unwrap();

    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &request("idem-missing-session"),
            &authorization,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn request_and_session_delivery_bindings_must_match_exactly() {
    let (provider, client) = setup();
    let authorization = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
        subject_context(),
    )
    .unwrap()
    .with_authenticated_session(session_binding())
    .unwrap();
    let other_request = CredentialRequest::new(
        ResourceRef::parse("Credential/other-entra").unwrap(),
        "operation-other",
        "idem-other-binding",
        common::EXPIRY,
        15_000,
    )
    .unwrap();

    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &other_request,
            &authorization,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn provider_generation_must_match_the_delivery_component_generation() {
    let (provider, client) = setup();
    let authorization = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery_with_component_generation(
            CredentialMethod::AcquireToken,
            1,
            2,
        )),
        subject_context(),
    )
    .unwrap()
    .with_authenticated_session(session_binding())
    .unwrap();

    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &request("idem-generation-binding"),
            &authorization,
        )
        .unwrap_err()
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn authenticated_session_subject_must_match_the_authorized_subject() {
    let (provider, client) = setup();
    let authorization = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
        subject_context(),
    )
    .unwrap()
    .with_authenticated_session(
        CredentialSessionBinding::new(
            subject_context_for(
                ResourceRef::parse("Provider/other").unwrap(),
                ResourceRef::parse("Zone/work").unwrap(),
                Locality::Local,
            ),
            common::EXPIRY,
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &request("idem-session-subject"),
            &authorization,
        )
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
fn missing_or_mismatched_authenticated_claims_are_denied_before_client_dispatch() {
    let cases = [
        (
            "missing execution",
            None,
            Some(ResourceRef::parse("Provider/credential-entra").unwrap()),
        ),
        (
            "host execution",
            Some(ResourceRef::parse("Host/workstation").unwrap()),
            Some(ResourceRef::parse("Provider/credential-entra").unwrap()),
        ),
        (
            "wrong Guest execution",
            Some(ResourceRef::parse("Guest/other").unwrap()),
            Some(ResourceRef::parse("Provider/credential-entra").unwrap()),
        ),
        (
            "missing provider",
            Some(ResourceRef::parse("Guest/consumer").unwrap()),
            None,
        ),
        (
            "wrong provider",
            Some(ResourceRef::parse("Guest/consumer").unwrap()),
            Some(ResourceRef::parse("Provider/other").unwrap()),
        ),
    ];

    for (index, (label, execution_ref, provider_ref)) in cases.into_iter().enumerate() {
        let (provider, client) = setup();
        let authenticated_subject = subject_context_with_bindings(
            ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            Locality::Local,
            execution_ref,
            provider_ref,
        );
        let authorization = CredentialAuthorization::new_for_subject(
            CredentialMethod::AcquireToken,
            Some(delivery(CredentialMethod::AcquireToken, 1)),
            authenticated_subject.clone(),
        )
        .unwrap();
        let authorization = authorization
            .with_authenticated_session(
                CredentialSessionBinding::new(authenticated_subject, common::EXPIRY).unwrap(),
            )
            .unwrap();

        assert_eq!(
            dispatch_authorized_provider(
                &provider,
                CredentialMethod::AcquireToken,
                &request(&format!("idem-claims-{index}")),
                &authorization,
            )
            .unwrap_err()
            .code(),
            CredentialServiceErrorCode::OperationDenied,
            "{label}"
        );
        assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0, "{label}");
    }
}

#[test]
fn same_credential_name_from_another_zone_is_denied_before_client_dispatch() {
    let (provider, client) = setup();
    let authenticated_subject = subject_context_with_bindings(
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceRef::parse("Zone/personal").unwrap(),
        Locality::Local,
        Some(ResourceRef::parse("Guest/consumer").unwrap()),
        Some(ResourceRef::parse("Provider/credential-entra").unwrap()),
    );
    let authorization = CredentialAuthorization::new_for_subject(
        CredentialMethod::AcquireToken,
        Some(delivery(CredentialMethod::AcquireToken, 1)),
        authenticated_subject.clone(),
    )
    .unwrap();
    let authorization = authorization
        .with_authenticated_session(
            CredentialSessionBinding::new(authenticated_subject, common::EXPIRY).unwrap(),
        )
        .unwrap();

    assert_eq!(
        dispatch_authorized_provider(
            &provider,
            CredentialMethod::AcquireToken,
            &request("idem-cross-zone"),
            &authorization,
        )
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
fn committed_remote_refresh_metadata_is_adopted_for_later_recovery() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-refresh-adopt"),
        )
        .unwrap();

    let too_short = CredentialRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        "operation-refresh-adopt",
        "idem-refresh-too-short",
        10_000,
        5_000,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::RefreshToken, too_short)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(client.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.refresh_retry_state(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some((0, d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS))
    );

    let inspected = server
        .call(
            CredentialMethod::InspectMetadata,
            request("idem-inspect-adopted"),
        )
        .unwrap();
    let inspected_metadata = match inspected {
        CredentialResponse::InspectMetadata(response) => response.metadata,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(inspected_metadata.rotation_generation, 2);

    let refreshed = server
        .call(
            CredentialMethod::RefreshToken,
            request("idem-refresh-after-adopt"),
        )
        .unwrap();
    let refreshed_metadata = match refreshed {
        CredentialResponse::RefreshToken(response) => response.metadata,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(refreshed_metadata.rotation_generation, 2);
    assert_eq!(
        provider.resource_health(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some(d2b_provider_credential_entra::EntraResourceHealth::Ready)
    );
    assert_eq!(
        provider.refresh_retry_state(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some((0, d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS))
    );
    assert_eq!(client.inspect_calls.load(Ordering::SeqCst), 3);
    assert_eq!(client.refresh_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn refresh_rejects_remote_generation_rollback() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-generation-rollback"),
        )
        .unwrap();
    server
        .call(
            CredentialMethod::RefreshToken,
            request("idem-generation-rollback-first-refresh"),
        )
        .unwrap();
    *client.refresh_generation.lock().unwrap() = 1;

    assert_eq!(
        server
            .call(
                CredentialMethod::RefreshToken,
                request("idem-generation-rollback-refresh"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(
        provider.resource_health(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some(d2b_provider_credential_entra::EntraResourceHealth::Degraded)
    );
}

#[test]
fn inspect_persists_remote_revocation_and_degrades_only_that_resource() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-inspect-revoked"),
        )
        .unwrap();
    *client.inspection.lock().unwrap() = Some(EntraLeaseInspection {
        state: d2b_contracts_provider::v3::credential::CredentialLeaseState::Revoked,
        source_version: d2b_contracts_provider::v3::credential::CredentialSourceVersion::parse(
            "entra-source-revoked",
        )
        .unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });

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
    assert_eq!(
        provider.resource_health(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some(d2b_provider_credential_entra::EntraResourceHealth::Revoked)
    );
}

#[test]
fn unknown_inspection_is_transient_and_acquire_revokes_before_replacement() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(CredentialMethod::AcquireToken, request("idem-unknown-base"))
        .unwrap();
    *client.inspection.lock().unwrap() = Some(EntraLeaseInspection {
        state: d2b_contracts_provider::v3::credential::CredentialLeaseState::Unknown,
        source_version: d2b_contracts_provider::v3::credential::CredentialSourceVersion::parse(
            "entra-source-unknown",
        )
        .unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: common::EXPIRY,
    });

    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-unknown-inspect"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    assert_eq!(
        provider.refresh_retry_state(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some((0, d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS))
    );
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-unknown-replacement"),
        )
        .unwrap();
    assert_eq!(client.revoke_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn clock_expired_inspection_is_reclaimed_before_acquire_replacement() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(CredentialMethod::AcquireToken, request("idem-expired-base"))
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    *client.inspection.lock().unwrap() = Some(EntraLeaseInspection {
        state: d2b_contracts_provider::v3::credential::CredentialLeaseState::Active,
        source_version: d2b_contracts_provider::v3::credential::CredentialSourceVersion::parse(
            "entra-source-expired",
        )
        .unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: now - 1,
    });

    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-expired-inspect"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseExpired
    );
    assert_eq!(
        provider.resource_health(&ResourceRef::parse("Credential/work-entra").unwrap()),
        Some(d2b_provider_credential_entra::EntraResourceHealth::Degraded)
    );
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-expired-replacement"),
        )
        .unwrap();
    assert_eq!(client.revoke_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn expired_remote_revoke_is_idempotent_for_explicit_revoke() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-expired-revoke"),
        )
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    *client.inspection.lock().unwrap() = Some(EntraLeaseInspection {
        state: d2b_contracts_provider::v3::credential::CredentialLeaseState::Active,
        source_version: d2b_contracts_provider::v3::credential::CredentialSourceVersion::parse(
            "entra-source-expired-revoke",
        )
        .unwrap(),
        rotation_generation: 1,
        expires_at_unix_ms: now - 1,
    });
    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-expired-revoke-inspect"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseExpired
    );
    *client.revoke_error.lock().unwrap() = Some(EntraClientError::LeaseExpired);

    let response = server
        .call(
            CredentialMethod::RevokeToken,
            request("idem-expired-revoke-finalize"),
        )
        .unwrap();
    let CredentialResponse::RevokeToken(response) = response else {
        panic!("expected revoke response");
    };
    assert_eq!(
        response.metadata.state,
        d2b_contracts_provider::v3::credential::CredentialLeaseState::Revoked
    );
    assert_eq!(client.revoke_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn local_revocation_is_terminal_even_when_remote_inspection_is_stale() {
    let (provider, client) = setup();
    let server = ProviderHarness::new(&provider, admitted());
    server
        .call(
            CredentialMethod::AcquireToken,
            request("idem-local-revoked"),
        )
        .unwrap();
    server
        .call(
            CredentialMethod::RevokeToken,
            request("idem-local-revoked-revoke"),
        )
        .unwrap();
    let inspect_calls = client.inspect_calls.load(Ordering::SeqCst);
    let refresh_calls = client.refresh_calls.load(Ordering::SeqCst);

    assert_eq!(
        server
            .call(
                CredentialMethod::InspectMetadata,
                request("idem-local-revoked-inspect"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseRevoked
    );
    assert_eq!(
        server
            .call(
                CredentialMethod::RefreshToken,
                request("idem-local-revoked-refresh"),
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::LeaseRevoked
    );
    assert_eq!(client.inspect_calls.load(Ordering::SeqCst), inspect_calls);
    assert_eq!(client.refresh_calls.load(Ordering::SeqCst), refresh_calls);
}

#[test]
fn client_call_stops_at_request_deadline() {
    let client = Arc::new(NeverClient {
        issue_calls: AtomicUsize::new(0),
    });
    let provider = EntraCredentialProviderFactory::new(
        EntraConfig::new("tenant-1234", 64).unwrap(),
        EntraPlacement::new_in_zone(
            ResourceRef::parse("Zone/work").unwrap(),
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

#[test]
fn absolute_request_bounds_are_ordered_against_legacy_relative_session_values() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let (provider, client) = setup();
    let server = ProviderHarness::new(provider, admitted());
    let request = CredentialRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        "operation-absolute",
        "idem-absolute",
        now + 10_000,
        now + 5_000,
    )
    .unwrap();

    server
        .call(CredentialMethod::AcquireToken, request)
        .unwrap();
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
