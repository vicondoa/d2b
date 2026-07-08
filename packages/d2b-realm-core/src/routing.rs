//! Tree-only discovery, admission, and route metadata for realm-native routing.
//!
//! This module is schema-only. It describes bounded routing contracts and
//! audit/telemetry metadata, but it deliberately does not implement a live relay,
//! VPN/overlay, STUN/ICE, NAT traversal, or raw tunnel transport.

use crate::capability::{Capability, CapabilitySet};
use crate::enrollment::{KeyFingerprint, RealmKeyRole};
use crate::frame::OperationKind;
use crate::ids::{ControllerGenerationId, CorrelationId, OperationId, RealmId, RouteId};
use crate::realm::{RealmControllerPlacement, RealmPath};
use crate::token::ProtocolToken;
use crate::trace_context::TraceContext;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum routes in one advertisement.
pub const MAX_ADVERTISED_ROUTES: usize = 64;
/// Maximum bytes in a detached signature reference/fingerprint.
pub const MAX_SIGNATURE_REF_LEN: usize = 128;
/// Maximum non-secret routing metadata token length.
pub const MAX_ROUTE_METADATA_REF_LEN: usize = 128;
/// Maximum allowed namespace prefixes in one allocation.
pub const MAX_ROUTE_NAMESPACE_PREFIXES: usize = 16;
/// Maximum hops carried in one tree route path.
pub const MAX_ROUTE_PATH_HOPS: usize = 32;
/// Maximum unauthenticated discovery queue depth.
pub const MAX_DISCOVERY_QUEUE_DEPTH: u32 = 4096;
/// Maximum simultaneously tracked unverified peers per relay class.
pub const MAX_UNVERIFIED_PEERS: u32 = 1024;
/// Maximum pre-auth discovery/admission events per minute.
pub const MAX_PREAUTH_RATE_LIMIT_PER_MINUTE: u32 = 60_000;
/// Maximum replay-window entries represented by metadata.
pub const MAX_REPLAY_WINDOW_ENTRIES: u32 = 1_000_000;
/// Maximum replay-window TTL represented by metadata.
pub const MAX_REPLAY_WINDOW_TTL_SECONDS: u64 = 86_400;
/// Maximum telemetry samples in one bounded batch.
pub const MAX_ROUTE_TELEMETRY_SAMPLES: usize = 64;

fn deserialize_bounded_vec<'de, D, T, const MAX: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let values = Vec::<T>::deserialize(deserializer)?;
    if values.len() > MAX {
        return Err(serde::de::Error::custom(format!(
            "array exceeds maximum length {MAX}"
        )));
    }
    Ok(values)
}

fn valid_metadata_ref(raw: &str) -> bool {
    if raw.is_empty()
        || raw.len() > MAX_ROUTE_METADATA_REF_LEN
        || !raw.as_bytes()[0].is_ascii_alphanumeric()
        || raw.contains("..")
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
    {
        return false;
    }
    let compact = raw
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '_' | '.'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    ![
        "secret",
        "password",
        "passwd",
        "bearer",
        "credential",
        "privatekey",
        "token",
        "endpoint",
        "socketpath",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
}

macro_rules! route_ref_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate and construct a bounded, non-secret metadata reference.
            pub fn parse(raw: impl Into<String>) -> Option<Self> {
                let raw = raw.into();
                valid_metadata_ref(&raw).then_some(Self(raw))
            }

            /// Borrow the reference token.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($name), "(<{} bytes>)"), self.0.len())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?)
                    .ok_or_else(|| serde::de::Error::custom("invalid route metadata reference"))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    string: Some(Box::new(StringValidation {
                        max_length: Some(MAX_ROUTE_METADATA_REF_LEN as u32),
                        min_length: Some(1),
                        pattern: Some("^[A-Za-z0-9][A-Za-z0-9:._-]*$".to_owned()),
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

route_ref_newtype!(
    /// Redacted, non-secret handle for an unverified discovery peer. This is not
    /// a relay endpoint and not an authenticated realm identity.
    UnverifiedPeerRef
);
route_ref_newtype!(
    /// Bounded policy rule id safe for route audit metadata.
    RoutePolicyRuleId
);
route_ref_newtype!(
    /// Redacted replay-window metadata handle.
    RouteReplayWindowId
);
route_ref_newtype!(
    /// Redacted direct-shortcut authorization handle.
    ShortcutAuthorizationId
);

/// Detached signature reference or fingerprint. This is not private key
/// material and is redacted from `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SignatureRef(String);

impl SignatureRef {
    /// Validate a bounded printable signature reference.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > MAX_SIGNATURE_REF_LEN
            || !raw.as_bytes()[0].is_ascii_alphanumeric()
            || !raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
        {
            return None;
        }
        Some(Self(raw))
    }

    /// Borrow the signature reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SignatureRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SignatureRef(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for SignatureRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid signature reference"))
    }
}

impl JsonSchema for SignatureRef {
    fn schema_name() -> String {
        "SignatureRef".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_SIGNATURE_REF_LEN as u32),
                min_length: Some(1),
                pattern: Some("^[A-Za-z0-9][A-Za-z0-9:._-]*$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Discovery queue overflow behavior for unauthenticated input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoveryQueueDropPolicy {
    /// Fail closed by dropping the new unauthenticated item when full.
    DropNew,
}

/// Bounded policy for the pre-auth discovery/admission queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryQueuePolicy {
    /// Maximum queued unauthenticated discovery items.
    #[schemars(range(min = 1, max = 4096))]
    pub max_depth: u32,
    /// Maximum simultaneously tracked unverified peer handles.
    #[schemars(range(min = 1, max = 1024))]
    pub max_unverified_peers: u32,
    /// Per-relay-class pre-auth rate limit.
    #[schemars(range(min = 1, max = 60000))]
    pub per_relay_rate_limit_per_minute: u32,
    /// Per-unverified-peer pre-auth rate limit.
    #[schemars(range(min = 1, max = 60000))]
    pub per_unverified_peer_rate_limit_per_minute: u32,
    /// Required fail-closed overflow behavior.
    pub drop_policy: DiscoveryQueueDropPolicy,
}

impl DiscoveryQueuePolicy {
    /// Construct a policy when all queue/rate bounds are within the contract.
    pub fn new(
        max_depth: u32,
        max_unverified_peers: u32,
        per_relay_rate_limit_per_minute: u32,
        per_unverified_peer_rate_limit_per_minute: u32,
        drop_policy: DiscoveryQueueDropPolicy,
    ) -> Option<Self> {
        if max_depth == 0
            || max_depth > MAX_DISCOVERY_QUEUE_DEPTH
            || max_unverified_peers == 0
            || max_unverified_peers > MAX_UNVERIFIED_PEERS
            || per_relay_rate_limit_per_minute == 0
            || per_relay_rate_limit_per_minute > MAX_PREAUTH_RATE_LIMIT_PER_MINUTE
            || per_unverified_peer_rate_limit_per_minute == 0
            || per_unverified_peer_rate_limit_per_minute > MAX_PREAUTH_RATE_LIMIT_PER_MINUTE
        {
            return None;
        }
        Some(Self {
            max_depth,
            max_unverified_peers,
            per_relay_rate_limit_per_minute,
            per_unverified_peer_rate_limit_per_minute,
            drop_policy,
        })
    }
}

impl<'de> Deserialize<'de> for DiscoveryQueuePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            max_depth: u32,
            max_unverified_peers: u32,
            per_relay_rate_limit_per_minute: u32,
            per_unverified_peer_rate_limit_per_minute: u32,
            drop_policy: DiscoveryQueueDropPolicy,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.max_depth,
            raw.max_unverified_peers,
            raw.per_relay_rate_limit_per_minute,
            raw.per_unverified_peer_rate_limit_per_minute,
            raw.drop_policy,
        )
        .ok_or_else(|| serde::de::Error::custom("discovery queue policy exceeds bounds"))
    }
}

/// Coarse discovery ingress class. Raw relay endpoints are intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoveryIngressClass {
    LocalRoot,
    ParentRelay,
    ChildRelay,
    ProviderRelay,
    StaticConfig,
    Unknown,
}

impl DiscoveryIngressClass {
    /// Stable low-cardinality metric label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalRoot => "local-root",
            Self::ParentRelay => "parent-relay",
            Self::ChildRelay => "child-relay",
            Self::ProviderRelay => "provider-relay",
            Self::StaticConfig => "static-config",
            Self::Unknown => "unknown",
        }
    }
}

/// Metadata for an unauthenticated peer discovery/admission attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnverifiedPeerAdmissionAttemptMetadata {
    /// Per-attempt audit id.
    pub attempt_id: OperationId,
    /// Cross-hop route/admission correlation id.
    pub correlation_id: CorrelationId,
    /// Optional bounded trace context propagated by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    /// Redacted queue-local peer handle, never a metric label.
    pub unverified_peer_ref: UnverifiedPeerRef,
    /// Coarse ingress class, never a raw relay endpoint.
    pub ingress_class: DiscoveryIngressClass,
    /// Claimed realm, if the pre-auth message carried a parseable realm path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_realm: Option<RealmPath>,
    /// Current queue depth after applying the admission decision.
    #[schemars(range(max = 4096))]
    pub queue_depth: u32,
    /// Outcome of queue/rate/replay pre-auth screening.
    pub outcome: PreAuthAdmissionOutcome,
}

impl UnverifiedPeerAdmissionAttemptMetadata {
    /// Construct only when queue metadata remains within the configured bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        attempt_id: OperationId,
        correlation_id: CorrelationId,
        trace: Option<TraceContext>,
        unverified_peer_ref: UnverifiedPeerRef,
        ingress_class: DiscoveryIngressClass,
        claimed_realm: Option<RealmPath>,
        queue_depth: u32,
        outcome: PreAuthAdmissionOutcome,
    ) -> Option<Self> {
        if queue_depth > MAX_DISCOVERY_QUEUE_DEPTH {
            return None;
        }
        Some(Self {
            attempt_id,
            correlation_id,
            trace,
            unverified_peer_ref,
            ingress_class,
            claimed_realm,
            queue_depth,
            outcome,
        })
    }
}

impl<'de> Deserialize<'de> for UnverifiedPeerAdmissionAttemptMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            attempt_id: OperationId,
            correlation_id: CorrelationId,
            #[serde(default)]
            trace: Option<TraceContext>,
            unverified_peer_ref: UnverifiedPeerRef,
            ingress_class: DiscoveryIngressClass,
            #[serde(default)]
            claimed_realm: Option<RealmPath>,
            queue_depth: u32,
            outcome: PreAuthAdmissionOutcome,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.attempt_id,
            raw.correlation_id,
            raw.trace,
            raw.unverified_peer_ref,
            raw.ingress_class,
            raw.claimed_realm,
            raw.queue_depth,
            raw.outcome,
        )
        .ok_or_else(|| serde::de::Error::custom("unverified admission metadata exceeds bounds"))
    }
}

/// Pre-auth admission screening result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PreAuthAdmissionOutcome {
    Queued,
    Dropped { reason: RouteFailClosedReason },
}

/// Replay-window metadata for discovery/session/ad route deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayWindowMetadata {
    pub replay_window_id: RouteReplayWindowId,
    #[schemars(range(min = 1, max = 1000000))]
    pub max_entries: u32,
    #[schemars(range(max = 1000000))]
    pub current_entries: u32,
    #[schemars(range(min = 1, max = 86400))]
    pub ttl_seconds: u64,
    #[schemars(range(max = 1000000))]
    pub observed_replay_count: u32,
    pub opened_at_unix_seconds: u64,
}

impl ReplayWindowMetadata {
    /// Construct when all replay-window counters are bounded.
    pub fn new(
        replay_window_id: RouteReplayWindowId,
        max_entries: u32,
        current_entries: u32,
        ttl_seconds: u64,
        observed_replay_count: u32,
        opened_at_unix_seconds: u64,
    ) -> Option<Self> {
        if max_entries == 0
            || max_entries > MAX_REPLAY_WINDOW_ENTRIES
            || current_entries > max_entries
            || ttl_seconds == 0
            || ttl_seconds > MAX_REPLAY_WINDOW_TTL_SECONDS
            || observed_replay_count > MAX_REPLAY_WINDOW_ENTRIES
        {
            return None;
        }
        Some(Self {
            replay_window_id,
            max_entries,
            current_entries,
            ttl_seconds,
            observed_replay_count,
            opened_at_unix_seconds,
        })
    }
}

impl<'de> Deserialize<'de> for ReplayWindowMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            replay_window_id: RouteReplayWindowId,
            max_entries: u32,
            current_entries: u32,
            ttl_seconds: u64,
            observed_replay_count: u32,
            opened_at_unix_seconds: u64,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.replay_window_id,
            raw.max_entries,
            raw.current_entries,
            raw.ttl_seconds,
            raw.observed_replay_count,
            raw.opened_at_unix_seconds,
        )
        .ok_or_else(|| serde::de::Error::custom("replay window metadata exceeds bounds"))
    }
}

/// Post-auth session admission attempt metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionAdmissionAttemptMetadata {
    pub attempt_id: OperationId,
    pub correlation_id: CorrelationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    pub local_realm: RealmPath,
    pub remote_realm: RealmPath,
    pub operation_kind: OperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_capability: Option<Capability>,
    pub replay_window: ReplayWindowMetadata,
    pub outcome: SessionAdmissionOutcome,
}

/// Post-auth session admission result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SessionAdmissionOutcome {
    Admitted,
    Denied { reason: RouteFailClosedReason },
}

/// Parent/child tree edge metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmTreeEdge {
    /// Parent realm.
    pub parent: RealmPath,
    /// Direct child realm.
    pub child: RealmPath,
}

impl RealmTreeEdge {
    /// Construct only when `child` is exactly one label below `parent`.
    pub fn new(parent: RealmPath, child: RealmPath) -> Option<Self> {
        if child.is_direct_child_of(&parent) {
            Some(Self { parent, child })
        } else {
            None
        }
    }
}

impl<'de> Deserialize<'de> for RealmTreeEdge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            parent: RealmPath,
            child: RealmPath,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.parent, raw.child)
            .ok_or_else(|| serde::de::Error::custom("child realm is not a direct child of parent"))
    }
}

/// One descendant route advertised by a realm controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DescendantRoute {
    /// Route id for this descendant path.
    pub route_id: RouteId,
    /// Descendant reachable below the advertising realm.
    pub descendant: RealmPath,
    /// Next child label below the advertiser.
    pub next_hop_child: RealmId,
    /// Positive capabilities reachable on this route.
    pub capabilities: CapabilitySet,
}

/// Signature metadata for a route advertisement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteSignature {
    /// Signature algorithm token.
    pub algorithm: ProtocolToken,
    /// Controller key role used for signing.
    pub key_role: RealmKeyRole,
    /// Fingerprint of the signing key; no key bytes.
    pub signing_key_fingerprint: KeyFingerprint,
    /// Detached signature reference or bounded signature fingerprint.
    pub signature_ref: SignatureRef,
}

/// Signed, expiring descendant-only route advertisement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteAdvertisement {
    /// Advertising realm.
    pub advertising_realm: RealmPath,
    /// Parent/child edge that authorizes this advertisement.
    pub tree_edge: RealmTreeEdge,
    /// Controller generation that signed the advertisement.
    pub controller_generation: ControllerGenerationId,
    /// Routes below `advertising_realm`.
    #[schemars(length(min = 1, max = 64))]
    pub routes: Vec<DescendantRoute>,
    /// Issue time as Unix seconds.
    pub issued_at_unix_seconds: u64,
    /// Expiry time as Unix seconds. Must be greater than issue time.
    pub expires_at_unix_seconds: u64,
    /// Signature metadata.
    pub signature: RouteSignature,
}

impl RouteAdvertisement {
    /// Validate descendant-only and bounded route invariants.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        advertising_realm: RealmPath,
        tree_edge: RealmTreeEdge,
        controller_generation: ControllerGenerationId,
        routes: Vec<DescendantRoute>,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
        signature: RouteSignature,
    ) -> Option<Self> {
        if tree_edge.child != advertising_realm
            || routes.is_empty()
            || routes.len() > MAX_ADVERTISED_ROUTES
            || expires_at_unix_seconds <= issued_at_unix_seconds
            || routes.iter().any(|route| {
                !route.descendant.is_descendant_of(&advertising_realm)
                    || next_hop_child(&route.descendant, &advertising_realm)
                        != Some(&route.next_hop_child)
            })
        {
            return None;
        }
        Some(Self {
            advertising_realm,
            tree_edge,
            controller_generation,
            routes,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            signature,
        })
    }
}

impl<'de> Deserialize<'de> for RouteAdvertisement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            advertising_realm: RealmPath,
            tree_edge: RealmTreeEdge,
            controller_generation: ControllerGenerationId,
            routes: Vec<DescendantRoute>,
            issued_at_unix_seconds: u64,
            expires_at_unix_seconds: u64,
            signature: RouteSignature,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.advertising_realm,
            raw.tree_edge,
            raw.controller_generation,
            raw.routes,
            raw.issued_at_unix_seconds,
            raw.expires_at_unix_seconds,
            raw.signature,
        )
        .ok_or_else(|| serde::de::Error::custom("invalid route advertisement shape"))
    }
}

/// Envelope around a signed route advertisement as seen by the receiving realm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteAdvertisementEnvelope {
    /// Bounded receipt/admission metadata, not raw relay bytes.
    pub admission: UnverifiedPeerAdmissionAttemptMetadata,
    /// Replay window used to reject duplicate adverts.
    pub replay_window: ReplayWindowMetadata,
    /// Bounded correlation id shared across route-decision audit records.
    pub correlation_id: CorrelationId,
    /// Optional bounded trace context carried alongside the advert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    /// Signed and expiring advertisement payload.
    pub advertisement: RouteAdvertisement,
    /// Receipt time at the verifier.
    pub received_at_unix_seconds: u64,
}

/// Namespace delegated by a parent to a direct child for route advertisements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteNamespaceAllocation {
    pub tree_edge: RealmTreeEdge,
    pub allocated_to_generation: ControllerGenerationId,
    /// Prefixes the child may advertise. Each prefix must be the child realm or
    /// a descendant under it; siblings and parents are rejected.
    #[serde(deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ROUTE_NAMESPACE_PREFIXES>")]
    #[schemars(length(min = 1, max = 16))]
    pub allowed_prefixes: Vec<RealmPath>,
    #[schemars(range(min = 1, max = 64))]
    pub max_routes: u32,
    pub capability_ceiling: CapabilitySet,
}

impl RouteNamespaceAllocation {
    /// Validate direct child ownership and bounded prefixes.
    pub fn new(
        tree_edge: RealmTreeEdge,
        allocated_to_generation: ControllerGenerationId,
        allowed_prefixes: Vec<RealmPath>,
        max_routes: u32,
        capability_ceiling: CapabilitySet,
    ) -> Option<Self> {
        if allowed_prefixes.is_empty()
            || allowed_prefixes.len() > MAX_ROUTE_NAMESPACE_PREFIXES
            || max_routes == 0
            || max_routes > MAX_ADVERTISED_ROUTES as u32
            || allowed_prefixes.iter().any(|prefix| {
                prefix != &tree_edge.child && !prefix.is_descendant_of(&tree_edge.child)
            })
        {
            return None;
        }
        Some(Self {
            tree_edge,
            allocated_to_generation,
            allowed_prefixes,
            max_routes,
            capability_ceiling,
        })
    }
}

impl<'de> Deserialize<'de> for RouteNamespaceAllocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            tree_edge: RealmTreeEdge,
            allocated_to_generation: ControllerGenerationId,
            #[serde(
                deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ROUTE_NAMESPACE_PREFIXES>"
            )]
            allowed_prefixes: Vec<RealmPath>,
            max_routes: u32,
            capability_ceiling: CapabilitySet,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.tree_edge,
            raw.allocated_to_generation,
            raw.allowed_prefixes,
            raw.max_routes,
            raw.capability_ceiling,
        )
        .ok_or_else(|| serde::de::Error::custom("invalid route namespace allocation"))
    }
}

/// Direction for one hop along the realm tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TreeRouteHopDirection {
    UpToParent,
    DownToChild,
}

/// One validated parent/child hop in a tree route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeRouteHop {
    pub from: RealmPath,
    pub to: RealmPath,
    pub edge: RealmTreeEdge,
    pub direction: TreeRouteHopDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_id: Option<RouteId>,
}

impl TreeRouteHop {
    /// Construct only if `from`/`to` match the declared tree edge and direction.
    pub fn new(
        from: RealmPath,
        to: RealmPath,
        edge: RealmTreeEdge,
        direction: TreeRouteHopDirection,
        route_id: Option<RouteId>,
    ) -> Option<Self> {
        let valid = match direction {
            TreeRouteHopDirection::UpToParent => from == edge.child && to == edge.parent,
            TreeRouteHopDirection::DownToChild => from == edge.parent && to == edge.child,
        };
        valid.then_some(Self {
            from,
            to,
            edge,
            direction,
            route_id,
        })
    }
}

impl<'de> Deserialize<'de> for TreeRouteHop {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            from: RealmPath,
            to: RealmPath,
            edge: RealmTreeEdge,
            direction: TreeRouteHopDirection,
            #[serde(default)]
            route_id: Option<RouteId>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.from, raw.to, raw.edge, raw.direction, raw.route_id)
            .ok_or_else(|| serde::de::Error::custom("route hop does not match tree edge"))
    }
}

/// Bounded tree path used for a route decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeRoutePath {
    pub source_realm: RealmPath,
    pub target_realm: RealmPath,
    pub nearest_common_ancestor: RealmPath,
    #[serde(deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ROUTE_PATH_HOPS>")]
    #[schemars(length(max = 32))]
    pub hops: Vec<TreeRouteHop>,
}

impl TreeRoutePath {
    /// Construct a bounded, contiguous tree path.
    pub fn new(
        source_realm: RealmPath,
        target_realm: RealmPath,
        nearest_common_ancestor: RealmPath,
        hops: Vec<TreeRouteHop>,
    ) -> Option<Self> {
        if hops.len() > MAX_ROUTE_PATH_HOPS {
            return None;
        }
        if let Some(first) = hops.first()
            && first.from != source_realm
        {
            return None;
        }
        if let Some(last) = hops.last()
            && last.to != target_realm
        {
            return None;
        }
        if hops.windows(2).any(|pair| pair[0].to != pair[1].from) {
            return None;
        }
        if !source_realm.is_descendant_of(&nearest_common_ancestor)
            && source_realm != nearest_common_ancestor
        {
            return None;
        }
        if !target_realm.is_descendant_of(&nearest_common_ancestor)
            && target_realm != nearest_common_ancestor
        {
            return None;
        }
        Some(Self {
            source_realm,
            target_realm,
            nearest_common_ancestor,
            hops,
        })
    }
}

impl<'de> Deserialize<'de> for TreeRoutePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            source_realm: RealmPath,
            target_realm: RealmPath,
            nearest_common_ancestor: RealmPath,
            #[serde(deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ROUTE_PATH_HOPS>")]
            hops: Vec<TreeRouteHop>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.source_realm,
            raw.target_realm,
            raw.nearest_common_ancestor,
            raw.hops,
        )
        .ok_or_else(|| serde::de::Error::custom("invalid tree route path"))
    }
}

/// Closed fail-closed route/admission reason. These labels are safe for audit
/// and low-cardinality counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RouteFailClosedReason {
    MalformedAdvert,
    UnknownParent,
    NamespaceViolation,
    SiblingOrParentRouteAdvert,
    Loop,
    MultiParent,
    Expired,
    Replay,
    RateLimited,
    QueueFullDropNew,
    MissingCapability,
    PolicyDenial,
}

impl RouteFailClosedReason {
    /// Stable kebab-case label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MalformedAdvert => "malformed-advert",
            Self::UnknownParent => "unknown-parent",
            Self::NamespaceViolation => "namespace-violation",
            Self::SiblingOrParentRouteAdvert => "sibling-or-parent-route-advert",
            Self::Loop => "loop",
            Self::MultiParent => "multi-parent",
            Self::Expired => "expired",
            Self::Replay => "replay",
            Self::RateLimited => "rate-limited",
            Self::QueueFullDropNew => "queue-full-drop-new",
            Self::MissingCapability => "missing-capability",
            Self::PolicyDenial => "policy-denial",
        }
    }
}

/// Result of a tree route decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TreeRouteDecisionOutcome {
    Allowed { path: TreeRoutePath },
    Denied { reason: RouteFailClosedReason },
}

/// Pure metadata route decision. It carries no transport sockets or relay endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeRouteDecision {
    pub decision_id: OperationId,
    pub correlation_id: CorrelationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    pub source_realm: RealmPath,
    pub target_realm: RealmPath,
    pub operation_kind: OperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_capability: Option<Capability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_rule_id: Option<RoutePolicyRuleId>,
    pub outcome: TreeRouteDecisionOutcome,
}

/// Shortcut authorization lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DirectShortcutState {
    Authorized,
    Established,
    TeardownRequested,
    TornDown,
    Denied,
}

/// Direct shortcut authorization metadata. No underlay address or tunnel endpoint
/// is represented; the path remains the authorized tree path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectShortcutAuthorizationMetadata {
    pub shortcut_id: ShortcutAuthorizationId,
    pub correlation_id: CorrelationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    pub authorizing_ancestor: RealmPath,
    pub source_realm: RealmPath,
    pub target_realm: RealmPath,
    pub operation_kind: OperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_capability: Option<Capability>,
    pub authorized_tree_path: TreeRoutePath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_rule_id: Option<RoutePolicyRuleId>,
    pub state: DirectShortcutState,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

impl DirectShortcutAuthorizationMetadata {
    /// Construct only when the shortcut metadata matches the authorized tree
    /// path and has a positive expiry interval.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        shortcut_id: ShortcutAuthorizationId,
        correlation_id: CorrelationId,
        trace: Option<TraceContext>,
        authorizing_ancestor: RealmPath,
        source_realm: RealmPath,
        target_realm: RealmPath,
        operation_kind: OperationKind,
        required_capability: Option<Capability>,
        authorized_tree_path: TreeRoutePath,
        policy_rule_id: Option<RoutePolicyRuleId>,
        state: DirectShortcutState,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Option<Self> {
        if expires_at_unix_seconds <= issued_at_unix_seconds
            || authorized_tree_path.source_realm != source_realm
            || authorized_tree_path.target_realm != target_realm
            || authorized_tree_path.nearest_common_ancestor != authorizing_ancestor
        {
            return None;
        }
        Some(Self {
            shortcut_id,
            correlation_id,
            trace,
            authorizing_ancestor,
            source_realm,
            target_realm,
            operation_kind,
            required_capability,
            authorized_tree_path,
            policy_rule_id,
            state,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }
}

impl<'de> Deserialize<'de> for DirectShortcutAuthorizationMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            shortcut_id: ShortcutAuthorizationId,
            correlation_id: CorrelationId,
            #[serde(default)]
            trace: Option<TraceContext>,
            authorizing_ancestor: RealmPath,
            source_realm: RealmPath,
            target_realm: RealmPath,
            operation_kind: OperationKind,
            #[serde(default)]
            required_capability: Option<Capability>,
            authorized_tree_path: TreeRoutePath,
            #[serde(default)]
            policy_rule_id: Option<RoutePolicyRuleId>,
            state: DirectShortcutState,
            issued_at_unix_seconds: u64,
            expires_at_unix_seconds: u64,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.shortcut_id,
            raw.correlation_id,
            raw.trace,
            raw.authorizing_ancestor,
            raw.source_realm,
            raw.target_realm,
            raw.operation_kind,
            raw.required_capability,
            raw.authorized_tree_path,
            raw.policy_rule_id,
            raw.state,
            raw.issued_at_unix_seconds,
            raw.expires_at_unix_seconds,
        )
        .ok_or_else(|| serde::de::Error::custom("invalid direct shortcut authorization metadata"))
    }
}

/// Direct shortcut teardown metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectShortcutTeardownMetadata {
    pub shortcut_id: ShortcutAuthorizationId,
    pub correlation_id: CorrelationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
    pub source_realm: RealmPath,
    pub target_realm: RealmPath,
    pub reason: DirectShortcutTeardownReason,
    pub torn_down_at_unix_seconds: u64,
}

/// Stable shortcut teardown reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DirectShortcutTeardownReason {
    Completed,
    Expired,
    PolicyRevoked,
    RouteRevoked,
    TransportUnavailable,
    PeerDisconnected,
}

/// Route event kind for bounded audit metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RouteAuditEventKind {
    DiscoveryQueued,
    DiscoveryDropped,
    SessionAdmitted,
    SessionDenied,
    AdvertisementAccepted,
    AdvertisementDenied,
    RouteAllowed,
    RouteDenied,
    ShortcutAuthorized,
    ShortcutDenied,
    ShortcutTornDown,
}

/// Low-cardinality realm class suitable for route metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RouteRealmClass {
    LocalRoot,
    StaticConfigured,
    HostLocalPeer,
    GatewayBacked,
    CloudFullHost,
    ProviderManaged,
    EphemeralDiscovered,
    Unknown,
}

impl RouteRealmClass {
    /// Stable low-cardinality label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalRoot => "local-root",
            Self::StaticConfigured => "static-configured",
            Self::HostLocalPeer => "host-local-peer",
            Self::GatewayBacked => "gateway-backed",
            Self::CloudFullHost => "cloud-full-host",
            Self::ProviderManaged => "provider-managed",
            Self::EphemeralDiscovered => "ephemeral-discovered",
            Self::Unknown => "unknown",
        }
    }
}

/// Low-cardinality placement class. Provider ids and raw transport addresses
/// stay out of telemetry labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RoutePlacementClass {
    HostLocal,
    GatewayVm,
    CloudFullHost,
    ProviderController,
    ProviderAgent,
    ProviderSpecific,
    Unknown,
}

impl From<&RealmControllerPlacement> for RoutePlacementClass {
    fn from(value: &RealmControllerPlacement) -> Self {
        match value {
            RealmControllerPlacement::HostLocal => Self::HostLocal,
            RealmControllerPlacement::GatewayVm => Self::GatewayVm,
            RealmControllerPlacement::CloudFullHost => Self::CloudFullHost,
            RealmControllerPlacement::ProviderController { .. } => Self::ProviderController,
            RealmControllerPlacement::ProviderAgent { .. } => Self::ProviderAgent,
            RealmControllerPlacement::ProviderSpecific { .. } => Self::ProviderSpecific,
        }
    }
}

impl RoutePlacementClass {
    /// Stable low-cardinality label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::HostLocal => "host-local",
            Self::GatewayVm => "gateway-vm",
            Self::CloudFullHost => "cloud-full-host",
            Self::ProviderController => "provider-controller",
            Self::ProviderAgent => "provider-agent",
            Self::ProviderSpecific => "provider-specific",
            Self::Unknown => "unknown",
        }
    }
}

/// Route audit labels. These may include a bounded policy id for operator audit,
/// but never payloads, relay endpoints, peer identity strings, or raw metrics
/// dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteAuditLabels {
    pub event: RouteAuditEventKind,
    pub source_realm_class: RouteRealmClass,
    pub target_realm_class: RouteRealmClass,
    pub placement: RoutePlacementClass,
    pub operation_kind: OperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<RouteFailClosedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_rule_id: Option<RoutePolicyRuleId>,
}

/// Route telemetry counter kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RouteTelemetryCounterKind {
    ApiLatencyCount,
    ApiErrorCount,
    DiscoveryQueueDepth,
    DiscoveryDropNewCount,
    PreAuthRateLimitHitCount,
    RouteAdvertisementAcceptedCount,
    RouteAdvertisementDeniedCount,
    RouteDecisionAllowedCount,
    RouteDecisionDeniedCount,
    ShortcutAuthorizedCount,
    ShortcutDeniedCount,
    RevocationTeardownCount,
    SessionTeardownCount,
}

impl RouteTelemetryCounterKind {
    /// Stable low-cardinality metric name suffix.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ApiLatencyCount => "api-latency-count",
            Self::ApiErrorCount => "api-error-count",
            Self::DiscoveryQueueDepth => "discovery-queue-depth",
            Self::DiscoveryDropNewCount => "discovery-drop-new-count",
            Self::PreAuthRateLimitHitCount => "pre-auth-rate-limit-hit-count",
            Self::RouteAdvertisementAcceptedCount => "route-advertisement-accepted-count",
            Self::RouteAdvertisementDeniedCount => "route-advertisement-denied-count",
            Self::RouteDecisionAllowedCount => "route-decision-allowed-count",
            Self::RouteDecisionDeniedCount => "route-decision-denied-count",
            Self::ShortcutAuthorizedCount => "shortcut-authorized-count",
            Self::ShortcutDeniedCount => "shortcut-denied-count",
            Self::RevocationTeardownCount => "revocation-teardown-count",
            Self::SessionTeardownCount => "session-teardown-count",
        }
    }
}

/// Low-cardinality metric labels for route telemetry. It intentionally carries
/// no realm path, target address, raw transport address, or peer identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteTelemetryLabels {
    pub source_realm_class: RouteRealmClass,
    pub target_realm_class: RouteRealmClass,
    pub placement: RoutePlacementClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<OperationKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<RouteFailClosedReason>,
}

/// One bounded route telemetry counter sample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteTelemetrySample {
    pub counter: RouteTelemetryCounterKind,
    pub labels: RouteTelemetryLabels,
    pub value: u64,
}

/// Bounded telemetry sample batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteTelemetryBatch {
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ROUTE_TELEMETRY_SAMPLES>"
    )]
    #[schemars(length(max = 64))]
    pub samples: Vec<RouteTelemetrySample>,
}

fn next_hop_child<'a>(descendant: &'a RealmPath, advertiser: &RealmPath) -> Option<&'a RealmId> {
    let descendant_index = descendant
        .labels()
        .len()
        .checked_sub(advertiser.labels().len() + 1)?;
    descendant.labels().get(descendant_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::ids::RealmId;
    use schemars::schema_for;

    fn fp() -> KeyFingerprint {
        KeyFingerprint::parse(format!("sha256:{}", "b".repeat(64))).unwrap()
    }

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(
            labels
                .iter()
                .map(|label| RealmId::parse(*label).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn signature() -> RouteSignature {
        RouteSignature {
            algorithm: ProtocolToken::parse("ed25519-v1").unwrap(),
            key_role: RealmKeyRole::ControllerGeneration,
            signing_key_fingerprint: fp(),
            signature_ref: SignatureRef::parse("sig-1").unwrap(),
        }
    }

    fn route_path() -> TreeRoutePath {
        let local = realm(&["local"]);
        let dev = realm(&["dev", "local"]);
        let work = realm(&["work", "local"]);
        let hop_up = TreeRouteHop::new(
            dev.clone(),
            local.clone(),
            RealmTreeEdge::new(local.clone(), dev.clone()).unwrap(),
            TreeRouteHopDirection::UpToParent,
            Some(RouteId::parse("route-dev").unwrap()),
        )
        .unwrap();
        let hop_down = TreeRouteHop::new(
            local.clone(),
            work.clone(),
            RealmTreeEdge::new(local.clone(), work.clone()).unwrap(),
            TreeRouteHopDirection::DownToChild,
            Some(RouteId::parse("route-work").unwrap()),
        )
        .unwrap();
        TreeRoutePath::new(dev, work, local, vec![hop_up, hop_down]).unwrap()
    }

    #[test]
    fn signature_ref_rejects_leading_punctuation() {
        assert!(SignatureRef::parse("sig-1").is_some());
        assert!(SignatureRef::parse("-sig-1").is_none());
        assert!(SignatureRef::parse(".sig-1").is_none());
        assert!(SignatureRef::parse("_sig-1").is_none());
        assert!(SignatureRef::parse(":sig-1").is_none());
    }

    #[test]
    fn route_advertisement_accepts_descendants_only() {
        let parent = realm(&["work"]);
        let child = realm(&["payments", "work"]);
        let edge = RealmTreeEdge::new(parent, child.clone()).unwrap();
        let route = DescendantRoute {
            route_id: RouteId::parse("route-1").unwrap(),
            descendant: realm(&["api", "payments", "work"]),
            next_hop_child: RealmId::parse("api").unwrap(),
            capabilities: CapabilitySet::empty().with(Capability::Exec),
        };
        assert!(
            RouteAdvertisement::new(
                child,
                edge,
                ControllerGenerationId::parse("gen-1").unwrap(),
                vec![route],
                10,
                20,
                signature(),
            )
            .is_some()
        );
    }

    #[test]
    fn route_advertisement_rejects_sibling_parent_and_unbounded_routes() {
        let work = realm(&["work"]);
        let payments = realm(&["payments", "work"]);
        let edge = RealmTreeEdge::new(work, payments.clone()).unwrap();
        let sibling = DescendantRoute {
            route_id: RouteId::parse("route-1").unwrap(),
            descendant: realm(&["dev"]),
            next_hop_child: RealmId::parse("dev").unwrap(),
            capabilities: CapabilitySet::empty(),
        };
        assert!(
            RouteAdvertisement::new(
                payments.clone(),
                edge.clone(),
                ControllerGenerationId::parse("gen-1").unwrap(),
                vec![sibling],
                10,
                20,
                signature(),
            )
            .is_none()
        );

        let bad_next_hop = DescendantRoute {
            route_id: RouteId::parse("route-2").unwrap(),
            descendant: realm(&["api", "payments", "work"]),
            next_hop_child: RealmId::parse("payments").unwrap(),
            capabilities: CapabilitySet::empty(),
        };
        assert!(
            RouteAdvertisement::new(
                payments.clone(),
                edge.clone(),
                ControllerGenerationId::parse("gen-1").unwrap(),
                vec![bad_next_hop],
                10,
                20,
                signature(),
            )
            .is_none()
        );

        let too_many = (0..=MAX_ADVERTISED_ROUTES)
            .map(|i| DescendantRoute {
                route_id: RouteId::parse(format!("route-{i}")).unwrap(),
                descendant: realm(&[&format!("api{i}"), "payments", "work"]),
                next_hop_child: RealmId::parse(format!("api{i}")).unwrap(),
                capabilities: CapabilitySet::empty(),
            })
            .collect::<Vec<_>>();
        assert!(
            RouteAdvertisement::new(
                payments,
                edge,
                ControllerGenerationId::parse("gen-1").unwrap(),
                too_many,
                10,
                20,
                signature(),
            )
            .is_none()
        );
    }

    #[test]
    fn route_advertisement_decode_rejects_unknown_fields_and_bad_expiry() {
        let json = "{\"advertisingRealm\":[\"payments\",\"work\"],\
            \"treeEdge\":{\"parent\":[\"work\"],\"child\":[\"payments\",\"work\"]},\
            \"controllerGeneration\":\"gen-1\",\
            \"routes\":[{\"routeId\":\"route-1\",\"descendant\":[\"api\",\"payments\",\"work\"],\
            \"nextHopChild\":\"api\",\"capabilities\":[\"exec\"]}],\
            \"issuedAtUnixSeconds\":20,\"expiresAtUnixSeconds\":10,\
            \"signature\":{\"algorithm\":\"ed25519-v1\",\"keyRole\":\"controller-generation\",\
            \"signingKeyFingerprint\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\
            \"signatureRef\":\"sig-1\"}}";
        assert!(serde_json::from_str::<RouteAdvertisement>(json).is_err());

        let edge = "{\"parent\":[\"work\"],\"child\":[\"payments\",\"work\"],\"extra\":1}";
        assert!(serde_json::from_str::<RealmTreeEdge>(edge).is_err());
    }

    #[test]
    fn queue_replay_namespace_and_path_bounds_decode_fail_closed() {
        assert!(
            DiscoveryQueuePolicy::new(128, 32, 60, 30, DiscoveryQueueDropPolicy::DropNew).is_some()
        );
        let bad_queue = "{\"maxDepth\":4097,\"maxUnverifiedPeers\":1,\
            \"perRelayRateLimitPerMinute\":1,\"perUnverifiedPeerRateLimitPerMinute\":1,\
            \"dropPolicy\":\"drop-new\"}";
        assert!(serde_json::from_str::<DiscoveryQueuePolicy>(bad_queue).is_err());

        let replay = ReplayWindowMetadata::new(
            RouteReplayWindowId::parse("replay-1").unwrap(),
            10,
            11,
            60,
            0,
            1,
        );
        assert!(replay.is_none());

        let edge = RealmTreeEdge::new(realm(&["work"]), realm(&["payments", "work"])).unwrap();
        assert!(
            RouteNamespaceAllocation::new(
                edge.clone(),
                ControllerGenerationId::parse("gen-1").unwrap(),
                vec![realm(&["dev"])],
                1,
                CapabilitySet::empty(),
            )
            .is_none()
        );
        assert!(
            RouteNamespaceAllocation::new(
                edge,
                ControllerGenerationId::parse("gen-1").unwrap(),
                vec![realm(&["api", "payments", "work"])],
                1,
                CapabilitySet::empty(),
            )
            .is_some()
        );

        let path = route_path();
        let mut bad_path = serde_json::to_value(&path).unwrap();
        bad_path
            .as_object_mut()
            .unwrap()
            .insert("extra".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<TreeRoutePath>(bad_path).is_err());
    }

    #[test]
    fn session_and_shortcut_metadata_carry_correlation_and_trace_only() {
        let trace = TraceContext::new("trace-1", "span-1").unwrap();
        let replay = ReplayWindowMetadata::new(
            RouteReplayWindowId::parse("replay-1").unwrap(),
            10,
            1,
            60,
            0,
            1,
        )
        .unwrap();
        let session = SessionAdmissionAttemptMetadata {
            attempt_id: OperationId::parse("attempt-1").unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            trace: Some(trace.clone()),
            local_realm: realm(&["local"]),
            remote_realm: realm(&["work", "local"]),
            operation_kind: OperationKind::ExecStart,
            required_capability: Some(Capability::Exec),
            replay_window: replay,
            outcome: SessionAdmissionOutcome::Admitted,
        };
        let encoded = serde_json::to_string(&session).unwrap();
        assert!(encoded.contains("corr-1"));
        assert!(encoded.contains("trace-1"));
        assert!(!encoded.contains("endpoint"));

        let shortcut = DirectShortcutAuthorizationMetadata {
            shortcut_id: ShortcutAuthorizationId::parse("shortcut-1").unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            trace: Some(trace),
            authorizing_ancestor: realm(&["local"]),
            source_realm: realm(&["dev", "local"]),
            target_realm: realm(&["work", "local"]),
            operation_kind: OperationKind::ExecStart,
            required_capability: Some(Capability::Exec),
            authorized_tree_path: route_path(),
            policy_rule_id: RoutePolicyRuleId::parse("policy-1"),
            state: DirectShortcutState::Authorized,
            issued_at_unix_seconds: 10,
            expires_at_unix_seconds: 20,
        };
        let shortcut_json = serde_json::to_string(&shortcut).unwrap();
        assert!(shortcut_json.contains("authorizedTreePath"));
        assert!(!shortcut_json.contains("relayEndpoint"));
    }

    #[test]
    fn route_fail_closed_reason_labels_are_stable() {
        let reasons = [
            (RouteFailClosedReason::MalformedAdvert, "malformed-advert"),
            (RouteFailClosedReason::UnknownParent, "unknown-parent"),
            (
                RouteFailClosedReason::NamespaceViolation,
                "namespace-violation",
            ),
            (
                RouteFailClosedReason::SiblingOrParentRouteAdvert,
                "sibling-or-parent-route-advert",
            ),
            (RouteFailClosedReason::Loop, "loop"),
            (RouteFailClosedReason::MultiParent, "multi-parent"),
            (RouteFailClosedReason::Expired, "expired"),
            (RouteFailClosedReason::Replay, "replay"),
            (RouteFailClosedReason::RateLimited, "rate-limited"),
            (
                RouteFailClosedReason::QueueFullDropNew,
                "queue-full-drop-new",
            ),
            (
                RouteFailClosedReason::MissingCapability,
                "missing-capability",
            ),
            (RouteFailClosedReason::PolicyDenial, "policy-denial"),
        ];
        for (reason, label) in reasons {
            assert_eq!(reason.label(), label);
            assert_eq!(
                serde_json::to_string(&reason).unwrap(),
                format!("\"{label}\"")
            );
        }
    }

    #[test]
    fn metric_labels_are_low_cardinality_and_stable() {
        assert_eq!(DiscoveryIngressClass::ParentRelay.label(), "parent-relay");
        assert_eq!(RouteRealmClass::ProviderManaged.label(), "provider-managed");
        assert_eq!(RoutePlacementClass::GatewayVm.label(), "gateway-vm");
        assert_eq!(
            RouteTelemetryCounterKind::PreAuthRateLimitHitCount.label(),
            "pre-auth-rate-limit-hit-count"
        );

        let sample = RouteTelemetrySample {
            counter: RouteTelemetryCounterKind::RouteDecisionDeniedCount,
            labels: RouteTelemetryLabels {
                source_realm_class: RouteRealmClass::StaticConfigured,
                target_realm_class: RouteRealmClass::ProviderManaged,
                placement: RoutePlacementClass::ProviderAgent,
                operation_kind: Some(OperationKind::ExecStart),
                reason: Some(RouteFailClosedReason::PolicyDenial),
            },
            value: 1,
        };
        let encoded = serde_json::to_string(&sample).unwrap();
        assert!(!encoded.contains("work.local"));
        assert!(!encoded.contains("peerIdentity"));
        assert!(!encoded.contains("endpoint"));
    }

    #[test]
    fn redacted_refs_do_not_leak_debug_values() {
        let peer = UnverifiedPeerRef::parse("peer-handle-1").unwrap();
        let policy = RoutePolicyRuleId::parse("policy-allow-exec").unwrap();
        let shortcut = ShortcutAuthorizationId::parse("shortcut-1").unwrap();
        assert!(!format!("{peer:?}").contains("peer-handle-1"));
        assert!(!format!("{policy:?}").contains("policy-allow-exec"));
        assert!(!format!("{shortcut:?}").contains("shortcut-1"));
        assert!(UnverifiedPeerRef::parse("relay-endpoint-1").is_none());
        assert!(RoutePolicyRuleId::parse("secret-policy").is_none());
    }

    #[test]
    fn route_schema_roots_are_generated() {
        assert!(schema_for!(RouteAdvertisement).schema.metadata.is_some());
        assert!(schema_for!(DiscoveryQueuePolicy).schema.metadata.is_some());
        assert!(schema_for!(ReplayWindowMetadata).schema.metadata.is_some());
        assert!(
            schema_for!(RouteNamespaceAllocation)
                .schema
                .metadata
                .is_some()
        );
        assert!(schema_for!(TreeRouteDecision).schema.metadata.is_some());
        assert!(
            schema_for!(DirectShortcutAuthorizationMetadata)
                .schema
                .metadata
                .is_some()
        );
        assert!(schema_for!(RouteTelemetryBatch).schema.metadata.is_some());
    }

    #[test]
    fn telemetry_schema_has_no_secret_endpoint_or_path_shaped_labels() {
        let schema = serde_json::to_string(&schema_for!(RouteTelemetrySample)).unwrap();
        for forbidden in [
            "RealmPath",
            "realmPath",
            "peerIdentity",
            "relayEndpoint",
            "endpoint",
            "socketPath",
            "credential",
            "secret",
        ] {
            assert!(
                !schema.contains(forbidden),
                "telemetry schema unexpectedly contains {forbidden}"
            );
        }
    }
}
