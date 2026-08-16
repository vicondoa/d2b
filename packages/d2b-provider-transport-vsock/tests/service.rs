use async_trait::async_trait;
use d2b_contracts::v3::{ResourceRef, ZoneId};
use d2b_provider_transport_vsock::{
    GuestControlKey, GuestIdentity, NamedStreamError, NamedStreamId, NamedStreamPort,
    OpaqueBindingId, OpaqueEndpointId, OpenTransportRequest, PeerCid, ReadySession, ServiceError,
    SessionAuthority, SessionProof, TransportPhase, TransportRole, VsockEffectError,
    VsockEffectPort, VsockTransportService,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

#[derive(Clone)]
struct FakeEffect {
    peers: Arc<Mutex<Vec<DuplexStream>>>,
    closes: Arc<Mutex<usize>>,
}

#[async_trait]
impl VsockEffectPort for FakeEffect {
    type Stream = DuplexStream;

    async fn open(
        &self,
        _: &OpaqueEndpointId,
        _: &OpaqueBindingId,
        _: TransportRole,
        _: Instant,
    ) -> Result<Self::Stream, VsockEffectError> {
        let (local, peer) = duplex(1024);
        self.peers.lock().unwrap().push(peer);
        Ok(local)
    }

    async fn close(&self, _: Self::Stream) -> Result<(), VsockEffectError> {
        *self.closes.lock().unwrap() += 1;
        Ok(())
    }
}

#[derive(Clone)]
struct FakeStreams {
    next: Arc<Mutex<u64>>,
    closes: Arc<Mutex<usize>>,
    peers: Arc<Mutex<Vec<DuplexStream>>>,
}

#[async_trait]
impl NamedStreamPort for FakeStreams {
    type Stream = DuplexStream;

    async fn open_named_stream(&self) -> Result<(NamedStreamId, Self::Stream), NamedStreamError> {
        let mut next = self.next.lock().unwrap();
        *next += 1;
        let (local, peer) = duplex(1024);
        self.peers.lock().unwrap().push(peer);
        Ok((NamedStreamId::from_core(*next), local))
    }

    async fn close_named_stream(&self, _: NamedStreamId) -> Result<(), NamedStreamError> {
        *self.closes.lock().unwrap() += 1;
        Ok(())
    }
}

fn session() -> ReadySession {
    let identity = GuestIdentity::new(
        ResourceRef::parse("Guest/guest-a").unwrap(),
        ZoneId::parse("work").unwrap(),
        PeerCid::from_core(42).unwrap(),
        "boot-a",
    )
    .unwrap();
    let key = GuestControlKey::from_core([1; 32]);
    let mut authority = SessionAuthority::new(identity.clone(), key.clone(), 1);
    authority
        .authenticate(SessionProof::sign(&key, &identity, [2; 32], 1))
        .unwrap()
}

fn request() -> OpenTransportRequest {
    OpenTransportRequest::new(
        OpaqueEndpointId::parse("endpoint-a").unwrap(),
        OpaqueBindingId::parse("binding-a").unwrap(),
        TransportRole::Initiator,
        1_000,
    )
}

#[test]
fn open_observe_and_close_release_the_bridge() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let effect = FakeEffect {
            peers: Arc::new(Mutex::new(Vec::new())),
            closes: Arc::new(Mutex::new(0)),
        };
        let effect_closes = Arc::clone(&effect.closes);
        let effect_peers = Arc::clone(&effect.peers);
        let streams = FakeStreams {
            next: Arc::new(Mutex::new(0)),
            closes: Arc::new(Mutex::new(0)),
            peers: Arc::new(Mutex::new(Vec::new())),
        };
        let stream_closes = Arc::clone(&streams.closes);
        let stream_peers = Arc::clone(&streams.peers);
        let service = VsockTransportService::new(effect, streams);
        let opened = service.open_transport(&session(), request()).await.unwrap();
        let mut effect_peer = effect_peers.lock().unwrap().pop().unwrap();
        let mut stream_peer = stream_peers.lock().unwrap().pop().unwrap();
        effect_peer.write_all(b"guest-to-core").await.unwrap();
        let mut received = [0_u8; 13];
        stream_peer.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"guest-to-core");
        stream_peer.write_all(b"core-to-guest").await.unwrap();
        let mut received = [0_u8; 13];
        effect_peer.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"core-to-guest");
        let observed = service
            .observe_transport(d2b_provider_transport_vsock::ObserveTransportRequest {
                transport_handle: opened.transport_handle,
                include_bytes: true,
            })
            .await
            .unwrap();
        assert_eq!(observed.phase, TransportPhase::Acquired);
        service
            .close_transport(d2b_provider_transport_vsock::CloseTransportRequest {
                transport_handle: opened.transport_handle,
            })
            .await
            .unwrap();
        assert_eq!(*effect_closes.lock().unwrap(), 1);
        assert_eq!(*stream_closes.lock().unwrap(), 1);
        assert_eq!(
            service
                .observe_transport(d2b_provider_transport_vsock::ObserveTransportRequest {
                    transport_handle: opened.transport_handle,
                    include_bytes: false,
                })
                .await
                .unwrap()
                .phase,
            TransportPhase::Released
        );
        assert_eq!(
            service
                .observe_transport(d2b_provider_transport_vsock::ObserveTransportRequest {
                    transport_handle: d2b_provider_transport_vsock::TransportHandle::from_core(999),
                    include_bytes: false,
                })
                .await
                .unwrap_err(),
            ServiceError::UnknownTransportHandle
        );
    });
}
