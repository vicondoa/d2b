use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::PlacementBinding;
use d2b_provider_credential_managed_identity::{
    ManagedIdentityPlacement, ManagedIdentityProviderError,
};

#[test]
fn machine_placements_are_accepted_and_user_agent_is_rejected() {
    assert!(
        ManagedIdentityPlacement::new(
            PlacementBinding::HostSystem,
            ResourceRef::parse("Host/azure-vm").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
        )
        .is_ok()
    );
    assert!(
        ManagedIdentityPlacement::new(
            PlacementBinding::GuestAgent,
            ResourceRef::parse("Guest/aca-sandbox").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
        )
        .is_ok()
    );
    assert_eq!(
        ManagedIdentityPlacement::new(
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/azure-vm").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
        ),
        Err(ManagedIdentityProviderError::InvalidPlacement)
    );
}

#[test]
fn unbound_or_non_zone_placements_are_rejected() {
    assert_eq!(
        ManagedIdentityPlacement::new(
            PlacementBinding::GuestAgent,
            ResourceRef::parse("Guest/aca-sandbox").unwrap(),
            ResourceRef::parse("Host/workstation").unwrap(),
        ),
        Err(ManagedIdentityProviderError::InvalidPlacement)
    );
}
