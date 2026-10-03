//! Out-of-process, mount-namespace-isolated store-view hardlink farm
//! build.
//!
//! ## Why this exists
//!
//! On a stock NixOS host `/nix/store` is bind-mounted read-only on top
//! of itself (`/nix/store` is a distinct vfsmount from `/`). Linux
//! refuses `link(2)` across vfsmounts (`EXDEV`) even when both paths
//! resolve to the same underlying filesystem (same `st_dev`). The
//! per-VM store-view farm hardlinks every closure path from
//! `/nix/store/<x>` into `/var/lib/d2b/vms/<vm>/store-view/...`, so
//! a direct in-process `link(2)` from the long-lived privileged broker
//! fails with `EXDEV` on exactly those hosts.
//!
//! The legacy systemd-activation builder (`packages/d2b-provider-volume-local/nix/store.nix`)
//! already solved this: re-exec under a private mount namespace and
//! lazily `umount /nix/store`, after which `/nix/store` is just a
//! directory on the root mount and the hardlinks succeed. This module
//! gives the daemon-native broker the same behaviour WITHOUT doing
//! `unshare(CLONE_NEWNS)` in the broker process itself (which would
//! corrupt the mount view of a long-lived, multi-request daemon) and
//! WITHOUT fork-then-run-Rust (async-signal-unsafe in a multithreaded
//! process) and without a shell. Instead it execs a dedicated helper
//! subprocess that performs namespace setup itself:
//!
//! ```text
//! /run/current-system/sw/bin/d2b-activation-helper private-store build-store-view
//! ```
//!
//! The helper unshares its mount namespace, sets recursive private
//! propagation, lazily detaches `/nix/store`, then reads the JSON request on
//! stdin and calls the selected `d2b_host::hardlink_farm` primitive.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use d2b_contracts_resource::v3::BoundedToken;
use d2b_host::hardlink_farm::{
    self, BuildStoreViewFarmRequest, BuildStoreViewRequest, GenerationMarker, HardlinkFarmError,
    StoreViewLinkCounts,
};

/// The fd-safe activation helper, installed into the system profile by
/// `nixos-modules/host-daemon.nix`.
const HELPER_BIN: &str = "/run/current-system/sw/bin/d2b-activation-helper";

// ---------------------------------------------------------------------------
// The admitted export (U15)
// ---------------------------------------------------------------------------
//
// A store-view farm is materialised FOR one admitted relationship: the
// named view of one Volume that one consumer holds under one access mode,
// at one generation. The farm itself is provider-neutral machinery; this
// is the part that binds it to that relationship and states, in one place,
// what the build is allowed to touch.
//
// The hardlink farm SHARES inodes with the content store, so a mutation
// "below" the farm is a mutation OF a store inode. The rule the whole
// module already obeys - never `chown -R`, `chmod -R`, or `setfacl -R`
// under the farm - is what keeps a read-only export read-only for every
// consumer at once. [`StoreViewExportBinding`] makes that rule a value:
// [`StoreViewExportBinding::admits`] is the closed set of mutations the
// build performs, and it contains no recursion, no ownership change, and
// no permission change at all, so there is nothing a caller can ask for
// that would reach a shared inode's metadata.

/// The closed set of filesystem mutations a store-view build is measured
/// against.
///
/// The four `Admitted` entries are the whole of what a build performs, and
/// every one is additive and confined to the farm tree. The rest are the
/// operations the farm's shared-inode rule forbids, listed rather than
/// merely omitted: the farm's live pool is made of hardlinks to the content
/// store, so a recursive posture walk, an ownership change, a permission
/// change, or a write to a source path is a write to a store inode that
/// every consumer shares. Naming them makes [`StoreViewExportBinding::admits`]
/// a fence with a testable negative side instead of an allow-list whose
/// absence proves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FarmMutation {
    /// `link(2)` one declared closure path into the farm's live pool.
    LinkClosurePath,
    /// Create a directory inside the farm tree.
    CreateFarmDirectory,
    /// Write one generation's own metadata document.
    WriteGenerationMetadata,
    /// Publish a generation by moving the farm's own `current` symlinks.
    PublishGenerationPointer,
    /// Walk the farm recursively to set an owner (`chown -R`).
    RecursiveOwnershipWalk,
    /// Walk the farm recursively to set a mode or an ACL
    /// (`chmod -R` / `setfacl -R`).
    RecursivePostureWalk,
    /// Set the owner of one farm path, following links into the store.
    OwnershipChange,
    /// Set the mode or ACL of one farm path, following links into the
    /// store.
    PermissionChange,
    /// Write the bytes of a declared closure path itself.
    SourceInodeWrite,
}

impl FarmMutation {
    /// Every mutation a build performs, in the order it performs them.
    pub const BUILD_SET: [Self; 4] = [
        Self::CreateFarmDirectory,
        Self::LinkClosurePath,
        Self::WriteGenerationMetadata,
        Self::PublishGenerationPointer,
    ];

    /// Every mutation an admitted export refuses.
    ///
    /// These are the operations that reach a shared content-store inode
    /// through the farm's hardlinks. They are refused by
    /// [`StoreViewExportBinding::admits`] before any build runs, so a
    /// read-only export cannot be talked into one.
    pub const REFUSED_SET: [Self; 5] = [
        Self::RecursiveOwnershipWalk,
        Self::RecursivePostureWalk,
        Self::OwnershipChange,
        Self::PermissionChange,
        Self::SourceInodeWrite,
    ];

    /// Whether this mutation reaches a shared content-store inode.
    pub fn mutates_shared_inodes(self) -> bool {
        Self::REFUSED_SET.contains(&self)
    }
}

/// Why an admitted export refused a request.
///
/// The codes name the relationship and the condition and carry no host
/// path, no generation id, and no view name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportRefusal {
    /// The relationship asks for a writable view of a shared closure
    /// store. A hardlink farm cannot separate a writer from the readers
    /// sharing its inodes, so the request is refused rather than served
    /// with a weaker guarantee than it claims.
    WritableExportUnsupported,
    /// The request names a generation other than the one the relationship
    /// was admitted at.
    GenerationNotAdmitted,
    /// The mutation is one that reaches a shared content-store inode, or
    /// is otherwise not part of what an admitted build performs.
    MutationNotAdmitted,
    /// The mutation target is outside the farm tree the export owns, and
    /// so is a mutation of a shared content-store inode.
    TargetOutsideFarm,
}

impl ExportRefusal {
    /// The stable `^[a-z][a-z0-9-]*$` code for this refusal.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WritableExportUnsupported => "writable-export-unsupported",
            Self::GenerationNotAdmitted => "generation-not-admitted",
            Self::MutationNotAdmitted => "mutation-not-admitted",
            Self::TargetOutsideFarm => "target-outside-farm",
        }
    }
}

impl fmt::Display for ExportRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ExportRefusal {}

/// Why a read-only export could not be materialised.
#[derive(Debug)]
pub enum StoreViewExportError {
    /// The relationship itself does not admit this export.
    Refused(ExportRefusal),
    /// The farm primitive refused the build.
    Farm(HardlinkFarmError),
}

impl fmt::Display for StoreViewExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "store-view export refused: {refusal}"),
            Self::Farm(error) => write!(f, "store-view export: {error}"),
        }
    }
}

impl std::error::Error for StoreViewExportError {}

/// One admitted read-only store-view export.
///
/// The value is the whole authority a materialisation runs under: which
/// relationship asked, which view it asked for, which generation it is
/// admitted at, and that the export is read-only. It is derived from the
/// admitted relationship, so a build cannot be pointed at another
/// relationship's view or at a generation the consumer was never admitted
/// for, and a restart re-derives an identical one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreViewExportBinding {
    consumer: BoundedToken,
    view: BoundedToken,
    generation: u64,
    read_only: bool,
}

impl StoreViewExportBinding {
    /// Admit one export, or refuse the relationship that asks for it.
    ///
    /// A writable export of a shared closure store is refused: the farm
    /// shares inodes with the store, so a write through one consumer's
    /// view is a write through every other consumer's view and the
    /// "read-only" claim of a sibling export would be false.
    pub fn admit(
        consumer: BoundedToken,
        view: BoundedToken,
        generation: u64,
        read_only: bool,
    ) -> Result<Self, ExportRefusal> {
        if !read_only {
            return Err(ExportRefusal::WritableExportUnsupported);
        }
        Ok(Self {
            consumer,
            view,
            generation,
            read_only,
        })
    }

    /// The consumer this export was admitted for.
    pub const fn consumer(&self) -> &BoundedToken {
        &self.consumer
    }

    /// The named view this export was admitted for.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// The one generation this export may materialise.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the export is read-only. It always is: the admission
    /// refuses every other shape.
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether this export performs `mutation`.
    ///
    /// The answer is `false` for every operation that would reach a
    /// shared content-store inode through the farm's hardlinks, whatever
    /// the caller asks for and however it is phrased.
    pub fn admits(&self, mutation: FarmMutation) -> Result<(), ExportRefusal> {
        if FarmMutation::BUILD_SET.contains(&mutation) {
            Ok(())
        } else {
            Err(ExportRefusal::MutationNotAdmitted)
        }
    }

    /// Whether this export reaches a shared content-store inode with
    /// `mutation`.
    pub fn reaches_shared_inodes(&self, mutation: FarmMutation) -> bool {
        self.admits(mutation).is_err()
    }

    /// Whether `target` is a path this export's build created and may
    /// mutate.
    ///
    /// The farm root itself is not a valid target: it is the directory the
    /// broker owns, and mutating it would be a mutation of the tree the
    /// export is confined to rather than within it. A declared closure
    /// path is never a valid target, which is the whole point: the source
    /// inode stays untouched because no mutation ever names it.
    pub fn owns_mutation_target(&self, farm_root: &Path, target: &Path) -> bool {
        target != farm_root && target.starts_with(farm_root)
    }

    /// Admit one build request for this export.
    fn admit_build(
        &self,
        farm_root: &Path,
        generation: u64,
    ) -> Result<(), ExportRefusal> {
        if generation != self.generation {
            return Err(ExportRefusal::GenerationNotAdmitted);
        }
        for mutation in FarmMutation::BUILD_SET {
            self.admits(mutation)?;
        }
        let live_pool = farm_root.join("live");
        if !self.owns_mutation_target(farm_root, &live_pool) {
            return Err(ExportRefusal::TargetOutsideFarm);
        }
        Ok(())
    }
}

/// Materialise one admitted read-only export, transparently handling the
/// NixOS `/nix/store` self-bind-mount.
///
/// The admission runs before the build and refuses a generation the
/// relationship was not admitted for, so a build can never publish a
/// generation the consumer was not granted. The build itself is the same
/// primitive [`build_store_view_cross_mount_safe_async`] drives, and it
/// performs only [`FarmMutation::BUILD_SET`]: `link(2)` into the farm's
/// own tree, the farm's own directories and metadata, and the farm's own
/// publish symlinks. No ownership or permission change is issued, and
/// nothing walks out of the farm, so the shared content-store inodes the
/// links point at are never written.
pub async fn build_read_only_store_view(
    export: &StoreViewExportBinding,
    farm_root: &Path,
    generation: u64,
    generation_id: &str,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<StoreViewLinkCounts, StoreViewExportError> {
    export
        .admit_build(farm_root, generation)
        .map_err(StoreViewExportError::Refused)?;
    build_store_view_cross_mount_safe_async(farm_root, generation_id, closure_paths, marker)
        .await
        .map_err(StoreViewExportError::Farm)
}

/// Build (or idempotently reconcile) one generation of the per-VM
/// store-view hardlink farm, transparently handling the NixOS
/// `/nix/store` self-bind-mount.
///
/// Strategy:
/// 1. Try the build in-process. On hosts where `/nix/store` and
///    `/var/lib/d2b` are the same mount (and in unit tests against
///    a `tempdir`) this succeeds directly with no subprocess.
/// 2. If - and only if - the in-process attempt fails with
///    [`HardlinkFarmError::CrossMountLink`] (the `link(2)` EXDEV on the
///    *same* `st_dev` that a `/nix/store` self-bind-mount produces),
///    retry the build in a private mount namespace where `/nix/store`
///    is lazily detached. The retry rebuilds the markerless partial
///    directory the failed attempt left behind.
///
/// A genuine distinct-`st_dev` [`HardlinkFarmError::DifferentFilesystem`]
/// is FATAL (the farm root and `/nix/store` are truly different
/// filesystems) and is NOT retried - unmounting `/nix/store` there would
/// expose the covered mount directory and could hardlink the wrong
/// inodes. All other errors (collision / marker / genuine I/O) propagate
/// unchanged. Returns the generation directory on success.
///
/// Async counterpart used by the async exec_reconcile/store_sync callers.
pub async fn build_farm_cross_mount_safe_async(
    farm_root: &Path,
    generation: u64,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<PathBuf, HardlinkFarmError> {
    match hardlink_farm::build_farm(farm_root, generation, closure_paths, marker).await {
        Ok(dir) => Ok(dir),
        Err(HardlinkFarmError::CrossMountLink { .. }) => {
            build_farm_via_namespace(farm_root, generation, closure_paths, marker).await
        }
        Err(other) => Err(other),
    }
}

fn farm_build_argv(helper_bin: &str) -> Vec<String> {
    private_store_argv(helper_bin, "build-store-view-farm")
}

/// Spawn the store helper, write the JSON request to stdin, drain its
/// output, and return the raw output for the caller's success handling.
async fn run_store_helper(
    argv: &[String],
    payload: Vec<u8>,
    farm_root: &Path,
    verb_label: &str,
) -> Result<std::process::Output, HardlinkFarmError> {
    let mut child = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env_remove("NOTIFY_SOCKET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| HardlinkFarmError::Io {
            path: argv[0].clone(),
            detail: format!("spawn unshare for {verb_label}: {e}"),
        })?;

    // Write the request from a task and close stdin so the helper sees
    // EOF; `wait_with_output` drains stdout/stderr concurrently, so no
    // pipe-buffer deadlock can occur even when the request exceeds the
    // stdin pipe capacity.

    let mut stdin = child.stdin.take().ok_or_else(|| HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: format!("child stdin unavailable for {verb_label}"),
    })?;
    let writer = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(&payload).await;
        // stdin dropped here -> EOF for the helper.

    });

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: farm_root.display().to_string(),
            detail: format!("await {verb_label}: {e}"),
        })?;
    let _ = writer.await;
    Ok(output)
}

/// Convert a failed store-helper output into the typed farm error, or
/// a generic Io error for spawn/protocol faults carrying stderr.
fn store_helper_failure(
    output: std::process::Output,
    farm_root: &Path,
    verb_label: &str,
) -> HardlinkFarmError {
    if let Some(line) = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        && let Ok(typed) = serde_json::from_str::<HardlinkFarmError>(line)
    {
        return typed;
    }
    HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: format!(
            "{verb_label} helper failed (exit {}): {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_owned()),
            String::from_utf8_lossy(&output.stderr).trim(),
        ),
    }
}

/// Run the hardlink-farm build inside a private mount namespace where
/// `/nix/store` is lazily detached.
///
/// Errors are surfaced as the typed [`HardlinkFarmError`] - recovered
/// from the helper's stdout JSON when the failure was a farm-build
/// error (collision / different-filesystem / marker), or wrapped as
/// [`HardlinkFarmError::Io`] for spawn / protocol failures - so callers
/// keep their existing `map_hardlink_farm_error` / `?` mapping.
async fn build_farm_via_namespace(
    farm_root: &Path,
    generation: u64,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<PathBuf, HardlinkFarmError> {
    let request = BuildStoreViewFarmRequest {
        farm_root: farm_root.to_path_buf(),
        generation,
        closure_paths: closure_paths.to_vec(),
        marker: marker.clone(),
    };
    let payload = serde_json::to_vec(&request).map_err(|e| HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: format!("serialise store-view farm request: {e}"),
    })?;

    let argv = farm_build_argv(HELPER_BIN);
    let output = run_store_helper(&argv, payload, farm_root, "store-view farm build").await?;

    let generation_dir = farm_root.join("generations").join(generation.to_string());

    if output.status.success() {
        return Ok(generation_dir);
    }

    Err(store_helper_failure(output, farm_root, "store-view farm build"))
}

/// Materialise one generation of the ADR 0027 **split** store view
/// ([`hardlink_farm::build_store_view`]), transparently handling the
/// NixOS `/nix/store` self-bind-mount exactly like
/// [`build_farm_cross_mount_safe_async`].
///
/// Returns the top-level link/skip accounting. This materialises `live/`,
/// `meta/generations/<id>/`, `state/generations/<id>/`, and the
/// `gcroots/generation-<id>` root; it does NOT swap `state/current` /
/// `meta/current` or plant the live marker - the broker performs those
/// in-process publish steps after a successful materialisation.
///
/// Async counterpart used by the async exec_reconcile/store_sync callers.
///
/// # Errors
///
/// Returns farm errors with the same recovery semantics as
/// [`build_farm_cross_mount_safe_async`]: a
/// [`HardlinkFarmError::CrossMountLink`] from the in-process attempt
/// triggers the namespace-isolated retry, a fatal
/// [`HardlinkFarmError::DifferentFilesystem`] propagates unchanged, and
/// all other errors (collision / marker / I/O) are surfaced as the typed
/// [`HardlinkFarmError`].
pub async fn build_store_view_cross_mount_safe_async(
    farm_root: &Path,
    generation_id: &str,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<StoreViewLinkCounts, HardlinkFarmError> {
    match hardlink_farm::build_store_view(farm_root, generation_id, closure_paths, marker).await {
        Ok(counts) => Ok(counts),
        Err(HardlinkFarmError::CrossMountLink { .. }) => {
            build_store_view_via_namespace(farm_root, generation_id, closure_paths, marker).await
        }
        Err(other) => Err(other),
    }
}

/// Build the `unshare … sh -ceu … helper build-store-view` argv. Split
/// out so the wiring can be asserted without spawning.
fn private_store_argv(helper_bin: &str, verb: &str) -> Vec<String> {
    vec![
        helper_bin.to_owned(),
        "private-store".to_owned(),
        verb.to_owned(),
    ]
}

fn store_view_build_argv(helper_bin: &str) -> Vec<String> {
    private_store_argv(helper_bin, "build-store-view")
}
/// Run the split-layout store-view build inside a private mount namespace
/// where `/nix/store` is lazily detached. On success the helper prints
/// the [`StoreViewLinkCounts`] as one JSON line on stdout; on failure it
/// prints the typed [`HardlinkFarmError`] (recovered here so the
/// collision / different-fs / marker mapping is preserved).
async fn build_store_view_via_namespace(
    farm_root: &Path,
    generation_id: &str,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<StoreViewLinkCounts, HardlinkFarmError> {
    let request = BuildStoreViewRequest {
        farm_root: farm_root.to_path_buf(),
        generation_id: generation_id.to_owned(),
        closure_paths: closure_paths.to_vec(),
        marker: marker.clone(),
    };
    let payload = serde_json::to_vec(&request).map_err(|e| HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: format!("serialise store-view request: {e}"),
    })?;

    let argv = store_view_build_argv(HELPER_BIN);
    let output = run_store_helper(&argv, payload, farm_root, "store-view build").await?;

    if output.status.success() {
        return parse_store_view_counts(&output.stdout, farm_root);
    }

    Err(store_helper_failure(output, farm_root, "store-view build"))
}
fn parse_store_view_counts(
    stdout: &[u8],
    farm_root: &Path,
) -> Result<StoreViewLinkCounts, HardlinkFarmError> {
    if let Some(line) = String::from_utf8_lossy(stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
    {
        return serde_json::from_str::<StoreViewLinkCounts>(line).map_err(|err| {
            HardlinkFarmError::Io {
                path: farm_root.display().to_string(),
                detail: format!("parse store-view build counts: {err}"),
            }
        });
    }
    Err(HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: "store-view build helper exited successfully without link counts".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farm_build_argv_wires_private_namespace_and_helper_verb() {
        let argv = private_store_argv("/h/helper", "build-store-view-farm");
        assert_eq!(
            argv,
            vec!["/h/helper", "private-store", "build-store-view-farm"]
        );
    }

    #[test]
    fn private_store_argv_has_no_shell_or_unshare_binary() {
        let argv = private_store_argv("/h/helper", "build-store-view");
        assert_eq!(argv[0], "/h/helper");
        assert!(!argv.iter().any(|arg| arg == "/bin/sh" || arg == "unshare"));
        assert!(!argv.iter().any(|arg| arg.contains("umount")));
    }

    #[test]
    fn store_view_build_argv_wires_private_namespace_and_split_verb() {
        let argv = store_view_build_argv("/h/helper");
        assert_eq!(argv, vec!["/h/helper", "private-store", "build-store-view"]);
    }
    #[test]
    fn successful_helper_without_counts_fails_closed() {
        let err = parse_store_view_counts(b"", Path::new("/tmp/store-view"))
            .expect_err("missing helper counts must not become a success-shaped zero count");
        assert!(
            err.to_string().contains("without link counts"),
            "unexpected error: {err}"
        );
    }
}
