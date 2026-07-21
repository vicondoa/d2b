//! Provision / rotate / rollback / retire engine for per-realm secrets
//! material (TPM-bound credentials, guest signing keys, and
//! security-key channel state — see [`SecretKind`]).
//!
//! This module is the storage/atomicity/rotation-state layer only. It
//! never generates raw secret material itself (that stays a caller
//! concern — e.g. calling `ssh-keygen` for a [`SecretKind::GuestSigningKey`],
//! or deriving a TPM attestation blob for a [`SecretKind::TpmBoundCredential`]),
//! and it never touches `swtpm_dir.rs`'s physical NVRAM state or
//! `security_key.rs`'s CTAPHID transport directly. It provides one
//! generic, fd-relative, fail-closed engine that every secret kind
//! shares for:
//!
//!   - laying out `<per_vm_state_root>/secrets/<kind-slug>/generations/<n>/material`
//!     plus a `current` symlink, atomically swapped with a
//!     rename-based two-step (mirrors the broker's other atomic-swap
//!     idioms, e.g. `d2b_host::hardlink_farm::swap_current_symlink`,
//!     but implemented directly against an fd-anchored `secrets_dir`
//!     rather than reusing that store-view-specific helper);
//!   - an identity-bound tamper-guard marker (dev/ino of the active
//!     generation directory) under a dedicated marker tree, kept
//!     independent of `swtpm_dir.rs`'s own per-VM marker so this
//!     component never touches that file;
//!   - fail-closed behaviour for every drift/tamper condition,
//!     mirroring the `swtpm_dir.rs` philosophy: a previously-active
//!     marker whose material vanished, or on-disk material with no
//!     matching active marker, both abort rather than silently adopt
//!     or silently reprovision.
//!
//! # Integration wiring points (deliberately NOT performed here)
//!
//! Per the W8 `secrets-lifecycle` component scope, this module is not
//! wired into any shared sink. The integrator still needs to:
//!
//!   1. Add `pub mod secrets_lifecycle;` and
//!      `pub mod secrets_rotation_audit;` to
//!      `packages/d2b-priv-broker/src/ops/mod.rs`.
//!   2. Add an `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
//!      variant (and a matching `from_operation_value` arm) to
//!      `packages/d2b-priv-broker/src/ops/audit_op.rs`.
//!   3. Add new wire request/response DTOs (in `d2b-contracts`) and a
//!      dispatch path in `packages/d2b-priv-broker/src/runtime.rs` that
//!      calls [`provision`]/[`rotate`]/[`rollback`]/[`retire`] and
//!      emits the returned [`SecretsLifecycleAuditFields`] through
//!      `crate::audit::AuditLog`. The plan text explicitly forbids
//!      adding a new broker op enum family from within this component,
//!      so this wiring is out of scope here.
//!   4. Decide, at the integration site, what bundle-resolved field
//!      supplies `per_vm_state_root` for [`derive_paths`] (mirrors how
//!      `swtpm_dir::derive_paths` takes a `&SpawnRunnerPlan`).
//!   5. Decide whether `SecretKind::GuestSigningKey` material comes
//!      from `exec_reconcile::run_ssh_keygen` output fed into
//!      [`rotate`]'s `material` parameter, or stays separate.
//!   6. Decide the exact coupling between `SecretKind::TpmBoundCredential`
//!      rotation and `swtpm_dir.rs`'s physical NVRAM (e.g. whether a
//!      rotate here should also trigger a swtpm reseal) — this is a
//!      product/security decision beyond this component's scope.
//!   7. Wire `packages/d2b-sk-frontend/src/secrets_channel.rs`'s
//!      `ChannelState` into `services/security_key/mod.rs`'s
//!      `SessionConfig` so a `SecretKind::SecurityKeyChannelState`
//!      rotation on the broker side can propagate a fresh
//!      `channel_binding`/`reconnect_generation` to the guest session
//!      (neither file is owned by this component).
//!   8. Run `gen-nix-unit-pins.sh` after this component lands so
//!      `tests/unit/nix/pinned/*.txt` picks up the new
//!      `w8-secrets-lifecycle-eval.nix` case names (not run here since
//!      the pinned files are not owned by this component).

use std::fmt;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use nix::libc;
use rustix::fs::OFlags;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::ops::hosts::stable_hash_str;
use crate::ops::secrets_rotation_audit::{
    LifecycleAction, MarkerResult, SecretKind, SecretsLifecycleAuditContext,
    SecretsLifecycleAuditFields,
};
use crate::sys::path_safe;

/// Root of the dedicated per-kind tamper-guard marker tree. Kept
/// distinct from `swtpm_dir.rs`'s own per-VM marker tree
/// (`/var/lib/d2b/swtpm-markers/<vm>`) so this component never reads
/// or writes that file.
pub const MARKER_TREE_ROOT: &str = "/var/lib/d2b/secrets-lifecycle-markers";

/// On-disk schema version for the marker JSON body.
const MARKER_SCHEMA_VERSION: u32 = 1;

const SECRETS_ROOT_DIR_MODE: u32 = 0o750;
const SECRETS_KIND_DIR_MODE: u32 = 0o700;
const GENERATION_DIR_MODE: u32 = 0o700;
const MATERIAL_FILE_MODE: u32 = 0o600;
const MARKER_DIR_MODE: u32 = 0o700;
const MARKER_FILE_MODE: u32 = 0o600;

/// Closed set of path-free, redaction-safe failure reason slugs. Every
/// public function in this module only ever surfaces one of these —
/// never a raw `io::Error` message, which could embed a path.
pub mod reasons {
    pub const INVALID_VM_ID: &str = "invalid-vm-id";
    pub const DERIVATION_FAILED: &str = "path-derivation-failed";
    pub const PARENT_OPEN_FAILED: &str = "secrets-dir-open-failed";
    pub const MARKER_TREE_FAILED: &str = "marker-tree-open-failed";
    pub const MARKER_WRITE_FAILED: &str = "marker-write-failed";
    pub const MARKER_TAMPERED: &str = "marker-tampered-or-missing-material";
    pub const ALREADY_PROVISIONED: &str = "already-provisioned";
    pub const ALREADY_RETIRED: &str = "already-retired";
    pub const NOT_PROVISIONED: &str = "not-provisioned";
    pub const NO_ROLLBACK_TARGET: &str = "no-rollback-target";
    pub const PREVIOUSLY_PROVISIONED_MATERIAL_MISSING: &str =
        "previously-provisioned-material-missing";
    pub const GENERATION_CONFLICT: &str = "generation-conflict";
    pub const MATERIAL_WRITE_FAILED: &str = "material-write-failed";
    pub const CURRENT_SWAP_FAILED: &str = "current-swap-failed";
    pub const INVALID_MATERIAL: &str = "invalid-material";
}

/// Runtime configuration for a single lifecycle call. Kept separate
/// from [`SecretsLifecyclePaths`] so tests can point ownership/enforcement
/// at scratch-safe values without touching the derived path shape.
#[derive(Debug, Clone, Copy)]
pub struct SecretsLifecycleConfig {
    /// Expected owner of the secrets-state tree (typically the
    /// broker's own uid/gid; per-kind consumers never get direct
    /// filesystem access).
    pub expected_uid: u32,
    pub expected_gid: u32,
    /// Owner of the tamper-guard marker file/dir. Usually identical to
    /// `expected_uid`/`expected_gid`, but kept distinct in case a
    /// future policy roots the marker tree under a narrower principal.
    pub marker_owner_uid: u32,
    pub marker_owner_gid: u32,
    /// Monotonic wall-clock milliseconds used to stamp the marker.
    /// Caller-supplied (not read from the system clock in this
    /// module) so tests are deterministic.
    pub now_ms: u64,
    /// When `true`, refuse a world-writable parent directory on
    /// production-looking paths (`/var/lib/d2b/...`). Tests targeting
    /// a scratch tempdir set this to `false`.
    pub enforce_root_parents: bool,
}

/// Derived, path-bearing (but never logged) locations for one
/// `(vm, kind)` pair.
#[derive(Debug, Clone)]
pub struct SecretsLifecyclePaths {
    pub vm_id: String,
    pub kind: SecretKind,
    /// `<per_vm_state_root>/secrets/<kind-slug>`
    pub secrets_dir: PathBuf,
    /// `<MARKER_TREE_ROOT>/<vm_id>`
    pub marker_dir: PathBuf,
}

fn valid_vm_id(vm_id: &str) -> bool {
    let mut chars = vm_id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Derive the paths this component owns for `(vm_id, kind)` beneath
/// `per_vm_state_root`. `per_vm_state_root` is expected to already
/// exist (established by the caller's own per-VM state-dir prepare
/// step); this function does not create it.
pub fn derive_paths(
    per_vm_state_root: &Path,
    vm_id: &str,
    kind: SecretKind,
) -> Result<SecretsLifecyclePaths, &'static str> {
    if !valid_vm_id(vm_id) {
        return Err(reasons::INVALID_VM_ID);
    }
    if !per_vm_state_root.is_absolute() {
        return Err(reasons::DERIVATION_FAILED);
    }
    Ok(SecretsLifecyclePaths {
        vm_id: vm_id.to_owned(),
        kind,
        secrets_dir: per_vm_state_root.join("secrets").join(kind.as_slug()),
        marker_dir: PathBuf::from(MARKER_TREE_ROOT).join(vm_id),
    })
}

/// Zeroizing, digest-only-Debug wrapper around raw secret material
/// bytes. Never implements a byte-exposing `Display`, and `Debug`
/// never prints the bytes or their length-derived shape beyond a
/// fixed placeholder.
pub struct SecretMaterial(Zeroizing<Vec<u8>>);

impl SecretMaterial {
    /// Generous but bounded ceiling so a single material blob can
    /// never grow unbounded (a 1 MiB budget comfortably covers a TPM
    /// attestation blob, an SSH host key bundle, or channel-binding
    /// wire material with headroom).
    pub const MAX_LEN: usize = 1 << 20;

    pub fn new(bytes: Vec<u8>) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > Self::MAX_LEN {
            return Err(reasons::INVALID_MATERIAL);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// SHA-256 hex digest of the material. Safe to audit/log — the
    /// raw bytes never are.
    pub fn digest_hex(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.as_bytes());
        let digest = hasher.finalize();
        let mut out = String::with_capacity(64);
        for byte in digest {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretMaterial")
            .field(&"<redacted>")
            .finish()
    }
}

/// Path-free, material-free error returned by every public function
/// in this module. Carries the fully-formed audit record so a caller
/// need not reconstruct it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsLifecycleError {
    pub reason: &'static str,
    pub audit: SecretsLifecycleAuditFields,
}

impl fmt::Display for SecretsLifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "secrets-lifecycle {:?} failed: {}",
            self.audit.action, self.reason
        )
    }
}

impl std::error::Error for SecretsLifecycleError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MarkerData {
    v: u32,
    vm: String,
    kind: String,
    retired: bool,
    generation: Option<u32>,
    previous_generation: Option<u32>,
    active_dev: Option<u64>,
    active_ino: Option<u64>,
    first_provisioned_ms: u64,
    updated_ms: u64,
}

fn io_from_rustix(err: rustix::io::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(err.raw_os_error())
}

fn production_path(path: &Path) -> bool {
    path.starts_with("/var/lib/d2b")
}

fn context(paths: &SecretsLifecyclePaths, action: LifecycleAction) -> SecretsLifecycleAuditContext {
    SecretsLifecycleAuditContext {
        vm_id: paths.vm_id.clone(),
        kind: paths.kind,
        action,
        base_dir_hash: stable_hash_str(&paths.secrets_dir.display().to_string()),
    }
}

fn ensure_marker_tree(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
) -> Result<OwnedFd, &'static str> {
    if cfg.enforce_root_parents && production_path(&paths.marker_dir) {
        path_safe::refuse_world_writable_parent(&paths.marker_dir)
            .map_err(|_| reasons::MARKER_TREE_FAILED)?;
    }
    // `paths.marker_dir` is `<marker_tree_root>/<vm_id>`. Unlike the
    // per-VM secrets root (established by an earlier, out-of-scope
    // state-dir prepare step), the marker tree root itself is a
    // fresh top-level directory this module owns end-to-end, so it
    // is not safe to assume it already exists. Create it first (a
    // no-op once any VM has provisioned once) before creating the
    // per-VM leaf beneath it.
    let marker_root = paths
        .marker_dir
        .parent()
        .ok_or(reasons::MARKER_TREE_FAILED)?;
    path_safe::ensure_dir(
        marker_root,
        MARKER_DIR_MODE,
        Some(cfg.marker_owner_uid),
        Some(cfg.marker_owner_gid),
    )
    .map_err(|_| reasons::MARKER_TREE_FAILED)?;
    path_safe::ensure_dir(
        &paths.marker_dir,
        MARKER_DIR_MODE,
        Some(cfg.marker_owner_uid),
        Some(cfg.marker_owner_gid),
    )
    .map_err(|_| reasons::MARKER_TREE_FAILED)?;
    path_safe::open_dir_path_safe(&paths.marker_dir).map_err(|_| reasons::MARKER_TREE_FAILED)
}

fn read_marker(
    marker_dir_fd: &OwnedFd,
    paths: &SecretsLifecyclePaths,
) -> Result<Option<MarkerData>, &'static str> {
    let name = paths.kind.as_slug();
    let stat = match path_safe::fstatat_nofollow(marker_dir_fd, name) {
        Ok(Some(stat)) => stat,
        Ok(None) => return Ok(None),
        Err(_) => return Err(reasons::MARKER_TAMPERED),
    };
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(reasons::MARKER_TAMPERED);
    }
    let fd = path_safe::open_file_at_safe(marker_dir_fd, name, libc::O_RDONLY)
        .map_err(|_| reasons::MARKER_TAMPERED)?;
    let mut file = std::fs::File::from(fd);
    let mut buf = String::new();
    {
        use std::io::Read;
        file.read_to_string(&mut buf)
            .map_err(|_| reasons::MARKER_TAMPERED)?;
    }
    let data: MarkerData = serde_json::from_str(&buf).map_err(|_| reasons::MARKER_TAMPERED)?;
    if data.v != MARKER_SCHEMA_VERSION || data.vm != paths.vm_id || data.kind != name {
        return Err(reasons::MARKER_TAMPERED);
    }
    Ok(Some(data))
}

fn write_marker(
    marker_dir_fd: &OwnedFd,
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
    data: &MarkerData,
) -> Result<(), &'static str> {
    let body = serde_json::to_vec(data).map_err(|_| reasons::MARKER_WRITE_FAILED)?;
    path_safe::atomic_replace_fd_with_owner(
        marker_dir_fd,
        paths.kind.as_slug(),
        &body,
        MARKER_FILE_MODE,
        Some(cfg.marker_owner_uid),
        Some(cfg.marker_owner_gid),
    )
    .map_err(|_| reasons::MARKER_WRITE_FAILED)
}

fn open_or_create_secrets_dir(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
) -> Result<OwnedFd, &'static str> {
    let secrets_root = paths
        .secrets_dir
        .parent()
        .ok_or(reasons::DERIVATION_FAILED)?;
    path_safe::ensure_dir(
        secrets_root,
        SECRETS_ROOT_DIR_MODE,
        Some(cfg.expected_uid),
        Some(cfg.expected_gid),
    )
    .map_err(|_| reasons::PARENT_OPEN_FAILED)?;
    path_safe::ensure_dir(
        &paths.secrets_dir,
        SECRETS_KIND_DIR_MODE,
        Some(cfg.expected_uid),
        Some(cfg.expected_gid),
    )
    .map_err(|_| reasons::PARENT_OPEN_FAILED)?;
    path_safe::ensure_dir(
        &paths.secrets_dir.join("generations"),
        SECRETS_KIND_DIR_MODE,
        Some(cfg.expected_uid),
        Some(cfg.expected_gid),
    )
    .map_err(|_| reasons::PARENT_OPEN_FAILED)?;
    path_safe::open_dir_path_safe(&paths.secrets_dir).map_err(|_| reasons::PARENT_OPEN_FAILED)
}

fn open_generations_dir(secrets_dir_fd: &OwnedFd) -> Result<OwnedFd, &'static str> {
    path_safe::open_at(
        secrets_dir_fd.as_fd(),
        Path::new("generations"),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| reasons::PARENT_OPEN_FAILED)
}

fn write_generation(
    secrets_dir_fd: &OwnedFd,
    generation: u32,
    material: &SecretMaterial,
    cfg: &SecretsLifecycleConfig,
) -> Result<libc::stat, &'static str> {
    let generations_fd = open_generations_dir(secrets_dir_fd)?;
    let name = generation.to_string();
    path_safe::mkdir_at_exclusive(
        generations_fd.as_fd(),
        Path::new(&name),
        GENERATION_DIR_MODE,
    )
    .map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            reasons::GENERATION_CONFLICT
        } else {
            reasons::MATERIAL_WRITE_FAILED
        }
    })?;
    let generation_fd = path_safe::open_at(
        generations_fd.as_fd(),
        Path::new(&name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| reasons::MATERIAL_WRITE_FAILED)?;
    path_safe::fchmod(generation_fd.as_fd(), GENERATION_DIR_MODE)
        .map_err(|_| reasons::MATERIAL_WRITE_FAILED)?;
    path_safe::fchown(
        generation_fd.as_fd(),
        Some(cfg.expected_uid),
        Some(cfg.expected_gid),
    )
    .map_err(|_| reasons::MATERIAL_WRITE_FAILED)?;

    let file_fd = path_safe::create_file_at_safe(
        &generation_fd,
        "material",
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        MATERIAL_FILE_MODE,
    )
    .map_err(|_| reasons::MATERIAL_WRITE_FAILED)?;
    {
        use std::io::Write;
        let mut file = std::fs::File::from(file_fd);
        file.write_all(material.as_bytes())
            .map_err(|_| reasons::MATERIAL_WRITE_FAILED)?;
        file.sync_all()
            .map_err(|_| reasons::MATERIAL_WRITE_FAILED)?;
    }
    path_safe::fstat_fd(generation_fd.as_fd()).map_err(|_| reasons::MATERIAL_WRITE_FAILED)
}

fn stat_generation(secrets_dir_fd: &OwnedFd, generation: u32) -> Result<libc::stat, &'static str> {
    let generations_fd = open_generations_dir(secrets_dir_fd)?;
    let name = generation.to_string();
    let fd = path_safe::open_at(
        generations_fd.as_fd(),
        Path::new(&name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| reasons::PREVIOUSLY_PROVISIONED_MATERIAL_MISSING)?;
    path_safe::fstat_fd(fd.as_fd()).map_err(|_| reasons::PREVIOUSLY_PROVISIONED_MATERIAL_MISSING)
}

/// Best-effort removal of one generation directory's `material` file
/// and the directory itself. Never fails the caller: a stale
/// generation left behind after a crash is picked up by the next
/// rotate/retire's own retention bookkeeping, never silently trusted
/// as current.
fn remove_generation(secrets_dir_fd: &OwnedFd, generation: u32) {
    let Ok(generations_fd) = open_generations_dir(secrets_dir_fd) else {
        return;
    };
    let name = generation.to_string();
    if let Ok(generation_fd) = path_safe::open_at(
        generations_fd.as_fd(),
        Path::new(&name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    ) {
        let _ = path_safe::remove_path_safe(&generation_fd, "material");
    }
    let _ = path_safe::remove_path_safe(&generations_fd, &name);
}

/// Best-effort removal of the `current` symlink itself. `current` is
/// always a symlink by construction (see [`atomic_swap_current`]), so
/// unlike [`path_safe::remove_path_safe`] (which deliberately refuses
/// to operate on a symlink as a generic anti-planting guard for
/// contexts where a symlink would be unexpected) this removes the
/// link entry directly without following it. Never fails the caller.
fn remove_current_symlink(secrets_dir_fd: &OwnedFd) {
    let _ = rustix::fs::unlinkat(
        secrets_dir_fd.as_fd(),
        "current",
        rustix::fs::AtFlags::empty(),
    );
}

/// Atomically point the `current` symlink at `generations/<generation>`
/// using a hidden-name symlink-then-rename swap against the already
/// fd-anchored `secrets_dir`, so no window ever exposes a half-written
/// or dangling `current`.
fn atomic_swap_current(
    secrets_dir_fd: BorrowedFd<'_>,
    generation: u32,
) -> Result<(), &'static str> {
    let target = format!("generations/{generation}");
    let tmp_name = format!(".current.tmp.{}", std::process::id());
    let _ = rustix::fs::unlinkat(
        secrets_dir_fd,
        tmp_name.as_str(),
        rustix::fs::AtFlags::empty(),
    );
    rustix::fs::symlinkat(target.as_str(), secrets_dir_fd, tmp_name.as_str())
        .map_err(io_from_rustix)
        .map_err(|_| reasons::CURRENT_SWAP_FAILED)?;
    match rustix::fs::renameat(secrets_dir_fd, tmp_name.as_str(), secrets_dir_fd, "current") {
        Ok(()) => {
            let _ = rustix::fs::fsync(secrets_dir_fd);
            Ok(())
        }
        Err(_) => {
            let _ = rustix::fs::unlinkat(
                secrets_dir_fd,
                tmp_name.as_str(),
                rustix::fs::AtFlags::empty(),
            );
            Err(reasons::CURRENT_SWAP_FAILED)
        }
    }
}

fn current_present(secrets_dir_fd: &OwnedFd) -> Result<bool, &'static str> {
    path_safe::fstatat_nofollow(secrets_dir_fd, "current")
        .map(|stat| stat.is_some())
        .map_err(|_| reasons::MARKER_TAMPERED)
}

fn verify_active_identity(
    secrets_dir_fd: &OwnedFd,
    marker: &MarkerData,
) -> Result<(), &'static str> {
    let generation = marker.generation.ok_or(reasons::MARKER_TAMPERED)?;
    let (expected_dev, expected_ino) = match (marker.active_dev, marker.active_ino) {
        (Some(dev), Some(ino)) => (dev, ino),
        _ => return Err(reasons::MARKER_TAMPERED),
    };
    let stat = stat_generation(secrets_dir_fd, generation)?;
    if stat.st_dev != expected_dev || stat.st_ino != expected_ino {
        return Err(reasons::MARKER_TAMPERED);
    }
    Ok(())
}

/// Provision fresh (generation 1) material for `(vm, kind)`.
///
/// Fails closed with [`reasons::ALREADY_PROVISIONED`] if active
/// material already exists (callers should use [`rotate`] instead).
/// Fails closed with [`reasons::PREVIOUSLY_PROVISIONED_MATERIAL_MISSING`]
/// if the marker records active material but the generation vanished.
/// Fails closed with [`reasons::MARKER_TAMPERED`] if material exists on
/// disk with no matching active marker (never silently adopted).
/// Succeeds (fresh generation 1) when never provisioned, or when the
/// prior provisioning was cleanly retired.
pub fn provision(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
    material: &SecretMaterial,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(paths, LifecycleAction::Provision);
    let marker_dir_fd = ensure_marker_tree(paths, cfg).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let existing = read_marker(&marker_dir_fd, paths).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let secrets_dir_fd =
        open_or_create_secrets_dir(paths, cfg).map_err(|reason| SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        })?;
    let has_current = current_present(&secrets_dir_fd).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let marker_active = existing.as_ref().map(|m| !m.retired).unwrap_or(false);

    match (marker_active, has_current) {
        (true, true) => {
            return Err(SecretsLifecycleError {
                reason: reasons::ALREADY_PROVISIONED,
                audit: SecretsLifecycleAuditFields::denied(&ctx, reasons::ALREADY_PROVISIONED),
            });
        }
        (true, false) => {
            return Err(SecretsLifecycleError {
                reason: reasons::PREVIOUSLY_PROVISIONED_MATERIAL_MISSING,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reasons::PREVIOUSLY_PROVISIONED_MATERIAL_MISSING,
                ),
            });
        }
        (false, true) => {
            // Material present with no active marker backing it: a
            // planted or drifted directory. Never silently adopted.
            return Err(SecretsLifecycleError {
                reason: reasons::MARKER_TAMPERED,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reasons::MARKER_TAMPERED,
                ),
            });
        }
        (false, false) => {}
    }

    let stat = write_generation(&secrets_dir_fd, 1, material, cfg).map_err(|reason| {
        SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        }
    })?;
    atomic_swap_current(secrets_dir_fd.as_fd(), 1).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;

    let marker = MarkerData {
        v: MARKER_SCHEMA_VERSION,
        vm: paths.vm_id.clone(),
        kind: paths.kind.as_slug().to_owned(),
        retired: false,
        generation: Some(1),
        previous_generation: None,
        active_dev: Some(stat.st_dev),
        active_ino: Some(stat.st_ino),
        first_provisioned_ms: cfg.now_ms,
        updated_ms: cfg.now_ms,
    };
    write_marker(&marker_dir_fd, paths, cfg, &marker).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;

    Ok(SecretsLifecycleAuditFields::provisioned(
        &ctx,
        material.digest_hex(),
    ))
}

/// Rotate `(vm, kind)` to a new generation, retaining exactly the
/// immediately-prior generation for [`rollback`].
///
/// Requires an active (non-retired) marker; verifies the marker's
/// recorded active generation identity against the live directory
/// before mutating, so a TOCTOU swap of the active generation's
/// content is caught rather than silently rotated forward from an
/// attacker-controlled base.
pub fn rotate(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
    material: &SecretMaterial,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(paths, LifecycleAction::Rotate);
    let marker_dir_fd = ensure_marker_tree(paths, cfg).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let marker = match read_marker(&marker_dir_fd, paths) {
        Ok(Some(marker)) if !marker.retired => marker,
        Ok(Some(_)) => {
            return Err(SecretsLifecycleError {
                reason: reasons::ALREADY_RETIRED,
                audit: SecretsLifecycleAuditFields::denied(&ctx, reasons::ALREADY_RETIRED),
            });
        }
        Ok(None) => {
            return Err(SecretsLifecycleError {
                reason: reasons::NOT_PROVISIONED,
                audit: SecretsLifecycleAuditFields::denied(&ctx, reasons::NOT_PROVISIONED),
            });
        }
        Err(reason) => {
            return Err(SecretsLifecycleError {
                reason,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reason,
                ),
            });
        }
    };

    let secrets_dir_fd =
        open_or_create_secrets_dir(paths, cfg).map_err(|reason| SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        })?;
    verify_active_identity(&secrets_dir_fd, &marker).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;

    let current_generation = marker.generation.ok_or_else(|| SecretsLifecycleError {
        reason: reasons::MARKER_TAMPERED,
        audit: SecretsLifecycleAuditFields::failed(
            &ctx,
            MarkerResult::FailedClosed,
            reasons::MARKER_TAMPERED,
        ),
    })?;
    let new_generation =
        current_generation
            .checked_add(1)
            .ok_or_else(|| SecretsLifecycleError {
                reason: reasons::GENERATION_CONFLICT,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reasons::GENERATION_CONFLICT,
                ),
            })?;

    let stat =
        write_generation(&secrets_dir_fd, new_generation, material, cfg).map_err(|reason| {
            SecretsLifecycleError {
                reason,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reason,
                ),
            }
        })?;
    atomic_swap_current(secrets_dir_fd.as_fd(), new_generation).map_err(|reason| {
        SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        }
    })?;

    // Retention policy: keep exactly the generation we just rotated
    // away from; prune anything older. Best-effort — a prune failure
    // never rolls back the already-committed rotate.
    if let Some(stale) = marker.previous_generation {
        remove_generation(&secrets_dir_fd, stale);
    }

    let updated_marker = MarkerData {
        v: MARKER_SCHEMA_VERSION,
        vm: paths.vm_id.clone(),
        kind: paths.kind.as_slug().to_owned(),
        retired: false,
        generation: Some(new_generation),
        previous_generation: Some(current_generation),
        active_dev: Some(stat.st_dev),
        active_ino: Some(stat.st_ino),
        first_provisioned_ms: marker.first_provisioned_ms,
        updated_ms: cfg.now_ms,
    };
    write_marker(&marker_dir_fd, paths, cfg, &updated_marker).map_err(|reason| {
        SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        }
    })?;

    Ok(SecretsLifecycleAuditFields::rotated(
        &ctx,
        new_generation,
        vec![current_generation],
        material.digest_hex(),
    ))
}

/// Swap `current` back to the immediately-prior generation recorded
/// by the marker. Fails closed with [`reasons::NO_ROLLBACK_TARGET`] if
/// no previous generation is tracked (either never rotated, or the
/// retained generation was already pruned by a later action).
pub fn rollback(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(paths, LifecycleAction::Rollback);
    let marker_dir_fd = ensure_marker_tree(paths, cfg).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let marker = match read_marker(&marker_dir_fd, paths) {
        Ok(Some(marker)) if !marker.retired => marker,
        Ok(Some(_)) => {
            return Err(SecretsLifecycleError {
                reason: reasons::ALREADY_RETIRED,
                audit: SecretsLifecycleAuditFields::denied(&ctx, reasons::ALREADY_RETIRED),
            });
        }
        Ok(None) => {
            return Err(SecretsLifecycleError {
                reason: reasons::NOT_PROVISIONED,
                audit: SecretsLifecycleAuditFields::denied(&ctx, reasons::NOT_PROVISIONED),
            });
        }
        Err(reason) => {
            return Err(SecretsLifecycleError {
                reason,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reason,
                ),
            });
        }
    };
    let Some(previous_generation) = marker.previous_generation else {
        return Err(SecretsLifecycleError {
            reason: reasons::NO_ROLLBACK_TARGET,
            audit: SecretsLifecycleAuditFields::denied(&ctx, reasons::NO_ROLLBACK_TARGET),
        });
    };

    let secrets_dir_fd =
        open_or_create_secrets_dir(paths, cfg).map_err(|reason| SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        })?;
    verify_active_identity(&secrets_dir_fd, &marker).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let previous_stat =
        stat_generation(&secrets_dir_fd, previous_generation).map_err(|reason| {
            SecretsLifecycleError {
                reason,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reason,
                ),
            }
        })?;

    let current_generation = marker.generation.ok_or_else(|| SecretsLifecycleError {
        reason: reasons::MARKER_TAMPERED,
        audit: SecretsLifecycleAuditFields::failed(
            &ctx,
            MarkerResult::FailedClosed,
            reasons::MARKER_TAMPERED,
        ),
    })?;

    atomic_swap_current(secrets_dir_fd.as_fd(), previous_generation).map_err(|reason| {
        SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        }
    })?;

    let updated_marker = MarkerData {
        v: MARKER_SCHEMA_VERSION,
        vm: paths.vm_id.clone(),
        kind: paths.kind.as_slug().to_owned(),
        retired: false,
        generation: Some(previous_generation),
        previous_generation: Some(current_generation),
        active_dev: Some(previous_stat.st_dev),
        active_ino: Some(previous_stat.st_ino),
        first_provisioned_ms: marker.first_provisioned_ms,
        updated_ms: cfg.now_ms,
    };
    write_marker(&marker_dir_fd, paths, cfg, &updated_marker).map_err(|reason| {
        SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        }
    })?;

    Ok(SecretsLifecycleAuditFields::rolled_back(
        &ctx,
        previous_generation,
        vec![current_generation],
    ))
}

/// Retire `(vm, kind)`: remove all tracked generations, the `current`
/// symlink, and tombstone the marker. Idempotent — retiring an
/// already-retired or never-provisioned kind returns
/// [`crate::ops::secrets_rotation_audit::LifecycleResult::VerifiedClean`]
/// rather than an error.
///
/// Fails closed (rather than proceeding to tombstone) if the marker
/// records active material that has already vanished from disk — an
/// operator must investigate the drift before this component treats
/// the kind as cleanly retired.
pub fn retire(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(paths, LifecycleAction::Retire);
    let marker_dir_fd = ensure_marker_tree(paths, cfg).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;
    let marker = match read_marker(&marker_dir_fd, paths) {
        Ok(None) => return Ok(SecretsLifecycleAuditFields::verified_clean(&ctx, None)),
        Ok(Some(marker)) if marker.retired => {
            return Ok(SecretsLifecycleAuditFields::verified_clean(&ctx, None));
        }
        Ok(Some(marker)) => marker,
        Err(reason) => {
            return Err(SecretsLifecycleError {
                reason,
                audit: SecretsLifecycleAuditFields::failed(
                    &ctx,
                    MarkerResult::FailedClosed,
                    reason,
                ),
            });
        }
    };

    let secrets_dir_fd =
        open_or_create_secrets_dir(paths, cfg).map_err(|reason| SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        })?;
    verify_active_identity(&secrets_dir_fd, &marker).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
    })?;

    remove_current_symlink(&secrets_dir_fd);
    if let Some(generation) = marker.generation {
        remove_generation(&secrets_dir_fd, generation);
    }
    if let Some(previous) = marker.previous_generation {
        remove_generation(&secrets_dir_fd, previous);
    }

    let tombstoned_marker = MarkerData {
        v: MARKER_SCHEMA_VERSION,
        vm: paths.vm_id.clone(),
        kind: paths.kind.as_slug().to_owned(),
        retired: true,
        generation: None,
        previous_generation: None,
        active_dev: None,
        active_ino: None,
        first_provisioned_ms: marker.first_provisioned_ms,
        updated_ms: cfg.now_ms,
    };
    write_marker(&marker_dir_fd, paths, cfg, &tombstoned_marker).map_err(|reason| {
        SecretsLifecycleError {
            reason,
            audit: SecretsLifecycleAuditFields::failed(&ctx, MarkerResult::FailedClosed, reason),
        }
    })?;

    Ok(SecretsLifecycleAuditFields::retired(&ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::secrets_rotation_audit::LifecycleResult;

    fn test_cfg(now_ms: u64) -> SecretsLifecycleConfig {
        let uid = nix::unistd::getuid().as_raw();
        let gid = nix::unistd::getgid().as_raw();
        SecretsLifecycleConfig {
            expected_uid: uid,
            expected_gid: gid,
            marker_owner_uid: uid,
            marker_owner_gid: gid,
            now_ms,
            enforce_root_parents: false,
        }
    }

    /// Build test paths that mirror `derive_paths`'s real contract:
    /// `secrets_dir` sits beneath a `per_vm_state_root` that the
    /// (out-of-scope) state-dir prepare step is assumed to have
    /// already created, so this helper creates that stand-in root
    /// itself. `marker_dir`'s parent (the marker tree root) is
    /// deliberately left uncreated so tests exercise this module's
    /// own first-use creation of it.
    fn test_paths(root: &Path, vm: &str, kind: SecretKind) -> SecretsLifecyclePaths {
        let per_vm_state_root = root.join("state").join(vm);
        std::fs::create_dir_all(&per_vm_state_root).expect("stand-in per-vm root");
        SecretsLifecyclePaths {
            vm_id: vm.to_owned(),
            kind,
            secrets_dir: per_vm_state_root.join("secrets").join(kind.as_slug()),
            marker_dir: root.join("markers").join(vm),
        }
    }

    fn material(byte: u8) -> SecretMaterial {
        SecretMaterial::new(vec![byte; 32]).expect("valid material")
    }

    #[test]
    fn valid_vm_id_accepts_and_rejects_expected_shapes() {
        assert!(valid_vm_id("work"));
        assert!(valid_vm_id("corp-vm-1"));
        assert!(!valid_vm_id(""));
        assert!(!valid_vm_id("Work"));
        assert!(!valid_vm_id("1work"));
        assert!(!valid_vm_id("work/etc"));
    }

    #[test]
    fn derive_paths_rejects_invalid_vm_id_and_relative_root() {
        assert!(
            derive_paths(
                Path::new("/var/lib/d2b/vms/work"),
                "Bad",
                SecretKind::GuestSigningKey
            )
            .is_err()
        );
        assert!(derive_paths(Path::new("relative"), "work", SecretKind::GuestSigningKey).is_err());
        let paths = derive_paths(
            Path::new("/var/lib/d2b/vms/work"),
            "work",
            SecretKind::GuestSigningKey,
        )
        .expect("valid derivation");
        assert_eq!(
            paths.secrets_dir,
            Path::new("/var/lib/d2b/vms/work/secrets/guest-signing-key")
        );
        assert_eq!(paths.marker_dir, Path::new(MARKER_TREE_ROOT).join("work"));
    }

    #[test]
    fn secret_material_rejects_empty_and_oversized() {
        assert!(SecretMaterial::new(Vec::new()).is_err());
        assert!(SecretMaterial::new(vec![0u8; SecretMaterial::MAX_LEN + 1]).is_err());
        assert!(SecretMaterial::new(vec![1u8; 4]).is_ok());
    }

    #[test]
    fn secret_material_debug_never_prints_bytes() {
        let m = material(0xAB);
        let debug = format!("{m:?}");
        assert!(!debug.contains("171"));
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn provision_then_rotate_then_rollback_then_retire_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);

        let provisioned = provision(&paths, &cfg, &material(1)).expect("provision succeeds");
        assert_eq!(provisioned.result, LifecycleResult::Created);
        assert_eq!(provisioned.generation, Some(1));

        let rotated = rotate(&paths, &test_cfg(2_000), &material(2)).expect("rotate succeeds");
        assert_eq!(rotated.result, LifecycleResult::Rotated);
        assert_eq!(rotated.generation, Some(2));
        assert_eq!(rotated.retained_generations, vec![1]);

        let rolled_back = rollback(&paths, &test_cfg(3_000)).expect("rollback succeeds");
        assert_eq!(rolled_back.result, LifecycleResult::RolledBack);
        assert_eq!(rolled_back.generation, Some(1));
        assert_eq!(rolled_back.retained_generations, vec![2]);

        let retired = retire(&paths, &test_cfg(4_000)).expect("retire succeeds");
        assert_eq!(retired.result, LifecycleResult::Retired);

        // Idempotent retire.
        let retired_again = retire(&paths, &test_cfg(5_000)).expect("retire is idempotent");
        assert_eq!(retired_again.result, LifecycleResult::VerifiedClean);
    }

    #[test]
    fn provision_twice_without_retire_is_denied() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::TpmBoundCredential);
        let cfg = test_cfg(1_000);
        provision(&paths, &cfg, &material(1)).expect("first provision succeeds");
        let err = provision(&paths, &cfg, &material(2)).expect_err("second provision denied");
        assert_eq!(err.reason, reasons::ALREADY_PROVISIONED);
        assert_eq!(err.audit.result, LifecycleResult::Denied);
    }

    #[test]
    fn rotate_without_provision_is_denied() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::SecurityKeyChannelState);
        let cfg = test_cfg(1_000);
        let err = rotate(&paths, &cfg, &material(1)).expect_err("rotate without provision denied");
        assert_eq!(err.reason, reasons::NOT_PROVISIONED);
    }

    #[test]
    fn rollback_without_prior_rotate_has_no_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);
        provision(&paths, &cfg, &material(1)).expect("provision succeeds");
        let err = rollback(&paths, &test_cfg(2_000)).expect_err("no rollback target");
        assert_eq!(err.reason, reasons::NO_ROLLBACK_TARGET);
    }

    #[test]
    fn provision_after_clean_retire_succeeds_at_generation_one() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);
        provision(&paths, &cfg, &material(1)).expect("provision succeeds");
        retire(&paths, &test_cfg(2_000)).expect("retire succeeds");
        let reprovisioned =
            provision(&paths, &test_cfg(3_000), &material(3)).expect("re-provision succeeds");
        assert_eq!(reprovisioned.generation, Some(1));
    }

    #[test]
    fn provision_fails_closed_when_marker_missing_but_current_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);
        // Simulate drift: the secrets tree exists (with a `current`
        // symlink and a generation) but the marker was never written
        // (or was removed out of band).
        let secrets_dir_fd = open_or_create_secrets_dir(&paths, &cfg).expect("dir created");
        write_generation(&secrets_dir_fd, 1, &material(9), &cfg).expect("material written");
        atomic_swap_current(secrets_dir_fd.as_fd(), 1).expect("swap succeeds");

        let err = provision(&paths, &cfg, &material(1)).expect_err("must fail closed");
        assert_eq!(err.reason, reasons::MARKER_TAMPERED);
    }

    #[test]
    fn provision_fails_closed_when_marker_active_but_material_vanished() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);
        provision(&paths, &cfg, &material(1)).expect("provision succeeds");

        // Simulate an out-of-band deletion of the material tree while
        // the marker still claims it is active. `current` is a
        // symlink by construction, so simulate a manual/out-of-band
        // unlink directly rather than through the safe wrapper (which
        // deliberately refuses to touch symlinks elsewhere).
        let secrets_dir_fd = open_or_create_secrets_dir(&paths, &cfg).expect("dir reopened");
        remove_current_symlink(&secrets_dir_fd);
        remove_generation(&secrets_dir_fd, 1);

        let err = provision(&paths, &cfg, &material(2)).expect_err("must fail closed");
        assert_eq!(err.reason, reasons::PREVIOUSLY_PROVISIONED_MATERIAL_MISSING);
    }

    #[test]
    fn rotate_fails_closed_when_active_generation_content_swapped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);
        provision(&paths, &cfg, &material(1)).expect("provision succeeds");

        // Simulate a TOCTOU swap by directly corrupting the marker's
        // recorded identity rather than deleting/recreating the
        // generation directory: on some filesystems a freed inode
        // number can be reused immediately, which would make a
        // delete-then-recreate swap non-deterministic to detect here
        // even though the same dev/ino check still fails closed in
        // practice against a genuinely different backing inode.
        let marker_dir_fd = ensure_marker_tree(&paths, &cfg).expect("marker tree");
        let mut marker = read_marker(&marker_dir_fd, &paths)
            .expect("marker read")
            .expect("marker present");
        marker.active_ino = Some(
            marker
                .active_ino
                .expect("active ino recorded")
                .wrapping_add(1),
        );
        write_marker(&marker_dir_fd, &paths, &cfg, &marker).expect("marker corrupted");

        let err = rotate(&paths, &test_cfg(2_000), &material(2)).expect_err("must fail closed");
        assert_eq!(err.reason, reasons::MARKER_TAMPERED);
    }

    #[test]
    fn multiple_kinds_for_same_vm_are_independent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tpm_paths = test_paths(tmp.path(), "work", SecretKind::TpmBoundCredential);
        let signing_paths = test_paths(tmp.path(), "work", SecretKind::GuestSigningKey);
        let cfg = test_cfg(1_000);

        provision(&tpm_paths, &cfg, &material(1)).expect("tpm provision succeeds");
        // The other kind for the same VM must still report not-provisioned.
        let err =
            rotate(&signing_paths, &cfg, &material(2)).expect_err("independent kind is untouched");
        assert_eq!(err.reason, reasons::NOT_PROVISIONED);
    }
}
