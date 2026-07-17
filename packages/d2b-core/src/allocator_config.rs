use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::Digest as _;
use std::fmt;

use crate::contract_id::{ContractId, ContractStringError, PathTemplate};

pub const MAX_ALLOCATOR_REALM_PATH_BYTES: usize = 255;
pub const MAX_ALLOCATOR_REALM_PATH_LABELS: usize = 16;
pub const MAX_ALLOCATOR_PROVIDER_KIND_BYTES: usize = 64;
pub const MAX_ALLOCATOR_PROCESS_LAUNCH_ROWS: usize = 64;
pub const MAX_ALLOCATOR_LAUNCH_REF_BYTES: usize = 128;
pub const MAX_ALLOCATOR_LAUNCH_RESOURCE_REFS: usize = 32;
pub const MAX_ALLOCATOR_EXECUTABLE_REF_BYTES: usize = 1024;
pub const MAX_ALLOCATOR_REALM_ID_BYTES: usize = 20;

const ALLOCATOR_REALM_PATH_PATTERN: &str = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*){0,15}$";
const ALLOCATOR_PROVIDER_KIND_PATTERN: &str = "^[a-z][a-z0-9-]*$";
const ALLOCATOR_LAUNCH_REF_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._-]*$";
const ALLOCATOR_EXECUTABLE_REF_PATTERN: &str =
    "^(/nix/store/|/run/current-system/sw/bin/)[A-Za-z0-9._+/-]+$";

fn validate_realm_path(raw: String) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > MAX_ALLOCATOR_REALM_PATH_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_ALLOCATOR_REALM_PATH_BYTES,
        });
    }

    let labels = raw.split('.').collect::<Vec<_>>();
    if labels.is_empty() || labels.len() > MAX_ALLOCATOR_REALM_PATH_LABELS {
        return Err(ContractStringError::BadShape);
    }
    if labels.iter().all(|label| {
        let mut chars = label.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }) {
        Ok(raw)
    } else {
        Err(ContractStringError::BadShape)
    }
}

fn validate_provider_kind(raw: String) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > MAX_ALLOCATOR_PROVIDER_KIND_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_ALLOCATOR_PROVIDER_KIND_BYTES,
        });
    }

    let mut chars = raw.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Err(ContractStringError::BadShape),
    }
    if chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        Ok(raw)
    } else {
        Err(ContractStringError::BadShape)
    }
}

macro_rules! allocator_string {
    ($name:ident, $parse_fn:ident, $max:expr, $pattern:expr, $description:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: impl Into<String>) -> Result<Self, ContractStringError> {
                $parse_fn(raw.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
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
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    string: Some(Box::new(StringValidation {
                        max_length: Some($max as u32),
                        min_length: Some(1),
                        pattern: Some($pattern.to_owned()),
                    })),
                    metadata: Some(Box::new(schemars::schema::Metadata {
                        description: Some($description.to_owned()),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

allocator_string!(
    AllocatorRealmPath,
    validate_realm_path,
    MAX_ALLOCATOR_REALM_PATH_BYTES,
    ALLOCATOR_REALM_PATH_PATTERN,
    "Realm path copied from d2b.realms, most-specific first, with 1-16 DNS-style labels and a 255-byte cap."
);

allocator_string!(
    AllocatorProviderKind,
    validate_provider_kind,
    MAX_ALLOCATOR_PROVIDER_KIND_BYTES,
    ALLOCATOR_PROVIDER_KIND_PATTERN,
    "Opaque provider adapter kind slug, bounded for schema-only allocator metadata."
);

fn validate_launch_ref(raw: String) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > MAX_ALLOCATOR_LAUNCH_REF_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_ALLOCATOR_LAUNCH_REF_BYTES,
        });
    }
    let mut chars = raw.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        || raw.contains("..")
    {
        return Err(ContractStringError::BadShape);
    }
    let compact = raw
        .chars()
        .filter(|c| !matches!(c, '.' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if [
        "secret",
        "password",
        "passwd",
        "bearer",
        "credential",
        "privatekey",
        "apikey",
        "accesstoken",
        "refreshtoken",
        "sessiontoken",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
    {
        return Err(ContractStringError::BadShape);
    }
    Ok(raw)
}

fn validate_realm_id(raw: String) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > MAX_ALLOCATOR_REALM_ID_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_ALLOCATOR_REALM_ID_BYTES,
        });
    }
    validate_provider_kind(raw)
}

fn validate_executable_ref(raw: String) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > MAX_ALLOCATOR_EXECUTABLE_REF_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_ALLOCATOR_EXECUTABLE_REF_BYTES,
        });
    }
    if (!raw.starts_with("/nix/store/") && !raw.starts_with("/run/current-system/sw/bin/"))
        || raw.contains("//")
        || raw.split('/').any(|part| part == "..")
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '+' | '-'))
    {
        return Err(ContractStringError::BadShape);
    }
    Ok(raw)
}

macro_rules! redacted_allocator_string {
    ($name:ident, $parse_fn:ident, $max:expr, $pattern:expr, $description:expr) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: impl Into<String>) -> Result<Self, ContractStringError> {
                $parse_fn(raw.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}(<{} bytes>)", stringify!($name), self.0.len())
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
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    string: Some(Box::new(StringValidation {
                        max_length: Some($max as u32),
                        min_length: Some(1),
                        pattern: Some($pattern.to_owned()),
                    })),
                    metadata: Some(Box::new(schemars::schema::Metadata {
                        description: Some($description.to_owned()),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

redacted_allocator_string!(
    AllocatorLaunchRef,
    validate_launch_ref,
    MAX_ALLOCATOR_LAUNCH_REF_BYTES,
    ALLOCATOR_LAUNCH_REF_PATTERN,
    "Opaque bounded allocator launch-authority reference. Paths and credential-shaped values are rejected."
);

redacted_allocator_string!(
    AllocatorRealmId,
    validate_realm_id,
    MAX_ALLOCATOR_REALM_ID_BYTES,
    ALLOCATOR_PROVIDER_KIND_PATTERN,
    "Canonical bounded lowercase realm identifier used by host-local child processes."
);

redacted_allocator_string!(
    AllocatorExecutableRef,
    validate_executable_ref,
    MAX_ALLOCATOR_EXECUTABLE_REF_BYTES,
    ALLOCATOR_EXECUTABLE_REF_PATTERN,
    "Integrity-bound executable reference under /nix/store or /run/current-system/sw/bin."
);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AllocatorLaunchDigest([u8; 32]);

impl AllocatorLaunchDigest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for AllocatorLaunchDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AllocatorLaunchDigest(<redacted>)")
    }
}

impl Serialize for AllocatorLaunchDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let hex = self
            .0
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        serializer.serialize_str(&format!("sha256:{hex}"))
    }
}

impl<'de> Deserialize<'de> for AllocatorLaunchDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let hex = raw
            .strip_prefix("sha256:")
            .filter(|value| value.len() == 64)
            .ok_or_else(|| serde::de::Error::custom("launch digest must be sha256:<hex64>"))?;
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                .map_err(|_| serde::de::Error::custom("launch digest contains non-hex bytes"))?;
        }
        Ok(Self(bytes))
    }
}

impl JsonSchema for AllocatorLaunchDigest {
    fn schema_name() -> String {
        "AllocatorLaunchDigest".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(71),
                min_length: Some(71),
                pattern: Some("^sha256:[0-9a-f]{64}$".to_owned()),
            })),
            metadata: Some(Box::new(schemars::schema::Metadata {
                description: Some("SHA-256 digest. Debug output is always redacted.".to_owned()),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorJson {
    pub schema_version: String,
    pub allocator: AllocatorRoot,
    pub realms: Vec<AllocatorRealm>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_requests: Vec<AllocatorResourceRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_partitions: Vec<AllocatorPathPartition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_placements: Vec<AllocatorProviderPlacement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_bridge: Vec<AllocatorEnvBridge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 64))]
    pub process_launch: Vec<AllocatorProcessLaunch>,
    pub invariants: AllocatorInvariants,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AllocatorJsonWire {
    schema_version: String,
    allocator: AllocatorRoot,
    realms: Vec<AllocatorRealm>,
    #[serde(default)]
    resource_requests: Vec<AllocatorResourceRequest>,
    #[serde(default)]
    path_partitions: Vec<AllocatorPathPartition>,
    #[serde(default)]
    provider_placements: Vec<AllocatorProviderPlacement>,
    #[serde(default)]
    env_bridge: Vec<AllocatorEnvBridge>,
    #[serde(default)]
    process_launch: Vec<AllocatorProcessLaunch>,
    invariants: AllocatorInvariants,
}

impl<'de> Deserialize<'de> for AllocatorJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AllocatorJsonWire::deserialize(deserializer)?;
        let value = Self {
            schema_version: wire.schema_version,
            allocator: wire.allocator,
            realms: wire.realms,
            resource_requests: wire.resource_requests,
            path_partitions: wire.path_partitions,
            provider_placements: wire.provider_placements,
            env_bridge: wire.env_bridge,
            process_launch: wire.process_launch,
            invariants: wire.invariants,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl AllocatorJson {
    pub fn validate(&self) -> Result<(), AllocatorConfigError> {
        if self.process_launch.len() > MAX_ALLOCATOR_PROCESS_LAUNCH_ROWS {
            return Err(AllocatorConfigError::TooManyProcessLaunchRows);
        }

        let mut previous = None;
        for row in &self.process_launch {
            row.validate()?;
            let key = (row.realm_id.as_str(), row.controller_generation.as_str());
            if previous.is_some_and(|prior| prior >= key) {
                return Err(AllocatorConfigError::ProcessLaunchNotStrictlySorted);
            }
            previous = Some(key);

            let mut realms = self.realms.iter().filter(|realm| {
                realm.realm_id.as_str() == row.realm_id.as_str()
                    && realm.realm_path == row.realm_path
            });
            let Some(realm) = realms.next() else {
                return Err(AllocatorConfigError::ProcessLaunchRealmMismatch);
            };
            if realms.next().is_some()
                || !realm.enabled
                || realm.placement != RealmPlacement::HostLocal
            {
                return Err(AllocatorConfigError::ProcessLaunchRealmMismatch);
            }
        }
        Ok(())
    }

    pub fn find_process_launch(
        &self,
        realm_id: &str,
        controller_generation: &str,
    ) -> Option<&AllocatorProcessLaunch> {
        let realm_id = AllocatorRealmId::parse(realm_id).ok()?;
        let generation = AllocatorLaunchRef::parse(controller_generation).ok()?;
        self.process_launch
            .binary_search_by(|row| {
                (row.realm_id.as_str(), row.controller_generation.as_str())
                    .cmp(&(realm_id.as_str(), generation.as_str()))
            })
            .ok()
            .map(|index| &self.process_launch[index])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorConfigError {
    TooManyProcessLaunchRows,
    ProcessLaunchNotStrictlySorted,
    ProcessLaunchRealmMismatch,
    LaunchRecordDigestMismatch,
    ChildRoleMismatch,
    ChildIdentityMismatch,
    InvalidChildIdentity,
    ResourceRefsNotStrictlySorted,
    LeaseRefsNotStrictlySorted,
    TooManyResourceRefs,
    TooManyLeaseRefs,
    UnsafeSpawnAuthority,
}

impl fmt::Display for AllocatorConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyProcessLaunchRows => "processLaunch exceeds its row bound",
            Self::ProcessLaunchNotStrictlySorted => {
                "processLaunch rows are not strictly sorted and unique"
            }
            Self::ProcessLaunchRealmMismatch => {
                "processLaunch realm identity does not match one enabled host-local realm"
            }
            Self::LaunchRecordDigestMismatch => "processLaunch digest does not match its row",
            Self::ChildRoleMismatch => "processLaunch child role is not canonical",
            Self::ChildIdentityMismatch => {
                "processLaunch child process identities are not distinct"
            }
            Self::InvalidChildIdentity => "processLaunch child identity must be non-root",
            Self::ResourceRefsNotStrictlySorted => {
                "processLaunch resource refs are not strictly sorted and unique"
            }
            Self::LeaseRefsNotStrictlySorted => {
                "processLaunch lease refs are not strictly sorted and unique"
            }
            Self::TooManyResourceRefs => "processLaunch resource refs exceed their bound",
            Self::TooManyLeaseRefs => "processLaunch lease refs exceed their bound",
            Self::UnsafeSpawnAuthority => "processLaunch spawn authority is not fail-closed",
        })
    }
}

impl std::error::Error for AllocatorConfigError {}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorProcessLaunch {
    pub realm_id: AllocatorRealmId,
    pub realm_path: AllocatorRealmPath,
    pub controller_generation: AllocatorLaunchRef,
    pub launch_record_digest: AllocatorLaunchDigest,
    pub controller: AllocatorRealmChildLaunch,
    pub broker: AllocatorRealmChildLaunch,
}

impl fmt::Debug for AllocatorProcessLaunch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AllocatorProcessLaunch")
            .field("realm_id", &"<redacted>")
            .field("realm_path", &"<redacted>")
            .field("controller_generation", &"<redacted>")
            .field("launch_record_digest", &self.launch_record_digest)
            .field("controller", &self.controller)
            .field("broker", &self.broker)
            .finish()
    }
}

impl AllocatorProcessLaunch {
    pub fn computed_digest(&self) -> AllocatorLaunchDigest {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DigestMaterial<'a> {
            realm_id: &'a AllocatorRealmId,
            realm_path: &'a AllocatorRealmPath,
            controller_generation: &'a AllocatorLaunchRef,
            controller: &'a AllocatorRealmChildLaunch,
            broker: &'a AllocatorRealmChildLaunch,
        }

        let material = DigestMaterial {
            realm_id: &self.realm_id,
            realm_path: &self.realm_path,
            controller_generation: &self.controller_generation,
            controller: &self.controller,
            broker: &self.broker,
        };
        let canonical = serde_json::to_value(&material)
            .expect("allocator launch digest material serialization cannot fail");
        let bytes = serde_json::to_vec(&canonical)
            .expect("allocator launch digest canonicalization cannot fail");
        AllocatorLaunchDigest::from_bytes(sha2::Sha256::digest(bytes).into())
    }

    pub fn validate(&self) -> Result<(), AllocatorConfigError> {
        if self.controller.role != AllocatorRealmChildRole::Controller
            || self.broker.role != AllocatorRealmChildRole::Broker
        {
            return Err(AllocatorConfigError::ChildRoleMismatch);
        }
        if self.controller.process_id == self.broker.process_id {
            return Err(AllocatorConfigError::ChildIdentityMismatch);
        }
        if self.controller.uid == 0
            || self.broker.uid == 0
            || self.controller.uid == self.broker.uid
        {
            return Err(AllocatorConfigError::InvalidChildIdentity);
        }
        self.controller.validate()?;
        self.broker.validate()?;
        if self.computed_digest() != self.launch_record_digest {
            return Err(AllocatorConfigError::LaunchRecordDigestMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorRealmChildRole {
    Controller,
    Broker,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorRealmChildLaunch {
    pub role: AllocatorRealmChildRole,
    pub process_id: AllocatorLaunchRef,
    pub executable_ref: AllocatorExecutableRef,
    pub executable_digest: AllocatorLaunchDigest,
    pub config_ref: AllocatorLaunchRef,
    pub config_digest: AllocatorLaunchDigest,
    pub uid: u32,
    pub gid: u32,
    pub listener_ref: AllocatorLaunchRef,
    pub bootstrap_session_ref: AllocatorLaunchRef,
    pub cgroup_ref: AllocatorLaunchRef,
    pub cgroup_digest: AllocatorLaunchDigest,
    pub state_root_ref: AllocatorLaunchRef,
    pub audit_root_ref: AllocatorLaunchRef,
    pub namespaces: AllocatorRealmChildNamespaces,
    #[schemars(length(max = 32))]
    pub resource_refs: Vec<AllocatorLaunchRef>,
    #[schemars(length(max = 32))]
    pub lease_refs: Vec<AllocatorLaunchRef>,
    pub spawn: AllocatorRealmChildSpawnAuthority,
}

impl fmt::Debug for AllocatorRealmChildLaunch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AllocatorRealmChildLaunch")
            .field("role", &self.role)
            .field("authority", &"<redacted>")
            .finish()
    }
}

impl AllocatorRealmChildLaunch {
    fn validate(&self) -> Result<(), AllocatorConfigError> {
        if self.uid == 0 {
            return Err(AllocatorConfigError::InvalidChildIdentity);
        }
        validate_sorted_refs(
            &self.resource_refs,
            MAX_ALLOCATOR_LAUNCH_RESOURCE_REFS,
            AllocatorConfigError::TooManyResourceRefs,
            AllocatorConfigError::ResourceRefsNotStrictlySorted,
        )?;
        validate_sorted_refs(
            &self.lease_refs,
            MAX_ALLOCATOR_LAUNCH_RESOURCE_REFS,
            AllocatorConfigError::TooManyLeaseRefs,
            AllocatorConfigError::LeaseRefsNotStrictlySorted,
        )?;
        self.spawn.validate()
    }
}

fn validate_sorted_refs(
    refs: &[AllocatorLaunchRef],
    max: usize,
    too_many: AllocatorConfigError,
    not_sorted: AllocatorConfigError,
) -> Result<(), AllocatorConfigError> {
    if refs.len() > max {
        return Err(too_many);
    }
    if refs.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(not_sorted);
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorRealmChildNamespaces {
    pub user: AllocatorNamespaceAuthority,
    pub mount: AllocatorNamespaceAuthority,
    pub network: AllocatorNamespaceAuthority,
    pub ipc: AllocatorNamespaceAuthority,
    pub pid: AllocatorNamespaceAuthority,
    pub cgroup: AllocatorNamespaceAuthority,
}

impl fmt::Debug for AllocatorRealmChildNamespaces {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AllocatorRealmChildNamespaces(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorNamespaceAuthority {
    pub ref_id: AllocatorLaunchRef,
    pub digest: AllocatorLaunchDigest,
}

impl fmt::Debug for AllocatorNamespaceAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AllocatorNamespaceAuthority(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorRealmChildSpawnAuthority {
    pub clone3_with_pidfd: bool,
    pub direct_cgroup_placement: bool,
    pub no_new_privileges: bool,
    pub empty_initial_capabilities: bool,
    pub executable_only_argv: bool,
    pub closed_environment: bool,
    pub inherited_fd_authority_only: bool,
}

impl AllocatorRealmChildSpawnAuthority {
    fn validate(self) -> Result<(), AllocatorConfigError> {
        if self.clone3_with_pidfd
            && self.direct_cgroup_placement
            && self.no_new_privileges
            && self.empty_initial_capabilities
            && self.executable_only_argv
            && self.closed_environment
            && self.inherited_fd_authority_only
        {
            Ok(())
        } else {
            Err(AllocatorConfigError::UnsafeSpawnAuthority)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorRoot {
    pub enabled: bool,
    pub runtime_state: AllocatorRuntimeState,
    pub root_socket: PathTemplate,
    pub state_dir: PathTemplate,
    pub lease_ledger: PathTemplate,
    pub audit_dir: PathTemplate,
    pub runtime: AllocatorRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorRuntimeState {
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorRuntime {
    pub spawns_service: bool,
    pub socket_activated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorRealm {
    pub realm_name: String,
    pub realm_id: ContractId,
    pub realm_path: AllocatorRealmPath,
    pub enabled: bool,
    pub placement: RealmPlacement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_provider: Option<ContractId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_specific_placement: Option<String>,
    pub host_mutation: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmPlacement {
    HostLocal,
    GatewayVm,
    CloudFullHost,
    ProviderController,
    ProviderAgent,
    ProviderSpecific,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorResourceRequest {
    pub realm_path: AllocatorRealmPath,
    pub resource_id: ContractId,
    pub kind: AllocatorHostResourceKind,
    pub share: AllocatorResourceShare,
    pub acquisition_order: AllocatorAcquisitionOrder,
    pub source: AllocatorResourceSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorHostResourceKind {
    Bridge,
    Tap,
    VethPair,
    NftablesTable,
    NftablesPartition,
    CgroupSubtree,
    HostFilePartition,
    NamespaceBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorResourceShare {
    Exclusive,
    SharedPartition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorAcquisitionOrder {
    pub phase: u16,
    pub ordinal: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorResourceSource {
    pub kind: AllocatorResourceSourceKind,
    pub ref_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorResourceSourceKind {
    RealmPath,
    RealmSocket,
    RealmStateDir,
    RealmRunDir,
    RealmAuditDir,
    RealmNetwork,
    RealmBroker,
    EnvBridge,
    EnvNetVm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorPathPartition {
    pub realm_path: AllocatorRealmPath,
    pub state_dir: PathTemplate,
    pub run_dir: PathTemplate,
    pub audit_dir: PathTemplate,
    pub public_socket: PathTemplate,
    pub broker_socket: PathTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorProviderPlacement {
    pub realm_path: AllocatorRealmPath,
    pub provider_name: String,
    pub provider_id: ContractId,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AllocatorProviderKind>,
    pub placement: RealmPlacement,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorEnvBridge {
    pub realm_path: AllocatorRealmPath,
    pub env_name: String,
    pub declared: bool,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<AllocatorEnvBridgeMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_vm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_bridge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink_bridge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorInvariants {
    pub no_runtime_allocator_service: bool,
    pub preserves_env_runtime_source_of_truth: bool,
    pub private_metadata_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorEnvBridgeMode {
    None,
    InheritEnv,
    Declared,
    External,
}
