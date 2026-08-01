//! Integrity-verified input contract for one Zone configuration generation.
//!
//! Bundle resources contain only authored desired state and the permitted
//! metadata projection. Store-owned management metadata is added only when the
//! configuration controller persists the resource.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::v3::{
    CanonicalJsonError, CanonicalJsonObject, ResourceBundleGenerationId, ResourceName, ResourceRef,
    ResourceTypeName, SchemaFingerprint, ZoneId, canonical_digest, canonical_json_bytes,
};

/// Bundle schema version for the resource-plane contract.
pub const ZONE_BUNDLE_SCHEMA_VERSION: u32 = 3;
/// First version of the monolithic per-Zone bundle envelope.
pub const ZONE_BUNDLE_VERSION: u32 = 1;
/// Reproducible build timestamp carried by every bundle.
pub const ZONE_BUNDLE_GENERATED_AT: &str = "1970-01-01T00:00:00.000Z";
/// Domain tag for the canonical sorted resources-array digest.
pub const ZONE_BUNDLE_DOMAIN_TAG: &str = "d2b:v3:resource-bundle";

/// Closed validation failure for a bundle input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneBundleError {
    /// An envelope version or reproducibility field is not canonical.
    InvalidEnvelope,
    /// A resource belongs to a different Zone than the envelope.
    ZoneMismatch,
    /// The same `(type, name)` identity appears more than once.
    DuplicateResource,
    /// The resources array is not in canonical `(type, zone, name)` order.
    ResourcesNotSorted,
    /// The supplied `contentHash` does not match the resources array.
    IntegrityFailure,
    /// A string or dynamic spec value is outside the canonical JSON profile.
    CanonicalJson,
    /// A bundled Provider schema digest does not match the installed artifact.
    ProviderSchemaDigestMismatch,
}

impl ZoneBundleError {
    /// Return the stable failure label without caller-supplied data.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidEnvelope => "bundle-envelope-invalid",
            Self::ZoneMismatch => "bundle-zone-mismatch",
            Self::DuplicateResource => "bundle-duplicate-resource",
            Self::ResourcesNotSorted => "bundle-resources-not-sorted",
            Self::IntegrityFailure => "bundle-integrity-failure",
            Self::CanonicalJson => "bundle-canonical-json-invalid",
            Self::ProviderSchemaDigestMismatch => "bundle-provider-schema-digest-mismatch",
        }
    }
}

impl core::fmt::Display for ZoneBundleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for ZoneBundleError {}

impl From<CanonicalJsonError> for ZoneBundleError {
    fn from(_: CanonicalJsonError) -> Self {
        Self::CanonicalJson
    }
}

/// The Nix-authorable metadata subset of one bundle resource.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BundleMetadata {
    name: ResourceName,
    zone: ZoneId,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    labels: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    annotations: BTreeMap<String, String>,
}

impl BundleMetadata {
    /// Construct the permitted metadata projection.
    pub fn new(
        name: ResourceName,
        zone: ZoneId,
        owner_ref: Option<ResourceRef>,
        labels: BTreeMap<String, String>,
        annotations: BTreeMap<String, String>,
    ) -> Result<Self, ZoneBundleError> {
        canonical_json_bytes(&labels)?;
        canonical_json_bytes(&annotations)?;
        Ok(Self {
            name,
            zone,
            owner_ref,
            labels,
            annotations,
        })
    }

    /// Borrow the Zone-local resource name.
    pub const fn name(&self) -> &ResourceName {
        &self.name
    }

    /// Borrow the containing Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the optional owner reference.
    pub const fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }

    /// Borrow presentation labels.
    pub const fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }

    /// Borrow presentation annotations.
    pub const fn annotations(&self) -> &BTreeMap<String, String> {
        &self.annotations
    }
}

impl core::fmt::Debug for BundleMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BundleMetadata")
            .field("labels", &self.labels.len())
            .field("annotations", &self.annotations.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for BundleMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            name: ResourceName,
            zone: ZoneId,
            #[serde(default)]
            owner_ref: Option<ResourceRef>,
            #[serde(default)]
            labels: BTreeMap<String, String>,
            #[serde(default)]
            annotations: BTreeMap<String, String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.name,
            wire.zone,
            wire.owner_ref,
            wire.labels,
            wire.annotations,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One authored resource in a generation bundle.
///
/// This input type intentionally has no `managedBy` or
/// `configurationGeneration` field. Those fields are store-owned metadata set
/// by the configuration controller after activation commits.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleResource {
    api_version: String,
    #[serde(rename = "type")]
    resource_type: ResourceTypeName,
    metadata: BundleMetadata,
    spec: CanonicalJsonObject,
}

// Listing every field makes any addition to this input DTO a compile failure.
// Store-owned metadata therefore cannot silently cross the bundle boundary.
const _: fn(BundleResource) = |BundleResource {
                                   api_version: _,
                                   resource_type: _,
                                   metadata: _,
                                   spec: _,
                               }| {};

impl BundleResource {
    /// Closed serialized field set for the authored input DTO.
    pub const INPUT_FIELD_NAMES: [&'static str; 4] = ["apiVersion", "type", "metadata", "spec"];

    /// Construct one canonical bundle input resource.
    pub fn new(
        resource_type: ResourceTypeName,
        metadata: BundleMetadata,
        spec: CanonicalJsonObject,
    ) -> Result<Self, ZoneBundleError> {
        canonical_json_bytes(&spec)?;
        Ok(Self {
            api_version: crate::v3::resource::RESOURCE_API_VERSION.to_owned(),
            resource_type,
            metadata,
            spec,
        })
    }

    /// Borrow the ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Borrow the permitted bundle metadata.
    pub const fn metadata(&self) -> &BundleMetadata {
        &self.metadata
    }

    /// Borrow the exact authored spec object.
    pub const fn spec(&self) -> &CanonicalJsonObject {
        &self.spec
    }
}

impl core::fmt::Debug for BundleResource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BundleResource")
            .field("spec_fields", &self.spec.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for BundleResource {
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
            metadata: BundleMetadata,
            spec: CanonicalJsonObject,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.api_version != crate::v3::resource::RESOURCE_API_VERSION {
            return Err(serde::de::Error::custom(ZoneBundleError::InvalidEnvelope));
        }
        Self::new(wire.resource_type, wire.metadata, wire.spec).map_err(serde::de::Error::custom)
    }
}

/// One integrity-pinned monolithic resource bundle for a Zone.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneBundle {
    schema_version: u32,
    bundle_version: u32,
    zone: ZoneId,
    content_hash: ResourceBundleGenerationId,
    artifact_catalog_digest: SchemaFingerprint,
    generated_at: String,
    resources: Vec<BundleResource>,
    provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
}

impl ZoneBundle {
    /// Construct a sorted bundle and compute its generation identity.
    pub fn build(
        zone: ZoneId,
        artifact_catalog_digest: SchemaFingerprint,
        mut resources: Vec<BundleResource>,
        provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
    ) -> Result<Self, ZoneBundleError> {
        resources.sort_by(resource_order);
        validate_resources(&zone, &resources)?;
        canonical_json_bytes(&provider_schema_digests)?;
        let content_hash = Self::compute_content_hash(&resources)?;
        Ok(Self {
            schema_version: ZONE_BUNDLE_SCHEMA_VERSION,
            bundle_version: ZONE_BUNDLE_VERSION,
            zone,
            content_hash,
            artifact_catalog_digest,
            generated_at: ZONE_BUNDLE_GENERATED_AT.to_owned(),
            resources,
            provider_schema_digests,
        })
    }

    /// Validate an already-encoded envelope and its declared content hash.
    #[allow(clippy::too_many_arguments)]
    fn from_wire(
        schema_version: u32,
        bundle_version: u32,
        zone: ZoneId,
        content_hash: ResourceBundleGenerationId,
        artifact_catalog_digest: SchemaFingerprint,
        generated_at: String,
        resources: Vec<BundleResource>,
        provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
    ) -> Result<Self, ZoneBundleError> {
        if schema_version != ZONE_BUNDLE_SCHEMA_VERSION
            || bundle_version != ZONE_BUNDLE_VERSION
            || generated_at != ZONE_BUNDLE_GENERATED_AT
        {
            return Err(ZoneBundleError::InvalidEnvelope);
        }
        validate_resources(&zone, &resources)?;
        canonical_json_bytes(&provider_schema_digests)?;
        if !resources
            .windows(2)
            .all(|pair| resource_order(&pair[0], &pair[1]).is_le())
        {
            return Err(ZoneBundleError::ResourcesNotSorted);
        }
        if content_hash != Self::compute_content_hash(&resources)? {
            return Err(ZoneBundleError::IntegrityFailure);
        }
        Ok(Self {
            schema_version,
            bundle_version,
            zone,
            content_hash,
            artifact_catalog_digest,
            generated_at,
            resources,
            provider_schema_digests,
        })
    }

    /// Parse and integrity-verify canonical JSON bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ZoneBundleError> {
        crate::v3::CanonicalJsonValue::parse(bytes)?;
        let bundle: Self = serde_json::from_slice(bytes).map_err(classify_bundle_json_error)?;
        Ok(bundle)
    }

    /// Render exact canonical bundle bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ZoneBundleError> {
        canonical_json_bytes(self).map_err(Into::into)
    }

    /// Compute the generation identity over the canonical resources array.
    pub fn compute_content_hash(
        resources: &[BundleResource],
    ) -> Result<ResourceBundleGenerationId, ZoneBundleError> {
        let bytes = canonical_json_bytes(&resources.to_vec())?;
        ResourceBundleGenerationId::parse(canonical_digest(ZONE_BUNDLE_DOMAIN_TAG, &bytes))
            .map_err(|_| ZoneBundleError::CanonicalJson)
    }

    /// Borrow the Zone this bundle configures.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the content-derived generation identity.
    pub const fn content_hash(&self) -> &ResourceBundleGenerationId {
        &self.content_hash
    }

    /// Borrow the pinned site artifact-catalog digest.
    pub const fn artifact_catalog_digest(&self) -> &SchemaFingerprint {
        &self.artifact_catalog_digest
    }

    /// Borrow the canonically sorted resource inputs.
    pub fn resources(&self) -> &[BundleResource] {
        &self.resources
    }

    /// Borrow the Provider settings-schema digest map.
    pub const fn provider_schema_digests(&self) -> &BTreeMap<String, SchemaFingerprint> {
        &self.provider_schema_digests
    }

    /// Verify every bundled Provider schema digest against installed artifacts.
    ///
    /// Installed Providers not referenced by this bundle are permitted. Every
    /// bundled entry must be present and equal before application proceeds.
    pub fn verify_provider_schema_digests(
        &self,
        installed: &BTreeMap<String, SchemaFingerprint>,
    ) -> Result<(), ZoneBundleError> {
        if self
            .provider_schema_digests
            .iter()
            .all(|(provider, digest)| installed.get(provider) == Some(digest))
        {
            Ok(())
        } else {
            Err(ZoneBundleError::ProviderSchemaDigestMismatch)
        }
    }
}

impl core::fmt::Debug for ZoneBundle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZoneBundle")
            .field("resources", &self.resources.len())
            .field("provider_schemas", &self.provider_schema_digests.len())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for ZoneBundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            bundle_version: u32,
            zone: ZoneId,
            content_hash: ResourceBundleGenerationId,
            artifact_catalog_digest: SchemaFingerprint,
            generated_at: String,
            resources: Vec<BundleResource>,
            provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_wire(
            wire.schema_version,
            wire.bundle_version,
            wire.zone,
            wire.content_hash,
            wire.artifact_catalog_digest,
            wire.generated_at,
            wire.resources,
            wire.provider_schema_digests,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn resource_order(left: &BundleResource, right: &BundleResource) -> core::cmp::Ordering {
    (
        left.resource_type(),
        left.metadata().zone(),
        left.metadata().name(),
    )
        .cmp(&(
            right.resource_type(),
            right.metadata().zone(),
            right.metadata().name(),
        ))
}

fn validate_resources(zone: &ZoneId, resources: &[BundleResource]) -> Result<(), ZoneBundleError> {
    let mut seen = BTreeSet::new();
    for resource in resources {
        if resource.metadata().zone() != zone {
            return Err(ZoneBundleError::ZoneMismatch);
        }
        if !seen.insert((resource.resource_type(), resource.metadata().name())) {
            return Err(ZoneBundleError::DuplicateResource);
        }
    }
    Ok(())
}

fn classify_bundle_json_error(error: serde_json::Error) -> ZoneBundleError {
    if error
        .to_string()
        .contains(ZoneBundleError::IntegrityFailure.label())
    {
        ZoneBundleError::IntegrityFailure
    } else {
        ZoneBundleError::CanonicalJson
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
    }

    fn resource(resource_type: &str, name: &str) -> BundleResource {
        BundleResource::new(
            ResourceTypeName::parse(resource_type).unwrap(),
            BundleMetadata::new(
                ResourceName::parse(name).unwrap(),
                ZoneId::parse("work").unwrap(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            )
            .unwrap(),
            CanonicalJsonObject::parse(br#"{"value":"configured"}"#).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn build_sorts_resources_and_hashes_only_the_resources_array() {
        let bundle = ZoneBundle::build(
            ZoneId::parse("work").unwrap(),
            digest('a'),
            vec![
                resource("Volume", "state"),
                resource("Credential", "access"),
            ],
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(bundle.resources()[0].resource_type().as_str(), "Credential");
        assert_eq!(
            bundle.content_hash(),
            &ZoneBundle::compute_content_hash(bundle.resources()).unwrap()
        );

        let changed_catalog = ZoneBundle::build(
            ZoneId::parse("work").unwrap(),
            digest('b'),
            bundle.resources().to_vec(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(bundle.content_hash(), changed_catalog.content_hash());
    }

    #[test]
    fn canonical_round_trip_rejects_tampering_and_runtime_metadata() {
        let bundle = ZoneBundle::build(
            ZoneId::parse("work").unwrap(),
            digest('a'),
            vec![resource("Credential", "access")],
            BTreeMap::new(),
        )
        .unwrap();
        let bytes = bundle.canonical_bytes().unwrap();
        assert_eq!(ZoneBundle::from_json(&bytes).unwrap(), bundle);

        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["resources"][0]["metadata"]["managedBy"] = serde_json::json!("configuration");
        assert_eq!(
            ZoneBundle::from_json(&serde_json::to_vec(&value).unwrap()),
            Err(ZoneBundleError::CanonicalJson)
        );

        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["contentHash"] = serde_json::json!(format!("sha256:{}", "f".repeat(64)));
        assert_eq!(
            ZoneBundle::from_json(&serde_json::to_vec(&value).unwrap()),
            Err(ZoneBundleError::IntegrityFailure)
        );
    }

    #[test]
    fn debug_output_redacts_bundle_identity_and_resource_names() {
        let bundle = ZoneBundle::build(
            ZoneId::parse("secret-zone").unwrap(),
            digest('a'),
            vec![resource("Credential", "secret-name")],
            BTreeMap::new(),
        );
        assert!(bundle.is_err(), "cross-Zone metadata is rejected");

        let bundle = ZoneBundle::build(
            ZoneId::parse("work").unwrap(),
            digest('a'),
            vec![resource("Credential", "secret-name")],
            BTreeMap::new(),
        )
        .unwrap();
        let rendered = format!("{bundle:?}");
        assert!(!rendered.contains("work"));
        assert!(!rendered.contains("secret-name"));
        assert!(!rendered.contains("sha256:"));
    }
}
