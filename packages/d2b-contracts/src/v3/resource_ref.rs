//! Canonical same-Zone resource references.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{
        InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation, SubschemaValidation,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::identity::{IdentityError, ResourceName, ResourceTypeName, STANDARD_RESOURCE_TYPES};

/// Maximum canonical ResourceRef byte length.
pub const MAX_RESOURCE_REF_BYTES: usize = 201;
const QUALIFIED_RESOURCE_REF_PATTERN: &str =
    "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62}/[a-z][a-z0-9-]{0,62}$";

/// Reason a canonical ResourceRef could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRefError {
    Empty,
    MissingSeparator,
    ExtraSeparator,
    Type(IdentityError),
    Name(IdentityError),
}

impl core::fmt::Display for ResourceRefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("ResourceRef must not be empty"),
            Self::MissingSeparator => {
                f.write_str("ResourceRef must contain one type/name separator")
            }
            Self::ExtraSeparator => f.write_str("ResourceRef must not contain a nested separator"),
            Self::Type(error) => write!(f, "invalid ResourceRef type: {error}"),
            Self::Name(error) => write!(f, "invalid ResourceRef name: {error}"),
        }
    }
}

impl std::error::Error for ResourceRefError {}

/// A canonical same-Zone `<ResourceType>/<resource_name>` reference.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceRef {
    resource_type: ResourceTypeName,
    name: ResourceName,
}

impl ResourceRef {
    /// Construct a reference from already validated components.
    pub const fn new(resource_type: ResourceTypeName, name: ResourceName) -> Self {
        Self {
            resource_type,
            name,
        }
    }

    /// Parse exactly one canonical type/name pair.
    pub fn parse(value: &str) -> Result<Self, ResourceRefError> {
        if value.is_empty() {
            return Err(ResourceRefError::Empty);
        }
        let (resource_type, name) = value
            .split_once('/')
            .ok_or(ResourceRefError::MissingSeparator)?;
        if name.contains('/') {
            return Err(ResourceRefError::ExtraSeparator);
        }
        Ok(Self::new(
            ResourceTypeName::parse(resource_type).map_err(ResourceRefError::Type)?,
            ResourceName::parse(name).map_err(ResourceRefError::Name)?,
        ))
    }

    /// Borrow the ResourceType component.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Borrow the resource-name component.
    pub const fn name(&self) -> &ResourceName {
        &self.name
    }

    /// Render the canonical reference for an authorized encoding or key surface.
    pub fn to_canonical_string(&self) -> String {
        format!("{}/{}", self.resource_type.as_str(), self.name.as_str())
    }
}

impl core::fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceRef(<redacted>)")
    }
}

impl core::fmt::Debug for ResourceRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceRef(<redacted>)")
    }
}

impl core::str::FromStr for ResourceRef {
    type Err = ResourceRefError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for ResourceRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for ResourceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ResourceRef {
    fn schema_name() -> String {
        "ResourceRef".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let standard_min = STANDARD_RESOURCE_TYPES
            .iter()
            .map(|value| value.len())
            .min()
            .expect("standard ResourceType catalog is nonempty") as u32
            + 2;
        let standard_max = STANDARD_RESOURCE_TYPES
            .iter()
            .map(|value| value.len())
            .max()
            .expect("standard ResourceType catalog is nonempty") as u32
            + 64;
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(vec![
                    reference_string_schema(
                        &format!(
                            "^({})/[a-z][a-z0-9-]{{0,62}}$",
                            STANDARD_RESOURCE_TYPES.join("|")
                        ),
                        standard_min,
                        standard_max,
                    ),
                    reference_string_schema(
                        QUALIFIED_RESOURCE_REF_PATTERN,
                        15,
                        MAX_RESOURCE_REF_BYTES as u32,
                    ),
                ]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

fn reference_string_schema(pattern: &str, min: u32, max: u32) -> Schema {
    Schema::Object(SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        string: Some(Box::new(StringValidation {
            max_length: Some(max),
            min_length: Some(min),
            pattern: Some(pattern.to_owned()),
        })),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::ResourceUid;

    const VALID_REF_VECTORS: &[&str] = &[
        "Zone/dev",
        "Provider/system-core",
        "Host/host-system",
        "Process/wayland-proxy",
        "User/alice",
        "Volume/work-state",
        "acme.d2bus.org.Widget/widget-1",
    ];
    const INVALID_REF_VECTORS: &[&str] = &[
        "",
        "Host",
        "/host",
        "Host/",
        "Host/dev/child",
        "host/dev",
        "Widget/dev",
        "acme.io.Widget/dev",
        "Zone/dev?query",
        "Zone/dev#fragment",
        "Zone/../dev",
        "dev/Zone/name",
        "d2b://Host/dev",
    ];

    #[test]
    fn golden_vectors_parse_and_round_trip() {
        for value in VALID_REF_VECTORS {
            let parsed = ResourceRef::parse(value).expect("valid ref");
            assert_eq!(parsed.to_canonical_string(), *value);
            assert_eq!(format!("{parsed:?}"), "ResourceRef(<redacted>)");
            let json = serde_json::to_string(&parsed).expect("serialize");
            assert_eq!(json, format!("\"{value}\""));
            assert_eq!(
                serde_json::from_str::<ResourceRef>(&format!("\"{value}\"")).unwrap(),
                parsed
            );
        }
        for value in INVALID_REF_VECTORS {
            assert!(ResourceRef::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn every_standard_type_has_an_unambiguous_reference() {
        let mut refs = std::collections::BTreeSet::new();
        for resource_type in STANDARD_RESOURCE_TYPES {
            let parsed = ResourceRef::parse(&format!("{resource_type}/same-name"))
                .expect("standard reference");
            assert!(refs.insert(parsed));
        }
        assert_eq!(refs.len(), STANDARD_RESOURCE_TYPES.len());

        let vendor = ResourceRef::parse("host.d2bus.org.Host/same-name").unwrap();
        let standard = ResourceRef::parse("Host/same-name").unwrap();
        assert_ne!(vendor, standard);
    }

    #[test]
    fn grammar_property_rejects_invalid_name_character_at_every_position() {
        let valid = "a".repeat(63);
        for index in 0..valid.len() {
            let mut candidate = valid.clone();
            candidate.replace_range(index..=index, "_");
            assert!(
                ResourceRef::parse(&format!("Host/{candidate}")).is_err(),
                "invalid character at {index}"
            );
        }
    }

    #[test]
    fn maximum_qualified_reference_is_exact() {
        let provider = format!("a{}", "z".repeat(62));
        let local_type = format!("A{}", "z".repeat(62));
        let name = format!("a{}", "z".repeat(62));
        let value = format!("{provider}.d2bus.org.{local_type}/{name}");
        assert_eq!(value.len(), MAX_RESOURCE_REF_BYTES);
        assert_eq!(
            ResourceRef::parse(&value).unwrap().to_canonical_string(),
            value
        );
    }

    #[test]
    fn schema_excludes_unknown_unqualified_resource_types() {
        let Schema::Object(schema) = ResourceRef::json_schema(&mut SchemaGenerator::default())
        else {
            panic!("ResourceRef schema must be an object");
        };
        let alternatives = schema
            .subschemas
            .expect("ResourceRef alternatives")
            .one_of
            .expect("ResourceRef oneOf");
        let Schema::Object(standard) = &alternatives[0] else {
            panic!("standard ResourceRef schema must be an object");
        };
        let pattern = standard
            .string
            .as_ref()
            .and_then(|string| string.pattern.as_deref())
            .expect("standard ResourceRef pattern");
        assert!(pattern.contains("ResourceImport"));
        assert!(!pattern.contains("|Widget|"));
    }

    #[test]
    fn name_recreation_does_not_reuse_immutable_identity() {
        let resource_ref = ResourceRef::parse("Host/work").unwrap();
        let first_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let recreated_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap();

        assert_eq!(resource_ref, ResourceRef::parse("Host/work").unwrap());
        assert_ne!(first_uid, recreated_uid);
    }

    #[test]
    fn reference_diagnostics_redact_both_identity_components() {
        let nonce = u64::from(std::process::id());
        let name_marker = format!("name-{nonce:x}");
        let type_marker = format!("provider-{nonce:x}.d2bus.org.Marker");
        let canonical = format!("{type_marker}/{name_marker}");
        let reference = ResourceRef::parse(&canonical).unwrap();

        for rendered in [
            format!("{reference:?}"),
            format!("{reference}"),
            reference.to_string(),
        ] {
            assert!(!rendered.contains(&name_marker));
            assert!(!rendered.contains(&type_marker));
        }
        assert_eq!(reference.to_canonical_string(), canonical);
        assert_eq!(
            serde_json::to_string(&reference).unwrap(),
            format!("\"{canonical}\"")
        );
    }
}
