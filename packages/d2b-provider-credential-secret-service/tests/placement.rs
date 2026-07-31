use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::PlacementBinding;
use d2b_provider_credential_secret_service::{SecretServicePlacement, SecretServiceProviderError};

#[test]
fn only_user_agent_on_host_or_guest_is_accepted() {
    let user = ResourceRef::parse("User/alice").unwrap();
    for execution in ["Host/workstation", "Guest/work-vm"] {
        assert!(
            SecretServicePlacement::new(
                PlacementBinding::UserAgent,
                ResourceRef::parse(execution).unwrap(),
                user.clone(),
            )
            .is_ok()
        );
    }
    for binding in [PlacementBinding::HostSystem, PlacementBinding::GuestAgent] {
        assert_eq!(
            SecretServicePlacement::new(
                binding,
                ResourceRef::parse("Guest/work-vm").unwrap(),
                user.clone(),
            ),
            Err(SecretServiceProviderError::InvalidPlacement)
        );
    }
}
