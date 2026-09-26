//! Host virtualization preconditions.
//!
//! The lane runs on a contributor's own machine, so a host that cannot run
//! it is a host where the checks are silently six times slower rather than a
//! host where they are unavailable. The lane therefore refuses to start
//! unless the host has hardware virtualization, nested virtualization, and
//! the host kernel's ability to save a nested guest's state - the third is
//! what snapshotting a guest that has run a nested check needs, and its
//! absence surfaces as a `-EINVAL` from the emulator at snapshot time rather
//! than as anything a contributor could act on.
//!
//! Every capability is read from a host fact, never inferred from the
//! emulator's own behaviour, and every missing one is reported by name in a
//! single message so a contributor fixes them in one pass.

use std::{fmt, fs, path::Path};

use crate::error::{HarnessError, Result};

/// The KVM character device the lane's guests attach to.
const KVM_DEVICE: &str = "/dev/kvm";

/// The in-tree x86 KVM modules, with the module parameter that gates nested
/// virtualization and the one that gates saving a nested guest's state.
///
/// `kvm_amd` separates the two: `nested` admits an L2 guest at all, and
/// `nested_svm` is what makes the nested SVM state extractable. `kvm_intel`
/// has no second knob - the nested VMCS save path is compiled in whenever
/// `nested` is on - so on that module the two capabilities are satisfied by
/// one read, and the state-save capability reports the module that decided
/// it.
const KVM_MODULES: &[KvmModule] = &[
    KvmModule {
        name: "kvm_amd",
        nested: "nested",
        nested_state: Some("nested_svm"),
    },
    KvmModule {
        name: "kvm_intel",
        nested: "nested",
        nested_state: None,
    },
];

struct KvmModule {
    name: &'static str,
    nested: &'static str,
    nested_state: Option<&'static str>,
}

/// One precondition the lane states as a precondition rather than assuming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// `/dev/kvm` exists and this process can open it for both reading and
    /// writing.
    KvmDevice,
    /// The KVM module in use has nested virtualization enabled, so a guest
    /// can run the nested guest the Cloud Hypervisor checks boot.
    NestedVirtualization,
    /// The host kernel can save a nested guest's virtualization state, so a
    /// guest that has run one can still be snapshotted.
    NestedStateSave,
}

impl Capability {
    /// A short, stable name for the capability, used in the failure message.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KvmDevice => "hardware virtualization (/dev/kvm)",
            Self::NestedVirtualization => "nested virtualization",
            Self::NestedStateSave => "nested-state save support",
        }
    }

    /// What a contributor does to provide the capability.
    fn remedy(self) -> &'static str {
        match self {
            Self::KvmDevice => {
                "load the kvm_intel or kvm_amd module, then add this user to the `kvm` group"
            }
            Self::NestedVirtualization => {
                "boot the host kernel with the `kvm-intel.nested=1` or `kvm-amd.nested=1` module parameter"
            }
            Self::NestedStateSave => {
                "boot the host kernel with the `kvm-amd.nested_svm=1` module parameter (on Intel the nested-state path needs only `nested=1`)"
            }
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.as_str(), self.remedy())
    }
}

/// The host facts the assessment reads.
///
/// Gathering them is separated from judging them so the judgment is a pure
/// function: a host that is missing a capability is described by the same
/// facts a healthy host is, and the missing-capability path is exercised
/// without needing a second machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFacts {
    /// `Some(())` when `/dev/kvm` opened read-write, `Some(reason)` when it
    /// exists but did not, `None` when it is not there at all.
    pub kvm_device: std::result::Result<(), String>,
    /// The loaded KVM module and the value of each parameter the assessment
    /// reads, keyed `"<module>/<parameter>"`.
    pub module_parameters: Vec<(String, Option<String>)>,
}

impl HostFacts {
    /// Read the facts off this host.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn probe() -> Self {
        let kvm_device = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(KVM_DEVICE)
        {
            Ok(_file) => Ok(()),
            Err(error) => Err(error.to_string()),
        };
        let mut module_parameters = Vec::new();
        for module in KVM_MODULES {
            for parameter in [Some(module.nested), module.nested_state]
                .into_iter()
                .flatten()
            {
                let path = Path::new("/sys/module")
                    .join(module.name)
                    .join("parameters")
                    .join(parameter);
                module_parameters.push((
                    format!("{}/{}", module.name, parameter),
                    read_trimmed(&path),
                ));
            }
        }
        Self {
            kvm_device,
            module_parameters,
        }
    }

    fn parameter(&self, key: &str) -> Option<bool> {
        self.module_parameters
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.as_deref())
            .map(is_enabled)
    }
}

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|text| text.trim().to_owned())
}

/// A KVM module parameter is enabled when it reads `Y`, `y`, `1`, or `true`;
/// every other value - including a module that is not loaded at all, which
/// has no parameter node - is not.
fn is_enabled(value: &str) -> bool {
    matches!(value, "Y" | "y" | "1" | "true" | "True")
}

/// The capabilities this host is missing, in a stable order.
///
/// A host with no loaded KVM module at all is missing both nested
/// capabilities rather than reporting a module it never found: the lane needs
/// the capability, and naming the knob is the actionable half of the message.
pub fn missing_capabilities(facts: &HostFacts) -> Vec<Capability> {
    let mut missing = Vec::new();
    if facts.kvm_device.is_err() {
        missing.push(Capability::KvmDevice);
    }
    let nested = KVM_MODULES
        .iter()
        .any(|module| facts.parameter(&format!("{}/{}", module.name, module.nested)) == Some(true));
    if !nested {
        missing.push(Capability::NestedVirtualization);
    }
    // Saved nested state is a superset of nested virtualization: on AMD it
    // has its own knob, and on Intel the single `nested` read already covers
    // it. A host with neither knob is reported as missing both, because
    // neither is satisfied.
    let state = KVM_MODULES.iter().any(|module| match module.nested_state {
        Some(parameter) => {
            facts.parameter(&format!("{}/{}", module.name, parameter)) == Some(true)
                && facts.parameter(&format!("{}/{}", module.name, module.nested)) == Some(true)
        }
        None => facts.parameter(&format!("{}/{}", module.name, module.nested)) == Some(true),
    });
    if !state {
        missing.push(Capability::NestedStateSave);
    }
    missing
}

/// Refuse to start the lane on a host that cannot run it.
pub fn require(facts: &HostFacts) -> Result<()> {
    let missing = missing_capabilities(facts);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(HarnessError::HostUnsupported { missing })
    }
}

/// Read this host's facts and require every capability.
pub fn require_this_host() -> Result<HostFacts> {
    let facts = HostFacts::probe();
    require(&facts)?;
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(
        kvm_device: std::result::Result<(), String>,
        parameters: &[(&str, &str)],
    ) -> HostFacts {
        HostFacts {
            kvm_device,
            module_parameters: parameters
                .iter()
                .map(|(name, value)| ((*name).to_owned(), Some((*value).to_owned())))
                .collect(),
        }
    }

    #[test]
    fn a_healthy_intel_host_reports_nothing_missing() {
        let healthy = facts(Ok(()), &[("kvm_intel/nested", "Y")]);
        assert_eq!(missing_capabilities(&healthy), Vec::new());
        require(&healthy).expect("an Intel host with nested enabled runs the lane");
    }

    #[test]
    fn a_healthy_amd_host_needs_both_of_its_knobs() {
        let healthy = facts(Ok(()), &[("kvm_amd/nested", "Y"), ("kvm_amd/nested_svm", "Y")]);
        assert_eq!(missing_capabilities(&healthy), Vec::new());
    }

    #[test]
    fn an_amd_host_without_nested_state_save_names_that_capability() {
        // The one the plan calls out: a host that admits a nested guest but
        // cannot save its state stops the lane before any guest boots, and
        // the message says which of the two knobs is wrong.
        let amd = facts(
            Ok(()),
            &[("kvm_amd/nested", "Y"), ("kvm_amd/nested_svm", "N")],
        );
        let missing = missing_capabilities(&amd);
        assert_eq!(missing, vec![Capability::NestedStateSave]);
        let error = require(&amd).expect_err("the lane refuses this host");
        let rendered = error.to_string();
        assert!(rendered.contains("nested-state save support"), "{rendered}");
        assert!(rendered.contains("nested_svm=1"), "{rendered}");
    }

    #[test]
    fn a_host_without_kvm_names_every_capability_it_is_missing() {
        let bare = facts(Err("No such file or directory".to_owned()), &[]);
        let error = require(&bare).expect_err("the lane refuses a host with no KVM");
        let rendered = error.to_string();
        for expected in [
            "/dev/kvm",
            "nested virtualization",
            "nested-state save support",
            "does not fall back to emulation",
        ] {
            assert!(rendered.contains(expected), "{rendered}");
        }
    }

    #[test]
    fn nested_disabled_is_reported_as_both_nested_capabilities() {
        let host = facts(Ok(()), &[("kvm_intel/nested", "N")]);
        assert_eq!(
            missing_capabilities(&host),
            vec![Capability::NestedVirtualization, Capability::NestedStateSave]
        );
    }

    #[test]
    fn an_unloaded_module_is_not_an_enabled_parameter() {
        assert!(!is_enabled("N"));
        assert!(!is_enabled(""));
        assert!(is_enabled("Y"));
        assert!(is_enabled("1"));
    }
}
