//! Graph-bound relay attach proof.
//!
//! The injection case is proved end to end rather than asserted about a
//! classifier: a real `RelayConnection` is opened through a real Provider, a
//! forged control frame is written through the real `RelaySocket` seam, read
//! back off the live carriage, and handed to the one function that could turn
//! carriage bytes into a control request. The connection's live state is then
//! compared before and after.

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::ResourceRef;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingSlot, BoundedToken, CredentialBindingRequest, CredentialLifetime,
    CredentialOperation, DesiredRevision, RefusalReason, ResourceGeneration, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};
use d2b_provider_transport_azure_relay::{
    AdmittedRelayDelivery, AdmittedTransportBinding, AzureRelayTransportProvider, CarriageClass,
    ControlPlaneInjectionRefusal, ControlPlaneRequest, GraphBoundRelayError,
    MAX_ADMITTED_TRANSPORT_BINDINGS, RELAY_CREDENTIAL_AUDIENCE, RelayConnection,
    RelayCredentialBinding, RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial,
    RelayCredentialRole, RelayEndpoint, RelayEnrollmentChallenge, RelayEnrollmentProof,
    RelayEnrollmentVerifier, RelayFrame, RelayRole, RelaySecret, RelaySessionPhase, RelaySocket,
    RelaySocketConnector, RelayTransportConfig, RelayTransportError, RelayTransportSettings,
    RelationshipFence, RelationshipPhase, ScopedCredentialClient, ScopedCredentialRequest,
    TransportAttachEvidence, TransportBindingRefusal, TransportBindingRegistry,
    TransportControlOperation, admit_attach, classify_carriage,
};
use tokio::sync::Mutex;

const CREDENTIAL_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const CONSUMER_UID: &str = "ffffffff-ffff-4fff-bfff-ffffffffffff";

fn zone() -> ZoneId {
    ZoneId::parse("work").expect("zone")
}

fn store() -> StoreIncarnation {
    StoreIncarnation::parse("store-1").expect("store")
}

fn fence() -> RelationshipFence {
    RelationshipFence::new(
        store(),
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        ResourceGeneration::new(3).expect("source generation"),
        ResourceGeneration::new(4).expect("consumer generation"),
        ReconnectGeneration::new(1).expect("reconnect generation"),
    )
}

fn evidence() -> TransportAttachEvidence {
    TransportAttachEvidence::new(
        zone(),
        store(),
        ResourceGeneration::new(3).expect("source generation"),
        ResourceGeneration::new(4).expect("consumer generation"),
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        ReconnectGeneration::new(2).expect("reconnect generation"),
    )
}

fn scoped_request(generation: u64) -> ScopedCredentialRequest {
    ScopedCredentialRequest::new(
        zone(),
        ResourceRef::parse("Credential/relay-send").expect("credential"),
        ResourceRef::parse("Guest/gateway").expect("consumer"),
        RelayCredentialRole::Send,
        RelayCredentialBinding::new_scoped(zone(), "link-test", "session-test", generation)
            .expect("binding"),
        1_000,
    )
    .expect("scoped request")
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
        ResourceRef::parse(&format!("Credential/{name}")).expect("credential"),
        ResourceRef::parse("Guest/gateway").expect("consumer"),
        BindingSlot::parse(slot).expect("slot"),
        BoundedToken::parse(audience).expect("audience"),
        operations,
        CredentialLifetime::new("600s", "600s").expect("lifetime"),
    )
    .expect("credential request");
    let key = request
        .key(
            zone(),
            ResourceUid::parse(CREDENTIAL_UID).expect("credential uid"),
            ResourceUid::parse(CONSUMER_UID).expect("consumer uid"),
        )
        .expect("binding key");
    AdmittedTransportBinding::new(key, request, fence())
}

/// One admitted credential relationship that admits only `AcquireToken`.
fn acquire_only(name: &str, audience: &str) -> AdmittedTransportBinding {
    admitted_credential(
        name,
        "relay-egress",
        audience,
        vec![CredentialOperation::AcquireToken],
    )
}

/// The real `RelaySocket` seam, as a duplex.
///
/// The sealed `RelayFrame` deliberately exposes no reader, so a peer cannot
/// read carriage bytes back out. The seam therefore records, at the exact
/// moment it hands a frame to the live carriage, the bytes that frame was
/// built from - that record is what the injection attempt is fed.
struct DuplexSocket {
    inbound: Mutex<VecDeque<QueuedFrame>>,
    delivered: Mutex<Vec<Vec<u8>>>,
    closed: AtomicUsize,
}

struct QueuedFrame {
    bytes: Vec<u8>,
    frame: RelayFrame,
}

impl DuplexSocket {
    fn new() -> Self {
        Self {
            inbound: Mutex::new(VecDeque::new()),
            delivered: Mutex::new(Vec::new()),
            closed: AtomicUsize::new(0),
        }
    }

    /// Write bytes into the carriage's read direction as a peer would.
    async fn push(&self, bytes: &[u8]) {
        self.inbound.lock().await.push_back(QueuedFrame {
            bytes: bytes.to_vec(),
            frame: RelayFrame::new(bytes.to_vec()).expect("bounded frame"),
        });
    }
}

#[async_trait]
impl RelaySocket for DuplexSocket {
    async fn send(&self, frame: RelayFrame) -> Result<(), RelayTransportError> {
        self.inbound.lock().await.push_back(QueuedFrame {
            bytes: Vec::new(),
            frame,
        });
        Ok(())
    }

    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        let Some(queued) = self.inbound.lock().await.pop_front() else {
            return Ok(None);
        };
        self.delivered.lock().await.push(queued.bytes);
        Ok(Some(queued.frame))
    }

    async fn close(&self) -> Result<(), RelayTransportError> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct DuplexConnector {
    socket: Arc<DuplexSocket>,
}

#[async_trait]
impl RelaySocketConnector for DuplexConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
    }
}

struct RecordingCredentials {
    reads: Arc<AtomicUsize>,
}

impl RecordingCredentials {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ScopedCredentialClient for RecordingCredentials {
    async fn read_credential(
        &self,
        request: &ScopedCredentialRequest,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(b"relay-token".to_vec()).expect("secret"),
            ),
            request.role(),
            valid_expiry(),
            request.binding().clone(),
        )
        .expect("bound lease"))
    }

    async fn revoke_credential(
        &self,
        _: RelayCredentialLease,
    ) -> Result<(), RelayCredentialError> {
        Ok(())
    }
}

fn valid_expiry() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
        + 60_000
}

struct AcceptingEnrollment;

impl RelayEnrollmentVerifier for AcceptingEnrollment {
    fn verify_enrollment(&self, transcript: &[u8], _: &RelayEnrollmentChallenge) -> bool {
        !transcript.is_empty()
    }
}

fn provider(
    credentials: Arc<RecordingCredentials>,
    socket: Arc<DuplexSocket>,
) -> AzureRelayTransportProvider<RecordingCredentials, DuplexConnector> {
    AzureRelayTransportProvider::new(
        RelayTransportConfig {
            execution_ref: ResourceRef::parse("Guest/gateway").expect("guest"),
            network_ref: ResourceRef::parse("Network/relay").expect("network"),
            max_concurrent_sessions: 4,
            connect_timeout_seconds: 30,
        },
        RelayEndpoint {
            settings: RelayTransportSettings::new("relns-d2b-prod", "hc-d2b-k2").expect("settings"),
        },
        credentials,
        Arc::new(DuplexConnector { socket }),
    )
    .expect("provider")
}

fn carriage() -> AdmittedTransportBinding {
    acquire_only("relay-send", RELAY_CREDENTIAL_AUDIENCE)
}

async fn open_carriage(
    provider: &AzureRelayTransportProvider<RecordingCredentials, DuplexConnector>,
    delivery: &AdmittedRelayDelivery,
) -> Result<RelayConnection, GraphBoundRelayError> {
    provider
        .open_under_delivery(delivery, &evidence(), scoped_request(2))
        .await
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

/// Take the refusal out of a carriage open.
///
/// `RelayConnection` has no `Debug`, and it is deliberately kept that way: a
/// carriage carries a live socket and a credential lease, and neither belongs
/// in a panic message. Matching by hand keeps that property instead of
/// widening it to make an assertion convenient.
fn refusal_of(
    opened: Result<RelayConnection, GraphBoundRelayError>,
    context: &str,
) -> GraphBoundRelayError {
    match opened {
        Ok(_) => panic!("{context}"),
        Err(error) => error,
    }
}

/// A relay stream carries data. A forged control frame written through the
/// real socket seam, read back off the live carriage, and handed to the one
/// injection entry point in the crate is refused - and the carriage is exactly
/// as it was.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn relay_stream_carriage_cannot_inject_a_control_operation() {
    let credentials = Arc::new(RecordingCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let socket = Arc::new(DuplexSocket::new());
    let provider = provider(Arc::clone(&credentials), Arc::clone(&socket));

    let binding = carriage();
    let route = provider
        .bindings()
        .admit(binding.clone())
        .expect("relationship admitted");
    let connection = open_carriage(
        &provider,
        &AdmittedRelayDelivery::new(binding.clone(), None),
    )
    .await
    .expect("carriage opened under the admitted delivery");
    connection
        .enroll(
            RelayEnrollmentProof::authenticate(
                &AcceptingEnrollment,
                b"enrollment-transcript",
                &connection.enrollment_challenge(),
            )
            .expect("proof"),
        )
        .await
        .expect("enrolled");
    assert_eq!(connection.phase().await, RelaySessionPhase::EnrolledKk);
    let before = connection.observe_transport().await;

    // The forged frame is real bytes built from real values: the admitted
    // route's own slot, the live Zone, the live reconnect generation, and a
    // real operation name. It also carries a `token` field, so a parser that
    // looked for authority in the carriage would find something to find.
    let forged = format!(
        r#"{{"op":"close","slot":"{}","zone":"{}","reconnect":{},"token":"forged"}}"#,
        binding.key().slot().as_str(),
        zone().as_str(),
        connection.reconnect_generation(),
    );
    assert_eq!(
        classify_carriage(forged.as_bytes()),
        CarriageClass::ControlShaped,
        "the scan really does read the bytes"
    );
    socket.push(forged.as_bytes()).await;
    connection
        .receive()
        .await
        .expect("receive")
        .expect("the live carriage took the frame off the seam");
    let read_back = socket.delivered.lock().await[0].clone();
    assert_eq!(
        read_back, forged.as_bytes(),
        "the carriage was handed exactly the forged bytes"
    );

    let refusal =
        ControlPlaneRequest::from_carriage(Some(&route), &read_back).expect_err("carriage is data");
    assert_eq!(refusal, ControlPlaneInjectionRefusal::NotAControlRequest);
    assert_eq!(refusal.code(), "not-a-control-request");
    assert_eq!(
        ControlPlaneRequest::from_carriage(None, &read_back)
            .expect_err("no route, no request")
            .code(),
        "route-not-admitted"
    );
    assert_eq!(
        ControlPlaneRequest::from_carriage(Some(&route), b"session-packet")
            .expect_err("ordinary carriage is data")
            .code(),
        "no-operation-discriminant"
    );

    // The privileged path is real, and it is reachable only in process: the
    // route mints the token, the token is spent on the request, and nothing in
    // `read_back` participates.
    let issued = ControlPlaneRequest::issue(
        TransportControlOperation::Close,
        route.control_token(),
        route.carriage(),
    );
    assert_eq!(issued.operation(), TransportControlOperation::Close);
    assert_eq!(issued.carriage().relationship(), binding.key());

    // The live carriage is untouched by the attempt.
    assert_eq!(connection.phase().await, RelaySessionPhase::EnrolledKk);
    assert_eq!(connection.observe_transport().await, before);
    assert_eq!(connection.reconnect_generation(), before.reconnect_generation);
    assert_eq!(connection.binding().reconnect_generation(), 2);
    connection.close().await.expect("close still succeeds");
    assert_eq!(socket.closed.load(Ordering::SeqCst), 1);
}

/// A carriage needs its credential relationship to be admitted for the
/// relay's own audience and to admit `AcquireToken` before any credential byte
/// is read.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn admitted_delivery_requires_both_relationships() {
    let credentials = Arc::new(RecordingCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let socket = Arc::new(DuplexSocket::new());
    let provider = provider(Arc::clone(&credentials), Arc::clone(&socket));

    let foreign_audience = acquire_only("relay-listen", "relay-listener");
    provider
        .bindings()
        .admit(foreign_audience.clone())
        .expect("relationship admitted");
    let refused = refusal_of(
        open_carriage(
            &provider,
            &AdmittedRelayDelivery::new(foreign_audience, None),
        )
        .await,
        "a foreign audience admits no carriage",
    );
    assert_attach_refusal(
        refused,
        "relay-audience-not-admitted",
        AdmissionStage::Admit,
        RefusalReason::TargetSupportMissing,
    );
    assert_eq!(credentials.reads(), 0);

    let no_acquire = admitted_credential(
        "relay-send-refresh",
        "relay-egress-refresh",
        RELAY_CREDENTIAL_AUDIENCE,
        vec![CredentialOperation::RefreshToken],
    );
    provider
        .bindings()
        .admit(no_acquire.clone())
        .expect("relationship admitted");
    let refused = refusal_of(
        open_carriage(&provider, &AdmittedRelayDelivery::new(no_acquire, None)).await,
        "a delivery without AcquireToken admits no carriage",
    );
    assert_attach_refusal(
        refused,
        "relay-operation-not-admitted",
        AdmissionStage::Admit,
        RefusalReason::RequiredCapabilityOutsideCeiling,
    );
    assert_eq!(credentials.reads(), 0);

    // The admitted class opens, and only then does a credential byte move.
    let admitted = carriage();
    let route = provider
        .bindings()
        .admit(admitted.clone())
        .expect("relationship admitted");
    assert_eq!(route.audience().as_str(), RELAY_CREDENTIAL_AUDIENCE);
    let connection = open_carriage(&provider, &AdmittedRelayDelivery::new(admitted, None))
        .await
        .expect("carriage opened");
    assert_eq!(credentials.reads(), 1);
    connection.close().await.expect("close");
}

/// A revocation committed in the registry is what a reconnect meets, not the
/// caller's stale copy of the relationship, and no reconnect ordinal revives
/// it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_revocation_outlives_the_callers_copy_of_the_relationship() {
    let credentials = Arc::new(RecordingCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let socket = Arc::new(DuplexSocket::new());
    let provider = provider(Arc::clone(&credentials), Arc::clone(&socket));
    let binding = carriage();
    let key = binding.key().clone();
    let registry = provider.bindings();
    registry.admit(binding.clone()).expect("admitted");

    let stale = AdmittedRelayDelivery::new(binding, None);
    let connection = open_carriage(&provider, &stale).await.expect("first carriage");
    connection.close().await.expect("close");
    registry.revoke(&key).expect("revoked");

    let error = refusal_of(
        open_carriage(&provider, &stale).await,
        "a stale caller copy cannot revive a revoked relationship",
    );
    assert_attach_refusal(
        error,
        "relationship-revoked",
        AdmissionStage::Revoke,
        RefusalReason::StaleAuthority,
    );
    assert_eq!(credentials.reads(), 1, "the reconnect read no credential");

    // The registry's retained copy is what a reconnect is measured against,
    // and the freshest evidence in existence still meets its revoked fence.
    let live = registry
        .binding(&key)
        .expect("a revoked relationship is retained");
    assert_eq!(live.fence().phase(), RelationshipPhase::Revoked);
    let newest = TransportAttachEvidence::new(
        zone(),
        store(),
        ResourceGeneration::new(3).expect("source generation"),
        ResourceGeneration::new(4).expect("consumer generation"),
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        ReconnectGeneration::new(u64::MAX).expect("reconnect generation"),
    );
    let refusal = admit_attach(&live, &newest)
        .expect_err("a revoked fence refuses even the newest evidence");
    assert_eq!(refusal.code(), "relationship-revoked");
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
}

/// A hand-built delivery is not authority: the Provider resolves every attempt
/// against the relationship its own registry holds.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn unadmitted_relationship_has_no_open_path() {
    let credentials = Arc::new(RecordingCredentials {
        reads: Arc::new(AtomicUsize::new(0)),
    });
    let socket = Arc::new(DuplexSocket::new());
    let provider = provider(Arc::clone(&credentials), Arc::clone(&socket));
    let stray = carriage();

    let error = refusal_of(
        open_carriage(&provider, &AdmittedRelayDelivery::new(stray.clone(), None)).await,
        "a hand-built delivery is not authority",
    );
    assert_eq!(
        error,
        GraphBoundRelayError::BindingRefused(TransportBindingRefusal::NotAdmitted)
    );
    assert_eq!(credentials.reads(), 0);
    assert_eq!(error.to_string(), "binding-not-admitted");

    // The fence itself admits this evidence: the refusal came from the
    // registry, which is the only list that can say a carriage may open.
    assert!(
        admit_attach(&stray, &evidence()).is_ok(),
        "the registry is the authority, not the evidence"
    );
}

/// A foreign Zone, stale generations, a drain, and an old reconnect ordinal
/// each refuse at their own stage.
#[test]
fn the_gate_names_the_first_reason_an_attempt_is_not_admitted() {
    let binding = carriage();
    let attempt = |zone_id: ZoneId,
                   source: ResourceGeneration,
                   consumer: ResourceGeneration,
                   revision: DesiredRevision,
                   sequence: ZoneDesiredSequence,
                   reconnect: ReconnectGeneration| {
        TransportAttachEvidence::new(
            zone_id,
            store(),
            source,
            consumer,
            revision,
            sequence,
            reconnect,
        )
    };

    let cases = [
        (
            attempt(
                ZoneId::parse("other").expect("zone"),
                ResourceGeneration::new(3).expect("source generation"),
                ResourceGeneration::new(4).expect("consumer generation"),
                DesiredRevision::INITIAL,
                ZoneDesiredSequence::INITIAL,
                ReconnectGeneration::new(2).expect("reconnect generation"),
            ),
            "foreign-zone",
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
        ),
        (
            attempt(
                zone(),
                ResourceGeneration::new(2).expect("source generation"),
                ResourceGeneration::new(4).expect("consumer generation"),
                DesiredRevision::INITIAL,
                ZoneDesiredSequence::INITIAL,
                ReconnectGeneration::new(2).expect("reconnect generation"),
            ),
            "stale-source-generation",
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
        ),
        (
            attempt(
                zone(),
                ResourceGeneration::new(3).expect("source generation"),
                ResourceGeneration::new(5).expect("consumer generation"),
                DesiredRevision::INITIAL,
                ZoneDesiredSequence::INITIAL,
                ReconnectGeneration::new(2).expect("reconnect generation"),
            ),
            "stale-consumer-generation",
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
        ),
        (
            attempt(
                zone(),
                ResourceGeneration::new(3).expect("source generation"),
                ResourceGeneration::new(4).expect("consumer generation"),
                DesiredRevision::INITIAL.try_next().expect("revision"),
                ZoneDesiredSequence::INITIAL,
                ReconnectGeneration::new(2).expect("reconnect generation"),
            ),
            "stale-desired-revision",
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
        ),
        (
            attempt(
                zone(),
                ResourceGeneration::new(3).expect("source generation"),
                ResourceGeneration::new(4).expect("consumer generation"),
                DesiredRevision::INITIAL,
                ZoneDesiredSequence::INITIAL.try_next().expect("sequence"),
                ReconnectGeneration::new(2).expect("reconnect generation"),
            ),
            "stale-desired-sequence",
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
        ),
    ];
    for (presented, code, stage, reason) in cases {
        let refusal = admit_attach(&binding, &presented).expect_err("evidence must not attach");
        assert_eq!(refusal.code(), code);
        assert_eq!(refusal.stage(), stage);
        assert_eq!(refusal.reason(), reason);
    }

    // Raising the reconnect floor is what makes an older ordinal refuse; the
    // floor is the fence's, so the evidence has nothing to compare against
    // until the graph has committed the new one.
    let raised = AdmittedTransportBinding::new(
        binding.key().clone(),
        binding.request().clone(),
        binding
            .fence()
            .clone()
            .raise_minimum_reconnect(ReconnectGeneration::new(3).expect("reconnect generation")),
    );
    let refusal = admit_attach(&raised, &evidence()).expect_err("an old reconnect admits nothing");
    assert_eq!(refusal.code(), "stale-reconnect-generation");
    assert_eq!(refusal.stage(), AdmissionStage::Activate);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);

    let draining = AdmittedTransportBinding::new(
        binding.key().clone(),
        binding.request().clone(),
        binding.fence().clone().drain(),
    );
    let refusal = admit_attach(&draining, &evidence()).expect_err("a drain admits nothing");
    assert_eq!(refusal.code(), "relationship-draining");
    assert_eq!(refusal.stage(), AdmissionStage::Drain);
    assert_eq!(refusal.reason(), RefusalReason::UnprovenEffect);

    // A different store incarnation is a different graph, not a stale peer.
    let other_store = TransportAttachEvidence::new(
        zone(),
        StoreIncarnation::parse("store-2").expect("store"),
        ResourceGeneration::new(3).expect("source generation"),
        ResourceGeneration::new(4).expect("consumer generation"),
        DesiredRevision::INITIAL,
        ZoneDesiredSequence::INITIAL,
        ReconnectGeneration::new(2).expect("reconnect generation"),
    );
    let refusal =
        admit_attach(&binding, &other_store).expect_err("another store admits nothing");
    assert_eq!(refusal.code(), "store-incarnation-mismatch");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), RefusalReason::StoreIncarnationMismatch);
}

/// The registry is bounded, and its ceiling can be narrowed but never raised.
#[test]
fn the_registry_ceiling_is_frozen() {
    let registry = TransportBindingRegistry::with_ceiling(usize::MAX);
    assert_eq!(registry.ceiling(), MAX_ADMITTED_TRANSPORT_BINDINGS);
    let narrow = TransportBindingRegistry::with_ceiling(1);
    assert_eq!(narrow.ceiling(), 1);
    narrow.admit(carriage()).expect("first admitted");
    assert_eq!(
        narrow
            .admit(admitted_credential(
                "relay-other",
                "relay-other",
                RELAY_CREDENTIAL_AUDIENCE,
                vec![CredentialOperation::AcquireToken],
            ))
            .expect_err("the frozen ceiling refuses"),
        TransportBindingRefusal::RegistryFull
    );
    assert_eq!(narrow.len(), 1);
    narrow.finalize();
    assert_eq!(narrow.len(), 0);
}
