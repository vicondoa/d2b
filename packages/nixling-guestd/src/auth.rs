use std::collections::BTreeSet;
use std::fmt;

use hmac::{Hmac, Mac};
use nixling_ipc::{
    guest_auth::{self, GuestAuthTranscript},
    guest_proto as pb,
    guest_wire::{GUEST_CONTROL_PROTOCOL_VERSION, HARD_MAX_CHUNK_BYTES, TTRPC_FRAME_CAP_BYTES},
};
use protobuf::{EnumOrUnknown, MessageField};
use sha2::Sha256;

use crate::{AuthError, TokenSource};

pub use nixling_ipc::guest_auth::{
    AuthDirection, AuthPurpose, ProofRole, AUTH_NONCE_LEN, AUTH_TAG_LEN, AUTH_TRANSCRIPT_VERSION,
    GUEST_CONTROL_AUTH_PORT,
};
pub const CONNECTION_INSTANCE_LEN: usize = 16;
pub const DEFAULT_CHALLENGE_TTL_MS: u64 = 30_000;
pub const DEFAULT_CHALLENGE_CAPACITY: usize = 128;
pub const MAX_AUTH_HEALTH_CAPABILITIES: usize = 32;
pub const MAX_AUTH_DEGRADED_SUBSYSTEMS: usize = 16;
pub const MAX_CAPABILITIES_HASH_LEN: usize = 128;
const HARD_MAX_DECODED_WRITE_STDIN_BYTES_PER_CONNECTION: u64 = 16 * 1024 * 1024;
const HARD_MAX_WRITE_STDIN_HANDLERS_PER_CONNECTION: u32 = 4;
const HARD_MAX_STDIN_QUEUE_CHUNKS_PER_EXEC: u32 = 1;
const HARD_MAX_STDOUT_LIVE_BUFFER_BYTES: u64 = 8 * 1024 * 1024;
const HARD_MAX_STDERR_LIVE_BUFFER_BYTES: u64 = 8 * 1024 * 1024;
const HARD_MAX_DETACHED_STDOUT_LOG_BYTES: u64 = 128 * 1024 * 1024;
const HARD_MAX_DETACHED_STDERR_LOG_BYTES: u64 = 128 * 1024 * 1024;
const HARD_MAX_LONG_POLL_TIMEOUT_MS: u64 = 1_000;
const HARD_MAX_SLOW_CONSUMER_GRACE_MS: u64 = 5 * 60 * 1_000;
const HARD_MAX_EXEC_SESSIONS_PER_VM: u32 = 256;
const HARD_MAX_ATTACHED_SESSIONS_PER_VM: u32 = 64;
const HARD_MAX_PENDING_READ_OUTPUT_WAITS_PER_STREAM: u32 = 512;
const HARD_MAX_PENDING_EXEC_WAITS_PER_VM: u32 = 512;
const HARD_MAX_RPC_RATE_PER_CONNECTION_PER_SECOND: u32 = 200;
const HARD_MAX_RPC_RATE_PER_VM_BURST: u32 = 1_000;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAuthError {
    MetadataMissing,
    MetadataMismatch,
    ProtocolVersionMismatch,
    TranscriptVersionMismatch,
    NonceLengthInvalid,
    TagLengthInvalid,
    BootIdMismatch,
    ChallengeCapacityExceeded,
    ChallengeNotFound,
    ChallengeExpired,
    ChallengeMismatch,
    TokenUnavailable,
    MacRejected,
    CapabilitiesUnavailable,
    InvalidCapabilitiesSnapshot,
    InvalidHealthSnapshot,
    Unauthenticated,
}

impl From<AuthError> for GuestAuthError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::TokenUnavailable => Self::TokenUnavailable,
            AuthError::MacRejected => Self::MacRejected,
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthConnectionContext {
    pub vm_id: String,
    pub protocol_version: u32,
    pub guest_control_port: u32,
    pub peer_cid: u32,
    pub direction: AuthDirection,
    pub purpose: AuthPurpose,
    pub connection_instance: [u8; CONNECTION_INSTANCE_LEN],
}

impl AuthConnectionContext {
    pub fn validate(&self) -> Result<(), GuestAuthError> {
        if self.vm_id.is_empty()
            || self.protocol_version != GUEST_CONTROL_PROTOCOL_VERSION
            || self.guest_control_port != GUEST_CONTROL_AUTH_PORT
        {
            return Err(GuestAuthError::MetadataMismatch);
        }
        Ok(())
    }
}

pub trait Clock {
    fn now_ms(&self) -> u64;
}

pub trait NonceRng {
    fn fill_nonce(&mut self, out: &mut [u8; AUTH_NONCE_LEN]) -> Result<(), GuestAuthError>;
}

pub trait BootIdSource {
    fn guest_boot_id(&self) -> Result<String, GuestAuthError>;
}

#[derive(Clone)]
pub struct CapabilitiesSnapshot {
    pub capabilities_hash: String,
    pub health: pb::HealthResponse,
    pub capabilities: pb::CapabilitiesResponse,
}

pub trait CapabilitiesProvider {
    fn snapshot(&self) -> Result<CapabilitiesSnapshot, GuestAuthError>;
}

#[derive(Clone)]
pub struct PendingChallenge {
    pub context: AuthConnectionContext,
    pub host_nonce: [u8; AUTH_NONCE_LEN],
    pub guest_nonce: [u8; AUTH_NONCE_LEN],
    pub guest_boot_id: String,
    pub issued_at_ms: u64,
}

pub trait ChallengeStore {
    fn remove_expired(&mut self, now_ms: u64, ttl_ms: u64);
    fn insert(
        &mut self,
        challenge: PendingChallenge,
        capacity: usize,
    ) -> Result<(), GuestAuthError>;
    fn consume(
        &mut self,
        connection_instance: &[u8; CONNECTION_INSTANCE_LEN],
        guest_nonce: &[u8; AUTH_NONCE_LEN],
    ) -> Option<PendingChallenge>;
    fn drop_connection(&mut self, connection_instance: &[u8; CONNECTION_INSTANCE_LEN]);
}

#[derive(Default)]
pub struct InMemoryChallengeStore {
    challenges: Vec<PendingChallenge>,
}

impl InMemoryChallengeStore {
    pub fn len(&self) -> usize {
        self.challenges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.challenges.is_empty()
    }
}

impl ChallengeStore for InMemoryChallengeStore {
    fn remove_expired(&mut self, now_ms: u64, ttl_ms: u64) {
        self.challenges
            .retain(|challenge| now_ms.saturating_sub(challenge.issued_at_ms) <= ttl_ms);
    }

    fn insert(
        &mut self,
        challenge: PendingChallenge,
        capacity: usize,
    ) -> Result<(), GuestAuthError> {
        if self.challenges.len() >= capacity {
            return Err(GuestAuthError::ChallengeCapacityExceeded);
        }
        self.challenges.push(challenge);
        Ok(())
    }

    fn consume(
        &mut self,
        connection_instance: &[u8; CONNECTION_INSTANCE_LEN],
        guest_nonce: &[u8; AUTH_NONCE_LEN],
    ) -> Option<PendingChallenge> {
        let position = self.challenges.iter().position(|challenge| {
            &challenge.context.connection_instance == connection_instance
                && &challenge.guest_nonce == guest_nonce
        })?;
        Some(self.challenges.remove(position))
    }

    fn drop_connection(&mut self, connection_instance: &[u8; CONNECTION_INSTANCE_LEN]) {
        self.challenges
            .retain(|challenge| &challenge.context.connection_instance != connection_instance);
    }
}

pub struct SharedSecretToken {
    secret: Vec<u8>,
}

impl SharedSecretToken {
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self, AuthError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(AuthError::TokenUnavailable);
        }
        Ok(Self { secret })
    }
}

impl fmt::Debug for SharedSecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedSecretToken")
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl TokenSource for SharedSecretToken {
    fn verify_tag(&self, transcript: &[u8], tag: &[u8]) -> Result<(), AuthError> {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| AuthError::TokenUnavailable)?;
        mac.update(transcript);
        mac.verify_slice(tag).map_err(|_| AuthError::MacRejected)
    }

    fn sign_tag(&self, transcript: &[u8]) -> Result<[u8; AUTH_TAG_LEN], AuthError> {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| AuthError::TokenUnavailable)?;
        mac.update(transcript);
        let bytes = mac.finalize().into_bytes();
        let mut out = [0_u8; AUTH_TAG_LEN];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

pub struct GuestAuthCore<T, R, B, C, S, K> {
    token: T,
    rng: R,
    boot_id: B,
    capabilities: C,
    challenges: S,
    clock: K,
    challenge_ttl_ms: u64,
    challenge_capacity: usize,
    authenticated_connections: BTreeSet<AuthConnectionContext>,
}

impl<T, R, B, C, S, K> GuestAuthCore<T, R, B, C, S, K> {
    pub fn new(token: T, rng: R, boot_id: B, capabilities: C, challenges: S, clock: K) -> Self {
        Self {
            token,
            rng,
            boot_id,
            capabilities,
            challenges,
            clock,
            challenge_ttl_ms: DEFAULT_CHALLENGE_TTL_MS,
            challenge_capacity: DEFAULT_CHALLENGE_CAPACITY,
            authenticated_connections: BTreeSet::new(),
        }
    }

    pub fn with_limits(mut self, challenge_ttl_ms: u64, challenge_capacity: usize) -> Self {
        self.challenge_ttl_ms = challenge_ttl_ms;
        self.challenge_capacity = challenge_capacity;
        self
    }
}

impl<T, R, B, C, S, K> GuestAuthCore<T, R, B, C, S, K>
where
    T: TokenSource,
    R: NonceRng,
    B: BootIdSource,
    C: CapabilitiesProvider,
    S: ChallengeStore,
    K: Clock,
{
    pub fn hello(
        &mut self,
        context: &AuthConnectionContext,
        request: &pb::HelloRequest,
    ) -> Result<pb::HelloResponse, GuestAuthError> {
        validate_context_and_metadata(context, request.metadata.as_ref())?;
        if request.transcript_version != AUTH_TRANSCRIPT_VERSION {
            return Err(GuestAuthError::TranscriptVersionMismatch);
        }
        let host_nonce = fixed_nonce(&request.host_nonce)?;
        let mut guest_nonce = [0_u8; AUTH_NONCE_LEN];
        self.rng.fill_nonce(&mut guest_nonce)?;
        let guest_boot_id = self.boot_id.guest_boot_id()?;
        let now_ms = self.clock.now_ms();
        self.challenges
            .remove_expired(now_ms, self.challenge_ttl_ms);
        self.challenges.insert(
            PendingChallenge {
                context: context.clone(),
                host_nonce,
                guest_nonce,
                guest_boot_id: guest_boot_id.clone(),
                issued_at_ms: now_ms,
            },
            self.challenge_capacity,
        )?;

        let mut response = pb::HelloResponse::new();
        response.guest_nonce = guest_nonce.to_vec();
        response.guest_boot_id = guest_boot_id;
        response.protocol_version = context.protocol_version;
        Ok(response)
    }

    pub fn authenticate(
        &mut self,
        context: &AuthConnectionContext,
        request: &pb::AuthenticateRequest,
    ) -> Result<pb::AuthenticateResponse, GuestAuthError> {
        validate_context_and_metadata(context, request.metadata.as_ref())?;
        if request.transcript_version != AUTH_TRANSCRIPT_VERSION {
            return Err(GuestAuthError::TranscriptVersionMismatch);
        }
        let host_nonce = fixed_nonce(&request.host_nonce)?;
        let guest_nonce = fixed_nonce(&request.guest_nonce)?;
        let host_tag = fixed_tag(&request.host_auth_tag)?;
        let challenge = self
            .challenges
            .consume(&context.connection_instance, &guest_nonce)
            .ok_or(GuestAuthError::ChallengeNotFound)?;
        let now_ms = self.clock.now_ms();
        if now_ms.saturating_sub(challenge.issued_at_ms) > self.challenge_ttl_ms {
            return Err(GuestAuthError::ChallengeExpired);
        }
        if challenge.context != *context || challenge.host_nonce != host_nonce {
            return Err(GuestAuthError::ChallengeMismatch);
        }
        if challenge.guest_boot_id != request.guest_boot_id {
            return Err(GuestAuthError::BootIdMismatch);
        }

        let host_transcript = encode_transcript(
            ProofRole::Host,
            context,
            &host_nonce,
            &guest_nonce,
            &request.guest_boot_id,
            None,
        );
        self.token.verify_tag(&host_transcript, &host_tag)?;

        let snapshot = validate_snapshot(self.capabilities.snapshot()?)?;
        let guest_transcript = encode_transcript(
            ProofRole::Guest,
            context,
            &host_nonce,
            &guest_nonce,
            &request.guest_boot_id,
            Some(snapshot.capabilities_hash.as_bytes()),
        );
        let guest_tag = self.token.sign_tag(&guest_transcript)?;
        self.authenticated_connections.insert(context.clone());

        let mut response = pb::AuthenticateResponse::new();
        response.guest_auth_tag = Some(guest_tag.to_vec());
        response.capabilities_hash = Some(snapshot.capabilities_hash);
        response.health = MessageField::some(snapshot.health);
        response.capabilities = MessageField::some(snapshot.capabilities);
        Ok(response)
    }

    pub fn health(
        &self,
        context: &AuthConnectionContext,
    ) -> Result<pb::HealthResponse, GuestAuthError> {
        self.require_authenticated(context)?;
        Ok(validate_snapshot(self.capabilities.snapshot()?)?.health)
    }

    pub fn capabilities(
        &self,
        context: &AuthConnectionContext,
    ) -> Result<pb::CapabilitiesResponse, GuestAuthError> {
        self.require_authenticated(context)?;
        Ok(validate_snapshot(self.capabilities.snapshot()?)?.capabilities)
    }

    pub fn close_connection(&mut self, context: &AuthConnectionContext) {
        self.challenges
            .drop_connection(&context.connection_instance);
        self.authenticated_connections.remove(context);
    }

    pub fn is_authenticated(&self, context: &AuthConnectionContext) -> bool {
        self.authenticated_connections.contains(context)
    }

    fn require_authenticated(&self, context: &AuthConnectionContext) -> Result<(), GuestAuthError> {
        if self.authenticated_connections.contains(context) {
            Ok(())
        } else {
            Err(GuestAuthError::Unauthenticated)
        }
    }
}

fn validate_snapshot(
    snapshot: CapabilitiesSnapshot,
) -> Result<CapabilitiesSnapshot, GuestAuthError> {
    if snapshot.capabilities_hash.is_empty()
        || snapshot.capabilities_hash.len() > MAX_CAPABILITIES_HASH_LEN
    {
        return Err(GuestAuthError::InvalidCapabilitiesSnapshot);
    }
    validate_health(&snapshot.health)?;
    validate_capabilities(&snapshot.capabilities)?;
    Ok(snapshot)
}

fn validate_health(health: &pb::HealthResponse) -> Result<(), GuestAuthError> {
    let origin = health
        .origin
        .enum_value()
        .map_err(|_| GuestAuthError::InvalidHealthSnapshot)?;
    let state = health
        .state
        .enum_value()
        .map_err(|_| GuestAuthError::InvalidHealthSnapshot)?;
    let reason = health
        .reason
        .enum_value()
        .map_err(|_| GuestAuthError::InvalidHealthSnapshot)?;
    let remediation = health
        .remediation
        .enum_value()
        .map_err(|_| GuestAuthError::InvalidHealthSnapshot)?;
    if origin != pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED {
        return Err(GuestAuthError::InvalidHealthSnapshot);
    }
    if health.protocol_version != GUEST_CONTROL_PROTOCOL_VERSION {
        return Err(GuestAuthError::InvalidHealthSnapshot);
    }
    if health.capabilities.len() > MAX_AUTH_HEALTH_CAPABILITIES
        || health.degraded_subsystems.len() > MAX_AUTH_DEGRADED_SUBSYSTEMS
        || health.capabilities.iter().any(|capability| {
            !matches!(
                capability.enum_value(),
                Ok(value) if value != pb::GuestCapability::GUEST_CAPABILITY_UNSPECIFIED
            )
        })
        || health.degraded_subsystems.iter().any(|subsystem| {
            !matches!(
                subsystem.enum_value(),
                Ok(value) if value != pb::GuestSubsystem::GUEST_SUBSYSTEM_UNSPECIFIED
            )
        })
    {
        return Err(GuestAuthError::InvalidHealthSnapshot);
    }

    match state {
        pb::HealthState::HEALTH_STATE_HEALTHY => {
            if reason != pb::HealthReason::HEALTH_REASON_NONE
                || remediation != pb::HealthRemediation::HEALTH_REMEDIATION_NONE
                || !health.degraded_subsystems.is_empty()
            {
                return Err(GuestAuthError::InvalidHealthSnapshot);
            }
        }
        pb::HealthState::HEALTH_STATE_DEGRADED => {
            let valid_reason = matches!(
                reason,
                pb::HealthReason::HEALTH_REASON_EXEC_SUBSYSTEM_UNAVAILABLE
                    | pb::HealthReason::HEALTH_REASON_LOG_STORAGE_UNAVAILABLE
                    | pb::HealthReason::HEALTH_REASON_QUOTA_EXCEEDED
                    | pb::HealthReason::HEALTH_REASON_RATE_LIMITED
                    | pb::HealthReason::HEALTH_REASON_INTERNAL_HEALTH_CHECK_FAILED
            );
            let valid_remediation = matches!(
                remediation,
                pb::HealthRemediation::HEALTH_REMEDIATION_RETRY
                    | pb::HealthRemediation::HEALTH_REMEDIATION_REDUCE_LOAD
                    | pb::HealthRemediation::HEALTH_REMEDIATION_INSPECT_GUEST_LOGS
                    | pb::HealthRemediation::HEALTH_REMEDIATION_RESTART_VM
            );
            if !valid_reason || !valid_remediation || health.degraded_subsystems.is_empty() {
                return Err(GuestAuthError::InvalidHealthSnapshot);
            }
        }
        _ => return Err(GuestAuthError::InvalidHealthSnapshot),
    }
    Ok(())
}

fn validate_capabilities(capabilities: &pb::CapabilitiesResponse) -> Result<(), GuestAuthError> {
    let Some(limits) = capabilities.limits.as_ref() else {
        return Err(GuestAuthError::InvalidCapabilitiesSnapshot);
    };
    if capabilities.protocol_version != GUEST_CONTROL_PROTOCOL_VERSION
        || capabilities.capabilities.len() > MAX_AUTH_HEALTH_CAPABILITIES
        || capabilities.capabilities.iter().any(|capability| {
            !matches!(
                capability.enum_value(),
                Ok(value) if value != pb::GuestCapability::GUEST_CAPABILITY_UNSPECIFIED
            )
        })
        || limits.max_chunk_bytes == 0
        || limits.max_chunk_bytes > HARD_MAX_CHUNK_BYTES
        || limits.max_recv_message_bytes == 0
        || limits.max_recv_message_bytes > TTRPC_FRAME_CAP_BYTES
        || limits.decoded_write_stdin_bytes_per_connection
            > HARD_MAX_DECODED_WRITE_STDIN_BYTES_PER_CONNECTION
        || limits.write_stdin_handlers_per_connection > HARD_MAX_WRITE_STDIN_HANDLERS_PER_CONNECTION
        || limits.stdin_queue_chunks_per_exec > HARD_MAX_STDIN_QUEUE_CHUNKS_PER_EXEC
        || limits.stdout_live_buffer_bytes > HARD_MAX_STDOUT_LIVE_BUFFER_BYTES
        || limits.stderr_live_buffer_bytes > HARD_MAX_STDERR_LIVE_BUFFER_BYTES
        || limits.detached_stdout_log_bytes > HARD_MAX_DETACHED_STDOUT_LOG_BYTES
        || limits.detached_stderr_log_bytes > HARD_MAX_DETACHED_STDERR_LOG_BYTES
        || limits.long_poll_timeout_ms > HARD_MAX_LONG_POLL_TIMEOUT_MS
        || limits.slow_consumer_grace_ms > HARD_MAX_SLOW_CONSUMER_GRACE_MS
        || limits.exec_sessions_per_vm > HARD_MAX_EXEC_SESSIONS_PER_VM
        || limits.attached_sessions_per_vm > HARD_MAX_ATTACHED_SESSIONS_PER_VM
        || limits.pending_read_output_waits_per_stream
            > HARD_MAX_PENDING_READ_OUTPUT_WAITS_PER_STREAM
        || limits.pending_exec_waits_per_vm > HARD_MAX_PENDING_EXEC_WAITS_PER_VM
        || limits.rpc_rate_per_connection_per_second > HARD_MAX_RPC_RATE_PER_CONNECTION_PER_SECOND
        || limits.rpc_rate_per_vm_burst > HARD_MAX_RPC_RATE_PER_VM_BURST
    {
        return Err(GuestAuthError::InvalidCapabilitiesSnapshot);
    }
    Ok(())
}

fn validate_context_and_metadata(
    context: &AuthConnectionContext,
    metadata: Option<&pb::RequestMetadata>,
) -> Result<(), GuestAuthError> {
    context.validate()?;
    let metadata = metadata.ok_or(GuestAuthError::MetadataMissing)?;
    if metadata.vm_id != context.vm_id {
        return Err(GuestAuthError::MetadataMismatch);
    }
    if metadata.protocol_version != context.protocol_version {
        return Err(GuestAuthError::ProtocolVersionMismatch);
    }
    Ok(())
}

fn fixed_nonce(value: &[u8]) -> Result<[u8; AUTH_NONCE_LEN], GuestAuthError> {
    if value.len() != AUTH_NONCE_LEN {
        return Err(GuestAuthError::NonceLengthInvalid);
    }
    let mut out = [0_u8; AUTH_NONCE_LEN];
    out.copy_from_slice(value);
    Ok(out)
}

fn fixed_tag(value: &[u8]) -> Result<[u8; AUTH_TAG_LEN], GuestAuthError> {
    if value.len() != AUTH_TAG_LEN {
        return Err(GuestAuthError::TagLengthInvalid);
    }
    let mut out = [0_u8; AUTH_TAG_LEN];
    out.copy_from_slice(value);
    Ok(out)
}

pub fn encode_transcript(
    role: ProofRole,
    context: &AuthConnectionContext,
    host_nonce: &[u8; AUTH_NONCE_LEN],
    guest_nonce: &[u8; AUTH_NONCE_LEN],
    guest_boot_id: &str,
    capabilities_hash: Option<&[u8]>,
) -> Vec<u8> {
    guest_auth::encode_transcript(&GuestAuthTranscript {
        role,
        direction: context.direction,
        purpose: context.purpose,
        vm_id: &context.vm_id,
        protocol_version: context.protocol_version,
        guest_control_port: context.guest_control_port,
        peer_cid: Some(context.peer_cid),
        host_nonce,
        guest_nonce,
        guest_boot_id,
        capabilities_hash,
    })
}

pub struct StaticCapabilitiesProvider {
    snapshot: CapabilitiesSnapshot,
}

impl StaticCapabilitiesProvider {
    pub fn healthy(capabilities_hash: impl Into<String>) -> Self {
        let mut health = pb::HealthResponse::new();
        health.origin = EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
        health.reason = EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
        health.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
        health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        health.capabilities.push(EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_HEALTH,
        ));

        let mut limits = pb::GuestEffectiveLimits::new();
        limits.max_chunk_bytes = 64 * 1024;
        limits.max_recv_message_bytes = 4 * 1024 * 1024;
        limits.stdin_queue_chunks_per_exec = 1;

        let mut capabilities = pb::CapabilitiesResponse::new();
        capabilities.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        capabilities.capabilities.push(EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_HEALTH,
        ));
        capabilities.limits = MessageField::some(limits);

        Self {
            snapshot: CapabilitiesSnapshot {
                capabilities_hash: capabilities_hash.into(),
                health,
                capabilities,
            },
        }
    }
}

impl CapabilitiesProvider for StaticCapabilitiesProvider {
    fn snapshot(&self) -> Result<CapabilitiesSnapshot, GuestAuthError> {
        Ok(self.snapshot.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &[u8] = b"test-token-material-for-auth";
    const HOST_NONCE: [u8; AUTH_NONCE_LEN] = [0x11; AUTH_NONCE_LEN];
    const GUEST_NONCE: [u8; AUTH_NONCE_LEN] = [0x22; AUTH_NONCE_LEN];
    const CONNECTION: [u8; CONNECTION_INSTANCE_LEN] = [0x33; CONNECTION_INSTANCE_LEN];

    #[derive(Clone)]
    struct StaticClock(u64);

    impl Clock for StaticClock {
        fn now_ms(&self) -> u64 {
            self.0
        }
    }

    struct FixedNonceRng([u8; AUTH_NONCE_LEN]);

    impl NonceRng for FixedNonceRng {
        fn fill_nonce(&mut self, out: &mut [u8; AUTH_NONCE_LEN]) -> Result<(), GuestAuthError> {
            *out = self.0;
            Ok(())
        }
    }

    #[derive(Clone)]
    struct StaticBoot(&'static str);

    impl BootIdSource for StaticBoot {
        fn guest_boot_id(&self) -> Result<String, GuestAuthError> {
            Ok(self.0.to_owned())
        }
    }

    fn context() -> AuthConnectionContext {
        AuthConnectionContext {
            vm_id: "corp-vm".to_owned(),
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            guest_control_port: GUEST_CONTROL_AUTH_PORT,
            peer_cid: 2,
            direction: AuthDirection::HostToGuest,
            purpose: AuthPurpose::GuestControlAuthV1,
            connection_instance: CONNECTION,
        }
    }

    fn metadata() -> MessageField<pb::RequestMetadata> {
        let mut metadata = pb::RequestMetadata::new();
        metadata.vm_id = "corp-vm".to_owned();
        metadata.request_id = "req-1".to_owned();
        metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        MessageField::some(metadata)
    }

    fn new_core() -> GuestAuthCore<
        SharedSecretToken,
        FixedNonceRng,
        StaticBoot,
        StaticCapabilitiesProvider,
        InMemoryChallengeStore,
        StaticClock,
    > {
        GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            StaticCapabilitiesProvider::healthy("caps-sha256"),
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        )
    }

    fn hello_request() -> pb::HelloRequest {
        let mut request = pb::HelloRequest::new();
        request.metadata = metadata();
        request.host_nonce = HOST_NONCE.to_vec();
        request.transcript_version = AUTH_TRANSCRIPT_VERSION;
        request
    }

    fn authenticate_request(context: &AuthConnectionContext) -> pb::AuthenticateRequest {
        let host_transcript = encode_transcript(
            ProofRole::Host,
            context,
            &HOST_NONCE,
            &GUEST_NONCE,
            "boot-1",
            None,
        );
        let host_tag = SharedSecretToken::new(TOKEN.to_vec())
            .unwrap()
            .sign_tag(&host_transcript)
            .unwrap();
        let mut request = pb::AuthenticateRequest::new();
        request.metadata = metadata();
        request.host_nonce = HOST_NONCE.to_vec();
        request.guest_nonce = GUEST_NONCE.to_vec();
        request.guest_boot_id = "boot-1".to_owned();
        request.transcript_version = AUTH_TRANSCRIPT_VERSION;
        request.host_auth_tag = host_tag.to_vec();
        request
    }

    #[test]
    fn hello_is_challenge_only() {
        let context = context();
        let mut core = new_core();
        let response = core.hello(&context, &hello_request()).unwrap();
        assert_eq!(response.guest_nonce, GUEST_NONCE);
        assert_eq!(response.guest_boot_id, "boot-1");
        assert_eq!(response.protocol_version, GUEST_CONTROL_PROTOCOL_VERSION);
        assert!(core.health(&context).is_err());
        assert!(core.capabilities(&context).is_err());
    }

    #[test]
    fn authenticate_returns_guest_proof_and_unlocks_health() {
        let context = context();
        let mut core = new_core();
        core.hello(&context, &hello_request()).unwrap();
        let response = core
            .authenticate(&context, &authenticate_request(&context))
            .unwrap();
        assert_eq!(
            response.guest_auth_tag.as_ref().unwrap().len(),
            AUTH_TAG_LEN
        );
        assert_eq!(response.capabilities_hash.as_deref(), Some("caps-sha256"));
        assert_eq!(
            response
                .health
                .as_ref()
                .unwrap()
                .state
                .enum_value()
                .unwrap(),
            pb::HealthState::HEALTH_STATE_HEALTHY
        );
        assert!(core.health(&context).is_ok());
        assert!(core.capabilities(&context).is_ok());
    }

    #[test]
    fn failed_authentication_consumes_challenge() {
        let context = context();
        let mut core = new_core();
        core.hello(&context, &hello_request()).unwrap();
        let mut bad = authenticate_request(&context);
        bad.host_auth_tag = [0x55; AUTH_TAG_LEN].to_vec();
        assert_eq!(
            core.authenticate(&context, &bad),
            Err(GuestAuthError::MacRejected)
        );
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::ChallengeNotFound)
        );
    }

    #[test]
    fn trusted_context_mismatch_rejects_and_consumes() {
        let context = context();
        let mut wrong = context.clone();
        wrong.peer_cid = 3;
        let mut core = new_core();
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&wrong, &authenticate_request(&context)),
            Err(GuestAuthError::ChallengeMismatch)
        );
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::ChallengeNotFound)
        );

        let mut core = new_core();
        core.hello(&context, &hello_request()).unwrap();
        let mut mismatched_metadata = authenticate_request(&context);
        mismatched_metadata.metadata.as_mut().unwrap().vm_id = "other-vm".to_owned();
        assert_eq!(
            core.authenticate(&context, &mismatched_metadata),
            Err(GuestAuthError::MetadataMismatch)
        );
    }

    #[test]
    fn connection_close_drops_pending_and_authenticated_state() {
        let context = context();
        let mut core = new_core();
        core.hello(&context, &hello_request()).unwrap();
        core.close_connection(&context);
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::ChallengeNotFound)
        );

        core.hello(&context, &hello_request()).unwrap();
        core.authenticate(&context, &authenticate_request(&context))
            .unwrap();
        assert!(core.health(&context).is_ok());
        core.close_connection(&context);
        assert_eq!(core.health(&context), Err(GuestAuthError::Unauthenticated));
    }

    #[test]
    fn authenticated_rpc_requires_full_trusted_context() {
        let context = context();
        let mut wrong = context.clone();
        wrong.peer_cid = 3;
        let mut core = new_core();
        core.hello(&context, &hello_request()).unwrap();
        core.authenticate(&context, &authenticate_request(&context))
            .unwrap();

        assert_eq!(core.health(&wrong), Err(GuestAuthError::Unauthenticated));
        core.close_connection(&wrong);
        assert!(core.health(&context).is_ok());
    }

    #[test]
    fn invalid_guest_reported_health_is_rejected_before_response() {
        let context = context();
        let mut provider = StaticCapabilitiesProvider::healthy("caps-sha256");
        provider.snapshot.health.origin =
            EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_HOST_SYNTHESIZED);
        let mut core = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            provider,
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        );
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::InvalidHealthSnapshot)
        );
    }

    #[test]
    fn unspecified_capability_enums_are_rejected() {
        let context = context();
        let mut provider = StaticCapabilitiesProvider::healthy("caps-sha256");
        provider.snapshot.health.capabilities = vec![EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_UNSPECIFIED,
        )];
        let mut core = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            provider,
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        );
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::InvalidHealthSnapshot)
        );

        let mut provider = StaticCapabilitiesProvider::healthy("caps-sha256");
        provider.snapshot.capabilities.capabilities = vec![EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_UNSPECIFIED,
        )];
        let mut core = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            provider,
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        );
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::InvalidCapabilitiesSnapshot)
        );
    }

    #[test]
    fn capabilities_hash_and_limits_are_bounded_before_signing() {
        let context = context();
        let provider =
            StaticCapabilitiesProvider::healthy("x".repeat(MAX_CAPABILITIES_HASH_LEN + 1));
        let mut core = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            provider,
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        );
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::InvalidCapabilitiesSnapshot)
        );

        let mut provider = StaticCapabilitiesProvider::healthy("caps-sha256");
        provider
            .snapshot
            .capabilities
            .limits
            .as_mut()
            .unwrap()
            .max_recv_message_bytes = TTRPC_FRAME_CAP_BYTES + 1;
        let mut core = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            provider,
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        );
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::InvalidCapabilitiesSnapshot)
        );

        let mut provider = StaticCapabilitiesProvider::healthy("caps-sha256");
        provider
            .snapshot
            .capabilities
            .limits
            .as_mut()
            .unwrap()
            .rpc_rate_per_vm_burst = HARD_MAX_RPC_RATE_PER_VM_BURST + 1;
        let mut core = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            provider,
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        );
        core.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            core.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::InvalidCapabilitiesSnapshot)
        );
    }

    #[test]
    fn challenge_ttl_and_capacity_fail_closed() {
        let context = context();
        let mut expired = GuestAuthCore::new(
            SharedSecretToken::new(TOKEN.to_vec()).unwrap(),
            FixedNonceRng(GUEST_NONCE),
            StaticBoot("boot-1"),
            StaticCapabilitiesProvider::healthy("caps-sha256"),
            InMemoryChallengeStore::default(),
            StaticClock(1_000),
        )
        .with_limits(0, 1);
        expired.hello(&context, &hello_request()).unwrap();
        expired.clock = StaticClock(1_001);
        assert_eq!(
            expired.authenticate(&context, &authenticate_request(&context)),
            Err(GuestAuthError::ChallengeExpired)
        );

        let mut full = new_core().with_limits(DEFAULT_CHALLENGE_TTL_MS, 1);
        full.hello(&context, &hello_request()).unwrap();
        assert_eq!(
            full.hello(&context, &hello_request()),
            Err(GuestAuthError::ChallengeCapacityExceeded)
        );
    }

    #[test]
    fn fixed_lengths_are_enforced_before_mac_verification() {
        let context = context();
        let mut core = new_core();
        let mut short_hello = hello_request();
        short_hello.host_nonce = vec![1; AUTH_NONCE_LEN - 1];
        assert_eq!(
            core.hello(&context, &short_hello),
            Err(GuestAuthError::NonceLengthInvalid)
        );

        core.hello(&context, &hello_request()).unwrap();
        let mut short_tag = authenticate_request(&context);
        short_tag.host_auth_tag = vec![1; AUTH_TAG_LEN - 1];
        assert_eq!(
            core.authenticate(&context, &short_tag),
            Err(GuestAuthError::TagLengthInvalid)
        );
    }

    #[test]
    fn transcript_golden_vectors_are_stable() {
        let context = context();
        let host = encode_transcript(
            ProofRole::Host,
            &context,
            &HOST_NONCE,
            &GUEST_NONCE,
            "boot-1",
            None,
        );
        let guest = encode_transcript(
            ProofRole::Guest,
            &context,
            &HOST_NONCE,
            &GUEST_NONCE,
            "boot-1",
            Some(b"caps-sha256"),
        );
        assert_eq!(&host[..5], &[1, 0, 0, 0, 21]);
        assert_eq!(host[5..26].as_ref(), b"guest-control-auth-v1");
        assert_eq!(guest.last().copied(), Some(b'6'));

        let token = SharedSecretToken::new(TOKEN.to_vec()).unwrap();
        assert_eq!(
            token.sign_tag(&host).unwrap(),
            [
                3, 220, 133, 211, 171, 243, 68, 134, 228, 252, 177, 161, 155, 76, 107, 100, 243,
                228, 10, 211, 105, 247, 153, 120, 47, 219, 131, 82, 195, 62, 236, 83,
            ]
        );
        assert_eq!(
            token.sign_tag(&guest).unwrap(),
            [
                21, 130, 211, 122, 6, 235, 185, 232, 14, 177, 46, 66, 248, 207, 187, 34, 254, 203,
                19, 229, 11, 83, 15, 237, 241, 184, 89, 44, 166, 109, 241, 115,
            ]
        );
    }

    #[test]
    fn auth_debug_surfaces_do_not_expose_secret_material() {
        let token = SharedSecretToken::new(TOKEN.to_vec()).unwrap();
        let rendered = format!("{token:?} {:?}", GuestAuthError::MacRejected);
        assert!(!rendered.contains("test-token-material"));
        assert!(!rendered.contains("host-nonce"));
        assert!(!rendered.contains("caps-sha256"));
    }
}
