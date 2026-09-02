//! Child-local ZoneLink reconciliation (`ADR046-routing-004`).
//!
//! This module owns the crash-safe child-local enrollment-and-session state
//! machine `Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready` and the
//! reconciliation planner that turns one observed link event into a durable
//! record mutation plus a bounded set of link effects.
//!
//! The module is a planner, not a runner. It performs no transport work, opens
//! no session, and holds no key material: an enrollment is represented only by
//! a bounded fingerprint token and an allocator-issued bootstrap PSK is
//! represented only by its issuance ordinal and absolute expiry. Actual Noise
//! handshakes, advertisement signing, and route admission belong to the
//! transport Provider, the enrolled ComponentSession, and the parent
//! allocator's route engine respectively.
//!
//! Two invariants are structural rather than advisory:
//!
//! * **Commit before effect.** A plan computed by [`ZoneLinkHandler::begin`]
//!   releases nothing. The durable record mutation is applied by
//!   [`ZoneLinkHandler::commit`], which is the sole issuer of a
//!   [`ZoneLinkCommitProof`]. Effects are obtainable only by consuming that
//!   proof through [`ZoneLinkHandler::release_effects`]. An aborted pass, a
//!   stale proof, or a restart in the middle of a pass therefore cannot
//!   release an effect.
//! * **Per-link single flight.** At most one pass may be open at a time; a
//!   second [`ZoneLinkHandler::begin`] fails closed with
//!   [`ZoneLinkError::ReconcileInFlight`].
//!
//! Restart safety is expressed by [`ZoneLinkHandler::restore`], which derives
//! the session state purely from the durable [`ZoneLinkRecord`]. Replaying an
//! already-committed event after restart is a no-op that plans no effect.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
};

use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ResourceUid, ZoneRevision, identity::ReconnectGeneration,
};
use d2b_contracts_zone_session::v3::component_session::{OperationClass, OperationId};
use d2b_contracts_zone_session::v3::zone_routing::{
    MAX_ZONE_ROUTE_ENTRIES, ZoneLinkControllerGeneration, ZoneLinkRouteAdmissionRequest,
    ZoneRouteCapability, ZoneRouteFailClosedReason, ZoneSigningKeyFingerprint, ZoneTreeEdge,
};
use serde::{Deserialize, Serialize};

static NEXT_ZONE_LINK_HANDLER_OWNER: AtomicU64 = AtomicU64::new(1);

fn next_zone_link_handler_owner() -> u64 {
    loop {
        let owner = NEXT_ZONE_LINK_HANDLER_OWNER.fetch_add(1, Ordering::Relaxed);
        if owner != 0 {
            return owner;
        }
    }
}

/// Default absolute lifetime of one allocator-issued bootstrap PSK.
pub const BOOTSTRAP_PSK_TTL_MS_DEFAULT: u64 = 300_000;

/// Lowest permitted bootstrap PSK lifetime.
pub const BOOTSTRAP_PSK_TTL_MS_MIN: u64 = 60_000;

/// Highest permitted bootstrap PSK lifetime.
pub const BOOTSTRAP_PSK_TTL_MS_MAX: u64 = 3_600_000;

/// Default maximum lifetime of one enrolled KK session.
pub const KK_SESSION_MAX_LIFETIME_MS_DEFAULT: u64 = 86_400_000;

/// Lowest permitted enrolled KK session lifetime.
pub const KK_SESSION_MAX_LIFETIME_MS_MIN: u64 = 3_600_000;

/// Highest permitted enrolled KK session lifetime.
pub const KK_SESSION_MAX_LIFETIME_MS_MAX: u64 = 604_800_000;

/// Admission ceiling for `spec.limits.maxPendingIntents`.
pub const MAX_PENDING_LOCAL_INTENTS: u32 = 1024;

/// Admission ceiling for `spec.limits.maxActiveStreams`.
pub const MAX_ACTIVE_STREAMS: u32 = 128;

/// Maximum committed route-admission operation IDs retained for one immutable
/// ZoneLink identity and controller generation.
///
/// IDs are never evicted: reaching this ceiling refuses new admissions until
/// the ZoneLink identity or controller generation is replaced.
pub const MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS: usize = MAX_ZONE_ROUTE_ENTRIES;

/// Maximum canonical bytes in the durable route-admission dedup envelope.
pub const MAX_ROUTE_ADMISSION_DEDUP_BYTES: usize = 2 * 1024 * 1024;

/// Version of the durable route-admission operation-id envelope.
pub const ZONE_LINK_ROUTE_ADMISSION_DEDUP_VERSION: u32 = 1;

/// Closed metric label key set for every ZoneLink aggregate metric.
///
/// The set deliberately excludes `vm`, `zone`, `zone_id`, `zone_uid`, and
/// `link_name_hash`, and every admitted value is drawn from a closed enum, so
/// no ZoneLink, Zone, or resource identity can enter a label value.
pub const ZONE_LINK_METRIC_LABEL_KEYS: &[&str] = &["phase", "reason", "outcome"];

/// Child-local ZoneLink enrollment-and-session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkSessionState {
    /// No valid sealed enrollment exists; a fresh single-use PSK is required.
    Unenrolled,
    /// One-time Noise IKpsk2 bootstrap handshake in progress.
    IKpsk2,
    /// Sealed enrollment record durably persisted; PSK consumed.
    EnrollmentCommitted,
    /// Enrolled Noise KK handshake in progress.
    Kk,
    /// Enrolled KK session established; full resource traffic permitted.
    Ready,
}

impl ZoneLinkSessionState {
    /// Return the closed lowercase label for this state.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unenrolled => "unenrolled",
            Self::IKpsk2 => "ikpsk2",
            Self::EnrollmentCommitted => "enrollment-committed",
            Self::Kk => "kk",
            Self::Ready => "ready",
        }
    }

    /// Whether resource-plane traffic is permitted in this state.
    pub const fn permits_resource_traffic(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// D088 resource phase reported by the child-local handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkPhase {
    /// Transport not yet connected or child not yet authorized.
    Pending,
    /// Session established, child authorized, advertisement current.
    Ready,
    /// Session impaired but not permanently failed.
    Degraded,
    /// Retry policy exhausted or child permanently denied.
    Failed,
    /// Session state cannot currently be proven.
    Unknown,
}

impl ZoneLinkPhase {
    /// Return the closed label for this phase.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Ready => "Ready",
            Self::Degraded => "Degraded",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
        }
    }
}

/// Closed fail-closed reason for every ZoneLink refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkError {
    /// A PSK was presented that this link already burned.
    BootstrapPskConsumed,
    /// A PSK was presented after its absolute expiry.
    BootstrapPskExpired,
    /// A PSK was presented that a later issuance durably invalidated.
    BootstrapPskInvalidated,
    /// The one-time IKpsk2 bootstrap handshake failed; the PSK is burned.
    BootstrapHandshakeFailed,
    /// An enrolled KK peer key did not match the sealed enrollment.
    EnrollmentKeyMismatch,
    /// The sealed enrollment and active session were durably revoked.
    Revoked,
    /// The enrolled session is not established.
    Disconnected,
    /// Resource-plane traffic was attempted before `Ready`.
    ResourceTrafficBeforeReady,
    /// The bounded reconnect budget for this window is exhausted.
    ReconnectBudgetExhausted,
    /// The bounded outbound intent queue is full.
    IntentQueueFull,
    /// The link is administratively disabled.
    Disabled,
    /// The requested transition is not permitted from the current state.
    InvalidTransition,
    /// Another reconcile pass is already open for this link.
    ReconcileInFlight,
    /// The presented commit proof does not match the committed pass.
    StaleCommitProof,
    /// The supplied limits or protocol constants are out of their frozen range.
    InvalidLimits,
    /// A bootstrap PSK belongs to another controller generation.
    BootstrapPskGenerationMismatch,
    /// The durable route-admission binding is absent or invalid.
    RouteAdmissionBindingInvalid,
    /// The route cursor owner has not been durably adopted after restart.
    RouteAdmissionCursorUnavailable,
    /// The requested route operation does not match the committed policy.
    RouteAdmissionVerbMismatch,
    /// The operation identity was already durably committed for this link.
    RouteAdmissionConflict,
    /// A route admission was requested after durable revocation.
    RouteAdmissionRevoked,
    /// The durable route-admission operation-id set is full.
    RouteAdmissionOperationCapacity,
    /// The durable route-admission operation-id envelope is malformed.
    RouteAdmissionDedupInvalid,
    /// The durable route-admission operation-id envelope version is unknown.
    RouteAdmissionDedupVersionMismatch,
    /// Durable route-admission state belongs to another ZoneLink identity.
    RouteAdmissionDedupIdentityMismatch,
    /// Recovery attempted to replace an already populated committed set.
    RouteAdmissionDedupConflict,
}

impl ZoneLinkError {
    /// Return the closed kebab-case reason label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::BootstrapPskConsumed => "bootstrap-psk-consumed",
            Self::BootstrapPskExpired => "bootstrap-psk-expired",
            Self::BootstrapPskInvalidated => "bootstrap-psk-invalidated",
            Self::BootstrapHandshakeFailed => "bootstrap-handshake-failed",
            Self::EnrollmentKeyMismatch => "zone-link-enrollment-key-mismatch",
            Self::Revoked => "zone-link-revoked",
            Self::Disconnected => ZoneRouteFailClosedReason::ZoneLinkDisconnected.label(),
            Self::ResourceTrafficBeforeReady => "zone-link-not-ready",
            Self::ReconnectBudgetExhausted => "reconnect-budget-exhausted",
            Self::IntentQueueFull => ZoneRouteFailClosedReason::QueueFullDropNew.label(),
            Self::Disabled => "zone-link-disabled",
            Self::InvalidTransition => "invalid-transition",
            Self::ReconcileInFlight => "reconcile-in-flight",
            Self::StaleCommitProof => "stale-commit-proof",
            Self::InvalidLimits => "invalid-limits",
            Self::BootstrapPskGenerationMismatch => "bootstrap-psk-generation-mismatch",
            Self::RouteAdmissionBindingInvalid => "route-admission-binding-invalid",
            Self::RouteAdmissionCursorUnavailable => "route-admission-cursor-unavailable",
            Self::RouteAdmissionVerbMismatch => "route-admission-verb-mismatch",
            Self::RouteAdmissionConflict => "route-admission-conflict",
            Self::RouteAdmissionRevoked => "route-admission-revoked",
            Self::RouteAdmissionOperationCapacity => "route-admission-operation-capacity",
            Self::RouteAdmissionDedupInvalid => "route-admission-dedup-invalid",
            Self::RouteAdmissionDedupVersionMismatch => "route-admission-dedup-version-mismatch",
            Self::RouteAdmissionDedupIdentityMismatch => "route-admission-dedup-identity-mismatch",
            Self::RouteAdmissionDedupConflict => "route-admission-dedup-conflict",
        }
    }
}

impl core::fmt::Display for ZoneLinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for ZoneLinkError {}

/// Bounded ZoneLink connection and queue limits from `spec.limits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneLinkLimits {
    max_pending_intents: u32,
    max_active_streams: u32,
    reconnect_max_attempts: u32,
    reconnect_window_secs: u32,
}

impl ZoneLinkLimits {
    /// Validate one complete `spec.limits` object against its frozen bounds.
    pub const fn new(
        max_pending_intents: u32,
        max_active_streams: u32,
        reconnect_max_attempts: u32,
        reconnect_window_secs: u32,
    ) -> Result<Self, ZoneLinkError> {
        if max_pending_intents == 0
            || max_pending_intents > MAX_PENDING_LOCAL_INTENTS
            || max_active_streams > MAX_ACTIVE_STREAMS
            || reconnect_max_attempts == 0
            || reconnect_window_secs == 0
        {
            return Err(ZoneLinkError::InvalidLimits);
        }
        Ok(Self {
            max_pending_intents,
            max_active_streams,
            reconnect_max_attempts,
            reconnect_window_secs,
        })
    }

    /// Return the bounded outbound intent queue ceiling.
    pub const fn max_pending_intents(self) -> u32 {
        self.max_pending_intents
    }

    /// Return the bounded concurrent named-stream ceiling.
    pub const fn max_active_streams(self) -> u32 {
        self.max_active_streams
    }

    /// Return the bounded reconnect attempt budget.
    pub const fn reconnect_max_attempts(self) -> u32 {
        self.reconnect_max_attempts
    }

    /// Return the reconnect budget window in seconds.
    pub const fn reconnect_window_secs(self) -> u32 {
        self.reconnect_window_secs
    }
}

impl Default for ZoneLinkLimits {
    fn default() -> Self {
        Self {
            max_pending_intents: 256,
            max_active_streams: 32,
            reconnect_max_attempts: 10,
            reconnect_window_secs: 300,
        }
    }
}

/// Frozen protocol constants owned by the handler and the parent allocator.
///
/// These are deliberately not ZoneLink `spec` fields; the locked six-field
/// schema is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneLinkKeyPolicy {
    bootstrap_psk_ttl_ms: u64,
    kk_session_max_lifetime_ms: u64,
}

impl ZoneLinkKeyPolicy {
    /// Validate both cryptoperiods against their frozen ranges.
    pub const fn new(
        bootstrap_psk_ttl_ms: u64,
        kk_session_max_lifetime_ms: u64,
    ) -> Result<Self, ZoneLinkError> {
        if bootstrap_psk_ttl_ms < BOOTSTRAP_PSK_TTL_MS_MIN
            || bootstrap_psk_ttl_ms > BOOTSTRAP_PSK_TTL_MS_MAX
            || kk_session_max_lifetime_ms < KK_SESSION_MAX_LIFETIME_MS_MIN
            || kk_session_max_lifetime_ms > KK_SESSION_MAX_LIFETIME_MS_MAX
        {
            return Err(ZoneLinkError::InvalidLimits);
        }
        Ok(Self {
            bootstrap_psk_ttl_ms,
            kk_session_max_lifetime_ms,
        })
    }

    /// Return the bootstrap PSK lifetime.
    pub const fn bootstrap_psk_ttl_ms(self) -> u64 {
        self.bootstrap_psk_ttl_ms
    }

    /// Return the enrolled KK session maximum lifetime.
    pub const fn kk_session_max_lifetime_ms(self) -> u64 {
        self.kk_session_max_lifetime_ms
    }
}

impl Default for ZoneLinkKeyPolicy {
    fn default() -> Self {
        Self {
            bootstrap_psk_ttl_ms: BOOTSTRAP_PSK_TTL_MS_DEFAULT,
            kk_session_max_lifetime_ms: KK_SESSION_MAX_LIFETIME_MS_DEFAULT,
        }
    }
}

/// One allocator-issued single-use bootstrap PSK handle.
///
/// The handle carries no key material. It names the issuance ordinal the
/// allocator assigned within this link identity generation and the absolute
/// expiry after which bootstrap admission refuses it.
#[derive(Clone, PartialEq, Eq)]
pub struct BootstrapPsk {
    generation: ZoneLinkControllerGeneration,
    issuance: u64,
    expires_at_ms: u64,
}

impl BootstrapPsk {
    /// Bind one allocator issuance ordinal to its absolute expiry.
    pub const fn issue(
        generation: ZoneLinkControllerGeneration,
        issuance: u64,
        expires_at_ms: u64,
    ) -> Self {
        Self {
            generation,
            issuance,
            expires_at_ms,
        }
    }

    /// Borrow the link identity generation this PSK was issued against.
    pub const fn generation(&self) -> &ZoneLinkControllerGeneration {
        &self.generation
    }

    /// Return the allocator issuance ordinal.
    pub const fn issuance(&self) -> u64 {
        self.issuance
    }

    /// Return the absolute expiry.
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
}

impl core::fmt::Debug for BootstrapPsk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BootstrapPsk")
            .field("has_generation", &true)
            .field("issuance", &self.issuance)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// One sealed enrollment record: the child static key-pin bound to the
/// acknowledged child Zone uid.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedEnrollment {
    child_zone_uid: ResourceUid,
    key_fingerprint: ZoneSigningKeyFingerprint,
}

impl SealedEnrollment {
    /// Bind one acknowledged child Zone uid to its pinned static key.
    pub const fn new(
        child_zone_uid: ResourceUid,
        key_fingerprint: ZoneSigningKeyFingerprint,
    ) -> Self {
        Self {
            child_zone_uid,
            key_fingerprint,
        }
    }

    /// Borrow the acknowledged child Zone uid.
    pub const fn child_zone_uid(&self) -> &ResourceUid {
        &self.child_zone_uid
    }

    /// Borrow the pinned child static key fingerprint.
    pub const fn key_fingerprint(&self) -> &ZoneSigningKeyFingerprint {
        &self.key_fingerprint
    }
}

impl core::fmt::Debug for SealedEnrollment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SealedEnrollment")
            .field("has_child_zone_uid", &true)
            .field("has_key_fingerprint", &true)
            .finish()
    }
}

/// Bounded monotonic child-store route cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ZoneLinkCursor {
    last_sent_revision: u64,
    last_acked_revision: u64,
    last_received_revision: u64,
    last_applied_revision: u64,
}

impl ZoneLinkCursor {
    /// Return the highest revision sent to the parent.
    pub const fn last_sent_revision(self) -> u64 {
        self.last_sent_revision
    }

    /// Return the highest revision acknowledged by the parent.
    pub const fn last_acked_revision(self) -> u64 {
        self.last_acked_revision
    }

    /// Return the highest parent revision received.
    pub const fn last_received_revision(self) -> u64 {
        self.last_received_revision
    }

    /// Return the highest parent revision applied locally.
    pub const fn last_applied_revision(self) -> u64 {
        self.last_applied_revision
    }

    fn advance(self, sent: u64, acked: u64, received: u64, applied: u64) -> Self {
        Self {
            last_sent_revision: self.last_sent_revision.max(sent),
            last_acked_revision: self.last_acked_revision.max(acked),
            last_received_revision: self.last_received_revision.max(received),
            last_applied_revision: self.last_applied_revision.max(applied),
        }
    }
}

/// Durable identity and policy for one child-to-parent ZoneLink route.
///
/// This is trusted state derived from the committed ZoneLink resource,
/// topology, authenticated controller/session generations, and policy index.
/// A route request never carries any of these fields.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneLinkRouteBinding {
    zone_link_uid: ResourceUid,
    edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    reconnect_generation: ReconnectGeneration,
    source_zone_uid: ResourceUid,
    target_zone_uid: ResourceUid,
    policy_revision: ZoneRevision,
    required_capability: ZoneRouteCapability,
    verb: OperationClass,
}

impl ZoneLinkRouteBinding {
    /// Bind a route to the exact committed ZoneLink and session policy.
    ///
    /// The values must come from the trusted Zone authority. The public
    /// operation request contains only an operation id and verb.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone_link_uid: ResourceUid,
        edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        reconnect_generation: ReconnectGeneration,
        source_zone_uid: ResourceUid,
        target_zone_uid: ResourceUid,
        policy_revision: ZoneRevision,
        required_capability: ZoneRouteCapability,
        verb: OperationClass,
    ) -> Result<Self, ZoneLinkError> {
        if source_zone_uid == target_zone_uid
            || policy_revision.get() == 0
            || verb == OperationClass::Attach
        {
            return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
        }
        Ok(Self {
            zone_link_uid,
            edge,
            controller_generation,
            reconnect_generation,
            source_zone_uid,
            target_zone_uid,
            policy_revision,
            required_capability,
            verb,
        })
    }

    /// Borrow the ZoneLink resource UID.
    pub const fn zone_link_uid(&self) -> &ResourceUid {
        &self.zone_link_uid
    }

    /// Borrow the exact parent/child edge.
    pub const fn edge(&self) -> &ZoneTreeEdge {
        &self.edge
    }

    /// Borrow the controller generation.
    pub const fn controller_generation(&self) -> &ZoneLinkControllerGeneration {
        &self.controller_generation
    }

    /// Return the authenticated reconnect generation.
    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    /// Borrow the source Zone UID.
    pub const fn source_zone_uid(&self) -> &ResourceUid {
        &self.source_zone_uid
    }

    /// Borrow the target Zone UID.
    pub const fn target_zone_uid(&self) -> &ResourceUid {
        &self.target_zone_uid
    }

    /// Return the committed policy revision.
    pub const fn policy_revision(&self) -> ZoneRevision {
        self.policy_revision
    }

    /// Borrow the required route capability.
    pub const fn required_capability(&self) -> &ZoneRouteCapability {
        &self.required_capability
    }

    /// Return the committed route operation class.
    pub const fn verb(&self) -> OperationClass {
        self.verb
    }
}

impl core::fmt::Debug for ZoneLinkRouteBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneLinkRouteBinding")
            .field("edge", &self.edge)
            .field("verb", &self.verb)
            .field("policy_revision", &self.policy_revision)
            .finish_non_exhaustive()
    }
}

/// Post-commit route-admission input for the existing sealed issuer.
///
/// This value can only be produced from a committed ZoneLink pass. It carries
/// no clock or expiry claim; the downstream runtime-owned issuer supplies
/// daemon time and its bounded lifetime. Consuming it yields only the
/// untrusted operation request accepted by that issuer.
pub(crate) struct ZoneLinkRouteAdmissionContext {
    sequence: u64,
    binding: ZoneLinkRouteBinding,
    request: ZoneLinkRouteAdmissionRequest,
}

impl ZoneLinkRouteAdmissionContext {
    fn new(
        sequence: u64,
        binding: ZoneLinkRouteBinding,
        request: ZoneLinkRouteAdmissionRequest,
    ) -> Self {
        Self {
            sequence,
            binding,
            request,
        }
    }

    /// Borrow the committed ZoneLink UID.
    #[cfg(test)]
    pub(crate) const fn zone_link_uid(&self) -> &ResourceUid {
        self.binding.zone_link_uid()
    }

    /// Borrow the committed route edge.
    #[cfg(test)]
    pub(crate) const fn edge(&self) -> &ZoneTreeEdge {
        self.binding.edge()
    }

    /// Borrow the committed controller generation.
    #[cfg(test)]
    pub(crate) const fn controller_generation(&self) -> &ZoneLinkControllerGeneration {
        self.binding.controller_generation()
    }

    /// Return the committed reconnect generation.
    #[cfg(test)]
    pub(crate) const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.binding.reconnect_generation()
    }

    /// Borrow the committed source Zone UID.
    #[cfg(test)]
    pub(crate) const fn source_zone_uid(&self) -> &ResourceUid {
        self.binding.source_zone_uid()
    }

    /// Borrow the committed target Zone UID.
    #[cfg(test)]
    pub(crate) const fn target_zone_uid(&self) -> &ResourceUid {
        self.binding.target_zone_uid()
    }

    /// Return the committed policy revision.
    #[cfg(test)]
    pub(crate) const fn policy_revision(&self) -> ZoneRevision {
        self.binding.policy_revision()
    }

    /// Borrow the committed required capability.
    #[cfg(test)]
    pub(crate) const fn required_capability(&self) -> &ZoneRouteCapability {
        self.binding.required_capability()
    }

    /// Borrow the requested operation identity.
    #[cfg(test)]
    pub(crate) const fn operation_id(&self) -> &OperationId {
        self.request.operation_id()
    }

    /// Return the requested operation class.
    #[cfg(test)]
    pub(crate) const fn verb(&self) -> OperationClass {
        self.request.verb()
    }

    /// Consume the post-commit context into the sealed issuer's request.
    pub(crate) fn into_request(self) -> ZoneLinkRouteAdmissionRequest {
        self.request
    }
}

impl core::fmt::Debug for ZoneLinkRouteAdmissionContext {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneLinkRouteAdmissionContext")
            .field("sequence", &self.sequence)
            .field("binding", &self.binding)
            .field("request", &"<redacted>")
            .finish()
    }
}

/// Versioned durable envelope for committed route-admission operation IDs.
///
/// The identity fields make old forensic rows fail closed when a same-named
/// resource is recreated with a different ZoneLink UID or controller
/// generation.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ZoneLinkRouteAdmissionDedupWire {
    version: u32,
    zone_link_uid: ResourceUid,
    edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    source_zone_uid: ResourceUid,
    target_zone_uid: ResourceUid,
    operation_ids: Vec<OperationId>,
}

/// The durable child-local ZoneLink record.
///
/// This is the only state a restart may rely on. [`ZoneLinkHandler::restore`]
/// derives the session state from it, so a crash in any handshake window
/// resolves to the state the specification prescribes for that window.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneLinkRecord {
    generation: ZoneLinkControllerGeneration,
    route_binding: Option<ZoneLinkRouteBinding>,
    committed_route_operations: BTreeSet<OperationId>,
    enrollment: Option<SealedEnrollment>,
    enrollment_invalidated: bool,
    consumed_psk_issuance: Option<u64>,
    highest_psk_issuance: Option<u64>,
    link_epoch: u64,
    pending_local_intents: u32,
    child_authorized: bool,
    connected: bool,
    reconnect_attempts: u32,
    disabled: bool,
    cursor: ZoneLinkCursor,
    advertised_routes: u32,
}

impl ZoneLinkRecord {
    /// Create the durable record of a link that has never been enrolled.
    pub const fn unenrolled(generation: ZoneLinkControllerGeneration) -> Self {
        Self {
            generation,
            route_binding: None,
            committed_route_operations: BTreeSet::new(),
            enrollment: None,
            enrollment_invalidated: false,
            consumed_psk_issuance: None,
            highest_psk_issuance: None,
            link_epoch: 0,
            pending_local_intents: 0,
            child_authorized: false,
            connected: false,
            reconnect_attempts: 0,
            disabled: false,
            cursor: ZoneLinkCursor {
                last_sent_revision: 0,
                last_acked_revision: 0,
                last_received_revision: 0,
                last_applied_revision: 0,
            },
            advertised_routes: 0,
        }
    }

    /// Mark the link administratively disabled in the durable record.
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Borrow the link identity generation.
    pub const fn generation(&self) -> &ZoneLinkControllerGeneration {
        &self.generation
    }

    /// Bind this durable record to one exact route authority.
    ///
    /// The controller generation is also the ZoneLink identity generation, so
    /// a route binding from another controller generation is refused.
    pub fn with_route_binding(
        mut self,
        binding: ZoneLinkRouteBinding,
    ) -> Result<Self, ZoneLinkError> {
        if self.route_binding.is_some() || binding.controller_generation() != &self.generation {
            return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
        }
        self.route_binding = Some(binding);
        Ok(self)
    }

    /// Borrow the durable route identity and policy, when configured.
    pub const fn route_binding(&self) -> Option<&ZoneLinkRouteBinding> {
        self.route_binding.as_ref()
    }

    /// Borrow every operation ID durably committed for this ZoneLink identity.
    ///
    /// The set is non-lossy for the lifetime of the immutable ZoneLink
    /// identity and controller generation. New admissions fail closed when the
    /// bounded set reaches [`MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS`].
    pub const fn committed_route_operations(&self) -> &BTreeSet<OperationId> {
        &self.committed_route_operations
    }

    /// Encode the committed route-operation set for durable storage.
    ///
    /// The envelope is canonical, versioned, identity-bound, and bounded.
    /// There is no expiry or eviction because an OperationId remains
    /// non-reusable for the active ZoneLink identity and generation.
    pub fn encode_route_admission_dedup(&self) -> Result<Vec<u8>, ZoneLinkError> {
        let binding = self
            .route_binding
            .as_ref()
            .ok_or(ZoneLinkError::RouteAdmissionBindingInvalid)?;
        if self.committed_route_operations.len() > MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS {
            return Err(ZoneLinkError::RouteAdmissionOperationCapacity);
        }
        let wire = ZoneLinkRouteAdmissionDedupWire {
            version: ZONE_LINK_ROUTE_ADMISSION_DEDUP_VERSION,
            zone_link_uid: binding.zone_link_uid().clone(),
            edge: binding.edge().clone(),
            controller_generation: binding.controller_generation().clone(),
            source_zone_uid: binding.source_zone_uid().clone(),
            target_zone_uid: binding.target_zone_uid().clone(),
            operation_ids: self.committed_route_operations.iter().cloned().collect(),
        };
        let bytes =
            serde_json::to_vec(&wire).map_err(|_| ZoneLinkError::RouteAdmissionDedupInvalid)?;
        let canonical = CanonicalJsonValue::parse(&bytes)
            .map_err(|_| ZoneLinkError::RouteAdmissionDedupInvalid)?
            .to_canonical_bytes();
        if canonical.len() > MAX_ROUTE_ADMISSION_DEDUP_BYTES {
            return Err(ZoneLinkError::RouteAdmissionDedupInvalid);
        }
        Ok(canonical)
    }

    /// Restore committed route-operation IDs from a versioned durable envelope.
    ///
    /// Identity mismatch and unknown versions are quarantine conditions. The
    /// receiver is consumed so a failed recovery cannot partially mutate a
    /// live record.
    pub fn with_route_admission_dedup(mut self, encoded: &[u8]) -> Result<Self, ZoneLinkError> {
        if encoded.is_empty() || encoded.len() > MAX_ROUTE_ADMISSION_DEDUP_BYTES {
            return Err(ZoneLinkError::RouteAdmissionDedupInvalid);
        }
        let canonical = CanonicalJsonValue::parse(encoded)
            .map_err(|_| ZoneLinkError::RouteAdmissionDedupInvalid)?
            .to_canonical_bytes();
        if canonical != encoded {
            return Err(ZoneLinkError::RouteAdmissionDedupInvalid);
        }
        let wire: ZoneLinkRouteAdmissionDedupWire = serde_json::from_slice(encoded)
            .map_err(|_| ZoneLinkError::RouteAdmissionDedupInvalid)?;
        if wire.version != ZONE_LINK_ROUTE_ADMISSION_DEDUP_VERSION {
            return Err(ZoneLinkError::RouteAdmissionDedupVersionMismatch);
        }
        if wire.operation_ids.len() > MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS {
            return Err(ZoneLinkError::RouteAdmissionOperationCapacity);
        }
        let binding = self
            .route_binding
            .as_ref()
            .ok_or(ZoneLinkError::RouteAdmissionBindingInvalid)?;
        if &wire.zone_link_uid != binding.zone_link_uid()
            || &wire.edge != binding.edge()
            || &wire.controller_generation != binding.controller_generation()
            || &wire.source_zone_uid != binding.source_zone_uid()
            || &wire.target_zone_uid != binding.target_zone_uid()
        {
            return Err(ZoneLinkError::RouteAdmissionDedupIdentityMismatch);
        }
        let mut operations = BTreeSet::new();
        let mut previous = None;
        for operation_id in wire.operation_ids {
            if previous
                .as_ref()
                .is_some_and(|previous| previous >= &operation_id)
                || !operations.insert(operation_id.clone())
            {
                return Err(ZoneLinkError::RouteAdmissionDedupInvalid);
            }
            previous = Some(operation_id);
        }
        if !self.committed_route_operations.is_empty()
            && self.committed_route_operations != operations
        {
            return Err(ZoneLinkError::RouteAdmissionDedupConflict);
        }
        self.committed_route_operations = operations;
        Ok(self)
    }

    /// Borrow the sealed enrollment record, if one is committed and valid.
    pub const fn enrollment(&self) -> Option<&SealedEnrollment> {
        self.enrollment.as_ref()
    }

    /// Whether a durable revocation marker is present.
    pub const fn enrollment_invalidated(&self) -> bool {
        self.enrollment_invalidated
    }

    /// Return the burned PSK issuance ordinal, if any.
    pub const fn consumed_psk_issuance(&self) -> Option<u64> {
        self.consumed_psk_issuance
    }

    /// Return the current link epoch.
    pub const fn link_epoch(&self) -> u64 {
        self.link_epoch
    }

    /// Return the bounded queued outbound intent count.
    pub const fn pending_local_intents(&self) -> u32 {
        self.pending_local_intents
    }

    /// Whether the parent allocator authorized this child subject.
    pub const fn child_authorized(&self) -> bool {
        self.child_authorized
    }

    /// Whether an enrolled session is currently established.
    pub const fn connected(&self) -> bool {
        self.connected
    }

    /// Whether the link is administratively disabled.
    pub const fn disabled(&self) -> bool {
        self.disabled
    }

    /// Return the durable route cursor.
    pub const fn cursor(&self) -> ZoneLinkCursor {
        self.cursor
    }

    /// Derive the session state this record proves.
    pub const fn derived_state(&self) -> ZoneLinkSessionState {
        if self.enrollment_invalidated {
            return ZoneLinkSessionState::Unenrolled;
        }
        match self.enrollment {
            // A crash after the sealed enrollment transaction commits resolves
            // to `EnrollmentCommitted`; a live session is never derived from a
            // durable record, so reconnect always re-enters at the enrolled KK
            // handshake.
            Some(_) => ZoneLinkSessionState::EnrollmentCommitted,
            // A crash before the transaction commits resolves to `Unenrolled`,
            // whether or not the PSK was already burned. A burned PSK is
            // refused separately, so that window requires a fresh PSK.
            None => ZoneLinkSessionState::Unenrolled,
        }
    }
}

impl core::fmt::Debug for ZoneLinkRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZoneLinkRecord")
            .field("has_generation", &true)
            .field("has_route_binding", &self.route_binding.is_some())
            .field(
                "committed_route_operation_count",
                &self.committed_route_operations.len(),
            )
            .field("has_enrollment", &self.enrollment.is_some())
            .field("enrollment_invalidated", &self.enrollment_invalidated)
            .field("link_epoch", &self.link_epoch)
            .field("pending_local_intents", &self.pending_local_intents)
            .field("child_authorized", &self.child_authorized)
            .field("connected", &self.connected)
            .field("disabled", &self.disabled)
            .finish()
    }
}

/// The D088 `status.resource` projection written by the child-local handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneLinkStatus {
    phase: ZoneLinkPhase,
    session_state: ZoneLinkSessionState,
    child_zone_uid: Option<ResourceUid>,
    connected: bool,
    link_epoch: u64,
    pending_local_intents: u32,
    child_authorized: bool,
    cursor: ZoneLinkCursor,
}

impl ZoneLinkStatus {
    /// Return the universal-base phase.
    pub const fn phase(&self) -> ZoneLinkPhase {
        self.phase
    }

    /// Return the enrollment-and-session state backing the phase.
    pub const fn session_state(&self) -> ZoneLinkSessionState {
        self.session_state
    }

    /// Borrow the acknowledged child Zone uid, if the parent acknowledged one.
    pub const fn child_zone_uid(&self) -> Option<&ResourceUid> {
        self.child_zone_uid.as_ref()
    }

    /// Whether the enrolled session is established.
    pub const fn connected(&self) -> bool {
        self.connected
    }

    /// Return the current link epoch.
    pub const fn link_epoch(&self) -> u64 {
        self.link_epoch
    }

    /// Return the bounded queued outbound intent count.
    pub const fn pending_local_intents(&self) -> u32 {
        self.pending_local_intents
    }

    /// Whether the parent allocator authorized this child subject.
    pub const fn child_authorized(&self) -> bool {
        self.child_authorized
    }

    /// Return the durable route cursor.
    pub const fn cursor(&self) -> ZoneLinkCursor {
        self.cursor
    }
}

/// One observed link event offered to the reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneLinkEvent {
    /// Note a fresh allocator PSK issuance, invalidating any prior one.
    PskIssued {
        /// The newly issued single-use bootstrap PSK handle.
        psk: BootstrapPsk,
    },
    /// Admit the one-time IKpsk2 bootstrap handshake using a single-use PSK.
    BootstrapAdmit {
        /// The PSK to burn.
        psk: BootstrapPsk,
        /// Current wall-clock milliseconds used only for expiry comparison.
        now_ms: u64,
    },
    /// The one-time IKpsk2 bootstrap handshake failed.
    BootstrapHandshakeFailed,
    /// Commit the sealed enrollment record in one durable transaction.
    SealEnrollment {
        /// The sealed child static key-pin and acknowledged child Zone uid.
        enrollment: SealedEnrollment,
    },
    /// Begin the enrolled KK handshake from the sealed enrollment record.
    BeginEnrolledHandshake,
    /// The enrolled KK session was established by the named peer static key.
    EnrolledSessionEstablished {
        /// The peer static key fingerprint presented during the handshake.
        peer_key_fingerprint: ZoneSigningKeyFingerprint,
    },
    /// The enrolled KK session disconnected.
    SessionDisconnected,
    /// The enrolled KK session exceeded `KK_SESSION_MAX_LIFETIME_MS`.
    SessionLifetimeExpired,
    /// Issue or renew the child's route advertisement.
    AdvertiseRoutes {
        /// Number of descendant routes carried by the advertisement.
        route_count: u32,
    },
    /// Resynchronize the child-store route cursor against the parent.
    ResyncCursor {
        /// Highest revision sent to the parent.
        sent: u64,
        /// Highest revision acknowledged by the parent.
        acked: u64,
        /// Highest parent revision received.
        received: u64,
        /// Highest parent revision applied locally.
        applied: u64,
    },
    /// Drain the bounded outbound intent queue.
    ReplayIntents,
    /// Administratively disable or re-enable the link.
    SetDisabled {
        /// Whether the link is disabled.
        disabled: bool,
    },
    /// Durably revoke the sealed enrollment and tear down the active session.
    Revoke,
    /// Admit one operation through the current ready ZoneLink route.
    ///
    /// The request contains only the operation identity and verb. All route,
    /// session, policy, and time claims come from trusted durable state or the
    /// downstream sealed issuer.
    AdmitRoute {
        /// The untrusted operation intent.
        request: ZoneLinkRouteAdmissionRequest,
    },
    /// Publish a newer committed route policy snapshot.
    ///
    /// This event represents an authority/store observation, not caller
    /// claims supplied to route admission.
    RoutePolicyCommitted {
        /// The new required capability.
        required_capability: ZoneRouteCapability,
        /// The operation class allowed by the new policy.
        verb: OperationClass,
        /// The strictly newer committed policy revision.
        policy_revision: ZoneRevision,
    },
    /// Fence the current route on a newly authenticated session generation.
    SessionGenerationAdvanced {
        /// The nonzero authenticated reconnect generation.
        reconnect_generation: ReconnectGeneration,
    },
}

/// One effect a committed pass releases.
///
/// Every variant is a resource-plane or transport action performed only after
/// the durable record mutation has committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkEffect {
    /// Begin the one-time IKpsk2 bootstrap handshake.
    StartBootstrapHandshake,
    /// Begin the enrolled KK handshake from the sealed enrollment record.
    StartEnrolledHandshake,
    /// Issue or renew the signed route advertisement.
    IssueAdvertisement,
    /// Withdraw every advertised route id.
    WithdrawAdvertisements,
    /// Replay the bounded outbound intent queue.
    ReplayQueuedIntents,
    /// Re-issue parent route watches from the durable cursor.
    ResyncRouteCursor,
    /// Tear down the enrolled session and drop its transport keys.
    TearDownSession,
}

/// An open reconcile pass holding a planned, unreleased mutation.
///
/// A pass releases nothing. It is surrendered to [`ZoneLinkHandler::commit`]
/// or [`ZoneLinkHandler::abort`]. It is deliberately neither `Clone` nor
/// `Copy` so a plan cannot be committed twice.
#[derive(Debug)]
pub struct ZoneLinkPass {
    owner_token: u64,
    sequence: u64,
    next_record: ZoneLinkRecord,
    next_state: ZoneLinkSessionState,
    next_phase: ZoneLinkPhase,
    next_cursor_adopted: bool,
    effects: Vec<ZoneLinkEffect>,
    route_admission: Option<ZoneLinkRouteAdmissionContext>,
    observed_failure: Option<ZoneLinkError>,
}

impl ZoneLinkPass {
    /// Return the session state this pass would commit.
    pub const fn planned_state(&self) -> ZoneLinkSessionState {
        self.next_state
    }

    /// Return the phase this pass would commit.
    pub const fn planned_phase(&self) -> ZoneLinkPhase {
        self.next_phase
    }

    /// Borrow the effects this pass would release once committed.
    pub fn planned_effects(&self) -> &[ZoneLinkEffect] {
        &self.effects
    }

    /// Return the fail-closed reason this pass durably records, if any.
    pub const fn observed_failure(&self) -> Option<ZoneLinkError> {
        self.observed_failure
    }
}

/// Durable-commit evidence for exactly one committed pass.
///
/// The type has no public constructor and no `Clone`. It is issued only by
/// [`ZoneLinkHandler::commit`] and consumed by value in
/// [`ZoneLinkHandler::release_effects`], so effects are releasable exactly
/// once and only after the durable record mutation.
#[derive(Debug)]
pub struct ZoneLinkCommitProof {
    owner_token: u64,
    sequence: u64,
    route_admission: Option<ZoneLinkRouteAdmissionContext>,
}

/// The child-local ZoneLink handler.
pub struct ZoneLinkHandler {
    limits: ZoneLinkLimits,
    key_policy: ZoneLinkKeyPolicy,
    record: ZoneLinkRecord,
    state: ZoneLinkSessionState,
    phase: ZoneLinkPhase,
    owner_token: u64,
    sequence: u64,
    pass_open: bool,
    cursor_adopted: bool,
    pending_effects: Option<(u64, u64, Vec<ZoneLinkEffect>)>,
}

impl ZoneLinkHandler {
    /// Restore a handler from its durable record.
    ///
    /// The session state is derived purely from the record, so restart after a
    /// crash in any handshake window resolves to the prescribed state and no
    /// live session is ever assumed.
    pub fn restore(
        limits: ZoneLinkLimits,
        key_policy: ZoneLinkKeyPolicy,
        mut record: ZoneLinkRecord,
    ) -> Self {
        record.connected = false;
        record.advertised_routes = 0;
        let state = record.derived_state();
        let phase = if record.disabled {
            ZoneLinkPhase::Pending
        } else if record.enrollment_invalidated {
            ZoneLinkPhase::Unknown
        } else if state == ZoneLinkSessionState::EnrollmentCommitted {
            ZoneLinkPhase::Degraded
        } else {
            ZoneLinkPhase::Pending
        };
        Self {
            limits,
            key_policy,
            record,
            state,
            phase,
            owner_token: next_zone_link_handler_owner(),
            sequence: 0,
            pass_open: false,
            cursor_adopted: false,
            pending_effects: None,
        }
    }

    /// Return the current session state.
    pub const fn session_state(&self) -> ZoneLinkSessionState {
        self.state
    }

    /// Borrow the durable record.
    pub const fn record(&self) -> &ZoneLinkRecord {
        &self.record
    }

    /// Return the bounded limits this handler enforces.
    pub const fn limits(&self) -> ZoneLinkLimits {
        self.limits
    }

    /// Return the frozen cryptoperiod constants this handler enforces.
    pub const fn key_policy(&self) -> ZoneLinkKeyPolicy {
        self.key_policy
    }

    /// Project the current D088 `status.resource` observation.
    pub fn status(&self) -> ZoneLinkStatus {
        ZoneLinkStatus {
            phase: self.phase,
            session_state: self.state,
            child_zone_uid: self
                .record
                .enrollment
                .as_ref()
                .map(|enrollment| enrollment.child_zone_uid.clone()),
            connected: self.record.connected,
            link_epoch: self.record.link_epoch,
            pending_local_intents: self.record.pending_local_intents,
            child_authorized: self.record.child_authorized,
            cursor: self.record.cursor,
        }
    }

    /// Queue one outbound intent while disconnected.
    ///
    /// Queuing is child-local bookkeeping and releases no effect, so it is not
    /// gated on a commit proof. It fails closed at the bounded ceiling.
    pub fn enqueue_intent(&mut self) -> Result<u32, ZoneLinkError> {
        if self.record.pending_local_intents >= self.limits.max_pending_intents {
            return Err(ZoneLinkError::IntentQueueFull);
        }
        self.record.pending_local_intents += 1;
        Ok(self.record.pending_local_intents)
    }

    /// Plan one event into an uncommitted pass.
    ///
    /// Exactly one pass may be open per link; a second call fails closed with
    /// [`ZoneLinkError::ReconcileInFlight`].
    pub fn begin(&mut self, event: ZoneLinkEvent) -> Result<ZoneLinkPass, ZoneLinkError> {
        if self.pass_open {
            return Err(ZoneLinkError::ReconcileInFlight);
        }
        let plan = self.plan(event)?;
        self.pass_open = true;
        Ok(plan)
    }

    /// Discard an open pass without mutating durable state or releasing an
    /// effect.
    pub fn abort(&mut self, pass: ZoneLinkPass) {
        if pass.owner_token == self.owner_token && pass.sequence == self.sequence + 1 {
            self.pass_open = false;
        }
    }

    /// Apply the planned durable mutation and issue its commit proof.
    pub fn commit(&mut self, pass: ZoneLinkPass) -> Result<ZoneLinkCommitProof, ZoneLinkError> {
        if !self.pass_open
            || pass.owner_token != self.owner_token
            || pass.sequence != self.sequence + 1
        {
            return Err(ZoneLinkError::StaleCommitProof);
        }
        self.sequence = pass.sequence;
        self.pass_open = false;
        self.record = pass.next_record;
        self.state = pass.next_state;
        self.phase = pass.next_phase;
        self.cursor_adopted = pass.next_cursor_adopted;
        self.pending_effects = Some((self.owner_token, pass.sequence, pass.effects));
        Ok(ZoneLinkCommitProof {
            owner_token: self.owner_token,
            sequence: pass.sequence,
            route_admission: pass.route_admission,
        })
    }

    /// Consume one commit proof and release its effects exactly once.
    pub fn release_effects(
        &mut self,
        proof: ZoneLinkCommitProof,
    ) -> Result<Vec<ZoneLinkEffect>, ZoneLinkError> {
        match self.pending_effects.take() {
            Some((owner_token, sequence, effects))
                if owner_token == self.owner_token
                    && owner_token == proof.owner_token
                    && sequence == proof.sequence =>
            {
                Ok(effects)
            }
            Some((owner_token, sequence, effects)) => {
                self.pending_effects = Some((owner_token, sequence, effects));
                Err(ZoneLinkError::StaleCommitProof)
            }
            None => Err(ZoneLinkError::StaleCommitProof),
        }
    }

    /// Mark the durable cursor owner as adopted after restart.
    pub(crate) fn mark_cursor_adopted(&mut self) {
        self.cursor_adopted = true;
    }

    /// Clear cursor ownership when the authenticated session is fenced.
    pub(crate) fn clear_cursor_adoption(&mut self) {
        self.cursor_adopted = false;
    }

    /// Consume a committed route proof and invoke the existing sealed issuer.
    ///
    /// The issuer callback receives only the operation request extracted from
    /// a context created from the committed record. It cannot supply identity,
    /// policy, connectivity, or time claims. The callback's implementation is
    /// expected to delegate directly to the runtime-owned sealed route issuer.
    pub fn issue_route_admission<T>(
        &mut self,
        mut proof: ZoneLinkCommitProof,
        issuer: impl FnOnce(ZoneLinkRouteAdmissionRequest) -> Result<T, ZoneLinkError>,
    ) -> Result<T, ZoneLinkError> {
        self.validate_commit_proof(&proof)?;
        if !self.cursor_adopted {
            return Err(ZoneLinkError::RouteAdmissionCursorUnavailable);
        }
        let route_admission = proof
            .route_admission
            .take()
            .ok_or(ZoneLinkError::RouteAdmissionBindingInvalid)?;
        let effects = self.release_effects(proof)?;
        if !effects.is_empty() {
            return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
        }
        issuer(route_admission.into_request())
    }

    #[cfg(test)]
    pub(crate) fn route_admission_context<'a>(
        &self,
        proof: &'a ZoneLinkCommitProof,
    ) -> Result<&'a ZoneLinkRouteAdmissionContext, ZoneLinkError> {
        self.validate_commit_proof(proof)?;
        proof
            .route_admission
            .as_ref()
            .ok_or(ZoneLinkError::RouteAdmissionBindingInvalid)
    }

    fn validate_commit_proof(&self, proof: &ZoneLinkCommitProof) -> Result<(), ZoneLinkError> {
        if proof.owner_token != self.owner_token
            || self
                .pending_effects
                .as_ref()
                .is_none_or(|(owner_token, sequence, _)| {
                    *owner_token != self.owner_token || *sequence != proof.sequence
                })
        {
            return Err(ZoneLinkError::StaleCommitProof);
        }
        Ok(())
    }

    fn plan(&self, event: ZoneLinkEvent) -> Result<ZoneLinkPass, ZoneLinkError> {
        let sequence = self.sequence + 1;
        let mut record = self.record.clone();
        let mut state = self.state;
        let mut phase = self.phase;
        let mut cursor_adopted = self.cursor_adopted;
        let mut effects = Vec::new();
        let mut route_admission = None;
        let mut observed_failure = None;

        match event {
            ZoneLinkEvent::PskIssued { psk } => {
                if record
                    .highest_psk_issuance
                    .is_some_and(|prior| prior >= psk.issuance)
                {
                    return Err(ZoneLinkError::BootstrapPskInvalidated);
                }
                record.highest_psk_issuance = Some(psk.issuance);
            }
            ZoneLinkEvent::BootstrapAdmit { psk, now_ms } => {
                if record.disabled {
                    return Err(ZoneLinkError::Disabled);
                }
                if state != ZoneLinkSessionState::Unenrolled {
                    // An `EnrollmentCommitted` link never downgrades to
                    // IKpsk2 or an unauthenticated pattern.
                    return Err(ZoneLinkError::InvalidTransition);
                }
                if psk.generation() != &record.generation {
                    return Err(ZoneLinkError::BootstrapPskGenerationMismatch);
                }
                if record.consumed_psk_issuance == Some(psk.issuance) {
                    return Err(ZoneLinkError::BootstrapPskConsumed);
                }
                if record
                    .highest_psk_issuance
                    .is_some_and(|latest| latest > psk.issuance)
                {
                    return Err(ZoneLinkError::BootstrapPskInvalidated);
                }
                if now_ms >= psk.expires_at_ms {
                    return Err(ZoneLinkError::BootstrapPskExpired);
                }
                // The PSK is burned atomically with entering `IKpsk2`, so a
                // crash in this window can never retry the same PSK.
                record.consumed_psk_issuance = Some(psk.issuance);
                record.highest_psk_issuance = Some(
                    record
                        .highest_psk_issuance
                        .map_or(psk.issuance, |latest| latest.max(psk.issuance)),
                );
                record.enrollment_invalidated = false;
                state = ZoneLinkSessionState::IKpsk2;
                phase = ZoneLinkPhase::Pending;
                effects.push(ZoneLinkEffect::StartBootstrapHandshake);
            }
            ZoneLinkEvent::BootstrapHandshakeFailed => {
                if state != ZoneLinkSessionState::IKpsk2 {
                    return Err(ZoneLinkError::InvalidTransition);
                }
                state = ZoneLinkSessionState::Unenrolled;
                phase = ZoneLinkPhase::Pending;
                observed_failure = Some(ZoneLinkError::BootstrapHandshakeFailed);
            }
            ZoneLinkEvent::SealEnrollment { enrollment } => {
                if state == ZoneLinkSessionState::EnrollmentCommitted
                    && record.enrollment.as_ref() == Some(&enrollment)
                {
                    // Restart-safe replay of an already-committed seal.
                    return Ok(ZoneLinkPass {
                        owner_token: self.owner_token,
                        sequence,
                        next_record: record,
                        next_state: state,
                        next_phase: phase,
                        next_cursor_adopted: cursor_adopted,
                        effects,
                        route_admission,
                        observed_failure,
                    });
                }
                if state != ZoneLinkSessionState::IKpsk2 {
                    return Err(ZoneLinkError::InvalidTransition);
                }
                record.enrollment = Some(enrollment);
                record.enrollment_invalidated = false;
                state = ZoneLinkSessionState::EnrollmentCommitted;
                phase = ZoneLinkPhase::Pending;
            }
            ZoneLinkEvent::BeginEnrolledHandshake => {
                if record.disabled {
                    return Err(ZoneLinkError::Disabled);
                }
                if state != ZoneLinkSessionState::EnrollmentCommitted {
                    return Err(ZoneLinkError::InvalidTransition);
                }
                if record.reconnect_attempts >= self.limits.reconnect_max_attempts {
                    return Err(ZoneLinkError::ReconnectBudgetExhausted);
                }
                record.reconnect_attempts += 1;
                state = ZoneLinkSessionState::Kk;
                effects.push(ZoneLinkEffect::StartEnrolledHandshake);
            }
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint,
            } => {
                if state != ZoneLinkSessionState::Kk {
                    return Err(ZoneLinkError::InvalidTransition);
                }
                let Some(enrollment) = record.enrollment.clone() else {
                    return Err(ZoneLinkError::InvalidTransition);
                };
                if enrollment.key_fingerprint != peer_key_fingerprint {
                    // A pre-revocation or foreign static key is refused
                    // before any resource exchange.
                    state = ZoneLinkSessionState::EnrollmentCommitted;
                    phase = if record.reconnect_attempts >= self.limits.reconnect_max_attempts {
                        ZoneLinkPhase::Failed
                    } else {
                        ZoneLinkPhase::Degraded
                    };
                    observed_failure = Some(ZoneLinkError::EnrollmentKeyMismatch);
                } else {
                    record.link_epoch += 1;
                    record.connected = true;
                    record.child_authorized = true;
                    record.reconnect_attempts = 0;
                    record.advertised_routes = 0;
                    state = ZoneLinkSessionState::Ready;
                    phase = ZoneLinkPhase::Ready;
                }
            }
            ZoneLinkEvent::SessionDisconnected => {
                if state != ZoneLinkSessionState::Ready && state != ZoneLinkSessionState::Kk {
                    return Err(ZoneLinkError::InvalidTransition);
                }
                record.connected = false;
                record.advertised_routes = 0;
                state = ZoneLinkSessionState::EnrollmentCommitted;
                phase = ZoneLinkPhase::Degraded;
                cursor_adopted = false;
                observed_failure = Some(ZoneLinkError::Disconnected);
            }
            ZoneLinkEvent::SessionLifetimeExpired => {
                if state != ZoneLinkSessionState::Ready {
                    return Err(ZoneLinkError::InvalidTransition);
                }
                // Renewal is always a fresh enrolled KK handshake from the
                // sealed enrollment record; the bootstrap session is never
                // rekeyed and no PSK is consumed.
                record.connected = false;
                record.advertised_routes = 0;
                record.reconnect_attempts = 0;
                state = ZoneLinkSessionState::EnrollmentCommitted;
                phase = ZoneLinkPhase::Degraded;
                cursor_adopted = false;
                effects.push(ZoneLinkEffect::TearDownSession);
            }
            ZoneLinkEvent::AdvertiseRoutes { route_count } => {
                self.require_ready(state, &record)?;
                record.advertised_routes = route_count;
                phase = ZoneLinkPhase::Ready;
                effects.push(ZoneLinkEffect::IssueAdvertisement);
            }
            ZoneLinkEvent::ResyncCursor {
                sent,
                acked,
                received,
                applied,
            } => {
                self.require_ready(state, &record)?;
                record.cursor = record.cursor.advance(sent, acked, received, applied);
                cursor_adopted = false;
                effects.push(ZoneLinkEffect::ResyncRouteCursor);
            }
            ZoneLinkEvent::ReplayIntents => {
                self.require_ready(state, &record)?;
                record.pending_local_intents = 0;
                effects.push(ZoneLinkEffect::ReplayQueuedIntents);
            }
            ZoneLinkEvent::SetDisabled { disabled } => {
                if record.disabled == disabled {
                    return Ok(ZoneLinkPass {
                        owner_token: self.owner_token,
                        sequence,
                        next_record: record,
                        next_state: state,
                        next_phase: phase,
                        next_cursor_adopted: cursor_adopted,
                        effects,
                        route_admission,
                        observed_failure,
                    });
                }
                record.disabled = disabled;
                if disabled {
                    if record.advertised_routes > 0 {
                        effects.push(ZoneLinkEffect::WithdrawAdvertisements);
                    }
                    if record.connected {
                        effects.push(ZoneLinkEffect::TearDownSession);
                    }
                    record.connected = false;
                    record.advertised_routes = 0;
                    record.reconnect_attempts = 0;
                    state = if record.enrollment.is_some() && !record.enrollment_invalidated {
                        ZoneLinkSessionState::EnrollmentCommitted
                    } else {
                        ZoneLinkSessionState::Unenrolled
                    };
                    phase = ZoneLinkPhase::Pending;
                    cursor_adopted = false;
                }
            }
            ZoneLinkEvent::Revoke => {
                // Withdrawal is planned before the durable invalidation
                // marker is released, and both are committed in one
                // transaction, so a crash mid-teardown re-derives
                // `Unenrolled` and never leaves a usable stale enrollment.
                if record.advertised_routes > 0 {
                    effects.push(ZoneLinkEffect::WithdrawAdvertisements);
                }
                effects.push(ZoneLinkEffect::TearDownSession);
                record.enrollment = None;
                record.enrollment_invalidated = true;
                record.connected = false;
                record.child_authorized = false;
                record.advertised_routes = 0;
                record.reconnect_attempts = 0;
                record.pending_local_intents = 0;
                state = ZoneLinkSessionState::Unenrolled;
                phase = ZoneLinkPhase::Unknown;
                cursor_adopted = false;
                observed_failure = Some(ZoneLinkError::Revoked);
            }
            ZoneLinkEvent::AdmitRoute { request } => {
                if record.disabled {
                    return Err(ZoneLinkError::Disabled);
                }
                if record.enrollment_invalidated {
                    return Err(ZoneLinkError::RouteAdmissionRevoked);
                }
                self.require_ready(state, &record)?;
                if !cursor_adopted {
                    return Err(ZoneLinkError::RouteAdmissionCursorUnavailable);
                }
                let Some(binding) = record.route_binding.clone() else {
                    return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
                };
                if binding.verb() != request.verb() {
                    return Err(ZoneLinkError::RouteAdmissionVerbMismatch);
                }
                if record
                    .committed_route_operations
                    .contains(request.operation_id())
                {
                    return Err(ZoneLinkError::RouteAdmissionConflict);
                }
                if record.committed_route_operations.len()
                    >= MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS
                {
                    return Err(ZoneLinkError::RouteAdmissionOperationCapacity);
                }
                record
                    .committed_route_operations
                    .insert(request.operation_id().clone());
                route_admission = Some(ZoneLinkRouteAdmissionContext::new(
                    sequence, binding, request,
                ));
            }
            ZoneLinkEvent::RoutePolicyCommitted {
                required_capability,
                verb,
                policy_revision,
            } => {
                let Some(mut binding) = record.route_binding.clone() else {
                    return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
                };
                if verb == OperationClass::Attach || policy_revision <= binding.policy_revision() {
                    return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
                }
                binding.required_capability = required_capability;
                binding.verb = verb;
                binding.policy_revision = policy_revision;
                record.route_binding = Some(binding);
            }
            ZoneLinkEvent::SessionGenerationAdvanced {
                reconnect_generation,
            } => {
                let Some(mut binding) = record.route_binding.clone() else {
                    return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
                };
                if reconnect_generation <= binding.reconnect_generation() {
                    return Err(ZoneLinkError::RouteAdmissionBindingInvalid);
                }
                let was_connected = record.connected;
                binding.reconnect_generation = reconnect_generation;
                record.route_binding = Some(binding);
                record.connected = false;
                record.advertised_routes = 0;
                record.reconnect_attempts = 0;
                state = if record.enrollment.is_some() && !record.enrollment_invalidated {
                    ZoneLinkSessionState::EnrollmentCommitted
                } else {
                    ZoneLinkSessionState::Unenrolled
                };
                phase = if state == ZoneLinkSessionState::EnrollmentCommitted {
                    ZoneLinkPhase::Degraded
                } else {
                    ZoneLinkPhase::Pending
                };
                cursor_adopted = false;
                if was_connected {
                    effects.push(ZoneLinkEffect::TearDownSession);
                }
            }
        }

        Ok(ZoneLinkPass {
            owner_token: self.owner_token,
            sequence,
            next_record: record,
            next_state: state,
            next_phase: phase,
            next_cursor_adopted: cursor_adopted,
            effects,
            route_admission,
            observed_failure,
        })
    }

    fn require_ready(
        &self,
        state: ZoneLinkSessionState,
        record: &ZoneLinkRecord,
    ) -> Result<(), ZoneLinkError> {
        if record.disabled {
            return Err(ZoneLinkError::Disabled);
        }
        if !state.permits_resource_traffic() {
            return Err(ZoneLinkError::ResourceTrafficBeforeReady);
        }
        if !record.connected || !record.child_authorized {
            return Err(ZoneLinkError::ResourceTrafficBeforeReady);
        }
        Ok(())
    }
}

impl core::fmt::Debug for ZoneLinkHandler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZoneLinkHandler")
            .field("state", &self.state)
            .field("phase", &self.phase)
            .field("sequence", &self.sequence)
            .field("pass_open", &self.pass_open)
            .field("has_pending_effects", &self.pending_effects.is_some())
            .finish()
    }
}

/// One aggregate metric sample.
///
/// Every field is a closed enum, so no ZoneLink, Zone, or resource identity
/// can reach a label value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneLinkMetricSample {
    phase: ZoneLinkPhase,
    reason: Option<ZoneLinkError>,
    succeeded: bool,
}

impl ZoneLinkMetricSample {
    /// Build one sample from closed semantic inputs.
    pub const fn new(phase: ZoneLinkPhase, reason: Option<ZoneLinkError>, succeeded: bool) -> Self {
        Self {
            phase,
            reason,
            succeeded,
        }
    }

    /// Return the closed label values in [`ZONE_LINK_METRIC_LABEL_KEYS`] order.
    pub const fn label_values(&self) -> [&'static str; 3] {
        [
            self.phase.label(),
            match self.reason {
                Some(reason) => reason.label(),
                None => "none",
            },
            if self.succeeded { "success" } else { "failure" },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_zone_session::v3::zone_routing::{ZoneLabelId, ZonePath};

    fn generation() -> ZoneLinkControllerGeneration {
        ZoneLinkControllerGeneration::parse("link-generation-1").unwrap()
    }

    fn fingerprint(value: &str) -> ZoneSigningKeyFingerprint {
        ZoneSigningKeyFingerprint::parse(value).unwrap()
    }

    fn child_uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap()
    }

    fn enrollment() -> SealedEnrollment {
        SealedEnrollment::new(child_uid(), fingerprint("fp-child-static-1"))
    }

    /// Assert one event is refused outright and return its typed reason.
    fn refused(handler: &mut ZoneLinkHandler, event: ZoneLinkEvent) -> ZoneLinkError {
        handler
            .begin(event)
            .expect_err("event is refused before a pass opens")
    }

    fn handler() -> ZoneLinkHandler {
        ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            ZoneLinkRecord::unenrolled(generation()),
        )
    }

    fn psk_for(controller_generation: ZoneLinkControllerGeneration, issuance: u64) -> BootstrapPsk {
        BootstrapPsk::issue(controller_generation, issuance, 300_000)
    }

    fn psk(issuance: u64) -> BootstrapPsk {
        psk_for(generation(), issuance)
    }

    /// Plan, commit, and release one event, asserting each stage succeeded.
    fn apply(handler: &mut ZoneLinkHandler, event: ZoneLinkEvent) -> Vec<ZoneLinkEffect> {
        let pass = handler.begin(event).expect("event is admissible");
        let proof = handler.commit(pass).expect("open pass commits");
        handler.release_effects(proof).expect("proof is fresh")
    }

    fn drive_to_ready_with_generation(
        handler: &mut ZoneLinkHandler,
        controller_generation: ZoneLinkControllerGeneration,
    ) {
        apply(
            handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk_for(controller_generation, 1),
                now_ms: 0,
            },
        );
        apply(
            handler,
            ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            },
        );
        apply(handler, ZoneLinkEvent::BeginEnrolledHandshake);
        apply(
            handler,
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: fingerprint("fp-child-static-1"),
            },
        );
    }

    fn drive_to_ready(handler: &mut ZoneLinkHandler) {
        drive_to_ready_with_generation(handler, generation());
    }

    fn route_binding_with_generation(
        controller_generation: ZoneLinkControllerGeneration,
    ) -> ZoneLinkRouteBinding {
        route_binding_with_identity(
            controller_generation,
            "123e4567-e89b-42d3-a456-426614174010",
        )
    }

    fn route_binding_with_identity(
        controller_generation: ZoneLinkControllerGeneration,
        zone_link_uid: &str,
    ) -> ZoneLinkRouteBinding {
        ZoneLinkRouteBinding::new(
            ResourceUid::parse(zone_link_uid).unwrap(),
            ZoneTreeEdge::new(
                ZonePath::new(vec![ZoneLabelId::parse("parent").unwrap()]).unwrap(),
                ZonePath::new(vec![
                    ZoneLabelId::parse("child").unwrap(),
                    ZoneLabelId::parse("parent").unwrap(),
                ])
                .unwrap(),
            )
            .unwrap(),
            controller_generation,
            ReconnectGeneration::new(7).unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174011").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174012").unwrap(),
            ZoneRevision::new(9),
            ZoneRouteCapability::parse("resource-read").unwrap(),
            OperationClass::Invoke,
        )
        .unwrap()
    }

    fn route_binding() -> ZoneLinkRouteBinding {
        route_binding_with_generation(generation())
    }

    fn route_handler() -> ZoneLinkHandler {
        let record = ZoneLinkRecord::unenrolled(generation())
            .with_route_binding(route_binding())
            .unwrap();
        ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            record,
        )
    }

    #[test]
    fn durable_route_binding_and_bootstrap_psk_generation_are_exact() {
        let other_generation = ZoneLinkControllerGeneration::parse("link-generation-2").unwrap();
        assert_eq!(
            ZoneLinkRecord::unenrolled(generation())
                .with_route_binding(route_binding_with_generation(other_generation.clone())),
            Err(ZoneLinkError::RouteAdmissionBindingInvalid)
        );

        let mut handler = handler();
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: BootstrapPsk::issue(other_generation, 1, 300_000),
                    now_ms: 0,
                },
            ),
            ZoneLinkError::BootstrapPskGenerationMismatch
        );
        let rendered = format!("{:?}", route_binding());
        assert!(!rendered.contains("123e4567"));
    }

    fn route_request(marker: u8) -> ZoneLinkRouteAdmissionRequest {
        ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(vec![marker; 16]).unwrap(),
            OperationClass::Invoke,
        )
        .unwrap()
    }

    fn route_request_id(value: u128) -> ZoneLinkRouteAdmissionRequest {
        ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(value.to_be_bytes().to_vec()).unwrap(),
            OperationClass::Invoke,
        )
        .unwrap()
    }

    fn issue_route(
        handler: &mut ZoneLinkHandler,
        request: ZoneLinkRouteAdmissionRequest,
    ) -> Result<(), ZoneLinkError> {
        let pass = handler.begin(ZoneLinkEvent::AdmitRoute { request })?;
        let proof = handler.commit(pass)?;
        handler.issue_route_admission(proof, |_| Ok(()))
    }

    #[test]
    fn committed_route_operation_ids_are_non_lossy_across_intervening_commits() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();

        issue_route(&mut handler, route_request(10)).unwrap();
        issue_route(&mut handler, route_request(11)).unwrap();
        assert_eq!(
            issue_route(&mut handler, route_request(10)),
            Err(ZoneLinkError::RouteAdmissionConflict)
        );
        assert_eq!(handler.record().committed_route_operations().len(), 2);
    }

    #[test]
    fn committed_route_operation_history_refuses_without_eviction_at_capacity() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        for value in 0..MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS as u128 {
            issue_route(&mut handler, route_request_id(value)).unwrap();
        }
        assert_eq!(
            handler.record().committed_route_operations().len(),
            MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS
        );
        assert_eq!(
            issue_route(&mut handler, route_request_id(0)),
            Err(ZoneLinkError::RouteAdmissionConflict)
        );
        assert_eq!(
            issue_route(
                &mut handler,
                route_request_id(MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS as u128),
            ),
            Err(ZoneLinkError::RouteAdmissionOperationCapacity)
        );
        assert_eq!(
            handler.record().committed_route_operations().len(),
            MAX_COMMITTED_ROUTE_ADMISSION_OPERATIONS
        );
    }

    #[test]
    fn multiple_committed_route_ids_survive_versioned_restart_recovery() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        issue_route(&mut handler, route_request(12)).unwrap();
        issue_route(&mut handler, route_request(13)).unwrap();

        let encoded = handler
            .record()
            .encode_route_admission_dedup()
            .expect("dedup state encodes");
        let empty_encoded = ZoneLinkRecord::unenrolled(generation())
            .with_route_binding(route_binding())
            .unwrap()
            .encode_route_admission_dedup()
            .expect("empty dedup state encodes");
        let restored_record = ZoneLinkRecord::unenrolled(generation())
            .with_route_binding(route_binding())
            .unwrap()
            .with_route_admission_dedup(&encoded)
            .expect("dedup state recovers");
        assert_eq!(
            restored_record
                .clone()
                .with_route_admission_dedup(&empty_encoded),
            Err(ZoneLinkError::RouteAdmissionDedupConflict)
        );
        let mut restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            restored_record,
        );
        drive_to_ready(&mut restarted);
        restarted.mark_cursor_adopted();
        assert_eq!(
            issue_route(&mut restarted, route_request(12)),
            Err(ZoneLinkError::RouteAdmissionConflict)
        );
        assert_eq!(
            issue_route(&mut restarted, route_request(13)),
            Err(ZoneLinkError::RouteAdmissionConflict)
        );

        let mut unsupported = encoded;
        let version = b"\"version\":1";
        let replacement = b"\"version\":2";
        let offset = unsupported
            .windows(version.len())
            .position(|window| window == version)
            .expect("version field is encoded");
        unsupported[offset..offset + version.len()].copy_from_slice(replacement);
        assert_eq!(
            ZoneLinkRecord::unenrolled(generation())
                .with_route_binding(route_binding())
                .unwrap()
                .with_route_admission_dedup(&unsupported),
            Err(ZoneLinkError::RouteAdmissionDedupVersionMismatch)
        );
    }

    #[test]
    fn aborted_route_ids_are_reusable_but_recreated_identity_is_isolated() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        let aborted_request = route_request(14);
        let aborted = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: aborted_request.clone(),
            })
            .unwrap();
        handler.abort(aborted);
        assert!(issue_route(&mut handler, aborted_request).is_ok());

        let old_encoded = handler
            .record()
            .encode_route_admission_dedup()
            .expect("old dedup state encodes");
        let new_generation = ZoneLinkControllerGeneration::parse("link-generation-2").unwrap();
        let new_binding = route_binding_with_identity(
            new_generation.clone(),
            "123e4567-e89b-42d3-a456-426614174020",
        );
        let new_record = ZoneLinkRecord::unenrolled(new_generation.clone())
            .with_route_binding(new_binding)
            .unwrap();
        assert_eq!(
            new_record
                .with_route_admission_dedup(&old_encoded)
                .expect_err("stale forensic state must not cross identities"),
            ZoneLinkError::RouteAdmissionDedupIdentityMismatch
        );

        let mut recreated = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            ZoneLinkRecord::unenrolled(new_generation.clone())
                .with_route_binding(route_binding_with_identity(
                    new_generation.clone(),
                    "123e4567-e89b-42d3-a456-426614174020",
                ))
                .unwrap(),
        );
        drive_to_ready_with_generation(&mut recreated, new_generation);
        recreated.mark_cursor_adopted();
        assert!(issue_route(&mut recreated, route_request(14)).is_ok());
    }

    #[test]
    fn route_admission_requires_ready_and_adopted_cursor() {
        let mut handler = route_handler();
        let request = route_request(1);
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdmitRoute {
                    request: request.clone(),
                }
            ),
            ZoneLinkError::ResourceTrafficBeforeReady
        );

        drive_to_ready(&mut handler);
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdmitRoute {
                    request: request.clone(),
                }
            ),
            ZoneLinkError::RouteAdmissionCursorUnavailable
        );

        handler.mark_cursor_adopted();
        let pass = handler
            .begin(ZoneLinkEvent::AdmitRoute { request })
            .expect("ready route opens");
        assert!(pass.planned_effects().is_empty());
        handler.abort(pass);
        assert!(handler.record().committed_route_operations().is_empty());

        let pass = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: route_request(9),
            })
            .unwrap();
        let proof = handler.commit(pass).unwrap();
        handler.clear_cursor_adoption();
        assert_eq!(
            handler.issue_route_admission(proof, |_| Ok(())),
            Err(ZoneLinkError::RouteAdmissionCursorUnavailable)
        );
    }

    #[test]
    fn route_admission_uses_only_committed_identity_and_daemon_issuer() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        let request = route_request(2);
        let pass = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: request.clone(),
            })
            .expect("route request opens");
        let proof = handler.commit(pass).expect("durable route commit");
        assert!(
            handler
                .record()
                .committed_route_operations()
                .contains(request.operation_id())
        );

        {
            let context = handler
                .route_admission_context(&proof)
                .expect("commit proof is current");
            assert_eq!(
                context.zone_link_uid().as_str(),
                "123e4567-e89b-42d3-a456-426614174010"
            );
            assert_eq!(context.edge().parent().to_storage_string(), "parent");
            assert_eq!(context.edge().child().to_storage_string(), "parent/child");
            assert_eq!(context.controller_generation(), &generation());
            assert_eq!(context.reconnect_generation().get(), 7);
            assert_eq!(
                context.source_zone_uid().as_str(),
                "123e4567-e89b-42d3-a456-426614174011"
            );
            assert_eq!(
                context.target_zone_uid().as_str(),
                "123e4567-e89b-42d3-a456-426614174012"
            );
            assert_eq!(context.policy_revision(), ZoneRevision::new(9));
            assert_eq!(context.required_capability().as_str(), "resource-read");
            assert_eq!(context.operation_id(), request.operation_id());
            assert_eq!(context.verb(), OperationClass::Invoke);
        }

        let issued = handler
            .issue_route_admission(proof, Ok)
            .expect("sealed issuer receives a post-commit request");
        assert_eq!(issued.operation_id(), request.operation_id());
        assert_eq!(issued.verb(), OperationClass::Invoke);
    }

    #[test]
    fn aborted_or_stale_route_passes_cannot_issue_admission() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        let aborted = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: route_request(3),
            })
            .unwrap();
        handler.abort(aborted);

        let request = route_request(4);
        let pass = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: request.clone(),
            })
            .unwrap();
        let proof = handler.commit(pass).unwrap();
        let next = handler
            .begin(ZoneLinkEvent::RoutePolicyCommitted {
                required_capability: ZoneRouteCapability::parse("resource-read").unwrap(),
                verb: OperationClass::Invoke,
                policy_revision: ZoneRevision::new(10),
            })
            .unwrap();
        let fresh = handler.commit(next).unwrap();
        assert_eq!(
            handler.issue_route_admission(proof, |_| Ok(())),
            Err(ZoneLinkError::StaleCommitProof)
        );
        handler.release_effects(fresh).unwrap();
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::AdmitRoute { request }),
            ZoneLinkError::RouteAdmissionConflict
        );
    }

    #[test]
    fn stale_reconnect_generation_cannot_reuse_a_route_proof() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        let request = route_request(8);
        let pass = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: request.clone(),
            })
            .unwrap();
        let proof = handler.commit(pass).unwrap();
        let generation = ReconnectGeneration::new(8).unwrap();
        let reconnect = handler
            .begin(ZoneLinkEvent::SessionGenerationAdvanced {
                reconnect_generation: generation,
            })
            .unwrap();
        let reconnect_proof = handler.commit(reconnect).unwrap();
        assert_eq!(
            handler.issue_route_admission(proof, |_| Ok(())),
            Err(ZoneLinkError::StaleCommitProof)
        );
        assert_eq!(
            handler
                .record()
                .route_binding()
                .unwrap()
                .reconnect_generation(),
            generation
        );
        assert!(
            handler
                .record()
                .committed_route_operations()
                .contains(request.operation_id())
        );
        assert_eq!(
            handler.release_effects(reconnect_proof).unwrap(),
            vec![ZoneLinkEffect::TearDownSession]
        );
    }

    #[test]
    fn route_admission_rejects_substitution_revocation_and_restart_ambiguity() {
        let mut handler = route_handler();
        drive_to_ready(&mut handler);
        handler.mark_cursor_adopted();
        let wrong_verb = ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(vec![5; 16]).unwrap(),
            OperationClass::Relay,
        )
        .unwrap();
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdmitRoute {
                    request: wrong_verb,
                }
            ),
            ZoneLinkError::RouteAdmissionVerbMismatch
        );

        let request = route_request(6);
        let pass = handler
            .begin(ZoneLinkEvent::AdmitRoute {
                request: request.clone(),
            })
            .unwrap();
        let proof = handler.commit(pass).unwrap();
        let record = handler.record().clone();
        let mut restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            record,
        );
        apply(&mut restarted, ZoneLinkEvent::BeginEnrolledHandshake);
        apply(
            &mut restarted,
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: fingerprint("fp-child-static-1"),
            },
        );
        restarted.mark_cursor_adopted();
        assert_eq!(
            restarted.issue_route_admission(proof, |_| Ok(())),
            Err(ZoneLinkError::StaleCommitProof)
        );
        assert_eq!(
            refused(&mut restarted, ZoneLinkEvent::AdmitRoute { request }),
            ZoneLinkError::RouteAdmissionConflict
        );

        apply(&mut handler, ZoneLinkEvent::Revoke);
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdmitRoute {
                    request: route_request(7),
                }
            ),
            ZoneLinkError::RouteAdmissionRevoked
        );
    }

    #[test]
    fn forward_transitions_reach_ready_in_the_canonical_order() {
        let mut handler = handler();
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Unenrolled);

        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        assert_eq!(handler.session_state(), ZoneLinkSessionState::IKpsk2);

        apply(
            &mut handler,
            ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            },
        );
        assert_eq!(
            handler.session_state(),
            ZoneLinkSessionState::EnrollmentCommitted
        );

        apply(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake);
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Kk);

        apply(
            &mut handler,
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: fingerprint("fp-child-static-1"),
            },
        );
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Ready);
        assert_eq!(handler.status().phase(), ZoneLinkPhase::Ready);
        assert_eq!(handler.status().link_epoch(), 1);
        assert!(handler.status().child_authorized());
    }

    #[test]
    fn a_second_pass_is_refused_while_one_is_open() {
        let mut handler = handler();
        let pass = handler
            .begin(ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            })
            .expect("first pass opens");
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::BootstrapHandshakeFailed),
            ZoneLinkError::ReconcileInFlight
        );
        handler.abort(pass);
        assert!(
            handler
                .begin(ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(1),
                    now_ms: 0,
                })
                .is_ok()
        );
    }

    #[test]
    fn an_aborted_pass_mutates_nothing_and_releases_no_effect() {
        let mut handler = handler();
        let pass = handler
            .begin(ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            })
            .expect("pass opens");
        assert_eq!(
            pass.planned_effects(),
            &[ZoneLinkEffect::StartBootstrapHandshake]
        );
        handler.abort(pass);
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Unenrolled);
        assert_eq!(handler.record().consumed_psk_issuance(), None);
    }

    #[test]
    fn effects_require_a_commit_proof_and_release_exactly_once() {
        let mut handler = handler();
        let pass = handler
            .begin(ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            })
            .expect("pass opens");
        let proof = handler.commit(pass).expect("commit issues a proof");
        assert_eq!(
            handler.record().consumed_psk_issuance(),
            Some(1),
            "the durable record mutates at commit, before any effect"
        );
        let effects = handler.release_effects(proof).expect("fresh proof");
        assert_eq!(effects, vec![ZoneLinkEffect::StartBootstrapHandshake]);
    }

    #[test]
    fn a_stale_proof_releases_nothing() {
        let mut handler = handler();
        let first = handler
            .begin(ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            })
            .expect("pass opens");
        let stale = handler.commit(first).expect("commit issues a proof");
        let second = handler
            .begin(ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            })
            .expect("pass opens");
        let fresh = handler.commit(second).expect("commit issues a proof");
        assert_eq!(
            handler.release_effects(stale),
            Err(ZoneLinkError::StaleCommitProof)
        );
        assert!(handler.release_effects(fresh).is_ok());
    }

    #[test]
    fn resource_traffic_before_ready_is_refused_in_every_pre_ready_state() {
        let mut handler = handler();
        for state in [
            ZoneLinkSessionState::Unenrolled,
            ZoneLinkSessionState::IKpsk2,
            ZoneLinkSessionState::EnrollmentCommitted,
            ZoneLinkSessionState::Kk,
        ] {
            assert!(!state.permits_resource_traffic());
        }
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdvertiseRoutes { route_count: 1 }
            ),
            ZoneLinkError::ResourceTrafficBeforeReady
        );
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::ReplayIntents),
            ZoneLinkError::ResourceTrafficBeforeReady
        );
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::ResyncCursor {
                    sent: 1,
                    acked: 1,
                    received: 1,
                    applied: 1,
                }
            ),
            ZoneLinkError::ResourceTrafficBeforeReady
        );

        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        apply(
            &mut handler,
            ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            },
        );
        apply(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake);
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdvertiseRoutes { route_count: 1 }
            ),
            ZoneLinkError::ResourceTrafficBeforeReady
        );
    }

    #[test]
    fn an_expired_bootstrap_psk_is_refused() {
        let mut handler = handler();
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(1),
                    now_ms: 300_000,
                }
            ),
            ZoneLinkError::BootstrapPskExpired
        );
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Unenrolled);
    }

    #[test]
    fn issuing_a_fresh_psk_invalidates_the_prior_outstanding_psk() {
        let mut handler = handler();
        apply(&mut handler, ZoneLinkEvent::PskIssued { psk: psk(1) });
        apply(&mut handler, ZoneLinkEvent::PskIssued { psk: psk(2) });
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(1),
                    now_ms: 0,
                }
            ),
            ZoneLinkError::BootstrapPskInvalidated
        );
        assert!(
            handler
                .begin(ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(2),
                    now_ms: 0,
                })
                .is_ok()
        );
    }

    #[test]
    fn a_failed_bootstrap_handshake_burns_the_psk_and_returns_unenrolled() {
        let mut handler = handler();
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        let pass = handler
            .begin(ZoneLinkEvent::BootstrapHandshakeFailed)
            .expect("failure is recorded durably");
        assert_eq!(
            pass.observed_failure(),
            Some(ZoneLinkError::BootstrapHandshakeFailed)
        );
        let proof = handler.commit(pass).expect("commit");
        assert!(handler.release_effects(proof).expect("release").is_empty());
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Unenrolled);
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(1),
                    now_ms: 0,
                }
            ),
            ZoneLinkError::BootstrapPskConsumed
        );
    }

    #[test]
    fn a_consumed_psk_crash_fails_closed_and_refuses_psk_reuse() {
        let mut handler = handler();
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        // Crash after the PSK burn commits but before the enrollment seal.
        let mut restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            handler.record().clone(),
        );
        assert_eq!(restarted.session_state(), ZoneLinkSessionState::Unenrolled);
        assert_eq!(
            refused(
                &mut restarted,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(1),
                    now_ms: 0,
                }
            ),
            ZoneLinkError::BootstrapPskConsumed
        );
        assert!(
            restarted
                .begin(ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(2),
                    now_ms: 0,
                })
                .is_ok(),
            "a fresh allocator-issued PSK is required and admitted"
        );
    }

    #[test]
    fn a_persist_crash_before_the_enrollment_commit_stays_unenrolled() {
        let mut handler = handler();
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        let pass = handler
            .begin(ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            })
            .expect("pass opens");
        // The process dies before the sealed-enrollment transaction commits.
        drop(pass);
        let restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            handler.record().clone(),
        );
        assert_eq!(restarted.session_state(), ZoneLinkSessionState::Unenrolled);
        assert!(restarted.record().enrollment().is_none());
    }

    #[test]
    fn a_teardown_crash_rederives_unenrolled_from_the_invalidation_marker() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        apply(
            &mut handler,
            ZoneLinkEvent::AdvertiseRoutes { route_count: 2 },
        );

        let pass = handler.begin(ZoneLinkEvent::Revoke).expect("pass opens");
        assert_eq!(
            pass.planned_effects(),
            &[
                ZoneLinkEffect::WithdrawAdvertisements,
                ZoneLinkEffect::TearDownSession,
            ]
        );
        let proof = handler.commit(pass).expect("commit");
        // The process dies after the invalidation marker commits but before
        // the effects are released.
        let _unreleased = proof;
        let restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            handler.record().clone(),
        );
        assert_eq!(restarted.session_state(), ZoneLinkSessionState::Unenrolled);
        assert!(restarted.record().enrollment_invalidated());
        assert!(restarted.record().enrollment().is_none());
    }

    #[test]
    fn revocation_requires_a_fresh_psk_and_a_new_bootstrap() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        apply(&mut handler, ZoneLinkEvent::Revoke);
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Unenrolled);
        assert!(!handler.record().connected());
        assert!(!handler.record().child_authorized());
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake),
            ZoneLinkError::InvalidTransition
        );
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(1),
                    now_ms: 0,
                }
            ),
            ZoneLinkError::BootstrapPskConsumed
        );
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(2),
                now_ms: 0,
            },
        );
        assert_eq!(handler.session_state(), ZoneLinkSessionState::IKpsk2);
    }

    #[test]
    fn a_pre_revocation_static_key_is_refused_before_any_resource_exchange() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        apply(&mut handler, ZoneLinkEvent::Revoke);
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(2),
                now_ms: 0,
            },
        );
        apply(
            &mut handler,
            ZoneLinkEvent::SealEnrollment {
                enrollment: SealedEnrollment::new(child_uid(), fingerprint("fp-child-static-2")),
            },
        );
        apply(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake);
        let pass = handler
            .begin(ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: fingerprint("fp-child-static-1"),
            })
            .expect("mismatch is recorded durably");
        assert_eq!(
            pass.observed_failure(),
            Some(ZoneLinkError::EnrollmentKeyMismatch)
        );
        assert!(pass.planned_effects().is_empty());
        let proof = handler.commit(pass).expect("commit");
        assert!(handler.release_effects(proof).expect("release").is_empty());
        assert_eq!(
            handler.session_state(),
            ZoneLinkSessionState::EnrollmentCommitted
        );
        assert_eq!(handler.status().phase(), ZoneLinkPhase::Degraded);
        assert!(!handler.record().connected());
    }

    #[test]
    fn an_enrolled_key_mismatch_retries_only_under_the_reconnect_budget() {
        let limits = ZoneLinkLimits::new(256, 32, 2, 300).expect("valid limits");
        let mut handler = ZoneLinkHandler::restore(
            limits,
            ZoneLinkKeyPolicy::default(),
            ZoneLinkRecord::unenrolled(generation()),
        );
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        apply(
            &mut handler,
            ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            },
        );
        for _ in 0..2 {
            apply(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake);
            apply(
                &mut handler,
                ZoneLinkEvent::EnrolledSessionEstablished {
                    peer_key_fingerprint: fingerprint("fp-wrong-static"),
                },
            );
        }
        assert_eq!(handler.status().phase(), ZoneLinkPhase::Failed);
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake),
            ZoneLinkError::ReconnectBudgetExhausted
        );
    }

    #[test]
    fn an_expired_kk_session_rehandshakes_from_the_enrollment_record() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        let first_epoch = handler.record().link_epoch();

        apply(&mut handler, ZoneLinkEvent::SessionLifetimeExpired);
        assert_eq!(
            handler.session_state(),
            ZoneLinkSessionState::EnrollmentCommitted
        );
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::BootstrapAdmit {
                    psk: psk(2),
                    now_ms: 0,
                }
            ),
            ZoneLinkError::InvalidTransition,
            "an EnrollmentCommitted link never downgrades to IKpsk2"
        );
        assert_eq!(handler.record().consumed_psk_issuance(), Some(1));

        apply(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake);
        apply(
            &mut handler,
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: fingerprint("fp-child-static-1"),
            },
        );
        assert_eq!(handler.session_state(), ZoneLinkSessionState::Ready);
        assert_eq!(handler.record().link_epoch(), first_epoch + 1);
    }

    #[test]
    fn reconnect_reenters_at_kk_without_consuming_a_psk() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        apply(&mut handler, ZoneLinkEvent::SessionDisconnected);
        assert_eq!(
            handler.session_state(),
            ZoneLinkSessionState::EnrollmentCommitted
        );
        assert_eq!(handler.status().phase(), ZoneLinkPhase::Degraded);

        let restarted_record = handler.record().clone();
        let mut restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            restarted_record,
        );
        assert_eq!(
            restarted.session_state(),
            ZoneLinkSessionState::EnrollmentCommitted,
            "restart on an enrolled link re-enters at the enrolled handshake"
        );
        apply(&mut restarted, ZoneLinkEvent::BeginEnrolledHandshake);
        apply(
            &mut restarted,
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: fingerprint("fp-child-static-1"),
            },
        );
        assert_eq!(restarted.session_state(), ZoneLinkSessionState::Ready);
        assert_eq!(restarted.record().consumed_psk_issuance(), Some(1));
    }

    #[test]
    fn replaying_a_committed_seal_after_restart_is_idempotent() {
        let mut handler = handler();
        apply(
            &mut handler,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        apply(
            &mut handler,
            ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            },
        );
        let mut restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            handler.record().clone(),
        );
        let before = restarted.record().clone();
        let effects = apply(
            &mut restarted,
            ZoneLinkEvent::SealEnrollment {
                enrollment: enrollment(),
            },
        );
        assert!(effects.is_empty());
        assert_eq!(restarted.record(), &before);
        assert_eq!(
            restarted.session_state(),
            ZoneLinkSessionState::EnrollmentCommitted
        );
    }

    #[test]
    fn the_intent_queue_is_bounded_and_drains_only_when_ready() {
        let limits = ZoneLinkLimits::new(2, 32, 10, 300).expect("valid limits");
        let mut handler = ZoneLinkHandler::restore(
            limits,
            ZoneLinkKeyPolicy::default(),
            ZoneLinkRecord::unenrolled(generation()),
        );
        assert_eq!(handler.enqueue_intent(), Ok(1));
        assert_eq!(handler.enqueue_intent(), Ok(2));
        assert_eq!(
            handler.enqueue_intent(),
            Err(ZoneLinkError::IntentQueueFull)
        );
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::ReplayIntents),
            ZoneLinkError::ResourceTrafficBeforeReady
        );

        drive_to_ready(&mut handler);
        assert_eq!(handler.status().pending_local_intents(), 2);
        let effects = apply(&mut handler, ZoneLinkEvent::ReplayIntents);
        assert_eq!(effects, vec![ZoneLinkEffect::ReplayQueuedIntents]);
        assert_eq!(handler.status().pending_local_intents(), 0);
    }

    #[test]
    fn cursor_resync_is_monotonic_and_survives_restart() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        apply(
            &mut handler,
            ZoneLinkEvent::ResyncCursor {
                sent: 10,
                acked: 9,
                received: 8,
                applied: 7,
            },
        );
        apply(
            &mut handler,
            ZoneLinkEvent::ResyncCursor {
                sent: 4,
                acked: 4,
                received: 12,
                applied: 11,
            },
        );
        let cursor = handler.status().cursor();
        assert_eq!(cursor.last_sent_revision(), 10);
        assert_eq!(cursor.last_acked_revision(), 9);
        assert_eq!(cursor.last_received_revision(), 12);
        assert_eq!(cursor.last_applied_revision(), 11);

        let restarted = ZoneLinkHandler::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            handler.record().clone(),
        );
        assert_eq!(restarted.record().cursor(), cursor);
    }

    #[test]
    fn advertisement_renewal_reissues_only_from_ready() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        let issued = apply(
            &mut handler,
            ZoneLinkEvent::AdvertiseRoutes { route_count: 3 },
        );
        assert_eq!(issued, vec![ZoneLinkEffect::IssueAdvertisement]);
        let renewed = apply(
            &mut handler,
            ZoneLinkEvent::AdvertiseRoutes { route_count: 3 },
        );
        assert_eq!(renewed, vec![ZoneLinkEffect::IssueAdvertisement]);

        apply(&mut handler, ZoneLinkEvent::SessionDisconnected);
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdvertiseRoutes { route_count: 3 }
            ),
            ZoneLinkError::ResourceTrafficBeforeReady
        );
    }

    #[test]
    fn disabling_withdraws_advertisements_and_suppresses_reconnect() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        apply(
            &mut handler,
            ZoneLinkEvent::AdvertiseRoutes { route_count: 1 },
        );
        let effects = apply(&mut handler, ZoneLinkEvent::SetDisabled { disabled: true });
        assert_eq!(
            effects,
            vec![
                ZoneLinkEffect::WithdrawAdvertisements,
                ZoneLinkEffect::TearDownSession,
            ]
        );
        assert_eq!(
            refused(&mut handler, ZoneLinkEvent::BeginEnrolledHandshake),
            ZoneLinkError::Disabled
        );
        assert_eq!(
            refused(
                &mut handler,
                ZoneLinkEvent::AdvertiseRoutes { route_count: 1 }
            ),
            ZoneLinkError::Disabled
        );

        apply(&mut handler, ZoneLinkEvent::SetDisabled { disabled: false });
        assert!(handler.begin(ZoneLinkEvent::BeginEnrolledHandshake).is_ok());
    }

    #[test]
    fn limits_and_cryptoperiods_reject_out_of_range_values() {
        assert_eq!(
            ZoneLinkLimits::new(MAX_PENDING_LOCAL_INTENTS + 1, 32, 10, 300),
            Err(ZoneLinkError::InvalidLimits)
        );
        assert_eq!(
            ZoneLinkLimits::new(256, MAX_ACTIVE_STREAMS + 1, 10, 300),
            Err(ZoneLinkError::InvalidLimits)
        );
        assert_eq!(
            ZoneLinkLimits::new(256, 32, 0, 300),
            Err(ZoneLinkError::InvalidLimits)
        );
        assert_eq!(
            ZoneLinkKeyPolicy::new(
                BOOTSTRAP_PSK_TTL_MS_MIN - 1,
                KK_SESSION_MAX_LIFETIME_MS_DEFAULT
            ),
            Err(ZoneLinkError::InvalidLimits)
        );
        assert_eq!(
            ZoneLinkKeyPolicy::new(
                BOOTSTRAP_PSK_TTL_MS_DEFAULT,
                KK_SESSION_MAX_LIFETIME_MS_MAX + 1
            ),
            Err(ZoneLinkError::InvalidLimits)
        );
        let policy = ZoneLinkKeyPolicy::default();
        assert_eq!(policy.bootstrap_psk_ttl_ms(), BOOTSTRAP_PSK_TTL_MS_DEFAULT);
        assert_eq!(
            policy.kk_session_max_lifetime_ms(),
            KK_SESSION_MAX_LIFETIME_MS_DEFAULT
        );
    }

    #[test]
    fn metric_labels_carry_no_identity() {
        let forbidden = ["vm", "zone", "zone_id", "zone_uid", "link_name_hash"];
        for key in ZONE_LINK_METRIC_LABEL_KEYS {
            assert!(!forbidden.contains(key), "forbidden metric label key");
        }
        let canary = "k1-uplink";
        let samples = [
            ZoneLinkMetricSample::new(ZoneLinkPhase::Ready, None, true),
            ZoneLinkMetricSample::new(
                ZoneLinkPhase::Degraded,
                Some(ZoneLinkError::EnrollmentKeyMismatch),
                false,
            ),
            ZoneLinkMetricSample::new(ZoneLinkPhase::Unknown, Some(ZoneLinkError::Revoked), false),
        ];
        for sample in samples {
            let values = sample.label_values();
            assert_eq!(values.len(), ZONE_LINK_METRIC_LABEL_KEYS.len());
            for value in values {
                assert!(!value.contains(canary), "ZoneLink name reached a label");
                assert!(!value.contains("123e4567"), "a uid reached a label");
            }
        }
    }

    #[test]
    fn debug_surfaces_redact_identity_and_key_material() {
        let mut handler = handler();
        drive_to_ready(&mut handler);
        let rendered = format!(
            "{:?} {:?} {:?} {:?}",
            handler,
            handler.record(),
            enrollment(),
            psk(1)
        );
        assert!(!rendered.contains("123e4567"));
        assert!(!rendered.contains("fp-child-static-1"));
        assert!(!rendered.contains("link-generation-1"));
    }

    #[test]
    fn every_error_label_is_a_bounded_lowercase_token() {
        for error in [
            ZoneLinkError::BootstrapPskConsumed,
            ZoneLinkError::BootstrapPskExpired,
            ZoneLinkError::BootstrapPskInvalidated,
            ZoneLinkError::BootstrapHandshakeFailed,
            ZoneLinkError::EnrollmentKeyMismatch,
            ZoneLinkError::Revoked,
            ZoneLinkError::Disconnected,
            ZoneLinkError::ResourceTrafficBeforeReady,
            ZoneLinkError::ReconnectBudgetExhausted,
            ZoneLinkError::IntentQueueFull,
            ZoneLinkError::Disabled,
            ZoneLinkError::InvalidTransition,
            ZoneLinkError::ReconcileInFlight,
            ZoneLinkError::StaleCommitProof,
            ZoneLinkError::InvalidLimits,
            ZoneLinkError::BootstrapPskGenerationMismatch,
            ZoneLinkError::RouteAdmissionBindingInvalid,
            ZoneLinkError::RouteAdmissionCursorUnavailable,
            ZoneLinkError::RouteAdmissionVerbMismatch,
            ZoneLinkError::RouteAdmissionConflict,
            ZoneLinkError::RouteAdmissionRevoked,
            ZoneLinkError::RouteAdmissionOperationCapacity,
            ZoneLinkError::RouteAdmissionDedupInvalid,
            ZoneLinkError::RouteAdmissionDedupVersionMismatch,
            ZoneLinkError::RouteAdmissionDedupIdentityMismatch,
            ZoneLinkError::RouteAdmissionDedupConflict,
        ] {
            let label = error.label();
            assert!(!label.is_empty() && label.len() <= 64);
            assert!(
                label
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '-')
            );
            assert_eq!(error.to_string(), label);
        }
        assert_eq!(
            ZoneLinkError::Disconnected.label(),
            "zone-link-disconnected"
        );
        assert_eq!(
            ZoneLinkError::IntentQueueFull.label(),
            "queue-full-drop-new"
        );
    }
}
