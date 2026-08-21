//! Strict, locator-free Guest resource types for the qemu-media Provider.

use std::collections::BTreeSet;

use d2b_contracts_resource::v3::{
    CanonicalJsonObject,
    ProviderSpecExtension,
    ResourceRef,
    ResourceSpec,
    SchemaVersion,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

pub use d2b_contracts_resource::v3::GuestSpec;
pub use d2b_contracts_resource::v3::{
    execution_policy::{DeviceAttachment, NetworkAttachment},
};

/// Maximum removable media attachments on one Guest.
pub const MAX_REMOVABLE_VOLUMES: usize = 4;
/// Maximum bytes in a provider phase.
pub const MAX_PROVIDER_PHASE_BYTES: usize = 64;
/// Provider schema identifier for Guest settings.
pub const GUEST_SPEC_SCHEMA_ID: &str = "runtime-qemu-media.d2bus.org/Guest/spec";
/// Provider schema identifier for Guest status.
pub const GUEST_STATUS_SCHEMA_ID: &str = "runtime-qemu-media.d2bus.org/Guest/status";
/// Provider reference implemented by this crate.
pub const PROVIDER_REF: &str = "Provider/runtime-qemu-media";

/// QEMU CPU model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CpuModel {
    /// Host CPU model.
    Host,
    /// Maximum available emulated CPU model.
    Max,
    /// Broad compatibility CPU model.
    Qemu64,
}

/// QEMU machine type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MachineType {
    /// Modern Q35 machine.
    Q35,
    /// Legacy PC machine.
    Pc,
}

/// QEMU firmware selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Bios {
    /// OVMF firmware.
    Ovmf,
    /// SeaBIOS firmware.
    Seabios,
}

/// Guest RTC base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RtcBase {
    /// UTC guest clock.
    Utc,
    /// Local-time guest clock.
    Localtime,
}

/// Signed capability values accepted by `extraFeatures`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExtraFeature {
    /// Virtio random device.
    VirtioRng,
    /// Virtio balloon device.
    VirtioBalloon,
    /// Firmware SMM support.
    Smm,
    /// USB 3 controller support.
    Usb3,
}

/// A removable Volume reference and its named view.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemovableVolumeRef {
    /// Referenced Volume.
    pub volume_ref: ResourceRef,
    /// Named Volume view.
    pub view: String,
}

impl RemovableVolumeRef {
    /// Validate a removable Volume reference.
    pub fn new(volume_ref: ResourceRef, view: impl Into<String>) -> Result<Self, GuestSpecError> {
        if volume_ref.resource_type().as_str() != "Volume" {
            return Err(GuestSpecError::InvalidVolumeRef);
        }
        let view = view.into();
        validate_token(&view).map_err(|_| GuestSpecError::InvalidView)?;
        Ok(Self { volume_ref, view })
    }
}

impl<'de> Deserialize<'de> for RemovableVolumeRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            volume_ref: ResourceRef,
            view: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.volume_ref, wire.view).map_err(serde::de::Error::custom)
    }
}

impl core::fmt::Debug for RemovableVolumeRef {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("RemovableVolumeRef(<redacted>)")
    }
}

/// Guest provider-specific settings.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GuestProviderSpecSettings {
    /// Virtual CPU count.
    pub vcpu: u16,
    /// Guest memory in MiB.
    pub memory_mib: u32,
    /// Optional boot Volume.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_media_ref: Option<ResourceRef>,
    /// Boot Volume view.
    pub boot_media_view: String,
    /// Removable media Volumes.
    pub removable_volume_refs: Vec<RemovableVolumeRef>,
    /// CPU model.
    pub cpu_model: CpuModel,
    /// Machine type.
    pub machine_type: MachineType,
    /// Firmware.
    pub bios: Bios,
    /// Start QEMU paused.
    pub pause_at_boot: bool,
    /// Create a WaylandSession for a host window.
    pub display_window: bool,
    /// Publish a serial Endpoint.
    pub serial_console: bool,
    /// Add an absolute USB tablet.
    pub tablet: bool,
    /// Guest RTC base.
    pub rtc_base: RtcBase,
    /// Signed optional features.
    pub extra_features: Vec<ExtraFeature>,
}

impl Default for GuestProviderSpecSettings {
    fn default() -> Self {
        Self {
            vcpu: 2,
            memory_mib: 4096,
            boot_media_ref: None,
            boot_media_view: "guest-attach".to_owned(),
            removable_volume_refs: Vec::new(),
            cpu_model: CpuModel::Host,
            machine_type: MachineType::Q35,
            bios: Bios::Ovmf,
            pause_at_boot: true,
            display_window: false,
            serial_console: true,
            tablet: true,
            rtc_base: RtcBase::Utc,
            extra_features: Vec::new(),
        }
    }
}

impl GuestProviderSpecSettings {
    /// Validate every provider setting bound.
    pub fn validate(&self) -> Result<(), GuestSpecError> {
        if self.vcpu == 0 || self.vcpu > 1024 || !(128..=524_288).contains(&self.memory_mib) {
            return Err(GuestSpecError::InvalidResources);
        }
        if self
            .boot_media_ref
            .as_ref()
            .is_some_and(|reference| reference.resource_type().as_str() != "Volume")
        {
            return Err(GuestSpecError::InvalidVolumeRef);
        }
        validate_token(&self.boot_media_view).map_err(|_| GuestSpecError::InvalidView)?;
        if self.removable_volume_refs.len() > MAX_REMOVABLE_VOLUMES {
            return Err(GuestSpecError::TooManyRemovableVolumes);
        }
        let mut refs = BTreeSet::new();
        for removable in &self.removable_volume_refs {
            if !refs.insert(removable.volume_ref.to_canonical_string()) {
                return Err(GuestSpecError::DuplicateVolumeRef);
            }
        }
        if self.extra_features.len() > 16 {
            return Err(GuestSpecError::TooManyExtraFeatures);
        }
        let mut features = self.extra_features.clone();
        features.sort();
        features.dedup();
        if features.len() != self.extra_features.len() {
            return Err(GuestSpecError::DuplicateExtraFeature);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for GuestProviderSpecSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "default_vcpu")]
            vcpu: u16,
            #[serde(default = "default_memory_mib")]
            memory_mib: u32,
            #[serde(default)]
            boot_media_ref: Option<ResourceRef>,
            #[serde(default = "default_boot_media_view")]
            boot_media_view: String,
            #[serde(default)]
            removable_volume_refs: Vec<RemovableVolumeRef>,
            #[serde(default = "default_cpu_model")]
            cpu_model: CpuModel,
            #[serde(default = "default_machine_type")]
            machine_type: MachineType,
            #[serde(default = "default_bios")]
            bios: Bios,
            #[serde(default = "default_true")]
            pause_at_boot: bool,
            #[serde(default)]
            display_window: bool,
            #[serde(default = "default_true")]
            serial_console: bool,
            #[serde(default = "default_true")]
            tablet: bool,
            #[serde(default = "default_rtc_base")]
            rtc_base: RtcBase,
            #[serde(default)]
            extra_features: Vec<ExtraFeature>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let settings = Self {
            vcpu: wire.vcpu,
            memory_mib: wire.memory_mib,
            boot_media_ref: wire.boot_media_ref,
            boot_media_view: wire.boot_media_view,
            removable_volume_refs: wire.removable_volume_refs,
            cpu_model: wire.cpu_model,
            machine_type: wire.machine_type,
            bios: wire.bios,
            pause_at_boot: wire.pause_at_boot,
            display_window: wire.display_window,
            serial_console: wire.serial_console,
            tablet: wire.tablet,
            rtc_base: wire.rtc_base,
            extra_features: wire.extra_features,
        };
        settings.validate().map_err(serde::de::Error::custom)?;
        Ok(settings)
    }
}

impl core::fmt::Debug for GuestProviderSpecSettings {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestProviderSpecSettings")
            .field("boot_media_ref", &self.boot_media_ref.is_some())
            .field("removable_volume_refs", &self.removable_volume_refs.len())
            .field("cpu_model", &self.cpu_model)
            .field("machine_type", &self.machine_type)
            .field("bios", &self.bios)
            .field("pause_at_boot", &self.pause_at_boot)
            .field("display_window", &self.display_window)
            .field("serial_console", &self.serial_console)
            .field("tablet", &self.tablet)
            .field("rtc_base", &self.rtc_base)
            .field("extra_features", &self.extra_features)
            .finish()
    }
}

/// Construct a qemu-media Guest `ResourceSpec` from the canonical Guest base
/// and the Provider-owned settings envelope.
pub fn build_guest_resource_spec(
    boot_media_ref: Option<ResourceRef>,
    vcpu: u16,
    memory_mib: u32,
    mut settings: GuestProviderSpecSettings,
) -> Result<ResourceSpec, GuestResourceSpecError> {
    let provider_ref =
        ResourceRef::parse(PROVIDER_REF).map_err(|_| GuestResourceSpecError::InvalidProviderRef)?;
    settings.boot_media_ref = boot_media_ref;
    settings.vcpu = vcpu;
    settings.memory_mib = memory_mib;
    settings.validate()?;

    let base = GuestSpec::system_default();
    let base = serde_json::to_vec(&base).map_err(|_| GuestResourceSpecError::CanonicalJson)?;
    let base =
        CanonicalJsonObject::parse(&base).map_err(|_| GuestResourceSpecError::CanonicalJson)?;
    let settings =
        serde_json::to_vec(&settings).map_err(|_| GuestResourceSpecError::CanonicalJson)?;
    let settings =
        CanonicalJsonObject::parse(&settings).map_err(|_| GuestResourceSpecError::CanonicalJson)?;
    let provider = ProviderSpecExtension::new(
        d2b_contracts_resource::v3::ExtensionSchemaId::parse(GUEST_SPEC_SCHEMA_ID)
            .map_err(|_| GuestResourceSpecError::SchemaMismatch)?,
        SchemaVersion::parse("1.0").map_err(|_| GuestResourceSpecError::SchemaMismatch)?,
        settings,
    )
    .map_err(|_| GuestResourceSpecError::SchemaMismatch)?;
    ResourceSpec::new(Some(provider_ref), None, base, Some(provider))
        .map_err(|_| GuestResourceSpecError::CanonicalJson)
}

/// Failure while building the canonical Guest ResourceSpec envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestResourceSpecError {
    /// The fixed qemu-media Provider reference could not be parsed.
    InvalidProviderRef,
    /// Provider settings are invalid.
    InvalidSettings,
    /// The canonical Provider extension schema is invalid.
    SchemaMismatch,
    /// The canonical JSON base or settings object could not be rendered.
    CanonicalJson,
}

impl From<GuestSpecError> for GuestResourceSpecError {
    fn from(_: GuestSpecError) -> Self {
        Self::InvalidSettings
    }
}

impl core::fmt::Display for GuestResourceSpecError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidProviderRef => "qemu-media-provider-ref-invalid",
            Self::InvalidSettings => "qemu-media-guest-settings-invalid",
            Self::SchemaMismatch => "qemu-media-schema-mismatch",
            Self::CanonicalJson => "qemu-media-canonical-json-invalid",
        })
    }
}

impl std::error::Error for GuestResourceSpecError {}

/// Common Guest phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum GuestPhase {
    /// Dependencies are pending.
    Pending,
    /// Runner and QMP are ready.
    Ready,
    /// The Guest is usable with a degraded condition.
    Degraded,
    /// A required dependency or runner failed.
    Failed,
    /// Terminal deleted phase.
    Deleted,
    /// The controller lost current process observation.
    Unknown,
}

/// Backend provider phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderPhase {
    /// Newly created resource.
    Empty,
    /// Dependencies are pending.
    WaitingDependencies,
    /// Runtime Volume is being created.
    CreatingRuntimeVolume,
    /// LaunchTicket has been sealed.
    LaunchingRunner,
    /// Waiting for QMP greeting.
    WaitingQmp,
    /// QEMU is paused at boot.
    PausedAtBoot,
    /// QEMU is running.
    Running,
    /// Graceful shutdown is in progress.
    Stopping,
    /// Runner exited unexpectedly.
    RunnerFailed,
    /// Finalization is draining.
    FinalizePending,
}

impl ProviderPhase {
    /// Return the wire value used in `status.provider.details`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "",
            Self::WaitingDependencies => "waiting-dependencies",
            Self::CreatingRuntimeVolume => "creating-runtime-volume",
            Self::LaunchingRunner => "launching-runner",
            Self::WaitingQmp => "waiting-qmp",
            Self::PausedAtBoot => "paused-at-boot",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::RunnerFailed => "runner-failed",
            Self::FinalizePending => "finalize-pending",
        }
    }

    /// Parse a provider phase from a bounded wire value.
    pub fn parse(value: &str) -> Result<Self, GuestSpecError> {
        match value {
            "" => Ok(Self::Empty),
            "waiting-dependencies" => Ok(Self::WaitingDependencies),
            "creating-runtime-volume" => Ok(Self::CreatingRuntimeVolume),
            "launching-runner" => Ok(Self::LaunchingRunner),
            "waiting-qmp" => Ok(Self::WaitingQmp),
            "paused-at-boot" => Ok(Self::PausedAtBoot),
            "running" => Ok(Self::Running),
            "stopping" => Ok(Self::Stopping),
            "runner-failed" => Ok(Self::RunnerFailed),
            "finalize-pending" => Ok(Self::FinalizePending),
            _ => Err(GuestSpecError::InvalidProviderPhase),
        }
    }
}

/// Condition boolean state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConditionStatus {
    /// Condition holds.
    True,
    /// Condition does not hold.
    False,
}

/// Bounded Guest condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestCondition {
    /// Closed condition type.
    pub condition_type: String,
    /// Current condition state.
    pub status: ConditionStatus,
    /// Closed reason code.
    pub reason: String,
}

/// Common Guest runtime status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestRuntimeStatus {
    /// Whether the runner is ready.
    pub runtime_ready: bool,
    /// Whether the QMP session is ready.
    pub bootstrap_ready: bool,
    /// Number of active worker processes.
    pub active_process_count: u16,
}

/// Provider status details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestProviderDetails {
    /// Backend lifecycle detail.
    pub provider_phase: String,
}

/// Provider status extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestProviderStatus {
    /// Provider reference.
    pub provider_ref: ResourceRef,
    /// Status schema identifier.
    pub schema_id: String,
    /// Status schema version.
    pub schema_version: String,
    /// Observed Provider generation.
    pub observed_provider_generation: u64,
    /// Backend details.
    pub details: GuestProviderDetails,
}

/// Redacted Guest status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestStatus {
    /// Common phase.
    pub phase: GuestPhase,
    /// Common runtime status.
    pub resource: GuestRuntimeStatus,
    /// Provider extension.
    pub provider: GuestProviderStatus,
    /// Bounded conditions.
    #[serde(default)]
    pub conditions: Vec<GuestCondition>,
}

impl<'de> Deserialize<'de> for GuestStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            phase: GuestPhase,
            resource: GuestRuntimeStatus,
            provider: GuestProviderStatus,
            #[serde(default)]
            conditions: Vec<GuestCondition>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.conditions.len() > 16
            || wire.provider.provider_ref.to_canonical_string() != PROVIDER_REF
            || wire.provider.schema_id != GUEST_STATUS_SCHEMA_ID
            || wire.provider.schema_version != "1.0.0"
            || wire.provider.details.provider_phase.len() > MAX_PROVIDER_PHASE_BYTES
        {
            return Err(serde::de::Error::custom(GuestSpecError::SchemaMismatch));
        }
        ProviderPhase::parse(&wire.provider.details.provider_phase)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            phase: wire.phase,
            resource: wire.resource,
            provider: wire.provider,
            conditions: wire.conditions,
        })
    }
}

impl GuestStatus {
    /// Construct a status with the provider phase.
    pub fn new(phase: GuestPhase, provider_phase: ProviderPhase) -> Self {
        Self {
            phase,
            resource: GuestRuntimeStatus {
                runtime_ready: false,
                bootstrap_ready: false,
                active_process_count: 0,
            },
            provider: GuestProviderStatus {
                provider_ref: ResourceRef::parse(PROVIDER_REF).expect("frozen provider ref"),
                schema_id: GUEST_STATUS_SCHEMA_ID.to_owned(),
                schema_version: "1.0.0".to_owned(),
                observed_provider_generation: 0,
                details: GuestProviderDetails {
                    provider_phase: provider_phase.as_str().to_owned(),
                },
            },
            conditions: Vec::new(),
        }
    }

    /// Construct a status from a wire provider phase.
    pub fn from_provider_phase(value: &str) -> Result<Self, GuestSpecError> {
        if value.len() > MAX_PROVIDER_PHASE_BYTES {
            return Err(GuestSpecError::ProviderPhaseTooLong);
        }
        Ok(Self::new(GuestPhase::Pending, ProviderPhase::parse(value)?))
    }

    /// Return the common phase.
    pub const fn phase(&self) -> GuestPhase {
        self.phase
    }

    /// Return the parsed provider phase.
    pub fn provider_phase(&self) -> ProviderPhase {
        ProviderPhase::parse(&self.provider.details.provider_phase).unwrap_or(ProviderPhase::Empty)
    }
}

/// Guest specification validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSpecError {
    /// Provider reference is not runtime-qemu-media.
    InvalidProviderRef,
    /// VCPU or memory is outside the closed bounds.
    InvalidResources,
    /// A Volume reference had the wrong ResourceType.
    InvalidVolumeRef,
    /// A Network reference had the wrong ResourceType.
    InvalidNetworkRef,
    /// A Device reference had the wrong ResourceType.
    InvalidDeviceRef,
    /// A base attachment reference had the wrong ResourceType.
    InvalidReference,
    /// A Volume view was not a bounded token.
    InvalidView,
    /// Too many removable Volumes were requested.
    TooManyRemovableVolumes,
    /// A Volume was named more than once.
    DuplicateVolumeRef,
    /// An optional feature was named more than once.
    DuplicateExtraFeature,
    /// Too many optional features were requested.
    TooManyExtraFeatures,
    /// Provider schema identity did not match.
    SchemaMismatch,
    /// A media Guest cannot request a NixOS system artifact.
    SystemArtifactUnsupported,
    /// Provider phase is not in the closed set.
    InvalidProviderPhase,
    /// Provider phase exceeded its bound.
    ProviderPhaseTooLong,
}

impl core::fmt::Display for GuestSpecError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidProviderRef => "qemu-media-provider-ref-invalid",
            Self::InvalidResources => "qemu-media-guest-resources-invalid",
            Self::InvalidVolumeRef => "qemu-media-volume-ref-invalid",
            Self::InvalidNetworkRef => "qemu-media-network-ref-invalid",
            Self::InvalidDeviceRef => "qemu-media-device-ref-invalid",
            Self::InvalidReference => "qemu-media-reference-invalid",
            Self::InvalidView => "qemu-media-volume-view-invalid",
            Self::TooManyRemovableVolumes => "qemu-media-too-many-removable-volumes",
            Self::DuplicateVolumeRef => "qemu-media-duplicate-volume-ref",
            Self::DuplicateExtraFeature => "qemu-media-duplicate-extra-feature",
            Self::TooManyExtraFeatures => "qemu-media-too-many-extra-features",
            Self::SchemaMismatch => "qemu-media-schema-mismatch",
            Self::SystemArtifactUnsupported => "qemu-media-system-artifact-unsupported",
            Self::InvalidProviderPhase => "qemu-media-provider-phase-invalid",
            Self::ProviderPhaseTooLong => "qemu-media-provider-phase-too-long",
        })
    }
}

impl std::error::Error for GuestSpecError {}

fn validate_token(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 63
        || !value.as_bytes()[0].is_ascii_lowercase()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Err(())
    } else {
        Ok(())
    }
}

const fn default_true() -> bool {
    true
}

const fn default_vcpu() -> u16 {
    2
}

const fn default_memory_mib() -> u32 {
    4096
}

fn default_boot_media_view() -> String {
    "guest-attach".to_owned()
}

const fn default_cpu_model() -> CpuModel {
    CpuModel::Host
}

const fn default_machine_type() -> MachineType {
    MachineType::Q35
}

const fn default_bios() -> Bios {
    Bios::Ovmf
}

const fn default_rtc_base() -> RtcBase {
    RtcBase::Utc
}
