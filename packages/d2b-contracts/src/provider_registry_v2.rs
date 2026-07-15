//! Private provider-registry bundle contract for d2b 2.0.
//!
//! The artifact binds canonical provider descriptors to opaque intents in the
//! integrity-checked host bundle. It deliberately cannot carry command lines,
//! host paths, credential material, or provider SDK configuration.

use std::{collections::BTreeSet, error::Error, fmt};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    v2_identity::{ConfiguredProviderId, ProviderId, ProviderType, RealmId, WorkloadId},
    v2_provider::{
        Fingerprint, Generation, MAX_PROVIDER_REGISTRY_ENTRIES, MAX_SAFE_JSON_INTEGER,
        ProviderContractError, ProviderDescriptor, ProviderPlacement,
    },
};

pub const PROVIDER_REGISTRY_V2_SCHEMA_VERSION: &str = "v2";
pub const MAX_PROVIDER_INTENT_ID_BYTES: usize = 128;
pub const LOCAL_RUNTIME_CONFIGURATION_SCHEMA_SEED: &str =
    "d2b-provider-runtime-local-configuration-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRegistryV2Error {
    UnsupportedSchemaVersion,
    InvalidPublicationTime,
    BoundExceeded,
    InvalidDescriptor,
    GenerationMismatch,
    DuplicateProvider,
    NonCanonicalOrder,
    ProviderTypeMismatch,
    PlacementMismatch,
    BindingMismatch,
    InvalidOpaqueIntent,
}

impl fmt::Display for ProviderRegistryV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedSchemaVersion => "unsupported provider registry schema version",
            Self::InvalidPublicationTime => "invalid provider registry publication time",
            Self::BoundExceeded => "provider registry entry bound exceeded",
            Self::InvalidDescriptor => "provider registry descriptor is invalid",
            Self::GenerationMismatch => "provider registry generation mismatch",
            Self::DuplicateProvider => "provider registry contains a duplicate provider",
            Self::NonCanonicalOrder => "provider registry entries are not canonically ordered",
            Self::ProviderTypeMismatch => "provider registry binding has the wrong provider type",
            Self::PlacementMismatch => "provider registry binding has the wrong placement",
            Self::BindingMismatch => "provider registry descriptor and binding do not match",
            Self::InvalidOpaqueIntent => "provider registry contains an invalid opaque intent ID",
        })
    }
}

impl Error for ProviderRegistryV2Error {}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ProviderIntentId(
    #[schemars(length(min = 1, max = 128), regex(pattern = "^[a-z0-9][a-z0-9:_.-]*$"))] String,
);

impl ProviderIntentId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProviderRegistryV2Error> {
        let value = value.into();
        if !value.is_empty()
            && value.len() <= MAX_PROVIDER_INTENT_ID_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b":_-.".contains(&byte)
            })
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            Ok(Self(value))
        } else {
            Err(ProviderRegistryV2Error::InvalidOpaqueIntent)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderIntentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderIntentId(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for ProviderIntentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalRuntimeProviderBindingV2 {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub vm_start_intent_id: ProviderIntentId,
    pub runner_intent_id: ProviderIntentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "axis", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderBindingV2 {
    LocalRuntime(LocalRuntimeProviderBindingV2),
}

impl ProviderBindingV2 {
    pub const fn provider_type(&self) -> ProviderType {
        match self {
            Self::LocalRuntime(_) => ProviderType::Runtime,
        }
    }

    pub fn realm_id(&self) -> &RealmId {
        match self {
            Self::LocalRuntime(binding) => &binding.realm_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistryEntryV2 {
    pub descriptor: ProviderDescriptor,
    pub binding: ProviderBindingV2,
}

impl ProviderRegistryEntryV2 {
    pub fn validate(&self, generation: Generation) -> Result<(), ProviderRegistryV2Error> {
        self.descriptor
            .validate()
            .map_err(|_| ProviderRegistryV2Error::InvalidDescriptor)?;
        if self.descriptor.registry_generation != generation {
            return Err(ProviderRegistryV2Error::GenerationMismatch);
        }
        if self.descriptor.provider_type() != self.binding.provider_type() {
            return Err(ProviderRegistryV2Error::ProviderTypeMismatch);
        }
        if self.descriptor.placement.realm_id() != self.binding.realm_id() {
            return Err(ProviderRegistryV2Error::BindingMismatch);
        }
        if !matches!(
            self.descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(ProviderRegistryV2Error::PlacementMismatch);
        }
        match &self.binding {
            ProviderBindingV2::LocalRuntime(binding) => {
                let configured_provider_id = ConfiguredProviderId::parse(format!(
                    "runtime-{}",
                    binding.workload_id.as_str()
                ))
                .map_err(|_| ProviderRegistryV2Error::BindingMismatch)?;
                let expected_provider_id = ProviderId::derive(
                    &binding.realm_id,
                    ProviderType::Runtime,
                    &configured_provider_id,
                );
                if self.descriptor.provider_id != expected_provider_id
                    || self.descriptor.configuration_schema_fingerprint
                        != local_runtime_configuration_schema_fingerprint()?
                    || self.descriptor.configured_scope_digest
                        != local_runtime_configured_scope_digest(
                            &self.descriptor.provider_id,
                            binding,
                        )?
                {
                    return Err(ProviderRegistryV2Error::BindingMismatch);
                }
            }
        }
        Ok(())
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.descriptor.provider_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistryV2 {
    pub schema_version: String,
    pub registry_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub published_at_unix_ms: u64,
    pub providers: Vec<ProviderRegistryEntryV2>,
}

impl ProviderRegistryV2 {
    pub fn validate(&self) -> Result<(), ProviderRegistryV2Error> {
        if self.schema_version != PROVIDER_REGISTRY_V2_SCHEMA_VERSION {
            return Err(ProviderRegistryV2Error::UnsupportedSchemaVersion);
        }
        if self.published_at_unix_ms > MAX_SAFE_JSON_INTEGER {
            return Err(ProviderRegistryV2Error::InvalidPublicationTime);
        }
        if self.providers.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(ProviderRegistryV2Error::BoundExceeded);
        }

        let mut seen = BTreeSet::new();
        let mut previous: Option<&ProviderId> = None;
        for entry in &self.providers {
            entry.validate(self.registry_generation)?;
            if previous.is_some_and(|provider_id| provider_id >= entry.provider_id()) {
                return Err(if previous == Some(entry.provider_id()) {
                    ProviderRegistryV2Error::DuplicateProvider
                } else {
                    ProviderRegistryV2Error::NonCanonicalOrder
                });
            }
            if !seen.insert(entry.provider_id()) {
                return Err(ProviderRegistryV2Error::DuplicateProvider);
            }
            previous = Some(entry.provider_id());
        }
        Ok(())
    }

    pub fn find(&self, provider_id: &ProviderId) -> Option<&ProviderRegistryEntryV2> {
        self.providers
            .binary_search_by(|entry| entry.provider_id().cmp(provider_id))
            .ok()
            .map(|index| &self.providers[index])
    }
}

pub fn local_runtime_configuration_schema_fingerprint()
-> Result<Fingerprint, ProviderRegistryV2Error> {
    sha256_fingerprint(LOCAL_RUNTIME_CONFIGURATION_SCHEMA_SEED.as_bytes())
}

pub fn local_runtime_configured_scope_digest(
    provider_id: &ProviderId,
    binding: &LocalRuntimeProviderBindingV2,
) -> Result<Fingerprint, ProviderRegistryV2Error> {
    let canonical_scope = format!(
        concat!(
            r#"{{"providerId":"{}","realmId":"{}","#,
            r#""runnerIntentId":"{}","vmStartIntentId":"{}","#,
            r#""workloadId":"{}"}}"#
        ),
        provider_id.as_str(),
        binding.realm_id.as_str(),
        binding.runner_intent_id.as_str(),
        binding.vm_start_intent_id.as_str(),
        binding.workload_id.as_str(),
    );
    sha256_fingerprint(canonical_scope.as_bytes())
}

fn sha256_fingerprint(bytes: &[u8]) -> Result<Fingerprint, ProviderRegistryV2Error> {
    Fingerprint::parse(format!("{:x}", Sha256::digest(bytes)))
        .map_err(|_| ProviderRegistryV2Error::BindingMismatch)
}

impl From<ProviderContractError> for ProviderRegistryV2Error {
    fn from(_: ProviderContractError) -> Self {
        Self::InvalidDescriptor
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        v2_component_session::EndpointRole,
        v2_identity::{ConfiguredProviderId, ProviderType, RealmPath, WorkloadName},
        v2_provider::{
            CgroupAuthority, DeviceMediationPosture, ImplementationId, NetworkPosture,
            PROVIDER_SCHEMA_VERSION, PersistentIdentityPosture, ProcessAuthority,
            ProviderApiVersion, ProviderAuthority, ProviderCapability, ProviderCapabilitySet,
            ProviderMethod, RuntimeAuthorityPosture, UserNamespacePosture,
        },
    };

    use super::*;

    fn fixture() -> ProviderRegistryV2 {
        let realm_id = RealmId::derive(&RealmPath::parse("work.local-root").unwrap());
        let workload_id = WorkloadId::derive(&realm_id, &WorkloadName::parse("corp-vm").unwrap());
        let provider_id = ProviderId::derive(
            &realm_id,
            ProviderType::Runtime,
            &ConfiguredProviderId::parse(format!("runtime-{}", workload_id.as_str())).unwrap(),
        );
        let binding = LocalRuntimeProviderBindingV2 {
            realm_id: realm_id.clone(),
            workload_id,
            vm_start_intent_id: ProviderIntentId::parse(
                "vm-start:vm:corp-vm:role:cloud-hypervisor",
            )
            .unwrap(),
            runner_intent_id: ProviderIntentId::parse("runner:vm:corp-vm:role:cloud-hypervisor")
                .unwrap(),
        };
        let generation = Generation::new(1).unwrap();
        let configuration_schema_fingerprint =
            local_runtime_configuration_schema_fingerprint().unwrap();
        let configured_scope_digest =
            local_runtime_configured_scope_digest(&provider_id, &binding).unwrap();
        ProviderRegistryV2 {
            schema_version: PROVIDER_REGISTRY_V2_SCHEMA_VERSION.to_owned(),
            registry_generation: generation,
            configuration_fingerprint: Fingerprint::parse("1".repeat(64)).unwrap(),
            published_at_unix_ms: 0,
            providers: vec![ProviderRegistryEntryV2 {
                descriptor: ProviderDescriptor {
                    schema_version: PROVIDER_SCHEMA_VERSION,
                    provider_id,
                    authority: ProviderAuthority::Runtime {
                        posture: RuntimeAuthorityPosture {
                            process: ProcessAuthority::ProviderOwnedPidfd,
                            cgroup: CgroupAuthority::RealmDelegatedLeaf,
                            network: NetworkPosture::IsolatedNamespace,
                            user_namespace: UserNamespacePosture::BrokerPreestablished,
                            persistent_identity: PersistentIdentityPosture::FileBackedCloneable,
                            device_mediation: DeviceMediationPosture::BrokerDelegatedTyped,
                        },
                    },
                    implementation_id: ImplementationId::parse("cloud-hypervisor").unwrap(),
                    api_version: ProviderApiVersion::V2,
                    capabilities: ProviderCapabilitySet::new(
                        [
                            ProviderMethod::RuntimePlan,
                            ProviderMethod::RuntimeEnsure,
                            ProviderMethod::RuntimeStart,
                            ProviderMethod::RuntimeStop,
                            ProviderMethod::RuntimeInspect,
                            ProviderMethod::RuntimeAdopt,
                            ProviderMethod::RuntimeDestroy,
                        ]
                        .into_iter()
                        .map(ProviderCapability)
                        .collect(),
                    )
                    .unwrap(),
                    configuration_schema_fingerprint,
                    configured_scope_digest,
                    registry_generation: generation,
                    placement: ProviderPlacement::TrustedFirstPartyInProcess {
                        realm_id: realm_id.clone(),
                        controller_role: EndpointRole::RealmController,
                    },
                },
                binding: ProviderBindingV2::LocalRuntime(binding),
            }],
        }
    }

    #[test]
    fn validates_closed_local_runtime_mapping() {
        let registry = fixture();
        registry.validate().unwrap();
        let encoded = serde_json::to_string(&registry).unwrap();
        assert!(!encoded.contains("argv"));
        assert!(!encoded.contains("secret"));
        let decoded: ProviderRegistryV2 = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, registry);
    }

    #[test]
    fn rejects_generation_and_binding_mismatch() {
        let mut registry = fixture();
        registry.providers[0].descriptor.registry_generation = Generation::new(2).unwrap();
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::GenerationMismatch)
        );

        let mut registry = fixture();
        let other = RealmId::derive(&RealmPath::parse("other.local-root").unwrap());
        let ProviderBindingV2::LocalRuntime(binding) = &mut registry.providers[0].binding;
        binding.realm_id = other;
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );

        let mut registry = fixture();
        registry.providers[0]
            .descriptor
            .configuration_schema_fingerprint = Fingerprint::parse("4".repeat(64)).unwrap();
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );

        let mut registry = fixture();
        let ProviderBindingV2::LocalRuntime(binding) = &mut registry.providers[0].binding;
        binding.runner_intent_id =
            ProviderIntentId::parse("runner:vm:other:role:cloud-hypervisor").unwrap();
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );
    }

    #[test]
    fn accepts_explicit_empty_registry() {
        let registry = ProviderRegistryV2 {
            schema_version: PROVIDER_REGISTRY_V2_SCHEMA_VERSION.to_owned(),
            registry_generation: Generation::new(1).unwrap(),
            configuration_fingerprint: Fingerprint::parse("0".repeat(64)).unwrap(),
            published_at_unix_ms: 0,
            providers: Vec::new(),
        };
        registry.validate().unwrap();
    }
}
