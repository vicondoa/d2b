//! Universal and layered v3 resource status contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ObservedGeneration, ResourceGeneration, ResourceRef, Timestamp,
    resource_schema::{
        CanonicalJsonObject, ExtensionSchemaId, ExtensionSchemaLayer, SchemaVersion,
        canonical_json_bytes, validate_canonical_string,
    },
};
use d2b_realm_core::OperationId;

/// Maximum canonical bytes for a complete status object.
pub const MAX_STATUS_BYTES: usize = 64 * 1024;
/// Maximum canonical bytes for either typed status detail layer.
pub const MAX_STATUS_LAYER_BYTES: usize = 32 * 1024;
/// Maximum conditions retained in current status.
pub const MAX_STATUS_CONDITIONS: usize = 32;
/// Maximum entries in any bounded status collection.
pub const MAX_STATUS_COLLECTION_ENTRIES: usize = 64;
/// Maximum bytes in one bounded status string.
pub const MAX_STATUS_STRING_BYTES: usize = 4 * 1024;
/// Maximum retry recommendation in milliseconds.
pub const MAX_RETRY_AFTER_MS: u32 = 86_400_000;

/// Universal resource phase.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ResourcePhase {
    Pending,
    Ready,
    Succeeded,
    Degraded,
    Failed,
    Deleted,
    Unknown,
}

/// Three-valued condition state.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ConditionState {
    True,
    False,
    Unknown,
}

/// A stable lower-kebab status reason or outcome code.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StatusCode(String);

impl StatusCode {
    /// Parse a code at 1 to 64 ASCII bytes.
    pub fn parse(value: impl Into<String>) -> Result<Self, ResourceStatusError> {
        let value = value.into();
        let mut bytes = value.bytes();
        if value.len() > 64
            || !matches!(bytes.next(), Some(b'a'..=b'z'))
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ResourceStatusError::InvalidStatusCode);
        }
        Ok(Self(value))
    }

    /// Borrow the stable code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for StatusCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("StatusCode").field(&self.0).finish()
    }
}

impl<'de> Deserialize<'de> for StatusCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for StatusCode {
    fn schema_name() -> String {
        "StatusCode".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().min_length = Some(1);
        schema.string().max_length = Some(64);
        schema.string().pattern = Some("^[a-z][a-z0-9-]*$".to_owned());
        schemars::schema::Schema::Object(schema)
    }
}

/// Bounded operator-facing status detail whose Debug output is redacted.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StatusMessage(String);

impl StatusMessage {
    /// Validate a bounded, canonical string.
    pub fn parse(value: impl Into<String>) -> Result<Self, ResourceStatusError> {
        let value = value.into();
        if value.len() > MAX_STATUS_STRING_BYTES {
            return Err(ResourceStatusError::StatusStringTooLong);
        }
        validate_canonical_string(&value).map_err(|_| ResourceStatusError::InvalidStatusString)?;
        Ok(Self(value))
    }

    /// Borrow the message for an explicitly authorized presentation surface.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for StatusMessage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "StatusMessage(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for StatusMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for StatusMessage {
    fn schema_name() -> String {
        "StatusMessage".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().max_length = Some(MAX_STATUS_STRING_BYTES as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// Latest value for one condition type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCondition {
    #[serde(rename = "type")]
    condition_type: StatusCode,
    status: ConditionState,
    reason: StatusCode,
    message: StatusMessage,
    observed_generation: ObservedGeneration,
    last_transition_at: Timestamp,
}

impl ResourceCondition {
    /// Construct a validated current condition.
    pub fn new(
        condition_type: StatusCode,
        status: ConditionState,
        reason: StatusCode,
        message: StatusMessage,
        observed_generation: ObservedGeneration,
        last_transition_at: Timestamp,
    ) -> Self {
        Self {
            condition_type,
            status,
            reason,
            message,
            observed_generation,
            last_transition_at,
        }
    }

    /// Borrow the condition key.
    pub fn condition_type(&self) -> &StatusCode {
        &self.condition_type
    }
}

impl<'de> Deserialize<'de> for ResourceCondition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(rename = "type")]
            condition_type: StatusCode,
            status: ConditionState,
            reason: StatusCode,
            message: StatusMessage,
            observed_generation: ObservedGeneration,
            last_transition_at: Timestamp,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(
            wire.condition_type,
            wire.status,
            wire.reason,
            wire.message,
            wire.observed_generation,
            wire.last_transition_at,
        ))
    }
}

/// Latest bounded reconcile outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceOutcome {
    code: StatusCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    message: StatusMessage,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u32>,
    occurred_at: Timestamp,
}

impl ResourceOutcome {
    /// Construct an outcome, rejecting zero and overlong retry delays.
    pub fn new(
        code: StatusCode,
        exit_code: Option<i32>,
        message: StatusMessage,
        retryable: bool,
        retry_after_ms: Option<u32>,
        occurred_at: Timestamp,
    ) -> Result<Self, ResourceStatusError> {
        if retry_after_ms.is_some_and(|delay| delay == 0 || delay > MAX_RETRY_AFTER_MS) {
            return Err(ResourceStatusError::InvalidRetryAfter);
        }
        Ok(Self {
            code,
            exit_code,
            message,
            retryable,
            retry_after_ms,
            occurred_at,
        })
    }
}

impl<'de> Deserialize<'de> for ResourceOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            code: StatusCode,
            exit_code: Option<i32>,
            message: StatusMessage,
            retryable: bool,
            retry_after_ms: Option<u32>,
            occurred_at: Timestamp,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.code,
            wire.exit_code,
            wire.message,
            wire.retryable,
            wire.retry_after_ms,
            wire.occurred_at,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Currency state for the current desired generation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum UpdateState {
    Current,
    UpdateAvailable,
    UpgradeRequired,
    Upgrading,
    Blocked,
    Unknown,
}

/// Closed reason for an update assessment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum UpdateReason {
    CoreGenerationChanged,
    ProviderGenerationChanged,
    ArtifactChanged,
    ImageOrSystemGenerationChanged,
    SpecChanged,
    DependencyChanged,
    SecurityPolicyChanged,
}

/// Planned disruption class.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum UpdateDisruption {
    None,
    Reload,
    Restart,
    Recycle,
    Replace,
}

/// Bounded current currency of an owned or dependency resource set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCurrencySet {
    count: u64,
    refs: Vec<ResourceRef>,
}

impl ResourceCurrencySet {
    /// Construct a sorted, unique, possibly truncated resource aggregate.
    pub fn new(count: u64, mut refs: Vec<ResourceRef>) -> Result<Self, ResourceStatusError> {
        refs.sort();
        let original_len = refs.len();
        refs.dedup();
        if refs.len() != original_len {
            return Err(ResourceStatusError::DuplicateResourceRef);
        }
        if refs.len() > MAX_STATUS_COLLECTION_ENTRIES || count < refs.len() as u64 {
            return Err(ResourceStatusError::StatusCollectionTooLarge);
        }
        Ok(Self { count, refs })
    }

    /// Total resources represented, including any truncated references.
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Borrow the bounded canonical reference list.
    pub fn refs(&self) -> &[ResourceRef] {
        &self.refs
    }
}

impl<'de> Deserialize<'de> for ResourceCurrencySet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            count: u64,
            refs: Vec<ResourceRef>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.count, wire.refs).map_err(serde::de::Error::custom)
    }
}

/// Universal update currency object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUpdateStatus {
    state: UpdateState,
    reasons: Vec<UpdateReason>,
    observed_generation: ObservedGeneration,
    target_generation: ResourceGeneration,
    disruption: UpdateDisruption,
    preserve_state: bool,
    operation_id: Option<OperationId>,
    last_assessed_at: Option<Timestamp>,
    owned: ResourceCurrencySet,
    dependencies: ResourceCurrencySet,
}

impl ResourceUpdateStatus {
    /// Construct a canonical current update assessment.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: UpdateState,
        mut reasons: Vec<UpdateReason>,
        observed_generation: ObservedGeneration,
        target_generation: ResourceGeneration,
        disruption: UpdateDisruption,
        preserve_state: bool,
        operation_id: Option<OperationId>,
        last_assessed_at: Option<Timestamp>,
        owned: ResourceCurrencySet,
        dependencies: ResourceCurrencySet,
    ) -> Result<Self, ResourceStatusError> {
        reasons.sort();
        let original_len = reasons.len();
        reasons.dedup();
        if reasons.len() != original_len {
            return Err(ResourceStatusError::DuplicateUpdateReason);
        }
        Ok(Self {
            state,
            reasons,
            observed_generation,
            target_generation,
            disruption,
            preserve_state,
            operation_id,
            last_assessed_at,
            owned,
            dependencies,
        })
    }
}

impl<'de> Deserialize<'de> for ResourceUpdateStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            state: UpdateState,
            reasons: Vec<UpdateReason>,
            observed_generation: ObservedGeneration,
            target_generation: ResourceGeneration,
            disruption: UpdateDisruption,
            preserve_state: bool,
            operation_id: RequiredNullable<OperationId>,
            last_assessed_at: RequiredNullable<Timestamp>,
            owned: ResourceCurrencySet,
            dependencies: ResourceCurrencySet,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.state,
            wire.reasons,
            wire.observed_generation,
            wire.target_generation,
            wire.disruption,
            wire.preserve_state,
            wire.operation_id.0,
            wire.last_assessed_at.0,
            wire.owned,
            wire.dependencies,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Optional Provider-specific status layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusExtension {
    provider_ref: ResourceRef,
    schema_id: ExtensionSchemaId,
    schema_version: SchemaVersion,
    observed_provider_generation: ResourceGeneration,
    details: CanonicalJsonObject,
}

impl ProviderStatusExtension {
    /// Construct a bounded Provider status extension.
    pub fn new(
        provider_ref: ResourceRef,
        schema_id: ExtensionSchemaId,
        schema_version: SchemaVersion,
        observed_provider_generation: ResourceGeneration,
        details: CanonicalJsonObject,
    ) -> Result<Self, ResourceStatusError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(ResourceStatusError::ProviderRefWrongType);
        }
        if schema_id.layer() != ExtensionSchemaLayer::Status {
            return Err(ResourceStatusError::ProviderSchemaWrongLayer);
        }
        ensure_layer_size(&details)?;
        Ok(Self {
            provider_ref,
            schema_id,
            schema_version,
            observed_provider_generation,
            details,
        })
    }

    /// Borrow the writing Provider.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the registered schema ID.
    pub const fn schema_id(&self) -> &ExtensionSchemaId {
        &self.schema_id
    }

    /// Return the registered schema version.
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Borrow the Provider-specific details.
    pub const fn details(&self) -> &CanonicalJsonObject {
        &self.details
    }
}

impl core::fmt::Debug for ProviderStatusExtension {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderStatusExtension")
            .field("provider_ref", &self.provider_ref)
            .field("schema_id", &self.schema_id)
            .field("schema_version", &self.schema_version)
            .field(
                "observed_provider_generation",
                &self.observed_provider_generation,
            )
            .field("details", &"<redacted>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for ProviderStatusExtension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            schema_id: ExtensionSchemaId,
            schema_version: SchemaVersion,
            observed_provider_generation: ResourceGeneration,
            details: CanonicalJsonObject,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.provider_ref,
            wire.schema_id,
            wire.schema_version,
            wire.observed_provider_generation,
            wire.details,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Complete universal status plus ResourceType and optional Provider layers.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    observed_generation: ObservedGeneration,
    phase: ResourcePhase,
    conditions: Vec<ResourceCondition>,
    last_reconciled_at: Option<Timestamp>,
    started_at: Option<Timestamp>,
    completed_at: Option<Timestamp>,
    outcome: Option<ResourceOutcome>,
    update: ResourceUpdateStatus,
    resource: CanonicalJsonObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderStatusExtension>,
}

impl ResourceStatus {
    /// Construct and bound all three status layers.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        observed_generation: ObservedGeneration,
        phase: ResourcePhase,
        mut conditions: Vec<ResourceCondition>,
        last_reconciled_at: Option<Timestamp>,
        started_at: Option<Timestamp>,
        completed_at: Option<Timestamp>,
        outcome: Option<ResourceOutcome>,
        update: ResourceUpdateStatus,
        resource: CanonicalJsonObject,
        provider: Option<ProviderStatusExtension>,
    ) -> Result<Self, ResourceStatusError> {
        if conditions.len() > MAX_STATUS_CONDITIONS {
            return Err(ResourceStatusError::TooManyConditions);
        }
        conditions.sort_by(|left, right| left.condition_type.cmp(&right.condition_type));
        if conditions
            .windows(2)
            .any(|pair| pair[0].condition_type == pair[1].condition_type)
        {
            return Err(ResourceStatusError::DuplicateCondition);
        }
        ensure_layer_size(&resource)?;
        let value = Self {
            observed_generation,
            phase,
            conditions,
            last_reconciled_at,
            started_at,
            completed_at,
            outcome,
            update,
            resource,
            provider,
        };
        if canonical_json_bytes(&value)
            .map_err(|_| ResourceStatusError::InvalidStatusString)?
            .len()
            > MAX_STATUS_BYTES
        {
            return Err(ResourceStatusError::StatusTooLarge);
        }
        Ok(value)
    }

    /// Return the latest accounted-for spec generation.
    pub const fn observed_generation(&self) -> ObservedGeneration {
        self.observed_generation
    }

    /// Return the universal phase.
    pub const fn phase(&self) -> ResourcePhase {
        self.phase
    }

    /// Borrow the ResourceType-common layer, always present.
    pub const fn resource(&self) -> &CanonicalJsonObject {
        &self.resource
    }

    /// Borrow the optional Provider-specific layer.
    pub fn provider(&self) -> Option<&ProviderStatusExtension> {
        self.provider.as_ref()
    }

    /// Borrow update currency.
    pub const fn update(&self) -> &ResourceUpdateStatus {
        &self.update
    }

    /// Produce the base-only projection used by generic tooling.
    pub fn base_projection(&self) -> Self {
        let mut projected = self.clone();
        projected.provider = None;
        projected
    }
}

impl core::fmt::Debug for ResourceStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceStatus")
            .field("observed_generation", &self.observed_generation)
            .field("phase", &self.phase)
            .field("conditions", &self.conditions.len())
            .field("resource", &"<redacted>")
            .field("provider", &self.provider.is_some())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ResourceStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            observed_generation: ObservedGeneration,
            phase: ResourcePhase,
            conditions: Vec<ResourceCondition>,
            last_reconciled_at: RequiredNullable<Timestamp>,
            started_at: RequiredNullable<Timestamp>,
            completed_at: RequiredNullable<Timestamp>,
            outcome: RequiredNullable<ResourceOutcome>,
            update: ResourceUpdateStatus,
            resource: CanonicalJsonObject,
            provider: Option<ProviderStatusExtension>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.observed_generation,
            wire.phase,
            wire.conditions,
            wire.last_reconciled_at.0,
            wire.started_at.0,
            wire.completed_at.0,
            wire.outcome.0,
            wire.update,
            wire.resource,
            wire.provider,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn ensure_layer_size(value: &CanonicalJsonObject) -> Result<(), ResourceStatusError> {
    if value.to_canonical_bytes().len() > MAX_STATUS_LAYER_BYTES {
        return Err(ResourceStatusError::StatusLayerTooLarge);
    }
    if value.len() > MAX_STATUS_COLLECTION_ENTRIES {
        return Err(ResourceStatusError::StatusCollectionTooLarge);
    }
    for value in value.values() {
        ensure_dynamic_bounds(value)?;
    }
    Ok(())
}

fn ensure_dynamic_bounds(
    value: &super::resource_schema::CanonicalJsonValue,
) -> Result<(), ResourceStatusError> {
    use super::resource_schema::CanonicalJsonValue;

    match value {
        CanonicalJsonValue::String(value) if value.len() > MAX_STATUS_STRING_BYTES => {
            Err(ResourceStatusError::StatusStringTooLong)
        }
        CanonicalJsonValue::Array(values) => {
            if values.len() > MAX_STATUS_COLLECTION_ENTRIES {
                return Err(ResourceStatusError::StatusCollectionTooLarge);
            }
            for value in values {
                ensure_dynamic_bounds(value)?;
            }
            Ok(())
        }
        CanonicalJsonValue::Object(values) => {
            if values.len() > MAX_STATUS_COLLECTION_ENTRIES {
                return Err(ResourceStatusError::StatusCollectionTooLarge);
            }
            for value in values.values() {
                ensure_dynamic_bounds(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

struct RequiredNullable<T>(Option<T>);

impl<'de, T> Deserialize<'de> for RequiredNullable<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Self)
    }
}

/// Failure to construct a bounded resource status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceStatusError {
    InvalidStatusCode,
    InvalidStatusString,
    StatusStringTooLong,
    InvalidRetryAfter,
    TooManyConditions,
    DuplicateCondition,
    DuplicateUpdateReason,
    DuplicateResourceRef,
    StatusCollectionTooLarge,
    StatusLayerTooLarge,
    StatusTooLarge,
    ProviderRefWrongType,
    ProviderSchemaWrongLayer,
}

impl core::fmt::Display for ResourceStatusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidStatusCode => {
                f.write_str("status code must be lower-kebab at 1 to 64 bytes")
            }
            Self::InvalidStatusString => f.write_str("status string contains forbidden characters"),
            Self::StatusStringTooLong => f.write_str("status string exceeds 4096 bytes"),
            Self::InvalidRetryAfter => f.write_str("retryAfterMs must be between 1 and 86400000"),
            Self::TooManyConditions => f.write_str("status has more than 32 conditions"),
            Self::DuplicateCondition => f.write_str("status condition types must be unique"),
            Self::DuplicateUpdateReason => f.write_str("update reasons must be unique"),
            Self::DuplicateResourceRef => f.write_str("resource currency refs must be unique"),
            Self::StatusCollectionTooLarge => f.write_str("status collection exceeds its bound"),
            Self::StatusLayerTooLarge => f.write_str("typed status layer exceeds 32 KiB"),
            Self::StatusTooLarge => f.write_str("complete status exceeds 64 KiB"),
            Self::ProviderRefWrongType => f.write_str("providerRef must reference Provider"),
            Self::ProviderSchemaWrongLayer => {
                f.write_str("Provider schema ID is not a status schema")
            }
        }
    }
}

impl std::error::Error for ResourceStatusError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> Timestamp {
        Timestamp::parse("2026-07-22T00:00:01.000Z").unwrap()
    }

    fn empty_set() -> ResourceCurrencySet {
        ResourceCurrencySet::new(0, Vec::new()).unwrap()
    }

    fn update() -> ResourceUpdateStatus {
        ResourceUpdateStatus::new(
            UpdateState::Current,
            vec![UpdateReason::SpecChanged],
            ObservedGeneration::new(1),
            ResourceGeneration::new(1).unwrap(),
            UpdateDisruption::None,
            true,
            None,
            Some(timestamp()),
            empty_set(),
            empty_set(),
        )
        .unwrap()
    }

    #[test]
    fn status_update_literal_json_round_trip_pins_wire_names() {
        const JSON: &str = concat!(
            "{\"dependencies\":{\"count\":0,\"refs\":[]},\"disruption\":\"None\",",
            "\"lastAssessedAt\":\"2026-07-22T00:00:01.000Z\",",
            "\"observedGeneration\":1,\"operationId\":null,",
            "\"owned\":{\"count\":0,\"refs\":[]},",
            "\"preserveState\":true,\"reasons\":[\"SpecChanged\"],",
            "\"state\":\"Current\",\"targetGeneration\":1}"
        );
        let parsed: ResourceUpdateStatus = serde_json::from_str(JSON).unwrap();
        assert_eq!(canonical_json_bytes(&parsed).unwrap(), JSON.as_bytes());
    }

    #[test]
    fn base_projection_keeps_resource_and_omits_provider() {
        let provider = ProviderStatusExtension::new(
            ResourceRef::parse("Provider/runtime-qemu-media").unwrap(),
            ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/status").unwrap(),
            SchemaVersion::parse("1.0").unwrap(),
            ResourceGeneration::new(2).unwrap(),
            CanonicalJsonObject::parse(br#"{"backend":"qemu"}"#).unwrap(),
        )
        .unwrap();
        let status = ResourceStatus::new(
            ObservedGeneration::new(1),
            ResourcePhase::Ready,
            Vec::new(),
            Some(timestamp()),
            Some(timestamp()),
            None,
            None,
            update(),
            CanonicalJsonObject::empty(),
            Some(provider),
        )
        .unwrap();

        let projected = status.base_projection();
        assert!(projected.provider().is_none());
        assert_eq!(projected.resource(), &CanonicalJsonObject::empty());
        let json = canonical_json_bytes(&projected).unwrap();
        assert!(!String::from_utf8(json).unwrap().contains("\"provider\""));
    }

    #[test]
    fn status_bounds_and_time_shape_fail_closed() {
        assert!(StatusMessage::parse("x".repeat(MAX_STATUS_STRING_BYTES + 1)).is_err());
        assert!(Timestamp::parse("2026-07-22T00:00:01Z").is_err());
        assert!(
            ResourceCurrencySet::new(65, vec![ResourceRef::parse("Host/a").unwrap(); 65]).is_err()
        );
        assert!(
            ResourceOutcome::new(
                StatusCode::parse("retry").unwrap(),
                None,
                StatusMessage::parse("retry later").unwrap(),
                true,
                Some(0),
                timestamp(),
            )
            .is_err()
        );
    }

    #[test]
    fn status_debug_redacts_dynamic_and_message_values() {
        let secret_marker = StatusMessage::parse("operator-only detail").unwrap();
        assert!(!format!("{secret_marker:?}").contains("operator-only"));
        let status = ResourceStatus::new(
            ObservedGeneration::new(0),
            ResourcePhase::Pending,
            Vec::new(),
            None,
            None,
            None,
            None,
            update(),
            CanonicalJsonObject::parse(br#"{"privateObservation":"hidden"}"#).unwrap(),
            None,
        )
        .unwrap();
        assert!(!format!("{status:?}").contains("hidden"));
    }

    #[test]
    fn unknown_status_and_provider_fields_are_rejected() {
        let provider = r#"{
            "providerRef":"Provider/runtime-qemu-media",
            "schemaId":"runtime-qemu-media.d2bus.org/Guest/status",
            "schemaVersion":"1.0",
            "observedProviderGeneration":1,
            "details":{},
            "unexpected":true
        }"#;
        assert!(serde_json::from_str::<ProviderStatusExtension>(provider).is_err());

        let status = r#"{
            "observedGeneration":0,
            "phase":"Pending",
            "conditions":[],
            "lastReconciledAt":null,
            "startedAt":null,
            "completedAt":null,
            "outcome":null,
            "update":{
                "state":"Unknown","reasons":[],"observedGeneration":0,
                "targetGeneration":1,"disruption":"None","preserveState":true,
                "operationId":null,"lastAssessedAt":null,
                "owned":{"count":0,"refs":[]},"dependencies":{"count":0,"refs":[]}
            },
            "resource":{},
            "unexpected":true
        }"#;
        assert!(serde_json::from_str::<ResourceStatus>(status).is_err());

        let oversized_array = format!(
            "{{\"items\":[{}]}}",
            (0..65)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(
            ProviderStatusExtension::new(
                ResourceRef::parse("Provider/runtime-qemu-media").unwrap(),
                ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/status").unwrap(),
                SchemaVersion::parse("1.0").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                CanonicalJsonObject::parse(oversized_array.as_bytes()).unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn phase_wire_values_are_closed_and_case_sensitive() {
        for phase in [
            "Pending",
            "Ready",
            "Succeeded",
            "Degraded",
            "Failed",
            "Deleted",
            "Unknown",
        ] {
            assert!(serde_json::from_str::<ResourcePhase>(&format!("\"{phase}\"")).is_ok());
        }
        for phase in ["pending", "Starting", "Deleting", ""] {
            assert!(serde_json::from_str::<ResourcePhase>(&format!("\"{phase}\"")).is_err());
        }
    }
}
