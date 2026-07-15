//! Closed, bounded ACA configuration, discovery, and observed-state values.

use std::{error::Error, fmt};

use d2b_contracts::{
    v2_identity::{ProviderId, RealmId, WorkloadId},
    v2_provider::{Fingerprint, Generation, OperationBinding},
};

pub const MAX_ACA_RESOURCE_ID_LEN: usize = 60;
pub const MAX_ACA_CANDIDATES: usize = 8;
pub const MAX_ACA_READY_ATTEMPTS: u8 = 60;
pub const MAX_ACA_READY_INTERVAL_MS: u32 = 10_000;
pub const MAX_ACA_PLAN_TTL_MS: u32 = 5 * 60 * 1_000;
pub const MAX_ACA_COMPLETED_OPERATIONS: usize = 1_024;

const MIN_ACA_CPU_MILLIS: u16 = 250;
const MAX_ACA_CPU_MILLIS: u16 = 4_000;
const ACA_CPU_QUANTUM_MILLIS: u16 = 250;
const MIN_ACA_MEMORY_MIB: u32 = 512;
const MAX_ACA_MEMORY_MIB: u32 = 16 * 1_024;
const ACA_MEMORY_QUANTUM_MIB: u32 = 256;
const MIN_ACA_AUTO_SUSPEND_SECS: u32 = 60;
const MAX_ACA_AUTO_SUSPEND_SECS: u32 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaTypeError {
    InvalidIdentifier,
    InvalidResourceBounds,
    InvalidReadinessPolicy,
    InvalidPlanTtl,
    InvalidOperationCapacity,
    CandidateBoundExceeded,
}

impl fmt::Display for AcaTypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => "invalid bounded Azure Container Apps identifier",
            Self::InvalidResourceBounds => {
                "Azure Container Apps resource request is outside the supported bounds"
            }
            Self::InvalidReadinessPolicy => {
                "Azure Container Apps readiness policy is outside the supported bounds"
            }
            Self::InvalidPlanTtl => {
                "Azure Container Apps plan lifetime is outside the supported bounds"
            }
            Self::InvalidOperationCapacity => {
                "Azure Container Apps operation capacity is outside the supported bounds"
            }
            Self::CandidateBoundExceeded => {
                "Azure Container Apps candidate response exceeds the supported bound"
            }
        })
    }
}

impl Error for AcaTypeError {}

fn valid_opaque_id(value: &str, max: usize, require_lowercase_lead: bool) -> bool {
    !value.is_empty()
        && value.len() <= max
        && (!require_lowercase_lead || value.as_bytes()[0].is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

macro_rules! opaque_id {
    ($name:ident, $max:expr, $lowercase_lead:expr) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, AcaTypeError> {
                let value = value.into();
                if valid_opaque_id(&value, $max, $lowercase_lead) {
                    Ok(Self(value))
                } else {
                    Err(AcaTypeError::InvalidIdentifier)
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
    };
}

opaque_id!(AcaProfileId, 64, true);
opaque_id!(AcaConfiguredDiskId, 64, true);
opaque_id!(AcaConfiguredImageId, 64, true);
opaque_id!(AcaDiskImageName, 64, true);
opaque_id!(AcaManagedIdentityBindingId, 64, true);
opaque_id!(AcaSandboxId, MAX_ACA_RESOURCE_ID_LEN, false);
opaque_id!(AcaDiskImageId, MAX_ACA_RESOURCE_ID_LEN, false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcaCpuMillis(u16);

impl AcaCpuMillis {
    pub fn new(value: u16) -> Result<Self, AcaTypeError> {
        if (MIN_ACA_CPU_MILLIS..=MAX_ACA_CPU_MILLIS).contains(&value)
            && value.is_multiple_of(ACA_CPU_QUANTUM_MILLIS)
        {
            Ok(Self(value))
        } else {
            Err(AcaTypeError::InvalidResourceBounds)
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcaMemoryMib(u32);

impl AcaMemoryMib {
    pub fn new(value: u32) -> Result<Self, AcaTypeError> {
        if (MIN_ACA_MEMORY_MIB..=MAX_ACA_MEMORY_MIB).contains(&value)
            && value.is_multiple_of(ACA_MEMORY_QUANTUM_MIB)
        {
            Ok(Self(value))
        } else {
            Err(AcaTypeError::InvalidResourceBounds)
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum AcaDiskImageSource {
    ConfiguredDisk {
        binding_id: AcaConfiguredDiskId,
    },
    ConfiguredContainerImage {
        image_binding_id: AcaConfiguredImageId,
        disk_name: AcaDiskImageName,
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

#[derive(Clone, PartialEq, Eq)]
pub struct AcaSandboxProfile {
    profile_id: AcaProfileId,
    disk_image: AcaDiskImageSource,
    cpu: AcaCpuMillis,
    memory: AcaMemoryMib,
    auto_suspend_secs: u32,
    sandbox_identity_binding_id: Option<AcaManagedIdentityBindingId>,
}

impl AcaSandboxProfile {
    pub fn new(
        profile_id: AcaProfileId,
        disk_image: AcaDiskImageSource,
        cpu: AcaCpuMillis,
        memory: AcaMemoryMib,
        auto_suspend_secs: u32,
        sandbox_identity_binding_id: Option<AcaManagedIdentityBindingId>,
    ) -> Result<Self, AcaTypeError> {
        if !(MIN_ACA_AUTO_SUSPEND_SECS..=MAX_ACA_AUTO_SUSPEND_SECS).contains(&auto_suspend_secs) {
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

    pub fn profile_id(&self) -> &AcaProfileId {
        &self.profile_id
    }

    pub fn disk_image(&self) -> &AcaDiskImageSource {
        &self.disk_image
    }

    pub const fn cpu(&self) -> AcaCpuMillis {
        self.cpu
    }

    pub const fn memory(&self) -> AcaMemoryMib {
        self.memory
    }

    pub const fn auto_suspend_secs(&self) -> u32 {
        self.auto_suspend_secs
    }

    pub fn sandbox_identity_binding_id(&self) -> Option<&AcaManagedIdentityBindingId> {
        self.sandbox_identity_binding_id.as_ref()
    }
}

impl fmt::Debug for AcaSandboxProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaSandboxProfile")
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
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcaReadinessPolicy {
    attempts: u8,
    interval_ms: u32,
}

impl AcaReadinessPolicy {
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

    pub const fn attempts(self) -> u8 {
        self.attempts
    }

    pub const fn interval_ms(self) -> u32 {
        self.interval_ms
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaRuntimeConfig {
    profile: AcaSandboxProfile,
    readiness: AcaReadinessPolicy,
    plan_ttl_ms: u32,
    completed_operation_capacity: usize,
    initial_resource_generation: Generation,
}

impl AcaRuntimeConfig {
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
        let initial_resource_generation =
            Generation::new(1).map_err(|_| AcaTypeError::InvalidResourceBounds)?;
        Ok(Self {
            profile,
            readiness,
            plan_ttl_ms,
            completed_operation_capacity,
            initial_resource_generation,
        })
    }

    pub fn profile(&self) -> &AcaSandboxProfile {
        &self.profile
    }

    pub const fn readiness(&self) -> AcaReadinessPolicy {
        self.readiness
    }

    pub const fn plan_ttl_ms(&self) -> u32 {
        self.plan_ttl_ms
    }

    pub const fn completed_operation_capacity(&self) -> usize {
        self.completed_operation_capacity
    }

    pub const fn initial_resource_generation(&self) -> Generation {
        self.initial_resource_generation
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
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaWorkloadQuery {
    realm_id: RealmId,
    workload_id: WorkloadId,
}

impl AcaWorkloadQuery {
    pub(crate) fn new(realm_id: RealmId, workload_id: WorkloadId) -> Self {
        Self {
            realm_id,
            workload_id,
        }
    }

    pub fn realm_id(&self) -> &RealmId {
        &self.realm_id
    }

    pub fn workload_id(&self) -> &WorkloadId {
        &self.workload_id
    }
}

impl fmt::Debug for AcaWorkloadQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AcaWorkloadQuery(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaResourceBinding {
    realm_id: RealmId,
    workload_id: WorkloadId,
    provider_id: ProviderId,
    provider_generation: Generation,
    configuration_fingerprint: Fingerprint,
    resource_generation: Generation,
    created_by: OperationBinding,
}

impl AcaResourceBinding {
    pub fn new(
        realm_id: RealmId,
        workload_id: WorkloadId,
        provider_id: ProviderId,
        provider_generation: Generation,
        configuration_fingerprint: Fingerprint,
        resource_generation: Generation,
        created_by: OperationBinding,
    ) -> Self {
        Self {
            realm_id,
            workload_id,
            provider_id,
            provider_generation,
            configuration_fingerprint,
            resource_generation,
            created_by,
        }
    }

    pub fn realm_id(&self) -> &RealmId {
        &self.realm_id
    }

    pub fn workload_id(&self) -> &WorkloadId {
        &self.workload_id
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub const fn provider_generation(&self) -> Generation {
        self.provider_generation
    }

    pub fn configuration_fingerprint(&self) -> &Fingerprint {
        &self.configuration_fingerprint
    }

    pub const fn resource_generation(&self) -> Generation {
        self.resource_generation
    }

    pub fn created_by(&self) -> &OperationBinding {
        &self.created_by
    }
}

impl fmt::Debug for AcaResourceBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaResourceBinding")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaDesiredDiskImage {
    binding: AcaResourceBinding,
    profile_id: AcaProfileId,
    source: AcaDiskImageSource,
}

impl AcaDesiredDiskImage {
    pub(crate) fn new(
        binding: AcaResourceBinding,
        profile_id: AcaProfileId,
        source: AcaDiskImageSource,
    ) -> Self {
        Self {
            binding,
            profile_id,
            source,
        }
    }

    pub fn binding(&self) -> &AcaResourceBinding {
        &self.binding
    }

    pub fn profile_id(&self) -> &AcaProfileId {
        &self.profile_id
    }

    pub fn source(&self) -> &AcaDiskImageSource {
        &self.source
    }
}

impl fmt::Debug for AcaDesiredDiskImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaDesiredDiskImage")
            .field("binding", &self.binding)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaDesiredSandbox {
    binding: AcaResourceBinding,
    profile: AcaSandboxProfile,
    disk_image_id: AcaDiskImageId,
}

impl AcaDesiredSandbox {
    pub(crate) fn new(
        binding: AcaResourceBinding,
        profile: AcaSandboxProfile,
        disk_image_id: AcaDiskImageId,
    ) -> Self {
        Self {
            binding,
            profile,
            disk_image_id,
        }
    }

    pub fn binding(&self) -> &AcaResourceBinding {
        &self.binding
    }

    pub fn profile(&self) -> &AcaSandboxProfile {
        &self.profile
    }

    pub fn disk_image_id(&self) -> &AcaDiskImageId {
        &self.disk_image_id
    }
}

impl fmt::Debug for AcaDesiredSandbox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaDesiredSandbox")
            .field("binding", &self.binding)
            .field("profile", &self.profile)
            .field("disk_image_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaDiskImageRecord {
    id: AcaDiskImageId,
    binding: AcaResourceBinding,
    profile_id: AcaProfileId,
    source: AcaDiskImageSource,
}

impl AcaDiskImageRecord {
    pub fn new(
        id: AcaDiskImageId,
        binding: AcaResourceBinding,
        profile_id: AcaProfileId,
        source: AcaDiskImageSource,
    ) -> Self {
        Self {
            id,
            binding,
            profile_id,
            source,
        }
    }

    pub fn id(&self) -> &AcaDiskImageId {
        &self.id
    }

    pub fn binding(&self) -> &AcaResourceBinding {
        &self.binding
    }

    pub fn profile_id(&self) -> &AcaProfileId {
        &self.profile_id
    }

    pub fn source(&self) -> &AcaDiskImageSource {
        &self.source
    }
}

impl fmt::Debug for AcaDiskImageRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaDiskImageRecord")
            .field("binding", &self.binding)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaSandboxLifecycle {
    Provisioning,
    Ready,
    Running,
    Idle,
    Stopping,
    Stopped,
    Failed,
    Deleted,
    Unknown,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaSandboxRecord {
    id: AcaSandboxId,
    binding: AcaResourceBinding,
    lifecycle: AcaSandboxLifecycle,
}

impl AcaSandboxRecord {
    pub fn new(
        id: AcaSandboxId,
        binding: AcaResourceBinding,
        lifecycle: AcaSandboxLifecycle,
    ) -> Self {
        Self {
            id,
            binding,
            lifecycle,
        }
    }

    pub fn id(&self) -> &AcaSandboxId {
        &self.id
    }

    pub fn binding(&self) -> &AcaResourceBinding {
        &self.binding
    }

    pub const fn lifecycle(&self) -> AcaSandboxLifecycle {
        self.lifecycle
    }
}

impl fmt::Debug for AcaSandboxRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaSandboxRecord")
            .field("binding", &self.binding)
            .field("lifecycle", &self.lifecycle)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaSandboxCandidates(Vec<AcaSandboxRecord>);

impl AcaSandboxCandidates {
    pub fn new(candidates: Vec<AcaSandboxRecord>) -> Result<Self, AcaTypeError> {
        if candidates.len() > MAX_ACA_CANDIDATES {
            Err(AcaTypeError::CandidateBoundExceeded)
        } else {
            Ok(Self(candidates))
        }
    }

    pub fn as_slice(&self) -> &[AcaSandboxRecord] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for AcaSandboxCandidates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaSandboxCandidates")
            .field("count", &self.0.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcaDiskImageCandidates(Vec<AcaDiskImageRecord>);

impl AcaDiskImageCandidates {
    pub fn new(candidates: Vec<AcaDiskImageRecord>) -> Result<Self, AcaTypeError> {
        if candidates.len() > MAX_ACA_CANDIDATES {
            Err(AcaTypeError::CandidateBoundExceeded)
        } else {
            Ok(Self(candidates))
        }
    }

    pub fn as_slice(&self) -> &[AcaDiskImageRecord] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for AcaDiskImageCandidates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaDiskImageCandidates")
            .field("count", &self.0.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaDeleteOutcome {
    Deleted,
    AlreadyAbsent,
}
