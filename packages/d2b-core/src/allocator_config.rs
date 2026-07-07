use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

use crate::contract_id::{ContractId, ContractStringError, PathTemplate};

pub const MAX_ALLOCATOR_REALM_PATH_BYTES: usize = 255;
pub const MAX_ALLOCATOR_REALM_PATH_LABELS: usize = 16;
pub const MAX_ALLOCATOR_PROVIDER_KIND_BYTES: usize = 64;

const ALLOCATOR_REALM_PATH_PATTERN: &str = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*){0,15}$";
const ALLOCATOR_PROVIDER_KIND_PATTERN: &str = "^[a-z][a-z0-9-]*$";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    pub invariants: AllocatorInvariants,
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
