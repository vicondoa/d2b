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
    v2_identity::{ConfiguredProviderId, ProviderId, ProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::{
        Fingerprint, Generation, MAX_OBSERVABILITY_EXPORT_RANGE_MS, MAX_OBSERVABILITY_QUERY_BYTES,
        MAX_OBSERVABILITY_QUERY_LIMIT, MAX_PROVIDER_REGISTRY_ENTRIES, MAX_SAFE_JSON_INTEGER,
        OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES, ProviderContractError, ProviderDescriptor,
        ProviderPlacement,
    },
};

pub const PROVIDER_REGISTRY_V2_SCHEMA_VERSION: &str = "v2";
pub const MAX_PROVIDER_INTENT_ID_BYTES: usize = 128;
pub const MAX_PROVIDER_MAPPING_IDS: usize = 64;
pub const LOCAL_RUNTIME_CONFIGURATION_SCHEMA_SEED: &str =
    "d2b-provider-runtime-local-configuration-v1";
pub const LOCAL_OBSERVABILITY_CONFIGURATION_SCHEMA_SEED: &str =
    "d2b-provider-observability-local-configuration-v1";

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
    ProviderIdMismatch,
    ConfigurationSchemaFingerprintMismatch,
    ConfiguredScopeDigestMismatch,
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
            Self::ProviderIdMismatch => {
                "provider ID does not match descriptor placement and workload binding"
            }
            Self::ConfigurationSchemaFingerprintMismatch => {
                "configuration schema fingerprint does not match the first-party provider contract"
            }
            Self::ConfiguredScopeDigestMismatch => {
                "configured scope digest does not match the closed provider binding"
            }
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
    pub workload_id: WorkloadId,
    pub vm_start_intent_id: ProviderIntentId,
    pub runner_intent_id: ProviderIntentId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalObservabilityProviderBindingV2 {
    #[schemars(range(min = 1, max = 256))]
    pub max_records: u16,
    #[schemars(range(min = 512, max = 1048576))]
    pub max_bytes: u32,
    #[schemars(range(min = 1, max = 2678400000_u64))]
    pub max_time_window_ms: u64,
}

impl LocalObservabilityProviderBindingV2 {
    fn validate(self) -> Result<(), ProviderRegistryV2Error> {
        if self.max_records == 0
            || self.max_records > MAX_OBSERVABILITY_QUERY_LIMIT
            || !(OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES..=MAX_OBSERVABILITY_QUERY_BYTES)
                .contains(&self.max_bytes)
            || self.max_time_window_ms == 0
            || self.max_time_window_ms > MAX_OBSERVABILITY_EXPORT_RANGE_MS
        {
            return Err(ProviderRegistryV2Error::BoundExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalTransportProviderBindingV2 {
    #[schemars(length(min = 1, max = 64))]
    pub transport_binding_ids: Vec<ProviderIntentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalSubstrateProviderBindingV2 {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalDisplayEndpointIdsV2 {
    pub wayland: ProviderIntentId,
    pub cross_domain: ProviderIntentId,
    pub waypipe: ProviderIntentId,
    pub proxy: ProviderIntentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalDisplayProviderBindingV2 {
    pub workload_id: WorkloadId,
    pub owner_role_id: RoleId,
    pub endpoint_ids: LocalDisplayEndpointIdsV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkProviderBindingV2 {
    pub network_id: ProviderIntentId,
    pub allocator_lease_id: ProviderIntentId,
    pub bridge_set_id: ProviderIntentId,
    pub tap_set_id: ProviderIntentId,
    pub net_vm_role_id: RoleId,
    pub nat_policy_id: ProviderIntentId,
    pub dhcp_policy_id: ProviderIntentId,
    pub nft_policy_id: ProviderIntentId,
    pub netlink_policy_id: ProviderIntentId,
    pub external_attachment_id: Option<ProviderIntentId>,
    pub resource_generation: Generation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageProviderBindingV2 {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub local_state_id: ProviderIntentId,
    pub disk_set_id: ProviderIntentId,
    pub store_view_id: ProviderIntentId,
    pub closure_sync_id: ProviderIntentId,
    pub media_set_id: ProviderIntentId,
    pub resource_generation: Generation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalDeviceProviderBindingV2 {
    #[schemars(length(max = 64))]
    pub device_resource_ids: Vec<ProviderIntentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalAudioProviderBindingV2 {
    pub workload_id: WorkloadId,
    pub role_id: RoleId,
    pub process_id: ProviderIntentId,
    pub endpoint_id: ProviderIntentId,
    pub state_storage_id: ProviderIntentId,
    pub lock_storage_id: ProviderIntentId,
    pub mediation_storage_id: ProviderIntentId,
    pub lease_id: ProviderIntentId,
}

fn validate_mapping_ids<'a>(
    ids: impl IntoIterator<Item = &'a ProviderIntentId>,
) -> Result<(), ProviderRegistryV2Error> {
    let ids = ids.into_iter().collect::<Vec<_>>();
    if ids.is_empty()
        || ids.len() > MAX_PROVIDER_MAPPING_IDS
        || ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len()
    {
        return Err(ProviderRegistryV2Error::BindingMismatch);
    }
    Ok(())
}

fn validate_optional_mapping_ids<'a>(
    ids: impl IntoIterator<Item = &'a ProviderIntentId>,
) -> Result<(), ProviderRegistryV2Error> {
    let ids = ids.into_iter().collect::<Vec<_>>();
    if ids.len() > MAX_PROVIDER_MAPPING_IDS
        || ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len()
    {
        return Err(ProviderRegistryV2Error::BindingMismatch);
    }
    Ok(())
}

fn validate_binding_realm(
    placement: &ProviderPlacement,
    binding_realm_id: &RealmId,
) -> Result<(), ProviderRegistryV2Error> {
    if placement.realm_id() == binding_realm_id {
        Ok(())
    } else {
        Err(ProviderRegistryV2Error::BindingMismatch)
    }
}

/// Closed wire binding with an extension-safe consumer surface.
///
/// Wire decoding remains strict: serde accepts only variants declared by the
/// current schema. Downstream consumers must nevertheless retain a fallback so
/// adding a declared variant does not silently activate behavior.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "axis", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProviderBindingV2 {
    LocalRuntime(LocalRuntimeProviderBindingV2),
    LocalObservability(LocalObservabilityProviderBindingV2),
    LocalTransport(LocalTransportProviderBindingV2),
    LocalSubstrate(LocalSubstrateProviderBindingV2),
    LocalDisplay(LocalDisplayProviderBindingV2),
    Network(NetworkProviderBindingV2),
    LocalStorage(StorageProviderBindingV2),
    LocalDevice(LocalDeviceProviderBindingV2),
    LocalAudio(LocalAudioProviderBindingV2),
}

/// Error returned for a binding without a registered consumer adapter.
///
/// An external exhaustive match does not compile:
///
/// ```compile_fail
/// use d2b_contracts::provider_registry_v2::ProviderBindingV2;
///
/// fn consume(binding: &ProviderBindingV2) {
///     match binding {
///         ProviderBindingV2::LocalRuntime(_) => {}
///         ProviderBindingV2::LocalObservability(_) => {}
///     }
/// }
/// ```
///
/// Extension-safe consumers use the registered view and retain a fallback:
///
/// ```
/// use d2b_contracts::provider_registry_v2::{
///     ProviderBindingV2, ProviderBindingV2ConsumerView, UnsupportedProviderBindingV2,
/// };
///
/// fn consume(binding: &ProviderBindingV2) -> Result<(), UnsupportedProviderBindingV2> {
///     match binding.consumer_view()? {
///         ProviderBindingV2ConsumerView::LocalRuntime(_) => Ok(()),
///         ProviderBindingV2ConsumerView::LocalObservability(_) => Ok(()),
///         _ => Err(UnsupportedProviderBindingV2),
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedProviderBindingV2;

impl fmt::Display for UnsupportedProviderBindingV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("provider binding has no registered consumer adapter")
    }
}

impl Error for UnsupportedProviderBindingV2 {}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderBindingV2ConsumerView<'a> {
    LocalRuntime(&'a LocalRuntimeProviderBindingV2),
    LocalObservability(&'a LocalObservabilityProviderBindingV2),
    LocalTransport(&'a LocalTransportProviderBindingV2),
    LocalSubstrate(&'a LocalSubstrateProviderBindingV2),
    LocalDisplay(&'a LocalDisplayProviderBindingV2),
    Network(&'a NetworkProviderBindingV2),
    LocalStorage(&'a StorageProviderBindingV2),
    LocalDevice(&'a LocalDeviceProviderBindingV2),
    LocalAudio(&'a LocalAudioProviderBindingV2),
}

impl ProviderBindingV2 {
    pub const fn provider_type(&self) -> ProviderType {
        match self {
            Self::LocalRuntime(_) => ProviderType::Runtime,
            Self::LocalObservability(_) => ProviderType::Observability,
            Self::LocalTransport(_) => ProviderType::Transport,
            Self::LocalSubstrate(_) => ProviderType::Substrate,
            Self::LocalDisplay(_) => ProviderType::Display,
            Self::Network(_) => ProviderType::Network,
            Self::LocalStorage(_) => ProviderType::Storage,
            Self::LocalDevice(_) => ProviderType::Device,
            Self::LocalAudio(_) => ProviderType::Audio,
        }
    }

    /// Returns the declared closed variants through a forward-compatible view.
    ///
    /// This does not register a daemon adapter. Downstream consumers still
    /// reject every view they do not explicitly handle. The wildcard remains
    /// reachable when a later wire variant precedes its consumer view.
    #[allow(unreachable_patterns)]
    pub const fn consumer_view(
        &self,
    ) -> Result<ProviderBindingV2ConsumerView<'_>, UnsupportedProviderBindingV2> {
        match self {
            Self::LocalRuntime(binding) => Ok(ProviderBindingV2ConsumerView::LocalRuntime(binding)),
            Self::LocalObservability(binding) => {
                Ok(ProviderBindingV2ConsumerView::LocalObservability(binding))
            }
            Self::LocalTransport(binding) => {
                Ok(ProviderBindingV2ConsumerView::LocalTransport(binding))
            }
            Self::LocalSubstrate(binding) => {
                Ok(ProviderBindingV2ConsumerView::LocalSubstrate(binding))
            }
            Self::LocalDisplay(binding) => Ok(ProviderBindingV2ConsumerView::LocalDisplay(binding)),
            Self::Network(binding) => Ok(ProviderBindingV2ConsumerView::Network(binding)),
            Self::LocalStorage(binding) => Ok(ProviderBindingV2ConsumerView::LocalStorage(binding)),
            Self::LocalDevice(binding) => Ok(ProviderBindingV2ConsumerView::LocalDevice(binding)),
            Self::LocalAudio(binding) => Ok(ProviderBindingV2ConsumerView::LocalAudio(binding)),
            _ => Err(UnsupportedProviderBindingV2),
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
        if !matches!(
            self.descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(ProviderRegistryV2Error::PlacementMismatch);
        }
        match &self.binding {
            ProviderBindingV2::LocalRuntime(binding) => {
                let realm_id = self.descriptor.placement.realm_id();
                let configured_provider_id = ConfiguredProviderId::parse(format!(
                    "runtime-{}",
                    binding.workload_id.as_str()
                ))
                .map_err(|_| ProviderRegistryV2Error::BindingMismatch)?;
                let expected_provider_id =
                    ProviderId::derive(realm_id, ProviderType::Runtime, &configured_provider_id);
                if self.descriptor.provider_id != expected_provider_id {
                    return Err(ProviderRegistryV2Error::ProviderIdMismatch);
                }
                if self.descriptor.configuration_schema_fingerprint
                    != local_runtime_configuration_schema_fingerprint()?
                {
                    return Err(ProviderRegistryV2Error::ConfigurationSchemaFingerprintMismatch);
                }
                if self.descriptor.configured_scope_digest
                    != local_runtime_configured_scope_digest(
                        &self.descriptor.provider_id,
                        realm_id,
                        binding,
                    )?
                {
                    return Err(ProviderRegistryV2Error::ConfiguredScopeDigestMismatch);
                }
            }
            ProviderBindingV2::LocalObservability(binding) => {
                binding.validate()?;
                let realm_id = self.descriptor.placement.realm_id();
                let configured_provider_id = ConfiguredProviderId::parse("observability-local")
                    .map_err(|_| ProviderRegistryV2Error::BindingMismatch)?;
                let expected_provider_id = ProviderId::derive(
                    realm_id,
                    ProviderType::Observability,
                    &configured_provider_id,
                );
                if self.descriptor.provider_id != expected_provider_id {
                    return Err(ProviderRegistryV2Error::ProviderIdMismatch);
                }
                if self.descriptor.configuration_schema_fingerprint
                    != local_observability_configuration_schema_fingerprint()?
                {
                    return Err(ProviderRegistryV2Error::ConfigurationSchemaFingerprintMismatch);
                }
                if self.descriptor.configured_scope_digest
                    != local_observability_configured_scope_digest(
                        &self.descriptor.provider_id,
                        binding,
                    )?
                {
                    return Err(ProviderRegistryV2Error::ConfiguredScopeDigestMismatch);
                }
            }
            ProviderBindingV2::LocalTransport(binding) => {
                validate_mapping_ids(&binding.transport_binding_ids)?;
            }
            ProviderBindingV2::LocalSubstrate(_) => {}
            ProviderBindingV2::LocalDisplay(binding) => {
                validate_mapping_ids([
                    &binding.endpoint_ids.wayland,
                    &binding.endpoint_ids.cross_domain,
                    &binding.endpoint_ids.waypipe,
                    &binding.endpoint_ids.proxy,
                ])?;
            }
            ProviderBindingV2::Network(binding) => {
                validate_mapping_ids(
                    [
                        &binding.network_id,
                        &binding.allocator_lease_id,
                        &binding.bridge_set_id,
                        &binding.tap_set_id,
                        &binding.nat_policy_id,
                        &binding.dhcp_policy_id,
                        &binding.nft_policy_id,
                        &binding.netlink_policy_id,
                    ]
                    .into_iter()
                    .chain(binding.external_attachment_id.as_ref()),
                )?;
            }
            ProviderBindingV2::LocalStorage(binding) => {
                validate_binding_realm(&self.descriptor.placement, &binding.realm_id)?;
                validate_mapping_ids([
                    &binding.local_state_id,
                    &binding.disk_set_id,
                    &binding.store_view_id,
                    &binding.closure_sync_id,
                    &binding.media_set_id,
                ])?;
            }
            ProviderBindingV2::LocalDevice(binding) => {
                validate_optional_mapping_ids(&binding.device_resource_ids)?;
            }
            ProviderBindingV2::LocalAudio(binding) => {
                validate_mapping_ids([
                    &binding.process_id,
                    &binding.endpoint_id,
                    &binding.state_storage_id,
                    &binding.lock_storage_id,
                    &binding.mediation_storage_id,
                    &binding.lease_id,
                ])?;
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
    realm_id: &RealmId,
    binding: &LocalRuntimeProviderBindingV2,
) -> Result<Fingerprint, ProviderRegistryV2Error> {
    let canonical_scope = format!(
        concat!(
            r#"{{"providerId":"{}","realmId":"{}","#,
            r#""runnerIntentId":"{}","vmStartIntentId":"{}","#,
            r#""workloadId":"{}"}}"#
        ),
        provider_id.as_str(),
        realm_id.as_str(),
        binding.runner_intent_id.as_str(),
        binding.vm_start_intent_id.as_str(),
        binding.workload_id.as_str(),
    );
    sha256_fingerprint(canonical_scope.as_bytes())
}

pub fn local_observability_configuration_schema_fingerprint()
-> Result<Fingerprint, ProviderRegistryV2Error> {
    sha256_fingerprint(LOCAL_OBSERVABILITY_CONFIGURATION_SCHEMA_SEED.as_bytes())
}

pub fn local_observability_configured_scope_digest(
    provider_id: &ProviderId,
    binding: &LocalObservabilityProviderBindingV2,
) -> Result<Fingerprint, ProviderRegistryV2Error> {
    let canonical_scope = format!(
        concat!(
            r#"{{"maxBytes":{},"maxRecords":{},"maxTimeWindowMs":{},"#,
            r#""providerId":"{}"}}"#
        ),
        binding.max_bytes,
        binding.max_records,
        binding.max_time_window_ms,
        provider_id.as_str(),
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
        v2_identity::{ConfiguredProviderId, ProviderType, RealmPath, RoleKind, WorkloadName},
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
            local_runtime_configured_scope_digest(&provider_id, &realm_id, &binding).unwrap();
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
        assert!(matches!(
            registry.providers[0].binding.consumer_view(),
            Ok(ProviderBindingV2ConsumerView::LocalRuntime(_))
        ));
        let encoded = serde_json::to_string(&registry).unwrap();
        assert!(!encoded.contains("argv"));
        assert!(!encoded.contains("secret"));
        let encoded_value = serde_json::to_value(&registry).unwrap();
        assert!(
            encoded_value["providers"][0]["binding"]
                .get("realmId")
                .is_none(),
            "the binding must not duplicate descriptor placement realm"
        );
        let decoded: ProviderRegistryV2 = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, registry);
    }

    #[test]
    fn validates_closed_local_observability_mapping() {
        let realm_id = RealmId::derive(&RealmPath::parse("home.local-root").unwrap());
        let provider_id = ProviderId::derive(
            &realm_id,
            ProviderType::Observability,
            &ConfiguredProviderId::parse("observability-local").unwrap(),
        );
        let binding = LocalObservabilityProviderBindingV2 {
            max_records: 64,
            max_bytes: 32_768,
            max_time_window_ms: 86_400_000,
        };
        let generation = Generation::new(1).unwrap();
        let registry = ProviderRegistryV2 {
            schema_version: PROVIDER_REGISTRY_V2_SCHEMA_VERSION.to_owned(),
            registry_generation: generation,
            configuration_fingerprint: Fingerprint::parse("1".repeat(64)).unwrap(),
            published_at_unix_ms: 0,
            providers: vec![ProviderRegistryEntryV2 {
                descriptor: ProviderDescriptor {
                    schema_version: PROVIDER_SCHEMA_VERSION,
                    provider_id: provider_id.clone(),
                    authority: ProviderAuthority::Observability,
                    implementation_id: ImplementationId::parse("local").unwrap(),
                    api_version: ProviderApiVersion::V2,
                    capabilities: ProviderCapabilitySet::new(
                        [
                            ProviderMethod::ObservabilityStatus,
                            ProviderMethod::ObservabilityQuery,
                            ProviderMethod::ObservabilityExport,
                        ]
                        .into_iter()
                        .map(ProviderCapability)
                        .collect(),
                    )
                    .unwrap(),
                    configuration_schema_fingerprint:
                        local_observability_configuration_schema_fingerprint().unwrap(),
                    configured_scope_digest: local_observability_configured_scope_digest(
                        &provider_id,
                        &binding,
                    )
                    .unwrap(),
                    registry_generation: generation,
                    placement: ProviderPlacement::TrustedFirstPartyInProcess {
                        realm_id,
                        controller_role: EndpointRole::LocalRootController,
                    },
                },
                binding: ProviderBindingV2::LocalObservability(binding),
            }],
        };

        registry.validate().unwrap();
        assert!(matches!(
            registry.providers[0].binding.consumer_view(),
            Ok(ProviderBindingV2ConsumerView::LocalObservability(_))
        ));
        let encoded = serde_json::to_value(&registry).unwrap();
        let binding = &encoded["providers"][0]["binding"];
        assert_eq!(binding["axis"], "local-observability");
        assert_eq!(binding["maxRecords"], 64);
        assert_eq!(binding["maxBytes"], 32_768);
        assert_eq!(binding["maxTimeWindowMs"], 86_400_000);
        assert!(binding.get("realmId").is_none());
        assert!(binding.get("workloadId").is_none());
        assert!(binding.get("providerId").is_none());
    }

    #[test]
    fn serializes_declared_mapping_axes_as_closed_variants() {
        let realm_id = RealmId::derive(&RealmPath::parse("home.local-root").unwrap());
        let workload_id = WorkloadId::derive(&realm_id, &WorkloadName::parse("desktop").unwrap());
        let role_id = RoleId::derive(&realm_id, &workload_id, RoleKind::CloudHypervisor);
        let intent = |value: &str| ProviderIntentId::parse(value).unwrap();
        let generation = Generation::new(1).unwrap();
        let bindings = vec![
            ProviderBindingV2::LocalTransport(LocalTransportProviderBindingV2 {
                transport_binding_ids: vec![intent("binding-public")],
            }),
            ProviderBindingV2::LocalSubstrate(LocalSubstrateProviderBindingV2 {}),
            ProviderBindingV2::LocalDisplay(LocalDisplayProviderBindingV2 {
                workload_id: workload_id.clone(),
                owner_role_id: role_id.clone(),
                endpoint_ids: LocalDisplayEndpointIdsV2 {
                    wayland: intent("endpoint-wayland"),
                    cross_domain: intent("endpoint-cross-domain"),
                    waypipe: intent("endpoint-waypipe"),
                    proxy: intent("endpoint-proxy"),
                },
            }),
            ProviderBindingV2::Network(NetworkProviderBindingV2 {
                network_id: intent("network-home"),
                allocator_lease_id: intent("lease-network-home"),
                bridge_set_id: intent("network-bridges"),
                tap_set_id: intent("network-taps"),
                net_vm_role_id: role_id.clone(),
                nat_policy_id: intent("network-nat"),
                dhcp_policy_id: intent("network-dhcp"),
                nft_policy_id: intent("network-nft"),
                netlink_policy_id: intent("network-netlink"),
                external_attachment_id: None,
                resource_generation: generation,
            }),
            ProviderBindingV2::LocalStorage(StorageProviderBindingV2 {
                realm_id,
                workload_id: workload_id.clone(),
                local_state_id: intent("storage-state"),
                disk_set_id: intent("storage-disks"),
                store_view_id: intent("storage-store-view"),
                closure_sync_id: intent("storage-closure-sync"),
                media_set_id: intent("storage-media"),
                resource_generation: generation,
            }),
            ProviderBindingV2::LocalDevice(LocalDeviceProviderBindingV2 {
                device_resource_ids: vec![intent("device-tpm"), intent("device-gpu")],
            }),
            ProviderBindingV2::LocalAudio(LocalAudioProviderBindingV2 {
                workload_id,
                role_id,
                process_id: intent("audio-process"),
                endpoint_id: intent("audio-endpoint"),
                state_storage_id: intent("audio-state"),
                lock_storage_id: intent("audio-lock"),
                mediation_storage_id: intent("audio-mediation"),
                lease_id: intent("audio-lease"),
            }),
        ];

        let expected_axes = [
            "local-transport",
            "local-substrate",
            "local-display",
            "network",
            "local-storage",
            "local-device",
            "local-audio",
        ];
        let expected_types = [
            ProviderType::Transport,
            ProviderType::Substrate,
            ProviderType::Display,
            ProviderType::Network,
            ProviderType::Storage,
            ProviderType::Device,
            ProviderType::Audio,
        ];
        for ((binding, expected_axis), expected_type) in
            bindings.into_iter().zip(expected_axes).zip(expected_types)
        {
            assert_eq!(binding.provider_type(), expected_type);
            assert!(binding.consumer_view().is_ok());
            let encoded = serde_json::to_value(&binding).unwrap();
            assert_eq!(encoded["axis"], expected_axis);
            if expected_axis == "local-storage" {
                assert!(encoded.get("realmId").is_some());
            } else {
                assert!(encoded.get("realmId").is_none());
            }
            assert!(encoded.get("argv").is_none());
            assert!(encoded.get("path").is_none());
            let decoded: ProviderBindingV2 = serde_json::from_value(encoded).unwrap();
            assert_eq!(decoded, binding);
        }
    }

    #[test]
    fn rejects_duplicate_or_unbounded_mapping_ids() {
        let duplicate = ProviderIntentId::parse("duplicate").unwrap();
        assert_eq!(
            validate_mapping_ids([&duplicate, &duplicate]),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );
        let too_many = (0..=MAX_PROVIDER_MAPPING_IDS)
            .map(|index| ProviderIntentId::parse(format!("mapping-{index}")).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            validate_mapping_ids(&too_many),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );
        assert_eq!(validate_optional_mapping_ids([]), Ok(()));
        assert_eq!(
            validate_optional_mapping_ids([&duplicate, &duplicate]),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );
        assert_eq!(
            validate_optional_mapping_ids(&too_many),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );
    }

    #[test]
    fn local_storage_binding_realm_must_match_descriptor_placement() {
        let descriptor_realm = RealmId::derive(&RealmPath::parse("home.local-root").unwrap());
        let binding_realm = RealmId::derive(&RealmPath::parse("work.local-root").unwrap());
        let placement = ProviderPlacement::TrustedFirstPartyInProcess {
            realm_id: descriptor_realm.clone(),
            controller_role: EndpointRole::RealmController,
        };

        assert_eq!(
            validate_binding_realm(&placement, &descriptor_realm),
            Ok(())
        );
        assert_eq!(
            validate_binding_realm(&placement, &binding_realm),
            Err(ProviderRegistryV2Error::BindingMismatch)
        );
    }

    #[test]
    fn rejects_generation_and_exact_identity_mismatches() {
        let mut registry = fixture();
        registry.providers[0].descriptor.registry_generation = Generation::new(2).unwrap();
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::GenerationMismatch)
        );

        let mut registry = fixture();
        let other = RealmId::derive(&RealmPath::parse("other.local-root").unwrap());
        registry.providers[0].descriptor.provider_id = ProviderId::derive(
            &other,
            ProviderType::Runtime,
            &ConfiguredProviderId::parse("runtime-other").unwrap(),
        );
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::ProviderIdMismatch)
        );

        let mut registry = fixture();
        registry.providers[0]
            .descriptor
            .configuration_schema_fingerprint = Fingerprint::parse("4".repeat(64)).unwrap();
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::ConfigurationSchemaFingerprintMismatch)
        );

        let mut registry = fixture();
        let ProviderBindingV2::LocalRuntime(binding) = &mut registry.providers[0].binding else {
            unreachable!("fixture is a local runtime binding");
        };
        binding.runner_intent_id =
            ProviderIntentId::parse("runner:vm:other:role:cloud-hypervisor").unwrap();
        assert_eq!(
            registry.validate(),
            Err(ProviderRegistryV2Error::ConfiguredScopeDigestMismatch)
        );
    }

    #[test]
    fn contradictory_binding_realm_json_is_unrepresentable() {
        let mut encoded = serde_json::to_value(fixture()).unwrap();
        encoded["providers"][0]["binding"]["realmId"] =
            serde_json::Value::String("contradictory-realm".to_owned());
        assert!(serde_json::from_value::<ProviderRegistryV2>(encoded).is_err());
    }

    #[test]
    fn unknown_binding_axis_remains_rejected_on_the_wire() {
        let encoded = serde_json::json!({
            "axis": "future-network",
            "resourceId": "opaque"
        });
        assert!(serde_json::from_value::<ProviderBindingV2>(encoded).is_err());
        assert_eq!(
            UnsupportedProviderBindingV2.to_string(),
            "provider binding has no registered consumer adapter"
        );
    }

    #[test]
    fn identity_mismatch_messages_name_the_failed_contract() {
        assert_eq!(
            ProviderRegistryV2Error::ProviderIdMismatch.to_string(),
            "provider ID does not match descriptor placement and workload binding"
        );
        assert_eq!(
            ProviderRegistryV2Error::ConfigurationSchemaFingerprintMismatch.to_string(),
            "configuration schema fingerprint does not match the first-party provider contract"
        );
        assert_eq!(
            ProviderRegistryV2Error::ConfiguredScopeDigestMismatch.to_string(),
            "configured scope digest does not match the closed provider binding"
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
