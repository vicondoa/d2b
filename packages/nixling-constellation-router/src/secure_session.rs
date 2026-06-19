//! Authenticated + encrypted peer-session boundary (ADR 0032, P0).
//!
//! A transport session is reachability only. [`SecurePeerSession`] performs a
//! mutual HMAC handshake bound to realm/principal/node identities, protocol
//! version, codec id, and both nonces before any payload frame is accepted.
//! After the handshake every semantic frame is encoded by the negotiated codec
//! and encrypted with ChaCha20-Poly1305 under transcript-derived directional
//! keys.

use std::collections::HashSet;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hmac::{Mac, SimpleHmac};
use nixling_constellation_core::{
    ConstellationFrame, ErrorKind, NodeId, PrincipalId, ProtocolToken, RealmPath,
};
use nixling_constellation_provider::error::{ProviderError, ProviderResult};
use nixling_constellation_provider::provider::ProtocolCodec;
use nixling_constellation_provider::types::{ByteStream, TransportSession};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{MAX_FRAME_BYTES, PROTOCOL_VERSION};

type HmacSha256 = SimpleHmac<Sha256>;

/// Secret shared by the two enrolled peers. `Debug` redacts bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct SecureSessionKey([u8; 32]);

impl SecureSessionKey {
    /// Wrap exact key bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for SecureSessionKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SecureSessionKey(<redacted>)")
    }
}

/// Authenticated identity expected at the peer-session boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurePeerIdentity {
    /// Peer realm.
    pub realm: RealmPath,
    /// Peer principal.
    pub principal: PrincipalId,
    /// Peer node.
    pub node: NodeId,
}

/// Replay guard for client nonces accepted by a gateway.
#[derive(Debug, Default)]
pub struct NonceReplayGuard {
    seen: HashSet<[u8; 32]>,
}

impl NonceReplayGuard {
    /// Claim a nonce, returning `false` on replay.
    pub fn claim(&mut self, nonce: [u8; 32]) -> bool {
        self.seen.insert(nonce)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    version: u32,
    codec_id: ProtocolToken,
    identity: SecurePeerIdentity,
    nonce: [u8; 32],
    mac: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
enum Role {
    Client,
    Server,
}

/// Authenticated, encrypted semantic-frame session.
pub struct SecurePeerSession<C: ProtocolCodec> {
    stream: Box<dyn ByteStream>,
    codec: C,
    tx: ChaCha20Poly1305,
    rx: ChaCha20Poly1305,
    tx_nonce_tag: u8,
    rx_nonce_tag: u8,
    tx_seq: u64,
    rx_seq: u64,
    /// Authenticated remote identity.
    pub remote: SecurePeerIdentity,
}

impl<C: ProtocolCodec> SecurePeerSession<C> {
    /// Client-side connect: send identity+nonce+MAC, verify server reply, then
    /// expose encrypted frame I/O.
    pub async fn connect(
        session: TransportSession,
        codec: C,
        key: SecureSessionKey,
        local: SecurePeerIdentity,
        expected_remote: SecurePeerIdentity,
        client_nonce: [u8; 32],
    ) -> ProviderResult<Self> {
        let mut stream = session.into_stream();
        let client = hello(
            "client",
            &key,
            codec.codec_id(),
            local.clone(),
            client_nonce,
            &[],
        )?;
        write_plain(
            stream.as_mut(),
            &serde_json::to_vec(&client).map_err(json_err)?,
        )
        .await?;
        let server: Hello = serde_json::from_slice(&read_plain(stream.as_mut()).await?)
            .map_err(|err| ProviderError::new(ErrorKind::MalformedFrame, err.to_string()))?;
        verify_hello(
            "server",
            &key,
            codec.codec_id(),
            &server,
            &expected_remote,
            &client.nonce,
        )?;
        Ok(Self::new_encrypted(
            stream,
            codec,
            key,
            client.nonce,
            server.nonce,
            expected_remote,
            Role::Client,
        ))
    }

    /// Server-side accept with replay guard: verify the client hello, claim its
    /// nonce, reply with server identity+nonce+MAC, then expose encrypted I/O.
    pub async fn accept(
        session: TransportSession,
        codec: C,
        key: SecureSessionKey,
        local: SecurePeerIdentity,
        expected_remote: SecurePeerIdentity,
        server_nonce: [u8; 32],
        replay: &mut NonceReplayGuard,
    ) -> ProviderResult<Self> {
        let mut stream = session.into_stream();
        let client: Hello = serde_json::from_slice(&read_plain(stream.as_mut()).await?)
            .map_err(|err| ProviderError::new(ErrorKind::MalformedFrame, err.to_string()))?;
        verify_hello(
            "client",
            &key,
            codec.codec_id(),
            &client,
            &expected_remote,
            &[],
        )?;
        if !replay.claim(client.nonce) {
            return Err(ProviderError::new(
                ErrorKind::AuthenticationFailed,
                "secure-session client nonce replayed",
            ));
        }
        let server = hello(
            "server",
            &key,
            codec.codec_id(),
            local,
            server_nonce,
            &client.nonce,
        )?;
        write_plain(
            stream.as_mut(),
            &serde_json::to_vec(&server).map_err(json_err)?,
        )
        .await?;
        Ok(Self::new_encrypted(
            stream,
            codec,
            key,
            client.nonce,
            server.nonce,
            expected_remote,
            Role::Server,
        ))
    }

    /// Send one encrypted semantic frame.
    pub async fn send(&mut self, frame: &ConstellationFrame) -> ProviderResult<()> {
        let plain = self.codec.encode_frame(frame)?;
        let nonce = seq_nonce(self.tx_seq, self.tx_nonce_tag);
        self.tx_seq = self.tx_seq.wrapping_add(1);
        let cipher = self
            .tx
            .encrypt(Nonce::from_slice(&nonce), plain.as_slice())
            .map_err(|_| ProviderError::new(ErrorKind::AuthenticationFailed, "encrypt failed"))?;
        write_plain(self.stream.as_mut(), &cipher).await
    }

    /// Receive and decrypt one semantic frame.
    pub async fn recv(&mut self) -> ProviderResult<ConstellationFrame> {
        let cipher = read_plain(self.stream.as_mut()).await?;
        let nonce = seq_nonce(self.rx_seq, self.rx_nonce_tag);
        self.rx_seq = self.rx_seq.wrapping_add(1);
        let plain = self
            .rx
            .decrypt(Nonce::from_slice(&nonce), cipher.as_slice())
            .map_err(|_| ProviderError::new(ErrorKind::AuthenticationFailed, "decrypt failed"))?;
        Ok(self.codec.decode_frame(&plain)?)
    }

    fn new_encrypted(
        stream: Box<dyn ByteStream>,
        codec: C,
        key: SecureSessionKey,
        client_nonce: [u8; 32],
        server_nonce: [u8; 32],
        remote: SecurePeerIdentity,
        role: Role,
    ) -> Self {
        let client_tx = derive_key(&key, b"client-tx", &client_nonce, &server_nonce);
        let server_tx = derive_key(&key, b"server-tx", &client_nonce, &server_nonce);
        let (tx_key, rx_key, tx_nonce_tag, rx_nonce_tag) = match role {
            Role::Client => (client_tx, server_tx, 1, 2),
            Role::Server => (server_tx, client_tx, 2, 1),
        };
        Self {
            stream,
            codec,
            tx: ChaCha20Poly1305::new(Key::from_slice(&tx_key)),
            rx: ChaCha20Poly1305::new(Key::from_slice(&rx_key)),
            tx_nonce_tag,
            rx_nonce_tag,
            tx_seq: 0,
            rx_seq: 0,
            remote,
        }
    }
}

fn hello(
    label: &str,
    key: &SecureSessionKey,
    codec_id: &str,
    identity: SecurePeerIdentity,
    nonce: [u8; 32],
    peer_nonce: &[u8],
) -> ProviderResult<Hello> {
    let codec_id = ProtocolToken::parse(codec_id)
        .map_err(|err| ProviderError::new(ErrorKind::MalformedFrame, format!("codec id: {err}")))?;
    let mut hello = Hello {
        version: PROTOCOL_VERSION,
        codec_id,
        identity,
        nonce,
        mac: [0u8; 32],
    };
    hello.mac = mac_hello(label, key, &hello, peer_nonce)?;
    Ok(hello)
}

fn verify_hello(
    label: &str,
    key: &SecureSessionKey,
    codec_id: &str,
    hello: &Hello,
    expected_identity: &SecurePeerIdentity,
    peer_nonce: &[u8],
) -> ProviderResult<()> {
    if hello.version != PROTOCOL_VERSION
        || hello.codec_id.as_str() != codec_id
        || &hello.identity != expected_identity
    {
        return Err(ProviderError::new(
            ErrorKind::AuthenticationFailed,
            "secure-session hello binding mismatch",
        ));
    }
    let expected = mac_hello(label, key, hello, peer_nonce)?;
    if !bool::from(expected.ct_eq(&hello.mac)) {
        return Err(ProviderError::new(
            ErrorKind::AuthenticationFailed,
            "secure-session hello MAC mismatch",
        ));
    }
    Ok(())
}

fn mac_hello(
    label: &str,
    key: &SecureSessionKey,
    hello: &Hello,
    peer_nonce: &[u8],
) -> ProviderResult<[u8; 32]> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&key.0).map_err(|_| {
        ProviderError::new(ErrorKind::AuthenticationFailed, "bad secure-session key")
    })?;
    mac_update_field(&mut mac, label.as_bytes());
    mac_update_field(&mut mac, &hello.version.to_be_bytes());
    mac_update_field(&mut mac, hello.codec_id.as_str().as_bytes());
    mac_update_field(&mut mac, hello.identity.realm.target_form().as_bytes());
    mac_update_field(&mut mac, hello.identity.principal.as_str().as_bytes());
    mac_update_field(&mut mac, hello.identity.node.as_str().as_bytes());
    mac_update_field(&mut mac, &hello.nonce);
    mac_update_field(&mut mac, peer_nonce);
    Ok(mac.finalize().into_bytes().into())
}

fn mac_update_field(mac: &mut HmacSha256, field: &[u8]) {
    mac.update(&(field.len() as u64).to_be_bytes());
    mac.update(field);
}

fn derive_key(key: &SecureSessionKey, label: &[u8], c: &[u8; 32], s: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(key.0);
    h.update(label);
    h.update(c);
    h.update(s);
    h.finalize().into()
}

fn seq_nonce(seq: u64, tag: u8) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[3] = tag;
    n[4..].copy_from_slice(&seq.to_be_bytes());
    n
}

async fn write_plain(stream: &mut dyn ByteStream, payload: &[u8]) -> ProviderResult<()> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "secure-session frame exceeds cap",
        ));
    }
    stream
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await
        .map_err(|err| ProviderError::new(ErrorKind::GatewayUnavailable, err.kind().to_string()))?;
    stream
        .write_all(payload)
        .await
        .map_err(|err| ProviderError::new(ErrorKind::GatewayUnavailable, err.kind().to_string()))?;
    stream
        .flush()
        .await
        .map_err(|err| ProviderError::new(ErrorKind::GatewayUnavailable, err.kind().to_string()))
}

async fn read_plain(stream: &mut dyn ByteStream) -> ProviderResult<Vec<u8>> {
    let mut len = [0u8; 4];
    stream
        .read_exact(&mut len)
        .await
        .map_err(|err| ProviderError::new(ErrorKind::GatewayUnavailable, err.kind().to_string()))?;
    let declared = u32::from_le_bytes(len) as usize;
    if declared > MAX_FRAME_BYTES {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "secure-session declared frame exceeds cap",
        ));
    }
    let mut body = vec![0u8; declared];
    stream.read_exact(&mut body).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("truncated: {}", err.kind()),
        )
    })?;
    Ok(body)
}

fn json_err(err: serde_json::Error) -> ProviderError {
    ProviderError::new(ErrorKind::MalformedFrame, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_codec_protobuf::ProtobufCodec;
    use nixling_constellation_core::{Capability, ConstellationError, RealmId};
    use nixling_constellation_provider::provider::TransportProvider;
    use nixling_constellation_provider::types::{
        NodeRegistration, TransportSession, TransportTarget,
    };
    use nixling_constellation_transport::LoopbackTransport;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;

    async fn connected_pair() -> (TransportSession, TransportSession) {
        let transport = Arc::new(LoopbackTransport::new());
        let listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gateway").unwrap(),
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

    fn id(label: &str) -> SecurePeerIdentity {
        SecurePeerIdentity {
            realm: RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap(),
            principal: PrincipalId::parse(label).unwrap(),
            node: NodeId::parse(label).unwrap(),
        }
    }

    fn test_nonce(label: &str) -> [u8; 32] {
        Sha256::digest(label.as_bytes()).into()
    }

    #[tokio::test]
    async fn mutual_auth_then_encrypted_frame_round_trip() {
        let (client_s, server_s) = connected_pair().await;
        let key = SecureSessionKey::from_bytes([7u8; 32]);
        let client_id = id("client");
        let server_id = id("server");
        let server_key = key.clone();
        let server_client_id = client_id.clone();
        let server_server_id = server_id.clone();
        let server = tokio::spawn(async move {
            let mut guard = NonceReplayGuard::default();
            let mut s = SecurePeerSession::accept(
                server_s,
                ProtobufCodec::new(),
                server_key,
                server_server_id,
                server_client_id,
                test_nonce("server-round-trip"),
                &mut guard,
            )
            .await
            .unwrap();
            s.recv().await.unwrap()
        });
        let mut client = SecurePeerSession::connect(
            client_s,
            ProtobufCodec::new(),
            key,
            client_id,
            server_id,
            test_nonce("client-round-trip"),
        )
        .await
        .unwrap();
        let frame =
            ConstellationFrame::TypedError(ConstellationError::capability_denied(Capability::Exec));
        client.send(&frame).await.unwrap();
        assert_eq!(server.await.unwrap(), frame);
    }

    #[tokio::test]
    async fn wrong_principal_or_replayed_nonce_rejects() {
        let key = SecureSessionKey::from_bytes([7u8; 32]);
        let client_id = id("client");
        let server_id = id("server");

        let hello = hello(
            "client",
            &key,
            ProtobufCodec::new().codec_id(),
            client_id.clone(),
            test_nonce("wrong-principal"),
            &[],
        )
        .unwrap();
        let err = verify_hello(
            "client",
            &key,
            ProtobufCodec::new().codec_id(),
            &hello,
            &server_id,
            &[],
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthenticationFailed);

        let mut guard = NonceReplayGuard::default();
        let nonce = test_nonce("replayed");
        assert!(guard.claim(nonce));
        assert!(!guard.claim(nonce));
    }

    #[test]
    fn invalid_mac_version_or_codec_rejects() {
        let key = SecureSessionKey::from_bytes([7u8; 32]);
        let client_id = id("client");
        let codec = ProtobufCodec::new();
        let mut h = hello(
            "client",
            &key,
            codec.codec_id(),
            client_id.clone(),
            test_nonce("invalid-mac"),
            &[],
        )
        .unwrap();

        h.mac[0] ^= 0xff;
        assert_eq!(
            verify_hello("client", &key, codec.codec_id(), &h, &client_id, &[])
                .unwrap_err()
                .kind(),
            ErrorKind::AuthenticationFailed
        );

        let mut h = hello(
            "client",
            &key,
            codec.codec_id(),
            client_id.clone(),
            test_nonce("invalid-version"),
            &[],
        )
        .unwrap();
        h.version += 1;
        assert_eq!(
            verify_hello("client", &key, codec.codec_id(), &h, &client_id, &[])
                .unwrap_err()
                .kind(),
            ErrorKind::AuthenticationFailed
        );

        let h = hello(
            "client",
            &key,
            codec.codec_id(),
            client_id.clone(),
            test_nonce("invalid-codec"),
            &[],
        )
        .unwrap();
        assert_eq!(
            verify_hello("client", &key, "other-codec", &h, &client_id, &[])
                .unwrap_err()
                .kind(),
            ErrorKind::AuthenticationFailed
        );
    }

    #[test]
    fn key_debug_redacts() {
        assert_eq!(
            format!("{:?}", SecureSessionKey::from_bytes([1u8; 32])),
            "SecureSessionKey(<redacted>)"
        );
    }
}
