use d2b_contracts_provider::v3::credential::PlacementBinding;
use d2b_contracts_zone_session::v3::ResourceRef;
use d2b_provider_credential_entra::{EntraPlacement, EntraProviderError};

#[test]
fn guest_user_and_system_domains_are_accepted_but_host_system_is_rejected() {
    for binding in [PlacementBinding::UserAgent, PlacementBinding::GuestAgent] {
        assert!(
            EntraPlacement::new(
                binding,
                ResourceRef::parse("Guest/consumer").unwrap(),
                ResourceRef::parse("Guest/identity").unwrap(),
                ResourceRef::parse("Endpoint/entra-login").unwrap(),
                1,
            )
            .is_ok()
        );
    }

    assert_eq!(
        EntraPlacement::new(
            PlacementBinding::HostSystem,
            ResourceRef::parse("Host/workstation").unwrap(),
            ResourceRef::parse("Guest/identity").unwrap(),
            ResourceRef::parse("Endpoint/entra-login").unwrap(),
            1,
        ),
        Err(EntraProviderError::InvalidPlacement)
    );
}

#[test]
fn zone_bound_placement_rejects_a_cross_zone_binding() {
    let placement = EntraPlacement::new_in_zone(
        ResourceRef::parse("Zone/work").unwrap(),
        PlacementBinding::GuestAgent,
        ResourceRef::parse("Guest/consumer").unwrap(),
        ResourceRef::parse("Guest/identity").unwrap(),
        ResourceRef::parse("Endpoint/entra-login").unwrap(),
        1,
    )
    .unwrap();

    assert_eq!(
        placement.validate_zone(&ResourceRef::parse("Zone/personal").unwrap()),
        Err(EntraProviderError::InvalidEndpoint)
    );
}

#[test]
fn unbound_placement_never_validates_against_a_zone() {
    let placement = EntraPlacement::new(
        PlacementBinding::GuestAgent,
        ResourceRef::parse("Guest/consumer").unwrap(),
        ResourceRef::parse("Guest/identity").unwrap(),
        ResourceRef::parse("Endpoint/entra-login").unwrap(),
        1,
    )
    .unwrap();

    assert_eq!(
        placement.validate_zone(&ResourceRef::parse("Zone/work").unwrap()),
        Err(EntraProviderError::InvalidEndpoint)
    );
}
