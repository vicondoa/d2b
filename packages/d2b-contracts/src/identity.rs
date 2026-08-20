//! Validated identity primitives shared by the v3 resource plane.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{
        InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation, SubschemaValidation,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum byte length of a resource name.
pub const MAX_RESOURCE_NAME_BYTES: usize = 63;
/// Maximum byte length of a standard ResourceType name.
pub const MAX_RESOURCE_TYPE_SEGMENT_BYTES: usize = 63;
/// Maximum byte length of a qualified ResourceType name.
pub const MAX_QUALIFIED_RESOURCE_TYPE_BYTES: usize = 137;
/// Domain tag for the content-addressed resource-bundle generation identity.
pub const RESOURCE_BUNDLE_GENERATION_DOMAIN_TAG: &str = "d2b:v3:resource-bundle";

const LABEL_PATTERN: &str = "^[a-z][a-z0-9-]{0,62}$";
const QUALIFIED_RESOURCE_TYPE_PATTERN: &str =
    "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62}$";
const UUID_V4_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";
const SHA256_PATTERN: &str = "^sha256:[0-9a-f]{64}$";
const RESOURCE_TYPE_QUALIFIER: &str = ".d2bus.org.";

/// The complete standard ResourceType catalog.
pub const STANDARD_RESOURCE_TYPES: [&str; 19] = [
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "EmergencyPolicy",
    "Host",
    "Guest",
    "Process",
    "EphemeralProcess",
    "Volume",
    "Network",
    "Device",
    "User",
    "Credential",
    "Endpoint",
    "ResourceExport",
    "ResourceImport",
];

/// Identity class used by typed validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityClass {
    ZoneId,
    ResourceName,
    ResourceTypeName,
    ResourceUid,
    SessionPurpose,
    ServiceName,
    SchemaFingerprint,
    BindingDigest,
    ResourceBundleGenerationId,
    TranscriptHash,
    Timestamp,
    ResourceGeneration,
    ReconnectGeneration,
    ControllerGeneration,
    ConfigurationGeneration,
}

/// Reason a canonical identity could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    Empty {
        class: IdentityClass,
    },
    TooLong {
        class: IdentityClass,
        max_bytes: usize,
    },
    InvalidShape {
        class: IdentityClass,
    },
    UnknownStandardResourceType,
    Zero {
        class: IdentityClass,
    },
}

impl core::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty { class } => write!(f, "{class:?} must not be empty"),
            Self::TooLong { class, max_bytes } => {
                write!(f, "{class:?} exceeds {max_bytes} bytes")
            }
            Self::InvalidShape { class } => write!(f, "{class:?} has an invalid shape"),
            Self::UnknownStandardResourceType => {
                f.write_str("unqualified ResourceType is not in the standard catalog")
            }
            Self::Zero { class } => write!(f, "{class:?} must be nonzero"),
        }
    }
}

impl std::error::Error for IdentityError {}

fn string_schema(pattern: &str, min: u32, max: u32) -> Schema {
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

fn enum_string_schema(values: &[&str], max: u32) -> Schema {
    Schema::Object(SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        enum_values: Some(
            values
                .iter()
                .map(|value| serde_json::Value::String((*value).to_owned()))
                .collect(),
        ),
        string: Some(Box::new(StringValidation {
            max_length: Some(max),
            min_length: Some(1),
            ..Default::default()
        })),
        ..Default::default()
    })
}

fn is_lower_label(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_type_segment(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z')) && bytes.all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_label(
    value: impl Into<String>,
    class: IdentityClass,
    max_bytes: usize,
) -> Result<String, IdentityError> {
    let value = value.into();
    if value.is_empty() {
        return Err(IdentityError::Empty { class });
    }
    if value.len() > max_bytes {
        return Err(IdentityError::TooLong { class, max_bytes });
    }
    if !is_lower_label(&value) {
        return Err(IdentityError::InvalidShape { class });
    }
    Ok(value)
}

macro_rules! label_identity {
    ($name:ident, $class:expr, $max:expr, $pattern:expr) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate and construct this identity.
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                parse_label(value, $class, $max).map(Self)
            }

            /// Borrow the canonical string.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Render the canonical value for an authorized encoding or key surface.
            pub fn to_canonical_string(&self) -> String {
                self.0.clone()
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}(<redacted>)", stringify!($name))
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}(<redacted>)", stringify!($name))
            }
        }

        impl core::str::FromStr for $name {
            type Err = IdentityError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
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

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                string_schema($pattern, 1, $max as u32)
            }
        }
    };
}

label_identity!(
    ResourceName,
    IdentityClass::ResourceName,
    MAX_RESOURCE_NAME_BYTES,
    LABEL_PATTERN
);

/// A standard or Provider-qualified ResourceType name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ResourceTypeName(String);

impl ResourceTypeName {
    /// Parse a standard catalog name or a qualified Provider type.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::Empty {
                class: IdentityClass::ResourceTypeName,
            });
        }
        if value.len() > MAX_QUALIFIED_RESOURCE_TYPE_BYTES {
            return Err(IdentityError::TooLong {
                class: IdentityClass::ResourceTypeName,
                max_bytes: MAX_QUALIFIED_RESOURCE_TYPE_BYTES,
            });
        }

        if let Some((provider, local_type)) = value.split_once(RESOURCE_TYPE_QUALIFIER) {
            if local_type.contains(RESOURCE_TYPE_QUALIFIER)
                || provider.len() > MAX_RESOURCE_TYPE_SEGMENT_BYTES
                || local_type.len() > MAX_RESOURCE_TYPE_SEGMENT_BYTES
                || !is_lower_label(provider)
                || !is_type_segment(local_type)
            {
                return Err(IdentityError::InvalidShape {
                    class: IdentityClass::ResourceTypeName,
                });
            }
        } else {
            if value.len() > MAX_RESOURCE_TYPE_SEGMENT_BYTES || !is_type_segment(&value) {
                return Err(IdentityError::InvalidShape {
                    class: IdentityClass::ResourceTypeName,
                });
            }
            if !STANDARD_RESOURCE_TYPES.contains(&value.as_str()) {
                return Err(IdentityError::UnknownStandardResourceType);
            }
        }

        Ok(Self(value))
    }

    /// Borrow the canonical string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Render the canonical value for an authorized encoding or key surface.
    pub fn to_canonical_string(&self) -> String {
        self.0.clone()
    }

    /// Whether this is one of the standard unqualified ResourceTypes.
    pub fn is_standard(&self) -> bool {
        !self.0.contains(RESOURCE_TYPE_QUALIFIER)
    }
}

impl core::fmt::Display for ResourceTypeName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceTypeName(<redacted>)")
    }
}

impl core::fmt::Debug for ResourceTypeName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceTypeName(<redacted>)")
    }
}

impl core::str::FromStr for ResourceTypeName {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ResourceTypeName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ResourceTypeName {
    fn schema_name() -> String {
        "ResourceTypeName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(vec![
                    enum_string_schema(
                        &STANDARD_RESOURCE_TYPES,
                        MAX_RESOURCE_TYPE_SEGMENT_BYTES as u32,
                    ),
                    string_schema(
                        QUALIFIED_RESOURCE_TYPE_PATTERN,
                        13,
                        MAX_QUALIFIED_RESOURCE_TYPE_BYTES as u32,
                    ),
                ]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// Immutable store-generated UUIDv4 resource identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ResourceUid(String);

impl ResourceUid {
    /// Parse a canonical lowercase RFC 9562 UUIDv4 string.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::Empty {
                class: IdentityClass::ResourceUid,
            });
        }
        if value.len() > 36 {
            return Err(IdentityError::TooLong {
                class: IdentityClass::ResourceUid,
                max_bytes: 36,
            });
        }

        let bytes = value.as_bytes();
        let valid = bytes.len() == 36
            && [8, 13, 18, 23]
                .into_iter()
                .all(|index| bytes[index] == b'-')
            && bytes.iter().enumerate().all(|(index, byte)| {
                [8, 13, 18, 23].contains(&index)
                    || byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            })
            && bytes[14] == b'4'
            && matches!(bytes[19], b'8' | b'9' | b'a' | b'b');
        if !valid {
            return Err(IdentityError::InvalidShape {
                class: IdentityClass::ResourceUid,
            });
        }
        Ok(Self(value))
    }

    /// Borrow the canonical UUID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Render the canonical value for an authorized encoding or key surface.
    pub fn to_canonical_string(&self) -> String {
        self.0.clone()
    }
}

impl core::fmt::Display for ResourceUid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceUid(<redacted>)")
    }
}

impl core::fmt::Debug for ResourceUid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceUid(<redacted>)")
    }
}

impl core::str::FromStr for ResourceUid {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ResourceUid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ResourceUid {
    fn schema_name() -> String {
        "ResourceUid".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        string_schema(UUID_V4_PATTERN, 36, 36)
    }
}

fn parse_sha256(value: impl Into<String>, class: IdentityClass) -> Result<String, IdentityError> {
    let value = value.into();
    if value.is_empty() {
        return Err(IdentityError::Empty { class });
    }
    if value.len() > 71 {
        return Err(IdentityError::TooLong {
            class,
            max_bytes: 71,
        });
    }
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    });
    if !valid {
        return Err(IdentityError::InvalidShape { class });
    }
    Ok(value)
}

macro_rules! digest_identity {
    ($name:ident, $class:expr, clear_debug = $clear:expr) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parse a canonical SHA-256 digest.
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                parse_sha256(value, $class).map(Self)
            }

            /// Borrow the canonical digest.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let _ = $clear;
                write!(f, "{}(<redacted>)", stringify!($name))
            }
        }

        impl core::str::FromStr for $name {
            type Err = IdentityError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
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

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                string_schema(SHA256_PATTERN, 71, 71)
            }
        }
    };
}

digest_identity!(
    ResourceBundleGenerationId,
    IdentityClass::ResourceBundleGenerationId,
    clear_debug = true
);

impl ResourceBundleGenerationId {
    /// Return the domain tag used to compute this generation identity.
    pub const fn domain_tag() -> &'static str {
        RESOURCE_BUNDLE_GENERATION_DOMAIN_TAG
    }
}

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
