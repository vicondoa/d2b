use std::fmt;

use d2b_contracts::v2_component_session::{
    BinaryError, ComponentSessionPreface, ENDPOINT_POLICY_IDENTITY_CANONICAL_LEN, EndpointPolicy,
    EndpointPolicyIdentity, HandshakeOffer, LimitProfile, NoiseProfile, PREFACE_LEN, PrefaceError,
    SessionErrorCode,
};
use sha2::{Digest, Sha256};
use snow::{
    Builder, HandshakeState, TransportState,
    params::{DHChoice, NoiseParams},
    resolvers::{CryptoResolver, DefaultResolver},
};

use crate::{AdmittedBootstrapPsk, Result, Secret32, SessionError};

const INIT_PAYLOAD: &[u8] = b"d2b-component-session-v2-init";
const ACCEPT_PAYLOAD: &[u8] = b"d2b-component-session-v2-accept";
const GENERATION_QUERY_MAGIC: &[u8; 8] = b"D2BGD2Q\n";
const GENERATION_REPLY_MAGIC: &[u8; 8] = b"D2BGD2A\n";
pub const GENERATION_DISCOVERY_REQUEST_LEN: usize =
    GENERATION_QUERY_MAGIC.len() + ENDPOINT_POLICY_IDENTITY_CANONICAL_LEN;
pub const GENERATION_DISCOVERY_RESPONSE_LEN: usize = GENERATION_REPLY_MAGIC.len() + 32 + 8;

pub fn x25519_public_key(private_key: &[u8; 32]) -> Result<[u8; 32]> {
    if private_key == &[0; 32] {
        return Err(SessionError::new(SessionErrorCode::AuthenticationFailed));
    }
    let mut dh = DefaultResolver
        .resolve_dh(&DHChoice::Curve25519)
        .ok_or_else(|| SessionError::new(SessionErrorCode::AuthenticationFailed))?;
    dh.set(private_key);
    dh.pubkey()
        .try_into()
        .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeRole {
    Initiator,
    Responder,
}

pub enum HandshakeCredentials {
    Nn,
    Kk {
        local_private: Secret32,
        remote_public: [u8; 32],
    },
    IkPsk2Initiator {
        local_private: Secret32,
        remote_public: [u8; 32],
        psk: AdmittedBootstrapPsk,
    },
    IkPsk2Responder {
        local_private: Secret32,
        psk: AdmittedBootstrapPsk,
    },
}

impl fmt::Debug for HandshakeCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = match self {
            Self::Nn => "nn",
            Self::Kk { .. } => "kk",
            Self::IkPsk2Initiator { .. } | Self::IkPsk2Responder { .. } => "ikpsk2",
        };
        formatter
            .debug_struct("HandshakeCredentials")
            .field("profile", &profile)
            .field("key_material", &"<redacted>")
            .finish()
    }
}

pub struct NegotiatedOffer {
    preface: ComponentSessionPreface,
    offer: HandshakeOffer,
    canonical_offer: Vec<u8>,
}

impl NegotiatedOffer {
    pub fn offer(&self) -> &HandshakeOffer {
        &self.offer
    }

    pub fn preface(&self) -> ComponentSessionPreface {
        self.preface
    }

    fn prologue(&self) -> Vec<u8> {
        let mut prologue = Vec::with_capacity(PREFACE_LEN + self.canonical_offer.len());
        prologue.extend_from_slice(&self.preface.encode());
        prologue.extend_from_slice(&self.canonical_offer);
        prologue
    }
}

impl fmt::Debug for NegotiatedOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NegotiatedOffer")
            .field("purpose", &self.offer.purpose.as_str())
            .field("service", &self.offer.service.as_str())
            .field("noise", &self.offer.noise_profile.as_str())
            .field("generation", &"<redacted>")
            .finish_non_exhaustive()
    }
}

pub fn encode_offer(policy: &EndpointPolicy) -> Result<([u8; PREFACE_LEN], Vec<u8>)> {
    let offer = HandshakeOffer::from(policy.clone());
    let canonical = offer.encode_canonical()?;
    let preface = ComponentSessionPreface::new(canonical.len())
        .map_err(preface_error)?
        .encode();
    Ok((preface, canonical))
}

pub fn negotiate_offer(
    preface_bytes: &[u8],
    offer_bytes: &[u8],
    policy: &EndpointPolicy,
) -> Result<NegotiatedOffer> {
    let preface = ComponentSessionPreface::parse(preface_bytes).map_err(preface_error)?;
    if usize::try_from(preface.offer_len).ok() != Some(offer_bytes.len()) {
        return Err(SessionError::new(SessionErrorCode::MalformedPreface));
    }
    let offer = HandshakeOffer::decode_canonical(offer_bytes).map_err(handshake_binary_error)?;
    offer.validate_exact(policy)?;
    Ok(NegotiatedOffer {
        preface,
        offer,
        canonical_offer: offer_bytes.to_vec(),
    })
}

pub fn encode_generation_discovery_request(identity: &EndpointPolicyIdentity) -> Result<Vec<u8>> {
    identity
        .validate_local_generation_discovery()
        .map_err(SessionError::from)?;
    let encoded = identity
        .encode_canonical()
        .map_err(handshake_binary_error)?;
    let mut request = Vec::with_capacity(GENERATION_DISCOVERY_REQUEST_LEN);
    request.extend_from_slice(GENERATION_QUERY_MAGIC);
    request.extend_from_slice(&encoded);
    Ok(request)
}

pub fn is_generation_discovery_request(bytes: &[u8]) -> bool {
    bytes.starts_with(GENERATION_QUERY_MAGIC)
}

pub fn accept_generation_discovery_request(
    bytes: &[u8],
    policy: &EndpointPolicy,
) -> Result<[u8; 32]> {
    if bytes.len() != GENERATION_DISCOVERY_REQUEST_LEN || !bytes.starts_with(GENERATION_QUERY_MAGIC)
    {
        return Err(SessionError::new(SessionErrorCode::MalformedHandshake));
    }
    let identity = EndpointPolicyIdentity::decode_canonical(&bytes[GENERATION_QUERY_MAGIC.len()..])
        .map_err(handshake_binary_error)?;
    identity
        .validate_local_generation_discovery()
        .map_err(SessionError::from)?;
    identity
        .validate_exact(policy)
        .map_err(SessionError::from)?;
    HandshakeOffer::from(policy.clone())
        .validate()
        .map_err(SessionError::from)?;
    Ok(Sha256::digest(bytes).into())
}

pub fn encode_generation_discovery_response(
    request_binding: [u8; 32],
    generation: u64,
) -> Result<Vec<u8>> {
    if request_binding == [0; 32] || generation == 0 {
        return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
    }
    let mut response = Vec::with_capacity(GENERATION_DISCOVERY_RESPONSE_LEN);
    response.extend_from_slice(GENERATION_REPLY_MAGIC);
    response.extend_from_slice(&request_binding);
    response.extend_from_slice(&generation.to_be_bytes());
    Ok(response)
}

pub fn decode_generation_discovery_response(bytes: &[u8], request: &[u8]) -> Result<u64> {
    if bytes.len() != GENERATION_DISCOVERY_RESPONSE_LEN
        || !bytes.starts_with(GENERATION_REPLY_MAGIC)
        || request.len() != GENERATION_DISCOVERY_REQUEST_LEN
    {
        return Err(SessionError::new(SessionErrorCode::MalformedHandshake));
    }
    let expected: [u8; 32] = Sha256::digest(request).into();
    if bytes[GENERATION_REPLY_MAGIC.len()..GENERATION_REPLY_MAGIC.len() + 32] != expected {
        return Err(SessionError::new(SessionErrorCode::TranscriptMismatch));
    }
    let generation = u64::from_be_bytes(
        bytes[GENERATION_REPLY_MAGIC.len() + 32..]
            .try_into()
            .map_err(|_| SessionError::new(SessionErrorCode::MalformedHandshake))?,
    );
    if generation == 0 {
        return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
    }
    Ok(generation)
}

fn handshake_binary_error(error: BinaryError) -> SessionError {
    match error {
        BinaryError::UnsupportedVersion => SessionError::new(SessionErrorCode::UnsupportedVersion),
        BinaryError::InvalidContract(inner) => SessionError::from(inner),
        BinaryError::Truncated
        | BinaryError::TrailingBytes
        | BinaryError::LengthExceeded
        | BinaryError::UnknownEnumTag
        | BinaryError::NonCanonical => SessionError::new(SessionErrorCode::MalformedHandshake),
    }
}

fn preface_error(error: PrefaceError) -> SessionError {
    let code = match error {
        PrefaceError::UnsupportedMajor | PrefaceError::UnsupportedMinor => {
            SessionErrorCode::UnsupportedVersion
        }
        PrefaceError::Truncated
        | PrefaceError::InvalidLength
        | PrefaceError::InvalidMagic
        | PrefaceError::EmptyOffer
        | PrefaceError::OfferTooLarge => SessionErrorCode::MalformedPreface,
    };
    SessionError::new(code)
}

pub struct NoiseHandshake {
    state: HandshakeState,
    role: HandshakeRole,
    step: u8,
    limits: LimitProfile,
    generation: u64,
}

impl NoiseHandshake {
    pub fn new(
        role: HandshakeRole,
        negotiated: &NegotiatedOffer,
        credentials: HandshakeCredentials,
    ) -> Result<Self> {
        validate_credentials(role, negotiated.offer.noise_profile, &credentials)?;
        let params: NoiseParams = negotiated
            .offer
            .noise_profile
            .as_str()
            .parse()
            .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
        let prologue = negotiated.prologue();
        let builder = Builder::new(params)
            .prologue(&prologue)
            .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
        let state = build_state(builder, role, &credentials)?;
        Ok(Self {
            state,
            role,
            step: 0,
            limits: negotiated.offer.limits,
            generation: negotiated.offer.reconnect_generation,
        })
    }

    pub fn write_next(&mut self) -> Result<Vec<u8>> {
        let payload = match (self.role, self.step) {
            (HandshakeRole::Initiator, 0) => INIT_PAYLOAD,
            (HandshakeRole::Responder, 1) => ACCEPT_PAYLOAD,
            _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
        };
        let mut output = vec![0_u8; self.limits.protected_ciphertext_bytes as usize];
        let written = self
            .state
            .write_message(payload, &mut output)
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?;
        ensure_handshake_bound(written, self.limits)?;
        output.truncate(written);
        self.step += 1;
        Ok(output)
    }

    pub fn read_next(&mut self, message: &[u8]) -> Result<()> {
        if message.len() > self.limits.protected_ciphertext_bytes as usize {
            return Err(SessionError::new(SessionErrorCode::MalformedHandshake));
        }
        let expected = match (self.role, self.step) {
            (HandshakeRole::Responder, 0) => INIT_PAYLOAD,
            (HandshakeRole::Initiator, 1) => ACCEPT_PAYLOAD,
            _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
        };
        let mut plaintext = vec![0_u8; self.limits.protected_ciphertext_bytes as usize];
        let read = self
            .state
            .read_message(message, &mut plaintext)
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?;
        if plaintext.get(..read) != Some(expected) {
            return Err(SessionError::new(SessionErrorCode::TranscriptMismatch));
        }
        self.step += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<EstablishedHandshake> {
        if self.step != 2 || !self.state.is_handshake_finished() {
            return Err(SessionError::new(SessionErrorCode::MalformedHandshake));
        }
        let transcript_hash: [u8; 32] = self
            .state
            .get_handshake_hash()
            .try_into()
            .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
        let transport = self
            .state
            .into_transport_mode()
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?;
        Ok(EstablishedHandshake {
            transport,
            transcript_hash,
            limits: self.limits,
            generation: self.generation,
        })
    }
}

fn ensure_handshake_bound(written: usize, limits: LimitProfile) -> Result<()> {
    let written = u32::try_from(written)
        .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
    limits
        .checked_handshake_allocation(written, 0, 0)
        .map(|_| ())
        .map_err(SessionError::from)
}

fn validate_credentials(
    role: HandshakeRole,
    profile: NoiseProfile,
    credentials: &HandshakeCredentials,
) -> Result<()> {
    let matches = matches!(
        (profile, role, credentials),
        (
            NoiseProfile::Nn25519ChaChaPolySha256,
            _,
            HandshakeCredentials::Nn
        ) | (
            NoiseProfile::Kk25519ChaChaPolySha256,
            _,
            HandshakeCredentials::Kk { .. }
        ) | (
            NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
            HandshakeRole::Initiator,
            HandshakeCredentials::IkPsk2Initiator { .. }
        ) | (
            NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
            HandshakeRole::Responder,
            HandshakeCredentials::IkPsk2Responder { .. }
        )
    );
    let public_keys_valid = match credentials {
        HandshakeCredentials::Kk { remote_public, .. }
        | HandshakeCredentials::IkPsk2Initiator { remote_public, .. } => remote_public != &[0; 32],
        HandshakeCredentials::Nn | HandshakeCredentials::IkPsk2Responder { .. } => true,
    };
    if matches && public_keys_valid {
        Ok(())
    } else {
        Err(SessionError::new(SessionErrorCode::AuthenticationFailed))
    }
}

fn build_state(
    builder: Builder<'_>,
    role: HandshakeRole,
    credentials: &HandshakeCredentials,
) -> Result<HandshakeState> {
    let builder = match credentials {
        HandshakeCredentials::Nn => builder,
        HandshakeCredentials::Kk {
            local_private,
            remote_public,
        } => builder
            .local_private_key(local_private.expose())
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?
            .remote_public_key(remote_public)
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?,
        HandshakeCredentials::IkPsk2Initiator {
            local_private,
            remote_public,
            psk,
        } => builder
            .local_private_key(local_private.expose())
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?
            .remote_public_key(remote_public)
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?
            .psk(2, psk.expose())
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?,
        HandshakeCredentials::IkPsk2Responder { local_private, psk } => builder
            .local_private_key(local_private.expose())
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?
            .psk(2, psk.expose())
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?,
    };
    match role {
        HandshakeRole::Initiator => builder.build_initiator(),
        HandshakeRole::Responder => builder.build_responder(),
    }
    .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))
}

pub struct EstablishedHandshake {
    pub(crate) transport: TransportState,
    transcript_hash: [u8; 32],
    pub(crate) limits: LimitProfile,
    pub(crate) generation: u64,
}

impl EstablishedHandshake {
    pub fn transcript_hash(&self) -> &[u8; 32] {
        &self.transcript_hash
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl fmt::Debug for EstablishedHandshake {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EstablishedHandshake")
            .field("generation", &"<redacted>")
            .field("transcript_hash", &"<redacted>")
            .finish_non_exhaustive()
    }
}
