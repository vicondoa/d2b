//! The authenticated peer-session boundary (ADR 0032).
//!
//! A raw [`TransportSession`] carries only **reachability** — a duplex byte
//! stream over a loopback duplex, an AF_VSOCK pair, or an Azure Relay
//! connection. It is never, on its own, a constellation principal. This
//! module wraps that raw stream and refuses to expose any operation/stream
//! traffic until a [`Handshake`] has been exchanged and the negotiated
//! protocol version + codec id agree (fail-closed on skew). The rust design
//! reviewer required exactly this seam so a later wave can add mutual
//! authentication, encryption, and channel binding *above* relay
//! reachability without reshaping the call sites.
//!
//! Framing is a little-endian `u32` length prefix followed by the codec
//! bytes, bounded by [`MAX_FRAME_BYTES`]. Reads use `read_exact` so a
//! fragmenting stream transport (Relay, vsock) is reassembled correctly —
//! unlike a seqpacket one-read-per-frame socket.

use d2b_realm_core::{
    CapabilitySet, ConstellationFrame, ErrorKind, Handshake, HandshakeAccepted, HandshakeRejected,
    HandshakeRejectedReason,
};
use d2b_realm_provider::error::{ProviderError, ProviderResult};
use d2b_realm_provider::provider::ProtocolCodec;
use d2b_realm_provider::types::{ByteStream, TransportSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The constellation protocol version this build speaks. A peer that
/// proposes a different version is rejected (`VersionSkew`); there is no
/// silent downgrade.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum length-delimited frame size accepted on the wire (1 MiB), matched
/// to the daemon/public frame cap. A declared length above this is rejected
/// before any allocation.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Which side of the handshake an endpoint plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Dials out and proposes the version/codec.
    Client,
    /// Accepts and validates the proposal.
    Server,
}

/// A peer session whose operation/stream API only becomes reachable after a
/// successful handshake. `C` is the negotiated [`ProtocolCodec`].
pub struct PeerSession<C: ProtocolCodec> {
    stream: Box<dyn ByteStream>,
    codec: C,
    /// The peer's accepted handshake (version + codec id, plus the reserved
    /// peer-binding seam). Present iff the handshake completed.
    negotiated: Handshake,
}

impl<C: ProtocolCodec> PeerSession<C> {
    /// Client handshake with the explicit capabilities this endpoint proposes.
    pub async fn connect_with_capabilities(
        session: TransportSession,
        codec: C,
        capabilities: CapabilitySet,
    ) -> ProviderResult<Self> {
        Self::establish(session, codec, Role::Client, capabilities).await
    }

    /// Server handshake with the explicit capabilities this endpoint supports.
    pub async fn accept_with_capabilities(
        session: TransportSession,
        codec: C,
        capabilities: CapabilitySet,
    ) -> ProviderResult<Self> {
        Self::establish(session, codec, Role::Server, capabilities).await
    }

    /// The handshake the peers agreed on.
    pub fn negotiated(&self) -> &Handshake {
        &self.negotiated
    }

    async fn establish(
        session: TransportSession,
        codec: C,
        role: Role,
        capabilities: CapabilitySet,
    ) -> ProviderResult<Self> {
        let mut stream = session.into_stream();
        let ours = Handshake {
            protocol_version: PROTOCOL_VERSION,
            codec_id: parse_codec_id(codec.codec_id())?,
            schema_fingerprint: parse_codec_id(&codec.schema_fingerprint())?,
            capabilities: capabilities.negotiation(),
            peer: None,
        };
        let negotiated = match role {
            Role::Client => {
                write_frame(stream.as_mut(), &codec.encode_frame(&hs(ours.clone()))?).await?;
                let accepted =
                    expect_handshake_accept(&codec, &read_frame(stream.as_mut()).await?)?;
                if let Err(reason) = reconcile_selected(&ours, &accepted.selected) {
                    return Err(handshake_rejected_error(reason));
                }
                accepted.selected
            }
            Role::Server => {
                let theirs = expect_handshake(&codec, &read_frame(stream.as_mut()).await?)?;
                if let Err(reason) = reconcile_proposal(&ours, &theirs) {
                    let rejected =
                        ConstellationFrame::HandshakeRejected(HandshakeRejected { reason });
                    let _ = write_frame(stream.as_mut(), &codec.encode_frame(&rejected)?).await;
                    return Err(handshake_rejected_error(reason));
                }
                let mut selected = ours.clone();
                selected.capabilities = ours
                    .capabilities
                    .capabilities
                    .intersection(&theirs.capabilities.capabilities)
                    .negotiation();
                let accepted = ConstellationFrame::HandshakeAccepted(HandshakeAccepted {
                    selected: selected.clone(),
                });
                write_frame(stream.as_mut(), &codec.encode_frame(&accepted)?).await?;
                selected
            }
        };
        Ok(Self {
            stream,
            codec,
            negotiated,
        })
    }

    /// Encode and send one semantic frame (post-handshake).
    pub async fn send(&mut self, frame: &ConstellationFrame) -> ProviderResult<()> {
        let bytes = self.codec.encode_frame(frame)?;
        write_frame(self.stream.as_mut(), &bytes).await
    }

    /// Receive and decode one semantic frame (post-handshake, fail-closed).
    pub async fn recv(&mut self) -> ProviderResult<ConstellationFrame> {
        let bytes = read_frame(self.stream.as_mut()).await?;
        Ok(self.codec.decode_frame(&bytes)?)
    }
}

fn hs(handshake: Handshake) -> ConstellationFrame {
    ConstellationFrame::Handshake(handshake)
}

fn parse_codec_id(id: &str) -> ProviderResult<d2b_realm_core::ProtocolToken> {
    d2b_realm_core::ProtocolToken::parse(id)
        .map_err(|err| ProviderError::new(ErrorKind::MalformedFrame, format!("codec id: {err}")))
}

fn expect_handshake_accept(
    codec: &impl ProtocolCodec,
    bytes: &[u8],
) -> ProviderResult<HandshakeAccepted> {
    match codec.decode_frame(bytes)? {
        ConstellationFrame::HandshakeAccepted(h) => Ok(h),
        ConstellationFrame::HandshakeRejected(rejected) => {
            Err(handshake_rejected_error(rejected.reason))
        }
        _ => Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "expected a handshake-accepted frame before any other traffic",
        )),
    }
}

fn expect_handshake(codec: &impl ProtocolCodec, bytes: &[u8]) -> ProviderResult<Handshake> {
    match codec.decode_frame(bytes)? {
        ConstellationFrame::Handshake(h) => Ok(h),
        _ => Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "expected a handshake frame before any other traffic",
        )),
    }
}

fn reconcile_common(ours: &Handshake, theirs: &Handshake) -> Result<(), HandshakeRejectedReason> {
    if theirs.protocol_version != ours.protocol_version {
        return Err(HandshakeRejectedReason::VersionSkew);
    }
    if theirs.codec_id != ours.codec_id {
        return Err(HandshakeRejectedReason::CodecMismatch);
    }
    if theirs.schema_fingerprint != ours.schema_fingerprint {
        return Err(HandshakeRejectedReason::SchemaFingerprintMismatch);
    }
    if theirs.peer != ours.peer {
        return Err(HandshakeRejectedReason::ChannelBindingMismatch);
    }
    Ok(())
}

fn reconcile_proposal(ours: &Handshake, theirs: &Handshake) -> Result<(), HandshakeRejectedReason> {
    reconcile_common(ours, theirs)
}

fn reconcile_selected(
    ours: &Handshake,
    selected: &Handshake,
) -> Result<(), HandshakeRejectedReason> {
    reconcile_common(ours, selected)?;
    if !selected
        .capabilities
        .capabilities
        .is_subset_of(&ours.capabilities.capabilities)
    {
        return Err(HandshakeRejectedReason::CapabilityMismatch);
    }
    if selected.capabilities.fingerprint != selected.capabilities.capabilities.stable_fingerprint()
    {
        return Err(HandshakeRejectedReason::CapabilityMismatch);
    }
    Ok(())
}

fn handshake_rejected_error(reason: HandshakeRejectedReason) -> ProviderError {
    let kind = match reason {
        HandshakeRejectedReason::VersionSkew => ErrorKind::VersionSkew,
        HandshakeRejectedReason::CodecMismatch
        | HandshakeRejectedReason::SchemaFingerprintMismatch
        | HandshakeRejectedReason::CapabilityMismatch => ErrorKind::VersionSkew,
        HandshakeRejectedReason::ChannelBindingMismatch => ErrorKind::AuthenticationFailed,
        HandshakeRejectedReason::MalformedHandshake => ErrorKind::MalformedFrame,
    };
    ProviderError::new(kind, format!("peer handshake rejected: {reason:?}"))
}

async fn write_frame(stream: &mut dyn ByteStream, payload: &[u8]) -> ProviderResult<()> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "peer-session frame exceeds the maximum size",
        ));
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    stream.write_all(&frame).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("peer-session write failed: {}", err.kind()),
        )
    })?;
    stream.flush().await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("peer-session flush failed: {}", err.kind()),
        )
    })?;
    Ok(())
}

async fn read_frame(stream: &mut dyn ByteStream) -> ProviderResult<Vec<u8>> {
    let mut len_buf = [0_u8; 4];
    stream.read_exact(&mut len_buf).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("peer-session closed before a frame length: {}", err.kind()),
        )
    })?;
    let declared = u32::from_le_bytes(len_buf) as usize;
    if declared > MAX_FRAME_BYTES {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "peer declared a frame above the maximum size",
        ));
    }
    let mut body = vec![0_u8; declared];
    stream.read_exact(&mut body).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("peer-session truncated a frame body: {}", err.kind()),
        )
    })?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_codec_protobuf::ProtobufCodec;
    use d2b_realm_core::{
        Capability, ConstellationError, NodeId, PrincipalId, ProtocolToken,
    };
    use d2b_realm_provider::provider::TransportProvider;
    use d2b_realm_provider::types::{NodeRegistration, TransportTarget};
    use d2b_realm_transport::LoopbackTransport;
    use std::sync::Arc;
    use tokio::io::AsyncWriteExt;

    async fn connected_pair() -> (TransportSession, TransportSession) {
        let transport = Arc::new(LoopbackTransport::new());
        let registration = NodeRegistration {
            node: d2b_realm_core::NodeId::parse("gateway").unwrap(),
        };
        let listener = transport.listen(registration).await.unwrap();
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

    #[tokio::test]
    async fn handshake_succeeds_and_frame_round_trips() {
        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            let mut s = PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                CapabilitySet::empty(),
            )
            .await
            .unwrap();
            // The server reads one frame the client sends post-handshake.
            s.recv().await.unwrap()
        });
        let mut client = PeerSession::connect_with_capabilities(
            client_s,
            ProtobufCodec::new(),
            CapabilitySet::empty(),
        )
        .await
        .unwrap();
        assert_eq!(client.negotiated().protocol_version, PROTOCOL_VERSION);
        let frame =
            ConstellationFrame::TypedError(ConstellationError::capability_denied(Capability::Exec));
        client.send(&frame).await.unwrap();
        let received = server.await.unwrap();
        assert_eq!(received, frame);
    }

    #[tokio::test]
    async fn handshake_negotiates_capability_intersection() {
        let (client_s, server_s) = connected_pair().await;
        let server_caps = CapabilitySet::empty().with(Capability::Exec);
        let client_caps = CapabilitySet::empty()
            .with(Capability::Exec)
            .with(Capability::WindowForwarding);
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(server_s, ProtobufCodec::new(), server_caps)
                .await
                .unwrap()
                .negotiated()
                .capabilities
                .capabilities
                .clone()
        });
        let client =
            PeerSession::connect_with_capabilities(client_s, ProtobufCodec::new(), client_caps)
                .await
                .unwrap();
        let selected = &client.negotiated().capabilities.capabilities;
        assert!(selected.has(Capability::Exec));
        assert!(!selected.has(Capability::WindowForwarding));
        assert_eq!(server.await.unwrap(), selected.clone());
    }

    #[tokio::test]
    async fn handshake_negotiates_empty_capability_intersection() {
        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                CapabilitySet::empty(),
            )
            .await
            .unwrap()
            .negotiated()
            .capabilities
            .capabilities
            .clone()
        });
        let client = PeerSession::connect_with_capabilities(
            client_s,
            ProtobufCodec::new(),
            CapabilitySet::empty().with(Capability::Exec),
        )
        .await
        .unwrap();
        assert!(
            !client
                .negotiated()
                .capabilities
                .capabilities
                .has(Capability::Exec)
        );
        assert_eq!(
            server.await.unwrap(),
            client.negotiated().capabilities.capabilities
        );
    }

    #[tokio::test]
    async fn client_rejects_accepted_capabilities_outside_its_proposal() {
        let (client_s, mut server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            let codec = ProtobufCodec::new();
            let _proposal = read_frame(server_s.stream_mut()).await.unwrap();
            let accepted = ConstellationFrame::HandshakeAccepted(HandshakeAccepted {
                selected: Handshake {
                    protocol_version: PROTOCOL_VERSION,
                    codec_id: parse_codec_id(codec.codec_id()).unwrap(),
                    schema_fingerprint: parse_codec_id(&codec.schema_fingerprint()).unwrap(),
                    capabilities: CapabilitySet::empty()
                        .with(Capability::Lifecycle)
                        .negotiation(),
                    peer: None,
                },
            });
            write_frame(
                server_s.stream_mut(),
                &codec.encode_frame(&accepted).unwrap(),
            )
            .await
            .unwrap();
        });
        let err = match PeerSession::connect_with_capabilities(
            client_s,
            ProtobufCodec::new(),
            CapabilitySet::empty(),
        )
        .await
        {
            Ok(_) => panic!("server-selected capabilities outside the proposal must be rejected"),
            Err(err) => err,
        };
        server.await.unwrap();
        assert_eq!(err.kind(), ErrorKind::VersionSkew);
    }

    #[tokio::test]
    async fn client_version_skew_is_rejected_fail_closed() {
        // Server speaks the real protocol; the client writes a handshake with
        // a bogus version, so the server must reject with VersionSkew.
        let (mut client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                CapabilitySet::empty(),
            )
            .await
            .map(|_| ())
        });
        let codec = ProtobufCodec::new();
        let bogus = Handshake {
            protocol_version: PROTOCOL_VERSION + 99,
            codec_id: parse_codec_id(codec.codec_id()).unwrap(),
            schema_fingerprint: parse_codec_id(&codec.schema_fingerprint()).unwrap(),
            capabilities: CapabilitySet::empty().negotiation(),
            peer: None,
        };
        let bytes = codec.encode_frame(&hs(bogus)).unwrap();
        write_frame(client_s.stream_mut(), &bytes).await.unwrap();
        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::VersionSkew);
    }

    #[tokio::test]
    async fn client_schema_fingerprint_skew_is_rejected_fail_closed() {
        let (mut client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                CapabilitySet::empty(),
            )
            .await
            .map(|_| ())
        });
        let codec = ProtobufCodec::new();
        let bogus = Handshake {
            protocol_version: PROTOCOL_VERSION,
            codec_id: parse_codec_id(codec.codec_id()).unwrap(),
            schema_fingerprint: parse_codec_id("pb.v1:other-schema").unwrap(),
            capabilities: CapabilitySet::empty().negotiation(),
            peer: None,
        };
        let bytes = codec.encode_frame(&hs(bogus)).unwrap();
        write_frame(client_s.stream_mut(), &bytes).await.unwrap();
        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::VersionSkew);
    }

    #[tokio::test]
    async fn client_receives_explicit_handshake_rejection_fail_closed() {
        #[derive(Clone, Copy)]
        struct SkewedSchemaCodec;

        impl ProtocolCodec for SkewedSchemaCodec {
            fn codec_id(&self) -> &str {
                d2b_realm_codec_protobuf::CODEC_ID
            }

            fn encode_frame(
                &self,
                frame: &ConstellationFrame,
            ) -> Result<Vec<u8>, ConstellationError> {
                ProtobufCodec::new().encode_frame(frame)
            }

            fn decode_frame(&self, bytes: &[u8]) -> Result<ConstellationFrame, ConstellationError> {
                ProtobufCodec::new().decode_frame(bytes)
            }

            fn schema_fingerprint(&self) -> String {
                "pb.v1:other-schema".to_owned()
            }
        }

        let (client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(
                server_s,
                SkewedSchemaCodec,
                CapabilitySet::empty(),
            )
            .await
            .map(|_| ())
        });
        let client_err = match PeerSession::connect_with_capabilities(
            client_s,
            ProtobufCodec::new(),
            CapabilitySet::empty(),
        )
        .await
        {
            Ok(_) => panic!("schema mismatch must reject the client"),
            Err(err) => err,
        };
        let server_err = server.await.unwrap().unwrap_err();
        assert_eq!(client_err.kind(), ErrorKind::VersionSkew);
        assert_eq!(server_err.kind(), ErrorKind::VersionSkew);
    }

    #[tokio::test]
    async fn peer_binding_mismatch_is_rejected_fail_closed() {
        let (mut client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                CapabilitySet::empty(),
            )
            .await
            .map(|_| ())
        });
        let codec = ProtobufCodec::new();
        let forged = Handshake {
            protocol_version: PROTOCOL_VERSION,
            codec_id: parse_codec_id(codec.codec_id()).unwrap(),
            schema_fingerprint: parse_codec_id(&codec.schema_fingerprint()).unwrap(),
            capabilities: CapabilitySet::empty().negotiation(),
            peer: Some(d2b_realm_core::PeerContext {
                auth_mechanism: ProtocolToken::parse("forged-binding").unwrap(),
                peer_principal: Some(PrincipalId::parse("principal-1").unwrap()),
                peer_node: Some(NodeId::parse("node-a").unwrap()),
            }),
        };
        let bytes = codec.encode_frame(&hs(forged)).unwrap();
        write_frame(client_s.stream_mut(), &bytes).await.unwrap();
        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthenticationFailed);
    }

    #[tokio::test]
    async fn non_handshake_first_frame_is_rejected() {
        let (mut client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept_with_capabilities(
                server_s,
                ProtobufCodec::new(),
                CapabilitySet::empty(),
            )
            .await
            .map(|_| ())
        });
        // Client sends a non-handshake frame first.
        let codec = ProtobufCodec::new();
        let frame =
            ConstellationFrame::TypedError(ConstellationError::capability_denied(Capability::Exec));
        let bytes = codec.encode_frame(&frame).unwrap();
        write_frame(client_s.stream_mut(), &bytes).await.unwrap();
        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn write_frame_rejects_payload_above_cap() {
        let (mut a, _b) = tokio::io::duplex(64);
        let payload = vec![0_u8; MAX_FRAME_BYTES + 1];
        let err = write_frame(&mut a, &payload).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FrameTooLarge);
    }

    #[tokio::test]
    async fn read_frame_rejects_declared_length_above_cap() {
        let (mut writer, mut reader) = tokio::io::duplex(8);
        writer
            .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_le_bytes())
            .await
            .unwrap();
        let err = read_frame(&mut reader).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FrameTooLarge);
    }

    #[tokio::test]
    async fn read_frame_rejects_truncated_payload() {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        let writer = tokio::spawn(async move {
            writer.write_all(&5_u32.to_le_bytes()).await.unwrap();
            writer.write_all(b"ab").await.unwrap();
        });
        let err = read_frame(&mut reader).await.unwrap_err();
        writer.await.unwrap();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }
}
