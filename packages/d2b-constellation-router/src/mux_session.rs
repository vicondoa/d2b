//! Post-handshake stream-mux driver (ADR 0032).
//!
//! [`PeerSession`](crate::PeerSession) proves that the byte transport completed
//! protocol/codec negotiation. [`MuxSession`] is the next boundary: every
//! post-handshake stream frame is applied to the pure
//! [`d2b_constellation_core::StreamMux`] state machine before it is exposed
//! to a caller (inbound) or written to the peer (outbound). This is the
//! fail-closed runtime seam between semantic frames and transport I/O.

use d2b_constellation_core::{
    CapabilitySet, ConstellationError, ConstellationFrame, OpaquePayload, StreamChannel,
    StreamClose, StreamCloseReason, StreamData, StreamId, StreamMux, StreamOpen, StreamResume,
};
use d2b_constellation_provider::error::ProviderResult;
use d2b_constellation_provider::provider::ProtocolCodec;

use crate::PeerSession;

/// A peer session with stream-mux accounting attached.
pub struct MuxSession<C: ProtocolCodec> {
    peer: PeerSession<C>,
    mux: StreamMux,
    capabilities: CapabilitySet,
}

impl<C: ProtocolCodec> MuxSession<C> {
    /// Wrap a successfully-handshaken peer session with a fresh mux.
    pub fn new(peer: PeerSession<C>) -> Self {
        let capabilities = peer.negotiated().capabilities.capabilities.clone();
        Self {
            peer,
            mux: StreamMux::new(),
            capabilities,
        }
    }

    /// Wrap a peer with an explicit mux (used by tests and future restored
    /// sessions).
    pub fn with_mux(peer: PeerSession<C>, mux: StreamMux) -> Self {
        let capabilities = peer.negotiated().capabilities.capabilities.clone();
        Self {
            peer,
            mux,
            capabilities,
        }
    }

    /// Borrow the mux state for reconciliation/observability.
    pub fn mux(&self) -> &StreamMux {
        &self.mux
    }

    /// Open a stream initiated by this endpoint: validate/register it locally,
    /// then send the `StreamOpen` frame to the peer.
    pub async fn open_stream(&mut self, open: StreamOpen) -> ProviderResult<()> {
        self.mux.open_with_capabilities(&open, &self.capabilities)?;
        self.peer.send(&ConstellationFrame::StreamOpen(open)).await
    }

    /// Grant inbound credit to the peer and send the resulting flow frame.
    pub async fn grant_inbound(&mut self, stream: &StreamId, credits: u32) -> ProviderResult<()> {
        let flow = self.mux.grant_inbound(stream, credits)?;
        self.peer.send(&ConstellationFrame::StreamFlow(flow)).await
    }

    /// Send one bounded data chunk after spending outbound credit.
    pub async fn send_data(
        &mut self,
        stream: &StreamId,
        channel: StreamChannel,
        data: OpaquePayload,
    ) -> ProviderResult<()> {
        let sequence = self.mux.reserve_send(stream, channel)?;
        let frame = StreamData {
            stream: stream.clone(),
            sequence,
            channel,
            cursor: None,
            data,
        };
        self.peer.send(&ConstellationFrame::StreamData(frame)).await
    }

    /// Close a stream locally, then tell the peer. Idempotency is owned by the
    /// mux: a double-close is rejected fail-closed.
    pub async fn close_stream(
        &mut self,
        stream: &StreamId,
        reason: StreamCloseReason,
    ) -> ProviderResult<()> {
        let close = StreamClose {
            stream: stream.clone(),
            reason,
        };
        self.mux.close(&close)?;
        self.peer
            .send(&ConstellationFrame::StreamClose(close))
            .await
    }

    /// Send a stream resume request after validating the stream/cursor shape.
    pub async fn resume_stream(&mut self, resume: StreamResume) -> ProviderResult<()> {
        self.mux.validate_resume(&resume)?;
        self.peer
            .send(&ConstellationFrame::StreamResume(resume))
            .await
    }

    /// Cancel a stream idempotently and notify the peer only for the first
    /// cancel transition.
    pub async fn cancel_stream(&mut self, stream: &StreamId) -> ProviderResult<bool> {
        let first_cancel = self.mux.cancel(stream)?;
        if first_cancel {
            self.peer
                .send(&ConstellationFrame::StreamClose(StreamClose {
                    stream: stream.clone(),
                    reason: StreamCloseReason::Cancelled,
                }))
                .await?;
        }
        Ok(first_cancel)
    }

    /// Receive one frame and apply the mux state transition before exposing it.
    pub async fn recv(&mut self) -> ProviderResult<ConstellationFrame> {
        let frame = self.peer.recv().await?;
        self.apply_inbound(&frame)?;
        Ok(frame)
    }

    fn apply_inbound(&mut self, frame: &ConstellationFrame) -> Result<(), ConstellationError> {
        match frame {
            ConstellationFrame::StreamOpen(open) => {
                self.mux.open_with_capabilities(open, &self.capabilities)
            }
            ConstellationFrame::StreamData(data) => self.mux.accept_data(data),
            ConstellationFrame::StreamFlow(flow) => self.mux.receive_flow(flow),
            ConstellationFrame::StreamClose(close) => self.mux.close(close),
            ConstellationFrame::StreamResume(resume) => self.mux.validate_resume(resume),
            ConstellationFrame::Handshake(_)
            | ConstellationFrame::HandshakeAccepted(_)
            | ConstellationFrame::HandshakeRejected(_)
            | ConstellationFrame::OperationRequest(_)
            | ConstellationFrame::OperationResponse(_)
            | ConstellationFrame::TypedError(_)
            | ConstellationFrame::AdmissionAudit(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_constellation_codec_protobuf::ProtobufCodec;
    use d2b_constellation_core::{
        Capability, CapabilitySet, PrincipalId, RealmPath, StreamAuthz, StreamDescriptor,
        StreamFlow, StreamKind,
    };
    use d2b_constellation_provider::provider::TransportProvider;
    use d2b_constellation_provider::types::{NodeRegistration, TransportSession, TransportTarget};
    use d2b_constellation_transport::LoopbackTransport;
    use std::sync::Arc;

    async fn connected_pair() -> (TransportSession, TransportSession) {
        let transport = Arc::new(LoopbackTransport::new());
        let listener = transport
            .listen(NodeRegistration {
                node: d2b_constellation_core::NodeId::parse("gateway").unwrap(),
            })
            .await
            .unwrap();
        let connect = {
            let transport = Arc::clone(&transport);
            tokio::spawn(async move {
                transport
                    .connect(TransportTarget {
                        endpoint: "loopback".to_owned(),
                    })
                    .await
            })
        };
        let server = listener.accept().await.unwrap();
        let client = connect.await.unwrap().unwrap();
        (client, server)
    }

    fn stream_id() -> StreamId {
        StreamId::parse("display-1").unwrap()
    }

    fn display_open() -> StreamOpen {
        let principal = PrincipalId::parse("alice").unwrap();
        StreamOpen {
            descriptor: StreamDescriptor {
                id: stream_id(),
                kind: StreamKind::Display,
            },
            operation_id: d2b_constellation_core::OperationId::parse("op-1").unwrap(),
            authz: StreamAuthz::for_kind(principal, RealmPath::local(), StreamKind::Display),
        }
    }

    fn display_caps() -> CapabilitySet {
        CapabilitySet::empty().with(Capability::WindowForwarding)
    }

    #[tokio::test]
    async fn mux_session_gates_open_flow_data_close_round_trip() {
        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            let peer = PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                display_caps(),
            )
            .await
            .unwrap();
            let mut s = MuxSession::new(peer);
            assert!(matches!(
                s.recv().await.unwrap(),
                ConstellationFrame::StreamOpen(_)
            ));
            assert!(s.mux().is_open(&stream_id()));
            s.grant_inbound(&stream_id(), 1).await.unwrap();
            assert!(matches!(
                s.recv().await.unwrap(),
                ConstellationFrame::StreamData(_)
            ));
            assert!(matches!(
                s.recv().await.unwrap(),
                ConstellationFrame::StreamClose(_)
            ));
            assert_eq!(
                s.mux().close_reason(&stream_id()),
                Some(StreamCloseReason::Completed)
            );
        });

        let peer =
            PeerSession::connect_with_capabilities(client_s, ProtobufCodec::new(), display_caps())
                .await
                .unwrap();
        let mut client = MuxSession::new(peer);
        client.open_stream(display_open()).await.unwrap();
        assert!(matches!(
            client.recv().await.unwrap(),
            ConstellationFrame::StreamFlow(StreamFlow { credits: 1, .. })
        ));
        client
            .send_data(
                &stream_id(),
                StreamChannel::Primary,
                OpaquePayload::new(b"wayland".to_vec()).unwrap(),
            )
            .await
            .unwrap();
        client
            .close_stream(&stream_id(), StreamCloseReason::Completed)
            .await
            .unwrap();

        server.await.unwrap();
    }

    #[tokio::test]
    async fn inbound_data_before_credit_is_rejected() {
        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            let peer = PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                display_caps(),
            )
            .await
            .unwrap();
            let mut s = MuxSession::new(peer);
            assert!(matches!(
                s.recv().await.unwrap(),
                ConstellationFrame::StreamOpen(_)
            ));
            s.recv().await.unwrap_err()
        });

        let mut peer =
            PeerSession::connect_with_capabilities(client_s, ProtobufCodec::new(), display_caps())
                .await
                .unwrap();
        peer.send(&ConstellationFrame::StreamOpen(display_open()))
            .await
            .unwrap();
        peer.send(&ConstellationFrame::StreamData(StreamData {
            stream: stream_id(),
            sequence: 0,
            channel: StreamChannel::Primary,
            cursor: None,
            data: OpaquePayload::new(b"without-credit".to_vec()).unwrap(),
        }))
        .await
        .unwrap();

        let err = server.await.unwrap();
        assert_eq!(err.kind(), d2b_constellation_core::ErrorKind::Backpressure);
    }

    #[tokio::test]
    async fn send_data_without_outbound_credit_is_rejected() {
        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            let peer = PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                display_caps(),
            )
            .await
            .unwrap();
            let mut s = MuxSession::new(peer);
            let _ = s.recv().await.unwrap();
        });

        let peer =
            PeerSession::connect_with_capabilities(client_s, ProtobufCodec::new(), display_caps())
                .await
                .unwrap();
        let mut client = MuxSession::new(peer);
        client.open_stream(display_open()).await.unwrap();
        let err = client
            .send_data(
                &stream_id(),
                StreamChannel::Primary,
                OpaquePayload::new(b"without-credit".to_vec()).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), d2b_constellation_core::ErrorKind::Backpressure);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancel_stream_is_idempotent() {
        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            let peer = PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                display_caps(),
            )
            .await
            .unwrap();
            let mut s = MuxSession::new(peer);
            assert!(matches!(
                s.recv().await.unwrap(),
                ConstellationFrame::StreamOpen(_)
            ));
            assert!(matches!(
                s.recv().await.unwrap(),
                ConstellationFrame::StreamClose(StreamClose {
                    reason: StreamCloseReason::Cancelled,
                    ..
                })
            ));
        });

        let peer =
            PeerSession::connect_with_capabilities(client_s, ProtobufCodec::new(), display_caps())
                .await
                .unwrap();
        let mut client = MuxSession::new(peer);
        client.open_stream(display_open()).await.unwrap();
        assert!(client.cancel_stream(&stream_id()).await.unwrap());
        assert!(!client.cancel_stream(&stream_id()).await.unwrap());
        server.await.unwrap();
    }
}
