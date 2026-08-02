//! Canonical Zone resource-bundle contracts.
//!
//! The Nix compiler emits a configuration bundle before runtime metadata
//! exists.  It therefore has a deliberately smaller resource item than the
//! live [`super::ResourceEnvelope`]: the item contains author metadata and
//! desired spec only, while UID, status, finalizers, and store paths remain
//! runtime or private-artifact concerns.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef, ResourceTypeName, ZoneId,
    resource_schema::{
        CanonicalJsonObject, CanonicalJsonValue, canonical_digest, canonical_json_bytes,
    },
};

/// The canonical domain tag used for the resource array content hash.
pub const RESOURCE_BUNDLE_CONTENT_DOMAIN_TAG: &str = "d2b:v3:resource-bundle";
/// The canonical domain tag used for an artifact-catalog preimage digest.
pub const ARTIFACT_CATALOG_DOMAIN_TAG: &str = "d2b:v3:artifact-catalog";
/// Maximum resources in one Zone bundle.
pub const MAX_BUNDLE_RESOURCES: usize = 16_384;
/// Maximum schema/provider fingerprint entries in a private bundle.
pub const MAX_BUNDLE_FINGERPRINTS: usize = 256;

/// Author-controlled metadata carried by a bundle resource item.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleResourceMetadata {
    name: super::ResourceName,
    zone: ZoneId,
    #[serde(default)]
    owner_ref: Option<ResourceRef>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

impl BundleResourceMetadata {
    /// Construct bundle metadata.
    pub fn new(
        name: super::ResourceName,
        zone: ZoneId,
        owner_ref: Option<ResourceRef>,
        labels: BTreeMap<String, String>,
        annotations: BTreeMap<String, String>,
    ) -> Self {
        Self {
            name,
            zone,
            owner_ref,
            labels,
            annotations,
        }
    }

    /// Borrow the derived resource name.
    pub const fn name(&self) -> &super::ResourceName {
        &self.name
    }

    /// Borrow the enclosing Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the optional owner reference.
    pub const fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
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

impl core::fmt::Debug for BundleResourceMetadata {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BundleResourceMetadata(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for BundleResourceMetadata {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            name: super::ResourceName,
            zone: ZoneId,
            #[serde(default)]
            owner_ref: Option<ResourceRef>,
            #[serde(default)]
            labels: BTreeMap<String, String>,
            #[serde(default)]
            annotations: BTreeMap<String, String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(
            wire.name,
            wire.zone,
            wire.owner_ref,
            wire.labels,
            wire.annotations,
        ))
    }
}

/// One desired-state resource item in a Zone bundle.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleResource {
    api_version: String,
    #[serde(rename = "type")]
    resource_type: ResourceTypeName,
    metadata: BundleResourceMetadata,
    spec: CanonicalJsonObject,
}

impl BundleResource {
    /// Construct one bundle item.
    pub fn new(
        resource_type: ResourceTypeName,
        metadata: BundleResourceMetadata,
        spec: CanonicalJsonObject,
    ) -> Result<Self, ResourceBundleError> {
        if metadata.owner_ref().is_some_and(|owner| {
            owner.resource_type() == &resource_type && owner.name() == metadata.name()
        }) {
            return Err(ResourceBundleError::SelfOwner);
        }
        reject_runtime_or_private_fields(&spec)?;
        Ok(Self {
            api_version: super::resource::RESOURCE_API_VERSION.to_owned(),
            resource_type,
            metadata,
            spec,
        })
    }

    /// Borrow the ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Borrow item metadata.
    pub const fn metadata(&self) -> &BundleResourceMetadata {
        &self.metadata
    }

    /// Borrow the desired spec object.
    pub const fn spec(&self) -> &CanonicalJsonObject {
        &self.spec
    }

    /// Return the canonical `(type, name)` sorting key.
    pub fn sort_key(&self) -> (&str, &str) {
        (self.resource_type.as_str(), self.metadata.name().as_str())
    }
}

impl core::fmt::Debug for BundleResource {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BundleResource(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for BundleResource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            api_version: String,
            #[serde(rename = "type")]
            resource_type: ResourceTypeName,
            metadata: BundleResourceMetadata,
            spec: CanonicalJsonObject,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.api_version != super::resource::RESOURCE_API_VERSION {
            return Err(serde::de::Error::custom(
                "bundle resource apiVersion mismatch",
            ));
        }
        Self::new(wire.resource_type, wire.metadata, wire.spec).map_err(serde::de::Error::custom)
    }
}

/// Private integrity metadata carried alongside the public resource array.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleIntegrityPin {
    /// Digest of canonical JSON for the `resources` array.
    pub content_hash: String,
    /// Digest of the artifact-catalog preimage.
    pub artifact_catalog_digest: String,
    /// ResourceType schema fingerprints.
    #[serde(default)]
    pub schema_fingerprints: BTreeMap<String, String>,
    /// Selected Provider schema fingerprints.
    #[serde(default)]
    pub provider_schema_digests: BTreeMap<String, String>,
}

impl core::fmt::Debug for BundleIntegrityPin {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BundleIntegrityPin(<redacted>)")
    }
}

/// A complete Nix-authored Zone resource bundle.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceBundle {
    /// Bundle schema version.
    pub schema_version: u32,
    /// Bundle format version.
    pub bundle_version: u32,
    /// Enclosing Zone.
    pub zone: ZoneId,
    /// Content/integrity pins.
    #[serde(flatten)]
    pub integrity: BundleIntegrityPin,
    /// Sorted desired-state resources.
    pub resources: Vec<BundleResource>,
    /// Stable generation timestamp supplied by the compiler.
    pub generated_at: super::Timestamp,
}

impl ResourceBundle {
    /// Build a canonical bundle and compute its content hash.
    pub fn new(
        zone: ZoneId,
        mut resources: Vec<BundleResource>,
        artifact_catalog_digest: String,
        schema_fingerprints: BTreeMap<String, String>,
        provider_schema_digests: BTreeMap<String, String>,
        generated_at: super::Timestamp,
    ) -> Result<Self, ResourceBundleError> {
        if resources.len() > MAX_BUNDLE_RESOURCES
            || schema_fingerprints.len() > MAX_BUNDLE_FINGERPRINTS
            || provider_schema_digests.len() > MAX_BUNDLE_FINGERPRINTS
        {
            return Err(ResourceBundleError::TooLarge);
        }
        if !is_digest(&artifact_catalog_digest) {
            return Err(ResourceBundleError::InvalidDigest);
        }
        for fingerprint in schema_fingerprints
            .values()
            .chain(provider_schema_digests.values())
        {
            if !is_digest(fingerprint) {
                return Err(ResourceBundleError::InvalidDigest);
            }
        }
        for resource in &resources {
            if resource.metadata().zone() != &zone {
                return Err(ResourceBundleError::ZoneMismatch);
            }
        }
        resources.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        if resources
            .windows(2)
            .any(|pair| pair[0].sort_key() == pair[1].sort_key())
        {
            return Err(ResourceBundleError::DuplicateResource);
        }
        let content_hash = digest_resources(&resources)?;
        Ok(Self {
            schema_version: 3,
            bundle_version: 1,
            zone,
            integrity: BundleIntegrityPin {
                content_hash,
                artifact_catalog_digest,
                schema_fingerprints,
                provider_schema_digests,
            },
            resources,
            generated_at,
        })
    }

    /// Parse and verify a bundle through canonical duplicate-key decoding.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ResourceBundleError> {
        CanonicalJsonValue::parse(bytes).map_err(ResourceBundleError::CanonicalJson)?;
        let bundle: Self =
            serde_json::from_slice(bytes).map_err(|_| ResourceBundleError::Malformed)?;
        bundle.verify()?;
        Ok(bundle)
    }

    /// Verify ordering, Zone identity, and content hash.
    pub fn verify(&self) -> Result<(), ResourceBundleError> {
        if self.schema_version != 3 || self.bundle_version != 1 {
            return Err(ResourceBundleError::UnsupportedVersion);
        }
        if !is_digest(&self.integrity.artifact_catalog_digest)
            || self
                .integrity
                .schema_fingerprints
                .values()
                .chain(self.integrity.provider_schema_digests.values())
                .any(|digest| !is_digest(digest))
        {
            return Err(ResourceBundleError::InvalidDigest);
        }
        for resource in &self.resources {
            if resource.metadata().zone() != &self.zone {
                return Err(ResourceBundleError::ZoneMismatch);
            }
        }
        if self
            .resources
            .windows(2)
            .any(|pair| pair[0].sort_key() >= pair[1].sort_key())
        {
            return Err(ResourceBundleError::UnsortedResources);
        }
        if digest_resources(&self.resources)? != self.integrity.content_hash {
            return Err(ResourceBundleError::ContentHashMismatch);
        }
        Ok(())
    }

    /// Borrow the bundle's integrity fields.
    pub const fn integrity(&self) -> &BundleIntegrityPin {
        &self.integrity
    }
}

impl core::fmt::Debug for ResourceBundle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResourceBundle")
            .field("schema_version", &self.schema_version)
            .field("bundle_version", &self.bundle_version)
            .field("resource_count", &self.resources.len())
            .finish()
    }
}

/// Closed bundle validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceBundleError {
    /// Canonical JSON could not be decoded.
    CanonicalJson(super::resource_schema::CanonicalJsonError),
    /// The JSON shape was not a bundle.
    Malformed,
    /// A runtime/private field appeared in a bundle item.
    ForbiddenField,
    /// A resource owns itself.
    SelfOwner,
    /// A resource belongs to another Zone.
    ZoneMismatch,
    /// A bundle contains duplicate `(type,name)` rows.
    DuplicateResource,
    /// A bundle exceeds a frozen bound.
    TooLarge,
    /// A digest is not a canonical sha256 value.
    InvalidDigest,
    /// The bundle format version is unsupported.
    UnsupportedVersion,
    /// Resource rows are not sorted.
    UnsortedResources,
    /// The recorded content hash differs from the resource array.
    ContentHashMismatch,
    /// A canonical rendering operation failed.
    CanonicalJsonEncode(super::resource_schema::CanonicalJsonError),
}

impl core::fmt::Display for ResourceBundleError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::CanonicalJson(_) => "resource bundle canonical JSON is invalid",
            Self::Malformed => "resource bundle shape is invalid",
            Self::ForbiddenField => "resource bundle contains a forbidden field",
            Self::SelfOwner => "resource bundle resource owns itself",
            Self::ZoneMismatch => "resource bundle resource belongs to another Zone",
            Self::DuplicateResource => "resource bundle contains a duplicate resource",
            Self::TooLarge => "resource bundle exceeds a frozen bound",
            Self::InvalidDigest => "resource bundle contains an invalid digest",
            Self::UnsupportedVersion => "resource bundle version is unsupported",
            Self::UnsortedResources => "resource bundle resources are not sorted",
            Self::ContentHashMismatch => "resource bundle content hash does not match resources",
            Self::CanonicalJsonEncode(_) => "resource bundle could not be rendered canonically",
        })
    }
}

impl std::error::Error for ResourceBundleError {}

fn digest_resources(resources: &[BundleResource]) -> Result<String, ResourceBundleError> {
    let bytes =
        canonical_json_bytes(&resources).map_err(ResourceBundleError::CanonicalJsonEncode)?;
    Ok(canonical_digest(RESOURCE_BUNDLE_CONTENT_DOMAIN_TAG, &bytes))
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn reject_runtime_or_private_fields(
    object: &CanonicalJsonObject,
) -> Result<(), ResourceBundleError> {
    fn walk(value: &CanonicalJsonValue) -> bool {
        match value {
            CanonicalJsonValue::Object(map) => map.iter().any(|(key, value)| {
                matches!(
                    key.as_str(),
                    "status"
                        | "storePath"
                        | "nixSystem"
                        | "schemaFingerprint"
                        | "providerSchemaFingerprint"
                        | "managedBy"
                        | "configurationGeneration"
                ) || walk(value)
            }),
            CanonicalJsonValue::Array(values) => values.iter().any(walk),
            _ => false,
        }
    }
    if walk(&CanonicalJsonValue::Object(object.clone().into_inner())) {
        Err(ResourceBundleError::ForbiddenField)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{ResourceName, Timestamp};

    fn resource(kind: &str, name: &str) -> BundleResource {
        BundleResource::new(
            ResourceTypeName::parse(kind).unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse(name).unwrap(),
                ZoneId::parse("dev").unwrap(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::empty(),
        )
        .unwrap()
    }

    fn timestamp() -> Timestamp {
        Timestamp::parse("2026-07-22T00:00:00.000Z").unwrap()
    }

    #[test]
    fn bundle_sorts_rows_and_hashes_only_the_resource_array() {
        let bundle = ResourceBundle::new(
            ZoneId::parse("dev").unwrap(),
            vec![resource("Process", "z"), resource("Host", "a")],
            "sha256:".to_owned() + &"11".repeat(32),
            BTreeMap::new(),
            BTreeMap::new(),
            timestamp(),
        )
        .unwrap();
        assert_eq!(bundle.resources[0].resource_type().as_str(), "Host");
        assert_eq!(bundle.resources[1].metadata().name().as_str(), "z");
        let bytes = canonical_json_bytes(&bundle).unwrap();
        assert_eq!(ResourceBundle::from_json(&bytes).unwrap(), bundle);
    }

    #[test]
    fn forbidden_private_fields_never_enter_a_resource_item() {
        let spec = CanonicalJsonObject::parse(
            br#"{"provider":{"settings":{"storePath":"/nix/store/x"}}}"#,
        )
        .unwrap();
        assert_eq!(
            BundleResource::new(
                ResourceTypeName::parse("Guest").unwrap(),
                BundleResourceMetadata::new(
                    ResourceName::parse("guest").unwrap(),
                    ZoneId::parse("dev").unwrap(),
                    None,
                    BTreeMap::new(),
                    BTreeMap::new(),
                ),
                spec,
            )
            .unwrap_err(),
            ResourceBundleError::ForbiddenField
        );
    }

    #[test]
    fn content_tampering_is_rejected() {
        let bundle = ResourceBundle::new(
            ZoneId::parse("dev").unwrap(),
            vec![resource("Host", "a")],
            "sha256:".to_owned() + &"11".repeat(32),
            BTreeMap::new(),
            BTreeMap::new(),
            timestamp(),
        )
        .unwrap();
        let mut value = serde_json::to_value(bundle).unwrap();
        value["resources"][0]["metadata"]["name"] = serde_json::json!("b");
        assert!(ResourceBundle::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
