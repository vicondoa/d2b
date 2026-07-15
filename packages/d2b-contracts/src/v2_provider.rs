//! Strict d2b 2.0 provider contracts.
//!
//! This module is the single serialized contract rail shared by trusted
//! in-process implementations and ComponentSession provider-agent proxies.
//! Values are bounded, closed, and opaque: provider SDK responses, credentials,
//! command arguments, and host paths have no representation here.

use std::{collections::BTreeSet, error::Error, fmt, future::Future, pin::Pin};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType, RealmId, RoleId, WorkloadId},
};

pub const PROVIDER_SCHEMA_VERSION: u32 = 2;
pub const PROVIDER_API_MAJOR: u16 = 2;
pub const PROVIDER_API_MINOR: u16 = 0;
pub const MAX_PROVIDER_CAPABILITIES: usize = 64;
pub const MAX_PROVIDER_REGISTRY_ENTRIES: usize = 256;
pub const MAX_PROVIDER_PLAN_RESOURCES: usize = 32;
pub const MAX_CREDENTIAL_OPERATION_CLASSES: usize = 32;
pub const MAX_PROVIDER_REQUEST_LIFETIME_MS: u64 = 15 * 60 * 1_000;
pub const MAX_PROVIDER_LEASE_LIFETIME_MS: u64 = 60 * 60 * 1_000;
pub const MAX_PROVIDER_DRAIN_MS: u32 = 5 * 60 * 1_000;
pub const MAX_OBSERVABILITY_QUERY_LIMIT: u16 = 256;
pub const MAX_OBSERVABILITY_EXPORT_RANGE_MS: u64 = 31 * 24 * 60 * 60 * 1_000;
pub const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
pub const PROVIDER_CONTRACT_FINGERPRINT: &str =
    "91e665314ffbc0fbcc2d4f3bc788dd1d7f4d694382fa2795a47e877eb4ac9b57";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderContractError {
    BoundExceeded,
    InvalidIdentifier,
    InvalidFingerprint,
    InvalidGeneration,
    UnsupportedSchemaVersion,
    UnsupportedApiVersion,
    ProviderTypeMismatch,
    CapabilityMismatch,
    MissingRequiredCapability,
    DuplicateCapability,
    PlacementMismatch,
    ScopeMismatch,
    OperationBindingMismatch,
    OperationInputMismatch,
    RequestExpired,
    RequestLifetimeExceeded,
    InvalidTimeRange,
    InvalidTransition,
    HandleBindingMismatch,
    OwnershipTransferInvalid,
    AdoptionAmbiguous,
    AdoptionEvidenceMismatch,
    RegistryNotCanonical,
    DuplicateProvider,
    DuplicateFactory,
    RegistryGenerationMismatch,
    RegistryFingerprintMismatch,
    UnknownProvider,
    NoEligibleProvider,
    LeaseNotColocated,
    LeaseExpired,
    LeaseRevoked,
    LeaseTransferForbidden,
    ContractFingerprintMismatch,
}

impl fmt::Display for ProviderContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BoundExceeded => "provider contract bound exceeded",
            Self::InvalidIdentifier => "invalid bounded provider identifier",
            Self::InvalidFingerprint => "invalid provider fingerprint",
            Self::InvalidGeneration => "invalid provider generation",
            Self::UnsupportedSchemaVersion => "unsupported provider schema version",
            Self::UnsupportedApiVersion => "unsupported provider API version",
            Self::ProviderTypeMismatch => "provider authority type mismatch",
            Self::CapabilityMismatch => "provider capability or method mismatch",
            Self::MissingRequiredCapability => "required provider capability is absent",
            Self::DuplicateCapability => "duplicate provider capability",
            Self::PlacementMismatch => "provider placement metadata mismatch",
            Self::ScopeMismatch => "provider operation scope mismatch",
            Self::OperationBindingMismatch => "provider operation binding mismatch",
            Self::OperationInputMismatch => "provider operation input mismatch",
            Self::RequestExpired => "provider request or lease expired",
            Self::RequestLifetimeExceeded => "provider request lifetime exceeded",
            Self::InvalidTimeRange => "provider observability time range is invalid",
            Self::InvalidTransition => "invalid provider lifecycle transition",
            Self::HandleBindingMismatch => "provider handle binding mismatch",
            Self::OwnershipTransferInvalid => "provider ownership transfer is invalid",
            Self::AdoptionAmbiguous => "provider adoption is ambiguous",
            Self::AdoptionEvidenceMismatch => "provider adoption evidence mismatch",
            Self::RegistryNotCanonical => "provider registry is not canonical",
            Self::DuplicateProvider => "duplicate provider ID",
            Self::DuplicateFactory => "duplicate provider factory key",
            Self::RegistryGenerationMismatch => "provider registry generation mismatch",
            Self::RegistryFingerprintMismatch => "provider registry fingerprint mismatch",
            Self::UnknownProvider => "unknown provider",
            Self::NoEligibleProvider => "no eligible provider",
            Self::LeaseNotColocated => "credential provider and consumer are not co-located",
            Self::LeaseExpired => "credential lease expired",
            Self::LeaseRevoked => "credential lease revoked",
            Self::LeaseTransferForbidden => "credential lease transfer is forbidden",
            Self::ContractFingerprintMismatch => "provider contract fingerprint mismatch",
        })
    }
}

impl Error for ProviderContractError {}

fn valid_bounded_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

macro_rules! bounded_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(#[schemars(regex(pattern = "^[a-z][a-z0-9-]{0,63}$"))] String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, ProviderContractError> {
                let value = value.into();
                if valid_bounded_id(&value) {
                    Ok(Self(value))
                } else {
                    Err(ProviderContractError::InvalidIdentifier)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&"<redacted>")
                    .finish()
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

bounded_id!(
    ImplementationId,
    "A canonical provider implementation identifier."
);
bounded_id!(OperationId, "An opaque provider operation identifier.");
bounded_id!(IdempotencyKey, "An opaque mutation idempotency key.");
bounded_id!(
    PrincipalRef,
    "An opaque already-authorized principal reference."
);
bounded_id!(CorrelationId, "A bounded audit correlation identifier.");
bounded_id!(PlanId, "An opaque provider plan identifier.");
bounded_id!(HandleId, "An opaque provider-owned resource handle.");
bounded_id!(
    LeaseId,
    "An opaque credential lease identifier meaningful only to its agent."
);
bounded_id!(TransferId, "An opaque ownership-transfer identifier.");
bounded_id!(SourceVersion, "A non-secret credential source version.");
bounded_id!(
    ConfiguredItemId,
    "A bounded configured runtime item identifier."
);
bounded_id!(
    TransportBindingId,
    "An opaque transport binding identifier."
);
bounded_id!(StorageSnapshotId, "An opaque storage snapshot identifier.");
bounded_id!(DeviceSelectorId, "A bounded configured device selector.");
bounded_id!(
    ObservabilityCursor,
    "An opaque bounded observability pagination cursor."
);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Fingerprint(#[schemars(regex(pattern = "^[0-9a-f]{64}$"))] String);

impl Fingerprint {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProviderContractError> {
        let value = value.into();
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value))
        } else {
            Err(ProviderContractError::InvalidFingerprint)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Fingerprint")
            .field(&"<redacted>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Generation(u64);

impl Generation {
    pub fn new(value: u64) -> Result<Self, ProviderContractError> {
        if value == 0 || value > MAX_SAFE_JSON_INTEGER {
            Err(ProviderContractError::InvalidGeneration)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, ProviderContractError> {
        self.0
            .checked_add(1)
            .ok_or(ProviderContractError::InvalidGeneration)
            .and_then(Self::new)
    }
}

impl<'de> Deserialize<'de> for Generation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProviderApiVersion {
    pub const V2: Self = Self {
        major: PROVIDER_API_MAJOR,
        minor: PROVIDER_API_MINOR,
    };

    pub fn validate(self) -> Result<(), ProviderContractError> {
        if self == Self::V2 {
            Ok(())
        } else {
            Err(ProviderContractError::UnsupportedApiVersion)
        }
    }
}

macro_rules! provider_methods {
    ($($variant:ident => ($wire:literal, $axis:ident, $required:literal)),+ $(,)?) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize, JsonSchema,
        )]
        pub enum ProviderMethod {
            $(
                #[serde(rename = $wire)]
                #[schemars(rename = $wire)]
                $variant,
            )+
        }

        impl ProviderMethod {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }

            pub const fn provider_type(self) -> ProviderType {
                match self {
                    $(Self::$variant => ProviderType::$axis,)+
                }
            }

            pub const fn required(self) -> bool {
                match self {
                    $(Self::$variant => $required,)+
                }
            }
        }
    };
}

provider_methods! {
    RuntimePlan => ("runtime.plan", Runtime, true),
    RuntimeEnsure => ("runtime.ensure", Runtime, true),
    RuntimeStart => ("runtime.start", Runtime, true),
    RuntimeStop => ("runtime.stop", Runtime, true),
    RuntimeExecute => ("runtime.execute", Runtime, false),
    RuntimeInspect => ("runtime.inspect", Runtime, true),
    RuntimeAdopt => ("runtime.adopt", Runtime, true),
    RuntimeDestroy => ("runtime.destroy", Runtime, true),
    InfrastructurePlan => ("infrastructure.plan", Infrastructure, true),
    InfrastructureApply => ("infrastructure.apply", Infrastructure, true),
    InfrastructureSetPowerState => ("infrastructure.set-power-state", Infrastructure, true),
    InfrastructureInspect => ("infrastructure.inspect", Infrastructure, true),
    InfrastructureAdopt => ("infrastructure.adopt", Infrastructure, true),
    InfrastructureBootstrapBinding => ("infrastructure.bootstrap-binding", Infrastructure, true),
    InfrastructureDestroy => ("infrastructure.destroy", Infrastructure, true),
    TransportConnect => ("transport.connect", Transport, true),
    TransportListen => ("transport.listen", Transport, false),
    TransportIssueBinding => ("transport.issue-binding", Transport, false),
    TransportRevokeBinding => ("transport.revoke-binding", Transport, true),
    TransportInspect => ("transport.inspect", Transport, true),
    SubstrateCheck => ("substrate.check", Substrate, true),
    SubstratePlanRemediation => ("substrate.plan-remediation", Substrate, true),
    SubstrateApply => ("substrate.apply", Substrate, true),
    CredentialStatus => ("credential.status", Credential, true),
    CredentialAcquireLease => ("credential.acquire-lease", Credential, true),
    CredentialRefreshLease => ("credential.refresh-lease", Credential, true),
    CredentialRevokeLease => ("credential.revoke-lease", Credential, true),
    DisplayOpen => ("display.open", Display, true),
    DisplayInspect => ("display.inspect", Display, true),
    DisplayAdopt => ("display.adopt", Display, true),
    DisplayClose => ("display.close", Display, true),
    NetworkPlan => ("network.plan", Network, true),
    NetworkEnsure => ("network.ensure", Network, true),
    NetworkInspect => ("network.inspect", Network, true),
    NetworkAdopt => ("network.adopt", Network, true),
    NetworkRelease => ("network.release", Network, true),
    StoragePlan => ("storage.plan", Storage, true),
    StorageEnsure => ("storage.ensure", Storage, true),
    StorageInspect => ("storage.inspect", Storage, true),
    StorageAdopt => ("storage.adopt", Storage, true),
    StorageSnapshot => ("storage.snapshot", Storage, false),
    StorageDestroy => ("storage.destroy", Storage, true),
    DevicePlanAttach => ("device.plan-attach", Device, true),
    DeviceAttach => ("device.attach", Device, true),
    DeviceInspect => ("device.inspect", Device, true),
    DeviceAdopt => ("device.adopt", Device, true),
    DeviceDetach => ("device.detach", Device, true),
    AudioOpen => ("audio.open", Audio, true),
    AudioSetState => ("audio.set-state", Audio, true),
    AudioInspect => ("audio.inspect", Audio, true),
    AudioAdopt => ("audio.adopt", Audio, true),
    AudioClose => ("audio.close", Audio, true),
    ObservabilityStatus => ("observability.status", Observability, true),
    ObservabilityQuery => ("observability.query", Observability, true),
    ObservabilitySubscribe => ("observability.subscribe", Observability, false),
    ObservabilityExport => ("observability.export", Observability, true),
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ProviderCapability(pub ProviderMethod);

impl ProviderCapability {
    pub const fn provider_type(self) -> ProviderType {
        self.0.provider_type()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProviderCapabilitySet(Vec<ProviderCapability>);

impl ProviderCapabilitySet {
    pub fn new(mut capabilities: Vec<ProviderCapability>) -> Result<Self, ProviderContractError> {
        if capabilities.is_empty() || capabilities.len() > MAX_PROVIDER_CAPABILITIES {
            return Err(ProviderContractError::BoundExceeded);
        }
        capabilities.sort_unstable();
        if capabilities.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ProviderContractError::DuplicateCapability);
        }
        Ok(Self(capabilities))
    }

    pub fn as_slice(&self) -> &[ProviderCapability] {
        &self.0
    }

    pub fn contains_method(&self, method: ProviderMethod) -> bool {
        self.0.binary_search(&ProviderCapability(method)).is_ok()
    }

    pub fn validate_for(&self, provider_type: ProviderType) -> Result<(), ProviderContractError> {
        if self
            .0
            .iter()
            .any(|capability| capability.provider_type() != provider_type)
        {
            return Err(ProviderContractError::ProviderTypeMismatch);
        }
        if ProviderMethod::ALL.iter().any(|method| {
            method.provider_type() == provider_type
                && method.required()
                && !self.contains_method(*method)
        }) {
            return Err(ProviderContractError::MissingRequiredCapability);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ProviderCapabilitySet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<ProviderCapability>::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ProviderCapabilitySet {
    fn schema_name() -> String {
        "ProviderCapabilitySet".to_owned()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        <BoundedVec<ProviderCapability, 1, MAX_PROVIDER_CAPABILITIES>>::json_schema(generator)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkPosture {
    HostShared,
    None,
    IsolatedNamespace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UserNamespacePosture {
    BrokerPreestablished,
    UnprivilegedSelfManaged,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessAuthority {
    ProviderOwnedPidfd,
    VerifiedSystemdUserScope,
    ProviderManagedRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CgroupAuthority {
    RealmDelegatedLeaf,
    VerifiedSystemdUserScope,
    ProviderManagedRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PersistentIdentityPosture {
    None,
    FileBackedCloneable,
    NonCopyableAttested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceMediationPosture {
    None,
    BrokerDelegatedTyped,
    ProviderManagedTyped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAuthorityPosture {
    pub process: ProcessAuthority,
    pub cgroup: CgroupAuthority,
    pub network: NetworkPosture,
    pub user_namespace: UserNamespacePosture,
    pub persistent_identity: PersistentIdentityPosture,
    pub device_mediation: DeviceMediationPosture,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderAuthority {
    Runtime { posture: RuntimeAuthorityPosture },
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

impl ProviderAuthority {
    pub const ALL_TYPES: [ProviderType; 11] = ProviderType::ALL;

    pub const fn provider_type(&self) -> ProviderType {
        match self {
            Self::Runtime { .. } => ProviderType::Runtime,
            Self::Infrastructure => ProviderType::Infrastructure,
            Self::Transport => ProviderType::Transport,
            Self::Substrate => ProviderType::Substrate,
            Self::Credential => ProviderType::Credential,
            Self::Display => ProviderType::Display,
            Self::Network => ProviderType::Network,
            Self::Storage => ProviderType::Storage,
            Self::Device => ProviderType::Device,
            Self::Audio => ProviderType::Audio,
            Self::Observability => ProviderType::Observability,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderPlacement {
    TrustedFirstPartyInProcess {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "controllerRole")]
        controller_role: EndpointRole,
    },
    ProviderAgent {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
        #[serde(rename = "endpointRole")]
        endpoint_role: EndpointRole,
        service: ServicePackage,
        #[serde(rename = "agentGeneration")]
        agent_generation: Generation,
    },
    UserAgent {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
        #[serde(rename = "endpointRole")]
        endpoint_role: EndpointRole,
        service: ServicePackage,
        #[serde(rename = "agentGeneration")]
        agent_generation: Generation,
    },
}

impl fmt::Debug for ProviderPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrustedFirstPartyInProcess {
                controller_role, ..
            } => formatter
                .debug_struct("TrustedFirstPartyInProcess")
                .field("controller_role", controller_role)
                .finish_non_exhaustive(),
            Self::ProviderAgent {
                endpoint_role,
                service,
                agent_generation,
                ..
            } => formatter
                .debug_struct("ProviderAgent")
                .field("endpoint_role", endpoint_role)
                .field("service", service)
                .field("agent_generation", agent_generation)
                .finish_non_exhaustive(),
            Self::UserAgent {
                endpoint_role,
                service,
                agent_generation,
                ..
            } => formatter
                .debug_struct("UserAgent")
                .field("endpoint_role", endpoint_role)
                .field("service", service)
                .field("agent_generation", agent_generation)
                .finish_non_exhaustive(),
        }
    }
}

impl ProviderPlacement {
    pub fn realm_id(&self) -> &RealmId {
        match self {
            Self::TrustedFirstPartyInProcess { realm_id, .. }
            | Self::ProviderAgent { realm_id, .. }
            | Self::UserAgent { realm_id, .. } => realm_id,
        }
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        match self {
            Self::TrustedFirstPartyInProcess {
                controller_role: EndpointRole::LocalRootController | EndpointRole::RealmController,
                ..
            } => Ok(()),
            Self::ProviderAgent {
                endpoint_role,
                service,
                ..
            } if *endpoint_role == EndpointRole::ProviderAgent
                && *service == ServicePackage::ProviderV2 =>
            {
                Ok(())
            }
            Self::UserAgent {
                endpoint_role,
                service,
                ..
            } if *endpoint_role == EndpointRole::UserAgent
                && *service == ServicePackage::UserV2 =>
            {
                Ok(())
            }
            _ => Err(ProviderContractError::PlacementMismatch),
        }
    }

    pub fn agent_binding(&self) -> Option<AgentPlacementBinding> {
        match self {
            Self::ProviderAgent {
                realm_id,
                workload_id,
                role_id,
                agent_generation,
                ..
            } => Some(AgentPlacementBinding {
                realm_id: realm_id.clone(),
                workload_id: workload_id.clone(),
                role_id: role_id.clone(),
                agent_generation: *agent_generation,
            }),
            Self::TrustedFirstPartyInProcess { .. } | Self::UserAgent { .. } => None,
        }
    }

    pub fn credential_binding(&self) -> Option<CredentialPlacementBinding> {
        match self {
            Self::ProviderAgent {
                realm_id,
                workload_id,
                role_id,
                agent_generation,
                ..
            } => Some(CredentialPlacementBinding::ProviderAgent {
                binding: AgentPlacementBinding {
                    realm_id: realm_id.clone(),
                    workload_id: workload_id.clone(),
                    role_id: role_id.clone(),
                    agent_generation: *agent_generation,
                },
            }),
            Self::UserAgent {
                realm_id,
                role_id,
                agent_generation,
                ..
            } => Some(CredentialPlacementBinding::UserAgent {
                realm_id: realm_id.clone(),
                role_id: role_id.clone(),
                agent_generation: *agent_generation,
            }),
            Self::TrustedFirstPartyInProcess { .. } => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPlacementBinding {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub role_id: RoleId,
    pub agent_generation: Generation,
}

impl fmt::Debug for AgentPlacementBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentPlacementBinding")
            .field("agent_generation", &self.agent_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CredentialPlacementBinding {
    ProviderAgent {
        binding: AgentPlacementBinding,
    },
    UserAgent {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
        #[serde(rename = "agentGeneration")]
        agent_generation: Generation,
    },
}

impl fmt::Debug for CredentialPlacementBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderAgent { binding } => formatter
                .debug_struct("CredentialPlacementBinding::ProviderAgent")
                .field("agent_generation", &binding.agent_generation)
                .finish_non_exhaustive(),
            Self::UserAgent {
                agent_generation, ..
            } => formatter
                .debug_struct("CredentialPlacementBinding::UserAgent")
                .field("agent_generation", agent_generation)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDescriptor {
    pub schema_version: u32,
    pub provider_id: ProviderId,
    pub authority: ProviderAuthority,
    pub implementation_id: ImplementationId,
    pub api_version: ProviderApiVersion,
    pub capabilities: ProviderCapabilitySet,
    pub configuration_schema_fingerprint: Fingerprint,
    pub configured_scope_digest: Fingerprint,
    pub registry_generation: Generation,
    pub placement: ProviderPlacement,
}

impl fmt::Debug for ProviderDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderDescriptor")
            .field("provider_type", &self.authority.provider_type())
            .field("implementation_id", &self.implementation_id)
            .field("registry_generation", &self.registry_generation)
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}

impl ProviderDescriptor {
    pub fn provider_type(&self) -> ProviderType {
        self.authority.provider_type()
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION {
            return Err(ProviderContractError::UnsupportedSchemaVersion);
        }
        self.api_version.validate()?;
        self.placement.validate()?;
        self.capabilities.validate_for(self.provider_type())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderHealthState {
    Healthy,
    Degraded,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderHealthReason {
    None,
    ProviderDegraded,
    HealthTimeout,
    HealthStale,
    SessionDisconnected,
    QueuePressure,
    HandshakeTimeout,
    AuthenticationFailed,
    IdentityMismatch,
    ConfigurationMismatch,
    GenerationMismatch,
    CapabilityMismatch,
    AdoptionAmbiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderRemediation {
    None,
    RetryBounded,
    InspectProvider,
    RestartAgent,
    ReEnrollPeer,
    RepairConfiguration,
    ReplaceGeneration,
    OperatorInteraction,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderHealth {
    pub provider_id: ProviderId,
    pub registry_generation: Generation,
    pub observed_at_unix_ms: u64,
    pub state: ProviderHealthState,
    pub reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl fmt::Debug for ProviderHealth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHealth")
            .field("registry_generation", &self.registry_generation)
            .field("state", &self.state)
            .field("reason", &self.reason)
            .field("remediation", &self.remediation)
            .finish_non_exhaustive()
    }
}

impl ProviderHealth {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.observed_at_unix_ms > MAX_SAFE_JSON_INTEGER {
            return Err(ProviderContractError::BoundExceeded);
        }
        let valid = match self.state {
            ProviderHealthState::Healthy => {
                self.reason == ProviderHealthReason::None
                    && self.remediation == ProviderRemediation::None
            }
            ProviderHealthState::Degraded => {
                self.reason != ProviderHealthReason::None
                    && self.remediation != ProviderRemediation::None
            }
            ProviderHealthState::Unavailable => {
                matches!(
                    self.reason,
                    ProviderHealthReason::HealthTimeout
                        | ProviderHealthReason::HealthStale
                        | ProviderHealthReason::SessionDisconnected
                        | ProviderHealthReason::HandshakeTimeout
                ) && self.remediation != ProviderRemediation::None
            }
            ProviderHealthState::Failed => {
                matches!(
                    self.reason,
                    ProviderHealthReason::AuthenticationFailed
                        | ProviderHealthReason::IdentityMismatch
                        | ProviderHealthReason::ConfigurationMismatch
                        | ProviderHealthReason::GenerationMismatch
                        | ProviderHealthReason::CapabilityMismatch
                        | ProviderHealthReason::AdoptionAmbiguous
                ) && matches!(
                    self.remediation,
                    ProviderRemediation::ReEnrollPeer
                        | ProviderRemediation::RepairConfiguration
                        | ProviderRemediation::ReplaceGeneration
                        | ProviderRemediation::OperatorInteraction
                )
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ProviderContractError::InvalidTransition)
        }
    }

    pub fn admits_operations(&self) -> bool {
        matches!(
            self.state,
            ProviderHealthState::Healthy | ProviderHealthState::Degraded
        )
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuthorizedProviderScope {
    Realm {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    Workload {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
    },
    WorkloadRole {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
    },
}

impl AuthorizedProviderScope {
    pub fn realm_id(&self) -> &RealmId {
        match self {
            Self::Realm { realm_id }
            | Self::Workload { realm_id, .. }
            | Self::WorkloadRole { realm_id, .. } => realm_id,
        }
    }

    pub fn workload_id(&self) -> Option<&WorkloadId> {
        match self {
            Self::Workload { workload_id, .. } | Self::WorkloadRole { workload_id, .. } => {
                Some(workload_id)
            }
            Self::Realm { .. } => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderOperationContext {
    pub schema_version: u32,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub request_digest: Fingerprint,
    pub scope: AuthorizedProviderScope,
    pub principal: PrincipalRef,
    pub provider_id: ProviderId,
    pub provider_type: ProviderType,
    pub provider_generation: Generation,
    pub capability: ProviderCapability,
    pub method: ProviderMethod,
    pub policy_epoch: Generation,
    pub authorization_decision_digest: Fingerprint,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub correlation_id: CorrelationId,
    pub trace_id: Fingerprint,
}

impl fmt::Debug for ProviderOperationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderOperationContext")
            .field("provider_type", &self.provider_type)
            .field("capability", &self.capability)
            .field("method", &self.method)
            .field("provider_generation", &self.provider_generation)
            .finish_non_exhaustive()
    }
}

impl ProviderOperationContext {
    pub fn validate(
        &self,
        descriptor: &ProviderDescriptor,
        now_unix_ms: u64,
    ) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION {
            return Err(ProviderContractError::UnsupportedSchemaVersion);
        }
        descriptor.validate()?;
        if self.provider_id != descriptor.provider_id
            || self.provider_type != descriptor.provider_type()
            || self.provider_generation != descriptor.registry_generation
        {
            return Err(ProviderContractError::OperationBindingMismatch);
        }
        if self.method.provider_type() != self.provider_type
            || self.capability.0 != self.method
            || !descriptor.capabilities.contains_method(self.method)
        {
            return Err(ProviderContractError::CapabilityMismatch);
        }
        if self.scope.realm_id() != descriptor.placement.realm_id() {
            return Err(ProviderContractError::ScopeMismatch);
        }
        if self.issued_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.expires_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.expires_at_unix_ms <= self.issued_at_unix_ms
            || self.expires_at_unix_ms - self.issued_at_unix_ms > MAX_PROVIDER_REQUEST_LIFETIME_MS
        {
            return Err(ProviderContractError::RequestLifetimeExceeded);
        }
        if now_unix_ms >= self.expires_at_unix_ms {
            return Err(ProviderContractError::RequestExpired);
        }
        Ok(())
    }

    pub fn binding(&self) -> OperationBinding {
        OperationBinding {
            operation_id: self.operation_id.clone(),
            idempotency_key: self.idempotency_key.clone(),
            request_digest: self.request_digest.clone(),
            provider_id: self.provider_id.clone(),
            provider_generation: self.provider_generation,
        }
    }
}

pub struct ProviderCallContext<'a> {
    pub operation: &'a ProviderOperationContext,
    pub peer_role: EndpointRole,
    pub service: ServicePackage,
    pub monotonic_deadline_remaining_ms: u32,
    pub cancelled: bool,
}

impl fmt::Debug for ProviderCallContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCallContext")
            .field("peer_role", &self.peer_role)
            .field("service", &self.service)
            .field(
                "monotonic_deadline_remaining_ms",
                &self.monotonic_deadline_remaining_ms,
            )
            .field("cancelled", &self.cancelled)
            .finish_non_exhaustive()
    }
}

impl ProviderCallContext<'_> {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.cancelled || self.monotonic_deadline_remaining_ms == 0 {
            return Err(ProviderContractError::RequestExpired);
        }
        let placement_service_matches = matches!(
            (self.peer_role, self.service),
            (
                EndpointRole::LocalRootController
                    | EndpointRole::RealmController
                    | EndpointRole::ProviderAgent,
                ServicePackage::ProviderV2
            ) | (EndpointRole::UserAgent, ServicePackage::UserV2)
        );
        if !placement_service_matches {
            return Err(ProviderContractError::PlacementMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationBinding {
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub request_digest: Fingerprint,
    pub provider_id: ProviderId,
    pub provider_generation: Generation,
}

impl fmt::Debug for OperationBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationBinding")
            .field("provider_generation", &self.provider_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderTarget {
    Realm {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    Workload {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
    },
    Handle {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: Option<WorkloadId>,
        #[serde(rename = "handleId")]
        handle_id: HandleId,
        #[serde(rename = "handleGeneration")]
        handle_generation: Generation,
    },
}

impl fmt::Debug for ProviderTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Realm { .. } => "ProviderTarget::Realm(<redacted>)",
            Self::Workload { .. } => "ProviderTarget::Workload(<redacted>)",
            Self::Handle { .. } => "ProviderTarget::Handle(<redacted>)",
        })
    }
}

impl ProviderTarget {
    pub fn realm_id(&self) -> &RealmId {
        match self {
            Self::Realm { realm_id }
            | Self::Workload { realm_id, .. }
            | Self::Handle { realm_id, .. } => realm_id,
        }
    }

    pub fn workload_id(&self) -> Option<&WorkloadId> {
        match self {
            Self::Workload { workload_id, .. } => Some(workload_id),
            Self::Handle { workload_id, .. } => workload_id.as_ref(),
            Self::Realm { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum InfrastructurePowerState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioChannel {
    Speaker,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioDirection {
    Output,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ObservabilityView {
    Health,
    Lifecycle,
    Operations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ObservabilityExportFormat {
    JsonLines,
    OtlpProtobuf,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderOperationInput {
    NoInput,
    ConfiguredRuntimeExecution {
        #[serde(rename = "configuredItemId")]
        configured_item_id: ConfiguredItemId,
    },
    InfrastructurePowerState {
        state: InfrastructurePowerState,
    },
    TransportBinding {
        #[serde(rename = "transportBindingId")]
        transport_binding_id: TransportBindingId,
    },
    StorageSnapshot {
        #[serde(rename = "snapshotId")]
        snapshot_id: StorageSnapshotId,
    },
    DeviceSelector {
        #[serde(rename = "deviceSelectorId")]
        device_selector_id: DeviceSelectorId,
    },
    AudioState {
        channel: AudioChannel,
        direction: AudioDirection,
        mute: Option<bool>,
        volume: Option<u8>,
    },
    ObservabilityQuery {
        view: ObservabilityView,
        cursor: Option<ObservabilityCursor>,
        limit: u16,
    },
    ObservabilityExport {
        format: ObservabilityExportFormat,
        #[serde(rename = "startAtUnixMs")]
        start_at_unix_ms: u64,
        #[serde(rename = "endAtUnixMs")]
        end_at_unix_ms: u64,
    },
}

impl fmt::Debug for ProviderOperationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInput => formatter.write_str("ProviderOperationInput::NoInput"),
            Self::ConfiguredRuntimeExecution { .. } => formatter
                .write_str("ProviderOperationInput::ConfiguredRuntimeExecution(<redacted>)"),
            Self::InfrastructurePowerState { state } => formatter
                .debug_struct("ProviderOperationInput::InfrastructurePowerState")
                .field("state", state)
                .finish(),
            Self::TransportBinding { .. } => {
                formatter.write_str("ProviderOperationInput::TransportBinding(<redacted>)")
            }
            Self::StorageSnapshot { .. } => {
                formatter.write_str("ProviderOperationInput::StorageSnapshot(<redacted>)")
            }
            Self::DeviceSelector { .. } => {
                formatter.write_str("ProviderOperationInput::DeviceSelector(<redacted>)")
            }
            Self::AudioState {
                channel,
                direction,
                mute,
                volume,
            } => formatter
                .debug_struct("ProviderOperationInput::AudioState")
                .field("channel", channel)
                .field("direction", direction)
                .field("mute", mute)
                .field("volume", volume)
                .finish(),
            Self::ObservabilityQuery { view, limit, .. } => formatter
                .debug_struct("ProviderOperationInput::ObservabilityQuery")
                .field("view", view)
                .field("cursor", &"<redacted>")
                .field("limit", limit)
                .finish(),
            Self::ObservabilityExport {
                format,
                start_at_unix_ms,
                end_at_unix_ms,
            } => formatter
                .debug_struct("ProviderOperationInput::ObservabilityExport")
                .field("format", format)
                .field("start_at_unix_ms", start_at_unix_ms)
                .field("end_at_unix_ms", end_at_unix_ms)
                .finish(),
        }
    }
}

impl ProviderOperationInput {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        match self {
            Self::AudioState {
                channel,
                direction,
                mute,
                volume,
            } => {
                let direction_matches = matches!(
                    (channel, direction),
                    (AudioChannel::Speaker, AudioDirection::Output)
                        | (AudioChannel::Microphone, AudioDirection::Input)
                );
                if !direction_matches || (mute.is_none() && volume.is_none()) {
                    return Err(ProviderContractError::OperationInputMismatch);
                }
                if volume.is_some_and(|value| value > 100) {
                    return Err(ProviderContractError::BoundExceeded);
                }
                Ok(())
            }
            Self::ObservabilityQuery { limit, .. } => {
                if *limit == 0 || *limit > MAX_OBSERVABILITY_QUERY_LIMIT {
                    Err(ProviderContractError::BoundExceeded)
                } else {
                    Ok(())
                }
            }
            Self::ObservabilityExport {
                start_at_unix_ms,
                end_at_unix_ms,
                ..
            } => {
                if *start_at_unix_ms > MAX_SAFE_JSON_INTEGER
                    || *end_at_unix_ms > MAX_SAFE_JSON_INTEGER
                    || *end_at_unix_ms <= *start_at_unix_ms
                    || *end_at_unix_ms - *start_at_unix_ms > MAX_OBSERVABILITY_EXPORT_RANGE_MS
                {
                    Err(ProviderContractError::InvalidTimeRange)
                } else {
                    Ok(())
                }
            }
            Self::NoInput
            | Self::ConfiguredRuntimeExecution { .. }
            | Self::InfrastructurePowerState { .. }
            | Self::TransportBinding { .. }
            | Self::StorageSnapshot { .. }
            | Self::DeviceSelector { .. } => Ok(()),
        }
    }

    pub fn validate_for(&self, method: ProviderMethod) -> Result<(), ProviderContractError> {
        self.validate()?;
        let compatible = match method {
            ProviderMethod::RuntimeExecute => {
                matches!(self, Self::ConfiguredRuntimeExecution { .. })
            }
            ProviderMethod::InfrastructureSetPowerState => {
                matches!(self, Self::InfrastructurePowerState { .. })
            }
            ProviderMethod::TransportConnect
            | ProviderMethod::InfrastructureBootstrapBinding
            | ProviderMethod::TransportRevokeBinding => {
                matches!(self, Self::TransportBinding { .. })
            }
            ProviderMethod::StorageSnapshot => matches!(self, Self::StorageSnapshot { .. }),
            ProviderMethod::DevicePlanAttach => matches!(self, Self::DeviceSelector { .. }),
            ProviderMethod::AudioSetState => matches!(self, Self::AudioState { .. }),
            ProviderMethod::ObservabilityQuery => {
                matches!(self, Self::ObservabilityQuery { .. })
            }
            ProviderMethod::ObservabilityExport => {
                matches!(self, Self::ObservabilityExport { .. })
            }
            _ => matches!(self, Self::NoInput),
        };
        if compatible {
            Ok(())
        } else {
            Err(ProviderContractError::OperationInputMismatch)
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderOperationRequest {
    pub context: ProviderOperationContext,
    pub target: ProviderTarget,
    pub expected_configuration_fingerprint: Fingerprint,
    pub input: ProviderOperationInput,
}

impl fmt::Debug for ProviderOperationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderOperationRequest")
            .field("context", &self.context)
            .field("target", &self.target)
            .field("input", &self.input)
            .finish_non_exhaustive()
    }
}

impl ProviderOperationRequest {
    pub fn validate(
        &self,
        descriptor: &ProviderDescriptor,
        now_unix_ms: u64,
    ) -> Result<(), ProviderContractError> {
        self.context.validate(descriptor, now_unix_ms)?;
        if self.target.realm_id() != self.context.scope.realm_id()
            || self.target.workload_id() != self.context.scope.workload_id()
            || self.expected_configuration_fingerprint
                != descriptor.configuration_schema_fingerprint
        {
            return Err(ProviderContractError::ScopeMismatch);
        }
        self.input.validate_for(self.context.method)?;
        Ok(())
    }

    pub fn validate_method(
        &self,
        descriptor: &ProviderDescriptor,
        now_unix_ms: u64,
        expected: ProviderMethod,
    ) -> Result<(), ProviderContractError> {
        self.validate(descriptor, now_unix_ms)?;
        if self.context.method == expected {
            Ok(())
        } else {
            Err(ProviderContractError::CapabilityMismatch)
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum PlannedResourceClass {
    WorkloadExecution,
    Infrastructure,
    CarriageSession,
    SubstrateRemediation,
    DisplaySession,
    Network,
    Storage,
    DeviceAttachment,
    AudioSession,
    ObservationStream,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPlan {
    pub schema_version: u32,
    pub plan_id: PlanId,
    pub binding: OperationBinding,
    pub realm_id: RealmId,
    pub workload_id: Option<WorkloadId>,
    pub method: ProviderMethod,
    pub configuration_fingerprint: Fingerprint,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub resources: BoundedVec<PlannedResourceClass, 0, MAX_PROVIDER_PLAN_RESOURCES>,
}

impl fmt::Debug for ProviderPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderPlan")
            .field("method", &self.method)
            .field("provider_generation", &self.binding.provider_generation)
            .field("resource_count", &self.resources.len())
            .finish_non_exhaustive()
    }
}

impl ProviderPlan {
    pub fn validate(
        &self,
        request: &ProviderOperationRequest,
        now_unix_ms: u64,
    ) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION
            || self.binding != request.context.binding()
            || self.realm_id != *request.target.realm_id()
            || self.workload_id.as_ref() != request.target.workload_id()
            || self.method != request.context.method
            || self.configuration_fingerprint != request.expected_configuration_fingerprint
        {
            return Err(ProviderContractError::OperationBindingMismatch);
        }
        if self.resources.len() > MAX_PROVIDER_PLAN_RESOURCES
            || self.resources.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ProviderContractError::BoundExceeded);
        }
        if self.created_at_unix_ms > now_unix_ms
            || self.expires_at_unix_ms <= now_unix_ms
            || self.expires_at_unix_ms > request.context.expires_at_unix_ms
        {
            return Err(ProviderContractError::RequestExpired);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderHandleKind {
    Runtime,
    Infrastructure,
    Transport,
    Display,
    Network,
    Storage,
    Device,
    Audio,
    Observation,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum HandleOwner {
    Provider {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "providerId")]
        provider_id: ProviderId,
    },
    RealmController {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
    },
    WorkloadRole {
        #[serde(rename = "realmId")]
        realm_id: RealmId,
        #[serde(rename = "workloadId")]
        workload_id: WorkloadId,
        #[serde(rename = "roleId")]
        role_id: RoleId,
    },
}

impl fmt::Debug for HandleOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Provider { .. } => "HandleOwner::Provider(<redacted>)",
            Self::RealmController { .. } => "HandleOwner::RealmController(<redacted>)",
            Self::WorkloadRole { .. } => "HandleOwner::WorkloadRole(<redacted>)",
        })
    }
}

impl HandleOwner {
    pub fn realm_id(&self) -> &RealmId {
        match self {
            Self::Provider { realm_id, .. }
            | Self::RealmController { realm_id, .. }
            | Self::WorkloadRole { realm_id, .. } => realm_id,
        }
    }

    pub fn workload_id(&self) -> Option<&WorkloadId> {
        match self {
            Self::WorkloadRole { workload_id, .. } => Some(workload_id),
            Self::Provider { .. } | Self::RealmController { .. } => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub enum OwnershipTransfer {
    Stationary {
        #[serde(rename = "ownershipEpoch")]
        ownership_epoch: Generation,
    },
    Pending {
        #[serde(rename = "transferId")]
        transfer_id: TransferId,
        #[serde(rename = "ownershipEpoch")]
        ownership_epoch: Generation,
        from: HandleOwner,
        to: HandleOwner,
        #[serde(rename = "issuedAtUnixMs")]
        issued_at_unix_ms: u64,
        #[serde(rename = "expiresAtUnixMs")]
        expires_at_unix_ms: u64,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderHandle {
    pub schema_version: u32,
    pub handle_id: HandleId,
    pub kind: ProviderHandleKind,
    pub provider_id: ProviderId,
    pub realm_id: RealmId,
    pub workload_id: Option<WorkloadId>,
    pub owner: HandleOwner,
    pub provider_generation: Generation,
    pub resource_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub created_by: OperationBinding,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: Option<u64>,
    pub ownership_transfer: OwnershipTransfer,
}

impl fmt::Debug for ProviderHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHandle")
            .field("kind", &self.kind)
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl ProviderHandle {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION
            || self.provider_id != self.created_by.provider_id
            || self.provider_generation != self.created_by.provider_generation
            || self.owner.realm_id() != &self.realm_id
            || self
                .owner
                .workload_id()
                .is_some_and(|workload| self.workload_id.as_ref() != Some(workload))
            || self.created_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self
                .expires_at_unix_ms
                .is_some_and(|expiry| expiry <= self.created_at_unix_ms)
        {
            return Err(ProviderContractError::HandleBindingMismatch);
        }
        match &self.ownership_transfer {
            OwnershipTransfer::Stationary { .. } => Ok(()),
            OwnershipTransfer::Pending {
                from,
                to,
                issued_at_unix_ms,
                expires_at_unix_ms,
                ..
            } if from == &self.owner
                && from != to
                && from.realm_id() == &self.realm_id
                && to.realm_id() == &self.realm_id
                && issued_at_unix_ms < expires_at_unix_ms =>
            {
                Ok(())
            }
            OwnershipTransfer::Pending { .. } => {
                Err(ProviderContractError::OwnershipTransferInvalid)
            }
        }
    }

    pub fn complete_transfer(
        &mut self,
        transfer_id: &TransferId,
        now_unix_ms: u64,
    ) -> Result<(), ProviderContractError> {
        let OwnershipTransfer::Pending {
            transfer_id: pending,
            ownership_epoch,
            to,
            expires_at_unix_ms,
            ..
        } = &self.ownership_transfer
        else {
            return Err(ProviderContractError::OwnershipTransferInvalid);
        };
        if pending != transfer_id || now_unix_ms >= *expires_at_unix_ms {
            return Err(ProviderContractError::OwnershipTransferInvalid);
        }
        self.owner = to.clone();
        self.ownership_transfer = OwnershipTransfer::Stationary {
            ownership_epoch: ownership_epoch.next()?,
        };
        self.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ObservedLifecycleState {
    Planned,
    Ready,
    Running,
    Stopped,
    Released,
    Destroyed,
    Unknown,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AdoptionState {
    NotAttempted,
    Adopted,
    Rejected,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationReason {
    None,
    IdentityMismatch,
    ConfigurationMismatch,
    GenerationMismatch,
    OwnerMismatch,
    MultipleCandidates,
    MissingEvidence,
    Cancelled,
    DeadlineExpired,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderObservation {
    pub provider_id: ProviderId,
    pub provider_generation: Generation,
    pub realm_id: RealmId,
    pub workload_id: Option<WorkloadId>,
    pub handle_id: Option<HandleId>,
    pub resource_generation: Option<Generation>,
    pub observed_at_unix_ms: u64,
    pub lifecycle: ObservedLifecycleState,
    pub adoption: AdoptionState,
    pub reason: ObservationReason,
    pub health: ProviderHealth,
}

impl fmt::Debug for ProviderObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderObservation")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .field("lifecycle", &self.lifecycle)
            .field("adoption", &self.adoption)
            .field("reason", &self.reason)
            .field("health", &self.health)
            .finish_non_exhaustive()
    }
}

impl ProviderObservation {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        self.health.validate()?;
        if self.provider_id != self.health.provider_id
            || self.provider_generation != self.health.registry_generation
            || self.observed_at_unix_ms != self.health.observed_at_unix_ms
        {
            return Err(ProviderContractError::AdoptionEvidenceMismatch);
        }
        if self.adoption == AdoptionState::Ambiguous {
            if self.lifecycle != ObservedLifecycleState::Quarantined
                || self.reason != ObservationReason::MultipleCandidates
                || self.health.state != ProviderHealthState::Failed
                || self.health.reason != ProviderHealthReason::AdoptionAmbiguous
            {
                return Err(ProviderContractError::AdoptionAmbiguous);
            }
        } else if self.reason == ObservationReason::MultipleCandidates {
            return Err(ProviderContractError::AdoptionEvidenceMismatch);
        }
        Ok(())
    }

    pub fn admits_mutation(&self) -> bool {
        self.adoption != AdoptionState::Ambiguous
            && self.lifecycle != ObservedLifecycleState::Quarantined
            && self.health.admits_operations()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdoptionRequest {
    pub context: ProviderOperationContext,
    pub handle: ProviderHandle,
    pub expected_owner: HandleOwner,
    pub expected_configuration_fingerprint: Fingerprint,
    pub expected_resource_generation: Generation,
}

impl fmt::Debug for AdoptionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdoptionRequest")
            .field("context", &self.context)
            .field("handle", &self.handle)
            .field("expected_owner", &self.expected_owner)
            .field(
                "expected_resource_generation",
                &self.expected_resource_generation,
            )
            .finish_non_exhaustive()
    }
}

impl AdoptionRequest {
    pub fn validate(
        &self,
        descriptor: &ProviderDescriptor,
        now_unix_ms: u64,
    ) -> Result<(), ProviderContractError> {
        self.context.validate(descriptor, now_unix_ms)?;
        self.handle.validate()?;
        if self.context.method.provider_type() != descriptor.provider_type()
            || !self.context.method.as_str().ends_with(".adopt")
            || self.handle.provider_id != descriptor.provider_id
            || self.handle.provider_generation != descriptor.registry_generation
            || self.handle.owner != self.expected_owner
            || self.handle.configuration_fingerprint != self.expected_configuration_fingerprint
            || self.handle.resource_generation != self.expected_resource_generation
        {
            return Err(ProviderContractError::AdoptionEvidenceMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RetryClass {
    Never,
    SameOperation,
    AfterObservation,
    AfterInteraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderFailureKind {
    CapabilityDenied,
    InvalidRequest,
    UnauthorizedScope,
    Cancelled,
    DeadlineExpired,
    Unavailable,
    InvariantViolation,
    AmbiguousMutation,
    AdoptionRejected,
    RegistryChanged,
    CredentialLeaseInvalid,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderFailure {
    pub kind: ProviderFailureKind,
    pub retry: RetryClass,
    pub provider_type: ProviderType,
    pub binding: OperationBinding,
    pub correlation_id: CorrelationId,
    pub occurred_at_unix_ms: u64,
    pub reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl fmt::Debug for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderFailure")
            .field("kind", &self.kind)
            .field("retry", &self.retry)
            .field("provider_type", &self.provider_type)
            .field("reason", &self.reason)
            .field("remediation", &self.remediation)
            .finish_non_exhaustive()
    }
}

impl ProviderFailure {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.occurred_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || matches!(
                self.kind,
                ProviderFailureKind::InvariantViolation
                    | ProviderFailureKind::UnauthorizedScope
                    | ProviderFailureKind::CapabilityDenied
            ) && self.retry != RetryClass::Never
            || self.kind == ProviderFailureKind::AmbiguousMutation
                && self.retry != RetryClass::AfterObservation
        {
            return Err(ProviderContractError::InvalidTransition);
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<(), ProviderContractError> {
        self.validate()?;
        if self.provider_type != descriptor.provider_type()
            || self.binding.provider_id != descriptor.provider_id
            || self.binding.provider_generation != descriptor.registry_generation
        {
            Err(ProviderContractError::OperationBindingMismatch)
        } else {
            Ok(())
        }
    }
}

pub type ProviderResult<T> = Result<T, ProviderFailure>;
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = ProviderResult<T>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MutationState {
    Applied,
    AlreadyApplied,
    NotApplicable,
    CancelledBeforeMutation,
    CompletionAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationReceipt {
    pub binding: OperationBinding,
    pub state: MutationState,
    pub observed_at_unix_ms: u64,
    pub observation_required_before_retry: bool,
}

impl MutationReceipt {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.observed_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || (self.state == MutationState::CompletionAmbiguous
                && !self.observation_required_before_retry)
            || (self.state != MutationState::CompletionAmbiguous
                && self.observation_required_before_retry)
        {
            return Err(ProviderContractError::InvalidTransition);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CancellationReason {
    CallerCancelled,
    DeadlineExpired,
    RegistryDraining,
    AgentDisconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCancellation {
    pub operation_id: OperationId,
    pub provider_id: ProviderId,
    pub provider_generation: Generation,
    pub reason: CancellationReason,
    pub cancelled_at_unix_ms: u64,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SdkOperationClass {
    Authenticate,
    Discover,
    Read,
    Create,
    Update,
    Delete,
    Power,
    Connect,
    Listen,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialLeaseState {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialLeaseTransferPolicy {
    Forbidden,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialLease {
    pub lease_id: LeaseId,
    pub credential_provider_id: ProviderId,
    pub consumer_provider_id: ProviderId,
    pub placement_binding: CredentialPlacementBinding,
    pub allowed_operations: BoundedVec<SdkOperationClass, 1, MAX_CREDENTIAL_OPERATION_CLASSES>,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub credential_provider_generation: Generation,
    pub consumer_provider_generation: Generation,
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub state: CredentialLeaseState,
    pub transfer_policy: CredentialLeaseTransferPolicy,
    pub revoked_at_unix_ms: Option<u64>,
}

impl fmt::Debug for CredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialLease")
            .field("state", &self.state)
            .field("rotation_generation", &self.rotation_generation)
            .field("transfer_policy", &self.transfer_policy)
            .finish_non_exhaustive()
    }
}

impl CredentialLease {
    pub fn validate(
        &self,
        credential: &ProviderDescriptor,
        consumer: &ProviderDescriptor,
        now_unix_ms: u64,
    ) -> Result<(), ProviderContractError> {
        credential.validate()?;
        consumer.validate()?;
        let credential_binding = credential.placement.credential_binding();
        let consumer_binding = consumer.placement.credential_binding();
        if credential.provider_type() != ProviderType::Credential
            || self.credential_provider_id != credential.provider_id
            || self.consumer_provider_id != consumer.provider_id
            || self.credential_provider_generation != credential.registry_generation
            || self.consumer_provider_generation != consumer.registry_generation
            || credential_binding.as_ref() != Some(&self.placement_binding)
            || consumer_binding.as_ref() != Some(&self.placement_binding)
            || self.credential_provider_id == self.consumer_provider_id
        {
            return Err(ProviderContractError::LeaseNotColocated);
        }
        if self.issued_at_unix_ms >= self.expires_at_unix_ms
            || self.expires_at_unix_ms - self.issued_at_unix_ms > MAX_PROVIDER_LEASE_LIFETIME_MS
        {
            return Err(ProviderContractError::RequestLifetimeExceeded);
        }
        if self.transfer_policy != CredentialLeaseTransferPolicy::Forbidden {
            return Err(ProviderContractError::LeaseTransferForbidden);
        }
        if self.allowed_operations.len() > MAX_CREDENTIAL_OPERATION_CLASSES
            || self
                .allowed_operations
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(ProviderContractError::BoundExceeded);
        }
        match self.state {
            CredentialLeaseState::Active
                if self.revoked_at_unix_ms.is_none() && now_unix_ms < self.expires_at_unix_ms =>
            {
                Ok(())
            }
            CredentialLeaseState::Active if now_unix_ms >= self.expires_at_unix_ms => {
                Err(ProviderContractError::LeaseExpired)
            }
            CredentialLeaseState::Revoked
                if self.revoked_at_unix_ms.is_some_and(|revoked_at| {
                    revoked_at >= self.issued_at_unix_ms && revoked_at <= self.expires_at_unix_ms
                }) =>
            {
                Err(ProviderContractError::LeaseRevoked)
            }
            CredentialLeaseState::Expired if now_unix_ms >= self.expires_at_unix_ms => {
                Err(ProviderContractError::LeaseExpired)
            }
            _ => Err(ProviderContractError::InvalidTransition),
        }
    }

    pub fn revoke(&mut self, now_unix_ms: u64) -> Result<(), ProviderContractError> {
        if self.state != CredentialLeaseState::Active
            || now_unix_ms < self.issued_at_unix_ms
            || now_unix_ms > self.expires_at_unix_ms
        {
            return Err(ProviderContractError::InvalidTransition);
        }
        self.state = CredentialLeaseState::Revoked;
        self.revoked_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    pub fn refresh(
        &mut self,
        now_unix_ms: u64,
        new_expiry_unix_ms: u64,
        source_version: SourceVersion,
        rotation_generation: Generation,
    ) -> Result<(), ProviderContractError> {
        if self.state != CredentialLeaseState::Active
            || self.revoked_at_unix_ms.is_some()
            || now_unix_ms >= self.expires_at_unix_ms
            || new_expiry_unix_ms <= now_unix_ms
            || new_expiry_unix_ms - now_unix_ms > MAX_PROVIDER_LEASE_LIFETIME_MS
            || rotation_generation < self.rotation_generation
        {
            return Err(ProviderContractError::InvalidTransition);
        }
        self.issued_at_unix_ms = now_unix_ms;
        self.expires_at_unix_ms = new_expiry_unix_ms;
        self.source_version = source_version;
        self.rotation_generation = rotation_generation;
        Ok(())
    }

    pub fn transfer_to(&self, _consumer: &ProviderId) -> Result<(), ProviderContractError> {
        Err(ProviderContractError::LeaseTransferForbidden)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialLeaseRequest {
    pub context: ProviderOperationContext,
    pub consumer_provider_id: ProviderId,
    pub placement_binding: CredentialPlacementBinding,
    pub allowed_operations: BoundedVec<SdkOperationClass, 1, MAX_CREDENTIAL_OPERATION_CLASSES>,
    pub requested_expiry_unix_ms: u64,
}

impl fmt::Debug for CredentialLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialLeaseRequest")
            .field("operation_count", &self.allowed_operations.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryLifecycle {
    Accepting,
    Draining,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistryAxis {
    pub provider_type: ProviderType,
    pub providers: BoundedVec<ProviderId, 0, MAX_PROVIDER_REGISTRY_ENTRIES>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderFactoryKey {
    pub provider_type: ProviderType,
    pub implementation_id: ImplementationId,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistrySnapshot {
    pub schema_version: u32,
    pub generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub published_at_unix_ms: u64,
    pub lifecycle: RegistryLifecycle,
    pub axes: BoundedVec<ProviderRegistryAxis, 11, 11>,
    pub factories: BoundedVec<ProviderFactoryKey, 1, MAX_PROVIDER_REGISTRY_ENTRIES>,
    pub providers: BoundedVec<ProviderDescriptor, 1, MAX_PROVIDER_REGISTRY_ENTRIES>,
}

impl fmt::Debug for ProviderRegistrySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistrySnapshot")
            .field("generation", &self.generation)
            .field("lifecycle", &self.lifecycle)
            .field("factory_count", &self.factories.len())
            .field("provider_count", &self.providers.len())
            .finish_non_exhaustive()
    }
}

impl ProviderRegistrySnapshot {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION {
            return Err(ProviderContractError::UnsupportedSchemaVersion);
        }
        if self.published_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.axes.len() != ProviderType::ALL.len()
            || self.factories.is_empty()
            || self.factories.len() > MAX_PROVIDER_REGISTRY_ENTRIES
            || self.providers.is_empty()
            || self.providers.len() > MAX_PROVIDER_REGISTRY_ENTRIES
        {
            return Err(ProviderContractError::BoundExceeded);
        }
        let expected_axes = ProviderType::ALL;
        if self
            .axes
            .iter()
            .map(|axis| axis.provider_type)
            .ne(expected_axes)
        {
            return Err(ProviderContractError::RegistryNotCanonical);
        }
        let mut all_axis_ids = BTreeSet::new();
        for axis in self.axes.iter() {
            if axis.providers.len() > MAX_PROVIDER_REGISTRY_ENTRIES
                || axis.providers.windows(2).any(|pair| pair[0] >= pair[1])
                || axis
                    .providers
                    .iter()
                    .any(|provider_id| !all_axis_ids.insert(provider_id.clone()))
            {
                return Err(ProviderContractError::RegistryNotCanonical);
            }
        }
        let mut previous: Option<&ProviderId> = None;
        for descriptor in self.providers.iter() {
            descriptor.validate()?;
            if descriptor.registry_generation != self.generation
                || previous.is_some_and(|provider_id| provider_id >= &descriptor.provider_id)
            {
                return Err(ProviderContractError::RegistryNotCanonical);
            }
            let axis = &self.axes[ProviderType::ALL
                .iter()
                .position(|kind| *kind == descriptor.provider_type())
                .ok_or(ProviderContractError::RegistryNotCanonical)?];
            if axis
                .providers
                .binary_search(&descriptor.provider_id)
                .is_err()
            {
                return Err(ProviderContractError::ProviderTypeMismatch);
            }
            previous = Some(&descriptor.provider_id);
        }
        let descriptor_ids: BTreeSet<_> = self
            .providers
            .iter()
            .map(|descriptor| descriptor.provider_id.clone())
            .collect();
        if descriptor_ids != all_axis_ids {
            return Err(ProviderContractError::RegistryNotCanonical);
        }
        if self.factories.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ProviderContractError::DuplicateFactory);
        }
        let mut used_factory_keys = BTreeSet::new();
        for descriptor in self.providers.iter() {
            let key = ProviderFactoryKey {
                provider_type: descriptor.provider_type(),
                implementation_id: descriptor.implementation_id.clone(),
            };
            if self.factories.binary_search(&key).is_err() {
                return Err(ProviderContractError::DuplicateFactory);
            }
            used_factory_keys.insert(key);
        }
        if used_factory_keys
            != self
                .factories
                .iter()
                .cloned()
                .collect::<BTreeSet<ProviderFactoryKey>>()
        {
            return Err(ProviderContractError::RegistryNotCanonical);
        }
        Ok(())
    }

    pub fn descriptor(&self, provider_id: &ProviderId) -> Option<&ProviderDescriptor> {
        self.providers
            .binary_search_by(|descriptor| descriptor.provider_id.cmp(provider_id))
            .ok()
            .map(|index| &self.providers[index])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryDrainPolicy {
    pub drain_deadline_ms: u32,
    pub cancel_in_flight_at_deadline: bool,
    pub revoke_transport_bindings: bool,
    pub revoke_credential_leases: bool,
    pub close_provider_sessions: bool,
}

impl RegistryDrainPolicy {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.drain_deadline_ms == 0
            || self.drain_deadline_ms > MAX_PROVIDER_DRAIN_MS
            || !self.cancel_in_flight_at_deadline
            || !self.revoke_transport_bindings
            || !self.revoke_credential_leases
            || !self.close_provider_sessions
        {
            Err(ProviderContractError::InvalidTransition)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistryUpdate {
    pub from_generation: Generation,
    pub from_configuration_fingerprint: Fingerprint,
    pub replacement: ProviderRegistrySnapshot,
    pub drain_policy: RegistryDrainPolicy,
}

impl ProviderRegistryUpdate {
    pub fn validate(
        &self,
        current: &ProviderRegistrySnapshot,
    ) -> Result<(), ProviderContractError> {
        current.validate()?;
        self.replacement.validate()?;
        self.drain_policy.validate()?;
        if current.lifecycle != RegistryLifecycle::Accepting
            || self.from_generation != current.generation
            || self.from_configuration_fingerprint != current.configuration_fingerprint
            || self.replacement.generation != current.generation.next()?
            || self.replacement.lifecycle != RegistryLifecycle::Accepting
            || self.replacement.configuration_fingerprint == current.configuration_fingerprint
        {
            return Err(ProviderContractError::RegistryGenerationMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSelectionRequest {
    pub realm_id: RealmId,
    pub workload_id: Option<WorkloadId>,
    pub provider_type: ProviderType,
    pub capability: ProviderCapability,
    pub required_registry_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub preferred_provider_id: Option<ProviderId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderSelectionReason {
    ExactConfiguredProvider,
    SoleEligibleProvider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSelection {
    pub provider_id: ProviderId,
    pub provider_type: ProviderType,
    pub capability: ProviderCapability,
    pub registry_generation: Generation,
    pub reason: ProviderSelectionReason,
}

impl ProviderSelectionRequest {
    pub fn select(
        &self,
        registry: &ProviderRegistrySnapshot,
    ) -> Result<ProviderSelection, ProviderContractError> {
        registry.validate()?;
        if self.required_registry_generation != registry.generation {
            return Err(ProviderContractError::RegistryGenerationMismatch);
        }
        if self.configuration_fingerprint != registry.configuration_fingerprint {
            return Err(ProviderContractError::RegistryFingerprintMismatch);
        }
        if self.capability.provider_type() != self.provider_type {
            return Err(ProviderContractError::CapabilityMismatch);
        }
        let eligible: Vec<_> = registry
            .providers
            .iter()
            .filter(|descriptor| {
                descriptor.provider_type() == self.provider_type
                    && descriptor.placement.realm_id() == &self.realm_id
                    && descriptor
                        .capabilities
                        .as_slice()
                        .contains(&self.capability)
            })
            .collect();
        let (descriptor, reason) = match &self.preferred_provider_id {
            Some(provider_id) => (
                eligible
                    .into_iter()
                    .find(|descriptor| &descriptor.provider_id == provider_id)
                    .ok_or(ProviderContractError::UnknownProvider)?,
                ProviderSelectionReason::ExactConfiguredProvider,
            ),
            None if eligible.len() == 1 => {
                (eligible[0], ProviderSelectionReason::SoleEligibleProvider)
            }
            None => return Err(ProviderContractError::NoEligibleProvider),
        };
        Ok(ProviderSelection {
            provider_id: descriptor.provider_id.clone(),
            provider_type: descriptor.provider_type(),
            capability: self.capability,
            registry_generation: registry.generation,
            reason,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderContractDocument {
    pub schema_version: u32,
    pub contract_fingerprint: Fingerprint,
    pub registry: ProviderRegistrySnapshot,
    pub operation: ProviderOperationRequest,
    pub plan: ProviderPlan,
    pub handle: ProviderHandle,
    pub observation: ProviderObservation,
    pub credential_lease: CredentialLease,
    pub failure: ProviderFailure,
}

impl ProviderContractDocument {
    pub fn validate(&self, now_unix_ms: u64) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_SCHEMA_VERSION {
            return Err(ProviderContractError::UnsupportedSchemaVersion);
        }
        if self.contract_fingerprint.as_str() != PROVIDER_CONTRACT_FINGERPRINT {
            return Err(ProviderContractError::ContractFingerprintMismatch);
        }
        self.registry.validate()?;
        let descriptor = self
            .registry
            .descriptor(&self.operation.context.provider_id)
            .ok_or(ProviderContractError::UnknownProvider)?;
        self.operation.validate(descriptor, now_unix_ms)?;
        self.plan.validate(&self.operation, now_unix_ms)?;
        self.handle.validate()?;
        self.observation.validate()?;
        self.failure.validate_against(descriptor)?;
        let credential = self
            .registry
            .descriptor(&self.credential_lease.credential_provider_id)
            .ok_or(ProviderContractError::UnknownProvider)?;
        let consumer = self
            .registry
            .descriptor(&self.credential_lease.consumer_provider_id)
            .ok_or(ProviderContractError::UnknownProvider)?;
        self.credential_lease
            .validate(credential, consumer, now_unix_ms)?;
        Ok(())
    }
}

pub trait Provider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth>;
}

macro_rules! lifecycle_provider_trait {
    (
        $name:ident {
            $($method:ident($request:ty) -> $result:ty;)+
        }
    ) => {
        pub trait $name: Provider {
            fn capabilities(&self) -> ProviderCapabilitySet;
            $(
                fn $method<'a>(
                    &'a self,
                    context: &'a ProviderCallContext<'a>,
                    request: &'a $request,
                ) -> ProviderFuture<'a, $result>;
            )+
        }
    };
}

lifecycle_provider_trait!(RuntimeProvider {
    plan(ProviderOperationRequest) -> ProviderPlan;
    ensure(ProviderPlan) -> ProviderHandle;
    start(ProviderOperationRequest) -> ProviderObservation;
    stop(ProviderOperationRequest) -> ProviderObservation;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    destroy(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(InfrastructureProvider {
    plan(ProviderOperationRequest) -> ProviderPlan;
    apply(ProviderPlan) -> ProviderHandle;
    set_power_state(ProviderOperationRequest) -> ProviderObservation;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    bootstrap_binding(ProviderOperationRequest) -> ProviderHandle;
    destroy(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(TransportProvider {
    connect(ProviderOperationRequest) -> ProviderHandle;
    listen(ProviderOperationRequest) -> ProviderHandle;
    issue_binding(ProviderOperationRequest) -> ProviderHandle;
    revoke_binding(ProviderOperationRequest) -> MutationReceipt;
    inspect(ProviderOperationRequest) -> ProviderObservation;
});

lifecycle_provider_trait!(SubstrateProvider {
    check(ProviderOperationRequest) -> ProviderObservation;
    plan_remediation(ProviderOperationRequest) -> ProviderPlan;
    apply(ProviderPlan) -> MutationReceipt;
});

pub trait CredentialProvider: Provider {
    fn status<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation>;
    fn acquire_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a CredentialLeaseRequest,
    ) -> ProviderFuture<'a, CredentialLease>;
    fn refresh_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, CredentialLease>;
    fn revoke_lease<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        lease: &'a CredentialLease,
    ) -> ProviderFuture<'a, MutationReceipt>;
}

lifecycle_provider_trait!(DisplayProvider {
    open(ProviderOperationRequest) -> ProviderHandle;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    close(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(NetworkProvider {
    plan(ProviderOperationRequest) -> ProviderPlan;
    ensure(ProviderPlan) -> ProviderHandle;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    release(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(StorageProvider {
    plan(ProviderOperationRequest) -> ProviderPlan;
    ensure(ProviderPlan) -> ProviderHandle;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    snapshot(ProviderOperationRequest) -> ProviderHandle;
    destroy(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(DeviceProvider {
    plan_attach(ProviderOperationRequest) -> ProviderPlan;
    attach(ProviderPlan) -> ProviderHandle;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    detach(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(AudioProvider {
    open(ProviderOperationRequest) -> ProviderHandle;
    set_state(ProviderOperationRequest) -> ProviderObservation;
    inspect(ProviderOperationRequest) -> ProviderObservation;
    adopt(AdoptionRequest) -> ProviderObservation;
    close(ProviderOperationRequest) -> MutationReceipt;
});

lifecycle_provider_trait!(ObservabilityProvider {
    status(ProviderOperationRequest) -> ProviderObservation;
    query(ProviderOperationRequest) -> ProviderObservation;
    subscribe(ProviderOperationRequest) -> ProviderHandle;
    export(ProviderOperationRequest) -> MutationReceipt;
});
