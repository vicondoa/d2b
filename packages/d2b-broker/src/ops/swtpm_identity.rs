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
//! grant is applied to, and
//! [`verify_argv_names_only_trusted_paths`] fences the launch's arguments
//! against it: because the worker opens the directory by pathname, a
//! launch that names anywhere else is refused rather than trusted with a
//! grant for a directory it does not name.
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
    /// The state Volume name the worker opens under that root
    /// (`device-<32hex>-tpm-state`, the TPM Provider's own naming from the
    /// owning Device's durable uid, which the verified Zone bundle's Device
    /// row derives). It is never absent: an identity with no Device-scoped
    /// leaf would name the *shared* state root, which no launch may be
    /// granted.
    pub state_volume: String,
}

/// Why one resource-backed launch's trusted identity could not be pinned to
/// the verified bundle, or why its launch arguments do not name the trusted
/// paths.
///
/// A closed set of path-free slugs: the launch-failure envelope and the
/// `kind="critical"` audit record render the slug, never a caller-supplied
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedPathMismatch {
    /// The launch asserts a Device uid that is not the durable uid the
    /// verified Zone bundle derives for the Device it names.
    DeviceUid { claimed: String, resolved: String },
    /// The launch arguments name a path that is neither the trusted state
    /// directory (or anything under it) nor the trusted per-Guest runtime
    /// directory (or anything under it).
    ArgvOutsideTrustedPaths,
}

impl std::fmt::Display for TrustedPathMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceUid { .. } => f.write_str("device-worker-scope-uid-mismatch"),
            Self::ArgvOutsideTrustedPaths => {
                f.write_str("device-worker-argv-outside-trusted-paths")
            }
        }
    }
}

/// The trusted identity of a resource-backed (typed) launch, or the reason it
/// is not one.
///
/// `Ok(None)` is a trusted input the bundle does not resolve (no Zone bundle,
/// no storage row, no Device row), which the caller treats as "grants
/// nothing". `Err` is a launch that *contradicts* the verified bundle, which
/// the caller refuses: it is never downgraded to a grant-nothing path.
pub type TrustedIdentity = Result<Option<ResourceBackedSwtpm>, TrustedPathMismatch>;

/// Resolve the trusted identity of a resource-backed `w1-swtpm` launch from
/// the verified bundle.
///
/// The granted leaf is derived from the bundle alone: the Zone resource
/// bundle names the Device's own Guest, and the state Volume directory is
/// named from the durable uid the framework derives for
/// `(zone, "Device", <name>)` - never from the uid the launch carries.
/// `asserted_device_uid` is a cross-check only: a launch that asserts
/// another Device's uid is refused, because a scope whose own claim
/// contradicts the bundle it was resolved against is not a scope to derive
/// anything from.
pub fn resource_backed_identity(
    resolver: &BundleResolver,
    zone_uid: &ResourceUid,
    device_ref: &ResourceRef,
    asserted_device_uid: &ResourceUid,
) -> TrustedIdentity {
    if device_ref.resource_type().as_str() != "Device" {
        return Ok(None);
    }
    let (zone, bundle_bytes) = match crate::ops::device_worker::zone_bundle_for_uid(resolver, zone_uid)
    {
        Some(resolved) => resolved,
        None => return Ok(None),
    };
    let owning_uid =
        crate::ops::device_worker::deterministic_resource_uid(&zone, "Device", device_ref.name().as_str());
    if asserted_device_uid != &owning_uid {
        return Err(TrustedPathMismatch::DeviceUid {
            claimed: asserted_device_uid.to_canonical_string(),
            resolved: owning_uid.to_canonical_string(),
        });
    }
    let Some(guest) =
        crate::ops::device_worker::device_guest_owner(bundle_bytes, device_ref.name().as_str())
    else {
        return Ok(None);
    };
    let Some(state_root) = storage_root(resolver, &format!("path:swtpm-state:{guest}")) else {
        return Ok(None);
    };
    // The Provider's opaque `sourcePolicyId: "tpm-state"` resolves through the
    // storage contract's `path:tpm-state` row. If the two rows disagree, the
    // directory the worker opens is not the directory the Volume controller
    // provisions, so no trusted identity is derived.
    if storage_root(resolver, "path:tpm-state") != Some(state_root.clone()) {
        return Ok(None);
    }
    Ok(Some(ResourceBackedSwtpm {
        guest,
        state_root,
        state_volume: state_volume_name(&owning_uid),
    }))
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
    identity.state_root.join(&identity.state_volume)
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
/// `argv[0]` is the launch's own executable name, not a path the launch
/// opens, so it is not a target.
pub fn verify_argv_names_only_trusted_paths(
    plan: &SpawnRunnerPlan,
    identity: &ResourceBackedSwtpm,
    runtime_dir: &Path,
) -> Result<(), TrustedPathMismatch> {
    let state_dir = trusted_state_dir(identity);
    for argument in plan.argv.iter().skip(1) {
        for named in named_absolute_paths(argument) {
            if !is_within(&named, &state_dir) && !is_within(&named, runtime_dir) {
                return Err(TrustedPathMismatch::ArgvOutsideTrustedPaths);
            }
        }
    }
    Ok(())
}

/// Every absolute path one argument names: the argument itself when it is a
/// path, plus the value of each `key=path` field of a comma-separated option.
fn named_absolute_paths(argument: &str) -> impl Iterator<Item = PathBuf> + '_ {
    argument
        .split(',')
        .map(|field| field.rsplit_once('=').map_or(field, |(_, value)| value))
        .filter(|value| Path::new(value).is_absolute())
        .map(PathBuf::from)
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
    use d2b_contracts::contract_id::{ContractId, PathTemplate};
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
            privileges_path: "privileges.json".to_owned(),
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

    /// The identity the verified bundle derives for `Device/tpm0`: the
    /// state Volume directory named from the Device's own durable uid, never
    /// from anything the launch carries.
    #[test]
    fn the_granted_leaf_is_derived_from_the_bundles_device() {
        let identity = resource_backed_identity(
            &resolver(),
            &zone_uid(),
            &device_ref("tpm0"),
            &ResourceUid::parse(TPM0_UID).expect("device uid"),
        )
        .expect("the asserted uid agrees with the bundle")
        .expect("the verified rows resolve a state directory");
        assert_eq!(identity.state_volume, TPM0_VOLUME);
        assert_eq!(
            trusted_state_dir(&identity),
            PathBuf::from(STATE_ROOT).join(TPM0_VOLUME),
            "the grant names this Device's own Volume directory under the trusted root"
        );
    }

    /// A launch that asserts ANOTHER Device's uid is refused outright: the
    /// grant it would otherwise receive is a sibling Device's TPM state
    /// directory under the same shared root, so a payload that can name the
    /// uid can reach that directory. The uid is a cross-check, never the
    /// source of the granted path.
    #[test]
    fn a_launch_asserting_another_devices_uid_is_refused() {
        let foreign =
            crate::ops::device_worker::deterministic_resource_uid("work", "Device", "gpu0");
        let error = resource_backed_identity(&resolver(), &zone_uid(), &device_ref("tpm0"), &foreign)
            .expect_err("a foreign Device uid must be refused, not downgraded");
        assert_eq!(
            error,
            TrustedPathMismatch::DeviceUid {
                claimed: foreign.to_canonical_string(),
                resolved: TPM0_UID.to_owned(),
            }
        );
        assert_eq!(
            error.to_string(),
            "device-worker-scope-uid-mismatch",
            "the refusal is a typed, path-free slug"
        );

        // The same launch with its own uid still resolves: the refusal is the
        // disagreement, not the shape of the claim.
        assert!(
            resource_backed_identity(
                &resolver(),
                &zone_uid(),
                &device_ref("tpm0"),
                &ResourceUid::parse(TPM0_UID).expect("device uid"),
            )
            .expect("the matching uid resolves")
            .is_some()
        );
    }

    /// A row no verified artifact names is an unresolved trusted input, not
    /// a contradicted one: the caller grants nothing rather than refusing a
    /// launch on a gap in the artifacts.
    #[test]
    fn a_row_no_verified_artifact_names_derives_nothing() {
        assert_eq!(
            resource_backed_identity(
                &resolver(),
                &zone_uid(),
                &ResourceRef::parse("Process/swtpm-tpm0").expect("not a device"),
                &ResourceUid::parse(TPM0_UID).expect("device uid"),
            ),
            Ok(None)
        );
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
