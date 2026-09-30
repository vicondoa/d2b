use d2b_contracts_resource::v3::{
    AdmissionStage, BindingSlot, BoundedToken, DesiredRevision, EndpointAttachmentKind,
    EndpointBindingRequest, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};
use d2b_provider_transport_unix::{
    AdmittedTransportBinding, BrokerRole, ControlPlaneInjectionRefusal, ControlPlaneRequest,
    ExpectedPeer, KernelPeerPin, OpenTransportRequest, PortalError, RelationshipFence,
    RelationshipPhase, RouteClass, SocketKind, TransportAttachEvidence, TransportBindingRefusal,
    TransportBindingRegistry, TransportControlOperation, TransportObservation, TransportPortal,
    TransportRequestBinding, TransportService, admit_attach, MAX_ADMITTED_TRANSPORT_BINDINGS,
};
use rustix::{
    fd::AsFd,
    fs::fcntl_getfd,
    io::FdFlags,
    net::{
        AddressFamily, SocketFlags, SocketType, socket, socketpair,
        sockopt::{get_socket_passcred, get_socket_peercred, get_socket_type},
    },
    process::{getgid, getuid},
};

fn binding() -> TransportRequestBinding {
    TransportRequestBinding::new(
        ZoneId::parse("local-root").expect("zone"),
        ResourceRef::parse("Provider/system-core").expect("subject"),
        BrokerRole::ZoneController,
    )
}

fn pair(kind: SocketType) -> (rustix::fd::OwnedFd, rustix::fd::OwnedFd) {
    socketpair(
        AddressFamily::UNIX,
        kind,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .expect("socketpair")
}

// --- Graph-bound relationship fixtures -------------------------------------
//
// A local transport attaches under an admitted `EndpointBinding`
// relationship, not under a caller-supplied peer policy. The relationship
// carries the source, consumer, and slot that identify it, the fence its
// evidence is measured against, and the kernel peer it is pinned to.

const SOURCE_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
const CONSUMER_UID: &str = "2b4e28ba-2fa1-41d2-883f-0016d3cca428";
const ADMITTED_SOURCE_GENERATION: u64 = 4;
const ADMITTED_CONSUMER_GENERATION: u64 = 5;
const ADMITTED_RECONNECT: u64 = 3;

fn desired_revision(step: u64) -> DesiredRevision {
    (1..=step).fold(DesiredRevision::INITIAL, |revision, _| {
        revision.try_next().expect("desired revision")
    })
}

fn desired_sequence(step: u64) -> ZoneDesiredSequence {
    (1..=step).fold(ZoneDesiredSequence::INITIAL, |sequence, _| {
        sequence.try_next().expect("desired sequence")
    })
}

fn admitted_in_slot(
    attachment: EndpointAttachmentKind,
    slot: &str,
) -> AdmittedTransportBinding {
    let request = EndpointBindingRequest::new(
        ResourceRef::parse("Endpoint/portal").expect("endpoint"),
        ResourceRef::parse("Process/session").expect("consumer"),
        BindingSlot::parse(slot).expect("slot"),
        attachment,
        BoundedToken::parse("session-transport").expect("purpose"),
    )
    .expect("endpoint binding request");
    let key = request
        .key(
            ZoneId::parse("local-root").expect("zone"),
            ResourceUid::parse(SOURCE_UID).expect("source uid"),
            ResourceUid::parse(CONSUMER_UID).expect("consumer uid"),
        )
        .expect("binding key");
    AdmittedTransportBinding::new(
        key,
        request,
        RelationshipFence::new(
            StoreIncarnation::parse("store-one").expect("incarnation"),
            desired_revision(1),
            desired_sequence(1),
            ResourceGeneration::new(ADMITTED_SOURCE_GENERATION).expect("source generation"),
            ResourceGeneration::new(ADMITTED_CONSUMER_GENERATION).expect("consumer generation"),
            ReconnectGeneration::new(ADMITTED_RECONNECT).expect("reconnect generation"),
        ),
    )
}

fn admitted(attachment: EndpointAttachmentKind) -> AdmittedTransportBinding {
    admitted_in_slot(attachment, "primary")
}

fn evidence_with(
    zone: &str,
    store: &str,
    source_generation: u64,
    consumer_generation: u64,
    revision: u64,
    sequence: u64,
    reconnect: u64,
) -> TransportAttachEvidence {
    TransportAttachEvidence::new(
        ZoneId::parse(zone).expect("zone"),
        StoreIncarnation::parse(store).expect("incarnation"),
        ResourceGeneration::new(source_generation).expect("source generation"),
        ResourceGeneration::new(consumer_generation).expect("consumer generation"),
        desired_revision(revision),
        desired_sequence(sequence),
        ReconnectGeneration::new(reconnect).expect("reconnect generation"),
    )
}

fn evidence() -> TransportAttachEvidence {
    evidence_with(
        "local-root",
        "store-one",
        ADMITTED_SOURCE_GENERATION,
        ADMITTED_CONSUMER_GENERATION,
        1,
        1,
        ADMITTED_RECONNECT,
    )
}

fn stream_request() -> OpenTransportRequest {
    OpenTransportRequest::new(SocketKind::Stream, RouteClass::LocalPortal, false)
}

/// Assert that one piece of evidence is refused by the attach gate, and that
/// the refusal carries the enforcing stage and reason an operator needs, and
/// that the refusal opened no transport.
fn assert_attach_refused(
    service: &TransportService,
    binding: &AdmittedTransportBinding,
    evidence: &TransportAttachEvidence,
    stage: AdmissionStage,
    reason: RefusalReason,
    code: &'static str,
) {
    let refusal = admit_attach(binding, evidence).expect_err("evidence must not attach");
    assert_eq!(refusal.code(), code);
    assert_eq!(refusal.stage(), stage);
    assert_eq!(refusal.reason(), reason);
    let (accepted, _peer) = pair(SocketType::STREAM);
    assert_eq!(
        service
            .open_under_binding(binding, evidence, stream_request(), accepted)
            .expect_err("a refused attach must not open a transport"),
        PortalError::RelationshipRefused
    );
    assert_eq!(
        service.portal().open_count(),
        0,
        "a refused attach must not open a transport"
    );
}

#[test]
fn accepted_fd_peer_and_request_context_are_bound_once() {
    let (accepted, _peer) = pair(SocketType::SEQPACKET);
    let portal = TransportPortal::new();
    let opened = portal
        .open(
            binding(),
            OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, true),
            accepted,
        )
        .expect("accepted request");

    assert_eq!(opened.descriptor().socket_kind(), SocketKind::Seqpacket);
    assert!(opened.descriptor().attachments_enabled());
    assert_eq!(
        get_socket_type(opened.transport_fd()).expect("socket type"),
        SocketType::SEQPACKET
    );
    assert!(
        get_socket_passcred(opened.transport_fd()).expect("passcred"),
        "seqpacket transport must accept only kernel credentials"
    );
    assert!(
        fcntl_getfd(opened.transport_fd())
            .expect("fd flags")
            .contains(FdFlags::CLOEXEC),
        "transport fd must not survive exec"
    );
    assert!(portal.close(opened.handle()).is_ok());
    assert!(portal.close(opened.handle()).is_ok(), "close is idempotent");
}

#[test]
fn peer_credentials_are_kernel_bound_and_wrong_peers_are_rejected() {
    let portal = TransportPortal::new();
    let expected = ExpectedPeer::new(getuid().as_raw(), getgid().as_raw());
    let (accepted, _peer) = pair(SocketType::SEQPACKET);
    let opened = portal
        .open(
            binding().with_expected_peer(expected),
            OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
            accepted,
        )
        .expect("current process peer credentials");
    portal.close(opened.handle()).expect("close accepted peer");

    let (wrong_peer, _peer) = pair(SocketType::SEQPACKET);
    let wrong = ExpectedPeer::new(expected.uid().saturating_add(1), expected.gid());
    assert_eq!(
        portal
            .open(
                binding().with_expected_peer(wrong),
                OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
                wrong_peer,
            )
            .expect_err("mismatched kernel peer must fail closed"),
        PortalError::PeerCredentials
    );
}

#[test]
fn route_class_and_socket_kind_refuse_fd_substitution() {
    let portal = TransportPortal::new();
    let (stream, _peer) = pair(SocketType::STREAM);
    assert_eq!(
        portal
            .open(
                binding(),
                OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
                stream,
            )
            .expect_err("stream cannot satisfy seqpacket request"),
        PortalError::SocketKindMismatch
    );

    let (seqpacket, _peer) = pair(SocketType::SEQPACKET);
    assert_eq!(
        portal
            .open(
                binding(),
                OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::ZoneLink, true),
                seqpacket,
            )
            .expect_err("ZoneLink fd grants are forbidden"),
        PortalError::AttachmentPolicyConflict
    );

    let (stream, _peer) = pair(SocketType::STREAM);
    assert_eq!(
        portal
            .open(
                binding(),
                OpenTransportRequest::new(SocketKind::Stream, RouteClass::LocalPortal, true),
                stream,
            )
            .expect_err("stream cannot carry SCM_RIGHTS"),
        PortalError::AttachmentPolicyConflict
    );
}

#[test]
fn finalization_retires_only_portal_owned_monitor_fds() {
    let portal = TransportPortal::new();
    let (accepted, peer) = pair(SocketType::SEQPACKET);
    let opened = portal
        .open(
            binding(),
            OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
            accepted,
        )
        .expect("open transport");
    let handle = opened.handle();
    drop(opened);

    portal.finalize();
    assert_eq!(portal.observe(handle), Err(PortalError::UnknownHandle));
    assert_eq!(
        get_socket_type(peer.as_fd()).expect("peer remains caller-owned"),
        SocketType::SEQPACKET
    );
}

#[test]
fn portal_refuses_foreign_or_stale_handles_and_observes_disconnects() {
    let portal = TransportPortal::new();
    let foreign_portal = TransportPortal::new();
    let (accepted, peer) = pair(SocketType::SEQPACKET);
    let opened = portal
        .open(
            binding(),
            OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
            accepted,
        )
        .expect("open transport");
    let handle = opened.handle();

    assert_eq!(
        foreign_portal.close(handle),
        Err(PortalError::UnknownHandle),
        "a handle cannot authorize another portal"
    );
    drop(peer);
    assert_eq!(
        portal.observe(handle),
        Ok(d2b_provider_transport_unix::TransportObservation::PeerDisconnected)
    );
    portal.close(handle).expect("owner closes handle");
    assert_eq!(
        portal.observe(handle),
        Err(PortalError::UnknownHandle),
        "a finalized handle cannot be replayed"
    );
}

#[test]
fn open_refuses_a_full_handle_table_then_recovers_after_close() {
    let portal = TransportPortal::new();
    let mut opened = Vec::new();
    for _ in 0..256 {
        let (accepted, _peer) = pair(SocketType::SEQPACKET);
        opened.push(
            portal
                .open(
                    binding(),
                    OpenTransportRequest::new(
                        SocketKind::Seqpacket,
                        RouteClass::LocalPortal,
                        false,
                    ),
                    accepted,
                )
                .expect("open within table bound"),
        );
    }

    let (overflow, peer) = pair(SocketType::SEQPACKET);
    assert_eq!(
        portal
            .open(
                binding(),
                OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
                overflow,
            )
            .expect_err("full table must refuse"),
        PortalError::HandleTableFull
    );
    drop(peer);

    let released = opened.pop().expect("occupied table");
    portal.close(released.handle()).expect("release one handle");

    let (accepted, _peer) = pair(SocketType::SEQPACKET);
    let recovered = portal
        .open(
            binding(),
            OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, false),
            accepted,
        )
        .expect("close must free a table slot");
    assert!(portal.close(recovered.handle()).is_ok());
}

#[test]
fn foreign_zone_evidence_fails_attachment() {
    let service = TransportService::new();
    let binding = admitted(EndpointAttachmentKind::Connect);
    service
        .bindings()
        .admit(binding.clone())
        .expect("admitted relationship");

    // Evidence from another Zone is not evidence about this relationship,
    // whatever generations and revisions it carries.
    let foreign = evidence_with(
        "other-zone",
        "store-one",
        ADMITTED_SOURCE_GENERATION,
        ADMITTED_CONSUMER_GENERATION,
        1,
        1,
        ADMITTED_RECONNECT,
    );
    assert_attach_refused(
        &service,
        &binding,
        &foreign,
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
        "foreign-zone",
    );
    assert_eq!(
        PortalError::RelationshipRefused.to_string(),
        "relationship-refused"
    );
}

#[test]
fn stale_source_generation_and_stale_boot_session_fail_attachment() {
    let service = TransportService::new();
    let binding = admitted(EndpointAttachmentKind::Connect);
    service
        .bindings()
        .admit(binding.clone())
        .expect("admitted relationship");

    // An older source generation: this is not the source the relationship
    // was admitted against.
    assert_attach_refused(
        &service,
        &binding,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION - 1,
            ADMITTED_CONSUMER_GENERATION,
            1,
            1,
            ADMITTED_RECONNECT,
        ),
        AdmissionStage::Admit,
        RefusalReason::StaleAuthority,
        "stale-source-generation",
    );

    // An older consumer generation: this is not the session the relationship
    // was admitted for.
    assert_attach_refused(
        &service,
        &binding,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION - 1,
            1,
            1,
            ADMITTED_RECONNECT,
        ),
        AdmissionStage::Admit,
        RefusalReason::StaleAuthority,
        "stale-consumer-generation",
    );

    // A different store incarnation is a different boot, not a newer one.
    assert_attach_refused(
        &service,
        &binding,
        &evidence_with(
            "local-root",
            "store-two",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION,
            1,
            1,
            ADMITTED_RECONNECT,
        ),
        AdmissionStage::Authorize,
        RefusalReason::StoreIncarnationMismatch,
        "store-incarnation-mismatch",
    );

    // A desired mutation the relationship has not caught up with.
    assert_attach_refused(
        &service,
        &binding,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION,
            2,
            1,
            ADMITTED_RECONNECT,
        ),
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        "stale-desired-revision",
    );
    assert_attach_refused(
        &service,
        &binding,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION,
            1,
            2,
            ADMITTED_RECONNECT,
        ),
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        "stale-desired-sequence",
    );

    // A reconnect ordinal the relationship has not reached.
    assert_attach_refused(
        &service,
        &binding,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION,
            1,
            1,
            ADMITTED_RECONNECT - 1,
        ),
        AdmissionStage::Activate,
        RefusalReason::StaleAuthority,
        "stale-reconnect-generation",
    );

    // The graph moves the relationship into its draining phase: outstanding
    // use is driven closed and nothing new attaches.
    service
        .bindings()
        .drain(binding.key())
        .expect("drain the admitted relationship");
    let draining = service
        .bindings()
        .binding(binding.key())
        .expect("a drained relationship stays observable");
    assert_eq!(draining.fence().phase(), RelationshipPhase::Draining);
    assert_attach_refused(
        &service,
        &draining,
        &evidence(),
        AdmissionStage::Drain,
        RefusalReason::UnprovenEffect,
        "relationship-draining",
    );

    // A fence the graph has moved on: only evidence at the new fence decides
    // the relationship, and the checks run in their declared order.
    let moved = TransportService::new();
    let advanced = admitted(EndpointAttachmentKind::Connect).with_fence(
        admitted(EndpointAttachmentKind::Connect)
            .fence()
            .clone()
            .advance_desired_revision(desired_revision(2))
            .advance_sequence(desired_sequence(2))
            .raise_minimum_reconnect(ReconnectGeneration::new(9).expect("reconnect generation")),
    );
    moved
        .bindings()
        .admit(advanced.clone())
        .expect("admitted relationship");
    assert_attach_refused(
        &moved,
        &advanced,
        &evidence(),
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        "stale-desired-revision",
    );
    assert_attach_refused(
        &moved,
        &advanced,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION,
            2,
            1,
            ADMITTED_RECONNECT,
        ),
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        "stale-desired-sequence",
    );
    assert_attach_refused(
        &moved,
        &advanced,
        &evidence_with(
            "local-root",
            "store-one",
            ADMITTED_SOURCE_GENERATION,
            ADMITTED_CONSUMER_GENERATION,
            2,
            2,
            3,
        ),
        AdmissionStage::Activate,
        RefusalReason::StaleAuthority,
        "stale-reconnect-generation",
    );
    let (accepted, _peer) = pair(SocketType::STREAM);
    let opened = moved
        .open_under_binding(
            &advanced,
            &evidence_with(
                "local-root",
                "store-one",
                ADMITTED_SOURCE_GENERATION,
                ADMITTED_CONSUMER_GENERATION,
                2,
                2,
                9,
            ),
            stream_request(),
            accepted,
        )
        .expect("evidence at the advanced fence attaches");
    assert!(moved.portal().close(opened.handle()).is_ok());
}

#[test]
fn reconnect_does_not_revive_revoked_binding_authority() {
    let service = TransportService::new();
    let binding = admitted(EndpointAttachmentKind::Connect);
    service
        .bindings()
        .admit(binding.clone())
        .expect("admitted relationship");
    let (accepted, _peer) = pair(SocketType::STREAM);
    let opened = service
        .open_under_binding(&binding, &evidence(), stream_request(), accepted)
        .expect("graph-bound open");
    let handle = opened.handle();

    service
        .bindings()
        .revoke(binding.key())
        .expect("revoke the admitted relationship");

    // A genuine reconnect: fresh evidence built after the revoke, carrying a
    // higher reconnect ordinal than the relationship ever admitted.
    let reconnect = evidence_with(
        "local-root",
        "store-one",
        ADMITTED_SOURCE_GENERATION,
        ADMITTED_CONSUMER_GENERATION,
        1,
        1,
        ADMITTED_RECONNECT + 1,
    );
    let revoked = service
        .bindings()
        .binding(binding.key())
        .expect("a revoked relationship stays observable");
    assert_eq!(revoked.fence().phase(), RelationshipPhase::Revoked);
    let refusal =
        admit_attach(&revoked, &reconnect).expect_err("a revoked relationship admits nothing");
    assert_eq!(refusal.code(), "relationship-revoked");
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);

    // Presenting the value admitted before the revoke does not get around the
    // registry: the service attaches against what it currently admits.
    let (accepted, _peer) = pair(SocketType::STREAM);
    assert_eq!(
        service
            .open_under_binding(&binding, &reconnect, stream_request(), accepted)
            .expect_err("a reconnect must not revive revoked authority"),
        PortalError::RelationshipRefused
    );

    // The refused reconnect opened nothing, and the transport the
    // relationship opened before the revoke is untouched: still attributable,
    // still monitored, still closable.
    assert_eq!(
        service.portal().open_count(),
        1,
        "the refused reconnect opened nothing"
    );
    assert_eq!(
        service.portal().relationship(handle),
        Ok(Some(binding.key().clone()))
    );
    assert!(service.portal().close(handle).is_ok());
    assert_eq!(
        service.portal().relationship(handle),
        Err(PortalError::UnknownHandle)
    );
}

#[test]
fn stream_carriage_cannot_inject_a_control_operation() {
    let service = TransportService::new();
    let binding = admitted(EndpointAttachmentKind::Connect);
    let route = service
        .bindings()
        .admit(binding.clone())
        .expect("admitted relationship");
    // A BLOCKING stream pair, so the round trip below observes real carriage
    // rather than racing a non-blocking read. The portal sets close-on-exec
    // on the descriptor it is given either way.
    let (accepted, peer) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .expect("socketpair");
    let opened = service
        .open_under_binding(&binding, &evidence(), stream_request(), accepted)
        .expect("graph-bound open");
    let handle = opened.handle();

    // A forged control frame built from real field values: a real
    // discriminant naming `Close`, the live handle in its only observable
    // rendering (an opaque handle has no byte accessor by design, which is
    // exactly why a forger cannot reconstruct one), and a plausible payload.
    let mut forged = Vec::new();
    forged.extend_from_slice(b"D2B-CTL\x01\x00");
    forged.extend_from_slice(b"close\x00");
    forged.extend_from_slice(format!("{handle:?}").as_bytes());
    forged.extend_from_slice(&[0x5a; 16]);

    // The forged frame is written into the real peer end and read back out of
    // the transport descriptor the portal handed the caller, so the bytes the
    // control entry point is asked about really travelled the opened stream.
    let written = rustix::io::write(&peer, &forged).expect("write the forged control frame");
    assert_eq!(written, forged.len(), "the real stream took the whole frame");
    let mut carried = Vec::new();
    let mut chunk = [0_u8; 64];
    while carried.len() < forged.len() {
        let read = match rustix::io::read(opened.transport_fd(), &mut chunk) {
            Ok(read) => read,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => panic!("read the carried bytes back: {error}"),
        };
        assert!(read > 0, "the real stream must return what was written");
        carried.extend_from_slice(&chunk[..read]);
    }
    assert_eq!(carried, forged, "the stream carried the exact forged bytes");

    // Carriage that names no operation at all is data by construction.
    assert!(matches!(
        ControlPlaneRequest::from_carriage(Some(&route), b"GET /health HTTP/1.0\r\n\r\n"),
        Err(ControlPlaneInjectionRefusal::NoOperationDiscriminant)
    ));

    // The exact bytes that came off the real stream are refused as a control
    // request, and the same bytes with no admitted route behind them are
    // refused even earlier.
    assert!(matches!(
        ControlPlaneRequest::from_carriage(Some(&route), &carried),
        Err(ControlPlaneInjectionRefusal::NotAControlRequest)
    ));
    assert!(matches!(
        ControlPlaneRequest::from_carriage(None, &carried),
        Err(ControlPlaneInjectionRefusal::RouteNotAdmitted)
    ));
    assert!(
        ControlPlaneRequest::from_carriage(Some(&route), &carried).is_err(),
        "no byte sequence can become a privileged control request"
    );

    // A control request against the live route is constructible, because its
    // token comes from the admitted route rather than from the stream.
    let issued = ControlPlaneRequest::issue(
        TransportControlOperation::Close,
        route.control_token(),
        handle,
    );
    assert_eq!(issued.operation(), TransportControlOperation::Close);
    assert_eq!(issued.handle(), handle);

    // Live state is unchanged by the refused injection: the transport is
    // still the one the relationship admitted, still monitored, its
    // relationship still admitted, and still closable.
    assert_eq!(
        service.portal().observe(handle),
        Ok(TransportObservation::Pending)
    );
    assert_eq!(
        service.portal().relationship(handle),
        Ok(Some(binding.key().clone()))
    );
    assert_eq!(
        service
            .bindings()
            .binding(binding.key())
            .map(|live| live.fence().phase()),
        Some(RelationshipPhase::Admitted)
    );
    assert!(service.portal().close(handle).is_ok());
    assert_eq!(
        service.portal().observe(handle),
        Err(PortalError::UnknownHandle)
    );
}

#[test]
fn relationship_pin_not_independently_supplied() {
    let service = TransportService::new();
    let real = KernelPeerPin::new(getuid().as_raw(), getgid().as_raw());
    let wrong = KernelPeerPin::new(real.uid().saturating_add(1), real.gid());

    // The caller's own request binding carries no peer policy, and the
    // graph-bound open has no argument in which one could be placed: the pin
    // comes from the relationship, or the open is refused.
    assert!(binding().expected_peer().is_none());

    let mismatched = admitted(EndpointAttachmentKind::Connect).with_kernel_peer_pin(wrong);
    service
        .bindings()
        .admit(mismatched.clone())
        .expect("admitted relationship");
    let (accepted, _peer) = pair(SocketType::STREAM);
    assert_eq!(
        service
            .open_under_binding(&mismatched, &evidence(), stream_request(), accepted)
            .expect_err("a relationship pinned to another peer must not attach"),
        PortalError::PeerPolicyMismatch
    );
    assert_eq!(
        PortalError::PeerPolicyMismatch.to_string(),
        "peer-policy-mismatch"
    );

    // A socket the kernel will not attribute to any peer is a different
    // refusal: the transport could not learn who this is, rather than
    // learning that this is not the peer the relationship admits.
    let unattributed = socket(AddressFamily::UNIX, SocketType::STREAM, None)
        .expect("unconnected unix socket");
    assert!(
        get_socket_peercred(&unattributed).is_err(),
        "an unconnected socket exposes no peer credentials"
    );
    assert_eq!(
        service
            .open_under_binding(&mismatched, &evidence(), stream_request(), unattributed)
            .expect_err("an unattributable peer must not attach"),
        PortalError::PeerCredentials
    );
    assert_eq!(
        PortalError::PeerCredentials.to_string(),
        "peer-credentials-unavailable"
    );

    // The same socket, under a relationship pinned to the peer the kernel
    // actually reports, attaches.
    let matching = admitted_in_slot(EndpointAttachmentKind::Connect, "secondary")
        .with_kernel_peer_pin(real);
    service
        .bindings()
        .admit(matching.clone())
        .expect("admitted relationship");
    let (accepted, _peer) = pair(SocketType::STREAM);
    let opened = service
        .open_under_binding(&matching, &evidence(), stream_request(), accepted)
        .expect("the relationship's own peer attaches");
    assert!(service.portal().close(opened.handle()).is_ok());
}

#[test]
fn binding_registry_is_bounded_and_finalized_keys_are_not_reissued() {
    // The production ceiling with its shape intact, narrowed so the overflow
    // is cheap to reach. No caller may raise it back.
    const NARROW: usize = 8;
    let registry = TransportBindingRegistry::with_ceiling(NARROW);
    assert_eq!(registry.ceiling(), NARROW);
    assert_eq!(
        TransportBindingRegistry::new().ceiling(),
        MAX_ADMITTED_TRANSPORT_BINDINGS
    );
    assert_eq!(
        TransportBindingRegistry::with_ceiling(usize::MAX).ceiling(),
        MAX_ADMITTED_TRANSPORT_BINDINGS,
        "a caller may narrow the registry but never raise it"
    );

    for index in 0..NARROW {
        registry
            .admit(admitted_in_slot(
                EndpointAttachmentKind::Connect,
                &format!("slot-{index}"),
            ))
            .expect("admit within the frozen ceiling");
    }
    assert_eq!(registry.len(), NARROW);
    let overflow = registry
        .admit(admitted_in_slot(
            EndpointAttachmentKind::Connect,
            "slot-overflow",
        ))
        .expect_err("the frozen ceiling refuses rather than growing");
    assert_eq!(overflow, TransportBindingRefusal::RegistryFull);
    assert_eq!(overflow.code(), "binding-registry-full");
    assert_eq!(registry.len(), NARROW, "a refused admission changed nothing");

    // A live key is not silently replaced by a second admission, and
    // finalization retires every relationship it held: nothing it held
    // survives as authority.
    let live = admitted_in_slot(EndpointAttachmentKind::Connect, "slot-0");
    assert_eq!(
        registry
            .admit(live.clone())
            .expect_err("a live key is not re-admitted"),
        TransportBindingRefusal::AlreadyAdmitted
    );
    registry.finalize();
    assert!(registry.is_empty());
    assert!(registry.binding(live.key()).is_none());
    assert_eq!(
        registry.len(),
        0,
        "finalization retires every admitted relationship"
    );

    // Putting a relationship back is an explicit admission, never a leftover.
    registry
        .admit(live.clone())
        .expect("an explicit admission re-establishes the relationship");
    assert_eq!(registry.len(), 1);
    registry.finalize();

    // A revoked and forgotten key carries no authority of its own: nothing
    // resurrects it, and the value the caller still holds cannot open one.
    let service = TransportService::new();
    let revocable = admitted(EndpointAttachmentKind::Connect);
    service
        .bindings()
        .admit(revocable.clone())
        .expect("admitted relationship");
    service
        .bindings()
        .revoke(revocable.key())
        .expect("revoke");
    service
        .bindings()
        .forget(revocable.key())
        .expect("forget");
    assert!(service.bindings().is_empty());
    let (accepted, _peer) = pair(SocketType::STREAM);
    assert_eq!(
        service
            .open_under_binding(&revocable, &evidence(), stream_request(), accepted)
            .expect_err("a forgotten relationship is never silently resurrected"),
        PortalError::BindingNotAdmitted
    );
    assert_eq!(
        PortalError::BindingNotAdmitted.to_string(),
        "binding-not-admitted"
    );
    assert_eq!(
        service.portal().open_count(),
        0,
        "a refused open owns no descriptor"
    );

    // Only an explicit admission re-establishes authority.
    service
        .bindings()
        .admit(revocable.clone())
        .expect("explicit re-admission");
    let (accepted, _peer) = pair(SocketType::STREAM);
    let opened = service
        .open_under_binding(&revocable, &evidence(), stream_request(), accepted)
        .expect("an explicitly re-admitted relationship attaches");
    assert!(service.portal().close(opened.handle()).is_ok());
}
