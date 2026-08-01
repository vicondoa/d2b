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
        )
        .is_ok()
    );
    assert!(
        ManagedIdentityPlacement::new(
            PlacementBinding::GuestAgent,
            ResourceRef::parse("Guest/aca-sandbox").unwrap(),
        )
        .is_ok()
    );
    assert_eq!(
        ManagedIdentityPlacement::new(
            PlacementBinding::UserAgent,
            ResourceRef::parse("Host/azure-vm").unwrap(),
        ),
        Err(ManagedIdentityProviderError::InvalidPlacement)
    );
}
