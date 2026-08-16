mod common;

use d2b_contracts::v3::credential::CredentialServiceErrorCode;
use d2b_contracts::v3::{Locality, ResourceRef};
use d2b_provider_credential_entra::{
    EntraClientState, EntraController, EntraEndpointPolicy, EntraPlacement, EntraResourceHealth,
};

use common::{subject_context, subject_context_for};

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
