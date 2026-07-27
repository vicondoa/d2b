//! Strict v3 resource envelope, metadata, and desired-state layers.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

use super::{
    ConfigurationGeneration, ControllerGeneration, ResourceGeneration, ResourceName, ResourceRef,
    ResourceStatus, ResourceTypeName, ResourceUid, Timestamp, ZoneId, ZoneRevision,
    resource_schema::{
        CanonicalJsonCodecReason, CanonicalJsonError, CanonicalJsonObject, CanonicalJsonValue,
        ExtensionSchemaId, ExtensionSchemaLayer, RESOURCE_ENVELOPE_DOMAIN_TAG,
        RESOURCE_SPEC_DOMAIN_TAG, SchemaVersion, canonical_digest, canonical_json_bytes,
        serde_json_error_metadata, validate_canonical_string,
    },
};

/// Resource API version carried by every complete envelope.
pub const RESOURCE_API_VERSION: &str = "resources.d2bus.org/v3";
/// Maximum canonical bytes in one complete resource envelope.
pub const MAX_RESOURCE_ENVELOPE_BYTES: usize = 256 * 1024;
/// Maximum finalizers on one resource.
pub const MAX_FINALIZERS: usize = 8;
/// Maximum bytes in one finalizer ID.
pub const MAX_FINALIZER_ID_BYTES: usize = 128;
/// Maximum labels or annotations on one resource.
pub const MAX_PRESENTATION_METADATA_ENTRIES: usize = 32;
/// Maximum bytes in one metadata key.
pub const MAX_METADATA_KEY_BYTES: usize = 64;
/// Maximum bytes in one label value.
pub const MAX_LABEL_VALUE_BYTES: usize = 256;
/// Maximum bytes in one annotation value.
pub const MAX_ANNOTATION_VALUE_BYTES: usize = 4 * 1024;
/// Maximum aggregate bytes in annotations.
pub const MAX_ANNOTATIONS_BYTES: usize = 16 * 1024;
/// A validated core- or Provider-owned finalizer ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct FinalizerId(String);

impl FinalizerId {
    /// Parse one of the two admitted finalizer forms.
    pub fn parse(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_FINALIZER_ID_BYTES {
            return Err(ResourceError::InvalidFinalizer);
        }
        let valid = if let Some(name) = value.strip_prefix("core.") {
            is_lower_name(name)
        } else if let Some((namespace, name)) = value.split_once(".d2bus.org/") {
            !name.contains('/')
                && ResourceName::parse(namespace).is_ok()
                && name.len() <= 63
                && is_lower_name(name)
        } else {
            false
        };
        if !valid {
            return Err(ResourceError::InvalidFinalizer);
        }
        Ok(Self(value))
    }

    /// Borrow the canonical ID.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Render the canonical ID for an authorized encoding or key surface.
    pub fn to_canonical_string(&self) -> String {
        self.0.clone()
    }

    /// Render the canonical ID when explicitly requested.
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        self.to_canonical_string()
    }
}

impl core::fmt::Display for FinalizerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FinalizerId(<redacted>)")
    }
}

impl core::fmt::Debug for FinalizerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FinalizerId(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for FinalizerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for FinalizerId {
    fn schema_name() -> String {
        "FinalizerId".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().min_length = Some(1);
        schema.string().max_length = Some(MAX_FINALIZER_ID_BYTES as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// Core authority that manages a resource's lifecycle.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ManagedBy {
    Configuration,
    Controller,
    Api,
}

/// Bounded labels and annotations used only for presentation and exact-match indexes.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PresentationMetadata {
    labels: BTreeMap<String, String>,
    annotations: BTreeMap<String, String>,
}

impl PresentationMetadata {
    /// Construct presentation metadata after validating all bounds.
    pub fn new(
        labels: BTreeMap<String, String>,
        annotations: BTreeMap<String, String>,
    ) -> Result<Self, ResourceError> {
        if labels.len() > MAX_PRESENTATION_METADATA_ENTRIES
            || annotations.len() > MAX_PRESENTATION_METADATA_ENTRIES
        {
            return Err(ResourceError::PresentationMetadataTooLarge);
        }
        for (key, value) in &labels {
            validate_metadata_key(key)?;
            validate_metadata_value(value, MAX_LABEL_VALUE_BYTES)?;
        }
        let mut annotation_bytes = 0usize;
        for (key, value) in &annotations {
            validate_metadata_key(key)?;
            validate_metadata_value(value, MAX_ANNOTATION_VALUE_BYTES)?;
            annotation_bytes = annotation_bytes
                .checked_add(key.len())
                .and_then(|size| size.checked_add(value.len()))
                .ok_or(ResourceError::PresentationMetadataTooLarge)?;
        }
        if annotation_bytes > MAX_ANNOTATIONS_BYTES {
            return Err(ResourceError::PresentationMetadataTooLarge);
        }
        Ok(Self {
            labels,
            annotations,
        })
    }

    /// Borrow labels.
    pub const fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }

    /// Borrow annotations.
    pub const fn annotations(&self) -> &BTreeMap<String, String> {
        &self.annotations
    }
}

impl core::fmt::Debug for PresentationMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PresentationMetadata")
            .field("labels", &self.labels.len())
            .field("annotations", &self.annotations.len())
            .finish()
    }
}

/// Strict resource metadata.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetadata {
    name: ResourceName,
    zone: ZoneId,
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
    owner_ref: Option<ResourceRef>,
    finalizers: Vec<FinalizerId>,
    deletion_requested_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
    managed_by: ManagedBy,
    #[serde(skip_serializing_if = "Option::is_none")]
    configuration_generation: Option<ConfigurationGeneration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    controller_generation: Option<ControllerGeneration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_generation: Option<ResourceGeneration>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    labels: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    annotations: BTreeMap<String, String>,
}

impl ResourceMetadata {
    /// Construct metadata from store-authoritative fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: ResourceName,
        zone: ZoneId,
        uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
        owner_ref: Option<ResourceRef>,
        mut finalizers: Vec<FinalizerId>,
        deletion_requested_at: Option<Timestamp>,
        created_at: Timestamp,
        updated_at: Timestamp,
        managed_by: ManagedBy,
        configuration_generation: Option<ConfigurationGeneration>,
        controller_generation: Option<ControllerGeneration>,
        provider_generation: Option<ResourceGeneration>,
        presentation: PresentationMetadata,
    ) -> Result<Self, ResourceError> {
        if finalizers.len() > MAX_FINALIZERS {
            return Err(ResourceError::TooManyFinalizers);
        }
        finalizers.sort();
        let original_len = finalizers.len();
        finalizers.dedup();
        if finalizers.len() != original_len {
            return Err(ResourceError::DuplicateFinalizer);
        }
        if updated_at < created_at
            || deletion_requested_at
                .as_ref()
                .is_some_and(|deleted| deleted < &created_at)
        {
            return Err(ResourceError::InvalidTimestampOrder);
        }
        if managed_by == ManagedBy::Configuration && configuration_generation.is_none() {
            return Err(ResourceError::ConfigurationGenerationRequired);
        }
        Ok(Self {
            name,
            zone,
            uid,
            generation,
            revision,
            owner_ref,
            finalizers,
            deletion_requested_at,
            created_at,
            updated_at,
            managed_by,
            configuration_generation,
            controller_generation,
            provider_generation,
            labels: presentation.labels,
            annotations: presentation.annotations,
        })
    }

    /// Borrow the Zone-local name.
    pub const fn name(&self) -> &ResourceName {
        &self.name
    }

    /// Borrow the containing Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the immutable store-generated UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the current desired-state generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Return the latest Zone revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Borrow the optional singular owner reference.
    pub fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }
}

impl core::fmt::Debug for ResourceMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceMetadata(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for ResourceMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            name: ResourceName,
            zone: ZoneId,
            uid: ResourceUid,
            generation: ResourceGeneration,
            revision: ZoneRevision,
            owner_ref: RequiredNullable<ResourceRef>,
            finalizers: Vec<FinalizerId>,
            deletion_requested_at: RequiredNullable<Timestamp>,
            created_at: Timestamp,
            updated_at: Timestamp,
            managed_by: ManagedBy,
            configuration_generation: Option<ConfigurationGeneration>,
            controller_generation: Option<ControllerGeneration>,
            provider_generation: Option<ResourceGeneration>,
            #[serde(default)]
            labels: BTreeMap<String, String>,
            #[serde(default)]
            annotations: BTreeMap<String, String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let presentation = PresentationMetadata::new(wire.labels, wire.annotations)
            .map_err(serde::de::Error::custom)?;
        Self::new(
            wire.name,
            wire.zone,
            wire.uid,
            wire.generation,
            wire.revision,
            wire.owner_ref.0,
            wire.finalizers,
            wire.deletion_requested_at.0,
            wire.created_at,
            wire.updated_at,
            wire.managed_by,
            wire.configuration_generation,
            wire.controller_generation,
            wire.provider_generation,
            presentation,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Policy for disruptive desired-state changes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DisruptiveUpdateMode {
    Manual,
}

/// Policy for changes assessed as non-disruptive.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum NonDisruptiveUpdateMode {
    Automatic,
    Manual,
}

/// Provider-neutral update policy in the ResourceType base spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdatePolicy {
    disruptive: DisruptiveUpdateMode,
    non_disruptive: NonDisruptiveUpdateMode,
}

impl UpdatePolicy {
    /// Construct an explicit update policy.
    pub const fn new(
        disruptive: DisruptiveUpdateMode,
        non_disruptive: NonDisruptiveUpdateMode,
    ) -> Self {
        Self {
            disruptive,
            non_disruptive,
        }
    }
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self::new(
            DisruptiveUpdateMode::Manual,
            NonDisruptiveUpdateMode::Automatic,
        )
    }
}

/// Optional Provider-specific desired-state layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpecExtension {
    schema_id: ExtensionSchemaId,
    schema_version: SchemaVersion,
    settings: CanonicalJsonObject,
}

impl ProviderSpecExtension {
    /// Construct a Provider spec extension bound to the spec layer.
    pub fn new(
        schema_id: ExtensionSchemaId,
        schema_version: SchemaVersion,
        settings: CanonicalJsonObject,
    ) -> Result<Self, ResourceError> {
        if schema_id.layer() != ExtensionSchemaLayer::Spec {
            return Err(ResourceError::ProviderSchemaWrongLayer);
        }
        Ok(Self {
            schema_id,
            schema_version,
            settings,
        })
    }

    /// Borrow the registered schema ID.
    pub const fn schema_id(&self) -> &ExtensionSchemaId {
        &self.schema_id
    }

    /// Return the registered schema version.
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Borrow the desired settings object.
    pub const fn settings(&self) -> &CanonicalJsonObject {
        &self.settings
    }
}

impl core::fmt::Debug for ProviderSpecExtension {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProviderSpecExtension(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for ProviderSpecExtension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            schema_id: ExtensionSchemaId,
            schema_version: SchemaVersion,
            settings: CanonicalJsonObject,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.schema_id, wire.schema_version, wire.settings)
            .map_err(serde::de::Error::custom)
    }
}

/// ResourceType base spec fields plus the optional Provider extension.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceSpec {
    provider_ref: Option<ResourceRef>,
    update_policy: Option<UpdatePolicy>,
    base: CanonicalJsonObject,
    provider: Option<ProviderSpecExtension>,
}

impl ResourceSpec {
    /// Construct all desired-state layers.
    pub fn new(
        provider_ref: Option<ResourceRef>,
        update_policy: Option<UpdatePolicy>,
        base: CanonicalJsonObject,
        provider: Option<ProviderSpecExtension>,
    ) -> Result<Self, ResourceError> {
        for key in base.keys() {
            if matches!(key, "provider" | "providerRef" | "updatePolicy") {
                return Err(ResourceError::BaseFieldReserved);
            }
        }
        if provider.is_some() && provider_ref.is_none() {
            return Err(ResourceError::ProviderRefRequired);
        }
        if provider_ref
            .as_ref()
            .is_some_and(|reference| reference.resource_type().as_str() != "Provider")
        {
            return Err(ResourceError::ProviderRefWrongType);
        }
        Ok(Self {
            provider_ref,
            update_policy,
            base,
            provider,
        })
    }

    /// Construct an empty base spec.
    pub fn empty() -> Self {
        Self::new(None, None, CanonicalJsonObject::empty(), None)
            .expect("empty spec is always valid")
    }

    /// Borrow the selected Provider reference.
    pub fn provider_ref(&self) -> Option<&ResourceRef> {
        self.provider_ref.as_ref()
    }

    /// Borrow ResourceType-specific base fields.
    pub const fn base(&self) -> &CanonicalJsonObject {
        &self.base
    }

    /// Borrow the optional Provider extension.
    pub fn provider(&self) -> Option<&ProviderSpecExtension> {
        self.provider.as_ref()
    }

    /// Return the provider-neutral update policy, if explicitly stored.
    pub const fn update_policy(&self) -> Option<UpdatePolicy> {
        self.update_policy
    }

    /// Render the canonical desired-state bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalJsonError> {
        canonical_json_bytes(self)
    }

    /// Return the domain-separated desired-state digest.
    pub fn digest(&self) -> Result<String, CanonicalJsonError> {
        Ok(canonical_digest(
            RESOURCE_SPEC_DOMAIN_TAG,
            &self.canonical_bytes()?,
        ))
    }
}

impl core::fmt::Debug for ResourceSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceSpec(<redacted>)")
    }
}

impl Serialize for ResourceSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(
            self.base.len()
                + usize::from(self.provider_ref.is_some())
                + usize::from(self.update_policy.is_some())
                + usize::from(self.provider.is_some()),
        ))?;
        if let Some(provider_ref) = &self.provider_ref {
            map.serialize_entry("providerRef", provider_ref)?;
        }
        if let Some(update_policy) = &self.update_policy {
            map.serialize_entry("updatePolicy", update_policy)?;
        }
        for key in self.base.keys() {
            map.serialize_entry(
                key,
                self.base
                    .get(key)
                    .expect("key returned by canonical object"),
            )?;
        }
        if let Some(provider) = &self.provider {
            map.serialize_entry("provider", provider)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ResourceSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let object = CanonicalJsonObject::deserialize(deserializer)?;
        let mut fields = object.into_inner();
        let provider_ref = remove_typed(&mut fields, "providerRef")?;
        let update_policy = remove_typed(&mut fields, "updatePolicy")?;
        let provider = remove_typed(&mut fields, "provider")?;
        Self::new(
            provider_ref,
            update_policy,
            CanonicalJsonObject::from_inner(fields),
            provider,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ResourceSpec {
    fn schema_name() -> String {
        "ResourceSpec".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::Object,
            ))),
            ..Default::default()
        })
    }
}

fn remove_typed<T, E>(
    fields: &mut BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<T>, E>
where
    T: for<'de> Deserialize<'de>,
    E: serde::de::Error,
{
    fields
        .remove(key)
        .map(|value| serde_json::from_slice(&value.to_canonical_bytes()).map_err(E::custom))
        .transpose()
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

/// Complete strict resource envelope.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEnvelope {
    api_version: String,
    #[serde(rename = "type")]
    resource_type: ResourceTypeName,
    metadata: ResourceMetadata,
    spec: ResourceSpec,
    status: ResourceStatus,
}

impl ResourceEnvelope {
    /// Construct and validate a complete resource envelope.
    pub fn new(
        resource_type: ResourceTypeName,
        metadata: ResourceMetadata,
        spec: ResourceSpec,
        status: ResourceStatus,
    ) -> Result<Self, ResourceError> {
        let self_ref = ResourceRef::new(resource_type.clone(), metadata.name.clone());
        if metadata.owner_ref.as_ref() == Some(&self_ref) {
            return Err(ResourceError::SelfOwner);
        }
        if status.observed_generation().get() > metadata.generation.get() {
            return Err(ResourceError::ObservedGenerationAhead);
        }
        validate_provider_binding(&resource_type, &spec, &status)?;
        let envelope = Self {
            api_version: RESOURCE_API_VERSION.to_owned(),
            resource_type,
            metadata,
            spec,
            status,
        };
        if envelope
            .canonical_bytes()
            .map_err(ResourceError::CanonicalJson)?
            .len()
            > MAX_RESOURCE_ENVELOPE_BYTES
        {
            return Err(ResourceError::EnvelopeTooLarge);
        }
        Ok(envelope)
    }

    /// Parse a complete envelope through the duplicate-rejecting canonical profile.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ResourceError> {
        CanonicalJsonValue::parse(bytes).map_err(ResourceError::CanonicalJson)?;
        serde_json::from_slice(bytes).map_err(|error| {
            let (reason, line, column) = serde_json_error_metadata(&error);
            ResourceError::Serde {
                reason,
                line,
                column,
            }
        })
    }

    /// Borrow the ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Borrow metadata.
    pub const fn metadata(&self) -> &ResourceMetadata {
        &self.metadata
    }

    /// Borrow desired state.
    pub const fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    /// Borrow observed state.
    pub const fn status(&self) -> &ResourceStatus {
        &self.status
    }

    /// Render exact `d2b-cjson/v1` bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalJsonError> {
        canonical_json_bytes(self)
    }

    /// Return the domain-separated complete-envelope digest.
    pub fn digest(&self) -> Result<String, CanonicalJsonError> {
        Ok(canonical_digest(
            RESOURCE_ENVELOPE_DOMAIN_TAG,
            &self.canonical_bytes()?,
        ))
    }
}

impl core::fmt::Debug for ResourceEnvelope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceEnvelope(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for ResourceEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            api_version: String,
            #[serde(rename = "type")]
            resource_type: ResourceTypeName,
            metadata: ResourceMetadata,
            spec: ResourceSpec,
            status: ResourceStatus,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.api_version != RESOURCE_API_VERSION {
            return Err(serde::de::Error::custom(
                "apiVersion must be resources.d2bus.org/v3",
            ));
        }
        Self::new(wire.resource_type, wire.metadata, wire.spec, wire.status)
            .map_err(serde::de::Error::custom)
    }
}

fn validate_provider_binding(
    resource_type: &ResourceTypeName,
    spec: &ResourceSpec,
    status: &ResourceStatus,
) -> Result<(), ResourceError> {
    let selected = spec.provider_ref();
    if let Some(provider) = spec.provider() {
        let selected = selected.ok_or(ResourceError::ProviderRefRequired)?;
        if provider.schema_id().provider_name() != selected.name()
            || provider.schema_id().resource_type() != resource_type
        {
            return Err(ResourceError::ProviderSchemaBinding);
        }
    }
    if let Some(provider) = status.provider() {
        let selected = selected.ok_or(ResourceError::ProviderRefRequired)?;
        if provider.provider_ref() != selected
            || provider.schema_id().provider_name() != selected.name()
            || provider.schema_id().resource_type() != resource_type
        {
            return Err(ResourceError::ProviderSchemaBinding);
        }
    }
    Ok(())
}

fn validate_metadata_key(key: &str) -> Result<(), ResourceError> {
    if key.is_empty()
        || key.len() > MAX_METADATA_KEY_BYTES
        || key.matches('/').count() > 1
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(ResourceError::InvalidMetadataKey);
    }
    Ok(())
}

fn validate_metadata_value(value: &str, max: usize) -> Result<(), ResourceError> {
    if value.len() > max || validate_canonical_string(value).is_err() {
        return Err(ResourceError::InvalidMetadataValue);
    }
    Ok(())
}

fn is_lower_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Invalid resource envelope or metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    CanonicalJson(CanonicalJsonError),
    Serde {
        reason: CanonicalJsonCodecReason,
        line: u32,
        column: u32,
    },
    InvalidFinalizer,
    TooManyFinalizers,
    DuplicateFinalizer,
    InvalidTimestampOrder,
    ConfigurationGenerationRequired,
    PresentationMetadataTooLarge,
    InvalidMetadataKey,
    InvalidMetadataValue,
    BaseFieldReserved,
    ProviderRefRequired,
    ProviderRefWrongType,
    ProviderSchemaWrongLayer,
    ProviderSchemaBinding,
    SelfOwner,
    ObservedGenerationAhead,
    EnvelopeTooLarge,
}

impl core::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CanonicalJson(error) => error.fmt(f),
            Self::Serde {
                reason,
                line,
                column,
            } => write!(
                f,
                "invalid resource envelope: {} at line {line}, column {column}",
                reason.as_str()
            ),
            Self::InvalidFinalizer => f.write_str("invalid finalizer ID"),
            Self::TooManyFinalizers => f.write_str("resource has more than 8 finalizers"),
            Self::DuplicateFinalizer => f.write_str("resource finalizers must be unique"),
            Self::InvalidTimestampOrder => f.write_str("resource timestamps are out of order"),
            Self::ConfigurationGenerationRequired => {
                f.write_str("configuration-managed resource requires configurationGeneration")
            }
            Self::PresentationMetadataTooLarge => {
                f.write_str("labels or annotations exceed their bounds")
            }
            Self::InvalidMetadataKey => f.write_str("invalid label or annotation key"),
            Self::InvalidMetadataValue => f.write_str("invalid label or annotation value"),
            Self::BaseFieldReserved => {
                f.write_str("ResourceType base fields must not shadow universal spec fields")
            }
            Self::ProviderRefRequired => {
                f.write_str("Provider extension requires spec.providerRef")
            }
            Self::ProviderRefWrongType => f.write_str("spec.providerRef must reference Provider"),
            Self::ProviderSchemaWrongLayer => {
                f.write_str("Provider schema ID is not a spec schema")
            }
            Self::ProviderSchemaBinding => {
                f.write_str("Provider extension is not bound to spec.providerRef and ResourceType")
            }
            Self::SelfOwner => f.write_str("resource cannot own itself"),
            Self::ObservedGenerationAhead => {
                f.write_str("status observedGeneration exceeds metadata generation")
            }
            Self::EnvelopeTooLarge => f.write_str("resource envelope exceeds 256 KiB"),
        }
    }
}

impl std::error::Error for ResourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CanonicalJson(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{
        ObservedGeneration, ResourceCurrencySet, ResourcePhase, ResourceUpdateStatus,
        UpdateDisruption, UpdateState,
    };

    const GOLDEN_ENVELOPE: &[u8] = br#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"host-system","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}},"type":"Host"}"#;

    fn empty_currency() -> ResourceCurrencySet {
        ResourceCurrencySet::new(0, Vec::new()).unwrap()
    }

    fn status(provider: Option<super::super::ProviderStatusExtension>) -> ResourceStatus {
        ResourceStatus::new(
            ObservedGeneration::new(0),
            ResourcePhase::Pending,
            Vec::new(),
            None,
            None,
            None,
            None,
            ResourceUpdateStatus::new(
                UpdateState::Unknown,
                Vec::new(),
                ObservedGeneration::new(0),
                ResourceGeneration::new(1).unwrap(),
                UpdateDisruption::None,
                true,
                None,
                None,
                empty_currency(),
                empty_currency(),
            )
            .unwrap(),
            CanonicalJsonObject::empty(),
            provider,
        )
        .unwrap()
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceName::parse("host-system").unwrap(),
            ZoneId::parse("dev").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ZoneRevision::new(1),
            None,
            Vec::new(),
            None,
            Timestamp::parse("2026-07-22T00:00:00.000Z").unwrap(),
            Timestamp::parse("2026-07-22T00:00:00.000Z").unwrap(),
            ManagedBy::Configuration,
            Some(ConfigurationGeneration::new(7).unwrap()),
            None,
            None,
            PresentationMetadata::default(),
        )
        .unwrap()
    }

    fn base_spec() -> ResourceSpec {
        ResourceSpec::new(
            Some(ResourceRef::parse("Provider/system-core").unwrap()),
            Some(UpdatePolicy::default()),
            CanonicalJsonObject::empty(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn golden_json_vector_pins_literal_envelope_bytes() {
        let parsed = ResourceEnvelope::from_json(GOLDEN_ENVELOPE).unwrap();
        assert_eq!(parsed.canonical_bytes().unwrap(), GOLDEN_ENVELOPE);

        let constructed = ResourceEnvelope::new(
            ResourceTypeName::parse("Host").unwrap(),
            metadata(),
            base_spec(),
            status(None),
        )
        .unwrap();
        assert_eq!(constructed.canonical_bytes().unwrap(), GOLDEN_ENVELOPE);
    }

    #[test]
    fn three_layer_spec_round_trip_and_unknown_envelopes() {
        let spec_json = br#"{
            "providerRef":"Provider/runtime-qemu-media",
            "updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"},
            "imageId":"guest-system",
            "provider":{
                "schemaId":"runtime-qemu-media.d2bus.org/Guest/spec",
                "schemaVersion":"1.0",
                "settings":{"machine":"microvm"}
            }
        }"#;
        let spec: ResourceSpec = serde_json::from_slice(spec_json).unwrap();
        assert_eq!(
            spec.provider_ref().unwrap().to_string(),
            "Provider/runtime-qemu-media"
        );
        assert_eq!(
            spec.base().get("imageId"),
            Some(&CanonicalJsonValue::String("guest-system".to_owned()))
        );
        assert!(spec.provider().is_some());
        let reparsed: ResourceSpec =
            serde_json::from_slice(&spec.canonical_bytes().unwrap()).unwrap();
        assert_eq!(reparsed, spec);

        let extension = r#"{
            "schemaId":"runtime-qemu-media.d2bus.org/Guest/spec",
            "schemaVersion":"1.0","settings":{},"unknown":true
        }"#;
        assert!(serde_json::from_str::<ProviderSpecExtension>(extension).is_err());
        let envelope = String::from_utf8(GOLDEN_ENVELOPE.to_vec())
            .unwrap()
            .replace("\"type\":\"Host\"", "\"type\":\"Host\",\"unknown\":true");
        assert!(ResourceEnvelope::from_json(envelope.as_bytes()).is_err());
    }

    #[test]
    fn provider_extension_is_bound_to_selected_provider_and_resource_type() {
        let extension = ProviderSpecExtension::new(
            ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/spec").unwrap(),
            SchemaVersion::parse("1.0").unwrap(),
            CanonicalJsonObject::empty(),
        )
        .unwrap();
        let spec = ResourceSpec::new(
            Some(ResourceRef::parse("Provider/system-core").unwrap()),
            None,
            CanonicalJsonObject::empty(),
            Some(extension),
        )
        .unwrap();
        assert_eq!(
            ResourceEnvelope::new(
                ResourceTypeName::parse("Host").unwrap(),
                metadata(),
                spec,
                status(None),
            ),
            Err(ResourceError::ProviderSchemaBinding)
        );
    }

    #[test]
    fn update_policy_base_round_trip_has_frozen_defaults() {
        const JSON: &str = r#"{"disruptive":"manual","nonDisruptive":"automatic"}"#;
        let policy: UpdatePolicy = serde_json::from_str(JSON).unwrap();
        assert_eq!(policy, UpdatePolicy::default());
        assert_eq!(canonical_json_bytes(&policy).unwrap(), JSON.as_bytes());
    }

    #[test]
    fn metadata_bounds_and_owner_self_reference_fail_closed() {
        assert!(FinalizerId::parse("core.zone-drain").is_ok());
        assert!(FinalizerId::parse("device-usbip.d2bus.org/attachment-released").is_ok());
        assert!(FinalizerId::parse("other").is_err());
        assert!(
            PresentationMetadata::new(
                BTreeMap::from([("x".to_owned(), "y".repeat(257))]),
                BTreeMap::new(),
            )
            .is_err()
        );

        let mut owned = metadata();
        owned.owner_ref = Some(ResourceRef::parse("Host/host-system").unwrap());
        assert!(
            ResourceEnvelope::new(
                ResourceTypeName::parse("Host").unwrap(),
                owned,
                base_spec(),
                status(None),
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_json_keys_are_rejected_before_envelope_materialization() {
        let duplicate =
            br#"{"apiVersion":"resources.d2bus.org/v3","apiVersion":"resources.d2bus.org/v3"}"#;
        assert!(matches!(
            ResourceEnvelope::from_json(duplicate),
            Err(ResourceError::CanonicalJson(_))
        ));
    }

    #[test]
    fn resource_codec_error_chains_never_retain_serde_text_or_payload_keys() {
        fn assert_chain_redacted(
            mut error: &(dyn std::error::Error + 'static),
            marker: &str,
        ) -> usize {
            let mut depth = 0;
            loop {
                let rendered = format!("{error:?}\n{error}");
                assert!(
                    !rendered.contains(marker),
                    "codec error chain exposed an attacker-controlled marker"
                );
                depth += 1;
                let Some(source) = error.source() else {
                    return depth;
                };
                error = source;
            }
        }

        let payload_marker = format!("payload-marker-{:x}", std::process::id());
        let unknown_field = format!(r#"{{"{payload_marker}":null}}"#);
        let serde_error = ResourceEnvelope::from_json(unknown_field.as_bytes()).unwrap_err();
        assert!(matches!(
            &serde_error,
            ResourceError::Serde {
                reason: CanonicalJsonCodecReason::Data,
                line: 1,
                column
            } if *column > 0
        ));
        assert_eq!(assert_chain_redacted(&serde_error, &payload_marker), 1);

        let duplicate = format!(r#"{{"{payload_marker}":1,"{payload_marker}":2}}"#);
        let canonical_error = ResourceEnvelope::from_json(duplicate.as_bytes()).unwrap_err();
        assert!(matches!(
            &canonical_error,
            ResourceError::CanonicalJson(CanonicalJsonError::DuplicateKey { key_ordinal: 2, .. })
        ));
        assert_eq!(assert_chain_redacted(&canonical_error, &payload_marker), 2);
    }

    #[test]
    fn resource_diagnostics_redact_identity_and_payload_layers() {
        let nonce = u64::from(std::process::id());
        let zone_marker = format!("zone-{nonce:x}");
        let name_marker = format!("name-{nonce:x}");
        let uid_marker = format!("123e4567-e89b-4abc-a456-{nonce:012x}");
        let payload_marker = format!("payload-marker-{nonce:x}");
        let markers = [
            zone_marker.as_str(),
            name_marker.as_str(),
            uid_marker.as_str(),
            payload_marker.as_str(),
        ];
        let finalizer =
            FinalizerId::parse(format!("{name_marker}.d2bus.org/{payload_marker}")).unwrap();
        let presentation = PresentationMetadata::new(
            BTreeMap::from([("marker".to_owned(), payload_marker.clone())]),
            BTreeMap::from([("marker".to_owned(), payload_marker.clone())]),
        )
        .unwrap();
        let timestamp = Timestamp::parse("2026-07-27T02:14:20.413Z").unwrap();
        let metadata = ResourceMetadata::new(
            ResourceName::parse(&name_marker).unwrap(),
            ZoneId::parse(&zone_marker).unwrap(),
            ResourceUid::parse(&uid_marker).unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ZoneRevision::new(1),
            Some(ResourceRef::parse(&format!("Provider/{name_marker}")).unwrap()),
            vec![finalizer.clone()],
            None,
            timestamp.clone(),
            timestamp,
            ManagedBy::Api,
            None,
            None,
            None,
            presentation.clone(),
        )
        .unwrap();
        let extension = ProviderSpecExtension::new(
            ExtensionSchemaId::parse(&format!("{name_marker}.d2bus.org/Host/spec")).unwrap(),
            SchemaVersion::parse("1.0").unwrap(),
            CanonicalJsonObject::parse(format!(r#"{{"marker":"{payload_marker}"}}"#).as_bytes())
                .unwrap(),
        )
        .unwrap();
        let spec = ResourceSpec::new(
            Some(ResourceRef::parse(&format!("Provider/{name_marker}")).unwrap()),
            None,
            CanonicalJsonObject::parse(format!(r#"{{"marker":"{payload_marker}"}}"#).as_bytes())
                .unwrap(),
            Some(extension.clone()),
        )
        .unwrap();
        let envelope = ResourceEnvelope::new(
            ResourceTypeName::parse("Host").unwrap(),
            metadata.clone(),
            spec.clone(),
            status(None),
        )
        .unwrap();

        let formatted = [
            format!("{finalizer:?}"),
            format!("{finalizer}"),
            format!("{presentation:?}"),
            format!("{metadata:?}"),
            format!("{extension:?}"),
            format!("{spec:?}"),
            format!("{envelope:?}"),
        ];
        for rendered in formatted {
            for marker in &markers {
                assert!(
                    !rendered.contains(marker),
                    "resource marker appeared in diagnostic formatting"
                );
            }
        }

        assert!(
            String::from_utf8(envelope.canonical_bytes().unwrap())
                .unwrap()
                .contains(&payload_marker)
        );
        assert!(finalizer.to_canonical_string().contains(&payload_marker));
    }
}
