//! `ModprobeIfAllowed` broker op.
//!
//! `ModprobeIfAllowed` reads `/proc/sys/kernel/modules_disabled`
//! **before** attempting any load and accepts only module names that
//! appear in the trusted bundle's `kernelModules` matrix with
//! `loadAllowed: true`. Every decision (allow + deny) is audited with
//! `module_name`, `matrix_entry_id`, and `modules_disabled_sysctl`.
//!
//! The actual `modprobe(8)` invocation is intentionally factored out
//! behind the [`ModprobeBackend`] trait so the L1c fake-backend canary
//! `tests/kernel-module-matrix.sh` can drive the dispatcher without
//! root.

use serde::{Deserialize, Serialize};

use nixling_contracts::broker_wire::ModprobeIfAllowedRequest;
use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::host_w3::KernelModuleEntry;
use nixling_host::modules::{
    ModuleDisposition, ProbeInputs, probe_modules_disabled, probe_with,
    read_builtin_modules_with_fallback, read_loaded_modules,
};

use crate::audit::AuditLog;
use crate::ops::exec_reconcile::SystemLiveExec;

/// Audit fields emitted by every decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModprobeAuditRecord {
    pub module_name: String,
    pub matrix_entry_id: String,
    pub modules_disabled_sysctl: bool,
    pub disposition: ModprobeDecision,
}

/// Possible decisions for the dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModprobeDecision {
    /// Module is already loaded; no-op success.
    AlreadyLoaded,
    /// Module is compiled-in; no-op success.
    AlreadyBuiltin,
    /// Load attempted and succeeded.
    LoadedNow,
    /// Refused: requested module is not in the trusted bundle matrix.
    DeniedNotInMatrix,
    /// Refused: `/proc/sys/kernel/modules_disabled = 1`.
    DeniedHostModulesLocked,
    /// Refused: matrix entry exists but `loadAllowed = false`.
    DeniedNotLoadAllowed,
    /// Load attempted and failed at the backend.
    LoadFailed,
}

/// Trusted-bundle row controlling whether a module can be loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowlistRow {
    pub entry: KernelModuleEntry,
    pub load_allowed: bool,
}

/// Backend trait so the L1c canary can swap a fake `modprobe`.
pub trait ModprobeBackend {
    fn load(&mut self, module: &str) -> Result<(), String>;
}

/// Fake backend used by `tests/kernel-module-matrix.sh`.
#[derive(Debug, Default)]
pub struct RecordingBackend {
    pub loaded: Vec<String>,
    pub fail_on: Vec<String>,
}

impl ModprobeBackend for RecordingBackend {
    fn load(&mut self, module: &str) -> Result<(), String> {
        if self.fail_on.iter().any(|m| m == module) {
            return Err(format!("fake modprobe refused {module}"));
        }
        self.loaded.push(module.to_owned());
        Ok(())
    }
}

/// Dispatcher entry point. The four-step probe is run via the typed
/// `nixling_host::modules` helpers; this function only resolves the
/// matrix row and audit record.
pub fn dispatch(
    requested: &str,
    allowlist: &[AllowlistRow],
    inputs: &ProbeInputs,
    backend: &mut dyn ModprobeBackend,
) -> ModprobeAuditRecord {
    let row = allowlist.iter().find(|r| r.entry.module == requested);
    let Some(row) = row else {
        return ModprobeAuditRecord {
            module_name: requested.to_owned(),
            matrix_entry_id: String::new(),
            modules_disabled_sysctl: inputs.modules_disabled,
            disposition: ModprobeDecision::DeniedNotInMatrix,
        };
    };

    let matrix_entry_id = row.entry.matrix_entry_id.clone();
    let module_name = row.entry.module.clone();
    let modules_disabled_sysctl = inputs.modules_disabled;

    let probe = probe_with(std::slice::from_ref(&row.entry), inputs);
    let disposition = match probe.rows[0].disposition {
        ModuleDisposition::Loaded => ModprobeDecision::AlreadyLoaded,
        ModuleDisposition::Builtin => ModprobeDecision::AlreadyBuiltin,
        ModuleDisposition::OptionalAbsent if !row.load_allowed => {
            ModprobeDecision::DeniedNotLoadAllowed
        }
        ModuleDisposition::OptionalAbsent | ModuleDisposition::Loadable => {
            if !row.load_allowed {
                ModprobeDecision::DeniedNotLoadAllowed
            } else {
                match backend.load(&module_name) {
                    Ok(()) => ModprobeDecision::LoadedNow,
                    Err(_) => ModprobeDecision::LoadFailed,
                }
            }
        }
        ModuleDisposition::HostModulesLocked => ModprobeDecision::DeniedHostModulesLocked,
    };

    ModprobeAuditRecord {
        module_name,
        matrix_entry_id,
        modules_disabled_sysctl,
        disposition,
    }
}

struct LiveBackend<'a> {
    exec: &'a SystemLiveExec,
}

impl ModprobeBackend for LiveBackend<'_> {
    fn load(&mut self, module: &str) -> Result<(), String> {
        self.exec
            .run_modprobe(module)
            .map_err(|err| err.to_string())
    }
}

pub fn live_modprobe_if_allowed(
    exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &ModprobeIfAllowedRequest,
    _audit_log: &AuditLog,
) -> Result<ModprobeAuditRecord, String> {
    let inputs = ProbeInputs {
        modules_disabled: probe_modules_disabled(),
        loaded: read_loaded_modules(),
        builtin: read_builtin_modules_with_fallback(),
    };
    let allowlist = resolver
        .resolve_kernel_module_intent(req.module_name.as_str())
        .map(|intent| AllowlistRow {
            entry: KernelModuleEntry {
                module: intent.module_name,
                matrix_entry_id: intent.matrix_entry_id,
                feature: intent.feature,
                requirement: intent.requirement,
                fail_if_modules_disabled: intent.fail_if_modules_disabled,
            },
            load_allowed: intent.load_allowed,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let mut backend = LiveBackend { exec };
    Ok(dispatch(
        req.module_name.as_str(),
        &allowlist,
        &inputs,
        &mut backend,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::host_w3::ModuleRequirementW3;
    use nixling_host::modules::{BuiltinModuleSet, LoadedModuleSet};

    fn allow(module: &str, load_allowed: bool) -> AllowlistRow {
        AllowlistRow {
            entry: KernelModuleEntry {
                module: module.to_owned(),
                matrix_entry_id: format!("matrix-{module}"),
                feature: "test".to_owned(),
                requirement: ModuleRequirementW3::Required,
                fail_if_modules_disabled: true,
            },
            load_allowed,
        }
    }

    fn empty_inputs(modules_disabled: bool) -> ProbeInputs {
        ProbeInputs {
            modules_disabled,
            loaded: LoadedModuleSet::default(),
            builtin: BuiltinModuleSet::default(),
        }
    }

    #[test]
    fn unknown_module_denied_not_in_matrix() {
        let mut backend = RecordingBackend::default();
        let record = dispatch("rogue", &[], &empty_inputs(false), &mut backend);
        assert_eq!(record.disposition, ModprobeDecision::DeniedNotInMatrix);
        assert!(backend.loaded.is_empty());
    }

    #[test]
    fn modules_disabled_blocks_loadable_request() {
        let mut backend = RecordingBackend::default();
        let record = dispatch(
            "kvm",
            &[allow("kvm", true)],
            &empty_inputs(true),
            &mut backend,
        );
        assert_eq!(
            record.disposition,
            ModprobeDecision::DeniedHostModulesLocked
        );
        assert!(backend.loaded.is_empty());
    }

    #[test]
    fn load_allowed_module_invokes_backend() {
        let mut backend = RecordingBackend::default();
        let record = dispatch(
            "kvm",
            &[allow("kvm", true)],
            &empty_inputs(false),
            &mut backend,
        );
        assert_eq!(record.disposition, ModprobeDecision::LoadedNow);
        assert_eq!(backend.loaded, vec!["kvm".to_owned()]);
    }

    #[test]
    fn matrix_row_with_load_allowed_false_refuses_silently() {
        let mut backend = RecordingBackend::default();
        let record = dispatch(
            "kvm",
            &[allow("kvm", false)],
            &empty_inputs(false),
            &mut backend,
        );
        assert_eq!(record.disposition, ModprobeDecision::DeniedNotLoadAllowed);
    }

    #[test]
    fn already_loaded_module_short_circuits() {
        let mut backend = RecordingBackend::default();
        let mut inputs = empty_inputs(false);
        inputs.loaded.names.insert("kvm".to_owned());
        let record = dispatch("kvm", &[allow("kvm", true)], &inputs, &mut backend);
        assert_eq!(record.disposition, ModprobeDecision::AlreadyLoaded);
        assert!(backend.loaded.is_empty());
    }

    #[test]
    fn audit_record_carries_matrix_entry_id() {
        let mut backend = RecordingBackend::default();
        let record = dispatch(
            "kvm",
            &[allow("kvm", true)],
            &empty_inputs(false),
            &mut backend,
        );
        assert_eq!(record.module_name, "kvm");
        assert_eq!(record.matrix_entry_id, "matrix-kvm");
        assert!(!record.modules_disabled_sysctl);
    }
}
