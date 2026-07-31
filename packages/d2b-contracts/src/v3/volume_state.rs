//! Provider payload state contracts shared by Volume and Provider descriptors.
//!
//! The default durable operational-state surface is resource status. These
//! types describe the exceptional payloads that require a state Volume because
//! they are secret, large, private from status readers, or unsuitable for the
//! revision log. No type in this module carries a path, principal, credential,
//! or execution authority.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    identity::{SchemaFingerprint, Timestamp},
    resource_schema::{CanonicalJsonError, SchemaVersion, canonical_digest, canonical_json_bytes},
};

/// Largest component generation retained from the atomic-state contract.
pub const MAX_STATE_GENERATION: u64 = 9_007_199_254_740_991;
/// Maximum canonical JSON state document retained from the atomic-state contract.
pub const MAX_STATE_DOCUMENT_BYTES: usize = 1_048_576;

/// Failure to construct or verify a Volume state contract value.
///
/// The variants are closed labels and never retain a schema ID, digest,
/// payload, path, or other caller-supplied value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VolumeStateError {
    /// A state schema ID did not have the qualified three-segment shape.
    InvalidSchemaId,
    /// A component generation was zero or outside the canonical JSON range.
    InvalidGeneration,
    /// Canonical JSON encoding failed.
    CanonicalJson,
    /// The envelope digest did not match its canonical payload.
    DigestMismatch,
    /// A quota counter was outside the canonical JSON range.
    QuotaOutOfRange,
    /// The canonical state document exceeded its retained byte bound.
    DocumentTooLarge,
}

impl VolumeStateError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidSchemaId => "volume-state-schema-id-invalid",
            Self::InvalidGeneration => "volume-state-generation-invalid",
            Self::CanonicalJson => "volume-state-canonical-json-invalid",
            Self::DigestMismatch => "volume-state-digest-mismatch",
            Self::QuotaOutOfRange => "volume-state-quota-out-of-range",
            Self::DocumentTooLarge => "volume-state-document-too-large",
        }
    }
}

impl core::fmt::Display for VolumeStateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for VolumeStateError {}

impl From<CanonicalJsonError> for VolumeStateError {
    fn from(_error: CanonicalJsonError) -> Self {
        Self::CanonicalJson
    }
}

/// A qualified immutable Provider payload schema ID.
///
/// The wire shape is `<provider-crate>/<component>/<namespace>`. Each segment
/// is lower-case and the latter two use the component-local identifier
/// grammar. The value is always redacted in diagnostics.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct VolumeStateSchemaId(
    #[schemars(regex(pattern = "^[a-z][a-z0-9.-]*/[a-z][a-z0-9-]*/[a-z][a-z0-9-]*$"))] String,
);

impl VolumeStateSchemaId {
    /// Parse a qualified state schema ID.
    pub fn parse(value: impl Into<String>) -> Result<Self, VolumeStateError> {
        let value = value.into();
        let mut segments = value.split('/');
        let provider = segments.next().ok_or(VolumeStateError::InvalidSchemaId)?;
        let component = segments.next().ok_or(VolumeStateError::InvalidSchemaId)?;
        let namespace = segments.next().ok_or(VolumeStateError::InvalidSchemaId)?;
        if segments.next().is_some()
            || !valid_provider_segment(provider)
            || !valid_local_segment(component)
            || !valid_local_segment(namespace)
        {
            return Err(VolumeStateError::InvalidSchemaId);
        }
        Ok(Self(value))
    }

    /// Borrow the canonical schema ID for authorized wire encoding.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for VolumeStateSchemaId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("VolumeStateSchemaId(<redacted>)")
    }
}

impl core::fmt::Display for VolumeStateSchemaId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("VolumeStateSchemaId(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for VolumeStateSchemaId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A canonical digest over one Provider payload state's canonical JSON bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct StateDigest(
    #[schemars(regex(pattern = "^sha256:[0-9a-f]{64}$"), length(equal = 71))] String,
);

impl StateDigest {
    /// Parse exactly `sha256:<64 lower-case hex>`.
    pub fn parse(value: impl Into<String>) -> Result<Self, VolumeStateError> {
        let value = value.into();
        SchemaFingerprint::parse(value.clone()).map_err(|_| VolumeStateError::CanonicalJson)?;
        Ok(Self(value))
    }

    /// Borrow the canonical digest for authorized wire encoding.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for StateDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StateDigest(<redacted>)")
    }
}

impl core::fmt::Display for StateDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StateDigest(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for StateDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn valid_provider_segment(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-.".contains(&byte))
}

fn valid_local_segment(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Migration behavior attached immutably to a state schema.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationPolicy {
    /// Migration must complete before the component Process starts.
    PreLaunchRequired,
    /// The component may run while an online migration completes.
    OnlineOptional,
    /// The schema defines no migration logic.
    None,
}

/// State schema identity signed into a component descriptor and copied into
/// the corresponding Volume.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeStateSchema {
    schema_id: VolumeStateSchemaId,
    schema_version: SchemaVersion,
    schema_digest: SchemaFingerprint,
    migration_policy: MigrationPolicy,
}

impl VolumeStateSchema {
    /// Construct an immutable state schema declaration.
    pub const fn new(
        schema_id: VolumeStateSchemaId,
        schema_version: SchemaVersion,
        schema_digest: SchemaFingerprint,
        migration_policy: MigrationPolicy,
    ) -> Self {
        Self {
            schema_id,
            schema_version,
            schema_digest,
            migration_policy,
        }
    }

    /// Borrow the qualified schema ID.
    pub const fn schema_id(&self) -> &VolumeStateSchemaId {
        &self.schema_id
    }

    /// Return the desired schema version.
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Borrow the canonical schema digest.
    pub const fn schema_digest(&self) -> &SchemaFingerprint {
        &self.schema_digest
    }

    /// Return the immutable migration policy.
    pub const fn migration_policy(&self) -> MigrationPolicy {
        self.migration_policy
    }
}

impl core::fmt::Debug for VolumeStateSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("VolumeStateSchema(<redacted>)")
    }
}

/// Persistence class used by state Volume and component declarations.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum PersistenceClass {
    Persistent,
    Ephemeral,
    Cache,
    Config,
}

/// Sharing and visibility class of a Provider state payload.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SensitivityClass {
    /// Exactly one component Process may mount the Volume.
    Private,
    /// Components controlled by the same Provider may share the Volume.
    Internal,
    /// Other Providers may receive read-only views.
    SharedRead,
}

/// State schema lifecycle phase reported in Volume status.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum StateSchemaPhase {
    Current,
    MigrationRequired,
    Migrating,
    MigrationCommitted,
    MigrationFailed,
}

impl StateSchemaPhase {
    /// Every phase in canonical wire order.
    pub const ALL: [Self; 5] = [
        Self::Current,
        Self::MigrationRequired,
        Self::Migrating,
        Self::MigrationCommitted,
        Self::MigrationFailed,
    ];
}

/// Identity marker observation reported in Volume status.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MarkerStatus {
    Verified,
    Missing,
    Replaced,
    Unknown,
}

impl MarkerStatus {
    /// Every marker observation in canonical wire order.
    pub const ALL: [Self; 4] = [Self::Verified, Self::Missing, Self::Replaced, Self::Unknown];
}

/// Sealing lifecycle reported in Volume status.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SealingStatus {
    None,
    Sealed,
    RotationPending,
    RotationFailed,
}

impl SealingStatus {
    /// Every sealing observation in canonical wire order.
    pub const ALL: [Self; 4] = [
        Self::None,
        Self::Sealed,
        Self::RotationPending,
        Self::RotationFailed,
    ];
}

/// Bounded current quota use reported by the Volume Provider.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuotaUsage {
    #[schemars(range(max = 9_223_372_036_854_775_807_u64))]
    used_bytes: u64,
    #[schemars(range(max = 9_223_372_036_854_775_807_u64))]
    inode_count: u64,
}

impl QuotaUsage {
    /// Construct counters that fit the canonical JSON integer profile.
    pub fn new(used_bytes: u64, inode_count: u64) -> Result<Self, VolumeStateError> {
        if used_bytes > i64::MAX as u64 || inode_count > i64::MAX as u64 {
            return Err(VolumeStateError::QuotaOutOfRange);
        }
        Ok(Self {
            used_bytes,
            inode_count,
        })
    }

    /// Return the observed byte count.
    pub const fn used_bytes(self) -> u64 {
        self.used_bytes
    }

    /// Return the observed inode count.
    pub const fn inode_count(self) -> u64 {
        self.inode_count
    }
}

impl core::fmt::Debug for QuotaUsage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("QuotaUsage(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for QuotaUsage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            used_bytes: u64,
            inode_count: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.used_bytes, wire.inode_count).map_err(serde::de::Error::custom)
    }
}

/// Provider-owned Volume status extension for payload state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeStateStatus {
    state_schema_phase: StateSchemaPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    installed_schema_version: Option<SchemaVersion>,
    marker_status: MarkerStatus,
    sealing_status: SealingStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    quota_usage: Option<QuotaUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_migration_at: Option<Timestamp>,
}

impl VolumeStateStatus {
    /// Construct the complete state status extension.
    pub const fn new(
        state_schema_phase: StateSchemaPhase,
        installed_schema_version: Option<SchemaVersion>,
        marker_status: MarkerStatus,
        sealing_status: SealingStatus,
        quota_usage: Option<QuotaUsage>,
        last_migration_at: Option<Timestamp>,
    ) -> Self {
        Self {
            state_schema_phase,
            installed_schema_version,
            marker_status,
            sealing_status,
            quota_usage,
            last_migration_at,
        }
    }

    pub const fn state_schema_phase(&self) -> StateSchemaPhase {
        self.state_schema_phase
    }

    pub const fn installed_schema_version(&self) -> Option<SchemaVersion> {
        self.installed_schema_version
    }

    pub const fn marker_status(&self) -> MarkerStatus {
        self.marker_status
    }

    pub const fn sealing_status(&self) -> SealingStatus {
        self.sealing_status
    }

    pub const fn quota_usage(&self) -> Option<QuotaUsage> {
        self.quota_usage
    }

    pub const fn last_migration_at(&self) -> Option<&Timestamp> {
        self.last_migration_at.as_ref()
    }
}

impl core::fmt::Debug for VolumeStateStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VolumeStateStatus")
            .field("state_schema_phase", &self.state_schema_phase)
            .field("marker_status", &self.marker_status)
            .field("sealing_status", &self.sealing_status)
            .field(
                "has_installed_schema_version",
                &self.installed_schema_version.is_some(),
            )
            .field("has_quota_usage", &self.quota_usage.is_some())
            .field("has_last_migration_at", &self.last_migration_at.is_some())
            .finish()
    }
}

/// Generation-bound canonical Provider payload state.
///
/// `generation` is the component's optimistic state counter, not a Zone
/// resource generation. The digest covers only canonical payload bytes under
/// the trusted domain tag supplied by the owning schema contract.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StateEnvelope<T> {
    #[schemars(range(min = 1, max = 9_007_199_254_740_991_u64))]
    generation: u64,
    digest: StateDigest,
    payload: T,
}

impl<T> StateEnvelope<T> {
    /// Construct an envelope with an already computed canonical digest.
    pub fn new(generation: u64, digest: StateDigest, payload: T) -> Result<Self, VolumeStateError> {
        validate_generation(generation)?;
        Ok(Self {
            generation,
            digest,
            payload,
        })
    }

    /// Return the component-local optimistic generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Return the next component-local optimistic generation.
    pub fn next_generation(&self) -> Result<u64, VolumeStateError> {
        let next = self
            .generation
            .checked_add(1)
            .ok_or(VolumeStateError::InvalidGeneration)?;
        validate_generation(next)?;
        Ok(next)
    }

    /// Borrow the payload digest.
    pub const fn digest(&self) -> &StateDigest {
        &self.digest
    }

    /// Borrow the payload after the caller has validated the envelope.
    pub const fn payload(&self) -> &T {
        &self.payload
    }
}

impl<T: Serialize> StateEnvelope<T> {
    /// Canonicalize a payload and construct its generation-bound envelope.
    pub fn from_payload(
        domain_tag: &str,
        generation: u64,
        payload: T,
    ) -> Result<Self, VolumeStateError> {
        let digest = canonical_state_payload_digest(domain_tag, &payload)?;
        Self::new(generation, digest, payload)
    }

    /// Verify that the stored digest covers the canonical payload bytes.
    pub fn validate_digest(&self, domain_tag: &str) -> Result<(), VolumeStateError> {
        if canonical_state_payload_digest(domain_tag, &self.payload)? == self.digest {
            Ok(())
        } else {
            Err(VolumeStateError::DigestMismatch)
        }
    }
}

impl<T> core::fmt::Debug for StateEnvelope<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StateEnvelope(<redacted>)")
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for StateEnvelope<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire<T> {
            generation: u64,
            digest: StateDigest,
            payload: T,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.generation, wire.digest, wire.payload).map_err(serde::de::Error::custom)
    }
}

/// Render payload bytes using the resource plane canonical JSON profile.
pub fn canonical_state_payload_bytes<T: Serialize>(
    payload: &T,
) -> Result<Vec<u8>, VolumeStateError> {
    let bytes = canonical_json_bytes(payload)?;
    if bytes.len() > MAX_STATE_DOCUMENT_BYTES {
        return Err(VolumeStateError::DocumentTooLarge);
    }
    Ok(bytes)
}

/// Digest canonical payload bytes under a trusted schema-specific domain tag.
pub fn canonical_state_payload_digest<T: Serialize>(
    domain_tag: &str,
    payload: &T,
) -> Result<StateDigest, VolumeStateError> {
    let bytes = canonical_state_payload_bytes(payload)?;
    StateDigest::parse(canonical_digest(domain_tag, &bytes))
}

fn validate_generation(generation: u64) -> Result<(), VolumeStateError> {
    if (1..=MAX_STATE_GENERATION).contains(&generation) {
        Ok(())
    } else {
        Err(VolumeStateError::InvalidGeneration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    const STATE_DOMAIN: &str = "test:provider-state:payload";
    const SCHEMA_VECTOR: &[u8] = br#"{"migrationPolicy":"pre-launch-required","schemaDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","schemaId":"example-provider.d2bus.org/controller/main-state","schemaVersion":"1.0"}"#;
    const STATUS_VECTOR: &[u8] = br#"{"installedSchemaVersion":"1.0","lastMigrationAt":"2026-07-22T00:00:00.000Z","markerStatus":"verified","quotaUsage":{"inodeCount":3,"usedBytes":42},"sealingStatus":"sealed","stateSchemaPhase":"current"}"#;

    fn schema() -> VolumeStateSchema {
        VolumeStateSchema::new(
            VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(DIGEST).unwrap(),
            MigrationPolicy::PreLaunchRequired,
        )
    }

    #[test]
    fn schema_and_status_golden_vectors_are_canonical() {
        let state_schema = schema();
        assert_eq!(canonical_json_bytes(&state_schema).unwrap(), SCHEMA_VECTOR);
        assert_eq!(
            serde_json::from_slice::<VolumeStateSchema>(SCHEMA_VECTOR).unwrap(),
            state_schema
        );

        let status = VolumeStateStatus::new(
            StateSchemaPhase::Current,
            Some(SchemaVersion::new(1, 0).unwrap()),
            MarkerStatus::Verified,
            SealingStatus::Sealed,
            Some(QuotaUsage::new(42, 3).unwrap()),
            Some(Timestamp::parse("2026-07-22T00:00:00.000Z").unwrap()),
        );
        assert_eq!(canonical_json_bytes(&status).unwrap(), STATUS_VECTOR);
        assert_eq!(
            serde_json::from_slice::<VolumeStateStatus>(STATUS_VECTOR).unwrap(),
            status
        );
    }

    #[test]
    fn phase_and_status_reason_tokens_round_trip() {
        for phase in StateSchemaPhase::ALL {
            let encoded = serde_json::to_vec(&phase).unwrap();
            assert_eq!(
                serde_json::from_slice::<StateSchemaPhase>(&encoded).unwrap(),
                phase
            );
        }
        for status in MarkerStatus::ALL {
            let encoded = serde_json::to_vec(&status).unwrap();
            assert_eq!(
                serde_json::from_slice::<MarkerStatus>(&encoded).unwrap(),
                status
            );
        }
        for status in SealingStatus::ALL {
            let encoded = serde_json::to_vec(&status).unwrap();
            assert_eq!(
                serde_json::from_slice::<SealingStatus>(&encoded).unwrap(),
                status
            );
        }
    }

    #[test]
    fn state_envelope_digest_binds_canonical_payload_and_carries_generation() {
        let payload = json!({"checkpoint": 7, "ready": true});
        let envelope = StateEnvelope::from_payload(STATE_DOMAIN, 4, payload).unwrap();
        envelope.validate_digest(STATE_DOMAIN).unwrap();
        assert_eq!(envelope.next_generation().unwrap(), 5);

        let bytes = canonical_json_bytes(&envelope).unwrap();
        let decoded: StateEnvelope<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        decoded.validate_digest(STATE_DOMAIN).unwrap();
        assert_eq!(decoded, envelope);

        let mut changed = decoded;
        changed.payload = json!({"checkpoint": 8, "ready": true});
        assert_eq!(
            changed.validate_digest(STATE_DOMAIN),
            Err(VolumeStateError::DigestMismatch)
        );
        assert_eq!(
            changed.validate_digest("test:other-domain"),
            Err(VolumeStateError::DigestMismatch)
        );
    }

    #[test]
    fn state_envelope_rejects_invalid_generations_and_redacts_payload() {
        let digest =
            canonical_state_payload_digest(STATE_DOMAIN, &json!({"secret": "canary"})).unwrap();
        assert!(StateEnvelope::new(0, digest.clone(), json!({})).is_err());
        assert!(StateEnvelope::new(MAX_STATE_GENERATION + 1, digest, json!({})).is_err());

        let envelope = StateEnvelope::from_payload(
            STATE_DOMAIN,
            1,
            json!({"secret": "caller-supplied-canary"}),
        )
        .unwrap();
        let diagnostic = format!("{envelope:?}");
        assert!(!diagnostic.contains("caller-supplied-canary"));
        assert!(!diagnostic.contains("sha256:"));
    }

    #[test]
    fn state_schema_id_rejects_unqualified_or_path_shaped_values() {
        for value in [
            "state",
            "provider/component",
            "provider//state",
            "Provider/component/state",
            "provider/component/../state",
            "/nix/store/component/state",
        ] {
            assert!(VolumeStateSchemaId::parse(value).is_err());
        }
    }
}
