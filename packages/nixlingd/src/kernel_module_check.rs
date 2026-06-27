//! Daemon startup self-check that
//! verifies the kernel-module matrix the running trusted bundle
//! requires is loaded into the live kernel.
//!
//! # Why this preflight exists
//!
//! Every VM nixling spawns relies on a small, predictable set of
//! host kernel modules: KVM for hardware-assisted virtualization,
//! `vhost_net` + `tun` for the TAP-fd data path, `virtio_*` for the
//! guest-side device drivers, plus the per-feature add-ons
//! (`virtiofs` for the per-VM `/nix/store` share, `udmabuf` for graphics VMs, `usbip_host` for USBIP
//! passthrough, `tpm_vtpm_proxy` for swtpm). If any of those are
//! missing at daemon startup the eventual VM-start request fails
//! deep inside `cloud-hypervisor` with a generic ENODEV that takes
//! a journal dive to root-cause.
//!
//! This module turns that latent failure into an explicit, typed,
//! fail-closed startup check. The pure check function
//! [`check_kernel_modules`] takes the resolved bundle + a parsed
//! [`LoadedModuleSet`] snapshot of `/proc/modules` and returns a
//! [`ModuleCheckReport`] enumerating:
//!
//! * `required` — modules every VM in the bundle conditionally
//!   demands (kvm_*, vhost_net, tun, virtio_*, plus virtiofs /
//!   udmabuf when the bundle uses them).
//! * `present` — required modules detected loaded.
//! * `missing_required` — required modules NOT loaded. Non-empty →
//!   daemon refuses to start.
//! * `optional_missing` — optional modules NOT loaded, with the
//!   VMs that would have benefited from each. Non-empty → log a
//!   warning + mark each affected VM as degraded so the autostart
//!   pass skips it instead of looping.
//!
//! KVM is special-cased: kvm_intel OR kvm_amd satisfies the
//! requirement. Modules detected as built-in (via the existing
//! [`nixling_host::modules`] probe surface) are treated as
//! present.
//!
//! # Scope
//!
//! The check is read-only and hermetic — the pure function takes
//! `&LoadedModuleSet` and never touches the filesystem. The
//! side-effecting wrapper [`run_kernel_module_check`] reads
//! `/proc/modules` and `/proc/sys/kernel/modules_disabled` once,
//! then defers to the pure function. Tests exercise the pure
//! function with fixtures.

use std::collections::BTreeSet;
use std::path::Path;

use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::processes::ProcessRole;
use nixling_host::modules::{
    KernelConfig, LoadedModuleSet, read_builtin_modules_with_fallback, read_kernel_config,
    read_loaded_modules_at,
};
use serde::{Deserialize, Serialize};

use crate::typed_error::TypedError;

/// Required modules every nixling VM needs, regardless of opt-in
/// features. KVM (intel | amd) is handled separately by
/// [`REQUIRED_KVM_ALTERNATIVES`] because exactly one of the two
/// satisfies the requirement.
pub const REQUIRED_ALWAYS: &[&str] = &[
    "vhost_net",
    "tun",
    "virtio_net",
    "virtio_blk",
    "virtio_pci",
    "virtio_console",
];

/// At least one of these MUST be loaded (or built-in). The check
/// fails iff every alternative is absent.
pub const REQUIRED_KVM_ALTERNATIVES: &[&str] = &["kvm_intel", "kvm_amd"];

/// Required if and only if at least one VM in the bundle uses
/// virtiofs (= has a `Virtiofsd` process node).
pub const REQUIRED_IF_VIRTIOFS: &str = "virtiofs";

/// Required if and only if at least one VM in the bundle is a
/// graphics VM (= manifest `graphics = true` and/or `Gpu` process
/// node). `udmabuf` may be built into the host kernel; the startup
/// wrapper maps `CONFIG_UDMABUF=y` to this module name.
pub const REQUIRED_IF_GRAPHICS: &[&str] = &["udmabuf"];

/// Optional accelerator modules for nvidia-equipped graphics
/// hosts. Absence is warn-only — VMs continue to autostart with
/// software rendering / no accelerator handoff.
pub const OPTIONAL_GRAPHICS_NVIDIA: &[&str] = &["nvidia", "nvidia_uvm"];

/// Optional module that enables USBIP host passthrough. When
/// absent, VMs that opted into USBIP are flagged degraded.
pub const OPTIONAL_USBIP: &str = "usbip_host";

/// Optional module that enables swtpm proxy on the host. When
/// absent, VMs with `tpm = true` are flagged degraded.
pub const OPTIONAL_TPM: &str = "tpm_vtpm_proxy";

/// One row in [`ModuleCheckReport::optional_missing`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalMissing {
    /// Kernel module name (e.g. `nvidia`, `usbip_host`,
    /// `tpm_vtpm_proxy`).
    pub module: String,
    /// VMs whose declared features depend on this module. Empty
    /// for purely host-wide optionals (e.g. `nvidia` with no
    /// graphics VMs declared).
    pub affected_vms: BTreeSet<String>,
    /// Short human-readable reason (e.g. "graphics VMs may fall
    /// back to software rendering"). Suitable for log + the
    /// operator reference.
    pub reason: String,
}

/// Hermetic report produced by [`check_kernel_modules`]. Holds
/// enough state for the daemon to (1) refuse startup with a typed
/// error, (2) log a structured warning, and (3) tell the autostart
/// executor which VMs should be skipped as degraded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleCheckReport {
    /// All modules the bundle declared a hard requirement on,
    /// after KVM alternatives + feature gates were resolved. KVM
    /// is rendered as `kvm_intel|kvm_amd` (single entry) when the
    /// alternative is satisfied; otherwise both alternatives are
    /// listed individually in `missing_required`.
    pub required: BTreeSet<String>,
    /// Subset of `required` (or any KVM alternative) detected
    /// loaded into the live kernel.
    pub present: BTreeSet<String>,
    /// Subset of `required` NOT detected loaded. Non-empty →
    /// daemon refuses to start with [`TypedError::HostKernelModulesMissing`].
    pub missing_required: BTreeSet<String>,
    /// Optional modules NOT detected loaded, with the VMs each
    /// would have helped. Drives the per-VM degraded marking the
    /// autostart pass consumes.
    pub optional_missing: Vec<OptionalMissing>,
}

impl ModuleCheckReport {
    /// Convenience: union of `optional_missing[*].affected_vms`.
    /// These VMs should be marked degraded by the autostart pass.
    pub fn degraded_vms(&self) -> BTreeSet<String> {
        let mut acc = BTreeSet::new();
        for row in &self.optional_missing {
            acc.extend(row.affected_vms.iter().cloned());
        }
        acc
    }

    /// True iff the daemon should refuse to start.
    pub fn is_fatal(&self) -> bool {
        !self.missing_required.is_empty()
    }

    /// Render `missing_required` as a stable, comma-separated
    /// list. Used in the public typed-error message + the
    /// startup log line. KVM alternatives appear as
    /// `kvm_intel|kvm_amd` in the public surface so operators
    /// know either satisfies the requirement.
    pub fn missing_required_summary(&self) -> String {
        let mut acc: Vec<String> = Vec::new();
        let kvm = REQUIRED_KVM_ALTERNATIVES
            .iter()
            .filter(|m| self.missing_required.contains(**m))
            .copied()
            .collect::<Vec<_>>();
        if kvm.len() == REQUIRED_KVM_ALTERNATIVES.len() {
            acc.push(kvm.join("|"));
        }
        for m in &self.missing_required {
            if REQUIRED_KVM_ALTERNATIVES.contains(&m.as_str()) {
                continue;
            }
            acc.push(m.clone());
        }
        acc.join(", ")
    }
}

/// Detect per-bundle feature usage so the conditional REQUIRED /
/// OPTIONAL rows can be folded in. Pure inspection of the
/// resolver state — no I/O.
fn classify_vms(resolver: &BundleResolver) -> BundleFeatureSet {
    let mut features = BundleFeatureSet::default();

    for (vm_id, vm) in &resolver.manifest.vms {
        if vm.graphics {
            features.graphics_vms.insert(vm_id.clone());
        }
        if vm.tpm {
            features.tpm_vms.insert(vm_id.clone());
        }
        if vm.usbip_yubikey {
            features.usbip_vms.insert(vm_id.clone());
        }
    }

    // Virtiofs presence is structural: any VM whose process DAG
    // carries a `Virtiofsd` node requires the host `virtiofs`
    // module. We additionally pick up Gpu / Usbip / Swtpm nodes
    // for VMs whose manifest-level booleans might lag a
    // future-shape change.
    for dag in &resolver.processes.vms {
        let vm_id = &dag.vm;
        for node in &dag.nodes {
            match node.role {
                ProcessRole::Virtiofsd => {
                    features.virtiofs_vms.insert(vm_id.clone());
                }
                ProcessRole::Gpu | ProcessRole::GpuRenderNode => {
                    features.graphics_vms.insert(vm_id.clone());
                }
                ProcessRole::Usbip => {
                    features.usbip_vms.insert(vm_id.clone());
                }
                ProcessRole::Swtpm | ProcessRole::SwtpmPreStartFlush => {
                    features.tpm_vms.insert(vm_id.clone());
                }
                _ => {}
            }
        }
    }

    features
}

#[derive(Debug, Clone, Default)]
struct BundleFeatureSet {
    /// VMs that declared `graphics = true` or carry a `Gpu` node.
    graphics_vms: BTreeSet<String>,
    /// VMs whose process DAG carries a `Virtiofsd` node.
    virtiofs_vms: BTreeSet<String>,
    /// VMs that declared `tpm = true` or carry a `Swtpm` node.
    tpm_vms: BTreeSet<String>,
    /// VMs that declared `usbip_yubikey = true` or carry a
    /// `Usbip` node.
    usbip_vms: BTreeSet<String>,
}

/// Pure check: given the resolver and a parsed `/proc/modules`
/// snapshot, decide which modules are required / present /
/// missing. No I/O. Tests inject [`LoadedModuleSet`] fixtures.
///
/// `builtin_modules` is the set of modules detected built-in via
/// the host probe (see [`nixling_host::modules`]). A module that
/// is built-in counts as present.
pub fn check_kernel_modules(
    resolver: &BundleResolver,
    loaded: &LoadedModuleSet,
    builtin: &BTreeSet<String>,
) -> ModuleCheckReport {
    let features = classify_vms(resolver);
    let is_present = |m: &str| loaded.contains(m) || builtin.contains(m);

    let mut required = BTreeSet::new();
    let mut present = BTreeSet::new();
    let mut missing_required = BTreeSet::new();

    // Always-required base.
    for m in REQUIRED_ALWAYS {
        required.insert((*m).to_owned());
        if is_present(m) {
            present.insert((*m).to_owned());
        } else {
            missing_required.insert((*m).to_owned());
        }
    }

    // KVM alternative: kvm_intel OR kvm_amd must be present.
    let kvm_present: Vec<&str> = REQUIRED_KVM_ALTERNATIVES
        .iter()
        .copied()
        .filter(|m| is_present(m))
        .collect();
    for m in REQUIRED_KVM_ALTERNATIVES {
        required.insert((*m).to_owned());
    }
    if kvm_present.is_empty() {
        for m in REQUIRED_KVM_ALTERNATIVES {
            missing_required.insert((*m).to_owned());
        }
    } else {
        for m in &kvm_present {
            present.insert((*m).to_owned());
        }
    }

    // Conditional: virtiofs.
    if !features.virtiofs_vms.is_empty() {
        required.insert(REQUIRED_IF_VIRTIOFS.to_owned());
        if is_present(REQUIRED_IF_VIRTIOFS) {
            present.insert(REQUIRED_IF_VIRTIOFS.to_owned());
        } else {
            missing_required.insert(REQUIRED_IF_VIRTIOFS.to_owned());
        }
    }

    // Conditional: graphics → udmabuf + drm_virtgpu.
    if !features.graphics_vms.is_empty() {
        for m in REQUIRED_IF_GRAPHICS {
            required.insert((*m).to_owned());
            if is_present(m) {
                present.insert((*m).to_owned());
            } else {
                missing_required.insert((*m).to_owned());
            }
        }
    }

    // Optional rows. Each is recorded only when (a) the gate is
    // satisfied (e.g. there is at least one graphics VM) and
    // (b) the module is NOT present.
    let mut optional_missing: Vec<OptionalMissing> = Vec::new();

    if !features.graphics_vms.is_empty() {
        for m in OPTIONAL_GRAPHICS_NVIDIA {
            if !is_present(m) {
                optional_missing.push(OptionalMissing {
                    module: (*m).to_owned(),
                    // nvidia is warn-only and does NOT degrade
                    // VMs (software-render fallback is the
                    // expected behaviour on non-nvidia hosts).
                    affected_vms: BTreeSet::new(),
                    reason:
                        "graphics VMs may fall back to software rendering on nvidia-equipped hosts"
                            .to_owned(),
                });
            }
        }
    }

    if !features.usbip_vms.is_empty() && !is_present(OPTIONAL_USBIP) {
        optional_missing.push(OptionalMissing {
            module: OPTIONAL_USBIP.to_owned(),
            affected_vms: features.usbip_vms.clone(),
            reason: "USBIP-enabled VMs cannot bind a host bus until usbip_host is loaded"
                .to_owned(),
        });
    }

    if !features.tpm_vms.is_empty() && !is_present(OPTIONAL_TPM) {
        optional_missing.push(OptionalMissing {
            module: OPTIONAL_TPM.to_owned(),
            affected_vms: features.tpm_vms.clone(),
            reason: "swtpm-backed VMs cannot present a vTPM until tpm_vtpm_proxy is loaded"
                .to_owned(),
        });
    }

    ModuleCheckReport {
        required,
        present,
        missing_required,
        optional_missing,
    }
}

fn kernel_config_builtin_modules(config: &KernelConfig) -> BTreeSet<String> {
    [
        ("kvm_intel", "CONFIG_KVM_INTEL"),
        ("kvm_amd", "CONFIG_KVM_AMD"),
        ("vhost_net", "CONFIG_VHOST_NET"),
        ("tun", "CONFIG_TUN"),
        ("virtio_net", "CONFIG_VIRTIO_NET"),
        ("virtio_blk", "CONFIG_VIRTIO_BLK"),
        ("virtio_pci", "CONFIG_VIRTIO_PCI"),
        ("virtio_console", "CONFIG_VIRTIO_CONSOLE"),
        ("virtiofs", "CONFIG_VIRTIO_FS"),
        ("udmabuf", "CONFIG_UDMABUF"),
        ("usbip_host", "CONFIG_USBIP_HOST"),
        ("tpm_vtpm_proxy", "CONFIG_TCG_VTPM_PROXY"),
    ]
    .into_iter()
    .filter_map(|(module, config_key)| (config.get(config_key) == Some("y")).then_some(module))
    .map(str::to_owned)
    .collect()
}

/// Default path used by [`run_kernel_module_check`] when the
/// `proc_modules_override` is `None`. Exposed so tests can assert
/// the production read path is `/proc/modules`.
pub const PROC_MODULES_PATH: &str = "/proc/modules";
/// `/sys/module` directory used alongside `/proc/modules` to detect
/// built-in modules that appear in sysfs even when not listed in
/// `/proc/modules`.
pub const SYS_MODULE_DIR: &str = "/sys/module";

/// Side-effecting wrapper: read `/proc/modules` + `/sys/module` + the
/// `modules.builtin` text file, then dispatch to [`check_kernel_modules`].
///
/// Module detection order (union of all three sources):
///   1. `/proc/modules` — loadable modules currently inserted.
///   2. `/sys/module/<name>/` directory entries — also covers built-in
///      modules that the kernel exposes in sysfs even though they do
///      not appear in `/proc/modules`. This is the primary fix for
///      false-positive "missing" reports on hosts where virtio modules
///      are compiled in (`=y`) rather than loadable (`=m`).
///   3. `/lib/modules/$(uname -r)/modules.builtin` — text list of
///      built-in modules for offline/early-boot coverage, merged into
///      the `builtin` set.
///   4. `/boot/config-$(uname -r)` / `/proc/config.gz` — kernel config
///      for `CONFIG_*=y` built-in detection (existing path).
///
/// On any read failure we conservatively treat the failed source as
/// empty (the worst case is an unnecessary warning, not a silent skip).
/// Operators on hosts where `/proc` is unavailable can short-circuit
/// with `NIXLING_SKIP_KERNEL_MODULE_CHECK`.
pub fn run_kernel_module_check(resolver: &BundleResolver) -> ModuleCheckReport {
    // Step 1+2: /proc/modules union /sys/module.
    if let Err(error) = std::fs::read_to_string(PROC_MODULES_PATH) {
        tracing::warn!(
            path = PROC_MODULES_PATH,
            error = %error,
            "kernel-module-check: could not read /proc/modules; falling back to /sys/module and builtin evidence"
        );
    }
    let loaded = read_loaded_modules_at(Path::new(PROC_MODULES_PATH), Path::new(SYS_MODULE_DIR));

    // Step 3: modules.builtin text file (uname handled internally).
    let modules_builtin = read_builtin_modules_with_fallback();

    // Step 4: kernel config (existing path).
    let mut builtin: BTreeSet<String> = modules_builtin.names;
    if let Some(config) = read_kernel_config() {
        builtin.extend(kernel_config_builtin_modules(&config));
    }
    check_kernel_modules(resolver, &loaded, &builtin)
}

/// Map a fatal [`ModuleCheckReport`] into the public typed error
/// the daemon serve loop returns to bail startup.
pub fn fatal_typed_error(report: &ModuleCheckReport) -> TypedError {
    TypedError::HostKernelModulesMissing {
        missing: report.missing_required_summary(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::bundle::{Bundle, BundleGeneration};
    use nixling_core::host::HostJson;
    use nixling_core::manifest_v04::ManifestV04;
    use nixling_core::processes::{
        DagEdge, ProcessNode, ProcessRole, ProcessesJson, VmProcessDag, VmProcessInvariants,
    };

    const HOST_JSON_FIXTURE: &str =
        include_str!("../../../tests/fixtures/deny-unknown/host-valid.json");
    const MANIFEST_FIXTURE: &str =
        include_str!("../../../tests/golden/manifest_v04/baseline-vms.json");

    /// A profile stub valid against the schema. Built once via JSON to
    /// keep the test fixture decoupled from the (large,
    /// occasionally-resnaped) [`nixling_core::processes::RoleProfile`]
    /// struct.
    fn role_profile_stub_json() -> serde_json::Value {
        serde_json::json!({
            "profileId": "test-stub",
            "uid": 1000,
            "gid": 1000,
            "adr_carve_out": null,
            "caps": [],
            "namespaces": {
                "mount": true,
                "pid": true,
                "net": true,
                "ipc": true,
                "uts": true,
                "user": true
            },
            "seccompPolicyRef": null,
            "mountPolicy": {
                "readOnlyPaths": [],
                "writablePaths": [],
                "nixStoreReadOnly": true,
                "hideDeviceNodesByDefault": true
            },
            "cgroupPlacement": {
                "subtree": "nixling.slice/test",
                "controllers": [],
                "delegated": false
            }
        })
    }

    fn process_node_json(vm: &str, idx: usize, role: &ProcessRole) -> serde_json::Value {
        let role_str = serde_json::to_value(role).unwrap();
        serde_json::json!({
            "id": format!("{vm}-{idx}"),
            "role": role_str,
            "unit": null,
            "profile": role_profile_stub_json(),
            "readiness": []
        })
    }

    fn process_dag(vm: &str, roles: &[ProcessRole]) -> VmProcessDag {
        let nodes: Vec<ProcessNode> = roles
            .iter()
            .enumerate()
            .map(|(idx, role)| serde_json::from_value(process_node_json(vm, idx, role)).unwrap())
            .collect();
        VmProcessDag {
            vm: vm.to_owned(),
            nodes,
            edges: Vec::<DagEdge>::new(),
            invariants: VmProcessInvariants {
                swtpm_pre_start_flush: true,
                per_vm_audit_pipeline: true,
                usbip_gating: true,
                tpm_ownership_migration_without_running_vm_mutation: true,
            },
        }
    }

    /// Build a resolver from the canonical baseline-vms fixture and
    /// then apply a closure that mutates the manifest VMs (toggle
    /// graphics / tpm / usbip) plus any process DAG additions.
    fn build_resolver(
        mutate_vms: impl FnOnce(
            &mut std::collections::BTreeMap<String, nixling_core::manifest_v04::VmEntry>,
        ),
        process_vms: Vec<VmProcessDag>,
    ) -> BundleResolver {
        let host: HostJson = serde_json::from_str(HOST_JSON_FIXTURE).expect("host fixture parses");
        let mut manifest =
            ManifestV04::from_slice(MANIFEST_FIXTURE.as_bytes()).expect("manifest fixture parses");
        mutate_vms(&mut manifest.vms);
        let processes = ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: process_vms,
        };
        let bundle = Bundle {
            bundle_version: 4,
            schema_version: "v2".to_owned(),
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            sync_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: Some("2025-01-01T00:00:00Z".to_owned()),
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        BundleResolver::from_artifacts(bundle, host, processes, manifest)
    }

    fn loaded(names: &[&str]) -> LoadedModuleSet {
        LoadedModuleSet {
            names: names.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn empty_builtin() -> BTreeSet<String> {
        BTreeSet::new()
    }

    fn base_required_modules() -> Vec<&'static str> {
        vec![
            "kvm_intel",
            "vhost_net",
            "tun",
            "virtio_net",
            "virtio_blk",
            "virtio_pci",
            "virtio_console",
        ]
    }

    #[test]
    fn production_probe_logs_proc_modules_read_failure() {
        let source = include_str!("kernel_module_check.rs");
        assert!(
            source.contains("std::fs::read_to_string(PROC_MODULES_PATH)")
                && source.contains("kernel-module-check: could not read /proc/modules"),
            "run_kernel_module_check must log /proc/modules read failures before falling back"
        );
    }

    #[test]
    fn minimal_bundle_with_all_required_modules_passes() {
        let resolver = build_resolver(|_| {}, vec![]);
        let loaded = loaded(&base_required_modules());
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(!report.is_fatal(), "expected non-fatal, got {:?}", report);
        assert!(report.missing_required.is_empty());
        assert!(report.optional_missing.is_empty());
    }

    #[test]
    fn missing_kvm_alternative_is_fatal() {
        let resolver = build_resolver(|_| {}, vec![]);
        let loaded = loaded(&[
            "vhost_net",
            "tun",
            "virtio_net",
            "virtio_blk",
            "virtio_pci",
            "virtio_console",
        ]);
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(report.is_fatal());
        assert!(report.missing_required.contains("kvm_intel"));
        assert!(report.missing_required.contains("kvm_amd"));
        let summary = report.missing_required_summary();
        assert!(
            summary.contains("kvm_intel|kvm_amd"),
            "expected union form in summary, got {summary:?}"
        );
    }

    #[test]
    fn kvm_amd_alone_satisfies_alternative() {
        let resolver = build_resolver(|_| {}, vec![]);
        let loaded = loaded(&[
            "kvm_amd",
            "vhost_net",
            "tun",
            "virtio_net",
            "virtio_blk",
            "virtio_pci",
            "virtio_console",
        ]);
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(!report.is_fatal(), "{:?}", report);
        assert!(report.present.contains("kvm_amd"));
    }

    #[test]
    fn graphics_vm_requires_udmabuf() {
        let resolver = build_resolver(
            |vms| {
                vms.get_mut("corp-vm").unwrap().graphics = true;
            },
            vec![],
        );
        let loaded = loaded(&base_required_modules());
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(report.is_fatal());
        assert!(report.missing_required.contains("udmabuf"));
        assert!(!report.required.contains("drm_virtgpu"));
    }

    #[test]
    fn kernel_config_y_counts_as_builtin_module() {
        let config = KernelConfig::parse("CONFIG_UDMABUF=y\nCONFIG_DRM_VIRTIO_GPU=m\n");
        let builtins = kernel_config_builtin_modules(&config);
        assert!(builtins.contains("udmabuf"));
        assert!(!builtins.contains("virtio_gpu"));
    }

    #[test]
    fn virtiofs_required_only_when_vm_declares_virtiofsd_node() {
        let resolver_no_vfs = build_resolver(|_| {}, vec![]);
        let resolver_with_vfs = build_resolver(
            |_| {},
            vec![process_dag("corp-vm", &[ProcessRole::Virtiofsd])],
        );
        let base = loaded(&base_required_modules());
        let r1 = check_kernel_modules(&resolver_no_vfs, &base, &empty_builtin());
        assert!(!r1.is_fatal());
        assert!(!r1.required.contains("virtiofs"));

        let r2 = check_kernel_modules(&resolver_with_vfs, &base, &empty_builtin());
        assert!(r2.is_fatal());
        assert!(r2.missing_required.contains("virtiofs"));
    }

    #[test]
    fn usbip_vm_without_usbip_host_is_optional_degraded() {
        let resolver = build_resolver(
            |vms| {
                vms.get_mut("corp-vm").unwrap().usbip_yubikey = true;
            },
            vec![],
        );
        let loaded = loaded(&base_required_modules());
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(!report.is_fatal(), "{:?}", report);
        let row = report
            .optional_missing
            .iter()
            .find(|r| r.module == "usbip_host")
            .expect("usbip_host row");
        assert!(row.affected_vms.contains("corp-vm"));
        assert!(report.degraded_vms().contains("corp-vm"));
    }

    #[test]
    fn tpm_vm_without_tpm_vtpm_proxy_is_optional_degraded() {
        let resolver = build_resolver(
            |vms| {
                vms.get_mut("corp-vm").unwrap().tpm = true;
            },
            vec![],
        );
        let loaded = loaded(&base_required_modules());
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(!report.is_fatal());
        let row = report
            .optional_missing
            .iter()
            .find(|r| r.module == "tpm_vtpm_proxy")
            .expect("tpm row");
        assert!(row.affected_vms.contains("corp-vm"));
        assert!(report.degraded_vms().contains("corp-vm"));
    }

    #[test]
    fn graphics_without_nvidia_is_warn_only_not_degraded() {
        let resolver = build_resolver(
            |vms| {
                vms.get_mut("corp-vm").unwrap().graphics = true;
            },
            vec![],
        );
        let loaded = loaded(&[
            "kvm_intel",
            "vhost_net",
            "tun",
            "virtio_net",
            "virtio_blk",
            "virtio_pci",
            "virtio_console",
            "udmabuf",
        ]);
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(!report.is_fatal(), "{:?}", report);
        assert!(report.optional_missing.iter().any(|r| r.module == "nvidia"));
        assert!(report.degraded_vms().is_empty());
    }

    #[test]
    fn builtin_module_counts_as_present() {
        let resolver = build_resolver(|_| {}, vec![]);
        let loaded = loaded(&[
            "vhost_net",
            "tun",
            "virtio_net",
            "virtio_blk",
            "virtio_pci",
            "virtio_console",
        ]);
        let mut builtin = BTreeSet::new();
        builtin.insert("kvm_intel".to_owned());
        let report = check_kernel_modules(&resolver, &loaded, &builtin);
        assert!(!report.is_fatal(), "{:?}", report);
        assert!(report.present.contains("kvm_intel"));
    }

    #[test]
    fn fatal_typed_error_carries_missing_summary() {
        let resolver = build_resolver(|_| {}, vec![]);
        let loaded = loaded(&[]);
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(report.is_fatal());
        let err = fatal_typed_error(&report);
        let msg = err.message();
        assert!(msg.contains("kvm_intel|kvm_amd"), "msg={msg}");
        assert!(msg.contains("vhost_net"), "msg={msg}");
    }

    // ------------------------------------------------------------------
    // /sys/module and modules.builtin detection tests
    // ------------------------------------------------------------------

    /// Virtio modules compiled as =y appear in `/sys/module/<name>/` but
    /// NOT in `/proc/modules`. `read_loaded_modules_at` unions both
    /// sources, so a built-in virtio module appearing only in the sysfs
    /// dir must not be reported as missing.
    #[test]
    fn builtin_virtio_module_in_sys_module_dir_is_detected_as_present() {
        use nixling_host::modules::read_loaded_modules_at;
        use std::fs;

        let base = std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let test_dir = base.join("kernel-module-check-sysfs-test");
        let proc_modules_path = test_dir.join("proc_modules");
        let sys_module_dir = test_dir.join("sys_module");

        // Create a fake /proc/modules with all required except virtio_net.
        let proc_content = "vhost_net 12345 0 - Live\n\
                            tun 12345 0 - Live\n\
                            kvm_intel 12345 0 - Live\n\
                            virtio_blk 12345 0 - Live\n\
                            virtio_pci 12345 0 - Live\n\
                            virtio_console 12345 0 - Live\n";
        fs::create_dir_all(&test_dir).expect("create test dir");
        fs::write(&proc_modules_path, proc_content).expect("write proc_modules");

        // Put virtio_net only in /sys/module (simulating =y built-in).
        let virtio_net_dir = sys_module_dir.join("virtio_net");
        fs::create_dir_all(&virtio_net_dir).expect("create virtio_net sys dir");

        let loaded = read_loaded_modules_at(&proc_modules_path, &sys_module_dir);
        assert!(
            loaded.names.contains("virtio_net"),
            "virtio_net in /sys/module must be detected as present; names={:?}",
            loaded.names
        );

        let resolver = build_resolver(|_| {}, vec![]);
        let report = check_kernel_modules(&resolver, &loaded, &empty_builtin());
        assert!(
            !report.is_fatal(),
            "virtio_net only in /sys/module must not trigger fatal check: {:?}",
            report
        );
        assert!(
            report.present.contains("virtio_net"),
            "virtio_net must appear in present set; present={:?}",
            report.present
        );

        let _ = fs::remove_dir_all(&test_dir);
    }

    /// `run_kernel_module_check` pulls from both `/proc/modules` and
    /// `/sys/module`, so the PROC_MODULES_PATH constant must remain the
    /// production `/proc/modules` path.
    #[test]
    fn run_kernel_module_check_still_reads_proc_modules() {
        assert_eq!(PROC_MODULES_PATH, "/proc/modules");
        assert_eq!(SYS_MODULE_DIR, "/sys/module");
    }
}
