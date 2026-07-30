//! Cross-Zone bus routing.
//!
//! This module owns the originating side of a cross-Zone call: it consumes a
//! route decision that the Zone routing engine already made, pins the reverse
//! path for the lifetime of the operation, names the forwarded call in the
//! full six-tuple `ZoneLinkIdempotencyKey` namespace, seals the forwarded
//! envelope so no descriptor, credential, or host path can cross a ZoneLink,
//! and tracks the child Zone's own watch cursor.
//!
//! It deliberately re-derives none of the routing rules. The nearest-common
//! ancestor walk, the advertised capability ceiling, the hop budget, the
//! per-hop relay rule, and the closed refusal reasons all live in
//! `d2b-zone-routing`, and this module consumes their results through
//! [`ZoneRouteOracle`]. A Zone runtime supplies the adapter that maps the
//! engine's request and decision types onto [`ZoneRouteQuery`] and
//! [`ZoneRouteOutcome`] field for field.
//!
//! Nothing here carries authority. There is no session, admission evidence,
//! verified peer, subject configuration, or store handle, and no socket path,
//! store path, host path, uid, gid, or credential appears in any public type.
//! An authenticated subject is only ever borrowed, and only so its already
//! resolved subject reference can be digested; this module cannot construct
//! one and cannot accept a peer-supplied subject.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    AuthenticatedSubjectContext, MAX_BATCH_MUTATIONS, MAX_FILTER_VALUES, MAX_LIST_FILTERS,
    ReconnectGeneration, ResourceName, ResourceTypeName, ZoneRevision,
    zone_routing::{
        ZONE_ROUTE_INITIAL_HOP_BUDGET, ZonePath, ZoneRouteCapability, ZoneRouteCapabilitySet,
        ZoneRouteFailClosedReason, ZoneRouteHop, ZoneRoutePath,
    },
};
use d2b_resource_api::authz::ApiMethod;
use sha2::{Digest, Sha256};

use crate::operations::OperationId;

/// Seconds a completed cross-Zone dedup record is retained.
///
/// Adapted from the `DEFAULT_RETENTION` value of the superseded operation
/// router: fifteen minutes.
pub const ZONE_LINK_DEDUP_RETENTION_SECONDS: u64 = 900;

/// Seconds a dedup tombstone survives after the retention window closes.
///
/// Adapted from the superseded `DEFAULT_NO_REUSE_HORIZON`: sixty minutes. A
/// key reused inside the tombstone window fails closed rather than being
/// treated as fresh.
pub const ZONE_LINK_DEDUP_NO_REUSE_HORIZON_SECONDS: u64 = 3_600;

/// Maximum dedup records one namespace tracks.
///
/// Adapted from the superseded `DEFAULT_MAX_DEDUP_RECORDS`.
pub const MAX_ZONE_LINK_DEDUP_RECORDS: usize = 65_536;

/// Maximum bytes in one caller-assigned opaque call token.
pub const MAX_OPAQUE_CALL_TOKEN_BYTES: usize = 128;

/// Structural refusals raised while constructing routing values.
///
/// These are construction-time defects in locally assembled values, not
/// routing decisions. Every routing decision is reported with the closed
/// [`ZoneRouteFailClosedReason`] instead. No variant carries a field, so a
/// diagnostic can never echo caller text, a Zone path, or a resource identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneRouteError {
    /// A caller-assigned opaque token was empty, over-long, or not bounded
    /// printable ASCII.
    InvalidCallToken,
    /// A digest was not the `sha256:<64 lowercase hex>` spelling.
    InvalidDigest,
    /// An admitted route carried no hops.
    EmptyRoutePath,
    /// The supplied session generations did not cover exactly one hop each.
    HopGenerationMismatch,
    /// A nameless `List` or `Watch` selector carried an empty authorized name
    /// set.
    EmptyAuthorizedNameSet,
    /// A selector exceeded a frozen admission bound.
    SelectorBoundExceeded,
    /// A watch cursor moved backwards.
    NonMonotonicRevision,
    /// A watch cursor was driven with a Zone it is not bound to.
    ForeignWatchZone,
    /// An atomic batch named more than one target Zone.
    MultiZoneBatch,
    /// An atomic batch named no target Zone.
    EmptyBatch,
}

fn is_bounded_call_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPAQUE_CALL_TOKEN_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn is_sha256_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn render_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut rendered = String::with_capacity(71);
    rendered.push_str("sha256:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        rendered.push(char::from(HEX[usize::from(byte >> 4)]));
        rendered.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    rendered
}

/// A caller-assigned opaque call token.
///
/// This is the shape of the `idempotencyKey`, `correlationId`, and `traceId`
/// values a forwarded call preserves unchanged. It is bounded printable ASCII
/// and is never interpreted, so it confers nothing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpaqueCallToken(String);

impl OpaqueCallToken {
    /// Parse a bounded opaque token.
    pub fn parse(value: impl Into<String>) -> Result<Self, ZoneRouteError> {
        let value = value.into();
        if !is_bounded_call_token(&value) {
            return Err(ZoneRouteError::InvalidCallToken);
        }
        Ok(Self(value))
    }

    /// Borrow the exact token for an authorized wire encoding.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for OpaqueCallToken {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("OpaqueCallToken(<redacted>)")
    }
}

/// A digest of the authenticated subject reference.
///
/// The routing spec names this `principalDigest` and defines it as the SHA-256
/// of the subject reference. It is taken over the canonical `Type/name`
/// rendering the identity contract already produces, so a caller cannot supply
/// one it did not earn.
///
/// It is a correlation key, not a confidentiality primitive, and must not be
/// relied on as one. The digest is unsalted and undomained, and the subject
/// namespace it covers is small and structurally constrained, so a Zone that
/// receives a digest can recover the subject by enumerating candidate
/// `Type/name` strings. What the digest does provide is a stable identifier
/// that is the same for the same subject across hops and that a peer cannot
/// forge. Hiding the identity from a receiving Zone would need a keyed
/// construction, which is a contract change rather than a local one.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrincipalDigest(String);

impl PrincipalDigest {
    /// Digest the subject reference of an already authenticated subject.
    ///
    /// The subject is borrowed, never stored, and never constructed here: the
    /// registrar remains the only owner of subject resolution.
    pub fn of_subject(context: &AuthenticatedSubjectContext) -> Self {
        Self(render_sha256(
            context.subject_ref().to_canonical_string().as_bytes(),
        ))
    }

    /// Adopt an already rendered digest received over a ZoneLink.
    pub fn parse(value: impl Into<String>) -> Result<Self, ZoneRouteError> {
        let value = value.into();
        if !is_sha256_digest(&value) {
            return Err(ZoneRouteError::InvalidDigest);
        }
        Ok(Self(value))
    }

    /// Borrow the rendered digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for PrincipalDigest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PrincipalDigest(<redacted>)")
    }
}

/// A digest of the canonical request bytes.
///
/// Two calls presenting one idempotency key with different fingerprints are a
/// conflict rather than a replay.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestFingerprint(String);

impl RequestFingerprint {
    /// Adopt an already rendered canonical-request digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, ZoneRouteError> {
        let value = value.into();
        if !is_sha256_digest(&value) {
            return Err(ZoneRouteError::InvalidDigest);
        }
        Ok(Self(value))
    }

    /// Borrow the rendered digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for RequestFingerprint {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("RequestFingerprint(<redacted>)")
    }
}

/// The full six-tuple dedup namespace of one forwarded mutating call.
///
/// The namespace is the whole tuple, so one opaque `idempotencyKey` reused
/// under a different source Zone, target Zone, method, or principal cannot
/// collide with an unrelated call.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneLinkIdempotencyKey {
    operation_id: OperationId,
    idempotency_key: OpaqueCallToken,
    source_zone_path: ZonePath,
    target_zone_path: ZonePath,
    operation_kind: ApiMethod,
    principal_digest: PrincipalDigest,
}

impl ZoneLinkIdempotencyKey {
    /// Assemble the six-tuple key.
    pub fn new(
        operation_id: OperationId,
        idempotency_key: OpaqueCallToken,
        source_zone_path: ZonePath,
        target_zone_path: ZonePath,
        operation_kind: ApiMethod,
        principal_digest: PrincipalDigest,
    ) -> Self {
        Self {
            operation_id,
            idempotency_key,
            source_zone_path,
            target_zone_path,
            operation_kind,
            principal_digest,
        }
    }

    /// Borrow the caller-assigned operation identifier.
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Borrow the caller-assigned idempotency token.
    pub const fn idempotency_key(&self) -> &OpaqueCallToken {
        &self.idempotency_key
    }

    /// Borrow the originating Zone path.
    pub const fn source_zone_path(&self) -> &ZonePath {
        &self.source_zone_path
    }

    /// Borrow the target Zone path.
    pub const fn target_zone_path(&self) -> &ZonePath {
        &self.target_zone_path
    }

    /// Return the resource API method kind.
    pub const fn operation_kind(&self) -> ApiMethod {
        self.operation_kind
    }

    /// Borrow the principal digest.
    pub const fn principal_digest(&self) -> &PrincipalDigest {
        &self.principal_digest
    }
}

impl core::fmt::Debug for ZoneLinkIdempotencyKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneLinkIdempotencyKey(<redacted>)")
    }
}

/// The disposition of one key presented to the dedup namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupDisposition {
    /// The key is new; the call proceeds.
    Fresh,
    /// The same key and request are still running.
    InProgress {
        /// The originally recorded operation identifier.
        operation_id: OperationId,
    },
    /// The same key and request already completed inside the retention window.
    Replay {
        /// The originally recorded operation identifier.
        operation_id: OperationId,
    },
    /// The same key was presented with a different request fingerprint.
    Conflict,
    /// The key was reused after the retention window but inside the no-reuse
    /// horizon.
    Expired,
}

#[derive(Clone)]
enum DedupState {
    InProgress,
    Completed { completed_at_unix_seconds: u64 },
}

#[derive(Clone)]
struct DedupRecord {
    operation_id: OperationId,
    fingerprint: RequestFingerprint,
    state: DedupState,
}

/// The bounded dedup namespace for forwarded mutating calls.
///
/// This is the target Zone's namespace. An intermediate relay hop owns no
/// instance of this type and never deduplicates; it forwards.
#[derive(Default)]
pub struct ZoneLinkDedupNamespace {
    records: BTreeMap<ZoneLinkIdempotencyKey, DedupRecord>,
}

impl ZoneLinkDedupNamespace {
    /// An empty namespace.
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
        }
    }

    /// Number of live records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when no record is tracked.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Present one key and request fingerprint.
    ///
    /// A fresh key is recorded as in progress. Overflow is drop-new so an
    /// established call cannot be evicted by a flood of new keys.
    pub fn admit(
        &mut self,
        key: &ZoneLinkIdempotencyKey,
        fingerprint: &RequestFingerprint,
        now_unix_seconds: u64,
    ) -> Result<DedupDisposition, ZoneRouteFailClosedReason> {
        self.prune(now_unix_seconds);
        if let Some(record) = self.records.get(key) {
            if &record.fingerprint != fingerprint {
                return Ok(DedupDisposition::Conflict);
            }
            return Ok(match record.state {
                DedupState::InProgress => DedupDisposition::InProgress {
                    operation_id: record.operation_id.clone(),
                },
                DedupState::Completed {
                    completed_at_unix_seconds,
                } => {
                    if now_unix_seconds.saturating_sub(completed_at_unix_seconds)
                        <= ZONE_LINK_DEDUP_RETENTION_SECONDS
                    {
                        DedupDisposition::Replay {
                            operation_id: record.operation_id.clone(),
                        }
                    } else {
                        DedupDisposition::Expired
                    }
                }
            });
        }
        if self.records.len() >= MAX_ZONE_LINK_DEDUP_RECORDS {
            return Err(ZoneRouteFailClosedReason::QueueFullDropNew);
        }
        self.records.insert(
            key.clone(),
            DedupRecord {
                operation_id: key.operation_id().clone(),
                fingerprint: fingerprint.clone(),
                state: DedupState::InProgress,
            },
        );
        Ok(DedupDisposition::Fresh)
    }

    /// Mark a tracked key completed.
    pub fn complete(&mut self, key: &ZoneLinkIdempotencyKey, now_unix_seconds: u64) {
        if let Some(record) = self.records.get_mut(key) {
            record.state = DedupState::Completed {
                completed_at_unix_seconds: now_unix_seconds,
            };
        }
    }

    /// Drop records past the no-reuse horizon.
    pub fn prune(&mut self, now_unix_seconds: u64) {
        let horizon = ZONE_LINK_DEDUP_RETENTION_SECONDS + ZONE_LINK_DEDUP_NO_REUSE_HORIZON_SECONDS;
        self.records.retain(|_, record| match record.state {
            DedupState::InProgress => true,
            DedupState::Completed {
                completed_at_unix_seconds,
            } => now_unix_seconds.saturating_sub(completed_at_unix_seconds) <= horizon,
        });
    }
}

/// One route question posed to the Zone routing engine.
///
/// Every field mirrors an input the engine's own request type already
/// declares. `policy_allows` is the local authorizer's answer and
/// `zone_link_connected` is the link controller's; neither is inferred here,
/// and both default to their refusing value in [`ZoneRouteQuery::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRouteQuery {
    /// The Zone the call originates in.
    pub source_zone: ZonePath,
    /// The Zone the call targets.
    pub target_zone: ZonePath,
    /// The verifier-supplied decision time in Unix seconds.
    pub current_time_unix_seconds: u64,
    /// The capability the operation needs at the target Zone.
    pub required_capability: Option<ZoneRouteCapability>,
    /// Hops still available to this call.
    pub remaining_hops: u32,
    /// Whether the caller's authorizer allowed the operation.
    pub policy_allows: bool,
    /// Whether the uplink toward the target is established.
    pub zone_link_connected: bool,
}

impl ZoneRouteQuery {
    /// A query with the protocol-wide initial hop budget and refusing
    /// defaults for authorization and connectivity.
    pub const fn new(
        source_zone: ZonePath,
        target_zone: ZonePath,
        current_time_unix_seconds: u64,
    ) -> Self {
        Self {
            source_zone,
            target_zone,
            current_time_unix_seconds,
            required_capability: None,
            remaining_hops: ZONE_ROUTE_INITIAL_HOP_BUDGET,
            policy_allows: false,
            zone_link_connected: false,
        }
    }
}

/// A route the engine admitted.
///
/// The value is exactly the payload of an allowed engine decision. It carries
/// no transport, endpoint, or credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedZoneRoute {
    path: ZoneRoutePath,
    effective_capabilities: Option<ZoneRouteCapabilitySet>,
    remaining_hops_after: u32,
}

impl AdmittedZoneRoute {
    /// Adopt an allowed engine decision field for field.
    pub const fn from_engine_decision(
        path: ZoneRoutePath,
        effective_capabilities: Option<ZoneRouteCapabilitySet>,
        remaining_hops_after: u32,
    ) -> Self {
        Self {
            path,
            effective_capabilities,
            remaining_hops_after,
        }
    }

    /// Borrow the immutable route path.
    pub const fn path(&self) -> &ZoneRoutePath {
        &self.path
    }

    /// Borrow the capability ceiling surviving every advertised hop.
    pub const fn effective_capabilities(&self) -> Option<&ZoneRouteCapabilitySet> {
        self.effective_capabilities.as_ref()
    }

    /// Hops left after paying for this path.
    pub const fn remaining_hops_after(&self) -> u32 {
        self.remaining_hops_after
    }
}

/// The engine's answer, as the bus consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneRouteOutcome {
    /// The route is allowed.
    Admitted(AdmittedZoneRoute),
    /// The route is refused with a closed reason.
    Denied(ZoneRouteFailClosedReason),
}

/// The port through which the bus asks the Zone routing engine for a route.
///
/// The implementation belongs to the Zone runtime and is a direct delegation
/// to `ZoneRouteEngine::decide_route`. The bus adds no rule of its own: it
/// does not walk the Zone tree, intersect capability sets, or compute a hop
/// budget.
pub trait ZoneRouteOracle {
    /// Decide the route for one operation.
    fn decide(&self, query: &ZoneRouteQuery) -> ZoneRouteOutcome;
}

/// The reverse path pinned for the lifetime of one operation.
///
/// Reply, cancel, and status-poll traffic follows these hops in reverse. There
/// is deliberately no method that replaces a hop, so an intermediate Zone
/// cannot reroute in-flight traffic. The value holds Zone tree edges and the
/// session generation bound to each hop, and no socket, endpoint, or
/// credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedReversePath {
    path: ZoneRoutePath,
    hop_generations: Vec<ReconnectGeneration>,
}

impl PinnedReversePath {
    /// Pin an admitted route to the session generation observed at each hop.
    pub fn pin(
        route: &AdmittedZoneRoute,
        hop_generations: Vec<ReconnectGeneration>,
    ) -> Result<Self, ZoneRouteError> {
        let path = route.path().clone();
        if path.hop_count() == 0 {
            return Err(ZoneRouteError::EmptyRoutePath);
        }
        if hop_generations.len() != path.hop_count() {
            return Err(ZoneRouteError::HopGenerationMismatch);
        }
        Ok(Self {
            path,
            hop_generations,
        })
    }

    /// Borrow the pinned forward hops.
    pub fn hops(&self) -> &[ZoneRouteHop] {
        self.path.hops()
    }

    /// The pinned hops in reply order.
    pub fn reverse_hops(&self) -> Vec<&ZoneRouteHop> {
        self.path.hops().iter().rev().collect()
    }

    /// Borrow the pinned per-hop session generations.
    pub fn hop_generations(&self) -> &[ReconnectGeneration] {
        &self.hop_generations
    }

    /// Confirm the pinned path is still live.
    ///
    /// A hop that reconnected carries a new session generation, which
    /// invalidates the path. The operation then fails closed rather than being
    /// silently rerouted.
    pub fn revalidate(
        &self,
        observed_generations: &[ReconnectGeneration],
    ) -> Result<(), ZoneRouteFailClosedReason> {
        if observed_generations.len() != self.hop_generations.len()
            || observed_generations != self.hop_generations.as_slice()
        {
            return Err(ZoneRouteFailClosedReason::ZoneLinkDisconnected);
        }
        Ok(())
    }
}

/// A cancel forwarded down a pinned reverse path.
///
/// Cancellation is best-effort: this value records the delivery intent and
/// carries no deadline, so a failed delivery cannot extend the caller's
/// deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelForward {
    operation_id: OperationId,
    hop_count: usize,
}

impl CancelForward {
    /// Borrow the operation being cancelled.
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Number of hops the cancel traverses.
    pub const fn hop_count(&self) -> usize {
        self.hop_count
    }
}

/// Forward a cancel along a pinned reverse path.
///
/// The path is revalidated first, so a reconnected hop refuses the cancel with
/// the same closed reason the operation itself would fail with.
pub fn forward_cancel(
    path: &PinnedReversePath,
    operation_id: &OperationId,
    observed_generations: &[ReconnectGeneration],
) -> Result<CancelForward, ZoneRouteFailClosedReason> {
    path.revalidate(observed_generations)?;
    Ok(CancelForward {
        operation_id: operation_id.clone(),
        hop_count: path.hops().len(),
    })
}

/// The exact target a forwarded call names.
///
/// A named method retains one exact resource name. A nameless `List` or
/// `Watch` retains a non-empty authorized name set and bounded filters. Every
/// hop preserves this value unchanged; there is no widening constructor and no
/// wildcard variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardedSelector {
    /// One exact resource.
    Named {
        /// The immutable ResourceType.
        resource_type: ResourceTypeName,
        /// The exact resource name.
        resource_name: ResourceName,
    },
    /// A nameless `List` or `Watch` bounded to an authorized name set.
    Nameless {
        /// The immutable ResourceType.
        resource_type: ResourceTypeName,
        /// The exact non-empty authorized name set.
        resource_names: Vec<ResourceName>,
        /// Bounded exact-match filters on indexed fields.
        filters: Vec<ForwardedFilter>,
    },
}

impl ForwardedSelector {
    /// Build a named selector.
    pub const fn named(resource_type: ResourceTypeName, resource_name: ResourceName) -> Self {
        Self::Named {
            resource_type,
            resource_name,
        }
    }

    /// Build a nameless selector, enforcing the frozen admission bounds.
    pub fn nameless(
        resource_type: ResourceTypeName,
        resource_names: Vec<ResourceName>,
        filters: Vec<ForwardedFilter>,
    ) -> Result<Self, ZoneRouteError> {
        if resource_names.is_empty() {
            return Err(ZoneRouteError::EmptyAuthorizedNameSet);
        }
        if resource_names.len() > MAX_FILTER_VALUES || filters.len() > MAX_LIST_FILTERS {
            return Err(ZoneRouteError::SelectorBoundExceeded);
        }
        Ok(Self::Nameless {
            resource_type,
            resource_names,
            filters,
        })
    }

    /// Borrow the immutable ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        match self {
            Self::Named { resource_type, .. } | Self::Nameless { resource_type, .. } => {
                resource_type
            }
        }
    }
}

/// One bounded exact-match filter preserved across every hop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedFilter {
    field: String,
    values: Vec<String>,
}

impl ForwardedFilter {
    /// Build a filter, enforcing the frozen value bound.
    pub fn new(field: impl Into<String>, values: Vec<String>) -> Result<Self, ZoneRouteError> {
        let field = field.into();
        if field.is_empty() || values.is_empty() {
            return Err(ZoneRouteError::SelectorBoundExceeded);
        }
        if values.len() > MAX_FILTER_VALUES {
            return Err(ZoneRouteError::SelectorBoundExceeded);
        }
        Ok(Self { field, values })
    }

    /// Borrow the indexed field name.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Borrow the exact accepted values.
    pub fn values(&self) -> &[String] {
        &self.values
    }
}

/// The sealed envelope of one forwarded cross-Zone call.
///
/// The envelope is the serialization boundary. It has no descriptor,
/// credential, endpoint, or host-path field at all, and [`Self::seal`] refuses
/// a call that offers a descriptor attachment, so neither a structural nor an
/// accidental leak can cross a ZoneLink. Every field except the hop budget is
/// preserved verbatim by [`Self::forwarded`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedEnvelope {
    idempotency: ZoneLinkIdempotencyKey,
    selector: ForwardedSelector,
    correlation_id: OpaqueCallToken,
    trace_id: OpaqueCallToken,
    watch_after_revision: Option<ZoneRevision>,
    remaining_hops: u32,
}

impl ForwardedEnvelope {
    /// Seal a call for forwarding.
    ///
    /// `attachment_count` is the number of descriptor attachments the inbound
    /// call offered. Any nonzero count fails closed.
    pub fn seal(
        idempotency: ZoneLinkIdempotencyKey,
        selector: ForwardedSelector,
        correlation_id: OpaqueCallToken,
        trace_id: OpaqueCallToken,
        watch_after_revision: Option<ZoneRevision>,
        remaining_hops: u32,
        attachment_count: usize,
    ) -> Result<Self, ZoneRouteFailClosedReason> {
        if attachment_count != 0 {
            return Err(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink);
        }
        if remaining_hops == 0 {
            return Err(ZoneRouteFailClosedReason::HopLimitExceeded);
        }
        Ok(Self {
            idempotency,
            selector,
            correlation_id,
            trace_id,
            watch_after_revision,
            remaining_hops,
        })
    }

    /// Borrow the six-tuple idempotency key.
    pub const fn idempotency(&self) -> &ZoneLinkIdempotencyKey {
        &self.idempotency
    }

    /// Borrow the preserved selector.
    pub const fn selector(&self) -> &ForwardedSelector {
        &self.selector
    }

    /// Borrow the preserved correlation identifier.
    pub const fn correlation_id(&self) -> &OpaqueCallToken {
        &self.correlation_id
    }

    /// Borrow the preserved trace identifier.
    pub const fn trace_id(&self) -> &OpaqueCallToken {
        &self.trace_id
    }

    /// Borrow the preserved watch cursor.
    pub const fn watch_after_revision(&self) -> Option<ZoneRevision> {
        self.watch_after_revision
    }

    /// The hop budget carried by this envelope.
    pub const fn remaining_hops(&self) -> u32 {
        self.remaining_hops
    }

    /// Re-serialize the envelope with a new hop budget.
    ///
    /// Only the budget changes. The selector, filters, watch cursor, and the
    /// operation, idempotency, correlation, and trace identifiers are copied
    /// verbatim.
    pub fn forwarded(&self, remaining_hops: u32) -> Self {
        Self {
            idempotency: self.idempotency.clone(),
            selector: self.selector.clone(),
            correlation_id: self.correlation_id.clone(),
            trace_id: self.trace_id.clone(),
            watch_after_revision: self.watch_after_revision,
            remaining_hops,
        }
    }
}

/// Admit an atomic batch, which may name exactly one target Zone.
///
/// A batch spanning several Zones is refused here, before any forwarding; the
/// caller splits it. The frozen per-batch mutation bound is enforced too, so a
/// batch that could never be admitted downstream is not forwarded.
pub fn admit_single_zone_batch(target_zones: &[ZonePath]) -> Result<ZonePath, ZoneRouteError> {
    let Some(first) = target_zones.first() else {
        return Err(ZoneRouteError::EmptyBatch);
    };
    if target_zones.len() > MAX_BATCH_MUTATIONS {
        return Err(ZoneRouteError::SelectorBoundExceeded);
    }
    if target_zones.iter().any(|zone| zone != first) {
        return Err(ZoneRouteError::MultiZoneBatch);
    }
    Ok(first.clone())
}

/// What a parent must do to resume a child watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchResync {
    /// Re-open `Watch` with the identical selector after this revision.
    ResumeWatch {
        /// The last child revision the parent saw, if any.
        after_revision: Option<ZoneRevision>,
    },
    /// Re-issue `List` with the identical selector, then re-open `Watch`
    /// after the snapshot revision.
    RelistThenWatch,
}

/// A parent-side cursor over one child Zone's revision namespace.
///
/// The cursor is bound to the child Zone and holds only that Zone's
/// revisions, so a parent can never merge child revisions into its own
/// namespace: there is no constructor or accessor that mixes the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildWatchCursor {
    child_zone: ZonePath,
    last_seen: Option<ZoneRevision>,
}

impl ChildWatchCursor {
    /// A cursor bound to one child Zone with nothing seen yet.
    pub const fn new(child_zone: ZonePath) -> Self {
        Self {
            child_zone,
            last_seen: None,
        }
    }

    /// Borrow the bound child Zone.
    pub const fn child_zone(&self) -> &ZonePath {
        &self.child_zone
    }

    /// The last child revision observed.
    pub const fn last_seen(&self) -> Option<ZoneRevision> {
        self.last_seen
    }

    /// Record one child revision.
    pub fn observe(
        &mut self,
        child_zone: &ZonePath,
        revision: ZoneRevision,
    ) -> Result<(), ZoneRouteError> {
        if child_zone != &self.child_zone {
            return Err(ZoneRouteError::ForeignWatchZone);
        }
        if let Some(last) = self.last_seen
            && revision < last
        {
            return Err(ZoneRouteError::NonMonotonicRevision);
        }
        self.last_seen = Some(revision);
        Ok(())
    }

    /// The resume plan after a ZoneLink disconnect.
    pub const fn on_disconnect(&self) -> WatchResync {
        WatchResync::ResumeWatch {
            after_revision: self.last_seen,
        }
    }

    /// The resume plan after the child reported a revision-expired cursor.
    ///
    /// The cursor is cleared, because the recorded revision is no longer a
    /// valid resume point.
    pub fn on_revision_expired(&mut self) -> WatchResync {
        self.last_seen = None;
        WatchResync::RelistThenWatch
    }

    /// Adopt the revision of a fresh snapshot obtained by re-listing.
    pub fn adopt_snapshot(
        &mut self,
        child_zone: &ZonePath,
        revision: ZoneRevision,
    ) -> Result<(), ZoneRouteError> {
        if child_zone != &self.child_zone {
            return Err(ZoneRouteError::ForeignWatchZone);
        }
        self.last_seen = Some(revision);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::{
        ZoneLabelId, ZoneRouteHopDirection, ZoneRouteId, ZoneTreeEdge,
    };

    /// Build a Zone path from root-first labels.
    ///
    /// The contract type stores labels most specific first, so the readable
    /// root-first spelling used by these tests is reversed here.
    fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .rev()
                .map(|label| ZoneLabelId::parse(*label).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn token(value: &str) -> OpaqueCallToken {
        OpaqueCallToken::parse(value).unwrap()
    }

    fn digest(seed: char) -> String {
        format!("sha256:{}", String::from(seed).repeat(64))
    }

    fn principal(seed: char) -> PrincipalDigest {
        PrincipalDigest::parse(digest(seed)).unwrap()
    }

    fn fingerprint(seed: char) -> RequestFingerprint {
        RequestFingerprint::parse(digest(seed)).unwrap()
    }

    fn operation(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    fn key(
        operation_value: &str,
        idempotency: &str,
        source: &[&str],
        target: &[&str],
        method: ApiMethod,
        principal_seed: char,
    ) -> ZoneLinkIdempotencyKey {
        ZoneLinkIdempotencyKey::new(
            operation(operation_value),
            token(idempotency),
            zone(source),
            zone(target),
            method,
            principal(principal_seed),
        )
    }

    fn hop(from: &[&str], to: &[&str], route: &str) -> ZoneRouteHop {
        ZoneRouteHop::new(
            zone(from),
            zone(to),
            ZoneTreeEdge::new(zone(from), zone(to)).unwrap(),
            ZoneRouteHopDirection::DownToChild,
            Some(ZoneRouteId::parse(route).unwrap()),
        )
        .unwrap()
    }

    fn k0_to_k2_route() -> AdmittedZoneRoute {
        let path = ZoneRoutePath::new(
            zone(&["k0"]),
            zone(&["k0", "k1", "k2"]),
            zone(&["k0"]),
            vec![
                hop(&["k0"], &["k0", "k1"], "r1"),
                hop(&["k0", "k1"], &["k0", "k1", "k2"], "r2"),
            ],
        )
        .unwrap();
        AdmittedZoneRoute::from_engine_decision(path, None, 14)
    }

    fn generation(value: u64) -> ReconnectGeneration {
        ReconnectGeneration::new(value).unwrap()
    }

    struct ScriptedOracle(ZoneRouteOutcome);

    impl ZoneRouteOracle for ScriptedOracle {
        fn decide(&self, _query: &ZoneRouteQuery) -> ZoneRouteOutcome {
            self.0.clone()
        }
    }

    #[test]
    fn a_route_query_defaults_every_authorization_and_connectivity_input_to_refusing() {
        let query = ZoneRouteQuery::new(zone(&["k0"]), zone(&["k0", "k1"]), 10);
        assert!(!query.policy_allows);
        assert!(!query.zone_link_connected);
        assert_eq!(query.remaining_hops, ZONE_ROUTE_INITIAL_HOP_BUDGET);
        assert!(query.required_capability.is_none());
    }

    #[test]
    fn the_bus_reports_the_engine_decision_it_was_given_without_re_deciding() {
        let admitted = ScriptedOracle(ZoneRouteOutcome::Admitted(k0_to_k2_route()));
        let query = ZoneRouteQuery::new(zone(&["k0"]), zone(&["k0", "k1", "k2"]), 10);
        let ZoneRouteOutcome::Admitted(route) = admitted.decide(&query) else {
            panic!("expected the scripted admission");
        };
        assert_eq!(route.path().hop_count(), 2);
        assert_eq!(route.remaining_hops_after(), 14);

        let denied = ScriptedOracle(ZoneRouteOutcome::Denied(
            ZoneRouteFailClosedReason::HopLimitExceeded,
        ));
        assert_eq!(
            denied.decide(&query),
            ZoneRouteOutcome::Denied(ZoneRouteFailClosedReason::HopLimitExceeded)
        );
    }

    #[test]
    fn an_end_to_end_call_pins_the_reverse_path_and_seals_one_envelope() {
        let route = k0_to_k2_route();
        let pinned = PinnedReversePath::pin(&route, vec![generation(1), generation(2)]).unwrap();
        assert_eq!(pinned.hops().len(), 2);
        let reversed = pinned.reverse_hops();
        assert_eq!(reversed[0].to(), &zone(&["k0", "k1", "k2"]));
        assert_eq!(reversed[1].to(), &zone(&["k0", "k1"]));

        let envelope = ForwardedEnvelope::seal(
            key(
                "op-1",
                "idem-1",
                &["k0"],
                &["k0", "k1", "k2"],
                ApiMethod::UpdateSpec,
                'a',
            ),
            ForwardedSelector::named(
                ResourceTypeName::parse("Process").unwrap(),
                ResourceName::parse("worker").unwrap(),
            ),
            token("corr-1"),
            token("trace-1"),
            None,
            route.remaining_hops_after(),
            0,
        )
        .unwrap();
        assert_eq!(envelope.remaining_hops(), 14);
    }

    #[test]
    fn pinning_requires_one_session_generation_per_hop() {
        let route = k0_to_k2_route();
        assert_eq!(
            PinnedReversePath::pin(&route, vec![generation(1)]),
            Err(ZoneRouteError::HopGenerationMismatch)
        );
    }

    #[test]
    fn a_reconnected_hop_invalidates_the_pinned_path_rather_than_rerouting() {
        let route = k0_to_k2_route();
        let pinned = PinnedReversePath::pin(&route, vec![generation(1), generation(2)]).unwrap();
        assert_eq!(pinned.revalidate(&[generation(1), generation(2)]), Ok(()));
        assert_eq!(
            pinned.revalidate(&[generation(1), generation(3)]),
            Err(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
        assert_eq!(
            pinned.revalidate(&[generation(1)]),
            Err(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
    }

    #[test]
    fn a_cancel_is_delivered_on_the_pinned_reverse_path_and_refused_after_reconnect() {
        let route = k0_to_k2_route();
        let pinned = PinnedReversePath::pin(&route, vec![generation(1), generation(2)]).unwrap();
        let forwarded =
            forward_cancel(&pinned, &operation("op-1"), &[generation(1), generation(2)]).unwrap();
        assert_eq!(forwarded.operation_id(), &operation("op-1"));
        assert_eq!(forwarded.hop_count(), 2);

        assert_eq!(
            forward_cancel(&pinned, &operation("op-1"), &[generation(9), generation(2)]),
            Err(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
        );
    }

    #[test]
    fn an_offered_descriptor_attachment_is_refused_at_the_serialization_boundary() {
        let sealed = ForwardedEnvelope::seal(
            key(
                "op-1",
                "idem-1",
                &["k0"],
                &["k0", "k1"],
                ApiMethod::Create,
                'a',
            ),
            ForwardedSelector::named(
                ResourceTypeName::parse("Process").unwrap(),
                ResourceName::parse("worker").unwrap(),
            ),
            token("corr-1"),
            token("trace-1"),
            None,
            8,
            1,
        );
        assert_eq!(
            sealed,
            Err(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink)
        );
    }

    #[test]
    fn an_exhausted_hop_budget_cannot_be_sealed_into_an_envelope() {
        let sealed = ForwardedEnvelope::seal(
            key(
                "op-1",
                "idem-1",
                &["k0"],
                &["k0", "k1"],
                ApiMethod::Get,
                'a',
            ),
            ForwardedSelector::named(
                ResourceTypeName::parse("Process").unwrap(),
                ResourceName::parse("worker").unwrap(),
            ),
            token("corr-1"),
            token("trace-1"),
            None,
            0,
            0,
        );
        assert_eq!(sealed, Err(ZoneRouteFailClosedReason::HopLimitExceeded));
    }

    #[test]
    fn forwarding_preserves_every_field_except_the_hop_budget() {
        let selector = ForwardedSelector::nameless(
            ResourceTypeName::parse("Process").unwrap(),
            vec![
                ResourceName::parse("worker").unwrap(),
                ResourceName::parse("relay").unwrap(),
            ],
            vec![ForwardedFilter::new("phase", vec!["ready".to_owned()]).unwrap()],
        )
        .unwrap();
        let envelope = ForwardedEnvelope::seal(
            key(
                "op-1",
                "idem-1",
                &["k0"],
                &["k0", "k1", "k2"],
                ApiMethod::Watch,
                'a',
            ),
            selector.clone(),
            token("corr-1"),
            token("trace-1"),
            Some(ZoneRevision::new(42)),
            8,
            0,
        )
        .unwrap();

        let forwarded = envelope.forwarded(7);
        assert_eq!(forwarded.remaining_hops(), 7);
        assert_eq!(forwarded.selector(), &selector);
        assert_eq!(forwarded.correlation_id(), envelope.correlation_id());
        assert_eq!(forwarded.trace_id(), envelope.trace_id());
        assert_eq!(forwarded.idempotency(), envelope.idempotency());
        assert_eq!(
            forwarded.watch_after_revision(),
            Some(ZoneRevision::new(42))
        );
    }

    #[test]
    fn a_nameless_selector_requires_a_non_empty_authorized_name_set() {
        assert_eq!(
            ForwardedSelector::nameless(
                ResourceTypeName::parse("Process").unwrap(),
                Vec::new(),
                Vec::new()
            ),
            Err(ZoneRouteError::EmptyAuthorizedNameSet)
        );
    }

    #[test]
    fn a_selector_over_the_frozen_filter_bound_is_refused() {
        let filters = (0..MAX_LIST_FILTERS + 1)
            .map(|index| {
                ForwardedFilter::new(format!("field{index}"), vec!["value".to_owned()]).unwrap()
            })
            .collect();
        assert_eq!(
            ForwardedSelector::nameless(
                ResourceTypeName::parse("Process").unwrap(),
                vec![ResourceName::parse("worker").unwrap()],
                filters
            ),
            Err(ZoneRouteError::SelectorBoundExceeded)
        );
    }

    #[test]
    fn the_principal_digest_is_a_rendered_sha256_and_never_the_subject_reference() {
        let rendered = PrincipalDigest::parse(digest('a')).unwrap();
        assert!(rendered.as_str().starts_with("sha256:"));
        assert_eq!(rendered.as_str().len(), 71);
        assert_eq!(
            format!("{rendered:?}"),
            "PrincipalDigest(<redacted>)".to_owned()
        );
        assert_eq!(
            PrincipalDigest::parse("Process/worker"),
            Err(ZoneRouteError::InvalidDigest)
        );
    }

    #[test]
    fn one_idempotency_token_under_a_different_tuple_member_never_collides() {
        let mut namespace = ZoneLinkDedupNamespace::new();
        let base = key(
            "op-1",
            "idem-1",
            &["k0"],
            &["k0", "k1"],
            ApiMethod::Create,
            'a',
        );
        assert_eq!(
            namespace.admit(&base, &fingerprint('b'), 0),
            Ok(DedupDisposition::Fresh)
        );

        let other_source = key(
            "op-1",
            "idem-1",
            &["k9"],
            &["k0", "k1"],
            ApiMethod::Create,
            'a',
        );
        let other_target = key(
            "op-1",
            "idem-1",
            &["k0"],
            &["k0", "k9"],
            ApiMethod::Create,
            'a',
        );
        let other_method = key(
            "op-1",
            "idem-1",
            &["k0"],
            &["k0", "k1"],
            ApiMethod::Delete,
            'a',
        );
        let other_principal = key(
            "op-1",
            "idem-1",
            &["k0"],
            &["k0", "k1"],
            ApiMethod::Create,
            'c',
        );
        let other_operation = key(
            "op-9",
            "idem-1",
            &["k0"],
            &["k0", "k1"],
            ApiMethod::Create,
            'a',
        );
        for distinct in [
            other_source,
            other_target,
            other_method,
            other_principal,
            other_operation,
        ] {
            assert_eq!(
                namespace.admit(&distinct, &fingerprint('b'), 0),
                Ok(DedupDisposition::Fresh),
                "a distinct tuple member must open a distinct namespace entry"
            );
        }
        assert_eq!(namespace.len(), 6);
    }

    #[test]
    fn the_dedup_namespace_reports_in_progress_replay_conflict_and_expired() {
        let mut namespace = ZoneLinkDedupNamespace::new();
        let entry = key(
            "op-1",
            "idem-1",
            &["k0"],
            &["k0", "k1"],
            ApiMethod::Create,
            'a',
        );
        assert_eq!(
            namespace.admit(&entry, &fingerprint('b'), 0),
            Ok(DedupDisposition::Fresh)
        );
        assert_eq!(
            namespace.admit(&entry, &fingerprint('b'), 1),
            Ok(DedupDisposition::InProgress {
                operation_id: operation("op-1")
            })
        );
        assert_eq!(
            namespace.admit(&entry, &fingerprint('c'), 1),
            Ok(DedupDisposition::Conflict)
        );
        namespace.complete(&entry, 2);
        assert_eq!(
            namespace.admit(&entry, &fingerprint('b'), 3),
            Ok(DedupDisposition::Replay {
                operation_id: operation("op-1")
            })
        );
        assert_eq!(
            namespace.admit(
                &entry,
                &fingerprint('b'),
                2 + ZONE_LINK_DEDUP_RETENTION_SECONDS + 1
            ),
            Ok(DedupDisposition::Expired)
        );
    }

    #[test]
    fn a_key_reused_past_the_no_reuse_horizon_is_pruned_and_admitted_fresh() {
        let mut namespace = ZoneLinkDedupNamespace::new();
        let entry = key(
            "op-1",
            "idem-1",
            &["k0"],
            &["k0", "k1"],
            ApiMethod::Create,
            'a',
        );
        namespace.admit(&entry, &fingerprint('b'), 0).unwrap();
        namespace.complete(&entry, 0);
        let past = ZONE_LINK_DEDUP_RETENTION_SECONDS + ZONE_LINK_DEDUP_NO_REUSE_HORIZON_SECONDS + 1;
        assert_eq!(
            namespace.admit(&entry, &fingerprint('b'), past),
            Ok(DedupDisposition::Fresh)
        );
    }

    #[test]
    fn a_batch_spanning_two_zones_is_refused_before_any_forwarding() {
        assert_eq!(
            admit_single_zone_batch(&[zone(&["k0", "k1"]), zone(&["k0", "k1"])]),
            Ok(zone(&["k0", "k1"]))
        );
        assert_eq!(
            admit_single_zone_batch(&[zone(&["k0", "k1"]), zone(&["k0", "k2"])]),
            Err(ZoneRouteError::MultiZoneBatch)
        );
        assert_eq!(
            admit_single_zone_batch(&[]),
            Err(ZoneRouteError::EmptyBatch)
        );
    }

    #[test]
    fn a_disconnect_resumes_the_child_watch_after_the_last_seen_child_revision() {
        let child = zone(&["k0", "k1"]);
        let mut cursor = ChildWatchCursor::new(child.clone());
        assert_eq!(
            cursor.on_disconnect(),
            WatchResync::ResumeWatch {
                after_revision: None
            }
        );
        cursor.observe(&child, ZoneRevision::new(7)).unwrap();
        cursor.observe(&child, ZoneRevision::new(9)).unwrap();
        assert_eq!(
            cursor.on_disconnect(),
            WatchResync::ResumeWatch {
                after_revision: Some(ZoneRevision::new(9))
            }
        );
    }

    #[test]
    fn a_revision_expired_report_relists_before_reopening_the_watch() {
        let child = zone(&["k0", "k1"]);
        let mut cursor = ChildWatchCursor::new(child.clone());
        cursor.observe(&child, ZoneRevision::new(9)).unwrap();
        assert_eq!(cursor.on_revision_expired(), WatchResync::RelistThenWatch);
        assert_eq!(cursor.last_seen(), None);
        cursor
            .adopt_snapshot(&child, ZoneRevision::new(30))
            .unwrap();
        assert_eq!(
            cursor.on_disconnect(),
            WatchResync::ResumeWatch {
                after_revision: Some(ZoneRevision::new(30))
            }
        );
    }

    #[test]
    fn a_child_cursor_refuses_a_foreign_zone_revision_and_a_backwards_revision() {
        let child = zone(&["k0", "k1"]);
        let mut cursor = ChildWatchCursor::new(child.clone());
        cursor.observe(&child, ZoneRevision::new(9)).unwrap();
        assert_eq!(
            cursor.observe(&zone(&["k0"]), ZoneRevision::new(10)),
            Err(ZoneRouteError::ForeignWatchZone)
        );
        assert_eq!(
            cursor.adopt_snapshot(&zone(&["k0"]), ZoneRevision::new(10)),
            Err(ZoneRouteError::ForeignWatchZone)
        );
        assert_eq!(
            cursor.observe(&child, ZoneRevision::new(8)),
            Err(ZoneRouteError::NonMonotonicRevision)
        );
    }

    #[test]
    fn opaque_call_tokens_are_bounded_and_redacted() {
        assert_eq!(
            OpaqueCallToken::parse(""),
            Err(ZoneRouteError::InvalidCallToken)
        );
        assert_eq!(
            OpaqueCallToken::parse("a".repeat(MAX_OPAQUE_CALL_TOKEN_BYTES + 1)),
            Err(ZoneRouteError::InvalidCallToken)
        );
        assert_eq!(
            OpaqueCallToken::parse("/run/d2b/public.sock"),
            Err(ZoneRouteError::InvalidCallToken)
        );
        assert_eq!(
            format!("{:?}", token("corr-1")),
            "OpaqueCallToken(<redacted>)".to_owned()
        );
    }
}
