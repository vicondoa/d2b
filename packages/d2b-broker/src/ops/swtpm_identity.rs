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
//!   subtree deliberately carries no VM identity. The Device it names is not
//!   the launch's to choose: it is the one
//!   [`crate::ops::device_worker::repin_launch_scope`] resolved from the
//!   launched row's own `metadata.ownerRef` in the verified Zone bundle, and
//!   a launch asserting any other Device or Guest was refused before it got
//!   here;
//! - [`legacy_runtime_dir`] derives the runtime socket directory of a
//!   legacy VM-scoped launch by cross-checking the plan's own writable
//!   paths against the VM its cgroup placement names.
//!
//! [`trusted_state_dir`] turns a [`ResourceBackedSwtpm`] into the directory
//! the worker opens, which is what the launch's state-directory traverse
//! grant is applied to, and
//! [`verify_argv_names_only_trusted_paths`] fences the launch's arguments
//! against it: because the worker opens the directory by pathname, a
//! launch that names anywhere else is refused rather than trusted with a
//! grant for a directory it does not name. Containment is textual, so a
//! candidate is anchored before it is compared and a path-shaped field that
//! is not an absolute path is refused rather than skipped.
//!
//! This module resolves paths; it does not provision or own them. In the v3
//! model the TPM state Volume owns that lifecycle
//! (`createPolicy: create-if-never-provisioned`, `repairPolicy:
//! fail-closed`, `packages/d2b-provider-volume-local`) and host activation's
//! per-VM tmpfiles rule provisions the legacy per-VM tree. Nothing here
//! creates, wipes, or re-owns a state directory.

use std::path::{Path, PathBuf};

use d2b_contracts_resource::v3::ResourceUid;
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
    /// The state Volume name the worker opens under that root
    /// (`device-<32hex>-tpm-state`, the TPM Provider's own naming from the
    /// owning Device's durable uid, which the verified Zone bundle's Device
    /// row derives). It is never absent: an identity with no Device-scoped
    /// leaf would name the *shared* state root, which no launch may be
    /// granted.
    pub state_volume: String,
}

/// Why one resource-backed launch's arguments do not name the trusted paths.
///
/// A closed set of path-free slugs: the launch-failure envelope and the
/// `kind="critical"` audit record render the slug, never a caller-supplied
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedPathMismatch {
    /// The launch arguments name a path that is neither the trusted state
    /// directory (or anything under it) nor the trusted per-Guest runtime
    /// directory (or anything under it).
    ArgvOutsideTrustedPaths,
    /// A path-shaped argument names something that is not an anchored
    /// absolute path: relative, or carrying a `.`/`..` component, so it is a
    /// textual prefix of a trusted root while resolving somewhere else.
    ArgvUnanchoredPath,
}

impl std::fmt::Display for TrustedPathMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArgvOutsideTrustedPaths => {
                f.write_str("device-worker-argv-outside-trusted-paths")
            }
            Self::ArgvUnanchoredPath => f.write_str("device-worker-argv-unanchored-path"),
        }
    }
}

/// Resolve the trusted identity of a resource-backed `w1-swtpm` launch from
/// the verified bundle and the scope pinned against it.
///
/// The scope is not a payload claim: it is
/// [`crate::ops::device_worker::resolve_launch_scope`]'s own answer, which
/// names the `Device` row that owns the *launched row* in the verified Zone
/// resource bundle, that Device's durable uid, and the Guest that Device
/// declares. The state Volume directory is therefore named from the launched
/// row's own owner, so a payload that names another Device - carrying that
/// Device's publicly derivable uid - never reaches this function: the pin
/// refused it by name first.
///
/// What is left is the storage contract, and a gap in it is a gap rather than
/// a contradiction: the state root comes from the `path:swtpm-state:<guest>`
/// row of the pinned Guest and must be the same directory the Provider policy
/// root `path:tpm-state` names, or no identity is derived at all.
pub fn resource_backed_identity(
    resolver: &BundleResolver,
    scope: &crate::ops::device_worker::DeviceWorkerScope,
) -> Option<ResourceBackedSwtpm> {
    let state_root = storage_root(resolver, &format!("path:swtpm-state:{}", scope.guest()))?;
    // The Provider's opaque `sourcePolicyId: "tpm-state"` resolves through the
    // storage contract's `path:tpm-state` row. If the two rows disagree, the
    // directory the worker opens is not the directory the Volume controller
    // provisions, so no trusted identity is derived.
    if storage_root(resolver, "path:tpm-state") != Some(state_root.clone()) {
        return None;
    }
    Some(ResourceBackedSwtpm {
        guest: scope.guest.clone(),
        state_root,
        state_volume: state_volume_name(&scope.device_uid),
    })
}

/// The trusted TPM state row of one zone-native Guest, when the verified
/// storage contract carries that Guest's `path:swtpm-state:<guest>` row: the
/// row itself plus the state root it names.
///
/// `None` for a subject no trusted artifact names: the caller keeps failing
/// closed rather than inventing a directory.
// Exists for this module's tests and the cfg(test) `PrepareStateDir` op: no
// live dispatch arm resolves this row by itself.
#[cfg(test)]
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
    identity.state_root.join(&identity.state_volume)
}

/// The worker's persistent TPM state blob, by name, inside the state
/// directory. The worker creates it on its first start and keeps its own
/// header inside it, so a blob of zero length is a blob whose header was
/// never written.
const NVRAM_BLOB_NAME: &str = "tpm2-00.permall";

/// The long-lived worker's own control socket inside its per-Guest runtime
/// directory, by name. It is the one entry a previous run of that worker can
/// leave where the next one must bind.
pub(crate) const WORKER_SOCKET_NAME: &str = "tpm.sock";

/// Why a state directory cannot carry the worker that is about to open it.
///
/// A closed set of path-free slugs: the audit record and the launch-failure
/// envelope render the slug and the errno class, never a raw path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateDirHealth {
    /// The directory is absent, or carries no persistent state blob at all:
    /// the worker manufactures on its first start, and the caller's own
    /// presence rule (`StateVolumeLeaf`) decides whether that is a refusal.
    Usable,
    /// The directory carries a persistent state blob of zero length. The
    /// blob's header is inside the blob, so a headerless blob is read as a
    /// corrupt state rather than as a new one: the worker enters a fatal
    /// power-on failure and exits, and every later start repeats it. This is
    /// not a state the worker can recover from, and the broker will not
    /// touch the bytes - that blob is the Endorsement Key's home, and
    /// guessing when discarding it is safe destroys unrecoverable key
    /// material. Refuse, and say so.
    HeaderlessState,
    /// The state directory could not be read. The refusal stays path-free.
    Unreadable(std::io::ErrorKind),
}

impl std::fmt::Display for StateDirHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usable => f.write_str("state-dir-usable"),
            Self::HeaderlessState => f.write_str("state-dir-persistent-state-headerless"),
            Self::Unreadable(kind) => {
                write!(f, "state-dir-unreadable:{kind:?}")
            }
        }
    }
}

/// Classify the state directory a resource-backed worker is about to open,
/// without modifying anything in it.
///
/// The check is deliberately narrow and deliberately non-destructive: it
/// looks at exactly one entry, the persistent state blob, and reports a
/// zero-length one. It never opens the blob for writing, never truncates it,
/// never removes it, and never re-manufactures a replacement. An operator,
/// not the broker, decides what unrecoverable key material is worth.
///
/// The one stat it needs runs on the async runtime's own blocking pool, the
/// way every other filesystem read on the launch path does: this runs inside
/// an async spawn arm, and a `std::fs` call here would block a runtime
/// worker on the state directory's first byte.
pub(crate) async fn classify_state_dir(identity: &ResourceBackedSwtpm) -> StateDirHealth {
    let state_dir = trusted_state_dir(identity);
    let blob = state_dir.join(NVRAM_BLOB_NAME);
    let metadata = match tokio::fs::symlink_metadata(&blob).await {
        Ok(metadata) => metadata,
        // Absent: the worker creates it. A symlink is never followed, and one
        // standing where the blob belongs is not a state the worker can use,
        // so it is read as an unreadable directory rather than as a size.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return StateDirHealth::Usable,
        Err(error) => return StateDirHealth::Unreadable(error.kind()),
    };
    if !metadata.is_file() {
        return StateDirHealth::Unreadable(std::io::ErrorKind::InvalidData);
    }
    if metadata.len() == 0 {
        return StateDirHealth::HeaderlessState;
    }
    StateDirHealth::Usable
}

/// Fence one resource-backed launch's arguments against the trusted paths the
/// broker grants on, and refuse an argument set that names anything else.
///
/// The worker opens its state **by pathname** (the broker skips
/// `apply_mount_actions` under a user namespace), so a launch that names a
/// directory the broker never derived would hand the worker a path the
/// traverse grant does not cover - or one it does not own. Every path an
/// argument carries, whether the argument is the path itself or a
/// comma-separated `key=path` field, must therefore be the trusted state
/// directory or something under it, or the trusted per-Guest runtime
/// directory or something under it. Nothing else in the vocabulary is
/// consulted: the rule reads the arguments' paths, not the family that
/// spells them, so it holds for whichever flag names them.
///
/// Containment is a textual component comparison, so every candidate is
/// *anchored* before it is compared: `/state/../etc/shadow` is a component
/// prefix of the trusted state root while resolving at `/etc/shadow`, so an
/// unanchored candidate is refused rather than admitted. A candidate that is
/// not a path at all (`unixio`, `0660`, `--tpm2`) is not a target, but a
/// value in a path-shaped field always is - a relative or empty one is
/// refused, never skipped.
///
/// `argv[0]` is the launch's own executable name, not a path the launch
/// opens, so it is not a target.
pub fn verify_argv_names_only_trusted_paths(
    plan: &SpawnRunnerPlan,
    identity: &ResourceBackedSwtpm,
    runtime_dir: &Path,
) -> Result<(), TrustedPathMismatch> {
    let state_dir = trusted_state_dir(identity);
    for argument in plan.argv.iter().skip(1) {
        for field in argument.split(',') {
            let (key, value) = match field.split_once('=') {
                Some((key, value)) => (Some(key), value),
                None => (None, field),
            };
            if !names_a_path(key, value) {
                continue;
            }
            let named = Path::new(value);
            if !crate::live_handlers::is_anchored_absolute(named) {
                return Err(TrustedPathMismatch::ArgvUnanchoredPath);
            }
            if !is_within(named, &state_dir) && !is_within(named, runtime_dir) {
                return Err(TrustedPathMismatch::ArgvOutsideTrustedPaths);
            }
        }
    }
    Ok(())
}

/// The `key=path` field names this launch vocabulary spells a path with.
const PATH_FIELD_NAMES: [&str; 4] = ["path", "mountPath", "dir", "file"];

/// Whether one field of a launch argument names a path, which is what makes
/// its value a target the fence has to resolve.
///
/// A field named by [`PATH_FIELD_NAMES`] is a path claim whatever its value
/// spells, so `dir=../../../tmp/evil` is refused instead of being read as a
/// plain token. Every other field is judged by its value: a separator or a
/// bare relative step names a path the worker may open, a plain word does not.
fn names_a_path(key: Option<&str>, value: &str) -> bool {
    if key.is_some_and(|key| PATH_FIELD_NAMES.contains(&key)) {
        return true;
    }
    value.contains('/') || matches!(value, "." | "..")
}

/// Whether `candidate` is `root` itself or lives strictly inside it.
fn is_within(candidate: &Path, root: &Path) -> bool {
    candidate == root || candidate.starts_with(root)
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
        presentation: crate::ops::spawn_runner::PresentationRealization::NamespaceFirstServiceSource,
        admitted_presentation: crate::ops::spawn_runner::AdmittedPresentation {
            private_execution_root: std::path::PathBuf::new(),
            binds: Vec::new(),
        },
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

    /// The state directory a worker opens is the Device's own Volume
    /// directory under the trusted root. There is no root-only fallback: an
    /// identity with no Device-scoped leaf would name the *shared* state
    /// root, which is a cross-device directory no launch may be granted.
    #[test]
    fn trusted_state_dir_joins_the_volume_under_the_trusted_root() {
        let identity = ResourceBackedSwtpm {
            guest: "acceptance-guest".to_owned(),
            state_root: PathBuf::from("/var/lib/d2b/tpm-state"),
            state_volume: "device-0123456789abcdef0123456789abcdef-tpm-state".to_owned(),
        };
        assert_eq!(
            trusted_state_dir(&identity),
            PathBuf::from(
                "/var/lib/d2b/tpm-state/device-0123456789abcdef0123456789abcdef-tpm-state"
            )
        );
    }
}

#[cfg(test)]
mod trusted_identity_tests {
    use super::*;
    use crate::ops::device_worker::DeviceWorkerScope;
    use d2b_contracts::contract_id::{ContractId, PathTemplate};
    use d2b_contracts_resource::v3::ResourceRef;
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;
    use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
    use d2b_core::storage::{
        ActorKind, ActorRef, CleanupPolicy, LeaseClass, PrincipalKind, PrincipalRef, RepairPolicy,
        SensitivityClass, StorageAdoptionPolicy, StorageInvariant, StorageJson, StorageLifecycle,
        StoragePathKind, StoragePathSpec, StoragePersistence, StorageRestartPolicy,
    };
    use std::collections::BTreeMap;

    const ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const GUEST: &str = "acceptance-guest";
    const STATE_ROOT: &str = "/var/lib/d2b/tpm-state";
    /// The durable uid the manager mints for the fixture's Device row
    /// `("work", "Device", "tpm0")`.
    const TPM0_UID: &str = "0940f65d-b4f7-427f-992b-a65d545ec479";
    const TPM0_VOLUME: &str = "device-0940f65db4f7427f992ba65d545ec479-tpm-state";

    fn row(resource_type: &str, name: &str, owner_ref: &str) -> serde_json::Value {
        serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_type,
            "metadata": {
                "name": name,
                "zone": "work",
                "ownerRef": owner_ref,
            },
            "spec": {},
        })
    }

    /// The canonical content hash `ResourceBundle` computes for a resource
    /// array, so the fixture bundle verifies.
    fn fixture_content_hash(resources: &[serde_json::Value]) -> String {
        use d2b_contracts_resource::v3::resource_schema::{
            CanonicalJsonValue, canonical_json_bytes, framed_canonical_digest,
        };

        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&serde_json::Value::Array(resources.to_vec()))
                .expect("fixture resources serialize"),
        )
        .expect("fixture resources are canonical JSON");
        framed_canonical_digest(
            "d2b:v3:resource-bundle",
            &canonical_json_bytes(&canonical).expect("fixture resources encode"),
        )
    }

    /// One storage row of the verified contract. Only its id, scope and
    /// anchored path template are load-bearing for a state identity.
    fn state_row(id: &str, path: &str) -> StoragePathSpec {
        let principal = |kind| PrincipalRef {
            kind,
            value: ContractId::parse("d2bd").unwrap(),
        };
        StoragePathSpec {
            id: ContractId::parse(id).unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: PathTemplate::parse(path).unwrap(),
            kind: StoragePathKind::Directory,
            lifecycle: StorageLifecycle::BootScopedReadoptable,
            persistence: StoragePersistence::BootScoped,
            owner: principal(PrincipalKind::Uid),
            group: principal(PrincipalKind::Gid),
            mode: "0700".to_owned(),
            access_acl: Vec::new(),
            default_acl: Vec::new(),
            creator: ActorRef {
                kind: ActorKind::NixModule,
                value: ContractId::parse("tmpfiles").unwrap(),
            },
            writers: Vec::new(),
            readers: Vec::new(),
            cleanup_policy: CleanupPolicy::Never,
            repair_policy: RepairPolicy::NixActivation,
            restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
            adoption_policy: StorageAdoptionPolicy::QuarantineOnAmbiguity,
            lease_class: LeaseClass::ProcessPidfd,
            sensitivity: SensitivityClass::Private,
            no_follow: true,
            recursive: false,
            invariants: vec![StorageInvariant::NoSymlink],
        }
    }

    /// Two Devices in two Guests, each with the worker row the bundle
    /// declares as its child, plus the two storage rows a resource-backed
    /// TPM state identity resolves through.
    fn resolver() -> BundleResolver {
        let resources = vec![
            row("Device", "gpu0", "Guest/other-guest"),
            row("Device", "tpm0", "Guest/acceptance-guest"),
            row("Process", "swtpm-gpu0", "Device/gpu0"),
            row("Process", "swtpm-tpm0", "Device/tpm0"),
        ];
        let bundle = serde_json::json!({
            "schemaVersion": 3,
            "bundleVersion": 1,
            "zone": "work",
            "zoneUid": ZONE_UID,
            "contentHash": fixture_content_hash(&resources),
            "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
            "schemaFingerprints": {},
            "providerSchemaDigests": {},
            "resources": resources,
            "generatedAt": "1970-01-01T00:00:00.000Z",
        });
        let bundle_manifest = Bundle {
            bundle_version: 11,
            schema_version: "v2".to_owned(),
            storage_path: Some("storage.json".to_owned()),
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        let host: HostJson = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture parses");
        let manifest = ManifestV04::from_slice(
            include_str!("../../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture parses");
        let mut resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            bundle_manifest,
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::from([(
                "work".to_owned(),
                serde_json::to_vec(&bundle).expect("zone bundle bytes"),
            )]),
        );
        resolver.set_storage(StorageJson {
            schema_version: "v2".to_owned(),
            roots: Vec::new(),
            paths: vec![
                state_row(&format!("path:swtpm-state:{GUEST}"), STATE_ROOT),
                state_row("path:tpm-state", STATE_ROOT),
            ],
            restart_policies: Vec::new(),
            degraded_states: Vec::new(),
            remediations: Vec::new(),
        });
        resolver
    }

    fn zone_uid() -> ResourceUid {
        ResourceUid::parse(ZONE_UID).expect("zone uid")
    }

    fn device_ref(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("Device/{name}")).expect("device ref")
    }

    /// The scope `device_worker::resolve_launch_scope` pins to the fixture's
    /// `Process/swtpm-tpm0` row: the `Device` that owns it, that Device's own
    /// durable uid, and the Guest that Device declares. It is the only Device
    /// identity this module sees - a launch's own claim about it is re-pinned
    /// against the bundle before it gets here, and refused if it names any
    /// other Device or any other Guest.
    fn pinned_scope() -> DeviceWorkerScope {
        DeviceWorkerScope {
            zone_uid: zone_uid(),
            device_ref: device_ref("tpm0"),
            device_uid: ResourceUid::parse(TPM0_UID).expect("device uid"),
            guest: GUEST.to_owned(),
        }
    }

    /// The identity the verified bundle derives for the pinned `Device/tpm0`:
    /// the state Volume directory named from that Device's own durable uid,
    /// never from anything the launch carries.
    #[test]
    fn the_granted_leaf_is_derived_from_the_pinned_device() {
        let identity = resource_backed_identity(&resolver(), &pinned_scope())
            .expect("the verified rows resolve a state directory");
        assert_eq!(identity.state_volume, TPM0_VOLUME);
        assert_eq!(
            trusted_state_dir(&identity),
            PathBuf::from(STATE_ROOT).join(TPM0_VOLUME),
            "the grant names this Device's own Volume directory under the trusted root"
        );
    }

    /// A Guest the verified storage contract names no TPM state row for is an
    /// unresolved trusted input, not a contradicted one: the caller grants
    /// nothing rather than refusing a launch on a gap in the artifacts.
    #[test]
    fn a_guest_no_storage_row_names_derives_nothing() {
        let scope = DeviceWorkerScope {
            guest: "other-guest".to_owned(),
            ..pinned_scope()
        };
        assert_eq!(resource_backed_identity(&resolver(), &scope), None);
    }

    fn resource_backed_plan(state_dir: &Path, runtime_dir: &Path) -> SpawnRunnerPlan {
        let state_dir = state_dir.display().to_string();
        let runtime_dir = runtime_dir.display().to_string();
        SpawnRunnerPlan {
            binary_path: PathBuf::from("/run/current-system/sw/bin/swtpm"),
            argv: vec![
                "swtpm".into(),
                "socket".into(),
                "--tpm2".into(),
                "--tpmstate".into(),
                format!("dir={state_dir}"),
                "--ctrl".into(),
                format!("type=unixio,path={state_dir}/ctrl.sock,mode=0660"),
                "--server".into(),
                format!("type=unixio,path={runtime_dir}/tpm.sock,mode=0660"),
                "--log".into(),
                format!("file={state_dir}/swtpm.log,level=20"),
                "--pid".into(),
                format!("file={state_dir}/swtpm.pid"),
            ],
            uid: 60_100,
            gid: 60_100,
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
                writable_paths: vec![],
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_placement: CgroupPlacement {
                subtree: format!("d2b.slice/process-{}/swtpm", "a".repeat(64)),
                controllers: vec![],
                delegated: true,
            },
            user_namespace: None,
            umask: None,
        presentation: crate::ops::spawn_runner::PresentationRealization::NamespaceFirstServiceSource,
        admitted_presentation: crate::ops::spawn_runner::AdmittedPresentation {
            private_execution_root: std::path::PathBuf::new(),
            binds: Vec::new(),
        },
        }
    }

    fn identity() -> ResourceBackedSwtpm {
        ResourceBackedSwtpm {
            guest: GUEST.to_owned(),
            state_root: PathBuf::from(STATE_ROOT),
            state_volume: TPM0_VOLUME.to_owned(),
        }
    }

    /// The launch the Provider composes names only the trusted state
    /// directory, its own files, and the trusted per-Guest runtime
    /// directory's socket - every path the traverse grant covers.
    #[test]
    fn the_composed_launch_arguments_name_only_the_trusted_paths() {
        let state_dir = trusted_state_dir(&identity());
        let runtime_dir = PathBuf::from("/run/d2b/vms").join(GUEST);
        verify_argv_names_only_trusted_paths(
            &resource_backed_plan(&state_dir, &runtime_dir),
            &identity(),
            &runtime_dir,
        )
        .expect("the launch the Provider composes is admitted");
    }

    /// The worker opens its state by pathname, so an argument that names
    /// anywhere else would aim the launch at a directory the grant does not
    /// cover - a sibling Device's Volume, or a path of the payload's own
    /// choosing. Every one of those is refused, whichever flag carries it.
    #[test]
    fn a_launch_argument_naming_a_non_trusted_directory_is_refused() {
        let state_dir = trusted_state_dir(&identity());
        let runtime_dir = PathBuf::from("/run/d2b/vms").join(GUEST);
        let sibling_volume =
            PathBuf::from(STATE_ROOT).join("device-00000000000000000000000000000000-tpm-state");
        for (index, argument) in [
            // `--tpmstate` aimed at a sibling Device's state Volume.
            format!("dir={}", sibling_volume.display()),
            // A bare path argument (the one-shot control-socket spelling).
            sibling_volume.join("ctrl.sock").display().to_string(),
            // The server socket outside the trusted runtime directory.
            "/run/d2b/vms/other-guest/tpm.sock".to_owned(),
            // A path the payload chose outright.
            "/var/lib/d2b/other/ctrl.sock".to_owned(),
        ]
        .into_iter()
        .enumerate()
        {
            let mut plan = resource_backed_plan(&state_dir, &runtime_dir);
            plan.argv[4] = argument.clone();
            assert_eq!(
                verify_argv_names_only_trusted_paths(&plan, &identity(), &runtime_dir),
                Err(TrustedPathMismatch::ArgvOutsideTrustedPaths),
                "argument {index} ({argument}) must be refused"
            );
        }
    }

    /// Containment is a textual component comparison, so a candidate that
    /// merely *spells* itself as a prefix of a trusted root - by walking out
    /// of it - must be refused, not admitted. Each shape below is one
    /// argument the previous fence admitted, and each is asserted on its own.
    #[test]
    fn an_unanchored_or_out_of_root_argument_is_refused() {
        let state_dir = trusted_state_dir(&identity());
        let runtime_dir = PathBuf::from("/run/d2b/vms").join(GUEST);
        let state = state_dir.display().to_string();
        let runtime = runtime_dir.display().to_string();
        let sibling_volume = "device-00000000000000000000000000000000-tpm-state";
        let fence = |index: usize, argument: &str| {
            let mut plan = resource_backed_plan(&state_dir, &runtime_dir);
            plan.argv[index] = argument.to_owned();
            verify_argv_names_only_trusted_paths(&plan, &identity(), &runtime_dir)
        };
        // `--tpmstate dir=` walking out of the trusted state directory.
        assert_eq!(
            fence(4, &format!("dir={state}/../../etc/shadow")),
            Err(TrustedPathMismatch::ArgvUnanchoredPath)
        );
        // The same walk landing on a sibling Device's state Volume.
        assert_eq!(
            fence(4, &format!("dir={state}/../{STATE_ROOT}/{sibling_volume}")),
            Err(TrustedPathMismatch::ArgvUnanchoredPath)
        );
        // `--server path=` walking out of the trusted runtime directory.
        assert_eq!(
            fence(
                8,
                &format!("type=unixio,path={runtime}/../../../etc/cron.d/x,mode=0777")
            ),
            Err(TrustedPathMismatch::ArgvUnanchoredPath)
        );
        // A relative value in a path-shaped field: the worker resolves it
        // against its own cwd, which the fence never derives.
        assert_eq!(
            fence(8, "type=unixio,path=../../etc/shadow,mode=0660"),
            Err(TrustedPathMismatch::ArgvUnanchoredPath)
        );
        // The same, in the other path-shaped field.
        assert_eq!(
            fence(4, "dir=../../../tmp/evil"),
            Err(TrustedPathMismatch::ArgvUnanchoredPath)
        );
        // A whole argument replaced by a shell command: every path a command
        // line names is a path the launch can open, and this one names two
        // the grant never covers.
        let mut shell = resource_backed_plan(&state_dir, &runtime_dir);
        shell.argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "cat /etc/shadow > /tmp/x".to_owned(),
        ];
        assert_eq!(
            verify_argv_names_only_trusted_paths(&shell, &identity(), &runtime_dir),
            Err(TrustedPathMismatch::ArgvUnanchoredPath)
        );
    }

    /// A relative value is refused, not discarded: the old fence filtered
    /// every non-absolute value out of its comparison, so a path-shaped field
    /// that named one was read as though it named no path at all.
    #[test]
    fn a_relative_value_in_a_path_shaped_field_is_refused_not_discarded() {
        let state_dir = trusted_state_dir(&identity());
        let runtime_dir = PathBuf::from("/run/d2b/vms").join(GUEST);
        for (argument, index) in [
            ("path=../../etc/shadow", 8),
            ("dir=../../../tmp/evil", 4),
            ("file=swtpm.pid", 12),
            ("mountPath=vms", 4),
            // A path under a field name that is not path-shaped is still a
            // path, and is still anchored-checked.
            ("type=../../etc/shadow", 8),
            // A bare relative argument, with no field name at all.
            ("../ctrl.sock", 4),
        ] {
            let mut plan = resource_backed_plan(&state_dir, &runtime_dir);
            plan.argv[index] = argument.to_owned();
            assert_eq!(
                verify_argv_names_only_trusted_paths(&plan, &identity(), &runtime_dir),
                Err(TrustedPathMismatch::ArgvUnanchoredPath),
                "argument {index} ({argument}) must be refused"
            );
        }
    }

    /// The rule reads the paths the arguments carry, not the flags: an
    /// argument that names nothing absolute is never a target, so the
    /// launch vocabulary stays free to add flags.
    #[test]
    fn an_argument_that_names_no_path_is_never_a_target() {
        let state_dir = trusted_state_dir(&identity());
        let runtime_dir = PathBuf::from("/run/d2b/vms").join(GUEST);
        let mut plan = resource_backed_plan(&state_dir, &runtime_dir);
        plan.argv.extend([
            "socket".to_owned(),
            "--flags".to_owned(),
            "startup-clear".to_owned(),
            "--level".to_owned(),
            "20".to_owned(),
        ]);
        verify_argv_names_only_trusted_paths(&plan, &identity(), &runtime_dir)
            .expect("a valueless flag names no path");
    }
}
