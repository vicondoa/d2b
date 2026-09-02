use async_trait::async_trait;
use d2b_contracts::ResourceRef;
use d2b_provider_transport_azure_relay::{
    AzureRelaySocketConnector, RelayCredentialBinding, RelayCredentialError, RelayCredentialLease,
    RelayCredentialMaterial, RelayCredentialPort, RelayCredentialRole, RelaySecret,
    RelayTransportConfig,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
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

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"secret-token".to_vec()).unwrap()),
            role,
            10_000,
            binding.clone(),
        )
        .unwrap())
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

#[test]
fn lease_binding_is_exact_and_redacted() {
    let binding = RelayCredentialBinding::new("zonelink-canary", "session-canary", 7).unwrap();
    let lease = RelayCredentialLease::new_bound(
        RelayCredentialMaterial::SasToken(RelaySecret::new(b"token-canary".to_vec()).unwrap()),
        RelayCredentialRole::Send,
        10_000,
        binding.clone(),
    )
    .unwrap();

    assert_eq!(lease.binding(), Some(&binding));
    assert_eq!(lease.reconnect_generation(), 7);
    let debug = format!("{lease:?}");
    assert!(!debug.contains("token-canary"));
    assert!(!debug.contains("zonelink-canary"));
    assert!(!debug.contains("session-canary"));
}

#[test]
fn request_debug_redacts_binding_canaries() {
    let binding = RelayCredentialBinding::new("link-canary", "session-canary", 7).unwrap();
    let request = d2b_provider_transport_azure_relay::RelayCredentialRequest::new(
        RelayCredentialRole::Listen,
        binding,
        500,
    );
    let debug = format!("{request:?}");
    assert!(!debug.contains("link-canary"));
    assert!(!debug.contains("session-canary"));
}

#[test]
fn unbound_port_lease_can_be_bound_only_once() {
    let binding = RelayCredentialBinding::new("zonelink-a", "session-a", 1).unwrap();
    let other = RelayCredentialBinding::new("zonelink-b", "session-b", 2).unwrap();
    let lease = RelayCredentialLease::new(
        RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
        RelayCredentialRole::Send,
        10_000,
    );
    let lease = lease.bind(binding.clone()).unwrap();
    assert_eq!(lease.binding(), Some(&binding));
    assert!(matches!(
        lease.bind(other),
        Err(RelayCredentialError::AlreadyBound)
    ));
}

#[test]
fn credential_binding_rejects_zero_generation_and_secret_shaped_ids() {
    assert_eq!(
        RelayCredentialBinding::new("zonelink", "session", 0),
        Err(RelayCredentialError::InvalidBinding)
    );
    assert_eq!(
        RelayCredentialBinding::new("zonelink", "SharedAccessSignature secret", 1),
        Err(RelayCredentialError::InvalidBinding)
    );
}

#[test]
fn connector_debug_does_not_materialize_guest_ca_bytes() {
    let connector =
        AzureRelaySocketConnector::new().with_ca_pem(Some(b"ca-secret-canary".to_vec()));
    let debug = format!("{connector:?}");
    assert!(debug.contains("configured"));
    assert!(!debug.contains("ca-secret-canary"));
}

#[test]
fn provider_config_debug_redacts_guest_and_network_refs() {
    let config = RelayTransportConfig {
        execution_ref: ResourceRef::parse("Guest/credential-canary").unwrap(),
        network_ref: ResourceRef::parse("Network/network-canary").unwrap(),
        max_concurrent_sessions: 1,
        connect_timeout_seconds: 5,
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("credential-canary"));
    assert!(!debug.contains("network-canary"));
}

#[test]
fn dropped_lease_runs_bounded_row_cleanup_hook() {
    let cleaned = Arc::new(AtomicUsize::new(0));
    let mut lease = RelayCredentialLease::new(
        RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
        RelayCredentialRole::Send,
        1_000,
    );
    let cleaned_for_drop = Arc::clone(&cleaned);
    lease.set_drop_hook(Arc::new(move |_| {
        cleaned_for_drop.fetch_add(1, Ordering::SeqCst);
    }));
    drop(lease);
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
}
