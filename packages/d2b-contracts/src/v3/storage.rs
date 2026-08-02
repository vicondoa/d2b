//! Closed storage contract for one Zone resource store.
//!
//! The row carries only broker-resolved opaque identifiers. It deliberately has
//! no path-bearing type or field, so a caller cannot smuggle a host path across
//! the storage-owner boundary.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum byte length of a broker-resolved Zone storage identifier.
pub const MAX_ZONE_STORAGE_ID_BYTES: usize = 160;
/// Closed grammar for opaque storage identifiers and local principals.
pub const ZONE_STORAGE_ID_PATTERN: &str = "^[a-z][a-z0-9-]{0,159}$";

/// Stable rejection classes for validated storage identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneStorageContractError {
    /// The identifier was empty, oversized, or outside the opaque-ID grammar.
    InvalidOpaqueId,
    /// The mode was not one leading zero plus three octal digits.
    InvalidMode,
    /// A database inode must have exactly one link.
    InvalidLinkCount,
}

impl core::fmt::Display for ZoneStorageContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidOpaqueId => "Zone storage identifier is invalid",
            Self::InvalidMode => "Zone store database mode is invalid",
            Self::InvalidLinkCount => "Zone store database link count must be one",
        })
    }
}

impl std::error::Error for ZoneStorageContractError {}

fn validate_opaque_id(value: &str) -> Result<(), ZoneStorageContractError> {
    if value.is_empty()
        || value.len() > MAX_ZONE_STORAGE_ID_BYTES
        || !value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' => true,
            b'0'..=b'9' | b'-' => index > 0,
            _ => false,
        })
    {
        return Err(ZoneStorageContractError::InvalidOpaqueId);
    }
    Ok(())
}

fn opaque_id_schema(description: &str) -> Schema {
    Schema::Object(SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        string: Some(Box::new(StringValidation {
            min_length: Some(1),
            max_length: Some(MAX_ZONE_STORAGE_ID_BYTES as u32),
            pattern: Some(ZONE_STORAGE_ID_PATTERN.to_owned()),
        })),
        metadata: Some(Box::new(schemars::schema::Metadata {
            description: Some(description.to_owned()),
            ..Default::default()
        })),
        ..Default::default()
    })
}

macro_rules! opaque_storage_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate and construct one broker-resolved opaque identifier.
            pub fn parse(value: impl Into<String>) -> Result<Self, ZoneStorageContractError> {
                let value = value.into();
                validate_opaque_id(&value)?;
                Ok(Self(value))
            }

            /// Borrow the validated identifier for contract serialization.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<opaque>)"))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                opaque_id_schema($description)
            }
        }
    };
}

opaque_storage_id!(
    ZoneStoreId,
    "Opaque Zone store identifier resolved only by the storage owner."
);
opaque_storage_id!(
    ZoneStoreParentDirectoryId,
    "Opaque parent-directory identifier resolved by the broker through an anchored descriptor."
);
opaque_storage_id!(
    ZoneStoreIdentityMarkerId,
    "Opaque identity-marker identifier resolved only by the storage owner."
);
opaque_storage_id!(
    ZoneStorePrincipal,
    "Validated local storage-owner, file-owner, or file-group principal."
);

/// Exact database-inode ownership and metadata requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneStoreOwnershipInvariant {
    /// Required database inode owner.
    pub owner: ZoneStorePrincipal,
    /// Required database inode group.
    pub group: ZoneStorePrincipal,
    /// Required database inode mode.
    pub mode: ZoneStoreFileMode,
    /// Required database inode link count.
    pub link_count: ZoneStoreLinkCount,
}

/// Required database inode mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ZoneStoreFileMode(String);

impl ZoneStoreFileMode {
    /// Validate and construct one four-digit octal mode.
    pub fn parse(value: impl Into<String>) -> Result<Self, ZoneStorageContractError> {
        let value = value.into();
        if value.len() != 4
            || !value.starts_with('0')
            || !value
                .bytes()
                .skip(1)
                .all(|byte| matches!(byte, b'0'..=b'7'))
        {
            return Err(ZoneStorageContractError::InvalidMode);
        }
        Ok(Self(value))
    }

    /// Borrow the configured mode for filesystem validation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ZoneStoreFileMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ZoneStoreFileMode {
    fn schema_name() -> String {
        "ZoneStoreFileMode".to_owned()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                min_length: Some(4),
                max_length: Some(4),
                pattern: Some("^0[0-7]{3}$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Exact one-link requirement for the database inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ZoneStoreLinkCount(u8);

impl ZoneStoreLinkCount {
    /// The closed database link-count invariant.
    pub const ONE: Self = Self(1);

    /// Validate the observed database inode link count.
    pub fn new(value: u8) -> Result<Self, ZoneStorageContractError> {
        if value == 1 {
            Ok(Self::ONE)
        } else {
            Err(ZoneStorageContractError::InvalidLinkCount)
        }
    }

    /// Return the required numeric link count.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ZoneStoreLinkCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ZoneStoreLinkCount {
    fn schema_name() -> String {
        "ZoneStoreLinkCount".to_owned()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        let mut schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Integer))),
            enum_values: Some(vec![serde_json::Value::from(1)]),
            ..Default::default()
        };
        schema.metadata().description = Some("Database inode link count, exactly one.".to_owned());
        Schema::Object(schema)
    }
}

/// Required filesystem posture for resolving and opening the database inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreFilesystemRequirement {
    /// Resolve beneath the anchored parent and open a regular file without following links.
    RegularFileAnchoredFdRelativeNoFollow,
}

/// Required lock primitive and descriptor inheritance posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreLockingRequirement {
    /// Use an open-file-description lock whose descriptor is close-on-exec.
    OfdCloseOnExec,
}

/// Identity marker required before a store descriptor may be published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneStoreMarkerInvariant {
    /// Opaque broker-resolved identity marker.
    pub identity_marker_id: ZoneStoreIdentityMarkerId,
}

/// Required response to missing, replaced, or identity-mismatched state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreReplacementDetection {
    /// Refuse the open rather than silently creating a replacement store.
    FailClosedOnMissingReplacedOrIdentityMismatch,
}

/// Required durability boundary for store creation and replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreFsyncRequirement {
    /// Fsync the database file and its anchored parent directory.
    DatabaseAndParentDirectory,
}

/// Required descriptor publication posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreDescriptorPublicationRequirement {
    /// Publish only an owned descriptor verified close-on-exec before concurrency.
    OwnedDescriptorCloseOnExecVerifiedBeforeConcurrency,
}

/// Required staged-replacement publication and ambiguous-recovery posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreReplacementPublicationRequirement {
    /// Atomically rename, retain the prior store, and quarantine ambiguity.
    AtomicRenameRetainPriorQuarantineAmbiguity,
}

/// Complete publication requirements for initial open and staged replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneStorePublicationInvariant {
    /// Required descriptor ownership, inheritance, and concurrency boundary.
    pub descriptor: ZoneStoreDescriptorPublicationRequirement,
    /// Required staged-replacement publication behavior.
    pub replacement: ZoneStoreReplacementPublicationRequirement,
}

/// Complete closed storage row for one Zone resource store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneStoreStorageRow {
    /// Opaque identifier for the database file.
    pub zone_store_id: ZoneStoreId,
    /// Principal with authority to provision, validate, and open the store.
    pub storage_owner_principal: ZoneStorePrincipal,
    /// Opaque identifier for its broker-resolved parent directory.
    pub parent_directory_id: ZoneStoreParentDirectoryId,
    /// Required owner, group, mode, and link-count posture.
    pub ownership: ZoneStoreOwnershipInvariant,
    /// Required filesystem capability and path-resolution posture.
    pub filesystem: ZoneStoreFilesystemRequirement,
    /// Required locking capability.
    pub locking: ZoneStoreLockingRequirement,
    /// Required identity marker.
    pub marker: ZoneStoreMarkerInvariant,
    /// Required replacement-detection behavior.
    pub replacement_detection: ZoneStoreReplacementDetection,
    /// Required file and parent-directory durability.
    pub fsync: ZoneStoreFsyncRequirement,
    /// Required crash-safe publication behavior.
    pub publication: ZoneStorePublicationInvariant,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_row() -> serde_json::Value {
        serde_json::json!({
            "zoneStoreId": "zone-store-local-root",
            "storageOwnerPrincipal": "d2b-zonert",
            "parentDirectoryId": "zone-store-parent-local-root",
            "ownership": {
                "owner": "d2b-zonert",
                "group": "d2b-zonert",
                "mode": "0640",
                "linkCount": 1
            },
            "filesystem": "regular-file-anchored-fd-relative-no-follow",
            "locking": "ofd-close-on-exec",
            "marker": {
                "identityMarkerId": "zone-store-marker-local-root"
            },
            "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
            "fsync": "database-and-parent-directory",
            "publication": {
                "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
                "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
            }
        })
    }

    #[test]
    fn canonical_row_round_trips_with_no_path_field() {
        let row: ZoneStoreStorageRow =
            serde_json::from_value(valid_row()).expect("canonical storage row");
        let serialized = serde_json::to_value(row).expect("serialize storage row");

        assert_eq!(serialized, valid_row());
        assert!(serialized.get("path").is_none());
        assert!(serialized.get("pathTemplate").is_none());
    }

    #[test]
    fn opaque_ids_reject_host_paths() {
        for field in ["zoneStoreId", "parentDirectoryId", "identityMarkerId"] {
            let mut candidate = valid_row();
            if field == "identityMarkerId" {
                candidate["marker"][field] = serde_json::json!("/var/lib/d2b/zones/work");
            } else {
                candidate[field] = serde_json::json!("/var/lib/d2b/zones/work");
            }
            assert!(
                serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
                "host path must be rejected in {field}"
            );
        }
    }

    #[test]
    fn every_invariant_is_required() {
        for field in [
            "zoneStoreId",
            "storageOwnerPrincipal",
            "parentDirectoryId",
            "ownership",
            "filesystem",
            "locking",
            "marker",
            "replacementDetection",
            "fsync",
            "publication",
        ] {
            let mut candidate = valid_row();
            candidate.as_object_mut().expect("row object").remove(field);
            assert!(
                serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
                "missing invariant {field} must be rejected"
            );
        }

        for field in ["owner", "group", "mode", "linkCount"] {
            let mut candidate = valid_row();
            candidate["ownership"]
                .as_object_mut()
                .expect("ownership object")
                .remove(field);
            assert!(
                serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
                "missing ownership invariant {field} must be rejected"
            );
        }

        let mut marker = valid_row();
        marker["marker"]
            .as_object_mut()
            .expect("marker object")
            .remove("identityMarkerId");
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(marker).is_err());

        for field in ["descriptor", "replacement"] {
            let mut candidate = valid_row();
            candidate["publication"]
                .as_object_mut()
                .expect("publication object")
                .remove(field);
            assert!(
                serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
                "missing publication invariant {field} must be rejected"
            );
        }
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_object_layer() {
        let mut top_level = valid_row();
        top_level["hostPath"] = serde_json::json!("/var/lib/d2b/zones/work/store.redb");
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(top_level).is_err());

        let mut ownership = valid_row();
        ownership["ownership"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(ownership).is_err());

        let mut marker = valid_row();
        marker["marker"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(marker).is_err());

        let mut publication = valid_row();
        publication["publication"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(publication).is_err());
    }

    #[test]
    fn link_count_is_closed() {
        let mut links = valid_row();
        links["ownership"]["linkCount"] = serde_json::json!(2);
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(links).is_err());

        let mut mode = valid_row();
        mode["ownership"]["mode"] = serde_json::json!("not-a-mode");
        assert!(serde_json::from_value::<ZoneStoreStorageRow>(mode).is_err());
    }
}
