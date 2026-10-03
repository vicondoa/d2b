use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingSlot, BoundedToken, DesiredRevision, EndpointAttachmentKind,
    EndpointBindingRequest, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};
use d2b_provider_transport_vsock::{
    AdmittedTransportBinding, AdmittedTransportRoute, GuestIdentity, MAX_REPLAY_ENTRIES,
    NamedStreamError,
    NamedStreamId, NamedStreamPort, OpaqueBindingId, OpaqueEndpointId, OpenTransportRequest,
    PeerCid, RelationshipFence, SessionAuthority, SessionKey, SessionProof, SessionRejectReason,
    SessionState, ServicePhase, TransportAttachEvidence, TransportBindingRegistry, TransportRole,
    VsockEffectError, VsockEffectPort, VsockTransportService, admit_attach,
};
use ring::rand::{SystemRandom, generate};
use tokio::io::DuplexStream;

fn nonce_for(index: u16) -> [u8; 32] {
    let mut nonce = generate::<[u8; 32]>(&SystemRandom::new()).unwrap().expose();
    nonce[..2].copy_from_slice(&index.to_be_bytes());
    nonce
}

fn identity(cid: u32) -> GuestIdentity {
    GuestIdentity::new(
        ResourceRef::parse("Guest/guest-a").unwrap(),
        ZoneId::parse("work").unwrap(),
        PeerCid::from_core(cid).unwrap(),
        "boot-a",
    )
    .unwrap()
}

#[test]
fn correct_cid_signature_guest_zone_and_session_establish_ready() {
    let key = SessionKey::from_core([7; 32]);
    let expected = identity(42);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 3);
    let proof = SessionProof::sign(&key, &expected, nonce_for(1), 3);

    let session = authority
        .authenticate(PeerCid::from_core(42).unwrap(), proof)
        .unwrap();
    assert_eq!(session.state(), SessionState::Ready);
    assert!(session.matches(&expected));
    assert_eq!(session.disconnect(), SessionState::Disconnected);
}

#[test]
fn cid_reuse_and_replay_are_rejected() {
    let key = SessionKey::from_core([8; 32]);
    let expected = identity(42);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 3);
    let proof = SessionProof::sign(&key, &expected, nonce_for(2), 3);
    authority
        .authenticate(PeerCid::from_core(42).unwrap(), proof.clone())
        .unwrap();
    assert_eq!(
        authority
            .authenticate(PeerCid::from_core(42).unwrap(), proof)
            .unwrap_err(),
        SessionRejectReason::Replay
    );

    let mut other = identity(43);
    let proof = SessionProof::sign(&key, &other, nonce_for(3), 3);
    assert_eq!(
        authority
            .authenticate(PeerCid::from_core(42).unwrap(), proof)
            .unwrap_err(),
        SessionRejectReason::CidMismatch
    );
    other = identity(42);
    let proof = SessionProof::sign(&key, &other, nonce_for(4), 2);
    assert_eq!(
        authority
            .authenticate(PeerCid::from_core(42).unwrap(), proof)
            .unwrap_err(),
        SessionRejectReason::StaleSignature
    );
}

#[test]
fn replay_cache_refuses_new_sessions_at_its_bound() {
    let key = SessionKey::from_core([3; 32]);
    let expected = identity(42);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 3);
    for index in 0..MAX_REPLAY_ENTRIES {
        authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &expected, nonce_for(index as u16), 3),
            )
            .unwrap();
    }
    assert_eq!(
        authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &expected, nonce_for(MAX_REPLAY_ENTRIES as u16), 3),
            )
            .unwrap_err(),
        SessionRejectReason::AuthorityUnavailable
    );
}

#[test]
fn guest_zone_and_signature_mismatches_are_refused() {
    let expected = identity(42);
    let key = SessionKey::from_core([3; 32]);
    let wrong_key = SessionKey::from_core([4; 32]);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 3);

    let guest = GuestIdentity::new(
        ResourceRef::parse("Guest/other").unwrap(),
        ZoneId::parse("work").unwrap(),
        PeerCid::from_core(42).unwrap(),
        "boot-a",
    )
    .unwrap();
    assert_eq!(
        authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce_for(5), 3),
            )
            .unwrap_err(),
        SessionRejectReason::GuestMismatch
    );

    let zone = GuestIdentity::new(
        ResourceRef::parse("Guest/guest-a").unwrap(),
        ZoneId::parse("personal").unwrap(),
        PeerCid::from_core(42).unwrap(),
        "boot-a",
    )
    .unwrap();
    assert_eq!(
        authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &zone, nonce_for(6), 3),
            )
            .unwrap_err(),
        SessionRejectReason::ZoneMismatch
    );

    assert_eq!(
        authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&wrong_key, &expected, nonce_for(7), 3),
            )
            .unwrap_err(),
        SessionRejectReason::SignatureInvalid
    );
}

#[test]
fn zero_reconnect_generation_is_never_admitted() {
    let expected = identity(42);
    let key = SessionKey::from_core([9; 32]);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 0);
    assert_eq!(
        authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &expected, nonce_for(8), 0),
            )
            .unwrap_err(),
        SessionRejectReason::StaleSignature
    );
}

const SOURCE_UID: &str = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
const CONSUMER_UID: &str = "9c858901-8a57-4791-81fe-4c455b099bc9";

fn endpoint_request() -> EndpointBindingRequest {
    EndpointBindingRequest::new(
        ResourceRef::parse("Endpoint/vsock").unwrap(),
        ResourceRef::parse("Process/vsock-agent").unwrap(),
        BindingSlot::parse("transport").unwrap(),
        EndpointAttachmentKind::Attach,
        BoundedToken::parse("vsock-transport").unwrap(),
    )
    .unwrap()
}

fn fence() -> RelationshipFence {
    RelationshipFence::new(
        StoreIncarnation::parse("store-a").unwrap(),
        DesiredRevision::INITIAL.try_next().unwrap(),
        ZoneDesiredSequence::INITIAL.try_next().unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    )
}

fn admitted_binding() -> AdmittedTransportBinding {
    let request = endpoint_request();
    let key = request
        .key(
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse(SOURCE_UID).unwrap(),
            ResourceUid::parse(CONSUMER_UID).unwrap(),
        )
        .unwrap();
    AdmittedTransportBinding::new(key, request, fence())
}

fn evidence_in(zone: &str) -> TransportAttachEvidence {
    TransportAttachEvidence::new(
        ZoneId::parse(zone).unwrap(),
        StoreIncarnation::parse("store-a").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        DesiredRevision::INITIAL.try_next().unwrap(),
        ZoneDesiredSequence::INITIAL.try_next().unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    )
}

fn open_request() -> OpenTransportRequest {
    OpenTransportRequest::new(
        OpaqueEndpointId::parse("endpoint-a").unwrap(),
        OpaqueBindingId::parse("binding-a").unwrap(),
        TransportRole::Initiator,
        1_000,
    )
    .with_session_generation(3)
}

fn route_for(binding: AdmittedTransportBinding) -> AdmittedTransportRoute {
    TransportBindingRegistry::new().admit(binding).unwrap()
}

#[derive(Clone)]
struct StubEffect;

#[async_trait]
impl VsockEffectPort for StubEffect {
    type Stream = DuplexStream;

    async fn open(
        &self,
        _: &OpaqueEndpointId,
        _: &OpaqueBindingId,
        _: TransportRole,
        _: tokio::time::Instant,
    ) -> Result<Self::Stream, VsockEffectError> {
        Ok(tokio::io::duplex(64).0)
    }

    async fn close(&self, _: Self::Stream) -> Result<(), VsockEffectError> {
        Ok(())
    }
}

#[derive(Clone)]
struct StubStreams;

#[async_trait]
impl NamedStreamPort for StubStreams {
    type Stream = DuplexStream;

    async fn open_named_stream(&self) -> Result<(NamedStreamId, Self::Stream), NamedStreamError> {
        Ok((NamedStreamId::from_core(1), tokio::io::duplex(64).0))
    }

    async fn close_named_stream(&self, _: NamedStreamId) -> Result<(), NamedStreamError> {
        Ok(())
    }
}

/// Evidence from another Zone names a relationship this Guest was never
/// admitted against. The gate refuses it at the authorizing stage, and the
/// refusal happens before the replay ledger is read, so the nonce the refused
/// attempt carried is still the peer's next valid proof.
#[test]
fn foreign_zone_fails_graph_bound_attachment() {
    let key = SessionKey::from_core([7; 32]);
    let guest = identity(42);
    let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 3);
    let route = admit_attach(&admitted_binding(), &evidence_in("work")).unwrap();
    let proof = SessionProof::sign(&key, &guest, nonce_for(21), 3);

    let refusal = authority
        .authenticate_under_binding(
            &route,
            &evidence_in("personal"),
            PeerCid::from_core(42).unwrap(),
            proof.clone(),
        )
        .unwrap_err();
    assert_eq!(refusal.code(), "foreign-zone");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);

    // The graph gate ran first and never touched the replay ledger: the very
    // nonce the refused attempt carried still authenticates afterwards.
    assert_eq!(
        authority
            .authenticate(PeerCid::from_core(42).unwrap(), proof)
            .unwrap()
            .state(),
        SessionState::Ready
    );
}

/// A boot or session identity the graph has moved past is refused before any
/// proof comparison: a stale source generation, a desired revision the fence
/// has advanced past, and a reconnect generation below the relationship's
/// floor each refuse with their own code.
#[test]
fn stale_boot_or_session_identity_fails_graph_bound_attachment() {
    let key = SessionKey::from_core([7; 32]);
    let guest = identity(42);
    let cid = PeerCid::from_core(42).unwrap();
    let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 3);

    let admitted = admitted_binding();
    let route = route_for(admitted.clone());
    let moved_source = TransportAttachEvidence::new(
        ZoneId::parse("work").unwrap(),
        StoreIncarnation::parse("store-a").unwrap(),
        ResourceGeneration::new(2).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        DesiredRevision::INITIAL.try_next().unwrap(),
        ZoneDesiredSequence::INITIAL.try_next().unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    );
    let refusal = authority
        .authenticate_under_binding(
            &route,
            &moved_source,
            cid,
            SessionProof::sign(&key, &guest, nonce_for(22), 3),
        )
        .unwrap_err();
    assert_eq!(refusal.code(), "stale-source-generation");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);

    let advanced_revision = DesiredRevision::INITIAL.try_next().unwrap().try_next().unwrap();
    let advanced = admitted.clone().with_fence(fence().advance_desired_revision(advanced_revision));
    let route = route_for(advanced);
    let refusal = authority
        .authenticate_under_binding(
            &route,
            &evidence_in("work"),
            cid,
            SessionProof::sign(&key, &guest, nonce_for(23), 3),
        )
        .unwrap_err();
    assert_eq!(refusal.code(), "stale-desired-revision");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);

    let raised_floor = ReconnectGeneration::new(5).unwrap();
    let reconnected = admitted.with_fence(fence().raise_minimum_reconnect(raised_floor));
    let route = route_for(reconnected);
    let refusal = authority
        .authenticate_under_binding(
            &route,
            &evidence_in("work"),
            cid,
            SessionProof::sign(&key, &guest, nonce_for(24), 3),
        )
        .unwrap_err();
    assert_eq!(refusal.code(), "stale-reconnect-generation");
    assert_eq!(refusal.stage(), AdmissionStage::Activate);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
}

/// Revocation is a property of the relationship, not of a proof. A reconnect
/// that presents a brand-new nonce over the same key, boot, CID, and
/// generation is admitted while the relationship is admitted and refused once
/// the graph revokes it, even though the legacy proof path alone would still
/// admit it, and a session authenticated before the revocation can no longer
/// realize a transport.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn reconnect_does_not_revive_revoked_binding_authority() {
    let key = SessionKey::from_core([7; 32]);
    let guest = identity(42);
    let cid = PeerCid::from_core(42).unwrap();
    let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 3);
    let binding = admitted_binding();
    let evidence = evidence_in("work");
    let mut service = VsockTransportService::new(StubEffect, StubStreams, guest.clone());
    let route = service.bindings().admit(binding.clone()).unwrap();

    let session = authority
        .authenticate_under_binding(
            &route,
            &evidence,
            cid,
            SessionProof::sign(&key, &guest, nonce_for(25), 3),
        )
        .unwrap();
    assert_eq!(session.state(), SessionState::Ready);
    assert!(session.matches_route(&route));
    assert_eq!(session.generation(), 3);

    service.bindings().revoke(binding.key()).unwrap();

    // RECONNECT: a brand-new proof over a brand-new nonce, same key, boot,
    // CID, and generation. It can only reach the proof comparison by holding
    // a route for the relationship, and the live relationship refuses.
    let reconnect = SessionProof::sign(&key, &guest, nonce_for(26), 3);
    let refusal = service.bindings().route(binding.key(), &evidence).unwrap_err();
    assert_eq!(refusal.code(), "relationship-revoked");
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);

    // The same reconnect over the proof path alone is admitted, which is the
    // authority the graph gate exists to withhold.
    assert_eq!(
        authority
            .authenticate(cid, reconnect)
            .unwrap()
            .state(),
        SessionState::Ready
    );

    let refusal = service
        .open_transport_under_binding(&session, &evidence, open_request())
        .await
        .unwrap_err();
    assert_eq!(refusal.code(), "relationship-revoked");
    assert_eq!(service.phase().await, ServicePhase::Ready);
}

/// The graph gate is an addition to the proof checks, not a replacement for
/// them: a reused nonce and a CID the relationship was not bound to are still
/// refused on a route the graph admits.
#[test]
fn graph_bound_attach_still_enforces_replay_and_cid_bounds() {
    let key = SessionKey::from_core([7; 32]);
    let guest = identity(42);
    let cid = PeerCid::from_core(42).unwrap();
    let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 3);
    let route = admit_attach(&admitted_binding(), &evidence_in("work")).unwrap();
    let nonce = nonce_for(27);

    let session = authority
        .authenticate_under_binding(
            &route,
            &evidence_in("work"),
            cid,
            SessionProof::sign(&key, &guest, nonce, 3),
        )
        .unwrap();
    assert_eq!(session.state(), SessionState::Ready);
    assert_eq!(session.ready().generation(), 3);

    let replay = authority
        .authenticate_under_binding(
            &route,
            &evidence_in("work"),
            cid,
            SessionProof::sign(&key, &guest, nonce, 3),
        )
        .unwrap_err();
    assert_eq!(replay.code(), SessionRejectReason::Replay.code());

    let wrong_cid = authority
        .authenticate_under_binding(
            &route,
            &evidence_in("work"),
            PeerCid::from_core(43).unwrap(),
            SessionProof::sign(&key, &guest, nonce_for(28), 3),
        )
        .unwrap_err();
    assert_eq!(wrong_cid.code(), SessionRejectReason::CidMismatch.code());
    assert_eq!(wrong_cid.stage(), AdmissionStage::Authorize);
}
