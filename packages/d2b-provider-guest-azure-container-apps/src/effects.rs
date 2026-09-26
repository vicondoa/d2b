//! Azure Container Apps effect contracts.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_provider::v3::credential::{CredentialLeaseHandle, OpaqueAzureRef};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};

/// Maximum length of an ACA resource identifier.
pub const MAX_ACA_RESOURCE_ID_LEN: usize = 60;
/// Maximum number of sandbox or disk image candidates accepted from a control plane query.
pub const MAX_ACA_CANDIDATES: usize = 8;
/// Maximum readiness attempts before a sandbox generation is marked failed.
pub const MAX_ACA_READY_ATTEMPTS: u8 = 60;
/// Maximum readiness probe interval in milliseconds.
pub const MAX_ACA_READY_INTERVAL_MS: u32 = 10_000;
/// Maximum plan time-to-live in milliseconds.
pub const MAX_ACA_PLAN_TTL_MS: u32 = 300_000;
/// Maximum completed operations retained by the ledger.
pub const MAX_ACA_COMPLETED_OPERATIONS: usize = 1_024;

/// Errors produced by validated ACA type constructors.
///
/// Each variant renders as a stable error code via `Display`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaTypeError {
    /// The identifier violates the opaque-id character or length rules.
    InvalidIdentifier,
    /// A CPU or memory bound falls outside the validated range.
    InvalidResourceBounds,
    /// The readiness policy violates the attempt or interval bounds.
    InvalidReadinessPolicy,
    /// The plan TTL is zero or exceeds [`MAX_ACA_PLAN_TTL_MS`].
    InvalidPlanTtl,
    /// The completed-operation capacity is zero or exceeds [`MAX_ACA_COMPLETED_OPERATIONS`].
    InvalidOperationCapacity,
    /// The candidate list exceeds [`MAX_ACA_CANDIDATES`].
    CandidateBoundExceeded,
    /// A reference points at a resource type outside the execution boundary.
    InvalidExecutionBoundary,
}

impl fmt::Display for AcaTypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => "aca-invalid-identifier",
            Self::InvalidResourceBounds => "aca-invalid-resource-bounds",
            Self::InvalidReadinessPolicy => "aca-invalid-readiness-policy",
            Self::InvalidPlanTtl => "aca-invalid-plan-ttl",
            Self::InvalidOperationCapacity => "aca-invalid-operation-capacity",
            Self::CandidateBoundExceeded => "aca-candidate-bound-exceeded",
            Self::InvalidExecutionBoundary => "aca-invalid-execution-boundary",
        })
    }
}

impl std::error::Error for AcaTypeError {}

fn valid_opaque_id(value: &str, max: usize, lowercase_lead: bool) -> bool {
    !value.is_empty()
        && value.len() <= max
        && (!lowercase_lead || value.as_bytes()[0].is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

macro_rules! opaque_id {
    ($name:ident, $max:expr, $lowercase_lead:expr) => {
        /// Opaque identifier validated against the ACA character rules.
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parse and validate an opaque identifier.
            ///
            /// # Errors
            ///
            /// Returns [`AcaTypeError::InvalidIdentifier`] when the value is empty,
            /// exceeds the length bound, or contains a disallowed character.
            pub fn parse(value: impl Into<String>) -> Result<Self, AcaTypeError> {
                let value = value.into();
                if valid_opaque_id(&value, $max, $lowercase_lead) {
                    Ok(Self(value))
                } else {
                    Err(AcaTypeError::InvalidIdentifier)
                }
            }

            /// Borrow the validated identifier text.
            pub fn as_str(&self) -> &str {
                &self.0
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

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}

opaque_id!(AcaProfileId, 64, true);
opaque_id!(AcaConfiguredDiskId, 64, true);
opaque_id!(AcaConfiguredImageId, 64, true);
opaque_id!(AcaDiskImageName, 64, true);
opaque_id!(AcaManagedIdentityBindingId, 64, true);
opaque_id!(AcaSandboxId, MAX_ACA_RESOURCE_ID_LEN, false);
opaque_id!(AcaDiskImageId, MAX_ACA_RESOURCE_ID_LEN, false);
opaque_id!(AcaOperationId, 96, true);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "u16")]
/// CPU allocation in millicores, validated to the 250..=4_000 range in 250 increments.
pub struct AcaCpuMillis(u16);

impl AcaCpuMillis {
    /// Construct validated CPU millicores.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidResourceBounds`] when `value` is outside
    /// 250..=4_000 or not a multiple of 250.
    pub fn new(value: u16) -> Result<Self, AcaTypeError> {
        if (250..=4_000).contains(&value) && value.is_multiple_of(250) {
            Ok(Self(value))
        } else {
            Err(AcaTypeError::InvalidResourceBounds)
        }
}
}
 
    impl TryFrom<u16> for AcaCpuMillis {
    type Error = AcaTypeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "u32")]
/// Memory allocation in MiB, validated to the 512..=16_384 range in 256 increments.
pub struct AcaMemoryMib(u32);

impl AcaMemoryMib {
    /// Construct validated memory MiB.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidResourceBounds`] when `value` is outside
    /// 512..=16_384 or not a multiple of 256.
    pub fn new(value: u32) -> Result<Self, AcaTypeError> {
        if (512..=16_384).contains(&value) && value.is_multiple_of(256) {
            Ok(Self(value))
        } else {
            Err(AcaTypeError::InvalidResourceBounds)
        }
}
}
 
    impl TryFrom<u32> for AcaMemoryMib {
    type Error = AcaTypeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Source of a disk image: a configured disk or a configured container image.
pub enum AcaDiskImageSource {
    /// A pre-configured disk binding.
    ConfiguredDisk {
        /// The configured disk binding id.
        binding_id: AcaConfiguredDiskId,
    },
    /// A container image pulled by an optional managed identity.
    ConfiguredContainerImage {
        /// The image binding id.
        image_binding_id: AcaConfiguredImageId,
        /// The disk name the image is materialized as.
        disk_name: AcaDiskImageName,
        /// The managed identity used for the pull, when one is configured.
        pull_identity_binding_id: Option<AcaManagedIdentityBindingId>,
    },
}

impl fmt::Debug for AcaDiskImageSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConfiguredDisk { .. } => "AcaDiskImageSource::ConfiguredDisk(<redacted>)",
            Self::ConfiguredContainerImage { .. } => {
                "AcaDiskImageSource::ConfiguredContainerImage(<redacted>)"
            }
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawAcaSandboxProfile")]
/// Validated sandbox profile: identity, disk image, CPU, memory, and suspend policy.
pub struct AcaSandboxProfile {
    profile_id: AcaProfileId,
    disk_image: AcaDiskImageSource,
    cpu: AcaCpuMillis,
    memory: AcaMemoryMib,
    auto_suspend_secs: u32,
    sandbox_identity_binding_id: Option<AcaManagedIdentityBindingId>,
}

impl AcaSandboxProfile {
    #[allow(clippy::too_many_arguments)]
    /// Construct a validated sandbox profile.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidResourceBounds`] when `auto_suspend_secs`
    /// is outside 60..=86_400.
    pub fn new(
        profile_id: AcaProfileId,
        disk_image: AcaDiskImageSource,
        cpu: AcaCpuMillis,
        memory: AcaMemoryMib,
        auto_suspend_secs: u32,
        sandbox_identity_binding_id: Option<AcaManagedIdentityBindingId>,
    ) -> Result<Self, AcaTypeError> {
        if !(60..=86_400).contains(&auto_suspend_secs) {
            return Err(AcaTypeError::InvalidResourceBounds);
        }
        Ok(Self {
            profile_id,
            disk_image,
            cpu,
            memory,
            auto_suspend_secs,
            sandbox_identity_binding_id,
        })
    }

    /// Borrow the profile id.
    pub fn profile_id(&self) -> &AcaProfileId {
        &self.profile_id
    }

    /// Borrow the disk image source.
    pub fn disk_image(&self) -> &AcaDiskImageSource {
        &self.disk_image
    }

    
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAcaSandboxProfile {
    profile_id: AcaProfileId,
    disk_image: AcaDiskImageSource,
    cpu: AcaCpuMillis,
    memory: AcaMemoryMib,
    auto_suspend_secs: u32,
    sandbox_identity_binding_id: Option<AcaManagedIdentityBindingId>,
}

impl TryFrom<RawAcaSandboxProfile> for AcaSandboxProfile {
    type Error = AcaTypeError;

    fn try_from(raw: RawAcaSandboxProfile) -> Result<Self, Self::Error> {
        Self::new(
            raw.profile_id,
            raw.disk_image,
            raw.cpu,
            raw.memory,
            raw.auto_suspend_secs,
            raw.sandbox_identity_binding_id,
        )
    }
}

impl fmt::Debug for AcaSandboxProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaSandboxProfile")
            .field("profile_id", &"<redacted>")
            .field("disk_image", &self.disk_image)
            .field("cpu", &self.cpu)
            .field("memory", &self.memory)
            .field("auto_suspend_secs", &self.auto_suspend_secs)
            .field(
                "sandbox_identity",
                &self
                    .sandbox_identity_binding_id
                    .as_ref()
                    .map(|_| "<configured>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawAcaReadinessPolicy")]
/// Validated readiness policy: bounded attempts and probe interval.
pub struct AcaReadinessPolicy {
    attempts: u8,
    interval_ms: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAcaReadinessPolicy {
    attempts: u8,
    interval_ms: u32,
}

impl TryFrom<RawAcaReadinessPolicy> for AcaReadinessPolicy {
    type Error = AcaTypeError;

    fn try_from(raw: RawAcaReadinessPolicy) -> Result<Self, Self::Error> {
        Self::new(raw.attempts, raw.interval_ms)
    }
}

impl AcaReadinessPolicy {
    /// Construct a validated readiness policy.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidReadinessPolicy`] when `attempts` or
    /// `interval_ms` is zero or exceeds the [`MAX_ACA_READY_ATTEMPTS`] /
    /// [`MAX_ACA_READY_INTERVAL_MS`] bounds.
    pub fn new(attempts: u8, interval_ms: u32) -> Result<Self, AcaTypeError> {
        if attempts == 0
            || attempts > MAX_ACA_READY_ATTEMPTS
            || interval_ms == 0
            || interval_ms > MAX_ACA_READY_INTERVAL_MS
        {
            return Err(AcaTypeError::InvalidReadinessPolicy);
        }
        Ok(Self {
            attempts,
            interval_ms,
        })
    }

    /// Return the readiness attempt bound.
    pub const fn attempts(self) -> u8 {
        self.attempts
    }

    /// Return the readiness probe interval in milliseconds.
    pub const fn interval_ms(self) -> u32 {
        self.interval_ms
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawAcaRuntimeConfig")]
/// Validated runtime configuration: profile, readiness, plan TTL, and ledger capacity.
pub struct AcaRuntimeConfig {
    profile: AcaSandboxProfile,
    readiness: AcaReadinessPolicy,
    plan_ttl_ms: u32,
    completed_operation_capacity: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAcaRuntimeConfig {
    profile: AcaSandboxProfile,
    readiness: AcaReadinessPolicy,
    plan_ttl_ms: u32,
    completed_operation_capacity: usize,
}

impl TryFrom<RawAcaRuntimeConfig> for AcaRuntimeConfig {
    type Error = AcaTypeError;

    fn try_from(raw: RawAcaRuntimeConfig) -> Result<Self, Self::Error> {
        Self::new(
            raw.profile,
            raw.readiness,
            raw.plan_ttl_ms,
            raw.completed_operation_capacity,
        )
    }
}

impl AcaRuntimeConfig {
    /// Construct a validated runtime configuration.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidPlanTtl`] when `plan_ttl_ms` is zero or
    /// exceeds [`MAX_ACA_PLAN_TTL_MS`], and [`AcaTypeError::InvalidOperationCapacity`]
    /// when `completed_operation_capacity` is zero or exceeds
    /// [`MAX_ACA_COMPLETED_OPERATIONS`].
    pub fn new(
        profile: AcaSandboxProfile,
        readiness: AcaReadinessPolicy,
        plan_ttl_ms: u32,
        completed_operation_capacity: usize,
    ) -> Result<Self, AcaTypeError> {
        if plan_ttl_ms == 0 || plan_ttl_ms > MAX_ACA_PLAN_TTL_MS {
            return Err(AcaTypeError::InvalidPlanTtl);
        }
        if completed_operation_capacity == 0
            || completed_operation_capacity > MAX_ACA_COMPLETED_OPERATIONS
        {
            return Err(AcaTypeError::InvalidOperationCapacity);
        }
        Ok(Self {
            profile,
            readiness,
            plan_ttl_ms,
            completed_operation_capacity,
        })
    }

    /// Borrow the sandbox profile.
    pub fn profile(&self) -> &AcaSandboxProfile {
        &self.profile
    }

    /// Return the readiness policy.
    pub const fn readiness(&self) -> AcaReadinessPolicy {
        self.readiness
    }

    /// Return the plan time-to-live in milliseconds.
    pub const fn plan_ttl_ms(&self) -> u32 {
        self.plan_ttl_ms
    }

    /// Return the completed-operation ledger capacity.
    pub const fn completed_operation_capacity(&self) -> usize {
        self.completed_operation_capacity
    }
}

impl fmt::Debug for AcaRuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaRuntimeConfig")
            .field("profile", &self.profile)
            .field("readiness", &self.readiness)
            .field("plan_ttl_ms", &self.plan_ttl_ms)
            .field(
                "completed_operation_capacity",
                &self.completed_operation_capacity,
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawAcaProviderConfig")]
/// Validated Provider configuration for the runtime-azure-container-apps provider.
pub struct AcaProviderConfig {
    /// Reference to the Guest execution boundary this provider serves.
    gateway_execution_ref: ResourceRef,
    /// Azure tenant id.
    tenant_id: OpaqueAzureRef,
    /// Azure client id.
    client_id: OpaqueAzureRef,
    /// Azure subscription id.
    subscription_id: OpaqueAzureRef,
    /// Reference to the credential used to acquire control-plane leases.
    control_credential_ref: ResourceRef,
    /// Reference to the credential used to pull sandbox images, when configured.
    pull_credential_ref: Option<ResourceRef>,
    /// Configured container-apps environment id.
    environment_id: AcaConfiguredImageId,
    /// Configured resource group id.
    resource_group_id: AcaConfiguredImageId,
    /// Reference to the network the sandbox joins, when configured.
    network_ref: Option<ResourceRef>,
    /// Profile alias used for the sandbox transport.
    sandbox_transport_alias: AcaProfileId,
    /// Runtime defaults applied to every controller created from this config.
    defaults: AcaRuntimeConfig,
}

impl AcaProviderConfig {
    /// Construct a validated Provider configuration.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidExecutionBoundary`] when a reference
    /// points outside the execution boundary (Guest gateway, Credential
    /// control, optional Credential pull, optional Network).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gateway_execution_ref: ResourceRef,
        tenant_id: OpaqueAzureRef,
        client_id: OpaqueAzureRef,
        subscription_id: OpaqueAzureRef,
        control_credential_ref: ResourceRef,
        pull_credential_ref: Option<ResourceRef>,
        environment_id: AcaConfiguredImageId,
        resource_group_id: AcaConfiguredImageId,
        network_ref: Option<ResourceRef>,
        sandbox_transport_alias: AcaProfileId,
        defaults: AcaRuntimeConfig,
    ) -> Result<Self, AcaTypeError> {
        Self::validate_refs(
            &gateway_execution_ref,
            &control_credential_ref,
            &pull_credential_ref,
            &network_ref,
        )?;
        Ok(Self {
            gateway_execution_ref,
            tenant_id,
            client_id,
            subscription_id,
            control_credential_ref,
            pull_credential_ref,
            environment_id,
            resource_group_id,
            network_ref,
            sandbox_transport_alias,
            defaults,
        })
    }

    /// Revalidate a Provider configuration at the admission boundary.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::InvalidExecutionBoundary`] when a reference
    /// points outside the execution boundary.
    pub fn validate(&self) -> Result<(), AcaTypeError> {
        Self::validate_refs(
            &self.gateway_execution_ref,
            &self.control_credential_ref,
            &self.pull_credential_ref,
            &self.network_ref,
        )
    }

    /// Borrow the Guest execution boundary reference.
    pub fn gateway_execution_ref(&self) -> &ResourceRef {
        &self.gateway_execution_ref
    }

    /// Borrow the Azure tenant id.
    pub fn tenant_id(&self) -> &OpaqueAzureRef {
        &self.tenant_id
    }

    /// Borrow the Azure client id.
    pub fn client_id(&self) -> &OpaqueAzureRef {
        &self.client_id
    }

    /// Borrow the Azure subscription id.
    pub fn subscription_id(&self) -> &OpaqueAzureRef {
        &self.subscription_id
    }

    /// Borrow the control credential reference.
    pub fn control_credential_ref(&self) -> &ResourceRef {
        &self.control_credential_ref
    }

    /// Borrow the pull credential reference, when configured.
    pub fn pull_credential_ref(&self) -> Option<&ResourceRef> {
        self.pull_credential_ref.as_ref()
    }

    /// Borrow the container-apps environment id.
    pub fn environment_id(&self) -> &AcaConfiguredImageId {
        &self.environment_id
    }

    /// Borrow the resource group id.
    pub fn resource_group_id(&self) -> &AcaConfiguredImageId {
        &self.resource_group_id
    }

    /// Borrow the network reference, when configured.
    pub fn network_ref(&self) -> Option<&ResourceRef> {
        self.network_ref.as_ref()
    }

    /// Borrow the sandbox transport profile alias.
    pub fn sandbox_transport_alias(&self) -> &AcaProfileId {
        &self.sandbox_transport_alias
    }

    /// Borrow the runtime defaults applied to every controller created from this config.
    pub fn defaults(&self) -> &AcaRuntimeConfig {
        &self.defaults
    }

    fn validate_refs(
        gateway_execution_ref: &ResourceRef,
        control_credential_ref: &ResourceRef,
        pull_credential_ref: &Option<ResourceRef>,
        network_ref: &Option<ResourceRef>,
    ) -> Result<(), AcaTypeError> {
        if gateway_execution_ref.resource_type().as_str() != "Guest"
            || control_credential_ref.resource_type().as_str() != "Credential"
            || pull_credential_ref
                .as_ref()
                .is_some_and(|reference| reference.resource_type().as_str() != "Credential")
            || network_ref
                .as_ref()
                .is_some_and(|reference| reference.resource_type().as_str() != "Network")
        {
            return Err(AcaTypeError::InvalidExecutionBoundary);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAcaProviderConfig {
    gateway_execution_ref: ResourceRef,
    tenant_id: OpaqueAzureRef,
    client_id: OpaqueAzureRef,
    subscription_id: OpaqueAzureRef,
    control_credential_ref: ResourceRef,
    pull_credential_ref: Option<ResourceRef>,
    environment_id: AcaConfiguredImageId,
    resource_group_id: AcaConfiguredImageId,
    network_ref: Option<ResourceRef>,
    sandbox_transport_alias: AcaProfileId,
    defaults: AcaRuntimeConfig,
}

impl TryFrom<RawAcaProviderConfig> for AcaProviderConfig {
    type Error = AcaTypeError;

    fn try_from(raw: RawAcaProviderConfig) -> Result<Self, Self::Error> {
        Self::new(
            raw.gateway_execution_ref,
            raw.tenant_id,
            raw.client_id,
            raw.subscription_id,
            raw.control_credential_ref,
            raw.pull_credential_ref,
            raw.environment_id,
            raw.resource_group_id,
            raw.network_ref,
            raw.sandbox_transport_alias,
            raw.defaults,
        )
    }
}

impl fmt::Debug for AcaProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaProviderConfig")
            .field("gateway_execution_ref", &"<redacted>")
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field("subscription_id", &self.subscription_id)
            .field("control_credential_ref", &"<redacted>")
            .field(
                "pull_credential_ref",
                &self.pull_credential_ref.as_ref().map(|_| "<configured>"),
            )
            .field("environment_id", &self.environment_id)
            .field("resource_group_id", &self.resource_group_id)
            .field(
                "network_ref",
                &self.network_ref.as_ref().map(|_| "<configured>"),
            )
            .field("sandbox_transport_alias", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Identity of the Guest resource a controller reconciles.
pub struct AcaResourceBinding {
    /// The Guest resource uid.
    pub guest_uid: ResourceUid,
    /// The provider generation the binding was created for.
    pub provider_generation: u64,
    /// Fingerprint of the config the binding was created from.
    pub config_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Query describing which sandboxes belong to a Guest.
pub struct AcaWorkloadQuery {
    /// The Guest binding the query scopes to.
    pub binding: AcaResourceBinding,
    /// The profile alias the sandbox must carry.
    pub profile_id: AcaProfileId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Desired disk image for a sandbox.
pub struct AcaDesiredDiskImage {
    /// The image source.
    pub source: AcaDiskImageSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Desired sandbox state passed to the create effect.
pub struct AcaDesiredSandbox {
    /// The Guest binding the sandbox belongs to.
    pub binding: AcaResourceBinding,
    /// The validated profile to apply.
    pub profile: AcaSandboxProfile,
    /// The disk image record the sandbox boots from.
    pub disk_image: AcaDiskImageRecord,
    /// The network to join, when configured.
    pub network_ref: Option<ResourceRef>,
    /// The sandbox transport profile alias.
    pub sandbox_transport_alias: AcaProfileId,
}

#[derive(Clone, PartialEq, Eq)]
/// Observed disk image identity and generation.
pub struct AcaDiskImageRecord {
    /// The disk image id.
    pub id: AcaDiskImageId,
    /// The generation the image was created for.
    pub generation: u64,
}

impl fmt::Debug for AcaDiskImageRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaDiskImageRecord")
            .field("id", &"<redacted>")
            .field("generation", &self.generation)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Bounded candidate list of disk images returned by a control plane query.
pub struct AcaDiskImageCandidates(Vec<AcaDiskImageRecord>);

impl AcaDiskImageCandidates {
    /// Construct a bounded candidate list.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::CandidateBoundExceeded`] when `records` is
    /// longer than [`MAX_ACA_CANDIDATES`].
    pub fn new(records: Vec<AcaDiskImageRecord>) -> Result<Self, AcaTypeError> {
        if records.len() > MAX_ACA_CANDIDATES {
            return Err(AcaTypeError::CandidateBoundExceeded);
        }
        Ok(Self(records))
    }

    /// Borrow the candidate records.
    pub fn as_slice(&self) -> &[AcaDiskImageRecord] {
        &self.0
    }
}

impl IntoIterator for AcaDiskImageCandidates {
    type Item = AcaDiskImageRecord;
    type IntoIter = std::vec::IntoIter<AcaDiskImageRecord>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Observed lifecycle of an ACA sandbox.
pub enum AcaSandboxLifecycle {
    /// The sandbox is being created.
    Creating,
    /// The sandbox is running.
    Running,
    /// The sandbox is suspended.
    Suspended,
    /// The sandbox is stopping.
    Stopping,
    /// The sandbox is stopped.
    Stopped,
    /// The sandbox failed.
    Failed,
    /// The lifecycle could not be determined.
    Unknown,
}

#[derive(Clone, PartialEq, Eq)]
/// Observed sandbox state returned by control plane effects.
pub struct AcaSandboxRecord {
    /// The sandbox id.
    pub id: AcaSandboxId,
    /// The observed lifecycle.
    pub lifecycle: AcaSandboxLifecycle,
    /// The generation the sandbox was created for.
    pub generation: u64,
}

impl fmt::Debug for AcaSandboxRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaSandboxRecord")
            .field("id", &"<redacted>")
            .field("lifecycle", &self.lifecycle)
            .field("generation", &self.generation)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Bounded candidate list of sandboxes returned by a control plane query.
pub struct AcaSandboxCandidates(Vec<AcaSandboxRecord>);

impl AcaSandboxCandidates {
    /// Construct a bounded candidate list.
    ///
    /// # Errors
    ///
    /// Returns [`AcaTypeError::CandidateBoundExceeded`] when `records` is
    /// longer than [`MAX_ACA_CANDIDATES`].
    pub fn new(records: Vec<AcaSandboxRecord>) -> Result<Self, AcaTypeError> {
        if records.len() > MAX_ACA_CANDIDATES {
            return Err(AcaTypeError::CandidateBoundExceeded);
        }
        Ok(Self(records))
    }

    /// Borrow the candidate records.
    pub fn as_slice(&self) -> &[AcaSandboxRecord] {
        &self.0
    }
}

impl IntoIterator for AcaSandboxCandidates {
    type Item = AcaSandboxRecord;
    type IntoIter = std::vec::IntoIter<AcaSandboxRecord>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Outcome of a delete effect.
pub enum AcaDeleteOutcome {
    /// The sandbox was deleted.
    Deleted,
    /// The sandbox was already absent.
    AlreadyAbsent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Purpose of a credential lease; drives lease acquisition policy.
pub enum AcaCredentialPurpose {
    /// Health probe.
    Health,
    /// Sandbox or disk image ensure.
    Ensure,
    /// Sandbox resume.
    Start,
    /// Sandbox stop.
    Stop,
    /// Sandbox inspection.
    Inspect,
    /// Sandbox adoption.
    Adopt,
    /// Sandbox destroy.
    Destroy,
}



#[derive(Clone, PartialEq, Eq)]
/// A lease on the control-plane credential, valid until an absolute expiry.
pub struct AcaCredentialLease {
    metadata: CredentialLeaseHandle,
    expires_at_unix_ms: u64,
}

impl AcaCredentialLease {
    /// Construct a lease from credential metadata and an absolute expiry.
    pub fn from_metadata(metadata: CredentialLeaseHandle, expires_at_unix_ms: u64) -> Self {
        Self {
            metadata,
            expires_at_unix_ms,
        }
    }

    /// Return the absolute lease expiry in unix milliseconds.
    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

impl fmt::Debug for AcaCredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaCredentialLease")
            .field("metadata", &"<opaque>")
            .field("expires_at_unix_ms", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
/// Request for a credential lease with a requested absolute expiry.
pub struct AcaCredentialLeaseRequest {
    operation_id: AcaOperationId,
    purpose: AcaCredentialPurpose,
    requested_expiry_unix_ms: u64,
}

impl AcaCredentialLeaseRequest {
    /// Construct a lease request.
    pub fn new(
        operation_id: AcaOperationId,
        purpose: AcaCredentialPurpose,
        requested_expiry_unix_ms: u64,
    ) -> Self {
        Self {
            operation_id,
            purpose,
            requested_expiry_unix_ms,
        }
    }

    /// Return the requested expiry in unix milliseconds.
    pub const fn requested_expiry_unix_ms(&self) -> u64 {
        self.requested_expiry_unix_ms
    }
}

impl fmt::Debug for AcaCredentialLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaCredentialLeaseRequest")
            .field("operation_id", &"<redacted>")
            .field("purpose", &self.purpose)
            .field("requested_expiry_unix_ms", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
/// Context passed to every control effect call.
pub struct AcaControlContext {
    operation_id: AcaOperationId,
    deadline_remaining_ms: u32,
}

impl AcaControlContext {
    /// Construct a control context for one operation.
    pub fn new(operation_id: AcaOperationId, deadline_remaining_ms: u32) -> Self {
        Self {
            operation_id,
            deadline_remaining_ms,
        }
    }

    }

impl fmt::Debug for AcaControlContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaControlContext")
            .field("operation_id", &"<redacted>")
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Health of the ACA control plane for one sandbox.
pub enum AcaControlHealth {
    /// The sandbox is ready.
    Ready,
    /// The sandbox is degraded.
    Degraded,
    /// The sandbox is unavailable.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of failure returned by a control effect.
pub enum AcaControlErrorKind {
    /// Authentication with the control plane failed.
    Authentication,
    /// Authorization was denied.
    Authorization,
    /// The control plane rate-limited the call.
    RateLimited,
    /// The control plane is unavailable.
    Unavailable,
    /// The call conflicted with concurrent state.
    Conflict,
    /// The addressed resource was not found.
    NotFound,
    /// The control plane returned an invalid response.
    InvalidResponse,
    /// The call was cancelled.
    Cancelled,
    /// The operation deadline expired.
    DeadlineExpired,
    /// The result was ambiguous.
    Ambiguous,
}

impl AcaControlErrorKind {
    /// Return the stable error code for this kind.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Authentication => "aca-control-authentication",
            Self::Authorization => "aca-control-authorization",
            Self::RateLimited => "aca-control-rate-limited",
            Self::Unavailable => "aca-control-unavailable",
            Self::Conflict => "aca-control-conflict",
            Self::NotFound => "aca-control-not-found",
            Self::InvalidResponse => "aca-control-invalid-response",
            Self::Cancelled => "aca-control-cancelled",
            Self::DeadlineExpired => "aca-control-deadline-expired",
            Self::Ambiguous => "aca-control-ambiguous",
        }
    }

    /// Return whether a retry may succeed.
    pub const fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Unavailable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Error returned by a control effect, carrying a stable code.
pub struct AcaControlError {
    kind: AcaControlErrorKind,
}

impl AcaControlError {
    /// Construct a control error from a kind.
    pub const fn new(kind: AcaControlErrorKind) -> Self {
        Self { kind }
    }

    /// Return the error kind.
    pub const fn kind(self) -> AcaControlErrorKind {
        self.kind
    }

    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for AcaControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for AcaControlError {}

/// Client for acquiring and revoking credential leases.
///
/// Both methods return [`AcaControlError`] when the lease operation fails.
#[async_trait]
pub trait AcaCredentialLeaseClient: Send + Sync {
    /// Acquire a credential lease for one operation.
    async fn acquire(
        &self,
        request: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError>;

    /// Revoke a previously acquired credential lease.
    async fn revoke(&self, lease: &AcaCredentialLease) -> Result<(), AcaControlError>;
}

/// Effect port for the Azure Container Apps control plane.
///
/// Every method returns [`AcaControlError`] when the control plane call fails.
#[async_trait]
pub trait AcaControl: Send + Sync {
    /// Probe sandbox health.
    async fn health(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError>;

    /// List sandbox candidates matching a workload query.
    async fn find_sandboxes(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        query: &AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError>;

    /// List disk image candidates matching a desired image.
    async fn find_disk_images(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError>;

    /// Create a disk image for a desired image.
    async fn create_disk_image(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError>;

    /// Create a sandbox from a desired state.
    async fn create_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    /// Resume a suspended or stopped sandbox.
    async fn resume_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    /// Stop a running sandbox.
    async fn stop_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    /// Delete a sandbox.
    async fn delete_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError>;
}

#[cfg(test)]
mod tests {
    use super::{
        AcaRuntimeConfig,
    };

    #[test]
    fn runtime_config_deserialization_revalidates_constructor_bounds() {
        let valid = r#"{
            "profile": {
                "profileId": "default",
                "diskImage": {"configuredDisk": {"binding_id": "image-1"}},
                "cpu": 500,
                "memory": 2048,
                "autoSuspendSecs": 300,
                "sandboxIdentityBindingId": null
            },
            "readiness": {"attempts": 3, "intervalMs": 10},
            "planTtlMs": 1000,
            "completedOperationCapacity": 4
        }"#;
        let parsed = serde_json::from_str::<AcaRuntimeConfig>(valid);
        assert!(parsed.is_ok(), "{parsed:?}");

        let invalid = valid.replace("\"cpu\": 500", "\"cpu\": 251");
        assert!(serde_json::from_str::<AcaRuntimeConfig>(&invalid).is_err());
    }

    
}
