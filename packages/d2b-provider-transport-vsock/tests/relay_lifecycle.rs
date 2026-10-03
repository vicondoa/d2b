use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    BindingSlot, BoundedToken, DesiredRevision, EndpointAttachmentKind, EndpointBindingRequest,
    ResourceGeneration, ResourceRef, ResourceUid, StoreIncarnation, ZoneDesiredSequence, ZoneId,
    identity::ReconnectGeneration,
};
use d2b_provider_transport_vsock::{
    AdmittedTransportBinding, CarriageClass, CloseTransportRequest,
    ControlPlaneInjectionRefusal, ControlPlaneRequest, FramedVsockTransport, GuestIdentity,
    NativeGuestRelay, NamedStreamError, NamedStreamId, NamedStreamPort, ObserveTransportRequest,
    OpaqueBindingId, OpaqueEndpointId, OpenTransportRequest, PeerCid, RelayBinding,
    RelayEffectError, RelayEffectPort, RelayObservation, RelayPhase, RelationshipFence,
    SessionAuthority, SessionKey, SessionProof, TransportAttachEvidence, TransportEvent,
    TransportHandle, TransportPhase, TransportRole, VsockEffectError, VsockEffectPort,
    VsockTransportService, admit_attach, classify_carriage,
};
use ring::rand::{SystemRandom, generate};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::sync::Mutex;

fn nonce() -> [u8; 32] {
    generate::<[u8; 32]>(&SystemRandom::new()).unwrap().expose()
}

#[derive(Default)]
struct FakeRelayPort {
    calls: Arc<Mutex<Vec<&'static str>>>,
    fail_reserve: Arc<Mutex<bool>>,
    fail_spawn: Arc<Mutex<bool>>,
    fail_close_listener: Arc<Mutex<bool>>,
    fail_release: Arc<Mutex<bool>>,
    observed: Arc<Mutex<Option<RelayObservation<u64, u64>>>>,
    observe_error: Arc<Mutex<Option<RelayEffectError>>>,
}

#[async_trait]
impl RelayEffectPort for FakeRelayPort {
    type CidReservation = u64;
    type Listener = u64;
    type RelayProcess = u64;

    async fn reserve_cid(
        &self,
        _: &RelayBinding,
    ) -> Result<Self::CidReservation, RelayEffectError> {
        self.calls.lock().await.push("reserve-cid");
        if *self.fail_reserve.lock().await {
            Err(RelayEffectError::CidAuthorityConflict)
        } else {
            Ok(1)
        }
    }

    async fn bind_listener(
        &self,
        _: &RelayBinding,
        _: &Self::CidReservation,
    ) -> Result<Self::Listener, RelayEffectError> {
        self.calls.lock().await.push("bind-listener");
        Ok(2)
    }

    async fn spawn_relay(
        &self,
        _: &RelayBinding,
        _: &Self::Listener,
        _: &Self::CidReservation,
    ) -> Result<Self::RelayProcess, RelayEffectError> {
        self.calls.lock().await.push("spawn-relay");
        if *self.fail_spawn.lock().await {
            Err(RelayEffectError::ProcessUnavailable)
        } else {
            Ok(3)
        }
    }

    async fn close_relay(&self, _: &Self::RelayProcess) -> Result<(), RelayEffectError> {
        self.calls.lock().await.push("close-relay");
        Ok(())
    }

    async fn close_listener(&self, _: &Self::Listener) -> Result<(), RelayEffectError> {
        self.calls.lock().await.push("close-listener");
        if *self.fail_close_listener.lock().await {
            Err(RelayEffectError::CloseUnconfirmed)
        } else {
            Ok(())
        }
    }

    async fn release_cid(&self, _: &Self::CidReservation) -> Result<(), RelayEffectError> {
        self.calls.lock().await.push("release-cid");
        if *self.fail_release.lock().await {
            Err(RelayEffectError::CloseUnconfirmed)
        } else {
            Ok(())
        }
    }

    async fn observe(
        &self,
        _: &RelayBinding,
    ) -> Result<Option<RelayObservation<Self::Listener, Self::RelayProcess>>, RelayEffectError>
    {
        if let Some(error) = *self.observe_error.lock().await {
            return Err(error);
        }
        Ok(self.observed.lock().await.clone())
    }
}

fn binding() -> RelayBinding {
    RelayBinding::new(
        GuestIdentity::new(
            ResourceRef::parse("Guest/guest-a").unwrap(),
            ZoneId::parse("work").unwrap(),
            PeerCid::from_core(42).unwrap(),
            "boot-a",
        )
        .unwrap(),
        [11; 16],
    )
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn finalization_closes_relay_before_releasing_cid_authority() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        let calls = Arc::clone(&port.calls);
        let key = SessionKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());
        relay.start(&session).await.unwrap();
        relay.finalize().await.unwrap();
        assert_eq!(
            *calls.lock().await,
            vec![
                "reserve-cid",
                "bind-listener",
                "spawn-relay",
                "close-relay",
                "close-listener",
                "release-cid",
            ]
        );
        assert_eq!(relay.phase(), RelayPhase::Closed);
    });
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn restart_adopts_only_the_matching_listener_and_process() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let binding = binding();
        let port = FakeRelayPort::default();
        *port.observed.lock().await = Some(RelayObservation {
            binding: binding.clone(),
            listener: 2,
            process: 3,
        });
        let mut relay = NativeGuestRelay::new(port, binding.clone());
        relay.adopt(1).await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Ready);
        relay.finalize().await.unwrap();

        let port = FakeRelayPort::default();
        *port.observed.lock().await = Some(RelayObservation {
            binding: RelayBinding::new(
                GuestIdentity::new(
                    ResourceRef::parse("Guest/other").unwrap(),
                    ZoneId::parse("work").unwrap(),
                    PeerCid::from_core(42).unwrap(),
                    "boot-a",
                )
                .unwrap(),
                [12; 16],
            ),
            listener: 2,
            process: 3,
        });
        let mut relay = NativeGuestRelay::new(port, binding);
        assert_eq!(
            relay.adopt(1).await.unwrap_err(),
            RelayEffectError::RestartMismatch
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
    });
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn reserve_failure_leaves_relay_retryable() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        let fail_reserve = Arc::clone(&port.fail_reserve);
        *port.fail_reserve.lock().await = true;
        let key = SessionKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());
        assert_eq!(
            relay.start(&session).await.unwrap_err(),
            RelayEffectError::CidAuthorityConflict
        );
        assert_eq!(relay.phase(), RelayPhase::Idle);

        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);

        *fail_reserve.lock().await = false;
        relay.start(&session).await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Ready);
        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);
    });
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn failed_cid_release_retains_authority_for_retry() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        let fail_release = Arc::clone(&port.fail_release);
        let key = SessionKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());
        relay.start(&session).await.unwrap();
        *fail_release.lock().await = true;
        assert_eq!(
            relay.finalize().await.unwrap_err(),
            RelayEffectError::CloseUnconfirmed
        );
        assert_eq!(relay.phase(), RelayPhase::Finalizing);
        *fail_release.lock().await = false;
        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);
    });
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn listener_close_failure_keeps_cid_authority_for_retry() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        *port.fail_spawn.lock().await = true;
        *port.fail_close_listener.lock().await = true;
        let calls = Arc::clone(&port.calls);
        let fail_close_listener = Arc::clone(&port.fail_close_listener);
        let key = SessionKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());

        assert_eq!(
            relay.start(&session).await.unwrap_err(),
            RelayEffectError::ProcessUnavailable
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
        assert_eq!(
            *calls.lock().await,
            vec![
                "reserve-cid",
                "bind-listener",
                "spawn-relay",
                "close-listener",
            ]
        );

        *fail_close_listener.lock().await = false;
        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);
        assert_eq!(
            *calls.lock().await,
            vec![
                "reserve-cid",
                "bind-listener",
                "spawn-relay",
                "close-listener",
                "close-listener",
                "release-cid",
            ]
        );
    });
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn restart_observation_error_degrades_without_adoption() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        *port.observe_error.lock().await = Some(RelayEffectError::Transient);
        let mut relay = NativeGuestRelay::new(port, binding());

        assert_eq!(
            relay.adopt(1).await.unwrap_err(),
            RelayEffectError::Transient
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
    });
}

#[test]
fn relay_observation_debug_is_redacted() {
    let observation = RelayObservation {
        binding: binding(),
        listener: 1234_u64,
        process: 5678_u64,
    };
    let rendered = format!("{observation:?}");
    assert!(!rendered.contains("1234"));
    assert!(!rendered.contains("5678"));
    assert!(!rendered.contains("guest-a"));
}

#[derive(Clone)]
struct CarriageEffect {
    peers: Arc<Mutex<Vec<DuplexStream>>>,
}

#[async_trait]
impl VsockEffectPort for CarriageEffect {
    type Stream = DuplexStream;

    async fn open(
        &self,
        _: &OpaqueEndpointId,
        _: &OpaqueBindingId,
        _: TransportRole,
        _: tokio::time::Instant,
    ) -> Result<Self::Stream, VsockEffectError> {
        let (local, peer) = duplex(4096);
        self.peers.lock().await.push(peer);
        Ok(local)
    }

    async fn close(&self, _: Self::Stream) -> Result<(), VsockEffectError> {
        Ok(())
    }
}

#[derive(Clone)]
struct CarriageStreams {
    peers: Arc<Mutex<Vec<DuplexStream>>>,
}

#[async_trait]
impl NamedStreamPort for CarriageStreams {
    type Stream = DuplexStream;

    async fn open_named_stream(&self) -> Result<(NamedStreamId, Self::Stream), NamedStreamError> {
        let (local, peer) = duplex(4096);
        self.peers.lock().await.push(peer);
        Ok((NamedStreamId::from_core(1), local))
    }

    async fn close_named_stream(&self, _: NamedStreamId) -> Result<(), NamedStreamError> {
        Ok(())
    }
}

fn carriage_binding() -> AdmittedTransportBinding {
    let request = EndpointBindingRequest::new(
        ResourceRef::parse("Endpoint/vsock").unwrap(),
        ResourceRef::parse("Process/vsock-agent").unwrap(),
        BindingSlot::parse("transport").unwrap(),
        EndpointAttachmentKind::Attach,
        BoundedToken::parse("vsock-transport").unwrap(),
    )
    .unwrap();
    let key = request
        .key(
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("3f2504e0-4f89-41d3-9a0c-0305e82c3301").unwrap(),
            ResourceUid::parse("9c858901-8a57-4791-81fe-4c455b099bc9").unwrap(),
        )
        .unwrap();
    let fence = RelationshipFence::new(
        StoreIncarnation::parse("store-a").unwrap(),
        DesiredRevision::INITIAL.try_next().unwrap(),
        ZoneDesiredSequence::INITIAL.try_next().unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    );
    AdmittedTransportBinding::new(key, request, fence)
}

fn carriage_evidence() -> TransportAttachEvidence {
    TransportAttachEvidence::new(
        ZoneId::parse("work").unwrap(),
        StoreIncarnation::parse("store-a").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        DesiredRevision::INITIAL.try_next().unwrap(),
        ZoneDesiredSequence::INITIAL.try_next().unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    )
}

/// The named stream is a data plane. A frame that names a privileged control
/// operation, is written through the real vsock framing, read back byte for
/// byte, and handed to the control entry point is refused there, and the live
/// transport it was carried over is unchanged: the bridge counted exactly the
/// carried bytes, the handle is still owned, and observe and close still
/// answer for it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn stream_carriage_cannot_inject_a_control_operation() {
    let binding = carriage_binding();
    let evidence = carriage_evidence();
    let route = admit_attach(&binding, &evidence).unwrap();

    let key = SessionKey::from_core([5; 32]);
    let guest = GuestIdentity::new(
        ResourceRef::parse("Guest/guest-a").unwrap(),
        ZoneId::parse("work").unwrap(),
        PeerCid::from_core(42).unwrap(),
        "boot-a",
    )
    .unwrap();
    let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 4);
    let session = authority
        .authenticate_under_binding(
            &route,
            &evidence,
            PeerCid::from_core(42).unwrap(),
            SessionProof::sign(&key, &guest, nonce(), 4),
        )
        .unwrap();

    let effect = CarriageEffect {
        peers: Arc::new(Mutex::new(Vec::new())),
    };
    let streams = CarriageStreams {
        peers: Arc::new(Mutex::new(Vec::new())),
    };
    let effect_peers = Arc::clone(&effect.peers);
    let stream_peers = Arc::clone(&streams.peers);
    let mut service = VsockTransportService::new(effect, streams, guest);
    service.bindings().admit(binding).unwrap();
    let opened = service
        .open_transport_under_binding(
            &session,
            &evidence,
            OpenTransportRequest::new(
                OpaqueEndpointId::parse("endpoint-a").unwrap(),
                OpaqueBindingId::parse("binding-a").unwrap(),
                TransportRole::Initiator,
                1_000,
            )
            .with_session_generation(4),
        )
        .await
        .unwrap();
    // The first handle this service allocates, so the forged frame carries the
    // real handle rather than a made-up one.
    assert_eq!(opened.transport_handle, TransportHandle::from_core(1));

    let mut forged = Vec::new();
    forged.extend_from_slice(b"close");
    forged.extend_from_slice(b"observe");
    forged.extend_from_slice(&1_u64.to_be_bytes());

    // Those exact bytes go through the real length-prefixed vsock framing.
    let (peer_side, host_side) = duplex(4096);
    let mut framed_writer = FramedVsockTransport::new(peer_side);
    let mut framed_reader = FramedVsockTransport::new(host_side);
    framed_writer.write_frame(&forged).await.unwrap();
    let read_back = framed_reader.read_frame().await.unwrap();
    assert_eq!(read_back, forged);

    // The control entry point is the only place carriage could become
    // privileged, and it refuses: the bytes name an operation, and no byte
    // sequence is a deserializer for a route token.
    assert_eq!(classify_carriage(&read_back), CarriageClass::ControlShaped);
    assert_eq!(
        ControlPlaneRequest::from_carriage(Some(&route), &read_back).unwrap_err(),
        ControlPlaneInjectionRefusal::NotAControlRequest
    );
    assert_eq!(
        ControlPlaneRequest::from_carriage(None, &read_back).unwrap_err(),
        ControlPlaneInjectionRefusal::RouteNotAdmitted
    );
    assert_eq!(
        ControlPlaneRequest::from_carriage(Some(&route), b"ordinary carriage").unwrap_err(),
        ControlPlaneInjectionRefusal::NoOperationDiscriminant
    );

    // The same bytes carried over the live named stream stay carriage: they
    // cross the bridge to the vsock side unchanged and in full.
    let mut named_peer = stream_peers.lock().await.pop().unwrap();
    let mut effect_peer = effect_peers.lock().await.pop().unwrap();
    named_peer.write_all(&forged).await.unwrap();
    let mut crossed = vec![0_u8; forged.len()];
    effect_peer.read_exact(&mut crossed).await.unwrap();
    assert_eq!(crossed, forged);

    let live = service
        .observe_snapshot(ObserveTransportRequest {
            transport_handle: opened.transport_handle,
            include_bytes: true,
        })
        .await
        .unwrap();
    assert_eq!(live.phase, TransportPhase::Acquired);
    let mut events = service
        .observe_transport(ObserveTransportRequest {
            transport_handle: opened.transport_handle,
            include_bytes: true,
        })
        .await
        .unwrap();
    assert_eq!(events.try_recv().unwrap(), TransportEvent::Acquired);

    // Closing both peers completes the bridge, which then reports the counters
    // for exactly the bytes that crossed it.
    drop(named_peer);
    drop(effect_peer);
    let mut observed = live;
    for _ in 0..200 {
        observed = service
            .observe_snapshot(ObserveTransportRequest {
                transport_handle: opened.transport_handle,
                include_bytes: true,
            })
            .await
            .unwrap();
        if observed.phase == TransportPhase::Released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(observed.phase, TransportPhase::Released);
    assert_eq!(observed.bytes_tx, Some(forged.len() as u64));
    assert_eq!(observed.bytes_rx, Some(0));

    // The handle is still owned: close answers for it, and the completed entry
    // still reports the release rather than anything the carriage could have
    // driven.
    service
        .close_transport(CloseTransportRequest {
            transport_handle: opened.transport_handle,
        })
        .await
        .unwrap();
    assert_eq!(
        service
            .observe_transport(ObserveTransportRequest {
                transport_handle: opened.transport_handle,
                include_bytes: false,
            })
            .await
            .unwrap()
            .try_recv()
            .unwrap(),
        TransportEvent::Released
    );
}
