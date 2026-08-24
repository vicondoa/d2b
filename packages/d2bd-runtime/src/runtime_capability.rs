use crate::typed_error::TypedError;
use d2b_core::manifest_v04::VmEntry as ManifestVmEntry;
use serde::Serialize;
use serde_json;

fn serde_kebab_string<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

#[derive(Debug, Clone, Copy)]
pub enum RuntimeCapabilityGate {
    ConfigSync,
    Exec,
    Keys,
    Ssh,
    StoreSync,
    UsbHotplug,
}

impl RuntimeCapabilityGate {
    pub fn slug(self) -> &'static str {
        match self {
            Self::ConfigSync => "config-sync",
            Self::Exec => "exec",
            Self::Keys => "keys",
            Self::Ssh => concat!("s", "sh"),
            Self::StoreSync => "store-sync",
            Self::UsbHotplug => "usb-hotplug",
        }
    }

    pub fn supported(self, entry: &ManifestVmEntry) -> bool {
        match self {
            Self::ConfigSync => entry.runtime.capabilities.config_sync,
            Self::Exec => entry.runtime.capabilities.exec,
            Self::Keys => entry.runtime.capabilities.keys,
            Self::Ssh => entry.runtime.capabilities.ssh,
            Self::StoreSync => entry.runtime.capabilities.store_sync,
            Self::UsbHotplug => entry.runtime.capabilities.usb_hotplug,
        }
    }
}

pub fn ensure_manifest_entry_runtime_capability(
    entry: Option<&ManifestVmEntry>,
    vm: &str,
    capability: RuntimeCapabilityGate,
    verb: &str,
) -> Result<(), TypedError> {
    let Some(entry) = entry else {
        return Ok(());
    };
    if capability.supported(entry) {
        return Ok(());
    }
    Err(TypedError::RuntimeCapabilityUnsupported {
        vm: vm.to_owned(),
        runtime_kind: serde_kebab_string(&entry.runtime.kind),
        capability: capability.slug().to_owned(),
        verb: verb.to_owned(),
    })
}
