//! `PrepareStateDir` + `PrepareRuntimeDir`.
//!
//! Fd-based `fchown`/`fchmod` analogue. Path safety same as `hosts.rs`.
//! Audit fields: `base_dir_hash`, `vm_id_or_scope`,
//! `created_paths_hash`, `mode`, `owner_uid`, `owner_gid`,
//! `replace_or_create_result`.

use crate::ops::exec_reconcile::SystemLiveExec;
use crate::ops::hosts::stable_hash_str;
use crate::sys::path_safe::{DirCreateResult, ensure_dir, ensure_dir_preserve_existing};
use std::io;
use std::path::{Path, PathBuf};

use d2b_contracts::types::PathClass;
use d2b_core::bundle_resolver::BundleResolver;

/// Failure from the generic state-directory operation.
///
/// swtpm hardening carries its path-free terminal audit record separately so
/// the runtime can preserve the typed `PrepareSwtpmDir` disposition rather
/// than reducing it to a generic live-handler error.
#[derive(Debug)]
pub enum PrepareStateDirError {
    Operation(super::OpError),
    SwtpmDirHardening(crate::ops::swtpm_dir::SwtpmHardenError),
}

impl From<super::OpError> for PrepareStateDirError {
    fn from(error: super::OpError) -> Self {
        Self::Operation(error)
    }
}

impl std::fmt::Display for PrepareStateDirError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(error) => error.fmt(formatter),
            Self::SwtpmDirHardening(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PrepareStateDirError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirKind {
    StateDir,
    RuntimeDir,
}

#[derive(Debug, Clone)]
pub struct PrepareDirRequest {
    pub kind: DirKind,
    pub base_dir: PathBuf,
    /// Per-VM or global scope (`global` if `vm_id` is `None`).
    pub vm_id_or_scope: String,
    /// 0o-mode (e.g. 0o750 for state, 0o755 for runtime).
    pub mode: u32,
    pub owner_uid: u32,
    pub owner_gid: u32,
    /// Directories to create under `base_dir` (relative paths).
    pub created_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PrepareDirAudit {
    pub kind: DirKind,
    pub base_dir_hash: String,
    pub vm_id_or_scope: String,
    pub created_paths_hash: String,
    pub mode: u32,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub replace_or_create_result: ReplaceOrCreateResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReplaceOrCreateResult {
    Created,
    Reused,
    MixedCreatedAndReused,
}

pub fn prepare_dir(req: &PrepareDirRequest) -> io::Result<PrepareDirAudit> {
    // Refuse non-root parent for production paths. Tests pass a scratch
    // base_dir so the refuse_non_root_parent guard is wired via the
    // `enforce_root_parent` knob below.
    if production_path(&req.base_dir) {
        crate::sys::path_safe::refuse_non_root_parent(&req.base_dir)?;
    }
    // The per-VM root base dir is created + owned by host activation
    // (`nixos-modules/host-ssh-host-keys.nix`: `install -d -m 2770 -o
    // d2bd -g users`) and carries per-runner POSIX ACLs. Preserve
    // that posture on an existing dir instead of re-stamping it to a
    // single runner principal (which clipped the ACL mask + tripped the
    // ownership-matrix preflight). Created subdirs below still get the
    // requested metadata.
    let base_audit = ensure_dir_preserve_existing(
        &req.base_dir,
        req.mode,
        Some(req.owner_uid),
        Some(req.owner_gid),
    )?;
    let mut any_created = matches!(base_audit, DirCreateResult::Created);
    let mut any_reused = matches!(base_audit, DirCreateResult::Reused);
    let mut paths_concat = String::new();
    for rel in &req.created_paths {
        if rel.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "path-safety-violation: created path must be relative: {}",
                    rel.display()
                ),
            ));
        }
        for component in rel.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "path-safety-violation: created path contains ..: {}",
                        rel.display()
                    ),
                ));
            }
        }
        let full = req.base_dir.join(rel);
        let r = ensure_dir(&full, req.mode, Some(req.owner_uid), Some(req.owner_gid))?;
        any_created |= matches!(r, DirCreateResult::Created);
        any_reused |= matches!(r, DirCreateResult::Reused);
        paths_concat.push_str(&full.display().to_string());
        paths_concat.push('\n');
    }
    let result = match (any_created, any_reused) {
        (true, false) => ReplaceOrCreateResult::Created,
        (false, true) => ReplaceOrCreateResult::Reused,
        _ => ReplaceOrCreateResult::MixedCreatedAndReused,
    };
    Ok(PrepareDirAudit {
        kind: req.kind,
        base_dir_hash: stable_hash_str(&req.base_dir.display().to_string()),
        vm_id_or_scope: req.vm_id_or_scope.clone(),
        created_paths_hash: stable_hash_str(&paths_concat),
        mode: req.mode,
        owner_uid: req.owner_uid,
        owner_gid: req.owner_gid,
        replace_or_create_result: result,
    })
}

fn production_path(p: &Path) -> bool {
    p.starts_with("/var/lib/d2b") || p.starts_with("/run/d2b")
}

pub fn live_prepare_runtime_dir(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts_broker::broker_wire::PrepareDirRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<(), super::OpError> {
    if req.path_class != PathClass::Runtime {
        return Err(super::OpError::InvalidInput {
            detail: format!(
                "PrepareRuntimeDir requires pathClass=runtime, got {:?}",
                req.path_class
            ),
        });
    }
    let intent = resolver
        .resolve_prepare_dir_intent(req.vm_id.as_str(), true)
        .ok_or_else(|| super::OpError::UnknownSubject {
            operation: "PrepareRuntimeDir",
            subject: req.vm_id.as_str().to_owned(),
        })?;
    ensure_dir_preserve_existing(
        &intent.base_dir,
        intent.mode,
        Some(intent.owner_uid),
        Some(intent.owner_gid),
    )
    .map_err(|e| super::OpError::Io {
        path: intent.base_dir.clone(),
        detail: e.to_string(),
    })?;
    Ok(())
}

/// The state directory one successful `PrepareStateDir` resolved, and the
/// posture recorded for it.
///
/// A legacy VM's directory is created/verified by this op, so the posture is
/// the manifest intent's. A v3 zone-native Guest's directory is owned by the
/// controller-created state Volume, which the trusted
/// `path:swtpm-state:<guest>` storage row roots, so this op resolves that row
/// and records the posture it declares - it creates nothing. Both postures
/// come from a verified bundle artifact, never from the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStateDir {
    pub base_dir: PathBuf,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub mode: u32,
}

pub fn live_prepare_state_dir(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts_broker::broker_wire::PrepareDirRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<PreparedStateDir, PrepareStateDirError> {
    if req.path_class != PathClass::Vm {
        return Err(super::OpError::InvalidInput {
            detail: format!(
                "PrepareStateDir requires pathClass=vm, got {:?}",
                req.path_class
            ),
        }
        .into());
    }
    let Some(intent) = resolver.resolve_prepare_dir_intent(req.vm_id.as_str(), false) else {
        // v3: a zone-native Guest carries no legacy state-directory intent.
        // Its TPM state directory is the controller-created state Volume the
        // trusted `path:swtpm-state:<guest>` storage row roots
        // (`createPolicy: create-if-never-provisioned`,
        // `repairPolicy: fail-closed`,
        // `packages/d2b-provider-device-tpm/src/resources.rs`), so there is no
        // legacy directory for this op to prepare. Accept exactly the subject
        // such a trusted row names - the same row the daemon's worker
        // derivation, the spawn-time swtpm-dir fence and the volume-local
        // controller's root all agree on - and keep every other unknown
        // subject failing closed.
        let (spec, state_root) =
            crate::ops::swtpm_dir::zone_native_swtpm_state_row(resolver, req.vm_id.as_str())
                .ok_or_else(|| super::OpError::UnknownSubject {
                    operation: "PrepareStateDir",
                    subject: req.vm_id.as_str().to_owned(),
                })?;
        let (owner_uid, owner_gid, mode) = crate::ops::storage_contract::row_posture(spec)
            .ok_or_else(|| super::OpError::Refused {
                operation: "PrepareStateDir",
                reason: "trusted-state-row-posture-unresolvable".to_owned(),
            })?;
        // The directory the worker actually opens: the state Volume of the
        // Guest's own TPM Device, which the TPM Provider names from that
        // Device's durable uid (`<root>/device-<32hex>-tpm-state`) - the same
        // name `d2bd`'s `device_state_dir` composes and the volume-local
        // controller provisions. The op is per-Guest, so it can name that
        // directory only when the verified bundles name exactly one TPM Device
        // for the Guest; a Device committed through the Resource API lives in
        // no bundle, and a Guest declaring several TPM Devices owns several
        // state Volumes, in which case the shared policy root the trusted row
        // itself names is recorded - never an invented one of them.
        let base_dir = crate::ops::device_worker::unique_tpm_state_dir(
            &crate::ops::device_worker::tpm_devices_of_guest(resolver, req.vm_id.as_str()),
            &state_root,
        )
        .unwrap_or(state_root);
        return Ok(PreparedStateDir {
            base_dir,
            owner_uid,
            owner_gid,
            mode,
        });
    };
    ensure_dir_preserve_existing(
        &intent.base_dir,
        intent.mode,
        Some(intent.owner_uid),
        Some(intent.owner_gid),
    )
    .map_err(|e| super::OpError::Io {
        path: intent.base_dir.clone(),
        detail: e.to_string(),
    })?;

    // Device TPM state must be identity-bound before any pre-start flush is
    // allowed to run.  The generic state-directory operation is the first
    // typed effect in the TPM lifecycle, so perform the broker-owned swtpm
    // hardening here rather than relying on the later SpawnRunner hook.
    if let Some(legacy) = resolver.resolve_legacy_swtpm_intent(req.vm_id.as_str()) {
        let marker_dir = legacy
            .marker
            .parent()
            .ok_or_else(|| super::OpError::Refused {
                operation: "PrepareStateDir",
                reason: crate::ops::swtpm_dir::reasons::DERIVATION_FAILED.to_owned(),
            })?;
        let marker_name = legacy
            .marker
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| super::OpError::Refused {
                operation: "PrepareStateDir",
                reason: crate::ops::swtpm_dir::reasons::DERIVATION_FAILED.to_owned(),
            })?
            .to_owned();
        let swtpm_dir = legacy.destination;
        let per_vm_root =
            swtpm_dir
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| super::OpError::Refused {
                    operation: "PrepareStateDir",
                    reason: crate::ops::swtpm_dir::reasons::DERIVATION_FAILED.to_owned(),
                })?;
        let paths = crate::ops::swtpm_dir::SwtpmDirPaths {
            vm_id: legacy.vm,
            swtpm_dir,
            per_vm_root,
            runtime_dir: PathBuf::from(format!("/run/d2b/vms/{}", req.vm_id.as_str())),
            marker_dir: marker_dir.to_path_buf(),
            marker_name,
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let config = crate::ops::swtpm_dir::SwtpmHardenConfig {
            expected_uid: legacy.owner_uid,
            expected_gid: legacy.owner_gid,
            marker_owner_uid: 0,
            marker_owner_gid: 0,
            now_ms,
            enforce_root_parents: paths.swtpm_dir.starts_with("/var/lib/d2b"),
        };
        crate::ops::swtpm_dir::harden(&paths, &config)
            .map_err(PrepareStateDirError::SwtpmDirHardening)?;
    }

    Ok(PreparedStateDir {
        base_dir: intent.base_dir,
        owner_uid: intent.owner_uid,
        owner_gid: intent.owner_gid,
        mode: intent.mode,
    })
}

/// The trusted `path:swtpm-state:<guest>` storage row a zone-native Guest's
/// TPM state root comes from, as one resolver - the same artifact the
/// spawn-time swtpm-dir fence resolves (`swtpm_dir::resource_backed_identity`
/// reads `path:swtpm-state:<guest>` and `path:tpm-state` through the same
/// `find_storage_path_spec`).
///
/// The row declares `0:0 0700` numerically: a named principal would only
/// resolve on a host that has that account, and these tests assert the
/// posture the row itself declares.
#[cfg(test)]
/// The canonical content hash of one fixture resource array, computed the way
/// `ResourceBundle` computes it, so the fixture bundle verifies.
fn fixture_content_hash(resources: &[serde_json::Value]) -> String {
    use d2b_contracts_resource::v3::resource_schema::{
        CanonicalJsonValue, canonical_json_bytes, framed_canonical_digest,
    };
    let array = serde_json::Value::Array(resources.to_vec());
    let canonical =
        CanonicalJsonValue::parse(&serde_json::to_vec(&array).expect("fixture resources serialize"))
            .expect("fixture resources are canonical JSON");
    framed_canonical_digest(
        "d2b:v3:resource-bundle",
        &canonical_json_bytes(&canonical).expect("fixture resources encode"),
    )
}

#[cfg(test)]
pub(crate) fn resolver_with_swtpm_state_row(guest: &str) -> BundleResolver {
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::contract_id::{ContractId, PathTemplate};
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;
    use d2b_core::storage::{
        ActorKind, ActorRef, CleanupPolicy, LeaseClass, PrincipalKind, PrincipalRef,
        RepairPolicy, SensitivityClass, StorageAdoptionPolicy, StorageInvariant, StorageJson,
        StorageLifecycle, StoragePathKind, StoragePathSpec, StoragePersistence,
        StorageRestartPolicy,
    };

    let principal = |kind, value: &str| PrincipalRef {
        kind,
        value: ContractId::parse(value).unwrap(),
    };
    let actor = |kind, value: &str| ActorRef {
        kind,
        value: ContractId::parse(value).unwrap(),
    };
    let storage = StorageJson {
        schema_version: "v2".to_owned(),
        roots: Vec::new(),
        paths: vec![StoragePathSpec {
            id: ContractId::parse(&format!("path:swtpm-state:{guest}")).unwrap(),
            scope: ContractId::parse(&format!("vm:{guest}")).unwrap(),
            path_template: PathTemplate::parse("/var/lib/d2b/tpm-state").unwrap(),
            kind: StoragePathKind::Directory,
            lifecycle: StorageLifecycle::Persistent,
            persistence: StoragePersistence::Persistent,
            owner: principal(PrincipalKind::Uid, "0"),
            group: principal(PrincipalKind::Gid, "0"),
            mode: "0700".to_owned(),
            access_acl: Vec::new(),
            default_acl: Vec::new(),
            creator: actor(ActorKind::NixModule, "tmpfiles"),
            writers: vec![actor(ActorKind::Daemon, "d2bd")],
            readers: vec![actor(ActorKind::Daemon, "d2bd")],
            cleanup_policy: CleanupPolicy::Never,
            repair_policy: RepairPolicy::BrokerFailClosed,
            restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
            adoption_policy: StorageAdoptionPolicy::QuarantineOnAmbiguity,
            lease_class: LeaseClass::None,
            sensitivity: SensitivityClass::SecretAdjacent,
            no_follow: true,
            recursive: false,
            invariants: vec![StorageInvariant::NoSymlink],
        }],
        restart_policies: Vec::new(),
        degraded_states: Vec::new(),
        remediations: Vec::new(),
    };
    let bundle = Bundle {
        bundle_version: 11,
        schema_version: "v2".to_owned(),
        public_manifest_path: "vms.json".to_owned(),
        host_path: "host.json".to_owned(),
        processes_path: "processes.json".to_owned(),
        privileges_path: "privileges.json".to_owned(),
        storage_path: Some("storage.json".to_owned()),
        sync_path: None,
        allocator_path: None,
        realm_controllers_path: None,
        realm_identity_path: None,
        realm_workloads_launcher_v2_path: None,
        unsafe_local_workloads_path: None,
        closures: Vec::new(),
        minijail_profiles: Vec::new(),
        managed_keys: Default::default(),
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
    // The Zone resource bundle names the Device that owns the Guest's TPM
    // function, which is what the `PrepareStateDir` record resolves the
    // worker's state Volume directory from.
    let resources = vec![serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Device",
        "metadata": {
            "name": "tpm0",
            "zone": "work",
            "ownerRef": format!("Guest/{guest}"),
        },
        "spec": {"providerRef": "Provider/device-tpm"},
    })];
    let zone_bundle = serde_json::json!({
        "schemaVersion": 3,
        "bundleVersion": 1,
        "zone": "work",
        "zoneUid": "123e4567-e89b-42d3-a456-426614174000",
        "contentHash": fixture_content_hash(&resources),
        "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
        "schemaFingerprints": {},
        "providerSchemaDigests": {},
        "resources": resources,
        "generatedAt": "1970-01-01T00:00:00.000Z",
    });
    let mut resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
        bundle,
        host,
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        },
        manifest,
        std::collections::BTreeMap::from([(
            "work".to_owned(),
            serde_json::to_vec(&zone_bundle).expect("zone bundle bytes"),
        )]),
    );
    // Storage travels on the same resolver: the production loader carries both
    // artifacts, and this fixture needs the trusted row and the Device scope
    // to resolve together.
    resolver.storage = Some(storage);
    resolver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "d2b-w3-s2-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    #[test]
    fn creates_base_and_relative_paths() {
        let dir = scratch();
        let base = dir.join("state");
        let req = PrepareDirRequest {
            kind: DirKind::StateDir,
            base_dir: base.clone(),
            vm_id_or_scope: "vm-a".into(),
            mode: 0o750,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            created_paths: vec![PathBuf::from("logs"), PathBuf::from("artifacts")],
        };
        let audit = prepare_dir(&req).unwrap();
        assert!(base.is_dir());
        assert!(base.join("logs").is_dir());
        assert!(base.join("artifacts").is_dir());
        assert_eq!(audit.vm_id_or_scope, "vm-a");
        assert_eq!(audit.mode, 0o750);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn idempotent_reuses_existing_dirs() {
        let dir = scratch();
        let base = dir.join("state");
        let req = PrepareDirRequest {
            kind: DirKind::StateDir,
            base_dir: base.clone(),
            vm_id_or_scope: "vm-a".into(),
            mode: 0o750,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            created_paths: vec![PathBuf::from("logs")],
        };
        let first = prepare_dir(&req).unwrap();
        assert_eq!(
            first.replace_or_create_result,
            ReplaceOrCreateResult::Created
        );
        let second = prepare_dir(&req).unwrap();
        assert_eq!(
            second.replace_or_create_result,
            ReplaceOrCreateResult::Reused
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn preserves_existing_base_dir_mode_instead_of_restamping() {
        // Regression: vm-start's per-VM root prepare must NOT re-stamp
        // mode/ownership on an EXISTING dir. Host activation creates the
        // per-VM root as `d2bd:users 2770` with per-runner POSIX
        // ACLs; re-`fchmod`-ing it to the prepare's mode clipped the ACL
        // mask to the group bits (so virtiofsd/gpu/video lost write
        // access to their per-VM runtime dir). Owner preservation needs
        // root to assert (chown), so this checks the MODE axis, which
        // exercises the same `reassert_metadata = false` reuse branch.
        let dir = scratch();
        let base = dir.join("state");
        // Pre-create the base dir with the activation-shaped 2770 mode.
        fs::create_dir_all(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o2770)).unwrap();
        let req = PrepareDirRequest {
            kind: DirKind::StateDir,
            base_dir: base.clone(),
            vm_id_or_scope: "vm-a".into(),
            // The prepare asks for 0o750 - the mask-clipping value the
            // regression came from. It MUST be ignored for the existing
            // dir.
            mode: 0o750,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            created_paths: vec![],
        };
        let audit = prepare_dir(&req).unwrap();
        assert_eq!(
            audit.replace_or_create_result,
            ReplaceOrCreateResult::Reused
        );
        let got = fs::metadata(&base).unwrap().permissions().mode() & 0o7777;
        assert_eq!(
            got, 0o2770,
            "existing base dir mode must be preserved, not restamped to 0o750 (got {got:o})"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fresh_base_dir_still_receives_requested_mode() {
        // The preserve-existing behavior must only apply to EXISTING
        // dirs; a freshly created base dir still gets the requested mode.
        let dir = scratch();
        let base = dir.join("state");
        let req = PrepareDirRequest {
            kind: DirKind::StateDir,
            base_dir: base.clone(),
            vm_id_or_scope: "vm-a".into(),
            mode: 0o2770,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            created_paths: vec![],
        };
        let audit = prepare_dir(&req).unwrap();
        assert_eq!(
            audit.replace_or_create_result,
            ReplaceOrCreateResult::Created
        );
        let got = fs::metadata(&base).unwrap().permissions().mode() & 0o7777;
        assert_eq!(got, 0o2770, "fresh base dir must get the requested mode");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_absolute_relative_path() {
        let dir = scratch();
        let base = dir.join("state");
        let req = PrepareDirRequest {
            kind: DirKind::StateDir,
            base_dir: base,
            vm_id_or_scope: "vm-a".into(),
            mode: 0o750,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            created_paths: vec![PathBuf::from("/etc/passwd")],
        };
        let err = prepare_dir(&req).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_parent_dir_escape() {
        let dir = scratch();
        let req = PrepareDirRequest {
            kind: DirKind::StateDir,
            base_dir: dir.join("state"),
            vm_id_or_scope: "vm-a".into(),
            mode: 0o750,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            created_paths: vec![PathBuf::from("../escape")],
        };
        let err = prepare_dir(&req).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(dir).ok();
    }

    fn prepare_request(vm: &str) -> d2b_contracts_broker::broker_wire::PrepareDirRequest {
        use d2b_contracts::types::VmId;
        d2b_contracts_broker::broker_wire::PrepareDirRequest {
            vm_id: VmId::new(vm),
            path_class: PathClass::Vm,
            tracing_span_id: None,
        }
    }

    #[test]
    fn prepare_state_dir_accepts_a_zone_native_guest_with_the_trusted_state_row() {
        // The v3 TPM state directory belongs to the controller-created state
        // Volume, so the legacy prepare is a no-op for a zone-native Guest -
        // and only for one a trusted `path:swtpm-state:<guest>` row names.
        // The record names the directory the worker actually opens: the state
        // Volume of the Guest's own Device under the trusted root.
        let resolver = resolver_with_swtpm_state_row("acceptance-guest");
        let (exec, audit_log) = live_fixture();
        let prepared = live_prepare_state_dir(
            &exec,
            &resolver,
            &prepare_request("acceptance-guest"),
            &audit_log,
        )
        .expect("the zone-native TPM subject prepares as a no-op");
        let device_uid = crate::ops::device_worker::deterministic_resource_uid(
            "work",
            "Device",
            "tpm0",
        );
        assert_eq!(
            prepared.base_dir,
            PathBuf::from("/var/lib/d2b/tpm-state").join(crate::ops::swtpm_dir::state_volume_name(
                &device_uid
            ))
        );
        assert_ne!(
            prepared.base_dir,
            PathBuf::from("/var/lib/d2b/tpm-state"),
            "the record must name the worker's own state Volume directory, not the shared root"
        );
        assert_eq!(prepared.owner_uid, 0);
        assert_eq!(prepared.owner_gid, 0);
        assert_eq!(prepared.mode, 0o700);
    }

    #[test]
    fn prepare_state_dir_still_refuses_an_unknown_subject() {
        let resolver = resolver_with_swtpm_state_row("other-guest");
        let (exec, audit_log) = live_fixture();
        let error = live_prepare_state_dir(
            &exec,
            &resolver,
            &prepare_request("acceptance-guest"),
            &audit_log,
        )
        .expect_err("a subject no trusted row names still fails closed");
        assert!(matches!(
            error,
            PrepareStateDirError::Operation(super::super::OpError::UnknownSubject { .. })
        ));
    }

    fn live_fixture() -> (SystemLiveExec, crate::audit::AuditLog) {
        let exec = SystemLiveExec::new(
            nix::unistd::geteuid().as_raw(),
            nix::unistd::getegid().as_raw(),
        );
        let audit_log = crate::audit::AuditLog::open(
            &crate::test_scratch_root().join(format!(
                "w3-state-dir-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            0,
            true,
            1,
        )
        .expect("test audit log opens");
        (exec, audit_log)
    }
}
