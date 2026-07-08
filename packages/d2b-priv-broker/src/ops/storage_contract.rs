//! Broker storage/sync contract handlers (ADR 0034).
//!
//! These handlers are the first broker-facing surface over the generated
//! `storage.json` and `sync.json` artifacts. They deliberately accept only
//! opaque bundle ids from the daemon and resolve every path/owner/mode from
//! the broker's trusted bundle copy.

use std::path::{Path, PathBuf};

use d2b_contracts::broker_wire::{
    ReconcileStorageScopeResponse, StorageReconcileStatus, ValidateLockSpecResponse,
};
use d2b_contracts::types::BundleOpId;
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::storage::{PrincipalKind, PrincipalRef, StoragePathKind};
use nix::unistd::{Gid, Group, Uid, User};

use super::hosts::stable_hash_str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageContractError {
    UnknownStorage(String),
    UnknownLock(String),
    Refused { subject: String, reason: String },
    Invalid { subject: String, detail: String },
    Io { path_hash: String, detail: String },
}

impl std::fmt::Display for StorageContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownStorage(id) => write!(f, "unknown storage contract id {id:?}"),
            Self::UnknownLock(id) => write!(f, "unknown lock contract id {id:?}"),
            Self::Refused { subject, reason } => write!(f, "{subject}: refused: {reason}"),
            Self::Invalid { subject, detail } => write!(f, "{subject}: invalid: {detail}"),
            Self::Io { path_hash, detail } => {
                write!(f, "I/O error on storage-path#{path_hash}: {detail}")
            }
        }
    }
}

impl std::error::Error for StorageContractError {}

pub fn reconcile_storage_scope(
    resolver: &BundleResolver,
    storage_ref: &BundleOpId,
    apply: bool,
) -> Result<ReconcileStorageScopeResponse, StorageContractError> {
    let spec = resolver
        .find_storage_path_spec(storage_ref.as_str())
        .ok_or_else(|| StorageContractError::UnknownStorage(storage_ref.as_str().to_owned()))?;
    let path = spec.path_template.as_str();
    let path_hash = stable_hash_str(path);
    if has_unexpanded_template(path) {
        if apply && path.starts_with("/etc/d2b") {
            return Err(StorageContractError::Refused {
                subject: storage_ref.as_str().to_owned(),
                reason: "storage-critical-template-unexpanded".to_owned(),
            });
        }
        return Ok(ReconcileStorageScopeResponse {
            storage_ref: storage_ref.clone(),
            scope: spec.scope.as_str().to_owned(),
            kind: format!("{:?}", spec.kind),
            status: StorageReconcileStatus::TemplateUnexpanded,
            applied: false,
            path_hash,
        });
    }
    let path_buf = PathBuf::from(path);
    if spec.kind == StoragePathKind::ExternalGrantOnly {
        return Ok(ReconcileStorageScopeResponse {
            storage_ref: storage_ref.clone(),
            scope: spec.scope.as_str().to_owned(),
            kind: format!("{:?}", spec.kind),
            status: StorageReconcileStatus::CheckedOnly,
            applied: false,
            path_hash,
        });
    }
    validate_owned_root(&path_buf, storage_ref.as_str())?;
    if apply && apply_is_check_only(&path_buf) {
        return Err(StorageContractError::Refused {
            subject: storage_ref.as_str().to_owned(),
            reason: "storage-config-root-is-nix-managed".to_owned(),
        });
    }
    match spec.kind {
        StoragePathKind::Directory => {
            if apply {
                let mode = parse_mode(&spec.mode, storage_ref.as_str())?;
                let uid = resolve_uid(&spec.owner)?;
                let gid = resolve_gid(&spec.group)?;
                let result = crate::sys::path_safe::ensure_dir(
                    &path_buf,
                    mode,
                    Some(uid.as_raw()),
                    Some(gid.as_raw()),
                )
                .map_err(|err| StorageContractError::Io {
                    path_hash: stable_hash_str(path_buf.to_string_lossy().as_ref()),
                    detail: err.to_string(),
                })?;
                let status = match result {
                    crate::sys::path_safe::DirCreateResult::Created => {
                        StorageReconcileStatus::Created
                    }
                    crate::sys::path_safe::DirCreateResult::Reused => {
                        StorageReconcileStatus::Reused
                    }
                };
                Ok(ReconcileStorageScopeResponse {
                    storage_ref: storage_ref.clone(),
                    scope: spec.scope.as_str().to_owned(),
                    kind: format!("{:?}", spec.kind),
                    status,
                    applied: true,
                    path_hash,
                })
            } else {
                let status = if path_buf.exists() {
                    StorageReconcileStatus::Clean
                } else {
                    StorageReconcileStatus::CheckedOnly
                };
                Ok(ReconcileStorageScopeResponse {
                    storage_ref: storage_ref.clone(),
                    scope: spec.scope.as_str().to_owned(),
                    kind: format!("{:?}", spec.kind),
                    status,
                    applied: false,
                    path_hash,
                })
            }
        }
        _ if apply => Err(StorageContractError::Refused {
            subject: storage_ref.as_str().to_owned(),
            reason: "storage-apply-supported-for-directory-only".to_owned(),
        }),
        _ => Ok(ReconcileStorageScopeResponse {
            storage_ref: storage_ref.clone(),
            scope: spec.scope.as_str().to_owned(),
            kind: format!("{:?}", spec.kind),
            status: StorageReconcileStatus::CheckedOnly,
            applied: false,
            path_hash,
        }),
    }
}

pub fn validate_lock_spec(
    resolver: &BundleResolver,
    lock_ref: &BundleOpId,
) -> Result<ValidateLockSpecResponse, StorageContractError> {
    let spec = resolver
        .find_sync_lock_spec(lock_ref.as_str())
        .ok_or_else(|| StorageContractError::UnknownLock(lock_ref.as_str().to_owned()))?;
    if spec.kind == d2b_core::sync::LockKind::Ofd && !spec.cloexec_required {
        return Err(StorageContractError::Invalid {
            subject: lock_ref.as_str().to_owned(),
            detail: "ofd-lock-missing-cloexec".to_owned(),
        });
    }
    if spec.fd_passing_policy.mechanism != d2b_core::sync::FdPassingMechanism::None
        && !spec.fd_passing_policy.lease_transfer_record_required
    {
        return Err(StorageContractError::Invalid {
            subject: lock_ref.as_str().to_owned(),
            detail: "fd-transfer-missing-lease-record".to_owned(),
        });
    }
    Ok(ValidateLockSpecResponse {
        lock_ref: lock_ref.clone(),
        scope: spec.scope.as_str().to_owned(),
        kind: format!("{:?}", spec.kind),
        cloexec_required: spec.cloexec_required,
        fd_passing_mechanism: format!("{:?}", spec.fd_passing_policy.mechanism),
        order_key: format!(
            "{:?}:{}:{}:{}",
            spec.acquire_order.scope_class,
            spec.acquire_order.anchored_root,
            spec.acquire_order.normalized_path,
            spec.acquire_order.lock_id
        ),
    })
}

fn has_unexpanded_template(path: &str) -> bool {
    path.contains('<') || path.contains('>') || path.contains("${")
}

fn apply_is_check_only(path: &Path) -> bool {
    path.starts_with("/etc/d2b")
}

fn validate_owned_root(path: &Path, subject: &str) -> Result<(), StorageContractError> {
    validate_owned_root_against(
        path,
        subject,
        &[
            Path::new("/etc/d2b"),
            Path::new("/var/lib/d2b"),
            Path::new("/run/d2b"),
            Path::new("/var/cache/d2b"),
        ],
    )
}

fn validate_owned_root_against(
    path: &Path,
    subject: &str,
    roots: &[&Path],
) -> Result<(), StorageContractError> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(StorageContractError::Refused {
            subject: subject.to_owned(),
            reason: "storage-path-parent-dir-refused".to_owned(),
        });
    }
    let root = roots
        .iter()
        .copied()
        .find(|root| path.starts_with(root))
        .ok_or_else(|| StorageContractError::Refused {
            subject: subject.to_owned(),
            reason: "storage-path-outside-owned-roots".to_owned(),
        })?;
    let canonical_root = canonicalize_existing_or_nearest_ancestor(root, subject)?;
    let canonical_target = canonicalize_existing_or_nearest_ancestor(path, subject)?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(StorageContractError::Refused {
            subject: subject.to_owned(),
            reason: "storage-path-escapes-owned-root".to_owned(),
        });
    }
    Ok(())
}

fn canonicalize_existing_or_nearest_ancestor(
    path: &Path,
    subject: &str,
) -> Result<PathBuf, StorageContractError> {
    let mut current = path;
    let mut missing_suffix = Vec::new();
    loop {
        match std::fs::canonicalize(current) {
            Ok(canonical) => {
                let mut resolved = canonical;
                for component in missing_suffix.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let leaf = current
                    .file_name()
                    .ok_or_else(|| StorageContractError::Refused {
                        subject: subject.to_owned(),
                        reason: "storage-path-has-no-leaf".to_owned(),
                    })?;
                missing_suffix.push(leaf.to_os_string());
                current = current
                    .parent()
                    .ok_or_else(|| StorageContractError::Refused {
                        subject: subject.to_owned(),
                        reason: "storage-path-has-no-parent".to_owned(),
                    })?;
            }
            Err(err) => {
                return Err(StorageContractError::Refused {
                    subject: subject.to_owned(),
                    reason: format!("storage-path-canonicalize-failed:{err}"),
                });
            }
        }
    }
}

fn parse_mode(raw: &str, subject: &str) -> Result<u32, StorageContractError> {
    let trimmed = raw.trim_start_matches('0');
    let normalized = if trimmed.is_empty() { "0" } else { trimmed };
    u32::from_str_radix(normalized, 8).map_err(|_| StorageContractError::Invalid {
        subject: subject.to_owned(),
        detail: format!("invalid-mode:{raw}"),
    })
}

fn resolve_uid(principal: &PrincipalRef) -> Result<Uid, StorageContractError> {
    match principal.kind {
        PrincipalKind::Uid => principal
            .value
            .as_str()
            .parse::<u32>()
            .map(Uid::from_raw)
            .map_err(|_| StorageContractError::Invalid {
                subject: principal.value.as_str().to_owned(),
                detail: "invalid-uid".to_owned(),
            }),
        PrincipalKind::User => User::from_name(principal.value.as_str())
            .map_err(|err| StorageContractError::Invalid {
                subject: principal.value.as_str().to_owned(),
                detail: err.to_string(),
            })?
            .map(|user| user.uid)
            .ok_or_else(|| StorageContractError::Invalid {
                subject: principal.value.as_str().to_owned(),
                detail: "unknown-user".to_owned(),
            }),
        _ => Err(StorageContractError::Invalid {
            subject: principal.value.as_str().to_owned(),
            detail: "principal-is-not-uid-or-user".to_owned(),
        }),
    }
}

fn resolve_gid(principal: &PrincipalRef) -> Result<Gid, StorageContractError> {
    match principal.kind {
        PrincipalKind::Gid => principal
            .value
            .as_str()
            .parse::<u32>()
            .map(Gid::from_raw)
            .map_err(|_| StorageContractError::Invalid {
                subject: principal.value.as_str().to_owned(),
                detail: "invalid-gid".to_owned(),
            }),
        PrincipalKind::Group => Group::from_name(principal.value.as_str())
            .map_err(|err| StorageContractError::Invalid {
                subject: principal.value.as_str().to_owned(),
                detail: err.to_string(),
            })?
            .map(|group| group.gid)
            .ok_or_else(|| StorageContractError::Invalid {
                subject: principal.value.as_str().to_owned(),
                detail: "unknown-group".to_owned(),
            }),
        _ => Err(StorageContractError::Invalid {
            subject: principal.value.as_str().to_owned(),
            detail: "principal-is-not-gid-or-group".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::types::BundleOpId;
    use d2b_core::bundle::Bundle;
    use d2b_core::bundle_resolver::BundleResolver;
    use d2b_core::contract_id::{ContractId, ContractText, PathTemplate};
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;
    use d2b_core::storage::{
        ActorKind, ActorRef, CleanupPolicy, DegradeScope, DegradedReason, LeaseClass,
        LedgerStorageClass, PrincipalKind, PrincipalRef, RepairPolicy, SensitivityClass,
        StorageAdoptionPolicy, StorageInvariant, StorageJson, StorageLifecycle, StoragePathSpec,
        StoragePersistence, StorageRestartPolicy,
    };
    use d2b_core::sync::{
        FdPassingMechanism, FdPassingPolicy, InheritancePolicy, LockAcquireOrder,
        LockAdoptionPolicy, LockKind, LockScopeClass, LockSpec, LockStaleKind, LockStalePolicy,
        LockTimeoutKind, LockTimeoutPolicy, SyncJson,
    };

    #[test]
    fn template_paths_are_check_only_unless_expanded() {
        assert!(has_unexpanded_template("/run/d2b/vms/<vm>"));
        assert!(!has_unexpanded_template("/run/d2b"));
    }

    #[test]
    fn etc_paths_are_apply_check_only() {
        assert!(apply_is_check_only(Path::new("/etc/d2b")));
        assert!(apply_is_check_only(Path::new("/etc/d2b/bundle.json")));
        assert!(!apply_is_check_only(Path::new("/run/d2b")));
    }

    #[test]
    fn owned_roots_are_closed() {
        assert!(validate_owned_root(Path::new("/run/d2b"), "x").is_ok());
        assert_refused_reason(
            validate_owned_root(Path::new("/var/lib/d2b/../../etc/malicious"), "x"),
            "storage-path-parent-dir-refused",
        );
        assert_refused_reason(
            validate_owned_root(Path::new("/var/lib/d2b/../d2b-escape"), "x"),
            "storage-path-parent-dir-refused",
        );
        assert_refused_reason(
            validate_owned_root(Path::new("/home/not-d2b"), "x"),
            "storage-path-outside-owned-roots",
        );
    }

    #[test]
    fn canonical_root_check_rejects_symlink_escape() {
        let tmp = project_scratch("canonical-root-check-rejects-symlink-escape");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", root.join("escape")).unwrap();
            assert_refused_reason(
                validate_owned_root_against(&root.join("escape/passwd"), "x", &[&root]),
                "storage-path-escapes-owned-root",
            );
        }
    }

    #[test]
    fn mode_parser_reads_octal() {
        assert_eq!(parse_mode("0750", "x").unwrap(), 0o750);
        assert_eq!(parse_mode("0", "x").unwrap(), 0);
        assert!(parse_mode("bad", "x").is_err());
    }

    #[test]
    fn reconcile_refuses_non_directory_apply_without_mutation() {
        let resolver = resolver_with_storage_path(
            "path:regular-file",
            "/var/lib/d2b/storage-contract-regular-file",
            StoragePathKind::RegularFile,
            sync_with_lock(lock("lock:daemon", true, FdPassingMechanism::None, false)),
        );

        let err = reconcile_storage_scope(&resolver, &BundleOpId::new("path:regular-file"), true)
            .expect_err("regular files are check-only in broker reconcile");
        assert_refused_reason(Err(err), "storage-apply-supported-for-directory-only");
    }

    #[test]
    fn reconcile_external_grant_skips_filesystem_root_validation() {
        let resolver = resolver_with_storage_path(
            "path:external-grant",
            "/sys/class/net/work-l2",
            StoragePathKind::ExternalGrantOnly,
            sync_with_lock(lock("lock:daemon", true, FdPassingMechanism::None, false)),
        );

        let checked =
            reconcile_storage_scope(&resolver, &BundleOpId::new("path:external-grant"), true)
                .expect(
                    "external grant rows are check-only and do not validate as filesystem paths",
                );
        assert_eq!(checked.status, StorageReconcileStatus::CheckedOnly);
        assert!(!checked.applied);
    }

    #[test]
    fn reconcile_refuses_etc_d2b_apply_attempts() {
        let resolver = resolver_with_storage_path(
            "path:config-root",
            "/etc/d2b/bundle.json",
            StoragePathKind::Directory,
            sync_with_lock(lock("lock:daemon", true, FdPassingMechanism::None, false)),
        );

        let err = reconcile_storage_scope(&resolver, &BundleOpId::new("path:config-root"), true)
            .expect_err("nix-managed config roots are not broker-mutated");
        assert_refused_reason(Err(err), "storage-config-root-is-nix-managed");
    }

    #[test]
    fn validate_lock_spec_requires_ofd_cloexec_and_fd_transfer_lease_records() {
        let missing_cloexec = resolver_with_storage_path(
            "path:run-root",
            "/run/d2b",
            StoragePathKind::Directory,
            sync_with_lock(lock("lock:daemon", false, FdPassingMechanism::None, false)),
        );
        let err = validate_lock_spec(&missing_cloexec, &BundleOpId::new("lock:daemon"))
            .expect_err("OFD locks must require close-on-exec");
        assert_invalid_detail(Err(err), "ofd-lock-missing-cloexec");

        let missing_lease = resolver_with_storage_path(
            "path:run-root",
            "/run/d2b",
            StoragePathKind::Directory,
            sync_with_lock(lock(
                "lock:daemon",
                true,
                FdPassingMechanism::ScmRights,
                false,
            )),
        );
        let err = validate_lock_spec(&missing_lease, &BundleOpId::new("lock:daemon"))
            .expect_err("fd transfer locks must require lease transfer records");
        assert_invalid_detail(Err(err), "fd-transfer-missing-lease-record");

        let valid = resolver_with_storage_path(
            "path:run-root",
            "/run/d2b",
            StoragePathKind::Directory,
            sync_with_lock(lock(
                "lock:daemon",
                true,
                FdPassingMechanism::ScmRights,
                true,
            )),
        );
        let response =
            validate_lock_spec(&valid, &BundleOpId::new("lock:daemon")).expect("valid lock");
        assert!(response.cloexec_required);
        assert_eq!(response.fd_passing_mechanism, "ScmRights");
    }

    #[test]
    fn broker_storage_and_sync_requests_are_opaque_id_only() {
        let storage =
            serde_json::to_value(d2b_contracts::broker_wire::ReconcileStorageScopeRequest {
                storage_ref: BundleOpId::new("path:run-root"),
                apply: true,
                tracing_span_id: None,
            })
            .expect("serialize storage request");
        assert_eq!(
            storage
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec![
                "apply".to_owned(),
                "storageRef".to_owned(),
                "tracingSpanId".to_owned(),
            ]
        );

        let lock = serde_json::to_value(d2b_contracts::broker_wire::ValidateLockSpecRequest {
            lock_ref: BundleOpId::new("lock:daemon"),
            tracing_span_id: None,
        })
        .expect("serialize lock request");
        assert_eq!(
            lock.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["lockRef".to_owned(), "tracingSpanId".to_owned()]
        );
        for value in [&storage, &lock] {
            for forbidden in [
                "path",
                "pathTemplate",
                "mode",
                "owner",
                "group",
                "cleanupPolicy",
                "repairPolicy",
                "fdPassingPolicy",
            ] {
                assert!(
                    value.get(forbidden).is_none(),
                    "request must not carry broker-resolved field {forbidden}: {value}"
                );
            }
        }
    }

    fn assert_refused_reason(
        result: Result<(), StorageContractError>,
        expected_reason: &'static str,
    ) {
        match result {
            Err(StorageContractError::Refused { reason, .. }) => {
                assert_eq!(reason, expected_reason);
            }
            other => panic!("expected refused reason {expected_reason}, got {other:?}"),
        }
    }

    fn assert_invalid_detail(
        result: Result<ValidateLockSpecResponse, StorageContractError>,
        expected_detail: &'static str,
    ) {
        match result {
            Err(StorageContractError::Invalid { detail, .. }) => {
                assert_eq!(detail, expected_detail);
            }
            other => panic!("expected invalid detail {expected_detail}, got {other:?}"),
        }
    }

    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn project_scratch(name: &str) -> ScratchDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("storage-contract-test-scratch");
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join(format!(
            "{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ScratchDir(dir)
    }

    fn resolver_with_storage_path(
        id: &str,
        path: &str,
        kind: StoragePathKind,
        sync_contract: SyncJson,
    ) -> BundleResolver {
        let storage_contract = storage(id, path, kind);
        let bundle = Bundle {
            bundle_version: 6,
            schema_version: "v2".to_owned(),
            public_manifest_path: "manifest.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: Some("storage.json".to_owned()),
            sync_path: Some("sync.json".to_owned()),
            allocator_path: None,
            realm_controllers_path: None,
            realm_identity_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: d2b_core::bundle::BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        BundleResolver::from_artifacts_with_optional_contracts(
            bundle,
            minimal_host(),
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            Some(storage_contract),
            Some(sync_contract),
            None,
            None,
            manifest(),
        )
    }

    fn storage(id: &str, path: &str, kind: StoragePathKind) -> StorageJson {
        StorageJson {
            schema_version: "v2".to_owned(),
            roots: Vec::new(),
            paths: vec![StoragePathSpec {
                id: ContractId::parse(id).unwrap(),
                scope: ContractId::parse("host").unwrap(),
                path_template: PathTemplate::parse(path).unwrap(),
                kind,
                lifecycle: StorageLifecycle::BootScopedReadoptable,
                persistence: StoragePersistence::BootScoped,
                owner: principal(PrincipalKind::Uid, "0"),
                group: principal(PrincipalKind::Gid, "0"),
                mode: "0750".to_owned(),
                access_acl: Vec::new(),
                default_acl: Vec::new(),
                creator: actor(ActorKind::Broker, "d2b-priv-broker"),
                writers: vec![actor(ActorKind::Broker, "d2b-priv-broker")],
                readers: vec![actor(ActorKind::Daemon, "d2bd")],
                cleanup_policy: CleanupPolicy::Boot,
                repair_policy: RepairPolicy::BrokerReconcile,
                restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
                adoption_policy: StorageAdoptionPolicy::AdoptWithLiveOwnerProof,
                lease_class: LeaseClass::None,
                sensitivity: SensitivityClass::Private,
                no_follow: true,
                recursive: false,
                invariants: vec![StorageInvariant::NoSymlink],
            }],
            restart_policies: Vec::new(),
            degraded_states: vec![d2b_core::storage::DegradedStateSpec {
                reason: DegradedReason::LockOwnerAmbiguous,
                scope: DegradeScope::Host,
                storage_class: LedgerStorageClass::TamperEvidentSegmented,
                remediation_id: ContractId::parse("remediate:vm-status").unwrap(),
            }],
            remediations: vec![d2b_core::storage::RemediationSpec {
                id: ContractId::parse("remediate:vm-status").unwrap(),
                command: ContractText::parse("d2b vm status <vm>").unwrap(),
                description: ContractText::parse("Inspect VM status").unwrap(),
            }],
        }
    }

    fn sync_with_lock(lock: LockSpec) -> SyncJson {
        SyncJson {
            schema_version: "v2".to_owned(),
            locks: vec![lock],
        }
    }

    fn lock(
        id: &str,
        cloexec_required: bool,
        mechanism: FdPassingMechanism,
        lease_transfer_record_required: bool,
    ) -> LockSpec {
        LockSpec {
            id: ContractId::parse(id).unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: Some(PathTemplate::parse("/run/d2b/daemon.lock").unwrap()),
            resource_id: None,
            kind: LockKind::Ofd,
            owner_process: actor(ActorKind::Daemon, "d2bd"),
            allowed_holders: vec![actor(ActorKind::Daemon, "d2bd")],
            inheritance_policy: InheritancePolicy::CloseOnExec,
            fd_passing_policy: FdPassingPolicy {
                mechanism,
                lease_transfer_record_required,
            },
            acquire_order: LockAcquireOrder {
                scope_class: LockScopeClass::Global,
                anchored_root: ContractId::parse("run").unwrap(),
                normalized_path: ContractId::parse("daemon.lock").unwrap(),
                lock_id: ContractId::parse(id).unwrap(),
            },
            timeout_policy: LockTimeoutPolicy {
                kind: LockTimeoutKind::FailFast,
                timeout_ms: None,
            },
            stale_policy: LockStalePolicy {
                kind: LockStaleKind::PidfdProofRequired,
                degraded_reason: DegradedReason::LockOwnerAmbiguous,
            },
            adoption_policy: LockAdoptionPolicy::ReacquireAfterProof,
            degrade_scope: DegradeScope::Host,
            release_authority: actor(ActorKind::Daemon, "d2bd"),
            cloexec_required,
        }
    }

    fn actor(kind: ActorKind, value: &str) -> ActorRef {
        ActorRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    fn principal(kind: PrincipalKind, value: &str) -> PrincipalRef {
        PrincipalRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        }
    }

    fn minimal_host() -> HostJson {
        serde_json::from_str(r##"{
            "schemaVersion":"v2",
            "site":{"allowUnsafeEastWest":false},
            "environments":[],
            "nftables":{"family":"inet","table":"d2b","chains":[],"tableHashAfterApply":null,"ownershipId":"test"},
            "hostsFile":{"startMarker":"# begin","endMarker":"# end","rule":"test"},
            "networkManager":{"filePath":"/etc/NetworkManager/conf.d/00-d2b.conf","matchCriteria":[],"reloadBehavior":"none","ownership":{"owner":"root","group":"root","mode":"0644","driftPolicy":"replace-managed-block"}},
            "kernelModules":[],
            "fdOwnership":[],
            "cloudHypervisorCapabilities":[],
            "ifNameMappings":[],
            "ch":{"netHandoffMode":"tap-fd"},
            "firewallCoexistencePolicy":{"manager":"none","policy":"coexist","rationale":"test"}
        }"##)
        .expect("minimal HostJson")
    }

    fn manifest() -> ManifestV04 {
        ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"obsVsockCid":0,"obsVsockHostSocket":"","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318,"signozUrl":"","vmName":""}}"#,
        )
        .expect("minimal ManifestV04")
    }
}
