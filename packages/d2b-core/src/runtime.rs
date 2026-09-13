use crate::processes::ProcessRole;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use d2b_contracts::runtime::{
    RuntimeDisplayCapabilities, RuntimeGuestCapabilities, RuntimeLifecycleCapabilities,
    RuntimeMediaCapabilities, RuntimeOperationCapabilities, RuntimeServiceRole,
    RuntimeServiceSummary, RuntimeStorageCapabilities,
};

/// Runtime/provider metadata shared by the public manifest and private bundle
/// artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMetadata {
    #[serde(default, skip_serializing_if = "RuntimeAutostartPolicy::is_default")]
    pub autostart_policy: RuntimeAutostartPolicy,
    pub capabilities: RuntimeCapabilities,
    pub kind: RuntimeKind,
    #[serde(
        default,
        skip_serializing_if = "RuntimeOperationCapabilities::is_empty"
    )]
    pub operation_capabilities: RuntimeOperationCapabilities,
    pub provider: RuntimeProvider,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<RuntimeServiceSummary>,
}

impl RuntimeMetadata {
    pub fn local_nixos() -> Self {
        Self {
            autostart_policy: RuntimeAutostartPolicy::HostBootEligible,
            capabilities: RuntimeCapabilities {
                config_sync: true,
                display: true,
                exec: true,
                in_guest_observability: true,
                keys: true,
                lifecycle: true,
                ssh: true,
                store_sync: true,
                usb_hotplug: true,
            },
            operation_capabilities: RuntimeOperationCapabilities::local_nixos(),
            kind: RuntimeKind::Nixos,
            provider: RuntimeProvider {
                driver: RuntimeProviderDriver::CloudHypervisor,
                id: "local-cloud-hypervisor".to_owned(),
                provider_type: RuntimeProviderType::Local,
            },
            services: vec![
                service_summary("host-reconcile", ProcessRole::HostReconcile, false),
                service_summary(
                    "store-virtiofs-preflight",
                    ProcessRole::StoreVirtiofsPreflight,
                    false,
                ),
                service_summary("virtiofsd", ProcessRole::Virtiofsd, false),
                service_summary(
                    "cloud-hypervisor",
                    ProcessRole::CloudHypervisorRunner,
                    false,
                ),
                service_summary(
                    "component-session-health",
                    ProcessRole::ComponentSessionHealth,
                    false,
                ),
                service_summary("swtpm", ProcessRole::Swtpm, true),
                service_summary("gpu", ProcessRole::Gpu, true),
                service_summary("audio", ProcessRole::Audio, true),
                service_summary("video", ProcessRole::Video, true),
                service_summary("usbip", ProcessRole::Usbip, true),
            ],
        }
    }

    pub fn local_qemu_media() -> Self {
        Self {
            autostart_policy: RuntimeAutostartPolicy::ManualOnly,
            capabilities: RuntimeCapabilities {
                config_sync: false,
                display: true,
                exec: false,
                in_guest_observability: false,
                keys: false,
                lifecycle: true,
                ssh: false,
                store_sync: false,
                usb_hotplug: true,
            },
            operation_capabilities: RuntimeOperationCapabilities::local_qemu_media(),
            kind: RuntimeKind::QemuMedia,
            provider: RuntimeProvider {
                driver: RuntimeProviderDriver::Qemu,
                id: "local-qemu-media".to_owned(),
                provider_type: RuntimeProviderType::Local,
            },
            services: vec![
                service_summary("host-reconcile", ProcessRole::HostReconcile, false),
                service_summary("qemu-media", ProcessRole::QemuMediaRunner, false),
                service_summary("usbip", ProcessRole::Usbip, true),
            ],
        }
    }
}

/// VM runtime family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    Nixos,
    QemuMedia,
}

/// Local runtime provider identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeProvider {
    pub driver: RuntimeProviderDriver,
    pub id: String,
    #[serde(rename = "type")]
    pub provider_type: RuntimeProviderType,
}

/// Provider locality class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeProviderType {
    Local,
}

/// Provider driver family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeProviderDriver {
    CloudHypervisor,
    Crosvm,
    Qemu,
}

/// Runtime/provider support matrix. These flags describe support, not whether
/// a VM currently enables a feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilities {
    pub config_sync: bool,
    pub display: bool,
    pub exec: bool,
    pub in_guest_observability: bool,
    pub keys: bool,
    pub lifecycle: bool,
    pub ssh: bool,
    pub store_sync: bool,
    pub usb_hotplug: bool,
}

/// Runtime-level autostart policy exposed in public summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeAutostartPolicy {
    #[default]
    Unknown,
    HostBootEligible,
    ManualOnly,
    Disabled,
}

impl RuntimeAutostartPolicy {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl From<&ProcessRole> for RuntimeServiceRole {
    fn from(role: &ProcessRole) -> Self {
        match role {
            ProcessRole::HostReconcile => Self::Host,
            ProcessRole::ProviderController => Self::ComponentSession,
            ProcessRole::StoreVirtiofsPreflight | ProcessRole::Virtiofsd => Self::Storage,
            ProcessRole::SwtpmPreStartFlush | ProcessRole::Swtpm => Self::Tpm,
            ProcessRole::Video => Self::Video,
            ProcessRole::Gpu | ProcessRole::GpuRenderNode | ProcessRole::WaylandProxy => {
                Self::Display
            }
            ProcessRole::Audio => Self::Audio,
            ProcessRole::CloudHypervisorRunner | ProcessRole::QemuMediaRunner => Self::Hypervisor,
            ProcessRole::ActivationNixosRunner => Self::Host,
            ProcessRole::VsockRelay => Self::Network,
            ProcessRole::ComponentSessionHealth => Self::ComponentSession,
            ProcessRole::Usbip | ProcessRole::SecurityKeyFrontend => Self::Usb,
            ProcessRole::OtelHostBridge => Self::Observability,
        }
    }
}

/// Public service summary that can be derived from the private process DAG.
pub fn service_summary(
    id: impl Into<String>,
    process_role: ProcessRole,
    optional: bool,
) -> RuntimeServiceSummary {
    RuntimeServiceSummary {
        id: id.into(),
        optional,
        role: RuntimeServiceRole::from(&process_role),
    }
}
