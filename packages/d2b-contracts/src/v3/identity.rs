//! Validated identity primitives shared by the v3 resource plane.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{
        InstanceType, NumberValidation, Schema, SchemaObject, SingleOrVec, StringValidation,
        SubschemaValidation,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum byte length of a Zone or resource name.
pub const MAX_RESOURCE_NAME_BYTES: usize = 63;
/// Maximum byte length of a standard ResourceType name.
pub const MAX_RESOURCE_TYPE_SEGMENT_BYTES: usize = 63;
/// Maximum byte length of a qualified ResourceType name.
pub const MAX_QUALIFIED_RESOURCE_TYPE_BYTES: usize = 137;

const LABEL_PATTERN: &str = "^[a-z][a-z0-9-]{0,62}$";
const QUALIFIED_RESOURCE_TYPE_PATTERN: &str =
    "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62}$";
const UUID_V4_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";
const SESSION_PURPOSE_PATTERN: &str = "^[a-z][a-z0-9-]{0,63}$";
const SERVICE_NAME_PATTERN: &str = "^[a-z][a-z0-9]*(\\.[a-z0-9]+)+$";
const SHA256_PATTERN: &str = "^sha256:[0-9a-f]{64}$";
const TIMESTAMP_PATTERN: &str =
    "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}Z$";
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
    TranscriptHash,
    Timestamp,
    ResourceGeneration,
    ReconnectGeneration,
    ControllerGeneration,
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

fn unsigned_schema(minimum: f64) -> Schema {
    Schema::Object(SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Integer))),
        number: Some(Box::new(NumberValidation {
            minimum: Some(minimum),
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
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
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
    ZoneId,
    IdentityClass::ZoneId,
    MAX_RESOURCE_NAME_BYTES,
    LABEL_PATTERN
);
label_identity!(
    ResourceName,
    IdentityClass::ResourceName,
    MAX_RESOURCE_NAME_BYTES,
    LABEL_PATTERN
);
label_identity!(
    SessionPurpose,
    IdentityClass::SessionPurpose,
    64,
    SESSION_PURPOSE_PATTERN
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

    /// Whether this is one of the standard unqualified ResourceTypes.
    pub fn is_standard(&self) -> bool {
        !self.0.contains(RESOURCE_TYPE_QUALIFIER)
    }
}

impl core::fmt::Display for ResourceTypeName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::fmt::Debug for ResourceTypeName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ResourceTypeName").field(&self.0).finish()
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
}

impl core::fmt::Display for ResourceUid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::fmt::Debug for ResourceUid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ResourceUid(<{} bytes>)", self.0.len())
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

/// A canonical millisecond-precision UTC timestamp.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// Parse exactly `YYYY-MM-DDTHH:MM:SS.sssZ`.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::Empty {
                class: IdentityClass::Timestamp,
            });
        }
        if value.len() > 24 {
            return Err(IdentityError::TooLong {
                class: IdentityClass::Timestamp,
                max_bytes: 24,
            });
        }
        if !is_valid_timestamp(&value) {
            return Err(IdentityError::InvalidShape {
                class: IdentityClass::Timestamp,
            });
        }
        Ok(Self(value))
    }

    /// Borrow the canonical timestamp.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
        || bytes.iter().enumerate().any(|(index, byte)| {
            ![4, 7, 10, 13, 16, 19, 23].contains(&index) && !byte.is_ascii_digit()
        })
    {
        return false;
    }

    let number = |start: usize, end: usize| {
        value[start..end]
            .parse::<u32>()
            .expect("validated ASCII digits")
    };
    let year = number(0, 4);
    let month = number(5, 7);
    let day = number(8, 10);
    let hour = number(11, 13);
    let minute = number(14, 16);
    let second = number(17, 19);
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }

    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

impl core::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Timestamp").field(&self.0).finish()
    }
}

impl core::str::FromStr for Timestamp {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Timestamp {
    fn schema_name() -> String {
        "Timestamp".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        string_schema(TIMESTAMP_PATTERN, 24, 24)
    }
}

/// A ComponentSession purpose selected by trusted endpoint policy.
pub type ValidatedSessionPurpose = SessionPurpose;

/// A validated resource-service name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ServiceName(String);

impl ServiceName {
    /// Parse a dotted lowercase service name.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::Empty {
                class: IdentityClass::ServiceName,
            });
        }
        if value.len() > 128 {
            return Err(IdentityError::TooLong {
                class: IdentityClass::ServiceName,
                max_bytes: 128,
            });
        }
        let mut segments = value.split('.');
        let first = segments.next().unwrap_or_default();
        let remaining: Vec<_> = segments.collect();
        let valid_first = {
            let mut bytes = first.bytes();
            matches!(bytes.next(), Some(b'a'..=b'z'))
                && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        };
        let valid_remaining = !remaining.is_empty()
            && remaining.iter().all(|segment| {
                !segment.is_empty()
                    && segment
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });
        if !valid_first || !valid_remaining {
            return Err(IdentityError::InvalidShape {
                class: IdentityClass::ServiceName,
            });
        }
        Ok(Self(value))
    }

    /// Borrow the canonical service name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for ServiceName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::fmt::Debug for ServiceName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ServiceName").field(&self.0).finish()
    }
}

impl core::str::FromStr for ServiceName {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ServiceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ServiceName {
    fn schema_name() -> String {
        "ServiceName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        string_schema(SERVICE_NAME_PATTERN, 3, 128)
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
    let valid = value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
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
                if $clear {
                    f.debug_tuple(stringify!($name)).field(&self.0).finish()
                } else {
                    write!(f, "{}(<{} bytes>)", stringify!($name), self.0.len())
                }
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
    SchemaFingerprint,
    IdentityClass::SchemaFingerprint,
    clear_debug = true
);
digest_identity!(
    BindingDigest,
    IdentityClass::BindingDigest,
    clear_debug = false
);

macro_rules! nonzero_generation {
    ($name:ident, $class:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Construct a nonzero generation.
            pub fn new(value: u64) -> Result<Self, IdentityError> {
                if value == 0 {
                    return Err(IdentityError::Zero { class: $class });
                }
                Ok(Self(value))
            }

            /// Return the numeric generation.
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                unsigned_schema(1.0)
            }
        }
    };
}

nonzero_generation!(ResourceGeneration, IdentityClass::ResourceGeneration);
nonzero_generation!(ReconnectGeneration, IdentityClass::ReconnectGeneration);
nonzero_generation!(ControllerGeneration, IdentityClass::ControllerGeneration);

/// The latest generation a controller has observed, with zero meaning none.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ObservedGeneration(u64);

impl ObservedGeneration {
    /// Construct an observed generation.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the numeric generation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A Zone-local commit revision, with zero denoting an empty store.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ZoneRevision(u64);

impl ZoneRevision {
    /// Construct a Zone revision.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the numeric revision.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Return the next revision, or `None` on numeric exhaustion.
    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// Trusted evidence class used to establish a subject context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceClass {
    UnixPeer,
    EnrolledKk,
    BootstrapIkpsk2,
    NativeVsock,
}

/// Transport locality relevant to resource authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Locality {
    Local,
    AdjacentZone,
    Remote,
}

/// Redacted binding of an authenticated transport to its locality.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransportBinding {
    locality: Locality,
    binding_digest: BindingDigest,
}

impl TransportBinding {
    /// Construct a transport binding from validated components.
    pub fn new(locality: Locality, binding_digest: BindingDigest) -> Self {
        Self {
            locality,
            binding_digest,
        }
    }

    /// Return the transport locality.
    pub const fn locality(&self) -> Locality {
        self.locality
    }

    /// Borrow the channel-binding digest.
    pub fn binding_digest(&self) -> &BindingDigest {
        &self.binding_digest
    }
}

impl core::fmt::Debug for TransportBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransportBinding")
            .field("locality", &self.locality)
            .field("binding_digest", &"<redacted>")
            .finish()
    }
}

/// Opaque 32-byte transcript binding.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TranscriptHash([u8; 32]);

impl TranscriptHash {
    /// Construct from the exact transcript hash bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parse exactly 64 lowercase hexadecimal characters.
    pub fn parse_hex(value: &str) -> Result<Self, IdentityError> {
        if value.is_empty() {
            return Err(IdentityError::Empty {
                class: IdentityClass::TranscriptHash,
            });
        }
        if value.len() > 64 {
            return Err(IdentityError::TooLong {
                class: IdentityClass::TranscriptHash,
                max_bytes: 64,
            });
        }
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(IdentityError::InvalidShape {
                class: IdentityClass::TranscriptHash,
            });
        }

        let mut bytes = [0_u8; 32];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *slot = u8::from_str_radix(&value[offset..offset + 2], 16).map_err(|_| {
                IdentityError::InvalidShape {
                    class: IdentityClass::TranscriptHash,
                }
            })?;
        }
        Ok(Self(bytes))
    }

    /// Borrow the exact transcript hash bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Render lowercase hexadecimal only when explicitly requested.
    pub fn to_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        output
    }
}

impl core::fmt::Debug for TranscriptHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TranscriptHash(<32 bytes>)")
    }
}

impl Serialize for TranscriptHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for TranscriptHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse_hex(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for TranscriptHash {
    fn schema_name() -> String {
        "TranscriptHash".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        string_schema("^[0-9a-f]{64}$", 64, 64)
    }
}

/// Session-bound values required by an authenticated subject context.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionBinding {
    schema_fingerprint: SchemaFingerprint,
    transport_binding: TransportBinding,
    reconnect_generation: ReconnectGeneration,
    transcript_hash: TranscriptHash,
}

impl SessionBinding {
    /// Construct a session binding from validated components.
    pub fn new(
        schema_fingerprint: SchemaFingerprint,
        transport_binding: TransportBinding,
        reconnect_generation: ReconnectGeneration,
        transcript_hash: TranscriptHash,
    ) -> Self {
        Self {
            schema_fingerprint,
            transport_binding,
            reconnect_generation,
            transcript_hash,
        }
    }
}

/// Trusted subject identity supplied to authorization after authentication.
///
/// Fields are private and this type deliberately has no `Deserialize`
/// implementation. Peers cannot construct or alter it through a payload.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatedSubjectContext {
    subject_ref: crate::v3::resource_ref::ResourceRef,
    subject_uid: ResourceUid,
    zone_ref: crate::v3::resource_ref::ResourceRef,
    evidence_class: EvidenceClass,
    execution_ref: Option<crate::v3::resource_ref::ResourceRef>,
    provider_ref: Option<crate::v3::resource_ref::ResourceRef>,
    process_ref: Option<crate::v3::resource_ref::ResourceRef>,
    controller_generation: Option<ControllerGeneration>,
    provider_generation: Option<ResourceGeneration>,
    session_purpose: SessionPurpose,
    service: ServiceName,
    schema_fingerprint: SchemaFingerprint,
    transport_binding: TransportBinding,
    reconnect_generation: ReconnectGeneration,
    transcript_hash: TranscriptHash,
}

impl AuthenticatedSubjectContext {
    /// Construct the required authenticated identity and session fields.
    pub fn new(
        subject_ref: crate::v3::resource_ref::ResourceRef,
        subject_uid: ResourceUid,
        zone_ref: crate::v3::resource_ref::ResourceRef,
        evidence_class: EvidenceClass,
        session_purpose: SessionPurpose,
        service: ServiceName,
        session: SessionBinding,
    ) -> Self {
        Self {
            subject_ref,
            subject_uid,
            zone_ref,
            evidence_class,
            execution_ref: None,
            provider_ref: None,
            process_ref: None,
            controller_generation: None,
            provider_generation: None,
            session_purpose,
            service,
            schema_fingerprint: session.schema_fingerprint,
            transport_binding: session.transport_binding,
            reconnect_generation: session.reconnect_generation,
            transcript_hash: session.transcript_hash,
        }
    }

    /// Bind the execution resource established by trusted evidence.
    pub fn with_execution_ref(mut self, value: crate::v3::resource_ref::ResourceRef) -> Self {
        self.execution_ref = Some(value);
        self
    }

    /// Bind the Provider resource established by trusted evidence.
    pub fn with_provider_ref(mut self, value: crate::v3::resource_ref::ResourceRef) -> Self {
        self.provider_ref = Some(value);
        self
    }

    /// Bind the Process resource established by trusted evidence.
    pub fn with_process_ref(mut self, value: crate::v3::resource_ref::ResourceRef) -> Self {
        self.process_ref = Some(value);
        self
    }

    /// Bind the controller generation established by trusted evidence.
    pub fn with_controller_generation(mut self, value: ControllerGeneration) -> Self {
        self.controller_generation = Some(value);
        self
    }

    /// Bind the Provider resource generation established by trusted evidence.
    pub fn with_provider_generation(mut self, value: ResourceGeneration) -> Self {
        self.provider_generation = Some(value);
        self
    }

    /// Borrow the authenticated subject reference.
    pub fn subject_ref(&self) -> &crate::v3::resource_ref::ResourceRef {
        &self.subject_ref
    }

    /// Borrow the immutable authenticated subject UID.
    pub fn subject_uid(&self) -> &ResourceUid {
        &self.subject_uid
    }

    /// Borrow the subject's Zone self-resource reference.
    pub fn zone_ref(&self) -> &crate::v3::resource_ref::ResourceRef {
        &self.zone_ref
    }

    /// Return the evidence class.
    pub const fn evidence_class(&self) -> EvidenceClass {
        self.evidence_class
    }

    /// Borrow the optional execution reference.
    pub fn execution_ref(&self) -> Option<&crate::v3::resource_ref::ResourceRef> {
        self.execution_ref.as_ref()
    }

    /// Borrow the optional Provider reference.
    pub fn provider_ref(&self) -> Option<&crate::v3::resource_ref::ResourceRef> {
        self.provider_ref.as_ref()
    }

    /// Borrow the optional Process reference.
    pub fn process_ref(&self) -> Option<&crate::v3::resource_ref::ResourceRef> {
        self.process_ref.as_ref()
    }

    /// Return the optional controller generation.
    pub const fn controller_generation(&self) -> Option<ControllerGeneration> {
        self.controller_generation
    }

    /// Return the optional Provider resource generation.
    pub const fn provider_generation(&self) -> Option<ResourceGeneration> {
        self.provider_generation
    }

    /// Borrow the authenticated session purpose.
    pub fn session_purpose(&self) -> &SessionPurpose {
        &self.session_purpose
    }

    /// Borrow the selected service name.
    pub fn service(&self) -> &ServiceName {
        &self.service
    }

    /// Borrow the negotiated schema fingerprint.
    pub fn schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.schema_fingerprint
    }

    /// Borrow the authenticated transport binding.
    pub fn transport_binding(&self) -> &TransportBinding {
        &self.transport_binding
    }

    /// Return the reconnect generation.
    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    /// Borrow the transcript hash.
    pub fn transcript_hash(&self) -> &TranscriptHash {
        &self.transcript_hash
    }
}

impl core::fmt::Debug for AuthenticatedSubjectContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AuthenticatedSubjectContext(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_UUIDS: &[&str] = &[
        "123e4567-e89b-42d3-a456-426614174000",
        "ffffffff-ffff-4fff-bfff-ffffffffffff",
    ];
    const INVALID_UUIDS: &[&str] = &[
        "",
        "123e4567-e89b-12d3-a456-426614174000",
        "123e4567-e89b-42d3-c456-426614174000",
        "123E4567-E89B-42D3-A456-426614174000",
        "{123e4567-e89b-42d3-a456-426614174000}",
        "123e4567e89b42d3a456426614174000",
    ];

    #[test]
    fn name_bounds_and_grammar_are_exact() {
        for value in ["a", "work", "a-0", &format!("a{}", "z".repeat(62))] {
            assert!(ZoneId::parse(value).is_ok(), "{value}");
            assert!(ResourceName::parse(value).is_ok(), "{value}");
        }
        for value in ["", "A", "0a", "-a", "a_b", &format!("a{}", "z".repeat(63))] {
            assert!(ZoneId::parse(value).is_err(), "{value}");
            assert!(ResourceName::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn resource_type_vectors_pin_standard_and_qualified_spellings() {
        for value in STANDARD_RESOURCE_TYPES {
            let parsed = ResourceTypeName::parse(value).expect("standard type");
            assert!(parsed.is_standard());
        }
        let maximum_qualified = format!("a{}.d2bus.org.A{}", "z".repeat(62), "z".repeat(62));
        for value in [
            "acme.d2bus.org.Widget",
            "display-wayland.d2bus.org.WaylandSession",
            &maximum_qualified,
        ] {
            let parsed = ResourceTypeName::parse(value).expect("qualified type");
            assert!(!parsed.is_standard());
        }
        for value in [
            "Widget",
            "acme.io.Widget",
            "d2bus.org.acme.Widget",
            "acme.d2bus.org.widget",
            "acme.d2bus.org.Bad-Type",
            "Acme.d2bus.org.Widget",
            "acme.d2bus.org.Widget.extra",
        ] {
            assert!(ResourceTypeName::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn schemas_preserve_runtime_identity_bounds() {
        let mut generator = SchemaGenerator::default();
        let zone = ZoneId::json_schema(&mut generator);
        let resource_type = ResourceTypeName::json_schema(&mut generator);
        let Schema::Object(zone) = zone else {
            panic!("ZoneId schema must be an object");
        };
        let Schema::Object(resource_type) = resource_type else {
            panic!("ResourceTypeName schema must be an object");
        };

        let zone_string = zone.string.expect("ZoneId string validation");
        assert_eq!(zone_string.min_length, Some(1));
        assert_eq!(zone_string.max_length, Some(MAX_RESOURCE_NAME_BYTES as u32));
        assert_eq!(zone_string.pattern.as_deref(), Some(LABEL_PATTERN));

        let alternatives = resource_type
            .subschemas
            .expect("ResourceTypeName alternatives")
            .one_of
            .expect("ResourceTypeName oneOf");
        let Schema::Object(standard) = &alternatives[0] else {
            panic!("standard ResourceTypeName schema must be an object");
        };
        assert_eq!(
            standard.enum_values,
            Some(
                STANDARD_RESOURCE_TYPES
                    .iter()
                    .map(|value| serde_json::Value::String((*value).to_owned()))
                    .collect()
            )
        );
    }

    #[test]
    fn resource_uid_vectors_and_redaction_are_exact() {
        for value in VALID_UUIDS {
            let uid = ResourceUid::parse(*value).expect("valid UUIDv4");
            assert_eq!(uid.as_str(), *value);
            assert_eq!(format!("{uid:?}"), "ResourceUid(<36 bytes>)");
            assert!(!format!("{uid:?}").contains(value));
            let json = serde_json::to_string(&uid).expect("serialize");
            assert_eq!(serde_json::from_str::<ResourceUid>(&json).unwrap(), uid);
        }

        for value in INVALID_UUIDS {
            assert!(ResourceUid::parse(*value).is_err(), "{value}");
        }
    }

    #[test]
    fn timestamp_vectors_are_exact_and_calendar_valid() {
        for value in [
            "0001-01-01T00:00:00.000Z",
            "2000-02-29T23:59:59.999Z",
            "2026-07-25T14:52:58.123Z",
            "9999-12-31T23:59:59.999Z",
        ] {
            let timestamp = Timestamp::parse(value).expect("valid timestamp");
            assert_eq!(timestamp.as_str(), value);
            let json = serde_json::to_string(&timestamp).expect("serialize");
            assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), timestamp);
        }
        for value in [
            "",
            "0000-01-01T00:00:00.000Z",
            "1900-02-29T00:00:00.000Z",
            "2024-02-30T00:00:00.000Z",
            "2024-01-01T24:00:00.000Z",
            "2024-01-01T00:00:60.000Z",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00.000+00:00",
            "2024-01-01t00:00:00.000z",
        ] {
            assert!(Timestamp::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn numeric_generations_preserve_zero_semantics() {
        assert!(ResourceGeneration::new(0).is_err());
        assert!(ReconnectGeneration::new(0).is_err());
        assert!(ControllerGeneration::new(0).is_err());
        assert_eq!(ObservedGeneration::new(0).get(), 0);
        assert_eq!(
            ZoneRevision::new(0).checked_next(),
            Some(ZoneRevision::new(1))
        );
        assert_eq!(ZoneRevision::new(u64::MAX).checked_next(), None);
        assert!(serde_json::from_str::<ResourceGeneration>("0").is_err());
    }

    #[test]
    fn authenticated_context_is_wholly_redacted() {
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let context = AuthenticatedSubjectContext::new(
            "User/alice".parse().unwrap(),
            ResourceUid::parse(VALID_UUIDS[0]).unwrap(),
            "Zone/dev".parse().unwrap(),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse("resource-api").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(digest).unwrap(),
                TransportBinding::new(Locality::Local, BindingDigest::parse(digest).unwrap()),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([0x5a; 32]),
            ),
        )
        .with_provider_ref("Provider/system-core".parse().unwrap())
        .with_provider_generation(ResourceGeneration::new(2).unwrap());

        assert_eq!(
            format!("{context:?}"),
            "AuthenticatedSubjectContext(<redacted>)"
        );
        assert!(!format!("{context:?}").contains("alice"));
        assert_eq!(context.evidence_class(), EvidenceClass::UnixPeer);
        assert_eq!(context.provider_generation().unwrap().get(), 2);
        assert_eq!(context.transcript_hash().to_hex(), "5a".repeat(32));
        assert_eq!(
            format!("{:?}", context.transport_binding()),
            "TransportBinding { locality: Local, binding_digest: \"<redacted>\" }"
        );
    }

    #[test]
    fn component_types_reject_noncanonical_values() {
        assert!(SessionPurpose::parse("resource-api").is_ok());
        assert!(SessionPurpose::parse("A").is_err());
        assert!(ServiceName::parse("d2b.resource.v3").is_ok());
        assert!(ServiceName::parse("resource").is_err());
        assert!(ServiceName::parse("d2b.Resource.v3").is_err());
        assert!(SchemaFingerprint::parse("sha256:ABC").is_err());
        assert!(TranscriptHash::parse_hex(&"0".repeat(64)).is_ok());
        assert!(TranscriptHash::parse_hex(&"A".repeat(64)).is_err());
    }
}
