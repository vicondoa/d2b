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
//!    `setfacl -R` on the per-VM store-view path. Mutations there
//!    propagate INTO `/nix/store` via the shared inodes and break
//!    ssh's `safe_path()` check. The primitive only issues
//!    `link(2)` + `symlinkat`/`renameat`, never recursive mode or
//!    owner mutation. This module never invokes `chown`, `chmod`,
//!    or `setfacl` either.
//! 3. The op is audited with a single terminal `OperationFields::StoreSync`
//!    record carrying the signed ADR 0027 audit schema (see
//!    [`crate::ops::store_sync_audit`]): `generation_id`,
//!    `generation_token`, `sync_status`, `error_stage`, `cleanup_status`,
//!    `cleanup_reason`, `authz_outcome`, link/skip/sweep counts, and the
//!    resolved farm root path.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use nix::fcntl::{flock, FlockArg};
use nixling_core::bundle_resolver::ResolvedStoreViewIntent;
use nixling_host::hardlink_farm::{self, GenerationMarker, HardlinkFarmError};

/// Typed errors for the `StoreSync` handler. Maps cleanly onto the
/// dispatch-layer `BrokerError` variants in `runtime.rs`.
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
    /// (cross-fs, marker missing/unparseable, I/O failure).
    HardlinkFarm(HardlinkFarmError),
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
            Self::HardlinkFarm(err) => write!(f, "hardlink-farm: {err}"),
        }
    }
}

impl std::error::Error for StoreSyncError {}

impl From<HardlinkFarmError> for StoreSyncError {
    fn from(err: HardlinkFarmError) -> Self {
        Self::HardlinkFarm(err)
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

    let _lock = acquire_sync_lock(&intent.hardlink_farm_path)?;

    // Reconcile possible stale `state/current.tmp` / `meta/current.tmp`
    // left over by a previous crashed publish BEFORE building the new
    // generation — keeps the split layout in a known-good shape.
    hardlink_farm::reconcile_split_current_tmp(&intent.hardlink_farm_path)?;

    // Derive the collision-free on-disk key ONCE so the fast-path probe
    // and the materialise/publish steps agree on the same generation id.
    let system_path = hardlink_farm::system_store_path(&intent.closure_paths);
    let generation_id = hardlink_farm::generation_id(&intent.closure_paths, system_path);

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

    // Fast path: a complete, consistent same-generation split layout is
    // already published (state/current == meta/current == generation_id,
    // host marker matches, live marker + all top-level basenames
    // present). Skip relinking and republishing; preserve old behaviour.
    let fast_path = hardlink_farm::split_fast_path_ready(
        &intent.hardlink_farm_path,
        &generation_id,
        &intent.vm,
        &intent.closure_paths,
    );

    let closure_count = u32::try_from(intent.closure_paths.len()).unwrap_or(u32::MAX);

    let link_counts = if !fast_path {
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
        )?;
        hardlink_farm::write_meta_db_dump(
            &intent.hardlink_farm_path,
            &generation_id,
            &intent.db_dump_path,
        )?;
        // ADR 0027 publish ordering: state/current first (host view is
        // never behind), meta/current next (guest view), live marker
        // LAST (its existence implies a fully-published generation).
        hardlink_farm::swap_state_current(&intent.hardlink_farm_path, &generation_id)?;
        hardlink_farm::swap_meta_current(&intent.hardlink_farm_path, &generation_id)?;
        hardlink_farm::plant_live_marker(&intent.hardlink_farm_path, &intent.vm)?;
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
    let cleanup_deferred = true;
    let swept_count = 0;
    tracing::info!(
        vm = %intent.vm,
        generation = resolved_generation,
        generation_id = %generation_id,
        fast_path,
        "store-sync cleanup deferred until running-generation retention is available"
    );

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
    })
}

fn acquire_sync_lock(farm_root: &Path) -> Result<File, StoreSyncError> {
    std::fs::create_dir_all(farm_root).map_err(|err| {
        StoreSyncError::HardlinkFarm(HardlinkFarmError::Io {
            path: farm_root.display().to_string(),
            detail: format!("create farm root for sync.lock: {err}"),
        })
    })?;
    let path = hardlink_farm::sync_lock_path(farm_root);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| {
            StoreSyncError::HardlinkFarm(HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: format!("open sync.lock: {err}"),
            })
        })?;
    flock(file.as_raw_fd(), FlockArg::LockExclusive).map_err(|err| {
        StoreSyncError::HardlinkFarm(HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: format!("lock sync.lock: {err}"),
        })
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
    }

    #[test]
    fn refuses_vm_mismatch() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "delta", 1, 1);
        let err = run_store_sync(&intent, "epsilon", 1).expect_err("vm mismatch refused");
        match err {
            StoreSyncError::VmMismatch { wire, resolved } => {
                assert_eq!(wire, "epsilon");
                assert_eq!(resolved, "delta");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
