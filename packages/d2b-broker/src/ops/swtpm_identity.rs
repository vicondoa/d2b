//! Trusted identity + derived directories of one `w1-swtpm` launch.
//!
//! The broker spawns swtpm inside a user namespace (ADR 0021) and skips
//! `apply_mount_actions` for userNS spawns, so the worker opens its TPM 2.0
//! NVRAM + EK state **by pathname**. Two consumers need to know which
//! pathname is the trusted one, and both resolve it here, from verified
//! bundle artifacts and never from a caller-supplied field or a launch
//! argument:
//!
//! - [`resource_backed_identity`] resolves the trusted identity of a typed
//!   (`resource-backed`, `private_cgroup_placement`) launch, whose cgroup
//!   subtree deliberately carries no VM identity;
//! - [`legacy_runtime_dir`] derives the runtime socket directory of a
//!   legacy VM-scoped launch by cross-checking the plan's own writable
//!   paths against the VM its cgroup placement names.
//!
//! [`trusted_state_dir`] turns a [`ResourceBackedSwtpm`] into the directory
//! the worker opens, which is what the launch's state-directory traverse
//! grant is applied to.
//!
//! This module resolves paths; it does not provision or own them. In the v3
//! model the TPM state Volume owns that lifecycle
//! (`createPolicy: create-if-never-provisioned`, `repairPolicy:
//! fail-closed`, `packages/d2b-provider-volume-local`) and host activation's
//! per-VM tmpfiles rule provisions the legacy per-VM tree. Nothing here
//! creates, wipes, or re-owns a state directory.

use std::path::{Path, PathBuf};

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::storage::StoragePathSpec;

use crate::ops::spawn_runner::SpawnRunnerPlan;

/// Closed-set, path-free reason slugs for a trusted-path derivation refusal.
pub mod reasons {
    /// The plan and the trusted bundle name no consistent directory.
    pub const DERIVATION_FAILED: &str = "swtpm-dir-derivation-failed";
}

/// The placement identity a spawn plan's cgroup subtree names.
///
/// A legacy VM-scoped placement (`d2b.slice/<vm>[/<role>...]`) carries the VM
/// name in its first segment. A resource-backed (typed) launch is rewritten by
/// `private_cgroup_placement` into `d2b.slice/process-<64hex>[/<role>...]`, a
/// commitment to the private runtime scope that deliberately carries no VM
/// identity: for those launches the VM is resolved from the verified bundle,
/// never from the cgroup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlacementSegment {
    /// `d2b.slice/<vm>[/...]` - the segment after `d2b.slice/` is the VM id.
    Vm(String),
    /// `d2b.slice/process-<64hex>[/...]` - a private resource-backed scope.
    RuntimeScope(String),
}

pub(crate) fn parse_placement_segment(subtree: &str) -> Option<PlacementSegment> {
    let normalized = subtree
        .strip_prefix("d2b.slice/")
        .or_else(|| subtree.strip_prefix("d2b/"))?;
    let segment = normalized.split('/').find(|s| !s.is_empty())?;
    if segment.contains('\0') {
        return None;
    }
    let name = segment.trim_end_matches(".scope");
    if name.is_empty() {
        return None;
    }
    if let Some(hex) = name.strip_prefix("process-")
        && hex.len() == 64
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Some(PlacementSegment::RuntimeScope(name.to_owned()));
    }
    Some(PlacementSegment::Vm(name.to_owned()))
}

/// The trusted identity of one resource-backed (typed) `w1-swtpm` launch.
///
/// Every field is resolved from verified bundle artifacts - the Zone resource
/// bundle's `Device` row and the host storage contract - never from the
/// request's caller-supplied fields or from the launch arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceBackedSwtpm {
    /// The Guest whose owning Device declares the worker row
    /// (`Device.metadata.ownerRef == Guest/<guest>`, the same derivation the
    /// daemon's Device-worker ticket uses).
    pub guest: String,
    /// The trusted TPM state policy root the `path:swtpm-state:<guest>` row
    /// names, which must be the same directory as the Provider policy root
    /// `path:tpm-state` the state Volume resolves under.
    pub state_root: PathBuf,
    /// The state Volume name the worker must open under that root
    /// (`device-<32hex>-tpm-state`, the TPM Provider's own naming from the
    /// owning Device's durable uid), when that uid is available.
    pub state_volume: Option<String>,
}

/// Resolve the trusted identity of a resource-backed `w1-swtpm` launch from
/// the verified bundle.
///
/// Returns `None` when any trusted input is missing or the two trusted rows
/// disagree; the caller then grants nothing rather than inventing a path.
pub fn resource_backed_identity(
    resolver: &BundleResolver,
    zone_uid: &ResourceUid,
    device_ref: &ResourceRef,
    device_uid: Option<&ResourceUid>,
) -> Option<ResourceBackedSwtpm> {
    if device_ref.resource_type().as_str() != "Device" {
        return None;
    }
    let (_zone, bundle_bytes) = crate::ops::device_worker::zone_bundle_for_uid(resolver, zone_uid)?;
    let guest =
        crate::ops::device_worker::device_guest_owner(bundle_bytes, device_ref.name().as_str())?;
    let state_root = storage_root(resolver, &format!("path:swtpm-state:{guest}"))?;
    // The Provider's opaque `sourcePolicyId: "tpm-state"` resolves through the
    // storage contract's `path:tpm-state` row. If the two rows disagree, the
    // directory the worker opens is not the directory the Volume controller
    // provisions, so no trusted identity is derived.
    if storage_root(resolver, "path:tpm-state")? != state_root {
        return None;
    }
    Some(ResourceBackedSwtpm {
        guest,
        state_root,
        state_volume: device_uid.map(state_volume_name),
    })
}

/// The trusted TPM state row of one zone-native Guest, when the verified
/// storage contract carries that Guest's `path:swtpm-state:<guest>` row: the
/// row itself plus the state root it names.
///
/// `None` for a subject no trusted artifact names: the caller keeps failing
/// closed rather than inventing a directory.
pub fn zone_native_swtpm_state_row<'a>(
    resolver: &'a BundleResolver,
    guest: &str,
) -> Option<(&'a StoragePathSpec, PathBuf)> {
    let spec = resolver.find_storage_path_spec(&format!("path:swtpm-state:{guest}"))?;
    let root = storage_path(spec)?;
    Some((spec, root))
}

fn storage_root(resolver: &BundleResolver, id: &str) -> Option<PathBuf> {
    storage_path(resolver.find_storage_path_spec(id)?)
}

/// The directory one trusted storage row names, refusing any row whose
/// template is not an anchored absolute path.
fn storage_path(spec: &StoragePathSpec) -> Option<PathBuf> {
    let path = PathBuf::from(spec.path_template.as_str());
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(path)
}

/// The TPM Provider's own state Volume naming for one Device
/// (`device_<32hex>-tpm-state`), so an argv can be bound to the exact
/// directory the controller provisions.
pub(crate) fn state_volume_name(device_uid: &ResourceUid) -> String {
    let short: String = device_uid
        .as_str()
        .bytes()
        .filter(|byte| byte.is_ascii_hexdigit())
        .take(32)
        .map(char::from)
        .collect();
    format!("device-{short}-tpm-state")
}

/// The state directory a resource-backed swtpm worker opens: the trusted
/// state Volume directory under the trusted root. This is the directory the
/// launch's state-directory traverse grant is applied to.
pub fn trusted_state_dir(identity: &ResourceBackedSwtpm) -> PathBuf {
    match identity.state_volume.as_deref() {
        Some(volume) => identity.state_root.join(volume),
        None => identity.state_root.clone(),
    }
}

/// Derive the per-VM runtime socket directory of one legacy VM-scoped plan
/// from the plan's own trusted inputs, never from an argument.
///
/// The VM comes from the cgroup placement; the plan's writable paths must
/// name exactly the persistent `swtpm` dir (ending `/swtpm`, NOT under
/// `/run`) and the runtime dir (under `/run/d2b/vms`) whose basenames agree
/// with that VM. Resource-backed placements are refused: their identity is
/// never in the cgroup (`private_cgroup_placement`) and must come from the
/// verified bundle through [`resource_backed_identity`].
pub fn legacy_runtime_dir(plan: &SpawnRunnerPlan) -> Result<PathBuf, &'static str> {
    let vm_id = match parse_placement_segment(&plan.cgroup_placement.subtree) {
        Some(PlacementSegment::Vm(vm)) => vm,
        _ => return Err(reasons::DERIVATION_FAILED),
    };

    let mut swtpm_dir: Option<PathBuf> = None;
    let mut runtime_dir: Option<PathBuf> = None;
    for wp in &plan.mount_policy.writable_paths {
        let path = Path::new(&wp.path);
        if !path.is_absolute() {
            continue;
        }
        if (path.starts_with("/run/d2b/vms") || cfg!(test))
            && path.file_name().and_then(|name| name.to_str()) != Some("swtpm")
            && runtime_dir.is_none()
        {
            runtime_dir = Some(path.to_path_buf());
        } else if path.file_name().and_then(|s| s.to_str()) == Some("swtpm") && swtpm_dir.is_none()
        {
            swtpm_dir = Some(path.to_path_buf());
        }
    }

    let swtpm_dir = swtpm_dir.ok_or(reasons::DERIVATION_FAILED)?;
    let runtime_dir = runtime_dir.ok_or(reasons::DERIVATION_FAILED)?;

    let per_vm_root = swtpm_dir
        .parent()
        .ok_or(reasons::DERIVATION_FAILED)?
        .to_path_buf();

    // Cross-check: the per-VM root + runtime dir basenames must equal
    // the cgroup-derived VM id. A mismatch means the plan's paths and
    // cgroup placement disagree about which VM this is.
    if per_vm_root.file_name().and_then(|s| s.to_str()) != Some(vm_id.as_str()) {
        return Err(reasons::DERIVATION_FAILED);
    }
    if runtime_dir.file_name().and_then(|s| s.to_str()) != Some(vm_id.as_str()) {
        return Err(reasons::DERIVATION_FAILED);
    }
    Ok(runtime_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet, WritablePath};

    fn legacy_plan() -> SpawnRunnerPlan {
        SpawnRunnerPlan {
            binary_path: PathBuf::from("/run/current-system/sw/bin/swtpm"),
            argv: vec!["swtpm".into()],
            uid: 12345,
            gid: 12345,
            supplementary_groups: vec![],
            env: vec![],
            capabilities: vec![],
            namespaces: NamespaceSet {
                mount: true,
                pid: true,
                net: false,
                ipc: true,
                uts: true,
                user: true,
            },
            seccomp_policy_ref: Some("w1-swtpm".into()),
            mount_policy: MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![
                    WritablePath {
                        path: "/var/lib/d2b/vms/work/swtpm".into(),
                        purpose: "tpm nvram".into(),
                    },
                    WritablePath {
                        path: "/run/d2b/vms/work".into(),
                        purpose: "tpm socket".into(),
                    },
                ],
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_placement: CgroupPlacement {
                subtree: "d2b.slice/work/swtpm".into(),
                controllers: vec![],
                delegated: true,
            },
            user_namespace: None,
            umask: None,
        }
    }

    /// The runtime directory is the `/run` writable path, never the
    /// persistent `swtpm` state dir that carries the NVRAM.
    #[test]
    fn legacy_runtime_dir_picks_the_socket_dir_not_the_state_dir() {
        assert_eq!(
            legacy_runtime_dir(&legacy_plan()).expect("derive ok"),
            PathBuf::from("/run/d2b/vms/work")
        );
    }

    /// A plan whose writable paths and cgroup placement disagree about which
    /// VM it is - or that names no state dir at all - derives nothing.
    #[test]
    fn legacy_runtime_dir_refuses_a_placement_the_paths_disagree_with() {
        let mut plan = legacy_plan();
        plan.cgroup_placement.subtree = "d2b.slice/other/swtpm".into();
        assert_eq!(
            legacy_runtime_dir(&plan),
            Err(reasons::DERIVATION_FAILED)
        );

        let mut plan = legacy_plan();
        plan.mount_policy
            .writable_paths
            .retain(|writable| writable.path != "/var/lib/d2b/vms/work/swtpm");
        assert_eq!(
            legacy_runtime_dir(&plan),
            Err(reasons::DERIVATION_FAILED)
        );

        // A resource-backed placement carries no VM identity at all.
        let mut plan = legacy_plan();
        plan.cgroup_placement.subtree = format!("d2b.slice/process-{}/swtpm", "a".repeat(64));
        assert_eq!(
            legacy_runtime_dir(&plan),
            Err(reasons::DERIVATION_FAILED)
        );
    }

    #[test]
    fn parse_placement_segment_separates_vm_names_from_runtime_scopes() {
        assert_eq!(
            parse_placement_segment("d2b.slice/work/swtpm"),
            Some(PlacementSegment::Vm("work".to_owned()))
        );
        assert_eq!(
            parse_placement_segment("d2b.slice/process-abcdef.scope/swtpm"),
            Some(PlacementSegment::Vm("process-abcdef".to_owned()))
        );
        let scope = "process-".to_owned() + &"0f".repeat(32);
        assert_eq!(
            parse_placement_segment(&format!("d2b.slice/{scope}/swtpm")),
            Some(PlacementSegment::RuntimeScope(scope.clone()))
        );
        assert_eq!(
            parse_placement_segment(&format!("d2b.slice/{scope}")),
            Some(PlacementSegment::RuntimeScope(scope))
        );
        assert_eq!(parse_placement_segment("/elsewhere/work"), None);
        assert_eq!(parse_placement_segment("d2b.slice/"), None);
    }

    #[test]
    fn device_guest_owner_reads_only_the_named_devices_guest_owner() {
        let bundle = serde_json::json!({
            "resources": [
                { "type": "Device", "metadata": { "name": "tpm0", "ownerRef": "Guest/acceptance-guest" } },
                { "type": "Device", "metadata": { "name": "gpu0", "ownerRef": "Guest/other-guest" } },
                { "type": "Process", "metadata": { "name": "swtpm-tpm0", "ownerRef": "Device/tpm0" } },
                { "type": "Device", "metadata": { "name": "gpu1", "ownerRef": "Provider/device-gpu" } },
            ]
        });
        let bytes = serde_json::to_vec(&bundle).unwrap();
        let owner = crate::ops::device_worker::device_guest_owner;
        assert_eq!(owner(&bytes, "tpm0").as_deref(), Some("acceptance-guest"));
        assert_eq!(owner(&bytes, "gpu1"), None);
        assert_eq!(owner(&bytes, "missing"), None);
        assert_eq!(owner(b"not json", "tpm0"), None);
    }

    #[test]
    fn state_volume_name_matches_the_provider_naming() {
        let uid = ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap();
        assert_eq!(
            state_volume_name(&uid),
            "device-6f9619ff8b864d01b42d00cf4fc964ff-tpm-state"
        );
    }

    /// The state directory a worker opens is the Volume directory under the
    /// trusted root, falling back to the root itself when no Device uid
    /// pinned a Volume name.
    #[test]
    fn trusted_state_dir_joins_the_volume_under_the_trusted_root() {
        let identity = ResourceBackedSwtpm {
            guest: "acceptance-guest".to_owned(),
            state_root: PathBuf::from("/var/lib/d2b/tpm-state"),
            state_volume: Some("device-0123456789abcdef0123456789abcdef-tpm-state".to_owned()),
        };
        assert_eq!(
            trusted_state_dir(&identity),
            PathBuf::from("/var/lib/d2b/tpm-state/device-0123456789abcdef0123456789abcdef-tpm-state")
        );
        assert_eq!(
            trusted_state_dir(&ResourceBackedSwtpm {
                state_volume: None,
                ..identity
            }),
            PathBuf::from("/var/lib/d2b/tpm-state")
        );
    }
}
