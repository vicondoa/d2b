//! Broker storage/sync contract handlers (ADR 0034).
//!
//! These handlers are the first broker-facing surface over the generated
//! `storage.json` and `sync.json` artifacts. They deliberately accept only
//! opaque bundle ids from the daemon and resolve every path/owner/mode from
//! the broker's trusted bundle copy.

use std::path::{Path, PathBuf};

use nix::unistd::{Gid, Group, Uid, User};
use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::storage::{PrincipalKind, PrincipalRef, StoragePathKind};
use nixling_ipc::broker_wire::{
    ReconcileStorageScopeResponse, StorageReconcileStatus, ValidateLockSpecResponse,
};
use nixling_ipc::types::BundleOpId;

use super::hosts::stable_hash_str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageContractError {
    UnknownStorage(String),
    UnknownLock(String),
    Refused { subject: String, reason: String },
    Invalid { subject: String, detail: String },
    Io { path: PathBuf, detail: String },
}

impl std::fmt::Display for StorageContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownStorage(id) => write!(f, "unknown storage contract id {id:?}"),
            Self::UnknownLock(id) => write!(f, "unknown lock contract id {id:?}"),
            Self::Refused { subject, reason } => write!(f, "{subject}: refused: {reason}"),
            Self::Invalid { subject, detail } => write!(f, "{subject}: invalid: {detail}"),
            Self::Io { path, detail } => write!(f, "I/O error on {}: {detail}", path.display()),
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
        if apply && path.starts_with("/etc/nixling") {
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
                    path: path_buf.clone(),
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
    if spec.kind == nixling_core::sync::LockKind::Ofd && !spec.cloexec_required {
        return Err(StorageContractError::Invalid {
            subject: lock_ref.as_str().to_owned(),
            detail: "ofd-lock-missing-cloexec".to_owned(),
        });
    }
    if spec.fd_passing_policy.mechanism != nixling_core::sync::FdPassingMechanism::None
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
    path.starts_with("/etc/nixling")
}

fn validate_owned_root(path: &Path, subject: &str) -> Result<(), StorageContractError> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(StorageContractError::Refused {
            subject: subject.to_owned(),
            reason: "storage-path-parent-dir-refused".to_owned(),
        });
    }
    let root = owned_root_for(path).ok_or_else(|| StorageContractError::Refused {
        subject: subject.to_owned(),
        reason: "storage-path-outside-owned-roots".to_owned(),
    })?;
    let canonical_root = canonicalize_existing(root, subject)?;
    let canonical_target = canonicalize_existing_or_parent(path, subject)?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(StorageContractError::Refused {
            subject: subject.to_owned(),
            reason: "storage-path-escapes-owned-root".to_owned(),
        });
    }
    Ok(())
}

fn owned_root_for(path: &Path) -> Option<&'static Path> {
    [
        Path::new("/etc/nixling"),
        Path::new("/var/lib/nixling"),
        Path::new("/run/nixling"),
        Path::new("/var/cache/nixling"),
    ]
    .into_iter()
    .find(|root| path.starts_with(root))
}

fn canonicalize_existing(path: &Path, subject: &str) -> Result<PathBuf, StorageContractError> {
    std::fs::canonicalize(path).map_err(|err| StorageContractError::Refused {
        subject: subject.to_owned(),
        reason: format!("storage-root-canonicalize-failed:{err}"),
    })
}

fn canonicalize_existing_or_parent(
    path: &Path,
    subject: &str,
) -> Result<PathBuf, StorageContractError> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| StorageContractError::Refused {
                subject: subject.to_owned(),
                reason: "storage-path-has-no-parent".to_owned(),
            })?;
            let canonical_parent =
                std::fs::canonicalize(parent).map_err(|err| StorageContractError::Refused {
                    subject: subject.to_owned(),
                    reason: format!("storage-parent-canonicalize-failed:{err}"),
                })?;
            let leaf = path
                .file_name()
                .ok_or_else(|| StorageContractError::Refused {
                    subject: subject.to_owned(),
                    reason: "storage-path-has-no-leaf".to_owned(),
                })?;
            Ok(canonical_parent.join(leaf))
        }
        Err(err) => Err(StorageContractError::Refused {
            subject: subject.to_owned(),
            reason: format!("storage-path-canonicalize-failed:{err}"),
        }),
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

    #[test]
    fn template_paths_are_check_only_unless_expanded() {
        assert!(has_unexpanded_template("/run/nixling/vms/<vm>"));
        assert!(!has_unexpanded_template("/run/nixling"));
    }

    #[test]
    fn etc_paths_are_apply_check_only() {
        assert!(apply_is_check_only(Path::new("/etc/nixling")));
        assert!(apply_is_check_only(Path::new("/etc/nixling/bundle.json")));
        assert!(!apply_is_check_only(Path::new("/run/nixling")));
    }

    #[test]
    fn owned_roots_are_closed() {
        assert_eq!(
            owned_root_for(Path::new("/run/nixling")).unwrap(),
            Path::new("/run/nixling")
        );
        assert!(
            validate_owned_root(Path::new("/var/lib/nixling/../../etc/malicious"), "x").is_err()
        );
        assert!(validate_owned_root(Path::new("/tmp/nixling"), "x").is_err());
    }

    #[test]
    fn canonical_root_check_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
            let canonical_root = std::fs::canonicalize(&root).unwrap();
            let canonical_target =
                canonicalize_existing_or_parent(&root.join("escape/new"), "x").unwrap();
            assert!(!canonical_target.starts_with(canonical_root));
        }
    }

    #[test]
    fn mode_parser_reads_octal() {
        assert_eq!(parse_mode("0750", "x").unwrap(), 0o750);
        assert_eq!(parse_mode("0", "x").unwrap(), 0);
        assert!(parse_mode("bad", "x").is_err());
    }
}
