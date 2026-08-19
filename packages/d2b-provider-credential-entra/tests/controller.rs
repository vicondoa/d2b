mod common;

use d2b_contracts::v3::credential::CredentialServiceErrorCode;
use d2b_contracts::v3::{Locality, ResourceRef};
use d2b_provider_credential_entra::{
    EntraClientState, EntraController, EntraEndpointPolicy, EntraPlacement, EntraResourceHealth,
};

use common::{subject_context, subject_context_for, subject_context_with_bindings};

fn controller() -> (EntraController, EntraEndpointPolicy) {
    let placement = EntraPlacement::new_in_zone(
        ResourceRef::parse("Zone/work").unwrap(),
        d2b_contracts::v3::credential::PlacementBinding::GuestAgent,
        ResourceRef::parse("Guest/consumer").unwrap(),
        ResourceRef::parse("Guest/identity").unwrap(),
        ResourceRef::parse("Endpoint/entra-login").unwrap(),
        7,
    )
    .unwrap();
    let policy = EntraEndpointPolicy::new(
        "provider",
        ResourceRef::parse("Provider/credential-entra").unwrap(),
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceRef::parse("Guest/consumer").unwrap(),
    )
    .unwrap();
    (EntraController::new(placement), policy)
}

#[test]
fn status_projection_is_typed_redacted_and_locality_bound() {
    let (controller, policy) = controller();
    let projection = controller
        .project_for_subject(
            &policy,
            &subject_context(),
            EntraClientState::Ready,
            None,
            EntraResourceHealth::Degraded,
            2,
        )
        .unwrap();
    assert_eq!(projection.resource_health, EntraResourceHealth::Degraded);
    assert_eq!(projection.refresh_attempts, 2);
    assert_eq!(
        format!("{projection:?}"),
        "EntraStatusProjection(<redacted>)"
    );

    let relay = subject_context_for(
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceRef::parse("Zone/work").unwrap(),
        Locality::AdjacentZone,
    );
    assert_eq!(
        controller
            .project_for_subject(
                &policy,
                &relay,
                EntraClientState::Ready,
                None,
                EntraResourceHealth::Ready,
                0,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
}

#[test]
fn endpoint_policy_requires_guest_execution_and_exact_provider() {
    let (_, policy) = controller();
    for (label, execution_ref, provider_ref) in [
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
    ] {
        let subject = subject_context_with_bindings(
            ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            Locality::Local,
            execution_ref,
            provider_ref,
        );
        assert!(!policy.allows_authenticated_subject(&subject), "{label}");
    }
}

#[test]
fn status_projection_requires_the_committed_guest_execution() {
    let (controller, policy) = controller();
    let wrong_guest = subject_context_with_bindings(
        ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap(),
        ResourceRef::parse("Zone/work").unwrap(),
        Locality::Local,
        Some(ResourceRef::parse("Guest/other").unwrap()),
        Some(ResourceRef::parse("Provider/credential-entra").unwrap()),
    );
    assert_eq!(
        controller
            .project_for_subject(
                &policy,
                &wrong_guest,
                EntraClientState::Ready,
                None,
                EntraResourceHealth::Ready,
                0,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
}

#[test]
fn status_projection_rejects_retry_counts_above_the_provider_ceiling() {
    let (controller, policy) = controller();
    assert_eq!(
        controller
            .project_for_subject(
                &policy,
                &subject_context(),
                EntraClientState::Ready,
                None,
                EntraResourceHealth::Degraded,
                d2b_provider_credential_entra::MAX_REFRESH_ATTEMPTS + 1,
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
}
