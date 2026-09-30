use async_trait::async_trait;
use d2b_contracts::ResourceRef;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingSlot, BoundedToken, CredentialBindingRequest, CredentialLifetime,
    CredentialOperation, DesiredRevision, RefusalReason, ResourceGeneration, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};
use d2b_provider_transport_azure_relay::{
    AdmittedRelayDelivery, AdmittedTransportBinding, AzureRelaySocketConnector,
    AzureRelayTransportProvider, CredentialError, GatewayTransportConfiguration,
    GraphBoundRelayError, RELAY_CREDENTIAL_AUDIENCE, RelayConnection, RelayCredentialBinding,
    RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial, RelayCredentialPort,
    RelayCredentialRole, RelayEndpoint, RelayFrame, RelayRole, RelaySecret, RelaySocket,
    RelaySocketConnector, RelayTransportConfig, RelayTransportError, RelayTransportSettings,
    RelationshipFence, RelationshipPhase, ScopedCredentialClient, ScopedCredentialRequest,
    TransportAttachEvidence, auth::RELAY_TOKEN_RESOURCE,
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

/// The existing fake, wrapped so a refusal can be counted instead of
/// observed through its side effects.
struct ReadCountingCredentials {
    reads: Arc<AtomicUsize>,
}

impl ReadCountingCredentials {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ScopedCredentialClient for ReadCountingCredentials {
    async fn read_credential(
        &self,
        request: &ScopedCredentialRequest,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        RelayCredentialPort::acquire_bound(
            &FakeCredentials,
            request.role(),
            request.binding(),
            request.deadline_ms(),
        )
        .await
    }

    async fn revoke_credential(
        &self,
        lease: RelayCredentialLease,
    ) -> Result<(), RelayCredentialError> {
        RelayCredentialPort::revoke(&FakeCredentials, lease).await
    }
}

fn live_expiry() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
        + 60_000
}

/// A lease with a live expiry.
///
/// `FakeCredentials` pins its lease to a fixed past instant, which is right
/// for a redaction test and wrong for a carriage that has to actually open,
/// so the one test that opens for real uses this instead.
struct LiveLeaseCredentials {
    reads: Arc<AtomicUsize>,
}

impl LiveLeaseCredentials {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// Take the refusal out of a carriage open.
///
/// `RelayConnection` has no `Debug`, and it is deliberately kept that way: a
/// carriage carries a live socket and a credential lease, and neither belongs
/// in a panic message.
fn refusal_of(
    opened: Result<RelayConnection, GraphBoundRelayError>,
    context: &str,
) -> GraphBoundRelayError {
    match opened {
        Ok(_) => panic!("{context}"),
        Err(error) => error,
    }
}

#[async_trait]
impl ScopedCredentialClient for LiveLeaseCredentials {
    async fn read_credential(
        &self,
        request: &ScopedCredentialRequest,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(b"secret-token".to_vec()).unwrap(),
            ),
            request.role(),
            live_expiry(),
            request.binding().clone(),
        )
        .map_err(|_| RelayCredentialError::InvalidBinding)
    }

    async fn revoke_credential(
        &self,
        _: RelayCredentialLease,
    ) -> Result<(), RelayCredentialError> {
        Ok(())
    }
}

struct NoopSocket;

#[async_trait]
impl RelaySocket for NoopSocket {
    async fn send(&self, _: RelayFrame) -> Result<(), RelayTransportError> {
        Ok(())
    }

    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        Ok(None)
    }

    async fn close(&self) -> Result<(), RelayTransportError> {
        Ok(())
    }
}

struct NoopConnector;

#[async_trait]
impl RelaySocketConnector for NoopConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        Ok(Arc::new(NoopSocket))
    }
}

const CREDENTIAL_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const CONSUMER_UID: &str = "ffffffff-ffff-4fff-bfff-ffffffffffff";

fn work_zone() -> ZoneId {
    ZoneId::parse("work").unwrap()
}

fn store_one() -> StoreIncarnation {
    StoreIncarnation::parse("store-1").unwrap()
}

fn admitted_fence() -> RelationshipFence {
    RelationshipFence::new(
        store_one(),
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        ResourceGeneration::new(3).unwrap(),
        ResourceGeneration::new(4).unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    )
}

fn evidence_for(
    zone: ZoneId,
    source: u64,
    consumer: u64,
    revision: DesiredRevision,
    sequence: ZoneDesiredSequence,
    reconnect: u64,
) -> TransportAttachEvidence {
    TransportAttachEvidence::new(
        zone,
        store_one(),
        ResourceGeneration::new(source).unwrap(),
        ResourceGeneration::new(consumer).unwrap(),
        revision,
        sequence,
        ReconnectGeneration::new(reconnect).unwrap(),
    )
}

fn current_evidence() -> TransportAttachEvidence {
    evidence_for(
        work_zone(),
        3,
        4,
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        2,
    )
}

/// One admitted credential relationship.
///
/// `name` and `slot` exist so two cases in one test are two distinct
/// relationships: a `BindingKey` is derived from the source reference, both
/// identities, and the slot, so cases that share all four would collide in
/// the registry and would silently be testing the first one.
fn admitted_credential(
    name: &str,
    slot: &str,
    audience: &str,
    operations: Vec<CredentialOperation>,
) -> AdmittedTransportBinding {
    let request = CredentialBindingRequest::new(
        ResourceRef::parse(&format!("Credential/{name}")).unwrap(),
        ResourceRef::parse("Guest/gateway").unwrap(),
        BindingSlot::parse(slot).unwrap(),
        BoundedToken::parse(audience).unwrap(),
        operations,
        CredentialLifetime::new("600s", "600s").unwrap(),
    )
    .unwrap();
    let key = request
        .key(
            work_zone(),
            ResourceUid::parse(CREDENTIAL_UID).unwrap(),
            ResourceUid::parse(CONSUMER_UID).unwrap(),
        )
        .unwrap();
    AdmittedTransportBinding::new(key, request, admitted_fence())
}

fn relay_credential(
    name: &str,
    audience: &str,
    operations: Vec<CredentialOperation>,
) -> AdmittedTransportBinding {
    admitted_credential(name, "relay-egress", audience, operations)
}

fn scoped_request(generation: u64) -> ScopedCredentialRequest {
    ScopedCredentialRequest::new(
        work_zone(),
        ResourceRef::parse("Credential/relay-send").unwrap(),
        ResourceRef::parse("Guest/gateway").unwrap(),
        RelayCredentialRole::Send,
        RelayCredentialBinding::new_scoped(work_zone(), "link-test", "session-test", generation)
            .unwrap(),
        1_000,
    )
    .unwrap()
}

fn relay_endpoint() -> RelayEndpoint {
    RelayEndpoint {
        settings: RelayTransportSettings::new("relns-d2b-prod", "hc-d2b-k2").unwrap(),
    }
}

fn relay_config() -> RelayTransportConfig {
    RelayTransportConfig {
        execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
        network_ref: ResourceRef::parse("Network/relay").unwrap(),
        max_concurrent_sessions: 4,
        connect_timeout_seconds: 30,
    }
}

fn assert_attach_refusal(
    error: GraphBoundRelayError,
    code: &str,
    stage: AdmissionStage,
    reason: RefusalReason,
) {
    let GraphBoundRelayError::AttachRefused(refusal) = error else {
        panic!("expected a graph attach refusal, got {error}");
    };
    assert_eq!(refusal.code(), code);
    assert_eq!(refusal.stage(), stage);
    assert_eq!(refusal.reason(), reason);
}

/// A foreign Zone, a stale source generation, and an advanced desired
/// revision each fail attachment, and none of them reads a credential byte.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn foreign_zone_or_stale_session_evidence_fails_relay_attachment() {
    let credentials = Arc::new(ReadCountingCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let provider = AzureRelayTransportProvider::new(
        relay_config(),
        relay_endpoint(),
        Arc::clone(&credentials),
        Arc::new(NoopConnector),
    )
    .unwrap();
    let admitted = relay_credential(
        "relay-send",
        RELAY_CREDENTIAL_AUDIENCE,
        vec![CredentialOperation::AcquireToken],
    );
    provider
        .bindings()
        .admit(admitted.clone())
        .expect("relationship admitted");
    let delivery = AdmittedRelayDelivery::new(admitted, None);

    let foreign_zone = evidence_for(
        ZoneId::parse("other").unwrap(),
        3,
        4,
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        2,
    );
    let error = refusal_of(
        provider
            .open_under_delivery(&delivery, &foreign_zone, scoped_request(2))
            .await,
        "a foreign Zone fails attachment",
    );
    assert_attach_refusal(
        error,
        "foreign-zone",
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
    );
    assert_eq!(credentials.reads(), 0);

    let stale_source = evidence_for(
        work_zone(),
        2,
        4,
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        2,
    );
    let error = refusal_of(
        provider
            .open_under_delivery(&delivery, &stale_source, scoped_request(2))
            .await,
        "a stale source generation fails attachment",
    );
    assert_attach_refusal(
        error,
        "stale-source-generation",
        AdmissionStage::Admit,
        RefusalReason::StaleAuthority,
    );
    assert_eq!(credentials.reads(), 0);

    let advanced_revision = evidence_for(
        work_zone(),
        3,
        4,
        DesiredRevision::INITIAL.try_next().unwrap(),
        ZoneDesiredSequence::INITIAL,
        2,
    );
    let error = refusal_of(
        provider
            .open_under_delivery(&delivery, &advanced_revision, scoped_request(2))
            .await,
        "an advanced desired revision fails attachment",
    );
    assert_attach_refusal(
        error,
        "stale-desired-revision",
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
    );
    assert_eq!(
        credentials.reads(),
        0,
        "no refused attempt reached the credential Provider"
    );
}

/// A credential relationship admitted for a different audience, or without
/// `AcquireToken`, refuses before any credential byte is read.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn relay_credential_outside_the_admitted_audience_or_operations_refuses_before_any_read() {
    let credentials = Arc::new(ReadCountingCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let provider = AzureRelayTransportProvider::new(
        relay_config(),
        relay_endpoint(),
        Arc::clone(&credentials),
        Arc::new(NoopConnector),
    )
    .unwrap();

    let wrong_audience = relay_credential(
        "relay-listen",
        "relay-listener",
        vec![CredentialOperation::AcquireToken],
    );
    provider
        .bindings()
        .admit(wrong_audience.clone())
        .expect("relationship admitted");
    let error = refusal_of(
        provider
            .open_under_delivery(
                &AdmittedRelayDelivery::new(wrong_audience, None),
                &current_evidence(),
                scoped_request(2),
            )
            .await,
        "a foreign audience refuses before any read",
    );
    assert_attach_refusal(
        error,
        "relay-audience-not-admitted",
        AdmissionStage::Admit,
        RefusalReason::TargetSupportMissing,
    );
    assert_eq!(credentials.reads(), 0);

    let no_acquire = relay_credential(
        "relay-refresh",
        RELAY_CREDENTIAL_AUDIENCE,
        vec![CredentialOperation::RefreshToken],
    );
    provider
        .bindings()
        .admit(no_acquire.clone())
        .expect("relationship admitted");
    let error = refusal_of(
        provider
            .open_under_delivery(
                &AdmittedRelayDelivery::new(no_acquire, None),
                &current_evidence(),
                scoped_request(2),
            )
            .await,
        "a delivery without AcquireToken refuses before any read",
    );
    assert_attach_refusal(
        error,
        "relay-operation-not-admitted",
        AdmissionStage::Admit,
        RefusalReason::RequiredCapabilityOutsideCeiling,
    );
    assert_eq!(
        credentials.reads(),
        0,
        "a refused class never reads a credential byte"
    );
}

/// A carriage opens under a delivery, the relationship is revoked, and a
/// reconnect with fresh evidence and a higher reconnect ordinal still
/// refuses: revocation is retained, not dropped, so releasing a connection
/// never re-mints the authority behind it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn reconnect_does_not_revive_revoked_binding_authority() {
    let credentials = Arc::new(LiveLeaseCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let provider = AzureRelayTransportProvider::new(
        relay_config(),
        relay_endpoint(),
        Arc::clone(&credentials),
        Arc::new(NoopConnector),
    )
    .unwrap();
    let admitted = relay_credential(
        "relay-send",
        RELAY_CREDENTIAL_AUDIENCE,
        vec![CredentialOperation::AcquireToken],
    );
    let key = admitted.key().clone();
    provider
        .bindings()
        .admit(admitted.clone())
        .expect("relationship admitted");
    let delivery = AdmittedRelayDelivery::new(admitted, None);

    let connection = provider
        .open_under_delivery(&delivery, &current_evidence(), scoped_request(2))
        .await
        .expect("the admitted delivery opens");
    assert_eq!(connection.reconnect_generation(), 2);
    assert_eq!(credentials.reads(), 1);
    connection.close().await.expect("close");

    provider
        .bindings()
        .revoke(&key)
        .expect("relationship revoked");
    assert_eq!(
        provider
            .bindings()
            .binding(&key)
            .expect("a revoked relationship is retained")
            .fence()
            .phase(),
        RelationshipPhase::Revoked
    );

    let reconnect = evidence_for(
        work_zone(),
        3,
        4,
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        9,
    );
    let error = refusal_of(
        provider
            .open_under_delivery(&delivery, &reconnect, scoped_request(9))
            .await,
        "a reconnect cannot revive revoked binding authority",
    );
    assert_attach_refusal(
        error,
        "relationship-revoked",
        AdmissionStage::Revoke,
        RefusalReason::StaleAuthority,
    );
    assert_eq!(credentials.reads(), 1, "the reconnect read no credential");
}

/// A generated configuration cannot hand the host a gateway-owned
/// credential: naming `Host/host` is accepted as a document and refused as a
/// delivery, and a document carrying the material itself is refused before
/// it is ever parsed into a value.
#[test]
fn generated_configuration_cannot_hand_the_host_a_gateway_owned_credential() {
    let host_document = r#"{
        "executionRef": "Host/host",
        "credentialRef": "Credential/relay-send",
        "networkRef": "Network/relay-egress",
        "slot": "relay-egress",
        "audience": "relay-egress",
        "operations": ["acquire-token", "refresh-token"],
        "lifetime": { "validFor": "600s", "expiresIn": "600s" }
    }"#;
    let host = GatewayTransportConfiguration::parse_generated(host_document.as_bytes())
        .expect("the document itself is well formed");
    assert_eq!(host.execution_ref().to_canonical_string(), "Host/host");
    assert_eq!(
        host.delivery_binding()
            .expect_err("a Host consumer is refused"),
        CredentialError::NotAnAdmittedDelivery
    );

    for material in [
        r#""sasKey": "abc","#,
        r#""key": "abc","#,
        r#""token": "abc","#,
        r#""secret": "abc","#,
        r#""listenKey": "abc","#,
        r#""sendKey": "abc","#,
        r#""password": "abc","#,
        r#""bearer": "abc","#,
        r#""extra": { "listenKey": "abc" },"#,
    ] {
        let document = format!(
            r#"{{
                {material}
                "executionRef": "Guest/gateway",
                "credentialRef": "Credential/relay-send",
                "networkRef": "Network/relay-egress",
                "slot": "relay-egress",
                "audience": "relay-egress",
                "operations": ["acquire-token"],
                "lifetime": {{ "validFor": "600s", "expiresIn": "600s" }}
            }}"#
        );
        assert_eq!(
            GatewayTransportConfiguration::parse_generated(document.as_bytes())
                .expect_err("material is refused, not dropped"),
            CredentialError::ConfigurationCarriesMaterial,
            "field {material} was not refused"
        );
    }

    // An unknown field is still a refusal rather than something skipped.
    let unknown = r#"{
        "executionRef": "Guest/gateway",
        "credentialRef": "Credential/relay-send",
        "networkRef": "Network/relay-egress",
        "slot": "relay-egress",
        "audience": "relay-egress",
        "operations": ["acquire-token"],
        "lifetime": { "validFor": "600s", "expiresIn": "600s" },
        "listenKeyName": "gateway-listen"
    }"#;
    assert_eq!(
        GatewayTransportConfiguration::parse_generated(unknown.as_bytes())
            .expect_err("an unknown field is refused"),
        CredentialError::Malformed
    );
}

/// A well-formed guest-scoped configuration carries references only, and its
/// `Debug` rendering leaks none of them.
#[test]
fn generated_configuration_carries_references_only() {
    let document = r#"{
        "executionRef": "Guest/canary-exec",
        "credentialRef": "Credential/canary-cred",
        "networkRef": "Network/canary-egress",
        "slot": "canary-slot",
        "audience": "canary-audience",
        "operations": ["acquire-token", "sign-challenge"],
        "lifetime": { "validFor": "600s", "expiresIn": "900s" }
    }"#;
    let configuration = GatewayTransportConfiguration::parse_generated(document.as_bytes())
        .expect("a well-formed guest configuration parses");

    assert!(!configuration.carries_material());
    let debug = format!("{configuration:?}");
    for leaked in [
        "canary-exec",
        "canary-cred",
        "canary-egress",
        "canary-slot",
        "canary-audience",
    ] {
        assert!(!debug.contains(leaked), "Debug leaked {leaked}: {debug}");
    }

    let binding = configuration
        .delivery_binding()
        .expect("a guest consumer is an admitted delivery");
    assert_eq!(
        binding.consumer_ref().to_canonical_string(),
        "Guest/canary-exec"
    );
    assert_eq!(binding.audience().as_str(), "canary-audience");
    assert!(binding.admits_operation(CredentialOperation::AcquireToken));
    assert!(!binding.admits_operation(CredentialOperation::RefreshToken));
    assert_eq!(
        configuration.network_ref().to_canonical_string(),
        "Network/canary-egress"
    );
}

/// The graph audience is a bounded token, not the wire-protocol audience, and
/// the two are not interchangeable spellings of one thing.
#[test]
fn the_graph_audience_is_not_the_wire_audience() {
    assert!(BoundedToken::parse(RELAY_CREDENTIAL_AUDIENCE).is_ok());
    assert!(
        BoundedToken::parse(RELAY_TOKEN_RESOURCE).is_err(),
        "the wire audience is a URL and cannot be a graph audience"
    );
    assert_ne!(RELAY_CREDENTIAL_AUDIENCE, RELAY_TOKEN_RESOURCE);
}
