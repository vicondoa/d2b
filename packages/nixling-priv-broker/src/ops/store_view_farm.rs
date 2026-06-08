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
//! `/nix/store/<x>` into `/var/lib/nixling/vms/<vm>/store-view/...`, so
//! a direct in-process `link(2)` from the long-lived privileged broker
//! fails with `EXDEV` on exactly those hosts.
//!
//! The legacy systemd-activation builder (`nixos-modules/store.nix`)
//! already solved this: re-exec under a private mount namespace and
//! lazily `umount /nix/store`, after which `/nix/store` is just a
//! directory on the root mount and the hardlinks succeed. This module
//! gives the daemon-native broker the same behaviour WITHOUT doing
//! `unshare(CLONE_NEWNS)` in the broker process itself (which would
//! corrupt the mount view of a long-lived, multi-request daemon) and
//! WITHOUT fork-then-run-Rust (async-signal-unsafe in a multithreaded
//! process). Instead it execs a dedicated subprocess:
//!
//! ```text
//! unshare --mount --propagation private \
//!   /bin/sh -ceu 'umount -l /nix/store 2>/dev/null || true; \
//!                  exec "$0" build-store-view-farm' \
//!   /run/current-system/sw/bin/nixling-activation-helper
//! ```
//!
//! `--propagation private` ensures the lazy unmount stays inside the
//! child namespace and never detaches the host's real `/nix/store`.
//! The `nixling-activation-helper build-store-view-farm` verb reads the
//! [`BuildStoreViewFarmRequest`] as JSON on stdin and calls
//! `nixling_host::hardlink_farm::build_farm`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nixling_host::hardlink_farm::{
    self, BuildStoreViewFarmRequest, GenerationMarker, HardlinkFarmError,
};

/// `unshare(1)` — already assumed present by the activation path in
/// `exec_reconcile::run_activation_script`.
const UNSHARE_BIN: &str = "/run/current-system/sw/bin/unshare";
/// POSIX shell — same path the activation bind-mount step uses.
const SH_BIN: &str = "/bin/sh";
/// The fd-safe activation helper, installed into the system profile by
/// `nixos-modules/host-daemon.nix`.
const HELPER_BIN: &str = "/run/current-system/sw/bin/nixling-activation-helper";

/// The shell run inside the new mount namespace: drop `/nix/store`
/// (ignored on hosts where it is not a separate mount) then exec the
/// helper verb. `$0` is the helper path passed as the first positional.
const NS_SHELL_SCRIPT: &str =
    "umount -l /nix/store 2>/dev/null || true; exec \"$0\" build-store-view-farm";

/// Build (or idempotently reconcile) one generation of the per-VM
/// store-view hardlink farm, transparently handling the NixOS
/// `/nix/store` self-bind-mount.
///
/// Strategy:
/// 1. Try the build in-process. On hosts where `/nix/store` and
///    `/var/lib/nixling` are the same mount (and in unit tests against
///    a `tempdir`) this succeeds directly with no subprocess.
/// 2. If — and only if — the in-process attempt fails with
///    [`HardlinkFarmError::DifferentFilesystem`] (the `link(2)` EXDEV
///    that a `/nix/store` self-bind-mount produces, surfaced by
///    `hardlink_farm`'s EXDEV mapping), retry the build in a private
///    mount namespace where `/nix/store` is lazily detached. The retry
///    rebuilds the markerless partial directory the failed attempt left
///    behind.
///
/// All other errors (collision / marker / genuine I/O) propagate
/// unchanged. Returns the generation directory on success.
pub fn build_farm_cross_mount_safe(
    farm_root: &Path,
    generation: u64,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<PathBuf, HardlinkFarmError> {
    match hardlink_farm::build_farm(farm_root, generation, closure_paths, marker) {
        Ok(dir) => Ok(dir),
        Err(HardlinkFarmError::DifferentFilesystem { .. }) => {
            build_farm_via_namespace(farm_root, generation, closure_paths, marker)
        }
        Err(other) => Err(other),
    }
}

/// Build the `unshare … sh -ceu … helper` argv (binary + args). Split
/// out so the wiring can be asserted in a unit test without spawning.
fn farm_build_argv(unshare_bin: &str, sh_bin: &str, helper_bin: &str) -> Vec<String> {
    vec![
        unshare_bin.to_owned(),
        "--mount".to_owned(),
        "--propagation".to_owned(),
        "private".to_owned(),
        sh_bin.to_owned(),
        "-ceu".to_owned(),
        NS_SHELL_SCRIPT.to_owned(),
        helper_bin.to_owned(),
    ]
}

/// Run the hardlink-farm build inside a private mount namespace where
/// `/nix/store` is lazily detached.
///
/// Errors are surfaced as the typed [`HardlinkFarmError`] — recovered
/// from the helper's stdout JSON when the failure was a farm-build
/// error (collision / different-filesystem / marker), or wrapped as
/// [`HardlinkFarmError::Io`] for spawn / protocol failures — so callers
/// keep their existing `map_hardlink_farm_error` / `?` mapping.
fn build_farm_via_namespace(
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

    let argv = farm_build_argv(UNSHARE_BIN, SH_BIN, HELPER_BIN);
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| HardlinkFarmError::Io {
            path: argv[0].clone(),
            detail: format!("spawn unshare for store-view farm build: {e}"),
        })?;

    // Write the request on a dedicated thread and close stdin so the
    // helper sees EOF, while the main thread drains stdout/stderr —
    // avoids a pipe-buffer deadlock if the request exceeds the stdin
    // pipe capacity.
    let mut stdin = child.stdin.take().ok_or_else(|| HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: "child stdin unavailable for store-view farm build".to_owned(),
    })?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&payload);
        // stdin dropped here -> EOF for the helper.
    });

    let output = child.wait_with_output().map_err(|e| HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: format!("await store-view farm build: {e}"),
    })?;
    let _ = writer.join();

    let generation_dir = farm_root
        .join("generations")
        .join(generation.to_string());

    if output.status.success() {
        return Ok(generation_dir);
    }

    // The helper emits the typed HardlinkFarmError as a single JSON
    // line on stdout when build_farm itself failed; recover it so the
    // collision / different-fs / marker mapping is preserved. Fall back
    // to a generic Io error carrying stderr for spawn/protocol faults.
    if let Some(line) = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
    {
        if let Ok(typed) = serde_json::from_str::<HardlinkFarmError>(line) {
            return Err(typed);
        }
    }
    Err(HardlinkFarmError::Io {
        path: farm_root.display().to_string(),
        detail: format!(
            "store-view farm build helper failed (exit {}): {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_owned()),
            String::from_utf8_lossy(&output.stderr).trim(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farm_build_argv_wires_private_namespace_and_helper_verb() {
        let argv = farm_build_argv("/u/unshare", "/bin/sh", "/h/helper");
        assert_eq!(
            argv,
            vec![
                "/u/unshare",
                "--mount",
                "--propagation",
                "private",
                "/bin/sh",
                "-ceu",
                "umount -l /nix/store 2>/dev/null || true; exec \"$0\" build-store-view-farm",
                "/h/helper",
            ]
        );
    }

    #[test]
    fn ns_shell_script_lazy_unmounts_then_execs_helper() {
        // Lazy unmount (so a busy /nix/store still detaches), tolerant
        // of hosts where /nix/store is not a separate mount, then
        // exec (no lingering shell) into the helper verb.
        assert!(NS_SHELL_SCRIPT.contains("umount -l /nix/store"));
        assert!(NS_SHELL_SCRIPT.contains("|| true"));
        assert!(NS_SHELL_SCRIPT.contains("exec \"$0\" build-store-view-farm"));
    }
}
