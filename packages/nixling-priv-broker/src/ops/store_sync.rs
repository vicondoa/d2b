//! Typed broker handler for the per-VM
//! `/var/lib/nixling/vms/<vm>/store/` hardlink farm.
//!
//! Replaces the `nixling-<vm>-store-sync.service` per-VM systemd
//! oneshot (today: a bash script that hardlink-farms `/nix/store`
//! closure entries into `/var/lib/nixling/vms/<vm>/store`) with a
//! typed broker op.
//!
//! Implementation contract:
//!
//! 1. The per-VM hardlink farm SHARES inodes with `/nix/store`.
//!    The [`nixling_host::hardlink_farm`] primitive already
//!    enforces the same-filesystem check + atomic `current`
//!    symlink swap + stale `current.tmp` reconciliation. This
//!    module MUST NOT re-implement any of that — it is a thin
//!    wrapper that resolves the closure paths from the trusted
//!    bundle and delegates to the primitive.
//! 2. CRITICAL INVARIANT: NEVER `chown -R`, `chmod -R`, or
//!    `setfacl -R` on the per-VM store-view path. Recursive mutations
//!    under `live/` propagate INTO `/nix/store` via the shared inodes and
//!    break ssh's `safe_path()` check. The primitive only issues
//!    `link(2)` + `symlinkat`/`renameat` for live-pool content. This
//!    module may posture broker-created metadata/lock inodes with
//!    single-inode, no-recursion owner/mode updates so daemon preflight sees
//!    the documented matrix.
//! 3. The op is audited with a single terminal `OperationFields::StoreSync`
//!    record carrying the signed ADR 0027 audit schema (see
//!    [`crate::ops::store_sync_audit`]): `generation_id`,
//!    `generation_token`, `sync_status`, `error_stage`, `cleanup_status`,
//!    `cleanup_reason`, `authz_outcome`, link/skip/sweep counts, and the
//!    resolved farm root path.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use nix::fcntl::{flock, FlockArg};
use nixling_core::bundle_resolver::ResolvedStoreViewIntent;
use nixling_host::hardlink_farm::{self, GenerationMarker, HardlinkFarmError};

use crate::ops::store_sync_audit::{
    CleanupReason, CleanupStatus, ErrorStage, StoreSyncAuditContext, StoreSyncAuditFields,
    StoreSyncTimings,
};
use crate::ops::store_view_posture::{posture_store_view_matrix_paths, PostureError};

/// Typed errors for the `StoreSync` handler. Each value classifies the
/// signed ADR 0027 [`ErrorStage`] the attempt failed at (see
/// [`StoreSyncError::error_stage`]) so the dispatch layer can emit a
/// precise terminal `StoreSync` audit record instead of a generic
/// broker-error record.
#[derive(Debug)]
pub enum StoreSyncError {
    /// The wire-supplied generation overflowed `u32` (host-resolver
    /// generations are `u64`; the wire is `u32`).
    GenerationOverflow { wire: u32, resolved: u64 },
    /// The wire-supplied generation does not match the bundle's
    /// resolved store-view intent generation. Refusing fail-closed.
    GenerationMismatch { wire: u32, resolved: u64 },
    /// Bundle resolver returned an intent for a different VM than
    /// the wire `vm_id`.
    VmMismatch { wire: String, resolved: String },
    /// Underlying hardlink-farm primitive returned a typed error
    /// (cross-fs, marker missing/unparseable, I/O failure). `stage`
    /// records WHICH StoreSync phase invoked the primitive — the
    /// primitive's own error cannot disambiguate the phase, so it is
    /// tagged at the call-site.
    HardlinkFarm {
        stage: ErrorStage,
        source: HardlinkFarmError,
    },
}

impl StoreSyncError {
    /// Tag a hardlink-farm primitive failure with the StoreSync stage
    /// that invoked it.
    fn at(stage: ErrorStage, source: HardlinkFarmError) -> Self {
        Self::HardlinkFarm { stage, source }
    }

    /// The signed-schema [`ErrorStage`] this failure maps to.
    ///
    /// Pre-lock request validation (vm / generation identity checked
    /// against the trusted resolved intent) has no filesystem side
    /// effects; it is classified as the earliest pre-materialisation
    /// verification stage, `probe`. Hardlink-farm failures carry the
    /// stage tagged at their call-site. There is no success-shaped
    /// fallback: every variant resolves to a concrete non-`none` stage.
    pub fn error_stage(&self) -> ErrorStage {
        match self {
            Self::GenerationOverflow { .. }
            | Self::GenerationMismatch { .. }
            | Self::VmMismatch { .. } => ErrorStage::Probe,
            Self::HardlinkFarm { stage, .. } => *stage,
        }
    }
}

impl std::fmt::Display for StoreSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GenerationOverflow { wire, resolved } => write!(
                f,
                "store-sync generation overflow: wire={wire} resolved={resolved} exceeds u32"
            ),
            Self::GenerationMismatch { wire, resolved } => write!(
                f,
                "store-sync generation mismatch: wire={wire} resolved={resolved}"
            ),
            Self::VmMismatch { wire, resolved } => write!(
                f,
                "store-sync vm mismatch: wire={wire:?} resolved={resolved:?}"
            ),
            Self::HardlinkFarm { stage, source } => {
                write!(f, "hardlink-farm[{stage:?}]: {source}")
            }
        }
    }
}

impl std::error::Error for StoreSyncError {}

/// Classify a [`build_store_view_cross_mount_safe`] failure into a
/// StoreSync [`ErrorStage`]. A genuine distinct-`st_dev`
/// [`HardlinkFarmError::DifferentFilesystem`] is a fatal topology probe
/// failure (`probe`); everything else from the materialise step
/// (collision, escaped cross-mount, genuine I/O) is a `stage` failure.
///
/// [`build_store_view_cross_mount_safe`]: crate::ops::store_view_farm::build_store_view_cross_mount_safe
fn build_error_stage(err: &HardlinkFarmError) -> ErrorStage {
    match err {
        HardlinkFarmError::DifferentFilesystem { .. } => ErrorStage::Probe,
        _ => ErrorStage::Stage,
    }
}

/// Derive the collision-free on-disk generation id from a resolved
/// store-view intent. Single source of truth shared by [`run_store_sync`]
/// (which keys the materialise/publish steps on it) and the dispatch
/// layer's terminal audit record (which reports it even on a failure
/// before the sync proper begins).
pub fn generation_id_for_intent(intent: &ResolvedStoreViewIntent) -> String {
    let system_path = hardlink_farm::system_store_path(&intent.closure_paths);
    hardlink_farm::generation_id(&intent.closure_paths, system_path)
}

/// Map a [`run_store_sync`] result onto the signed ADR 0027 terminal
/// `StoreSync` audit record. Success becomes the pure fast-path or the
/// deferred-cleanup non-fast-path shape; failure becomes the `failed`
/// shape carrying the error's classified [`ErrorStage`]
/// (`sync_status=failed`, `cleanup_status=not_attempted`,
/// `cleanup_reason=none`). Correct-by-construction: the returned record
/// always passes [`StoreSyncAuditFields::validate`].
pub fn audit_fields_for_result(
    ctx: StoreSyncAuditContext,
    result: &Result<StoreSyncOutcome, StoreSyncError>,
) -> StoreSyncAuditFields {
    match result {
        Ok(outcome) if outcome.cleanup_status == CleanupStatus::Failed => {
            StoreSyncAuditFields::ok_cleanup_failed(
                ctx,
                outcome.linked_count,
                outcome.skipped_count,
                outcome.retained_generations.clone(),
                outcome.swept_count,
                outcome.fast_path,
            )
        }
        Ok(outcome) if outcome.fast_path => StoreSyncAuditFields::ok_fast_path_with_cleanup(
            ctx,
            outcome.retained_generations.clone(),
            outcome.swept_count,
            outcome.cleanup_status,
            outcome.cleanup_reason,
        ),
        Ok(outcome) => StoreSyncAuditFields::ok_non_fast_path_with_cleanup(
            ctx,
            outcome.linked_count,
            outcome.skipped_count,
            outcome.retained_generations.clone(),
            outcome.swept_count,
            outcome.cleanup_status,
            outcome.cleanup_reason,
        ),
        Err(err) => StoreSyncAuditFields::failed(ctx, err.error_stage()),
    }
}

/// Outcome of a successful `StoreSync` op: the activated generation
/// token + the collision-free on-disk generation id + the per-VM
/// hardlink-farm root path + top-level link accounting. Consumed by the
/// dispatch layer to build the wire `StoreSyncResponse` + the signed
/// `StoreSync` terminal audit record.
///
/// `generation_token` is the u32 wire/display token; `generation_id` is
/// the ADR 0027 on-disk key (full-closure SHA-256). `linked_count` /
/// `skipped_count` are the top-level basenames newly hardlinked into
/// `live/` vs already present (`linked + skipped == closure_count` on a
/// complete sync). `fast_path` records whether a complete, consistent
/// same-generation layout already existed so no relink/republish ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSyncOutcome {
    pub vm: String,
    pub generation_token: u32,
    pub generation_id: String,
    pub hardlink_farm_path: PathBuf,
    pub closure_count: u32,
    pub linked_count: u32,
    pub skipped_count: u32,
    pub retained_generations: Vec<u32>,
    pub swept_count: u32,
    pub fast_path: bool,
    pub cleanup_deferred: bool,
    pub cleanup_status: CleanupStatus,
    pub cleanup_reason: CleanupReason,
    pub timings: StoreSyncTimings,
}

/// Drive the per-VM hardlink-farm sync for one bundle-resolved
/// closure intent + wire-supplied generation.
///
/// Pure-shaped: takes a resolved intent + the wire's expected
/// `(vm, generation)` and returns either a typed outcome or a
/// typed error. The dispatch layer is responsible for resolving
/// `bundle_closure_ref` against the trusted bundle BEFORE calling
/// this function — the daemon never names raw closure paths.
///
/// ADR 0027 split layout: the broker is the sole canonical writer of
/// `live/` (flat hardlink pool), `meta/generations/<id>/` (guest-served
/// metadata), `state/generations/<id>/` (host-only), and the
/// `gcroots/generation-<id>` root, keyed by the collision-free
/// [`hardlink_farm::generation_id`]. Publish order is fixed:
/// materialise → db.dump → `state/current` → `meta/current` → live
/// marker LAST.
///
/// CRITICAL: This function uses the
/// [`hardlink_farm::build_store_view`] primitive + its publish helpers.
/// It MUST NOT call `chown`, `chmod`, `setfacl`, or any other recursive
/// ownership/permission op on the per-VM store-view path — mutations
/// there propagate INTO `/nix/store` via the shared inodes of the
/// hardlink farm.
pub fn run_store_sync(
    intent: &ResolvedStoreViewIntent,
    wire_vm: &str,
    wire_generation: u32,
) -> Result<StoreSyncOutcome, StoreSyncError> {
    run_store_sync_inner(intent, wire_vm, wire_generation, false)
}

pub fn run_store_sync_repair(
    intent: &ResolvedStoreViewIntent,
) -> Result<StoreSyncOutcome, StoreSyncError> {
    let generation =
        u32::try_from(intent.generation).map_err(|_| StoreSyncError::GenerationOverflow {
            wire: u32::MAX,
            resolved: intent.generation,
        })?;
    run_store_sync_inner(intent, &intent.vm, generation, true)
}

fn run_store_sync_inner(
    intent: &ResolvedStoreViewIntent,
    wire_vm: &str,
    wire_generation: u32,
    force_republish: bool,
) -> Result<StoreSyncOutcome, StoreSyncError> {
    let total_started = Instant::now();
    if intent.vm != wire_vm {
        return Err(StoreSyncError::VmMismatch {
            wire: wire_vm.to_owned(),
            resolved: intent.vm.clone(),
        });
    }

    let resolved_generation = intent.generation;
    let resolved_u32 =
        u32::try_from(resolved_generation).map_err(|_| StoreSyncError::GenerationOverflow {
            wire: wire_generation,
            resolved: resolved_generation,
        })?;
    if resolved_u32 != wire_generation {
        return Err(StoreSyncError::GenerationMismatch {
            wire: wire_generation,
            resolved: resolved_generation,
        });
    }

    let lock_wait_started = Instant::now();
    let _lock = acquire_sync_lock(&intent.hardlink_farm_path)?;
    let mut timings = StoreSyncTimings {
        lock_wait_ms: elapsed_ms(lock_wait_started),
        ..Default::default()
    };
    let lock_hold_started = Instant::now();

    let probe_started = Instant::now();
    posture_store_view_matrix_paths(&intent.hardlink_farm_path, &intent.vm)
        .map_err(|err| posture_error(ErrorStage::Lock, err))?;

    // Reconcile possible stale `state/current.tmp` / `meta/current.tmp`
    // left over by a previous crashed publish BEFORE building the new
    // generation — keeps the split layout in a known-good shape.
    hardlink_farm::reconcile_split_current_tmp(&intent.hardlink_farm_path)
        .map_err(|e| StoreSyncError::at(ErrorStage::CurrentSwap, e))?;

    // Derive the collision-free on-disk key ONCE so the fast-path probe
    // and the materialise/publish steps agree on the same generation id.
    let generation_id = generation_id_for_intent(intent);

    // Record the previously-published generation (host-only `state`
    // view) so retention can keep N-1 alongside the new generation.
    let previous_id = hardlink_farm::read_state_current_id(&intent.hardlink_farm_path);
    let previous_token = previous_id
        .as_deref()
        .filter(|id| *id != generation_id)
        .and_then(|id| hardlink_farm::read_generation_token(&intent.hardlink_farm_path, id));

    let marker = GenerationMarker {
        closure_hash: intent.closure_identity(),
        nixling_version: env!("CARGO_PKG_VERSION").to_owned(),
        activated_at: format!("unix-{}", current_unix_ms()),
        vm: intent.vm.clone(),
        generation_number: resolved_u32,
    };
    timings.probe_ms = elapsed_ms(probe_started);

    // Fast path: a complete, consistent same-generation split layout is
    // already published (state/current == meta/current == generation_id,
    // host marker matches, live marker + all top-level basenames
    // present). Skip relinking and republishing; preserve old behaviour.
    let verify_started = Instant::now();
    let fast_path = !force_republish
        && hardlink_farm::split_fast_path_ready(
            &intent.hardlink_farm_path,
            &generation_id,
            &intent.vm,
            &intent.closure_paths,
        );
    timings.verify_ms = elapsed_ms(verify_started);

    let closure_count = u32::try_from(intent.closure_paths.len()).unwrap_or(u32::MAX);

    let link_counts = if !fast_path {
        let stage_started = Instant::now();
        // Materialise the new generation inside a private mount namespace
        // where `/nix/store` is lazily detached, so the build succeeds
        // even when `/nix/store` is a separate (bind) mount from
        // `/var/lib/nixling`. The publish steps below only touch symlinks
        // / byte copies on the root fs (no `link(2)` cross-mount hazard),
        // so they stay in-process.
        let counts = crate::ops::store_view_farm::build_store_view_cross_mount_safe(
            &intent.hardlink_farm_path,
            &generation_id,
            &intent.closure_paths,
            &marker,
        )
        .map_err(|e| StoreSyncError::at(build_error_stage(&e), e))?;
        timings.stage_ms = elapsed_ms(stage_started);

        let metadata_started = Instant::now();
        posture_store_view_matrix_paths(&intent.hardlink_farm_path, &intent.vm)
            .map_err(|err| posture_error(ErrorStage::Metadata, err))?;
        hardlink_farm::write_meta_db_dump(
            &intent.hardlink_farm_path,
            &generation_id,
            &intent.db_dump_path,
        )
        .map_err(|e| StoreSyncError::at(ErrorStage::Metadata, e))?;
        // ADR 0027 publish ordering: state/current first (host view is
        // never behind), meta/current next (guest view), live marker
        // LAST (its existence implies a fully-published generation).
        hardlink_farm::swap_state_current(&intent.hardlink_farm_path, &generation_id)
            .map_err(|e| StoreSyncError::at(ErrorStage::CurrentSwap, e))?;
        hardlink_farm::swap_meta_current(&intent.hardlink_farm_path, &generation_id)
            .map_err(|e| StoreSyncError::at(ErrorStage::CurrentSwap, e))?;
        hardlink_farm::plant_live_marker(&intent.hardlink_farm_path, &intent.vm)
            .map_err(|e| StoreSyncError::at(ErrorStage::Marker, e))?;
        posture_store_view_matrix_paths(&intent.hardlink_farm_path, &intent.vm)
            .map_err(|err| posture_error(ErrorStage::Marker, err))?;
        timings.metadata_ms = elapsed_ms(metadata_started);
        (counts.linked, counts.skipped)
    } else {
        // Pure fast path: nothing relinked, every top-level basename is
        // already present in `live/` (ADR 0027 audit-schema shape).
        (0, closure_count)
    };
    let (linked_count, skipped_count) = link_counts;

    // Retention is expressed in u32 wire tokens for now; the on-disk key
    // moved to generation_id but the wire/audit still speak the token.
    let mut retained = vec![resolved_u32];
    if let Some(previous) = previous_token.filter(|previous| *previous != resolved_u32) {
        retained.push(previous);
    }
    let retained_ids = {
        let mut ids = vec![generation_id.clone()];
        if let Some(previous_id) = previous_id.filter(|id| *id != generation_id) {
            ids.push(previous_id);
        }
        ids
    };
    let cleanup_started = Instant::now();
    let cleanup = cleanup_store_view(&intent.hardlink_farm_path, &intent.vm, &retained_ids);
    timings.cleanup_ms = elapsed_ms(cleanup_started);
    let (cleanup_status, cleanup_reason, swept_count) = match cleanup {
        CleanupOutcome::Completed { swept_count } => {
            (CleanupStatus::Completed, CleanupReason::None, swept_count)
        }
        CleanupOutcome::DeferredOnline => {
            (CleanupStatus::DeferredOnline, CleanupReason::VmRunning, 0)
        }
        CleanupOutcome::DeferredMetadata => (
            CleanupStatus::DeferredMetadata,
            CleanupReason::MissingRetainedMetadata,
            0,
        ),
        CleanupOutcome::Failed { swept_count } => {
            (CleanupStatus::Failed, CleanupReason::IoError, swept_count)
        }
    };
    let cleanup_deferred = matches!(
        cleanup_status,
        CleanupStatus::DeferredOnline
            | CleanupStatus::DeferredAmbiguous
            | CleanupStatus::DeferredMetadata
    );
    tracing::info!(
        vm = %intent.vm,
        generation = resolved_generation,
        generation_id = %generation_id,
        fast_path,
        force_republish,
        cleanup_status = ?cleanup_status,
        cleanup_reason = ?cleanup_reason,
        swept_count,
        "store-sync cleanup disposition"
    );
    timings.lock_hold_ms = elapsed_ms(lock_hold_started);
    timings.total_ms = elapsed_ms(total_started);

    Ok(StoreSyncOutcome {
        vm: intent.vm.clone(),
        generation_token: resolved_u32,
        generation_id,
        hardlink_farm_path: intent.hardlink_farm_path.clone(),
        closure_count,
        linked_count,
        skipped_count,
        retained_generations: retained,
        swept_count,
        fast_path,
        cleanup_deferred,
        cleanup_status,
        cleanup_reason,
        timings,
    })
}

enum CleanupOutcome {
    Completed { swept_count: u32 },
    DeferredOnline,
    DeferredMetadata,
    Failed { swept_count: u32 },
}

fn cleanup_store_view(store_root: &Path, vm: &str, retained_ids: &[String]) -> CleanupOutcome {
    if live_pool_may_be_served(store_root, vm) {
        return CleanupOutcome::DeferredOnline;
    }
    match cleanup_store_view_inner(store_root, retained_ids) {
        Ok(swept_count) => CleanupOutcome::Completed { swept_count },
        Err(CleanupError::MissingMetadata) => CleanupOutcome::DeferredMetadata,
        Err(CleanupError::Io { swept_count }) => CleanupOutcome::Failed { swept_count },
    }
}

enum CleanupError {
    MissingMetadata,
    Io { swept_count: u32 },
}

fn cleanup_store_view_inner(
    store_root: &Path,
    retained_ids: &[String],
) -> Result<u32, CleanupError> {
    let retained: std::collections::BTreeSet<&str> =
        retained_ids.iter().map(String::as_str).collect();
    let mut desired = std::collections::BTreeSet::new();
    for id in retained_ids {
        let store_paths = hardlink_farm::meta_generation_dir(store_root, id).join("store-paths");
        let raw =
            std::fs::read_to_string(&store_paths).map_err(|_| CleanupError::MissingMetadata)?;
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let path = Path::new(line);
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return Err(CleanupError::MissingMetadata);
            };
            desired.insert(name.to_owned());
        }
    }

    let mut swept = 0u32;
    let live = hardlink_farm::live_dir(store_root);
    if let Ok(entries) = std::fs::read_dir(&live) {
        for entry in entries {
            let entry = entry.map_err(|_| CleanupError::Io { swept_count: swept })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with(".nixling-marker-") || desired.contains(name) {
                continue;
            }
            remove_path(&entry.path()).map_err(|_| CleanupError::Io { swept_count: swept })?;
            swept = swept.saturating_add(1);
        }
    }
    prune_generation_dir(
        &hardlink_farm::meta_dir(store_root).join("generations"),
        &retained,
    )
    .map_err(|_| CleanupError::Io { swept_count: swept })?;
    prune_generation_dir(
        &hardlink_farm::state_dir(store_root).join("generations"),
        &retained,
    )
    .map_err(|_| CleanupError::Io { swept_count: swept })?;
    prune_gcroots(&hardlink_farm::gcroots_dir(store_root), &retained)
        .map_err(|_| CleanupError::Io { swept_count: swept })?;
    Ok(swept)
}

fn live_pool_may_be_served(store_root: &Path, vm: &str) -> bool {
    let live = hardlink_farm::live_dir(store_root).display().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let cmdline = entry.path().join("cmdline");
        let Ok(raw) = std::fs::read(&cmdline) else {
            continue;
        };
        let text = String::from_utf8_lossy(&raw);
        if text.contains(&live) || (text.contains("virtiofs") && text.contains(vm)) {
            return true;
        }
    }
    false
}

fn prune_generation_dir(
    generations_dir: &Path,
    retained: &std::collections::BTreeSet<&str>,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(generations_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !retained.contains(name) {
            remove_path(&entry.path())?;
        }
    }
    Ok(())
}

fn prune_gcroots(
    gcroots: &Path,
    retained: &std::collections::BTreeSet<&str>,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(gcroots) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix("generation-"))
        else {
            continue;
        };
        if !retained.contains(id) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn posture_error(stage: ErrorStage, err: PostureError) -> StoreSyncError {
    StoreSyncError::at(
        stage,
        HardlinkFarmError::Io {
            path: err.path,
            detail: err.detail,
        },
    )
}

fn acquire_sync_lock(farm_root: &Path) -> Result<File, StoreSyncError> {
    std::fs::create_dir_all(farm_root).map_err(|err| {
        StoreSyncError::at(
            ErrorStage::Lock,
            HardlinkFarmError::Io {
                path: farm_root.display().to_string(),
                detail: format!("create farm root for sync.lock: {err}"),
            },
        )
    })?;
    let path = hardlink_farm::sync_lock_path(farm_root);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .mode(0o600)
        .write(true)
        .open(&path)
        .map_err(|err| {
            StoreSyncError::at(
                ErrorStage::Lock,
                HardlinkFarmError::Io {
                    path: path.display().to_string(),
                    detail: format!("open sync.lock: {err}"),
                },
            )
        })?;
    flock(file.as_raw_fd(), FlockArg::LockExclusive).map_err(|err| {
        StoreSyncError::at(
            ErrorStage::Lock,
            HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: format!("lock sync.lock: {err}"),
            },
        )
    })?;
    Ok(file)
}

fn current_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn build_fake_closure(root: &std::path::Path, n: usize) -> Vec<PathBuf> {
        let src = root.join("nix-store-mock");
        std::fs::create_dir_all(&src).unwrap();
        let mut out = Vec::new();
        for i in 0..n {
            let path = src.join(format!("xxxxxxxxxxxxxxxx-fake-{i}"));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("hello"), format!("payload-{i}")).unwrap();
            out.push(path);
        }
        out
    }

    fn intent_with(
        root: &std::path::Path,
        vm: &str,
        generation: u64,
        n: usize,
    ) -> ResolvedStoreViewIntent {
        let farm = root.join("vms").join(vm).join("store-view");
        let db_dump_path = root.join(format!("{vm}-{generation}.db.dump"));
        std::fs::write(&db_dump_path, format!("db-dump-{vm}-{generation}")).unwrap();
        std::fs::create_dir_all(&farm).unwrap();
        let target = farm.join("current");
        ResolvedStoreViewIntent {
            intent_id: format!("store-view:vm:{vm}"),
            vm: vm.to_owned(),
            generation,
            hardlink_farm_path: farm,
            target_view_path: target,
            closure_paths: build_fake_closure(root, n),
            db_dump_path,
        }
    }

    #[test]
    fn happy_path_populates_split_layout_and_swaps_currents() {
        use std::os::unix::fs::MetadataExt;

        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "alpha", 7, 2);
        let outcome = run_store_sync(&intent, "alpha", 7).expect("happy path succeeds");

        assert_eq!(outcome.vm, "alpha");
        assert_eq!(outcome.generation_token, 7);
        assert_eq!(outcome.closure_count, 2);
        // First sync of a fresh farm links every top-level basename.
        assert!(!outcome.fast_path);
        assert_eq!(outcome.linked_count, 2);
        assert_eq!(outcome.skipped_count, 0);
        assert_eq!(
            outcome.linked_count + outcome.skipped_count,
            outcome.closure_count
        );

        let farm = &intent.hardlink_farm_path;
        let gid = hardlink_farm::generation_id(
            &intent.closure_paths,
            hardlink_farm::system_store_path(&intent.closure_paths),
        );
        assert_eq!(outcome.generation_id, gid);

        // Guest-served metadata lives under meta/generations/<id>/ with
        // store-paths + meta.json + db.dump; never a host-only system
        // symlink or marker.
        let meta_gen = farm.join("meta").join("generations").join(&gid);
        assert!(meta_gen.join("store-paths").exists());
        assert!(meta_gen.join("meta.json").exists());
        assert!(meta_gen.join("db.dump").exists());
        assert!(!meta_gen.join("marker.json").exists());
        assert!(!meta_gen.join("system").exists());

        // Host-only metadata lives under state/generations/<id>/ with the
        // marker, system symlink, and host meta.json; never served to the
        // guest.
        let state_gen = farm.join("state").join("generations").join(&gid);
        assert!(state_gen.join("marker.json").exists());
        assert!(state_gen.join("system").exists());
        assert!(state_gen.join("meta.json").exists());

        // Flat live pool + zero-length readiness marker.
        assert!(farm.join("live/xxxxxxxxxxxxxxxx-fake-0/hello").exists());
        assert!(farm.join("live/xxxxxxxxxxxxxxxx-fake-1/hello").exists());
        let live_marker = farm.join("live/.nixling-marker-alpha");
        assert!(live_marker.exists());
        assert_eq!(
            std::fs::metadata(&live_marker).unwrap().len(),
            0,
            "live marker is zero-length"
        );

        // gcroots/generation-<id> planted (host-only).
        assert!(farm
            .join("gcroots")
            .join(format!("generation-{gid}"))
            .exists());

        // Both currents resolve to the same generation id; no stale tmp.
        let state_current = std::fs::read_link(farm.join("state/current")).unwrap();
        assert_eq!(state_current, PathBuf::from("generations").join(&gid));
        let meta_current = std::fs::read_link(farm.join("meta/current")).unwrap();
        assert_eq!(meta_current, PathBuf::from("generations").join(&gid));
        assert!(!farm.join("state/current.tmp").exists());
        assert!(!farm.join("meta/current.tmp").exists());

        // Host-only roots must NOT appear under the guest-served meta/.
        assert!(!farm.join("meta/state").exists());
        assert!(!farm.join("meta/gcroots").exists());

        // StoreSync must posture the single inodes it creates to match
        // daemon ownership preflight. Under cfg(test), the symbolic
        // nixlingd/nixling/users principals resolve to the current uid/gid.
        let expected_uid = nix::unistd::Uid::current().as_raw();
        let expected_gid = nix::unistd::Gid::current().as_raw();
        for (path, mode) in [
            (farm.to_path_buf(), 0o755),
            (farm.join("live"), 0o755),
            (farm.join("meta"), 0o755),
            (farm.join("meta/generations"), 0o755),
            (farm.join("state"), 0o750),
            (farm.join("state/generations"), 0o750),
            (farm.join("gcroots"), 0o750),
            (farm.join("sync.lock"), 0o600),
            (farm.join("live/.nixling-marker-alpha"), 0o644),
        ] {
            let meta = std::fs::symlink_metadata(&path).unwrap_or_else(|err| {
                panic!("stat {}: {err}", path.display());
            });
            assert_eq!(meta.uid(), expected_uid, "{} owner uid", path.display());
            assert_eq!(meta.gid(), expected_gid, "{} owner gid", path.display());
            assert_eq!(meta.mode() & 0o777, mode, "{} mode", path.display());
        }
    }

    #[test]
    fn guest_meta_excludes_host_only_fields() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "zeta", 9, 1);
        run_store_sync(&intent, "zeta", 9).expect("sync succeeds");

        let gid = hardlink_farm::generation_id(
            &intent.closure_paths,
            hardlink_farm::system_store_path(&intent.closure_paths),
        );
        let guest_meta = intent
            .hardlink_farm_path
            .join("meta")
            .join("generations")
            .join(&gid)
            .join("meta.json");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&guest_meta).unwrap()).unwrap();
        let obj = value.as_object().unwrap();
        // Exact guest allow-list: nothing host-only (no vm, linked_count,
        // skipped_count, or system path) leaks into the guest document.
        for forbidden in ["vm", "linked_count", "skipped_count", "system"] {
            assert!(
                !obj.contains_key(forbidden),
                "guest meta.json must not expose host-only field {forbidden}"
            );
        }
        assert_eq!(
            obj.get("generation_id").and_then(|v| v.as_str()),
            Some(gid.as_str())
        );
    }

    #[test]
    fn second_sync_same_closure_takes_fast_path() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "omega", 4, 2);
        run_store_sync(&intent, "omega", 4).expect("first sync succeeds");
        // A second sync with the identical closure must detect the
        // already-published generation and short-circuit without error,
        // leaving the currents pointing at the same generation id.
        let outcome = run_store_sync(&intent, "omega", 4).expect("second sync succeeds");
        let gid = hardlink_farm::generation_id(
            &intent.closure_paths,
            hardlink_farm::system_store_path(&intent.closure_paths),
        );
        assert_eq!(outcome.generation_id, gid);
        // Pure fast path: nothing relinked, every basename already present.
        assert!(outcome.fast_path);
        assert_eq!(outcome.linked_count, 0);
        assert_eq!(outcome.skipped_count, outcome.closure_count);
        assert_eq!(outcome.swept_count, 0);
        let state_current =
            std::fs::read_link(intent.hardlink_farm_path.join("state/current")).unwrap();
        assert_eq!(state_current, PathBuf::from("generations").join(&gid));
    }

    #[test]
    fn non_fast_sync_sweeps_stale_live_entries_when_not_served() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "theta", 8, 1);
        let stale = intent
            .hardlink_farm_path
            .join("live/zzzzzzzzzzzzzzzz-stale");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("payload"), b"stale").unwrap();

        let outcome = run_store_sync(&intent, "theta", 8).expect("sync succeeds");
        assert_eq!(outcome.cleanup_status, CleanupStatus::Completed);
        assert_eq!(outcome.cleanup_reason, CleanupReason::None);
        assert_eq!(outcome.swept_count, 1);
        assert!(!stale.exists());
        assert!(!outcome.cleanup_deferred);
    }

    #[test]
    fn farm_shares_inodes_with_source_no_recursive_chown() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "beta", 3, 1);

        let src_file = intent.closure_paths[0].join("hello");
        let pre_meta = std::fs::metadata(&src_file).unwrap();
        let pre_ino = pre_meta.ino();
        let pre_mode = pre_meta.mode();
        let pre_uid = pre_meta.uid();
        let pre_gid = pre_meta.gid();

        run_store_sync(&intent, "beta", 3).expect("sync succeeds");

        let linked = intent
            .hardlink_farm_path
            .join("live/xxxxxxxxxxxxxxxx-fake-0/hello");
        let linked_meta = std::fs::metadata(&linked).unwrap();
        // Shared inode: this is the "hardlink farm" contract.
        assert_eq!(linked_meta.ino(), pre_ino, "farm shares inodes with source");

        // CRITICAL invariant: the per-VM store-view path must NOT
        // propagate mode/uid/gid mutations into /nix/store via the
        // shared inodes. We assert the source file's mode/owner is
        // byte-identical post-sync — the broker handler must never
        // call chown/chmod/setfacl recursively across the farm.
        let post_meta = std::fs::metadata(&src_file).unwrap();
        assert_eq!(post_meta.mode(), pre_mode, "source mode unchanged");
        assert_eq!(post_meta.uid(), pre_uid, "source uid unchanged");
        assert_eq!(post_meta.gid(), pre_gid, "source gid unchanged");
    }

    #[test]
    fn refuses_generation_mismatch() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "gamma", 5, 1);
        let err = run_store_sync(&intent, "gamma", 6).expect_err("generation mismatch refused");
        assert!(matches!(
            err,
            StoreSyncError::GenerationMismatch {
                wire: 6,
                resolved: 5
            }
        ));
        // Pre-lock request validation classifies as the `probe` stage
        // (earliest pre-materialisation verification; no FS side effects).
        assert_eq!(err.error_stage(), ErrorStage::Probe);
    }

    #[test]
    fn refuses_vm_mismatch() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "delta", 1, 1);
        let err = run_store_sync(&intent, "epsilon", 1).expect_err("vm mismatch refused");
        assert_eq!(err.error_stage(), ErrorStage::Probe);
        match err {
            StoreSyncError::VmMismatch { wire, resolved } => {
                assert_eq!(wire, "epsilon");
                assert_eq!(resolved, "delta");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn generation_overflow_maps_to_probe_stage() {
        let tmp = tempdir().unwrap();
        // A resolved generation that does not fit in u32 overflows the
        // wire token; this is a pre-lock request guard → probe stage.
        let intent = intent_with(tmp.path(), "kappa", u64::from(u32::MAX) + 1, 1);
        let err = run_store_sync(&intent, "kappa", 0).expect_err("overflow refused");
        assert!(matches!(err, StoreSyncError::GenerationOverflow { .. }));
        assert_eq!(err.error_stage(), ErrorStage::Probe);
    }

    #[test]
    fn build_error_stage_classifies_topology_vs_materialise() {
        // A genuine distinct-st_dev failure is a fatal topology probe.
        assert_eq!(
            build_error_stage(&HardlinkFarmError::DifferentFilesystem {
                a: "/nix/store".to_owned(),
                a_dev: 1,
                b: "/var/lib/nixling".to_owned(),
                b_dev: 2,
            }),
            ErrorStage::Probe
        );
        // Everything else from the materialise step is a `stage` failure:
        // genuine I/O, escaped cross-mount link, generation collision.
        assert_eq!(
            build_error_stage(&HardlinkFarmError::Io {
                path: "/x".to_owned(),
                detail: "boom".to_owned(),
            }),
            ErrorStage::Stage
        );
        assert_eq!(
            build_error_stage(&HardlinkFarmError::CrossMountLink {
                source: "/nix/store/x".to_owned(),
                destination: "/var/lib/nixling/x".to_owned(),
                dev: 1,
            }),
            ErrorStage::Stage
        );
        assert_eq!(
            build_error_stage(&HardlinkFarmError::GenerationCollision {
                generation_dir: "/x".to_owned(),
                existing: "a".to_owned(),
                incoming: "b".to_owned(),
            }),
            ErrorStage::Stage
        );
    }

    #[test]
    fn failed_sync_before_lock_writes_no_guest_metadata() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "sigma", 5, 1);
        let err = run_store_sync(&intent, "sigma", 6).expect_err("generation mismatch refused");
        assert_eq!(err.error_stage(), ErrorStage::Probe);
        // A failure before the lock/materialise phase must not publish any
        // guest-served metadata or live pool: no meta/ subtree appears.
        let farm = &intent.hardlink_farm_path;
        assert!(
            !farm.join("meta").exists(),
            "failed sync must not create guest-served meta/"
        );
        assert!(
            !farm.join("live").exists(),
            "failed sync must not create the live pool"
        );
        assert!(
            !farm.join("state").exists(),
            "failed sync must not create host-only state/"
        );
    }

    fn audit_ctx_fixture() -> StoreSyncAuditContext {
        StoreSyncAuditContext {
            vm: "corp-vm".to_owned(),
            vm_id: "corp-vm".to_owned(),
            env: None,
            bundle_closure_ref: "store-view:vm:corp-vm".to_owned(),
            hardlink_farm_path: "/var/lib/nixling/vms/corp-vm/store-view".to_owned(),
            generation_id: "g-cafef00d".to_owned(),
            generation_token: 7,
            caller_principal: Some("uid:998/role:daemon".to_owned()),
            closure_count: 3,
            timings: crate::ops::store_sync_audit::StoreSyncTimings::default(),
        }
    }

    fn outcome_fixture(fast_path: bool, linked: u32, skipped: u32) -> StoreSyncOutcome {
        StoreSyncOutcome {
            vm: "corp-vm".to_owned(),
            generation_token: 7,
            generation_id: "g-cafef00d".to_owned(),
            hardlink_farm_path: PathBuf::from("/var/lib/nixling/vms/corp-vm/store-view"),
            closure_count: 3,
            linked_count: linked,
            skipped_count: skipped,
            retained_generations: vec![7],
            swept_count: 0,
            fast_path,
            cleanup_deferred: true,
            cleanup_status: if fast_path {
                CleanupStatus::SkippedFastPath
            } else {
                CleanupStatus::DeferredAmbiguous
            },
            cleanup_reason: if fast_path {
                CleanupReason::FastPath
            } else {
                CleanupReason::RunningGenerationAmbiguous
            },
            timings: StoreSyncTimings {
                total_ms: 10,
                lock_wait_ms: 1,
                lock_hold_ms: 9,
                probe_ms: 2,
                verify_ms: 3,
                stage_ms: if fast_path { 0 } else { 4 },
                metadata_ms: if fast_path { 0 } else { 5 },
                sweep_ms: 0,
                cleanup_ms: 0,
            },
        }
    }

    #[test]
    fn audit_fields_for_result_maps_non_fast_success() {
        use crate::ops::store_sync_audit::{CleanupReason, CleanupStatus, ErrorStage, SyncStatus};
        let result: Result<StoreSyncOutcome, StoreSyncError> = Ok(outcome_fixture(false, 2, 1));
        let fields = audit_fields_for_result(audit_ctx_fixture(), &result);
        fields.validate().expect("non-fast-path record is valid");
        assert_eq!(fields.sync_status, SyncStatus::Ok);
        assert_eq!(fields.error_stage, ErrorStage::None);
        assert!(!fields.fast_path);
        assert_eq!(fields.linked_count, 2);
        assert_eq!(fields.skipped_count, 1);
        assert_eq!(fields.cleanup_status, CleanupStatus::DeferredAmbiguous);
        assert_eq!(
            fields.cleanup_reason,
            CleanupReason::RunningGenerationAmbiguous
        );
    }

    #[test]
    fn audit_fields_for_result_maps_fast_path_success() {
        use crate::ops::store_sync_audit::{CleanupReason, CleanupStatus, SyncStatus};
        let result: Result<StoreSyncOutcome, StoreSyncError> = Ok(outcome_fixture(true, 0, 3));
        let fields = audit_fields_for_result(audit_ctx_fixture(), &result);
        fields.validate().expect("fast-path record is valid");
        assert_eq!(fields.sync_status, SyncStatus::Ok);
        assert!(fields.fast_path);
        assert_eq!(fields.linked_count, 0);
        assert_eq!(fields.skipped_count, fields.closure_count);
        assert_eq!(fields.cleanup_status, CleanupStatus::SkippedFastPath);
        assert_eq!(fields.cleanup_reason, CleanupReason::FastPath);
    }

    #[test]
    fn audit_fields_for_result_preserves_fast_path_cleanup_completed() {
        use crate::ops::store_sync_audit::{CleanupReason, CleanupStatus};
        let mut outcome = outcome_fixture(true, 0, 3);
        outcome.cleanup_status = CleanupStatus::Completed;
        outcome.cleanup_reason = CleanupReason::None;
        outcome.swept_count = 2;
        let result: Result<StoreSyncOutcome, StoreSyncError> = Ok(outcome);
        let fields = audit_fields_for_result(audit_ctx_fixture(), &result);
        fields
            .validate()
            .expect("fast cleanup-completed record is valid");
        assert!(fields.fast_path);
        assert_eq!(fields.cleanup_status, CleanupStatus::Completed);
        assert_eq!(fields.cleanup_reason, CleanupReason::None);
        assert_eq!(fields.swept_count, 2);
    }

    #[test]
    fn audit_fields_for_result_maps_failure_to_signed_failed_shape() {
        use crate::ops::store_sync_audit::{CleanupReason, CleanupStatus, ErrorStage, SyncStatus};
        let result: Result<StoreSyncOutcome, StoreSyncError> =
            Err(StoreSyncError::GenerationMismatch {
                wire: 8,
                resolved: 7,
            });
        let fields = audit_fields_for_result(audit_ctx_fixture(), &result);
        fields.validate().expect("failed record is valid");
        assert_eq!(fields.sync_status, SyncStatus::Failed);
        assert_eq!(fields.error_stage, ErrorStage::Probe);
        // Failure before cleanup: cleanup never ran, no guest-meta leak.
        assert_eq!(fields.cleanup_status, CleanupStatus::NotAttempted);
        assert_eq!(fields.cleanup_reason, CleanupReason::None);
        assert_eq!(
            fields.authz_outcome,
            crate::ops::store_sync_audit::AuthzOutcome::Allow
        );
    }
}
