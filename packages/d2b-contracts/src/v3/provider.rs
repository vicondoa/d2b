//! The Provider resource contract.
//!
//! `Provider` is the ResourceType a Zone installs so that a `providerRef`
//! resolves. Package presence alone is not installation: only a Ready
//! `Provider/<name>` row in the same Zone is selectable.
//!
//! The authored surface is deliberately tiny. Per
//! `ADR-046-provider-model-and-packaging` section "Provider resource", the
//! Provider ResourceSpec is exactly two fields, `artifactId` and `config`.
//! Every other Provider property - digests, publisher and trust identity,
//! exported ResourceTypes, component descriptors, dependency aliases, the
//! standard capability matrix, the registered extension schemas, the
//! export/import projection factories, and the upgrade policy - is read-only
//! derived data resolved from the signed manifest and catalog entry the
//! `artifactId` selects. Those live here as [`ProviderManifest`] and its
//! parts, never as authored spec fields.
//!
//! Redaction. Nothing in this module renders a caller-supplied value through
//! `Debug`. A publisher name, an artifact identifier, a digest, a component
//! identifier, and a config object are all comparison material, not log
//! material, so every wrapper renders `<redacted>` and the composite
//! descriptors render only closed discriminants and counts. No type here
//! carries a store path, package path, executable name, socket path,
//! numeric UID or GID, credential, or file descriptor: the specification
//! keeps the private catalog's Nix store path out of resource spec, status,
//! and audit, and this contract has no field that could hold one.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    execution_policy::{
        BoundedToken, ExecutionDomain, PrimitiveSpecError, redacted_debug, string_schema,
    },
    identity::{ResourceTypeName, SchemaFingerprint},
    resource_ref::ResourceRef,
    resource_schema::{CanonicalJsonObject, ExtensionSchemaId, SchemaVersion},
    volume::{MAX_VIEWS, ViewRight, ViewSpec},
    volume_state::{
        MigrationPolicy, PersistenceClass, SensitivityClass, VolumeStateSchema, VolumeStateSchemaId,
    },
};

/// The canonical ResourceType name for this module.
pub const PROVIDER_RESOURCE_TYPE: &str = "Provider";

/// Maximum bytes in one artifact identifier.
pub const MAX_ARTIFACT_ID_BYTES: usize = 63;

/// Maximum bytes in one publisher identifier.
pub const MAX_PUBLISHER_ID_BYTES: usize = 63;

/// Maximum component descriptors one Provider manifest declares.
pub const MAX_PROVIDER_COMPONENTS: usize = 32;

/// Maximum ResourceApiBindings one Provider manifest declares.
pub const MAX_PROVIDER_API_BINDINGS: usize = 16;

/// Maximum ResourceTypes one controller component owns.
pub const MAX_COMPONENT_RESOURCE_TYPES: usize = 8;

/// Maximum methods one component exports.
pub const MAX_COMPONENT_METHODS: usize = 32;

/// Maximum entries in one signed standard capability matrix.
pub const MAX_CAPABILITY_MATRIX_ENTRIES: usize = 32;

/// Maximum projection factories one Provider manifest advertises.
pub const MAX_PROJECTION_FACTORIES: usize = 8;

/// Maximum backing or target ResourceTypes one projection factory allows.
pub const MAX_PROJECTION_REF_TYPES: usize = 8;

/// The provider-neutral result a Provider returns when it refuses an
/// optional base capability its signed matrix marks unsupported.
///
/// A Provider never ignores, reinterprets, renames, duplicates, or weakens a
/// base field; refusing an optional capability through this exact code is
/// its only permitted escape.
pub const UNSUPPORTED_CAPABILITY_CODE: &str = "unsupported-capability";

/// Reason a Provider contract value could not be constructed or admitted.
///
/// Every variant is a closed reason. None carries a publisher, artifact
/// identifier, digest, path, or caller-supplied value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderContractError {
    /// A validated scalar failed its own grammar or bound. The scalar itself
    /// is caller-supplied, so the reason names the class and not the value.
    InvalidPrimitive,
    /// A declared collection bound was exceeded, or a required non-empty
    /// collection was empty.
    BoundExceeded,
    /// A required field was absent for the selected discriminant.
    MissingRequiredField,
    /// Two fields that cannot both be set for the selected discriminant
    /// were both set.
    ConflictingFields,
    /// The same identifier was declared twice.
    DuplicateDeclaration,
    /// The reference does not name the expected ResourceType.
    WrongResourceType,
    /// Trust admission is not satisfied, so the artifact is not admissible.
    TrustNotEstablished,
    /// The requested Provider API major version is not this artifact's.
    ApiMajorMismatch,
    /// The requested Provider API minor version is newer than this
    /// artifact's, and there is no handshake downgrade.
    ApiMinorTooNew,
    /// The selected descriptor fingerprint differs from the advertisement.
    DescriptorFingerprintMismatch,
    /// The state schema version is not compatible without a migration.
    StateSchemaIncompatible,
    /// A projection factory is absent, mismatched, or inconsistent with the
    /// capability it claims to export.
    ProjectionFactoryInvalid,
    /// A cross-Zone export was requested for a capability whose
    /// exportability is forbidden.
    ExportForbidden,
    /// A component state declaration has no qualifying storage need.
    ComponentStateNotJustified,
    /// A component state declaration is not a state Volume declaration.
    ComponentKindInvalid,
    /// A component state declaration uses a forbidden persistence class.
    ComponentPersistenceClassForbidden,
    /// A component state declaration's byte quota is below the minimum.
    ComponentQuotaTooSmall,
    /// Host custody was not explicitly permitted for host-backed Guest state.
    PlacementHostCustodyViolation,
    /// The schema category requires Guest-local custody.
    GuestLocalRequired,
}

impl core::fmt::Display for ProviderContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ProviderContractError {}

impl From<PrimitiveSpecError> for ProviderContractError {
    fn from(_error: PrimitiveSpecError) -> Self {
        Self::InvalidPrimitive
    }
}

impl ProviderContractError {
    /// The stable lower-kebab code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidPrimitive => "provider-primitive-invalid",
            Self::BoundExceeded => "provider-bound-exceeded",
            Self::MissingRequiredField => "provider-missing-required-field",
            Self::ConflictingFields => "provider-conflicting-fields",
            Self::DuplicateDeclaration => "provider-duplicate-declaration",
            Self::WrongResourceType => "provider-wrong-resource-type",
            Self::TrustNotEstablished => "provider-trust-not-established",
            Self::ApiMajorMismatch => "provider-api-major-mismatch",
            Self::ApiMinorTooNew => "provider-api-minor-too-new",
            Self::DescriptorFingerprintMismatch => "provider-descriptor-fingerprint-mismatch",
            Self::StateSchemaIncompatible => "provider-state-schema-incompatible",
            Self::ProjectionFactoryInvalid => "provider-projection-factory-invalid",
            Self::ExportForbidden => "provider-export-forbidden",
            Self::ComponentStateNotJustified => "component-state-not-justified",
            Self::ComponentKindInvalid => "component-kind-invalid",
            Self::ComponentPersistenceClassForbidden => "component-persistence-class-forbidden",
            Self::ComponentQuotaTooSmall => "component-quota-too-small",
            Self::PlacementHostCustodyViolation => "placement-host-custody-violation",
            Self::GuestLocalRequired => "guest-local-required",
        }
    }
}

/// A plain bounded artifact identifier.
///
/// The specification is explicit that an artifact is not a ResourceType and
/// that `artifactId` is a plain bounded ID rather than a ResourceRef, so
/// this type deliberately cannot be parsed from or converted into one.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ArtifactId(BoundedToken);

impl ArtifactId {
    /// Parse a `^[a-z][a-z0-9-]*$` artifact identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, ProviderContractError> {
        let value = value.into();
        if value.len() > MAX_ARTIFACT_ID_BYTES {
            return Err(ProviderContractError::InvalidPrimitive);
        }
        Ok(Self(BoundedToken::parse(value)?))
    }

    /// Borrow the canonical identifier.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(ArtifactId);
string_schema!(ArtifactId, 1, MAX_ARTIFACT_ID_BYTES);

impl<'de> Deserialize<'de> for ArtifactId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A validated `sha256:<64 lower-case hex>` artifact digest.
///
/// Selection is exact digest. There is no runtime marketplace, download,
/// PATH scan, directory discovery, latest, or version-range solving, so a
/// digest is the only thing that selects an artifact.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ArtifactDigest(String);

impl ArtifactDigest {
    /// Parse exactly `sha256:` followed by 64 lower-case hex digits.
    pub fn parse(value: impl Into<String>) -> Result<Self, ProviderContractError> {
        let value = value.into();
        let hex = value
            .strip_prefix("sha256:")
            .ok_or(ProviderContractError::InvalidPrimitive)?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ProviderContractError::InvalidPrimitive);
        }
        Ok(Self(value))
    }

    /// Borrow the canonical digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(ArtifactDigest);
string_schema!(ArtifactDigest, 71, 71);

impl<'de> Deserialize<'de> for ArtifactDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// The Provider ResourceType base spec: exactly two authored fields.
///
/// Adding a third field here would restate manifest-derived data in the
/// resource row, which the specification forbids.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpec {
    artifact_id: ArtifactId,
    config: CanonicalJsonObject,
}

impl ProviderSpec {
    /// Construct a Provider base spec.
    pub const fn new(artifact_id: ArtifactId, config: CanonicalJsonObject) -> Self {
        Self {
            artifact_id,
            config,
        }
    }

    /// Construct the canonical minimal Provider base spec: an artifact
    /// selector and an empty root configuration.
    pub fn minimal(artifact_id: ArtifactId) -> Self {
        Self::new(artifact_id, CanonicalJsonObject::default())
    }

    /// Borrow the artifact selector.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Borrow the root configuration object.
    ///
    /// The object is validated against the manifest's signed root JSON
    /// Schema before launch; this contract only pins its canonical shape.
    pub const fn config(&self) -> &CanonicalJsonObject {
        &self.config
    }
}

redacted_debug!(ProviderSpec);

impl<'de> Deserialize<'de> for ProviderSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            artifact_id: ArtifactId,
            #[serde(default)]
            config: CanonicalJsonObject,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(wire.artifact_id, wire.config))
    }
}

/// Whether a signature over the artifact verified.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SignatureState {
    /// The signature verified against a trusted publisher at the epoch.
    Valid,
    /// The signature did not verify.
    Invalid,
    /// No signature was presented.
    Absent,
}

/// Whether the artifact or its signing key is revoked.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RevocationState {
    /// Revocation data was fetched and this artifact is not revoked.
    Clear,
    /// This artifact or its key is revoked.
    Revoked,
    /// Revocation state could not be established.
    Unknown,
}

/// The outcome of one policy evaluation over the artifact's evidence.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyEvaluation {
    /// Evidence was present and the policy accepted it.
    Accepted,
    /// Evidence was present and the policy rejected it.
    Rejected,
    /// No evidence was evaluated. This is not an acceptance.
    Unevaluated,
}

impl PolicyEvaluation {
    /// Whether the evaluation admits the artifact. Only an explicit
    /// acceptance does; an unevaluated policy fails closed.
    pub const fn admits(self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// The exact digests a catalog entry pins for one artifact.
///
/// The package, executable, manifest, config, schema, and service digests
/// are separate because a Provider may ship several component binaries and
/// a change to any one of them is a different artifact.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactDigestSet {
    /// The signed package digest.
    pub package: ArtifactDigest,
    /// The component executable set digest.
    pub executable: ArtifactDigest,
    /// The signed manifest digest.
    pub manifest: ArtifactDigest,
    /// The root config projection digest.
    pub config: ArtifactDigest,
    /// The exported schema set digest.
    pub schema: ArtifactDigest,
    /// The exported service surface digest.
    pub service: ArtifactDigest,
}

redacted_debug!(ArtifactDigestSet);

/// Everything production admission requires before a Provider artifact is
/// installable.
///
/// [`TrustEvidence::admit`] is the whole rule and it is fail-closed: a
/// missing signature, an unknown revocation state, an unevaluated policy, or
/// an emergency deny all refuse. First- and third-party Providers use the
/// same admission, and admission never relaxes a runtime restriction.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustEvidence {
    /// The publisher identity the signature binds to.
    pub publisher: BoundedToken,
    /// The trust root epoch the signature was verified against.
    pub root_epoch: u32,
    /// Whether the publisher is in the Zone's trusted publisher set at that
    /// epoch.
    pub publisher_trusted: bool,
    /// Whether the signature verified.
    pub signature: SignatureState,
    /// Whether the artifact or its key is revoked.
    pub revocation: RevocationState,
    /// Whether an emergency deny names this artifact.
    pub emergency_deny: bool,
    /// The provenance policy outcome.
    pub provenance: PolicyEvaluation,
    /// The software bill of materials policy outcome.
    pub sbom: PolicyEvaluation,
    /// The license policy outcome.
    pub license: PolicyEvaluation,
    /// The vulnerability policy outcome.
    pub vulnerability: PolicyEvaluation,
    /// The exact package and API conformance attestation outcome.
    pub conformance: PolicyEvaluation,
    /// The support channel the artifact is published on.
    pub support_channel: BoundedToken,
}

redacted_debug!(TrustEvidence);

impl TrustEvidence {
    /// Decide production admission, fail-closed.
    pub fn admit(&self) -> Result<(), ProviderContractError> {
        let admitted = self.publisher_trusted
            && self.signature == SignatureState::Valid
            && self.revocation == RevocationState::Clear
            && !self.emergency_deny
            && self.provenance.admits()
            && self.sbom.admits()
            && self.license.admits()
            && self.vulnerability.admits()
            && self.conformance.admits();
        if admitted {
            Ok(())
        } else {
            Err(ProviderContractError::TrustNotEstablished)
        }
    }
}

/// The Provider API and state-schema compatibility the artifact advertises.
///
/// The rules the specification freezes are: the API major is exact, minor is
/// additive only, the exact descriptor fingerprint is selected before
/// launch, there is no handshake downgrade or fallback, and state schema
/// compatibility is checked independently of the API version.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompatibilityRange {
    /// The Provider API major version this artifact implements.
    pub api_major: u32,
    /// The Provider API minor version this artifact implements.
    pub api_minor: u32,
    /// The exact descriptor fingerprint selected before launch.
    pub descriptor_fingerprint: SchemaFingerprint,
    /// The state schema version this artifact reads and writes.
    pub state_schema_version: SchemaVersion,
}

redacted_debug!(CompatibilityRange);

impl CompatibilityRange {
    /// Decide whether this artifact satisfies a caller's exact requirement.
    ///
    /// There is no negotiation: a differing major, a newer requested minor,
    /// or a differing descriptor fingerprint all refuse rather than
    /// downgrade.
    pub fn admits(
        &self,
        required_major: u32,
        required_minor: u32,
        required_fingerprint: &SchemaFingerprint,
    ) -> Result<(), ProviderContractError> {
        if self.api_major != required_major {
            return Err(ProviderContractError::ApiMajorMismatch);
        }
        if required_minor > self.api_minor {
            return Err(ProviderContractError::ApiMinorTooNew);
        }
        if &self.descriptor_fingerprint != required_fingerprint {
            return Err(ProviderContractError::DescriptorFingerprintMismatch);
        }
        Ok(())
    }

    /// Decide whether installed state written at `installed` can be read by
    /// this artifact without a migration.
    ///
    /// State compatibility is checked independently of the API version: the
    /// state schema major must be exact and the installed minor must not be
    /// newer than this artifact's.
    pub fn admits_state(&self, installed: SchemaVersion) -> Result<(), ProviderContractError> {
        let (installed_major, installed_minor) = schema_version_parts(installed);
        let (artifact_major, artifact_minor) = schema_version_parts(self.state_schema_version);
        if installed_major != artifact_major || installed_minor > artifact_minor {
            return Err(ProviderContractError::StateSchemaIncompatible);
        }
        Ok(())
    }
}

/// Split a canonical `MAJOR.MINOR` schema version into its two components.
///
/// `SchemaVersion` exposes no component accessor, and its canonical string
/// is the contract's own round-trip spelling, so parsing that spelling back
/// is exact rather than lossy.
fn schema_version_parts(version: SchemaVersion) -> (u32, u32) {
    let rendered = version.to_canonical_string();
    let (major, minor) = rendered
        .split_once('.')
        .expect("a canonical schema version always carries one separator");
    (
        major
            .parse()
            .expect("a canonical schema version major is numeric"),
        minor
            .parse()
            .expect("a canonical schema version minor is numeric"),
    )
}

/// The closed Provider component type set.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentType {
    /// Owns one or more ResourceTypes and an async reconcile loop.
    Controller,
    /// Serves typed runtime or internal ComponentSession methods, and owns
    /// no ResourceType.
    Service,
    /// A narrow Process or EphemeralProcess with no ResourceClient, bus or
    /// dependency portal, Credential, CLI, broker, or child-spawn
    /// authority. Everything it needs is inherited through its
    /// LaunchTicket.
    Worker,
}

/// The closed dependency alias set a manifest may declare.
///
/// Zone config binds each alias to an exact Provider ResourceRef and service
/// fingerprint. A component asks for an alias and never receives a global
/// registry, a route table, or an arbitrary Provider endpoint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyAlias {
    /// The runtime execution target Provider.
    Runtime,
    /// The Volume Provider.
    Volume,
    /// The Network Provider.
    Network,
    /// The Credential Provider.
    Credential,
    /// The Transport carriage Provider.
    Transport,
}

impl DependencyAlias {
    /// Every declared alias, in a deterministic order.
    pub const ALL: [Self; 5] = [
        Self::Runtime,
        Self::Volume,
        Self::Network,
        Self::Credential,
        Self::Transport,
    ];

    /// The stable lower-kebab alias token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Volume => "volume",
            Self::Network => "network",
            Self::Credential => "credential",
            Self::Transport => "transport",
        }
    }
}

/// Minimum byte quota for a declared component state namespace.
pub const MIN_COMPONENT_STATE_QUOTA_BYTES: u64 = 4_096;

/// The component-Volume kind carried by a namespace declaration.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ComponentStateKind {
    /// A long-lived component payload Volume.
    State,
    /// A migration staging Volume. Component descriptors cannot declare one.
    Staging,
}

/// Why a payload cannot use resource status or the core Operation ledger.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum StorageNeed {
    /// Secret or sensitive private recovery data.
    Secret,
    /// Large binary or file content.
    LargeBinary,
    /// Private content unsafe for authorized status readers.
    PrivateUnsafeForStatus,
    /// Bounded content whose churn is unsuitable for revision history.
    RevisionUnsuitable,
}

/// Signed placement of state for a component executing under a Guest.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum StatePlacementMode {
    /// The Volume source and controller both live inside the Guest.
    GuestLocal,
    /// The source lives on a Host and Guest access uses Export children.
    HostBackedGuest,
}

/// Trusted schema classification used by ProviderDeployment custody checks.
///
/// The state schema ID alone does not encode this category. The signed schema
/// registry supplies it when admitting the namespace.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum StateSchemaCustodyClass {
    Ordinary,
    Credential,
    Audit,
    RemoteNode,
    CloudControl,
}

impl StateSchemaCustodyClass {
    const fn requires_guest_local(self) -> bool {
        !matches!(self, Self::Ordinary)
    }
}

/// Projection facts ProviderDeployment must copy into a generated state
/// Volume and its Export children.
#[derive(Clone, PartialEq, Eq)]
pub struct ComponentStateVolumeProjection {
    source_execution_ref: ResourceRef,
    quota_max_bytes: u64,
    quota_max_inodes: u64,
    export_count: usize,
}

impl ComponentStateVolumeProjection {
    /// Borrow the Host or Guest that owns the source Volume bytes.
    pub const fn source_execution_ref(&self) -> &ResourceRef {
        &self.source_execution_ref
    }

    /// Return the Volume `quota.maxBytes` value.
    pub const fn quota_max_bytes(&self) -> u64 {
        self.quota_max_bytes
    }

    /// Return the nonzero Volume `quota.maxInodes` value.
    pub const fn quota_max_inodes(&self) -> u64 {
        self.quota_max_inodes
    }

    /// Return the required Export child count.
    pub const fn export_count(&self) -> usize {
        self.export_count
    }
}

impl core::fmt::Debug for ComponentStateVolumeProjection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ComponentStateVolumeProjection(<redacted>)")
    }
}

/// One named component view copied into its generated state Volume.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStateView {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    rights: Vec<ViewRight>,
}

impl ComponentStateView {
    /// Construct a view after applying the Volume view path and rights rules.
    pub fn new(
        path: impl Into<String>,
        mut rights: Vec<ViewRight>,
    ) -> Result<Self, ProviderContractError> {
        let path = path.into();
        ViewSpec::new(path.clone(), rights.clone())?;
        rights.sort_unstable();
        Ok(Self { path, rights })
    }

    /// Borrow the anchored path relative to the Volume root.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Borrow the granted rights.
    pub fn rights(&self) -> &[ViewRight] {
        &self.rights
    }
}

impl core::fmt::Debug for ComponentStateView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComponentStateView")
            .field("right_count", &self.rights.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ComponentStateView {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            path: String,
            rights: Vec<ViewRight>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.path, wire.rights).map_err(serde::de::Error::custom)
    }
}

/// One state Volume namespace signed into a component descriptor.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStateNamespace {
    id: BoundedToken,
    kind: ComponentStateKind,
    #[serde(flatten)]
    state_schema: VolumeStateSchema,
    persistence_class: PersistenceClass,
    sensitivity_class: SensitivityClass,
    #[schemars(range(min = 4_096, max = 9_223_372_036_854_775_807_u64))]
    quota_bytes: u64,
    storage_need: StorageNeed,
    sealing_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    placement_mode: Option<StatePlacementMode>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    host_custody_permitted: bool,
    views: BTreeMap<String, ComponentStateView>,
}

impl ComponentStateNamespace {
    /// Construct and intrinsically validate a signed state declaration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: BoundedToken,
        kind: ComponentStateKind,
        schema_id: VolumeStateSchemaId,
        schema_version: SchemaVersion,
        schema_digest: SchemaFingerprint,
        persistence_class: PersistenceClass,
        sensitivity_class: SensitivityClass,
        migration_policy: MigrationPolicy,
        quota_bytes: u64,
        storage_need: Option<StorageNeed>,
        sealing_required: bool,
        placement_mode: Option<StatePlacementMode>,
        host_custody_permitted: bool,
        views: BTreeMap<String, ComponentStateView>,
    ) -> Result<Self, ProviderContractError> {
        if kind != ComponentStateKind::State {
            return Err(ProviderContractError::ComponentKindInvalid);
        }
        if persistence_class != PersistenceClass::Persistent {
            return Err(ProviderContractError::ComponentPersistenceClassForbidden);
        }
        if quota_bytes < MIN_COMPONENT_STATE_QUOTA_BYTES {
            return Err(ProviderContractError::ComponentQuotaTooSmall);
        }
        if quota_bytes > i64::MAX as u64 {
            return Err(ProviderContractError::InvalidPrimitive);
        }
        let storage_need = storage_need.ok_or(ProviderContractError::ComponentStateNotJustified)?;
        if views.is_empty() {
            return Err(ProviderContractError::MissingRequiredField);
        }
        if views.len() > MAX_VIEWS {
            return Err(ProviderContractError::BoundExceeded);
        }
        for name in views.keys() {
            BoundedToken::parse(name.clone())?;
        }
        match placement_mode {
            Some(StatePlacementMode::HostBackedGuest) if !host_custody_permitted => {
                return Err(ProviderContractError::PlacementHostCustodyViolation);
            }
            Some(StatePlacementMode::GuestLocal) | None if host_custody_permitted => {
                return Err(ProviderContractError::PlacementHostCustodyViolation);
            }
            _ => {}
        }
        Ok(Self {
            id,
            kind,
            state_schema: VolumeStateSchema::new(
                schema_id,
                schema_version,
                schema_digest,
                migration_policy,
            ),
            persistence_class,
            sensitivity_class,
            quota_bytes,
            storage_need,
            sealing_required,
            placement_mode,
            host_custody_permitted,
            views,
        })
    }

    /// Borrow the component-local namespace ID.
    pub const fn id(&self) -> &BoundedToken {
        &self.id
    }

    /// Return the required state kind.
    pub const fn kind(&self) -> ComponentStateKind {
        self.kind
    }

    /// Borrow the schema declaration copied into the Volume.
    pub const fn state_schema(&self) -> &VolumeStateSchema {
        &self.state_schema
    }

    /// Return the required persistence class.
    pub const fn persistence_class(&self) -> PersistenceClass {
        self.persistence_class
    }

    /// Return the payload visibility class.
    pub const fn sensitivity_class(&self) -> SensitivityClass {
        self.sensitivity_class
    }

    /// Return the byte quota copied to `quota.maxBytes`.
    pub const fn quota_bytes(&self) -> u64 {
        self.quota_bytes
    }

    /// Return the storage-need justification.
    pub const fn storage_need(&self) -> StorageNeed {
        self.storage_need
    }

    /// Whether the state must be sealed before it becomes Ready.
    pub const fn sealing_required(&self) -> bool {
        self.sealing_required
    }

    /// Return the signed Guest placement mode, if this targets a Guest.
    pub const fn placement_mode(&self) -> Option<StatePlacementMode> {
        self.placement_mode
    }

    /// Whether the signed descriptor explicitly permits Host custody.
    pub const fn host_custody_permitted(&self) -> bool {
        self.host_custody_permitted
    }

    /// Borrow the named Volume views.
    pub const fn views(&self) -> &BTreeMap<String, ComponentStateView> {
        &self.views
    }

    /// Reject Host-backed placement for schema classes that require Guest
    /// custody.
    pub fn validate_schema_custody(
        &self,
        class: StateSchemaCustodyClass,
    ) -> Result<(), ProviderContractError> {
        if class.requires_guest_local()
            && self.placement_mode == Some(StatePlacementMode::HostBackedGuest)
        {
            Err(ProviderContractError::GuestLocalRequired)
        } else {
            Ok(())
        }
    }

    /// Validate and project the signed declaration for one execution target.
    ///
    /// Host targets omit placement and use themselves as the source. Guest
    /// targets require a frozen placement. Guest-local uses the Guest as the
    /// source and creates no Export; Host-backed Guest state requires the
    /// supplied Host source and one Export per attachment.
    pub fn project_volume(
        &self,
        execution_ref: &ResourceRef,
        host_source: Option<&ResourceRef>,
        attachment_count: usize,
        quota_max_inodes: u64,
    ) -> Result<ComponentStateVolumeProjection, ProviderContractError> {
        if quota_max_inodes == 0 {
            return Err(ProviderContractError::ComponentQuotaTooSmall);
        }
        let (source_execution_ref, export_count) = match execution_ref.resource_type().as_str() {
            "Host" if self.placement_mode.is_none() && host_source.is_none() => {
                (execution_ref.clone(), 0)
            }
            "Guest" if self.placement_mode == Some(StatePlacementMode::GuestLocal) => {
                if host_source.is_some() {
                    return Err(ProviderContractError::ConflictingFields);
                }
                (execution_ref.clone(), 0)
            }
            "Guest" if self.placement_mode == Some(StatePlacementMode::HostBackedGuest) => {
                let host_source = host_source.ok_or(ProviderContractError::MissingRequiredField)?;
                if host_source.resource_type().as_str() != "Host" {
                    return Err(ProviderContractError::WrongResourceType);
                }
                (host_source.clone(), attachment_count)
            }
            "Host" | "Guest" => return Err(ProviderContractError::ConflictingFields),
            _ => return Err(ProviderContractError::WrongResourceType),
        };
        Ok(ComponentStateVolumeProjection {
            source_execution_ref,
            quota_max_bytes: self.quota_bytes,
            quota_max_inodes,
            export_count,
        })
    }
}

impl core::fmt::Debug for ComponentStateNamespace {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComponentStateNamespace")
            .field("kind", &self.kind)
            .field("persistence_class", &self.persistence_class)
            .field("sensitivity_class", &self.sensitivity_class)
            .field("storage_need", &self.storage_need)
            .field("sealing_required", &self.sealing_required)
            .field("placement_mode", &self.placement_mode)
            .field("host_custody_permitted", &self.host_custody_permitted)
            .field("view_count", &self.views.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ComponentStateNamespace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            id: BoundedToken,
            kind: ComponentStateKind,
            schema_id: VolumeStateSchemaId,
            schema_version: SchemaVersion,
            schema_digest: SchemaFingerprint,
            persistence_class: PersistenceClass,
            sensitivity_class: SensitivityClass,
            migration_policy: MigrationPolicy,
            quota_bytes: u64,
            #[serde(default)]
            storage_need: Option<StorageNeed>,
            sealing_required: bool,
            #[serde(default)]
            placement_mode: Option<StatePlacementMode>,
            #[serde(default)]
            host_custody_permitted: bool,
            views: BTreeMap<String, ComponentStateView>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.id,
            wire.kind,
            wire.schema_id,
            wire.schema_version,
            wire.schema_digest,
            wire.persistence_class,
            wire.sensitivity_class,
            wire.migration_policy,
            wire.quota_bytes,
            wire.storage_need,
            wire.sealing_required,
            wire.placement_mode,
            wire.host_custody_permitted,
            wire.views,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One declared dependency on an alias.
///
/// An optional dependency produces declared degraded behaviour only; it
/// never silently reaches a different Provider.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DependencyDeclaration {
    /// The alias this component asks the bus for.
    pub alias: DependencyAlias,
    /// Whether the component cannot start without the alias bound.
    pub required: bool,
}

/// One component descriptor from the signed manifest.
///
/// Core ProviderDeployment creates every component's static Process from
/// these descriptors. The descriptor names no binary path: the executable
/// set is pinned by digest in the catalog entry and resolved privately.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDescriptor {
    component_id: BoundedToken,
    component_type: ComponentType,
    exported_resource_types: BTreeSet<ResourceTypeName>,
    exported_methods: BTreeSet<BoundedToken>,
    allowed_domains: BTreeSet<ExecutionDomain>,
    cardinality: u32,
    config_digest: ArtifactDigest,
    dependencies: BTreeSet<DependencyDeclaration>,
    declares_state_volume: bool,
    state_namespaces: Vec<ComponentStateNamespace>,
}

impl ComponentDescriptor {
    /// Construct a component descriptor after checking every per-type rule.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        component_id: BoundedToken,
        component_type: ComponentType,
        exported_resource_types: impl IntoIterator<Item = ResourceTypeName>,
        exported_methods: impl IntoIterator<Item = BoundedToken>,
        allowed_domains: impl IntoIterator<Item = ExecutionDomain>,
        cardinality: u32,
        config_digest: ArtifactDigest,
        dependencies: impl IntoIterator<Item = DependencyDeclaration>,
        declares_state_volume: bool,
    ) -> Result<Self, ProviderContractError> {
        if declares_state_volume {
            return Err(ProviderContractError::MissingRequiredField);
        }
        let exported_resource_types: BTreeSet<_> = exported_resource_types.into_iter().collect();
        let exported_methods: BTreeSet<_> = exported_methods.into_iter().collect();
        let allowed_domains: BTreeSet<_> = allowed_domains.into_iter().collect();
        let mut aliases = BTreeSet::new();
        let mut dependency_set = BTreeSet::new();
        for dependency in dependencies {
            if !aliases.insert(dependency.alias) {
                return Err(ProviderContractError::DuplicateDeclaration);
            }
            dependency_set.insert(dependency);
        }
        if exported_resource_types.len() > MAX_COMPONENT_RESOURCE_TYPES
            || exported_methods.len() > MAX_COMPONENT_METHODS
            || allowed_domains.is_empty()
            || cardinality == 0
        {
            return Err(ProviderContractError::BoundExceeded);
        }
        match component_type {
            // A controller owns at least one ResourceType; that ownership
            // is what makes it a controller.
            ComponentType::Controller if exported_resource_types.is_empty() => {
                return Err(ProviderContractError::MissingRequiredField);
            }
            // A service serves methods and owns no ResourceType.
            ComponentType::Service => {
                if !exported_resource_types.is_empty() {
                    return Err(ProviderContractError::ConflictingFields);
                }
                if exported_methods.is_empty() {
                    return Err(ProviderContractError::MissingRequiredField);
                }
            }
            // A worker has no ResourceType ownership, no exported method,
            // and no dependency portal at all.
            ComponentType::Worker => {
                if !exported_resource_types.is_empty()
                    || !exported_methods.is_empty()
                    || !dependency_set.is_empty()
                {
                    return Err(ProviderContractError::ConflictingFields);
                }
            }
            ComponentType::Controller => {}
        }
        Ok(Self {
            component_id,
            component_type,
            exported_resource_types,
            exported_methods,
            allowed_domains,
            cardinality,
            config_digest,
            dependencies: dependency_set,
            declares_state_volume,
            state_namespaces: Vec::new(),
        })
    }

    /// Add the signed state namespace declarations to this descriptor.
    pub fn with_state_namespaces(
        mut self,
        state_namespaces: impl IntoIterator<Item = ComponentStateNamespace>,
    ) -> Result<Self, ProviderContractError> {
        let state_namespaces: Vec<_> = state_namespaces.into_iter().collect();
        if state_namespaces.is_empty() && self.declares_state_volume {
            return Err(ProviderContractError::MissingRequiredField);
        }
        let mut ids = BTreeSet::new();
        for namespace in &state_namespaces {
            if !ids.insert(namespace.id().clone()) {
                return Err(ProviderContractError::DuplicateDeclaration);
            }
        }
        if !state_namespaces.is_empty() {
            self.declares_state_volume = true;
        }
        self.state_namespaces = state_namespaces;
        Ok(self)
    }

    /// The component identifier, unique within the manifest.
    pub const fn component_id(&self) -> &BoundedToken {
        &self.component_id
    }

    /// The component type.
    pub const fn component_type(&self) -> ComponentType {
        self.component_type
    }

    /// The ResourceTypes this component owns.
    pub const fn exported_resource_types(&self) -> &BTreeSet<ResourceTypeName> {
        &self.exported_resource_types
    }

    /// The methods this component exports.
    pub const fn exported_methods(&self) -> &BTreeSet<BoundedToken> {
        &self.exported_methods
    }

    /// The execution domains this component may be placed in.
    pub const fn allowed_domains(&self) -> &BTreeSet<ExecutionDomain> {
        &self.allowed_domains
    }

    /// The maximum instance count for this component.
    pub const fn cardinality(&self) -> u32 {
        self.cardinality
    }

    /// The digest of this component's config projection.
    pub const fn config_digest(&self) -> &ArtifactDigest {
        &self.config_digest
    }

    /// The declared dependency aliases.
    pub const fn dependencies(&self) -> &BTreeSet<DependencyDeclaration> {
        &self.dependencies
    }

    /// Whether this component declared a state Volume under the
    /// storage-need test. A stateless component declares none, receives
    /// none, and contributes none to the ProviderStateSet.
    pub const fn declares_state_volume(&self) -> bool {
        self.declares_state_volume
    }

    /// The zero or more state namespaces signed into this descriptor.
    pub fn state_namespaces(&self) -> &[ComponentStateNamespace] {
        &self.state_namespaces
    }
}

impl core::fmt::Debug for ComponentDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComponentDescriptor")
            .field("component_type", &self.component_type)
            .field("resource_type_count", &self.exported_resource_types.len())
            .field("method_count", &self.exported_methods.len())
            .field("dependency_count", &self.dependencies.len())
            .field("declares_state_volume", &self.declares_state_volume)
            .field("state_namespace_count", &self.state_namespaces.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ComponentDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            component_id: BoundedToken,
            component_type: ComponentType,
            #[serde(default)]
            exported_resource_types: BTreeSet<ResourceTypeName>,
            #[serde(default)]
            exported_methods: BTreeSet<BoundedToken>,
            allowed_domains: BTreeSet<ExecutionDomain>,
            cardinality: u32,
            config_digest: ArtifactDigest,
            #[serde(default)]
            dependencies: BTreeSet<DependencyDeclaration>,
            #[serde(default)]
            declares_state_volume: bool,
            #[serde(default)]
            state_namespaces: Vec<ComponentStateNamespace>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.declares_state_volume == wire.state_namespaces.is_empty() {
            let error = if wire.declares_state_volume {
                ProviderContractError::MissingRequiredField
            } else {
                ProviderContractError::ConflictingFields
            };
            return Err(serde::de::Error::custom(error));
        }
        Self::new(
            wire.component_id,
            wire.component_type,
            wire.exported_resource_types,
            wire.exported_methods,
            wire.allowed_domains,
            wire.cardinality,
            wire.config_digest,
            wire.dependencies,
            false,
        )
        .and_then(|descriptor| descriptor.with_state_namespaces(wire.state_namespaces))
        .map_err(serde::de::Error::custom)
    }
}

/// Whether a Provider supports one optional base capability.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilitySupport {
    /// The Provider implements the optional base capability.
    Supported,
    /// The Provider refuses the optional base capability, and refuses it
    /// through the provider-neutral [`UNSUPPORTED_CAPABILITY_CODE`] result
    /// rather than by ignoring or reinterpreting the base field.
    Unsupported,
}

/// The signed standard capability matrix for one bound ResourceType.
///
/// The matrix is keyed on a bounded capability token rather than a frozen
/// enum. The specification requires the matrix and names the classes it must
/// cover - currency states, disruption classes, and the expedited
/// `waitForReconcile` path - but does not enumerate the closed set of
/// optional base capability identifiers, so freezing one here would invent a
/// wire contract every Provider and every base conformance suite would then
/// be bound to.
///
/// Absence is not support. [`StandardCapabilityMatrix::supports`] returns
/// `false` for an unlisted capability, so an unmentioned capability fails
/// closed exactly like an explicitly unsupported one.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct StandardCapabilityMatrix(BTreeMap<BoundedToken, CapabilitySupport>);

impl StandardCapabilityMatrix {
    /// Construct a bounded capability matrix.
    pub fn new(
        entries: impl IntoIterator<Item = (BoundedToken, CapabilitySupport)>,
    ) -> Result<Self, ProviderContractError> {
        let mut matrix = BTreeMap::new();
        for (capability, support) in entries {
            if matrix.insert(capability, support).is_some() {
                return Err(ProviderContractError::DuplicateDeclaration);
            }
        }
        if matrix.len() > MAX_CAPABILITY_MATRIX_ENTRIES {
            return Err(ProviderContractError::BoundExceeded);
        }
        Ok(Self(matrix))
    }

    /// Whether the Provider supports the exact capability. An unlisted
    /// capability is not supported.
    pub fn supports(&self, capability: &BoundedToken) -> bool {
        matches!(self.0.get(capability), Some(CapabilitySupport::Supported))
    }

    /// The declared entries in canonical order.
    pub fn entries(&self) -> impl Iterator<Item = (&BoundedToken, CapabilitySupport)> {
        self.0.iter().map(|(key, value)| (key, *value))
    }

    /// The number of declared entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the matrix declares nothing, in which case every optional
    /// base capability is refused.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl core::fmt::Debug for StandardCapabilityMatrix {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StandardCapabilityMatrix")
            .field("entries", &self.0.len())
            .finish()
    }
}

impl<'de> Deserialize<'de> for StandardCapabilityMatrix {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(BTreeMap::<BoundedToken, CapabilitySupport>::deserialize(
            deserializer,
        )?)
        .map_err(serde::de::Error::custom)
    }
}

/// One registered `spec.provider` or `status.provider` extension schema.
///
/// The resource store validates every extension write against the installed
/// Provider's registration, rejecting an unregistered or version-mismatched
/// identity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionSchemaRegistration {
    /// The qualified immutable schema identity.
    pub schema_id: ExtensionSchemaId,
    /// The registered schema version.
    pub schema_version: SchemaVersion,
    /// The digest of the signed strict schema.
    pub schema_fingerprint: SchemaFingerprint,
}

redacted_debug!(ExtensionSchemaRegistration);

/// What one Provider implements for one ResourceType.
///
/// The binding declares the exact base spec and status schema version and
/// fingerprint it implements, the signed capability matrix, and the strict
/// extension schemas it registers. The base itself is never redefined here:
/// fields shared across implementations are promoted to the ResourceType
/// base and are never registered under `spec.provider` or
/// `status.provider`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceApiBinding {
    resource_type: ResourceTypeName,
    base_spec_version: SchemaVersion,
    base_spec_fingerprint: SchemaFingerprint,
    base_status_version: SchemaVersion,
    base_status_fingerprint: SchemaFingerprint,
    capability_matrix: StandardCapabilityMatrix,
    #[serde(skip_serializing_if = "Option::is_none")]
    spec_extension: Option<ExtensionSchemaRegistration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_extension: Option<ExtensionSchemaRegistration>,
}

impl ResourceApiBinding {
    /// Construct a ResourceApiBinding.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resource_type: ResourceTypeName,
        base_spec_version: SchemaVersion,
        base_spec_fingerprint: SchemaFingerprint,
        base_status_version: SchemaVersion,
        base_status_fingerprint: SchemaFingerprint,
        capability_matrix: StandardCapabilityMatrix,
        spec_extension: Option<ExtensionSchemaRegistration>,
        status_extension: Option<ExtensionSchemaRegistration>,
    ) -> Result<Self, ProviderContractError> {
        for registration in [spec_extension.as_ref(), status_extension.as_ref()]
            .into_iter()
            .flatten()
        {
            if registration.schema_id.resource_type() != &resource_type {
                return Err(ProviderContractError::WrongResourceType);
            }
        }
        Ok(Self {
            resource_type,
            base_spec_version,
            base_spec_fingerprint,
            base_status_version,
            base_status_fingerprint,
            capability_matrix,
            spec_extension,
            status_extension,
        })
    }

    /// The bound ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// The base spec schema version this binding implements.
    pub const fn base_spec_version(&self) -> SchemaVersion {
        self.base_spec_version
    }

    /// The base spec schema fingerprint this binding implements.
    pub const fn base_spec_fingerprint(&self) -> &SchemaFingerprint {
        &self.base_spec_fingerprint
    }

    /// The base status schema version this binding implements.
    pub const fn base_status_version(&self) -> SchemaVersion {
        self.base_status_version
    }

    /// The base status schema fingerprint this binding implements.
    pub const fn base_status_fingerprint(&self) -> &SchemaFingerprint {
        &self.base_status_fingerprint
    }

    /// The signed capability matrix.
    pub const fn capability_matrix(&self) -> &StandardCapabilityMatrix {
        &self.capability_matrix
    }

    /// The registered `spec.provider` extension schema, if any.
    pub const fn spec_extension(&self) -> Option<&ExtensionSchemaRegistration> {
        self.spec_extension.as_ref()
    }

    /// The registered `status.provider` extension schema, if any.
    pub const fn status_extension(&self) -> Option<&ExtensionSchemaRegistration> {
        self.status_extension.as_ref()
    }
}

impl core::fmt::Debug for ResourceApiBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceApiBinding")
            .field("capability_entries", &self.capability_matrix.len())
            .field("registers_spec_extension", &self.spec_extension.is_some())
            .field(
                "registers_status_extension",
                &self.status_extension.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ResourceApiBinding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            resource_type: ResourceTypeName,
            base_spec_version: SchemaVersion,
            base_spec_fingerprint: SchemaFingerprint,
            base_status_version: SchemaVersion,
            base_status_fingerprint: SchemaFingerprint,
            #[serde(default)]
            capability_matrix: StandardCapabilityMatrix,
            #[serde(default)]
            spec_extension: Option<ExtensionSchemaRegistration>,
            #[serde(default)]
            status_extension: Option<ExtensionSchemaRegistration>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.resource_type,
            wire.base_spec_version,
            wire.base_spec_fingerprint,
            wire.base_status_version,
            wire.base_status_fingerprint,
            wire.capability_matrix,
            wire.spec_extension,
            wire.status_extension,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Whether a capability may leave its Zone, and how.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Exportability {
    /// Cross-Zone export is denied. This is the default for every backing
    /// Device, Endpoint, and backend descriptor.
    Forbidden,
    /// Cross-Zone export is permitted for the owner Service, through an
    /// explicit export and a signed projection factory.
    ExplicitExport,
}

/// The closed target set a Binding may name.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingTargetType {
    /// A Guest in the same Zone.
    Guest,
    /// A User in the same Zone.
    User,
    /// The Zone itself.
    Zone,
}

/// One signed export and import projection factory.
///
/// The factory is immutable within a Provider artifact. It binds the owner
/// Service ResourceType, its Binding ResourceType, the closed backing and
/// target reference sets, the strict projection schema and its fingerprint,
/// and a factory fingerprint that binds all of them plus the semantic
/// projection-protocol version - and never Provider or adapter identity.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionFactory {
    service_type: ResourceTypeName,
    binding_type: ResourceTypeName,
    allowed_backing_ref_types: BTreeSet<ResourceTypeName>,
    allowed_binding_target_ref_types: BTreeSet<BindingTargetType>,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    exportability: Exportability,
}

impl ProjectionFactory {
    /// Construct a projection factory.
    ///
    /// A factory whose exportability is forbidden is rejected: a factory
    /// exists only to serve an exportable capability, so an inconsistent
    /// pair is a manifest defect rather than a silently inert row.
    pub fn new(
        service_type: ResourceTypeName,
        binding_type: ResourceTypeName,
        allowed_backing_ref_types: impl IntoIterator<Item = ResourceTypeName>,
        allowed_binding_target_ref_types: impl IntoIterator<Item = BindingTargetType>,
        projection_schema_fingerprint: SchemaFingerprint,
        factory_fingerprint: SchemaFingerprint,
        exportability: Exportability,
    ) -> Result<Self, ProviderContractError> {
        let allowed_backing_ref_types: BTreeSet<_> =
            allowed_backing_ref_types.into_iter().collect();
        let allowed_binding_target_ref_types: BTreeSet<_> =
            allowed_binding_target_ref_types.into_iter().collect();
        if exportability == Exportability::Forbidden {
            return Err(ProviderContractError::ExportForbidden);
        }
        if service_type == binding_type {
            return Err(ProviderContractError::ConflictingFields);
        }
        if allowed_backing_ref_types.is_empty()
            || allowed_binding_target_ref_types.is_empty()
            || allowed_backing_ref_types.len() > MAX_PROJECTION_REF_TYPES
        {
            return Err(ProviderContractError::BoundExceeded);
        }
        Ok(Self {
            service_type,
            binding_type,
            allowed_backing_ref_types,
            allowed_binding_target_ref_types,
            projection_schema_fingerprint,
            factory_fingerprint,
            exportability,
        })
    }

    /// The owner authority and consumer projection ResourceType.
    pub const fn service_type(&self) -> &ResourceTypeName {
        &self.service_type
    }

    /// The local consumer intent ResourceType.
    pub const fn binding_type(&self) -> &ResourceTypeName {
        &self.binding_type
    }

    /// The closed backing reference set the owner Service may name.
    pub const fn allowed_backing_ref_types(&self) -> &BTreeSet<ResourceTypeName> {
        &self.allowed_backing_ref_types
    }

    /// The closed target set a Binding may name.
    pub const fn allowed_binding_target_ref_types(&self) -> &BTreeSet<BindingTargetType> {
        &self.allowed_binding_target_ref_types
    }

    /// The strict projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.projection_schema_fingerprint
    }

    /// The factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.factory_fingerprint
    }

    /// The declared exportability.
    pub const fn exportability(&self) -> Exportability {
        self.exportability
    }

    /// Decide whether an export may target the supplied resource.
    ///
    /// `ResourceExport.resourceRef` must target the owner Service, never a
    /// Device, an Endpoint, or a Binding.
    pub fn admits_export_target(
        &self,
        resource_ref: &ResourceRef,
    ) -> Result<(), ProviderContractError> {
        if resource_ref.resource_type() == &self.service_type {
            Ok(())
        } else {
            Err(ProviderContractError::ProjectionFactoryInvalid)
        }
    }
}

impl core::fmt::Debug for ProjectionFactory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProjectionFactory")
            .field("exportability", &self.exportability)
            .field("backing_ref_types", &self.allowed_backing_ref_types.len())
            .field(
                "binding_target_types",
                &self.allowed_binding_target_ref_types.len(),
            )
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ProjectionFactory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            service_type: ResourceTypeName,
            binding_type: ResourceTypeName,
            allowed_backing_ref_types: BTreeSet<ResourceTypeName>,
            allowed_binding_target_ref_types: BTreeSet<BindingTargetType>,
            projection_schema_fingerprint: SchemaFingerprint,
            factory_fingerprint: SchemaFingerprint,
            exportability: Exportability,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.service_type,
            wire.binding_type,
            wire.allowed_backing_ref_types,
            wire.allowed_binding_target_ref_types,
            wire.projection_schema_fingerprint,
            wire.factory_fingerprint,
            wire.exportability,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// How a controller reports a change it cannot apply in place.
///
/// A controller reports [`UpgradeDisposition::UpgradeRequired`] for a
/// disruptive change rather than applying it; a non-disruptive change
/// reconciles normally. Replacing the resource-row identity is used only
/// when explicitly required and planned with ownership and state transfer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum UpgradeDisposition {
    /// The change reconciles in place with no disruption.
    InPlace,
    /// The change is disruptive and requires a planned upgrade.
    UpgradeRequired,
    /// The change requires replacing the resource-row identity, with
    /// ownership and state transfer.
    Replace,
}

/// The upgrade, drain, and restart policy a Provider manifest declares.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpgradePolicy {
    /// Whether components are drained before an upgrade recycles them.
    pub drain_before_upgrade: bool,
    /// The maximum disposition this Provider will apply without an
    /// operator-planned upgrade.
    pub max_automatic_disposition: UpgradeDisposition,
    /// Whether an upgrade preserves durable, state, and secret Volumes and
    /// TPM identity, recycling only realization and owned ephemeral
    /// Processes.
    pub preserves_durable_state: bool,
}

impl core::fmt::Debug for UpgradePolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UpgradePolicy")
            .field("drain_before_upgrade", &self.drain_before_upgrade)
            .field("max_automatic_disposition", &self.max_automatic_disposition)
            .field("preserves_durable_state", &self.preserves_durable_state)
            .finish()
    }
}

/// The signed manifest and catalog entry one `artifactId` selects.
///
/// This is the read-only derived data the Provider resource row does not
/// author. Core ProviderDeployment reads it and creates the Provider's
/// static component graph from it.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderManifest {
    artifact_id: ArtifactId,
    digests: ArtifactDigestSet,
    trust: TrustEvidence,
    compatibility: CompatibilityRange,
    components: Vec<ComponentDescriptor>,
    api_bindings: Vec<ResourceApiBinding>,
    projection_factories: Vec<ProjectionFactory>,
    upgrade_policy: UpgradePolicy,
}

impl ProviderManifest {
    /// Construct a manifest after checking every structural invariant.
    ///
    /// Component identifiers are unique, a ResourceType is declared once
    /// across the component graph, every bound ResourceType is owned by
    /// exactly one controller, and a projection factory names a Service
    /// type that some binding covers.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        artifact_id: ArtifactId,
        digests: ArtifactDigestSet,
        trust: TrustEvidence,
        compatibility: CompatibilityRange,
        components: impl IntoIterator<Item = ComponentDescriptor>,
        api_bindings: impl IntoIterator<Item = ResourceApiBinding>,
        projection_factories: impl IntoIterator<Item = ProjectionFactory>,
        upgrade_policy: UpgradePolicy,
    ) -> Result<Self, ProviderContractError> {
        let components: Vec<_> = components.into_iter().collect();
        let api_bindings: Vec<_> = api_bindings.into_iter().collect();
        let projection_factories: Vec<_> = projection_factories.into_iter().collect();
        if components.is_empty()
            || components.len() > MAX_PROVIDER_COMPONENTS
            || api_bindings.len() > MAX_PROVIDER_API_BINDINGS
            || projection_factories.len() > MAX_PROJECTION_FACTORIES
        {
            return Err(ProviderContractError::BoundExceeded);
        }
        let mut component_ids = BTreeSet::new();
        let mut owned_types = BTreeSet::new();
        for component in &components {
            if component.declares_state_volume() == component.state_namespaces().is_empty() {
                return Err(ProviderContractError::MissingRequiredField);
            }
            if !component_ids.insert(component.component_id().clone()) {
                return Err(ProviderContractError::DuplicateDeclaration);
            }
            for resource_type in component.exported_resource_types() {
                // "The same ResourceType is declared once." Several
                // controller instances may run under different Hosts,
                // Guests, or domains, but not under duplicate schemas.
                if !owned_types.insert(resource_type.clone()) {
                    return Err(ProviderContractError::DuplicateDeclaration);
                }
            }
        }
        let mut bound_types = BTreeSet::new();
        for binding in &api_bindings {
            if !bound_types.insert(binding.resource_type().clone()) {
                return Err(ProviderContractError::DuplicateDeclaration);
            }
            if !owned_types.contains(binding.resource_type()) {
                return Err(ProviderContractError::MissingRequiredField);
            }
        }
        for factory in &projection_factories {
            if !bound_types.contains(factory.service_type()) {
                return Err(ProviderContractError::ProjectionFactoryInvalid);
            }
        }
        Ok(Self {
            artifact_id,
            digests,
            trust,
            compatibility,
            components,
            api_bindings,
            projection_factories,
            upgrade_policy,
        })
    }

    /// The artifact identifier a Provider spec selects this manifest with.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// The pinned digests.
    pub const fn digests(&self) -> &ArtifactDigestSet {
        &self.digests
    }

    /// The trust evidence.
    pub const fn trust(&self) -> &TrustEvidence {
        &self.trust
    }

    /// The advertised compatibility.
    pub const fn compatibility(&self) -> &CompatibilityRange {
        &self.compatibility
    }

    /// The component descriptors.
    pub fn components(&self) -> &[ComponentDescriptor] {
        &self.components
    }

    /// The ResourceApiBindings.
    pub fn api_bindings(&self) -> &[ResourceApiBinding] {
        &self.api_bindings
    }

    /// The advertised projection factories.
    pub fn projection_factories(&self) -> &[ProjectionFactory] {
        &self.projection_factories
    }

    /// The upgrade, drain, and restart policy.
    pub const fn upgrade_policy(&self) -> &UpgradePolicy {
        &self.upgrade_policy
    }

    /// Whether any component declared a state Volume.
    ///
    /// A Provider whose components all declare none has an empty
    /// ProviderStateSet, which is a normal outcome and not a defect.
    pub fn declares_state_volume(&self) -> bool {
        self.components
            .iter()
            .any(ComponentDescriptor::declares_state_volume)
    }

    /// The binding for the exact ResourceType, if this Provider binds it.
    pub fn binding_for(&self, resource_type: &ResourceTypeName) -> Option<&ResourceApiBinding> {
        self.api_bindings
            .iter()
            .find(|binding| binding.resource_type() == resource_type)
    }

    /// Decide production admission for this artifact against an exact
    /// required Provider API version and descriptor fingerprint.
    ///
    /// Trust is evaluated first and independently: an untrusted artifact is
    /// refused before any compatibility question is asked, so a compatible
    /// but untrusted artifact can never be admitted.
    pub fn admit(
        &self,
        required_major: u32,
        required_minor: u32,
        required_fingerprint: &SchemaFingerprint,
    ) -> Result<(), ProviderContractError> {
        self.trust.admit()?;
        self.compatibility
            .admits(required_major, required_minor, required_fingerprint)
    }
}

impl core::fmt::Debug for ProviderManifest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderManifest")
            .field("component_count", &self.components.len())
            .field("api_binding_count", &self.api_bindings.len())
            .field("projection_factory_count", &self.projection_factories.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ProviderManifest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            artifact_id: ArtifactId,
            digests: ArtifactDigestSet,
            trust: TrustEvidence,
            compatibility: CompatibilityRange,
            components: Vec<ComponentDescriptor>,
            #[serde(default)]
            api_bindings: Vec<ResourceApiBinding>,
            #[serde(default)]
            projection_factories: Vec<ProjectionFactory>,
            upgrade_policy: UpgradePolicy,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.artifact_id,
            wire.digests,
            wire.trust,
            wire.compatibility,
            wire.components,
            wire.api_bindings,
            wire.projection_factories,
            wire.upgrade_policy,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// A Provider method identifier the specification itself names.
///
/// This catalogue is deliberately partial, and the partiality is the point.
/// Two method families are named normatively and are therefore frozen here:
/// the controller currency and upgrade triple, and the Transport carriage
/// triple. The remaining Provider families in the frozen initial catalog
/// have no method names, request payloads, or response payloads written
/// anywhere in the specification set, so enumerating them would be
/// inventing a wire contract rather than transcribing one. A Zone runtime
/// therefore keeps naming its own instance handle, and this enum names only
/// what is actually specified.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SpecifiedProviderMethod {
    /// Report whether a change is current, and its disruption class.
    AssessUpdate,
    /// Produce a plan for a disruptive change.
    PlanUpgrade,
    /// Execute a planned upgrade.
    ExecuteUpgrade,
    /// Open a carriage transport and return an opaque owned handle.
    OpenTransport,
    /// Close a carriage transport.
    CloseTransport,
    /// Observe a carriage transport.
    ObserveTransport,
}

impl SpecifiedProviderMethod {
    /// The controller currency and upgrade triple every controller
    /// component implements alongside ordinary reconcile.
    pub const CONTROLLER_CURRENCY: [Self; 3] =
        [Self::AssessUpdate, Self::PlanUpgrade, Self::ExecuteUpgrade];

    /// The Transport carriage triple. A Transport Provider returns only an
    /// opaque owned transport handle and observations; it holds no ZoneLink
    /// state, which the core ZoneLink handler alone owns.
    pub const TRANSPORT_CARRIAGE: [Self; 3] = [
        Self::OpenTransport,
        Self::CloseTransport,
        Self::ObserveTransport,
    ];

    /// Every specified method, in a deterministic order.
    pub const ALL: [Self; 6] = [
        Self::AssessUpdate,
        Self::PlanUpgrade,
        Self::ExecuteUpgrade,
        Self::OpenTransport,
        Self::CloseTransport,
        Self::ObserveTransport,
    ];

    /// The stable lower-camel method token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AssessUpdate => "assessUpdate",
            Self::PlanUpgrade => "planUpgrade",
            Self::ExecuteUpgrade => "executeUpgrade",
            Self::OpenTransport => "openTransport",
            Self::CloseTransport => "closeTransport",
            Self::ObserveTransport => "observeTransport",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{
        execution_policy::to_base_object, resource_schema::canonical_json_bytes, volume::ViewRight,
    };

    const DIGEST_A: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    const DIGEST_B: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000002";

    const MINIMAL_PROVIDER_SPEC: &[u8] = br#"{"artifactId":"provider-wayland","config":{}}"#;
    const STATEFUL_COMPONENT: &[u8] = br#"{"allowedDomains":["system"],"cardinality":1,"componentId":"volume-controller","componentType":"controller","configDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000002","declaresStateVolume":true,"dependencies":[{"alias":"volume","required":true}],"exportedMethods":["assess-update"],"exportedResourceTypes":["Volume"],"stateNamespaces":[{"id":"main-state","kind":"state","migrationPolicy":"pre-launch-required","persistenceClass":"persistent","placementMode":"guest-local","quotaBytes":4096,"schemaDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","schemaId":"example-provider.d2bus.org/controller/main-state","schemaVersion":"1.0","sealingRequired":false,"sensitivityClass":"private","storageNeed":"secret","views":{"main":{"rights":["read","write","create","delete","traverse"]}}}]}"#;

    fn fingerprint(hex_tail: &str) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}{hex_tail}", "0".repeat(63))).unwrap()
    }

    fn digests() -> ArtifactDigestSet {
        ArtifactDigestSet {
            package: ArtifactDigest::parse(DIGEST_A).unwrap(),
            executable: ArtifactDigest::parse(DIGEST_A).unwrap(),
            manifest: ArtifactDigest::parse(DIGEST_A).unwrap(),
            config: ArtifactDigest::parse(DIGEST_B).unwrap(),
            schema: ArtifactDigest::parse(DIGEST_B).unwrap(),
            service: ArtifactDigest::parse(DIGEST_B).unwrap(),
        }
    }

    fn trusted() -> TrustEvidence {
        TrustEvidence {
            publisher: BoundedToken::parse("first-party").unwrap(),
            root_epoch: 1,
            publisher_trusted: true,
            signature: SignatureState::Valid,
            revocation: RevocationState::Clear,
            emergency_deny: false,
            provenance: PolicyEvaluation::Accepted,
            sbom: PolicyEvaluation::Accepted,
            license: PolicyEvaluation::Accepted,
            vulnerability: PolicyEvaluation::Accepted,
            conformance: PolicyEvaluation::Accepted,
            support_channel: BoundedToken::parse("stable").unwrap(),
        }
    }

    fn compatibility() -> CompatibilityRange {
        CompatibilityRange {
            api_major: 3,
            api_minor: 4,
            descriptor_fingerprint: fingerprint("1"),
            state_schema_version: SchemaVersion::new(2, 3).unwrap(),
        }
    }

    fn controller() -> ComponentDescriptor {
        ComponentDescriptor::new(
            BoundedToken::parse("volume-controller").unwrap(),
            ComponentType::Controller,
            [ResourceTypeName::parse("Volume").unwrap()],
            [BoundedToken::parse("assess-update").unwrap()],
            [ExecutionDomain::System],
            1,
            ArtifactDigest::parse(DIGEST_B).unwrap(),
            [DependencyDeclaration {
                alias: DependencyAlias::Volume,
                required: true,
            }],
            false,
        )
        .unwrap()
    }

    fn state_views() -> BTreeMap<String, ComponentStateView> {
        BTreeMap::from([(
            "main".to_owned(),
            ComponentStateView::new(
                String::new(),
                vec![
                    ViewRight::Read,
                    ViewRight::Write,
                    ViewRight::Create,
                    ViewRight::Delete,
                    ViewRight::Traverse,
                ],
            )
            .unwrap(),
        )])
    }

    fn state_namespace(
        kind: ComponentStateKind,
        persistence_class: PersistenceClass,
        quota_bytes: u64,
        storage_need: Option<StorageNeed>,
        placement_mode: Option<StatePlacementMode>,
        host_custody_permitted: bool,
    ) -> Result<ComponentStateNamespace, ProviderContractError> {
        ComponentStateNamespace::new(
            BoundedToken::parse("main-state").unwrap(),
            kind,
            VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(DIGEST_A).unwrap(),
            persistence_class,
            SensitivityClass::Private,
            MigrationPolicy::PreLaunchRequired,
            quota_bytes,
            storage_need,
            false,
            placement_mode,
            host_custody_permitted,
            state_views(),
        )
    }

    fn stateful_controller(namespace: ComponentStateNamespace) -> ComponentDescriptor {
        controller().with_state_namespaces([namespace]).unwrap()
    }

    fn binding() -> ResourceApiBinding {
        ResourceApiBinding::new(
            ResourceTypeName::parse("Volume").unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint("2"),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint("3"),
            StandardCapabilityMatrix::new([(
                BoundedToken::parse("expedited-reconcile").unwrap(),
                CapabilitySupport::Supported,
            )])
            .unwrap(),
            None,
            None,
        )
        .unwrap()
    }

    fn manifest() -> ProviderManifest {
        ProviderManifest::new(
            ArtifactId::parse("provider-volume-local").unwrap(),
            digests(),
            trusted(),
            compatibility(),
            [controller()],
            [binding()],
            [],
            UpgradePolicy {
                drain_before_upgrade: true,
                max_automatic_disposition: UpgradeDisposition::InPlace,
                preserves_durable_state: true,
            },
        )
        .unwrap()
    }

    #[test]
    fn schema_vector_pins_the_minimal_provider_base_spec() {
        let spec = ProviderSpec::minimal(ArtifactId::parse("provider-wayland").unwrap());
        assert_eq!(canonical_json_bytes(&spec).unwrap(), MINIMAL_PROVIDER_SPEC);
        let parsed: ProviderSpec = serde_json::from_slice(MINIMAL_PROVIDER_SPEC).unwrap();
        assert_eq!(parsed, spec);
        let base = to_base_object(&spec).unwrap();
        // Every other Provider property is manifest-derived, so none of
        // them may appear as an authored spec field.
        for absent in [
            "providerRef",
            "provider",
            "updatePolicy",
            "package",
            "publisher",
            "components",
            "dependencies",
            "manifest",
            "digests",
        ] {
            assert!(base.get(absent).is_none());
        }
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn descriptor_schema_vector_pins_signed_state_namespaces() {
        let descriptor = stateful_controller(
            state_namespace(
                ComponentStateKind::State,
                PersistenceClass::Persistent,
                MIN_COMPONENT_STATE_QUOTA_BYTES,
                Some(StorageNeed::Secret),
                Some(StatePlacementMode::GuestLocal),
                false,
            )
            .unwrap(),
        );
        assert_eq!(
            canonical_json_bytes(&descriptor).unwrap(),
            STATEFUL_COMPONENT
        );
        let parsed: ComponentDescriptor = serde_json::from_slice(STATEFUL_COMPONENT).unwrap();
        assert_eq!(parsed, descriptor);
        assert_eq!(parsed.state_namespaces().len(), 1);
        assert!(parsed.declares_state_volume());
    }

    #[test]
    fn stateless_component_declares_no_namespace_round_trip() {
        let descriptor = controller();
        let bytes = canonical_json_bytes(&descriptor).unwrap();
        let parsed: ComponentDescriptor = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.state_namespaces().is_empty());
        assert!(!parsed.declares_state_volume());
    }

    #[test]
    fn declared_state_volume_requires_at_least_one_namespace() {
        assert_eq!(
            ComponentDescriptor::new(
                BoundedToken::parse("volume-controller").unwrap(),
                ComponentType::Controller,
                [ResourceTypeName::parse("Volume").unwrap()],
                [BoundedToken::parse("assess-update").unwrap()],
                [ExecutionDomain::System],
                1,
                ArtifactDigest::parse(DIGEST_B).unwrap(),
                [DependencyDeclaration {
                    alias: DependencyAlias::Volume,
                    required: true,
                }],
                true,
            ),
            Err(ProviderContractError::MissingRequiredField)
        );

        let mut descriptor = controller();
        descriptor.declares_state_volume = true;
        assert_eq!(
            descriptor.clone().with_state_namespaces([]),
            Err(ProviderContractError::MissingRequiredField)
        );
        assert_eq!(
            ProviderManifest::new(
                ArtifactId::parse("provider-volume-local").unwrap(),
                digests(),
                trusted(),
                compatibility(),
                [descriptor.clone()],
                [binding()],
                [],
                UpgradePolicy {
                    drain_before_upgrade: true,
                    max_automatic_disposition: UpgradeDisposition::InPlace,
                    preserves_durable_state: true,
                },
            ),
            Err(ProviderContractError::MissingRequiredField)
        );

        let mut wire = serde_json::to_value(descriptor).unwrap();
        wire["stateNamespaces"] = serde_json::json!([]);
        assert!(serde_json::from_value::<ComponentDescriptor>(wire).is_err());
    }

    #[test]
    fn component_state_not_justified_is_rejected() {
        assert_eq!(
            state_namespace(
                ComponentStateKind::State,
                PersistenceClass::Persistent,
                MIN_COMPONENT_STATE_QUOTA_BYTES,
                None,
                None,
                false,
            ),
            Err(ProviderContractError::ComponentStateNotJustified)
        );
        assert_eq!(
            ProviderContractError::ComponentStateNotJustified.code(),
            "component-state-not-justified"
        );
    }

    #[test]
    fn component_kind_invalid_is_rejected() {
        assert_eq!(
            state_namespace(
                ComponentStateKind::Staging,
                PersistenceClass::Persistent,
                MIN_COMPONENT_STATE_QUOTA_BYTES,
                Some(StorageNeed::LargeBinary),
                None,
                false,
            ),
            Err(ProviderContractError::ComponentKindInvalid)
        );
        assert_eq!(
            ProviderContractError::ComponentKindInvalid.code(),
            "component-kind-invalid"
        );
    }

    #[test]
    fn component_persistence_class_forbidden_is_rejected() {
        assert_eq!(
            state_namespace(
                ComponentStateKind::State,
                PersistenceClass::Ephemeral,
                MIN_COMPONENT_STATE_QUOTA_BYTES,
                Some(StorageNeed::RevisionUnsuitable),
                None,
                false,
            ),
            Err(ProviderContractError::ComponentPersistenceClassForbidden)
        );
        assert_eq!(
            ProviderContractError::ComponentPersistenceClassForbidden.code(),
            "component-persistence-class-forbidden"
        );
    }

    #[test]
    fn component_quota_too_small_is_rejected() {
        for quota_bytes in [0, 1_024] {
            assert_eq!(
                state_namespace(
                    ComponentStateKind::State,
                    PersistenceClass::Persistent,
                    quota_bytes,
                    Some(StorageNeed::PrivateUnsafeForStatus),
                    None,
                    false,
                ),
                Err(ProviderContractError::ComponentQuotaTooSmall)
            );
        }
        assert_eq!(
            ProviderContractError::ComponentQuotaTooSmall.code(),
            "component-quota-too-small"
        );
    }

    #[test]
    fn placement_host_custody_violation_is_rejected() {
        assert_eq!(
            state_namespace(
                ComponentStateKind::State,
                PersistenceClass::Persistent,
                MIN_COMPONENT_STATE_QUOTA_BYTES,
                Some(StorageNeed::Secret),
                Some(StatePlacementMode::HostBackedGuest),
                false,
            ),
            Err(ProviderContractError::PlacementHostCustodyViolation)
        );
        assert_eq!(
            ProviderContractError::PlacementHostCustodyViolation.code(),
            "placement-host-custody-violation"
        );
    }

    #[test]
    fn guest_local_required_schema_classes_reject_host_backed_guest() {
        let namespace = state_namespace(
            ComponentStateKind::State,
            PersistenceClass::Persistent,
            MIN_COMPONENT_STATE_QUOTA_BYTES,
            Some(StorageNeed::Secret),
            Some(StatePlacementMode::HostBackedGuest),
            true,
        )
        .unwrap();
        for class in [
            StateSchemaCustodyClass::Credential,
            StateSchemaCustodyClass::Audit,
            StateSchemaCustodyClass::RemoteNode,
            StateSchemaCustodyClass::CloudControl,
        ] {
            assert_eq!(
                namespace.validate_schema_custody(class),
                Err(ProviderContractError::GuestLocalRequired)
            );
        }
        assert!(
            namespace
                .validate_schema_custody(StateSchemaCustodyClass::Ordinary)
                .is_ok()
        );
        assert_eq!(
            ProviderContractError::GuestLocalRequired.code(),
            "guest-local-required"
        );
    }

    #[test]
    fn descriptor_volume_projection_preserves_quota_source_and_exports() {
        for quota_bytes in [MIN_COMPONENT_STATE_QUOTA_BYTES, 1_048_576, 100_000_000] {
            let guest_local = state_namespace(
                ComponentStateKind::State,
                PersistenceClass::Persistent,
                quota_bytes,
                Some(StorageNeed::LargeBinary),
                Some(StatePlacementMode::GuestLocal),
                false,
            )
            .unwrap();
            let guest = ResourceRef::parse("Guest/work").unwrap();
            let projected = guest_local.project_volume(&guest, None, 2, 4_096).unwrap();
            assert_eq!(projected.source_execution_ref(), &guest);
            assert_eq!(projected.quota_max_bytes(), quota_bytes);
            assert!(projected.quota_max_inodes() > 0);
            assert_eq!(projected.export_count(), 0);

            let host_backed = state_namespace(
                ComponentStateKind::State,
                PersistenceClass::Persistent,
                quota_bytes,
                Some(StorageNeed::LargeBinary),
                Some(StatePlacementMode::HostBackedGuest),
                true,
            )
            .unwrap();
            let host = ResourceRef::parse("Host/host-system").unwrap();
            let projected = host_backed
                .project_volume(&guest, Some(&host), 2, 4_096)
                .unwrap();
            assert_eq!(projected.source_execution_ref(), &host);
            assert_eq!(projected.quota_max_bytes(), quota_bytes);
            assert!(projected.quota_max_inodes() > 0);
            assert_eq!(projected.export_count(), 2);
        }
    }

    #[test]
    fn zero_volume_inode_quota_is_rejected() {
        let namespace = state_namespace(
            ComponentStateKind::State,
            PersistenceClass::Persistent,
            MIN_COMPONENT_STATE_QUOTA_BYTES,
            Some(StorageNeed::LargeBinary),
            Some(StatePlacementMode::GuestLocal),
            false,
        )
        .unwrap();
        assert_eq!(
            namespace.project_volume(&ResourceRef::parse("Guest/work").unwrap(), None, 0, 0),
            Err(ProviderContractError::ComponentQuotaTooSmall)
        );
    }

    #[test]
    fn the_provider_spec_admits_exactly_two_authored_fields() {
        for rejected in [
            br#"{"artifactId":"provider-wayland","config":{},"publisher":"acme"}"#.as_slice(),
            br#"{"artifactId":"provider-wayland","config":{},"providerRef":"Provider/x"}"#,
            br#"{"artifactId":"Provider/wayland","config":{}}"#,
            br#"{"config":{}}"#,
        ] {
            assert!(serde_json::from_slice::<ProviderSpec>(rejected).is_err());
        }
        assert!(ArtifactId::parse("provider-wayland").is_ok());
        assert!(ArtifactId::parse("Provider-Wayland").is_err());
        assert!(ArtifactId::parse("/nix/store/abc-provider").is_err());
    }

    #[test]
    fn trust_admission_fails_closed_on_every_missing_element() {
        assert!(trusted().admit().is_ok());
        let mutations: [fn(&mut TrustEvidence); 9] = [
            |trust| trust.publisher_trusted = false,
            |trust| trust.signature = SignatureState::Absent,
            |trust| trust.signature = SignatureState::Invalid,
            |trust| trust.revocation = RevocationState::Unknown,
            |trust| trust.emergency_deny = true,
            |trust| trust.provenance = PolicyEvaluation::Unevaluated,
            |trust| trust.sbom = PolicyEvaluation::Rejected,
            |trust| trust.license = PolicyEvaluation::Unevaluated,
            |trust| trust.conformance = PolicyEvaluation::Unevaluated,
        ];
        for mutate in mutations {
            let mut trust = trusted();
            mutate(&mut trust);
            assert_eq!(
                trust.admit(),
                Err(ProviderContractError::TrustNotEstablished)
            );
        }
    }

    #[test]
    fn compatibility_is_exact_major_additive_minor_and_never_downgrades() {
        let range = compatibility();
        assert!(range.admits(3, 4, &fingerprint("1")).is_ok());
        assert!(range.admits(3, 0, &fingerprint("1")).is_ok());
        assert_eq!(
            range.admits(2, 4, &fingerprint("1")),
            Err(ProviderContractError::ApiMajorMismatch)
        );
        assert_eq!(
            range.admits(3, 5, &fingerprint("1")),
            Err(ProviderContractError::ApiMinorTooNew)
        );
        assert_eq!(
            range.admits(3, 4, &fingerprint("9")),
            Err(ProviderContractError::DescriptorFingerprintMismatch)
        );
        assert!(
            range
                .admits_state(SchemaVersion::new(2, 0).unwrap())
                .is_ok()
        );
        assert_eq!(
            range.admits_state(SchemaVersion::new(2, 4).unwrap()),
            Err(ProviderContractError::StateSchemaIncompatible)
        );
        assert_eq!(
            range.admits_state(SchemaVersion::new(3, 0).unwrap()),
            Err(ProviderContractError::StateSchemaIncompatible)
        );
    }

    #[test]
    fn a_worker_component_carries_no_authority_and_a_service_owns_no_resource_type() {
        let worker = |methods: Vec<BoundedToken>, dependencies: Vec<DependencyDeclaration>| {
            ComponentDescriptor::new(
                BoundedToken::parse("virtiofsd-worker").unwrap(),
                ComponentType::Worker,
                [],
                methods,
                [ExecutionDomain::System],
                4,
                ArtifactDigest::parse(DIGEST_A).unwrap(),
                dependencies,
                false,
            )
        };
        assert!(worker(vec![], vec![]).is_ok());
        assert_eq!(
            worker(vec![BoundedToken::parse("open").unwrap()], vec![]),
            Err(ProviderContractError::ConflictingFields)
        );
        assert_eq!(
            worker(
                vec![],
                vec![DependencyDeclaration {
                    alias: DependencyAlias::Volume,
                    required: false,
                }]
            ),
            Err(ProviderContractError::ConflictingFields)
        );
        assert_eq!(
            ComponentDescriptor::new(
                BoundedToken::parse("policy-service").unwrap(),
                ComponentType::Service,
                [ResourceTypeName::parse("Volume").unwrap()],
                [BoundedToken::parse("open").unwrap()],
                [ExecutionDomain::User],
                1,
                ArtifactDigest::parse(DIGEST_A).unwrap(),
                [],
                false,
            ),
            Err(ProviderContractError::ConflictingFields)
        );
        assert_eq!(
            ComponentDescriptor::new(
                BoundedToken::parse("empty-controller").unwrap(),
                ComponentType::Controller,
                [],
                [],
                [ExecutionDomain::System],
                1,
                ArtifactDigest::parse(DIGEST_A).unwrap(),
                [],
                false,
            ),
            Err(ProviderContractError::MissingRequiredField)
        );
    }

    #[test]
    fn an_unlisted_optional_capability_is_not_supported() {
        let matrix = StandardCapabilityMatrix::new([
            (
                BoundedToken::parse("expedited-reconcile").unwrap(),
                CapabilitySupport::Supported,
            ),
            (
                BoundedToken::parse("in-place-resize").unwrap(),
                CapabilitySupport::Unsupported,
            ),
        ])
        .unwrap();
        assert!(matrix.supports(&BoundedToken::parse("expedited-reconcile").unwrap()));
        assert!(!matrix.supports(&BoundedToken::parse("in-place-resize").unwrap()));
        assert!(!matrix.supports(&BoundedToken::parse("never-declared").unwrap()));
        assert!(StandardCapabilityMatrix::default().is_empty());
        assert!(
            !StandardCapabilityMatrix::default()
                .supports(&BoundedToken::parse("expedited-reconcile").unwrap())
        );
        assert_eq!(UNSUPPORTED_CAPABILITY_CODE, "unsupported-capability");
    }

    #[test]
    fn a_projection_factory_is_export_only_and_targets_the_owner_service() {
        let service = ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap();
        let binding_type = ResourceTypeName::parse("audio.d2bus.org.AudioBinding").unwrap();
        let factory = ProjectionFactory::new(
            service.clone(),
            binding_type.clone(),
            [ResourceTypeName::parse("Device").unwrap()],
            [BindingTargetType::Guest, BindingTargetType::User],
            fingerprint("4"),
            fingerprint("5"),
            Exportability::ExplicitExport,
        )
        .unwrap();
        assert!(
            factory
                .admits_export_target(
                    &ResourceRef::parse("audio.d2bus.org.AudioService/desk").unwrap()
                )
                .is_ok()
        );
        for rejected in ["Device/headset", "audio.d2bus.org.AudioBinding/desk"] {
            assert_eq!(
                factory.admits_export_target(&ResourceRef::parse(rejected).unwrap()),
                Err(ProviderContractError::ProjectionFactoryInvalid)
            );
        }
        assert_eq!(
            ProjectionFactory::new(
                service.clone(),
                binding_type,
                [ResourceTypeName::parse("Device").unwrap()],
                [BindingTargetType::Zone],
                fingerprint("4"),
                fingerprint("5"),
                Exportability::Forbidden,
            ),
            Err(ProviderContractError::ExportForbidden)
        );
    }

    #[test]
    fn the_manifest_declares_each_resource_type_once_and_binds_only_what_it_owns() {
        let subject = manifest();
        assert!(
            subject
                .binding_for(&ResourceTypeName::parse("Volume").unwrap())
                .is_some()
        );
        assert!(
            subject
                .binding_for(&ResourceTypeName::parse("Network").unwrap())
                .is_none()
        );
        assert!(!subject.declares_state_volume());

        let duplicate_controller = ComponentDescriptor::new(
            BoundedToken::parse("second-volume-controller").unwrap(),
            ComponentType::Controller,
            [ResourceTypeName::parse("Volume").unwrap()],
            [],
            [ExecutionDomain::System],
            1,
            ArtifactDigest::parse(DIGEST_A).unwrap(),
            [],
            false,
        )
        .unwrap();
        assert_eq!(
            ProviderManifest::new(
                ArtifactId::parse("provider-volume-local").unwrap(),
                digests(),
                trusted(),
                compatibility(),
                [controller(), duplicate_controller],
                [binding()],
                [],
                *subject.upgrade_policy(),
            ),
            Err(ProviderContractError::DuplicateDeclaration)
        );

        let unowned_binding = ResourceApiBinding::new(
            ResourceTypeName::parse("Network").unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint("2"),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint("3"),
            StandardCapabilityMatrix::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            ProviderManifest::new(
                ArtifactId::parse("provider-volume-local").unwrap(),
                digests(),
                trusted(),
                compatibility(),
                [controller()],
                [unowned_binding],
                [],
                *subject.upgrade_policy(),
            ),
            Err(ProviderContractError::MissingRequiredField)
        );
    }

    #[test]
    fn an_extension_registration_cannot_name_a_foreign_resource_type() {
        let registration = ExtensionSchemaRegistration {
            schema_id: ExtensionSchemaId::parse("volume-local.d2bus.org/Network/spec").unwrap(),
            schema_version: SchemaVersion::new(1, 0).unwrap(),
            schema_fingerprint: fingerprint("6"),
        };
        assert_eq!(
            ResourceApiBinding::new(
                ResourceTypeName::parse("Volume").unwrap(),
                SchemaVersion::new(1, 0).unwrap(),
                fingerprint("2"),
                SchemaVersion::new(1, 0).unwrap(),
                fingerprint("3"),
                StandardCapabilityMatrix::default(),
                Some(registration),
                None,
            ),
            Err(ProviderContractError::WrongResourceType)
        );
    }

    #[test]
    fn admission_evaluates_trust_before_compatibility() {
        let subject = manifest();
        assert!(subject.admit(3, 4, &fingerprint("1")).is_ok());
        assert_eq!(
            subject.admit(9, 0, &fingerprint("1")),
            Err(ProviderContractError::ApiMajorMismatch)
        );

        let mut untrusted = trusted();
        untrusted.emergency_deny = true;
        let denied = ProviderManifest::new(
            ArtifactId::parse("provider-volume-local").unwrap(),
            digests(),
            untrusted,
            compatibility(),
            [controller()],
            [binding()],
            [],
            *subject.upgrade_policy(),
        )
        .unwrap();
        // A perfectly compatible artifact is still refused on trust, and
        // the refusal names trust rather than compatibility.
        assert_eq!(
            denied.admit(3, 4, &fingerprint("1")),
            Err(ProviderContractError::TrustNotEstablished)
        );
    }

    #[test]
    fn manifest_vector_round_trips_through_canonical_bytes() {
        let subject = manifest();
        let bytes = canonical_json_bytes(&subject).unwrap();
        let parsed: ProviderManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, subject);
        assert_eq!(canonical_json_bytes(&parsed).unwrap(), bytes);
        let rendered = String::from_utf8(bytes).unwrap();
        // A manifest carries digests and identifiers, never a location.
        assert!(!rendered.contains("/nix/store"));
        assert!(!rendered.contains(".sock"));
    }

    #[test]
    fn the_dependency_alias_set_is_closed() {
        assert_eq!(DependencyAlias::ALL.len(), 5);
        for alias in DependencyAlias::ALL {
            let encoded = serde_json::to_string(&alias).unwrap();
            assert_eq!(encoded, format!("\"{}\"", alias.as_str()));
        }
        assert!(serde_json::from_str::<DependencyAlias>("\"display\"").is_err());
        assert!(serde_json::from_str::<DependencyAlias>("\"graphics\"").is_err());
    }

    #[test]
    fn the_specified_method_catalogue_renders_stable_tokens() {
        assert_eq!(SpecifiedProviderMethod::ALL.len(), 6);
        for method in SpecifiedProviderMethod::ALL {
            assert_eq!(
                serde_json::to_string(&method).unwrap(),
                format!("\"{}\"", method.as_str())
            );
        }
        for method in SpecifiedProviderMethod::TRANSPORT_CARRIAGE {
            assert!(SpecifiedProviderMethod::ALL.contains(&method));
        }
        for method in SpecifiedProviderMethod::CONTROLLER_CURRENCY {
            assert!(SpecifiedProviderMethod::ALL.contains(&method));
        }
    }

    #[test]
    fn diagnostics_stay_redacted() {
        assert_eq!(
            format!(
                "{:?}",
                ProviderSpec::minimal(ArtifactId::parse("provider-wayland").unwrap())
            ),
            "ProviderSpec(<redacted>)"
        );
        assert_eq!(
            format!("{:?}", ArtifactId::parse("provider-wayland").unwrap()),
            "ArtifactId(<redacted>)"
        );
        assert_eq!(
            format!("{:?}", ArtifactDigest::parse(DIGEST_A).unwrap()),
            "ArtifactDigest(<redacted>)"
        );
        assert_eq!(format!("{:?}", trusted()), "TrustEvidence(<redacted>)");
        let rendered = format!("{:?}", manifest());
        assert!(!rendered.contains("provider-volume-local"));
        assert!(!rendered.contains("first-party"));
        assert!(!rendered.contains("sha256:"));
        let component = format!("{:?}", controller());
        assert!(!component.contains("volume-controller"));
        assert!(!component.contains("sha256:"));
    }
}
