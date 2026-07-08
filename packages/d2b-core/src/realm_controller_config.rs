use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;

use crate::contract_id::{ContractId, ContractStringError, PathTemplate};

pub const REALM_CONTROLLERS_SCHEMA_VERSION: &str = "v2";
pub const MAX_REALM_CONTROLLER_REALM_PATH_BYTES: usize = 255;
pub const MAX_REALM_CONTROLLER_REALM_PATH_LABELS: usize = 16;
pub const MAX_REALM_CONTROLLER_PROVIDER_KIND_BYTES: usize = 64;

const REALM_CONTROLLER_REALM_PATH_PATTERN: &str = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*){0,15}$";
const REALM_CONTROLLER_PROVIDER_KIND_PATTERN: &str = "^[a-z][a-z0-9-]*$";

fn validate_realm_path(raw: String) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > MAX_REALM_CONTROLLER_REALM_PATH_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_REALM_CONTROLLER_REALM_PATH_BYTES,
        });
    }

    let labels = raw.split('.').collect::<Vec<_>>();
    if labels.is_empty() || labels.len() > MAX_REALM_CONTROLLER_REALM_PATH_LABELS {
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
    if raw.len() > MAX_REALM_CONTROLLER_PROVIDER_KIND_BYTES {
        return Err(ContractStringError::TooLong {
            max: MAX_REALM_CONTROLLER_PROVIDER_KIND_BYTES,
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

macro_rules! realm_controller_string {
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

realm_controller_string!(
    RealmControllerRealmPath,
    validate_realm_path,
    MAX_REALM_CONTROLLER_REALM_PATH_BYTES,
    REALM_CONTROLLER_REALM_PATH_PATTERN,
    "Realm path copied from d2b.realms, most-specific first, with 1-16 DNS-style labels and a 255-byte cap."
);

realm_controller_string!(
    RealmControllerProviderKind,
    validate_provider_kind,
    MAX_REALM_CONTROLLER_PROVIDER_KIND_BYTES,
    REALM_CONTROLLER_PROVIDER_KIND_PATTERN,
    "Opaque provider adapter kind slug used by realm-controller metadata."
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllersJson {
    pub schema_version: String,
    pub runtime_state: RealmControllerRuntimeState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controllers: Vec<RealmControllerConfig>,
    pub invariants: RealmControllerInvariants,
}

impl RealmControllersJson {
    pub fn validate_metadata_only(
        &self,
    ) -> Result<RealmControllerMetadataSummary, RealmControllerConfigError> {
        if self.schema_version != REALM_CONTROLLERS_SCHEMA_VERSION {
            return Err(RealmControllerConfigError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
            });
        }
        if self.runtime_state != RealmControllerRuntimeState::MetadataOnly {
            return Err(RealmControllerConfigError::UnsupportedRuntimeState);
        }
        self.invariants.validate()?;

        let mut summary = RealmControllerMetadataSummary::default();
        let allow_materialized_systemd_units = !self.invariants.no_systemd_units_materialized;
        for controller in &self.controllers {
            summary.controller_count += 1;
            if matches!(controller.placement, RealmControllerPlacement::HostLocal) {
                summary.host_local_controller_count += 1;
            }
            if controller.broker.enabled {
                summary.broker_enabled_count += 1;
            }
            controller.validate_metadata_only(allow_materialized_systemd_units)?;
        }
        Ok(summary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmControllerRuntimeState {
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerConfig {
    pub realm_name: String,
    pub realm_id: ContractId,
    pub realm_path: RealmControllerRealmPath,
    pub placement: RealmControllerPlacement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_placement: Option<RealmControllerProviderPlacement>,
    pub daemon: RealmDaemonConfig,
    pub broker: RealmBrokerConfig,
    pub paths: RealmControllerPaths,
    pub sockets: RealmControllerSockets,
    pub allocator: RealmAllocatorBinding,
    pub access: RealmControllerAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_runtime: Option<RealmControllerLocalRuntime>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<RealmControllerProvider>,
}

impl RealmControllerConfig {
    fn validate_metadata_only(
        &self,
        allow_materialized_systemd_units: bool,
    ) -> Result<(), RealmControllerConfigError> {
        let realm = self.realm_path.as_str().to_owned();
        if self.daemon.socket_activated {
            return Err(RealmControllerConfigError::MaterializedRuntime {
                realm,
                field: "daemon.socketActivated",
            });
        }
        if self.daemon.materialized_service && !allow_materialized_systemd_units {
            return Err(RealmControllerConfigError::MaterializedRuntime {
                realm,
                field: "daemon.materializedService",
            });
        }
        if self.broker.materialized_socket && !allow_materialized_systemd_units {
            return Err(RealmControllerConfigError::MaterializedRuntime {
                realm,
                field: "broker.materializedSocket",
            });
        }
        if self.broker.materialized_service && !allow_materialized_systemd_units {
            return Err(RealmControllerConfigError::MaterializedRuntime {
                realm,
                field: "broker.materializedService",
            });
        }
        if self.broker.socket_path.as_str() != self.sockets.broker_socket_path.as_str() {
            return Err(RealmControllerConfigError::InconsistentSocket {
                realm,
                field: "broker.socketPath",
                expected: self.sockets.broker_socket_path.as_str().to_owned(),
                found: self.broker.socket_path.as_str().to_owned(),
            });
        }
        if let Some(local_runtime) = &self.local_runtime {
            local_runtime.validate_metadata_only(&realm, self.placement)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmControllerPlacement {
    HostLocal,
    GatewayVm,
    CloudFullHost,
    ProviderController,
    ProviderAgent,
    ProviderSpecific,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerProviderPlacement {
    pub provider_name: String,
    pub provider_id: ContractId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RealmControllerProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_specific_placement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerLocalRuntime {
    pub runtime_state: RealmControllerRuntimeState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<RealmControllerRuntimeMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workloads: Vec<RealmControllerLocalWorkload>,
    pub invariants: RealmControllerLocalRuntimeInvariants,
}

impl RealmControllerLocalRuntime {
    fn validate_metadata_only(
        &self,
        realm: &str,
        placement: RealmControllerPlacement,
    ) -> Result<(), RealmControllerConfigError> {
        if placement != RealmControllerPlacement::HostLocal {
            return Err(RealmControllerConfigError::LocalRuntimeForNonHostLocal {
                realm: realm.to_owned(),
                placement,
            });
        }
        if self.runtime_state != RealmControllerRuntimeState::MetadataOnly {
            return Err(RealmControllerConfigError::UnsupportedLocalRuntimeState {
                realm: realm.to_owned(),
            });
        }
        self.invariants.validate(realm)?;

        let provider_ids = self
            .providers
            .iter()
            .map(|provider| provider.provider.id.as_str())
            .collect::<BTreeSet<_>>();
        for workload in &self.workloads {
            if !provider_ids.contains(workload.runtime.provider.id.as_str()) {
                return Err(RealmControllerConfigError::MissingLocalRuntimeProvider {
                    realm: realm.to_owned(),
                    workload: workload.workload_id.as_str().to_owned(),
                    provider_id: workload.runtime.provider.id.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerRuntimeMetadata {
    pub kind: ContractId,
    pub provider: RealmControllerRuntimeProviderRef,
    pub capabilities: RealmControllerRuntimeCapabilities,
    pub operation_capabilities: RealmControllerRuntimeOperationCapabilities,
    pub autostart_policy: ContractId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<RealmControllerRuntimeServiceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerRuntimeProviderRef {
    pub id: ContractId,
    pub driver: ContractId,
    #[serde(rename = "type")]
    pub provider_type: RealmControllerRuntimeProviderType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmControllerRuntimeProviderType {
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerRuntimeCapabilities {
    pub lifecycle: bool,
    pub display: bool,
    pub usb_hotplug: bool,
    pub guest_control: bool,
    pub exec: bool,
    pub config_sync: bool,
    pub ssh: bool,
    pub store_sync: bool,
    pub keys: bool,
    pub in_guest_observability: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerRuntimeOperationCapabilities {
    pub lifecycle: RealmControllerLifecycleOperationCapabilities,
    pub media: RealmControllerMediaOperationCapabilities,
    pub display: RealmControllerDisplayOperationCapabilities,
    pub guest: RealmControllerGuestOperationCapabilities,
    pub storage: RealmControllerStorageOperationCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerLifecycleOperationCapabilities {
    pub start: bool,
    pub stop: bool,
    pub restart: bool,
    pub switch: bool,
    pub host_prepare: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerMediaOperationCapabilities {
    pub usb_hotplug: bool,
    pub removable_media: bool,
    pub qemu_media: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerDisplayOperationCapabilities {
    pub display: bool,
    pub graphics: bool,
    pub video: bool,
    pub wayland_proxy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerGuestOperationCapabilities {
    pub guest_control: bool,
    pub exec: bool,
    pub shell: bool,
    pub config_sync: bool,
    pub ssh: bool,
    pub keys: bool,
    pub in_guest_observability: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerStorageOperationCapabilities {
    pub store_sync: bool,
    pub virtiofs: bool,
    pub volumes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerRuntimeServiceSummary {
    pub id: ContractId,
    pub role: ContractId,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerLocalWorkload {
    pub workload_id: ContractId,
    pub vm_name: ContractId,
    pub env: ContractId,
    pub runtime: RealmControllerRuntimeMetadata,
    pub paths: RealmControllerLocalWorkloadPaths,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerLocalWorkloadPaths {
    pub state_dir: PathTemplate,
    pub run_dir: PathTemplate,
    pub store_view: PathTemplate,
    pub guest_control_dir: PathTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerLocalRuntimeInvariants {
    pub metadata_only: bool,
    pub existing_global_vm_paths_preserved: bool,
    pub no_state_migration_during_activation: bool,
    pub broker_effects_remain_realm_delegated: bool,
}

impl RealmControllerLocalRuntimeInvariants {
    fn validate(&self, realm: &str) -> Result<(), RealmControllerConfigError> {
        if !self.metadata_only {
            return Err(RealmControllerConfigError::LocalRuntimeInvariantDisabled {
                realm: realm.to_owned(),
                field: "metadataOnly",
            });
        }
        if !self.existing_global_vm_paths_preserved {
            return Err(RealmControllerConfigError::LocalRuntimeInvariantDisabled {
                realm: realm.to_owned(),
                field: "existingGlobalVmPathsPreserved",
            });
        }
        if !self.no_state_migration_during_activation {
            return Err(RealmControllerConfigError::LocalRuntimeInvariantDisabled {
                realm: realm.to_owned(),
                field: "noStateMigrationDuringActivation",
            });
        }
        if !self.broker_effects_remain_realm_delegated {
            return Err(RealmControllerConfigError::LocalRuntimeInvariantDisabled {
                realm: realm.to_owned(),
                field: "brokerEffectsRemainRealmDelegated",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmDaemonConfig {
    pub user: ContractId,
    pub group: ContractId,
    pub public_socket_group: ContractId,
    pub service_name: ContractId,
    pub config_path: PathTemplate,
    pub state_lock_path: PathTemplate,
    pub locks_dir: PathTemplate,
    pub socket_activated: bool,
    pub materialized_service: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmBrokerConfig {
    pub enabled: bool,
    pub host_mutation: bool,
    pub user: ContractId,
    pub group: ContractId,
    pub socket_path: PathTemplate,
    pub socket_unit_name: ContractId,
    pub service_unit_name: ContractId,
    pub audit_dir: PathTemplate,
    pub materialized_socket: bool,
    pub materialized_service: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerPaths {
    pub run_dir: PathTemplate,
    pub state_dir: PathTemplate,
    pub audit_dir: PathTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerSockets {
    pub public_socket_path: PathTemplate,
    pub broker_socket_path: PathTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAllocatorBinding {
    pub kind: RealmAllocatorBindingKind,
    pub config_path: PathTemplate,
    pub root_socket: PathTemplate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_request_refs: Vec<ContractId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmAllocatorBindingKind {
    LocalRootMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerAccess {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_users: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherited_admin_users: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerProvider {
    pub provider_name: String,
    pub provider_id: ContractId,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RealmControllerProviderKind>,
    pub placement: RealmControllerPlacement,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmControllerInvariants {
    pub metadata_only: bool,
    pub no_systemd_units_materialized: bool,
    pub preserves_global_daemon_behavior: bool,
    pub preserves_direct_unix_socket_semantics: bool,
}

impl RealmControllerInvariants {
    fn validate(&self) -> Result<(), RealmControllerConfigError> {
        if !self.metadata_only {
            return Err(RealmControllerConfigError::InvariantDisabled(
                "metadataOnly",
            ));
        }
        if !self.preserves_global_daemon_behavior {
            return Err(RealmControllerConfigError::InvariantDisabled(
                "preservesGlobalDaemonBehavior",
            ));
        }
        if !self.preserves_direct_unix_socket_semantics {
            return Err(RealmControllerConfigError::InvariantDisabled(
                "preservesDirectUnixSocketSemantics",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealmControllerMetadataSummary {
    pub controller_count: usize,
    pub host_local_controller_count: usize,
    pub broker_enabled_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmControllerConfigError {
    UnsupportedSchemaVersion {
        found: String,
    },
    UnsupportedRuntimeState,
    InvariantDisabled(&'static str),
    MaterializedRuntime {
        realm: String,
        field: &'static str,
    },
    InconsistentSocket {
        realm: String,
        field: &'static str,
        expected: String,
        found: String,
    },
    LocalRuntimeForNonHostLocal {
        realm: String,
        placement: RealmControllerPlacement,
    },
    UnsupportedLocalRuntimeState {
        realm: String,
    },
    LocalRuntimeInvariantDisabled {
        realm: String,
        field: &'static str,
    },
    MissingLocalRuntimeProvider {
        realm: String,
        workload: String,
        provider_id: String,
    },
}

impl std::fmt::Display for RealmControllerConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { found } => {
                write!(
                    f,
                    "unsupported realm controller schemaVersion {found:?}; expected {REALM_CONTROLLERS_SCHEMA_VERSION:?}"
                )
            }
            Self::UnsupportedRuntimeState => {
                f.write_str("unsupported realm controller runtimeState; expected metadata-only")
            }
            Self::InvariantDisabled(field) => {
                write!(f, "realm controller invariant {field} must be true")
            }
            Self::MaterializedRuntime { realm, field } => {
                write!(
                    f,
                    "realm controller {realm} field {field} must remain false while runtimeState is metadata-only"
                )
            }
            Self::InconsistentSocket {
                realm,
                field,
                expected,
                found,
            } => {
                write!(
                    f,
                    "realm controller {realm} field {field} is {found:?}; expected {expected:?}"
                )
            }
            Self::LocalRuntimeForNonHostLocal { realm, placement } => {
                write!(
                    f,
                    "realm controller {realm} has localRuntime metadata but placement is {placement:?}; expected host-local"
                )
            }
            Self::UnsupportedLocalRuntimeState { realm } => {
                write!(
                    f,
                    "realm controller {realm} localRuntime.runtimeState must remain metadata-only"
                )
            }
            Self::LocalRuntimeInvariantDisabled { realm, field } => {
                write!(
                    f,
                    "realm controller {realm} localRuntime invariant {field} must be true"
                )
            }
            Self::MissingLocalRuntimeProvider {
                realm,
                workload,
                provider_id,
            } => {
                write!(
                    f,
                    "realm controller {realm} localRuntime workload {workload} references undeclared provider {provider_id}"
                )
            }
        }
    }
}

impl std::error::Error for RealmControllerConfigError {}
