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

use nixling_core::bundle_resolver::BundleResolver;
use nixling_ipc::types::PathClass;

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
    // nixlingd -g users`) and carries per-runner POSIX ACLs. Preserve
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
    p.starts_with("/var/lib/nixling") || p.starts_with("/run/nixling")
}

pub fn live_prepare_runtime_dir(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &nixling_ipc::broker_wire::PrepareDirRequest,
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

pub fn live_prepare_state_dir(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &nixling_ipc::broker_wire::PrepareDirRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<(), super::OpError> {
    if req.path_class != PathClass::Vm {
        return Err(super::OpError::InvalidInput {
            detail: format!(
                "PrepareStateDir requires pathClass=vm, got {:?}",
                req.path_class
            ),
        });
    }
    let intent = resolver
        .resolve_prepare_dir_intent(req.vm_id.as_str(), false)
        .ok_or_else(|| super::OpError::UnknownSubject {
            operation: "PrepareStateDir",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nixling-w3-s2-state-{}-{}",
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
        // per-VM root as `nixlingd:users 2770` with per-runner POSIX
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
            // The prepare asks for 0o750 — the mask-clipping value the
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
}
