//! Shared execution sub-schemas for the primitive ResourceType base specs.
//!
//! This module owns the provider-neutral pieces that the Host and Guest base
//! specs share (`ExecutionPolicy`), the budget schema that Host, Guest,
//! Process, and EphemeralProcess all embed, and the small validated scalars
//! (`BoundedToken`, `BoundedText`, `DurationMs`, `MilliCpu`, `ByteQuantity`)
//! that the other primitive modules reuse.
//!
//! Every type here is a Layer 2 base-spec fragment: it never carries
//! `providerRef`, `updatePolicy`, or the Layer 3 `provider` extension
//! envelope, all of which live on the universal `ResourceSpec`.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    resource_schema::{CanonicalJsonError, CanonicalJsonObject, canonical_json_bytes},
};

macro_rules! redacted_debug {
    ($type:ty) => {
        impl core::fmt::Debug for $type {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($type), "(<redacted>)"))
            }
        }
    };
}

macro_rules! parsed_deserialize {
    ($type:ty) => {
        impl<'de> serde::Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! string_schema {
    ($type:ty, $min:expr, $max:expr) => {
        impl schemars::JsonSchema for $type {
            fn schema_name() -> String {
                stringify!($type).to_owned()
            }

            fn json_schema(
                _gen: &mut schemars::r#gen::SchemaGenerator,
            ) -> schemars::schema::Schema {
                crate::v3::execution_policy::string_schema_object($min as u32, $max as u32)
            }
        }
    };
}

pub(crate) use {parsed_deserialize, redacted_debug, string_schema};

/// Maximum bytes in one bounded lower-kebab token.
pub const MAX_BOUNDED_TOKEN_BYTES: usize = 63;
/// Maximum requested or limited millicpus.
pub const MAX_MILLICPU: u64 = 1_024_000;
/// Maximum requested or limited memory bytes.
pub const MAX_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024 * 1024;
/// Maximum cgroup relative I/O weight.
pub const MAX_IO_WEIGHT: u32 = 10_000;
/// Maximum egress rate in bytes per second.
pub const MAX_NETWORK_EGRESS_BPS: u64 = 1_000_000_000_000;
/// Maximum process ID limit.
pub const MAX_PIDS_LIMIT: u32 = 65_535;
/// Maximum file descriptor limit.
pub const MAX_FDS_LIMIT: u32 = 1_048_576;
/// Maximum network attachments on one execution target.
pub const MAX_NETWORK_ATTACHMENTS: usize = 64;
/// Maximum device attachments on one execution target.
pub const MAX_DEVICE_ATTACHMENTS: usize = 64;
/// Maximum Volume attachment defaults on one execution target.
pub const MAX_VOLUME_ATTACHMENT_DEFAULTS: usize = 64;
/// Maximum bytes in one bounded free-text label.
pub const MAX_BOUNDED_TEXT_BYTES: usize = 128;
/// Maximum bytes in one absolute or relative path field.
pub const MAX_PATH_BYTES: usize = 255;

/// Invalid primitive base-spec field.
///
/// Every variant is deliberately field-free so a rejection diagnostic can
/// never echo caller-supplied text, a path, or a resource identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimitiveSpecError {
    /// A bounded lower-kebab token was empty, over bound, or malformed.
    InvalidToken,
    /// A bounded free-text field was over bound or carried a control character.
    InvalidText,
    /// A path field was empty, over bound, or not anchored as required.
    InvalidPath,
    /// A POSIX mode string was not three or four octal digits.
    InvalidMode,
    /// A duration string was malformed or outside its bound.
    InvalidDuration,
    /// A CPU or memory quantity was malformed.
    InvalidQuantity,
    /// An integer field was outside its frozen bound.
    OutOfRange,
    /// A bounded collection exceeded its frozen entry ceiling.
    TooManyEntries,
    /// A collection that must be unique carried a duplicate entry.
    DuplicateEntry,
    /// A conditionally required field was absent.
    MissingRequiredField,
    /// Two fields were set in a combination the base schema rejects.
    ConflictingFields,
    /// A ResourceRef named the wrong ResourceType for its field.
    WrongResourceType,
    /// The base object could not be rendered as canonical JSON.
    CanonicalJson(CanonicalJsonError),
}

impl core::fmt::Display for PrimitiveSpecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidToken => f.write_str("invalid bounded token"),
            Self::InvalidText => f.write_str("invalid bounded text field"),
            Self::InvalidPath => f.write_str("invalid path field"),
            Self::InvalidMode => f.write_str("invalid octal mode"),
            Self::InvalidDuration => f.write_str("invalid duration"),
            Self::InvalidQuantity => f.write_str("invalid cpu or memory quantity"),
            Self::OutOfRange => f.write_str("value is outside its frozen bound"),
            Self::TooManyEntries => f.write_str("collection exceeds its frozen bound"),
            Self::DuplicateEntry => f.write_str("collection entries must be unique"),
            Self::MissingRequiredField => f.write_str("conditionally required field is absent"),
            Self::ConflictingFields => f.write_str("fields are set in a rejected combination"),
            Self::WrongResourceType => f.write_str("reference names the wrong ResourceType"),
            Self::CanonicalJson(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PrimitiveSpecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CanonicalJson(error) => Some(error),
            _ => None,
        }
    }
}

/// Require that a reference names exactly one ResourceType.
pub fn require_resource_type(
    reference: &ResourceRef,
    expected: &str,
) -> Result<(), PrimitiveSpecError> {
    if reference.resource_type().as_str() == expected {
        Ok(())
    } else {
        Err(PrimitiveSpecError::WrongResourceType)
    }
}

/// Require that a reference names a Host or a Guest.
pub fn require_execution_ref(reference: &ResourceRef) -> Result<(), PrimitiveSpecError> {
    if matches!(reference.resource_type().as_str(), "Host" | "Guest") {
        Ok(())
    } else {
        Err(PrimitiveSpecError::WrongResourceType)
    }
}

/// Render one base-spec value as a `ResourceSpec` base object.
///
/// Primitive base specs are Layer 2 data, so the rendered object never
/// contains `providerRef`, `updatePolicy`, or `provider`.
pub fn to_base_object<T: Serialize>(value: &T) -> Result<CanonicalJsonObject, PrimitiveSpecError> {
    let bytes = canonical_json_bytes(value).map_err(PrimitiveSpecError::CanonicalJson)?;
    CanonicalJsonObject::parse(&bytes).map_err(PrimitiveSpecError::CanonicalJson)
}

/// A validated lower-kebab token used for template, view, purpose, and
/// artifact identifiers.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BoundedToken(String);

impl BoundedToken {
    /// Parse a `^[a-z][a-z0-9-]*$` token bounded to 63 bytes.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_BOUNDED_TOKEN_BYTES {
            return Err(PrimitiveSpecError::InvalidToken);
        }
        let mut bytes = value.bytes();
        let head_ok = matches!(bytes.next(), Some(b'a'..=b'z'));
        let tail_ok =
            bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if head_ok && tail_ok {
            Ok(Self(value))
        } else {
            Err(PrimitiveSpecError::InvalidToken)
        }
    }

    /// Borrow the canonical token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(BoundedToken);
parsed_deserialize!(BoundedToken);
string_schema!(BoundedToken, 1, MAX_BOUNDED_TOKEN_BYTES);

/// A bounded, control-character-free free-text label.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BoundedText(String);

impl BoundedText {
    /// Parse text bounded to 128 bytes with no control characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.len() > MAX_BOUNDED_TEXT_BYTES
            || value
                .chars()
                .any(|character| character.is_control() || matches!(character, '\u{007f}'))
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        Ok(Self(value))
    }

    /// Borrow the text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(BoundedText);
parsed_deserialize!(BoundedText);
string_schema!(BoundedText, 0, MAX_BOUNDED_TEXT_BYTES);

/// A validated duration string such as `"30s"`, `"1h"`, or `"7d"`.
///
/// The canonical spelling authored in the specification set is preserved
/// verbatim so a round trip is byte-identical; the parsed millisecond value
/// is used only for bound checks.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DurationMs {
    text: String,
    #[serde(skip)]
    millis: u64,
}

impl DurationMs {
    /// Parse `^[0-9]+(ms|s|m|h|d)$` and check an inclusive millisecond bound.
    pub fn parse(
        value: impl Into<String>,
        min_millis: u64,
        max_millis: u64,
    ) -> Result<Self, PrimitiveSpecError> {
        let text = value.into();
        let (digits, unit_millis) = if let Some(digits) = text.strip_suffix("ms") {
            (digits, 1u64)
        } else if let Some(digits) = text.strip_suffix('s') {
            (digits, 1_000)
        } else if let Some(digits) = text.strip_suffix('m') {
            (digits, 60_000)
        } else if let Some(digits) = text.strip_suffix('h') {
            (digits, 3_600_000)
        } else if let Some(digits) = text.strip_suffix('d') {
            (digits, 86_400_000)
        } else {
            return Err(PrimitiveSpecError::InvalidDuration);
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PrimitiveSpecError::InvalidDuration);
        }
        if digits.len() > 1 && digits.starts_with('0') {
            return Err(PrimitiveSpecError::InvalidDuration);
        }
        let millis = digits
            .parse::<u64>()
            .ok()
            .and_then(|count| count.checked_mul(unit_millis))
            .ok_or(PrimitiveSpecError::InvalidDuration)?;
        if millis < min_millis || millis > max_millis {
            return Err(PrimitiveSpecError::InvalidDuration);
        }
        Ok(Self { text, millis })
    }

    /// Borrow the canonical duration spelling.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Return the duration in milliseconds.
    pub const fn as_millis(&self) -> u64 {
        self.millis
    }
}

impl core::fmt::Debug for DurationMs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DurationMs(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for DurationMs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?, 0, u64::MAX)
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for DurationMs {
    fn schema_name() -> String {
        "DurationMs".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        string_schema_object(1, 32)
    }
}

/// A millicpu quantity such as `"500m"`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MilliCpu(u64);

impl MilliCpu {
    /// Parse a `<integer>m` millicpu quantity bounded to 1024000m.
    pub fn parse(value: &str) -> Result<Self, PrimitiveSpecError> {
        let digits = value
            .strip_suffix('m')
            .ok_or(PrimitiveSpecError::InvalidQuantity)?;
        if digits.is_empty()
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return Err(PrimitiveSpecError::InvalidQuantity);
        }
        let millicpus = digits
            .parse::<u64>()
            .map_err(|_| PrimitiveSpecError::InvalidQuantity)?;
        if millicpus > MAX_MILLICPU {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self(millicpus))
    }

    /// Return the quantity in millicpus.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Render the canonical `<integer>m` spelling.
    pub fn to_canonical_string(self) -> String {
        format!("{}m", self.0)
    }
}

impl core::fmt::Debug for MilliCpu {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MilliCpu(<redacted>)")
    }
}

impl Serialize for MilliCpu {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for MilliCpu {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for MilliCpu {
    fn schema_name() -> String {
        "MilliCpu".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        string_schema_object(2, 32)
    }
}

/// A byte quantity such as `"128Mi"` or `"512"`.
///
/// The authored spelling is preserved verbatim so a round trip is
/// byte-identical.
#[derive(Clone, PartialEq, Eq)]
pub struct ByteQuantity {
    text: String,
    bytes: u64,
}

impl ByteQuantity {
    /// Parse an optionally suffixed byte quantity bounded to 4Ti.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let text = value.into();
        let (digits, multiplier) = split_byte_suffix(&text)?;
        if digits.is_empty()
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return Err(PrimitiveSpecError::InvalidQuantity);
        }
        let bytes = digits
            .parse::<u64>()
            .ok()
            .and_then(|count| count.checked_mul(multiplier))
            .ok_or(PrimitiveSpecError::InvalidQuantity)?;
        if bytes > MAX_MEMORY_BYTES {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self { text, bytes })
    }

    /// Borrow the canonical spelling.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Return the quantity in bytes.
    pub const fn as_bytes(&self) -> u64 {
        self.bytes
    }
}

fn split_byte_suffix(text: &str) -> Result<(&str, u64), PrimitiveSpecError> {
    const SUFFIXES: [(&str, u64); 8] = [
        ("Ki", 1024),
        ("Mi", 1024 * 1024),
        ("Gi", 1024 * 1024 * 1024),
        ("Ti", 1024 * 1024 * 1024 * 1024),
        ("K", 1_000),
        ("M", 1_000_000),
        ("G", 1_000_000_000),
        ("T", 1_000_000_000_000),
    ];
    for (suffix, multiplier) in SUFFIXES {
        if let Some(digits) = text.strip_suffix(suffix) {
            return Ok((digits, multiplier));
        }
    }
    Ok((text, 1))
}

impl core::fmt::Debug for ByteQuantity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ByteQuantity(<redacted>)")
    }
}

impl Serialize for ByteQuantity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

parsed_deserialize!(ByteQuantity);

impl JsonSchema for ByteQuantity {
    fn schema_name() -> String {
        "ByteQuantity".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        string_schema_object(1, 32)
    }
}

/// The plain execution domain enumeration shared by Host, Guest, and Process.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionDomain {
    System,
    User,
}

/// Requested and limited CPU.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CpuBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<MilliCpu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<MilliCpu>,
}

/// Requested and limited memory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<ByteQuantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<ByteQuantity>,
}

/// One optional integer ceiling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// The budget schema embedded in Host, Guest, Process, and EphemeralProcess.
///
/// Every field is optional; omitting a field means no d2b-level enforcement
/// for that resource.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<CpuBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<MemoryBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pids: Option<CountBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fds: Option<CountBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    io_weight: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_egress_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_limit: Option<u32>,
}

impl BudgetSpec {
    /// Construct a budget after checking every frozen bound.
    pub fn new(
        cpu: Option<CpuBudget>,
        memory: Option<MemoryBudget>,
        pids: Option<CountBudget>,
        fds: Option<CountBudget>,
        io_weight: Option<u32>,
        network_egress_bps: Option<u64>,
        thread_limit: Option<u32>,
    ) -> Result<Self, PrimitiveSpecError> {
        check_optional_range(pids.and_then(|budget| budget.limit), 1, MAX_PIDS_LIMIT)?;
        check_optional_range(fds.and_then(|budget| budget.limit), 1, MAX_FDS_LIMIT)?;
        check_optional_range(io_weight, 1, MAX_IO_WEIGHT)?;
        check_optional_range(network_egress_bps, 0, MAX_NETWORK_EGRESS_BPS)?;
        check_optional_range(thread_limit, 1, u32::MAX)?;
        Ok(Self {
            cpu,
            memory,
            pids,
            fds,
            io_weight,
            network_egress_bps,
            thread_limit,
        })
    }

    /// Borrow the CPU budget.
    pub const fn cpu(&self) -> Option<&CpuBudget> {
        self.cpu.as_ref()
    }

    /// Borrow the memory budget.
    pub const fn memory(&self) -> Option<&MemoryBudget> {
        self.memory.as_ref()
    }

    /// Borrow the process ID budget.
    pub const fn pids(&self) -> Option<&CountBudget> {
        self.pids.as_ref()
    }

    /// Borrow the file descriptor budget.
    pub const fn fds(&self) -> Option<&CountBudget> {
        self.fds.as_ref()
    }

    /// Return the relative I/O weight.
    pub const fn io_weight(&self) -> Option<u32> {
        self.io_weight
    }

    /// Return the egress rate ceiling in bytes per second.
    pub const fn network_egress_bps(&self) -> Option<u64> {
        self.network_egress_bps
    }

    /// Return the thread ceiling.
    pub const fn thread_limit(&self) -> Option<u32> {
        self.thread_limit
    }
}

fn check_optional_range<T>(value: Option<T>, min: T, max: T) -> Result<(), PrimitiveSpecError>
where
    T: PartialOrd,
{
    match value {
        Some(value) if value < min || value > max => Err(PrimitiveSpecError::OutOfRange),
        _ => Ok(()),
    }
}

impl core::fmt::Debug for BudgetSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BudgetSpec(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for BudgetSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            cpu: Option<CpuBudget>,
            #[serde(default)]
            memory: Option<MemoryBudget>,
            #[serde(default)]
            pids: Option<CountBudget>,
            #[serde(default)]
            fds: Option<CountBudget>,
            #[serde(default)]
            io_weight: Option<u32>,
            #[serde(default)]
            network_egress_bps: Option<u64>,
            #[serde(default)]
            thread_limit: Option<u32>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.cpu,
            wire.memory,
            wire.pids,
            wire.fds,
            wire.io_weight,
            wire.network_egress_bps,
            wire.thread_limit,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One Network made available to Processes under an execution target.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAttachment {
    network_ref: ResourceRef,
    default: bool,
}

impl NetworkAttachment {
    /// Construct an attachment after checking the reference type.
    pub fn new(network_ref: ResourceRef, default: bool) -> Result<Self, PrimitiveSpecError> {
        require_resource_type(&network_ref, "Network")?;
        Ok(Self {
            network_ref,
            default,
        })
    }

    /// Borrow the referenced Network.
    pub const fn network_ref(&self) -> &ResourceRef {
        &self.network_ref
    }

    /// Whether this Network is the execution target's default.
    pub const fn is_default(&self) -> bool {
        self.default
    }
}

redacted_debug!(NetworkAttachment);

impl<'de> Deserialize<'de> for NetworkAttachment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            network_ref: ResourceRef,
            #[serde(default)]
            default: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.network_ref, wire.default).map_err(serde::de::Error::custom)
    }
}

/// One Device made available to Processes under an execution target.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAttachment {
    device_ref: ResourceRef,
    exclusive: bool,
}

impl DeviceAttachment {
    /// Construct an attachment after checking the reference type.
    pub fn new(device_ref: ResourceRef, exclusive: bool) -> Result<Self, PrimitiveSpecError> {
        require_resource_type(&device_ref, "Device")?;
        Ok(Self {
            device_ref,
            exclusive,
        })
    }

    /// Borrow the referenced Device.
    pub const fn device_ref(&self) -> &ResourceRef {
        &self.device_ref
    }

    /// Whether only one Process may hold the device at a time.
    pub const fn is_exclusive(&self) -> bool {
        self.exclusive
    }
}

redacted_debug!(DeviceAttachment);

impl<'de> Deserialize<'de> for DeviceAttachment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            device_ref: ResourceRef,
            #[serde(default)]
            exclusive: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.device_ref, wire.exclusive).map_err(serde::de::Error::custom)
    }
}

/// The shared Host and Guest execution, policy, and budget parent schema.
///
/// `providerRef` is deliberately absent: it is a universal `ResourceSpec`
/// field, not a Layer 2 base field, so embedding it here would duplicate it
/// in the rendered base object.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPolicy {
    default_domain: ExecutionDomain,
    allowed_domains: Vec<ExecutionDomain>,
    default_user_ref: Option<ResourceRef>,
    budget: BudgetSpec,
    network_attachments: Vec<NetworkAttachment>,
    device_attachments: Vec<DeviceAttachment>,
    volume_attachment_defaults: Vec<CanonicalJsonObject>,
}

impl ExecutionPolicy {
    /// Construct an execution policy after checking every frozen invariant.
    ///
    /// The `user` domain admits a Process whose spec omits `userRef`, so a
    /// policy whose `allowedDomains` contains `user` must name a
    /// `defaultUserRef` fallback. This is the `allowedDomains` superset
    /// condition, not the narrower default-domain reading.
    pub fn new(
        default_domain: ExecutionDomain,
        allowed_domains: Vec<ExecutionDomain>,
        default_user_ref: Option<ResourceRef>,
        budget: BudgetSpec,
        network_attachments: Vec<NetworkAttachment>,
        device_attachments: Vec<DeviceAttachment>,
        volume_attachment_defaults: Vec<CanonicalJsonObject>,
    ) -> Result<Self, PrimitiveSpecError> {
        if allowed_domains.is_empty() || allowed_domains.len() > 2 {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mut unique = allowed_domains.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != allowed_domains.len() {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        if !allowed_domains.contains(&default_domain) {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if allowed_domains.contains(&ExecutionDomain::User) && default_user_ref.is_none() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        if let Some(user_ref) = &default_user_ref {
            require_resource_type(user_ref, "User")?;
        }
        if network_attachments.len() > MAX_NETWORK_ATTACHMENTS
            || device_attachments.len() > MAX_DEVICE_ATTACHMENTS
            || volume_attachment_defaults.len() > MAX_VOLUME_ATTACHMENT_DEFAULTS
        {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        if network_attachments
            .iter()
            .filter(|attachment| attachment.default)
            .count()
            > 1
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self {
            default_domain,
            allowed_domains,
            default_user_ref,
            budget,
            network_attachments,
            device_attachments,
            volume_attachment_defaults,
        })
    }

    /// Construct the canonical minimal system-domain policy.
    pub fn system_default() -> Self {
        Self::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("the minimal system policy is always valid")
    }

    /// Return the default Process domain.
    pub const fn default_domain(&self) -> ExecutionDomain {
        self.default_domain
    }

    /// Borrow the admitted Process domains.
    pub fn allowed_domains(&self) -> &[ExecutionDomain] {
        &self.allowed_domains
    }

    /// Borrow the fallback user identity.
    pub const fn default_user_ref(&self) -> Option<&ResourceRef> {
        self.default_user_ref.as_ref()
    }

    /// Borrow the aggregate budget.
    pub const fn budget(&self) -> &BudgetSpec {
        &self.budget
    }

    /// Borrow the available Networks.
    pub fn network_attachments(&self) -> &[NetworkAttachment] {
        &self.network_attachments
    }

    /// Borrow the available Devices.
    pub fn device_attachments(&self) -> &[DeviceAttachment] {
        &self.device_attachments
    }

    /// Borrow the Volume attachment defaults propagated to child Processes.
    pub fn volume_attachment_defaults(&self) -> &[CanonicalJsonObject] {
        &self.volume_attachment_defaults
    }

    /// Whether the policy admits the `user` domain.
    pub fn admits_user_domain(&self) -> bool {
        self.allowed_domains.contains(&ExecutionDomain::User)
    }
}

redacted_debug!(ExecutionPolicy);

impl<'de> Deserialize<'de> for ExecutionPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ExecutionPolicyWire::deserialize(deserializer)?
            .into_policy()
            .map_err(serde::de::Error::custom)
    }
}

/// The wire mirror of `ExecutionPolicy`, flattened into the Host and Guest
/// base specs so both render exactly one copy of the shared fields.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionPolicyWire {
    #[serde(default = "default_system_domain")]
    pub(crate) default_domain: ExecutionDomain,
    #[serde(default = "default_allowed_domains")]
    pub(crate) allowed_domains: Vec<ExecutionDomain>,
    #[serde(default)]
    pub(crate) default_user_ref: Option<ResourceRef>,
    #[serde(default)]
    pub(crate) budget: BudgetSpec,
    #[serde(default)]
    pub(crate) network_attachments: Vec<NetworkAttachment>,
    #[serde(default)]
    pub(crate) device_attachments: Vec<DeviceAttachment>,
    #[serde(default)]
    pub(crate) volume_attachment_defaults: Vec<CanonicalJsonObject>,
}

impl ExecutionPolicyWire {
    pub(crate) fn into_policy(self) -> Result<ExecutionPolicy, PrimitiveSpecError> {
        ExecutionPolicy::new(
            self.default_domain,
            self.allowed_domains,
            self.default_user_ref,
            self.budget,
            self.network_attachments,
            self.device_attachments,
            self.volume_attachment_defaults,
        )
    }
}

const fn default_system_domain() -> ExecutionDomain {
    ExecutionDomain::System
}

fn default_allowed_domains() -> Vec<ExecutionDomain> {
    vec![ExecutionDomain::System]
}

pub(crate) fn string_schema_object(min: u32, max: u32) -> schemars::schema::Schema {
    let mut schema = schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::String,
        ))),
        ..Default::default()
    };
    schema.string().min_length = Some(min);
    schema.string().max_length = Some(max);
    schemars::schema::Schema::Object(schema)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_execution_policy_renders_the_canonical_base_object() {
        let policy = ExecutionPolicy::system_default();
        assert_eq!(
            canonical_json_bytes(&policy).unwrap(),
            br#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"volumeAttachmentDefaults":[]}"#
        );
        let base = to_base_object(&policy).unwrap();
        assert!(base.get("providerRef").is_none());
        assert!(base.get("provider").is_none());
        assert!(base.get("updatePolicy").is_none());
    }

    #[test]
    fn user_domain_requires_a_default_user_ref() {
        assert_eq!(
            ExecutionPolicy::new(
                ExecutionDomain::System,
                vec![ExecutionDomain::System, ExecutionDomain::User],
                None,
                BudgetSpec::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert!(
            ExecutionPolicy::new(
                ExecutionDomain::System,
                vec![ExecutionDomain::System, ExecutionDomain::User],
                Some(ResourceRef::parse("User/alice").unwrap()),
                BudgetSpec::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .is_ok()
        );
    }

    #[test]
    fn allowed_domains_bounds_and_default_membership_fail_closed() {
        assert_eq!(
            ExecutionPolicy::new(
                ExecutionDomain::System,
                Vec::new(),
                None,
                BudgetSpec::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(PrimitiveSpecError::TooManyEntries)
        );
        assert_eq!(
            ExecutionPolicy::new(
                ExecutionDomain::System,
                vec![ExecutionDomain::System, ExecutionDomain::System],
                None,
                BudgetSpec::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
        assert_eq!(
            ExecutionPolicy::new(
                ExecutionDomain::User,
                vec![ExecutionDomain::System],
                None,
                BudgetSpec::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
    }

    #[test]
    fn at_most_one_network_attachment_is_the_default() {
        let first =
            NetworkAttachment::new(ResourceRef::parse("Network/work-net").unwrap(), true).unwrap();
        let second =
            NetworkAttachment::new(ResourceRef::parse("Network/home-net").unwrap(), true).unwrap();
        assert_eq!(
            ExecutionPolicy::new(
                ExecutionDomain::System,
                vec![ExecutionDomain::System],
                None,
                BudgetSpec::default(),
                vec![first, second],
                Vec::new(),
                Vec::new(),
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            NetworkAttachment::new(ResourceRef::parse("Volume/work-net").unwrap(), false),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        assert_eq!(
            DeviceAttachment::new(ResourceRef::parse("Volume/gpu").unwrap(), false),
            Err(PrimitiveSpecError::WrongResourceType)
        );
    }

    #[test]
    fn scalar_vectors_round_trip_and_reject_out_of_bound_values() {
        assert_eq!(MilliCpu::parse("500m").unwrap().get(), 500);
        assert_eq!(
            MilliCpu::parse("500m").unwrap().to_canonical_string(),
            "500m"
        );
        assert_eq!(
            MilliCpu::parse("500"),
            Err(PrimitiveSpecError::InvalidQuantity)
        );
        assert_eq!(
            MilliCpu::parse("0500m"),
            Err(PrimitiveSpecError::InvalidQuantity)
        );
        assert_eq!(
            MilliCpu::parse("2048000m"),
            Err(PrimitiveSpecError::OutOfRange)
        );

        let memory = ByteQuantity::parse("128Mi").unwrap();
        assert_eq!(memory.as_bytes(), 128 * 1024 * 1024);
        assert_eq!(memory.as_str(), "128Mi");
        assert_eq!(
            ByteQuantity::parse("8Ti"),
            Err(PrimitiveSpecError::OutOfRange)
        );

        let duration = DurationMs::parse("30s", 1_000, 300_000).unwrap();
        assert_eq!(duration.as_millis(), 30_000);
        assert_eq!(duration.as_str(), "30s");
        assert_eq!(
            DurationMs::parse("30", 0, u64::MAX),
            Err(PrimitiveSpecError::InvalidDuration)
        );
        assert_eq!(
            DurationMs::parse("1h", 0, 60_000),
            Err(PrimitiveSpecError::InvalidDuration)
        );

        assert!(BoundedToken::parse("controller-main").is_ok());
        assert_eq!(
            BoundedToken::parse("Controller"),
            Err(PrimitiveSpecError::InvalidToken)
        );
        assert_eq!(
            BoundedText::parse("a\u{0007}b"),
            Err(PrimitiveSpecError::InvalidText)
        );
    }

    #[test]
    fn budget_bounds_fail_closed() {
        assert_eq!(
            BudgetSpec::new(
                None,
                None,
                Some(CountBudget { limit: Some(0) }),
                None,
                None,
                None,
                None
            ),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert_eq!(
            BudgetSpec::new(None, None, None, None, Some(20_000), None, None),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert!(
            BudgetSpec::new(
                Some(CpuBudget {
                    request: Some(MilliCpu::parse("500m").unwrap()),
                    limit: Some(MilliCpu::parse("2000m").unwrap()),
                }),
                Some(MemoryBudget {
                    request: Some(ByteQuantity::parse("128Mi").unwrap()),
                    limit: Some(ByteQuantity::parse("512Mi").unwrap()),
                }),
                Some(CountBudget { limit: Some(512) }),
                Some(CountBudget { limit: Some(1024) }),
                Some(100),
                None,
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn every_primitive_resource_type_is_declared_exactly_once_and_stays_unqualified() {
        use crate::v3::{
            ResourceTypeName,
            credential::CREDENTIAL_RESOURCE_TYPE,
            device::DEVICE_RESOURCE_TYPE,
            guest::GUEST_RESOURCE_TYPE,
            host::HOST_RESOURCE_TYPE,
            network::NETWORK_RESOURCE_TYPE,
            process::{EPHEMERAL_PROCESS_RESOURCE_TYPE, PROCESS_RESOURCE_TYPE},
            user::USER_RESOURCE_TYPE,
            volume::VOLUME_RESOURCE_TYPE,
        };

        let declared = [
            HOST_RESOURCE_TYPE,
            GUEST_RESOURCE_TYPE,
            PROCESS_RESOURCE_TYPE,
            EPHEMERAL_PROCESS_RESOURCE_TYPE,
            VOLUME_RESOURCE_TYPE,
            USER_RESOURCE_TYPE,
            NETWORK_RESOURCE_TYPE,
            DEVICE_RESOURCE_TYPE,
            CREDENTIAL_RESOURCE_TYPE,
        ];
        let mut unique = declared.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            declared.len(),
            "a ResourceType name is declared by more than one primitive module"
        );
        for name in declared {
            let parsed = ResourceTypeName::parse(name).expect("standard ResourceType name");
            assert!(
                !parsed.as_str().contains(".d2bus.org."),
                "a standard ResourceType name must stay unqualified"
            );
        }
    }

    #[test]
    fn every_primitive_base_object_folds_rather_than_restating_a_universal_field() {
        use crate::v3::{
            ResourceRef,
            credential::{AudienceToken, CredentialSpec},
            device::DeviceSpec,
            guest::GuestSpec,
            host::HostSpec,
            process::{ExecutionSpec, ProcessClass, ProcessSpec},
            user::{OsUsername, UserSpec},
        };

        let execution = ExecutionSpec::minimal(
            ResourceRef::parse("Host/host-system").unwrap(),
            ProcessClass::Worker,
            BoundedToken::parse("worker-main").unwrap(),
        )
        .unwrap();
        let objects = [
            to_base_object(&HostSpec::system_default()).unwrap(),
            to_base_object(&GuestSpec::system_default()).unwrap(),
            to_base_object(&ProcessSpec::minimal(execution)).unwrap(),
            to_base_object(&UserSpec::minimal(OsUsername::parse("alice").unwrap())).unwrap(),
            to_base_object(&DeviceSpec::emulated_exclusive()).unwrap(),
            to_base_object(&CredentialSpec::minimal(
                AudienceToken::parse("azure-resource-manager").unwrap(),
            ))
            .unwrap(),
        ];
        for object in objects {
            for reserved in [
                "providerRef",
                "updatePolicy",
                "provider",
                "providerSettings",
                "settings",
                "artifactId",
                "config",
            ] {
                assert!(
                    object.get(reserved).is_none(),
                    "a primitive base spec restated a universal or Provider-layer field"
                );
            }
        }
    }

    #[test]
    fn diagnostics_never_echo_a_caller_supplied_marker() {
        let marker = format!("marker-{:x}", std::process::id());
        let token = BoundedToken::parse(marker.clone()).unwrap();
        let text = BoundedText::parse(marker.clone()).unwrap();
        let policy = ExecutionPolicy::system_default();
        let attachment =
            NetworkAttachment::new(ResourceRef::parse("Network/work-net").unwrap(), false).unwrap();
        for rendered in [
            format!("{token:?}"),
            format!("{text:?}"),
            format!("{policy:?}"),
            format!("{attachment:?}"),
            format!("{:?}", BudgetSpec::default()),
            format!("{:?}", DurationMs::parse("1s", 0, u64::MAX).unwrap()),
            format!("{:?}", ByteQuantity::parse("1Mi").unwrap()),
            format!("{:?}", MilliCpu::parse("1m").unwrap()),
        ] {
            assert!(!rendered.contains(&marker));
        }
        assert_eq!(token.as_str(), marker);
        assert_eq!(text.as_str(), marker);
    }
}
