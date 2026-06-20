use crate::processes::{ProcessNode, ProcessRole};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Runtime/provider metadata shared by the public manifest and private bundle
/// artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMetadata {
    pub capabilities: RuntimeCapabilities,
    pub kind: RuntimeKind,
    #[serde(
        default,
        skip_serializing_if = "RuntimeOperationCapabilities::is_empty"
    )]
    pub operation_capabilities: RuntimeOperationCapabilities,
    pub provider: RuntimeProvider,
    #[serde(default, skip_serializing_if = "RuntimeAutostartPolicy::is_default")]
    pub autostart_policy: RuntimeAutostartPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<RuntimeServiceSummary>,
}

impl RuntimeMetadata {
    pub fn local_nixos() -> Self {
        Self {
            capabilities: RuntimeCapabilities {
                config_sync: true,
                display: true,
                exec: true,
                guest_control: true,
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
            autostart_policy: RuntimeAutostartPolicy::HostBootEligible,
            services: vec![
                RuntimeServiceSummary::from_process_role(
                    "host-reconcile",
                    ProcessRole::HostReconcile,
                    false,
                ),
                RuntimeServiceSummary::from_process_role(
                    "store-virtiofs-preflight",
                    ProcessRole::StoreVirtiofsPreflight,
                    false,
                ),
                RuntimeServiceSummary::from_process_role(
                    "virtiofsd",
                    ProcessRole::Virtiofsd,
                    false,
                ),
                RuntimeServiceSummary::from_process_role(
                    "cloud-hypervisor",
                    ProcessRole::CloudHypervisorRunner,
                    false,
                ),
                RuntimeServiceSummary::from_process_role(
                    "guest-control-health",
                    ProcessRole::GuestControlHealth,
                    false,
                ),
                RuntimeServiceSummary::from_process_role("swtpm", ProcessRole::Swtpm, true),
                RuntimeServiceSummary::from_process_role("gpu", ProcessRole::Gpu, true),
                RuntimeServiceSummary::from_process_role("audio", ProcessRole::Audio, true),
                RuntimeServiceSummary::from_process_role("video", ProcessRole::Video, true),
                RuntimeServiceSummary::from_process_role("usbip", ProcessRole::Usbip, true),
            ],
        }
    }

    pub fn local_qemu_media() -> Self {
        Self {
            capabilities: RuntimeCapabilities {
                config_sync: false,
                display: true,
                exec: false,
                guest_control: false,
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
            autostart_policy: RuntimeAutostartPolicy::ManualOnly,
            services: vec![
                RuntimeServiceSummary::from_process_role(
                    "host-reconcile",
                    ProcessRole::HostReconcile,
                    false,
                ),
                RuntimeServiceSummary::from_process_role(
                    "qemu-media",
                    ProcessRole::QemuMediaRunner,
                    false,
                ),
                RuntimeServiceSummary::from_process_role("usbip", ProcessRole::Usbip, true),
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
    pub guest_control: bool,
    pub in_guest_observability: bool,
    pub keys: bool,
    pub lifecycle: bool,
    pub ssh: bool,
    pub store_sync: bool,
    pub usb_hotplug: bool,
}

/// Runtime/provider operation support grouped by public feature axis.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeOperationCapabilities {
    pub lifecycle: RuntimeLifecycleCapabilities,
    pub media: RuntimeMediaCapabilities,
    pub display: RuntimeDisplayCapabilities,
    pub guest: RuntimeGuestCapabilities,
    pub storage: RuntimeStorageCapabilities,
}

impl RuntimeOperationCapabilities {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn local_nixos() -> Self {
        Self {
            lifecycle: RuntimeLifecycleCapabilities {
                start: true,
                stop: true,
                restart: true,
                switch: true,
                host_prepare: true,
            },
            media: RuntimeMediaCapabilities {
                usb_hotplug: true,
                removable_media: false,
                qemu_media: false,
            },
            display: RuntimeDisplayCapabilities {
                display: true,
                graphics: true,
                video: true,
                wayland_proxy: true,
            },
            guest: RuntimeGuestCapabilities {
                guest_control: true,
                exec: true,
                config_sync: true,
                ssh: true,
                keys: true,
                in_guest_observability: true,
            },
            storage: RuntimeStorageCapabilities {
                store_sync: true,
                virtiofs: true,
                volumes: true,
            },
        }
    }

    pub fn local_qemu_media() -> Self {
        Self {
            lifecycle: RuntimeLifecycleCapabilities {
                start: true,
                stop: true,
                restart: true,
                switch: false,
                host_prepare: true,
            },
            media: RuntimeMediaCapabilities {
                usb_hotplug: true,
                removable_media: true,
                qemu_media: true,
            },
            display: RuntimeDisplayCapabilities {
                display: true,
                graphics: false,
                video: false,
                wayland_proxy: false,
            },
            guest: RuntimeGuestCapabilities::default(),
            storage: RuntimeStorageCapabilities::default(),
        }
    }
}

/// Lifecycle operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeLifecycleCapabilities {
    pub start: bool,
    pub stop: bool,
    pub restart: bool,
    pub switch: bool,
    pub host_prepare: bool,
}

/// Media and hotplug operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMediaCapabilities {
    pub usb_hotplug: bool,
    pub removable_media: bool,
    pub qemu_media: bool,
}

/// Display-side operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDisplayCapabilities {
    pub display: bool,
    pub graphics: bool,
    pub video: bool,
    pub wayland_proxy: bool,
}

/// Guest-facing operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeGuestCapabilities {
    pub guest_control: bool,
    pub exec: bool,
    pub config_sync: bool,
    pub ssh: bool,
    pub keys: bool,
    pub in_guest_observability: bool,
}

/// Storage operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStorageCapabilities {
    pub store_sync: bool,
    pub virtiofs: bool,
    pub volumes: bool,
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

/// Normalized public role for a runtime service, derived from process roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeServiceRole {
    Host,
    Hypervisor,
    Storage,
    Tpm,
    Display,
    Audio,
    Video,
    Network,
    GuestControl,
    Usb,
    Observability,
}

impl From<&ProcessRole> for RuntimeServiceRole {
    fn from(role: &ProcessRole) -> Self {
        match role {
            ProcessRole::HostReconcile => Self::Host,
            ProcessRole::StoreVirtiofsPreflight | ProcessRole::Virtiofsd => Self::Storage,
            ProcessRole::SwtpmPreStartFlush | ProcessRole::Swtpm => Self::Tpm,
            ProcessRole::Video => Self::Video,
            ProcessRole::Gpu | ProcessRole::GpuRenderNode | ProcessRole::WaylandProxy => {
                Self::Display
            }
            ProcessRole::Audio => Self::Audio,
            ProcessRole::CloudHypervisorRunner | ProcessRole::QemuMediaRunner => Self::Hypervisor,
            ProcessRole::VsockRelay => Self::Network,
            ProcessRole::GuestSshReadiness | ProcessRole::GuestControlHealth => Self::GuestControl,
            ProcessRole::Usbip => Self::Usb,
            ProcessRole::OtelHostBridge => Self::Observability,
        }
    }
}

/// Public service summary that can be derived from the private process DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeServiceSummary {
    pub id: String,
    pub role: RuntimeServiceRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_role: Option<ProcessRole>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
}

impl RuntimeServiceSummary {
    pub fn from_process_node(node: &ProcessNode, optional: bool) -> Self {
        Self::from_process_role(node.id.0.clone(), node.role.clone(), optional)
    }

    pub fn from_process_role(
        id: impl Into<String>,
        process_role: ProcessRole,
        optional: bool,
    ) -> Self {
        let role = RuntimeServiceRole::from(&process_role);
        Self {
            id: id.into(),
            role,
            process_role: Some(process_role),
            optional,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}
