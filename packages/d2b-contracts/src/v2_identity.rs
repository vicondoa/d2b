//! Canonical d2b 2.0 human names and runtime identities.
//!
//! Runtime IDs are the lowercase, unpadded RFC 4648 base32 encoding of the
//! first 96 bits of SHA-256 over ADR 0045's length-prefixed ASCII grammar.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, error::Error, fmt, str::FromStr};

pub const SHORT_ID_LEN: usize = 20;
pub const LINUX_UNIX_PATH_MAX_BYTES: usize = 107;
pub const MAX_CANONICAL_NAME_BYTES: usize = 63;

const PREFIX: &str = "d2b-id-v2;";
const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    InvalidRealmLabel,
    InvalidRealmPath,
    InvalidWorkloadName,
    InvalidProviderInstanceId,
    InvalidShortId,
    InvalidEncoding,
    UnknownDomain,
    InvalidDomainParts,
    RecomputedIdMismatch,
    DuplicateProviderId,
    ShortIdCollision,
    UnixPathContainsNul,
    UnixPathTooLong,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRealmLabel => "invalid canonical realm label",
            Self::InvalidRealmPath => "invalid canonical realm path",
            Self::InvalidWorkloadName => "invalid canonical workload name",
            Self::InvalidProviderInstanceId => "invalid configured provider instance id",
            Self::InvalidShortId => "invalid canonical short id",
            Self::InvalidEncoding => "invalid canonical identity encoding",
            Self::UnknownDomain => "unknown canonical identity domain",
            Self::InvalidDomainParts => "invalid parts for canonical identity domain",
            Self::RecomputedIdMismatch => "canonical identity recomputation mismatch",
            Self::DuplicateProviderId => "duplicate globally scoped provider id",
            Self::ShortIdCollision => "canonical short-id collision",
            Self::UnixPathContainsNul => "Unix pathname contains NUL",
            Self::UnixPathTooLong => "Unix pathname exceeds the Linux pathname limit",
        })
    }
}

impl Error for IdentityError {}

fn valid_canonical_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CANONICAL_NAME_BYTES
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

macro_rules! canonical_name {
    ($name:ident, $error:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                if valid_canonical_name(&value) {
                    Ok(Self(value))
                } else {
                    Err(IdentityError::$error)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
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
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

canonical_name!(
    RealmLabel,
    InvalidRealmLabel,
    "A schema-validated lowercase ASCII realm label."
);
canonical_name!(
    WorkloadName,
    InvalidWorkloadName,
    "A schema-validated lowercase ASCII workload name."
);
canonical_name!(
    ConfiguredProviderId,
    InvalidProviderInstanceId,
    "A schema-validated lowercase ASCII configured provider instance ID."
);

/// A leaf-to-root realm path ending in the literal `local-root`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct RealmPath(String);

impl RealmPath {
    pub fn root() -> Self {
        Self("local-root".to_owned())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        let components: Vec<&str> = value.split('.').collect();
        let valid = !value.is_empty()
            && value.is_ascii()
            && !value.ends_with(".d2b")
            && components.last() == Some(&"local-root")
            && components[..components.len().saturating_sub(1)]
                .iter()
                .all(|label| valid_canonical_name(label));
        if valid {
            Ok(Self(value))
        } else {
            Err(IdentityError::InvalidRealmPath)
        }
    }

    pub fn child(label: &RealmLabel, parent: &Self) -> Self {
        Self(format!("{}.{}", label.as_str(), parent.as_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RealmPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RealmPath").field(&self.0).finish()
    }
}

impl fmt::Display for RealmPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RealmPath {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for RealmPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Runtime,
    Infrastructure,
    Transport,
    Substrate,
    Credential,
    Display,
    Network,
    Storage,
    Device,
    Audio,
    Observability,
}

impl ProviderType {
    pub const ALL: [Self; 11] = [
        Self::Runtime,
        Self::Infrastructure,
        Self::Transport,
        Self::Substrate,
        Self::Credential,
        Self::Display,
        Self::Network,
        Self::Storage,
        Self::Device,
        Self::Audio,
        Self::Observability,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Infrastructure => "infrastructure",
            Self::Transport => "transport",
            Self::Substrate => "substrate",
            Self::Credential => "credential",
            Self::Display => "display",
            Self::Network => "network",
            Self::Storage => "storage",
            Self::Device => "device",
            Self::Audio => "audio",
            Self::Observability => "observability",
        }
    }
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderType {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|item| item.as_str() == value)
            .ok_or(IdentityError::InvalidDomainParts)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleKind {
    StoreVirtiofsPreflight,
    SwtpmPreStartFlush,
    Swtpm,
    Virtiofsd,
    Video,
    Gpu,
    GpuRenderNode,
    Audio,
    CloudHypervisor,
    QemuMedia,
    VsockRelay,
    GuestControlHealth,
    Usbip,
    SecurityKeyFrontend,
    WaylandProxy,
}

impl RoleKind {
    pub const ALL: [Self; 15] = [
        Self::StoreVirtiofsPreflight,
        Self::SwtpmPreStartFlush,
        Self::Swtpm,
        Self::Virtiofsd,
        Self::Video,
        Self::Gpu,
        Self::GpuRenderNode,
        Self::Audio,
        Self::CloudHypervisor,
        Self::QemuMedia,
        Self::VsockRelay,
        Self::GuestControlHealth,
        Self::Usbip,
        Self::SecurityKeyFrontend,
        Self::WaylandProxy,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StoreVirtiofsPreflight => "store-virtiofs-preflight",
            Self::SwtpmPreStartFlush => "swtpm-pre-start-flush",
            Self::Swtpm => "swtpm",
            Self::Virtiofsd => "virtiofsd",
            Self::Video => "video",
            Self::Gpu => "gpu",
            Self::GpuRenderNode => "gpu-render-node",
            Self::Audio => "audio",
            Self::CloudHypervisor => "cloud-hypervisor",
            Self::QemuMedia => "qemu-media",
            Self::VsockRelay => "vsock-relay",
            Self::GuestControlHealth => "guest-control-health",
            Self::Usbip => "usbip",
            Self::SecurityKeyFrontend => "security-key-frontend",
            Self::WaylandProxy => "wayland-proxy",
        }
    }
}

impl fmt::Display for RoleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RoleKind {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|item| item.as_str() == value)
            .ok_or(IdentityError::InvalidDomainParts)
    }
}

/// The four closed domains accepted by the canonical identity grammar.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityDomain {
    Realm,
    Workload,
    Provider,
    Role,
}

impl IdentityDomain {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Realm => "d2b-v2:realm",
            Self::Workload => "d2b-v2:workload",
            Self::Provider => "d2b-v2:provider",
            Self::Role => "d2b-v2:role",
        }
    }

    const fn part_count(self) -> usize {
        match self {
            Self::Realm => 1,
            Self::Workload => 2,
            Self::Provider | Self::Role => 3,
        }
    }
}

impl FromStr for IdentityDomain {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "d2b-v2:realm" => Ok(Self::Realm),
            "d2b-v2:workload" => Ok(Self::Workload),
            "d2b-v2:provider" => Ok(Self::Provider),
            "d2b-v2:role" => Ok(Self::Role),
            _ => Err(IdentityError::UnknownDomain),
        }
    }
}

/// A structurally canonical length-prefixed encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEncoding {
    domain: IdentityDomain,
    parts: Vec<String>,
}

impl CanonicalEncoding {
    pub fn new(domain: IdentityDomain, parts: Vec<String>) -> Result<Self, IdentityError> {
        if parts.len() != domain.part_count()
            || parts
                .iter()
                .any(|part| part.is_empty() || !printable_ascii(part))
        {
            return Err(IdentityError::InvalidEncoding);
        }
        Ok(Self { domain, parts })
    }

    pub fn parse(encoded: &str) -> Result<Self, IdentityError> {
        if !printable_ascii(encoded) {
            return Err(IdentityError::InvalidEncoding);
        }
        let mut cursor = encoded
            .strip_prefix(PREFIX)
            .ok_or(IdentityError::InvalidEncoding)?;
        let domain = parse_field(&mut cursor)?;
        let domain = IdentityDomain::from_str(domain)?;
        let count_text = take_until(&mut cursor, b';')?;
        let count = parse_decimal(count_text)?;
        if count != domain.part_count() {
            return Err(IdentityError::InvalidEncoding);
        }
        let mut parts = Vec::with_capacity(count);
        for _ in 0..count {
            parts.push(parse_field(&mut cursor)?.to_owned());
        }
        if !cursor.is_empty() {
            return Err(IdentityError::InvalidEncoding);
        }
        Self::new(domain, parts)
    }

    pub fn domain(&self) -> IdentityDomain {
        self.domain
    }

    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    pub fn encode(&self) -> String {
        let mut encoded = String::from(PREFIX);
        push_field(&mut encoded, self.domain.as_str());
        encoded.push_str(&self.parts.len().to_string());
        encoded.push(';');
        for part in &self.parts {
            push_field(&mut encoded, part);
        }
        encoded
    }

    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.encode().as_bytes()).into()
    }

    pub fn short_id(&self) -> ShortId {
        ShortId(base32_first_96(&self.digest()))
    }

    pub fn recompute(&self) -> Result<CanonicalIdentity, IdentityError> {
        match self.domain {
            IdentityDomain::Realm => {
                let path = RealmPath::parse(&self.parts[0])?;
                Ok(CanonicalIdentity::Realm(RealmId::derive(&path)))
            }
            IdentityDomain::Workload => {
                let realm = RealmId::from_str(&self.parts[0])?;
                let workload = WorkloadName::parse(&self.parts[1])?;
                Ok(CanonicalIdentity::Workload(WorkloadId::derive(
                    &realm, &workload,
                )))
            }
            IdentityDomain::Provider => {
                let realm = RealmId::from_str(&self.parts[0])?;
                let provider_type = ProviderType::from_str(&self.parts[1])?;
                let configured = ConfiguredProviderId::parse(&self.parts[2])?;
                Ok(CanonicalIdentity::Provider(ProviderId::derive(
                    &realm,
                    provider_type,
                    &configured,
                )))
            }
            IdentityDomain::Role => {
                let realm = RealmId::from_str(&self.parts[0])?;
                let workload = WorkloadId::from_str(&self.parts[1])?;
                let role = RoleKind::from_str(&self.parts[2])?;
                Ok(CanonicalIdentity::Role(RoleId::derive(
                    &realm, &workload, role,
                )))
            }
        }
    }
}

fn printable_ascii(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn push_field(encoded: &mut String, value: &str) {
    encoded.push_str(&value.len().to_string());
    encoded.push(':');
    encoded.push_str(value);
    encoded.push(';');
}

fn take_until<'a>(cursor: &mut &'a str, delimiter: u8) -> Result<&'a str, IdentityError> {
    let offset = cursor
        .bytes()
        .position(|byte| byte == delimiter)
        .ok_or(IdentityError::InvalidEncoding)?;
    let value = &cursor[..offset];
    *cursor = &cursor[offset + 1..];
    Ok(value)
}

fn parse_decimal(value: &str) -> Result<usize, IdentityError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(IdentityError::InvalidEncoding);
    }
    value
        .parse::<usize>()
        .map_err(|_| IdentityError::InvalidEncoding)
}

fn parse_field<'a>(cursor: &mut &'a str) -> Result<&'a str, IdentityError> {
    let length_text = take_until(cursor, b':')?;
    let length = parse_decimal(length_text)?;
    if length == 0 || cursor.len() <= length || cursor.as_bytes()[length] != b';' {
        return Err(IdentityError::InvalidEncoding);
    }
    let value = &cursor[..length];
    if !printable_ascii(value) {
        return Err(IdentityError::InvalidEncoding);
    }
    *cursor = &cursor[length + 1..];
    Ok(value)
}

fn base32_first_96(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(SHORT_ID_LEN);
    for index in 0..SHORT_ID_LEN {
        let first_bit = index * 5;
        let mut value = 0_u8;
        for offset in 0..5 {
            value <<= 1;
            let bit = first_bit + offset;
            if bit < 96 {
                value |= (digest[bit / 8] >> (7 - bit % 8)) & 1;
            }
        }
        output.push(char::from(BASE32_ALPHABET[usize::from(value)]));
    }
    output
}

/// The shared exact 20-character lowercase unpadded RFC 4648 representation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ShortId(String);

impl ShortId {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        let valid = value.len() == SHORT_ID_LEN
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte))
            && matches!(value.as_bytes()[SHORT_ID_LEN - 1], b'a' | b'q');
        if valid {
            Ok(Self(value))
        } else {
            Err(IdentityError::InvalidShortId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn has_path_safe_shape(&self) -> bool {
        self.0.len() == SHORT_ID_LEN && !self.0.as_bytes().contains(&0)
    }
}

impl fmt::Debug for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ShortId").field(&self.0).finish()
    }
}

impl fmt::Display for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ShortId {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ShortId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

macro_rules! runtime_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(ShortId);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                ShortId::parse(value).map(Self)
            }

            pub fn short_id(&self) -> &ShortId {
                &self.0
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

runtime_id!(RealmId);
runtime_id!(WorkloadId);
runtime_id!(ProviderId);
runtime_id!(RoleId);

impl RealmId {
    pub fn derive(path: &RealmPath) -> Self {
        Self(
            CanonicalEncoding::new(IdentityDomain::Realm, vec![path.as_str().to_owned()])
                .expect("validated realm path is structurally encodable")
                .short_id(),
        )
    }
}

impl WorkloadId {
    pub fn derive(realm: &RealmId, workload: &WorkloadName) -> Self {
        Self(
            CanonicalEncoding::new(
                IdentityDomain::Workload,
                vec![realm.as_str().to_owned(), workload.as_str().to_owned()],
            )
            .expect("validated workload identity is structurally encodable")
            .short_id(),
        )
    }
}

impl ProviderId {
    pub fn derive(
        realm: &RealmId,
        provider_type: ProviderType,
        configured: &ConfiguredProviderId,
    ) -> Self {
        Self(
            CanonicalEncoding::new(
                IdentityDomain::Provider,
                vec![
                    realm.as_str().to_owned(),
                    provider_type.as_str().to_owned(),
                    configured.as_str().to_owned(),
                ],
            )
            .expect("validated provider identity is structurally encodable")
            .short_id(),
        )
    }
}

impl RoleId {
    pub fn derive(realm: &RealmId, workload: &WorkloadId, role: RoleKind) -> Self {
        Self(
            CanonicalEncoding::new(
                IdentityDomain::Role,
                vec![
                    realm.as_str().to_owned(),
                    workload.as_str().to_owned(),
                    role.as_str().to_owned(),
                ],
            )
            .expect("validated role identity is structurally encodable")
            .short_id(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalIdentity {
    Realm(RealmId),
    Workload(WorkloadId),
    Provider(ProviderId),
    Role(RoleId),
}

impl CanonicalIdentity {
    pub fn short_id(&self) -> &ShortId {
        match self {
            Self::Realm(value) => value.short_id(),
            Self::Workload(value) => value.short_id(),
            Self::Provider(value) => value.short_id(),
            Self::Role(value) => value.short_id(),
        }
    }
}

pub fn recompute_canonical_identity(encoded: &str) -> Result<CanonicalIdentity, IdentityError> {
    CanonicalEncoding::parse(encoded)?.recompute()
}

pub fn verify_canonical_identity(
    encoded: &str,
    claimed: &ShortId,
) -> Result<CanonicalIdentity, IdentityError> {
    let identity = recompute_canonical_identity(encoded)?;
    if identity.short_id() == claimed {
        Ok(identity)
    } else {
        Err(IdentityError::RecomputedIdMismatch)
    }
}

/// Reject duplicate provider IDs and every repeated short ID across all domains.
pub fn validate_global_identities(
    realms: &[RealmId],
    workloads: &[WorkloadId],
    providers: &[ProviderId],
    roles: &[RoleId],
) -> Result<(), IdentityError> {
    let mut provider_ids = BTreeSet::new();
    for provider in providers {
        if !provider_ids.insert(provider.as_str()) {
            return Err(IdentityError::DuplicateProviderId);
        }
    }

    let mut all_ids = BTreeSet::new();
    for id in realms
        .iter()
        .map(RealmId::as_str)
        .chain(workloads.iter().map(WorkloadId::as_str))
        .chain(providers.iter().map(ProviderId::as_str))
        .chain(roles.iter().map(RoleId::as_str))
    {
        if !all_ids.insert(id) {
            return Err(IdentityError::ShortIdCollision);
        }
    }
    Ok(())
}

/// Return remaining pathname bytes before Linux's required terminating NUL.
pub fn unix_path_headroom(path: &str) -> Result<usize, IdentityError> {
    if path.as_bytes().contains(&0) {
        return Err(IdentityError::UnixPathContainsNul);
    }
    LINUX_UNIX_PATH_MAX_BYTES
        .checked_sub(path.len())
        .ok_or(IdentityError::UnixPathTooLong)
}

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
