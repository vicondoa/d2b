//! Canonical JSON and schema bindings for v3 resources.

use std::collections::{BTreeMap, BTreeSet};

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec},
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::{
    ResourceEnvelope, ResourceName, ResourceRef, ResourceTypeName, SchemaFingerprint,
    resource_status::ProviderStatusExtension,
};

/// Canonical JSON profile identifier.
pub const CANONICAL_JSON_PROFILE: &str = "d2b-cjson/v1";
/// Domain tag for complete resource envelopes.
pub const RESOURCE_ENVELOPE_DOMAIN_TAG: &str = "d2b:v3:resource-envelope";
/// Domain tag for resource specs.
pub const RESOURCE_SPEC_DOMAIN_TAG: &str = "d2b:v3:resource-spec";
/// Domain tag for resource status.
pub const RESOURCE_STATUS_DOMAIN_TAG: &str = "d2b:v3:resource-status";
/// Domain tag for schemas.
pub const SCHEMA_DOMAIN_TAG: &str = "d2b:v3:schema";
/// Maximum bytes in a printable ASCII canonical JSON object key.
pub const MAX_CANONICAL_KEY_BYTES: usize = 64;

/// Closed serde JSON failure class retained without attacker-controlled text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalJsonCodecReason {
    Io,
    Syntax,
    Data,
    UnexpectedEof,
}

impl CanonicalJsonCodecReason {
    /// Return the closed diagnostic label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "I/O",
            Self::Syntax => "syntax",
            Self::Data => "data",
            Self::UnexpectedEof => "unexpected EOF",
        }
    }
}

/// Failure to parse or render the canonical JSON profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalJsonError {
    Syntax {
        reason: CanonicalJsonCodecReason,
        line: u32,
        column: u32,
    },
    DuplicateKey {
        key_ordinal: u32,
        line: u32,
        column: u32,
    },
    InvalidKey,
    InvalidString,
    IntegerOutOfRange,
    RootNotObject,
}

impl core::fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Syntax {
                reason,
                line,
                column,
            } => write!(
                f,
                "invalid canonical JSON: {} at line {line}, column {column}",
                reason.as_str()
            ),
            Self::DuplicateKey {
                key_ordinal,
                line,
                column,
            } => write!(
                f,
                "duplicate canonical JSON key at ordinal {key_ordinal}, line {line}, column {column}"
            ),
            Self::InvalidKey => {
                f.write_str("canonical JSON key is not printable ASCII or is too long")
            }
            Self::InvalidString => {
                f.write_str("canonical JSON string is not NFC or contains a forbidden character")
            }
            Self::IntegerOutOfRange => {
                f.write_str("canonical JSON numbers must be signed 64-bit integers")
            }
            Self::RootNotObject => f.write_str("canonical JSON value must be an object"),
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

const DUPLICATE_KEY_MARKER: &str = "d2b-cjson duplicate-key ordinal=";

struct DuplicateKeyMarker(u32);

impl core::fmt::Display for DuplicateKeyMarker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{DUPLICATE_KEY_MARKER}{}", self.0)
    }
}

fn bounded_position(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(crate) fn serde_json_error_metadata(
    error: &serde_json::Error,
) -> (CanonicalJsonCodecReason, u32, u32) {
    let reason = match error.classify() {
        serde_json::error::Category::Io => CanonicalJsonCodecReason::Io,
        serde_json::error::Category::Syntax => CanonicalJsonCodecReason::Syntax,
        serde_json::error::Category::Data => CanonicalJsonCodecReason::Data,
        serde_json::error::Category::Eof => CanonicalJsonCodecReason::UnexpectedEof,
    };
    (
        reason,
        bounded_position(error.line()),
        bounded_position(error.column()),
    )
}

fn canonical_json_error(error: serde_json::Error) -> CanonicalJsonError {
    let (reason, line, column) = serde_json_error_metadata(&error);
    let rendered = error.to_string();
    if let Some(key_ordinal) = rendered
        .strip_prefix(DUPLICATE_KEY_MARKER)
        .and_then(|suffix| suffix.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok())
    {
        CanonicalJsonError::DuplicateKey {
            key_ordinal,
            line,
            column,
        }
    } else {
        CanonicalJsonError::Syntax {
            reason,
            line,
            column,
        }
    }
}

/// A JSON value that cannot contain data outside `d2b-cjson/v1`.
#[derive(Clone, PartialEq, Eq)]
pub enum CanonicalJsonValue {
    Null,
    Bool(bool),
    Integer(i64),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl CanonicalJsonValue {
    /// Parse JSON while rejecting duplicate keys before constructing a value.
    pub fn parse(bytes: &[u8]) -> Result<Self, CanonicalJsonError> {
        validate_number_tokens(bytes)?;
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let value = Self::deserialize(&mut deserializer).map_err(canonical_json_error)?;
        deserializer.end().map_err(canonical_json_error)?;
        Ok(value)
    }

    /// Render the exact canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("canonical JSON values always serialize")
    }

    /// Return this value as an object.
    pub fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }
}

impl core::fmt::Debug for CanonicalJsonValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CanonicalJsonValue(<redacted>)")
    }
}

impl Serialize for CanonicalJsonValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            }
            Self::Object(values) => {
                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

struct CanonicalJsonVisitor;

impl<'de> Visitor<'de> for CanonicalJsonVisitor {
    type Value = CanonicalJsonValue;

    fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("a value in the d2b canonical JSON profile")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(CanonicalJsonValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(CanonicalJsonValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(CanonicalJsonValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(CanonicalJsonValue::Integer(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        i64::try_from(value)
            .map(CanonicalJsonValue::Integer)
            .map_err(|_| E::custom(CanonicalJsonError::IntegerOutOfRange))
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Err(E::custom("floating-point JSON is forbidden"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_canonical_string(value)
            .map_err(E::custom)
            .map(|()| CanonicalJsonValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_canonical_string(&value)
            .map_err(E::custom)
            .map(|()| CanonicalJsonValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(CanonicalJsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        let mut key_ordinal = 0_u32;
        while let Some(key) = map.next_key::<String>()? {
            key_ordinal = key_ordinal.saturating_add(1);
            validate_canonical_key(&key).map_err(serde::de::Error::custom)?;
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom(DuplicateKeyMarker(key_ordinal)));
            }
            let value = map.next_value()?;
            values.insert(key, value);
        }
        Ok(CanonicalJsonValue::Object(values))
    }
}

impl<'de> Deserialize<'de> for CanonicalJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CanonicalJsonVisitor)
    }
}

impl JsonSchema for CanonicalJsonValue {
    fn schema_name() -> String {
        "CanonicalJsonValue".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Bool(true)
    }
}

/// A canonical JSON object used for schema-bound dynamic fields.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CanonicalJsonObject(BTreeMap<String, CanonicalJsonValue>);

impl CanonicalJsonObject {
    /// Parse a canonical JSON object.
    pub fn parse(bytes: &[u8]) -> Result<Self, CanonicalJsonError> {
        let value = CanonicalJsonValue::parse(bytes)?;
        match value {
            CanonicalJsonValue::Object(values) => Ok(Self(values)),
            _ => Err(CanonicalJsonError::RootNotObject),
        }
    }

    /// Construct an empty object.
    pub const fn empty() -> Self {
        Self(BTreeMap::new())
    }

    /// Return the number of top-level fields.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether this object has no fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over top-level field names in canonical order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// Look up one field.
    pub fn get(&self, key: &str) -> Option<&CanonicalJsonValue> {
        self.0.get(key)
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &CanonicalJsonValue> {
        self.0.values()
    }

    pub(crate) fn from_inner(values: BTreeMap<String, CanonicalJsonValue>) -> Self {
        Self(values)
    }

    pub(crate) fn into_inner(self) -> BTreeMap<String, CanonicalJsonValue> {
        self.0
    }

    /// Render the exact canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("canonical JSON objects always serialize")
    }
}

impl core::fmt::Debug for CanonicalJsonObject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CanonicalJsonObject(<{} fields>)", self.len())
    }
}

impl Serialize for CanonicalJsonObject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CanonicalJsonObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = CanonicalJsonValue::deserialize(deserializer)?;
        match value {
            CanonicalJsonValue::Object(values) => Ok(Self(values)),
            _ => Err(serde::de::Error::custom(CanonicalJsonError::RootNotObject)),
        }
    }
}

impl JsonSchema for CanonicalJsonObject {
    fn schema_name() -> String {
        "CanonicalJsonObject".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
            ..Default::default()
        })
    }
}

fn validate_canonical_key(key: &str) -> Result<(), CanonicalJsonError> {
    if key.is_empty()
        || key.len() > MAX_CANONICAL_KEY_BYTES
        || !key.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(CanonicalJsonError::InvalidKey);
    }
    Ok(())
}

fn validate_number_tokens(bytes: &[u8]) -> Result<(), CanonicalJsonError> {
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() {
            let start = index;
            if byte == b'-' {
                index += 1;
            }
            let digits_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            let digits = &bytes[digits_start..index];
            if digits.is_empty()
                || (digits.len() > 1 && digits[0] == b'0')
                || (bytes[start] == b'-' && digits == b"0")
                || index < bytes.len()
                    && !matches!(
                        bytes[index],
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}'
                    )
            {
                return Err(CanonicalJsonError::IntegerOutOfRange);
            }
            continue;
        }
        index += 1;
    }
    Ok(())
}

pub(crate) fn validate_canonical_string(value: &str) -> Result<(), CanonicalJsonError> {
    if value.nfc().ne(value.chars())
        || value.chars().any(|character| {
            matches!(character, '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}')
                || matches!(character, '\u{2028}' | '\u{2029}')
        })
    {
        return Err(CanonicalJsonError::InvalidString);
    }
    Ok(())
}

/// Canonicalize a serializable contract value.
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalJsonError> {
    let json = serde_json::to_vec(value).map_err(canonical_json_error)?;
    Ok(CanonicalJsonValue::parse(&json)?.to_canonical_bytes())
}

/// Compute a domain-separated SHA-256 digest over canonical bytes.
pub fn canonical_digest(domain_tag: &str, canonical_bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain_tag.as_bytes());
    digest.update([0]);
    digest.update(canonical_bytes);
    let bytes = digest.finalize();
    let mut rendered = String::with_capacity(71);
    rendered.push_str("sha256:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        rendered.push(char::from(HEX[usize::from(byte >> 4)]));
        rendered.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    rendered
}

/// A validated `MAJOR.MINOR` schema version.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaVersion {
    major: u32,
    minor: u32,
}

impl SchemaVersion {
    /// Construct a schema version. Major zero is not admitted.
    pub fn new(major: u32, minor: u32) -> Result<Self, ResourceSchemaError> {
        if major == 0 {
            return Err(ResourceSchemaError::InvalidSchemaVersion);
        }
        Ok(Self { major, minor })
    }

    /// Parse exactly `MAJOR.MINOR` without leading zeroes.
    pub fn parse(value: &str) -> Result<Self, ResourceSchemaError> {
        let (major, minor) = value
            .split_once('.')
            .ok_or(ResourceSchemaError::InvalidSchemaVersion)?;
        if major.is_empty()
            || minor.is_empty()
            || minor.contains('.')
            || (major.len() > 1 && major.starts_with('0'))
            || (minor.len() > 1 && minor.starts_with('0'))
            || !major.bytes().all(|byte| byte.is_ascii_digit())
            || !minor.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(ResourceSchemaError::InvalidSchemaVersion);
        }
        Self::new(
            major
                .parse()
                .map_err(|_| ResourceSchemaError::InvalidSchemaVersion)?,
            minor
                .parse()
                .map_err(|_| ResourceSchemaError::InvalidSchemaVersion)?,
        )
    }

    /// Render the canonical schema version.
    pub fn to_canonical_string(self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

impl core::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

impl core::fmt::Debug for SchemaVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SchemaVersion")
            .field(&self.to_canonical_string())
            .finish()
    }
}

impl Serialize for SchemaVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for SchemaVersion {
    fn schema_name() -> String {
        "SchemaVersion".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let mut schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            ..Default::default()
        };
        schema.string().pattern = Some("^[1-9][0-9]*\\.(0|[1-9][0-9]*)$".to_owned());
        Schema::Object(schema)
    }
}

/// Extension-schema layer encoded in a schema ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionSchemaLayer {
    Spec,
    Status,
}

impl ExtensionSchemaLayer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::Status => "status",
        }
    }
}

/// A qualified immutable Provider extension schema ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionSchemaId {
    provider_name: ResourceName,
    resource_type: ResourceTypeName,
    layer: ExtensionSchemaLayer,
}

impl ExtensionSchemaId {
    /// Construct an extension schema ID from validated components.
    pub const fn new(
        provider_name: ResourceName,
        resource_type: ResourceTypeName,
        layer: ExtensionSchemaLayer,
    ) -> Self {
        Self {
            provider_name,
            resource_type,
            layer,
        }
    }

    /// Parse `<provider>.d2bus.org/<ResourceType>/{spec|status}`.
    pub fn parse(value: &str) -> Result<Self, ResourceSchemaError> {
        let (authority, remainder) = value
            .split_once('/')
            .ok_or(ResourceSchemaError::InvalidSchemaId)?;
        let (resource_type, layer) = remainder
            .split_once('/')
            .ok_or(ResourceSchemaError::InvalidSchemaId)?;
        if layer.contains('/') {
            return Err(ResourceSchemaError::InvalidSchemaId);
        }
        let provider = authority
            .strip_suffix(".d2bus.org")
            .ok_or(ResourceSchemaError::InvalidSchemaId)?;
        let layer = match layer {
            "spec" => ExtensionSchemaLayer::Spec,
            "status" => ExtensionSchemaLayer::Status,
            _ => return Err(ResourceSchemaError::InvalidSchemaId),
        };
        Ok(Self::new(
            ResourceName::parse(provider).map_err(|_| ResourceSchemaError::InvalidSchemaId)?,
            ResourceTypeName::parse(resource_type)
                .map_err(|_| ResourceSchemaError::InvalidSchemaId)?,
            layer,
        ))
    }

    /// Borrow the Provider resource name encoded by this ID.
    pub const fn provider_name(&self) -> &ResourceName {
        &self.provider_name
    }

    /// Borrow the ResourceType encoded by this ID.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Return the extension layer encoded by this ID.
    pub const fn layer(&self) -> ExtensionSchemaLayer {
        self.layer
    }

    /// Render the canonical schema ID for an authorized encoding or key surface.
    pub fn to_canonical_string(&self) -> String {
        format!(
            "{}.d2bus.org/{}/{}",
            self.provider_name.as_str(),
            self.resource_type.as_str(),
            self.layer.as_str()
        )
    }
}

impl core::fmt::Display for ExtensionSchemaId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExtensionSchemaId(<redacted>)")
    }
}

impl core::fmt::Debug for ExtensionSchemaId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExtensionSchemaId(<redacted>)")
    }
}

impl Serialize for ExtensionSchemaId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for ExtensionSchemaId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ExtensionSchemaId {
    fn schema_name() -> String {
        "ExtensionSchemaId".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let mut schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            ..Default::default()
        };
        schema.string().max_length = Some(225);
        Schema::Object(schema)
    }
}

/// Version and fingerprint implemented for one ResourceType base layer.
#[derive(Clone, PartialEq, Eq)]
pub struct BaseSchemaIdentity {
    pub version: SchemaVersion,
    pub fingerprint: SchemaFingerprint,
}

/// Provider-advertised base schema identities.
#[derive(Clone, PartialEq, Eq)]
pub struct BaseSchemaBinding {
    pub spec: BaseSchemaIdentity,
    pub status: BaseSchemaIdentity,
}

/// Strict schema for one dynamic object.
#[derive(Clone, PartialEq, Eq)]
pub struct ObjectFieldSchema {
    allowed: BTreeSet<String>,
    required: BTreeSet<String>,
}

impl ObjectFieldSchema {
    /// Construct a closed object schema.
    pub fn new(
        allowed: impl IntoIterator<Item = String>,
        required: impl IntoIterator<Item = String>,
    ) -> Result<Self, ResourceSchemaError> {
        let allowed = allowed.into_iter().collect::<BTreeSet<_>>();
        let required = required.into_iter().collect::<BTreeSet<_>>();
        for key in allowed.iter().chain(required.iter()) {
            validate_canonical_key(key).map_err(|_| ResourceSchemaError::InvalidFieldName)?;
        }
        if !required.is_subset(&allowed) {
            return Err(ResourceSchemaError::RequiredFieldNotAllowed);
        }
        Ok(Self { allowed, required })
    }

    /// Construct a closed empty object schema.
    pub fn empty() -> Self {
        Self {
            allowed: BTreeSet::new(),
            required: BTreeSet::new(),
        }
    }

    fn validate(&self, value: &CanonicalJsonObject) -> Result<(), ResourceSchemaError> {
        self.validate_names(value.keys())
    }

    fn validate_names<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), ResourceSchemaError> {
        let names = names.into_iter().collect::<BTreeSet<_>>();
        for key in &names {
            if !self.allowed.contains(*key) {
                return Err(ResourceSchemaError::UnknownField((*key).to_owned()));
            }
        }
        for key in &self.required {
            if !names.contains(key.as_str()) {
                return Err(ResourceSchemaError::MissingField(key.clone()));
            }
        }
        Ok(())
    }

    fn shadows(&self, key: &str) -> bool {
        self.allowed.contains(key)
    }
}

/// Registered Provider extension schemas for one ResourceType.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderExtensionRegistration {
    pub provider_ref: ResourceRef,
    pub spec_schema_id: ExtensionSchemaId,
    pub spec_schema_version: SchemaVersion,
    pub spec_settings: ObjectFieldSchema,
    pub status_schema_id: ExtensionSchemaId,
    pub status_schema_version: SchemaVersion,
    pub status_details: ObjectFieldSchema,
}

/// The single base and Provider-extension contract for one ResourceType.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceSchemaContract {
    resource_type: ResourceTypeName,
    base_binding: BaseSchemaBinding,
    base_spec: ObjectFieldSchema,
    base_status: ObjectFieldSchema,
    provider_extensions: BTreeMap<ResourceRef, ProviderExtensionRegistration>,
}

impl core::fmt::Debug for BaseSchemaIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BaseSchemaIdentity(<redacted>)")
    }
}

impl core::fmt::Debug for BaseSchemaBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BaseSchemaBinding(<redacted>)")
    }
}

impl core::fmt::Debug for ObjectFieldSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ObjectFieldSchema")
            .field("allowed", &self.allowed.len())
            .field("required", &self.required.len())
            .finish()
    }
}

impl core::fmt::Debug for ProviderExtensionRegistration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProviderExtensionRegistration(<redacted>)")
    }
}

impl core::fmt::Debug for ResourceSchemaContract {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResourceSchemaContract(<redacted>)")
    }
}

impl ResourceSchemaContract {
    /// Construct a ResourceType schema contract.
    pub fn new(
        resource_type: ResourceTypeName,
        base_binding: BaseSchemaBinding,
        base_spec: ObjectFieldSchema,
        base_status: ObjectFieldSchema,
        provider_extensions: impl IntoIterator<Item = ProviderExtensionRegistration>,
    ) -> Result<Self, ResourceSchemaError> {
        let mut registrations = BTreeMap::new();
        for registration in provider_extensions {
            validate_provider_registration(
                &resource_type,
                &base_spec,
                &base_status,
                &registration,
            )?;
            if registrations
                .insert(registration.provider_ref.clone(), registration)
                .is_some()
            {
                return Err(ResourceSchemaError::DuplicateProviderRegistration);
            }
        }
        Ok(Self {
            resource_type,
            base_binding,
            base_spec,
            base_status,
            provider_extensions: registrations,
        })
    }

    /// Borrow the ResourceType this schema governs.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Verify a Provider's advertised base schema versions and fingerprints.
    pub fn verify_base_binding(
        &self,
        binding: &BaseSchemaBinding,
    ) -> Result<(), ResourceSchemaError> {
        if binding != &self.base_binding {
            return Err(ResourceSchemaError::BaseSchemaMismatch);
        }
        Ok(())
    }

    /// Validate a complete resource against all three spec and status layers.
    pub fn validate_envelope(
        &self,
        envelope: &ResourceEnvelope,
    ) -> Result<(), ResourceSchemaError> {
        if envelope.resource_type() != &self.resource_type {
            return Err(ResourceSchemaError::ResourceTypeMismatch);
        }
        let mut spec_names = envelope.spec().base().keys().collect::<Vec<_>>();
        if envelope.spec().provider_ref().is_some() {
            spec_names.push("providerRef");
        }
        if envelope.spec().update_policy().is_some() {
            spec_names.push("updatePolicy");
        }
        self.base_spec.validate_names(spec_names)?;
        self.base_status.validate(envelope.status().resource())?;

        let provider_ref = envelope.spec().provider_ref();
        if let Some(provider) = envelope.spec().provider() {
            let provider_ref = provider_ref.ok_or(ResourceSchemaError::ProviderRefRequired)?;
            let registration = self
                .provider_extensions
                .get(provider_ref)
                .ok_or(ResourceSchemaError::ProviderNotRegistered)?;
            validate_spec_provider(
                &self.resource_type,
                &self.base_spec,
                provider_ref,
                provider,
                registration,
            )?;
        }

        if let Some(provider) = envelope.status().provider() {
            let provider_ref = provider_ref.ok_or(ResourceSchemaError::ProviderRefRequired)?;
            let registration = self
                .provider_extensions
                .get(provider_ref)
                .ok_or(ResourceSchemaError::ProviderNotRegistered)?;
            validate_status_provider(&self.resource_type, provider_ref, provider, registration)?;
        }
        Ok(())
    }

    /// Validate the canonical minimal base spec without a Provider extension.
    pub fn validate_minimal_base_spec(
        &self,
        spec: &super::ResourceSpec,
    ) -> Result<(), ResourceSchemaError> {
        if spec.provider().is_some() {
            return Err(ResourceSchemaError::ProviderExtensionNotMinimal);
        }
        let mut spec_names = spec.base().keys().collect::<Vec<_>>();
        if spec.provider_ref().is_some() {
            spec_names.push("providerRef");
        }
        if spec.update_policy().is_some() {
            spec_names.push("updatePolicy");
        }
        self.base_spec.validate_names(spec_names)
    }
}

fn validate_provider_registration(
    resource_type: &ResourceTypeName,
    base_spec: &ObjectFieldSchema,
    base_status: &ObjectFieldSchema,
    registration: &ProviderExtensionRegistration,
) -> Result<(), ResourceSchemaError> {
    ensure_provider_ref(&registration.provider_ref)?;
    validate_extension_id(
        resource_type,
        &registration.provider_ref,
        &registration.spec_schema_id,
        ExtensionSchemaLayer::Spec,
    )?;
    validate_extension_id(
        resource_type,
        &registration.provider_ref,
        &registration.status_schema_id,
        ExtensionSchemaLayer::Status,
    )?;
    for key in &registration.spec_settings.allowed {
        if base_spec.shadows(key)
            || matches!(key.as_str(), "provider" | "providerRef" | "updatePolicy")
        {
            return Err(ResourceSchemaError::ProviderFieldShadowsBase(key.clone()));
        }
    }
    for key in &registration.status_details.allowed {
        if base_status.shadows(key)
            || matches!(
                key.as_str(),
                "observedGeneration"
                    | "phase"
                    | "conditions"
                    | "lastReconciledAt"
                    | "startedAt"
                    | "completedAt"
                    | "outcome"
                    | "update"
                    | "resource"
                    | "provider"
            )
        {
            return Err(ResourceSchemaError::ProviderFieldShadowsBase(key.clone()));
        }
    }
    Ok(())
}

fn validate_spec_provider(
    resource_type: &ResourceTypeName,
    base_spec: &ObjectFieldSchema,
    provider_ref: &ResourceRef,
    provider: &super::ProviderSpecExtension,
    registration: &ProviderExtensionRegistration,
) -> Result<(), ResourceSchemaError> {
    validate_extension_id(
        resource_type,
        provider_ref,
        provider.schema_id(),
        ExtensionSchemaLayer::Spec,
    )?;
    if provider.schema_id() != &registration.spec_schema_id
        || provider.schema_version() != registration.spec_schema_version
    {
        return Err(ResourceSchemaError::ProviderSchemaMismatch);
    }
    for key in provider.settings().keys() {
        if base_spec.shadows(key) || matches!(key, "provider" | "providerRef" | "updatePolicy") {
            return Err(ResourceSchemaError::ProviderFieldShadowsBase(
                key.to_owned(),
            ));
        }
    }
    registration.spec_settings.validate(provider.settings())
}

fn validate_status_provider(
    resource_type: &ResourceTypeName,
    provider_ref: &ResourceRef,
    provider: &ProviderStatusExtension,
    registration: &ProviderExtensionRegistration,
) -> Result<(), ResourceSchemaError> {
    if provider.provider_ref() != provider_ref {
        return Err(ResourceSchemaError::ProviderRefMismatch);
    }
    validate_extension_id(
        resource_type,
        provider_ref,
        provider.schema_id(),
        ExtensionSchemaLayer::Status,
    )?;
    if provider.schema_id() != &registration.status_schema_id
        || provider.schema_version() != registration.status_schema_version
    {
        return Err(ResourceSchemaError::ProviderSchemaMismatch);
    }
    registration.status_details.validate(provider.details())
}

fn ensure_provider_ref(provider_ref: &ResourceRef) -> Result<(), ResourceSchemaError> {
    if provider_ref.resource_type().as_str() != "Provider" {
        return Err(ResourceSchemaError::ProviderRefWrongType);
    }
    Ok(())
}

fn validate_extension_id(
    resource_type: &ResourceTypeName,
    provider_ref: &ResourceRef,
    schema_id: &ExtensionSchemaId,
    layer: ExtensionSchemaLayer,
) -> Result<(), ResourceSchemaError> {
    ensure_provider_ref(provider_ref)?;
    if schema_id.provider_name() != provider_ref.name()
        || schema_id.resource_type() != resource_type
        || schema_id.layer() != layer
    {
        return Err(ResourceSchemaError::ProviderSchemaBinding);
    }
    Ok(())
}

/// Schema or Provider-layer conformance failure.
#[derive(Clone, PartialEq, Eq)]
pub enum ResourceSchemaError {
    InvalidSchemaVersion,
    InvalidSchemaId,
    InvalidFieldName,
    RequiredFieldNotAllowed,
    UnknownField(String),
    MissingField(String),
    DuplicateProviderRegistration,
    ResourceTypeMismatch,
    BaseSchemaMismatch,
    ProviderRefRequired,
    ProviderRefWrongType,
    ProviderRefMismatch,
    ProviderNotRegistered,
    ProviderSchemaBinding,
    ProviderSchemaMismatch,
    ProviderFieldShadowsBase(String),
    ProviderExtensionNotMinimal,
}

impl core::fmt::Debug for ResourceSchemaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = match self {
            Self::InvalidSchemaVersion => "InvalidSchemaVersion",
            Self::InvalidSchemaId => "InvalidSchemaId",
            Self::InvalidFieldName => "InvalidFieldName",
            Self::RequiredFieldNotAllowed => "RequiredFieldNotAllowed",
            Self::UnknownField(_) => "UnknownField",
            Self::MissingField(_) => "MissingField",
            Self::DuplicateProviderRegistration => "DuplicateProviderRegistration",
            Self::ResourceTypeMismatch => "ResourceTypeMismatch",
            Self::BaseSchemaMismatch => "BaseSchemaMismatch",
            Self::ProviderRefRequired => "ProviderRefRequired",
            Self::ProviderRefWrongType => "ProviderRefWrongType",
            Self::ProviderRefMismatch => "ProviderRefMismatch",
            Self::ProviderNotRegistered => "ProviderNotRegistered",
            Self::ProviderSchemaBinding => "ProviderSchemaBinding",
            Self::ProviderSchemaMismatch => "ProviderSchemaMismatch",
            Self::ProviderFieldShadowsBase(_) => "ProviderFieldShadowsBase",
            Self::ProviderExtensionNotMinimal => "ProviderExtensionNotMinimal",
        };
        write!(f, "ResourceSchemaError::{kind}")
    }
}

impl core::fmt::Display for ResourceSchemaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSchemaVersion => f.write_str("schema version must be MAJOR.MINOR"),
            Self::InvalidSchemaId => f.write_str("invalid Provider extension schema ID"),
            Self::InvalidFieldName => f.write_str("invalid schema field name"),
            Self::RequiredFieldNotAllowed => {
                f.write_str("required schema field is not in the allowed set")
            }
            Self::UnknownField(_) => f.write_str("schema contains an unknown field"),
            Self::MissingField(_) => f.write_str("schema is missing a required field"),
            Self::DuplicateProviderRegistration => {
                f.write_str("Provider extension is registered more than once")
            }
            Self::ResourceTypeMismatch => f.write_str("resource does not match schema type"),
            Self::BaseSchemaMismatch => {
                f.write_str("Provider base schema version or fingerprint does not match")
            }
            Self::ProviderRefRequired => {
                f.write_str("Provider extension requires spec.providerRef")
            }
            Self::ProviderRefWrongType => {
                f.write_str("providerRef must reference a Provider resource")
            }
            Self::ProviderRefMismatch => {
                f.write_str("status Provider does not match spec.providerRef")
            }
            Self::ProviderNotRegistered => {
                f.write_str("Provider extension schema is not registered")
            }
            Self::ProviderSchemaBinding => {
                f.write_str("Provider schema ID is not bound to the selected Provider and type")
            }
            Self::ProviderSchemaMismatch => {
                f.write_str("Provider extension schema ID or version does not match")
            }
            Self::ProviderFieldShadowsBase(_) => {
                f.write_str("Provider extension shadows a base field")
            }
            Self::ProviderExtensionNotMinimal => {
                f.write_str("minimal base spec must not require a Provider extension")
            }
        }
    }
}

impl std::error::Error for ResourceSchemaError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{
        ManagedBy, ObservedGeneration, PresentationMetadata, ProviderSpecExtension,
        ProviderStatusExtension, ResourceCurrencySet, ResourceGeneration, ResourceMetadata,
        ResourcePhase, ResourceSpec, ResourceStatus, ResourceUid, ResourceUpdateStatus, Timestamp,
        UpdateDisruption, UpdateState, ZoneId, ZoneRevision,
    };

    fn fingerprint(suffix: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", suffix.to_string().repeat(64))).unwrap()
    }

    fn base_binding() -> BaseSchemaBinding {
        BaseSchemaBinding {
            spec: BaseSchemaIdentity {
                version: SchemaVersion::parse("1.0").unwrap(),
                fingerprint: fingerprint('a'),
            },
            status: BaseSchemaIdentity {
                version: SchemaVersion::parse("1.0").unwrap(),
                fingerprint: fingerprint('b'),
            },
        }
    }

    fn registration() -> ProviderExtensionRegistration {
        ProviderExtensionRegistration {
            provider_ref: ResourceRef::parse("Provider/runtime-qemu-media").unwrap(),
            spec_schema_id: ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/spec")
                .unwrap(),
            spec_schema_version: SchemaVersion::parse("1.0").unwrap(),
            spec_settings: ObjectFieldSchema::new(["machine".to_owned()], ["machine".to_owned()])
                .unwrap(),
            status_schema_id: ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/status")
                .unwrap(),
            status_schema_version: SchemaVersion::parse("1.0").unwrap(),
            status_details: ObjectFieldSchema::new(["backend".to_owned()], ["backend".to_owned()])
                .unwrap(),
        }
    }

    fn contract() -> ResourceSchemaContract {
        ResourceSchemaContract::new(
            ResourceTypeName::parse("Guest").unwrap(),
            base_binding(),
            ObjectFieldSchema::new(
                [
                    "providerRef".to_owned(),
                    "updatePolicy".to_owned(),
                    "imageId".to_owned(),
                ],
                ["providerRef".to_owned()],
            )
            .unwrap(),
            ObjectFieldSchema::empty(),
            [registration()],
        )
        .unwrap()
    }

    fn update() -> ResourceUpdateStatus {
        let empty = || ResourceCurrencySet::new(0, Vec::new()).unwrap();
        ResourceUpdateStatus::new(
            UpdateState::Current,
            Vec::new(),
            ObservedGeneration::new(1),
            ResourceGeneration::new(1).unwrap(),
            UpdateDisruption::None,
            true,
            None,
            None,
            empty(),
            empty(),
        )
        .unwrap()
    }

    fn envelope(
        settings: CanonicalJsonObject,
        spec_version: &str,
        details: CanonicalJsonObject,
        status_version: &str,
    ) -> ResourceEnvelope {
        let spec = ResourceSpec::new(
            Some(ResourceRef::parse("Provider/runtime-qemu-media").unwrap()),
            None,
            CanonicalJsonObject::parse(br#"{"imageId":"guest-system"}"#).unwrap(),
            Some(
                ProviderSpecExtension::new(
                    ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/spec").unwrap(),
                    SchemaVersion::parse(spec_version).unwrap(),
                    settings,
                )
                .unwrap(),
            ),
        )
        .unwrap();
        let status_provider = ProviderStatusExtension::new(
            ResourceRef::parse("Provider/runtime-qemu-media").unwrap(),
            ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/status").unwrap(),
            SchemaVersion::parse(status_version).unwrap(),
            ResourceGeneration::new(1).unwrap(),
            details,
        )
        .unwrap();
        let status = ResourceStatus::new(
            ObservedGeneration::new(1),
            ResourcePhase::Ready,
            Vec::new(),
            None,
            None,
            None,
            None,
            update(),
            CanonicalJsonObject::empty(),
            Some(status_provider),
        )
        .unwrap();
        let timestamp = Timestamp::parse("2026-07-22T00:00:00.000Z").unwrap();
        let metadata = ResourceMetadata::new(
            ResourceName::parse("work").unwrap(),
            ZoneId::parse("dev").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ZoneRevision::new(1),
            None,
            Vec::new(),
            None,
            timestamp.clone(),
            timestamp,
            ManagedBy::Api,
            None,
            None,
            None,
            PresentationMetadata::default(),
        )
        .unwrap();
        ResourceEnvelope::new(
            ResourceTypeName::parse("Guest").unwrap(),
            metadata,
            spec,
            status,
        )
        .unwrap()
    }

    #[test]
    fn canonical_json_pins_literal_bytes_and_digest() {
        const INPUT: &str = r#"{"z":1,"a":{"two":"é","one":1},"array":[true,null,-2]}"#;
        const CANONICAL: &str = r#"{"a":{"one":1,"two":"é"},"array":[true,null,-2],"z":1}"#;
        const DIGEST: &str =
            "sha256:e37fcecf3d1ba461173cc1b06ae6b6e97ce28ad37893a587719a578966760ff0";

        let parsed = CanonicalJsonValue::parse(INPUT.as_bytes()).unwrap();
        assert_eq!(parsed.to_canonical_bytes(), CANONICAL.as_bytes());
        assert_eq!(
            canonical_digest(RESOURCE_SPEC_DOMAIN_TAG, CANONICAL.as_bytes()),
            DIGEST
        );
    }

    #[test]
    fn canonical_json_rejects_duplicates_floats_controls_and_non_nfc() {
        for input in [
            br#"{"a":1,"a":2}"#.as_slice(),
            br#"{"a":1.0}"#,
            br#"{"a":1e0}"#,
            br#"{"a":-0}"#,
            br#"{"a":9223372036854775808}"#,
            "{\"a\":\"e\u{301}\"}".as_bytes(),
            "{\"a\":\"\\u2028\"}".as_bytes(),
            br#"{"bad
key":1}"#,
        ] {
            assert!(CanonicalJsonValue::parse(input).is_err());
        }
    }

    #[test]
    fn canonical_json_errors_keep_only_closed_reasons_and_safe_positions() {
        let payload_marker = format!("payload-marker-{:x}", std::process::id());
        let duplicate = format!(r#"{{"{payload_marker}":1,"{payload_marker}":2}}"#);
        let duplicate_error = CanonicalJsonValue::parse(duplicate.as_bytes()).unwrap_err();
        assert!(matches!(
            &duplicate_error,
            CanonicalJsonError::DuplicateKey {
                key_ordinal: 2,
                line: 1,
                column
            } if *column > 0
        ));
        for rendered in [format!("{duplicate_error:?}"), format!("{duplicate_error}")] {
            assert!(!rendered.contains(&payload_marker));
        }
        assert!(std::error::Error::source(&duplicate_error).is_none());

        let malformed = format!(r#"{{"safe":"{payload_marker}","broken":}}"#);
        let syntax_error = CanonicalJsonValue::parse(malformed.as_bytes()).unwrap_err();
        assert!(matches!(
            &syntax_error,
            CanonicalJsonError::Syntax {
                reason: CanonicalJsonCodecReason::Syntax
                    | CanonicalJsonCodecReason::UnexpectedEof,
                line: 1,
                column
            } if *column > 0
        ));
        for rendered in [format!("{syntax_error:?}"), format!("{syntax_error}")] {
            assert!(!rendered.contains(&payload_marker));
        }
        assert!(std::error::Error::source(&syntax_error).is_none());
    }

    #[test]
    fn schema_version_and_extension_id_have_one_spelling() {
        assert_eq!(SchemaVersion::parse("1.0").unwrap().to_string(), "1.0");
        for invalid in ["", "0.1", "01.0", "1", "1.00", "1.0.0"] {
            assert!(SchemaVersion::parse(invalid).is_err(), "{invalid}");
        }

        let id = ExtensionSchemaId::parse("runtime-qemu-media.d2bus.org/Guest/spec").unwrap();
        assert_eq!(id.provider_name().as_str(), "runtime-qemu-media");
        assert_eq!(id.resource_type().as_str(), "Guest");
        assert_eq!(id.layer(), ExtensionSchemaLayer::Spec);
        assert_eq!(
            id.to_canonical_string(),
            "runtime-qemu-media.d2bus.org/Guest/spec"
        );
    }

    #[test]
    fn minimal_base_and_base_binding_conformance_are_exact() {
        let contract = contract();
        let minimal = ResourceSpec::new(
            Some(ResourceRef::parse("Provider/runtime-qemu-media").unwrap()),
            None,
            CanonicalJsonObject::empty(),
            None,
        )
        .unwrap();
        contract.validate_minimal_base_spec(&minimal).unwrap();
        contract.verify_base_binding(&base_binding()).unwrap();

        let missing_provider = ResourceSpec::empty();
        assert_eq!(
            contract.validate_minimal_base_spec(&missing_provider),
            Err(ResourceSchemaError::MissingField("providerRef".to_owned()))
        );
        let mut wrong_binding = base_binding();
        wrong_binding.spec.fingerprint = fingerprint('c');
        assert_eq!(
            contract.verify_base_binding(&wrong_binding),
            Err(ResourceSchemaError::BaseSchemaMismatch)
        );
    }

    #[test]
    fn provider_layers_reject_unknown_version_and_shadow_fields() {
        let contract = contract();
        let valid = envelope(
            CanonicalJsonObject::parse(br#"{"machine":"microvm"}"#).unwrap(),
            "1.0",
            CanonicalJsonObject::parse(br#"{"backend":"qemu"}"#).unwrap(),
            "1.0",
        );
        contract.validate_envelope(&valid).unwrap();

        let unknown = envelope(
            CanonicalJsonObject::parse(br#"{"machine":"microvm","unregistered":true}"#).unwrap(),
            "1.0",
            CanonicalJsonObject::parse(br#"{"backend":"qemu"}"#).unwrap(),
            "1.0",
        );
        assert_eq!(
            contract.validate_envelope(&unknown),
            Err(ResourceSchemaError::UnknownField("unregistered".to_owned()))
        );

        let wrong_spec_version = envelope(
            CanonicalJsonObject::parse(br#"{"machine":"microvm"}"#).unwrap(),
            "2.0",
            CanonicalJsonObject::parse(br#"{"backend":"qemu"}"#).unwrap(),
            "1.0",
        );
        assert_eq!(
            contract.validate_envelope(&wrong_spec_version),
            Err(ResourceSchemaError::ProviderSchemaMismatch)
        );
        let wrong_status_version = envelope(
            CanonicalJsonObject::parse(br#"{"machine":"microvm"}"#).unwrap(),
            "1.0",
            CanonicalJsonObject::parse(br#"{"backend":"qemu"}"#).unwrap(),
            "2.0",
        );
        assert_eq!(
            contract.validate_envelope(&wrong_status_version),
            Err(ResourceSchemaError::ProviderSchemaMismatch)
        );

        let mut shadowing = registration();
        shadowing.spec_settings =
            ObjectFieldSchema::new(["imageId".to_owned()], Vec::new()).unwrap();
        assert!(matches!(
            ResourceSchemaContract::new(
                ResourceTypeName::parse("Guest").unwrap(),
                base_binding(),
                ObjectFieldSchema::new(
                    ["providerRef".to_owned(), "imageId".to_owned()],
                    ["providerRef".to_owned()],
                )
                .unwrap(),
                ObjectFieldSchema::empty(),
                [shadowing],
            ),
            Err(ResourceSchemaError::ProviderFieldShadowsBase(field)) if field == "imageId"
        ));
    }

    #[test]
    fn schema_diagnostics_redact_identity_field_and_payload_markers() {
        let nonce = u64::from(std::process::id());
        let name_marker = format!("name-{nonce:x}");
        let payload_marker = format!("payload-marker-{nonce:x}");
        let resource_type_marker = format!("provider-{nonce:x}.d2bus.org.Marker");
        let markers = [
            name_marker.as_str(),
            payload_marker.as_str(),
            resource_type_marker.as_str(),
        ];
        let resource_type = ResourceTypeName::parse(&resource_type_marker).unwrap();
        let provider_ref = ResourceRef::parse(&format!("Provider/{name_marker}")).unwrap();
        let spec_schema_id = ExtensionSchemaId::new(
            ResourceName::parse(&name_marker).unwrap(),
            resource_type.clone(),
            ExtensionSchemaLayer::Spec,
        );
        let status_schema_id = ExtensionSchemaId::new(
            ResourceName::parse(&name_marker).unwrap(),
            resource_type.clone(),
            ExtensionSchemaLayer::Status,
        );
        let object =
            ObjectFieldSchema::new([payload_marker.clone()], [payload_marker.clone()]).unwrap();
        let digest = SchemaFingerprint::parse(format!("sha256:{nonce:064x}")).unwrap();
        let identity = BaseSchemaIdentity {
            version: SchemaVersion::parse("1.0").unwrap(),
            fingerprint: digest,
        };
        let binding = BaseSchemaBinding {
            spec: identity.clone(),
            status: identity.clone(),
        };
        let registration = ProviderExtensionRegistration {
            provider_ref,
            spec_schema_id: spec_schema_id.clone(),
            spec_schema_version: SchemaVersion::parse("1.0").unwrap(),
            spec_settings: ObjectFieldSchema::empty(),
            status_schema_id,
            status_schema_version: SchemaVersion::parse("1.0").unwrap(),
            status_details: ObjectFieldSchema::empty(),
        };
        let contract = ResourceSchemaContract::new(
            resource_type,
            binding.clone(),
            ObjectFieldSchema::empty(),
            ObjectFieldSchema::empty(),
            [registration.clone()],
        )
        .unwrap();
        let dynamic = CanonicalJsonValue::parse(
            format!(r#"{{"{payload_marker}":"{payload_marker}"}}"#).as_bytes(),
        )
        .unwrap();
        let dynamic_object = CanonicalJsonObject::parse(
            format!(r#"{{"{payload_marker}":"{payload_marker}"}}"#).as_bytes(),
        )
        .unwrap();
        let schema_error = ResourceSchemaError::UnknownField(payload_marker.clone());

        let formatted = [
            format!("{spec_schema_id:?}"),
            format!("{spec_schema_id}"),
            spec_schema_id.to_string(),
            format!("{identity:?}"),
            format!("{binding:?}"),
            format!("{object:?}"),
            format!("{registration:?}"),
            format!("{contract:?}"),
            format!("{dynamic:?}"),
            format!("{dynamic_object:?}"),
            format!("{schema_error:?}"),
            format!("{schema_error}"),
        ];
        for rendered in formatted {
            for marker in &markers {
                assert!(
                    !rendered.contains(marker),
                    "schema marker appeared in diagnostic formatting"
                );
            }
        }

        assert!(spec_schema_id.to_canonical_string().contains(&name_marker));
        assert!(
            String::from_utf8(dynamic_object.to_canonical_bytes())
                .unwrap()
                .contains(&payload_marker)
        );
    }
}
