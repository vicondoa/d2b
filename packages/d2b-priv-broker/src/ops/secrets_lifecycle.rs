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
    /// Every `.stage-*` staging directory found in the same
    /// enumeration. Staged entries are real secret-tree contents (a
    /// stage dir contains, or is about to contain, a copy of
    /// caller-supplied material bytes): they are deleted exactly like
    /// `epochs`, with every failure propagated, never as a
    /// best-effort sweep.
    pub stage_names: Vec<String>,
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

/// `recovered` must reflect whether this call already successfully
/// drained a leftover crashed transaction (via [`open_and_recover`])
/// before reaching this denial — see
/// [`SecretsLifecycleAuditFields::denied`].
fn denied(
    ctx: &SecretsLifecycleAuditContext,
    recovered: bool,
    reason: FailReason,
) -> SecretsLifecycleError {
    SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::denied(ctx, recovered, reason),
    }
}

/// `recovered` must reflect whether this call already successfully
/// drained a leftover crashed transaction before this failure — see
/// [`SecretsLifecycleAuditFields::failed`].
fn failed_closed(
    ctx: &SecretsLifecycleAuditContext,
    recovered: bool,
    marker_result: MarkerResult,
    reason: FailReason,
) -> SecretsLifecycleError {
    SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(ctx, recovered, marker_result, reason),
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
    // Metadata directories (the per-VM directory, the per-kind
    // secrets root, and `generations/`) are trusted-broker-owned
    // bookkeeping, never a consumer-owned material leaf: `cfg.
    // owner_uid`/`owner_gid` apply only to the `material` file
    // written by `ensure_staged`. `ensure_dir` only reasserts mode on
    // every call (it does not re-`chown` when passed `None, None`),
    // so ownership drift is independently verified here rather than
    // trusted from construction alone.
    path_safe::ensure_dir(vm_dir, cfg.dir_mode, None, None)
        .map_err(|_| FailReason::SecretsDirOpenFailed)?;
    let vm_dir_fd =
        path_safe::open_dir_path_safe(vm_dir).map_err(|_| FailReason::SecretsDirOpenFailed)?;
    let vm_dir_stat =
        path_safe::fstat_fd(vm_dir_fd.as_fd()).map_err(|_| FailReason::BrokerOwnershipViolation)?;
    verify_broker_owned(&vm_dir_stat, nix::libc::S_IFDIR, cfg.dir_mode)?;
    drop(vm_dir_fd);

    path_safe::ensure_dir(&paths.kind_root, cfg.dir_mode, None, None)
        .map_err(|_| FailReason::SecretsDirOpenFailed)?;
    let secrets_fd = path_safe::open_dir_path_safe(&paths.kind_root)
        .map_err(|_| FailReason::SecretsDirOpenFailed)?;
    let secrets_stat = path_safe::fstat_fd(secrets_fd.as_fd())
        .map_err(|_| FailReason::BrokerOwnershipViolation)?;
    verify_broker_owned(&secrets_stat, nix::libc::S_IFDIR, cfg.dir_mode)?;
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
    let generations_stat = path_safe::fstat_fd(generations_fd.as_fd())
        .map_err(|_| FailReason::BrokerOwnershipViolation)?;
    verify_broker_owned(&generations_stat, nix::libc::S_IFDIR, cfg.dir_mode)?;
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
    fsync_fd(secrets_fd)?;
    Ok(())
}

/// Cross-field semantic validation of a leftover `txlog`, bound to the
/// exact `(vm, kind)` directory it was found in. Run immediately on
/// read, *before* any recovery attempt: a txlog naming a different
/// `(vm, kind)`, an internally inconsistent phase/epoch/stage/
/// high-water combination, or an action a `PromoteIntent` cannot
/// legally carry is [`FailReason::IntentCorrupt`] rather than
/// something recovery tries to interpret.
fn validate_txlog_semantics(log: &TxLog, vm_id: &str, kind: SecretKind) -> Result<(), FailReason> {
    match log {
        TxLog::Promote(intent) => validate_promote_intent_semantics(intent, vm_id, kind),
        TxLog::Retire(intent) => validate_retire_intent_semantics(intent, vm_id, kind),
    }
}

fn validate_promote_intent_semantics(
    intent: &PromoteIntent,
    vm_id: &str,
    kind: SecretKind,
) -> Result<(), FailReason> {
    if intent.vm != vm_id || intent.kind != kind.as_slug() {
        return Err(FailReason::IntentCorrupt);
    }
    if intent.to_epoch == 0 {
        return Err(FailReason::IntentCorrupt);
    }
    let action_wants_create = match intent.action {
        LifecycleAction::Provision | LifecycleAction::Rotate => true,
        LifecycleAction::Rollback => false,
        LifecycleAction::Retire => return Err(FailReason::IntentCorrupt),
    };
    if intent.create_epoch != action_wants_create {
        return Err(FailReason::IntentCorrupt);
    }
    if intent.create_epoch {
        if intent.stage_name.is_empty() || !is_well_formed_stage_name(&intent.stage_name) {
            return Err(FailReason::IntentCorrupt);
        }
        if intent.expected_identity.is_some() {
            return Err(FailReason::IntentCorrupt);
        }
    } else {
        if !intent.stage_name.is_empty() {
            return Err(FailReason::IntentCorrupt);
        }
        match &intent.expected_identity {
            Some(identity)
                if identity.epoch == intent.to_epoch
                    && identity.digest_hex == intent.expected_digest_hex => {}
            _ => return Err(FailReason::IntentCorrupt),
        }
    }
    if intent.new_high_water_epoch < intent.to_epoch {
        return Err(FailReason::IntentCorrupt);
    }
    if let Some(previous) = &intent.carry_previous
        && previous.epoch == intent.to_epoch
    {
        return Err(FailReason::IntentCorrupt);
    }
    if let Some(prune) = intent.prune_epoch
        && (prune == intent.to_epoch
            || intent
                .carry_previous
                .as_ref()
                .is_some_and(|p| p.epoch == prune))
    {
        return Err(FailReason::IntentCorrupt);
    }
    Ok(())
}

fn validate_retire_intent_semantics(
    intent: &RetireIntent,
    vm_id: &str,
    kind: SecretKind,
) -> Result<(), FailReason> {
    if intent.vm != vm_id || intent.kind != kind.as_slug() {
        return Err(FailReason::IntentCorrupt);
    }
    let mut sorted_epochs = intent.epochs.clone();
    sorted_epochs.sort_unstable();
    sorted_epochs.dedup();
    if sorted_epochs != intent.epochs {
        return Err(FailReason::IntentCorrupt);
    }
    if intent
        .epochs
        .iter()
        .any(|&e| e == 0 || e > intent.high_water_epoch)
    {
        return Err(FailReason::IntentCorrupt);
    }
    let mut sorted_stage_names = intent.stage_names.clone();
    sorted_stage_names.sort();
    sorted_stage_names.dedup();
    if sorted_stage_names != intent.stage_names
        || intent
            .stage_names
            .iter()
            .any(|s| !is_well_formed_stage_name(s))
    {
        return Err(FailReason::IntentCorrupt);
    }
    Ok(())
}

fn read_txlog(
    secrets_fd: BorrowedFd<'_>,
    vm_id: &str,
    kind: SecretKind,
) -> Result<Option<TxLog>, FailReason> {
    let owned = borrowed_to_owned(secrets_fd)?;
    let fd = match path_safe::open_file_at_safe(&owned, TXLOG_FILE_NAME, nix::libc::O_RDONLY) {
        Ok(fd) => fd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(FailReason::IntentCorrupt),
    };
    let bytes = read_fd_to_end(fd.as_fd()).map_err(|_| FailReason::IntentCorrupt)?;
    let log: TxLog = serde_json::from_slice(&bytes).map_err(|_| FailReason::IntentCorrupt)?;
    validate_txlog_semantics(&log, vm_id, kind)?;
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
    fsync_fd(secrets_fd)?;
    Ok(())
}

fn remove_txlog(secrets_fd: BorrowedFd<'_>) -> Result<(), FailReason> {
    let owned = borrowed_to_owned(secrets_fd)?;
    match path_safe::remove_path_safe(&owned, TXLOG_FILE_NAME) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(FailReason::IntentWriteFailed),
    }
    fsync_fd(secrets_fd)?;
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

// Test-only fault injection seam: when enabled (see
// `tests::FsyncFaultGuard`), every `fsync_fd` call fails with
// `FailReason::FsyncFailed` instead of touching the filesystem, so
// tests can prove each phase-advancement point genuinely blocks on a
// durable sync rather than merely calling `fsync` and ignoring the
// result.
#[cfg(test)]
thread_local! {
    static FSYNC_SHOULD_FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn fsync_fd(fd: BorrowedFd<'_>) -> Result<(), FailReason> {
    #[cfg(test)]
    if FSYNC_SHOULD_FAIL.with(|c| c.get()) {
        return Err(FailReason::FsyncFailed);
    }
    rustix::fs::fsync(fd).map_err(|_| FailReason::FsyncFailed)
}

fn io_from_rustix(err: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(err.raw_os_error())
}

// ---------------------------------------------------------------------
// Broker-trusted ownership (metadata dirs, lock file)
// ---------------------------------------------------------------------

/// The trusted "broker owner" identity for every path this module
/// manages *except* a `material` file leaf: metadata directories
/// (`kind_root`, `generations/`), the marker, the txlog, and the lock
/// file. Since this module only ever runs inside the broker process,
/// "owned by the broker" and "owned by this process's effective
/// uid/gid" are the same statement — a path this module is about to
/// trust for locking or transaction bookkeeping that reports some
/// *other* owner was not created by this code path and must never be
/// trusted, regardless of what `cfg.owner_uid`/`cfg.owner_gid` (the
/// separate, *consumer*-facing identity applied only to material file
/// leaves — see [`ensure_staged`]) say.
fn broker_identity() -> (u32, u32) {
    (
        rustix::process::geteuid().as_raw(),
        rustix::process::getegid().as_raw(),
    )
}

/// Verify `stat` is exactly `expected_type` (`S_IFDIR`/`S_IFREG`),
/// owned by the trusted broker identity, has exactly `expected_mode`
/// permission bits, and (for a regular file) exactly one hard link.
/// Any mismatch is [`FailReason::BrokerOwnershipViolation`] — this is
/// the check that makes broker-owned metadata/lock paths
/// "non-replaceable": a foreign-owned or hard-linked stand-in is
/// refused rather than silently trusted.
fn verify_broker_owned(
    stat: &nix::libc::stat,
    expected_type: nix::libc::mode_t,
    expected_mode: u32,
) -> Result<(), FailReason> {
    let (uid, gid) = broker_identity();
    if (stat.st_mode & nix::libc::S_IFMT) != expected_type {
        return Err(FailReason::BrokerOwnershipViolation);
    }
    if stat.st_uid != uid || stat.st_gid != gid {
        return Err(FailReason::BrokerOwnershipViolation);
    }
    if (stat.st_mode & 0o7777) != expected_mode {
        return Err(FailReason::BrokerOwnershipViolation);
    }
    if expected_type == nix::libc::S_IFREG && stat.st_nlink != 1 {
        return Err(FailReason::BrokerOwnershipViolation);
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Cross-process lock
// ---------------------------------------------------------------------

#[derive(Debug)]
pub struct LockGuard {
    _file: std::fs::File,
}

/// Acquire the cross-process exclusive lock for one `(vm, kind)`
/// directory. Before trusting the `flock`, this verifies: the
/// anchored parent (`secrets_fd`, i.e. `kind_root`) is a trusted-
/// broker-owned directory with the expected mode; the lock file
/// itself is a trusted-broker-owned regular file with the expected
/// mode and exactly one hard link (never a hard-link plant); and,
/// immediately after the `flock` succeeds, that the directory entry
/// named `lock` still resolves (by `(dev, ino)`) to the exact file
/// this call opened and locked — guarding against the classic
/// flock-via-path race where another actor unlinks-and-recreates the
/// lock file between this call's `open` and its `flock`, which would
/// let two callers each hold an uncontended lock on two different,
/// now-unlinked-vs-live inodes of the same name.
fn acquire_lock(
    secrets_fd: BorrowedFd<'_>,
    cfg: &SecretsLifecycleConfig,
) -> Result<LockGuard, FailReason> {
    let owned = borrowed_to_owned(secrets_fd)?;
    let parent_stat =
        path_safe::fstat_fd(secrets_fd).map_err(|_| FailReason::BrokerOwnershipViolation)?;
    verify_broker_owned(&parent_stat, nix::libc::S_IFDIR, cfg.dir_mode)?;

    let fd = path_safe::create_file_at_safe(
        &owned,
        LOCK_FILE_NAME,
        nix::libc::O_RDWR | nix::libc::O_CREAT,
        FILE_MODE_DEFAULT,
    )
    .map_err(|_| FailReason::LockUnavailable)?;
    let lock_stat =
        path_safe::fstat_fd(fd.as_fd()).map_err(|_| FailReason::BrokerOwnershipViolation)?;
    verify_broker_owned(&lock_stat, nix::libc::S_IFREG, FILE_MODE_DEFAULT)?;

    let file: std::fs::File = fd.into();
    let deadline = Instant::now() + cfg.lock_max_wait;
    loop {
        match flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
            Ok(()) => {
                // Re-stat by name (anchored beneath the already-
                // verified `secrets_fd` parent) and compare `(dev,
                // ino)` against the fd we just locked: if another
                // actor swapped the `lock` entry between our open and
                // our successful flock, the identity will differ and
                // we must not trust this lock.
                let by_name = path_safe::fstatat_nofollow(&owned, LOCK_FILE_NAME)
                    .map_err(|_| FailReason::BrokerOwnershipViolation)?
                    .ok_or(FailReason::BrokerOwnershipViolation)?;
                let by_fd = path_safe::fstat_fd(file.as_fd())
                    .map_err(|_| FailReason::BrokerOwnershipViolation)?;
                if by_name.st_dev != by_fd.st_dev || by_name.st_ino != by_fd.st_ino {
                    return Err(FailReason::BrokerOwnershipViolation);
                }
                return Ok(LockGuard { _file: file });
            }
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

/// The exact closed syntax [`random_stage_name`] always produces:
/// `STAGE_PREFIX` followed by exactly 32 lowercase-hex characters, as
/// one single path component with no separator or parent-reference
/// syntax. Used to validate every stage name that originates from a
/// txlog (fresh construction *and* recovery reload) before it is ever
/// used as an anchored path component -- a corrupted or hand-edited
/// txlog containing `/`, `..`, or any other value must never reach an
/// `openat`-family call.
fn is_well_formed_stage_name(name: &str) -> bool {
    if name.contains('/') || name.contains("..") {
        return false;
    }
    let Some(suffix) = name.strip_prefix(STAGE_PREFIX) else {
        return false;
    };
    suffix.len() == 32
        && suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_lowercase() && b.is_ascii_hexdigit()))
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

/// Validate a `.stage-*` directory's contents are consistent with an
/// in-progress or superseded staging attempt: at most one entry,
/// which (if present) must be a regular file with exactly one hard
/// link (either the final `material` name, or a stray hidden
/// temp-file name a crash left behind mid-write via
/// `path_safe::atomic_replace_fd_with_owner`'s named-stage fallback).
/// Anything else (a subdirectory, a symlink, more than one entry, or
/// a hard-linked file) is an anomaly this refuses to silently delete.
fn validate_stage_dir_contents(generations_fd: &OwnedFd, name: &str) -> Result<(), FailReason> {
    let dir_fd = path_safe::open_at(
        generations_fd.as_fd(),
        Path::new(name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let dir_owned = borrowed_to_owned(dir_fd.as_fd())?;
    let inner = rustix::fs::Dir::read_from(dir_fd.as_fd())
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let mut seen: Option<String> = None;
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
        if seen.is_some() {
            return Err(FailReason::RetirementTreeAnomaly);
        }
        seen = Some(ename);
    }
    if let Some(ename) = seen {
        let stat = path_safe::fstatat_nofollow(&dir_owned, &ename)
            .map_err(|_| FailReason::RetirementTreeAnomaly)?
            .ok_or(FailReason::RetirementTreeAnomaly)?;
        if (stat.st_mode & nix::libc::S_IFMT) != nix::libc::S_IFREG {
            return Err(FailReason::RetirementTreeAnomaly);
        }
        if stat.st_nlink != 1 {
            return Err(FailReason::RetirementTreeAnomaly);
        }
    }
    Ok(())
}

/// Immediately before deleting a `.stage-*` directory, re-stat it
/// (nofollow, anchored) and confirm it is still exactly a validated
/// stage directory. Mirrors [`revalidate_generation_before_delete`]
/// for stage directories: a missing entry is tolerated (another pass,
/// or this same pass on retry, already removed it); any other anomaly
/// injected between enumeration and deletion -- an extra entry, a
/// subdirectory, a hard-linked file -- fails closed rather than
/// deleting a directory that no longer matches what was originally
/// validated.
fn revalidate_stage_before_delete(
    generations_fd: BorrowedFd<'_>,
    name: &str,
) -> Result<(), FailReason> {
    let generations_owned = borrowed_to_owned(generations_fd)?;
    match path_safe::fstatat_nofollow(&generations_owned, name) {
        Ok(Some(_)) => validate_stage_dir_contents(&generations_owned, name),
        Ok(None) => Ok(()),
        Err(_) => Err(FailReason::RetirementTreeAnomaly),
    }
}

/// Discard a partially-staged `.stage-*` directory left behind by a
/// crashed attempt that never reached a fully-staged, digest-matching
/// state (so [`ensure_staged`] returned `Ok(false)` on recovery
/// re-entry and there is no material to promote). No-ops if the stage
/// directory is absent (the common, no-crash path, or a retry that
/// already discarded it). If present, validates its contents are
/// consistent with a partial/superseded staging attempt and deletes
/// it -- propagating any validation or deletion failure so the caller
/// never abandons the txlog record while leaving an orphaned,
/// unvalidated stage directory on disk.
fn discard_partial_stage_if_present(
    generations_fd: BorrowedFd<'_>,
    stage_name: &str,
) -> Result<(), FailReason> {
    let generations_owned = borrowed_to_owned(generations_fd)?;
    match path_safe::fstatat_nofollow(&generations_owned, stage_name) {
        Ok(Some(_)) => {
            validate_stage_dir_contents(&generations_owned, stage_name)?;
            remove_stage_dir(generations_fd, stage_name)
        }
        Ok(None) => Ok(()),
        Err(_) => Err(FailReason::RetirementTreeAnomaly),
    }
}

/// Fully delete one `.stage-*` directory: enumerate every child,
/// delete each one (propagating any failure other than the child
/// already being absent), prove the directory is observably empty,
/// fsync it, then remove the directory itself and fsync its parent.
/// Staged entries are real secret-tree contents — this is never a
/// best-effort sweep, unlike the old `36d9dcf8`/`dab583f1` design's
/// `sweep_stale_stage_dirs`: every failure here is propagated and
/// blocks the caller from advancing.
fn remove_stage_dir(generations_fd: BorrowedFd<'_>, name: &str) -> Result<(), FailReason> {
    let generations_owned = borrowed_to_owned(generations_fd)?;
    let dir_fd = match path_safe::open_at(
        generations_fd,
        Path::new(name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    ) {
        Ok(fd) => fd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // Already gone. This may be a durable retry re-observing
            // a completed deletion, or it may be a retry re-observing
            // a deletion whose *own* trailing
            // `fsync_fd(generations_fd)` failed before the caller
            // could advance the phase. Retry that durability barrier
            // now rather than assuming the earlier attempt's fsync
            // (which we cannot distinguish from "never happened")
            // actually completed.
            fsync_fd(generations_fd)?;
            return Ok(());
        }
        Err(_) => return Err(FailReason::RetirementTreeAnomaly),
    };
    let dir_owned = borrowed_to_owned(dir_fd.as_fd())?;
    let entries = rustix::fs::Dir::read_from(dir_fd.as_fd())
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| FailReason::RetirementTreeAnomaly)?;
        let ename = entry
            .file_name()
            .to_str()
            .map_err(|_| FailReason::RetirementTreeAnomaly)?
            .to_owned();
        if ename != "." && ename != ".." {
            names.push(ename);
        }
    }
    drop(dir_fd);
    for ename in &names {
        match path_safe::remove_path_safe(&dir_owned, ename) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(FailReason::RetirementTreeAnomaly),
        }
    }
    // Prove empty before removing: a deletion that silently left a
    // child behind must never result in the parent being removed
    // anyway.
    let remaining = rustix::fs::Dir::read_from(dir_owned.as_fd())
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    for entry in remaining {
        let entry = entry.map_err(|_| FailReason::RetirementTreeAnomaly)?;
        let ename = entry
            .file_name()
            .to_str()
            .map_err(|_| FailReason::RetirementTreeAnomaly)?
            .to_owned();
        if ename != "." && ename != ".." {
            return Err(FailReason::RetirementNotProvablyEmpty);
        }
    }
    fsync_fd(dir_owned.as_fd())?;
    match path_safe::remove_path_safe(&generations_owned, name) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(FailReason::RetirementTreeAnomaly),
    }
    fsync_fd(generations_fd)?;
    Ok(())
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
        // safe to discard and let the caller decide (abort/retry). The
        // deletion itself must fully succeed and propagate any error;
        // a stage directory is real secret material and is never
        // swept on a best-effort basis.
        remove_stage_dir(generations_owned.as_fd(), stage_name)?;
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
    fsync_fd(stage_fd.as_fd())?;
    fsync_fd(generations_fd)?;
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
    expected_digest_hex: &str,
) -> Result<(), FailReason> {
    let epoch_name = to_epoch.to_string();
    let generations_owned = borrowed_to_owned(generations_fd)?;
    if let Ok(existing_digest) = digest_of_material_dir(&generations_owned, &epoch_name) {
        // A directory already occupies this epoch number (e.g. a
        // recovery re-entry after the rename completed but the phase
        // advance did not commit). It is only safe to treat this as
        // "already promoted" if its content is exactly the content we
        // intended to promote; any other content -- stale, foreign,
        // or from an aborted/rolled-back attempt -- must never be
        // silently accepted or overwritten.
        if existing_digest == expected_digest_hex {
            return Ok(());
        }
        return Err(FailReason::GenerationConflict);
    }
    rustix::fs::renameat(
        generations_fd,
        Path::new(stage_name),
        generations_fd,
        Path::new(&epoch_name),
    )
    .map_err(|_| FailReason::GenerationConflict)?;
    fsync_fd(generations_fd)?;
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

/// The exact literal symlink target text `atomic_swap_current` writes
/// for a given epoch: `generations/<epoch>`. This is a relative path
/// anchored at the per-(vm,kind) secrets directory; any other literal
/// text -- an absolute path, a path with extra segments, or a
/// differently-rooted relative path -- must never be treated as
/// resolving to that epoch, even if its final component happens to
/// match the epoch number.
fn expected_current_target(epoch: u64) -> String {
    format!("{GENERATIONS_DIR_NAME}/{epoch}")
}

/// Read the exact literal text `current` resolves to, without any
/// component-wise parsing. Returns `None` if the link is absent,
/// unreadable, or not valid UTF-8.
fn current_target_text(secrets_fd: BorrowedFd<'_>) -> Option<String> {
    rustix::fs::readlinkat(secrets_fd, Path::new(CURRENT_LINK_NAME), Vec::new())
        .ok()?
        .into_string()
        .ok()
}

/// Read the literal name `current` resolves to (its symlink target's
/// exact literal text), parsed only if it is *exactly*
/// `generations/<epoch>` and nothing else.
fn read_current_target(secrets_fd: BorrowedFd<'_>) -> Option<u64> {
    let target = current_target_text(secrets_fd)?;
    let (dir, epoch_str) = target.split_once('/')?;
    if dir != GENERATIONS_DIR_NAME {
        return None;
    }
    parse_strict_epoch(epoch_str)
}

/// Verify that `current` resolves, by exact literal text, to
/// `generations/<epoch>` -- not merely to a path whose final
/// component happens to be that epoch number.
fn current_resolves_exactly_to(secrets_fd: BorrowedFd<'_>, epoch: u64) -> bool {
    match current_target_text(secrets_fd) {
        Some(target) => target == expected_current_target(epoch),
        None => false,
    }
}

/// Atomically swap `current` to point at `generations/<epoch>` using a
/// hidden-name-then-rename swap so there is never a moment `current`
/// is absent. Idempotent: if `current` already resolves, by exact
/// literal text, to `epoch`, this is a no-op.
fn atomic_swap_current(secrets_fd: BorrowedFd<'_>, epoch: u64) -> Result<(), FailReason> {
    if current_resolves_exactly_to(secrets_fd, epoch) {
        return Ok(());
    }
    let owned = borrowed_to_owned(secrets_fd)?;
    let target = expected_current_target(epoch);
    // Clean up a leftover swap-stage entry from a previous crash
    // between symlink-creation and rename. `CURRENT_SWAP_STAGE_NAME`
    // is only ever created via `symlinkat`, so
    // `path_safe::remove_path_safe` (which refuses to operate on
    // symlinks by design) can never actually remove it -- using it
    // here would silently leave the stale symlink in place and the
    // following `symlinkat` would then fail closed with `EEXIST`
    // every retry. Instead: nofollow-stat the slot; if absent, there
    // is nothing to clean up; if present and exactly a symlink,
    // `unlinkat` it; if present but *not* a symlink (an anomaly no
    // legitimate code path produces), fail closed rather than
    // deleting an unexpected entry.
    match path_safe::fstatat_nofollow(&owned, CURRENT_SWAP_STAGE_NAME)
        .map_err(|_| FailReason::CurrentSwapFailed)?
    {
        None => {}
        Some(stat) => {
            if (stat.st_mode & nix::libc::S_IFMT) != nix::libc::S_IFLNK {
                return Err(FailReason::CurrentSwapFailed);
            }
            match rustix::fs::unlinkat(
                secrets_fd,
                Path::new(CURRENT_SWAP_STAGE_NAME),
                rustix::fs::AtFlags::empty(),
            ) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => {}
                Err(_) => return Err(FailReason::CurrentSwapFailed),
            }
        }
    }
    rustix::fs::symlinkat(&target, secrets_fd, Path::new(CURRENT_SWAP_STAGE_NAME))
        .map_err(|_| FailReason::CurrentSwapFailed)?;
    rustix::fs::renameat(
        secrets_fd,
        Path::new(CURRENT_SWAP_STAGE_NAME),
        secrets_fd,
        Path::new(CURRENT_LINK_NAME),
    )
    .map_err(|_| FailReason::CurrentSwapFailed)?;
    fsync_fd(secrets_fd)?;
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
            fsync_fd(secrets_fd)?;
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
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // Already gone -- but retry the trailing durability
            // barrier in case a prior attempt deleted this entry and
            // then failed its own `fsync_fd(generations_fd)` before
            // the caller could advance the phase or commit a
            // tombstone. Never assume that fsync happened just
            // because the entry is absent.
            fsync_fd(generations_fd)?;
            return Ok(());
        }
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
    fsync_fd(generations_fd)?;
    Ok(())
}

/// The fully-enumerated, validated contents of `generations/`: real
/// generation epochs and real stage directories. Both halves are
/// "real secret tree contents" for retirement purposes -- a stage
/// directory is never swept separately or treated as ignorable.
struct GenerationTreeContents {
    epochs: Vec<u64>,
    stage_names: Vec<String>,
}

/// Anchored enumeration of `generations/`, failing closed on any
/// entry this cannot fully account for. Deletes nothing; a caller
/// invokes this to *plan* retirement (or to detect drift before
/// recovery/deletion), never as part of deleting.
fn enumerate_and_validate_generation_tree(
    generations_fd: BorrowedFd<'_>,
) -> Result<GenerationTreeContents, FailReason> {
    let generations_owned = borrowed_to_owned(generations_fd)?;
    let dir = rustix::fs::Dir::read_from(generations_fd)
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let mut epochs = Vec::new();
    let mut stage_names = Vec::new();
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
            // Stage directories are real secret-tree contents, not a
            // separately-swept concern: validate their shape now and
            // carry them in the plan so retirement/pruning enumerates,
            // validates, and deletes them under the same accounting as
            // generation epochs. Defense-in-depth: a live directory
            // name matching the prefix but not the full closed
            // `is_well_formed_stage_name` syntax (e.g. containing `/`
            // or `..`, which the kernel would never actually produce
            // as a single directory-entry name, or an unexpected
            // length/charset) is treated as an anomaly rather than a
            // stage directory this code will ever construct a path
            // from.
            if !is_well_formed_stage_name(&name) {
                return Err(FailReason::RetirementTreeAnomaly);
            }
            validate_stage_dir_contents(&generations_owned, &name)?;
            stage_names.push(name);
            continue;
        }
        let Some(epoch) = parse_strict_epoch(&name) else {
            return Err(FailReason::RetirementTreeAnomaly);
        };
        verify_generation_dir_is_exactly_material(&generations_owned, &name)?;
        epochs.push(epoch);
    }
    epochs.sort_unstable();
    stage_names.sort();
    Ok(GenerationTreeContents {
        epochs,
        stage_names,
    })
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
// Central pre-mutation validation
// ---------------------------------------------------------------------

/// Invoked by every public lifecycle action (provision/rotate/
/// rollback/retire) immediately after loading the on-disk marker,
/// whenever that marker records an active generation. Refuses to
/// proceed if the marker's own semantics are internally inconsistent,
/// or if it does not match the caller's own `(vm_id, kind)`, or if the
/// live on-disk state has drifted from what the marker claims -- most
/// importantly, whether `current` resolves, by *exact literal text*,
/// to the marker's recorded active epoch. A symlink retargeted to any
/// other path -- absolute, foreign-rooted, or merely ending in the
/// right epoch number -- is never accepted as a match. This is the
/// single reachable trigger for
/// [`FailReason::IdentityCurrentTargetMismatch`].
fn verify_marker_against_live_state(
    secrets_fd: BorrowedFd<'_>,
    generations_fd: BorrowedFd<'_>,
    vm_id: &str,
    kind: SecretKind,
    marker: &MarkerData,
) -> Result<(), FailReason> {
    if marker.vm != vm_id || marker.kind != kind.as_slug() {
        return Err(FailReason::MarkerTamperedOrMissingMaterial);
    }
    if marker.retired && marker.active.is_some() {
        // A tombstoned marker must never also claim an active
        // generation -- these two fields are mutually exclusive by
        // construction in every write path this module performs.
        return Err(FailReason::MarkerTamperedOrMissingMaterial);
    }
    let Some(active) = marker.active.as_ref() else {
        return Ok(());
    };
    if active.epoch == 0 || active.epoch > marker.high_water_epoch {
        return Err(FailReason::MarkerTamperedOrMissingMaterial);
    }
    if let Some(previous) = marker.previous.as_ref()
        && (previous.epoch == 0
            || previous.epoch == active.epoch
            || previous.epoch > marker.high_water_epoch)
    {
        // `previous` need not be numerically less than `active`: a
        // rollback swaps the pair, so `previous` can legitimately be
        // the *larger* epoch that was just rolled back away from.
        // What must always hold is that it is a distinct, real,
        // never-exceeding-high-water epoch.
        return Err(FailReason::MarkerTamperedOrMissingMaterial);
    }
    if !current_resolves_exactly_to(secrets_fd, active.epoch) {
        return Err(FailReason::IdentityCurrentTargetMismatch);
    }
    verify_existing_generation_identity(generations_fd, active)?;
    if let Some(previous) = marker.previous.as_ref() {
        verify_existing_generation_identity(generations_fd, previous)?;
    }
    Ok(())
}

/// Immediately before deleting a generation directory as part of
/// retirement, re-stat it (nofollow, anchored) and confirm it is
/// still either a validated material directory, or the exact
/// legitimate mid-deletion crash checkpoint `remove_generation` can
/// leave behind: `material` already unlinked, but the (now-empty)
/// epoch directory itself not yet removed. A missing entry is
/// tolerated (another recovery pass, or this same pass on retry,
/// already removed it entirely).
///
/// Recognizing the empty-directory checkpoint is reachable **only**
/// from here -- i.e. only from `execute_retire`'s `CurrentRemoved`
/// phase, always acting on a `RetireIntent` that has already passed
/// `validate_retire_intent_semantics` (checked both when the intent is
/// first written and every time a leftover txlog is re-read before
/// recovery acts on it). Fresh, pre-retirement planning never goes
/// through this function: `enumerate_and_validate_generation_tree`
/// (used to build a brand-new retirement plan, and again to prove the
/// tree fully empty once `EpochsRemoved` is reached) always calls the
/// strict `verify_generation_dir_is_exactly_material` and remains
/// exactly as strict as before -- an empty generation directory
/// encountered there is still `FailReason::RetirementTreeAnomaly`, not
/// a silently-accepted "nothing here yet" state.
///
/// To accept the directory as the empty checkpoint (rather than any
/// other empty-directory shape an attacker could construct in its
/// place) every one of these must hold:
/// - the entry is still a directory (never a symlink, regular file, or
///   any other type swapped into the same name -- that is a
///   replacement, not a partially-deleted survivor, and fails closed);
/// - it is still trusted-broker-owned at the expected `cfg.dir_mode`
///   (a foreign owner, or any other mode, means the directory itself
///   was replaced rather than left behind by this module's own
///   deletion sequence, and fails closed);
/// - it contains **zero** entries other than `.`/`..` -- any
///   unrecognised leftover entry, or a `material` entry of the wrong
///   type or link-count, fails closed rather than being swept away as
///   part of "finishing" the deletion.
///
/// Any other anomaly -- wrong type, an unexpected extra entry, a
/// hard-linked material file -- fails closed rather than deleting a
/// directory that no longer matches what enumeration originally
/// validated.
fn revalidate_generation_before_delete(
    generations_fd: BorrowedFd<'_>,
    epoch: u64,
    cfg: &SecretsLifecycleConfig,
) -> Result<(), FailReason> {
    let epoch_name = epoch.to_string();
    let generations_owned = borrowed_to_owned(generations_fd)?;
    let stat = match path_safe::fstatat_nofollow(&generations_owned, &epoch_name) {
        Ok(Some(stat)) => stat,
        Ok(None) => return Ok(()),
        Err(_) => return Err(FailReason::RetirementTreeAnomaly),
    };
    if (stat.st_mode & nix::libc::S_IFMT) != nix::libc::S_IFDIR {
        return Err(FailReason::RetirementTreeAnomaly);
    }
    match verify_generation_dir_is_exactly_material(&generations_owned, &epoch_name) {
        Ok(()) => return Ok(()),
        Err(FailReason::RetirementTreeAnomaly) => {}
        Err(other) => return Err(other),
    }
    // Not a fully-materialized generation: the only other shape this
    // retire deletion loop may ever accept is the exact
    // post-material-unlink, pre-directory-removal checkpoint, and only
    // over a directory that is still verifiably this module's own
    // trusted-broker-owned metadata directory.
    verify_broker_owned(&stat, nix::libc::S_IFDIR, cfg.dir_mode)
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let dir_fd = path_safe::open_at(
        generations_owned.as_fd(),
        Path::new(&epoch_name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    let inner = rustix::fs::Dir::read_from(dir_fd.as_fd())
        .map_err(|_| FailReason::RetirementTreeAnomaly)?;
    for entry in inner {
        let entry = entry.map_err(|_| FailReason::RetirementTreeAnomaly)?;
        let ename = entry
            .file_name()
            .to_str()
            .map_err(|_| FailReason::RetirementTreeAnomaly)?
            .to_owned();
        if ename != "." && ename != ".." {
            // A genuinely empty checkpoint has zero entries; anything
            // else here (an unrelated leftover, or a `material` entry
            // that `verify_generation_dir_is_exactly_material` already
            // rejected above as the wrong type/link-count) is an
            // anomaly, never silently accepted.
            return Err(FailReason::RetirementTreeAnomaly);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Unified promote engine (fresh action + crash recovery)
// ---------------------------------------------------------------------

#[derive(Debug)]
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
                    let epoch_name = intent.to_epoch.to_string();
                    let generations_owned = borrowed_to_owned(generations_fd)?;
                    let existing_digest =
                        digest_of_material_dir(&generations_owned, &epoch_name).ok();
                    match existing_digest {
                        Some(digest) if digest == intent.expected_digest_hex => {
                            // The rename to the final epoch name
                            // already completed in a previous crashed
                            // attempt. The epoch is fully materialized
                            // with exactly the expected content: never
                            // abandon or re-stage it, just continue
                            // forward. The crashed attempt's own
                            // trailing `fsync_fd(generations_fd)`
                            // inside `promote_stage_to_generation` may
                            // be exactly what failed and caused the
                            // crash before the phase could advance --
                            // retry that durability barrier now rather
                            // than trusting it silently already
                            // happened.
                            fsync_fd(generations_fd)?;
                        }
                        Some(_) => {
                            // Some directory already occupies this
                            // epoch number, but its content does not
                            // match what this transaction intended to
                            // promote. This can only be a stale,
                            // foreign, or previously-aborted
                            // generation; never silently adopt or
                            // overwrite it -- fail closed.
                            return Err(FailReason::RecoveryContentMismatch);
                        }
                        None => {
                            let staged = ensure_staged(
                                generations_fd,
                                cfg,
                                &intent.stage_name,
                                &intent.expected_digest_hex,
                                material,
                            )?;
                            if !staged {
                                // No fully-staged, digest-matching
                                // material exists (this is either the
                                // very first attempt with material
                                // absent -- should not happen given
                                // the public entry points always pass
                                // material, but defensively handled --
                                // or a recovery re-entry finding a
                                // stage dir left in a partial state by
                                // a crash between `mkdir_at_exclusive`
                                // and the material write completing).
                                // Never abandon a partial stage
                                // directory silently: validate and
                                // delete it (or fail closed) before
                                // discarding the txlog record of it.
                                discard_partial_stage_if_present(
                                    generations_fd,
                                    &intent.stage_name,
                                )?;
                                remove_txlog(secrets_fd)?;
                                return Ok(PromoteOutcome::AbortedNoMaterial);
                            }
                            promote_stage_to_generation(
                                generations_fd,
                                &intent.stage_name,
                                intent.to_epoch,
                                &intent.expected_digest_hex,
                            )?;
                        }
                    }
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
                // Never write a marker claiming an active generation
                // the live `current` symlink does not actually
                // corroborate. A hand-edited/corrupted txlog claiming
                // this phase over a `current` that does not resolve
                // to `to_epoch` must fail closed here, before any
                // marker mutation.
                if !current_resolves_exactly_to(secrets_fd, intent.to_epoch) {
                    return Err(FailReason::IdentityCurrentTargetMismatch);
                }
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
                // Phase-specific validation must occur before any
                // recovery mutation (pruning, stage cleanup) or txlog
                // removal: re-read and verify the marker actually
                // reflects this transaction's intended active
                // generation, and that `current` still corroborates
                // it, before proceeding. A hand-edited/corrupted
                // txlog claiming `MarkerCommitted` over a marker/
                // current that disagree must fail closed here rather
                // than pruning or discarding state based on an
                // unverified assumption.
                if !current_resolves_exactly_to(secrets_fd, intent.to_epoch) {
                    return Err(FailReason::IdentityCurrentTargetMismatch);
                }
                let marker = read_marker(secrets_fd)?.ok_or(FailReason::MarkerWriteFailed)?;
                let active_matches = marker.active.as_ref().is_some_and(|a| {
                    a.epoch == intent.to_epoch && a.digest_hex == intent.expected_digest_hex
                });
                if !active_matches {
                    return Err(FailReason::RecoveryAmbiguous);
                }
                if let Some(prune) = intent.prune_epoch {
                    // Best-effort: pruning failure does not fail the
                    // action (the marker already durably reflects the
                    // correct active/previous state); it only leaves
                    // an orphaned generation directory for a future
                    // retire() to clean up via its full-tree
                    // enumeration. Pruning only ever happens after the
                    // marker commit that no longer references the
                    // pruned epoch has been durably written above.
                    let _ = remove_generation(generations_fd, prune);
                }
                // Any leftover `.stage-*` directory at this point is a
                // real secret-tree entry from this or a prior crashed
                // attempt (never the one this transaction just
                // promoted -- that one was already renamed away).
                // Enumerate, validate, and delete it, propagating any
                // failure: a stage directory is never swept on a
                // best-effort basis. Revalidate each immediately
                // before deletion in case something changed between
                // enumeration and delete.
                let leftover = enumerate_and_validate_generation_tree(generations_fd)?;
                for stage_name in leftover.stage_names {
                    revalidate_stage_before_delete(generations_fd, &stage_name)?;
                    remove_stage_dir(generations_fd, &stage_name)?;
                }
                remove_txlog(secrets_fd)?;
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
    cfg: &SecretsLifecycleConfig,
    mut intent: RetireIntent,
) -> Result<(), FailReason> {
    loop {
        match intent.phase {
            RetirePhase::Enumerated => {
                let fresh = enumerate_and_validate_generation_tree(generations_fd)?;
                if !fresh.epochs.iter().all(|e| intent.epochs.contains(e))
                    || !fresh
                        .stage_names
                        .iter()
                        .all(|s| intent.stage_names.contains(s))
                {
                    return Err(FailReason::RecoveryAmbiguous);
                }
                remove_current_symlink(secrets_fd)?;
                intent.phase = RetirePhase::CurrentRemoved;
                write_txlog(secrets_fd, &TxLog::Retire(intent.clone()))?;
            }
            RetirePhase::CurrentRemoved => {
                for &epoch in &intent.epochs {
                    revalidate_generation_before_delete(generations_fd, epoch, cfg)?;
                    remove_generation(generations_fd, epoch)?;
                }
                for stage_name in &intent.stage_names {
                    // Staged entries are real secret-tree contents:
                    // enumerate/validate/delete them exactly like
                    // generation epochs, propagating every failure.
                    // Revalidate immediately before deletion in case
                    // something changed between enumeration and
                    // delete.
                    revalidate_stage_before_delete(generations_fd, stage_name)?;
                    remove_stage_dir(generations_fd, stage_name)?;
                }
                intent.phase = RetirePhase::EpochsRemoved;
                write_txlog(secrets_fd, &TxLog::Retire(intent.clone()))?;
            }
            RetirePhase::EpochsRemoved => {
                let remaining = enumerate_and_validate_generation_tree(generations_fd)?;
                if !remaining.epochs.is_empty() || !remaining.stage_names.is_empty() {
                    return Err(FailReason::RetirementNotProvablyEmpty);
                }
                intent.phase = RetirePhase::ProvenEmpty;
                write_txlog(secrets_fd, &TxLog::Retire(intent.clone()))?;
            }
            RetirePhase::ProvenEmpty => {
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
    vm_id: &str,
    kind: SecretKind,
    cfg: &SecretsLifecycleConfig,
) -> Result<bool, RecoveryError> {
    let txlog = read_txlog(secrets_fd, vm_id, kind).map_err(|reason| RecoveryError {
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
        TxLog::Retire(intent) => match execute_retire(secrets_fd, generations_fd, cfg, intent) {
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
            false,
            reason,
        ),
    })?;
    let (secrets_fd, generations_fd) =
        open_or_create_secrets_dir(&paths, cfg).map_err(|reason| {
            let ctx = context(vm_id, kind, LifecycleAction::Provision, &paths);
            denied(&ctx, false, reason)
        })?;
    let _lock = acquire_lock(secrets_fd.as_fd(), cfg).map_err(|reason| {
        let ctx = context(vm_id, kind, LifecycleAction::Provision, &paths);
        denied(&ctx, false, reason)
    })?;
    match recover_if_needed(secrets_fd.as_fd(), generations_fd.as_fd(), vm_id, kind, cfg) {
        Ok(recovered) => Ok(recovered),
        Err(err) => {
            let ctx = context(
                vm_id,
                kind,
                err.action.unwrap_or(LifecycleAction::Provision),
                &paths,
            );
            Err(failed_closed(
                &ctx,
                false,
                MarkerResult::FailedClosed,
                err.reason,
            ))
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
        denied(&ctx, false, reason)
    })?;
    let ctx = context(vm_id, kind, action, &paths);
    let (secrets_fd, generations_fd) =
        open_or_create_secrets_dir(&paths, cfg).map_err(|reason| denied(&ctx, false, reason))?;
    let lock =
        acquire_lock(secrets_fd.as_fd(), cfg).map_err(|reason| denied(&ctx, false, reason))?;
    let recovered =
        match recover_if_needed(secrets_fd.as_fd(), generations_fd.as_fd(), vm_id, kind, cfg) {
            Ok(recovered) => recovered,
            Err(err) => {
                let recovered_ctx = context(vm_id, kind, err.action.unwrap_or(action), &paths);
                return Err(failed_closed(
                    &recovered_ctx,
                    false,
                    MarkerResult::FailedClosed,
                    err.reason,
                ));
            }
        };
    let marker =
        read_marker(secrets_fd.as_fd()).map_err(|reason| denied(&ctx, recovered, reason))?;
    if let Some(marker) = &marker {
        verify_marker_against_live_state(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            vm_id,
            kind,
            marker,
        )
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    }
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
        return Err(denied(&ctx, recovered, FailReason::AlreadyProvisioned));
    }
    let existing = enumerate_and_validate_generation_tree(generations_fd.as_fd())
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    if !existing.epochs.is_empty() || !existing.stage_names.is_empty() {
        return Err(failed_closed(
            &ctx,
            recovered,
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
    validate_promote_intent_semantics(&intent, vm_id, kind)
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
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
            recovered,
            MarkerResult::FailedClosed,
            FailReason::MaterialWriteFailed,
        )),
        Err(reason) => Err(failed_closed(
            &ctx,
            recovered,
            MarkerResult::FailedClosed,
            reason,
        )),
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
        _ => return Err(denied(&ctx, recovered, FailReason::NotProvisioned)),
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
    validate_promote_intent_semantics(&intent, vm_id, kind)
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
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
            recovered,
            MarkerResult::FailedClosed,
            FailReason::MaterialWriteFailed,
        )),
        Err(reason) => Err(failed_closed(
            &ctx,
            recovered,
            MarkerResult::FailedClosed,
            reason,
        )),
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
        _ => return Err(denied(&ctx, recovered, FailReason::NotProvisioned)),
    };
    let Some(previous) = marker.previous.clone() else {
        return Err(denied(&ctx, recovered, FailReason::NoRollbackTarget));
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
    validate_promote_intent_semantics(&intent, vm_id, kind)
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
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
            recovered,
            MarkerResult::FailedClosed,
            FailReason::RecoveryAmbiguous,
        )),
        Err(reason) => Err(failed_closed(
            &ctx,
            recovered,
            MarkerResult::FailedClosed,
            reason,
        )),
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

    let existing = enumerate_and_validate_generation_tree(generations_fd.as_fd())
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;

    let recorded_high_water = marker.as_ref().map(|m| m.high_water_epoch).unwrap_or(0);
    let marker_claims_active = marker
        .as_ref()
        .map(|m| m.active.is_some() && !m.retired)
        .unwrap_or(false);

    if existing.epochs.is_empty() && existing.stage_names.is_empty() {
        if marker_claims_active {
            // Marker says material should exist; the physical tree
            // disagrees. Fail closed rather than silently accepting
            // "clean" over an unexplained discrepancy.
            return Err(failed_closed(
                &ctx,
                recovered,
                MarkerResult::FailedClosed,
                FailReason::PreviouslyProvisionedMaterialMissing,
            ));
        }
        return Ok(SecretsLifecycleAuditFields::verified_clean(
            &ctx,
            recovered,
            recorded_high_water,
        ));
    }

    let high_water_epoch =
        recorded_high_water.max(existing.epochs.iter().copied().max().unwrap_or(0));
    let intent = RetireIntent {
        vm: vm_id.to_owned(),
        kind: kind.as_slug().to_owned(),
        epochs: existing.epochs,
        stage_names: existing.stage_names,
        high_water_epoch,
        phase: RetirePhase::Enumerated,
    };
    validate_retire_intent_semantics(&intent, vm_id, kind)
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    write_txlog(secrets_fd.as_fd(), &TxLog::Retire(intent.clone()))
        .map_err(|reason| failed_closed(&ctx, recovered, MarkerResult::FailedClosed, reason))?;
    match execute_retire(secrets_fd.as_fd(), generations_fd.as_fd(), cfg, intent) {
        Ok(()) => Ok(SecretsLifecycleAuditFields::retired(
            &ctx,
            high_water_epoch,
            recovered,
        )),
        Err(reason) => Err(failed_closed(
            &ctx,
            recovered,
            MarkerResult::FailedClosed,
            reason,
        )),
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

    /// RAII guard that makes every `fsync_fd` call in this thread fail
    /// with [`FailReason::FsyncFailed`] for as long as it is held,
    /// restoring normal behavior on drop (including via early
    /// return/panic unwinding) so one test's fault injection can never
    /// leak into another.
    struct FsyncFaultGuard;
    impl FsyncFaultGuard {
        fn enable() -> Self {
            FSYNC_SHOULD_FAIL.with(|c| c.set(true));
            FsyncFaultGuard
        }
    }
    impl Drop for FsyncFaultGuard {
        fn drop(&mut self) {
            FSYNC_SHOULD_FAIL.with(|c| c.set(false));
        }
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
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2, &mat.digest_hex())
            .unwrap();
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
        assert!(read_txlog(secrets_fd.as_fd(), vm, kind).unwrap().is_none());
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
            stage_names: Vec::new(),
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
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2, &mat.digest_hex())
            .unwrap();
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

        // The hard-linked generation is still the marker's recorded
        // *active* generation, so the central pre-mutation identity
        // check (`verify_marker_against_live_state`, run before
        // retire's own tree enumeration) catches the link-count drift
        // first, with a more specific reason than the generic
        // tree-anomaly enumeration would have produced.
        let err = retire(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::IdentityLinkCountMismatch);
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

    // -------------------------------------------------------------
    // Finding-round-3 adversarial coverage: fsync-failure injection
    // -------------------------------------------------------------

    #[test]
    fn fsync_failure_in_ensure_staged_blocks_and_is_retryable() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("mia", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"fresh");
        let stage_name = random_stage_name();

        {
            let _fault = FsyncFaultGuard::enable();
            let err = ensure_staged(
                generations_fd.as_fd(),
                &cfg,
                &stage_name,
                &mat.digest_hex(),
                Some(&mat),
            )
            .unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // The material bytes were durably written to disk by the time
        // `fsync_fd` was reached (only the sync itself failed), so a
        // retry with fsync working again must observe the same digest
        // and complete idempotently rather than double-writing or
        // erroring.
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
    }

    #[test]
    fn fsync_failure_in_promote_stage_to_generation_blocks_and_is_retryable() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("nadia", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"fresh");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();

        {
            let _fault = FsyncFaultGuard::enable();
            let err = promote_stage_to_generation(
                generations_fd.as_fd(),
                &stage_name,
                7,
                &mat.digest_hex(),
            )
            .unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // The rename already landed on disk before the failed fsync;
        // retrying must see the matching digest at the final name and
        // treat it as already-promoted (never re-rename, never error).
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 7, &mat.digest_hex())
            .unwrap();
    }

    #[test]
    fn fsync_failure_in_atomic_swap_current_blocks_and_is_retryable() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "oscar";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();
        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        // Materialise a second generation directly (bypassing the
        // marker/txlog machinery) so swapping `current` to it is a
        // genuine change of target, not a same-epoch no-op that would
        // short-circuit before ever reaching `fsync_fd`.
        let mat2 = material(b"e2");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat2.digest_hex(),
            Some(&mat2),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2, &mat2.digest_hex())
            .unwrap();

        {
            let _fault = FsyncFaultGuard::enable();
            let err = atomic_swap_current(secrets_fd.as_fd(), 2).unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // `current` still resolves correctly afterward: either the
        // rename never completed (still epoch 1, unchanged) or it did
        // and only the sync failed -- both are safe, and a retry with
        // fsync restored must succeed.
        atomic_swap_current(secrets_fd.as_fd(), 2).unwrap();
        assert!(current_resolves_exactly_to(secrets_fd.as_fd(), 2));
    }

    #[test]
    fn fsync_failure_in_write_marker_blocks_marker_commit_phase() {
        // Drive `execute_promote` up to `CurrentPromoted` normally
        // (fsync working), then inject a failure exactly at the
        // `write_marker` call inside the `CurrentPromoted` phase and
        // confirm: (a) the call fails with `FsyncFailed`; (b) the
        // txlog still records `CurrentPromoted` (the phase advance to
        // `MarkerCommitted` never durably landed, so a crash here is
        // still recoverable rather than mistaken for "finished"); and
        // (c) a subsequent recovery pass with fsync restored
        // completes the promotion and removes the txlog.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "priya";
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
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2, &mat.digest_hex())
            .unwrap();
        atomic_swap_current(secrets_fd.as_fd(), 2).unwrap();
        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::CurrentPromoted,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone())).unwrap();

        {
            let _fault = FsyncFaultGuard::enable();
            let err = execute_promote(
                secrets_fd.as_fd(),
                generations_fd.as_fd(),
                &cfg,
                intent,
                None,
            )
            .unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // The marker rename+write already landed (fsync only
        // failed to make it durable, it did not block visibility),
        // so this process's own next read already observes epoch 2;
        // what fsync failing must guarantee is that the phase was
        // never advanced past `CurrentPromoted` -- the txlog still
        // records that phase, so a crash here is safely recoverable
        // rather than being treated as already-finished.
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert_eq!(marker.active.as_ref().unwrap().epoch, 2);
        let txlog = read_txlog(secrets_fd.as_fd(), vm, kind).unwrap().unwrap();
        match txlog {
            TxLog::Promote(intent) => assert_eq!(intent.phase, PromotePhase::CurrentPromoted),
            TxLog::Retire(_) => panic!("expected a leftover Promote txlog"),
        }

        // With fsync restored, recovery completes forward: the
        // txlog is durably removed and the marker remains at epoch 2.
        let recovered = recover_in_flight_transaction(vm, kind, &cfg).unwrap();
        assert!(recovered);
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert_eq!(marker.active.as_ref().unwrap().epoch, 2);
        assert!(read_txlog(secrets_fd.as_fd(), vm, kind).unwrap().is_none());
    }

    #[test]
    fn fsync_failure_in_remove_generation_blocks_and_is_retryable() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("quinn", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();

        {
            let _fault = FsyncFaultGuard::enable();
            let err = remove_generation(generations_fd.as_fd(), 1).unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // Idempotent retry (whether or not the delete itself already
        // landed) must succeed once fsync works again.
        remove_generation(generations_fd.as_fd(), 1).unwrap();
    }

    #[test]
    fn fsync_failure_in_remove_stage_dir_blocks_and_is_retryable() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("rex", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"stray");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();

        {
            let _fault = FsyncFaultGuard::enable();
            let err = remove_stage_dir(generations_fd.as_fd(), &stage_name).unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        remove_stage_dir(generations_fd.as_fd(), &stage_name).unwrap();
    }

    // -------------------------------------------------------------
    // Finding 2: orphan-epoch / pre-existing-digest scenarios
    // -------------------------------------------------------------

    #[test]
    fn promote_stage_to_generation_rejects_foreign_content_at_target_epoch() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("sasha", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();

        // A foreign directory already occupies the target epoch
        // number with content that does not match what we intend to
        // promote.
        let foreign = material(b"foreign-content");
        let foreign_stage = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &foreign_stage,
            &foreign.digest_hex(),
            Some(&foreign),
        )
        .unwrap();
        promote_stage_to_generation(
            generations_fd.as_fd(),
            &foreign_stage,
            5,
            &foreign.digest_hex(),
        )
        .unwrap();

        let intended = material(b"intended-content");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &intended.digest_hex(),
            Some(&intended),
        )
        .unwrap();
        let err = promote_stage_to_generation(
            generations_fd.as_fd(),
            &stage_name,
            5,
            &intended.digest_hex(),
        )
        .unwrap_err();
        assert_eq!(err, FailReason::GenerationConflict);
        // The foreign directory must be untouched -- never silently
        // overwritten or adopted.
        assert_eq!(
            digest_of_material_dir(&borrowed_to_owned(generations_fd.as_fd()).unwrap(), "5")
                .unwrap(),
            foreign.digest_hex()
        );
    }

    #[test]
    fn planned_phase_recovery_detects_already_renamed_orphan_epoch_and_continues() {
        // Regression guard for "never abandon/reuse an orphan epoch":
        // simulate a crash where the stage-to-epoch rename already
        // completed but the phase-advance write from `Planned` to
        // `EpochReady` never committed. Recovery must detect the
        // already-renamed epoch by exact digest match and continue
        // forward, never abandon the transaction nor silently adopt a
        // mismatched directory.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "tariq";
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
        // Simulate the rename having already happened, but the txlog
        // still recording `Planned` (as if the crash landed between
        // the rename and the phase-advance write).
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2, &mat.digest_hex())
            .unwrap();
        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
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
    }

    #[test]
    fn planned_phase_recovery_rejects_mismatched_content_at_orphan_epoch() {
        // The inverse of the above: the pre-existing directory at the
        // target epoch number has content that does NOT match the
        // recorded transaction's expected digest. This must never be
        // silently adopted -- fail closed instead.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "umberto";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let foreign = material(b"unexpected-content");
        let foreign_stage = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &foreign_stage,
            &foreign.digest_hex(),
            Some(&foreign),
        )
        .unwrap();
        promote_stage_to_generation(
            generations_fd.as_fd(),
            &foreign_stage,
            2,
            &foreign.digest_hex(),
        )
        .unwrap();

        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name: random_stage_name(),
            expected_digest_hex: material(b"e2").digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent)).unwrap();
        drop(secrets_fd);
        drop(generations_fd);

        let err = recover_in_flight_transaction(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::RecoveryContentMismatch);
        // Epoch 1 (the real active generation) must be entirely
        // untouched.
        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        assert_eq!(read_current_target(secrets_fd.as_fd()), Some(1));
    }

    // -------------------------------------------------------------
    // Finding 1: literal-`current`-target tamper
    // -------------------------------------------------------------

    #[test]
    fn current_resolves_exactly_to_rejects_foreign_path_with_matching_final_component() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("vera", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();

        // A symlink whose target merely *ends* in the right epoch
        // number, but is not the exact literal `generations/<epoch>`
        // text, must never be treated as resolving to that epoch.
        let _ = rustix::fs::unlinkat(
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
            rustix::fs::AtFlags::empty(),
        );
        rustix::fs::symlinkat(
            "/some/foreign/absolute/path/7",
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
        )
        .unwrap();
        assert!(!current_resolves_exactly_to(secrets_fd.as_fd(), 7));
        assert_eq!(read_current_target(secrets_fd.as_fd()), None);

        let _ = rustix::fs::unlinkat(
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
            rustix::fs::AtFlags::empty(),
        );
        rustix::fs::symlinkat(
            "../../foreign/7",
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
        )
        .unwrap();
        assert!(!current_resolves_exactly_to(secrets_fd.as_fd(), 7));
        assert_eq!(read_current_target(secrets_fd.as_fd()), None);

        let _ = rustix::fs::unlinkat(
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
            rustix::fs::AtFlags::empty(),
        );
        rustix::fs::symlinkat(
            expected_current_target(7),
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
        )
        .unwrap();
        assert!(current_resolves_exactly_to(secrets_fd.as_fd(), 7));
        assert_eq!(read_current_target(secrets_fd.as_fd()), Some(7));
    }

    #[test]
    fn tampered_current_symlink_is_reachable_and_fails_closed_on_rotate() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "wendy";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let _ = rustix::fs::unlinkat(
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
            rustix::fs::AtFlags::empty(),
        );
        // Retarget `current` to an absolute path whose final
        // component happens to be "1" (the real active epoch's
        // number) but which is not the literal `generations/1` text
        // this module ever writes.
        rustix::fs::symlinkat(
            "/etc/passwd/../1",
            secrets_fd.as_fd(),
            Path::new(CURRENT_LINK_NAME),
        )
        .unwrap();
        drop(secrets_fd);

        let err = rotate(vm, kind, material(b"e2"), &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::IdentityCurrentTargetMismatch);
    }

    // -------------------------------------------------------------
    // Finding 3: staged entries are real, propagated deletions
    // -------------------------------------------------------------

    #[test]
    fn validate_stage_dir_contents_rejects_extra_entries_and_subdirectories() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("xander", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();

        let owned = borrowed_to_owned(generations_fd.as_fd()).unwrap();
        let stage_dir_fd = path_safe::open_at(
            owned.as_fd(),
            Path::new(&stage_name),
            OFlags::RDONLY | OFlags::DIRECTORY,
        )
        .unwrap();
        let stage_dir_owned = borrowed_to_owned(stage_dir_fd.as_fd()).unwrap();
        path_safe::mkdir_at(stage_dir_owned.as_fd(), Path::new("extra-subdir"), 0o700).unwrap();

        let err = validate_stage_dir_contents(&owned, &stage_name).unwrap_err();
        assert_eq!(err, FailReason::RetirementTreeAnomaly);
    }

    #[test]
    fn stage_dir_deletion_failure_propagates_and_blocks_retire() {
        // A stage directory that cannot be fully enumerated/deleted
        // (here: it contains an anomalous subdirectory rather than at
        // most one regular-file entry) must block retirement entirely
        // -- never a best-effort, ignored sweep.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "yara";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let stray = random_stage_name();
        path_safe::mkdir_at_exclusive(generations_fd.as_fd(), Path::new(&stray), 0o700).unwrap();
        let stray_owned = borrowed_to_owned(generations_fd.as_fd()).unwrap();
        let stray_fd = path_safe::open_at(
            stray_owned.as_fd(),
            Path::new(&stray),
            OFlags::RDONLY | OFlags::DIRECTORY,
        )
        .unwrap();
        let stray_dir_owned = borrowed_to_owned(stray_fd.as_fd()).unwrap();
        path_safe::mkdir_at(stray_dir_owned.as_fd(), Path::new("nested"), 0o700).unwrap();

        let err = retire(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::RetirementTreeAnomaly);
        // Nothing was deleted: the legitimate generation and the
        // anomalous stray directory are both still present.
        let generations_dir = paths.kind_root.join(GENERATIONS_DIR_NAME);
        assert!(generations_dir.join("1").join(MATERIAL_FILE_NAME).is_file());
        assert!(generations_dir.join(&stray).is_dir());
    }

    #[test]
    fn retire_deletes_stray_stage_directories_as_real_content() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "zane";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let stray_mat = material(b"stray-stage-content");
        let stray_stage = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stray_stage,
            &stray_mat.digest_hex(),
            Some(&stray_mat),
        )
        .unwrap();

        let fields = retire(vm, kind, &cfg).unwrap();
        assert_valid(&fields);
        let generations_dir = paths.kind_root.join(GENERATIONS_DIR_NAME);
        assert_eq!(std::fs::read_dir(&generations_dir).unwrap().count(), 0);
    }

    // -------------------------------------------------------------
    // Finding 5: txlog semantic validation / pre-delete revalidation
    // -------------------------------------------------------------

    #[test]
    fn txlog_with_wrong_vm_is_rejected_before_recovery_acts() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "amara";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let intent = RetireIntent {
            vm: "someone-else".to_owned(),
            kind: kind.as_slug().to_owned(),
            epochs: vec![1],
            stage_names: Vec::new(),
            high_water_epoch: 1,
            phase: RetirePhase::CurrentRemoved,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Retire(intent)).unwrap();
        drop(secrets_fd);

        let err = recover_in_flight_transaction(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::IntentCorrupt);
        // Nothing was touched: the active generation is intact.
        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        assert_eq!(read_current_target(secrets_fd.as_fd()), Some(1));
    }

    #[test]
    fn txlog_with_non_monotonic_epochs_is_rejected() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("boaz", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let intent = RetireIntent {
            vm: "boaz".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            epochs: vec![3, 1],
            stage_names: Vec::new(),
            high_water_epoch: 3,
            phase: RetirePhase::Enumerated,
        };
        let err = validate_retire_intent_semantics(&intent, "boaz", SecretKind::GuestSigningKey)
            .unwrap_err();
        assert_eq!(err, FailReason::IntentCorrupt);
        drop(secrets_fd);
    }

    #[test]
    fn revalidate_generation_before_delete_catches_a_tamper_immediately_before_delete() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("carys", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();

        // Absent generation is tolerated (idempotent replay).
        revalidate_generation_before_delete(generations_fd.as_fd(), 2, &cfg).unwrap();

        // Tamper: hard-link a second name into the generation just
        // before deletion would occur.
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::hard_link(gen_dir.join(MATERIAL_FILE_NAME), gen_dir.join("planted")).unwrap();
        let err = revalidate_generation_before_delete(generations_fd.as_fd(), 1, &cfg).unwrap_err();
        assert_eq!(err, FailReason::RetirementTreeAnomaly);
    }

    // -------------------------------------------------------------
    // Finding 6: broker-owned metadata / lock-file verification
    // -------------------------------------------------------------

    #[test]
    fn acquire_lock_rejects_a_wrong_mode_lock_file() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("dov", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        drop(_generations_fd);
        drop(secrets_fd);

        let lock_path = paths.kind_root.join(LOCK_FILE_NAME);
        std::fs::write(&lock_path, b"").unwrap();
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o666)).unwrap();

        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let err = acquire_lock(secrets_fd.as_fd(), &cfg).unwrap_err();
        assert_eq!(err, FailReason::BrokerOwnershipViolation);
    }

    #[test]
    fn acquire_lock_rejects_a_hardlinked_lock_file() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("elan", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        drop(generations_fd);
        drop(secrets_fd);

        let lock_path = paths.kind_root.join(LOCK_FILE_NAME);
        std::fs::write(&lock_path, b"").unwrap();
        std::fs::set_permissions(
            &lock_path,
            std::fs::Permissions::from_mode(FILE_MODE_DEFAULT),
        )
        .unwrap();
        std::fs::hard_link(&lock_path, paths.kind_root.join("lock-plant")).unwrap();

        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let err = acquire_lock(secrets_fd.as_fd(), &cfg).unwrap_err();
        assert_eq!(err, FailReason::BrokerOwnershipViolation);
    }

    #[test]
    fn acquire_lock_rejects_a_wrong_mode_parent_directory() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("farid", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();

        // Tamper the mode on the already-open `kind_root` directory
        // (the lock file's anchored parent) via its live inode --
        // re-opening through `open_or_create_secrets_dir` would
        // silently reassert the correct mode via `ensure_dir` before
        // `acquire_lock` ever ran, masking the drift this test must
        // prove is caught. Using the still-open fd's `fstat` view
        // instead observes the tamper exactly like a concurrent
        // actor's `chmod` on the live directory would.
        std::fs::set_permissions(&paths.kind_root, std::fs::Permissions::from_mode(0o777)).unwrap();

        let err = acquire_lock(secrets_fd.as_fd(), &cfg).unwrap_err();
        assert_eq!(err, FailReason::BrokerOwnershipViolation);
    }

    #[test]
    fn open_or_create_secrets_dir_reasserts_mode_drift_on_metadata_dirs() {
        // `ensure_dir` only reasserts *mode*, not owner, when passed
        // `None, None`; confirm the mode side of that contract is
        // actually exercised end to end (a drifted mode on the
        // metadata directories is corrected on the next open) rather
        // than merely assumed from construction.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("gilad", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        drop(generations_fd);
        drop(secrets_fd);

        std::fs::set_permissions(&paths.kind_root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let stat = path_safe::fstat_fd(secrets_fd.as_fd()).unwrap();
        assert_eq!((stat.st_mode as u32) & 0o7777, cfg.dir_mode);
    }

    // -------------------------------------------------------------
    // Finding 1: central validation catches marker/live drift on
    // every action, not only the one that caused it
    // -------------------------------------------------------------

    #[test]
    fn central_validation_runs_identically_for_every_public_action() {
        // A single tamper (retargeting `current` away from the
        // marker's recorded active epoch) must be caught by
        // `provision`, `rotate`, `rollback`, and `retire` alike, since
        // all four route through the same `open_and_recover` choke
        // point rather than four separately-maintained checks.
        for action in ["rotate", "rollback", "retire"] {
            let dir = tempdir().unwrap();
            let cfg = cfg_at(dir.path());
            let vm = "hana";
            let kind = SecretKind::GuestSigningKey;
            provision(vm, kind, material(b"e1"), &cfg).unwrap();
            rotate(vm, kind, material(b"e2"), &cfg).unwrap();

            let paths = derive_paths(vm, kind, &cfg).unwrap();
            let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
            let _ = rustix::fs::unlinkat(
                secrets_fd.as_fd(),
                Path::new(CURRENT_LINK_NAME),
                rustix::fs::AtFlags::empty(),
            );
            rustix::fs::symlinkat(
                "/nonexistent/tamper/2",
                secrets_fd.as_fd(),
                Path::new(CURRENT_LINK_NAME),
            )
            .unwrap();
            drop(secrets_fd);

            let err = match action {
                "rotate" => rotate(vm, kind, material(b"e3"), &cfg).unwrap_err(),
                "rollback" => rollback(vm, kind, &cfg).unwrap_err(),
                "retire" => retire(vm, kind, &cfg).unwrap_err(),
                _ => unreachable!(),
            };
            assert_eq!(
                err.reason,
                FailReason::IdentityCurrentTargetMismatch,
                "action {action} did not reach the central validation choke point"
            );
        }
    }

    // -------------------------------------------------------------
    // Round-4 finding 1: Planned-phase partial-stage discard on abort
    // -------------------------------------------------------------

    #[test]
    fn planned_recovery_discards_a_partial_stage_before_aborting_no_material() {
        // Simulate a crash between `mkdir_at_exclusive` succeeding and
        // the material write ever completing: an empty `.stage-*`
        // directory exists on disk, but `ensure_staged` (given no
        // in-memory `material`, as on a real recovery re-entry) can
        // neither find a fully-staged digest-matching directory nor
        // write fresh material. The stage directory must be validated
        // and deleted -- never abandoned -- before the txlog record is
        // discarded.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("orphan-stage", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let stage_name = random_stage_name();
        // An empty stage directory: `mkdir_at_exclusive` succeeded,
        // nothing was ever written into it.
        path_safe::mkdir_at_exclusive(generations_fd.as_fd(), Path::new(&stage_name), cfg.dir_mode)
            .unwrap();

        let intent = PromoteIntent {
            vm: "orphan-stage".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name: stage_name.clone(),
            expected_digest_hex: material(b"never-written").digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        let outcome = execute_promote(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            &cfg,
            intent,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, PromoteOutcome::AbortedNoMaterial));
        // The partial stage directory must be gone, not orphaned.
        assert!(
            path_safe::fstatat_nofollow(
                &borrowed_to_owned(generations_fd.as_fd()).unwrap(),
                &stage_name
            )
            .unwrap()
            .is_none()
        );
        // The txlog must also be gone (the abort was fully committed,
        // not left wedged).
        assert!(
            read_txlog(
                secrets_fd.as_fd(),
                "orphan-stage",
                SecretKind::GuestSigningKey
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn planned_recovery_fails_closed_on_an_unvalidatable_partial_stage_and_retains_txlog() {
        // The inverse: the leftover stage directory is not a
        // recognizable partial-write shape (e.g. a stray subdirectory
        // was injected into it) -- `discard_partial_stage_if_present`
        // must propagate that anomaly rather than silently discarding
        // it, and the txlog record must survive so a future recovery
        // attempt can still reason about it. No orphan wedge: the
        // transaction fails closed instead of being silently dropped
        // over unrecognized on-disk state.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("bad-stage", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let stage_name = random_stage_name();
        path_safe::mkdir_at_exclusive(generations_fd.as_fd(), Path::new(&stage_name), cfg.dir_mode)
            .unwrap();
        let stage_dir_fd = path_safe::open_at(
            generations_fd.as_fd(),
            Path::new(&stage_name),
            OFlags::RDONLY | OFlags::DIRECTORY,
        )
        .unwrap();
        let stage_dir_owned = borrowed_to_owned(stage_dir_fd.as_fd()).unwrap();
        // A subdirectory is never a valid stage-dir entry shape.
        path_safe::mkdir_at(stage_dir_owned.as_fd(), Path::new("anomaly"), 0o700).unwrap();

        let intent = PromoteIntent {
            vm: "bad-stage".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name: stage_name.clone(),
            expected_digest_hex: material(b"never-written").digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent.clone())).unwrap();
        let err = execute_promote(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            &cfg,
            intent,
            None,
        )
        .unwrap_err();
        assert_eq!(err, FailReason::RetirementTreeAnomaly);
        // Never silently activated: the stage dir is untouched and the
        // txlog record still exists for a future recovery attempt to
        // reason about.
        assert!(
            path_safe::fstatat_nofollow(
                &borrowed_to_owned(generations_fd.as_fd()).unwrap(),
                &stage_name
            )
            .unwrap()
            .is_some()
        );
        assert!(
            read_txlog(secrets_fd.as_fd(), "bad-stage", SecretKind::GuestSigningKey)
                .unwrap()
                .is_some()
        );
    }

    // -------------------------------------------------------------
    // Round-4 finding 2: `current`-swap leftover stage-symlink cleanup
    // -------------------------------------------------------------

    #[test]
    fn atomic_swap_current_removes_a_leftover_swap_stage_symlink_from_a_prior_crash() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("swap-retry", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();
        // Leave a leftover swap-stage symlink behind, as if a prior
        // crash landed between `symlinkat` and `renameat`.
        rustix::fs::symlinkat(
            expected_current_target(1),
            secrets_fd.as_fd(),
            Path::new(CURRENT_SWAP_STAGE_NAME),
        )
        .unwrap();
        // Must not fail with EEXIST on retry: the leftover symlink is
        // actually removed (not left in place by a no-op
        // `remove_path_safe` call), so the fresh `symlinkat` below it
        // succeeds.
        atomic_swap_current(secrets_fd.as_fd(), 1).unwrap();
        assert!(current_resolves_exactly_to(secrets_fd.as_fd(), 1));
        assert!(
            path_safe::fstatat_nofollow(
                &borrowed_to_owned(secrets_fd.as_fd()).unwrap(),
                CURRENT_SWAP_STAGE_NAME
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn atomic_swap_current_fails_closed_on_a_non_symlink_swap_stage_entry() {
        // An anomaly no legitimate code path produces: something
        // other than a symlink occupies the swap-stage slot. Must
        // fail closed rather than deleting an unexpected entry.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("swap-anomaly", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();
        let secrets_owned = borrowed_to_owned(secrets_fd.as_fd()).unwrap();
        path_safe::create_file_at_safe(
            &secrets_owned,
            CURRENT_SWAP_STAGE_NAME,
            nix::libc::O_RDWR | nix::libc::O_CREAT,
            FILE_MODE_DEFAULT,
        )
        .unwrap();

        let err = atomic_swap_current(secrets_fd.as_fd(), 1).unwrap_err();
        assert_eq!(err, FailReason::CurrentSwapFailed);
        // The anomalous entry must be left untouched, not deleted.
        let stat = path_safe::fstatat_nofollow(&secrets_owned, CURRENT_SWAP_STAGE_NAME)
            .unwrap()
            .unwrap();
        assert_eq!(stat.st_mode & nix::libc::S_IFMT, nix::libc::S_IFREG);
    }

    // -------------------------------------------------------------
    // Round-4 finding 3: existing-digest-match `Planned` branch retries
    // its own durability barrier
    // -------------------------------------------------------------

    #[test]
    fn planned_recovery_existing_digest_match_retries_generations_fsync() {
        // The rename to the final epoch name already completed (a
        // prior crashed attempt got as far as
        // `promote_stage_to_generation`, whose own trailing
        // `fsync_fd(generations_fd)` may be exactly what failed and
        // crashed the attempt before the phase could advance). Recovery
        // re-entry finding a digest match must retry that durability
        // barrier, not silently assume it already happened.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("digest-match-fsync", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();
        let intent = PromoteIntent {
            vm: "digest-match-fsync".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        {
            let _fault = FsyncFaultGuard::enable();
            let err = execute_promote(
                secrets_fd.as_fd(),
                generations_fd.as_fd(),
                &cfg,
                intent.clone(),
                None,
            )
            .unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // Retry without the fault must succeed and complete the whole
        // transaction.
        let outcome = execute_promote(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            &cfg,
            intent,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, PromoteOutcome::Completed(_)));
    }

    // -------------------------------------------------------------
    // Round-4 finding 4: `NotFound` fast paths still retry the
    // generations-dir fsync durability barrier
    // -------------------------------------------------------------

    #[test]
    fn remove_generation_notfound_fast_path_still_retries_fsync() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("gen-notfound-fsync", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        // Epoch 7 was never created: the `NotFound` fast path is hit
        // immediately.
        {
            let _fault = FsyncFaultGuard::enable();
            let err = remove_generation(generations_fd.as_fd(), 7).unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        remove_generation(generations_fd.as_fd(), 7).unwrap();
    }

    #[test]
    fn remove_stage_dir_notfound_fast_path_still_retries_fsync() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths =
            derive_paths("stage-notfound-fsync", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let stage_name = random_stage_name();
        {
            let _fault = FsyncFaultGuard::enable();
            let err = remove_stage_dir(generations_fd.as_fd(), &stage_name).unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        remove_stage_dir(generations_fd.as_fd(), &stage_name).unwrap();
    }

    // -------------------------------------------------------------
    // Round-4 finding 5: phase-specific current/marker identity
    // validation before recovery mutation or txlog removal
    // -------------------------------------------------------------

    #[test]
    fn current_promoted_recovery_rejects_current_not_pointing_at_to_epoch() {
        // A hand-edited/corrupted txlog claims `CurrentPromoted` over
        // a `current` that does not actually resolve to `to_epoch`.
        // Must fail closed before any marker mutation, never write a
        // marker claiming an active generation `current` does not
        // corroborate.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("cp-mismatch", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();
        // `current` is left absent/unset -- never swapped to epoch 1 --
        // yet the txlog claims `CurrentPromoted`.
        let intent = PromoteIntent {
            vm: "cp-mismatch".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::CurrentPromoted,
        };
        let err = execute_promote(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            &cfg,
            intent,
            None,
        )
        .unwrap_err();
        assert_eq!(err, FailReason::IdentityCurrentTargetMismatch);
        // No marker must have been written.
        assert!(read_marker(secrets_fd.as_fd()).unwrap().is_none());
    }

    #[test]
    fn marker_committed_recovery_rejects_current_not_pointing_at_to_epoch() {
        // `MarkerCommitted` recovery must independently re-verify
        // `current` before pruning/cleanup/txlog-removal, not just
        // trust the phase label.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("mc-current-mismatch", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();
        // Note: `current` is never swapped here.
        let intent = PromoteIntent {
            vm: "mc-current-mismatch".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::MarkerCommitted,
        };
        let err = execute_promote(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            &cfg,
            intent,
            None,
        )
        .unwrap_err();
        assert_eq!(err, FailReason::IdentityCurrentTargetMismatch);
    }

    #[test]
    fn marker_committed_recovery_rejects_a_marker_disagreeing_with_the_intent() {
        // `current` correctly resolves to `to_epoch`, but the marker
        // on disk does not reflect this transaction's intended active
        // epoch/digest (e.g. a stale marker from before this
        // transaction, or one hand-edited independently of the
        // txlog). `MarkerCommitted` recovery must reject this rather
        // than proceeding with pruning/cleanup based on an unverified
        // assumption.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("mc-marker-mismatch", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();
        atomic_swap_current(secrets_fd.as_fd(), 1).unwrap();
        // Write a marker that does not correspond to this transaction
        // (wrong high-water/active state -- e.g. still reflecting
        // "nothing provisioned yet").
        let stale_marker = MarkerData {
            v: MARKER_SCHEMA_VERSION,
            vm: "mc-marker-mismatch".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            retired: false,
            high_water_epoch: 0,
            active: None,
            previous: None,
            first_provisioned_ms: 0,
            updated_ms: now_ms(),
        };
        write_marker(secrets_fd.as_fd(), &stale_marker).unwrap();

        let intent = PromoteIntent {
            vm: "mc-marker-mismatch".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat.digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::MarkerCommitted,
        };
        let err = execute_promote(
            secrets_fd.as_fd(),
            generations_fd.as_fd(),
            &cfg,
            intent,
            None,
        )
        .unwrap_err();
        assert_eq!(err, FailReason::RecoveryAmbiguous);
    }

    // -------------------------------------------------------------
    // Round-4 finding 6: closed stage-name syntax + immediate
    // pre-delete revalidation
    // -------------------------------------------------------------

    #[test]
    fn is_well_formed_stage_name_accepts_only_the_exact_random_stage_name_syntax() {
        assert!(is_well_formed_stage_name(&random_stage_name()));
        assert!(is_well_formed_stage_name(&random_stage_name()));
        for bad in [
            "",
            ".stage-",
            ".stage-too-short",
            &format!("{STAGE_PREFIX}{}", "a".repeat(31)),
            &format!("{STAGE_PREFIX}{}", "a".repeat(33)),
            &format!("{STAGE_PREFIX}{}", "A".repeat(32)),
            &format!("{STAGE_PREFIX}{}", "g".repeat(32)),
            &format!("{STAGE_PREFIX}../{}", "a".repeat(29)),
            &format!("{STAGE_PREFIX}{}/etc", "a".repeat(28)),
            "not-a-stage-name",
        ] {
            assert!(
                !is_well_formed_stage_name(bad),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn validate_promote_intent_semantics_rejects_a_path_traversal_stage_name() {
        // A corrupted/hand-edited txlog must never reach an anchored
        // path-safe call with an attacker-controlled path component.
        let intent = PromoteIntent {
            vm: "traversal".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Provision,
            to_epoch: 1,
            create_epoch: true,
            stage_name: format!("{STAGE_PREFIX}../../etc/passwd"),
            expected_digest_hex: material(b"x").digest_hex(),
            expected_identity: None,
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 1,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        let err =
            validate_promote_intent_semantics(&intent, "traversal", SecretKind::GuestSigningKey)
                .unwrap_err();
        assert_eq!(err, FailReason::IntentCorrupt);
    }

    #[test]
    fn validate_retire_intent_semantics_rejects_a_path_traversal_stage_name() {
        let intent = RetireIntent {
            vm: "traversal2".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            epochs: vec![],
            stage_names: vec![format!("{STAGE_PREFIX}/etc/passwd")],
            high_water_epoch: 1,
            phase: RetirePhase::Enumerated,
        };
        let err =
            validate_retire_intent_semantics(&intent, "traversal2", SecretKind::GuestSigningKey)
                .unwrap_err();
        assert_eq!(err, FailReason::IntentCorrupt);
    }

    #[test]
    fn revalidate_stage_before_delete_catches_a_tamper_injected_just_before_delete() {
        // Mirrors `revalidate_generation_before_delete_catches_a_tamper_
        // immediately_before_delete` for stage directories: a stage
        // dir enumerated and validated earlier is tampered with (an
        // extra entry injected) between enumeration and the delete
        // call. The immediate pre-delete revalidation must catch it.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("stage-tamper", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"pending");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        // Originally validates cleanly.
        revalidate_stage_before_delete(generations_fd.as_fd(), &stage_name).unwrap();

        // Tamper: inject a second entry into the stage dir.
        let generations_owned = borrowed_to_owned(generations_fd.as_fd()).unwrap();
        let stage_dir_fd = path_safe::open_at(
            generations_owned.as_fd(),
            Path::new(&stage_name),
            OFlags::RDONLY | OFlags::DIRECTORY,
        )
        .unwrap();
        let stage_dir_owned = borrowed_to_owned(stage_dir_fd.as_fd()).unwrap();
        path_safe::mkdir_at(stage_dir_owned.as_fd(), Path::new("tamper"), 0o700).unwrap();

        let err = revalidate_stage_before_delete(generations_fd.as_fd(), &stage_name).unwrap_err();
        assert_eq!(err, FailReason::RetirementTreeAnomaly);
    }

    #[test]
    fn revalidate_stage_before_delete_tolerates_absence() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths = derive_paths("stage-absent", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        revalidate_stage_before_delete(generations_fd.as_fd(), &random_stage_name()).unwrap();
    }

    // -------------------------------------------------------------
    // Round-4 finding 7: rollback intent digest/identity consistency
    // -------------------------------------------------------------

    #[test]
    fn validate_promote_intent_semantics_rejects_rollback_digest_identity_mismatch() {
        let identity = MaterialIdentity {
            epoch: 1,
            dev: 1,
            ino: 1,
            uid: 0,
            gid: 0,
            mode: 0o600,
            nlink: 1,
            has_acl: false,
            digest_hex: material(b"actual").digest_hex(),
        };
        let intent = PromoteIntent {
            vm: "rollback-mismatch".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Rollback,
            to_epoch: 1,
            create_epoch: false,
            stage_name: String::new(),
            // Deliberately inconsistent with `identity.digest_hex`.
            expected_digest_hex: material(b"different").digest_hex(),
            expected_identity: Some(identity),
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        let err = validate_promote_intent_semantics(
            &intent,
            "rollback-mismatch",
            SecretKind::GuestSigningKey,
        )
        .unwrap_err();
        assert_eq!(err, FailReason::IntentCorrupt);
    }

    #[test]
    fn validate_promote_intent_semantics_accepts_consistent_rollback_digest_identity() {
        let identity = MaterialIdentity {
            epoch: 1,
            dev: 1,
            ino: 1,
            uid: 0,
            gid: 0,
            mode: 0o600,
            nlink: 1,
            has_acl: false,
            digest_hex: material(b"actual").digest_hex(),
        };
        let intent = PromoteIntent {
            vm: "rollback-consistent".to_owned(),
            kind: SecretKind::GuestSigningKey.as_slug().to_owned(),
            action: LifecycleAction::Rollback,
            to_epoch: 1,
            create_epoch: false,
            stage_name: String::new(),
            expected_digest_hex: identity.digest_hex.clone(),
            expected_identity: Some(identity),
            carry_previous: None,
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: now_ms(),
            phase: PromotePhase::Planned,
        };
        validate_promote_intent_semantics(
            &intent,
            "rollback-consistent",
            SecretKind::GuestSigningKey,
        )
        .unwrap();
    }

    // -------------------------------------------------------------
    // Round-4 finding 8: `recovered_prior_transaction` threading
    // -------------------------------------------------------------

    #[test]
    fn recovered_prior_transaction_is_true_on_a_denial_reached_after_successful_recovery() {
        // `provision` first drains a leftover crashed `rotate`
        // transaction (recoverable: the rename to epoch 2 already
        // completed with a matching digest, so recovery can run it
        // all the way to completion), then hits `AlreadyProvisioned`
        // because the marker is now active. The denial must carry
        // `recovered_prior_transaction: true`.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "recovered-denied";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat2 = material(b"e2");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat2.digest_hex(),
            Some(&mat2),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 2, &mat2.digest_hex())
            .unwrap();
        let marker_before = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        let intent = PromoteIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            action: LifecycleAction::Rotate,
            to_epoch: 2,
            create_epoch: true,
            stage_name,
            expected_digest_hex: mat2.digest_hex(),
            expected_identity: None,
            carry_previous: marker_before.active.clone(),
            prune_epoch: None,
            new_high_water_epoch: 2,
            first_provisioned_ms: marker_before.first_provisioned_ms,
            phase: PromotePhase::Planned,
        };
        write_txlog(secrets_fd.as_fd(), &TxLog::Promote(intent)).unwrap();
        drop(secrets_fd);
        drop(generations_fd);

        let err = provision(vm, kind, material(b"e3"), &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::AlreadyProvisioned);
        assert!(
            err.audit.recovered_prior_transaction,
            "denial reached after a successful mid-call recovery must record it"
        );

        // Sanity: recovery genuinely ran (the crashed rotate is fully
        // completed, epoch 2 is now active).
        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let (secrets_fd, _generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert_eq!(marker.active.unwrap().epoch, 2);
    }

    #[test]
    fn recovered_prior_transaction_is_false_on_the_common_no_crash_denial_path() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let err = rotate(
            "never-provisioned",
            SecretKind::GuestSigningKey,
            material(b"e1"),
            &cfg,
        )
        .unwrap_err();
        assert_eq!(err.reason, FailReason::NotProvisioned);
        assert!(!err.audit.recovered_prior_transaction);
    }

    // -------------------------------------------------------------
    // Round-5 finding: txlog-backed retirement recovery must accept
    // the legitimate "material already unlinked, epoch directory not
    // yet removed" crash checkpoint -- but only from the retire
    // txlog-recovery deletion loop, and only over a directory that
    // still proves out as this module's own trusted-broker-owned,
    // fully empty metadata directory. Fresh pre-retirement planning
    // (`retire()`'s initial enumeration) must remain exactly as
    // strict as before.
    // -------------------------------------------------------------

    #[test]
    fn execute_retire_finishes_a_generation_crashed_immediately_after_material_unlink() {
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "crash-after-material-unlink";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        // Simulate the crash: `remove_generation` already unlinked
        // `material` but the process died before it removed the
        // now-empty epoch directory entry itself.
        std::fs::remove_file(gen_dir.join(MATERIAL_FILE_NAME)).unwrap();
        assert_eq!(std::fs::read_dir(&gen_dir).unwrap().count(), 0);

        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        remove_current_symlink(secrets_fd.as_fd()).unwrap();
        let intent = RetireIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            epochs: vec![1],
            stage_names: vec![],
            high_water_epoch: 1,
            phase: RetirePhase::CurrentRemoved,
        };
        execute_retire(secrets_fd.as_fd(), generations_fd.as_fd(), &cfg, intent).unwrap();

        assert!(!gen_dir.exists());
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert!(marker.retired);
        assert!(read_txlog(secrets_fd.as_fd(), vm, kind).unwrap().is_none());
    }

    #[test]
    fn execute_retire_cleans_up_mixed_crashed_and_still_materialized_generations() {
        // A multi-generation retry: one epoch was left in the empty
        // post-material-unlink checkpoint by a prior crashed attempt,
        // another is still fully materialized and was never touched.
        // Both must be correctly removed in one pass.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "mixed-crash-retry";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();
        rotate(vm, kind, material(b"e2"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let generations_dir = paths.kind_root.join(GENERATIONS_DIR_NAME);
        let gen1_dir = generations_dir.join("1");
        let gen2_dir = generations_dir.join("2");
        // Epoch 1: crashed mid-deletion (material unlinked, directory
        // left behind). Epoch 2: untouched, still fully materialized.
        std::fs::remove_file(gen1_dir.join(MATERIAL_FILE_NAME)).unwrap();
        assert_eq!(std::fs::read_dir(&gen1_dir).unwrap().count(), 0);
        assert!(gen2_dir.join(MATERIAL_FILE_NAME).exists());

        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        remove_current_symlink(secrets_fd.as_fd()).unwrap();
        let intent = RetireIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            epochs: vec![1, 2],
            stage_names: vec![],
            high_water_epoch: 2,
            phase: RetirePhase::CurrentRemoved,
        };
        execute_retire(secrets_fd.as_fd(), generations_fd.as_fd(), &cfg, intent).unwrap();

        assert!(!gen1_dir.exists());
        assert!(!gen2_dir.exists());
        assert_eq!(std::fs::read_dir(&generations_dir).unwrap().count(), 0);
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert!(marker.retired);
    }

    #[test]
    fn resumed_generation_deletion_retries_a_failed_directory_fsync() {
        // Fault-inject the trailing `fsync_fd(generations_fd)` barrier
        // specifically while finishing a crash-left empty checkpoint:
        // the failure must block phase advancement rather than being
        // silently swallowed, and a bare retry (no other mutation)
        // must complete the whole retirement.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "resumed-deletion-fsync-fault";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::remove_file(gen_dir.join(MATERIAL_FILE_NAME)).unwrap();

        let (secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        remove_current_symlink(secrets_fd.as_fd()).unwrap();
        let intent = RetireIntent {
            vm: vm.to_owned(),
            kind: kind.as_slug().to_owned(),
            epochs: vec![1],
            stage_names: vec![],
            high_water_epoch: 1,
            phase: RetirePhase::CurrentRemoved,
        };
        {
            let _fault = FsyncFaultGuard::enable();
            let err = execute_retire(
                secrets_fd.as_fd(),
                generations_fd.as_fd(),
                &cfg,
                intent.clone(),
            )
            .unwrap_err();
            assert_eq!(err, FailReason::FsyncFailed);
        }
        // The directory entry itself was already removed by
        // `remove_generation` before the fsync it retried failed; the
        // retry below must not require the entry to still be present.
        assert!(!gen_dir.exists());
        execute_retire(secrets_fd.as_fd(), generations_fd.as_fd(), &cfg, intent).unwrap();
        let marker = read_marker(secrets_fd.as_fd()).unwrap().unwrap();
        assert!(marker.retired);
    }

    #[test]
    fn revalidate_generation_before_delete_rejects_a_fake_empty_directory_with_wrong_mode() {
        // An empty directory alone is not sufficient: it must also
        // still be trusted-broker-owned at the expected mode. A
        // foreign/incorrectly-moded replacement fails closed rather
        // than being swept away as "finishing" a legitimate deletion.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths =
            derive_paths("fake-empty-wrong-mode", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();

        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::remove_file(gen_dir.join(MATERIAL_FILE_NAME)).unwrap();
        assert_eq!(std::fs::read_dir(&gen_dir).unwrap().count(), 0);
        let mut perms = std::fs::metadata(&gen_dir).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&gen_dir, perms).unwrap();

        let err = revalidate_generation_before_delete(generations_fd.as_fd(), 1, &cfg).unwrap_err();
        assert_eq!(err, FailReason::RetirementTreeAnomaly);
        // Nothing was deleted: the anomaly is detected, not swept.
        assert!(gen_dir.exists());
    }

    #[test]
    fn revalidate_generation_before_delete_rejects_an_empty_looking_directory_with_a_stray_entry() {
        // Zero *recognized* entries is required, not merely "no
        // `material`". An unrelated leftover entry alongside a
        // missing `material` must still fail closed.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let paths =
            derive_paths("fake-empty-stray-entry", SecretKind::GuestSigningKey, &cfg).unwrap();
        let (_secrets_fd, generations_fd) = open_or_create_secrets_dir(&paths, &cfg).unwrap();
        let mat = material(b"e1");
        let stage_name = random_stage_name();
        ensure_staged(
            generations_fd.as_fd(),
            &cfg,
            &stage_name,
            &mat.digest_hex(),
            Some(&mat),
        )
        .unwrap();
        promote_stage_to_generation(generations_fd.as_fd(), &stage_name, 1, &mat.digest_hex())
            .unwrap();

        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::remove_file(gen_dir.join(MATERIAL_FILE_NAME)).unwrap();
        std::fs::write(gen_dir.join("rogue"), b"unexpected").unwrap();

        let err = revalidate_generation_before_delete(generations_fd.as_fd(), 1, &cfg).unwrap_err();
        assert_eq!(err, FailReason::RetirementTreeAnomaly);
        assert!(gen_dir.exists());
    }

    #[test]
    fn fresh_retire_over_a_crashed_generation_checkpoint_still_fails_closed() {
        // The empty-checkpoint acceptance is reachable *only* from the
        // retire txlog-recovery deletion loop. A brand-new, fresh call
        // to `retire()` (no leftover txlog) must still go through the
        // strict initial enumeration and fail closed over the exact
        // same on-disk state the crash-recovery tests above complete
        // successfully.
        let dir = tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let vm = "fresh-retire-over-crash-state";
        let kind = SecretKind::GuestSigningKey;
        provision(vm, kind, material(b"e1"), &cfg).unwrap();

        let paths = derive_paths(vm, kind, &cfg).unwrap();
        let gen_dir = paths.kind_root.join(GENERATIONS_DIR_NAME).join("1");
        std::fs::remove_file(gen_dir.join(MATERIAL_FILE_NAME)).unwrap();

        let err = retire(vm, kind, &cfg).unwrap_err();
        assert_eq!(err.reason, FailReason::PreviouslyProvisionedMaterialMissing);
        // Fails closed without deleting anything.
        assert!(gen_dir.exists());
    }
}
