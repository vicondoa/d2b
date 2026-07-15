use std::{error::Error, fmt};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{Generation, HandleId, LeaseId, OperationBinding, TransportBindingId},
};

/// Existing Relay listeners use a bounded sixteen-session admission queue.
pub const RELAY_ACCEPT_QUEUE_CAPACITY: u16 = 16;
/// Relay WebSocket binary frames are bounded to the existing 64 KiB pump size.
pub const RELAY_MAX_FRAME_BYTES: u32 = 64 * 1024;
/// A session-authentication prologue is bounded before any local forwarding.
pub const RELAY_MAX_PROLOGUE_BYTES: u32 = 16 * 1024;
/// A sender may make at most the existing thirty bounded listener-race retries.
pub const RELAY_SENDER_RETRY_LIMIT: u8 = 30;
/// Sender listener-race retries retain the existing one-second delay.
pub const RELAY_SENDER_RETRY_DELAY_MS: u16 = 1_000;
/// Listener reconnect backoff is capped at the existing thirty seconds.
pub const RELAY_MAX_RECONNECT_BACKOFF_MS: u32 = 30_000;
/// A stable listener resets reconnect backoff after the existing thirty seconds.
pub const RELAY_RECONNECT_STABLE_RESET_MS: u32 = 30_000;
/// Relay authentication credentials retain the existing fifteen-minute limit.
pub const RELAY_MAX_CREDENTIAL_TTL_SECS: u16 = 15 * 60;

/// The fixed production resource, authentication, retry, and reconnect posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayTransportLimits {
    accept_queue_capacity: u16,
    max_frame_bytes: u32,
    max_prologue_bytes: u32,
    sender_retry_limit: u8,
    sender_retry_delay_ms: u16,
    max_reconnect_backoff_ms: u32,
    reconnect_stable_reset_ms: u32,
    max_credential_ttl_secs: u16,
}

impl RelayTransportLimits {
    /// Return the only production limits accepted by this provider.
    pub const fn production() -> Self {
        Self {
            accept_queue_capacity: RELAY_ACCEPT_QUEUE_CAPACITY,
            max_frame_bytes: RELAY_MAX_FRAME_BYTES,
            max_prologue_bytes: RELAY_MAX_PROLOGUE_BYTES,
            sender_retry_limit: RELAY_SENDER_RETRY_LIMIT,
            sender_retry_delay_ms: RELAY_SENDER_RETRY_DELAY_MS,
            max_reconnect_backoff_ms: RELAY_MAX_RECONNECT_BACKOFF_MS,
            reconnect_stable_reset_ms: RELAY_RECONNECT_STABLE_RESET_MS,
            max_credential_ttl_secs: RELAY_MAX_CREDENTIAL_TTL_SECS,
        }
    }

    pub const fn accept_queue_capacity(self) -> u16 {
        self.accept_queue_capacity
    }

    pub const fn max_frame_bytes(self) -> u32 {
        self.max_frame_bytes
    }

    pub const fn max_prologue_bytes(self) -> u32 {
        self.max_prologue_bytes
    }

    pub const fn sender_retry_limit(self) -> u8 {
        self.sender_retry_limit
    }

    pub const fn sender_retry_delay_ms(self) -> u16 {
        self.sender_retry_delay_ms
    }

    pub const fn max_reconnect_backoff_ms(self) -> u32 {
        self.max_reconnect_backoff_ms
    }

    pub const fn reconnect_stable_reset_ms(self) -> u32 {
        self.reconnect_stable_reset_ms
    }

    pub const fn max_credential_ttl_secs(self) -> u16 {
        self.max_credential_ttl_secs
    }
}

/// An agent-local Azure Relay rendezvous identifier.
///
/// It is deliberately not serializable. The co-located port resolves it to
/// Relay namespace/entity data without putting a URL or path in provider DTOs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelayRendezvousId(String);

impl RelayRendezvousId {
    pub fn parse(value: impl Into<String>) -> Result<Self, RelayIdentifierError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if valid {
            Ok(Self(value))
        } else {
            Err(RelayIdentifierError)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RelayRendezvousId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RelayRendezvousId")
            .field(&"<redacted>")
            .finish()
    }
}

/// A rendezvous identifier failed its closed bounded syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayIdentifierError;

impl fmt::Display for RelayIdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid opaque relay rendezvous identifier")
    }
}

impl Error for RelayIdentifierError {}

#[derive(Clone, PartialEq, Eq)]
struct RelayCallBinding {
    operation: OperationBinding,
    transport_binding_id: TransportBindingId,
    rendezvous_id: RelayRendezvousId,
    deadline_remaining_ms: u32,
    limits: RelayTransportLimits,
}

impl RelayCallBinding {
    fn debug(&self, name: &str, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(name)
            .field("provider_generation", &self.operation.provider_generation)
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

macro_rules! call_binding_accessors {
    () => {
        pub fn operation(&self) -> &OperationBinding {
            &self.binding.operation
        }

        pub fn transport_binding_id(&self) -> &TransportBindingId {
            &self.binding.transport_binding_id
        }

        pub fn rendezvous_id(&self) -> &RelayRendezvousId {
            &self.binding.rendezvous_id
        }

        pub const fn deadline_remaining_ms(&self) -> u32 {
            self.binding.deadline_remaining_ms
        }

        pub const fn limits(&self) -> RelayTransportLimits {
            self.binding.limits
        }
    };
}

/// A sender/listener open request passed only to the co-located Relay port.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayOpenRequest {
    binding: RelayCallBinding,
    credential_lease_id: LeaseId,
}

impl RelayOpenRequest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        operation: OperationBinding,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        credential_lease_id: LeaseId,
        deadline_remaining_ms: u32,
        limits: RelayTransportLimits,
    ) -> Self {
        Self {
            binding: RelayCallBinding {
                operation,
                transport_binding_id,
                rendezvous_id,
                deadline_remaining_ms,
                limits,
            },
            credential_lease_id,
        }
    }

    call_binding_accessors!();

    pub fn credential_lease_id(&self) -> &LeaseId {
        &self.credential_lease_id
    }
}

impl fmt::Debug for RelayOpenRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.binding.debug("RelayOpenRequest", formatter)
    }
}

/// A non-mutating Relay resource inspection request.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayInspectRequest {
    binding: RelayCallBinding,
}

impl RelayInspectRequest {
    pub(crate) fn new(
        operation: OperationBinding,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        deadline_remaining_ms: u32,
        limits: RelayTransportLimits,
    ) -> Self {
        Self {
            binding: RelayCallBinding {
                operation,
                transport_binding_id,
                rendezvous_id,
                deadline_remaining_ms,
                limits,
            },
        }
    }

    call_binding_accessors!();
}

impl fmt::Debug for RelayInspectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.binding.debug("RelayInspectRequest", formatter)
    }
}

/// An idempotent binding-close request.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayCloseRequest {
    binding: RelayCallBinding,
}

impl RelayCloseRequest {
    pub(crate) fn new(
        operation: OperationBinding,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        deadline_remaining_ms: u32,
        limits: RelayTransportLimits,
    ) -> Self {
        Self {
            binding: RelayCallBinding {
                operation,
                transport_binding_id,
                rendezvous_id,
                deadline_remaining_ms,
                limits,
            },
        }
    }

    call_binding_accessors!();
}

impl fmt::Debug for RelayCloseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.binding.debug("RelayCloseRequest", formatter)
    }
}

/// Exact non-secret evidence required to re-adopt a live Relay resource.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayExpectedResource {
    provider_id: ProviderId,
    handle_id: HandleId,
    provider_generation: Generation,
    resource_generation: Generation,
}

impl RelayExpectedResource {
    pub(crate) fn new(
        provider_id: ProviderId,
        handle_id: HandleId,
        provider_generation: Generation,
        resource_generation: Generation,
    ) -> Self {
        Self {
            provider_id,
            handle_id,
            provider_generation,
            resource_generation,
        }
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn handle_id(&self) -> &HandleId {
        &self.handle_id
    }

    pub const fn provider_generation(&self) -> Generation {
        self.provider_generation
    }

    pub const fn resource_generation(&self) -> Generation {
        self.resource_generation
    }
}

impl fmt::Debug for RelayExpectedResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayExpectedResource")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .finish_non_exhaustive()
    }
}

/// A restart-adoption request. Identity and both generations are mandatory.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayAdoptRequest {
    binding: RelayCallBinding,
    expected: RelayExpectedResource,
}

impl RelayAdoptRequest {
    pub(crate) fn new(
        operation: OperationBinding,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        expected: RelayExpectedResource,
        deadline_remaining_ms: u32,
        limits: RelayTransportLimits,
    ) -> Self {
        Self {
            binding: RelayCallBinding {
                operation,
                transport_binding_id,
                rendezvous_id,
                deadline_remaining_ms,
                limits,
            },
            expected,
        }
    }

    call_binding_accessors!();

    pub fn expected(&self) -> &RelayExpectedResource {
        &self.expected
    }
}

impl fmt::Debug for RelayAdoptRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayAdoptRequest")
            .field(
                "provider_generation",
                &self.binding.operation.provider_generation,
            )
            .field("expected", &self.expected)
            .field("deadline_remaining_ms", &self.binding.deadline_remaining_ms)
            .field("limits", &self.binding.limits)
            .finish_non_exhaustive()
    }
}

/// Closed Relay resource states visible to the canonical provider adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayResourceState {
    Connected,
    Listening,
    Closed,
    Unknown,
}

/// Non-secret resource evidence returned by the co-located Relay port.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayResource {
    provider_id: ProviderId,
    transport_binding_id: TransportBindingId,
    rendezvous_id: RelayRendezvousId,
    handle_id: HandleId,
    provider_generation: Generation,
    resource_generation: Generation,
    state: RelayResourceState,
    expires_at_unix_ms: Option<u64>,
}

impl RelayResource {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider_id: ProviderId,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        handle_id: HandleId,
        provider_generation: Generation,
        resource_generation: Generation,
        state: RelayResourceState,
        expires_at_unix_ms: Option<u64>,
    ) -> Self {
        Self {
            provider_id,
            transport_binding_id,
            rendezvous_id,
            handle_id,
            provider_generation,
            resource_generation,
            state,
            expires_at_unix_ms,
        }
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn transport_binding_id(&self) -> &TransportBindingId {
        &self.transport_binding_id
    }

    pub fn rendezvous_id(&self) -> &RelayRendezvousId {
        &self.rendezvous_id
    }

    pub fn handle_id(&self) -> &HandleId {
        &self.handle_id
    }

    pub const fn provider_generation(&self) -> Generation {
        self.provider_generation
    }

    pub const fn resource_generation(&self) -> Generation {
        self.resource_generation
    }

    pub const fn state(&self) -> RelayResourceState {
        self.state
    }

    pub const fn expires_at_unix_ms(&self) -> Option<u64> {
        self.expires_at_unix_ms
    }
}

impl fmt::Debug for RelayResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayResource")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .field("state", &self.state)
            .field("has_expiry", &self.expires_at_unix_ms.is_some())
            .finish_non_exhaustive()
    }
}

/// A bounded inspection result with no endpoint, token, path, or SDK payload.
#[derive(Clone, PartialEq, Eq)]
pub enum RelayInspection {
    Absent,
    Present(RelayResource),
}

impl fmt::Debug for RelayInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("RelayInspection::Absent"),
            Self::Present(resource) => formatter
                .debug_tuple("RelayInspection::Present")
                .field(resource)
                .finish(),
        }
    }
}

/// Idempotent close outcomes produced by the Relay control port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayCloseOutcome {
    Closed,
    AlreadyClosed,
    NotFound,
}

/// Closed, token-free failures produced by Relay control/client implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayPortFailure {
    CredentialLeaseInvalid,
    AuthenticationFailed,
    HandshakeTimeout,
    Unavailable,
    QueueFull,
    FrameTooLarge,
    Cancelled,
    CompletionAmbiguous,
    BindingMismatch,
    IdentityMismatch,
    GenerationMismatch,
}

impl fmt::Display for RelayPortFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CredentialLeaseInvalid => "relay credential lease is invalid",
            Self::AuthenticationFailed => "relay authentication failed",
            Self::HandshakeTimeout => "relay handshake deadline expired",
            Self::Unavailable => "relay transport is unavailable",
            Self::QueueFull => "relay transport queue is full",
            Self::FrameTooLarge => "relay transport frame exceeds its bound",
            Self::Cancelled => "relay operation was cancelled",
            Self::CompletionAmbiguous => "relay mutation completion is ambiguous",
            Self::BindingMismatch => "relay binding identity mismatch",
            Self::IdentityMismatch => "relay resource identity mismatch",
            Self::GenerationMismatch => "relay resource generation mismatch",
        })
    }
}

impl Error for RelayPortFailure {}

/// Co-located Azure Relay control/client boundary.
///
/// Implementations resolve the opaque lease, rendezvous, and binding IDs inside
/// the credential-owning provider agent. Secret material and endpoint URLs must
/// not be returned, logged, or placed in these calls. Relay authentication only
/// establishes the transport; this port receives no d2b principal or
/// authorization decision and cannot grant d2b authority.
///
/// Every mutating method must key idempotency by the supplied operation ID and
/// request digest, replay the same proven result for that exact pair, and reject
/// a conflicting replay. Implementations must enforce [`RelayTransportLimits`]
/// rather than treating them as advisory. Dropping a method future is the
/// cancellation path and must stop further local I/O; a remotely ambiguous
/// mutation is reported as [`RelayPortFailure::CompletionAmbiguous`].
#[async_trait]
pub trait RelayControlPort: Send + Sync {
    /// Connect the sender side using only the exact `Connect` credential lease.
    async fn connect(&self, request: RelayOpenRequest) -> Result<RelayResource, RelayPortFailure>;

    /// Arm the listener using only the exact `Listen` credential lease.
    ///
    /// This returns only after the control channel is registered, preserving
    /// the production ordering that prevents sender rendezvous resets.
    async fn listen(&self, request: RelayOpenRequest) -> Result<RelayResource, RelayPortFailure>;

    /// Inspect the configured rendezvous without returning provider SDK data.
    async fn inspect(
        &self,
        request: RelayInspectRequest,
    ) -> Result<RelayInspection, RelayPortFailure>;

    /// Close the configured binding idempotently.
    async fn close(
        &self,
        request: RelayCloseRequest,
    ) -> Result<RelayCloseOutcome, RelayPortFailure>;

    /// Re-adopt only the exact resource identity and generations supplied.
    async fn adopt(&self, request: RelayAdoptRequest) -> Result<RelayResource, RelayPortFailure>;
}
