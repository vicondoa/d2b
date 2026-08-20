//! Zone routing contracts.
//!
//! This module owns the plain data contracts the v3 Zone tree routing layer
//! is grounded on: the Zone tree path model (`ZonePath` / `ZoneLabelId`), the
//! opaque route and controller-generation identifiers, the signed and expiring
//! route advertisement envelope, its withdrawal message, the parent's private
//! namespace allocation, the immutable route-decision path, and the closed
//! fail-closed reason and audit-event enumerations.
//!
//! Every type here is desired-state or decision metadata. Nothing in this
//! module carries authority: there is no session, admission evidence, verified
//! peer, subject, or proof, and no transport socket, relay endpoint, device or
//! store path, credential, or key material. The engine that consumes these
//! types (`ZoneRouteEngine`) supplies all runtime state itself; these contracts
//! perform no I/O.
//!
//! Shared scalars, the shared error type, and the canonical JSON renderer are
//! reused from `super::execution_policy` and `super::resource_schema` rather
//! than restated here.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{ArrayValidation, InstanceType, Schema, SchemaObject, SingleOrVec},
};
use serde::{Deserialize, Deserializer, Serialize};

use super::execution_policy::{
    BoundedToken, MAX_BOUNDED_TOKEN_BYTES, PrimitiveSpecError, parsed_deserialize, redacted_debug,
    string_schema,
};

/// Schema version of the v3 Zone routing wire contracts.
///
/// The v3 advertisement envelope carries this value explicitly so a peer can
/// never reinterpret a v2 realm-route payload as a v3 Zone-route payload. v3
/// numbering is frozen independently of the v2 assignments.
pub const ZONE_ROUTING_SCHEMA_VERSION: u32 = 3;

/// Maximum Zone names in one compiler-authored ancestry path.
///
/// The local root counts as one label.
pub const MAX_ZONE_PATH_LABELS: usize = 16;
/// Maximum rendered bytes of one Zone tree path.
pub const MAX_ZONE_PATH_BYTES: usize = 255;
/// Maximum routes carried in one advertisement.
pub const MAX_ADVERTISED_ZONE_ROUTES: usize = 64;
/// Maximum allowed namespace prefixes in one allocation.
pub const MAX_ZONE_ROUTE_NAMESPACE_PREFIXES: usize = 16;
/// Maximum hops carried in one Zone route path.
pub const MAX_ZONE_ROUTE_PATH_HOPS: usize = 32;
/// Maximum parent entries tracked by one route engine.
pub const MAX_ZONE_PARENT_ENTRIES: usize = 4096;
/// Maximum route entries tracked by one route engine.
pub const MAX_ZONE_ROUTE_ENTRIES: usize = 4096;
/// Maximum capability assertions in one route capability set.
pub const MAX_ZONE_ROUTE_CAPABILITIES: usize = 64;
/// Maximum advertisement lifetime in seconds.
pub const MAX_ZONE_ADVERTISEMENT_LIFETIME_SECONDS: u64 = 7_200;
/// Maximum bytes in one bounded opaque routing token.
pub const MAX_ZONE_ROUTE_TOKEN_BYTES: usize = 128;
/// Protocol-wide initial per-call hop budget.
///
/// The budget is fixed by the protocol and is deliberately not a ZoneLink spec
/// field, so it is not configurable per link.
pub const ZONE_ROUTE_INITIAL_HOP_BUDGET: u32 = 16;

/// The distinguished local-root Zone name.
pub const LOCAL_ROOT_ZONE_NAME: &str = "local-root";

const SECRET_MARKERS: &[&str] = &[
    "secret",
    "password",
    "passwd",
    "bearer",
    "credential",
    "private",
    "apikey",
    "token",
    "privatekey",
    "accesstoken",
    "refreshtoken",
    "sessiontoken",
    "endpoint",
    "socketpath",
];

/// True for a bounded, non-empty opaque token safe for audit metadata.
///
/// Path-shaped and credential-shaped spellings are rejected because these
/// tokens appear in audit records.
fn is_opaque_routing_token(value: &str) -> bool {
    let Some(first) = value.chars().next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return false;
    }
    if value.contains("..") {
        return false;
    }
    let compact = value
        .chars()
        .filter(|character| !matches!(character, '-' | '_' | '.'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    !SECRET_MARKERS.iter().any(|marker| compact.contains(marker))
}

macro_rules! opaque_routing_token {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate and construct a bounded, non-secret opaque token.
            pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > MAX_ZONE_ROUTE_TOKEN_BYTES
                    || !is_opaque_routing_token(&value)
                {
                    return Err(PrimitiveSpecError::InvalidToken);
                }
                Ok(Self(value))
            }

            /// Borrow the token for an authorized encoding surface.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        redacted_debug!($name);
        parsed_deserialize!($name);
        string_schema!($name, 1, MAX_ZONE_ROUTE_TOKEN_BYTES);
    };
}

opaque_routing_token!(
    /// Bounded route identifier for one advertised descendant route.
    ZoneRouteId
);
opaque_routing_token!(
    /// Opaque controller-generation identifier bound to a child ZoneLink
    /// controller lease. It is a generation handle, never a credential.
    ZoneLinkControllerGeneration
);
opaque_routing_token!(
    /// Detached signature reference for one advertisement.
    ///
    /// This is a locator for a detached signature, never signature or key
    /// bytes.
    ZoneRouteSignatureRef
);
opaque_routing_token!(
    /// Fingerprint of the advertisement signing key.
    ///
    /// A fingerprint is not key material; no public or private key bytes are
    /// represented.
    ZoneSigningKeyFingerprint
);

/// One label in a Zone tree path.
///
/// The grammar is the d2b lowercase label shape `^[a-z][a-z0-9-]*$`, which is
/// exactly the grammar of a Zone resource name, so a label and the Zone's
/// `Zone/<zone-name>` resource name can never disagree.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ZoneLabelId(BoundedToken);

impl ZoneLabelId {
    /// Parse one lowercase Zone label.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        BoundedToken::parse(value).map(Self)
    }

    /// Borrow the canonical label.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(ZoneLabelId);
parsed_deserialize!(ZoneLabelId);
string_schema!(ZoneLabelId, 1, MAX_BOUNDED_TOKEN_BYTES);

/// A Zone tree position as an ordered label path, most specific first.
///
/// The rendered form exposed here is the parent-first storage form the routing
/// engine keys on (`work/payments`). The public v3 addressing form of a Zone is
/// its `Zone/<zone-name>` resource path, which is deliberately not rendered by
/// this type.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ZonePath(Vec<ZoneLabelId>);

impl ZonePath {
    /// Build a path from most-specific-first labels.
    ///
    /// An empty path, a path over [`MAX_ZONE_PATH_LABELS`] labels, and a path
    /// whose rendered form exceeds [`MAX_ZONE_PATH_BYTES`] are all rejected.
    pub fn new(labels: Vec<ZoneLabelId>) -> Result<Self, PrimitiveSpecError> {
        if labels.is_empty() {
            return Err(PrimitiveSpecError::InvalidPath);
        }
        if labels.len() > MAX_ZONE_PATH_LABELS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let rendered: usize = labels
            .iter()
            .map(|label| label.as_str().len())
            .sum::<usize>()
            + labels.len().saturating_sub(1);
        if rendered > MAX_ZONE_PATH_BYTES {
            return Err(PrimitiveSpecError::InvalidPath);
        }
        Ok(Self(labels))
    }

    /// The distinguished local-root Zone path.
    pub fn local_root() -> Self {
        Self(vec![
            ZoneLabelId::parse(LOCAL_ROOT_ZONE_NAME).expect("the local root name is a valid label"),
        ])
    }

    /// Borrow the labels, most specific first.
    pub fn labels(&self) -> &[ZoneLabelId] {
        &self.0
    }

    /// Number of labels in the path.
    pub fn depth(&self) -> usize {
        self.0.len()
    }

    /// Render the canonical parent-first storage form for an authorized
    /// encoding or engine key surface.
    pub fn to_storage_string(&self) -> String {
        self.0
            .iter()
            .rev()
            .map(ZoneLabelId::as_str)
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Whether `self` lies strictly below `ancestor` in the Zone tree.
    pub fn is_descendant_of(&self, ancestor: &Self) -> bool {
        self.0.len() > ancestor.0.len() && self.has_suffix(ancestor)
    }

    /// Whether `self` is exactly one label below `parent`.
    pub fn is_direct_child_of(&self, parent: &Self) -> bool {
        self.0.len() == parent.0.len() + 1 && self.has_suffix(parent)
    }

    /// The immediate child label of `ancestor` on the way toward `self`.
    ///
    /// Returns `None` when `self` is not strictly below `ancestor`.
    pub fn next_hop_label_below(&self, ancestor: &Self) -> Option<&ZoneLabelId> {
        if !self.is_descendant_of(ancestor) {
            return None;
        }
        let index = self.0.len().checked_sub(ancestor.0.len() + 1)?;
        self.0.get(index)
    }

    fn has_suffix(&self, suffix: &Self) -> bool {
        self.0.len() >= suffix.0.len()
            && self
                .0
                .iter()
                .rev()
                .zip(suffix.0.iter().rev())
                .all(|(left, right)| left == right)
    }
}

redacted_debug!(ZonePath);

impl<'de> Deserialize<'de> for ZonePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(Vec::<ZoneLabelId>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ZonePath {
    fn schema_name() -> String {
        "ZonePath".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Array))),
            array: Some(Box::new(ArrayValidation {
                items: Some(SingleOrVec::Single(Box::new(
                    generator.subschema_for::<ZoneLabelId>(),
                ))),
                min_items: Some(1),
                max_items: Some(MAX_ZONE_PATH_LABELS as u32),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// One positive routing capability assertion.
///
/// A capability is a positive assertion: an absent capability is a typed
/// refusal, never a silent fallback. The catalogue of capability codes is owned
/// by the specifications that declare them, so this contract carries the
/// bounded token grammar and the subset ordering rather than a closed
/// enumeration.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ZoneRouteCapability(BoundedToken);

impl ZoneRouteCapability {
    /// Parse one lowercase capability code.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        BoundedToken::parse(value).map(Self)
    }

    /// Borrow the canonical capability code.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(ZoneRouteCapability);
parsed_deserialize!(ZoneRouteCapability);
string_schema!(ZoneRouteCapability, 1, MAX_BOUNDED_TOKEN_BYTES);

/// A bounded, sorted, duplicate-free set of routing capabilities.
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ZoneRouteCapabilitySet(Vec<ZoneRouteCapability>);

impl ZoneRouteCapabilitySet {
    /// Construct a capability set after sorting and checking the frozen bound.
    ///
    /// A duplicate entry is rejected rather than silently folded, so an
    /// advertised set and its rendered canonical form always agree.
    pub fn new(capabilities: Vec<ZoneRouteCapability>) -> Result<Self, PrimitiveSpecError> {
        if capabilities.len() > MAX_ZONE_ROUTE_CAPABILITIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mut sorted = capabilities;
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        if sorted.len() != before {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        Ok(Self(sorted))
    }

    /// Borrow the sorted capabilities.
    pub fn capabilities(&self) -> &[ZoneRouteCapability] {
        &self.0
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the set contains one capability.
    pub fn contains(&self, capability: &ZoneRouteCapability) -> bool {
        self.0.binary_search(capability).is_ok()
    }

    /// Whether every capability in `self` is also present in `ceiling`.
    ///
    /// This is the monotonic downward narrowing check: a child may narrow the
    /// scope its parent allocated, never widen it.
    pub fn is_subset_of(&self, ceiling: &Self) -> bool {
        self.0.iter().all(|capability| ceiling.contains(capability))
    }
}

redacted_debug!(ZoneRouteCapabilitySet);

impl<'de> Deserialize<'de> for ZoneRouteCapabilitySet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(Vec::<ZoneRouteCapability>::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ZoneRouteCapabilitySet {
    fn schema_name() -> String {
        "ZoneRouteCapabilitySet".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Array))),
            array: Some(Box::new(ArrayValidation {
                items: Some(SingleOrVec::Single(Box::new(
                    generator.subschema_for::<ZoneRouteCapability>(),
                ))),
                max_items: Some(MAX_ZONE_ROUTE_CAPABILITIES as u32),
                unique_items: Some(true),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// Signature algorithm for a route advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneRouteSignatureAlgorithm {
    /// The frozen advertisement signature algorithm.
    Ed25519Blake3,
}

/// Controller key role permitted to sign a route advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneRouteKeyRole {
    /// The only role that may sign Zone route advertisements.
    ZoneControllerRouting,
}

/// Signature metadata for one route advertisement.
///
/// The signature itself is referenced, never embedded: no signature bytes and
/// no key bytes appear in this contract.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneRouteSignature {
    algorithm: ZoneRouteSignatureAlgorithm,
    key_role: ZoneRouteKeyRole,
    signing_key_fingerprint: ZoneSigningKeyFingerprint,
    signature_ref: ZoneRouteSignatureRef,
}

impl ZoneRouteSignature {
    /// Construct signature metadata.
    pub const fn new(
        algorithm: ZoneRouteSignatureAlgorithm,
        key_role: ZoneRouteKeyRole,
        signing_key_fingerprint: ZoneSigningKeyFingerprint,
        signature_ref: ZoneRouteSignatureRef,
    ) -> Self {
        Self {
            algorithm,
            key_role,
            signing_key_fingerprint,
            signature_ref,
        }
    }

    /// Return the signature algorithm.
    pub const fn algorithm(&self) -> ZoneRouteSignatureAlgorithm {
        self.algorithm
    }

    /// Return the signing key role.
    pub const fn key_role(&self) -> ZoneRouteKeyRole {
        self.key_role
    }

    /// Borrow the signing key fingerprint.
    pub const fn signing_key_fingerprint(&self) -> &ZoneSigningKeyFingerprint {
        &self.signing_key_fingerprint
    }

    /// Borrow the detached signature reference.
    pub const fn signature_ref(&self) -> &ZoneRouteSignatureRef {
        &self.signature_ref
    }
}

redacted_debug!(ZoneRouteSignature);

/// A parent and direct-child Zone tree edge.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneTreeEdge {
    parent: ZonePath,
    child: ZonePath,
}

impl ZoneTreeEdge {
    /// Construct an edge only when `child` is exactly one label below `parent`.
    pub fn new(parent: ZonePath, child: ZonePath) -> Result<Self, PrimitiveSpecError> {
        if child.is_direct_child_of(&parent) {
            Ok(Self { parent, child })
        } else {
            Err(PrimitiveSpecError::ConflictingFields)
        }
    }

    /// Borrow the parent Zone path.
    pub const fn parent(&self) -> &ZonePath {
        &self.parent
    }

    /// Borrow the direct child Zone path.
    pub const fn child(&self) -> &ZonePath {
        &self.child
    }
}

redacted_debug!(ZoneTreeEdge);

impl<'de> Deserialize<'de> for ZoneTreeEdge {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            parent: ZonePath,
            child: ZonePath,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.parent, wire.child).map_err(serde::de::Error::custom)
    }
}

/// One descendant route advertised by a child Zone controller.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneDescendantRoute {
    route_id: ZoneRouteId,
    descendant: ZonePath,
    next_hop_child: ZoneLabelId,
    capabilities: ZoneRouteCapabilitySet,
}

impl ZoneDescendantRoute {
    /// Construct one descendant route.
    ///
    /// Descendant-strictness and next-hop agreement are checked against the
    /// advertising Zone by [`ZoneLinkRouteAdvertisement::new`], which is the
    /// only place that knows the advertiser.
    pub const fn new(
        route_id: ZoneRouteId,
        descendant: ZonePath,
        next_hop_child: ZoneLabelId,
        capabilities: ZoneRouteCapabilitySet,
    ) -> Self {
        Self {
            route_id,
            descendant,
            next_hop_child,
            capabilities,
        }
    }

    /// Borrow the route identifier.
    pub const fn route_id(&self) -> &ZoneRouteId {
        &self.route_id
    }

    /// Borrow the reachable descendant Zone path.
    pub const fn descendant(&self) -> &ZonePath {
        &self.descendant
    }

    /// Borrow the immediate child label below the advertiser.
    pub const fn next_hop_child(&self) -> &ZoneLabelId {
        &self.next_hop_child
    }

    /// Borrow the positive capabilities reachable on this route.
    pub const fn capabilities(&self) -> &ZoneRouteCapabilitySet {
        &self.capabilities
    }
}

redacted_debug!(ZoneDescendantRoute);

impl<'de> Deserialize<'de> for ZoneDescendantRoute {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            route_id: ZoneRouteId,
            descendant: ZonePath,
            next_hop_child: ZoneLabelId,
            capabilities: ZoneRouteCapabilitySet,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(
            wire.route_id,
            wire.descendant,
            wire.next_hop_child,
            wire.capabilities,
        ))
    }
}

/// A signed, expiring, descendant-only route advertisement.
///
/// The advertising Zone signs this envelope with its enrolled key; the parent
/// validates it before admitting it into in-memory route projection state. An
/// admitted advertisement never mutates the parent's resource store.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneLinkRouteAdvertisement {
    schema_version: u32,
    advertising_zone: ZonePath,
    tree_edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    routes: Vec<ZoneDescendantRoute>,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    signature: ZoneRouteSignature,
    /// Allocator-private capability scope this advertisement was checked
    /// against.
    ///
    /// The allocated scope belongs to the parent's private namespace
    /// allocation, not to the ZoneLink resource spec, so it is never
    /// serialized, never rendered into a public advertisement, and never
    /// leaves the process that attached it.
    #[serde(skip)]
    #[schemars(skip)]
    allocated_capability_scope: Option<ZoneRouteCapabilitySet>,
}

impl ZoneLinkRouteAdvertisement {
    /// Validate the structural advertisement invariants.
    ///
    /// Checked here: the edge child equals the advertising Zone, the route list
    /// is nonempty and within bound, route identifiers are unique, the validity
    /// window is positive and within the frozen lifetime ceiling, and every
    /// route names a strict descendant whose next-hop label is the immediate
    /// child of the advertiser toward that descendant.
    ///
    /// Signature verification, replay-window screening, namespace matching, and
    /// capacity checks are engine concerns and are deliberately not performed
    /// here.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        schema_version: u32,
        advertising_zone: ZonePath,
        tree_edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        routes: Vec<ZoneDescendantRoute>,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
        signature: ZoneRouteSignature,
    ) -> Result<Self, PrimitiveSpecError> {
        if schema_version != ZONE_ROUTING_SCHEMA_VERSION {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        if tree_edge.child() != &advertising_zone {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if routes.is_empty() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        if routes.len() > MAX_ADVERTISED_ZONE_ROUTES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mut route_ids = routes
            .iter()
            .map(ZoneDescendantRoute::route_id)
            .cloned()
            .collect::<Vec<_>>();
        route_ids.sort_unstable();
        let before = route_ids.len();
        route_ids.dedup();
        if route_ids.len() != before {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        if expires_at_unix_seconds <= issued_at_unix_seconds {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if expires_at_unix_seconds - issued_at_unix_seconds
            > MAX_ZONE_ADVERTISEMENT_LIFETIME_SECONDS
        {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        for route in &routes {
            if advertising_zone.depth() >= MAX_ZONE_PATH_LABELS
                && route.descendant().depth() > MAX_ZONE_PATH_LABELS
            {
                return Err(PrimitiveSpecError::TooManyEntries);
            }
            match route.descendant().next_hop_label_below(&advertising_zone) {
                Some(label) if label == route.next_hop_child() => {}
                _ => return Err(PrimitiveSpecError::ConflictingFields),
            }
        }
        Ok(Self {
            schema_version,
            advertising_zone,
            tree_edge,
            controller_generation,
            routes,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            signature,
            allocated_capability_scope: None,
        })
    }

    /// Attach the allocator-private capability scope after proving that every
    /// advertised route narrows within it.
    ///
    /// A route advertising a capability outside the allocated scope is rejected,
    /// which is the monotonic downward narrowing invariant.
    pub fn with_allocated_capability_scope(
        mut self,
        scope: ZoneRouteCapabilitySet,
    ) -> Result<Self, PrimitiveSpecError> {
        if self
            .routes
            .iter()
            .any(|route| !route.capabilities().is_subset_of(&scope))
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        self.allocated_capability_scope = Some(scope);
        Ok(self)
    }

    /// Return the wire schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Borrow the advertising Zone path.
    pub const fn advertising_zone(&self) -> &ZonePath {
        &self.advertising_zone
    }

    /// Borrow the authorizing tree edge.
    pub const fn tree_edge(&self) -> &ZoneTreeEdge {
        &self.tree_edge
    }

    /// Borrow the signing controller generation.
    pub const fn controller_generation(&self) -> &ZoneLinkControllerGeneration {
        &self.controller_generation
    }

    /// Borrow the advertised routes.
    pub fn routes(&self) -> &[ZoneDescendantRoute] {
        &self.routes
    }

    /// Return the issue time as Unix seconds.
    pub const fn issued_at_unix_seconds(&self) -> u64 {
        self.issued_at_unix_seconds
    }

    /// Return the expiry time as Unix seconds.
    pub const fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    /// Borrow the signature metadata.
    pub const fn signature(&self) -> &ZoneRouteSignature {
        &self.signature
    }

    /// Borrow the allocator-private capability scope, when one was attached.
    pub const fn allocated_capability_scope(&self) -> Option<&ZoneRouteCapabilitySet> {
        self.allocated_capability_scope.as_ref()
    }
}

redacted_debug!(ZoneLinkRouteAdvertisement);

impl<'de> Deserialize<'de> for ZoneLinkRouteAdvertisement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            advertising_zone: ZonePath,
            tree_edge: ZoneTreeEdge,
            controller_generation: ZoneLinkControllerGeneration,
            routes: Vec<ZoneDescendantRoute>,
            issued_at_unix_seconds: u64,
            expires_at_unix_seconds: u64,
            signature: ZoneRouteSignature,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.schema_version,
            wire.advertising_zone,
            wire.tree_edge,
            wire.controller_generation,
            wire.routes,
            wire.issued_at_unix_seconds,
            wire.expires_at_unix_seconds,
            wire.signature,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// A signed withdrawal removing an exact set of advertised routes.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneLinkRouteWithdrawal {
    schema_version: u32,
    advertising_zone: ZonePath,
    controller_generation: ZoneLinkControllerGeneration,
    withdrawn_route_ids: Vec<ZoneRouteId>,
    issued_at_unix_seconds: u64,
    signature: ZoneRouteSignature,
}

impl ZoneLinkRouteWithdrawal {
    /// Validate the structural withdrawal invariants.
    ///
    /// The withdrawal must name a nonempty, duplicate-free route set within the
    /// advertisement bound. Whether the named routes are still present is an
    /// engine concern: withdrawing an already expired or unknown route is
    /// idempotent and is not an error here.
    pub fn new(
        schema_version: u32,
        advertising_zone: ZonePath,
        controller_generation: ZoneLinkControllerGeneration,
        withdrawn_route_ids: Vec<ZoneRouteId>,
        issued_at_unix_seconds: u64,
        signature: ZoneRouteSignature,
    ) -> Result<Self, PrimitiveSpecError> {
        if schema_version != ZONE_ROUTING_SCHEMA_VERSION {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        if withdrawn_route_ids.is_empty() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        if withdrawn_route_ids.len() > MAX_ADVERTISED_ZONE_ROUTES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mut unique = withdrawn_route_ids.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != withdrawn_route_ids.len() {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        Ok(Self {
            schema_version,
            advertising_zone,
            controller_generation,
            withdrawn_route_ids,
            issued_at_unix_seconds,
            signature,
        })
    }

    /// Return the wire schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Borrow the withdrawing Zone path.
    pub const fn advertising_zone(&self) -> &ZonePath {
        &self.advertising_zone
    }

    /// Borrow the controller generation that must match the advertisement.
    pub const fn controller_generation(&self) -> &ZoneLinkControllerGeneration {
        &self.controller_generation
    }

    /// Borrow the exact route identifiers to remove.
    pub fn withdrawn_route_ids(&self) -> &[ZoneRouteId] {
        &self.withdrawn_route_ids
    }

    /// Return the issue time as Unix seconds.
    pub const fn issued_at_unix_seconds(&self) -> u64 {
        self.issued_at_unix_seconds
    }

    /// Borrow the signature metadata.
    pub const fn signature(&self) -> &ZoneRouteSignature {
        &self.signature
    }
}

redacted_debug!(ZoneLinkRouteWithdrawal);

impl<'de> Deserialize<'de> for ZoneLinkRouteWithdrawal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            advertising_zone: ZonePath,
            controller_generation: ZoneLinkControllerGeneration,
            withdrawn_route_ids: Vec<ZoneRouteId>,
            issued_at_unix_seconds: u64,
            signature: ZoneRouteSignature,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.schema_version,
            wire.advertising_zone,
            wire.controller_generation,
            wire.withdrawn_route_ids,
            wire.issued_at_unix_seconds,
            wire.signature,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The route namespace a parent allocator delegates to one direct child edge.
///
/// This allocation is allocator-private state. It is not a ZoneLink resource
/// spec field and never appears in a resource bundle.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneLinkNamespaceAllocation {
    tree_edge: ZoneTreeEdge,
    allocated_to_generation: ZoneLinkControllerGeneration,
    allowed_prefixes: Vec<ZonePath>,
    max_routes: u32,
    allowed_capabilities: ZoneRouteCapabilitySet,
}

impl ZoneLinkNamespaceAllocation {
    /// Validate direct-child ownership and the bounded per-edge capacity.
    ///
    /// Every allowed prefix must be the child Zone itself or a descendant under
    /// it, so a sibling or ancestor prefix is rejected. `allowedPrefixes` and
    /// `maxRoutes` together are the explicit per-edge capacity quota, so one
    /// child cannot exhaust the parent's route namespace.
    pub fn new(
        tree_edge: ZoneTreeEdge,
        allocated_to_generation: ZoneLinkControllerGeneration,
        allowed_prefixes: Vec<ZonePath>,
        max_routes: u32,
        allowed_capabilities: ZoneRouteCapabilitySet,
    ) -> Result<Self, PrimitiveSpecError> {
        if allowed_prefixes.is_empty() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        if allowed_prefixes.len() > MAX_ZONE_ROUTE_NAMESPACE_PREFIXES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        if max_routes == 0 || max_routes > MAX_ADVERTISED_ZONE_ROUTES as u32 {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        if allowed_prefixes.iter().any(|prefix| {
            prefix != tree_edge.child() && !prefix.is_descendant_of(tree_edge.child())
        }) {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self {
            tree_edge,
            allocated_to_generation,
            allowed_prefixes,
            max_routes,
            allowed_capabilities,
        })
    }

    /// Borrow the allocated tree edge.
    pub const fn tree_edge(&self) -> &ZoneTreeEdge {
        &self.tree_edge
    }

    /// Borrow the controller generation the allocation is bound to.
    pub const fn allocated_to_generation(&self) -> &ZoneLinkControllerGeneration {
        &self.allocated_to_generation
    }

    /// Borrow the prefixes the child may advertise.
    pub fn allowed_prefixes(&self) -> &[ZonePath] {
        &self.allowed_prefixes
    }

    /// Return the per-edge route ceiling.
    pub const fn max_routes(&self) -> u32 {
        self.max_routes
    }

    /// Borrow the maximum capability scope the parent will route to the child.
    pub const fn allowed_capabilities(&self) -> &ZoneRouteCapabilitySet {
        &self.allowed_capabilities
    }

    /// Whether one advertised prefix falls inside the allocated namespace.
    pub fn admits_prefix(&self, prefix: &ZonePath) -> bool {
        self.allowed_prefixes
            .iter()
            .any(|allowed| prefix == allowed || prefix.is_descendant_of(allowed))
    }
}

redacted_debug!(ZoneLinkNamespaceAllocation);

impl<'de> Deserialize<'de> for ZoneLinkNamespaceAllocation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            tree_edge: ZoneTreeEdge,
            allocated_to_generation: ZoneLinkControllerGeneration,
            allowed_prefixes: Vec<ZonePath>,
            max_routes: u32,
            allowed_capabilities: ZoneRouteCapabilitySet,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.tree_edge,
            wire.allocated_to_generation,
            wire.allowed_prefixes,
            wire.max_routes,
            wire.allowed_capabilities,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Direction of one hop along the Zone tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneRouteHopDirection {
    /// The hop moves from a child toward its parent.
    UpToParent,
    /// The hop moves from a parent toward one of its children.
    DownToChild,
}

/// One validated parent or child hop in a route decision.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneRouteHop {
    from: ZonePath,
    to: ZonePath,
    edge: ZoneTreeEdge,
    direction: ZoneRouteHopDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route_id: Option<ZoneRouteId>,
}

impl ZoneRouteHop {
    /// Construct a hop only when its endpoints match the declared edge and
    /// direction.
    pub fn new(
        from: ZonePath,
        to: ZonePath,
        edge: ZoneTreeEdge,
        direction: ZoneRouteHopDirection,
        route_id: Option<ZoneRouteId>,
    ) -> Result<Self, PrimitiveSpecError> {
        let consistent = match direction {
            ZoneRouteHopDirection::UpToParent => &from == edge.child() && &to == edge.parent(),
            ZoneRouteHopDirection::DownToChild => &from == edge.parent() && &to == edge.child(),
        };
        if !consistent {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self {
            from,
            to,
            edge,
            direction,
            route_id,
        })
    }

    /// Borrow the origin Zone of this hop.
    pub const fn from(&self) -> &ZonePath {
        &self.from
    }

    /// Borrow the destination Zone of this hop.
    pub const fn to(&self) -> &ZonePath {
        &self.to
    }

    /// Borrow the tree edge this hop traverses.
    pub const fn edge(&self) -> &ZoneTreeEdge {
        &self.edge
    }

    /// Return the hop direction.
    pub const fn direction(&self) -> ZoneRouteHopDirection {
        self.direction
    }

    /// Borrow the route identifier, when the hop was selected by one.
    pub const fn route_id(&self) -> Option<&ZoneRouteId> {
        self.route_id.as_ref()
    }
}

redacted_debug!(ZoneRouteHop);

impl<'de> Deserialize<'de> for ZoneRouteHop {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            from: ZonePath,
            to: ZonePath,
            edge: ZoneTreeEdge,
            direction: ZoneRouteHopDirection,
            #[serde(default)]
            route_id: Option<ZoneRouteId>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.from, wire.to, wire.edge, wire.direction, wire.route_id)
            .map_err(serde::de::Error::custom)
    }
}

/// The immutable result of one Zone route decision.
///
/// The path is route metadata only. It carries no transport socket, relay
/// endpoint, credential, or host path; d2b-bus uses the hop list to compose the
/// sequence of forwarding calls.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneRoutePath {
    source_zone: ZonePath,
    target_zone: ZonePath,
    nearest_common_ancestor: ZonePath,
    hops: Vec<ZoneRouteHop>,
}

impl ZoneRoutePath {
    /// Construct a bounded, contiguous route path.
    ///
    /// The hop list must start at the source, end at the target, and be
    /// contiguous. Both endpoints must be the nearest common ancestor itself or
    /// lie below it, and the hop count must stay within
    /// [`MAX_ZONE_ROUTE_PATH_HOPS`].
    pub fn new(
        source_zone: ZonePath,
        target_zone: ZonePath,
        nearest_common_ancestor: ZonePath,
        hops: Vec<ZoneRouteHop>,
    ) -> Result<Self, PrimitiveSpecError> {
        if hops.len() > MAX_ZONE_ROUTE_PATH_HOPS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        if let Some(first) = hops.first()
            && first.from() != &source_zone
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if let Some(last) = hops.last()
            && last.to() != &target_zone
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if hops.windows(2).any(|pair| pair[0].to() != pair[1].from()) {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        for endpoint in [&source_zone, &target_zone] {
            if endpoint != &nearest_common_ancestor
                && !endpoint.is_descendant_of(&nearest_common_ancestor)
            {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
        }
        Ok(Self {
            source_zone,
            target_zone,
            nearest_common_ancestor,
            hops,
        })
    }

    /// Borrow the source Zone path.
    pub const fn source_zone(&self) -> &ZonePath {
        &self.source_zone
    }

    /// Borrow the target Zone path.
    pub const fn target_zone(&self) -> &ZonePath {
        &self.target_zone
    }

    /// Borrow the nearest common ancestor of the source and target.
    pub const fn nearest_common_ancestor(&self) -> &ZonePath {
        &self.nearest_common_ancestor
    }

    /// Borrow the ordered hops.
    pub fn hops(&self) -> &[ZoneRouteHop] {
        &self.hops
    }

    /// Number of hops in the path.
    pub fn hop_count(&self) -> usize {
        self.hops.len()
    }
}

redacted_debug!(ZoneRoutePath);

impl<'de> Deserialize<'de> for ZoneRoutePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            source_zone: ZonePath,
            target_zone: ZonePath,
            nearest_common_ancestor: ZonePath,
            hops: Vec<ZoneRouteHop>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.source_zone,
            wire.target_zone,
            wire.nearest_common_ancestor,
            wire.hops,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The closed fail-closed reason for a refused route decision, advertisement,
/// or relay hop.
///
/// Every label is a stable, low-cardinality kebab-case code safe for an audit
/// record and a metric reason label. No variant carries a field, so a
/// diagnostic can never echo a Zone path, a resource identity, or caller
/// supplied text.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneRouteFailClosedReason {
    /// An advertisement failed a structural invariant.
    MalformedAdvert,
    /// A referenced Zone has no known, non-expired parent or route entry.
    UnknownParent,
    /// An advertisement exceeded its allocated namespace or capability scope.
    NamespaceViolation,
    /// An advertisement named a sibling or ancestor rather than a descendant.
    SiblingOrParentRouteAdvert,
    /// The tree walk revisited a Zone.
    Loop,
    /// Two advertisements claim different next hops for one descendant.
    MultiParent,
    /// The advertisement was received after its expiry.
    Expired,
    /// The advertisement duplicated a live replay-window entry.
    Replay,
    /// The peer exceeded its pre-authentication rate limit.
    RateLimited,
    /// The bounded queue was full and the new item was dropped.
    QueueFullDropNew,
    /// The capability required by the operation is absent from the route.
    MissingCapability,
    /// Policy refused the decision.
    PolicyDenial,
    /// The ZoneLink session is not established.
    ZoneLinkDisconnected,
    /// The forwarded call has no remaining hops.
    HopLimitExceeded,
    /// A forwarding Zone lacks the exact ZoneLink-scoped relay grant.
    RelayDenied,
    /// A descriptor attachment was offered over a ZoneLink.
    AttachmentNotPermittedOverZoneLink,
}

impl ZoneRouteFailClosedReason {
    /// The stable kebab-case label for audit records and metric reason labels.
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
            Self::ZoneLinkDisconnected => "zone-link-disconnected",
            Self::HopLimitExceeded => "hop-limit-exceeded",
            Self::RelayDenied => "relay-denied",
            Self::AttachmentNotPermittedOverZoneLink => "attachment-not-permitted-over-zone-link",
        }
    }
}

/// The closed set of Zone routing audit event kinds.
///
/// The record bodies these kinds label carry Zone path digests rather than raw
/// paths, and never a transport endpoint, resource payload, credential, or host
/// path.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneRouteAuditEventKind {
    /// A route decision was allowed.
    ZoneRouteAllowed,
    /// A route decision was refused.
    ZoneRouteDenied,
    /// An advertisement was admitted.
    ZoneAdvertisementAccepted,
    /// An advertisement was refused.
    ZoneAdvertisementDenied,
    /// One or more routes were withdrawn.
    ZoneAdvertisementWithdrawn,
    /// A ZoneLink session was established.
    ZoneLinkSessionEstablished,
    /// A ZoneLink session failed.
    ZoneLinkSessionFailed,
    /// An intent was queued while the uplink was disconnected.
    ZoneLinkIntentQueued,
    /// A direct shortcut was authorized.
    ZoneLinkShortcutAuthorized,
    /// A direct shortcut was torn down.
    ZoneLinkShortcutTornDown,
    /// A ZoneLink was revoked.
    ZoneLinkRevoked,
    /// A relay hop was admitted.
    ZoneLinkRelayAdmitted,
    /// A relay hop was refused.
    ZoneLinkRelayDenied,
}

impl ZoneRouteAuditEventKind {
    /// The stable kebab-case audit event label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ZoneRouteAllowed => "zone-route-allowed",
            Self::ZoneRouteDenied => "zone-route-denied",
            Self::ZoneAdvertisementAccepted => "zone-advertisement-accepted",
            Self::ZoneAdvertisementDenied => "zone-advertisement-denied",
            Self::ZoneAdvertisementWithdrawn => "zone-advertisement-withdrawn",
            Self::ZoneLinkSessionEstablished => "zone-link-session-established",
            Self::ZoneLinkSessionFailed => "zone-link-session-failed",
            Self::ZoneLinkIntentQueued => "zone-link-intent-queued",
            Self::ZoneLinkShortcutAuthorized => "zone-link-shortcut-authorized",
            Self::ZoneLinkShortcutTornDown => "zone-link-shortcut-torn-down",
            Self::ZoneLinkRevoked => "zone-link-revoked",
            Self::ZoneLinkRelayAdmitted => "zone-link-relay-admitted",
            Self::ZoneLinkRelayDenied => "zone-link-relay-denied",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::resource_schema::canonical_json_bytes;

    fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .map(|label| ZoneLabelId::parse(*label).expect("valid label"))
                .collect(),
        )
        .expect("valid zone path")
    }

    fn capabilities(codes: &[&str]) -> ZoneRouteCapabilitySet {
        ZoneRouteCapabilitySet::new(
            codes
                .iter()
                .map(|code| ZoneRouteCapability::parse(*code).expect("valid capability"))
                .collect(),
        )
        .expect("valid capability set")
    }

    fn signature() -> ZoneRouteSignature {
        ZoneRouteSignature::new(
            ZoneRouteSignatureAlgorithm::Ed25519Blake3,
            ZoneRouteKeyRole::ZoneControllerRouting,
            ZoneSigningKeyFingerprint::parse(format!("sha256.{}", "b".repeat(64)))
                .expect("valid fingerprint"),
            ZoneRouteSignatureRef::parse("sigref-1").expect("valid signature ref"),
        )
    }

    fn advertisement() -> ZoneLinkRouteAdvertisement {
        ZoneLinkRouteAdvertisement::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            ZoneTreeEdge::new(zone(&["k0"]), zone(&["k1", "k0"])).expect("direct child"),
            ZoneLinkControllerGeneration::parse("gen-1").expect("valid generation"),
            vec![ZoneDescendantRoute::new(
                ZoneRouteId::parse("route-1").expect("valid route id"),
                zone(&["k2", "k1", "k0"]),
                ZoneLabelId::parse("k2").expect("valid label"),
                capabilities(&["get", "list"]),
            )],
            1_000,
            4_600,
            signature(),
        )
        .expect("valid advertisement")
    }

    #[test]
    fn golden_advertisement_vector_renders_canonical_bytes_and_round_trips() {
        let advert = advertisement();
        let bytes = canonical_json_bytes(&advert).expect("canonical bytes");
        assert_eq!(
            bytes,
            br#"{"advertisingZone":["k1","k0"],"controllerGeneration":"gen-1","expiresAtUnixSeconds":4600,"issuedAtUnixSeconds":1000,"routes":[{"capabilities":["get","list"],"descendant":["k2","k1","k0"],"nextHopChild":"k2","routeId":"route-1"}],"schemaVersion":3,"signature":{"algorithm":"ed25519-blake3","keyRole":"zone-controller-routing","signatureRef":"sigref-1","signingKeyFingerprint":"sha256.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"treeEdge":{"child":["k1","k0"],"parent":["k0"]}}"#
        );
        let decoded: ZoneLinkRouteAdvertisement =
            serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(decoded, advert);
        assert!(decoded.allocated_capability_scope().is_none());
    }

    #[test]
    fn golden_route_path_and_failure_labels_are_stable() {
        let hop = ZoneRouteHop::new(
            zone(&["k1", "k0"]),
            zone(&["k0"]),
            ZoneTreeEdge::new(zone(&["k0"]), zone(&["k1", "k0"])).unwrap(),
            ZoneRouteHopDirection::UpToParent,
            None,
        )
        .unwrap();
        let path = ZoneRoutePath::new(zone(&["k1", "k0"]), zone(&["k0"]), zone(&["k0"]), vec![hop])
            .unwrap();
        assert_eq!(path.hop_count(), 1);
        assert_eq!(
            canonical_json_bytes(&path).unwrap(),
            br#"{"hops":[{"direction":"up-to-parent","edge":{"child":["k1","k0"],"parent":["k0"]},"from":["k1","k0"],"to":["k0"]}],"nearestCommonAncestor":["k0"],"sourceZone":["k1","k0"],"targetZone":["k0"]}"#
        );

        let labels = [
            (
                ZoneRouteFailClosedReason::MalformedAdvert,
                "malformed-advert",
            ),
            (ZoneRouteFailClosedReason::UnknownParent, "unknown-parent"),
            (
                ZoneRouteFailClosedReason::NamespaceViolation,
                "namespace-violation",
            ),
            (
                ZoneRouteFailClosedReason::SiblingOrParentRouteAdvert,
                "sibling-or-parent-route-advert",
            ),
            (ZoneRouteFailClosedReason::Loop, "loop"),
            (ZoneRouteFailClosedReason::MultiParent, "multi-parent"),
            (ZoneRouteFailClosedReason::Expired, "expired"),
            (ZoneRouteFailClosedReason::Replay, "replay"),
            (ZoneRouteFailClosedReason::RateLimited, "rate-limited"),
            (
                ZoneRouteFailClosedReason::QueueFullDropNew,
                "queue-full-drop-new",
            ),
            (
                ZoneRouteFailClosedReason::MissingCapability,
                "missing-capability",
            ),
            (ZoneRouteFailClosedReason::PolicyDenial, "policy-denial"),
            (
                ZoneRouteFailClosedReason::ZoneLinkDisconnected,
                "zone-link-disconnected",
            ),
            (
                ZoneRouteFailClosedReason::HopLimitExceeded,
                "hop-limit-exceeded",
            ),
            (ZoneRouteFailClosedReason::RelayDenied, "relay-denied"),
            (
                ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink,
                "attachment-not-permitted-over-zone-link",
            ),
        ];
        for (reason, label) in labels {
            assert_eq!(reason.label(), label);
            assert_eq!(
                serde_json::to_string(&reason).unwrap(),
                format!("\"{label}\"")
            );
        }
        let mut unique = labels.iter().map(|(_, label)| *label).collect::<Vec<_>>();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len());
    }

    #[test]
    fn audit_event_labels_are_unique_and_zone_prefixed() {
        let kinds = [
            ZoneRouteAuditEventKind::ZoneRouteAllowed,
            ZoneRouteAuditEventKind::ZoneRouteDenied,
            ZoneRouteAuditEventKind::ZoneAdvertisementAccepted,
            ZoneRouteAuditEventKind::ZoneAdvertisementDenied,
            ZoneRouteAuditEventKind::ZoneAdvertisementWithdrawn,
            ZoneRouteAuditEventKind::ZoneLinkSessionEstablished,
            ZoneRouteAuditEventKind::ZoneLinkSessionFailed,
            ZoneRouteAuditEventKind::ZoneLinkIntentQueued,
            ZoneRouteAuditEventKind::ZoneLinkShortcutAuthorized,
            ZoneRouteAuditEventKind::ZoneLinkShortcutTornDown,
            ZoneRouteAuditEventKind::ZoneLinkRevoked,
            ZoneRouteAuditEventKind::ZoneLinkRelayAdmitted,
            ZoneRouteAuditEventKind::ZoneLinkRelayDenied,
        ];
        let mut labels = Vec::new();
        for kind in kinds {
            assert!(kind.label().starts_with("zone-"));
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.label())
            );
            labels.push(kind.label());
        }
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), kinds.len());
    }

    #[test]
    fn zone_path_bounds_and_ancestry_are_exact() {
        assert_eq!(
            ZonePath::new(Vec::new()),
            Err(PrimitiveSpecError::InvalidPath)
        );
        let too_many = (0..MAX_ZONE_PATH_LABELS + 1)
            .map(|index| ZoneLabelId::parse(format!("z{index}")).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            ZonePath::new(too_many),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        let long = (0..5)
            .map(|index| ZoneLabelId::parse(format!("{}{index}", "z".repeat(62))).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ZonePath::new(long), Err(PrimitiveSpecError::InvalidPath));

        let root = zone(&["k0"]);
        let child = zone(&["k1", "k0"]);
        let grandchild = zone(&["k2", "k1", "k0"]);
        let sibling = zone(&["k9", "k0"]);
        assert_eq!(child.to_storage_string(), "k0/k1");
        assert!(child.is_direct_child_of(&root));
        assert!(grandchild.is_descendant_of(&root));
        assert!(!grandchild.is_direct_child_of(&root));
        assert!(!sibling.is_descendant_of(&child));
        assert!(!root.is_descendant_of(&root));
        assert_eq!(
            grandchild
                .next_hop_label_below(&root)
                .map(ZoneLabelId::as_str),
            Some("k1")
        );
        assert_eq!(
            grandchild
                .next_hop_label_below(&child)
                .map(ZoneLabelId::as_str),
            Some("k2")
        );
        assert!(root.next_hop_label_below(&grandchild).is_none());
        assert!(sibling.next_hop_label_below(&child).is_none());
        assert_eq!(
            ZonePath::local_root().to_storage_string(),
            LOCAL_ROOT_ZONE_NAME
        );

        // Every ancestor of a path is exactly one prefix-free suffix chain.
        for depth in 1..=MAX_ZONE_PATH_LABELS {
            let labels = (0..depth)
                .map(|index| ZoneLabelId::parse(format!("z{index}")).unwrap())
                .collect::<Vec<_>>();
            let path = ZonePath::new(labels.clone()).unwrap();
            for ancestor_depth in 1..depth {
                let ancestor = ZonePath::new(labels[depth - ancestor_depth..].to_vec()).unwrap();
                assert!(path.is_descendant_of(&ancestor));
                assert!(path.next_hop_label_below(&ancestor).is_some());
            }
        }
    }

    #[test]
    fn advertisement_invariants_fail_closed() {
        let advertiser = zone(&["k1", "k0"]);
        let edge = ZoneTreeEdge::new(zone(&["k0"]), advertiser.clone()).unwrap();
        let generation = ZoneLinkControllerGeneration::parse("gen-1").unwrap();
        let route = |id: &str, descendant: ZonePath, next_hop: &str| {
            ZoneDescendantRoute::new(
                ZoneRouteId::parse(id).unwrap(),
                descendant,
                ZoneLabelId::parse(next_hop).unwrap(),
                capabilities(&["get"]),
            )
        };
        let build = |routes: Vec<ZoneDescendantRoute>, issued: u64, expires: u64, version: u32| {
            ZoneLinkRouteAdvertisement::new(
                version,
                advertiser.clone(),
                edge.clone(),
                generation.clone(),
                routes,
                issued,
                expires,
                signature(),
            )
        };
        let good = || route("route-1", zone(&["k2", "k1", "k0"]), "k2");

        assert_eq!(
            build(vec![good()], 1_000, 4_600, 2),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert_eq!(
            build(Vec::new(), 1_000, 4_600, ZONE_ROUTING_SCHEMA_VERSION),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            build(
                (0..MAX_ADVERTISED_ZONE_ROUTES + 1)
                    .map(|index| route(&format!("route-{index}"), zone(&["k2", "k1", "k0"]), "k2"))
                    .collect(),
                1_000,
                4_600,
                ZONE_ROUTING_SCHEMA_VERSION,
            ),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        assert_eq!(
            build(
                vec![good(), good()],
                1_000,
                4_600,
                ZONE_ROUTING_SCHEMA_VERSION
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
        assert_eq!(
            build(vec![good()], 4_600, 1_000, ZONE_ROUTING_SCHEMA_VERSION),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            build(vec![good()], 1_000, 1_000, ZONE_ROUTING_SCHEMA_VERSION),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            build(
                vec![good()],
                0,
                MAX_ZONE_ADVERTISEMENT_LIFETIME_SECONDS + 1,
                ZONE_ROUTING_SCHEMA_VERSION,
            ),
            Err(PrimitiveSpecError::OutOfRange)
        );
        // A sibling, an ancestor, and the advertiser itself are all rejected.
        for descendant in [zone(&["k9", "k0"]), zone(&["k0"]), advertiser.clone()] {
            assert_eq!(
                build(
                    vec![route("route-1", descendant, "k2")],
                    1_000,
                    4_600,
                    ZONE_ROUTING_SCHEMA_VERSION
                ),
                Err(PrimitiveSpecError::ConflictingFields)
            );
        }
        // A next-hop label that is not the immediate child is rejected.
        assert_eq!(
            build(
                vec![route("route-1", zone(&["k3", "k2", "k1", "k0"]), "k3")],
                1_000,
                4_600,
                ZONE_ROUTING_SCHEMA_VERSION,
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert!(
            build(
                vec![route("route-1", zone(&["k3", "k2", "k1", "k0"]), "k2")],
                1_000,
                4_600,
                ZONE_ROUTING_SCHEMA_VERSION
            )
            .is_ok()
        );
        // An advertisement whose edge child is not the advertiser is rejected.
        assert_eq!(
            ZoneLinkRouteAdvertisement::new(
                ZONE_ROUTING_SCHEMA_VERSION,
                zone(&["k9", "k0"]),
                edge.clone(),
                generation.clone(),
                vec![good()],
                1_000,
                4_600,
                signature(),
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
    }

    #[test]
    fn allocated_capability_scope_narrows_monotonically_and_stays_private() {
        let advert = advertisement();
        assert_eq!(
            advert
                .clone()
                .with_allocated_capability_scope(capabilities(&["get"]))
                .err(),
            Some(PrimitiveSpecError::ConflictingFields)
        );
        let scoped = advert
            .clone()
            .with_allocated_capability_scope(capabilities(&["get", "list", "watch"]))
            .expect("advertised routes narrow within the allocation");
        assert_eq!(
            scoped
                .allocated_capability_scope()
                .map(ZoneRouteCapabilitySet::capabilities),
            Some(capabilities(&["get", "list", "watch"]).capabilities())
        );
        // The private scope never reaches the wire.
        let rendered = canonical_json_bytes(&scoped).unwrap();
        assert_eq!(rendered, canonical_json_bytes(&advert).unwrap());
        assert!(!String::from_utf8(rendered).unwrap().contains("watch"));

        // Subset ordering is exact in both directions.
        let narrow = capabilities(&["get"]);
        let wide = capabilities(&["get", "list"]);
        assert!(narrow.is_subset_of(&wide));
        assert!(!wide.is_subset_of(&narrow));
        assert!(ZoneRouteCapabilitySet::default().is_subset_of(&narrow));
        assert!(narrow.is_subset_of(&narrow));
        assert_eq!(
            ZoneRouteCapabilitySet::new(vec![
                ZoneRouteCapability::parse("get").unwrap(),
                ZoneRouteCapability::parse("get").unwrap(),
            ]),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
        assert_eq!(
            ZoneRouteCapabilitySet::new(
                (0..MAX_ZONE_ROUTE_CAPABILITIES + 1)
                    .map(|index| ZoneRouteCapability::parse(format!("cap{index}")).unwrap())
                    .collect()
            ),
            Err(PrimitiveSpecError::TooManyEntries)
        );
    }

    #[test]
    fn withdrawal_and_namespace_allocation_bounds_fail_closed() {
        let generation = ZoneLinkControllerGeneration::parse("gen-1").unwrap();
        let route_id = ZoneRouteId::parse("route-1").unwrap();
        assert_eq!(
            ZoneLinkRouteWithdrawal::new(
                ZONE_ROUTING_SCHEMA_VERSION,
                zone(&["k1", "k0"]),
                generation.clone(),
                Vec::new(),
                1_000,
                signature(),
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            ZoneLinkRouteWithdrawal::new(
                ZONE_ROUTING_SCHEMA_VERSION,
                zone(&["k1", "k0"]),
                generation.clone(),
                vec![route_id.clone(), route_id.clone()],
                1_000,
                signature(),
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
        let withdrawal = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            zone(&["k1", "k0"]),
            generation.clone(),
            vec![route_id.clone()],
            1_000,
            signature(),
        )
        .unwrap();
        assert_eq!(
            canonical_json_bytes(&withdrawal).unwrap(),
            br#"{"advertisingZone":["k1","k0"],"controllerGeneration":"gen-1","issuedAtUnixSeconds":1000,"schemaVersion":3,"signature":{"algorithm":"ed25519-blake3","keyRole":"zone-controller-routing","signatureRef":"sigref-1","signingKeyFingerprint":"sha256.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"withdrawnRouteIds":["route-1"]}"#
        );

        let edge = ZoneTreeEdge::new(zone(&["k0"]), zone(&["k1", "k0"])).unwrap();
        assert_eq!(
            ZoneLinkNamespaceAllocation::new(
                edge.clone(),
                generation.clone(),
                Vec::new(),
                1,
                capabilities(&["get"]),
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            ZoneLinkNamespaceAllocation::new(
                edge.clone(),
                generation.clone(),
                (0..MAX_ZONE_ROUTE_NAMESPACE_PREFIXES + 1)
                    .map(|_| zone(&["k1", "k0"]))
                    .collect(),
                1,
                capabilities(&["get"]),
            ),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        for max_routes in [0, MAX_ADVERTISED_ZONE_ROUTES as u32 + 1] {
            assert_eq!(
                ZoneLinkNamespaceAllocation::new(
                    edge.clone(),
                    generation.clone(),
                    vec![zone(&["k1", "k0"])],
                    max_routes,
                    capabilities(&["get"]),
                ),
                Err(PrimitiveSpecError::OutOfRange)
            );
        }
        // A sibling or ancestor prefix is outside the delegated namespace.
        for prefix in [zone(&["k9", "k0"]), zone(&["k0"])] {
            assert_eq!(
                ZoneLinkNamespaceAllocation::new(
                    edge.clone(),
                    generation.clone(),
                    vec![prefix],
                    1,
                    capabilities(&["get"]),
                ),
                Err(PrimitiveSpecError::ConflictingFields)
            );
        }
        let allocation = ZoneLinkNamespaceAllocation::new(
            edge,
            generation,
            vec![zone(&["k1", "k0"])],
            MAX_ADVERTISED_ZONE_ROUTES as u32,
            capabilities(&["get", "list"]),
        )
        .unwrap();
        assert!(allocation.admits_prefix(&zone(&["k1", "k0"])));
        assert!(allocation.admits_prefix(&zone(&["k2", "k1", "k0"])));
        assert!(!allocation.admits_prefix(&zone(&["k9", "k0"])));
        assert!(!allocation.admits_prefix(&zone(&["k0"])));
    }

    #[test]
    fn route_path_hop_bounds_and_contiguity_fail_closed() {
        let root = zone(&["k0"]);
        let child = zone(&["k1", "k0"]);
        let grandchild = zone(&["k2", "k1", "k0"]);
        let up = ZoneRouteHop::new(
            child.clone(),
            root.clone(),
            ZoneTreeEdge::new(root.clone(), child.clone()).unwrap(),
            ZoneRouteHopDirection::UpToParent,
            None,
        )
        .unwrap();
        let down = ZoneRouteHop::new(
            child.clone(),
            grandchild.clone(),
            ZoneTreeEdge::new(child.clone(), grandchild.clone()).unwrap(),
            ZoneRouteHopDirection::DownToChild,
            Some(ZoneRouteId::parse("route-1").unwrap()),
        )
        .unwrap();

        // A hop whose endpoints contradict its direction is rejected.
        assert_eq!(
            ZoneRouteHop::new(
                root.clone(),
                child.clone(),
                ZoneTreeEdge::new(root.clone(), child.clone()).unwrap(),
                ZoneRouteHopDirection::UpToParent,
                None,
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        // A non-contiguous hop list is rejected.
        assert_eq!(
            ZoneRoutePath::new(
                child.clone(),
                grandchild.clone(),
                root.clone(),
                vec![up.clone(), down.clone()],
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        // A path whose endpoint is not below the ancestor is rejected.
        assert_eq!(
            ZoneRoutePath::new(
                child.clone(),
                grandchild.clone(),
                zone(&["k9", "k0"]),
                Vec::new()
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        // The hop ceiling is exact.
        let over_bound = (0..MAX_ZONE_ROUTE_PATH_HOPS + 1)
            .map(|_| up.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ZoneRoutePath::new(child.clone(), root.clone(), root.clone(), over_bound),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        assert!(
            ZoneRoutePath::new(
                child.clone(),
                grandchild.clone(),
                child.clone(),
                vec![down.clone()]
            )
            .is_ok()
        );
        assert_eq!(ZONE_ROUTE_INITIAL_HOP_BUDGET, 16);
        assert!((ZONE_ROUTE_INITIAL_HOP_BUDGET as usize) <= MAX_ZONE_ROUTE_PATH_HOPS);
    }

    #[test]
    fn opaque_routing_tokens_reject_secret_and_path_shapes() {
        assert!(ZoneRouteId::parse("route-1.a_b").is_ok());
        for rejected in [
            "",
            "-leading",
            "../escape",
            "a..b",
            "session-token-1",
            "my-credential",
            "bearer1",
            "private-key",
            "unix-socketpath",
            "relay-endpoint",
            "has space",
            "has/slash",
        ] {
            assert_eq!(
                ZoneRouteId::parse(rejected),
                Err(PrimitiveSpecError::InvalidToken),
                "{rejected}"
            );
        }
        assert_eq!(
            ZoneRouteId::parse("a".repeat(MAX_ZONE_ROUTE_TOKEN_BYTES + 1)),
            Err(PrimitiveSpecError::InvalidToken)
        );
        assert!(ZoneRouteId::parse("a".repeat(MAX_ZONE_ROUTE_TOKEN_BYTES)).is_ok());
        assert_eq!(
            ZoneLabelId::parse("K0"),
            Err(PrimitiveSpecError::InvalidToken)
        );
    }

    #[test]
    fn wire_decoding_is_fail_closed_and_rejects_unknown_fields() {
        // A structurally invalid advertisement cannot be decoded.
        let bytes = canonical_json_bytes(&advertisement()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let tampered = text.replace(r#""nextHopChild":"k2""#, r#""nextHopChild":"k9""#);
        assert!(serde_json::from_str::<ZoneLinkRouteAdvertisement>(&tampered).is_err());
        let downgraded = text.replace(r#""schemaVersion":3"#, r#""schemaVersion":2"#);
        assert!(serde_json::from_str::<ZoneLinkRouteAdvertisement>(&downgraded).is_err());
        let extra = text.replace(r#""schemaVersion":3"#, r#""schemaVersion":3,"extra":1"#);
        assert!(serde_json::from_str::<ZoneLinkRouteAdvertisement>(&extra).is_err());

        assert!(serde_json::from_str::<ZonePath>(r#"[]"#).is_err());
        assert!(serde_json::from_str::<ZonePath>(r#"["K0"]"#).is_err());
        assert!(serde_json::from_str::<ZonePath>(r#"["k0"]"#).is_ok());
        assert!(
            serde_json::from_str::<ZoneTreeEdge>(r#"{"parent":["k0"],"child":["k2","k1","k0"]}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<ZoneTreeEdge>(r#"{"parent":["k0"],"child":["k1","k0"]}"#)
                .is_ok()
        );
        assert!(serde_json::from_str::<ZoneRouteCapabilitySet>(r#"["get","get"]"#).is_err());
    }

    #[test]
    fn diagnostics_never_echo_a_caller_supplied_marker() {
        let marker = format!("marker{:x}", std::process::id());
        let label = ZoneLabelId::parse(marker.clone()).unwrap();
        let path = ZonePath::new(vec![label.clone()]).unwrap();
        let route_id = ZoneRouteId::parse(marker.clone()).unwrap();
        let generation = ZoneLinkControllerGeneration::parse(marker.clone()).unwrap();
        let capability = ZoneRouteCapability::parse(marker.clone()).unwrap();
        let capability_set = ZoneRouteCapabilitySet::new(vec![capability.clone()]).unwrap();
        let edge = ZoneTreeEdge::new(
            zone(&["k0"]),
            ZonePath::new(vec![label.clone(), ZoneLabelId::parse("k0").unwrap()]).unwrap(),
        )
        .unwrap();
        let route = ZoneDescendantRoute::new(
            route_id.clone(),
            zone(&["k2", "k1", "k0"]),
            ZoneLabelId::parse("k2").unwrap(),
            capability_set.clone(),
        );
        let hop = ZoneRouteHop::new(
            edge.child().clone(),
            edge.parent().clone(),
            edge.clone(),
            ZoneRouteHopDirection::UpToParent,
            Some(route_id.clone()),
        )
        .unwrap();
        let route_path = ZoneRoutePath::new(
            edge.child().clone(),
            edge.parent().clone(),
            edge.parent().clone(),
            vec![hop.clone()],
        )
        .unwrap();
        let allocation = ZoneLinkNamespaceAllocation::new(
            edge.clone(),
            generation.clone(),
            vec![edge.child().clone()],
            1,
            capability_set.clone(),
        )
        .unwrap();
        let withdrawal = ZoneLinkRouteWithdrawal::new(
            ZONE_ROUTING_SCHEMA_VERSION,
            edge.child().clone(),
            generation.clone(),
            vec![route_id.clone()],
            1_000,
            signature(),
        )
        .unwrap();
        let advert = advertisement()
            .with_allocated_capability_scope(capabilities(&["get", "list"]))
            .unwrap();

        for rendered in [
            format!("{label:?}"),
            format!("{path:?}"),
            format!("{route_id:?}"),
            format!("{generation:?}"),
            format!("{capability:?}"),
            format!("{capability_set:?}"),
            format!("{edge:?}"),
            format!("{route:?}"),
            format!("{hop:?}"),
            format!("{route_path:?}"),
            format!("{allocation:?}"),
            format!("{withdrawal:?}"),
            format!("{advert:?}"),
            format!("{:?}", signature()),
        ] {
            assert!(!rendered.contains(&marker), "{rendered}");
            assert!(!rendered.contains('/'), "{rendered}");
            assert!(rendered.contains("<redacted>"), "{rendered}");
        }
        // The authorized rendering surfaces still return the exact values.
        assert_eq!(label.as_str(), marker);
        assert_eq!(path.to_storage_string(), marker);
        assert_eq!(route_id.as_str(), marker);
    }
}
