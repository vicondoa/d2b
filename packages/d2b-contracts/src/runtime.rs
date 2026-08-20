use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Runtime/provider operation support grouped by public feature axis.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeOperationCapabilities {
    pub display: RuntimeDisplayCapabilities,
    pub guest: RuntimeGuestCapabilities,
    pub lifecycle: RuntimeLifecycleCapabilities,
    pub media: RuntimeMediaCapabilities,
    pub storage: RuntimeStorageCapabilities,
}

impl RuntimeOperationCapabilities {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn local_qemu_media() -> Self {
        Self {
            display: RuntimeDisplayCapabilities {
                display: true,
                ..Default::default()
            },
            lifecycle: RuntimeLifecycleCapabilities {
                host_prepare: true,
                restart: true,
                start: true,
                stop: true,
                ..Default::default()
            },
            media: RuntimeMediaCapabilities {
                qemu_media: true,
                removable_media: true,
                usb_hotplug: true,
            },
            ..Default::default()
        }
    }
}

/// Lifecycle operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeLifecycleCapabilities {
    pub host_prepare: bool,
    pub restart: bool,
    pub start: bool,
    pub stop: bool,
    pub switch: bool,
}

/// Media and hotplug operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMediaCapabilities {
    pub qemu_media: bool,
    pub removable_media: bool,
    pub usb_hotplug: bool,
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
    pub config_sync: bool,
    pub exec: bool,
    pub guest_control: bool,
    pub in_guest_observability: bool,
    pub keys: bool,
    #[serde(default)]
    pub shell: bool,
    pub ssh: bool,
}

/// Storage operations exposed by a runtime provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStorageCapabilities {
    pub store_sync: bool,
    pub virtiofs: bool,
    pub volumes: bool,
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

/// Public service summary that can be derived from the private process DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeServiceSummary {
    pub id: String,
    #[serde(default)]
    pub optional: bool,
    pub role: RuntimeServiceRole,
}
