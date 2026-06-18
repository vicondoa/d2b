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

use nixling_constellation_core::{ConstellationFrame, ErrorKind, Handshake};
use nixling_constellation_provider::error::{ProviderError, ProviderResult};
use nixling_constellation_provider::provider::ProtocolCodec;
use nixling_constellation_provider::types::{ByteStream, TransportSession};
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
    /// Perform the **client** handshake over `session`: propose
    /// [`PROTOCOL_VERSION`] + `codec.codec_id()`, then require the server to
    /// echo the same version and codec id (fail-closed on skew).
    pub async fn connect(session: TransportSession, codec: C) -> ProviderResult<Self> {
        Self::establish(session, codec, Role::Client).await
    }

    /// Perform the **server** handshake over `session`: read the client's
    /// proposal, require it to match this build's version + codec id, then
    /// echo the accepted handshake back (fail-closed on skew).
    pub async fn accept(session: TransportSession, codec: C) -> ProviderResult<Self> {
        Self::establish(session, codec, Role::Server).await
    }

    /// The handshake the peers agreed on.
    pub fn negotiated(&self) -> &Handshake {
        &self.negotiated
    }

    async fn establish(
        session: TransportSession,
        codec: C,
        role: Role,
    ) -> ProviderResult<Self> {
        let mut stream = session.into_stream();
        let ours = Handshake {
            protocol_version: PROTOCOL_VERSION,
            codec_id: parse_codec_id(codec.codec_id())?,
            peer: None,
        };
        let negotiated = match role {
            Role::Client => {
                write_frame(stream.as_mut(), &codec.encode_frame(&hs(ours.clone()))?).await?;
                let theirs = expect_handshake(&codec, &read_frame(stream.as_mut()).await?)?;
                reconcile(&codec, &ours, &theirs)?;
                theirs
            }
            Role::Server => {
                let theirs = expect_handshake(&codec, &read_frame(stream.as_mut()).await?)?;
                reconcile(&codec, &ours, &theirs)?;
                write_frame(stream.as_mut(), &codec.encode_frame(&hs(ours.clone()))?).await?;
                // The server adopts its own (validated-equal) view.
                ours
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

fn parse_codec_id(id: &str) -> ProviderResult<nixling_constellation_core::ProtocolToken> {
    nixling_constellation_core::ProtocolToken::parse(id)
        .map_err(|err| ProviderError::new(ErrorKind::MalformedFrame, format!("codec id: {err}")))
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

fn reconcile(
    codec: &impl ProtocolCodec,
    ours: &Handshake,
    theirs: &Handshake,
) -> ProviderResult<()> {
    if theirs.protocol_version != ours.protocol_version {
        return Err(ProviderError::new(
            ErrorKind::VersionSkew,
            "peer proposed an unsupported protocol version",
        ));
    }
    if theirs.codec_id.as_str() != codec.codec_id() {
        return Err(ProviderError::new(
            ErrorKind::VersionSkew,
            "peer proposed a codec this session does not speak",
        ));
    }
    Ok(())
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
    use nixling_constellation_codec_protobuf::ProtobufCodec;
    use nixling_constellation_core::{Capability, ConstellationError};
    use nixling_constellation_provider::provider::TransportProvider;
    use nixling_constellation_provider::types::{NodeRegistration, TransportTarget};
    use nixling_constellation_transport::LoopbackTransport;
    use std::sync::Arc;

    async fn connected_pair() -> (TransportSession, TransportSession) {
        let transport = Arc::new(LoopbackTransport::new());
        let registration = NodeRegistration {
            node: nixling_constellation_core::NodeId::parse("gateway").unwrap(),
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
            let mut s = PeerSession::accept(server_s, ProtobufCodec::new())
                .await
                .unwrap();
            // The server reads one frame the client sends post-handshake.
            s.recv().await.unwrap()
        });
        let mut client = PeerSession::connect(client_s, ProtobufCodec::new())
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
    async fn client_version_skew_is_rejected_fail_closed() {
        // Server speaks the real protocol; the client writes a handshake with
        // a bogus version, so the server must reject with VersionSkew.
        let (mut client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept(server_s, ProtobufCodec::new())
                .await
                .map(|_| ())
        });
        let codec = ProtobufCodec::new();
        let bogus = Handshake {
            protocol_version: PROTOCOL_VERSION + 99,
            codec_id: parse_codec_id(codec.codec_id()).unwrap(),
            peer: None,
        };
        let bytes = codec.encode_frame(&hs(bogus)).unwrap();
        write_frame(client_s.stream_mut(), &bytes)
            .await
            .unwrap();
        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::VersionSkew);
    }

    #[tokio::test]
    async fn non_handshake_first_frame_is_rejected() {
        let (mut client_s, server_s) = connected_pair().await;
        let server = tokio::spawn(async move {
            PeerSession::accept(server_s, ProtobufCodec::new())
                .await
                .map(|_| ())
        });
        // Client sends a non-handshake frame first.
        let codec = ProtobufCodec::new();
        let frame =
            ConstellationFrame::TypedError(ConstellationError::capability_denied(Capability::Exec));
        let bytes = codec.encode_frame(&frame).unwrap();
        write_frame(client_s.stream_mut(), &bytes)
            .await
            .unwrap();
        let err = server.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }
}
