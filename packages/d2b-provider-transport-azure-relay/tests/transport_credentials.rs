use async_trait::async_trait;
use d2b_provider_transport_azure_relay::{
    RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial, RelayCredentialPort,
    RelayCredentialRole, RelaySecret,
};

struct FakeCredentials;

#[async_trait]
impl RelayCredentialPort for FakeCredentials {
    async fn acquire(
        &self,
        role: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"secret-token".to_vec()).unwrap()),
            role,
            10_000,
        ))
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        Ok(())
    }
}

#[test]
fn credential_debug_never_contains_bytes() {
    let lease = RelayCredentialLease::new(
        RelayCredentialMaterial::EntraBearer(RelaySecret::new(b"bearer-secret".to_vec()).unwrap()),
        RelayCredentialRole::Send,
        10,
    );
    assert!(!format!("{lease:?}").contains("bearer-secret"));
    assert!(
        !format!("{:?}", RelaySecret::new(b"secret-token".to_vec()).unwrap()).contains("secret")
    );
    let _ = FakeCredentials;
}
