//! Per-realm secrets lifecycle engine: provision / rotate / rollback /
//! retire for TPM-bound credentials, guest signing keys, and
//! security-key channel state, anchored under a per-`(vm, kind)`
//! directory via fd-relative, `openat2(RESOLVE_BENEATH)`-safe
//! primitives from [`crate::sys::path_safe`].
//!
//! # Design summary (W8fu1 redesign)
//!
//! This is a from-scratch redesign of the original `36d9dcf8` engine,
//! addressing an external review that found the first draft's marker
//! identity, retirement enumeration, and generation numbering were not
//! crash-safe. The core structural change is a **durable
//! transaction/recovery log** (`txlog`, JSON, written beside the
//! marker) plus a **cross-process advisory `flock` lock** so:
//!
//! * every mutating action (`provision`/`rotate`/`rollback`/`retire`)
//!   is modelled as a small phase machine (see [`PromotePhase`] /
//!   [`RetirePhase`]) whose *current phase* is durably persisted
//!   before each side effect that cannot be trivially undone;
//! * a process crash at any point can be resumed by the *next* caller
//!   (or by the standalone [`recover_in_flight_transaction`] entry
//!   point) re-entering the exact same phase-driven state machine —
//!   [`execute_promote`] and [`execute_retire`] are single codepaths
//!   used for both a fresh action and crash recovery of a leftover
//!   one;
//! * the invariant that makes this safe is **forward-only commitment**:
//!   before `current` is swapped (promote) or the physical generation
//!   tree is proven empty (retire), recovery is free to safely discard
//!   the leftover transaction (nothing observable changed yet); once
//!   that point is passed, recovery may only ever complete the
//!   transaction forward to its terminal, marker-durable state — it
//!   never reverts a swap or resurrects a deleted generation. This is
//!   the property finding (1) called out as missing: "never return an
//!   error after silently activating unrecoverable state".
//!
//! Marker identity (finding 2) is strengthened from the original
//! draft's bare `(dev, ino)` pair to [`MaterialIdentity`]: lineage
//! epoch, `(dev, ino)`, owner uid/gid, permission bits, link count,
//! POSIX-ACL presence, and a SHA-256 content digest — captured via a
//! `nofollow`-safe fd open (never a path re-resolution), and verified
//! against `current`'s *literal* name resolution (not just a
//! coincidentally-matching stat), so a hard-link plant, a
//! digest-preserving directory swap, or a permission/ownership/ACL
//! drift are each independently detected and separately named in the
//! closed [`super::secrets_rotation_audit::FailReason`] enum.
//!
//! Retirement (finding 3) never trusts the marker's word that storage
//! is clean: every `retire()` call first anchored-enumerates and
//! strictly validates the *entire* physical `generations/` tree
//! ([`enumerate_and_validate_generation_tree`]) regardless of what the
//! marker says. An absent/tombstoned marker over a *non-empty* tree is
//! treated as leftover material that retirement must still clean up
//! (not as evidence of a clean state); an *active* marker over an
//! *empty* tree is treated as a hard-fail anomaly rather than a silent
//! "already retired". Any unrecognised entry, wrong file type, or a
//! `material` file with a link count other than 1 aborts retirement
//! with zero deletions.
//!
//! Generation numbering (finding 4) is a monotonic high-water mark
//! (`MarkerData::high_water_epoch`) carried in the marker, not
//! `current epoch + 1`: `rotate` always allocates
//! `high_water_epoch + 1`, so a `rotate` issued after a `rollback` can
//! never collide with (or silently resurrect) a still-materialised
//! newer epoch that the rollback moved away from. Physical pruning of
//! a superseded generation only ever happens strictly after the new
//! marker has been durably committed.
//!
//! `provision()` always allocates epoch `1` and resets
//! `high_water_epoch` to `1` — this is intentionally **not** a
//! monotonic baseline carried across a full retire/re-provision cycle.
//! It is sound only because `retire()` physically empties the entire
//! `generations/` tree (enumerated and proven empty) before the
//! marker is tombstoned, so there is no physical collision risk in
//! restarting numbering at `1` for a fresh lineage. The monotonic
//! high-water invariant matters strictly *within* one non-retired
//! lineage (i.e. for `rotate`-after-`rollback` collision avoidance).
//!
//! [`SecretMaterial`] (finding 6) wraps caller-supplied bytes in
//! [`zeroize::Zeroizing`] **before** the length/emptiness validation
//! check (the original draft validated the raw `Vec<u8>` first and
//! only wrapped it in `Zeroizing` on the success path, so a rejected
//! empty/oversized buffer was dropped without zeroization). It derives
//! neither `Copy` nor `Clone`.
//!
//! # Status: not wired into any live broker dispatch path
//!
//! Nothing in this module is reachable from a running broker today.
//! An integrator must, in follow-up commits that this component does
//! **not** own:
//!
//! 1. Add a `SecretsLifecycle { .. }` request/response shape to the
//!    broker's private wire contract and a matching
//!    `runtime.rs` dispatch arm that calls
//!    [`provision`]/[`rotate`]/[`rollback`]/[`retire`].
//! 2. Add exactly one new
//!    `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
//!    variant (and its `from_operation_value` arm) to
//!    `ops::audit_op`, and wire the returned
//!    [`super::secrets_rotation_audit::SecretsLifecycleAuditFields`]
//!    into the broker's `crate::audit::AuditLog` sink.
//! 3. Add a matching `ops::mod` `pub mod secrets_lifecycle;` /
//!    `pub mod secrets_rotation_audit;` declaration (this component
//!    intentionally does not edit `ops/mod.rs`).
//! 4. Decide the real `SecretsLifecycleConfig::state_root` source
//!    (candidate: a subdirectory of the per-realm state root established
//!    by the ADR 0034 storage-lifecycle contract) and the real
//!    owner uid/gid for `TpmBoundCredential` vs `GuestSigningKey` vs
//!    `SecurityKeyChannelState` — this module takes them as
//!    caller-supplied configuration and asserts nothing about their
//!    real-world values. **`state_root` itself must already exist**
//!    with a non-world-writable mode before this module is called
//!    (mirrors `ensure_dir_preserve_existing`'s host-activation-owns-
//!    the-root convention elsewhere in the broker): this module only
//!    creates `state_root/<vm_id>/<kind-slug>` beneath it, never
//!    `state_root` itself.
//! 5. Decide how `TpmBoundCredential` rotation here relates to the
//!    swtpm NVRAM state `swtpm_dir.rs` already owns: this module
//!    tracks a rotation *lineage* layered on top of, and never
//!    mutates, the physical swtpm state directory.
//! 6. Decide the real `GuestSigningKey` material source (this module
//!    accepts already-generated bytes via [`SecretMaterial::new`]; it
//!    does not generate key material itself).
//! 7. Wire `d2b-sk-frontend`'s `security_key.rs` /
//!    `ComponentSession` channel policy to
//!    `d2b-sk-frontend::secrets_channel` (see that module's own
//!    "Integration wiring points" doc section).
//! 8. Regenerate the W8 wave-plan nix-unit pin after any of the above
//!    lands, per `tests/AGENTS.md`.
//! 9. Call [`recover_in_flight_transaction`] once at controller/broker
//!    startup for every known `(vm, kind)` pair before dispatching any
//!    live request, so a leftover transaction from a prior crash is
//!    drained proactively rather than only on the next incoming
//!    request for that exact pair.
//!
//! Everything above is exactly why this module's tests exercise the
//! engine directly (`#[cfg(test)]`, in-process, real temp
//! directories) rather than through any dispatch path — there is no
//! dispatch path yet.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::fcntl::{FlockArg, flock};
use rustix::fs::OFlags;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::ops::hosts::stable_hash_str;
use crate::ops::secrets_rotation_audit::{
    FailReason, LifecycleAction, MarkerResult, SecretKind, SecretsLifecycleAuditContext,
    SecretsLifecycleAuditFields,
};
use crate::sys::path_safe;

// ---------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------

/// Bumped whenever [`MarkerData`]'s on-disk shape changes
/// incompatibly. `read_marker` fails closed
/// (`MarkerTamperedOrMissingMaterial`) on any other version.
const MARKER_SCHEMA_VERSION: u32 = 2;

const GENERATIONS_DIR_NAME: &str = "generations";
const MARKER_FILE_NAME: &str = "marker.json";
const CURRENT_LINK_NAME: &str = "current";
const CURRENT_SWAP_STAGE_NAME: &str = ".current.stage";
const MATERIAL_FILE_NAME: &str = "material";
const LOCK_FILE_NAME: &str = "lock";
const TXLOG_FILE_NAME: &str = "txlog";
const STAGE_PREFIX: &str = ".stage-";

const DIR_MODE_DEFAULT: u32 = 0o700;
const FILE_MODE_DEFAULT: u32 = 0o600;

/// Bound on retries when allocating a collision-resistant staging
/// directory name. Each attempt mixes fresh entropy (pid + monotonic
/// counter + wall-clock nanos + `RandomState`-seeded hash); observing
/// exhaustion here would indicate a broken randomness source, not
/// ordinary contention.
const MAX_STAGE_NAME_ATTEMPTS: u32 = 32;

// ---------------------------------------------------------------------
// Config / paths
// ---------------------------------------------------------------------

/// Caller-supplied configuration. See the module doc's "Integration
/// wiring points" §4 for the real-world `state_root` /
/// `owner_uid`/`owner_gid` source this component does not decide.
#[derive(Debug, Clone)]
pub struct SecretsLifecycleConfig {
    /// Anchored root directory beneath which `<vm_id>/<kind-slug>/` is
    /// derived. Must be an absolute path.
    pub state_root: PathBuf,
    pub dir_mode: u32,
    pub file_mode: u32,
    pub owner_uid: Option<u32>,
    pub owner_gid: Option<u32>,
    /// Upper bound on how long [`acquire_lock`] polls for the
    /// cross-process exclusive lock before failing closed with
    /// [`FailReason::LockUnavailable`].
    pub lock_max_wait: Duration,
    pub lock_poll_interval: Duration,
}

impl Default for SecretsLifecycleConfig {
    fn default() -> Self {
        Self {
            state_root: PathBuf::from("/var/lib/d2b/secrets"),
            dir_mode: DIR_MODE_DEFAULT,
            file_mode: FILE_MODE_DEFAULT,
            owner_uid: None,
            owner_gid: None,
            lock_max_wait: Duration::from_secs(10),
            lock_poll_interval: Duration::from_millis(20),
        }
    }
}

/// Derived, redaction-safe path bundle for one `(vm, kind)` pair.
#[derive(Debug, Clone)]
pub struct SecretsLifecyclePaths {
    pub kind_root: PathBuf,
    /// FNV1a-64 hex fingerprint of `kind_root`, safe to place on the
    /// audit surface (parity with [`stable_hash_str`]).
    pub base_dir_hash: String,
}

fn valid_vm_id(vm_id: &str) -> bool {
    // Mirrors the framework-wide VM name contract
    // (`^[a-z][a-z0-9-]*$`, see `nixos-modules/assertions.nix`): used
    // here purely as a path-component safety gate, not to duplicate
    // that eval-time assertion.
    let mut chars = vm_id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Derive the anchored per-`(vm, kind)` directory. Fails closed on an
/// invalid `vm_id` before any filesystem access.
pub fn derive_paths(
    vm_id: &str,
    kind: SecretKind,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecyclePaths, FailReason> {
    if !valid_vm_id(vm_id) || !cfg.state_root.is_absolute() {
        return Err(FailReason::InvalidVmId);
    }
    let kind_root = cfg.state_root.join(vm_id).join(kind.as_slug());
    let base_dir_hash = stable_hash_str(&kind_root.to_string_lossy());
    Ok(SecretsLifecyclePaths {
        kind_root,
        base_dir_hash,
    })
}

fn context(
    vm_id: &str,
    kind: SecretKind,
    action: LifecycleAction,
    paths: &SecretsLifecyclePaths,
) -> SecretsLifecycleAuditContext {
    SecretsLifecycleAuditContext {
        vm_id: vm_id.to_owned(),
        kind,
        action,
        base_dir_hash: paths.base_dir_hash.clone(),
    }
}

// ---------------------------------------------------------------------
// Secret material
// ---------------------------------------------------------------------

/// Caller-supplied secret bytes. Deliberately **not** `Copy` or
/// `Clone`: every holder of an owned `SecretMaterial` is a distinct,
/// independently zeroized buffer. The bytes are wrapped in
/// [`Zeroizing`] *before* validation, so a rejected (empty or
/// oversized) buffer is still zeroized on drop rather than discarded
/// as a plain `Vec<u8>`.
pub struct SecretMaterial {
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretMaterial {
    /// 1 MiB. Generous for TPM-bound credential blobs, guest signing
    /// keys, and security-key channel state; prevents an unbounded
    /// allocation/hash from a misbehaving caller.
    pub const MAX_LEN: usize = 1 << 20;

    pub fn new(bytes: Vec<u8>) -> Result<Self, FailReason> {
        // Wrap FIRST: a validation-rejected buffer is zeroized on
        // drop here, not silently dropped as a plain `Vec<u8>`.
        let bytes = Zeroizing::new(bytes);
        if bytes.is_empty() || bytes.len() > Self::MAX_LEN {
            return Err(FailReason::InvalidMaterial);
        }
        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn digest_hex(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&*self.bytes);
        hex_encode(&hasher.finalize())
    }
}

impl std::fmt::Debug for SecretMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretMaterial")
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ---------------------------------------------------------------------
// Marker identity
// ---------------------------------------------------------------------

/// Full tamper-detecting identity of one generation's `material` file,
/// captured via a `nofollow`-safe fd open (never a path
/// re-resolution).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialIdentity {
    pub epoch: u64,
    pub dev: u64,
    pub ino: u64,
    pub uid: u32,
    pub gid: u32,
    /// Permission bits only (`st_mode & 0o7777`).
    pub mode: u32,
    pub nlink: u64,
    pub has_acl: bool,
    pub digest_hex: String,
}

/// On-disk marker (`marker.json`), schema v2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerData {
    pub v: u32,
    pub vm: String,
    pub kind: String,
    pub retired: bool,
    pub high_water_epoch: u64,
    pub active: Option<MaterialIdentity>,
    pub previous: Option<MaterialIdentity>,
    pub first_provisioned_ms: u64,
    pub updated_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Transaction / recovery log
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotePhase {
    /// Intent recorded; the target generation may or may not be
    /// staged/created yet. Nothing observable (`current`/marker) has
    /// changed. Safe to discard.
    Planned,
    /// `generations/<to_epoch>/material` exists and is durable
    /// (fsynced), verified against `expected_digest_hex`. `current`
    /// still points elsewhere (or nowhere). Still safe to discard.
    EpochReady,
    /// `current` now resolves to `generations/<to_epoch>/`, durable.
    /// This is the activation point: recovery may only move forward
    /// from here.
    CurrentPromoted,
    /// `marker.json` durably reflects the new active/previous/
    /// high-water state. Only pruning + txlog removal remain
    /// (idempotent).
    MarkerCommitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetirePhase {
    /// The physical generation tree has been anchored-enumerated and
    /// validated; `intent.epochs` is the recorded deletion plan.
    /// `current` may still be present. Nothing has been deleted yet.
    Enumerated,
    /// `current` has been removed. No generation has been deleted
    /// yet.
    CurrentRemoved,
    /// Every recorded epoch has been removed (idempotently — already-
    /// removed entries are tolerated).
    EpochsRemoved,
    /// A fresh re-enumeration proved the tree is empty. Only the
    /// tombstone marker write + txlog removal remain.
    ProvenEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteIntent {
    pub vm: String,
    pub kind: String,
    pub action: LifecycleAction,
    pub to_epoch: u64,
    pub create_epoch: bool,
    /// Name of the staging directory used for a freshly-created
    /// epoch. Empty when `create_epoch` is `false` (rollback).
    pub stage_name: String,
    pub expected_digest_hex: String,
    /// Full recorded identity of the rollback target, used to
    /// tamper-verify the pre-existing generation before promoting it.
    /// `None` when `create_epoch` is `true`.
    pub expected_identity: Option<MaterialIdentity>,
    /// The identity that becomes the new `previous` once this action
    /// commits.
    pub carry_previous: Option<MaterialIdentity>,
    /// A generation to prune strictly after the new marker is
    /// durably committed. `None` when nothing is superseded.
    pub prune_epoch: Option<u64>,
    pub new_high_water_epoch: u64,
    pub first_provisioned_ms: u64,
    pub phase: PromotePhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetireIntent {
    pub vm: String,
    pub kind: String,
    /// The recorded deletion plan from the enumeration that started
    /// this transaction. Recovery re-enumerates fresh and requires the
    /// fresh result to be a subset of this plan.
    pub epochs: Vec<u64>,
    pub high_water_epoch: u64,
    pub phase: RetirePhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tx_variant", rename_all = "snake_case", deny_unknown_fields)]
pub enum TxLog {
    Promote(PromoteIntent),
    Retire(RetireIntent),
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsLifecycleError {
    pub reason: FailReason,
    pub audit: SecretsLifecycleAuditFields,
}

impl std::fmt::Display for SecretsLifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "secrets-lifecycle: {}", self.reason)
    }
}

impl std::error::Error for SecretsLifecycleError {}

fn denied(ctx: &SecretsLifecycleAuditContext, reason: FailReason) -> SecretsLifecycleError {
    SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::denied(ctx, reason),
    }
}

fn failed_closed(
    ctx: &SecretsLifecycleAuditContext,
    marker_result: MarkerResult,
    reason: FailReason,
) -> SecretsLifecycleError {
    SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(ctx, marker_result, reason),
    }
}

fn io_to_reason(_err: io::Error, on_failure: FailReason) -> FailReason {
    on_failure
}

// ---------------------------------------------------------------------
// Directory / marker / txlog primitives
// ---------------------------------------------------------------------

/// Open (creating if absent) the anchored `kind_root` directory and
/// its `generations/` child, returning both fds. Every subsequent
/// operation is fd-relative from here.
///
/// `cfg.state_root` itself is a precondition, not something this
/// component creates: [`path_safe::ensure_dir`] refuses to operate
/// under a world-writable parent, and the real deployment's secrets
/// root is expected to be established (with the correct non-world-
/// writable mode) by host activation before the broker ever calls
/// into this module — see the module doc's "Integration wiring
/// points" §4. `open_dir_path_safe` (no world-writable-parent check)
/// only verifies it exists. `vm_dir` and `kind_root` are created
/// on demand beneath it via `ensure_dir`, which is safe here because
/// their immediate parent (`state_root`, then `vm_dir`) is never
/// world-writable once `state_root` itself is correctly established.
fn open_or_create_secrets_dir(
    paths: &SecretsLifecyclePaths,
    cfg: &SecretsLifecycleConfig,
) -> Result<(OwnedFd, OwnedFd), FailReason> {
    let vm_dir = paths
        .kind_root
        .parent()
        .ok_or(FailReason::SecretsDirOpenFailed)?;
    path_safe::open_dir_path_safe(&cfg.state_root).map_err(|_| FailReason::SecretsDirOpenFailed)?;
    path_safe::ensure_dir(vm_dir, cfg.dir_mode, cfg.owner_uid, cfg.owner_gid)
        .map_err(|_| FailReason::SecretsDirOpenFailed)?;
    path_safe::ensure_dir(&paths.kind_root, cfg.dir_mode, cfg.owner_uid, cfg.owner_gid)
        .map_err(|_| FailReason::SecretsDirOpenFailed)?;
    let secrets_fd = path_safe::open_dir_path_safe(&paths.kind_root)
        .map_err(|_| FailReason::SecretsDirOpenFailed)?;
    path_safe::mkdir_at(
        secrets_fd.as_fd(),
        Path::new(GENERATIONS_DIR_NAME),
        cfg.dir_mode,
    )
    .map_err(|_| FailReason::MarkerTreeOpenFailed)?;
    let generations_fd = path_safe::open_at(
        secrets_fd.as_fd(),
        Path::new(GENERATIONS_DIR_NAME),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::MarkerTreeOpenFailed)?;
    Ok((secrets_fd, generations_fd))
}

fn open_generations_dir(secrets_fd: BorrowedFd<'_>) -> Result<OwnedFd, FailReason> {
    path_safe::open_at(
        secrets_fd,
        Path::new(GENERATIONS_DIR_NAME),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::MarkerTreeOpenFailed)
}

fn read_marker(secrets_fd: BorrowedFd<'_>) -> Result<Option<MarkerData>, FailReason> {
    let fd = match path_safe::open_file_at_safe(
        &borrowed_to_owned(secrets_fd)?,
        MARKER_FILE_NAME,
        nix::libc::O_RDONLY,
    ) {
        Ok(fd) => fd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(FailReason::MarkerTreeOpenFailed),
    };
    let bytes = read_fd_to_end(fd.as_fd()).map_err(|_| FailReason::MarkerTreeOpenFailed)?;
    let marker: MarkerData =
        serde_json::from_slice(&bytes).map_err(|_| FailReason::MarkerTamperedOrMissingMaterial)?;
    if marker.v != MARKER_SCHEMA_VERSION {
        return Err(FailReason::MarkerTamperedOrMissingMaterial);
    }
    Ok(Some(marker))
}

fn write_marker(secrets_fd: BorrowedFd<'_>, marker: &MarkerData) -> Result<(), FailReason> {
    let body = serde_json::to_vec(marker).map_err(|_| FailReason::MarkerWriteFailed)?;
    let owned = borrowed_to_owned(secrets_fd)?;
    path_safe::atomic_replace_fd_with_owner(
        &owned,
        MARKER_FILE_NAME,
        &body,
        FILE_MODE_DEFAULT,
        None,
        None,
    )
    .map_err(|_| FailReason::MarkerWriteFailed)?;
    fsync_fd(secrets_fd);
    Ok(())
}

fn read_txlog(secrets_fd: BorrowedFd<'_>) -> Result<Option<TxLog>, FailReason> {
    let owned = borrowed_to_owned(secrets_fd)?;
    let fd = match path_safe::open_file_at_safe(&owned, TXLOG_FILE_NAME, nix::libc::O_RDONLY) {
        Ok(fd) => fd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(FailReason::IntentCorrupt),
    };
    let bytes = read_fd_to_end(fd.as_fd()).map_err(|_| FailReason::IntentCorrupt)?;
    let log: TxLog = serde_json::from_slice(&bytes).map_err(|_| FailReason::IntentCorrupt)?;
    Ok(Some(log))
}

fn write_txlog(secrets_fd: BorrowedFd<'_>, log: &TxLog) -> Result<(), FailReason> {
    let body = serde_json::to_vec(log).map_err(|_| FailReason::IntentWriteFailed)?;
    let owned = borrowed_to_owned(secrets_fd)?;
    path_safe::atomic_replace_fd_with_owner(
        &owned,
        TXLOG_FILE_NAME,
        &body,
        FILE_MODE_DEFAULT,
        None,
        None,
    )
    .map_err(|_| FailReason::IntentWriteFailed)?;
    fsync_fd(secrets_fd);
    Ok(())
}

fn remove_txlog(secrets_fd: BorrowedFd<'_>) -> Result<(), FailReason> {
    let owned = borrowed_to_owned(secrets_fd)?;
    match path_safe::remove_path_safe(&owned, TXLOG_FILE_NAME) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(FailReason::IntentWriteFailed),
    }
    fsync_fd(secrets_fd);
    Ok(())
}

/// `BorrowedFd` -> `OwnedFd` without taking ownership of the original
/// descriptor: dups via `openat2(".")`-equivalent semantics through
/// [`path_safe::open_at`]. Needed because several `path_safe` helpers
/// take `&OwnedFd`, while callers here hold a `BorrowedFd` (e.g. from
/// a `LockGuard` or a caller-held directory fd) for the duration of a
/// single call.
fn borrowed_to_owned(fd: BorrowedFd<'_>) -> Result<OwnedFd, FailReason> {
    path_safe::open_at(fd, Path::new("."), OFlags::RDONLY | OFlags::DIRECTORY)
        .map_err(|_| FailReason::MarkerTreeOpenFailed)
}

fn read_fd_to_end(fd: BorrowedFd<'_>) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::fd::AsRawFd as _;
    // SAFETY-free: build a short-lived `File` view over the fd without
    // taking ownership, restoring the raw fd afterward via
    // `into_raw_fd` on a duplicated descriptor so the original isn't
    // double-closed.
    let dup = rustix::fs::fcntl_dupfd_cloexec(fd, 0).map_err(io_from_rustix)?;
    let mut file: std::fs::File = dup.into();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let _ = fd.as_raw_fd();
    Ok(buf)
}

fn fsync_fd(fd: BorrowedFd<'_>) {
    let _ = rustix::fs::fsync(fd);
}

fn io_from_rustix(err: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(err.raw_os_error())
}

// ---------------------------------------------------------------------
// Cross-process lock
// ---------------------------------------------------------------------

pub struct LockGuard {
    _file: std::fs::File,
}

fn acquire_lock(
    secrets_fd: BorrowedFd<'_>,
    cfg: &SecretsLifecycleConfig,
) -> Result<LockGuard, FailReason> {
    let owned = borrowed_to_owned(secrets_fd)?;
    let fd = path_safe::create_file_at_safe(
        &owned,
        LOCK_FILE_NAME,
        nix::libc::O_RDWR | nix::libc::O_CREAT,
        FILE_MODE_DEFAULT,
    )
    .map_err(|_| FailReason::LockUnavailable)?;
    let file: std::fs::File = fd.into();
    let deadline = Instant::now() + cfg.lock_max_wait;
    loop {
        match flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
            Ok(()) => return Ok(LockGuard { _file: file }),
            Err(nix::errno::Errno::EWOULDBLOCK) => {
                if Instant::now() >= deadline {
                    return Err(FailReason::LockUnavailable);
                }
                std::thread::sleep(cfg.lock_poll_interval);
            }
            Err(_) => return Err(FailReason::LockUnavailable),
        }
    }
}

// ---------------------------------------------------------------------
// Staging / generation helpers
// ---------------------------------------------------------------------

fn random_stage_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(pid);
    hasher.write_u64(counter);
    hasher.write_u64(nanos);
    let entropy = hasher.finish();

    let mut mix = Sha256::new();
    mix.update(pid.to_le_bytes());
    mix.update(counter.to_le_bytes());
    mix.update(nanos.to_le_bytes());
    mix.update(entropy.to_le_bytes());
    let digest = mix.finalize();
    format!("{STAGE_PREFIX}{}", hex_encode(&digest[..16]))
}

fn parse_strict_epoch(name: &str) -> Option<u64> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if name.len() > 1 && name.starts_with('0') {
        return None;
    }
    name.parse::<u64>().ok()
}

/// Sweep any leftover `.stage-*` directories (from a crash before a
/// stage was ever recorded in a txlog, or one already superseded).
/// Best-effort: failures are ignored, since a stray stage dir has no
/// bearing on correctness (it is never referenced by `current` or the
/// marker).
fn sweep_stale_stage_dirs(generations_fd: BorrowedFd<'_>, keep: Option<&str>) {
    let owned = match borrowed_to_owned(generations_fd) {
        Ok(fd) => fd,
        Err(_) => return,
    };
    let Ok(dir) = rustix::fs::Dir::read_from(generations_fd) else {
        return;
    };
    for entry in dir.flatten() {
        let Ok(name) = entry.file_name().to_str() else {
            continue;
        };
        if name == "." || name == ".." {
            continue;
        }
        if !name.starts_with(STAGE_PREFIX) {
            continue;
        }
        if keep == Some(name) {
            continue;
        }
        let _ = remove_dir_tree_one_level(&owned, name);
    }
}

/// Remove a one-level directory (a stage dir contains at most one
/// `material` file) via `remove_path_safe` on each child then the
/// directory itself.
fn remove_dir_tree_one_level(parent_fd: &OwnedFd, name: &str) -> io::Result<()> {
    if let Ok(child_fd) = path_safe::open_at(
        parent_fd.as_fd(),
        Path::new(name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    ) && let Ok(dir) = rustix::fs::Dir::read_from(child_fd.as_fd())
    {
        let child_owned = borrowed_to_owned_io(child_fd.as_fd())?;
        for entry in dir.flatten() {
            if let Ok(cname) = entry.file_name().to_str()
                && cname != "."
                && cname != ".."
            {
                let _ = path_safe::remove_path_safe(&child_owned, cname);
            }
        }
    }
    path_safe::remove_path_safe(parent_fd, name)
}

fn borrowed_to_owned_io(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    path_safe::open_at(fd, Path::new("."), OFlags::RDONLY | OFlags::DIRECTORY)
}

/// Stage `material` bytes into a fresh, collision-resistant directory
/// beneath `generations/`, fsyncing the material file and the stage
/// directory before returning. Idempotent with respect to
/// `stage_name`: if a directory of that name already exists with a
/// matching digest, this is a no-op (crash-recovery re-entry).
fn ensure_staged(
    generations_fd: BorrowedFd<'_>,
    cfg: &SecretsLifecycleConfig,
    stage_name: &str,
    expected_digest_hex: &str,
    material: Option<&SecretMaterial>,
) -> Result<bool, FailReason> {
    let generations_owned = borrowed_to_owned(generations_fd)?;
    if let Ok(existing_digest) = digest_of_material_dir(&generations_owned, stage_name) {
        if existing_digest == expected_digest_hex {
            return Ok(true);
        }
        // Tampered/partial stage from a previous crash: nothing has
        // been activated yet (we are still pre-EpochReady), so it is
        // safe to discard and let the caller decide (abort/retry).
        let _ = remove_dir_tree_one_level(&generations_owned, stage_name);
        return Err(FailReason::RecoveryContentMismatch);
    }

    let Some(material) = material else {
        return Ok(false);
    };

    path_safe::mkdir_at_exclusive(generations_fd, Path::new(stage_name), cfg.dir_mode)
        .map_err(|_| FailReason::MaterialWriteFailed)?;
    let stage_fd = path_safe::open_at(
        generations_fd,
        Path::new(stage_name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::MaterialWriteFailed)?;
    let stage_owned = borrowed_to_owned(stage_fd.as_fd())?;
    path_safe::atomic_replace_fd_with_owner(
        &stage_owned,
        MATERIAL_FILE_NAME,
        material.as_bytes(),
        cfg.file_mode,
        cfg.owner_uid,
        cfg.owner_gid,
    )
    .map_err(|_| FailReason::MaterialWriteFailed)?;
    fsync_fd(stage_fd.as_fd());
    fsync_fd(generations_fd);
    Ok(true)
}

fn digest_of_material_dir(generations_fd: &OwnedFd, dir_name: &str) -> io::Result<String> {
    let dir_fd = path_safe::open_at(
        generations_fd.as_fd(),
        Path::new(dir_name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )?;
    let material_fd = path_safe::open_file_at_safe(
        &borrowed_to_owned_io(dir_fd.as_fd())?,
        MATERIAL_FILE_NAME,
        nix::libc::O_RDONLY,
    )?;
    let bytes = Zeroizing::new(read_fd_to_end(material_fd.as_fd())?);
    let mut hasher = Sha256::new();
    hasher.update(&*bytes);
    Ok(hex_encode(&hasher.finalize()))
}

/// Rename the stage directory into its final numeric epoch name.
/// Idempotent: if the final directory already exists with a matching
/// digest (a previous crash-recovery pass already renamed it), this
/// is a no-op.
fn promote_stage_to_generation(
    generations_fd: BorrowedFd<'_>,
    stage_name: &str,
    to_epoch: u64,
) -> Result<(), FailReason> {
    let epoch_name = to_epoch.to_string();
    let generations_owned = borrowed_to_owned(generations_fd)?;
    if digest_of_material_dir(&generations_owned, &epoch_name).is_ok() {
        return Ok(());
    }
    rustix::fs::renameat(
        generations_fd,
        Path::new(stage_name),
        generations_fd,
        Path::new(&epoch_name),
    )
    .map_err(|_| FailReason::GenerationConflict)?;
    fsync_fd(generations_fd);
    Ok(())
}

fn capture_identity(
    generations_fd: BorrowedFd<'_>,
    epoch: u64,
) -> Result<MaterialIdentity, FailReason> {
    let epoch_name = epoch.to_string();
    let generations_owned = borrowed_to_owned(generations_fd)?;
    let dir_fd = path_safe::open_at(
        generations_fd,
        Path::new(&epoch_name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::PreviouslyProvisionedMaterialMissing)?;
    let dir_owned = borrowed_to_owned(dir_fd.as_fd())?;
    let material_fd =
        path_safe::open_file_at_safe(&dir_owned, MATERIAL_FILE_NAME, nix::libc::O_RDONLY)
            .map_err(|_| FailReason::PreviouslyProvisionedMaterialMissing)?;
    let stat = path_safe::fstat_fd(material_fd.as_fd())
        .map_err(|_| FailReason::PreviouslyProvisionedMaterialMissing)?;
    let (has_acl, _) = path_safe::fd_extended_acl_present(material_fd.as_fd())
        .map_err(|_| FailReason::PreviouslyProvisionedMaterialMissing)?;
    let bytes = Zeroizing::new(
        read_fd_to_end(material_fd.as_fd())
            .map_err(|_| FailReason::PreviouslyProvisionedMaterialMissing)?,
    );
    let mut hasher = Sha256::new();
    hasher.update(&*bytes);
    let digest_hex = hex_encode(&hasher.finalize());
    let _ = generations_owned;
    Ok(MaterialIdentity {
        epoch,
        dev: stat.st_dev as u64,
        ino: stat.st_ino as u64,
        uid: stat.st_uid,
        gid: stat.st_gid,
        mode: (stat.st_mode as u32) & 0o7777,
        nlink: stat.st_nlink as u64,
        has_acl,
        digest_hex,
    })
}

/// Tamper-verify a live generation against a recorded identity
/// (used by rollback's Planned phase to confirm the retained
/// generation has not drifted since it was recorded).
fn verify_existing_generation_identity(
    generations_fd: BorrowedFd<'_>,
    expected: &MaterialIdentity,
) -> Result<(), FailReason> {
    let live = capture_identity(generations_fd, expected.epoch)?;
    if live.uid != expected.uid || live.gid != expected.gid {
        return Err(FailReason::IdentityOwnerMismatch);
    }
    if live.mode != expected.mode {
        return Err(FailReason::IdentityModeMismatch);
    }
    if live.has_acl != expected.has_acl {
        return Err(FailReason::IdentityAclMismatch);
    }
    if live.nlink != expected.nlink {
        return Err(FailReason::IdentityLinkCountMismatch);
    }
    if live.digest_hex != expected.digest_hex {
        return Err(FailReason::IdentityDigestMismatch);
    }
    if live.dev != expected.dev || live.ino != expected.ino {
        return Err(FailReason::IdentityInodeMismatch);
    }
    Ok(())
}

/// Read the literal name `current` resolves to (its symlink target's
/// final component), without following the link past that.
fn read_current_target(secrets_fd: BorrowedFd<'_>) -> Option<u64> {
    let target = rustix::fs::readlinkat(secrets_fd, Path::new(CURRENT_LINK_NAME), Vec::new())
        .ok()?
        .into_string()
        .ok()?;
    let name = Path::new(&target).file_name()?.to_str()?;
    parse_strict_epoch(name)
}

/// Atomically swap `current` to point at `generations/<epoch>` using a
/// hidden-name-then-rename swap so there is never a moment `current`
/// is absent. Idempotent: if `current` already resolves to `epoch`,
/// this is a no-op.
fn atomic_swap_current(secrets_fd: BorrowedFd<'_>, epoch: u64) -> Result<(), FailReason> {
    if read_current_target(secrets_fd) == Some(epoch) {
        return Ok(());
    }
    let owned = borrowed_to_owned(secrets_fd)?;
    let target = PathBuf::from(GENERATIONS_DIR_NAME).join(epoch.to_string());
    // Best-effort cleanup of a leftover stage symlink from a previous
    // crash between symlink-creation and rename.
    let _ = path_safe::remove_path_safe(&owned, CURRENT_SWAP_STAGE_NAME);
    rustix::fs::symlinkat(&target, secrets_fd, Path::new(CURRENT_SWAP_STAGE_NAME))
        .map_err(|_| FailReason::CurrentSwapFailed)?;
    rustix::fs::renameat(
        secrets_fd,
        Path::new(CURRENT_SWAP_STAGE_NAME),
        secrets_fd,
        Path::new(CURRENT_LINK_NAME),
    )
    .map_err(|_| FailReason::CurrentSwapFailed)?;
    fsync_fd(secrets_fd);
    Ok(())
}

/// `current` is always a symlink; [`path_safe::remove_path_safe`]
/// refuses to operate on symlinks by design, so removal goes through
/// a raw `unlinkat` instead. Tolerates absence (idempotent).
fn remove_current_symlink(secrets_fd: BorrowedFd<'_>) -> Result<(), FailReason> {
    match rustix::fs::unlinkat(
        secrets_fd,
        Path::new(CURRENT_LINK_NAME),
        rustix::fs::AtFlags::empty(),
    ) {
        Ok(()) => {
            fsync_fd(secrets_fd);
            Ok(())
        }
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(_) => Err(FailReason::CurrentSwapFailed),
    }
}

/// Remove one already-validated generation directory
/// (`generations/<epoch>/material` then `generations/<epoch>/`).
/// Tolerates the directory already being absent (idempotent
/// crash-recovery replay). Any other failure is propagated: the
/// caller must fail the whole retirement closed rather than continue
/// past a partial deletion.
fn remove_generation(generations_fd: BorrowedFd<'_>, epoch: u64) -> Result<(), FailReason> {
    let epoch_name = epoch.to_string();
    let generations_owned = borrowed_to_owned(generations_fd)?;
    let dir_fd = match path_safe::open_at(
        generations_fd,
        Path::new(&epoch_name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    ) {
        Ok(fd) => fd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(FailReason::RetirementTreeAnomaly),
    };
    let dir_owned = borrowed_to_owned(dir_fd.as_fd())?;
    match path_safe::remove_path_safe(&dir_owned, MATERIAL_FILE_NAME) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(FailReason::RetirementTreeAnomaly),
    }
    drop(dir_fd);
    path_safe::remove_path_safe(&generations_owned, &epoch_name)
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    fsync_fd(generations_fd);
    Ok(())
}

/// Anchored enumeration of `generations/`, failing closed on any
/// entry this cannot fully account for. Deletes nothing; a caller
/// invokes this to *plan* retirement, never as part of deleting.
fn enumerate_and_validate_generation_tree(
    generations_fd: BorrowedFd<'_>,
) -> Result<Vec<u64>, FailReason> {
    let generations_owned = borrowed_to_owned(generations_fd)?;
    let dir = rustix::fs::Dir::read_from(generations_fd)
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let mut epochs = Vec::new();
    for entry in dir {
        let entry = entry.map_err(|_| FailReason::RetirementTreeAnomaly)?;
        let name = entry
            .file_name()
            .to_str()
            .map_err(|_| FailReason::RetirementTreeAnomaly)?
            .to_owned();
        if name == "." || name == ".." {
            continue;
        }
        if name.starts_with(STAGE_PREFIX) {
            // Stage dirs are swept separately, never part of the
            // retirement plan; their presence does not block
            // retirement, but they must not be miscounted as a
            // generation either.
            continue;
        }
        let Some(epoch) = parse_strict_epoch(&name) else {
            return Err(FailReason::RetirementTreeAnomaly);
        };
        verify_generation_dir_is_exactly_material(&generations_owned, &name)?;
        epochs.push(epoch);
    }
    epochs.sort_unstable();
    Ok(epochs)
}

fn verify_generation_dir_is_exactly_material(
    generations_fd: &OwnedFd,
    name: &str,
) -> Result<(), FailReason> {
    let stat = path_safe::fstatat_nofollow(generations_fd, name)
        .map_err(|_| FailReason::RetirementTreeAnomaly)?
        .ok_or(FailReason::RetirementTreeAnomaly)?;
    if (stat.st_mode & nix::libc::S_IFMT) != nix::libc::S_IFDIR {
        return Err(FailReason::RetirementTreeAnomaly);
    }
    let dir_fd = path_safe::open_at(
        generations_fd.as_fd(),
        Path::new(name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let dir_owned = borrowed_to_owned(dir_fd.as_fd())?;
    let inner = rustix::fs::Dir::read_from(dir_fd.as_fd())
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let mut saw_material = false;
    for entry in inner {
        let entry = entry.map_err(|_| FailReason::RetirementTreeAnomaly)?;
        let ename = entry
            .file_name()
            .to_str()
            .map_err(|_| FailReason::RetirementTreeAnomaly)?
            .to_owned();
        if ename == "." || ename == ".." {
            continue;
        }
        if ename != MATERIAL_FILE_NAME {
            return Err(FailReason::RetirementTreeAnomaly);
        }
        saw_material = true;
    }
    if !saw_material {
        return Err(FailReason::RetirementTreeAnomaly);
    }
    let material_stat = path_safe::fstatat_nofollow(&dir_owned, MATERIAL_FILE_NAME)
        .map_err(|_| FailReason::RetirementTreeAnomaly)?
        .ok_or(FailReason::RetirementTreeAnomaly)?;
    if (material_stat.st_mode & nix::libc::S_IFMT) != nix::libc::S_IFREG {
        return Err(FailReason::RetirementTreeAnomaly);
    }
    if material_stat.st_nlink != 1 {
        return Err(FailReason::RetirementTreeAnomaly);
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Unified promote engine (fresh action + crash recovery)
// ---------------------------------------------------------------------

enum PromoteOutcome {
    Completed(MarkerData),
    /// Only reachable when recovering a crash that occurred before
    /// any material was ever staged for a `create_epoch` intent —
    /// i.e. nothing was ever activated. Discarding is safe.
    AbortedNoMaterial,
}

fn execute_promote(
    secrets_fd: BorrowedFd<'_>,
    generations_fd: BorrowedFd<'_>,
    cfg: &SecretsLifecycleConfig,
    mut intent: PromoteIntent,
    mut material: Option<&SecretMaterial>,
) -> Result<PromoteOutcome, FailReason> {
    loop {
        match intent.phase {
            PromotePhase::Planned => {
                if intent.create_epoch {
                    let staged = ensure_staged(
                        generations_fd,
                        cfg,
                        &intent.stage_name,
                        &intent.expected_digest_hex,
                        material,
                    )?;
                    if !staged {
                        remove_txlog(secrets_fd)?;
                        return Ok(PromoteOutcome::AbortedNoMaterial);
                    }
                    promote_stage_to_generation(
                        generations_fd,
                        &intent.stage_name,
                        intent.to_epoch,
                    )?;
                } else {
                    let expected = intent
                        .expected_identity
                        .as_ref()
                        .ok_or(FailReason::RecoveryAmbiguous)?;
                    verify_existing_generation_identity(generations_fd, expected)?;
                }
                material = None;
                intent.phase = PromotePhase::EpochReady;
                write_txlog(secrets_fd, &TxLog::Promote(intent.clone()))?;
            }
            PromotePhase::EpochReady => {
                atomic_swap_current(secrets_fd, intent.to_epoch)?;
                intent.phase = PromotePhase::CurrentPromoted;
                write_txlog(secrets_fd, &TxLog::Promote(intent.clone()))?;
            }
            PromotePhase::CurrentPromoted => {
                let identity = capture_identity(generations_fd, intent.to_epoch)?;
                if identity.digest_hex != intent.expected_digest_hex {
                    // `current` already points at this epoch; we must
                    // never revert it. This can only mean the intent
                    // itself was corrupted (e.g. hand-edited txlog) —
                    // fail closed without touching current/marker.
                    return Err(FailReason::RecoveryAmbiguous);
                }
                if intent.new_high_water_epoch < intent.to_epoch {
                    return Err(FailReason::HighWaterRegressed);
                }
                let marker = MarkerData {
                    v: MARKER_SCHEMA_VERSION,
                    vm: intent.vm.clone(),
                    kind: intent.kind.clone(),
                    retired: false,
                    high_water_epoch: intent.new_high_water_epoch,
                    active: Some(identity),
                    previous: intent.carry_previous.clone(),
                    first_provisioned_ms: intent.first_provisioned_ms,
                    updated_ms: now_ms(),
                };
                write_marker(secrets_fd, &marker)?;
                intent.phase = PromotePhase::MarkerCommitted;
                write_txlog(secrets_fd, &TxLog::Promote(intent.clone()))?;
            }
            PromotePhase::MarkerCommitted => {
                if let Some(prune) = intent.prune_epoch {
                    // Best-effort: pruning failure does not fail the
                    // action (the marker already durably reflects the
                    // correct active/previous state); it only leaves
                    // an orphaned generation directory for a future
                    // retire() to clean up via its full-tree
                    // enumeration.
                    let _ = remove_generation(generations_fd, prune);
                }
                sweep_stale_stage_dirs(generations_fd, None);
                remove_txlog(secrets_fd)?;
                let marker = read_marker(secrets_fd)?.ok_or(FailReason::MarkerWriteFailed)?;
                return Ok(PromoteOutcome::Completed(marker));
            }
        }
    }
}

// ---------------------------------------------------------------------
// Unified retire engine (fresh action + crash recovery)
// ---------------------------------------------------------------------

fn execute_retire(
    secrets_fd: BorrowedFd<'_>,
    generations_fd: BorrowedFd<'_>,
    mut intent: RetireIntent,
) -> Result<(), FailReason> {
    loop {
        match intent.phase {
            RetirePhase::Enumerated => {
                let fresh = enumerate_and_validate_generation_tree(generations_fd)?;
                if !fresh.iter().all(|e| intent.epochs.contains(e)) {
                    return Err(FailReason::RecoveryAmbiguous);
                }
                remove_current_symlink(secrets_fd)?;
                intent.phase = RetirePhase::CurrentRemoved;
                write_txlog(secrets_fd, &TxLog::Retire(intent.clone()))?;
            }
            RetirePhase::CurrentRemoved => {
                for &epoch in &intent.epochs {
                    remove_generation(generations_fd, epoch)?;
                }
                intent.phase = RetirePhase::EpochsRemoved;
                write_txlog(secrets_fd, &TxLog::Retire(intent.clone()))?;
            }
            RetirePhase::EpochsRemoved => {
                let remaining = enumerate_and_validate_generation_tree(generations_fd)?;
                if !remaining.is_empty() {
                    return Err(FailReason::RetirementNotProvablyEmpty);
                }
                intent.phase = RetirePhase::ProvenEmpty;
                write_txlog(secrets_fd, &TxLog::Retire(intent.clone()))?;
            }
            RetirePhase::ProvenEmpty => {
                sweep_stale_stage_dirs(generations_fd, None);
                let marker = MarkerData {
                    v: MARKER_SCHEMA_VERSION,
                    vm: intent.vm.clone(),
                    kind: intent.kind.clone(),
                    retired: true,
                    high_water_epoch: intent.high_water_epoch,
                    active: None,
                    previous: None,
                    first_provisioned_ms: 0,
                    updated_ms: now_ms(),
                };
                write_marker(secrets_fd, &marker)?;
                remove_txlog(secrets_fd)?;
                return Ok(());
            }
        }
    }
}

// ---------------------------------------------------------------------
// Recovery dispatch
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryError {
    pub action: Option<LifecycleAction>,
    pub reason: FailReason,
}

fn recover_if_needed(
    secrets_fd: BorrowedFd<'_>,
    generations_fd: BorrowedFd<'_>,
    cfg: &SecretsLifecycleConfig,
) -> Result<bool, RecoveryError> {
    let txlog = read_txlog(secrets_fd).map_err(|reason| RecoveryError {
        action: None,
        reason,
    })?;
    let Some(txlog) = txlog else {
        return Ok(false);
    };
    match txlog {
        TxLog::Promote(intent) => {
            let action = intent.action;
            match execute_promote(secrets_fd, generations_fd, cfg, intent, None) {
                Ok(_) => Ok(true),
                Err(reason) => Err(RecoveryError {
                    action: Some(action),
                    reason,
                }),
            }
        }
        TxLog::Retire(intent) => match execute_retire(secrets_fd, generations_fd, intent) {
            Ok(()) => Ok(true),
            Err(reason) => Err(RecoveryError {
                action: Some(LifecycleAction::Retire),
                reason,
            }),
        },
    }
}

/// Standalone entry point for draining a leftover crash-interrupted
/// transaction independent of issuing a new lifecycle action (e.g. at
/// realm controller/broker startup, per the module doc's wiring point
/// §9). Returns `Ok(true)` if a leftover transaction was found and
/// completed/discarded, `Ok(false)` if there was nothing to recover.
pub fn recover_in_flight_transaction(
    vm_id: &str,
    kind: SecretKind,
    cfg: &SecretsLifecycleConfig,
) -> Result<bool, SecretsLifecycleError> {
    let paths = derive_paths(vm_id, kind, cfg).map_err(|reason| SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::denied(
            &SecretsLifecycleAuditContext {
                vm_id: vm_id.to_owned(),
                kind,
                action: LifecycleAction::Provision,
                base_dir_hash: String::new(),
            },
            reason,
        ),
    })?;
    let (secrets_fd, generations_fd) =
        open_or_create_secrets_dir(&paths, cfg).map_err(|reason| {
            let ctx = context(vm_id, kind, LifecycleAction::Provision, &paths);
            denied(&ctx, reason)
        })?;
    let _lock = acquire_lock(secrets_fd.as_fd(), cfg).map_err(|reason| {
        let ctx = context(vm_id, kind, LifecycleAction::Provision, &paths);
        denied(&ctx, reason)
    })?;
    match recover_if_needed(secrets_fd.as_fd(), generations_fd.as_fd(), cfg) {
        Ok(recovered) => Ok(recovered),
        Err(err) => {
            let ctx = context(
                vm_id,
                kind,
                err.action.unwrap_or(LifecycleAction::Provision),
                &paths,
            );
            Err(failed_closed(&ctx, MarkerResult::FailedClosed, err.reason))
        }
    }
}

// ---------------------------------------------------------------------
// Public lifecycle actions
// ---------------------------------------------------------------------

/// `(paths, secrets_fd, generations_fd, lock, recovered_prior_transaction, marker)`.
type OpenAndRecoverOk = (
    SecretsLifecyclePaths,
    OwnedFd,
    OwnedFd,
    LockGuard,
    bool,
    Option<MarkerData>,
);

fn open_and_recover(
    vm_id: &str,
    kind: SecretKind,
    action: LifecycleAction,
    cfg: &SecretsLifecycleConfig,
) -> Result<OpenAndRecoverOk, SecretsLifecycleError> {
    let paths = derive_paths(vm_id, kind, cfg).map_err(|reason| {
        let ctx = SecretsLifecycleAuditContext {
            vm_id: vm_id.to_owned(),
            kind,
            action,
            base_dir_hash: String::new(),
        };
        denied(&ctx, reason)
    })?;
    let ctx = context(vm_id, kind, action, &paths);
    let (secrets_fd, generations_fd) =
        open_or_create_secrets_dir(&paths, cfg).map_err(|reason| denied(&ctx, reason))?;
    let lock = acquire_lock(secrets_fd.as_fd(), cfg).map_err(|reason| denied(&ctx, reason))?;
    let recovered = match recover_if_needed(secrets_fd.as_fd(), generations_fd.as_fd(), cfg) {
        Ok(recovered) => recovered,
        Err(err) => {
            let recovered_ctx = context(vm_id, kind, err.action.unwrap_or(action), &paths);
            return Err(failed_closed(
                &recovered_ctx,
                MarkerResult::FailedClosed,
                err.reason,
            ));
        }
    };
    let marker = read_marker(secrets_fd.as_fd()).map_err(|reason| denied(&ctx, reason))?;
    Ok((paths, secrets_fd, generations_fd, lock, recovered, marker))
}

/// Provision fresh generation-1 material. Fails closed
/// ([`FailReason::AlreadyProvisioned`], denied) if the marker already
/// records an active (non-retired) generation, and
/// ([`FailReason::GenerationConflict`], failed) if the physical tree
/// is non-empty without a matching active marker (an anomaly this
/// module refuses to silently adopt).
pub fn provision(
    vm_id: &str,
    kind: SecretKind,
    material: SecretMaterial,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let (paths, secrets_fd, generations_fd, _lock, recovered, marker) =
        open_and_recover(vm_id, kind, LifecycleAction::Provision, cfg)?;
    let ctx = context(vm_id, kind, LifecycleAction::Provision, &paths);

    if let Some(marker) = &marker
        && marker.active.is_some()
        && !marker.retired
    {
        return Err(denied(&ctx, FailReason::AlreadyProvisioned));
    }
    let existing = enumerate_and_validate_generation_tree(generations_fd.as_fd())
        .map_err(|reason| failed_closed(&ctx, MarkerResult::FailedClosed, reason))?;
    if !existing.is_empty() {
        return Err(failed_closed(
            &ctx,
            MarkerResult::FailedClosed,
            FailReason::GenerationConflict,
        ));
    }

    let digest_hex = material.digest_hex();
    let intent = PromoteIntent {
        vm: vm_id.to_owned(),
        kind: kind.as_slug().to_owned(),
        action: LifecycleAction::Provision,
        to_epoch: 1,
        create_epoch: true,
        stage_name: random_stage_name(),
        expected_digest_hex: digest_hex.clone(),
        expected_identity: None,
        carry_previous: None,
        prune_epoch: None,
        new_high_water_epoch: 1,
        first_provisioned_ms: now_ms(),
        phase: PromotePhase::Planned,
    };
    write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, MarkerResult::FailedClosed, reason))?;
    match execute_promote(
        secrets_fd.as_fd(),
        generations_fd.as_fd(),
        cfg,
        intent,
        Some(&material),
    ) {
        Ok(PromoteOutcome::Completed(new_marker)) => Ok(SecretsLifecycleAuditFields::provisioned(
            &ctx,
            new_marker.high_water_epoch,
            digest_hex,
            recovered,
        )),
        Ok(PromoteOutcome::AbortedNoMaterial) => Err(failed_closed(
            &ctx,
            MarkerResult::FailedClosed,
            FailReason::MaterialWriteFailed,
        )),
        Err(reason) => Err(failed_closed(&ctx, MarkerResult::FailedClosed, reason)),
    }
}

/// Retained generations after a successful rotate/rollback: exactly
/// `previous`'s epoch, when one exists.
fn retained_from_previous(previous: &Option<MaterialIdentity>) -> Vec<u64> {
    previous.as_ref().map(|p| vec![p.epoch]).unwrap_or_default()
}

/// Create and activate a new generation. Requires an active,
/// non-retired marker (else [`FailReason::NotProvisioned`], denied).
/// Allocates `to_epoch = high_water_epoch + 1` — never
/// `current_epoch + 1` — so a rotate issued after a rollback can
/// never collide with a still-materialised newer epoch the rollback
/// moved away from.
pub fn rotate(
    vm_id: &str,
    kind: SecretKind,
    material: SecretMaterial,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let (paths, secrets_fd, generations_fd, _lock, recovered, marker) =
        open_and_recover(vm_id, kind, LifecycleAction::Rotate, cfg)?;
    let ctx = context(vm_id, kind, LifecycleAction::Rotate, &paths);

    let marker = match marker {
        Some(marker) if marker.active.is_some() && !marker.retired => marker,
        _ => return Err(denied(&ctx, FailReason::NotProvisioned)),
    };
    let active = marker.active.clone().expect("checked above");
    let to_epoch = marker.high_water_epoch + 1;
    let digest_hex = material.digest_hex();
    let intent = PromoteIntent {
        vm: vm_id.to_owned(),
        kind: kind.as_slug().to_owned(),
        action: LifecycleAction::Rotate,
        to_epoch,
        create_epoch: true,
        stage_name: random_stage_name(),
        expected_digest_hex: digest_hex.clone(),
        expected_identity: None,
        carry_previous: Some(active),
        prune_epoch: marker.previous.as_ref().map(|p| p.epoch),
        new_high_water_epoch: to_epoch,
        first_provisioned_ms: marker.first_provisioned_ms,
        phase: PromotePhase::Planned,
    };
    write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, MarkerResult::FailedClosed, reason))?;
    match execute_promote(
        secrets_fd.as_fd(),
        generations_fd.as_fd(),
        cfg,
        intent,
        Some(&material),
    ) {
        Ok(PromoteOutcome::Completed(new_marker)) => Ok(SecretsLifecycleAuditFields::rotated(
            &ctx,
            new_marker.active.as_ref().map(|i| i.epoch).unwrap_or(0),
            new_marker.high_water_epoch,
            retained_from_previous(&new_marker.previous),
            digest_hex,
            recovered,
        )),
        Ok(PromoteOutcome::AbortedNoMaterial) => Err(failed_closed(
            &ctx,
            MarkerResult::FailedClosed,
            FailReason::MaterialWriteFailed,
        )),
        Err(reason) => Err(failed_closed(&ctx, MarkerResult::FailedClosed, reason)),
    }
}

/// Swap `current` back to the retained `previous` generation.
/// Requires a `previous` entry (else [`FailReason::NoRollbackTarget`],
/// denied). Tamper-verifies the retained generation's full recorded
/// identity before promoting it. The generation being rolled back
/// *from* becomes the new `previous` (so a rollback may itself be
/// rolled back / "rolled forward"); `high_water_epoch` is unchanged
/// (rollback never grows the monotonic mark) and nothing is pruned.
pub fn rollback(
    vm_id: &str,
    kind: SecretKind,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let (paths, secrets_fd, generations_fd, _lock, recovered, marker) =
        open_and_recover(vm_id, kind, LifecycleAction::Rollback, cfg)?;
    let ctx = context(vm_id, kind, LifecycleAction::Rollback, &paths);

    let marker = match marker {
        Some(marker) if marker.active.is_some() && !marker.retired => marker,
        _ => return Err(denied(&ctx, FailReason::NotProvisioned)),
    };
    let Some(previous) = marker.previous.clone() else {
        return Err(denied(&ctx, FailReason::NoRollbackTarget));
    };
    let active = marker.active.clone().expect("checked above");
    let intent = PromoteIntent {
        vm: vm_id.to_owned(),
        kind: kind.as_slug().to_owned(),
        action: LifecycleAction::Rollback,
        to_epoch: previous.epoch,
        create_epoch: false,
        stage_name: String::new(),
        expected_digest_hex: previous.digest_hex.clone(),
        expected_identity: Some(previous),
        carry_previous: Some(active),
        prune_epoch: None,
        new_high_water_epoch: marker.high_water_epoch,
        first_provisioned_ms: marker.first_provisioned_ms,
        phase: PromotePhase::Planned,
    };
    write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, MarkerResult::FailedClosed, reason))?;
    match execute_promote(
        secrets_fd.as_fd(),
        generations_fd.as_fd(),
        cfg,
        intent,
        None,
    ) {
        Ok(PromoteOutcome::Completed(new_marker)) => Ok(SecretsLifecycleAuditFields::rolled_back(
            &ctx,
            new_marker.active.as_ref().map(|i| i.epoch).unwrap_or(0),
            new_marker.high_water_epoch,
            retained_from_previous(&new_marker.previous),
            recovered,
        )),
        Ok(PromoteOutcome::AbortedNoMaterial) => Err(failed_closed(
            &ctx,
            MarkerResult::FailedClosed,
            FailReason::RecoveryAmbiguous,
        )),
        Err(reason) => Err(failed_closed(&ctx, MarkerResult::FailedClosed, reason)),
    }
}

/// Remove every generation and tombstone the marker. Always
/// anchored-enumerates and validates the *entire physical*
/// `generations/` tree first, regardless of what the marker claims —
/// an absent/tombstoned marker over non-empty storage is still fully
/// retired (never treated as already-clean), and an active marker
/// over empty storage fails closed
/// ([`FailReason::PreviouslyProvisionedMaterialMissing`]) rather than
/// silently accepting the discrepancy.
pub fn retire(
    vm_id: &str,
    kind: SecretKind,
    cfg: &SecretsLifecycleConfig,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let (paths, secrets_fd, generations_fd, _lock, recovered, marker) =
        open_and_recover(vm_id, kind, LifecycleAction::Retire, cfg)?;
    let ctx = context(vm_id, kind, LifecycleAction::Retire, &paths);

    sweep_stale_stage_dirs(generations_fd.as_fd(), None);
    let existing = enumerate_and_validate_generation_tree(generations_fd.as_fd())
        .map_err(|reason| failed_closed(&ctx, MarkerResult::FailedClosed, reason))?;

    let recorded_high_water = marker.as_ref().map(|m| m.high_water_epoch).unwrap_or(0);
    let marker_claims_active = marker
        .as_ref()
        .map(|m| m.active.is_some() && !m.retired)
        .unwrap_or(false);

    if existing.is_empty() {
        if marker_claims_active {
            // Marker says material should exist; the physical tree
            // disagrees. Fail closed rather than silently accepting
            // "clean" over an unexplained discrepancy.
            return Err(failed_closed(
                &ctx,
                MarkerResult::FailedClosed,
                FailReason::PreviouslyProvisionedMaterialMissing,
            ));
        }
        return Ok(SecretsLifecycleAuditFields::verified_clean(
            &ctx,
            recorded_high_water,
        ));
    }

    let high_water_epoch = recorded_high_water.max(existing.iter().copied().max().unwrap_or(0));
    let intent = RetireIntent {
        vm: vm_id.to_owned(),
        kind: kind.as_slug().to_owned(),
        epochs: existing,
        high_water_epoch,
        phase: RetirePhase::Enumerated,
    };
    write_txlog(secrets_fd.as_fd(), &TxLog::Retire(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, MarkerResult::FailedClosed, reason))?;
    match execute_retire(secrets_fd.as_fd(), generations_fd.as_fd(), intent) {
        Ok(()) => Ok(SecretsLifecycleAuditFields::retired(
            &ctx,
            high_water_epoch,
            recovered,
        )),
        Err(reason) => Err(failed_closed(&ctx, MarkerResult::FailedClosed, reason)),
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn cfg_at(root: &std::path::Path) -> SecretsLifecycleConfig {
        SecretsLifecycleConfig {
            state_root: root.to_path_buf(),
            dir_mode: 0o700,
            file_mode: 0o600,
            owner_uid: None,
            owner_gid: None,
            lock_max_wait: Duration::from_millis(500),
            lock_poll_interval: Duration::from_millis(5),
        }
    }

    fn material(bytes: &[u8]) -> SecretMaterial {
        SecretMaterial::new(bytes.to_vec()).expect("valid material")
    }

    fn assert_valid(fields: &SecretsLifecycleAuditFields) {
        fields.validate().unwrap_or_else(|err| {
            panic!("audit fields failed validation: {err}\nfields: {fields:?}")
        });
    }

    // -------------------------------------------------------------
    // Happy path
    // -------------------------------------------------------------

    #[test]
    fn provision_rotate_rollback_retire_happy_path() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "alice";
        let kind = SecretKind::GuestSigningKey;

        let provisioned = provision(vm, kind, material(b"gen-1"), &cfg).unwrap();
        assert_valid(&provisioned);
        assert_eq!(provisioned.lineage_epoch, Some(1));
        assert_eq!(provisioned.high_water_epoch, Some(1));

        // Duplicate provision on an active lineage is denied.
        let err = provision(vm, kind, material(b"gen-1-again"), &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::AlreadyProvisioned);

        let rotated = rotate(vm, kind, material(b"gen-2"), &cfg).unwrap();
        assert_valid(&rotated);
        assert_eq!(rotated.lineage_epoch, Some(2));
        assert_eq!(rotated.high_water_epoch, Some(2));
        assert_eq!(rotated.retained_generations, vec![1]);

        let rotated_again = rotate(vm, kind, material(b"gen-3"), &cfg).unwrap();
        assert_valid(&rotated_again);
        assert_eq!(rotated_again.lineage_epoch, Some(3));
        assert_eq!(rotated_again.high_water_epoch, Some(3));
        // Rotation prunes everything but the immediately-previous
        // generation; epoch 1 is gone once epoch 3 supersedes epoch 2.
        assert_eq!(rotated_again.retained_generations, vec![2]);

        let rolled_back = rollback(vm, kind, &cfg).unwrap();
        assert_valid(&rolled_back);
        assert_eq!(rolled_back.lineage_epoch, Some(2));
        // Rollback never grows the monotonic high-water mark.
        assert_eq!(rolled_back.high_water_epoch, Some(3));
        assert_eq!(rolled_back.retained_generations, vec![3]);

        // Rolling back again ("redo") returns to epoch 3.
        let redo = rollback(vm, kind, &cfg).unwrap();
        assert_valid(&redo);
        assert_eq!(redo.lineage_epoch, Some(3));
        assert_eq!(redo.high_water_epoch, Some(3));
        assert_eq!(redo.retained_generations, vec![2]);

        let retired = retire(vm, kind, &cfg).unwrap();
        assert_valid(&retired);
        assert_eq!(retired.high_water_epoch, Some(3));

        // Retiring an already-retired (never re-provisioned) lineage
        // is a clean no-op, not an error.
        let verified = retire(vm, kind, &cfg).unwrap();
        assert_valid(&verified);
    }

    #[test]
    fn rotate_without_provision_is_denied() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let err = rotate("bob", SecretKind::TpmBoundCredential, material(b"x"), &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::NotProvisioned);
    }

    #[test]
    fn rollback_without_previous_generation_is_denied() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "carol";
        let kind = SecretKind::SecurityKeyChannelState;
        provision(vm, kind, material(b"only-gen"), &cfg).unwrap();
        let err = rollback(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::NoRollbackTarget);
    }

    #[test]
    fn retire_never_provisioned_is_verified_clean() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let fields = retire("never-provisioned", SecretKind::GuestSigningKey, &cfg).unwrap();
        assert_valid(&fields);
        assert_eq!(fields.high_water_epoch, Some(0));
    }

    #[test]
    fn rotate_after_rollback_never_reuses_a_still_materialised_epoch() {
        // Monotonic high-water allocation: rotate() must allocate
        // strictly past every epoch the marker has ever recorded,
        // even one a rollback moved *away* from and which therefore
        // still physically exists on disk.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "dana";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();
        rotate(vm, kind, material(b"e2"), &cfg).unwrap();
        // Now: active=2, previous=1, high_water=2.
        let after_rollback = rollback(vm, kind, &cfg).unwrap();
        assert_eq!(after_rollback.lineage_epoch, Some(1));
        // active=1, previous=2 (still on disk, unpruned), high_water=2.
        let after_rotate = rotate(vm, kind, material(b"e3"), &cfg).unwrap();
        // Must allocate epoch 3, never epoch 2 (which is still the
        // `previous` generation's live directory at this point).
        assert_eq!(after_rotate.lineage_epoch, Some(3));
        assert_eq!(after_rotate.high_water_epoch, Some(3));
    }

    // -------------------------------------------------------------
    // Crash recovery
    // -------------------------------------------------------------

    #[test]
    fn recovery_completes_a_promote_left_at_planned() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "erin";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name: random_stage_name(),
            expected_digest_hex: material(b"e2").digest_hex(),
            expected_identity: None,
            carry_previous: read_marker(secrets_fd.as_fd())
                .unwrap()
                .and_then(|m| m.active),
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent)).unwrap();
        drop(secrets_fd);
        drop(generations_fd);

        // No material handed to recovery: a `Planned` promote with no
        // staged bytes yet must abort (discard) rather than fabricate
        // material out of thin air.
        let recovered = recover_in_flight_transaction(vm, kind, &cfg).unwrap();
        assert!(recovered);
        let fields = rotate(vm, kind, material(b"e2-redo"), &cfg).unwrap();
        assert_valid(&fields);
        assert_eq!(fields.lineage_epoch, Some(2));
    }

    #[test]
    fn recovery_forward_completes_a_promote_left_at_current_promoted() {
        // Once `current` has been swapped, recovery must always
        // complete forward to a durable marker — never revert.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "frank";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e2");
        let stage_name = random_stage_name();
        assert!(
            ensure_staged(
                generations_fd.as_fd(),
                &cfg,
                &stage_name,
                &mat.digest_hex(),
                Some(&mat),
            )
            .unwrap()
        );
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2).unwrap();
        atomic_swap_current(secrets_fd.as_fd(), 2).unwrap();
        let prior_active = read_marker(secrets_fd.as_fd())
            .unwrap()
            .and_then(|m| m.active);
        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: prior_active,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::CurrentPromoted,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent)).unwrap();
        drop(secrets_fd);
        drop(generations_fd);

        let recovered = recover_in_flight_transaction(vm, kind, &cfg).unwrap();
        assert!(recovered);

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert_eq!(marker.active.unwrap().epoch, 2);
        assert!(read_txlog(secrets_fd.as_fd()).unwrap().is_none());
    }

    #[test]
    fn recovery_completes_a_retire_left_at_current_removed() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "gina";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        remove_current_symlink(secrets_fd.as_fd()).unwrap();
        let intent = RetireIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            epochs: vec![1],
            high_water_epoch: 1,
            phase: RetirePhase::CurrentRemoved,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Retire(intent)).unwrap();
        drop(secrets_fd);
        drop(generations_fd);

        let recovered = recover_in_flight_transaction(vm, kind, &cfg).unwrap();
        assert!(recovered);
        let fields = retire(vm, kind, &cfg).unwrap();
        assert_valid(&fields);
    }

    #[test]
    fn planned_promote_recovery_never_activates_partial_state_then_error() {
        // Regression guard for "never return error after silently
        // activating unrecoverable state": force recovery to fail
        // *after* `current` has already moved (a corrupted intent at
        // `CurrentPromoted`) and confirm the swap is never reverted —
        // the on-disk `current` link must keep resolving to the
        // (verifiably durable) epoch it was swapped to even though
        // the recovery call itself returns an error.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "harold";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e2");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2).unwrap();
        atomic_swap_current(secrets_fd.as_fd(), 2).unwrap();
        // Corrupt the recorded digest so `CurrentPromoted`'s identity
        // check cannot match — this must fail closed, not silently
        // re-swap `current` back to epoch 1.
        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name,
            expected_digest_hex: "0".repeat(64),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::CurrentPromoted,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent)).unwrap();
        let target_before = read_current_target(secrets_fd.as_fd());
        drop(secrets_fd);
        drop(generations_fd);

        let err = recover_in_flight_transaction(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::RecoveryAmbiguous);

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        // `current` must still point at epoch 2 — the activation that
        // already happened is never rolled back by a failed recovery.
        assert_eq!(read_current_target(secrets_fd.as_fd()), target_before);
        assert_eq!(read_current_target(secrets_fd.as_fd()), Some(2));
    }

    // -------------------------------------------------------------
    // Tamper detection
    // -------------------------------------------------------------

    #[test]
    fn rollback_detects_owner_mode_and_digest_tampering() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "iris";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();
        rotate(vm, kind, material(b"e2"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let material_path = paths
            .kind_root
            .join(GENERATIONS_DIR_NAME)
            .join("1")
            .join(MATERIAL_FILE_NAME);

        // Digest tamper: overwrite the retained generation's bytes.
        std::fs::write(&material_path, b"tampered-bytes").unwrap();
        let err = rollback(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::IdentityDigestMismatch);

        // Restore then tamper the mode instead.
        std::fs::write(&material_path, b"e1").unwrap();
        let mut perms = std::fs::metadata(&material_path).unwrap().permissions();
        perms.set_mode(0o666);
        std::fs::set_permissions(&material_path, perms).unwrap();
        let err = rollback(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::IdentityModeMismatch);
    }

    #[test]
    fn rollback_detects_hardlinked_material() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "jack";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();
        rotate(vm, kind, material(b"e2"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        let material_path = gen_dir.join(MATERIAL_FILE_NAME);
        let extra_link = gen_dir.join("extra-hardlink");
        std::fs::hard_link(&material_path, &extra_link).unwrap();

        let err = rollback(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::IdentityLinkCountMismatch);
    }

    // -------------------------------------------------------------
    // Retirement tree anomaly / fail-closed enumeration
    // -------------------------------------------------------------

    #[test]
    fn retire_fails_closed_on_unexpected_entry_and_deletes_nothing() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "karen";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let generations_dir = paths.kind_root.join(GENERATIONS_DIR_NAME);
        std::fs::write(generations_dir.join("not-an-epoch"), b"bogus").unwrap();

        let err = retire(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::RetirementTreeAnomaly);
        // Nothing was deleted: the legitimate generation is intact.
        assert!(generations_dir.join("1").join(MATERIAL_FILE_NAME).is_file());
        assert!(generations_dir.join("not-an-epoch").is_file());
    }

    #[test]
    fn retire_fails_closed_on_hardlinked_material_in_tree() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "leo";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::hard_link(
            gen_dir.join(MATERIAL_FILE_NAME),
            gen_dir.join("extra-hardlink"),
        )
        .unwrap();

        let err = retire(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::RetirementTreeAnomaly);
    }

    #[test]
    fn retire_fails_closed_when_marker_active_but_tree_already_empty() {
        // "Missing marker never implies clean storage" — and
        // symmetrically, an active marker over physically-empty
        // storage must never be silently accepted as clean either.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "mabel";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::remove_file(gen_dir.join(MATERIAL_FILE_NAME)).unwrap();
        std::fs::remove_dir(&gen_dir).unwrap();

        let err = retire(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::PreviouslyProvisionedMaterialMissing);
    }

    #[test]
    fn retire_on_nonempty_tree_with_missing_marker_still_retires() {
        // A missing marker file must never be interpreted as "already
        // clean" when the physical generation tree is non-empty.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "nate";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        std::fs::remove_file(paths.kind_root.join(MARKER_FILE_NAME)).unwrap();

        let fields = retire(vm, kind, &cfg).unwrap();
        assert_valid(&fields);
        let generations_dir = paths.kind_root.join(GENERATIONS_DIR_NAME);
        assert_eq!(std::fs::read_dir(&generations_dir).unwrap().count(), 0);
    }

    // -------------------------------------------------------------
    // Locking / concurrency
    // -------------------------------------------------------------

    #[test]
    fn concurrent_holder_blocks_a_second_lock_attempt() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "olive";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        // Hold the exclusive lock open across the whole scope below.
        let _held = acquire_lock(secrets_fd.as_fd(), &cfg).unwrap();

        let short_wait_cfg = SecretsLifecycleConfig {
            lock_max_wait: Duration::from_millis(60),
            lock_poll_interval: Duration::from_millis(5),
            ..cfg.clone()
        };
        let err = rotate(vm, kind, material(b"e2"), &short_wait_cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::LockUnavailable);
    }

    // -------------------------------------------------------------
    // Zeroization / material hygiene
    // -------------------------------------------------------------

    #[test]
    fn secret_material_rejects_empty_and_oversized_input() {
        assert_eq!(
            SecretMaterial::new(Vec::new()).unwrap_err(),
            FailReason::InvalidMaterial
        );
        assert_eq!(
            SecretMaterial::new(vec![0u8; SecretMaterial::MAX_LEN + 1]).unwrap_err(),
            FailReason::InvalidMaterial
        );
    }

    #[test]
    fn secret_material_debug_never_prints_bytes() {
        let mat = material(b"super-secret-bytes");
        let rendered = format!("{mat:?}");
        assert!(!rendered.contains("super-secret-bytes"));
        assert!(rendered.contains("len"));
    }

    // -------------------------------------------------------------
    // Path / vm-id validation
    // -------------------------------------------------------------

    #[test]
    fn invalid_vm_id_is_rejected_before_any_filesystem_access() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let err = provision(
            "Not-Valid!",
            SecretKind::GuestSigningKey,
            material(b"x"),
            &cfg,
        )
        .unwrap_err();
        assert_eq!(err.reason, FailReason::InvalidVmId);
        // Nothing was created under the state root.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
