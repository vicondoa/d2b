//! Broker swtpm-dir first-run hardening (issue #64).
//!
//! The persistent per-VM swtpm state dir (`<stateDir>/vms/<vm>/swtpm`,
//! mode 0700, owner `nixling-<vm>-swtpm`) holds the TPM 2.0 NVRAM + EK
//! seed. swtpm runs inside a user namespace (ADR 0021) and the broker
//! skips `apply_mount_actions` for userNS spawns, so swtpm opens the
//! NVRAM **by pathname**. This module is the broker's pre-spawn hook
//! that provisions + hardens ONLY that persistent dir before the
//! runner is cloned:
//!
//! - fresh create -> stamp owner + mode 0700 (owner/mode stamped only
//!   on the create path; never chown-wipes an existing dir);
//! - existing correct-owner dir -> reconcile in place (clear access +
//!   default ACLs, re-assert 0700), contents preserved;
//! - symlink / non-dir / owner-or-group mismatch -> fail closed with a
//!   typed, PATH-FREE error (never recreates / chown-wipes NVRAM).
//!
//! An identity-bound tamper-guard marker (`/var/lib/nixling/swtpm-
//! markers/<vm>`, root:root 0600, a REGULAR FILE outside both the
//! swtpm dir and the per-VM root) records the trusted swtpm-dir
//! identity (`st_dev`/`st_ino` + first-provision stamp). On every
//! subsequent spawn the marker is verified against the live dir's
//! identity; a missing dir after prior provision, or an `st_ino`
//! mismatch (e.g. a fresh correct-owner empty replacement smuggled in
//! under the sticky per-VM root), fails closed.
//!
//! The runtime socket dir (`/run/nixling/vms/<vm>`) posture is left
//! untouched; only a stale `tpm.sock` under it is unlinked.
//!
//! Every error is PATH-FREE: the [`SwtpmHardenError`] `Display` carries
//! only closed-set reason slugs, never a raw path, so the broker can
//! fold it straight into a public error envelope and a path-free
//! `PrepareSwtpmDir` audit record.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use nix::libc;

use crate::ops::audit_op::{SwtpmDirAudit, SwtpmDirResult, SwtpmMarkerResult};
use crate::ops::hosts::stable_hash_str;
use crate::ops::spawn_runner::SpawnRunnerPlan;
use crate::sys::path_safe;
use crate::sys::pidfd_sys;

/// Mode the persistent swtpm state dir always carries (0700).
const SWTPM_DIR_MODE: u32 = 0o700;
/// Mode of the root-owned per-VM marker file (0600).
const MARKER_FILE_MODE: u32 = 0o600;
/// Mode of the root-owned marker tree directory (0700).
const MARKER_DIR_MODE: u32 = 0o700;
/// Root of the identity-bound tamper-guard marker tree. Intentionally
/// NOT under `<stateDir>/vms/<vm>` (which the swtpm principal can
/// mutate) nor under the swtpm dir itself.
const MARKER_TREE: &str = "/var/lib/nixling/swtpm-markers";
/// The single trusted control-socket name the broker may unlink as a
/// stale leftover under the runtime dir.
const TRUSTED_SOCKET_NAME: &str = "tpm.sock";
/// Versioned tag for the marker payload.
const MARKER_VERSION: u32 = 1;

/// Closed-set, path-free reason slugs for swtpm-dir hardening failures.
pub mod reasons {
    pub const DERIVATION_FAILED: &str = "swtpm-dir-derivation-failed";
    pub const PARENT_OPEN_FAILED: &str = "swtpm-dir-parent-open-failed";
    pub const IS_SYMLINK: &str = "swtpm-dir-is-symlink";
    pub const NOT_A_DIRECTORY: &str = "swtpm-dir-not-a-directory";
    pub const OWNER_MISMATCH: &str = "swtpm-dir-owner-mismatch";
    pub const PREV_PROVISIONED_MISSING: &str = "previously-provisioned-swtpm-state-missing";
    pub const CREATE_FAILED: &str = "swtpm-dir-create-failed";
    pub const ACL_CLEAR_FAILED: &str = "swtpm-dir-acl-clear-failed";
    pub const ACL_RESIDUAL: &str = "swtpm-dir-acl-residual";
    pub const MARKER_TREE_FAILED: &str = "swtpm-marker-tree-failed";
    pub const MARKER_WRITE_FAILED: &str = "swtpm-marker-write-failed";
    pub const ANCESTOR_ACL_FAILED: &str = "swtpm-ancestor-acl-failed";
    pub const STALE_SOCKET_FAILED: &str = "swtpm-stale-socket-unlink-failed";
}

/// Path-free typed error. `Display` MUST NOT leak a raw path; only the
/// closed-set reason slug is rendered. The carried [`SwtpmDirAudit`]
/// already has `result == FailedClosed` + `fail_reason == Some(reason)`
/// so the runtime dispatch layer can emit the terminal `PrepareSwtpmDir`
/// record straight from the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwtpmHardenError {
    pub reason: &'static str,
    pub audit: SwtpmDirAudit,
}

impl std::fmt::Display for SwtpmHardenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // PATH-FREE: only the closed-set reason slug.
        write!(f, "swtpm-dir hardening failed: {}", self.reason)
    }
}

impl std::error::Error for SwtpmHardenError {}

/// Derived, validated path set for the swtpm-dir hardening step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwtpmDirPaths {
    pub vm_id: String,
    /// Persistent swtpm state dir: `<stateDir>/vms/<vm>/swtpm`.
    pub swtpm_dir: PathBuf,
    /// Per-VM root: `<stateDir>/vms/<vm>` (the sticky 3770 parent).
    pub per_vm_root: PathBuf,
    /// Runtime socket dir: `/run/nixling/vms/<vm>`. Posture untouched;
    /// only a stale `tpm.sock` under it is unlinked.
    pub runtime_dir: PathBuf,
    /// Marker tree root (`/var/lib/nixling/swtpm-markers`).
    pub marker_dir: PathBuf,
    /// Per-VM marker file basename (the VM id).
    pub marker_name: String,
}

/// Caller-supplied identity + clock. Parametrized so unit tests can run
/// against scratch dirs as a non-root uid: production passes the swtpm
/// principal uid/gid for the dir, `0`/`0` for the marker owner, and
/// `enforce_root_parents = true`; tests pass the current uid/gid and
/// `enforce_root_parents = false`.
#[derive(Debug, Clone, Copy)]
pub struct SwtpmHardenConfig {
    /// Owner uid the swtpm dir must carry (the `nixling-<vm>-swtpm`
    /// principal).
    pub expected_uid: u32,
    /// Owner gid the swtpm dir must carry.
    pub expected_gid: u32,
    /// Owner uid the marker tree + marker file must carry (root in
    /// production).
    pub marker_owner_uid: u32,
    /// Owner gid the marker tree + marker file must carry.
    pub marker_owner_gid: u32,
    /// First-provision timestamp (ms since epoch) recorded in a fresh
    /// marker.
    pub now_ms: u64,
    /// When true, assert root-owned, non-world-writable parents for the
    /// marker tree. Tests pass scratch dirs and set this false.
    pub enforce_root_parents: bool,
}

/// Marker payload: the trusted swtpm-dir identity. Serialized as JSON.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct MarkerData {
    v: u32,
    vm: String,
    dev: u64,
    ino: u64,
    first_provisioned_ms: u64,
}

/// True for the real production state-dir tree. Tests pass scratch dirs.
fn production_swtpm_path(p: &Path) -> bool {
    p.starts_with("/var/lib/nixling/vms")
}

/// Derive + validate the path set from a resolved spawn plan. Refuses
/// any plan whose writable paths don't contain exactly the persistent
/// swtpm dir (ending `/swtpm`, NOT under `/run`) and the runtime dir
/// (under `/run/nixling/vms`).
pub fn derive_paths(plan: &SpawnRunnerPlan) -> Result<SwtpmDirPaths, &'static str> {
    let vm_id =
        parse_vm_from_subtree(&plan.cgroup_placement.subtree).ok_or(reasons::DERIVATION_FAILED)?;

    let mut swtpm_dir: Option<PathBuf> = None;
    let mut runtime_dir: Option<PathBuf> = None;
    for wp in &plan.mount_policy.writable_paths {
        let path = Path::new(&wp.path);
        if !path.is_absolute() {
            continue;
        }
        if path.starts_with("/run/") {
            if path.starts_with("/run/nixling/vms") && runtime_dir.is_none() {
                runtime_dir = Some(path.to_path_buf());
            }
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

    Ok(SwtpmDirPaths {
        marker_dir: PathBuf::from(MARKER_TREE),
        marker_name: vm_id.clone(),
        vm_id,
        swtpm_dir,
        per_vm_root,
        runtime_dir,
    })
}

fn parse_vm_from_subtree(subtree: &str) -> Option<String> {
    let normalized = subtree
        .strip_prefix("nixling.slice/")
        .or_else(|| subtree.strip_prefix("nixling/"))?;
    let vm = normalized.split('/').find(|s| !s.is_empty())?;
    if vm.is_empty() || vm.contains('\0') {
        return None;
    }
    Some(vm.trim_end_matches(".scope").to_owned())
}

/// Provision + harden the persistent swtpm state dir. Returns a
/// path-free [`SwtpmDirAudit`] (success) or a path-free
/// [`SwtpmHardenError`] (fail closed) so the dispatch layer can emit
/// the terminal `PrepareSwtpmDir` audit record on BOTH paths.
pub fn harden(
    paths: &SwtpmDirPaths,
    cfg: &SwtpmHardenConfig,
) -> Result<SwtpmDirAudit, SwtpmHardenError> {
    let base_dir_hash = stable_hash_str(&paths.swtpm_dir.display().to_string());

    let fail = |reason: &'static str, marker_result: SwtpmMarkerResult| SwtpmHardenError {
        reason,
        audit: SwtpmDirAudit {
            vm_id: paths.vm_id.clone(),
            base_dir_hash: base_dir_hash.clone(),
            result: SwtpmDirResult::FailedClosed,
            mode: SWTPM_DIR_MODE,
            owner_uid: cfg.expected_uid,
            owner_gid: cfg.expected_gid,
            marker_result,
            fail_reason: Some(reason.to_owned()),
        },
    };

    // 1. Establish + open the root-owned marker tree, then read any
    //    existing marker (fail closed on tamper).
    let marker_dir_fd = ensure_marker_tree(paths, cfg)
        .map_err(|reason| fail(reason, SwtpmMarkerResult::FailedClosed))?;
    let existing_marker = read_marker(&marker_dir_fd, paths, cfg)
        .map_err(|reason| fail(reason, SwtpmMarkerResult::FailedClosed))?;

    // 2. Open the per-VM root path-safely.
    if cfg.enforce_root_parents && production_swtpm_path(&paths.swtpm_dir) {
        path_safe::refuse_world_writable_parent(&paths.swtpm_dir)
            .map_err(|_| fail(reasons::PARENT_OPEN_FAILED, SwtpmMarkerResult::FailedClosed))?;
    }
    let per_vm_root_fd = path_safe::open_dir_path_safe(&paths.per_vm_root)
        .map_err(|_| fail(reasons::PARENT_OPEN_FAILED, SwtpmMarkerResult::FailedClosed))?;

    // 3. Stat the swtpm dir under the per-VM root WITHOUT following a
    //    symlink.
    let swtpm_stat = path_safe::fstatat_nofollow(&per_vm_root_fd, "swtpm")
        .map_err(|_| fail(reasons::PARENT_OPEN_FAILED, SwtpmMarkerResult::FailedClosed))?;

    let (result, marker_result) = match swtpm_stat {
        None => {
            // Absent dir. A present marker means the NVRAM vanished
            // after a prior provision: fail closed, never silently
            // re-provision.
            if existing_marker.is_some() {
                return Err(fail(
                    reasons::PREV_PROVISIONED_MISSING,
                    SwtpmMarkerResult::FailedClosed,
                ));
            }
            // Fresh create: stamp owner + mode ONLY here.
            let (dev, ino) = create_fresh_swtpm_dir(&per_vm_root_fd, cfg)
                .map_err(|reason| fail(reason, SwtpmMarkerResult::FailedClosed))?;
            write_marker(&marker_dir_fd, paths, cfg, dev, ino)
                .map_err(|reason| fail(reason, SwtpmMarkerResult::FailedClosed))?;
            (SwtpmDirResult::Created, SwtpmMarkerResult::Created)
        }
        Some(stat) => {
            let fmt = stat.st_mode & libc::S_IFMT;
            if fmt == libc::S_IFLNK {
                return Err(fail(reasons::IS_SYMLINK, SwtpmMarkerResult::FailedClosed));
            }
            if fmt != libc::S_IFDIR {
                return Err(fail(
                    reasons::NOT_A_DIRECTORY,
                    SwtpmMarkerResult::FailedClosed,
                ));
            }
            if stat.st_uid != cfg.expected_uid || stat.st_gid != cfg.expected_gid {
                // NEVER chown-wipe: refuse fail closed.
                return Err(fail(
                    reasons::OWNER_MISMATCH,
                    SwtpmMarkerResult::FailedClosed,
                ));
            }

            // Open the existing dir; bind identity to the held fd.
            let dir_fd = path_safe::open_at(
                per_vm_root_fd.as_fd(),
                Path::new("swtpm"),
                rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
            )
            .map_err(|_| fail(reasons::PARENT_OPEN_FAILED, SwtpmMarkerResult::FailedClosed))?;
            let fd_stat = path_safe::fstat_fd(dir_fd.as_fd())
                .map_err(|_| fail(reasons::PARENT_OPEN_FAILED, SwtpmMarkerResult::FailedClosed))?;
            // Re-verify owner from the held fd (TOCTOU defense).
            if fd_stat.st_uid != cfg.expected_uid || fd_stat.st_gid != cfg.expected_gid {
                return Err(fail(
                    reasons::OWNER_MISMATCH,
                    SwtpmMarkerResult::FailedClosed,
                ));
            }
            let dev = fd_stat.st_dev as u64;
            let ino = fd_stat.st_ino as u64;

            let marker_result = match &existing_marker {
                Some(marker) => {
                    // Identity bind: a fresh correct-owner empty
                    // replacement (different st_ino) fails closed.
                    if marker.dev != dev || marker.ino != ino {
                        return Err(fail(
                            reasons::PREV_PROVISIONED_MISSING,
                            SwtpmMarkerResult::FailedClosed,
                        ));
                    }
                    SwtpmMarkerResult::Verified
                }
                None => SwtpmMarkerResult::FailedClosed, // placeholder; set below
            };

            // Reconcile: clear ACLs + re-assert 0700, preserve contents.
            let had_acl = reconcile_existing_swtpm_dir(dir_fd.as_fd(), &fd_stat)
                .map_err(|reason| fail(reason, SwtpmMarkerResult::FailedClosed))?;

            let marker_result = if existing_marker.is_none() {
                // Upgrade path: first run on a pre-existing dir, no
                // marker yet. Record current identity.
                write_marker(&marker_dir_fd, paths, cfg, dev, ino)
                    .map_err(|reason| fail(reason, SwtpmMarkerResult::FailedClosed))?;
                SwtpmMarkerResult::Created
            } else {
                marker_result
            };

            let result = if had_acl {
                SwtpmDirResult::Reconciled
            } else {
                SwtpmDirResult::VerifiedClean
            };
            (result, marker_result)
        }
    };

    // 4. Idempotent ancestor traverse ACL for the swtpm principal on
    //    the per-VM root.
    apply_ancestor_traverse_acl(per_vm_root_fd.as_fd(), cfg.expected_uid)
        .map_err(|reason| fail(reason, marker_result))?;

    // 5. Unlink a stale trusted control socket under the runtime dir.
    //    The runtime dir's own posture is left untouched.
    unlink_stale_socket(&paths.runtime_dir).map_err(|reason| fail(reason, marker_result))?;

    Ok(SwtpmDirAudit {
        vm_id: paths.vm_id.clone(),
        base_dir_hash,
        result,
        mode: SWTPM_DIR_MODE,
        owner_uid: cfg.expected_uid,
        owner_gid: cfg.expected_gid,
        marker_result,
        fail_reason: None,
    })
}

/// Ensure the marker tree dir exists root:root 0700 and return an
/// opened dirfd. Fails closed (path-free) on any failure.
fn ensure_marker_tree(
    paths: &SwtpmDirPaths,
    cfg: &SwtpmHardenConfig,
) -> Result<OwnedFd, &'static str> {
    if cfg.enforce_root_parents && paths.marker_dir.starts_with("/var/lib/nixling") {
        path_safe::refuse_world_writable_parent(&paths.marker_dir)
            .map_err(|_| reasons::MARKER_TREE_FAILED)?;
    }
    path_safe::ensure_dir(
        &paths.marker_dir,
        MARKER_DIR_MODE,
        Some(cfg.marker_owner_uid),
        Some(cfg.marker_owner_gid),
    )
    .map_err(|_| reasons::MARKER_TREE_FAILED)?;
    path_safe::open_dir_path_safe(&paths.marker_dir).map_err(|_| reasons::MARKER_TREE_FAILED)
}

/// Read + validate the per-VM marker file. Returns `Ok(None)` when no
/// marker exists, `Ok(Some(_))` for a valid marker, and a path-free
/// `previously-provisioned-swtpm-state-missing` slug for ANY tamper
/// (symlink, non-regular, foreign owner/mode, parse/vm mismatch).
fn read_marker(
    marker_dir_fd: &OwnedFd,
    paths: &SwtpmDirPaths,
    cfg: &SwtpmHardenConfig,
) -> Result<Option<MarkerData>, &'static str> {
    let stat = match path_safe::fstatat_nofollow(marker_dir_fd, &paths.marker_name) {
        Ok(Some(stat)) => stat,
        Ok(None) => return Ok(None),
        Err(_) => return Err(reasons::PREV_PROVISIONED_MISSING),
    };
    let fmt = stat.st_mode & libc::S_IFMT;
    if fmt == libc::S_IFLNK || fmt != libc::S_IFREG {
        return Err(reasons::PREV_PROVISIONED_MISSING);
    }
    if stat.st_uid != cfg.marker_owner_uid {
        return Err(reasons::PREV_PROVISIONED_MISSING);
    }
    // Reject any group/other permission bit: marker must be 0600.
    if (stat.st_mode & 0o077) != 0 {
        return Err(reasons::PREV_PROVISIONED_MISSING);
    }

    let file_fd = path_safe::open_at(
        marker_dir_fd.as_fd(),
        Path::new(&paths.marker_name),
        rustix::fs::OFlags::RDONLY,
    )
    .map_err(|_| reasons::PREV_PROVISIONED_MISSING)?;
    // TOCTOU re-check on the held fd.
    let fd_stat =
        path_safe::fstat_fd(file_fd.as_fd()).map_err(|_| reasons::PREV_PROVISIONED_MISSING)?;
    if (fd_stat.st_mode & libc::S_IFMT) != libc::S_IFREG || fd_stat.st_uid != cfg.marker_owner_uid {
        return Err(reasons::PREV_PROVISIONED_MISSING);
    }

    let mut buf = String::new();
    let mut file = std::fs::File::from(file_fd);
    file.read_to_string(&mut buf)
        .map_err(|_| reasons::PREV_PROVISIONED_MISSING)?;
    let marker: MarkerData =
        serde_json::from_str(&buf).map_err(|_| reasons::PREV_PROVISIONED_MISSING)?;
    if marker.v != MARKER_VERSION || marker.vm != paths.vm_id {
        return Err(reasons::PREV_PROVISIONED_MISSING);
    }
    Ok(Some(marker))
}

/// Atomically write the per-VM marker as `O_EXCL` (never overwrite an
/// existing marker), root:root 0600, with a parent fsync for
/// durability.
fn write_marker(
    marker_dir_fd: &OwnedFd,
    paths: &SwtpmDirPaths,
    cfg: &SwtpmHardenConfig,
    dev: u64,
    ino: u64,
) -> Result<(), &'static str> {
    let marker = MarkerData {
        v: MARKER_VERSION,
        vm: paths.vm_id.clone(),
        dev,
        ino,
        first_provisioned_ms: cfg.now_ms,
    };
    let payload = serde_json::to_vec(&marker).map_err(|_| reasons::MARKER_WRITE_FAILED)?;

    let fd = path_safe::create_file_at_safe(
        marker_dir_fd,
        &paths.marker_name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        MARKER_FILE_MODE,
    )
    .map_err(|_| reasons::MARKER_WRITE_FAILED)?;
    // Enforce exact owner + mode regardless of umask.
    path_safe::fchmod(fd.as_fd(), MARKER_FILE_MODE).map_err(|_| reasons::MARKER_WRITE_FAILED)?;
    path_safe::fchown(
        fd.as_fd(),
        Some(cfg.marker_owner_uid),
        Some(cfg.marker_owner_gid),
    )
    .map_err(|_| reasons::MARKER_WRITE_FAILED)?;
    {
        let mut file = std::fs::File::from(fd);
        file.write_all(&payload)
            .map_err(|_| reasons::MARKER_WRITE_FAILED)?;
        file.sync_all().map_err(|_| reasons::MARKER_WRITE_FAILED)?;
    }
    rustix::fs::fsync(marker_dir_fd.as_fd()).map_err(|_| reasons::MARKER_WRITE_FAILED)?;
    Ok(())
}

/// Fresh-create the swtpm dir under the per-VM root: `mkdirat` 0700,
/// stamp owner (create path ONLY), clear any ACL, verify clean, and
/// return the identity (`st_dev`/`st_ino`) bound to the held fd.
fn create_fresh_swtpm_dir(
    per_vm_root_fd: &OwnedFd,
    cfg: &SwtpmHardenConfig,
) -> Result<(u64, u64), &'static str> {
    path_safe::mkdir_at(per_vm_root_fd.as_fd(), Path::new("swtpm"), SWTPM_DIR_MODE)
        .map_err(|_| reasons::CREATE_FAILED)?;
    let dir_fd = path_safe::open_at(
        per_vm_root_fd.as_fd(),
        Path::new("swtpm"),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
    )
    .map_err(|_| reasons::CREATE_FAILED)?;
    // Confirm we opened a freshly-created directory (not a racing
    // symlink/file swapped in between mkdirat and openat — openat2
    // NOFOLLOW already refuses a symlink, but assert the type).
    let fd_stat = path_safe::fstat_fd(dir_fd.as_fd()).map_err(|_| reasons::CREATE_FAILED)?;
    if (fd_stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return Err(reasons::CREATE_FAILED);
    }
    // Owner stamp ONLY on the create path.
    path_safe::fchown(
        dir_fd.as_fd(),
        Some(cfg.expected_uid),
        Some(cfg.expected_gid),
    )
    .map_err(|_| reasons::CREATE_FAILED)?;
    path_safe::fchmod(dir_fd.as_fd(), SWTPM_DIR_MODE).map_err(|_| reasons::CREATE_FAILED)?;
    pidfd_sys::run_setfacl_clear_on_fd(dir_fd.as_fd()).map_err(|_| reasons::ACL_CLEAR_FAILED)?;
    path_safe::fchmod(dir_fd.as_fd(), SWTPM_DIR_MODE).map_err(|_| reasons::CREATE_FAILED)?;
    verify_acl_clean(dir_fd.as_fd())?;
    let final_stat = path_safe::fstat_fd(dir_fd.as_fd()).map_err(|_| reasons::CREATE_FAILED)?;
    Ok((final_stat.st_dev as u64, final_stat.st_ino as u64))
}

/// Reconcile an existing correct-owner swtpm dir in place: clear access
/// + default ACLs, re-assert 0700, verify no foreign ACL xattr remains.
/// Contents are preserved. Returns whether any reconcile mutation was
/// needed (extended ACL present OR mode != 0700) so the caller can
/// classify Reconciled vs VerifiedClean.
fn reconcile_existing_swtpm_dir(
    dir_fd: std::os::fd::BorrowedFd<'_>,
    pre_stat: &libc::stat,
) -> Result<bool, &'static str> {
    let (access_present, default_present) =
        path_safe::fd_extended_acl_present(dir_fd).map_err(|_| reasons::ACL_RESIDUAL)?;
    let mode_drift = (pre_stat.st_mode & 0o7777) != SWTPM_DIR_MODE;
    let needed = access_present || default_present || mode_drift;

    pidfd_sys::run_setfacl_clear_on_fd(dir_fd).map_err(|_| reasons::ACL_CLEAR_FAILED)?;
    path_safe::fchmod(dir_fd, SWTPM_DIR_MODE).map_err(|_| reasons::ACL_CLEAR_FAILED)?;
    verify_acl_clean(dir_fd)?;
    Ok(needed)
}

/// Verify no extended POSIX ACL xattr remains on the held fd.
fn verify_acl_clean(fd: std::os::fd::BorrowedFd<'_>) -> Result<(), &'static str> {
    let (access_present, default_present) =
        path_safe::fd_extended_acl_present(fd).map_err(|_| reasons::ACL_RESIDUAL)?;
    if access_present || default_present {
        return Err(reasons::ACL_RESIDUAL);
    }
    Ok(())
}

/// Idempotent `u:<uid>:--x` traverse ACL on the per-VM root so the
/// swtpm principal can reach its dir through the sticky 3770 parent.
fn apply_ancestor_traverse_acl(
    per_vm_root_fd: std::os::fd::BorrowedFd<'_>,
    uid: u32,
) -> Result<(), &'static str> {
    pidfd_sys::run_setfacl_op_on_fd(per_vm_root_fd, "-m", &format!("u:{uid}:--x"))
        .map_err(|_| reasons::ANCESTOR_ACL_FAILED)
}

/// Unlink only the trusted `tpm.sock` under the runtime dir, if present.
/// The runtime dir's own posture (mode / ACL / sibling entries) is left
/// untouched. A missing runtime dir is a no-op (not an error).
fn unlink_stale_socket(runtime_dir: &Path) -> Result<(), &'static str> {
    let runtime_fd = match path_safe::open_dir_path_safe(runtime_dir) {
        Ok(fd) => fd,
        // Runtime dir not yet created -> nothing stale to clean.
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(reasons::STALE_SOCKET_FAILED),
    };
    match path_safe::fstatat_nofollow(&runtime_fd, TRUSTED_SOCKET_NAME) {
        Ok(Some(_)) => path_safe::remove_path_safe(&runtime_fd, TRUSTED_SOCKET_NAME)
            .map_err(|_| reasons::STALE_SOCKET_FAILED),
        Ok(None) => Ok(()),
        Err(_) => Err(reasons::STALE_SOCKET_FAILED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn cur_uid() -> u32 {
        nix::unistd::geteuid().as_raw()
    }
    fn cur_gid() -> u32 {
        nix::unistd::getegid().as_raw()
    }

    /// Scratch root under `target/` (NOT /tmp). Each test gets a unique
    /// tree; dropped recursively.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!("swtpm-dir-{label}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
            Self { root }
        }

        /// Build a path set rooted under the scratch dir. `vm` is the VM
        /// id; `vms/<vm>` is the (sticky) per-VM root; markers live in a
        /// sibling tree.
        fn paths(&self, vm: &str) -> SwtpmDirPaths {
            let per_vm_root = self.root.join("vms").join(vm);
            SwtpmDirPaths {
                vm_id: vm.to_owned(),
                swtpm_dir: per_vm_root.join("swtpm"),
                per_vm_root,
                runtime_dir: self.root.join("run").join(vm),
                marker_dir: self.root.join("swtpm-markers"),
                marker_name: vm.to_owned(),
            }
        }

        fn cfg(&self) -> SwtpmHardenConfig {
            SwtpmHardenConfig {
                expected_uid: cur_uid(),
                expected_gid: cur_gid(),
                marker_owner_uid: cur_uid(),
                marker_owner_gid: cur_gid(),
                now_ms: 1_700_000_000_000,
                enforce_root_parents: false,
            }
        }

        /// Create the (sticky 3770) per-VM root the broker expects to
        /// already exist (PrepareStateDir made it).
        fn make_per_vm_root(&self, paths: &SwtpmDirPaths) {
            fs::create_dir_all(&paths.per_vm_root).unwrap();
            fs::set_permissions(&paths.per_vm_root, fs::Permissions::from_mode(0o3770)).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn mode_of(p: &Path) -> u32 {
        fs::symlink_metadata(p).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn fresh_create_sets_mode_owner_and_marker() {
        let s = Scratch::new("fresh");
        let paths = s.paths("alpha");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();

        let audit = harden(&paths, &cfg).expect("fresh harden ok");
        assert_eq!(audit.result, SwtpmDirResult::Created);
        assert_eq!(audit.marker_result, SwtpmMarkerResult::Created);
        assert_eq!(audit.mode, 0o700);
        assert!(paths.swtpm_dir.is_dir());
        assert_eq!(mode_of(&paths.swtpm_dir), 0o700);
        // Marker exists, regular file, 0600.
        let marker = paths.marker_dir.join(&paths.marker_name);
        assert!(marker.is_file());
        assert_eq!(mode_of(&marker), 0o600);
        // Audit/error carry no raw path.
        let json = serde_json::to_string(&audit).unwrap();
        assert!(!json.contains(&s.root.display().to_string()));
    }

    #[test]
    fn existing_correct_with_acl_drift_reconciles_and_preserves_contents() {
        let s = Scratch::new("reconcile");
        let paths = s.paths("beta");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        // First provision.
        harden(&paths, &cfg).expect("provision");
        // Drop an NVRAM-ish file + drift mode + add an ACL.
        let nvram = paths.swtpm_dir.join("tpm2-00.permall");
        fs::write(&nvram, b"nvram-state").unwrap();
        fs::set_permissions(&paths.swtpm_dir, fs::Permissions::from_mode(0o770)).unwrap();
        let _ = pidfd_sys::run_setfacl_op_on_fd(
            path_safe::open_dir_path_safe(&paths.swtpm_dir)
                .unwrap()
                .as_fd(),
            "-m",
            &format!("u:{}:rwx", cur_uid()),
        );

        let audit = harden(&paths, &cfg).expect("reconcile ok");
        assert_eq!(audit.result, SwtpmDirResult::Reconciled);
        assert_eq!(audit.marker_result, SwtpmMarkerResult::Verified);
        assert_eq!(mode_of(&paths.swtpm_dir), 0o700);
        // Contents preserved.
        assert_eq!(fs::read(&nvram).unwrap(), b"nvram-state");
        // ACL cleared.
        let (a, d) = path_safe::fd_extended_acl_present(
            path_safe::open_dir_path_safe(&paths.swtpm_dir)
                .unwrap()
                .as_fd(),
        )
        .unwrap();
        assert!(!a && !d, "extended ACL must be cleared");
    }

    #[test]
    fn upgrade_existing_dir_without_marker_creates_marker() {
        let s = Scratch::new("upgrade");
        let paths = s.paths("gamma");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        // Pre-existing correct-owner dir, NO marker (legacy deployment).
        fs::create_dir_all(&paths.swtpm_dir).unwrap();
        fs::set_permissions(&paths.swtpm_dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(paths.swtpm_dir.join("tpm2-00.permall"), b"legacy").unwrap();

        let audit = harden(&paths, &cfg).expect("upgrade ok");
        assert!(matches!(
            audit.result,
            SwtpmDirResult::Reconciled | SwtpmDirResult::VerifiedClean
        ));
        assert_eq!(audit.marker_result, SwtpmMarkerResult::Created);
        assert!(paths.marker_dir.join(&paths.marker_name).is_file());
        assert_eq!(
            fs::read(paths.swtpm_dir.join("tpm2-00.permall")).unwrap(),
            b"legacy"
        );
    }

    #[test]
    fn wrong_owner_dir_fails_closed() {
        let s = Scratch::new("wrongowner");
        let paths = s.paths("delta");
        s.make_per_vm_root(&paths);
        let mut cfg = s.cfg();
        // The dir on disk is owned by the current uid; configure a
        // DIFFERENT expected uid so the owner check trips.
        fs::create_dir_all(&paths.swtpm_dir).unwrap();
        cfg.expected_uid = cur_uid().wrapping_add(424_242);

        let err = harden(&paths, &cfg).expect_err("must fail closed");
        assert_eq!(err.reason, reasons::OWNER_MISMATCH);
        assert_eq!(err.audit.result, SwtpmDirResult::FailedClosed);
        // NVRAM dir untouched (still present).
        assert!(paths.swtpm_dir.is_dir());
        // Error display is path-free.
        assert!(!err.to_string().contains('/'));
    }

    #[test]
    fn symlink_swtpm_dir_fails_closed() {
        let s = Scratch::new("symlink");
        let paths = s.paths("epsilon");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        let target = s.root.join("evil-target");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &paths.swtpm_dir).unwrap();

        let err = harden(&paths, &cfg).expect_err("symlink must fail");
        assert_eq!(err.reason, reasons::IS_SYMLINK);
    }

    #[test]
    fn non_directory_swtpm_path_fails_closed() {
        let s = Scratch::new("nondir");
        let paths = s.paths("zeta");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        fs::write(&paths.swtpm_dir, b"i am a file").unwrap();

        let err = harden(&paths, &cfg).expect_err("non-dir must fail");
        assert_eq!(err.reason, reasons::NOT_A_DIRECTORY);
    }

    #[test]
    fn marker_present_dir_absent_fails_closed() {
        let s = Scratch::new("markerorphan");
        let paths = s.paths("eta");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        // Provision, then remove the swtpm dir (marker remains).
        harden(&paths, &cfg).expect("provision");
        fs::remove_dir_all(&paths.swtpm_dir).unwrap();

        let err = harden(&paths, &cfg).expect_err("orphan marker must fail");
        assert_eq!(err.reason, reasons::PREV_PROVISIONED_MISSING);
    }

    #[test]
    fn marker_present_empty_replacement_ino_mismatch_fails_closed() {
        let s = Scratch::new("inoswap");
        let paths = s.paths("theta");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        harden(&paths, &cfg).expect("provision");
        // Replace the dir with a fresh, correct-owner, empty dir that is
        // GUARANTEED to have a different st_ino than the marker
        // recorded. Renaming the original away (keeping its inode
        // allocated) before creating the replacement prevents inode
        // reuse from accidentally matching the marked identity.
        let stash = paths.per_vm_root.join("swtpm-stash");
        fs::rename(&paths.swtpm_dir, &stash).unwrap();
        fs::create_dir(&paths.swtpm_dir).unwrap();
        fs::set_permissions(&paths.swtpm_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let err = harden(&paths, &cfg).expect_err("ino mismatch must fail");
        assert_eq!(err.reason, reasons::PREV_PROVISIONED_MISSING);
    }

    #[test]
    fn idempotent_second_run_is_clean() {
        let s = Scratch::new("idem");
        let paths = s.paths("iota");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        harden(&paths, &cfg).expect("provision");
        let audit = harden(&paths, &cfg).expect("second run ok");
        assert_eq!(audit.marker_result, SwtpmMarkerResult::Verified);
        assert_eq!(audit.result, SwtpmDirResult::VerifiedClean);
    }

    #[test]
    fn runtime_dir_posture_untouched_and_stale_socket_removed() {
        let s = Scratch::new("runtime");
        let paths = s.paths("kappa");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        // Pre-create runtime dir with a distinctive mode + a sibling
        // file + a stale tpm.sock.
        fs::create_dir_all(&paths.runtime_dir).unwrap();
        fs::set_permissions(&paths.runtime_dir, fs::Permissions::from_mode(0o751)).unwrap();
        let sibling = paths.runtime_dir.join("vsock.sock");
        fs::write(&sibling, b"keepme").unwrap();
        let stale = paths.runtime_dir.join("tpm.sock");
        fs::write(&stale, b"stale").unwrap();

        harden(&paths, &cfg).expect("harden ok");
        // Runtime dir mode unchanged.
        assert_eq!(mode_of(&paths.runtime_dir), 0o751);
        // Sibling untouched.
        assert!(sibling.exists());
        // Stale trusted socket removed.
        assert!(!stale.exists());
    }

    #[test]
    fn ancestor_traverse_acl_applied() {
        let s = Scratch::new("ancestoracl");
        let paths = s.paths("lambda");
        s.make_per_vm_root(&paths);
        let cfg = s.cfg();
        harden(&paths, &cfg).expect("harden ok");
        // The per-VM root carries a u:<uid>:--x ACL entry.
        let (access, _default) = path_safe::fd_extended_acl_present(
            path_safe::open_dir_path_safe(&paths.per_vm_root)
                .unwrap()
                .as_fd(),
        )
        .unwrap();
        assert!(
            access,
            "ancestor traverse ACL must be present on per-VM root"
        );
    }

    #[test]
    fn sticky_parent_blocks_non_owner_replacement_or_skips() {
        // The per-VM root is mode 3770 (setgid + sticky) on the base
        // branch so a NON-owner role uid cannot rename/replace the
        // swtpm dir entry. Verifying that requires a second uid we can
        // drop to; the broker test process runs as a single
        // unprivileged uid, so when we are not root (cannot setuid to a
        // foreign uid) this test SKIPS with a clear message rather than
        // silently passing.
        if !nix::unistd::geteuid().is_root() {
            eprintln!(
                "SKIP sticky_parent_blocks_non_owner_replacement_or_skips: \
                 multi-uid sandbox unavailable (need root to drop to a foreign uid). \
                 The identity-bound marker test \
                 (marker_present_empty_replacement_ino_mismatch_fails_closed) \
                 covers the detect-and-fail-closed path single-uid."
            );
            return;
        }
        // Root path: build a sticky 3770 scratch parent, provision the
        // swtpm dir as a non-zero principal uid, then attempt a
        // replacement as a DIFFERENT non-owner uid. The kernel sticky
        // semantics must deny the rename/unlink.
        let s = Scratch::new("sticky");
        let paths = s.paths("mu");
        fs::create_dir_all(&paths.per_vm_root).unwrap();
        fs::set_permissions(&paths.per_vm_root, fs::Permissions::from_mode(0o3770)).unwrap();
        let owner_uid: u32 = 65_531;
        let foreign_uid: u32 = 65_532;
        let cfg = SwtpmHardenConfig {
            expected_uid: owner_uid,
            expected_gid: cur_gid(),
            marker_owner_uid: 0,
            marker_owner_gid: 0,
            now_ms: 1_700_000_000_000,
            enforce_root_parents: false,
        };
        // chown the sticky parent so the principal can create within it.
        nix::unistd::chown(
            &paths.per_vm_root,
            Some(nix::unistd::Uid::from_raw(owner_uid)),
            Some(nix::unistd::Gid::from_raw(cur_gid())),
        )
        .unwrap();
        harden(&paths, &cfg).expect("root provision ok");
        // Attempt a non-owner unlink of the swtpm dir under the sticky
        // parent: kernel must refuse with EPERM/EACCES.
        let swtpm_dir = paths.swtpm_dir.clone();
        let res = std::thread::spawn(move || {
            nix::unistd::setgid(nix::unistd::Gid::from_raw(cur_gid())).ok();
            nix::unistd::setuid(nix::unistd::Uid::from_raw(foreign_uid)).unwrap();
            std::fs::remove_dir(&swtpm_dir)
        })
        .join()
        .unwrap();
        assert!(
            res.is_err(),
            "sticky parent must deny a non-owner uid removing the swtpm dir"
        );
    }

    #[test]
    fn derive_paths_picks_state_dir_not_runtime() {
        use nixling_core::minijail_profile::{CgroupPlacement, MountPolicy, WritablePath};
        let plan = SpawnRunnerPlan {
            binary_path: PathBuf::from("/run/current-system/sw/bin/swtpm"),
            argv: vec!["swtpm".into()],
            uid: 12345,
            gid: 12345,
            supplementary_groups: vec![],
            env: vec![],
            capabilities: vec![],
            namespaces: nixling_core::minijail_profile::NamespaceSet {
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
                        path: "/var/lib/nixling/vms/work/swtpm".into(),
                        purpose: "tpm nvram".into(),
                    },
                    WritablePath {
                        path: "/run/nixling/vms/work".into(),
                        purpose: "tpm socket".into(),
                    },
                ],
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_placement: CgroupPlacement {
                subtree: "nixling.slice/work/swtpm".into(),
                controllers: vec![],
                delegated: true,
            },
            user_namespace: None,
            umask: None,
        };
        let paths = derive_paths(&plan).expect("derive ok");
        assert_eq!(paths.vm_id, "work");
        assert_eq!(
            paths.swtpm_dir,
            PathBuf::from("/var/lib/nixling/vms/work/swtpm")
        );
        assert_eq!(
            paths.per_vm_root,
            PathBuf::from("/var/lib/nixling/vms/work")
        );
        assert_eq!(paths.runtime_dir, PathBuf::from("/run/nixling/vms/work"));
        assert_eq!(paths.marker_dir, PathBuf::from(MARKER_TREE));
        assert_eq!(paths.marker_name, "work");
    }
}
