//! P2 ph2-store-sync: typed broker handler for the per-VM
//! `/var/lib/nixling/vms/<vm>/store/` hardlink farm.
//!
//! Replaces the `nixling-<vm>-store-sync.service` per-VM systemd
//! oneshot (today: a bash script that hardlink-farms `/nix/store`
//! closure entries into `/var/lib/nixling/vms/<vm>/store`) with a
//! typed broker op.
//!
//! Implementation contract (plan.md §"ph2-store-sync"):
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
//! 3. The op is audited with `OperationFields::StoreSync` carrying
//!    `vm_id`, `bundle_closure_ref`, `generation`, `closure_count`,
//!    and the resolved farm root path.

use std::path::PathBuf;

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
/// number + the per-VM hardlink-farm root path + the count of
/// top-level closure paths the broker linked in. Consumed by the
/// dispatch layer to build the wire `StoreSyncResponse` + the
/// `OperationFields::StoreSync` audit record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSyncOutcome {
    pub vm: String,
    pub generation: u32,
    pub hardlink_farm_path: PathBuf,
    pub closure_count: u32,
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
/// CRITICAL: This function uses the existing
/// `hardlink_farm::{build_farm, swap_current_symlink}` primitives.
/// It MUST NOT call `chown`, `chmod`, `setfacl`, or any other
/// recursive ownership/permission op on the per-VM store-view
/// path — mutations there propagate INTO `/nix/store` via the
/// shared inodes of the hardlink farm.
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
    let resolved_u32 = u32::try_from(resolved_generation).map_err(|_| {
        StoreSyncError::GenerationOverflow {
            wire: wire_generation,
            resolved: resolved_generation,
        }
    })?;
    if resolved_u32 != wire_generation {
        return Err(StoreSyncError::GenerationMismatch {
            wire: wire_generation,
            resolved: resolved_generation,
        });
    }

    // Reconcile a possible stale `current.tmp` left over by a
    // previous crashed swap BEFORE we start building the new
    // generation — keeps the farm in a known-good shape and
    // exercises the primitive's documented crash-recovery path.
    hardlink_farm::reconcile_stale_swap_tmp(&intent.hardlink_farm_path)?;

    let marker = GenerationMarker {
        closure_hash: format!("store-sync:{}:{}", intent.vm, resolved_generation),
        nixling_version: env!("CARGO_PKG_VERSION").to_owned(),
        activated_at: format!("unix-{}", current_unix_ms()),
        vm: intent.vm.clone(),
        generation_number: resolved_u32,
    };

    hardlink_farm::build_farm(
        &intent.hardlink_farm_path,
        resolved_generation,
        &intent.closure_paths,
        &marker,
    )?;

    hardlink_farm::swap_current_symlink(&intent.hardlink_farm_path, resolved_u32)?;

    let closure_count = u32::try_from(intent.closure_paths.len()).unwrap_or(u32::MAX);

    Ok(StoreSyncOutcome {
        vm: intent.vm.clone(),
        generation: resolved_u32,
        hardlink_farm_path: intent.hardlink_farm_path.clone(),
        closure_count,
    })
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

    fn intent_with(root: &std::path::Path, vm: &str, generation: u64, n: usize) -> ResolvedStoreViewIntent {
        let farm = root.join("vms").join(vm).join("store");
        std::fs::create_dir_all(&farm).unwrap();
        let target = farm.join("current");
        ResolvedStoreViewIntent {
            intent_id: format!("store-view:vm:{vm}"),
            vm: vm.to_owned(),
            generation,
            hardlink_farm_path: farm,
            target_view_path: target,
            closure_paths: build_fake_closure(root, n),
        }
    }

    #[test]
    fn happy_path_populates_farm_and_swaps_current() {
        let tmp = tempdir().unwrap();
        let intent = intent_with(tmp.path(), "alpha", 7, 2);
        let outcome = run_store_sync(&intent, "alpha", 7).expect("happy path succeeds");

        assert_eq!(outcome.vm, "alpha");
        assert_eq!(outcome.generation, 7);
        assert_eq!(outcome.closure_count, 2);

        let gen_dir = intent.hardlink_farm_path.join("generations/7");
        assert!(gen_dir.join("marker.json").exists());
        assert!(gen_dir.join("xxxxxxxxxxxxxxxx-fake-0/hello").exists());
        assert!(gen_dir.join("xxxxxxxxxxxxxxxx-fake-1/hello").exists());

        let current = intent.hardlink_farm_path.join("current");
        let link_target = std::fs::read_link(&current).expect("current is a symlink");
        assert_eq!(link_target, PathBuf::from("generations/7"));
        // Swap atomicity: the tmp must be absent post-swap.
        assert!(!intent.hardlink_farm_path.join("current.tmp").exists());
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
            .join("generations/3/xxxxxxxxxxxxxxxx-fake-0/hello");
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
        assert!(matches!(err, StoreSyncError::GenerationMismatch { wire: 6, resolved: 5 }));
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
