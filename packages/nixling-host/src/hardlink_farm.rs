//! Hardlink-farm primitive for the per-VM store activation lifecycle.
//!
//! Each daemon-managed VM owns a per-VM store under
//! `/var/lib/nixling/vms/<vm>/store/`. New generations land in
//! `generations/<N>/` as hardlink farms of the per-VM closure (every
//! file is a hardlink to the underlying `/var/lib/nixling/store/`
//! root; the per-VM dir gives the guest its own view of the closure
//! without copying bytes).
//!
//! The primitives in this module are:
//!
//! - [`assert_same_filesystem`]: refuses to hardlink across
//!   filesystems. The store lifecycle requires "same-filesystem fatal
//!   checks" because cross-fs hardlinks silently degrade to copy on
//!   POSIX `link(2)`
//!   (returns `EXDEV`).
//! - [`activate_generation_marker`]: writes a per-generation
//!   `marker.json` recording closure hash + nixling version. The
//!   activate path refuses to mutate any generation dir that lacks
//!   the marker — protects against an operator hand-rolling a
//!   directory and then having `nixling switch` activate it.
//! - [`swap_current_symlink`]: atomic tmp+rename of the
//!   `current -> generations/<N>` symlink. Crash-safe: the
//!   intermediate `current.tmp` symlink is removed by activation
//!   reconciliation if a previous swap crashed mid-way.
//!
//! All primitives are pure-ish: they touch the filesystem but do
//! not require root, and they accept the per-VM root path as a
//! parameter so tests can drive them in a `tempdir`.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Errors the hardlink-farm primitives can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HardlinkFarmError {
    /// Two paths live on different filesystems. Hardlinks across
    /// filesystems are illegal per POSIX `link(2)` (EXDEV).
    DifferentFilesystem {
        a: String,
        a_dev: u64,
        b: String,
        b_dev: u64,
    },
    /// Generation directory exists but lacks the `marker.json` the
    /// activate path expects. Refuses to activate an
    /// operator-rolled directory.
    MarkerMissing { generation_dir: String },
    /// Marker file present but unparseable as JSON.
    MarkerUnparseable { path: String, detail: String },
    /// I/O error during a primitive operation.
    Io { path: String, detail: String },
}

impl std::fmt::Display for HardlinkFarmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DifferentFilesystem { a, a_dev, b, b_dev } => write!(
                f,
                "paths on different filesystems: {a} (dev={a_dev}) vs {b} (dev={b_dev})"
            ),
            Self::MarkerMissing { generation_dir } => write!(
                f,
                "generation {generation_dir} lacks marker.json; refusing to activate"
            ),
            Self::MarkerUnparseable { path, detail } => {
                write!(f, "marker {path}: {detail}")
            }
            Self::Io { path, detail } => write!(f, "I/O error on {path}: {detail}"),
        }
    }
}

impl std::error::Error for HardlinkFarmError {}

/// Marker file shape, one per `generations/<N>/marker.json`.
/// Validates that a generation was created by nixling itself (not
/// hand-rolled by an operator) and pins the closure hash + nixling
/// version at activate time for audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationMarker {
    /// Closure hash the generation was built from.
    pub closure_hash: String,
    /// Nixling version that wrote the marker.
    pub nixling_version: String,
    /// RFC 3339 wall-clock at activate time.
    pub activated_at: String,
    /// Per-VM scope id; cross-check against the activate-time
    /// scope to refuse activating a different VM's generation that
    /// was somehow placed under this VM's `generations/` dir.
    pub vm: String,
    /// Generation number; redundant with the directory name but
    /// pinned in the marker so a rename can be detected.
    pub generation_number: u32,
}

/// Returns `Ok(())` iff `a` and `b` live on the same filesystem
/// (same `st_dev`). Surfaces [`HardlinkFarmError::DifferentFilesystem`]
/// otherwise — the broker uses this BEFORE issuing any `link(2)`
/// call so it can fail-fast with a typed error instead of EXDEV.
pub fn assert_same_filesystem(a: &Path, b: &Path) -> Result<(), HardlinkFarmError> {
    let a_dev = std::fs::metadata(a)
        .map_err(|e| HardlinkFarmError::Io {
            path: a.display().to_string(),
            detail: e.to_string(),
        })?
        .dev();
    let b_dev = std::fs::metadata(b)
        .map_err(|e| HardlinkFarmError::Io {
            path: b.display().to_string(),
            detail: e.to_string(),
        })?
        .dev();
    if a_dev != b_dev {
        return Err(HardlinkFarmError::DifferentFilesystem {
            a: a.display().to_string(),
            a_dev,
            b: b.display().to_string(),
            b_dev,
        });
    }
    Ok(())
}

/// Write the per-generation marker file. Tmp+rename+fsync so a
/// crash mid-write leaves either the old marker or no marker. The
/// parent dir is fsynced AFTER rename so the directory entry is
/// durable on ext4 / xfs / btrfs (matters under power loss; an
/// in-process crash without power loss is already safe via the
/// rename atomicity).
pub fn write_generation_marker(
    generation_dir: &Path,
    marker: &GenerationMarker,
) -> Result<(), HardlinkFarmError> {
    let marker_path = generation_dir.join("marker.json");
    let bytes = serde_json::to_vec_pretty(marker).map_err(|e| HardlinkFarmError::Io {
        path: marker_path.display().to_string(),
        detail: format!("serialize: {e}"),
    })?;
    std::fs::create_dir_all(generation_dir).map_err(|e| HardlinkFarmError::Io {
        path: generation_dir.display().to_string(),
        detail: e.to_string(),
    })?;
    let tmp = marker_path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        f.write_all(&bytes).map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        f.sync_all().map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    std::fs::rename(&tmp, &marker_path).map_err(|e| HardlinkFarmError::Io {
        path: marker_path.display().to_string(),
        detail: e.to_string(),
    })?;
    // W*-fu GPT-5.5 panel notable: fsync the parent dir after
    // rename so the directory entry is durable. tmpfs is a no-op
    // here (it has no on-disk backing) but ext4 / xfs / btrfs need
    // this for full crash safety. Best-effort: errors are
    // non-fatal — the marker file itself is already on disk via
    // the f.sync_all() above.
    if let Ok(dir) = std::fs::File::open(generation_dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Build or reconcile one generation of the per-VM hardlink farm.
///
/// `store_root` is the per-VM farm root (`.../store-view`), not the
/// target generation dir itself. Every source path lands under
/// `generations/<N>/` keyed by its Nix-store basename.
pub fn build_farm(
    store_root: &Path,
    generation_number: u64,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<PathBuf, HardlinkFarmError> {
    let generation_dir = store_root
        .join("generations")
        .join(generation_number.to_string());
    std::fs::create_dir_all(&generation_dir).map_err(|e| HardlinkFarmError::Io {
        path: generation_dir.display().to_string(),
        detail: e.to_string(),
    })?;
    assert_same_filesystem(store_root, &generation_dir)?;
    for source in closure_paths {
        assert_same_filesystem(source, store_root)?;
        let file_name = source.file_name().ok_or_else(|| HardlinkFarmError::Io {
            path: source.display().to_string(),
            detail: "source path has no basename".to_owned(),
        })?;
        hardlink_tree(source, &generation_dir.join(file_name))?;
    }
    write_generation_marker(&generation_dir, marker)?;
    Ok(generation_dir)
}

fn hardlink_tree(source: &Path, destination: &Path) -> Result<(), HardlinkFarmError> {
    let metadata = std::fs::symlink_metadata(source).map_err(|e| HardlinkFarmError::Io {
        path: source.display().to_string(),
        detail: e.to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(source).map_err(|e| HardlinkFarmError::Io {
            path: source.display().to_string(),
            detail: e.to_string(),
        })?;
        if let Ok(existing_target) = std::fs::read_link(destination) {
            if existing_target == target {
                return Ok(());
            }
            std::fs::remove_file(destination).map_err(|e| HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: e.to_string(),
            })?;
        } else if std::fs::symlink_metadata(destination).is_ok() {
            return Err(HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: "existing destination is not a symlink".to_owned(),
            });
        }
        std::os::unix::fs::symlink(&target, destination).map_err(|e| HardlinkFarmError::Io {
            path: destination.display().to_string(),
            detail: e.to_string(),
        })?;
        return Ok(());
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(destination).map_err(|e| HardlinkFarmError::Io {
            path: destination.display().to_string(),
            detail: e.to_string(),
        })?;
        for entry in std::fs::read_dir(source).map_err(|e| HardlinkFarmError::Io {
            path: source.display().to_string(),
            detail: e.to_string(),
        })? {
            let entry = entry.map_err(|e| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: e.to_string(),
            })?;
            hardlink_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if metadata.is_file() {
        if let Ok(existing) = std::fs::symlink_metadata(destination) {
            if existing.is_file() {
                return Ok(());
            }
            return Err(HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: "existing destination is not a file".to_owned(),
            });
        }
        std::fs::hard_link(source, destination).map_err(|e| HardlinkFarmError::Io {
            path: destination.display().to_string(),
            detail: e.to_string(),
        })?;
        return Ok(());
    }
    Err(HardlinkFarmError::Io {
        path: source.display().to_string(),
        detail: "unsupported store path file type".to_owned(),
    })
}

/// Read + parse the per-generation marker. Refuses to activate any
/// generation dir whose marker is missing or unparseable.
pub fn read_generation_marker(
    generation_dir: &Path,
) -> Result<GenerationMarker, HardlinkFarmError> {
    let marker_path = generation_dir.join("marker.json");
    if !marker_path.exists() {
        return Err(HardlinkFarmError::MarkerMissing {
            generation_dir: generation_dir.display().to_string(),
        });
    }
    let bytes = std::fs::read(&marker_path).map_err(|e| HardlinkFarmError::Io {
        path: marker_path.display().to_string(),
        detail: e.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|e| HardlinkFarmError::MarkerUnparseable {
        path: marker_path.display().to_string(),
        detail: e.to_string(),
    })
}

/// Atomic swap of the `<store_root>/current` symlink to point at
/// `<store_root>/generations/<N>`. Implementation:
///
/// 1. Verify the target generation has a valid marker (refuses
///    activation of marker-less dirs).
/// 2. Verify same-filesystem between the symlink parent and target
///    (hardlink farms only work on the same fs; symlinks are
///    cross-fs-tolerant, but the activation contract pins same-fs so
///    the surface stays consistent with the farm itself).
/// 3. Create `<store_root>/current.tmp -> generations/<N>` via
///    `symlinkat`.
/// 4. `renameat2` (via `nix::fcntl::renameat`) the tmp symlink over
///    `current`. POSIX `rename(2)` is atomic for symlinks; the
///    swap is either pre-state or post-state, never partial.
///
/// On a crash between step 3 and 4, `current.tmp` is left behind;
/// [`reconcile_stale_swap_tmp`] removes it on next activate-time.
pub fn swap_current_symlink(
    store_root: &Path,
    generation_number: u32,
) -> Result<(), HardlinkFarmError> {
    let generation_dir = store_root
        .join("generations")
        .join(format!("{generation_number}"));
    // Step 1: marker validation.
    let marker = read_generation_marker(&generation_dir)?;
    if marker.generation_number != generation_number {
        return Err(HardlinkFarmError::MarkerUnparseable {
            path: generation_dir.join("marker.json").display().to_string(),
            detail: format!(
                "marker.generationNumber={} does not match directory name {generation_number}",
                marker.generation_number
            ),
        });
    }

    let current_path = store_root.join("current");
    let tmp_path = store_root.join("current.tmp");

    // Step 2: same-filesystem check between store_root and the
    // generation dir. (Both are typically under the same prefix
    // anyway; the check catches a rare case where an operator
    // bind-mounted `generations/` from another fs.)
    assert_same_filesystem(store_root, &generation_dir)?;

    // Step 3: clean up any stale tmp from a previous crashed swap.
    reconcile_stale_swap_tmp(store_root)?;

    // Step 3: write the new tmp symlink.
    let relative_target = PathBuf::from("generations").join(format!("{generation_number}"));
    std::os::unix::fs::symlink(&relative_target, &tmp_path).map_err(|e| HardlinkFarmError::Io {
        path: tmp_path.display().to_string(),
        detail: e.to_string(),
    })?;

    // Step 4: atomic rename over the existing current symlink.
    std::fs::rename(&tmp_path, &current_path).map_err(|e| HardlinkFarmError::Io {
        path: current_path.display().to_string(),
        detail: e.to_string(),
    })?;

    // W*-fu GPT-5.5 panel notable: fsync the store root AFTER
    // the rename so the directory entry update is durable under
    // power loss (ext4 with `data=writeback`, XFS, etc.). Best
    // effort: errors are non-fatal because the rename itself is
    // POSIX-atomic for symlinks — fsync only matters when the
    // filesystem batches metadata updates.
    if let Ok(dir) = std::fs::File::open(store_root) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Remove a stale `current.tmp` left behind by a previous
/// activation that crashed between symlink-write and rename.
/// Idempotent: no error if the tmp doesn't exist.
pub fn reconcile_stale_swap_tmp(store_root: &Path) -> Result<(), HardlinkFarmError> {
    let tmp_path = store_root.join("current.tmp");
    match std::fs::remove_file(&tmp_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(HardlinkFarmError::Io {
            path: tmp_path.display().to_string(),
            detail: err.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_marker(gen: u32) -> GenerationMarker {
        GenerationMarker {
            closure_hash: format!("sha256:gen{gen}"),
            nixling_version: "0.4.0".to_owned(),
            activated_at: "2026-05-29T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: gen,
        }
    }

    fn build_generation(store: &Path, gen: u32) {
        let dir = store.join("generations").join(format!("{gen}"));
        std::fs::create_dir_all(&dir).unwrap();
        write_generation_marker(&dir, &make_marker(gen)).unwrap();
    }

    #[test]
    fn assert_same_filesystem_matches_self() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a");
        std::fs::create_dir_all(&p).unwrap();
        assert!(assert_same_filesystem(dir.path(), &p).is_ok());
    }

    #[test]
    fn assert_same_filesystem_surfaces_io_error_when_missing() {
        let dir = tempdir().unwrap();
        let result = assert_same_filesystem(dir.path(), &dir.path().join("nonexistent"));
        assert!(matches!(result, Err(HardlinkFarmError::Io { .. })));
    }

    #[test]
    fn build_farm_creates_generation_with_marker_and_hardlinks() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("abc-system");
        let subdir = system_path.join("bin");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("switch-to-configuration"), b"#!/bin/sh\n").unwrap();
        let shared = source_root.join("dep");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("data"), b"hello").unwrap();
        std::os::unix::fs::symlink("../dep/data", system_path.join("data-link")).unwrap();

        let generation_dir = build_farm(
            &farm_root,
            7,
            &[system_path.clone(), shared.clone()],
            &GenerationMarker {
                closure_hash: "sha256:test".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 7,
            },
        )
        .unwrap();

        let farm_binary = generation_dir.join("abc-system/bin/switch-to-configuration");
        assert!(farm_binary.exists());
        assert_eq!(
            std::fs::metadata(&farm_binary).unwrap().ino(),
            std::fs::metadata(system_path.join("bin/switch-to-configuration"))
                .unwrap()
                .ino()
        );
        assert_eq!(
            std::fs::read_link(generation_dir.join("abc-system/data-link")).unwrap(),
            PathBuf::from("../dep/data")
        );
        let marker = read_generation_marker(&generation_dir).unwrap();
        assert_eq!(marker.generation_number, 7);
        assert_eq!(marker.vm, "corp-vm");
    }

    #[test]
    fn build_farm_replaces_wrong_symlink_target() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("alpha-system");
        std::fs::create_dir_all(system_path.join("bin")).unwrap();
        std::fs::write(
            system_path.join("bin/switch-to-configuration"),
            b"#!/bin/sh\n",
        )
        .unwrap();
        let dep_dir = source_root.join("dep");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(dep_dir.join("real"), b"real").unwrap();
        std::fs::write(dep_dir.join("wrong"), b"wrong").unwrap();
        std::os::unix::fs::symlink("../dep/real", system_path.join("data-link")).unwrap();

        let generation_dir = farm_root.join("generations/8/alpha-system");
        std::fs::create_dir_all(&generation_dir).unwrap();
        std::os::unix::fs::symlink("../dep/wrong", generation_dir.join("data-link")).unwrap();

        build_farm(
            &farm_root,
            8,
            std::slice::from_ref(&system_path),
            &GenerationMarker {
                closure_hash: "sha256:test".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 8,
            },
        )
        .unwrap();

        assert_eq!(
            std::fs::read_link(generation_dir.join("data-link")).unwrap(),
            PathBuf::from("../dep/real")
        );
    }

    #[test]
    fn build_farm_recovers_from_broken_symlink_destination() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("beta-system");
        std::fs::create_dir_all(system_path.join("bin")).unwrap();
        std::fs::write(
            system_path.join("bin/switch-to-configuration"),
            b"#!/bin/sh\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("missing-target", system_path.join("data-link")).unwrap();

        let generation_dir = farm_root.join("generations/9/beta-system");
        std::fs::create_dir_all(&generation_dir).unwrap();
        std::os::unix::fs::symlink("broken-before", generation_dir.join("data-link")).unwrap();

        build_farm(
            &farm_root,
            9,
            std::slice::from_ref(&system_path),
            &GenerationMarker {
                closure_hash: "sha256:test".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 9,
            },
        )
        .unwrap();

        assert_eq!(
            std::fs::read_link(generation_dir.join("data-link")).unwrap(),
            PathBuf::from("missing-target")
        );
    }

    #[test]
    fn marker_round_trip() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/1");
        write_generation_marker(&gen_dir, &make_marker(1)).unwrap();
        let read = read_generation_marker(&gen_dir).unwrap();
        assert_eq!(read, make_marker(1));
    }

    #[test]
    fn marker_missing_is_typed_error() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/2");
        std::fs::create_dir_all(&gen_dir).unwrap();
        // No marker written.
        let result = read_generation_marker(&gen_dir);
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerMissing { .. })
        ));
    }

    #[test]
    fn marker_unparseable_is_typed_error() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/3");
        std::fs::create_dir_all(&gen_dir).unwrap();
        std::fs::write(gen_dir.join("marker.json"), b"not json").unwrap();
        let result = read_generation_marker(&gen_dir);
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerUnparseable { .. })
        ));
    }

    #[test]
    fn marker_rejects_unknown_fields() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/4");
        std::fs::create_dir_all(&gen_dir).unwrap();
        // Inject a marker with an extra field — deny_unknown_fields
        // makes this an unparseable error.
        let json = serde_json::json!({
            "closureHash": "sha256:abc",
            "nixlingVersion": "0.4.0",
            "activatedAt": "2026-05-29T09:00:00Z",
            "vm": "corp-vm",
            "generationNumber": 4,
            "extraField": "rejected"
        });
        std::fs::write(
            gen_dir.join("marker.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        let result = read_generation_marker(&gen_dir);
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerUnparseable { .. })
        ));
    }

    #[test]
    fn swap_current_creates_symlink_to_target_generation() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        build_generation(&store, 1);

        swap_current_symlink(&store, 1).unwrap();

        let current = store.join("current");
        assert!(current.exists());
        let target = std::fs::read_link(&current).unwrap();
        assert_eq!(target, PathBuf::from("generations/1"));
    }

    #[test]
    fn swap_current_overwrites_existing_symlink() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        build_generation(&store, 1);
        build_generation(&store, 2);

        swap_current_symlink(&store, 1).unwrap();
        swap_current_symlink(&store, 2).unwrap();

        let target = std::fs::read_link(store.join("current")).unwrap();
        assert_eq!(target, PathBuf::from("generations/2"));
    }

    #[test]
    fn swap_current_refuses_marker_less_generation() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(store.join("generations/5")).unwrap();
        // No marker written.

        let result = swap_current_symlink(&store, 5);
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerMissing { .. })
        ));
    }

    #[test]
    fn swap_current_refuses_marker_with_wrong_generation_number() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        let gen_dir = store.join("generations/6");
        // Marker claims generationNumber = 99 but lives in dir "6".
        let mut bogus_marker = make_marker(99);
        bogus_marker.generation_number = 99;
        write_generation_marker(&gen_dir, &bogus_marker).unwrap();

        let result = swap_current_symlink(&store, 6);
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerUnparseable { .. })
        ));
    }

    #[test]
    fn reconcile_removes_stale_swap_tmp() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        // Simulate a crashed swap: leave current.tmp behind.
        std::os::unix::fs::symlink("generations/1", store.join("current.tmp")).unwrap();
        // `.exists()` follows symlinks; for a dangling symlink it
        // returns false. Use `symlink_metadata` to check link
        // presence regardless of target.
        assert!(std::fs::symlink_metadata(store.join("current.tmp")).is_ok());

        reconcile_stale_swap_tmp(&store).unwrap();
        assert!(std::fs::symlink_metadata(store.join("current.tmp")).is_err());
    }

    #[test]
    fn reconcile_is_idempotent() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        // No current.tmp present; reconcile should be a no-op.
        reconcile_stale_swap_tmp(&store).unwrap();
        // Call twice for idempotency.
        reconcile_stale_swap_tmp(&store).unwrap();
    }

    #[test]
    fn swap_current_cleans_up_stale_tmp_before_writing_new_one() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        build_generation(&store, 1);
        // Leave a stale tmp from a previous crashed swap.
        std::os::unix::fs::symlink("generations/99", store.join("current.tmp")).unwrap();

        swap_current_symlink(&store, 1).unwrap();
        let target = std::fs::read_link(store.join("current")).unwrap();
        assert_eq!(target, PathBuf::from("generations/1"));
    }

    #[test]
    fn marker_round_trip_serializable() {
        let m = make_marker(7);
        let json = serde_json::to_string(&m).unwrap();
        let parsed: GenerationMarker = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, m);
    }
}
