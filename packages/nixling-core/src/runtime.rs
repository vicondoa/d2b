use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Runtime/provider metadata shared by the public manifest and private bundle
/// artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMetadata {
    pub capabilities: RuntimeCapabilities,
    pub kind: RuntimeKind,
    pub provider: RuntimeProvider,
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
            kind: RuntimeKind::Nixos,
            provider: RuntimeProvider {
                driver: RuntimeProviderDriver::CloudHypervisor,
                id: "local-cloud-hypervisor".to_owned(),
                provider_type: RuntimeProviderType::Local,
            },
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
            kind: RuntimeKind::QemuMedia,
            provider: RuntimeProvider {
                driver: RuntimeProviderDriver::Qemu,
                id: "local-qemu-media".to_owned(),
                provider_type: RuntimeProviderType::Local,
            },
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
