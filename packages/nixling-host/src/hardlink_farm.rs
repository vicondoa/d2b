//! Hardlink-farm primitive for the per-VM store activation lifecycle.
//!
//! Each daemon-managed VM owns a per-VM store-view under
//! `/var/lib/nixling/vms/<vm>/store-view/`. The guest is served from
//! `live/`, a flat hardlink pool containing the retained VM closure
//! basenames. `generations/<N>/` stores metadata only (`marker.json`,
//! `store-paths`, and the system symlink), so a new generation only
//! materialises top-level store paths that are not already present in
//! `live/`.
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
use std::collections::BTreeSet;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// `EXDEV` ("Invalid cross-device link") errno. `link(2)` returns this
/// when source and destination are on different mounts. Defined locally
/// so this `#![forbid(unsafe_code)]` crate needs no `libc` dependency
/// for a single integer constant.
const EXDEV: i32 = 18;

/// `EMLINK` ("Too many links") errno. `link(2)` returns this when the
/// source inode is already at the filesystem's maximum hardlink count
/// (ext4 `EXT4_LINK_MAX` = 65000). Defined locally for the same reason
/// as [`EXDEV`].
const EMLINK: i32 = 31;

/// Errors the hardlink-farm primitives can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HardlinkFarmError {
    /// Two paths live on genuinely different filesystems (distinct
    /// `st_dev`). Hardlinks across filesystems are impossible; the
    /// store-view farm cannot be built and this is FATAL — no mount
    /// namespace can help. Detected up-front by [`assert_same_filesystem`]
    /// before any `link(2)`.
    DifferentFilesystem {
        a: String,
        a_dev: u64,
        b: String,
        b_dev: u64,
    },
    /// `link(2)` returned `EXDEV` even though source and destination
    /// share the same `st_dev` — i.e. they are on the same underlying
    /// filesystem but in different *vfsmounts* (the canonical case is
    /// NixOS bind-mounting `/nix/store` read-only on top of itself).
    /// Unlike [`DifferentFilesystem`] this is RECOVERABLE: building the
    /// farm inside a private mount namespace where `/nix/store` is
    /// lazily detached makes the two paths share one mount and the
    /// hardlink succeeds.
    CrossMountLink {
        source: String,
        destination: String,
        dev: u64,
    },
    /// Generation directory exists but lacks the `marker.json` the
    /// activate path expects. Refuses to activate an
    /// operator-rolled directory.
    MarkerMissing { generation_dir: String },
    /// Marker file present but unparseable as JSON.
    MarkerUnparseable { path: String, detail: String },
    /// The target generation dir already holds a *different* closure
    /// than the one being built. This is the fail-closed guard for an
    /// (astronomically rare) u32 store-view generation-number collision
    /// between two distinct closures of the same VM: rather than union
    /// both closures into one generation dir (which would corrupt
    /// rollback + the activated store view), refuse the build.
    GenerationCollision {
        generation_dir: String,
        existing: String,
        incoming: String,
    },
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
            Self::CrossMountLink {
                source,
                destination,
                dev,
            } => write!(
                f,
                "cross-mount hardlink refused (EXDEV) on same filesystem (dev={dev}): \
                 {source} -> {destination}"
            ),
            Self::MarkerMissing { generation_dir } => write!(
                f,
                "generation {generation_dir} lacks marker.json; refusing to activate"
            ),
            Self::MarkerUnparseable { path, detail } => {
                write!(f, "marker {path}: {detail}")
            }
            Self::GenerationCollision {
                generation_dir,
                existing,
                incoming,
            } => write!(
                f,
                "store-view generation collision at {generation_dir}: already holds closure \
                 `{existing}`, refusing to build `{incoming}` over it"
            ),
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

/// Schema version for the guest-served generation metadata document
/// (`meta.json`). Bump when the guest-safe allow-list changes.
pub const GUEST_META_SCHEMA_VERSION: u32 = 1;

/// Guest-served, host-authored generation metadata.
///
/// ADR 0027: produced by an **independent** serializer with an exact
/// positive allow-list. The guest serializer never receives the full
/// host audit struct, so it cannot leak `live/`, `state/`, `gcroots/`,
/// marker payloads, caller/authz fields, retained generations, swept
/// counts, timings, cleanup fields, error details, host-only paths, or
/// host-absolute symlinks. The key set is exactly:
/// `schema_version`, `generation_id`, `generation_token`,
/// `sync_status`, `closure_count`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestGenerationMeta {
    pub schema_version: u32,
    /// Collision-free generation identity (full closure identity); the
    /// canonical key, distinct from the truncated u32 display token.
    pub generation_id: String,
    /// Display/wire u32 token. Never the on-disk generation key.
    pub generation_token: u32,
    /// Only `ok` ever reaches the guest: `meta.json` is written after
    /// the generation has materialised successfully.
    pub sync_status: String,
    pub closure_count: u32,
}

/// Wire request for an out-of-process store-view farm build.
///
/// The privileged broker serialises this to a subprocess that runs
/// the hardlink farm build inside a private mount namespace where
/// `/nix/store` is lazily detached (so cross-vfsmount `link(2)` EXDEV
/// — the NixOS `/nix/store` self-bind-mount — does not block the
/// hardlinks). The subprocess deserialises it and calls [`build_farm`].
/// Kept here, next to [`build_farm`] + [`GenerationMarker`], so the
/// broker (serialiser) and the `nixling-activation-helper` binary
/// (deserialiser) share one definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildStoreViewFarmRequest {
    /// Per-VM farm root (`.../store-view`), not the `live/` dir.
    pub farm_root: PathBuf,
    /// Content-derived u32 generation number (carried as u64 to match
    /// [`build_farm`]'s signature; validated to fit u32 upstream).
    pub generation: u64,
    /// Absolute `/nix/store/<...>` closure paths to hardlink in.
    pub closure_paths: Vec<PathBuf>,
    /// Marker pinned into `generations/<N>/marker.json`.
    pub marker: GenerationMarker,
}

/// Return the flat live hardlink pool served by virtiofsd.
pub fn live_dir(store_root: &Path) -> PathBuf {
    store_root.join("live")
}

/// Return the generation metadata directory.
pub fn generation_dir(store_root: &Path, generation_number: u64) -> PathBuf {
    store_root
        .join("generations")
        .join(generation_number.to_string())
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
/// `store_root` is the per-VM farm root (`.../store-view`). Every
/// source path lands under `live/<basename>` if it is not already
/// present. `generations/<N>/` contains metadata only.
pub fn build_farm(
    store_root: &Path,
    generation_number: u64,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<PathBuf, HardlinkFarmError> {
    let generation_dir = generation_dir(store_root, generation_number);
    let live_dir = live_dir(store_root);
    // Fail-closed collision guard: the store-view generation number is
    // a content-derived u32 (see closures-json.nix). If this generation
    // dir already exists with a marker for a DIFFERENT closure, two
    // distinct closures of this VM collided onto the same u32. Refuse
    // rather than hardlink the new closure on top of the old one (which
    // would produce a mixed store view and corrupt rollback). Reusing a
    // dir for the SAME closure stays idempotent.
    let existing_marker_path = generation_dir.join("marker.json");
    if generation_dir.exists() {
        if existing_marker_path.exists() {
            let existing = read_generation_marker(&generation_dir)?;
            if existing.closure_hash != marker.closure_hash {
                return Err(HardlinkFarmError::GenerationCollision {
                    generation_dir: generation_dir.display().to_string(),
                    existing: existing.closure_hash,
                    incoming: marker.closure_hash.clone(),
                });
            }
            if existing.vm == marker.vm
                && existing.generation_number == marker.generation_number
                && live_dir
                    .join(format!(".nixling-marker-{}", marker.vm))
                    .exists()
                && closure_paths.iter().all(|p| {
                    p.file_name()
                        .map(|name| live_dir.join(name).exists())
                        .unwrap_or(false)
                })
            {
                return Ok(generation_dir);
            }
        } else {
            // Populated generation dir with no trusted marker: a build
            // that crashed before write_generation_marker. It is never
            // activatable (swap_current_symlink + read_generation_marker
            // both require the marker) and its contents can't be trusted
            // to belong to this closure — so a colliding closure must not
            // be hardlinked on top of it. Rebuild the generation from
            // scratch instead of unioning the partial leftovers.
            std::fs::remove_dir_all(&generation_dir).map_err(|e| HardlinkFarmError::Io {
                path: generation_dir.display().to_string(),
                detail: e.to_string(),
            })?;
        }
    }
    std::fs::create_dir_all(store_root).map_err(|e| HardlinkFarmError::Io {
        path: store_root.display().to_string(),
        detail: e.to_string(),
    })?;
    std::fs::create_dir_all(&live_dir).map_err(|e| HardlinkFarmError::Io {
        path: live_dir.display().to_string(),
        detail: e.to_string(),
    })?;
    assert_same_filesystem(store_root, &live_dir)?;

    let stage_dir = store_root.join(format!(
        "live.stage.{}.{}",
        generation_number,
        std::process::id()
    ));
    if stage_dir.exists() {
        std::fs::remove_dir_all(&stage_dir).map_err(|e| HardlinkFarmError::Io {
            path: stage_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    std::fs::create_dir_all(&stage_dir).map_err(|e| HardlinkFarmError::Io {
        path: stage_dir.display().to_string(),
        detail: e.to_string(),
    })?;

    let build_result = (|| {
        for source in closure_paths {
            assert_same_filesystem(source, store_root)?;
            let file_name = source.file_name().ok_or_else(|| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: "source path has no basename".to_owned(),
            })?;
            let live_path = live_dir.join(file_name);
            if live_path.exists() {
                continue;
            }
            let staged_path = stage_dir.join(file_name);
            hardlink_tree(source, &staged_path)?;
            match std::fs::rename(&staged_path, &live_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_dir_all(&staged_path);
                }
                Err(err) => {
                    return Err(HardlinkFarmError::Io {
                        path: live_path.display().to_string(),
                        detail: err.to_string(),
                    });
                }
            }
        }
        Ok(())
    })();

    if let Err(err) = build_result {
        let _ = std::fs::remove_dir_all(&stage_dir);
        return Err(err);
    }
    let _ = std::fs::remove_dir_all(&stage_dir);

    std::fs::create_dir_all(&generation_dir).map_err(|e| HardlinkFarmError::Io {
        path: generation_dir.display().to_string(),
        detail: e.to_string(),
    })?;
    assert_same_filesystem(store_root, &generation_dir)?;
    write_store_paths(&generation_dir, closure_paths)?;
    write_system_symlink(&generation_dir, closure_paths)?;
    write_guest_meta(&generation_dir, marker, closure_paths.len())?;
    write_generation_marker(&generation_dir, marker)?;
    write_live_marker(&live_dir, &marker.vm)?;
    Ok(generation_dir)
}

fn write_store_paths(
    generation_dir: &Path,
    closure_paths: &[PathBuf],
) -> Result<(), HardlinkFarmError> {
    let path = generation_dir.join("store-paths");
    let tmp = generation_dir.join("store-paths.tmp");
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        for p in closure_paths {
            writeln!(file, "{}", p.display()).map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        }
        file.sync_all().map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| HardlinkFarmError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

/// Write the guest-served generation metadata (`meta.json`).
///
/// ADR 0027: an independent allow-list serializer. This function is the
/// only path that authors the guest-visible `meta.json`; it builds the
/// document from primitives ([`GuestGenerationMeta`]) and never from the
/// full host audit record, so host-only fields cannot leak to the guest
/// even if a future field is added to the audit struct. tmp+rename+fsync
/// for crash safety.
fn write_guest_meta(
    generation_dir: &Path,
    marker: &GenerationMarker,
    closure_count: usize,
) -> Result<(), HardlinkFarmError> {
    let meta = GuestGenerationMeta {
        schema_version: GUEST_META_SCHEMA_VERSION,
        generation_id: marker.closure_hash.clone(),
        generation_token: marker.generation_number,
        sync_status: "ok".to_owned(),
        closure_count: u32::try_from(closure_count).unwrap_or(u32::MAX),
    };
    let path = generation_dir.join("meta.json");
    let tmp = generation_dir.join("meta.json.tmp");
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| HardlinkFarmError::Io {
        path: path.display().to_string(),
        detail: format!("serialize: {e}"),
    })?;
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        file.write_all(&bytes).map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        file.sync_all().map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| HardlinkFarmError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    if let Ok(dir) = std::fs::File::open(generation_dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn write_system_symlink(
    generation_dir: &Path,
    closure_paths: &[PathBuf],
) -> Result<(), HardlinkFarmError> {
    let Some(system) = closure_paths
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains("nixos-system-"))
                .unwrap_or(false)
        })
        .or_else(|| closure_paths.first())
    else {
        return Ok(());
    };
    let link = generation_dir.join("system");
    let tmp = generation_dir.join("system.tmp");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(system, &tmp).map_err(|e| HardlinkFarmError::Io {
        path: tmp.display().to_string(),
        detail: e.to_string(),
    })?;
    std::fs::rename(&tmp, &link).map_err(|e| HardlinkFarmError::Io {
        path: link.display().to_string(),
        detail: e.to_string(),
    })
}

/// Plant the per-VM live readiness marker.
///
/// ADR 0027: the marker is a **zero-length** file. It is the
/// cold-start readiness signal and lives under the guest-served
/// `live/` pool, so it must carry no host paths, generation metadata,
/// counts, caller principal, or any other payload — its existence
/// alone is the signal and the readiness probe is a `test -e`.
///
/// Written via tmp+rename+fsync so a crash mid-plant leaves either the
/// old marker or no marker, never a torn file. The (empty) inode is
/// fsynced before the rename publishes it, and the `live/` directory is
/// fsynced after rename so the dirent is durable on ext4/xfs/btrfs.
fn write_live_marker(live_dir: &Path, vm: &str) -> Result<(), HardlinkFarmError> {
    let marker = live_dir.join(format!(".nixling-marker-{vm}"));
    let tmp = live_dir.join(format!(".nixling-marker-{vm}.tmp"));
    {
        let file = std::fs::File::create(&tmp).map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        // Zero-length: write nothing. fsync the empty file so the
        // inode is durable before the rename makes it visible.
        file.sync_all().map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    std::fs::rename(&tmp, &marker).map_err(|e| HardlinkFarmError::Io {
        path: marker.display().to_string(),
        detail: e.to_string(),
    })?;
    if let Ok(dir) = std::fs::File::open(live_dir) {
        let _ = dir.sync_all();
    }
    Ok(())
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
        if let Err(e) = std::fs::hard_link(source, destination) {
            let src_dev = std::fs::metadata(source).map(|m| m.dev()).unwrap_or(0);
            let dst_dev = destination
                .parent()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.dev())
                .unwrap_or(0);
            match classify_link_failure(e.raw_os_error(), src_dev, dst_dev) {
                // EXDEV on the SAME `st_dev`: source + destination are on
                // one underlying filesystem but different vfsmounts (the
                // NixOS `/nix/store` self-bind-mount). RECOVERABLE — the
                // broker retries inside a mount namespace where
                // `/nix/store` is lazily detached.
                LinkFailure::CrossMount => {
                    return Err(HardlinkFarmError::CrossMountLink {
                        source: source.display().to_string(),
                        destination: destination.display().to_string(),
                        dev: src_dev,
                    });
                }
                // EXDEV on DIFFERENT `st_dev`: genuinely different
                // filesystems (should already have been caught by
                // `assert_same_filesystem`). FATAL — no namespace helps.
                LinkFailure::DifferentFilesystem => {
                    return Err(HardlinkFarmError::DifferentFilesystem {
                        a: source.display().to_string(),
                        a_dev: src_dev,
                        b: destination.display().to_string(),
                        b_dev: dst_dev,
                    });
                }
                // EMLINK: the SOURCE inode is at the filesystem hardlink
                // ceiling (ext4 `EXT4_LINK_MAX` = 65000). `nix-store
                // --optimise` dedups every empty/tiny file onto a single
                // inode, so a long-lived host saturates those inodes —
                // after which no NEW hardlink to them can be created, in
                // ANY mount namespace (the limit is per-inode). Fall back
                // to a byte copy: the store file is read-only so the farm
                // view is identical, only already-saturated
                // (overwhelmingly empty) inodes pay the copy, and the copy
                // does not share the source inode (strictly safer for the
                // "never mutate a shared store inode" invariant).
                LinkFailure::CopyFallback => {
                    std::fs::copy(source, destination).map_err(|ce| HardlinkFarmError::Io {
                        path: destination.display().to_string(),
                        detail: format!("copy fallback after EMLINK: {ce}"),
                    })?;
                    return Ok(());
                }
                LinkFailure::Other => {
                    return Err(HardlinkFarmError::Io {
                        path: destination.display().to_string(),
                        detail: e.to_string(),
                    });
                }
            }
        }
        return Ok(());
    }
    Err(HardlinkFarmError::Io {
        path: source.display().to_string(),
        detail: "unsupported store path file type".to_owned(),
    })
}

/// Classification of a `link(2)` failure, used by [`hardlink_tree`] to
/// route the error. Kept as a pure function of `(errno, src_dev,
/// dst_dev)` so the EXDEV-vs-EMLINK branching can be unit-tested without
/// fabricating a cross-mount or a 65000-link inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFailure {
    /// EXDEV with matching `st_dev`: cross-vfsmount, retryable in a
    /// private mount namespace.
    CrossMount,
    /// EXDEV with differing `st_dev`: genuinely different filesystems,
    /// fatal.
    DifferentFilesystem,
    /// EMLINK: source inode at the hardlink ceiling, fall back to copy.
    CopyFallback,
    /// Any other errno: propagate as a generic I/O error.
    Other,
}

fn classify_link_failure(raw_os_error: Option<i32>, src_dev: u64, dst_dev: u64) -> LinkFailure {
    match raw_os_error {
        Some(EXDEV) if src_dev == dst_dev => LinkFailure::CrossMount,
        Some(EXDEV) => LinkFailure::DifferentFilesystem,
        Some(EMLINK) => LinkFailure::CopyFallback,
        _ => LinkFailure::Other,
    }
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

/// Read the current generation number from `<store_root>/current`.
pub fn current_generation(store_root: &Path) -> Result<Option<u64>, HardlinkFarmError> {
    let current = store_root.join("current");
    let target = match std::fs::read_link(&current) {
        Ok(target) => target,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(HardlinkFarmError::Io {
                path: current.display().to_string(),
                detail: err.to_string(),
            });
        }
    };
    let Some(name) = target.file_name().and_then(|n| n.to_str()) else {
        return Ok(None);
    };
    match name.parse::<u64>() {
        Ok(n) => Ok(Some(n)),
        Err(_) => Ok(None),
    }
}

/// Sweep `live/` to the union of top-level store path basenames required by
/// `retained_generations`.
pub fn sweep_live_pool(
    store_root: &Path,
    retained_generations: &[u64],
) -> Result<usize, HardlinkFarmError> {
    let live = live_dir(store_root);
    let mut desired = BTreeSet::new();
    for generation in retained_generations {
        let store_paths = generation_dir(store_root, *generation).join("store-paths");
        let content = match std::fs::read_to_string(&store_paths) {
            Ok(content) => content,
            Err(_) => return Ok(0),
        };
        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            let path = Path::new(line);
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                desired.insert(name.to_owned());
            }
        }
    }

    let mut removed = 0;
    let entries = match std::fs::read_dir(&live) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(HardlinkFarmError::Io {
                path: live.display().to_string(),
                detail: err.to_string(),
            });
        }
    };

    for entry in entries {
        let entry = entry.map_err(|e| HardlinkFarmError::Io {
            path: live.display().to_string(),
            detail: e.to_string(),
        })?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with(".nixling-marker-") || name_str.starts_with("live.stage.") {
            continue;
        }
        if desired.contains(name_str) {
            continue;
        }
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        if meta.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            })?;
        } else {
            std::fs::remove_file(&path).map_err(|e| HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            })?;
        }
        removed += 1;
    }
    Ok(removed)
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

    #[test]
    fn classify_link_failure_routes_each_errno() {
        // EXDEV with matching st_dev -> retryable cross-mount.
        assert_eq!(
            classify_link_failure(Some(EXDEV), 42, 42),
            LinkFailure::CrossMount
        );
        // EXDEV with differing st_dev -> fatal different-filesystem.
        assert_eq!(
            classify_link_failure(Some(EXDEV), 42, 99),
            LinkFailure::DifferentFilesystem
        );
        // EMLINK -> copy fallback regardless of devs.
        assert_eq!(
            classify_link_failure(Some(EMLINK), 42, 42),
            LinkFailure::CopyFallback
        );
        assert_eq!(
            classify_link_failure(Some(EMLINK), 42, 99),
            LinkFailure::CopyFallback
        );
        // Anything else -> generic Other (propagated as Io).
        assert_eq!(
            classify_link_failure(Some(libc_eacces()), 42, 42),
            LinkFailure::Other
        );
        assert_eq!(classify_link_failure(None, 42, 42), LinkFailure::Other);
    }

    // EACCES = 13; a representative "some other errno" without pulling in
    // libc just for the constant.
    fn libc_eacces() -> i32 {
        13
    }

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

        let farm_binary = live_dir(&farm_root).join("abc-system/bin/switch-to-configuration");
        assert!(farm_binary.exists());
        assert_eq!(
            std::fs::metadata(&farm_binary).unwrap().ino(),
            std::fs::metadata(system_path.join("bin/switch-to-configuration"))
                .unwrap()
                .ino()
        );
        assert_eq!(
            std::fs::read_link(live_dir(&farm_root).join("abc-system/data-link")).unwrap(),
            PathBuf::from("../dep/data")
        );
        let marker = read_generation_marker(&generation_dir).unwrap();
        assert_eq!(marker.generation_number, 7);
        assert_eq!(marker.vm, "corp-vm");
        assert!(generation_dir.join("store-paths").exists());
        assert!(generation_dir.join("system").exists());
    }

    #[test]
    fn live_marker_is_zero_length() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let pkg = source_root.join("abc-pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("payload"), b"data").unwrap();

        build_farm(&farm_root, 3, std::slice::from_ref(&pkg), &make_marker(3)).unwrap();

        // ADR 0027: the guest-served readiness marker carries no payload.
        let marker = live_dir(&farm_root).join(".nixling-marker-corp-vm");
        let meta = std::fs::metadata(&marker).expect("live marker planted");
        assert!(meta.is_file(), "marker is a regular file");
        assert_eq!(meta.len(), 0, "live readiness marker must be zero-length");
    }

    #[test]
    fn guest_meta_json_has_exact_allow_list() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let a = source_root.join("aaa-pkg");
        let b = source_root.join("bbb-pkg");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("payload"), b"a").unwrap();
        std::fs::write(b.join("payload"), b"b").unwrap();

        let marker = GenerationMarker {
            closure_hash: "sha256:deadbeef".to_owned(),
            nixling_version: "0.4.0".to_owned(),
            activated_at: "2026-06-09T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 9,
        };
        let generation_dir = build_farm(&farm_root, 9, &[a.clone(), b.clone()], &marker).unwrap();

        let raw = std::fs::read_to_string(generation_dir.join("meta.json"))
            .expect("guest meta.json written");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let obj = value.as_object().expect("meta.json is a JSON object");

        // ADR 0027: the key set must equal exactly the guest allow-list.
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "closure_count",
                "generation_id",
                "generation_token",
                "schema_version",
                "sync_status",
            ],
            "guest meta.json must expose exactly the allow-listed keys"
        );
        assert_eq!(
            obj["schema_version"],
            serde_json::json!(GUEST_META_SCHEMA_VERSION)
        );
        assert_eq!(obj["generation_id"], serde_json::json!("sha256:deadbeef"));
        assert_eq!(obj["generation_token"], serde_json::json!(9));
        assert_eq!(obj["sync_status"], serde_json::json!("ok"));
        assert_eq!(obj["closure_count"], serde_json::json!(2));

        // Round-trips through the typed independent serializer.
        let typed: GuestGenerationMeta = serde_json::from_str(&raw).unwrap();
        assert_eq!(typed.generation_id, "sha256:deadbeef");
        assert_eq!(typed.closure_count, 2);
    }

    #[test]
    fn build_farm_idempotent_for_same_closure() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("abc-system");
        std::fs::create_dir_all(&system_path).unwrap();
        std::fs::write(system_path.join("payload"), b"data").unwrap();
        let marker = GenerationMarker {
            closure_hash: "toplevel:abc-system".to_owned(),
            nixling_version: "0.4.0".to_owned(),
            activated_at: "2026-05-29T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 7,
        };
        // Building the same closure into the same generation twice is a
        // no-op-equivalent: the second call reuses the dir + marker.
        build_farm(&farm_root, 7, std::slice::from_ref(&system_path), &marker).unwrap();
        build_farm(&farm_root, 7, std::slice::from_ref(&system_path), &marker).unwrap();
    }

    #[test]
    fn build_farm_refuses_generation_collision() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        // Two DISTINCT closures (different toplevel identity) that
        // collided onto the same u32 generation number.
        let closure_a = source_root.join("aaa-system");
        let closure_b = source_root.join("bbb-system");
        std::fs::create_dir_all(&closure_a).unwrap();
        std::fs::create_dir_all(&closure_b).unwrap();
        std::fs::write(closure_a.join("payload"), b"a").unwrap();
        std::fs::write(closure_b.join("payload"), b"b").unwrap();

        build_farm(
            &farm_root,
            42,
            std::slice::from_ref(&closure_a),
            &GenerationMarker {
                closure_hash: "toplevel:aaa-system".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 42,
            },
        )
        .unwrap();

        // Same generation number, different closure identity → refuse
        // fail-closed rather than union the two closures.
        let result = build_farm(
            &farm_root,
            42,
            std::slice::from_ref(&closure_b),
            &GenerationMarker {
                closure_hash: "toplevel:bbb-system".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 42,
            },
        );
        assert!(matches!(
            result,
            Err(HardlinkFarmError::GenerationCollision { .. })
        ));
        // The original closure's store view is untouched.
        assert!(live_dir(&farm_root).join("aaa-system/payload").exists());
        assert!(!live_dir(&farm_root).join("bbb-system/payload").exists());
    }

    #[test]
    fn build_farm_rebuilds_markerless_partial_generation() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let closure = source_root.join("ccc-system");
        std::fs::create_dir_all(&closure).unwrap();
        std::fs::write(closure.join("payload"), b"c").unwrap();

        // Simulate a crashed earlier build: a populated generation dir
        // with leftover files but NO marker.json.
        let stale_dir = farm_root.join("generations").join("9");
        std::fs::create_dir_all(&stale_dir).unwrap();
        std::fs::write(stale_dir.join("leftover-from-crash"), b"stale").unwrap();

        // Building a (different) closure into the same generation must
        // NOT union the stale leftovers: it rebuilds from scratch.
        build_farm(
            &farm_root,
            9,
            std::slice::from_ref(&closure),
            &GenerationMarker {
                closure_hash: "toplevel:ccc-system".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 9,
            },
        )
        .unwrap();

        assert!(live_dir(&farm_root).join("ccc-system/payload").exists());
        assert!(!stale_dir.join("leftover-from-crash").exists());
        let marker = read_generation_marker(&stale_dir).unwrap();
        assert_eq!(marker.closure_hash, "toplevel:ccc-system");
    }

    #[test]
    fn build_farm_preserves_symlink_target_for_new_live_path() {
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

        let live_system_dir = live_dir(&farm_root).join("alpha-system");
        assert_eq!(
            std::fs::read_link(live_system_dir.join("data-link")).unwrap(),
            PathBuf::from("../dep/real")
        );
    }

    #[test]
    fn build_farm_preserves_broken_symlink_target_for_new_live_path() {
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

        let live_system_dir = live_dir(&farm_root).join("beta-system");
        assert_eq!(
            std::fs::read_link(live_system_dir.join("data-link")).unwrap(),
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
    fn sweep_live_pool_keeps_retained_generations_and_removes_stale_entries() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let gen1_path = source_root.join("aaa-system");
        let gen2_path = source_root.join("bbb-system");
        let stale_path = live_dir(&farm_root).join("stale-system");
        std::fs::create_dir_all(&gen1_path).unwrap();
        std::fs::create_dir_all(&gen2_path).unwrap();
        std::fs::write(gen1_path.join("payload"), b"a").unwrap();
        std::fs::write(gen2_path.join("payload"), b"b").unwrap();

        build_farm(
            &farm_root,
            1,
            std::slice::from_ref(&gen1_path),
            &GenerationMarker {
                closure_hash: "toplevel:aaa-system".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 1,
            },
        )
        .unwrap();
        build_farm(
            &farm_root,
            2,
            std::slice::from_ref(&gen2_path),
            &GenerationMarker {
                closure_hash: "toplevel:bbb-system".to_owned(),
                nixling_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 2,
            },
        )
        .unwrap();
        std::fs::create_dir_all(&stale_path).unwrap();
        std::fs::write(stale_path.join("payload"), b"stale").unwrap();

        let removed = sweep_live_pool(&farm_root, &[2]).unwrap();
        assert_eq!(removed, 2);
        assert!(!live_dir(&farm_root).join("aaa-system").exists());
        assert!(live_dir(&farm_root).join("bbb-system/payload").exists());
        assert!(!stale_path.exists());
        assert!(live_dir(&farm_root)
            .join(".nixling-marker-corp-vm")
            .exists());
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
