use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::PlacementBinding;
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
