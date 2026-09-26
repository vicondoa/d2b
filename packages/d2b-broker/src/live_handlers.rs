//! Live broker request handlers.
//!
//! These functions execute the side-effectful work for the broker's
//! production dispatch path - `pidfd_open(2)` + start-time
//! re-verification, `nft -f -` shellouts via `ReconcileExecutor`,
//! and `clone3(CLONE_PIDFD)` spawns via `sys::pidfd_sys`. They are
//! pure-shaped (take their inputs directly rather than reading the
//! bundle) so the integration layer (`runtime::dispatch_request`)
//! is the only place that mixes wire decoding + bundle resolution
//! + live execution.
//!
//! Every handler returns either a typed success payload (with an
//! `OwnedFd` when SCM_RIGHTS is needed) or a `LiveHandlerError`
//! that the dispatch layer maps onto the broker's wire error
//! envelope.
//!
//! Unit-tested with `FakeReconcileExecutor` (for reconcile
//! handlers) and pure-data assertions (for the spawn preflight).
//! The `pidfd_open` and `clone3` paths require a live kernel and are
//! exercised by the broker integration tests (broker-pidfd-adopt-roundtrip.sh
//! and broker-spawn-runner-smoke.sh).

use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use crate::ops::exec_reconcile::{IpRouteVerb, ReconcileExecError, ReconcileExecutor};
use crate::ops::spawn_runner::{
    SpawnRunnerError, SpawnRunnerPlan, SpawnRunnerPlanInput, build_cstring_vectors, preflight,
};
use d2b_contracts_resource::v3::{ActivationRunnerInput, MAX_ACTIVATION_RUNNER_INPUT_BYTES};
use d2b_core::bundle_resolver::HostRuntime;
use d2b_core::sandbox_profile::CgroupPlacement;
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags};

/// Aggregate error type for live handlers. Kept narrow so the
/// dispatch layer can match on the precise failure shape.
#[derive(Debug)]
pub enum LiveHandlerError {
    /// `pidfd_open(2)` succeeded but the post-open `/proc/<pid>/stat`
    /// field-22 re-check disagreed with the daemon's
    /// `expected_start_time_ticks`. The pid was reused between the
    /// daemon's pre-call `/proc` read and the broker's `pidfd_open`.
    /// The pidfd is already closed by the time this is returned.
    PidfdRace {
        pid: i32,
        expected_start_time_ticks: u64,
        observed_start_time_ticks: Option<u64>,
    },
    /// `pidfd_open(2)` failed (ESRCH, EPERM, ENOSYS, etc).
    PidfdOpenFailed {
        pid: i32,
        detail: String,
    },
    /// `/proc/<pid>/stat` read failed AFTER pidfd_open succeeded.
    /// Pidfd is closed.
    ProcStatReadFailed {
        pid: i32,
        detail: String,
    },
    /// Spawn preflight rejected the bundle-resolved plan.
    SpawnPreflight(SpawnRunnerError),
    /// `clone3(2)` failed.
    SpawnFailed {
        detail: String,
    },
    /// Reconcile executor returned an error.
    ReconcileExec(ReconcileExecError),
    /// Per-busid lock failure (already-held, owner mismatch on release,
    /// or I/O error on lock root).
    UsbipLock(String),
    /// Host install / migrate writer failure.
    HostInstall(String),
    /// Activation / GC / key-management failures that are not raw
    /// executor errors.
    Activation(String),
    Gc(String),
    KeysRotate(String),
    HostKey(String),
    /// NetworkManager reload failure after writing the unmanaged config
    /// snippet.
    NmReload(String),
    /// The NetworkManager unmanaged intent declared a reload behavior the
    /// contract does not admit. Carries the rejected value so the refusal
    /// names exactly what a hand-declared bundle got wrong.
    NmReloadBehaviorRefused(String),
    /// The declared owner/group of the NetworkManager unmanaged file could
    /// not be resolved or enforced. Carries the failing principal or the
    /// enforcement detail.
    NmFileOwnership(String),
    /// A foreign or ambiguous NetworkManager ownership marker occupied the
    /// d2b-managed file.
    NmOwnershipConflict,
    /// swtpm-dir first-run hardening (issue #64) refused to proceed.
    /// Carries the path-free [`SwtpmDirAudit`] (with `result ==
    /// FailedClosed`) so the dispatch layer can emit the terminal
    /// `PrepareSwtpmDir` audit record on the fail-closed path.
    SwtpmDirHardening {
        audit: crate::ops::audit_op::SwtpmDirAudit,
        reason: &'static str,
    },
}

impl std::fmt::Display for LiveHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PidfdRace {
                pid,
                expected_start_time_ticks,
                observed_start_time_ticks,
            } => write!(
                f,
                "pidfd race: pid {pid} start-time drifted from expected {expected_start_time_ticks} to observed {observed_start_time_ticks:?}"
            ),
            Self::PidfdOpenFailed { pid, detail } => {
                write!(f, "pidfd_open({pid}) failed: {detail}")
            }
            Self::ProcStatReadFailed { pid, detail } => {
                write!(f, "/proc/{pid}/stat read after pidfd_open: {detail}")
            }
            Self::SpawnPreflight(e) => write!(f, "spawn preflight rejected: {e}"),
            Self::SpawnFailed { detail } => write!(f, "clone3/spawn failed: {detail}"),
            Self::ReconcileExec(e) => write!(f, "reconcile exec: {e}"),
            Self::UsbipLock(detail) => write!(f, "usbip lock: {detail}"),
            Self::HostInstall(detail) => write!(f, "host install: {detail}"),
            Self::Activation(detail) => write!(f, "activation: {detail}"),
            Self::Gc(detail) => write!(f, "gc: {detail}"),
            Self::KeysRotate(detail) => write!(f, "keys rotate: {detail}"),
            Self::HostKey(detail) => write!(f, "host key: {detail}"),
            Self::NmReload(detail) => write!(f, "networkmanager reload: {detail}"),
            Self::NmReloadBehaviorRefused(value) => write!(
                f,
                "NetworkManager reload behavior {value:?} is not supported; expected \"atomic-reload\" or \"none\""
            ),
            Self::NmFileOwnership(detail) => {
                write!(f, "NetworkManager unmanaged file ownership: {detail}")
            }
            Self::NmOwnershipConflict => f.write_str("nm-managed-foreign-conflict"),
            Self::SwtpmDirHardening { reason, .. } => {
                // PATH-FREE: only the closed-set reason slug.
                write!(f, "swtpm-dir hardening failed: {reason}")
            }
        }
    }
}

impl std::error::Error for LiveHandlerError {}

/// Result of [`live_open_pidfd`].
#[derive(Debug)]
pub struct OpenPidfdResult {
    /// The opened + verified pidfd. Caller must transport this
    /// over SCM_RIGHTS to the daemon.
    pub pidfd: OwnedFd,
    pub pid: i32,
    pub verified_start_time_ticks: u64,
}

/// Live broker `OpenPidfd` handler.
///
/// Performs the open-AND-verify atomically:
/// 1. `pidfd_open(pid)`.
/// 2. `/proc/<pid>/stat` field-22 read.
/// 3. Compare against `expected_start_time_ticks`.
/// 4. On match: return the pidfd.
/// 5. On mismatch: drop the pidfd (closing it) and return
///    [`LiveHandlerError::PidfdRace`].
///
/// This closes the critical pid-reuse race - the daemon's pre-call
/// /proc read is augmented by the broker's post-open re-check so the
/// returned pidfd is provably bound to the original process.
pub fn live_open_pidfd(
    pid: i32,
    expected_start_time_ticks: u64,
) -> Result<OpenPidfdResult, LiveHandlerError> {
    let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).map_err(|e| {
        LiveHandlerError::PidfdOpenFailed {
            pid,
            detail: e.to_string(),
        }
    })?;
    let observed = match crate::sys::pidfd_sys::read_proc_stat_start_time(pid) {
        Ok(v) => v,
        Err(e) => {
            // Pidfd is dropped here (closed by OwnedFd::drop).
            drop(pidfd);
            return Err(LiveHandlerError::ProcStatReadFailed {
                pid,
                detail: e.to_string(),
            });
        }
    };
    if observed != expected_start_time_ticks {
        drop(pidfd);
        return Err(LiveHandlerError::PidfdRace {
            pid,
            expected_start_time_ticks,
            observed_start_time_ticks: Some(observed),
        });
    }
    Ok(OpenPidfdResult {
        pidfd,
        pid,
        verified_start_time_ticks: observed,
    })
}

/// Live broker `ApplyNftables` handler. Wraps the
/// `exec_reconcile::ReconcileExecutor::apply_nft_script` call.
pub async fn live_apply_nftables(
    executor: &dyn ReconcileExecutor,
    nft_binary: &Path,
    nft_script: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .apply_nft_script(nft_binary, nft_script)
        .await
        .map_err(LiveHandlerError::ReconcileExec)
}

/// Live broker `ApplySysctl` handler.
pub async fn live_apply_sysctl(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .write_sysctl(key, value)
        .await
        .map_err(LiveHandlerError::ReconcileExec)
}

/// Live broker `UpdateHostsFile` handler. Atomic write with fsync via
/// the executor.
pub async fn live_update_hosts_file(
    executor: &dyn ReconcileExecutor,
    path: &Path,
    contents: &[u8],
    mode: u32,
) -> Result<(), LiveHandlerError> {
    executor
        .write_atomic_file(path, contents, mode)
        .await
        .map_err(LiveHandlerError::ReconcileExec)
}

/// Live broker `ApplyRoute` handler.
pub async fn live_apply_route(
    executor: &dyn ReconcileExecutor,
    ip_binary: &Path,
    verb: IpRouteVerb,
    route_spec: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .ip_route(ip_binary, verb, route_spec)
        .await
        .map_err(LiveHandlerError::ReconcileExec)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NmReloadMethod {
    Dbus,
    SystemctlFallback,
}

impl NmReloadMethod {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Dbus => "dbus",
            Self::SystemctlFallback => "systemctl-fallback",
        }
    }
}

/// The closed reload-behavior set the NetworkManager unmanaged contract
/// admits. `"atomic-reload"` selects the reload branch; `"none"` and the
/// empty no-host-contract sentinel select the write-only path. Any other
/// value is a hand-declared bundle defect: refusing it here (before any
/// mutation) keeps a typo from silently skipping the NetworkManager reload
/// while the apply acks success.
///
/// Shared with the remove path in `ops/nm.rs`; both arms branch on the
/// same value, so both must apply the same contract check.
pub(crate) fn validate_nm_reload_behavior(
    reload_behavior: &str,
) -> Result<(), LiveHandlerError> {
    if matches!(reload_behavior, "atomic-reload" | "none" | "") {
        return Ok(());
    }
    Err(LiveHandlerError::NmReloadBehaviorRefused(reload_behavior.to_owned()))
}

/// Resolve the declared owner/group names of the NetworkManager unmanaged
/// drop-in to uid/gid. The declaration is part of the bundle contract and
/// is enforced on the written file; an unresolvable principal refuses the
/// apply naming the exact name that failed.
fn resolve_nm_file_principal(owner: &str, group: &str) -> Result<(u32, u32), LiveHandlerError> {
    use nix::unistd::{Group, User};
    let uid = User::from_name(owner)
        .map_err(|error| {
            LiveHandlerError::NmFileOwnership(format!(
                "resolving declared owner {owner:?} failed: {error}"
            ))
        })?
        .ok_or_else(|| {
            LiveHandlerError::NmFileOwnership(format!(
                "declared owner {owner:?} does not exist on the host"
            ))
        })?;
    let gid = Group::from_name(group)
        .map_err(|error| {
            LiveHandlerError::NmFileOwnership(format!(
                "resolving declared group {group:?} failed: {error}"
            ))
        })?
        .ok_or_else(|| {
            LiveHandlerError::NmFileOwnership(format!(
                "declared group {group:?} does not exist on the host"
            ))
        })?;
    Ok((uid.uid.as_raw(), gid.gid.as_raw()))
}

/// Live broker `ApplyNmUnmanaged` handler.
pub async fn live_apply_nm_unmanaged(
    executor: &dyn ReconcileExecutor,
    intent: &d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent,
) -> Result<(), LiveHandlerError> {
    if let Some(method) = live_apply_nm_unmanaged_with_reloaders(
        executor,
        intent,
        async || networkmanager_reload_via_dbus().await,
        async |args| systemctl_invoke(args).await,
    )
    .await?
    {
        tracing::info!(
            reload_method = method.as_str(),
            file_path = %intent.file_path.display(),
            "reloaded NetworkManager after writing unmanaged config"
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn live_apply_nm_unmanaged_with_reload<F>(
    executor: &dyn ReconcileExecutor,
    intent: &d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent,
    mut reload: F,
) -> Result<(), LiveHandlerError>
where
    F: AsyncFnMut(&[&str]) -> Result<(), String>,
{
    validate_nm_reload_behavior(&intent.reload_behavior)?;
    let existing = match crate::sys::path_safe::read_to_string_nofollow(&intent.file_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(LiveHandlerError::NmOwnershipConflict),
    };
    crate::ops::nm::validate_existing_managed_conf(&existing, &intent.contents)
        .map_err(|_| LiveHandlerError::NmOwnershipConflict)?;
    let (owner_uid, owner_gid) = resolve_nm_file_principal(&intent.owner, &intent.group)?;
    executor
        .write_atomic_file_with_ownership(
            &intent.file_path,
            intent.contents.as_bytes(),
            intent.mode,
            owner_uid,
            owner_gid,
        )
        .await
        .map_err(LiveHandlerError::ReconcileExec)?;
    if intent.reload_behavior == "atomic-reload" {
        reload(&["reload", "NetworkManager"])
            .await
            .map_err(LiveHandlerError::NmReload)?;
        tracing::info!(
            reload_method = "custom",
            file_path = %intent.file_path.display(),
            "reloaded NetworkManager after writing unmanaged config"
        );
    }
    Ok(())
}

async fn live_apply_nm_unmanaged_with_reloaders<D, F>(
    executor: &dyn ReconcileExecutor,
    intent: &d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent,
    mut dbus_reload: D,
    mut fallback_reload: F,
) -> Result<Option<NmReloadMethod>, LiveHandlerError>
where
    D: AsyncFnMut() -> Result<(), String>,
    F: AsyncFnMut(&[&str]) -> Result<(), String>,
{
    validate_nm_reload_behavior(&intent.reload_behavior)?;
    let existing = match crate::sys::path_safe::read_to_string_nofollow(&intent.file_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(LiveHandlerError::NmOwnershipConflict),
    };
    crate::ops::nm::validate_existing_managed_conf(&existing, &intent.contents)
        .map_err(|_| LiveHandlerError::NmOwnershipConflict)?;
    let (owner_uid, owner_gid) = resolve_nm_file_principal(&intent.owner, &intent.group)?;
    executor
        .write_atomic_file_with_ownership(
            &intent.file_path,
            intent.contents.as_bytes(),
            intent.mode,
            owner_uid,
            owner_gid,
        )
        .await
        .map_err(LiveHandlerError::ReconcileExec)?;
    if intent.reload_behavior != "atomic-reload" {
        return Ok(None);
    }
    match dbus_reload().await {
        Ok(()) => Ok(Some(NmReloadMethod::Dbus)),
        Err(dbus_err) => {
            tracing::warn!(
                reload_method = "dbus",
                file_path = %intent.file_path.display(),
                error = %dbus_err,
                "NetworkManager DBus reload failed; falling back to systemctl"
            );
            fallback_reload(&["reload", "NetworkManager"])
                .await
                .map_err(|systemctl_err| {
                LiveHandlerError::NmReload(format!(
                    "dbus Reload(0) failed: {dbus_err}; systemctl fallback failed: {systemctl_err}"
                ))
            })?;
            Ok(Some(NmReloadMethod::SystemctlFallback))
        }
    }
}

/// Live broker `UsbipBind` handler.
///
/// 1. Acquire `/run/d2b/locks/usbip/<bus_id>` for `vm_name`
///    (refuses if another VM already owns the busid).
/// 2. If sysfs already reports `usbip-host`, treat same-VM replay as
///    converged without shelling out.
/// 3. Otherwise run `usbip bind --busid <bus_id>` via the executor and
///    verify sysfs converged to `usbip-host`. On bind failure or failed
///    convergence inspection, release the lock so a retried `UsbipBind`
///    from the same VM can succeed.
pub async fn live_usbip_bind(
    executor: &dyn ReconcileExecutor,
    usbip_binary: &Path,
    sysfs_root: &Path,
    bus_id: &str,
    lock_path: &Path,
    vm_name: &str,
    daemon_gid: u32,
) -> Result<(), LiveHandlerError> {
    crate::ops::usbip_lock::acquire_lock(lock_path, vm_name, daemon_gid)
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?;
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
        .await
    {
        Err(e) => {
            let _ = crate::ops::usbip_lock::release_lock(lock_path, vm_name);
            return Err(LiveHandlerError::UsbipLock(e.to_string()));
        }
        Ok(crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost) => {
            return Ok(());
        }
        Ok(crate::ops::usbip_host::UsbipDriverBinding::Unbound)
        | Ok(crate::ops::usbip_host::UsbipDriverBinding::BoundToOtherDriver { .. }) => {}
    }
    if let Err(e) = executor
        .run_usbip(
            usbip_binary,
            crate::ops::exec_reconcile::UsbipSubcommand::Bind,
            bus_id,
        )
        .await
    {
        let _ = crate::ops::usbip_lock::release_lock(lock_path, vm_name);
        return Err(LiveHandlerError::ReconcileExec(e));
    }
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
        .await
    {
        Err(e) => {
            let _ = crate::ops::usbip_lock::release_lock(lock_path, vm_name);
            Err(LiveHandlerError::UsbipLock(e.to_string()))
        }
        Ok(crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost) => Ok(()),
        Ok(observed) => {
            let _ = crate::ops::usbip_lock::release_lock(lock_path, vm_name);
            Err(LiveHandlerError::UsbipLock(format!(
                "usbip bind did not converge to usbip-host for bus_id={bus_id}: observed {observed:?}"
            )))
        }
    }
}

/// Live broker `UsbipUnbind` handler.
///
/// 1. Verify the lock's recorded owner matches `vm_name` BEFORE
///    touching usbip. The unbind shellout previously ran first, so a
///    stale-but-authenticated request from VM A could detach VM B's
///    device before the owner-mismatch was caught at release_lock time.
/// 2. Run `usbip unbind --busid <bus_id>` via the executor. Production
///    first aborts the usbip-host socket-backed stream via the per-device
///    `usbip_sockfd` control, waits for the kernel stream-fd liveness surface to
///    leave `USED`, and then executes driver unbind through a bounded helper
///    because kernel driver detach can stall in sysfs; timeout keeps the
///    broker/d2bd control path live.
/// 3. Leave `/run/d2b/locks/usbip/<bus_id>` in place. The dispatch layer
///    revokes the backend device ACL after successful unbind, then releases the
///    host-session claim last. Timeout/failure deliberately preserves the claim for
///    operator recovery.
pub async fn live_usbip_unbind(
    executor: &dyn ReconcileExecutor,
    usbip_binary: &Path,
    sysfs_root: &Path,
    bus_id: &str,
    lock_path: &Path,
    vm_name: &str,
) -> Result<(), LiveHandlerError> {
    match crate::ops::usbip_lock::peek_owner(lock_path) {
        Some(observed) if observed != vm_name => {
            return Err(LiveHandlerError::UsbipLock(format!(
                "usbip unbind refused: bus_id={bus_id} lock at {} owned by {observed} but caller is {vm_name}",
                lock_path.display()
            )));
        }
        Some(_) => {}
        None => return Ok(()),
    }
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
        .await
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?
    {
        crate::ops::usbip_host::UsbipDriverBinding::Unbound => return Ok(()),
        crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost => {
            crate::ops::usbip_host::ensure_usbip_host_driver_unbind_supported(sysfs_root)
                .await
                .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?;
        }
        crate::ops::usbip_host::UsbipDriverBinding::BoundToOtherDriver { driver } => {
            return Err(LiveHandlerError::UsbipLock(format!(
                "usbip unbind refused: bus_id={bus_id} is no longer bound to usbip-host (observed driver {driver}); the session claim is preserved for manual recovery"
            )));
        }
    }
    executor
        .shutdown_usbip_streams(sysfs_root, bus_id)
        .await
        .map_err(LiveHandlerError::ReconcileExec)?;
    executor
        .wait_usbip_stream_fd_release(sysfs_root, bus_id)
        .await
        .map_err(LiveHandlerError::ReconcileExec)?;
    executor
        .run_usbip(
            usbip_binary,
            crate::ops::exec_reconcile::UsbipSubcommand::Unbind,
            bus_id,
        )
        .await
        .map_err(LiveHandlerError::ReconcileExec)?;
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
        .await
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?
    {
        crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost => {
            Err(LiveHandlerError::UsbipLock(format!(
                "usbip unbind did not detach usbip-host for bus_id={bus_id}; the session claim is preserved for manual recovery"
            )))
        }
        crate::ops::usbip_host::UsbipDriverBinding::Unbound
        | crate::ops::usbip_host::UsbipDriverBinding::BoundToOtherDriver { .. } => Ok(()),
    }
}

async fn ensure_dir_tree(path: &Path, mode: u32) -> std::io::Result<()> {
    if path == Path::new("/") {
        return Ok(());
    }
    if !path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "path-safety-violation: directory path must be absolute: {}",
                path.display()
            ),
        ));
    }
    // Walk up to the first existing ancestor (validating it), collecting the
    // missing levels; then create them bottom-up with the same per-level
    // checks the recursive form ran. The flat loop is the async form of the
    // recursion (an `async fn` recursion would need boxing, E0733).
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if current == Path::new("/") {
            break;
        }
        match tokio::fs::symlink_metadata(current).await {
            Ok(metadata) => {
                validate_existing_dir(current, &metadata)?;
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                let parent = current.parent().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("directory path has no parent: {}", current.display()),
                    )
                })?;
                current = parent;
            }
            Err(err) => return Err(err),
        }
    }
    for directory in missing.into_iter().rev() {
        if production_path(&directory) {
            crate::sys::path_safe::refuse_non_root_parent(&directory)?;
        }
        crate::sys::path_safe::refuse_world_writable_parent(&directory)?;
        crate::sys::path_safe::ensure_dir(&directory, mode, None, None).map(|_| ())?;
    }
    Ok(())
}

fn validate_existing_dir(path: &Path, metadata: &std::fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "path-safety-violation: directory is a symlink: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "path-safety-violation: existing path is not a directory: {}",
                path.display()
            ),
        ));
    }
    if metadata.mode() & 0o002 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "path-safety-violation: directory is world-writable: {}",
                path.display()
            ),
        ));
    }
    if production_path(path) {
        if metadata.uid() != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "path-safety-violation: production directory must be root-owned: {}",
                    path.display()
                ),
            ));
        }
        crate::sys::path_safe::refuse_non_root_parent(path)?;
    }
    crate::sys::path_safe::refuse_world_writable_parent(path)?;
    Ok(())
}

fn production_path(path: &Path) -> bool {
    path.starts_with("/etc") || path.starts_with("/run") || path.starts_with("/var/lib/d2b")
}

async fn systemctl_invoke(args: &[&str]) -> Result<(), String> {
    let output = Command::new("/usr/bin/systemctl")
        .args(args)
        .env_remove("NOTIFY_SOCKET")
        .output()
        .await
        .map_err(|e| format!("systemctl spawn failed: {e}"))?;
    if !output.status.success() {
        let action = args.first().copied().unwrap_or("invoke");
        let target = args.get(1).copied().unwrap_or("");
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            if target.is_empty() {
                format!("exit code {}", output.status.code().unwrap_or(-1))
            } else {
                format!("{target}: exit code {}", output.status.code().unwrap_or(-1))
            }
        } else if target.is_empty() {
            stderr
        } else {
            format!("{target}: {stderr}")
        };
        return Err(format!("systemctl {action} failed: {detail}"));
    }
    Ok(())
}

async fn networkmanager_reload_via_dbus() -> Result<(), String> {
    const BUSCTL_CANDIDATES: [&str; 2] = ["/usr/bin/busctl", "/run/current-system/sw/bin/busctl"];
    let busctl = BUSCTL_CANDIDATES
        .iter()
        .find(|candidate| Path::new(candidate).is_file())
        .copied()
        .unwrap_or(BUSCTL_CANDIDATES[0]);
    let output = Command::new(busctl)
        .args([
            "call",
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "Reload",
            "u",
            "0",
        ])
        .env_remove("NOTIFY_SOCKET")
        .output()
        .await
        .map_err(|e| format!("busctl Reload(0) spawn failed via {busctl}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            format!("exit code {}", output.status.code().unwrap_or(-1))
        } else {
            stderr
        };
        return Err(format!("busctl Reload(0) failed via {busctl}: {detail}"));
    }
    Ok(())
}

async fn read_host_runtime(path: &Path) -> Result<Option<HostRuntime>, ReconcileExecError> {
    let contents = match if path.is_absolute() {
        crate::sys::path_safe::read_to_string_nofollow(path)
    } else {
        tokio::fs::read_to_string(path).await
    } {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: err.to_string(),
            });
        }
    };
    let runtime = serde_json::from_str::<HostRuntime>(&contents).map_err(|err| {
        ReconcileExecError::InvalidInput {
            detail: format!("invalid host-runtime {}: {err}", path.display()),
        }
    })?;
    Ok(Some(runtime))
}

pub(crate) async fn read_host_runtime_nft_hash(
    path: &Path,
) -> Result<Option<String>, ReconcileExecError> {
    Ok(read_host_runtime(path)
        .await?
        .and_then(|runtime| runtime.nft_applied_hash))
}

pub(crate) async fn update_host_runtime_nft_hash(
    path: &Path,
    nft_hash: Option<&str>,
) -> Result<(), ReconcileExecError> {
    if !path.is_absolute() {
        return Err(ReconcileExecError::InvalidInput {
            detail: format!("host-runtime path must be absolute: {}", path.display()),
        });
    }
    let mut runtime = read_host_runtime(path)
        .await?
        .ok_or_else(|| ReconcileExecError::Io {
        path: path.display().to_string(),
        detail: "host-runtime.json missing".to_owned(),
    })?;
    runtime.nft_applied_hash = nft_hash.map(ToOwned::to_owned);
    let mut body =
        serde_json::to_vec_pretty(&runtime).map_err(|err| ReconcileExecError::InvalidInput {
            detail: format!("failed to serialize host-runtime {}: {err}", path.display()),
        })?;
    body.push(b'\n');
    let parent = path
        .parent()
        .ok_or_else(|| ReconcileExecError::InvalidInput {
            detail: format!("host-runtime path has no parent: {}", path.display()),
        })?;
    ensure_dir_tree(parent, 0o755)
        .await
        .map_err(|err| ReconcileExecError::Io {
            path: parent.display().to_string(),
            detail: err.to_string(),
        })?;
    let dir_fd = crate::sys::path_safe::open_dir_path_safe(parent).map_err(|err| {
        ReconcileExecError::Io {
            path: parent.display().to_string(),
            detail: err.to_string(),
        }
    })?;
    let target_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ReconcileExecError::InvalidInput {
            detail: format!(
                "host-runtime path must end in a UTF-8 file name: {}",
                path.display()
            ),
        })?;
    crate::sys::path_safe::atomic_replace_fd(&dir_fd, target_name, &body, 0o644).map_err(
        |err| ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: err.to_string(),
        },
    )?;
    Ok(())
}

/// Live broker `UsbipProxyReconcile` handler.
///
/// Walks each (bus_id, vm_name, lock_path) tuple from the trusted
/// bundle and asserts the lockfile (if present) records the
/// expected owner. Returns the first mismatch encountered, or `Ok`
/// if every present lock matches expected ownership. Missing locks
/// are treated as "not bound" (no-op).
///
/// This handler deliberately does NOT auto-rebind: reconcile-after-restart
/// is the daemon's responsibility (the daemon classifies each existing
/// claim against the bundle and either issues `UsbipBind` or
/// `UsbipUnbind` to resolve drift). This handler is the validation half.
pub fn live_usbip_proxy_reconcile(
    expectations: &[(String, String, std::path::PathBuf)],
) -> Result<(), LiveHandlerError> {
    for (bus_id, vm_name, lock_path) in expectations {
        if let Some(observed) = crate::ops::usbip_lock::peek_owner(lock_path)
            && &observed != vm_name
        {
            return Err(LiveHandlerError::UsbipLock(format!(
                "usbip proxy reconcile: bus_id={bus_id} lock at {} owned by {observed} but bundle expects {vm_name}",
                lock_path.display()
            )));
        }
    }
    Ok(())
}

/// Result of [`live_spawn_runner`].
#[derive(Debug)]
pub struct SpawnRunnerResult {
    pub pidfd: OwnedFd,
    pub extra_response_fds: Vec<OwnedFd>,
    pub pid: i32,
    pub start_time_ticks: u64,
    /// True if the broker fell back to `fork(2)` + `pidfd_open(2)`
    /// instead of the preferred `clone3(CLONE_PIDFD)`. Used for
    /// audit-record bookkeeping.
    pub used_fork_fallback: bool,
    /// Path-free swtpm-dir hardening audit (issue #64), present only
    /// for the `w1-swtpm` role. The dispatch layer emits a terminal
    /// `PrepareSwtpmDir` `OpAuditRecord` from this on the success path.
    pub swtpm_dir_audit: Option<crate::ops::audit_op::SwtpmDirAudit>,
}

fn parse_runner_cgroup_subtree(subtree: &str) -> Result<Option<Vec<String>>, LiveHandlerError> {
    if subtree.is_empty() {
        return Ok(None);
    }
    let normalized = subtree
        .strip_prefix("d2b.slice/")
        .or_else(|| subtree.strip_prefix("d2b/"))
        .ok_or_else(|| LiveHandlerError::SpawnFailed {
            detail: format!("unsupported cgroup subtree root: {subtree}"),
        })?;
    let segments = normalized.split('/').collect::<Vec<_>>();
    if segments.len() < 2
        || segments.iter().any(|segment| {
            segment.is_empty() || segment.contains('\0') || matches!(*segment, "." | "..")
        })
    {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!("invalid cgroup subtree segments: {subtree}"),
        });
    }
    Ok(Some(
        segments.into_iter().map(str::to_owned).collect::<Vec<_>>(),
    ))
}

fn cgroup_leaf_path(parent_slice: &Path, segments: &[String]) -> PathBuf {
    let mut path = parent_slice.to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    path
}

fn ensure_runner_cgroup_leaf<B: d2b_host::cgroup::CgroupBackend>(
    backend: &B,
    placement: &CgroupPlacement,
    unified_hierarchy_root: &Path,
    parent_slice: &Path,
    uid: u32,
    gid: u32,
) -> Result<Option<PathBuf>, LiveHandlerError> {
    use d2b_host::cgroup::create_nested_subtree;

    let Some(segments) = parse_runner_cgroup_subtree(&placement.subtree)? else {
        return Ok(None);
    };
    let leaf_path = cgroup_leaf_path(parent_slice, &segments);
    // Always materialize the cgroup leaf dir tree even when
    // placement.delegated == false. The delegated flag is about
    // controller delegation (enabling subtree control), not whether
    // the directory exists. The broker spawn path always needs the
    // Zone/Guest role leaf to write the child pid into cgroup.procs.
    let slice = crate::ops::cgroup::create_d2b_slice(
        backend,
        unified_hierarchy_root,
        parent_slice,
        uid,
        gid,
    )
    .map_err(|err| LiveHandlerError::SpawnFailed {
        detail: format!("delegate cgroup slice: {err}"),
    })?;
    let segment_refs = segments.iter().map(String::as_str).collect::<Vec<_>>();
    create_nested_subtree(backend, &slice, &segment_refs, uid, gid).map_err(|err| {
        LiveHandlerError::SpawnFailed {
            detail: format!("delegate cgroup subtree for {}: {err}", leaf_path.display()),
        }
    })?;
    Ok(Some(leaf_path))
}

struct RunnerCgroupFds {
    dir_fd: OwnedFd,
    procs_fd: OwnedFd,
}

fn prepare_runner_cgroup_fds(
    placement: &CgroupPlacement,
) -> Result<Option<RunnerCgroupFds>, LiveHandlerError> {
    use d2b_host::cgroup::RealCgroupBackend;
    use rustix::fs::{Mode, OFlags, open};

    let backend = RealCgroupBackend::new();
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    let Some(leaf_path) = ensure_runner_cgroup_leaf(
        &backend,
        placement,
        Path::new("/sys/fs/cgroup"),
        Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE),
        uid,
        gid,
    )?
    else {
        return Ok(None);
    };
    let dir_fd = open(
        &leaf_path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|err| LiveHandlerError::SpawnFailed {
        detail: format!("open cgroup dir {}: {err}", leaf_path.display()),
    })?;
    let procs_fd = open(
        leaf_path.join("cgroup.procs"),
        OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|err| LiveHandlerError::SpawnFailed {
        detail: format!("open {}: {err}", leaf_path.join("cgroup.procs").display()),
    })?;
    Ok(Some(RunnerCgroupFds { dir_fd, procs_fd }))
}

/// Maps known internal seccomp policy ref names to the `DeviceClass`
/// sets that define their ioctl allowlist.
///
/// These correspond 1-to-1 to the `seccompPolicyRef` values emitted by
/// `nixos-modules/minijail-profiles.nix`. A non-absolute `policy_ref`
/// NOT present in this map is an unknown policy and returns an error.
pub(crate) fn policy_ref_device_classes(
    policy_ref: &str,
) -> Option<&'static [d2b_host::devices::DeviceClass]> {
    use d2b_host::devices::DeviceClass;
    match policy_ref {
        // cloud-hypervisor-runner binds /dev/kvm + /dev/vhost-net + /dev/net/tun.
        // Returns empty (permissive BPF) - KVM uses 100+ ioctls and
        // /dev/kvm access is gated by ACL on the device node + per-VM UID.
        // BPF enforcement of KVM ioctl matrix is tracked for a future
        // release (requires complete matrix from CH 52 source). The
        // stabilization choice is to install permissive BPF so Seccomp:2
        // remains visible to the doctor probe without breaking CH spawn.
        "w1-cloud-hypervisor-runner" => Some(&[]),
        // qemu-media is fd-backed: no media paths, no tun/vhost-net device
        // binds, and any KVM access is scoped by the role-device claim + ACL/fd
        // handoff. QEMU's KVM ioctl surface is too broad for the current small
        // matrix, so install permissive BPF while keeping Seccomp:2 visible.
        "w1-qemu-media" => Some(&[]),
        // virtiofsd accesses /dev/fuse via read/write; FUSE_NO_IOCTL
        // sentinel → permissive BPF (FUSE mount handshake needs ioctls).
        "w1-virtiofsd" => Some(&[DeviceClass::Fuse]),
        // host-reconcile, store-virtiofs-preflight, component-session-health:
        // no device binds → permissive BPF. host-reconcile and
        // store-virtiofs-preflight run the nix toolchain (many ioctls for
        // terminal/file operations); component-session-health is the daemon-side
        // authenticated Health probe, which speaks ttRPC over the component-session
        // vsock and uses connect(2)/socket ioctls.
        "w1-host-reconcile"
        | "w1-provider-controller"
        | "w1-store-virtiofs-preflight"
        | "w1-component-session-health"
        | "w1-activation-nixos-runner" => Some(&[]),
        // swtpm is a software TPM emulator; no hardware device ioctls,
        // but it uses terminal/file ioctls during init → permissive BPF.
        "w1-swtpm" => Some(&[]),
        // gpu sidecar binds the full GPU device set.
        // Returns empty (permissive BPF) - same reasoning as KVM: DRM
        // ioctl surface is huge; ACL on /dev/dri/* + per-VM UID is
        // the primary control.
        "w1-gpu" => Some(&[]),
        // Render-node-only broker-pre-NS GPU sidecar (ADR 0021).
        // Render node uses small DRM ioctl set; matrix is representative.
        // Keep restrictive BPF.
        "w1-gpu-render-node" => Some(&[DeviceClass::Dri]),
        // video decoder sidecar uses /dev/dri for DRM ioctls.
        // Same as gpu: permissive due to incomplete DRM ioctl matrix.
        "w1-video" => Some(&[]),
        // audio sidecar connects to PipeWire socket; PipewireSocket has
        // no ioctl entries → permissive BPF (libpipewire uses ioctls).
        "w1-audio" => Some(&[DeviceClass::PipewireSocket]),
        // vsock-relay and otel-host-bridge: pre-opened fds only, no device ioctls.
        "w1-vsock-relay" | "w1-otel-host-bridge" => Some(&[]),
        // usbipd backend attaches to /dev/usbip-host; small ioctl matrix complete.
        "w1-usbip" => Some(&[DeviceClass::UsbipHost]),
        // USBIP proxy only binds/listens/connects TCP sockets; no device ioctls.
        "w1-usbip-proxy" => Some(&[]),
        // wayland-proxy sidecar: no device binds (only a Wayland socket bind-mount),
        // no ioctl surface → permissive BPF. Seccomp is mandatory for this role
        // (set in minijail-profiles.nix) but the ioctl matrix is empty.
        "w1-wayland-proxy" => Some(&[]),
        _ => None,
    }
}

async fn load_runner_seccomp(
    plan: &SpawnRunnerPlan,
) -> Result<Option<crate::sys::pidfd_sys::SeccompProgram>, LiveHandlerError> {
    match plan.seccomp_policy_ref.as_deref() {
        Some(policy_path) if Path::new(policy_path).is_absolute() => {
            crate::sys::pidfd_sys::load_seccomp_program(Path::new(policy_path))
                .await
                .map(Some)
                .map_err(|err| LiveHandlerError::SpawnFailed {
                    detail: format!("load seccomp program {policy_path}: {err}"),
                })
        }
        // Compile BPF from the ioctl_policy matrix for known internal
        // policy refs. The Ok(None) silent-skip deferral from
        // v1.1.2-final is retired.
        Some(policy_ref) => {
            let classes = policy_ref_device_classes(policy_ref).ok_or_else(|| {
                LiveHandlerError::SpawnFailed {
                    detail: format!(
                        "InvalidSeccompPolicy: unknown internal policy ref {policy_ref:?}"
                    ),
                }
            })?;
            tracing::debug!(
                seccomp_policy_ref = %policy_ref,
                device_classes = ?classes,
                "compiling seccomp BPF from ioctl_policy matrix"
            );
            let compiled = d2b_host::seccomp::compile_ioctl_policy_to_bpf(classes);
            Ok(Some(crate::sys::pidfd_sys::SeccompProgram::from_compiled(
                compiled,
            )))
        }
        None => Ok(None),
    }
}

fn is_qemu_media_runner(plan: &SpawnRunnerPlan) -> bool {
    plan.seccomp_policy_ref.as_deref() == Some("w1-qemu-media")
        || plan
            .argv
            .first()
            .map(|arg0| arg0.starts_with("d2b-qemu-media@"))
            .unwrap_or(false)
}

fn validate_qemu_media_runner_hardening(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
    if !is_qemu_media_runner(plan) {
        return Ok(());
    }

    if plan.seccomp_policy_ref.as_deref() != Some("w1-qemu-media") {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "qemu-media runner must declare seccompPolicyRef \"w1-qemu-media\"".to_owned(),
        });
    }
    if !plan.capabilities.is_empty() {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!(
                "qemu-media runner must have empty capabilities; got {:?}",
                plan.capabilities
            ),
        });
    }
    const FORBIDDEN_CAPS: &[&str] = &[
        "CAP_SYS_ADMIN",
        "CAP_SYS_RAWIO",
        "CAP_DAC_OVERRIDE",
        "CAP_NET_ADMIN",
    ];
    if let Some(cap) = plan
        .capabilities
        .iter()
        .find(|cap| FORBIDDEN_CAPS.contains(&cap.as_str()))
    {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!("qemu-media runner forbids {cap}"),
        });
    }
    if !plan.namespaces.mount || !plan.namespaces.pid {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!(
                "qemu-media runner requires mount and pid namespaces; got {:?}",
                plan.namespaces
            ),
        });
    }
    if !plan.mount_policy.nix_store_read_only
        || !plan
            .mount_policy
            .read_only_paths
            .iter()
            .any(|path| path == "/")
    {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "qemu-media runner requires read-only root and read-only /nix/store".to_owned(),
        });
    }
    if !plan.mount_policy.hide_device_nodes_by_default {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "qemu-media runner must hide device nodes by default".to_owned(),
        });
    }
    if plan.mount_policy.device_binds != ["/dev/kvm"] {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!(
                "qemu-media runner permits exactly /dev/kvm by path; got {:?}",
                plan.mount_policy.device_binds
            ),
        });
    }
    if plan.mount_policy.bind_mounts.iter().any(|bm| {
        bm.src.starts_with("/var/lib/d2b/media") || bm.dst.starts_with("/var/lib/d2b/media")
    }) {
        return Err(LiveHandlerError::SpawnFailed {
            detail:
                "qemu-media runner must receive media through inherited/pre-opened fds, not media path bind mounts"
                    .to_owned(),
        });
    }

    Ok(())
}

const QEMU_MEDIA_MEMLOCK_MIN_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const QEMU_MEDIA_MEMLOCK_HEADROOM_RATIO_DIVISOR: u64 = 4;
const QEMU_MEDIA_MEMLOCK_PREFLIGHT_OVERHEAD_BYTES: u64 = 1024 * 1024 * 1024;

fn qemu_media_memlock_limit_bytes(plan: &SpawnRunnerPlan) -> Result<Option<u64>, LiveHandlerError> {
    Ok(qemu_media_memlock_guest_bytes(plan)?.map(|guest_bytes| {
        guest_bytes.saturating_add(qemu_media_memlock_headroom_bytes(guest_bytes))
    }))
}

fn qemu_media_memlock_guest_bytes(plan: &SpawnRunnerPlan) -> Result<Option<u64>, LiveHandlerError> {
    if !is_qemu_media_runner(plan) || !qemu_media_argv_has_mem_lock(&plan.argv) {
        return Ok(None);
    }
    qemu_media_memory_backend_size_bytes(&plan.argv)
        .map(Some)
        .ok_or_else(|| LiveHandlerError::SpawnFailed {
            detail: "qemu-media mem-lock requires a memory-backend-ram size".to_owned(),
        })
}

fn qemu_media_memlock_headroom_bytes(guest_bytes: u64) -> u64 {
    (guest_bytes / QEMU_MEDIA_MEMLOCK_HEADROOM_RATIO_DIVISOR)
        .max(QEMU_MEDIA_MEMLOCK_MIN_HEADROOM_BYTES)
}

fn qemu_media_memlock_preflight_required_bytes(guest_bytes: u64) -> u64 {
    guest_bytes.saturating_add(QEMU_MEDIA_MEMLOCK_PREFLIGHT_OVERHEAD_BYTES)
}

async fn qemu_media_preflight_memlock_budget(required_bytes: u64) -> Result<(), LiveHandlerError> {
    let meminfo =
        tokio::fs::read_to_string("/proc/meminfo")
            .await
            .map_err(|err| LiveHandlerError::SpawnFailed {
                detail: format!(
                    "qemu-media mem-lock preflight could not read host memory availability: {err}"
                ),
            })?;
    let available =
        parse_meminfo_available_bytes(&meminfo).ok_or_else(|| LiveHandlerError::SpawnFailed {
            detail: "qemu-media mem-lock preflight could not parse host MemAvailable".to_owned(),
        })?;
    if let Some(shortfall) = qemu_media_memlock_budget_shortfall(required_bytes, available) {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!(
                "qemu-media mem-lock preflight requires {} bytes but host MemAvailable is {} bytes; lower qemuMedia.resources.memoryMiB or disable qemuMedia.security.lockMemory",
                shortfall.required_bytes, shortfall.available_bytes
            ),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QemuMediaMemlockShortfall {
    required_bytes: u64,
    available_bytes: u64,
}

fn qemu_media_memlock_budget_shortfall(
    required_bytes: u64,
    available_bytes: u64,
) -> Option<QemuMediaMemlockShortfall> {
    (available_bytes < required_bytes).then_some(QemuMediaMemlockShortfall {
        required_bytes,
        available_bytes,
    })
}

fn parse_meminfo_available_bytes(meminfo: &str) -> Option<u64> {
    let value = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))?;
    let mut parts = value.split_whitespace();
    let amount = parts.next()?.parse::<u64>().ok()?;
    let unit = parts.next().unwrap_or("kB");
    if !unit.eq_ignore_ascii_case("kb") {
        return None;
    }
    amount.checked_mul(1024)
}

fn qemu_media_argv_has_mem_lock(argv: &[String]) -> bool {
    argv.windows(2)
        .any(|pair| pair[0] == "-overcommit" && pair[1] == "mem-lock=on")
}

fn qemu_media_memory_backend_size_bytes(argv: &[String]) -> Option<u64> {
    let object = argv
        .windows(2)
        .find_map(|pair| (pair[0] == "-object").then_some(pair[1].as_str()))?;
    let mut saw_backend = false;
    let mut size = None;
    for part in object.split(',') {
        if part == "memory-backend-ram" {
            saw_backend = true;
        } else if let Some(value) = part.strip_prefix("size=") {
            size = parse_qemu_size_bytes(value);
        }
    }
    saw_backend.then_some(size).flatten()
}

fn parse_qemu_size_bytes(value: &str) -> Option<u64> {
    let (number, multiplier) = match value.as_bytes().last().copied() {
        Some(b'K') | Some(b'k') => (&value[..value.len() - 1], 1024_u64),
        Some(b'M') | Some(b'm') => (&value[..value.len() - 1], 1024_u64 * 1024),
        Some(b'G') | Some(b'g') => (&value[..value.len() - 1], 1024_u64 * 1024 * 1024),
        Some(b'T') | Some(b't') => (&value[..value.len() - 1], 1024_u64 * 1024 * 1024 * 1024),
        Some(b'0'..=b'9') => (value, 1),
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(multiplier)
}

#[derive(Debug, Clone, Copy)]
enum AclPathKind {
    Directory,
    Socket,
    CharDevice,
}

fn setfacl_fd_safe(path: &Path, acl_spec: &str, kind: AclPathKind) -> Result<(), String> {
    setfacl_fd_safe_op(path, "-m", acl_spec, kind).map(|_| ())
}

/// The fd-safe setfacl stage that failed. A closed-set classification so
/// component-session callers can build path-free, acl-spec-free error
/// details (the raw path / acl spec never escapes into logs or audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetfaclStage {
    Open,
    Fstat,
    TypeMismatch,
    Apply,
}

/// A classified fd-safe setfacl failure.
///
/// `legacy_detail` carries the historical path-bearing message for the
/// observability/device/session callers that already embed paths in
/// their error strings. The structured `stage` / `errno_kind` /
/// `raw_os_error` fields let the component-session path build a path-free,
/// acl-spec-free detail that satisfies the hash-only observability
/// contract. The component-session formatter MUST NOT read `legacy_detail`.
#[derive(Debug, PartialEq, Eq)]
struct SetfaclFailure {
    stage: SetfaclStage,
    errno_kind: std::io::ErrorKind,
    raw_os_error: Option<i32>,
    legacy_detail: String,
}

impl SetfaclFailure {
    fn stage_label(&self) -> &'static str {
        match self.stage {
            SetfaclStage::Open => "open",
            SetfaclStage::Fstat => "fstat",
            SetfaclStage::TypeMismatch => "type-mismatch",
            SetfaclStage::Apply => "apply",
        }
    }

    /// Path-free, acl-spec-free failure detail for the component-session
    /// observability contract. Carries only the closed-set operation
    /// label, target class, daemon principal, failed stage, and the
    /// numeric errno / `io::ErrorKind`. Never the raw socket / state-dir
    /// path or the acl-spec string.
    fn component_session_detail(&self, op_label: &str, target_class: &str) -> String {
        let errno = self
            .raw_os_error
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "component-session vsock daemon ACL {op_label} on {target_class} failed: \
             principal={COMPONENT_SESSION_DAEMON_PRINCIPAL} stage={} kind={:?} errno={errno}",
            self.stage_label(),
            self.errno_kind,
        )
    }
}

/// Open `path` as an `O_PATH|NOFOLLOW|RESOLVE_NO_SYMLINKS` fd and fstat
/// it, returning the live `File` (kept open so callers can mutate the
/// exact inode) plus its metadata. `Ok(None)` if the path is absent.
fn open_o_path_metadata(path: &Path) -> Result<Option<(File, std::fs::Metadata)>, SetfaclFailure> {
    let fd = match rustix::fs::openat2(
        CWD,
        path,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    ) {
        Ok(fd) => fd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(SetfaclFailure {
                stage: SetfaclStage::Open,
                errno_kind: err.kind(),
                raw_os_error: Some(err.raw_os_error()),
                legacy_detail: format!(
                    "openat2(O_PATH|NOFOLLOW, RESOLVE_NO_SYMLINKS) {}: {err}",
                    path.display()
                ),
            });
        }
    };
    let file = File::from(fd);
    match file.metadata() {
        Ok(metadata) => Ok(Some((file, metadata))),
        Err(err) => Err(SetfaclFailure {
            stage: SetfaclStage::Fstat,
            errno_kind: err.kind(),
            raw_os_error: err.raw_os_error(),
            legacy_detail: format!("fstat {}: {err}", path.display()),
        }),
    }
}

/// Classified core of [`setfacl_fd_safe_op`]. Returns the resolved
/// `(dev, ino)` of the target the ACL was applied to (`None` if the path
/// was absent), or a [`SetfaclFailure`] carrying both the legacy
/// path-bearing message and a closed-set classification.
fn setfacl_fd_safe_op_classed(
    path: &Path,
    op: &str,
    acl_spec: &str,
    kind: AclPathKind,
) -> Result<Option<(u64, u64)>, SetfaclFailure> {
    let Some((file, metadata)) = open_o_path_metadata(path)? else {
        return Ok(None);
    };
    let file_type = metadata.file_type();
    let matches_kind = match kind {
        AclPathKind::Directory => file_type.is_dir(),
        AclPathKind::Socket => file_type.is_socket(),
        AclPathKind::CharDevice => file_type.is_char_device(),
    };
    if !matches_kind {
        return Err(SetfaclFailure {
            stage: SetfaclStage::TypeMismatch,
            errno_kind: std::io::ErrorKind::InvalidInput,
            raw_os_error: None,
            legacy_detail: format!(
                "refusing setfacl on {}: expected {:?}, mode=0o{:o}",
                path.display(),
                kind,
                metadata.mode()
            ),
        });
    }

    if let Err(err) = crate::sys::pidfd_sys::run_setfacl_op_on_fd(file.as_fd(), op, acl_spec) {
        return Err(SetfaclFailure {
            stage: SetfaclStage::Apply,
            errno_kind: err.kind(),
            raw_os_error: err.raw_os_error(),
            legacy_detail: format!("setfacl {op} {acl_spec} on {}: {err}", path.display()),
        });
    }
    Ok(Some((metadata.dev(), metadata.ino())))
}

/// Like [`setfacl_fd_safe`] but parameterised on the setfacl operation
/// flag (`-m` to add/modify, `-x` to remove) and returns the resolved
/// `(dev, ino)` of the target the ACL was applied to (`None` if the
/// path was absent). The returned identity is consumed by path-free
/// audit hashing so audit records never carry raw socket/state-dir
/// paths. The error string is the historical path-bearing form for
/// the observability/device/session callers; component-session callers use
/// [`setfacl_fd_safe_op_classed`] for a path-free detail instead.
fn setfacl_fd_safe_op(
    path: &Path,
    op: &str,
    acl_spec: &str,
    kind: AclPathKind,
) -> Result<Option<(u64, u64)>, String> {
    setfacl_fd_safe_op_classed(path, op, acl_spec, kind).map_err(|failure| failure.legacy_detail)
}

async fn setfacl_verified_device(
    path: &Path,
    operation: &str,
    acl_spec: &str,
    missing_ok: bool,
) -> Result<(), String> {
    // setfacl refuses /proc/<pid>/fd/<N> for character devices on
    // this host, so device nodes use exact-path setfacl after
    // openat2(RESOLVE_NO_SYMLINKS) verifies the allowlisted /dev node
    // is a char device. Callers only pass closed-set broker constants
    // (/dev/kvm, /dev/vhost-net, /dev/net/tun, /dev/dri/renderD128),
    // never bundle- or user-supplied arbitrary paths.
    let fd = match rustix::fs::openat2(
        CWD,
        path,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    ) {
        Ok(fd) => fd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && missing_ok => return Ok(()),
        Err(err) => {
            return Err(format!(
                "openat2(O_PATH|NOFOLLOW, RESOLVE_NO_SYMLINKS) {}: {err}",
                path.display()
            ));
        }
    };
    let file = File::from(fd);
    let before = verified_char_device_metadata(path, &file)?;
    let output = Command::new("/run/current-system/sw/bin/setfacl")
        .arg(operation)
        .arg(acl_spec)
        .arg(path)
        .env_remove("NOTIFY_SOCKET")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| format!("spawn setfacl for {}: {err}", path.display()))?;
    if output.status.success() {
        let after_fd = match rustix::fs::openat2(
            CWD,
            path,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS,
        ) {
            Ok(fd) => fd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && missing_ok => return Ok(()),
            Err(err) => {
                return Err(format!(
                    "post-setfacl openat2(O_PATH|NOFOLLOW, RESOLVE_NO_SYMLINKS) {}: {err}",
                    path.display()
                ));
            }
        };
        let after_file = File::from(after_fd);
        let after = verified_char_device_metadata(path, &after_file)?;
        if before != after {
            if operation == "-m"
                && let Some(revoke_spec) = acl_spec.rsplit_once(':').map(|(entry, _)| entry)
            {
                let _ = Command::new("/run/current-system/sw/bin/setfacl")
                    .arg("-x")
                    .arg(revoke_spec)
                    .arg(path)
                    .env_remove("NOTIFY_SOCKET")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
            }
            return Err(format!(
                "setfacl target changed while applying ACL on {}: before={before:?} after={after:?}",
                path.display()
            ));
        }
        Ok(())
    } else {
        Err(format!(
            "setfacl {} {} on {} failed status={:?}: {}",
            operation,
            acl_spec,
            path.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceIdentity {
    dev: u64,
    ino: u64,
    rdev: u64,
}

fn verified_char_device_metadata(path: &Path, file: &File) -> Result<DeviceIdentity, String> {
    let metadata = file
        .metadata()
        .map_err(|err| format!("fstat {}: {err}", path.display()))?;
    if !metadata.file_type().is_char_device() {
        return Err(format!(
            "refusing setfacl on {}: expected CharDevice, mode=0o{:o}",
            path.display(),
            metadata.mode()
        ));
    }
    Ok(DeviceIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        rdev: metadata.rdev(),
    })
}

fn env_value<'a>(plan: &'a SpawnRunnerPlan, key: &str) -> Option<&'a str> {
    plan.env
        .iter()
        .filter_map(|entry| entry.split_once('='))
        .find_map(|(entry_key, value)| (entry_key == key).then_some(value))
}

fn ch_vsock_connect_socket_arg(plan: &SpawnRunnerPlan) -> Option<PathBuf> {
    plan.argv.iter().find_map(|arg| {
        if !arg.contains("d2b-ch-vsock-connect") {
            return None;
        }
        let exec = arg.strip_prefix("EXEC:").unwrap_or(arg).trim_matches('"');
        let fields: Vec<&str> = exec.split_whitespace().collect();
        let helper_index = fields.iter().position(|field| {
            field.trim_matches('"').ends_with("/d2b-ch-vsock-connect")
                || field.trim_matches('"') == "d2b-ch-vsock-connect"
        })?;
        let socket = fields.get(helper_index + 1)?.trim_matches('"');
        socket.starts_with('/').then(|| PathBuf::from(socket))
    })
}

fn grant_obs_vsock_acl_once(uid: u32, socket: &Path) -> Result<bool, String> {
    if !socket.exists() {
        return Ok(false);
    }
    let Some(parent) = socket.parent() else {
        return Err("socket path has no parent".to_owned());
    };
    setfacl_fd_safe(parent, &format!("u:{uid}:--x"), AclPathKind::Directory)?;
    setfacl_fd_safe(socket, &format!("u:{uid}:rw"), AclPathKind::Socket)?;
    Ok(socket.exists())
}

/// How long one obs-vsock ACL refresh keeps retrying before it gives up.
const OBS_VSOCK_ACL_RETRY_WINDOW: Duration = Duration::from_secs(30);
/// How long one obs-vsock ACL refresh waits between attempts.
const OBS_VSOCK_ACL_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// The sockets whose ACL refresh is still pending.
///
/// One entry per socket rather than per spawn: every runner sharing an obs
/// VM's socket shares its pending refresh, so what is in flight is the number
/// of distinct sockets, not the number of calls that asked.
///
/// `tokio::sync` per plan U8: the claim/removal run on synchronous handler
/// bodies and the retry task, so they use the non-blocking `try_lock` (the
/// critical sections are single set operations, never held across an await).
static PENDING_OBS_VSOCK_ACL_RETRIES: LazyLock<tokio::sync::Mutex<HashSet<(u32, PathBuf)>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashSet::new()));

/// The pending-set guard: the socket stays claimed until its refresh ends.
struct PendingObsVsockAclRetry {
    uid: u32,
    socket: PathBuf,
}

impl Drop for PendingObsVsockAclRetry {
    fn drop(&mut self) {
        match PENDING_OBS_VSOCK_ACL_RETRIES.try_lock() {
            Ok(mut pending) => {
                pending.remove(&(self.uid, self.socket.clone()));
            }
            // The set is momentarily held by another claim/removal
            // (sub-microsecond); clear the entry asynchronously so a
            // pending refresh can never leak past the task that owned it.
            Err(_) => {
                if let Some(background) = crate::runtime::broker_background() {
                    let uid = self.uid;
                    let socket = self.socket.clone();
                    background.runtime.spawn(async move {
                        PENDING_OBS_VSOCK_ACL_RETRIES
                            .lock()
                            .await
                            .remove(&(uid, socket));
                    });
                }
            }
        }
    }
}

/// Claim one socket's pending refresh; `false` means one is already waiting.
fn claim_obs_vsock_acl_retry(uid: u32, socket: &Path) -> bool {
    match PENDING_OBS_VSOCK_ACL_RETRIES.try_lock() {
        Ok(mut pending) => pending.insert((uid, socket.to_path_buf())),
        // A Busy collision (another claim or removal in flight,
        // sub-microsecond) must not skip a needed refresh: a duplicate
        // refresh is idempotent, a skipped one leaves the socket without
        // its ACL. Proceed exactly like the old poisoned path.
        Err(_) => true,
    }
}

/// Wait for a socket the runner will create, then grant it the runner's ACL.
///
/// The wait is an async deadline on the broker's reactor, not a thread: the
/// socket appears when the obs VM's relay binds it, which is sooner or later
/// than any sleep, so the retry sleeps in timer time and gives up on its own
/// budget. Each attempt is a `setfacl` shellout and a filesystem walk, which
/// have no async form; they run on the broker's bounded dispatch pool like
/// every other kernel-path step, so a socket that never appears holds no
/// worker and no thread.
fn spawn_obs_vsock_acl_retry(uid: u32, socket: PathBuf) {
    let Some(background) = crate::runtime::broker_background() else {
        // Only a serving broker has a reactor to wait on; a handler driven
        // outside one has nothing to retry against either.
        tracing::warn!(
            path = %socket.display(),
            "obs-vsock socket ACL refresh dropped: no broker reactor is running",
        );
        return;
    };
    if !claim_obs_vsock_acl_retry(uid, &socket) {
        return;
    }
    let pending = PendingObsVsockAclRetry {
        uid,
        socket: socket.clone(),
    };
    background.runtime.spawn(async move {
        let _pending = pending;
        let deadline = tokio::time::Instant::now() + OBS_VSOCK_ACL_RETRY_WINDOW;
        loop {
            let attempt = {
                let socket = socket.clone();
                background
                    .dispatches
                    .run(move || grant_obs_vsock_acl_once(uid, &socket))
                    .await
            };
            match attempt {
                Ok(Ok(true)) => return,
                Ok(Ok(false)) => {}
                Ok(Err(err)) => {
                    tracing::debug!(
                        path = %socket.display(),
                        error = %err,
                        "obs-vsock socket ACL refresh not ready yet",
                    );
                }
                // The pool is gone, so the broker is shutting down.
                Err(_) => return,
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    path = %socket.display(),
                    "obs-vsock socket ACL refresh timed out",
                );
                return;
            }
            tokio::time::sleep(OBS_VSOCK_ACL_RETRY_INTERVAL).await;
        }
    });
}

async fn refresh_obs_vsock_acl(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
    if !matches!(
        plan.seccomp_policy_ref.as_deref(),
        Some("w1-vsock-relay" | "w1-otel-host-bridge")
    ) {
        return Ok(());
    }
    let Some(socket) = ch_vsock_connect_socket_arg(plan) else {
        return Ok(());
    };
    let uid = plan.uid;
    // The initial attempt is a setfacl shellout, which has no async form;
    // it runs on the broker's bounded dispatch pool like every other
    // kernel-path step - the same pool the retry path uses - so a socket
    // that is not there yet holds no worker and no thread. A handler
    // driven outside a serving broker (tests) has no pool to defer to and
    // runs the attempt inline, exactly as before.
    let attempt = match crate::runtime::broker_background() {
        Some(background) => {
            let socket = socket.clone();
            background
                .dispatches
                .run(move || grant_obs_vsock_acl_once(uid, &socket))
                .await
        }
        None => Ok(grant_obs_vsock_acl_once(uid, &socket)),
    };
    match attempt {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => {
            spawn_obs_vsock_acl_retry(uid, socket);
            Ok(())
        }
        Ok(Err(detail)) => Err(LiveHandlerError::SpawnFailed {
            detail: format!("refresh obs-vsock ACL for runner uid {uid}: {detail}"),
        }),
        // The pool is gone, so the broker is shutting down.
        Err(_) => Ok(()),
    }
}

pub(crate) async fn live_grant_verified_device_acl(
    path: &Path,
    uid: u32,
) -> Result<(), LiveHandlerError> {
    live_set_verified_device_acl(path, uid, "-m", &format!("u:{uid}:rw"), "grant", false).await
}

pub(crate) async fn live_revoke_verified_device_acl(
    path: &Path,
    uid: u32,
) -> Result<(), LiveHandlerError> {
    live_set_verified_device_acl(path, uid, "-x", &format!("u:{uid}"), "revoke", true).await
}

async fn live_set_verified_device_acl(
    path: &Path,
    uid: u32,
    operation: &str,
    acl_spec: &str,
    verb: &str,
    missing_ok: bool,
) -> Result<(), LiveHandlerError> {
    setfacl_verified_device(path, operation, acl_spec, missing_ok)
        .await
        .map_err(|detail| {
            LiveHandlerError::Activation(format!(
                "{verb} USBIP device ACL for runner uid {uid}: {detail}"
            ))
        })
}

async fn refresh_spawn_runner_acls(
    plan: &SpawnRunnerPlan,
    broker_state_dir: &Path,
) -> Result<(), LiveHandlerError> {
    if plan.uid == 0 {
        return Ok(());
    }
    for device in &plan.mount_policy.device_binds {
        match device.as_str() {
            "/dev/kvm" | "/dev/vhost-net" | "/dev/net/tun" | "/dev/dri/renderD128" => {
                setfacl_verified_device(
                    Path::new(device),
                    "-m",
                    &format!("u:{}:rw", plan.uid),
                    true,
                )
                .await
                .map_err(|detail| LiveHandlerError::SpawnFailed {
                    detail: format!("refresh device ACL for runner uid {}: {detail}", plan.uid),
                })?;
            }
            _ => {}
        }
    }

    if matches!(
        plan.seccomp_policy_ref.as_deref(),
        Some("w1-audio" | "w1-gpu" | "w1-gpu-render-node")
    ) {
        let runtime_dir =
            env_value(plan, "PIPEWIRE_RUNTIME_DIR").or_else(|| env_value(plan, "XDG_RUNTIME_DIR"));
        if let Some(runtime_dir) = runtime_dir {
            let runtime = Path::new(runtime_dir);
            setfacl_fd_safe(
                runtime,
                &format!("u:{}:rx", plan.uid),
                AclPathKind::Directory,
            )
            .map_err(|detail| LiveHandlerError::SpawnFailed {
                detail: format!(
                    "refresh session runtime ACL for runner uid {}: {detail}",
                    plan.uid
                ),
            })?;
            for socket in ["pipewire-0", "wayland-0", "pulse/native"] {
                setfacl_fd_safe(
                    &runtime.join(socket),
                    &format!("u:{}:rwx", plan.uid),
                    AclPathKind::Socket,
                )
                .map_err(|detail| LiveHandlerError::SpawnFailed {
                    detail: format!(
                        "refresh session socket ACL {socket} for runner uid {}: {detail}",
                        plan.uid
                    ),
                })?;
            }
        }
    }
    if plan.seccomp_policy_ref.as_deref() == Some("w1-video") {
        let runtime_dir =
            env_value(plan, "PIPEWIRE_RUNTIME_DIR").or_else(|| env_value(plan, "XDG_RUNTIME_DIR"));
        if let Some(runtime_dir) = runtime_dir {
            let runtime = Path::new(runtime_dir);
            setfacl_fd_safe(
                runtime,
                &format!("u:{}:---", plan.uid),
                AclPathKind::Directory,
            )
            .map_err(|detail| LiveHandlerError::SpawnFailed {
                detail: format!(
                    "revoke session runtime ACL for video runner uid {}: {detail}",
                    plan.uid
                ),
            })?;
            for socket in ["pipewire-0", "wayland-0", "pulse/native"] {
                let path = runtime.join(socket);
                setfacl_fd_safe(&path, &format!("u:{}:---", plan.uid), AclPathKind::Socket)
                    .map_err(|detail| LiveHandlerError::SpawnFailed {
                        detail: format!(
                            "revoke session socket ACL for video runner uid {}: {detail}",
                            plan.uid
                        ),
                    })?;
            }
        }
    }

    if plan.seccomp_policy_ref.as_deref() == Some("w1-wayland-proxy") {
        // Wayland-proxy requires two ACL entries on the host compositor:
        //   1. Traverse (--x) on the runtime dir so the wlproxy uid can
        //      reach the socket at all. After reboot `/run/user/<uid>` is
        //      0700 until the session ACLs are applied; without this
        //      traverse grant the proxy fails immediately with EACCES.
        //   2. rwx on the configured Wayland socket.
        // PipeWire and Pulse sockets are explicitly revoked: the proxy
        // has no audio role and must not connect to them.
        //
        // Symlink-safe contract: open_o_path_metadata uses
        // openat2(O_PATH|NOFOLLOW, RESOLVE_NO_SYMLINKS) so a symlink planted
        // at the runtime-dir or socket path cannot redirect the verification
        // step. The ACL is then applied directly through the verified fd via
        // /proc/self/fd/<fd> (run_setfacl_op_on_fd), eliminating the TOCTOU
        // window that would exist if the path were re-opened after validation.
        let runtime_dir = env_value(plan, "XDG_RUNTIME_DIR");
        match runtime_dir {
            None => {
                return Err(LiveHandlerError::SpawnFailed {
                    detail: format!(
                        "graphical-session-not-active: XDG_RUNTIME_DIR not set for wayland-proxy uid {}; \
                         is the graphical session running?",
                        plan.uid
                    ),
                });
            }
            Some(runtime_dir) => {
                let runtime = Path::new(runtime_dir);
                // Derive the expected runtime-dir owner uid from the
                // declarative path `/run/user/<uid>` (from the bundle's
                // XDG_RUNTIME_DIR value, which originates in
                // d2b.site.waylandUser). Do not shell out.
                let wayland_user_uid = runtime
                    .file_name()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<u32>().ok());

                // Open, verify ownership, and hold the fd. The traverse ACL
                // is applied through this fd so the inode that was verified
                // is the exact inode mutated (no re-open window).
                let runtime_file = match open_o_path_metadata(runtime) {
                    Err(failure) => {
                        return Err(LiveHandlerError::SpawnFailed {
                            detail: format!(
                                "graphical-session-not-active: cannot open runtime dir for wayland-proxy uid {}: {}",
                                plan.uid, failure.legacy_detail
                            ),
                        });
                    }
                    Ok(None) => {
                        return Err(LiveHandlerError::SpawnFailed {
                            detail: format!(
                                "graphical-session-not-active: runtime dir {} is absent for wayland-proxy uid {}; \
                                     start the graphical session before starting this VM",
                                runtime.display(),
                                plan.uid
                            ),
                        });
                    }
                    Ok(Some((file, meta))) => {
                        if let Some(expected_uid) = wayland_user_uid
                            && meta.uid() != expected_uid
                        {
                            return Err(LiveHandlerError::SpawnFailed {
                                detail: format!(
                                    "graphical-session-not-active: runtime dir owner mismatch for wayland-proxy uid {}: \
                                             expected owner uid {expected_uid}",
                                    plan.uid,
                                ),
                            });
                        }
                        file
                    }
                };

                // Grant traverse through the verified fd (no re-open).
                crate::sys::pidfd_sys::run_setfacl_op_on_fd(
                    runtime_file.as_fd(),
                    "-m",
                    &format!("u:{}:--x", plan.uid),
                )
                .map_err(|err| LiveHandlerError::SpawnFailed {
                    detail: format!(
                        "refresh runtime dir traverse ACL for wayland-proxy uid {}: {err}",
                        plan.uid
                    ),
                })?;

                let wayland_display = env_value(plan, "WAYLAND_DISPLAY").unwrap_or("wayland-0");
                let socket_path = runtime.join(wayland_display);

                // Open, verify socket type, and hold the fd. The socket ACL
                // is applied through this fd (no re-open window).
                let socket_file = match open_o_path_metadata(&socket_path) {
                    Err(failure) => {
                        return Err(LiveHandlerError::SpawnFailed {
                            detail: format!(
                                "graphical-session-not-active: cannot open Wayland socket for wayland-proxy uid {}: {}",
                                plan.uid, failure.legacy_detail
                            ),
                        });
                    }
                    Ok(None) => {
                        return Err(LiveHandlerError::SpawnFailed {
                            detail: format!(
                                "graphical-session-not-active: Wayland socket {} is absent for wayland-proxy uid {}; \
                                 the compositor may not be running",
                                socket_path.display(),
                                plan.uid
                            ),
                        });
                    }
                    Ok(Some((_, meta))) if !meta.file_type().is_socket() => {
                        return Err(LiveHandlerError::SpawnFailed {
                            detail: format!(
                                "graphical-session-not-active: path at Wayland socket location is not a socket \
                                 for wayland-proxy uid {}",
                                plan.uid
                            ),
                        });
                    }
                    Ok(Some((file, _))) => file,
                };

                // Grant socket access through the verified fd (no re-open).
                crate::sys::pidfd_sys::run_setfacl_op_on_fd(
                    socket_file.as_fd(),
                    "-m",
                    &format!("u:{}:rwx", plan.uid),
                )
                .map_err(|err| LiveHandlerError::SpawnFailed {
                    detail: format!(
                        "refresh host compositor socket ACL for wayland-proxy uid {}: {err}",
                        plan.uid
                    ),
                })?;

                // Audio socket revocations use setfacl_fd_safe (which
                // internally re-opens). These sockets may legitimately be
                // absent; a symlink here can only block a deny-ACL, not
                // grant access. setfacl_fd_safe returns Ok on NotFound.
                for socket in ["pipewire-0", "pulse/native"] {
                    let path = runtime.join(socket);
                    setfacl_fd_safe(&path, &format!("u:{}:---", plan.uid), AclPathKind::Socket)
                        .map_err(|detail| LiveHandlerError::SpawnFailed {
                            detail: format!(
                                "revoke audio socket ACL for wayland-proxy uid {}: {detail}",
                                plan.uid
                            ),
                        })?;
                }
            }
        }
    }
    if plan.seccomp_policy_ref.as_deref() == Some("w1-qemu-media") {
        // qemu-media uses QEMU's GTK/Wayland display path. Grant only the
        // compositor socket plus directory traversal; keep audio sockets denied.
        let runtime_dir = env_value(plan, "XDG_RUNTIME_DIR");
        if let Some(runtime_dir) = runtime_dir {
            let runtime = Path::new(runtime_dir);
            let wayland_display = env_value(plan, "WAYLAND_DISPLAY").unwrap_or("wayland-0");
            setfacl_fd_safe(
                runtime,
                &format!("u:{}:rx", plan.uid),
                AclPathKind::Directory,
            )
            .map_err(|detail| LiveHandlerError::SpawnFailed {
                detail: format!(
                    "refresh session runtime ACL for qemu-media uid {}: {detail}",
                    plan.uid
                ),
            })?;
            setfacl_fd_safe(
                &runtime.join(wayland_display),
                &format!("u:{}:rwx", plan.uid),
                AclPathKind::Socket,
            )
            .map_err(|detail| LiveHandlerError::SpawnFailed {
                detail: format!(
                    "refresh host compositor socket ACL for qemu-media uid {}: {detail}",
                    plan.uid
                ),
            })?;
            for socket in ["pipewire-0", "pulse/native"] {
                let path = runtime.join(socket);
                setfacl_fd_safe(&path, &format!("u:{}:---", plan.uid), AclPathKind::Socket)
                    .map_err(|detail| LiveHandlerError::SpawnFailed {
                        detail: format!(
                            "revoke audio socket ACL for qemu-media uid {}: {detail}",
                            plan.uid
                        ),
                    })?;
            }
        }
    }
    if let Some(api_socket) = cloud_hypervisor_api_socket(plan) {
        let state_dir = api_socket
            .parent()
            .ok_or_else(|| LiveHandlerError::SpawnFailed {
                detail: "cloud-hypervisor api socket has no state-directory parent".to_owned(),
            })?;
        grant_runner_tree_acls(state_dir, broker_state_dir, plan.uid).map_err(|detail| {
            LiveHandlerError::SpawnFailed {
                detail: format!(
                    "refresh cloud-hypervisor state-directory ACL for runner uid {}: {detail}",
                    plan.uid
                ),
            }
        })?;
    }
    refresh_obs_vsock_acl(plan).await?;
    refresh_component_session_vsock_acl(plan)?;

    Ok(())
}

fn cloud_hypervisor_api_socket(plan: &SpawnRunnerPlan) -> Option<PathBuf> {
    if plan.seccomp_policy_ref.as_deref() != Some("w1-cloud-hypervisor-runner") {
        return None;
    }
    plan.argv
        .windows(2)
        .find_map(|pair| (pair[0] == "--api-socket").then(|| PathBuf::from(&pair[1])))
}

/// Per-runner ACL targets for one runner path tree the broker must open to
/// a runner principal: search (`u:<uid>:--x`) on every ancestor that no
/// unprivileged principal can already search, and full `u:<uid>:rwx` on the
/// tree leaf.
///
/// `leaf` must be `bound` or strictly inside it, and both must be absolute
/// and normalized: the bound is the broker-owned root the grant may never
/// reach above (the broker state directory for a runner's state tree, the
/// broker runtime directory for a runner's private socket tree). Ancestors
/// already world-searchable need no entry - every principal can already
/// reach them. A missing ancestor is refused instead of skipped, so a
/// partially-open chain can never be granted silently.
fn runner_tree_acl_targets(
    leaf: &Path,
    bound: &Path,
    uid: u32,
) -> Result<Vec<(PathBuf, String)>, String> {
    if !leaf.is_absolute() || !bound.is_absolute() {
        return Err("runner ACL paths must be absolute".to_owned());
    }
    if leaf
        .components()
        .chain(bound.components())
        .any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err("runner ACL paths must be normalized".to_owned());
    }
    if leaf.strip_prefix(bound).is_err() {
        return Err(format!(
            "runner path {} is outside the broker-owned root {}",
            leaf.display(),
            bound.display()
        ));
    }

    let mut chain = Vec::new();
    let mut directory = leaf;
    loop {
        chain.push(directory.to_path_buf());
        if directory == bound {
            break;
        }
        directory = directory
            .parent()
            .ok_or_else(|| "runner path has no broker-owned ancestor".to_owned())?;
    }
    chain.reverse();

    let last = chain.len().saturating_sub(1);
    let mut targets = Vec::with_capacity(chain.len());
    for (index, directory) in chain.into_iter().enumerate() {
        if index != last {
            match dir_needs_traverse_grant(&directory).map_err(|failure| failure.legacy_detail)? {
                Some(true) => targets.push((directory, format!("u:{uid}:--x"))),
                Some(false) => {}
                None => {
                    return Err(format!(
                        "runner path ancestor is absent: {}",
                        directory.display()
                    ));
                }
            }
        } else {
            targets.push((directory, format!("u:{uid}:rwx")));
        }
    }
    Ok(targets)
}

fn grant_runner_tree_acls(leaf: &Path, bound: &Path, uid: u32) -> Result<(), String> {
    for (directory, acl) in runner_tree_acl_targets(leaf, bound, uid)? {
        setfacl_fd_safe(&directory, &acl, AclPathKind::Directory)?;
    }
    Ok(())
}

/// The two path trees one binding-owned serving worker's launch ticket
/// names, parsed from the trusted argv the broker composed.
#[derive(Debug, PartialEq, Eq)]
struct ServingWorkerLaunchPaths {
    /// Directory the worker binds its private socket in.
    socket_dir: PathBuf,
    /// View root the worker serves.
    shared_dir: PathBuf,
    /// Whether the attachment is served read-only (`--readonly`).
    read_only: bool,
}

/// Whether `path` is absolute and carries no `.`/`..` component.
///
/// A `.`/`..` component is refused outright wherever the broker fences a
/// caller-named path with a `starts_with` comparison: `/run/d2b/../etc` is
/// a component prefix of `/run/d2b` while resolving outside it.
fn is_anchored_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

/// The absolute, normalized spelling of one argv path value.
fn anchored_absolute_path(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let path = PathBuf::from(raw);
    is_anchored_absolute(&path).then_some(path)
}

/// Parse `--socket-path=`, `--shared-dir=` and `--readonly` out of a
/// binding-owned serving worker's argv.
///
/// Both path flags are mandatory: the signed launch ticket always carries
/// the frozen virtiofsd spelling (`serving_worker_launch_args` renders
/// `--socket-path=<p>` / `--shared-dir=<p>`), so a serving-posture launch
/// that names neither is refused rather than left with a partially-open
/// tree the worker would silently fail to use.
fn serving_worker_launch_paths(argv: &[String]) -> Result<ServingWorkerLaunchPaths, String> {
    let mut socket_path = None;
    let mut shared_dir = None;
    let mut read_only = false;
    for argument in argv {
        if let Some(value) = argument.strip_prefix("--socket-path=") {
            socket_path = Some(value);
        } else if let Some(value) = argument.strip_prefix("--shared-dir=") {
            shared_dir = Some(value);
        } else if argument == "--readonly" {
            read_only = true;
        }
    }
    let socket_path = anchored_absolute_path(
        socket_path.ok_or_else(|| "serving worker argv names no --socket-path".to_owned())?,
    )
    .ok_or_else(|| "serving worker --socket-path is not an absolute anchored path".to_owned())?;
    let shared_dir = anchored_absolute_path(
        shared_dir.ok_or_else(|| "serving worker argv names no --shared-dir".to_owned())?,
    )
    .ok_or_else(|| "serving worker --shared-dir is not an absolute anchored path".to_owned())?;
    let socket_dir = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "serving worker --socket-path has no parent directory".to_owned())?
        .to_path_buf();
    Ok(ServingWorkerLaunchPaths {
        socket_dir,
        shared_dir,
        read_only,
    })
}

/// Per-runner ACL targets for one served view root: search
/// (`u:<uid>:--x`) on every ancestor no unprivileged principal can already
/// search, and read/traverse (`u:<uid>:r-x`) - or read/write/traverse for a
/// read-write attachment - on the served root itself.
///
/// Unlike [`runner_tree_acl_targets`] there is no broker-owned bound to
/// stop at: a served root is a bundle-declared storage path, so the walk
/// stops at the first ancestor every principal can already search. That
/// level (and every level above it) already grants search to everyone, so
/// nothing outside the served tree is ever opened.
fn served_view_root_acl_targets(
    root: &Path,
    uid: u32,
    read_only: bool,
) -> Result<Vec<(PathBuf, String)>, String> {
    if !is_anchored_absolute(root) {
        return Err("served view root must be an absolute normalized path".to_owned());
    }
    // `/` has no ancestors to stop the walk at, so the loop below would fall
    // through to the leaf push and open the filesystem root itself to the
    // runner principal. A served view root is a bundle-declared storage path;
    // the filesystem root is never one.
    if root.parent().is_none() {
        return Err("served view root must not be the filesystem root".to_owned());
    }
    let mut targets = Vec::new();
    for directory in root.ancestors().skip(1) {
        if directory.as_os_str().is_empty() {
            break;
        }
        match dir_needs_traverse_grant(directory).map_err(|failure| failure.legacy_detail)? {
            Some(true) => targets.push((directory.to_path_buf(), format!("u:{uid}:--x"))),
            Some(false) => break,
            None => {
                return Err(format!(
                    "served view root ancestor is absent: {}",
                    directory.display()
                ));
            }
        }
    }
    targets.push((
        root.to_path_buf(),
        format!("u:{uid}:{}", if read_only { "r-x" } else { "rwx" }),
    ));
    Ok(targets)
}

/// Whether `root` is an existing directory the worker can be granted a
/// served view on (symlinks refused: `open_o_path_metadata` resolves with
/// `RESOLVE_NO_SYMLINKS`).
fn validate_served_view_root(root: &Path) -> Result<(), String> {
    match open_o_path_metadata(root).map_err(|failure| failure.legacy_detail)? {
        Some((_file, metadata)) if metadata.file_type().is_dir() => Ok(()),
        Some(_) => Err(format!(
            "serving worker view root {} is not a directory",
            root.display()
        )),
        None => Err(format!(
            "serving worker view root {} does not exist",
            root.display()
        )),
    }
}

/// Open the two path trees a binding-owned serving worker's trusted launch
/// ticket names to the runner principal with per-runner ACLs.
///
/// The worker runs as the trusted intent's principal, never as the daemon:
/// the broker's only authentication factor is peer identity
/// (`peer_matches_instance` admits exactly the daemon uid/gid on
/// `/run/d2b/priv.sock`), so a worker launched with the daemon identity
/// would be able to call the whole daemon API after a guest -> worker
/// compromise. The daemon-provisioned trees the worker legitimately needs
/// are therefore opened to its own principal instead:
///
/// - its private socket directory (strictly inside `runtime_root`, the
///   directory the broker's own socket lives in) gets `rwx` plus search on
///   the chain up to `runtime_root`, the same shape the cloud-hypervisor
///   runner's per-VM socket directory already uses;
/// - its served view root gets `r-x` (`rwx` for a read-write attachment)
///   plus search on the non-world-searchable chain above it.
///
/// Every failure is fail-closed: a path that is absent, not a directory, a
/// symlink, outside `runtime_root`, or unopenable for setfacl refuses the
/// launch instead of spawning a worker that cannot serve.
pub(crate) fn grant_serving_worker_launch_acls(
    argv: &[String],
    uid: u32,
    runtime_root: &Path,
) -> Result<(), LiveHandlerError> {
    let paths = serving_worker_launch_paths(argv)
        .map_err(|detail| LiveHandlerError::SpawnFailed { detail })?;
    // The private socket is broker-runtime state: it must live strictly
    // below the broker's own runtime directory, so this grant can never
    // reach above a tree the broker owns.
    if paths.socket_dir == runtime_root || !paths.socket_dir.starts_with(runtime_root) {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!(
                "serving worker private socket directory {} is outside the broker runtime directory {}",
                paths.socket_dir.display(),
                runtime_root.display()
            ),
        });
    }
    // All path validation happens before any mutation: a launch whose view
    // root is unusable must not leave a half-opened socket tree behind.
    validate_served_view_root(&paths.shared_dir)
        .map_err(|detail| LiveHandlerError::SpawnFailed { detail })?;
    grant_runner_tree_acls(&paths.socket_dir, runtime_root, uid).map_err(|detail| {
        LiveHandlerError::SpawnFailed {
            detail: format!("serving worker private socket directory ACL: {detail}"),
        }
    })?;
    for (directory, acl) in served_view_root_acl_targets(&paths.shared_dir, uid, paths.read_only)
        .map_err(|detail| LiveHandlerError::SpawnFailed { detail })?
    {
        setfacl_fd_safe(&directory, &acl, AclPathKind::Directory).map_err(|detail| {
            LiveHandlerError::SpawnFailed {
                detail: format!("serving worker view root ACL: {detail}"),
            }
        })?;
    }
    Ok(())
}

/// One Device-owned worker's trusted per-Guest socket directory, and the
/// runtime root it must strictly live under.
///
/// The directory is derived from trusted identity - the Guest the owning
/// Device declares, or, for a legacy VM-scoped worker, the VM of the launch's
/// own trusted cgroup placement crossed with the plan's trusted writable
/// paths - never from the launch arguments: a launch that is refused must not
/// be able to leave an ACL on a directory it named.
///
/// The worker runs as the trusted intent's principal, never as the daemon: the
/// per-VM device socket directory under the broker runtime root is owned by
/// the daemon (`d2bd:d2bd 0770`) and carries no grant for that principal, so
/// the worker cannot bind its socket there without this. Only the derived
/// directory (strictly inside the broker's own runtime root) is opened, and
/// only after the launch's typed fences passed, so a refused launch is a
/// no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceWorkerSocketGrant {
    /// Broker runtime root (the private socket's directory).
    runtime_root: PathBuf,
    /// Directory the worker binds its socket in, strictly inside the root.
    directory: PathBuf,
}

impl DeviceWorkerSocketGrant {
    /// The per-Guest socket directory of one pinned owning-Device scope:
    /// `<runtime_root>/vms/<guest>`.
    fn for_guest(runtime_root: &Path, guest: &str) -> Result<Self, String> {
        let directory = crate::ops::device_worker::guest_socket_directory(runtime_root, guest)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            runtime_root: runtime_root.to_path_buf(),
            directory,
        })
    }

    /// The legacy VM-scoped worker's socket directory: the runtime directory
    /// the plan's own trusted derivation names
    /// ([`crate::ops::swtpm_dir::derive_paths`] cross-checks the cgroup
    /// placement's VM against the plan's writable paths, so this never reads
    /// an argument).
    fn for_legacy_plan(plan: &SpawnRunnerPlan, runtime_root: &Path) -> Result<Self, String> {
        let paths = crate::ops::swtpm_dir::derive_paths(plan)
            .map_err(|reason| format!("device worker runtime directory: {reason}"))?;
        let directory = paths.runtime_dir;
        if !is_anchored_absolute(&directory) {
            return Err(
                "device worker runtime directory is not an absolute normalized path".to_owned(),
            );
        }
        if directory == runtime_root || !directory.starts_with(runtime_root) {
            return Err(format!(
                "device worker socket directory {} is outside the broker runtime directory {}",
                directory.display(),
                runtime_root.display()
            ));
        }
        Ok(Self {
            runtime_root: runtime_root.to_path_buf(),
            directory,
        })
    }

    /// Open the directory (and the non-world-searchable ancestors up to the
    /// runtime root) to the launched principal.
    fn apply(&self, uid: u32) -> Result<(), String> {
        grant_runner_tree_acls(&self.directory, &self.runtime_root, uid)
    }
}

/// Derive the trusted per-Guest socket grant of one Device-owned worker
/// launch, when its role binds a socket under the runtime root.
///
/// A typed launch's directory comes from the owning Device the launch arm
/// pinned against the verified bundle (`DeviceWorkerLaunch::scope`); a legacy
/// VM-scoped launch's comes from the plan's own trusted swtpm-dir derivation
/// (`for_legacy_plan`). A role that binds no such socket - the one-shot flush
/// (ctrl socket in the state Volume) and the video sidecar (its own
/// `/run/d2b-video` runtime directory) - grants nothing.
fn device_worker_socket_grant(
    plan: &SpawnRunnerPlan,
    device_worker: &crate::ops::device_worker::DeviceWorkerLaunch,
    runtime_root: &Path,
) -> Result<Option<DeviceWorkerSocketGrant>, LiveHandlerError> {
    if !device_worker.binds_runtime_socket {
        return Ok(None);
    }
    let grant = match device_worker.scope.as_ref() {
        Some(scope) => DeviceWorkerSocketGrant::for_guest(runtime_root, scope.guest()),
        None => DeviceWorkerSocketGrant::for_legacy_plan(plan, runtime_root),
    }
    .map_err(|detail| LiveHandlerError::SpawnFailed {
        detail: format!("device worker socket directory: {detail}"),
    })?;
    Ok(Some(grant))
}

/// How long one daemon API-socket / component-session vsock ACL refresh
/// keeps retrying before it gives up.
const ACL_RETRY_WINDOW: Duration = Duration::from_secs(30);
/// How long one ACL refresh waits between attempts.
const ACL_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// Run one bounded ACL-grant retry on the broker's reactor.
///
/// The attempt runs on the bounded dispatch pool (its setfacl shellout has
/// no async form, so it belongs on the pool like every other kernel-path
/// step); the wait between attempts is timer time on the reactor, and the
/// loop gives up on its own budget. Returns when the attempt reports ready,
/// the deadline passes, or the pool is gone. This is the canonical async
/// retry shape (`spawn_obs_vsock_acl_retry`); the daemon API-socket and
/// component-session vsock refreshes share it (plan U8 item 4).
async fn retry_acl_grant(
    background: &crate::runtime::BrokerBackground,
    window: Duration,
    interval: Duration,
    label: &'static str,
    attempt: Arc<dyn Fn() -> Result<bool, String> + Send + Sync>,
) {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let attempt = Arc::clone(&attempt);
        let outcome = background
            .dispatches
            .run(move || attempt())
            .await;
        match outcome {
            Ok(Ok(true)) => return,
            Ok(Ok(false)) => {}
            Ok(Err(err)) => {
                tracing::debug!(
                    error = %err,
                    label = %label,
                    "ACL refresh not ready yet",
                );
            }
            // The pool is gone, so the broker is shutting down.
            Err(_) => return,
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(label = %label, "ACL refresh timed out");
            return;
        }
        tokio::time::sleep(interval).await;
    }
}

/// Retry the cloud-hypervisor api-socket ACL grant in a background task
/// until the socket appears (bounded to ~30s, matching the obs-vsock
/// precedent). The attempt's setfacl shellout has no async form, so it runs
/// on the broker's bounded dispatch pool; the wait between attempts is
/// timer time on the reactor, not a thread.
fn grant_daemon_api_socket_acl(api_socket: PathBuf) {
    let Some(background) = crate::runtime::broker_background() else {
        // Only a serving broker has a reactor to retry on; a handler driven
        // outside one has nothing to retry against either.
        tracing::warn!(
            path = %api_socket.display(),
            "cloud-hypervisor api socket ACL refresh dropped: no broker reactor is running",
        );
        return;
    };
    let attempt: Arc<dyn Fn() -> Result<bool, String> + Send + Sync> = Arc::new(move || {
        if api_socket.exists() {
            match setfacl_fd_safe(&api_socket, "u:d2bd:rwx", AclPathKind::Socket) {
                Ok(()) => Ok(true),
                Err(err) => Err(err),
            }
        } else {
            Ok(false)
        }
    });
    background.runtime.spawn(retry_acl_grant(
        background,
        ACL_RETRY_WINDOW,
        ACL_RETRY_INTERVAL,
        "cloud-hypervisor api socket",
        attempt,
    ));
}

/// The system principal the framework grants daemon-side component-session
/// vsock access to. The `d2bd` daemon owns the per-VM lifecycle DAG
/// and is the only host process that connects to the component-session
/// vsock socket for the readiness probe / config-sync over the bridge.
const COMPONENT_SESSION_DAEMON_PRINCIPAL: &str = "d2bd";

/// Extract the cloud-hypervisor `--vsock socket=<path>` argument for a
/// CH runner plan. Gated on the CH runner's seccomp policy ref so the
/// daemon-vsock ACL is only ever attached to a real cloud-hypervisor
/// runner's vsock socket, never to any other role's argv.
fn cloud_hypervisor_component_session_socket_arg(plan: &SpawnRunnerPlan) -> Option<PathBuf> {
    if plan.seccomp_policy_ref.as_deref() != Some("w1-cloud-hypervisor-runner") {
        return None;
    }
    plan.argv.windows(2).find_map(|pair| {
        if pair[0] != "--vsock" {
            return None;
        }
        pair[1]
            .split(',')
            .find_map(|field| field.strip_prefix("socket=").map(PathBuf::from))
    })
}

/// Path-free digest of a component-session vsock ACL mutation, for audit.
///
/// Returns `sha256:<hex>` over the operation, the target class, and the
/// target's resolved `(dev, ino)` - never the raw socket / state-dir
/// path. The component-session observability contract forbids raw vsock /
/// socket / state-dir paths in spans, logs, metrics, and audit.
fn component_session_acl_diff_hash(op: &str, target_class: &str, dev: u64, ino: u64) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"component-session-vsock-acl\0");
    hasher.update(op.as_bytes());
    hasher.update([0]);
    hasher.update(target_class.as_bytes());
    hasher.update([0]);
    hasher.update(dev.to_le_bytes());
    hasher.update(ino.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut out = String::from("sha256:");
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Emit a hash-only audit event for a component-session vsock ACL mutation.
/// Closed-enum labels only; no raw paths, uids-by-value, or content.
fn audit_component_session_vsock_acl(op: &str, target_class: &str, dev: u64, ino: u64) {
    tracing::info!(
        kind = "critical",
        subsystem = "component-session-health",
        op = op,
        daemon_principal = COMPONENT_SESSION_DAEMON_PRINCIPAL,
        target_class = target_class,
        acl_diff_hash = %component_session_acl_diff_hash(op, target_class, dev, ino),
        result = "ok",
        "component-session vsock daemon ACL mutation",
    );
}

/// Path-free wrapper over [`setfacl_fd_safe_op_classed`] for the
/// component-session ACL path: on failure, builds a detail string carrying
/// only the closed-set op/target-class/stage/errno classification -
/// never the raw socket/state-dir path or the acl-spec string.
fn setfacl_component_session(
    path: &Path,
    op: &str,
    acl_spec: &str,
    kind: AclPathKind,
    op_label: &str,
    target_class: &str,
) -> Result<Option<(u64, u64)>, String> {
    setfacl_fd_safe_op_classed(path, op, acl_spec, kind)
        .map_err(|failure| failure.component_session_detail(op_label, target_class))
}

/// Whether a runner principal needs an explicit `--x` grant to traverse
/// `path`.
///
/// `Ok(Some(true))` if `path` is a directory no unprivileged principal can
/// already search (world execute bit clear). `Ok(Some(false))` if it is a
/// world-traversable directory (no grant needed) or not a directory.
/// `Ok(None)` if the path is absent.
fn dir_needs_traverse_grant(path: &Path) -> Result<Option<bool>, SetfaclFailure> {
    let Some((_file, metadata)) = open_o_path_metadata(path)? else {
        return Ok(None);
    };
    if !metadata.file_type().is_dir() {
        return Ok(Some(false));
    }
    Ok(Some(metadata.mode() & 0o001 == 0))
}

/// Resolve the current `(dev, ino)` of `path` via an
/// `openat2(O_PATH|NOFOLLOW|RESOLVE_NO_SYMLINKS)` fstat. `Ok(None)` if
/// the path is absent.
fn current_path_dev_ino(path: &Path) -> Result<Option<(u64, u64)>, SetfaclFailure> {
    Ok(open_o_path_metadata(path)?.map(|(_file, metadata)| (metadata.dev(), metadata.ino())))
}

/// Grant the daemon `u:d2bd:--x` on every non-world-traversable
/// directory from the filesystem root down to `leaf` (inclusive), so the
/// daemon can `connect()` to the per-VM component-session socket through the
/// full ancestor chain - not just the immediate parent. World-
/// traversable directories already grant search to everyone and are
/// skipped. The immediate per-VM leaf is audited as `state-dir`; higher
/// non-world-x ancestors as `ancestor`. These grants are additive and
/// idempotent; they are never revoked because sibling VMs and the
/// per-VM api-socket also depend on them.
fn grant_component_session_traversal_acls(leaf: &Path) -> Result<(), String> {
    let mut chain: Vec<&Path> = leaf
        .ancestors()
        .filter(|component| !component.as_os_str().is_empty())
        .collect();
    chain.reverse();
    let last_idx = chain.len().saturating_sub(1);
    for (idx, dir) in chain.iter().enumerate() {
        let target_class = if idx == last_idx {
            "state-dir"
        } else {
            "ancestor"
        };
        let needs = dir_needs_traverse_grant(dir)
            .map_err(|failure| failure.component_session_detail("grant", target_class))?;
        if needs == Some(true)
            && let Some((dev, ino)) = setfacl_component_session(
                dir,
                "-m",
                &format!("u:{COMPONENT_SESSION_DAEMON_PRINCIPAL}:--x"),
                AclPathKind::Directory,
                "grant",
                target_class,
            )?
        {
            audit_component_session_vsock_acl("grant", target_class, dev, ino);
        }
    }
    Ok(())
}

/// Grant `u:d2bd:rw` on the component-session vsock socket inode, then
/// re-stat the path to confirm it still resolves to the same `(dev,
/// ino)` the fd-based setfacl mutated (inode pinning). If the
/// socket was replaced (or vanished) between the setfacl and the
/// re-stat, the grant landed on a now-stale inode: do not audit success
/// and report not-ready (`Ok(false)`) so the caller retries against the
/// current, live inode. Returns `Ok(false)` while the socket has not yet
/// been created by cloud-hypervisor.
fn grant_component_session_socket_acl_once(socket: &Path) -> Result<bool, String> {
    let Some((dev, ino)) = setfacl_component_session(
        socket,
        "-m",
        &format!("u:{COMPONENT_SESSION_DAEMON_PRINCIPAL}:rw"),
        AclPathKind::Socket,
        "grant",
        "vsock-socket",
    )?
    else {
        return Ok(false);
    };
    match current_path_dev_ino(socket)
        .map_err(|failure| failure.component_session_detail("grant", "vsock-socket"))?
    {
        Some(current) if current == (dev, ino) => {
            audit_component_session_vsock_acl("grant", "vsock-socket", dev, ino);
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Revoke the daemon-principal component-session vsock ACL from the socket
/// inode. Best-effort, idempotent, and path-free: a missing socket is a
/// no-op. Scoped to the per-VM socket inode only - the shared/ancestor
/// traversal grants are intentionally retained (the daemon also needs
/// them for the per-VM api-socket and sibling VMs depend on them). This
/// is the production revoke wiring: there is no CH-stop teardown hook
/// carrying the socket path (`SignalRunner` has only vm_id/role_id/
/// signal), so revoke runs as a revoke-then-grant at the next CH
/// (re-)spawn so a replaced/disabled socket cannot retain a stale grant.
fn revoke_component_session_vsock_acl(socket: &Path) -> Result<(), String> {
    if let Some((dev, ino)) = setfacl_component_session(
        socket,
        "-x",
        &format!("u:{COMPONENT_SESSION_DAEMON_PRINCIPAL}"),
        AclPathKind::Socket,
        "revoke",
        "vsock-socket",
    )? {
        audit_component_session_vsock_acl("revoke", "vsock-socket", dev, ino);
    }
    Ok(())
}

/// Retry the daemon-vsock socket ACL grant in a background task until
/// the cloud-hypervisor process has created the vsock socket (bounded to
/// ~30s, matching the obs-vsock precedent). The traversal ACLs are
/// granted synchronously before this task starts, so the retry only
/// re-attempts the socket grant. The attempt's setfacl shellout runs on
/// the broker's bounded dispatch pool; the wait is timer time on the
/// reactor (the canonical async retry, plan U8 item 4). Logs are
/// path-free.
fn spawn_component_session_vsock_acl_retry(socket: PathBuf) {
    let Some(background) = crate::runtime::broker_background() else {
        // Only a serving broker has a reactor to retry on; a handler driven
        // outside one has nothing to retry against either.
        tracing::warn!(
            subsystem = "component-session-health",
            "component-session vsock daemon ACL refresh dropped: no broker reactor is running",
        );
        return;
    };
    let attempt: Arc<dyn Fn() -> Result<bool, String> + Send + Sync> = Arc::new(move || {
        grant_component_session_socket_acl_once(&socket)
    });
    background.runtime.spawn(retry_acl_grant(
        background,
        ACL_RETRY_WINDOW,
        ACL_RETRY_INTERVAL,
        "component-session vsock daemon",
        attempt,
    ));
}

/// Refresh the daemon-vsock ACL for a CH runner plan. No-op for any
/// non-CH runner (no `--vsock socket=` arg).
///
/// Revoke-then-grant: first revoke any stale per-VM daemon grant left on
/// the (possibly replaced) socket inode from a prior generation, then
/// (re-)establish the full ancestor traversal chain and grant `rw` on
/// the live socket. The traversal grant is applied synchronously so the
/// daemon never loses search on the per-VM state dir (the api-socket
/// depends on it too); if the socket is not yet present, a bounded retry
/// thread completes the socket grant.
fn refresh_component_session_vsock_acl(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
    let Some(socket) = cloud_hypervisor_component_session_socket_arg(plan) else {
        return Ok(());
    };
    let Some(parent) = socket.parent().map(Path::to_path_buf) else {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "component-session vsock path has no parent".to_owned(),
        });
    };

    if let Err(detail) = revoke_component_session_vsock_acl(&socket) {
        tracing::debug!(
            subsystem = "component-session-health",
            detail = %detail,
            "pre-grant component-session daemon ACL revoke (best-effort)",
        );
    }

    grant_component_session_traversal_acls(&parent).map_err(|detail| {
        LiveHandlerError::SpawnFailed {
            detail: format!("refresh component-session traversal ACLs: {detail}"),
        }
    })?;

    match grant_component_session_socket_acl_once(&socket) {
        Ok(true) => Ok(()),
        Ok(false) => {
            spawn_component_session_vsock_acl_retry(socket);
            Ok(())
        }
        Err(detail) => Err(LiveHandlerError::SpawnFailed {
            detail: format!("refresh component-session vsock daemon ACL: {detail}"),
        }),
    }
}

/// Live broker `SpawnRunner` handler.
///
/// 1. Validates the plan via `ops::spawn_runner::preflight`.
/// 2. Builds the CString triple for execve.
/// 3. Loads the seccomp blob + cgroup placement fd before clone/fork.
/// 4. Spawns via `sys::pidfd_sys::clone3_spawn_runner`, whose child
///    closure applies no_new_privs, namespaces, mounts, capabilities,
///    seccomp, uid/gid drop, then `execve`.
/// 5. Reads the new child's `/proc/<pid>/stat` field 22 for the
///    daemon's pidfd-table bookkeeping.
///
/// Caller transports the pidfd via SCM_RIGHTS in the broker
/// response frame.
pub async fn live_spawn_runner(
    plan_input: &SpawnRunnerPlanInput,
    mut pre_opened_device_fds: Vec<std::os::fd::OwnedFd>,
    request_fds: Vec<std::os::fd::OwnedFd>,
    activation_input: Option<&ActivationRunnerInput>,
    broker_state_dir: &Path,
    // Trusted identity of a resource-backed `w1-swtpm` launch, resolved from
    // the verified bundle by the dispatch layer (`None` for every other
    // launch); the swtpm-dir fence uses it instead of the cgroup placement.
    swtpm_identity: Option<&crate::ops::swtpm_dir::ResourceBackedSwtpm>,
    // Device-owned worker scope (pinned owning Device + whether this role
    // binds a socket under `runtime_root`) resolved by the dispatch layer;
    // `default()` for every other launch.
    device_worker: &crate::ops::device_worker::DeviceWorkerLaunch,
    // Broker runtime root: the tree the worker's per-Guest socket directory
    // must strictly live under before this handler opens it.
    runtime_root: &Path,
) -> Result<SpawnRunnerResult, LiveHandlerError> {
    let plan = preflight(plan_input).map_err(LiveHandlerError::SpawnPreflight)?;

    // Wayland-proxy role: mandatory seccomp. The proxy parses untrusted
    // guest Wayland bytes while holding the host compositor socket; a
    // null/absent seccomp policy is a hard reject.
    if plan.seccomp_policy_ref.as_deref() == Some("w1-wayland-proxy") {
        // Caps must be empty: wayland-proxy must never hold host capabilities.
        if !plan.capabilities.is_empty() {
            return Err(LiveHandlerError::SpawnFailed {
                detail: format!(
                    "wayland-proxy role must have empty capabilities; \
                     got {:?}",
                    plan.capabilities
                ),
            });
        }
    }
    validate_qemu_media_runner_hardening(&plan)?;
    crate::ops::gpu::validate_spawn_plan_preflight(&plan).map_err(|error| {
        LiveHandlerError::SpawnFailed {
            detail: error.to_string(),
        }
    })?;

    let (binary, argv, env) =
        build_cstring_vectors(&plan).map_err(LiveHandlerError::SpawnPreflight)?;
    let seccomp_program = load_runner_seccomp(&plan).await?;
    let cgroup_fds = prepare_runner_cgroup_fds(&plan.cgroup_placement)?;
    refresh_spawn_runner_acls(&plan, broker_state_dir).await?;

    // swtpm-dir first-run hardening (issue #64). Gated on the
    // `w1-swtpm` role and run BEFORE clone3 so the persistent TPM2
    // NVRAM dir is provisioned + identity-bound (or fails closed)
    // before swtpm - which opens the NVRAM by pathname under its user
    // namespace - is ever spawned. ONLY the persistent state dir is
    // touched; the `/run` runtime-socket-dir posture is left intact.
    let swtpm_dir_audit = maybe_harden_swtpm_dir(&plan, swtpm_identity).await?;
    let api_socket_acl_path = cloud_hypervisor_api_socket(&plan);

    // Pre-open /dev/dri/renderD128 for gpu-render-node broker-pre-NS
    // spawns (ADR 0021).
    //
    // Detection: seccomp_policy_ref == "w1-gpu-render-node" AND
    // user_namespace.is_some() (both conditions must hold; the policy
    // ref is the canonical identifier for the render-node-only profile
    // and avoids introducing a new SpawnRunnerPlan field).
    //
    // The fd is opened here (parent side, before clone3(CLONE_NEWUSER))
    // so the DAC permission check runs as the broker UID - the child's
    // user-NS UID mapping provides no host-side access. The OwnedFd is
    // moved into RunnerIsolationSpec.pre_opened_device_fds; the broker
    // sys layer dup2's it to RENDER_NODE_INHERITED_FD (10) in the child
    // closure before execve. The crosvm argv carries
    // --gpu-device-node /proc/self/fd/10 as the render node path.
    if plan.seccomp_policy_ref.as_deref() == Some("w1-gpu-render-node")
        && plan.user_namespace.is_some()
    {
        let render_fd = crate::ops::device::open_device_fd(
            std::path::Path::new("/dev/dri/renderD128"),
            true, // read-write: render nodes require rw for DRI ioctls
        )
        .map_err(|e| LiveHandlerError::SpawnFailed {
            detail: format!("pre-open /dev/dri/renderD128 for gpu-render-node: {e}"),
        })?;
        pre_opened_device_fds.push(render_fd);
    }
    if !request_fds.is_empty() && !pre_opened_device_fds.is_empty() {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "request inherited fds cannot combine with broker-preopened fds".to_owned(),
        });
    }
    pre_opened_device_fds.extend(request_fds);
    crate::ops::gpu::validate_spawn_plan(&plan, pre_opened_device_fds.len()).map_err(|error| {
        LiveHandlerError::SpawnFailed {
            detail: error.to_string(),
        }
    })?;

    let memlock_guest_bytes = qemu_media_memlock_guest_bytes(&plan)?;
    let memlock_limit_bytes = memlock_guest_bytes.map(|guest_bytes| {
        guest_bytes.saturating_add(qemu_media_memlock_headroom_bytes(guest_bytes))
    });
    let activation_stdin = activation_input
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| LiveHandlerError::SpawnFailed {
            detail: format!("activation input serialization failed: {error}"),
        })?;
    if activation_stdin
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAX_ACTIVATION_RUNNER_INPUT_BYTES)
    {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "activation input exceeds bounded stdin envelope".to_owned(),
        });
    }
    if let Some(guest_bytes) = memlock_guest_bytes {
        qemu_media_preflight_memlock_budget(qemu_media_memlock_preflight_required_bytes(
            guest_bytes,
        ))
        .await?;
    }

    // Every typed fence above passed (the swtpm-dir hardening, the GPU plan
    // validation, the memlock budget): only now is the worker's trusted
    // socket directory opened to its principal. A launch the broker refuses
    // therefore leaves no ACL behind, and the directory is derived from
    // trusted identity - the pinned owning Device's Guest, or the legacy
    // plan's own trusted runtime directory - never from a launch argument.
    if let Some(grant) = device_worker_socket_grant(&plan, device_worker, runtime_root)? {
        grant
            .apply(plan.uid)
            .map_err(|detail| LiveHandlerError::SpawnFailed {
                detail: format!("device worker socket directory ACL: {detail}"),
            })?;
    }

    let isolation = crate::sys::pidfd_sys::RunnerIsolationSpec {
        capabilities: plan.capabilities.clone(),
        namespaces: plan.namespaces.clone(),
        seccomp_program,
        mount_policy: plan.mount_policy.clone(),
        cgroup_dir_fd: cgroup_fds
            .as_ref()
            .map(|fds| fds.dir_fd.try_clone())
            .transpose()
            .map_err(|err| LiveHandlerError::SpawnFailed {
                detail: format!("duplicate cgroup dir fd: {err}"),
            })?,
        cgroup_procs_fd: cgroup_fds.map(|fds| fds.procs_fd),
        // Plumb through the user-NS spec from the role profile. When
        // Some, the broker pre-creates the user NS and writes
        // uid_map/gid_map; the child runs fake-root inside with no
        // host-side capabilities. Used by virtiofsd (ADR 0021) for
        // least-privilege FS serving.
        user_namespace: plan
            .user_namespace
            .map(|spec| crate::sys::pidfd_sys::UserNamespaceSpec {
                host_uid_for_zero: spec.host_uid_for_zero,
                host_gid_for_zero: spec.host_gid_for_zero,
            }),
        // Plumb the role profile's umask through to the child. None =
        // inherit broker umask (current behaviour).
        umask: plan.umask,
        // Pre-opened render node fd (or empty vec for all other roles).
        // The sys layer dup2's it to fd 10 in the user-NS child before
        // execve.
        pre_opened_device_fds,
        memlock_limit_bytes,
        activation_stdin,
    };

    let outcome = crate::sys::pidfd_sys::clone3_spawn_runner(
        binary,
        argv,
        env,
        plan.uid,
        plan.gid,
        plan.supplementary_groups.clone(),
        isolation,
    )
    .map_err(|e| LiveHandlerError::SpawnFailed {
        detail: e.to_string(),
    })?;
    if let Some(path) = api_socket_acl_path {
        grant_daemon_api_socket_acl(path);
    }

    let start_time_ticks =
        crate::sys::pidfd_sys::read_proc_stat_start_time(outcome.pid).map_err(|e| {
            LiveHandlerError::ProcStatReadFailed {
                pid: outcome.pid,
                detail: e.to_string(),
            }
        })?;

    Ok(SpawnRunnerResult {
        pidfd: outcome.pidfd,
        extra_response_fds: Vec::new(),
        pid: outcome.pid,
        start_time_ticks,
        used_fork_fallback: outcome.used_fork_fallback,
        swtpm_dir_audit,
    })
}

/// Run the swtpm-dir first-run hardening step for the `w1-swtpm` role.
/// Returns `Ok(None)` for every other role (no-op), `Ok(Some(audit))`
/// on success, and a path-free [`LiveHandlerError::SwtpmDirHardening`]
/// on fail-closed so the dispatch layer can emit the terminal
/// `PrepareSwtpmDir` record from the carried audit.
///
/// Two placements exist:
///
/// - a **VM-scoped** placement (`d2b.slice/<vm>/...`, the legacy VM DAG) is
///   provisioned and identity-bound here, exactly as before;
/// - a **resource-backed** placement (`d2b.slice/process-<64hex>/...`,
///   `private_cgroup_placement`) carries no VM identity in the cgroup and the
///   typed Device-worker intent ships no writable paths, so the identity comes
///   from the verified bundle (`resource_backed`) and the launch is fenced
///   against it. Provisioning is NOT done here: in the v3 model the TPM state
///   Volume owns that lifecycle (`createPolicy: create-if-never-provisioned`,
///   `repairPolicy: fail-closed`, `packages/d2b-provider-volume-local`), and
///   the trusted root is shared by every Device of the host, so a per-launch
///   ownership stamp would be wrong. The hook's job for those launches is to
///   refuse any launch whose declared paths or argv do not name the trusted
///   directory.
async fn maybe_harden_swtpm_dir(
    plan: &SpawnRunnerPlan,
    resource_backed: Option<&crate::ops::swtpm_dir::ResourceBackedSwtpm>,
) -> Result<Option<crate::ops::audit_op::SwtpmDirAudit>, LiveHandlerError> {
    if plan.seccomp_policy_ref.as_deref() != Some("w1-swtpm") {
        return Ok(None);
    }
    let placement = crate::ops::swtpm_dir::parse_placement_segment(&plan.cgroup_placement.subtree);
    if let Some(crate::ops::swtpm_dir::PlacementSegment::RuntimeScope(_)) = placement {
        // Fail closed when no trusted identity resolved: an unidentifiable
        // resource-backed swtpm launch must not run against a directory no
        // trusted artifact names.
        let identity = resource_backed.ok_or_else(|| {
            hardening_refusal(plan, crate::ops::swtpm_dir::reasons::DERIVATION_FAILED)
        })?;
        crate::ops::swtpm_dir::derive_resource_backed_paths(plan, identity)
            .map_err(|reason| hardening_refusal(plan, reason))?;
        // The long-lived worker (`--tpmstate dir=...`) opens its state
        // directory for the log and pid file the moment it starts, so a launch
        // racing the Volume's layout must fail retryably here instead of
        // burning the row's restart budget on a child that dies on its first
        // write. The one-shot flush (`--unix <dir>/ctrl.sock`) only connects
        // to the worker's control socket inside that directory: it is admitted
        // and waits for the socket, so refusing it would spend the row's one
        // attempt on a race it can win. Presence is a filesystem fact, so it
        // stays out of the pure derivation above.
        if plan.argv.iter().any(|arg| arg == "--tpmstate")
            && !crate::ops::swtpm_dir::trusted_state_dir(identity).is_dir()
        {
            return Err(hardening_refusal(
                plan,
                crate::ops::swtpm_dir::reasons::STATE_DIR_NOT_PROVISIONED,
            ));
        }
        return Ok(None);
    }
    let paths = crate::ops::swtpm_dir::derive_paths(plan)
        .map_err(|reason| hardening_refusal(plan, reason))?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let cfg = crate::ops::swtpm_dir::SwtpmHardenConfig {
        expected_uid: plan.uid,
        expected_gid: plan.gid,
        marker_owner_uid: 0,
        marker_owner_gid: 0,
        now_ms,
        enforce_root_parents: true,
    };
    match crate::ops::swtpm_dir::harden(&paths, &cfg).await {
        Ok(audit) => Ok(Some(audit)),
        Err(err) => Err(LiveHandlerError::SwtpmDirHardening {
            audit: err.audit,
            reason: err.reason,
        }),
    }
}

/// The path-free fail-closed envelope for one swtpm-dir derivation refusal.
fn hardening_refusal(plan: &SpawnRunnerPlan, reason: &'static str) -> LiveHandlerError {
    LiveHandlerError::SwtpmDirHardening {
        audit: crate::ops::audit_op::SwtpmDirAudit {
            vm_id: String::new(),
            base_dir_hash: String::new(),
            result: crate::ops::audit_op::SwtpmDirResult::FailedClosed,
            mode: 0o700,
            owner_uid: plan.uid,
            owner_gid: plan.gid,
            marker_result: crate::ops::audit_op::SwtpmMarkerResult::FailedClosed,
            fail_reason: Some(reason.to_owned()),
        },
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};
    use d2b_core::bundle_resolver::{
        HostRuntime, HostRuntimeArtifact, HostRuntimeIfName, ResolvedNmUnmanagedIntent,
    };
    use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet, WritablePath};
    use d2b_host::cgroup::fake::FakeCgroupBackend;
    use std::future::Future;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn new(prefix: &str) -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};

            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let path = std::env::current_dir()
                .expect("cwd")
                .join("target")
                .join(format!("{prefix}-{unique}"));
            std::fs::create_dir_all(&path).expect("create test dir");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TestDir {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn fake_usbip_sysfs(root: &TestDir, bus_id: &str) -> PathBuf {
        let sysfs_root = root.join("sys").join("bus").join("usb").join("devices");
        let driver_root = root
            .join("sys")
            .join("bus")
            .join("usb")
            .join("drivers")
            .join("usbip-host");
        let device = sysfs_root.join(bus_id);
        std::fs::create_dir_all(&device).expect("create fake usb device");
        std::fs::create_dir_all(&driver_root).expect("create fake usbip driver");
        std::fs::write(driver_root.join("unbind"), b"").expect("driver unbind attr");
        symlink(&driver_root, device.join("driver")).expect("driver symlink");
        std::fs::write(device.join("usbip_status"), b"2\n").expect("usbip status");
        std::fs::write(device.join("usbip_sockfd"), b"").expect("usbip sockfd");
        sysfs_root
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn fake_unbound_usbip_sysfs(root: &TestDir, bus_id: &str) -> PathBuf {
        let sysfs_root = root.join("sys").join("bus").join("usb").join("devices");
        std::fs::create_dir_all(sysfs_root.join(bus_id)).expect("create fake usb device");
        sysfs_root
    }

    fn sample_host_runtime(path: PathBuf) -> HostRuntimeArtifact {
        HostRuntimeArtifact {
            path,
            runtime: HostRuntime {
                schema_version: "v2".to_owned(),
                bundle_version: 4,
                generated_at: "unix-123".to_owned(),
                nft_applied_hash: None,
                ifnames: vec![HostRuntimeIfName {
                    env: "work".to_owned(),
                    vm: Some("vm1".to_owned()),
                    user_visible_name: "vm1".to_owned(),
                    derived_ifname: "nlvvm1".to_owned(),
                    role_tag: "tap".to_owned(),
                }],
            },
        }
    }

    fn sample_nm_unmanaged_intent(root: &TestDir) -> ResolvedNmUnmanagedIntent {
        ResolvedNmUnmanagedIntent {
            intent_id: "nm-unmanaged:work".to_owned(),
            file_path: root.join("00-d2b-unmanaged.conf"),
            contents: concat!(
                "# d2b-managed begin\n",
                "# Generated by d2b-broker; do not edit by hand.\n",
                "[keyfile]\n",
                "unmanaged-devices=interface-name:d2b-*\n",
                "# marker-id=nm-unmanaged:host\n",
                "# d2b-managed end\n"
            )
            .to_owned(),
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            reload_behavior: "atomic-reload".to_owned(),
        }
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nftables_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_apply_nftables(&exec, Path::new("/usr/sbin/nft"), "table inet d2b {}")
            .await
            .unwrap();
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::ApplyNftScript { binary, script } => {
                assert!(binary.ends_with("nft"));
                assert!(script.contains("inet d2b"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_sysctl_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_apply_sysctl(&exec, "net.ipv4.ip_forward", "1").await.unwrap();
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteSysctl { key, value }
                if key == "net.ipv4.ip_forward" && value == "1"
        ));
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_update_hosts_file_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_update_hosts_file(
            &exec,
            Path::new("/etc/hosts"),
            b"127.0.0.1 localhost\n",
            0o644,
        )
        .await
        .unwrap();
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFile { mode: 0o644, .. }
        ));
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_route_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_apply_route(
            &exec,
            Path::new("/usr/sbin/ip"),
            IpRouteVerb::Add,
            "10.0.0.0/24 dev tap0",
        )
        .await
        .unwrap();
        let log = exec.take_log();
        match &log[0] {
            ReconcileOp::IpRoute {
                verb, route_spec, ..
            } => {
                assert_eq!(*verb, IpRouteVerb::Add);
                assert!(route_spec.contains("10.0.0.0/24"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    /// A failing reconcile executor surfaces ReconcileExec
    /// LiveHandlerError variant.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_propagates_executor_error() {
        struct FailExec;
        impl ReconcileExecutor for FailExec {
            fn apply_nft_script(
                &self,
                _nft: &Path,
                _script: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move {
                    Err(ReconcileExecError::NonZeroExit {
                        which: "nft".to_owned(),
                        exit_code: 1,
                        stderr: "fail".to_owned(),
                    })
                })
            }
            fn write_sysctl(
                &self,
                _: &str,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_atomic_file(
                &self,
                _: &Path,
                _: &[u8],
                _: u32,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_atomic_file_with_ownership(
                &self,
                _: &Path,
                _: &[u8],
                _: u32,
                _: u32,
                _: u32,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_path_value(
                &self,
                _: &Path,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn read_path_value(
                &self,
                _: &Path,
            ) -> Pin<Box<dyn Future<Output = Result<String, ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn ip_route(
                &self,
                _: &Path,
                _: IpRouteVerb,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn run_usbip(
                &self,
                _: &Path,
                _: crate::ops::exec_reconcile::UsbipSubcommand,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn run_ssh_keygen(
                &self,
                _: &Path,
                _: &str,
            ) -> Pin<
                Box<
                    dyn Future<Output = Result<crate::ops::exec_reconcile::GeneratedSshKey, ReconcileExecError>>
                        + Send
                        + '_,
                >,
            > {
                Box::pin(async move { unreachable!() })
            }
        }
        let err = live_apply_nftables(&FailExec, Path::new("/usr/sbin/nft"), "x")
            .await
            .unwrap_err();
        assert!(matches!(err, LiveHandlerError::ReconcileExec(_)));
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_unbind_failure_preserves_claim_for_operator_recovery() {
        struct FailUsbipUnbind;
        impl ReconcileExecutor for FailUsbipUnbind {
            fn apply_nft_script(
                &self,
                _: &Path,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_sysctl(
                &self,
                _: &str,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_atomic_file(
                &self,
                _: &Path,
                _: &[u8],
                _: u32,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_atomic_file_with_ownership(
                &self,
                _: &Path,
                _: &[u8],
                _: u32,
                _: u32,
                _: u32,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn write_path_value(
                &self,
                _: &Path,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn read_path_value(
                &self,
                _: &Path,
            ) -> Pin<Box<dyn Future<Output = Result<String, ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn ip_route(
                &self,
                _: &Path,
                _: IpRouteVerb,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move { unreachable!() })
            }
            fn run_usbip(
                &self,
                _: &Path,
                subcommand: crate::ops::exec_reconcile::UsbipSubcommand,
                _: &str,
            ) -> Pin<Box<dyn Future<Output = Result<(), ReconcileExecError>> + Send + '_>> {
                Box::pin(async move {
                    assert_eq!(
                        subcommand,
                        crate::ops::exec_reconcile::UsbipSubcommand::Unbind
                    );
                    Err(ReconcileExecError::TimedOut {
                        which: "usbip unbind".to_owned(),
                        timeout_ms: 1,
                        remediation: "manual recovery required".to_owned(),
                    })
                })
            }
            fn run_ssh_keygen(
                &self,
                _: &Path,
                _: &str,
            ) -> Pin<
                Box<
                    dyn Future<Output = Result<crate::ops::exec_reconcile::GeneratedSshKey, ReconcileExecError>>
                        + Send
                        + '_,
                >,
            > {
                Box::pin(async move { unreachable!() })
            }
        }

        let root = TestDir::new("usbip-unbind-preserves-claim");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed lock");

        let err = live_usbip_unbind(
            &FailUsbipUnbind,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
        )
        .await
        .unwrap_err();

        assert!(matches!(err, LiveHandlerError::ReconcileExec(_)));
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            Some("corp-vm".to_owned()),
            "failed or timed-out sysfs unbind must not falsely release the USBIP session claim"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_bind_same_vm_replay_skips_shellout_and_preserves_claim() {
        let root = TestDir::new("usbip-bind-same-vm-replay");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed session claim");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        let exec = FakeReconcileExecutor::new();

        live_usbip_bind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .await
        .expect("same-VM replay converges without mutation");

        assert_eq!(
            exec.take_log(),
            Vec::<ReconcileOp>::new(),
            "already-bound same-VM replay must not rerun usbip bind"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            Some("corp-vm".to_owned())
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_bind_shellout_failure_releases_claim_for_retry() {
        let root = TestDir::new("usbip-bind-failure-releases-claim");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_unbound_usbip_sysfs(&root, "1-2");
        let exec = FakeReconcileExecutor::new();
        exec.fail_run_usbip(ReconcileExecError::NonZeroExit {
            which: "usbip bind".to_owned(),
            exit_code: 1,
            stderr: "bind failed".to_owned(),
        });

        let err = live_usbip_bind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .await
        .expect_err("bind shellout failure fails closed");

        assert!(matches!(err, LiveHandlerError::ReconcileExec(_)));
        assert_eq!(
            exec.take_log(),
            vec![ReconcileOp::RunUsbip {
                binary: PathBuf::from("/run/current-system/sw/bin/usbip"),
                subcommand: crate::ops::exec_reconcile::UsbipSubcommand::Bind,
                bus_id: "1-2".to_owned(),
            }]
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            None,
            "failed first bind must release the claim so same VM can retry"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_bind_initial_driver_inspection_failure_releases_claim() {
        let root = TestDir::new("usbip-bind-initial-inspect-failure");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_unbound_usbip_sysfs(&root, "1-2");
        let exec = FakeReconcileExecutor::new();

        let err = live_usbip_bind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "invalid/busid",
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .await
        .expect_err("invalid busid fails initial driver inspection");

        assert!(matches!(err, LiveHandlerError::UsbipLock(_)));
        assert_eq!(
            exec.take_log(),
            Vec::<ReconcileOp>::new(),
            "bind shellout must not run after initial inspection failure"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            None,
            "initial inspection failure after lock acquisition must release the claim"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_bind_post_bind_driver_inspection_failure_releases_claim() {
        let root = TestDir::new("usbip-bind-post-inspect-failure");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_unbound_usbip_sysfs(&root, "1-2");
        let exec = FakeReconcileExecutor::new();
        exec.bind_creates_regular_driver(sysfs_root.clone(), "1-2".to_owned());

        let err = live_usbip_bind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .await
        .expect_err("post-bind driver inspection failure fails closed");

        assert!(matches!(err, LiveHandlerError::UsbipLock(_)));
        assert_eq!(
            exec.take_log(),
            vec![ReconcileOp::RunUsbip {
                binary: PathBuf::from("/run/current-system/sw/bin/usbip"),
                subcommand: crate::ops::exec_reconcile::UsbipSubcommand::Bind,
                bus_id: "1-2".to_owned(),
            }]
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            None,
            "post-bind inspection failure must release the claim"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_bind_non_converged_driver_releases_claim() {
        let root = TestDir::new("usbip-bind-non-converged-releases-claim");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_unbound_usbip_sysfs(&root, "1-2");
        let exec = FakeReconcileExecutor::new();

        let err = live_usbip_bind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .await
        .expect_err("bind that does not converge fails closed");

        assert!(matches!(err, LiveHandlerError::UsbipLock(_)));
        assert_eq!(
            exec.take_log(),
            vec![ReconcileOp::RunUsbip {
                binary: PathBuf::from("/run/current-system/sw/bin/usbip"),
                subcommand: crate::ops::exec_reconcile::UsbipSubcommand::Bind,
                bus_id: "1-2".to_owned(),
            }]
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            None,
            "non-converged bind must release the claim"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_unbind_aborts_stream_before_driver_unbind_and_preserves_claim_for_acl_phase() {
        let root = TestDir::new("usbip-unbind-order");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed lock");

        let exec = FakeReconcileExecutor::new();
        live_usbip_unbind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
        )
        .await
        .expect("unbind succeeds");

        assert_eq!(
            exec.take_log(),
            vec![
                ReconcileOp::ShutdownUsbipStreams {
                    sysfs_root: sysfs_root.clone(),
                    bus_id: "1-2".to_owned(),
                },
                ReconcileOp::WaitUsbipStreamFdRelease {
                    sysfs_root: sysfs_root.clone(),
                    bus_id: "1-2".to_owned(),
                },
                ReconcileOp::RunUsbip {
                    binary: PathBuf::from("/run/current-system/sw/bin/usbip"),
                    subcommand: crate::ops::exec_reconcile::UsbipSubcommand::Unbind,
                    bus_id: "1-2".to_owned(),
                },
            ]
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            Some("corp-vm".to_owned()),
            "live unbind leaves session claim until dispatch revokes ACL and releases it"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usbip_unbind_fd_release_timeout_preserves_claim_without_driver_unbind() {
        let root = TestDir::new("usbip-unbind-release-timeout");
        let lock_dir = root.join("locks");
        tokio::fs::create_dir_all(&lock_dir).await.expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed lock");

        let exec = FakeReconcileExecutor::new();
        exec.fail_wait_usbip_stream_fd_release(
            crate::ops::exec_reconcile::ReconcileExecError::TimedOut {
                which: "usbip stream fd release".to_owned(),
                timeout_ms: 1,
                remediation: "manual recovery required".to_owned(),
            },
        );

        let err = live_usbip_unbind(
            &exec,
            Path::new("/run/current-system/sw/bin/usbip"),
            &sysfs_root,
            "1-2",
            &lock_path,
            "corp-vm",
        )
        .await
        .expect_err("fd release timeout fails closed");

        assert!(matches!(err, LiveHandlerError::ReconcileExec(_)));
        assert_eq!(
            exec.take_log(),
            vec![
                ReconcileOp::ShutdownUsbipStreams {
                    sysfs_root: sysfs_root.clone(),
                    bus_id: "1-2".to_owned(),
                },
                ReconcileOp::WaitUsbipStreamFdRelease {
                    sysfs_root,
                    bus_id: "1-2".to_owned(),
                },
            ],
            "driver unbind helper must not run until stream fd release is proven"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            Some("corp-vm".to_owned()),
            "fd-release timeout must preserve the USBIP session claim"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nm_unmanaged_prefers_dbus_reload() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-dbus");
        let intent = sample_nm_unmanaged_intent(&root);
        let dbus_calls = std::cell::Cell::new(0);
        let fallback_calls = std::cell::RefCell::new(Vec::new());

        let method = live_apply_nm_unmanaged_with_reloaders(
            &exec,
            &intent,
            async || {
                dbus_calls.set(dbus_calls.get() + 1);
                Ok(())
            },
            async |args| {
                fallback_calls.borrow_mut().push(args.join(" "));
                Ok(())
            },
        )
        .await
        .expect("nm unmanaged apply succeeds");

        assert_eq!(method, Some(NmReloadMethod::Dbus));
        assert_eq!(dbus_calls.get(), 1);
        assert!(fallback_calls.borrow().is_empty());
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFileWithOwnership {
                path,
                mode: 0o644,
                contents,
                owner_uid,
                owner_gid,
            } if path == &intent.file_path
                && contents.as_slice() == intent.contents.as_bytes()
                && *owner_uid == 0
                && *owner_gid == 0
        ));
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nm_unmanaged_falls_back_to_systemctl() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-fallback");
        let intent = sample_nm_unmanaged_intent(&root);
        let fallback_calls = std::cell::RefCell::new(Vec::new());

        let method = live_apply_nm_unmanaged_with_reloaders(
            &exec,
            &intent,
            async || Err("dbus unavailable".to_owned()),
            async |args| {
                fallback_calls.borrow_mut().push(args.join(" "));
                Ok(())
            },
        )
        .await
        .expect("systemctl fallback succeeds");

        assert_eq!(method, Some(NmReloadMethod::SystemctlFallback));
        assert_eq!(
            fallback_calls.into_inner(),
            vec!["reload NetworkManager".to_owned()]
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nm_unmanaged_refuses_foreign_file_before_reload() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-foreign");
        let intent = sample_nm_unmanaged_intent(&root);
        tokio::fs::write(
            &intent.file_path,
            "# foreign NetworkManager configuration\n[keyfile]\n",
        ).await
        .unwrap();

        assert!(matches!(
            live_apply_nm_unmanaged_with_reloaders(
                &exec,
                &intent,
                async || Ok(()),
                async |_| Ok(())
            )
            .await,
            Err(LiveHandlerError::NmOwnershipConflict)
        ));
        assert!(exec.take_log().is_empty());
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nm_unmanaged_refuses_legacy_owned_file() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-legacy");
        let intent = sample_nm_unmanaged_intent(&root);
        tokio::fs::write(
            &intent.file_path,
            "# managed by d2b broker - do not edit by hand\n[keyfile]\nunmanaged-devices=interface-name:d2b-*\n",
        ).await
        .unwrap();

        assert!(matches!(
            live_apply_nm_unmanaged_with_reloaders(
                &exec,
                &intent,
                async || Ok(()),
                async |_| Ok(())
            )
            .await,
            Err(LiveHandlerError::NmOwnershipConflict)
        ));
        assert!(exec.take_log().is_empty());
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nm_unmanaged_refuses_unknown_reload_behavior_before_mutation() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-reload-refused");
        let mut intent = sample_nm_unmanaged_intent(&root);
        intent.reload_behavior = "atomic-reloadd".to_owned();

        let err = live_apply_nm_unmanaged_with_reloaders(
            &exec,
            &intent,
            async || Ok(()),
            async |_| Ok(()),
        )
        .await
        .expect_err("a typo'd reload behavior must refuse the apply");

        assert!(matches!(
            &err,
            LiveHandlerError::NmReloadBehaviorRefused(value) if value == "atomic-reloadd"
        ));
        assert!(
            exec.take_log().is_empty(),
            "the reload-behavior refusal must precede any file mutation"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_apply_nm_unmanaged_refuses_unresolvable_declared_owner() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-owner-refused");
        let mut intent = sample_nm_unmanaged_intent(&root);
        intent.owner = "rootd".to_owned();

        let err = live_apply_nm_unmanaged_with_reloaders(
            &exec,
            &intent,
            async || Ok(()),
            async |_| Ok(()),
        )
        .await
        .expect_err("an unresolvable declared owner must refuse the apply");

        assert!(matches!(
            &err,
            LiveHandlerError::NmFileOwnership(detail) if detail.contains("rootd")
        ));
        assert!(
            exec.take_log().is_empty(),
            "the ownership refusal must precede any file mutation"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn update_host_runtime_nft_hash_rewrites_runtime_json() {
        let root = TestDir::new("host-runtime-nft-hash");
        let runtime = sample_host_runtime(root.join("host-runtime.json"));
        tokio::fs::create_dir_all(runtime.path.parent().expect("runtime parent")).await.unwrap();
        tokio::fs::write(
            &runtime.path,
            runtime.render_json().expect("render runtime"),
        ).await
        .unwrap();

        update_host_runtime_nft_hash(&runtime.path, Some("0123456789abcdef"))
            .await
            .expect("update host runtime hash");

        assert_eq!(
            read_host_runtime_nft_hash(&runtime.path)
                .await
                .expect("read host runtime hash"),
            Some("0123456789abcdef".to_owned())
        );
        let updated = tokio::fs::read_to_string(&runtime.path).await.expect("read updated runtime");
        assert!(updated.contains("\"nftAppliedHash\": \"0123456789abcdef\""));
        assert!(updated.contains("\"derivedIfname\": \"nlvvm1\""));
    }

    fn test_namespaces() -> NamespaceSet {
        NamespaceSet {
            mount: false,
            pid: false,
            net: false,
            ipc: false,
            uts: false,
            user: false,
        }
    }

    fn test_mount_policy() -> MountPolicy {
        MountPolicy {
            read_only_paths: vec![],
            writable_paths: vec![WritablePath {
                path: "/var/lib/d2b/vms/test".to_owned(),
                purpose: "test".to_owned(),
            }],
            nix_store_read_only: false,
            hide_device_nodes_by_default: false,
            device_binds: Vec::new(),
            bind_mounts: Vec::new(),
        }
    }

    fn test_cgroup_placement() -> CgroupPlacement {
        CgroupPlacement {
            subtree: String::new(),
            controllers: vec![],
            delegated: false,
        }
    }

    /// A resource-backed (`private_cgroup_placement`) swtpm plan whose argv
    /// names `state_dir` and `runtime_dir`.
    fn resource_backed_swtpm_plan(state_dir: &Path, runtime_dir: &Path) -> SpawnRunnerPlan {
        let mut plan = test_spawn_plan_with_argv(
            vec![
                "swtpm".to_owned(),
                "socket".to_owned(),
                "--tpm2".to_owned(),
                "--tpmstate".to_owned(),
                format!("dir={}", state_dir.display()),
                "--ctrl".to_owned(),
                format!(
                    "type=unixio,path={},mode=0660,uid=61000,gid=61000",
                    state_dir.join("ctrl.sock").display()
                ),
                "--server".to_owned(),
                format!(
                    "type=unixio,path={},mode=0660,uid=61000,gid=61000",
                    runtime_dir.join("tpm.sock").display()
                ),
            ],
            "w1-swtpm",
        );
        plan.cgroup_placement.subtree = format!(
            "d2b.slice/{}/swtpm",
            "process-".to_owned() + &"b".repeat(64)
        );
        plan
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn resource_backed_swtpm_launch_is_fenced_against_the_trusted_identity() {
        use crate::ops::swtpm_dir::{ResourceBackedSwtpm, reasons};
        // The state Volume's layout creates the directory; the fence reads
        // that fact from the filesystem, so the fixture owns a real one.
        let scratch = tempfile::tempdir().expect("scratch");
        let identity = ResourceBackedSwtpm {
            guest: "acceptance-guest".to_owned(),
            state_root: scratch.path().join("tpm-state"),
            state_volume: Some("device-6f9619ff8b864d01b42d00cf4fc964ff-tpm-state".to_owned()),
        };
        let state_dir = identity
            .state_root
            .join(identity.state_volume.as_deref().expect("volume name"));
        let runtime_dir = PathBuf::from("/run/d2b/vms/acceptance-guest");

        // A launch naming exactly the trusted directories passes the fence
        // once its state Volume directory exists: in v3 that Volume's
        // `create-if-never-provisioned`/`fail-closed` lifecycle owns the
        // directory, and the spawn hook fences identity plus the presence the
        // lifecycle leaves behind.
        tokio::fs::create_dir_all(&state_dir).await.expect("state volume directory");
        let plan = resource_backed_swtpm_plan(&state_dir, &runtime_dir);
        assert!(
            maybe_harden_swtpm_dir(&plan, Some(&identity))
                .await
                .expect("trusted launch passes")
                .is_none()
        );

        // A launch racing the Volume's layout (the directory is not there yet)
        // fails closed retryably instead of starting a child that dies on its
        // first log write and burns the row's restart budget.
        std::fs::remove_dir(&state_dir).expect("drop the state volume directory");
        let refusal = maybe_harden_swtpm_dir(&plan, Some(&identity))
            .await
            .expect_err("unprovisioned refuses");
        assert!(matches!(
            refusal,
            LiveHandlerError::SwtpmDirHardening { reason, .. }
                if reason == reasons::STATE_DIR_NOT_PROVISIONED
        ));
        tokio::fs::create_dir_all(&state_dir).await.expect("restore the state volume directory");

        // The one-shot flush opens the worker's control socket inside that
        // same directory instead of writing into it, so it is admitted while
        // the layout is still absent: it waits for the socket, and refusing it
        // would spend the row's single launch attempt on a race it can win.
        std::fs::remove_dir(&state_dir).expect("drop the state volume directory");
        let mut flush = test_spawn_plan_with_argv(
            vec![
                "swtpm-ioctl".to_owned(),
                "flush".to_owned(),
                "--unix".to_owned(),
                state_dir.join("ctrl.sock").display().to_string(),
            ],
            "w1-swtpm",
        );
        flush.cgroup_placement.subtree =
            format!("d2b.slice/{}/swtpm", "process-".to_owned() + &"c".repeat(64));
        assert!(
            maybe_harden_swtpm_dir(&flush, Some(&identity))
                .await
                .expect("one-shot flush passes")
                .is_none()
        );
        tokio::fs::create_dir_all(&state_dir).await.expect("restore the state volume directory");

        // Without a trusted identity the launch fails closed by derivation.
        let refusal = maybe_harden_swtpm_dir(&plan, None)
            .await
            .expect_err("missing identity refuses");
        assert!(matches!(
            refusal,
            LiveHandlerError::SwtpmDirHardening { reason, .. }
                if reason == reasons::DERIVATION_FAILED
        ));

        // A launch opening another directory fails closed with the named
        // identity-mismatch reason and a matching path-free audit.
        let foreign = resource_backed_swtpm_plan(
            Path::new("/var/lib/d2b/tpm-state/device-foreign-tpm-state"),
            &runtime_dir,
        );
        let refusal = maybe_harden_swtpm_dir(&foreign, Some(&identity))
            .await
            .expect_err("mismatch refuses");
        assert!(matches!(
            refusal,
            LiveHandlerError::SwtpmDirHardening { reason, audit }
                if reason == reasons::IDENTITY_MISMATCH
                    && audit.fail_reason.as_deref() == Some(reasons::IDENTITY_MISMATCH)
        ));

        // Roles other than w1-swtpm are a no-op.
        let mut other = plan.clone();
        other.seccomp_policy_ref = Some("w1-gpu".to_owned());
        assert!(
            maybe_harden_swtpm_dir(&other, Some(&identity))
                .await
                .expect("other roles no-op")
                .is_none()
        );
    }

    #[test]
    fn ensure_runner_cgroup_leaf_uses_delegated_parent_slice() {
        let backend = FakeCgroupBackend::new(1000);
        let root = Path::new("/sys/fs/cgroup");
        let parent = Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE);
        backend.seed_unified(root);
        d2b_host::cgroup::CgroupBackend::mkdir(&backend, parent)
            .expect("seed delegated parent slice");
        let placement = CgroupPlacement {
            subtree: "d2b.slice/personal-dev/virtiofsd-ro-store".to_owned(),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: true,
        };

        let leaf = ensure_runner_cgroup_leaf(&backend, &placement, root, parent, 1000, 1000)
            .expect("prepare delegated cgroup leaf")
            .expect("leaf path");

        assert_eq!(
            leaf,
            Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE)
                .join("personal-dev")
                .join("virtiofsd-ro-store")
        );
        assert!(backend.directory_exists(&leaf));
        // DEFAULT_DELEGATED_PARENT_SLICE is the top-level
        // `/sys/fs/cgroup/d2b.slice` (systemd top-level slice naming
        // convention). The leaf path lives under that, so the slice MUST
        // exist for the leaf to exist.
        assert!(backend.directory_exists(Path::new("/sys/fs/cgroup/d2b.slice")));
        assert_eq!(
            backend
                .file_contents(&root.join("cgroup.subtree_control"))
                .as_deref(),
            Some("")
        );
    }

    #[test]
    fn ensure_runner_cgroup_leaf_keeps_same_guest_names_zone_distinct() {
        let backend = FakeCgroupBackend::new(1000);
        let root = Path::new("/sys/fs/cgroup");
        let parent = Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE);
        backend.seed_unified(root);
        d2b_host::cgroup::CgroupBackend::mkdir(&backend, parent)
            .expect("seed delegated parent slice");

        let work = CgroupPlacement {
            subtree: "d2b.slice/work/desktop/cloud-hypervisor".to_owned(),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: false,
        };
        let personal = CgroupPlacement {
            subtree: "d2b.slice/personal/desktop/cloud-hypervisor".to_owned(),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: false,
        };

        let work_leaf = ensure_runner_cgroup_leaf(&backend, &work, root, parent, 1000, 1000)
            .expect("create work cgroup")
            .expect("work leaf");
        let personal_leaf =
            ensure_runner_cgroup_leaf(&backend, &personal, root, parent, 1000, 1000)
                .expect("create personal cgroup")
                .expect("personal leaf");

        assert_eq!(
            work_leaf,
            parent.join("work").join("desktop").join("cloud-hypervisor")
        );
        assert_eq!(
            personal_leaf,
            parent
                .join("personal")
                .join("desktop")
                .join("cloud-hypervisor")
        );
        assert_ne!(work_leaf, personal_leaf);
        assert!(backend.directory_exists(&work_leaf));
        assert!(backend.directory_exists(&personal_leaf));
    }

    /// live_spawn_runner preflight failure surfaces SpawnPreflight.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_spawn_runner_propagates_preflight_error() {
        let plan = SpawnRunnerPlanInput {
            binary_path: PathBuf::from("not-absolute"),
            argv: vec!["x".to_owned()],
            uid: 1,
            gid: 1,
            supplementary_groups: vec![],
            env: vec![],
            capabilities: vec![],
            namespaces: test_namespaces(),
            seccomp_policy_ref: None,
            mount_policy: test_mount_policy(),
            cgroup_placement: test_cgroup_placement(),
            root_carve_out: false,
            skip_binary_exists_check: true,
            user_namespace: None,
            umask: None,
        };
        let err = live_spawn_runner(
            &plan,
            Vec::new(),
            Vec::new(),
            None,
            Path::new("/var/lib/d2b"),
            None,
            &crate::ops::device_worker::DeviceWorkerLaunch::default(),
            Path::new("/run/d2b"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LiveHandlerError::SpawnPreflight(_)));
    }

    fn test_spawn_plan_with_argv(argv: Vec<String>, seccomp_policy_ref: &str) -> SpawnRunnerPlan {
        SpawnRunnerPlan {
            binary_path: PathBuf::from("/bin/socat"),
            argv,
            uid: 1000,
            gid: 1000,
            supplementary_groups: vec![],
            env: vec![],
            capabilities: vec![],
            namespaces: test_namespaces(),
            seccomp_policy_ref: Some(seccomp_policy_ref.to_owned()),
            mount_policy: test_mount_policy(),
            cgroup_placement: test_cgroup_placement(),
            user_namespace: None,
            umask: None,
        }
    }

    fn hardened_qemu_media_plan() -> SpawnRunnerPlan {
        let mut plan =
            test_spawn_plan_with_argv(vec!["d2b-qemu-media@media".to_owned()], "w1-qemu-media");
        plan.namespaces = NamespaceSet {
            mount: true,
            pid: true,
            net: false,
            ipc: true,
            uts: false,
            user: false,
        };
        plan.mount_policy = MountPolicy {
            read_only_paths: vec!["/".to_owned()],
            writable_paths: vec![
                WritablePath {
                    path: "/run/d2b/vms/media".to_owned(),
                    purpose: "QMP socket".to_owned(),
                },
                WritablePath {
                    path: "/var/lib/d2b/vms/media".to_owned(),
                    purpose: "qemu-media state".to_owned(),
                },
            ],
            nix_store_read_only: true,
            hide_device_nodes_by_default: true,
            device_binds: vec!["/dev/kvm".to_owned()],
            bind_mounts: vec![],
        };
        plan
    }

    #[test]
    fn qemu_media_runner_hardening_accepts_fd_backed_profile() {
        let plan = hardened_qemu_media_plan();

        validate_qemu_media_runner_hardening(&plan)
            .expect("hardened qemu-media fd-backed profile should pass");
    }

    #[test]
    fn qemu_media_memlock_limit_is_bounded_to_guest_ram_plus_headroom() {
        let mut plan = hardened_qemu_media_plan();
        plan.argv = vec![
            "d2b-qemu-media@media".to_owned(),
            "-object".to_owned(),
            "memory-backend-ram,id=nlram,size=4096M,dump=off,merge=off,prealloc=on".to_owned(),
            "-overcommit".to_owned(),
            "mem-lock=on".to_owned(),
        ];

        assert_eq!(
            qemu_media_memlock_limit_bytes(&plan).expect("memlock parse"),
            Some(6 * 1024 * 1024 * 1024)
        );

        plan.argv = vec![
            "d2b-qemu-media@media".to_owned(),
            "-object".to_owned(),
            "memory-backend-ram,id=nlram,size=4096M,dump=off,merge=off".to_owned(),
        ];
        assert_eq!(qemu_media_memlock_limit_bytes(&plan).unwrap(), None);
    }

    #[test]
    fn qemu_media_memlock_headroom_scales_for_large_guests() {
        let guest = 10 * 1024_u64 * 1024 * 1024;

        assert_eq!(
            qemu_media_memlock_headroom_bytes(guest),
            guest / QEMU_MEDIA_MEMLOCK_HEADROOM_RATIO_DIVISOR
        );
        assert_eq!(
            qemu_media_memlock_limit_bytes(&SpawnRunnerPlan {
                argv: vec![
                    "d2b-qemu-media@media".to_owned(),
                    "-object".to_owned(),
                    "memory-backend-ram,id=nlram,size=10240M,dump=off,merge=off,prealloc=on"
                        .to_owned(),
                    "-overcommit".to_owned(),
                    "mem-lock=on".to_owned(),
                ],
                ..hardened_qemu_media_plan()
            })
            .expect("memlock parse"),
            Some(guest + (guest / QEMU_MEDIA_MEMLOCK_HEADROOM_RATIO_DIVISOR))
        );
        assert_eq!(
            qemu_media_memlock_preflight_required_bytes(guest),
            guest + QEMU_MEDIA_MEMLOCK_PREFLIGHT_OVERHEAD_BYTES
        );
    }

    #[test]
    fn qemu_media_memlock_preflight_parses_mem_available() {
        assert_eq!(
            parse_meminfo_available_bytes("MemTotal: 1 kB\nMemAvailable: 12345 kB\n"),
            Some(12_641_280)
        );
        assert_eq!(
            parse_meminfo_available_bytes("MemAvailable: 12345 B\n"),
            None
        );
        assert_eq!(parse_meminfo_available_bytes("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn qemu_media_memlock_preflight_shortfall_is_pure_and_actionable() {
        assert_eq!(
            qemu_media_memlock_budget_shortfall(4096, 4095),
            Some(QemuMediaMemlockShortfall {
                required_bytes: 4096,
                available_bytes: 4095,
            })
        );
        assert_eq!(qemu_media_memlock_budget_shortfall(4096, 4096), None);
        assert_eq!(qemu_media_memlock_budget_shortfall(4096, 8192), None);
    }

    #[test]
    fn qemu_media_runner_hardening_rejects_forbidden_caps_and_devices() {
        let mut cap_plan = hardened_qemu_media_plan();
        cap_plan.capabilities = vec!["CAP_SYS_ADMIN".to_owned()];
        let cap_err = validate_qemu_media_runner_hardening(&cap_plan).unwrap_err();
        assert!(cap_err.to_string().contains("empty capabilities"));

        let mut missing_kvm_plan = hardened_qemu_media_plan();
        missing_kvm_plan.mount_policy.device_binds.clear();
        let missing_kvm_err = validate_qemu_media_runner_hardening(&missing_kvm_plan).unwrap_err();
        assert!(missing_kvm_err.to_string().contains("exactly /dev/kvm"));

        let mut vhost_plan = hardened_qemu_media_plan();
        vhost_plan
            .mount_policy
            .device_binds
            .push("/dev/vhost-net".to_owned());
        let vhost_err = validate_qemu_media_runner_hardening(&vhost_plan).unwrap_err();
        assert!(vhost_err.to_string().contains("exactly /dev/kvm"));

        let mut media_bind_plan = hardened_qemu_media_plan();
        media_bind_plan
            .mount_policy
            .bind_mounts
            .push(d2b_core::sandbox_profile::BindMount {
                src: "/var/lib/d2b/media/install.iso".to_owned(),
                dst: "/media/install.iso".to_owned(),
            });
        let media_err = validate_qemu_media_runner_hardening(&media_bind_plan).unwrap_err();
        assert!(media_err.to_string().contains("inherited/pre-opened fds"));
    }

    #[test]
    fn qemu_media_runner_hardening_rejects_missing_sandbox_contract() {
        let mut no_seccomp = hardened_qemu_media_plan();
        no_seccomp.seccomp_policy_ref = None;
        let seccomp_err = validate_qemu_media_runner_hardening(&no_seccomp).unwrap_err();
        assert!(seccomp_err.to_string().contains("seccompPolicyRef"));

        let mut no_pid = hardened_qemu_media_plan();
        no_pid.namespaces.pid = false;
        let namespace_err = validate_qemu_media_runner_hardening(&no_pid).unwrap_err();
        assert!(
            namespace_err
                .to_string()
                .contains("mount and pid namespaces")
        );

        let mut no_readonly_root = hardened_qemu_media_plan();
        no_readonly_root.mount_policy.read_only_paths.clear();
        let readonly_err = validate_qemu_media_runner_hardening(&no_readonly_root).unwrap_err();
        assert!(readonly_err.to_string().contains("read-only root"));
    }

    #[test]
    fn parses_cloud_hypervisor_vsock_socket_for_ch_runner() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--api-socket".to_owned(),
                "/var/lib/d2b/vms/corp-vm/api.sock".to_owned(),
                "--vsock".to_owned(),
                "cid=42,socket=/var/lib/d2b/vms/corp-vm/vsock.sock".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(
            cloud_hypervisor_component_session_socket_arg(&plan),
            Some(PathBuf::from("/var/lib/d2b/vms/corp-vm/vsock.sock"))
        );
    }

    #[test]
    fn cloud_hypervisor_vsock_socket_gated_on_ch_runner_policy() {
        // Same argv shape but a non-CH seccomp policy: the daemon-vsock
        // ACL must never attach to a non-cloud-hypervisor runner.
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--vsock".to_owned(),
                "cid=42,socket=/var/lib/d2b/vms/corp-vm/vsock.sock".to_owned(),
            ],
            "w1-vsock-relay",
        );

        assert_eq!(cloud_hypervisor_component_session_socket_arg(&plan), None);
    }

    #[test]
    fn cloud_hypervisor_vsock_socket_absent_without_vsock_arg() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--api-socket".to_owned(),
                "/var/lib/d2b/vms/corp-vm/api.sock".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(cloud_hypervisor_component_session_socket_arg(&plan), None);
    }

    #[test]
    fn component_session_acl_diff_hash_is_path_free_and_stable() {
        let h1 = component_session_acl_diff_hash("grant", "vsock-socket", 0x10, 0x20);
        let h2 = component_session_acl_diff_hash("grant", "vsock-socket", 0x10, 0x20);
        let h3 = component_session_acl_diff_hash("revoke", "vsock-socket", 0x10, 0x20);
        assert_eq!(h1, h2, "hash must be deterministic for identical inputs");
        assert_ne!(h1, h3, "op must affect the hash");
        assert!(h1.starts_with("sha256:"));
        assert_eq!(h1.len(), "sha256:".len() + 64);
        // The digest must not embed any raw path component.
        assert!(!h1.contains('/'));
    }

    #[test]
    fn component_session_acl_grant_not_ready_for_absent_socket() {
        // Hermetic: before cloud-hypervisor creates the vsock socket,
        // the socket grant must report not-ready (Ok(false)) so the
        // caller retries - never erroring and never touching a foreign
        // inode.
        let dir = TestDir::new("gc-vsock-acl");
        let socket = dir.join("vsock.sock");
        assert!(!socket.exists());
        assert_eq!(
            grant_component_session_socket_acl_once(&socket),
            Ok(false),
            "absent socket must report not-ready",
        );
        // Revoke against an absent socket is a no-op (short-circuits on
        // the absent inode before any setfacl). The traversal-skip
        // behaviour (no setfacl on world-traversable ancestors) is
        // covered hermetically by `dir_traverse_classification_world_x_vs_private`
        // without invoking the host setfacl binary on real ancestors -
        // which a TestDir rooted under a non-world-x CI path (e.g.
        // `/home/runner`, mode 0750) would otherwise trigger.
        revoke_component_session_vsock_acl(&socket).expect("revoke of absent socket is a no-op");
    }

    #[test]
    fn setfacl_failure_component_session_detail_is_path_free() {
        // A path-bearing legacy detail must never leak through the
        // component-session formatter: it carries only closed-set class
        // tokens (op/target-class/stage), the daemon principal, the
        // io::ErrorKind, and the numeric errno.
        let failure = SetfaclFailure {
            stage: SetfaclStage::Apply,
            errno_kind: std::io::ErrorKind::PermissionDenied,
            raw_os_error: Some(13),
            legacy_detail: "setfacl -m u:d2bd:rw on /var/lib/d2b/vms/corp-vm/vsock.sock: denied"
                .to_owned(),
        };
        let detail = failure.component_session_detail("grant", "vsock-socket");
        assert!(
            !detail.contains('/'),
            "detail must not embed any path: {detail}"
        );
        assert!(
            !detail.contains("vsock.sock"),
            "detail leaked socket name: {detail}"
        );
        assert!(!detail.contains(":rw"), "detail leaked acl spec: {detail}");
        assert!(
            detail.contains("vsock-socket"),
            "missing target class: {detail}"
        );
        assert!(detail.contains("stage=apply"), "missing stage: {detail}");
        assert!(detail.contains("errno=13"), "missing errno: {detail}");
        assert!(
            detail.contains(COMPONENT_SESSION_DAEMON_PRINCIPAL),
            "missing daemon principal: {detail}"
        );
        // No-errno case renders a stable token.
        let mismatch = SetfaclFailure {
            stage: SetfaclStage::TypeMismatch,
            errno_kind: std::io::ErrorKind::InvalidInput,
            raw_os_error: None,
            legacy_detail: "refusing setfacl on /secret/path".to_owned(),
        };
        let detail = mismatch.component_session_detail("revoke", "state-dir");
        assert!(
            !detail.contains('/'),
            "detail must not embed any path: {detail}"
        );
        assert!(
            detail.contains("errno=none"),
            "missing errno token: {detail}"
        );
        assert!(
            detail.contains("stage=type-mismatch"),
            "missing stage: {detail}"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn dir_traverse_classification_world_x_vs_private() {
        use std::os::unix::fs::PermissionsExt as _;
        // World-traversable dir (0o755) needs no daemon --x grant; a
        // private dir (0o700) does. A regular file is not part of the
        // traversal grant. An absent path resolves to None.
        let dir = TestDir::new("gc-traverse");
        let world_x = dir.join("world");
        std::fs::create_dir(&world_x).expect("mkdir world");
        std::fs::set_permissions(&world_x, std::fs::Permissions::from_mode(0o755))
            .expect("chmod 0755");
        let private = dir.join("private");
        std::fs::create_dir(&private).expect("mkdir private");
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))
            .expect("chmod 0700");
        let file = dir.join("file");
        std::fs::write(&file, b"x").expect("write file");

        assert_eq!(dir_needs_traverse_grant(&world_x), Ok(Some(false)));
        assert_eq!(dir_needs_traverse_grant(&private), Ok(Some(true)));
        assert_eq!(dir_needs_traverse_grant(&file), Ok(Some(false)));
        assert_eq!(dir_needs_traverse_grant(&dir.join("absent")), Ok(None));
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn runner_tree_acl_targets_stop_at_owned_root_and_skip_world_x_ancestors() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("runner-state-acl");
        let owned_root = root.join("d2b");
        let world_x = owned_root.join("zones");
        let private_zone = world_x.join("work");
        let guest_parent = private_zone.join("guests");
        let state_dir = guest_parent.join("desktop");
        std::fs::create_dir_all(&state_dir).expect("create state directory");
        for path in [&owned_root, &private_zone, &guest_parent, &state_dir] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .expect("make ancestor private");
        }
        std::fs::set_permissions(&world_x, std::fs::Permissions::from_mode(0o755))
            .expect("make ancestor world traversable");

        let targets = runner_tree_acl_targets(&state_dir, &owned_root, 4242)
            .expect("derive bounded runner ACL targets");
        assert_eq!(
            targets,
            vec![
                (owned_root.clone(), "u:4242:--x".to_owned()),
                (private_zone.clone(), "u:4242:--x".to_owned()),
                (guest_parent.clone(), "u:4242:--x".to_owned()),
                (state_dir.clone(), "u:4242:rwx".to_owned()),
            ]
        );
        assert!(
            targets.iter().all(|(path, _)| path != Path::new("/")),
            "runner ACL planning must never include the filesystem root"
        );
        assert!(
            targets.iter().all(|(path, _)| path != &world_x),
            "world-traversable ancestors need no named-user ACL"
        );
    }

    #[test]
    fn runner_tree_acl_targets_reject_paths_outside_owned_root() {
        let root = TestDir::new("runner-state-acl-boundary");
        let owned_root = root.join("d2b");
        let outside = root.join("other").join("desktop");
        assert!(
            runner_tree_acl_targets(&outside, &owned_root, 4242).is_err(),
            "runner ACL planning must fail closed outside the configured state root"
        );
    }

    /// The serving worker's launch ticket must name both paths. A
    /// serving-posture launch is refused rather than spawned with a
    /// partially-open tree, and a path that is not absolute/anchored is
    /// refused because the socket directory is fenced with `starts_with`.
    #[test]
    fn serving_worker_launch_paths_require_both_ticket_flags() {
        let paths = serving_worker_launch_paths(&[
            "virtiofsd".to_owned(),
            "--socket-path=/run/d2b/vms/guest/vol-abcd.vfd.sock".to_owned(),
            "--shared-dir=/var/lib/d2b/store-view/live/system".to_owned(),
            "--readonly".to_owned(),
        ])
        .expect("the frozen ticket shape parses");
        assert_eq!(paths.socket_dir, Path::new("/run/d2b/vms/guest"));
        assert_eq!(
            paths.shared_dir,
            Path::new("/var/lib/d2b/store-view/live/system")
        );
        assert!(paths.read_only);
        assert!(
            serving_worker_launch_paths(&[
                "virtiofsd".to_owned(),
                "--socket-path=/run/d2b/vms/guest/vol-abcd.vfd.sock".to_owned(),
            ])
            .is_err(),
            "--shared-dir is mandatory"
        );
        for argv in [
            vec!["virtiofsd".to_owned(), "--shared-dir=/srv/view".to_owned()],
            vec![
                "virtiofsd".to_owned(),
                "--socket-path=/run/d2b/../etc/vol.vfd.sock".to_owned(),
                "--shared-dir=/srv/view".to_owned(),
            ],
            vec![
                "virtiofsd".to_owned(),
                "--socket-path=relative/vol.vfd.sock".to_owned(),
                "--shared-dir=/srv/view".to_owned(),
            ],
            vec![
                "virtiofsd".to_owned(),
                "--socket-path=/vol.vfd.sock".to_owned(),
                "--shared-dir=relative/view".to_owned(),
            ],
        ] {
            assert!(
                serving_worker_launch_paths(&argv).is_err(),
                "must refuse a launch without two anchored absolute ticket paths: {argv:?}"
            );
        }
    }

    /// The private socket directory is broker-runtime state: a ticket that
    /// points it outside the broker's own runtime directory (or at the
    /// runtime directory itself) is refused before any ACL is applied.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serving_worker_socket_directory_must_live_inside_the_broker_runtime_root() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("serving-worker-socket-fence");
        let runtime_root = root.join("run");
        std::fs::create_dir_all(&runtime_root).expect("create runtime root");
        std::fs::set_permissions(&runtime_root, std::fs::Permissions::from_mode(0o700))
            .expect("chmod runtime root");
        let shared = root.join("view");
        std::fs::create_dir_all(&shared).expect("create view root");

        for socket in [
            root.join("elsewhere").join("vol.vfd.sock"),
            runtime_root
                .clone()
                .join("..")
                .join("elsewhere")
                .join("vol.vfd.sock"),
        ] {
            let argv = vec![
                "virtiofsd".to_owned(),
                format!("--socket-path={}", socket.display()),
                format!("--shared-dir={}", shared.display()),
            ];
            assert!(
                grant_serving_worker_launch_acls(&argv, 4242, &runtime_root).is_err(),
                "a socket directory outside the runtime root must be refused: {}",
                socket.display()
            );
        }
        // The refusal names the fence it crossed.
        let outside = root.join("elsewhere").join("vol.vfd.sock");
        let argv = vec![
            "virtiofsd".to_owned(),
            format!("--socket-path={}", outside.display()),
            format!("--shared-dir={}", shared.display()),
        ];
        let error = grant_serving_worker_launch_acls(&argv, 4242, &runtime_root)
            .expect_err("a socket outside the runtime root must be refused");
        assert!(
            error.to_string().contains("outside the broker runtime"),
            "unexpected refusal: {error}"
        );
        // The runtime root itself is not a worker-writable tree either.
        let argv = vec![
            "virtiofsd".to_owned(),
            format!(
                "--socket-path={}",
                runtime_root.join("vol.vfd.sock").display()
            ),
            format!("--shared-dir={}", shared.display()),
        ];
        let error = grant_serving_worker_launch_acls(&argv, 4242, &runtime_root)
            .expect_err("the runtime root itself must not be opened");
        assert!(
            error.to_string().contains("outside the broker runtime"),
            "unexpected refusal: {error}"
        );
    }

    /// An unusable view root refuses the launch before anything is mutated:
    /// neither the socket tree nor the root gets an ACL entry.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serving_worker_refuses_an_unusable_view_root_before_mutating() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("serving-worker-view-root");
        let runtime_root = root.join("run");
        let socket_dir = runtime_root.join("vms").join("guest");
        std::fs::create_dir_all(&socket_dir).expect("create socket dir");
        std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))
            .expect("chmod socket dir");
        let absent = root.join("absent-view");

        let argv = |shared: &Path| {
            vec![
                "virtiofsd".to_owned(),
                format!(
                    "--socket-path={}",
                    socket_dir.join("vol.vfd.sock").display()
                ),
                format!("--shared-dir={}", shared.display()),
            ]
        };
        let error = grant_serving_worker_launch_acls(&argv(&absent), 4242, &runtime_root)
            .expect_err("an absent view root must be refused");
        assert!(error.to_string().contains("does not exist"), "{error}");

        // A regular file is not a servable root either.
        let file = root.join("view-is-a-file");
        std::fs::write(&file, b"x").expect("write file");
        let error = grant_serving_worker_launch_acls(&argv(&file), 4242, &runtime_root)
            .expect_err("a non-directory view root must be refused");
        assert!(error.to_string().contains("not a directory"), "{error}");

        // A symlinked view root is refused by the NOFOLLOW open.
        let real = root.join("real-view");
        std::fs::create_dir_all(&real).expect("create real view");
        let link = root.join("linked-view");
        std::os::unix::fs::symlink(&real, &link).expect("symlink view root");
        grant_serving_worker_launch_acls(&argv(&link), 4242, &runtime_root)
            .expect_err("a symlinked view root must be refused");

        // Nothing was opened on the worker's behalf.
        let socket_fd = crate::sys::path_safe::open_dir_path_safe(&socket_dir).expect("open dir");
        assert_eq!(
            crate::sys::path_safe::fd_extended_acl_present(socket_fd.as_fd())
                .expect("inspect socket dir ACL"),
            (false, false),
            "a refused launch must not leave the socket tree opened"
        );
    }

    /// The served view root is opened with read/traverse (read-write for a
    /// read-write attachment) to the runner principal, plus search on the
    /// non-world-searchable chain above it - and never above the first
    /// ancestor every principal can already search.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn served_view_root_acl_targets_cover_the_root_and_stop_at_world_search() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("served-view-acl");
        let world_x = root.join("world");
        let private = world_x.join("private");
        let served = private.join("served");
        std::fs::create_dir_all(&served).expect("create served root");
        for path in [&world_x, &private, &served] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .expect("make chain private");
        }
        std::fs::set_permissions(&world_x, std::fs::Permissions::from_mode(0o755))
            .expect("make outer ancestor world traversable");

        assert_eq!(
            served_view_root_acl_targets(&served, 4242, true).expect("read-only targets"),
            vec![
                (private.clone(), "u:4242:--x".to_owned()),
                (served.clone(), "u:4242:r-x".to_owned()),
            ]
        );
        assert_eq!(
            served_view_root_acl_targets(&served, 4242, false).expect("read-write targets"),
            vec![
                (private.clone(), "u:4242:--x".to_owned()),
                (served.clone(), "u:4242:rwx".to_owned()),
            ]
        );
        // The `ancestors()` walk of `/` has nowhere to stop and would open the
        // filesystem root to the runner principal: a `/`-shaped shared dir is
        // refused by name, before any ACL target is produced.
        let root_error = served_view_root_acl_targets(Path::new("/"), 4242, true)
            .expect_err("the filesystem root must never be a served view root");
        assert!(
            root_error.contains("must not be the filesystem root"),
            "{root_error}"
        );
        assert!(
            served_view_root_acl_targets(Path::new("relative/view"), 4242, true).is_err(),
            "a relative view root must be refused"
        );
    }

    /// The daemon-provisioned socket directory and served view root both end
    /// up with a per-runner access ACL for the runner principal, so the
    /// worker reaches them as itself rather than as the daemon.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serving_worker_launch_acls_open_the_ticket_paths_to_the_principal() {
        use std::os::unix::fs::PermissionsExt as _;

        if ![
            "/run/current-system/sw/bin/setfacl",
            "/usr/bin/setfacl",
            "/bin/setfacl",
        ]
        .iter()
        .any(|candidate| Path::new(candidate).exists())
        {
            eprintln!("skipping serving-worker ACL application test: no setfacl binary");
            return;
        }

        let root = TestDir::new("serving-worker-acl-apply");
        let runtime_root = root.join("run");
        let socket_dir = runtime_root.join("vms").join("guest");
        std::fs::create_dir_all(&socket_dir).expect("create socket dir");
        std::fs::set_permissions(&runtime_root, std::fs::Permissions::from_mode(0o700))
            .expect("chmod runtime root");
        std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))
            .expect("chmod socket dir");
        let shared = root.join("view");
        std::fs::create_dir_all(&shared).expect("create view root");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o750))
            .expect("chmod view root");

        let argv = vec![
            "virtiofsd".to_owned(),
            format!(
                "--socket-path={}",
                socket_dir.join("vol-abcd.vfd.sock").display()
            ),
            format!("--shared-dir={}", shared.display()),
        ];
        // The principal the resolver mints for the binding template.
        let uid = 50_123;
        grant_serving_worker_launch_acls(&argv, uid, &runtime_root).expect("grant ACLs");

        for (path, label) in [(&socket_dir, "socket dir"), (&shared, "view root")] {
            let fd = crate::sys::path_safe::open_dir_path_safe(path).expect("open dir");
            assert_eq!(
                crate::sys::path_safe::fd_extended_acl_present(fd.as_fd()).expect("inspect ACL"),
                (true, false),
                "{label} must carry an access ACL entry for the runner principal"
            );
        }
        // The mask carries the granted permissions - without it a named-user
        // entry on a `0700` directory would be masked away and the worker
        // could not bind or serve at all.
        let socket_mode = std::fs::metadata(&socket_dir)
            .expect("stat socket dir")
            .permissions()
            .mode();
        assert_eq!(
            socket_mode & 0o070,
            0o070,
            "the socket-directory mask must carry `rwx`"
        );
        let view_mode = std::fs::metadata(&shared)
            .expect("stat view root")
            .permissions()
            .mode();
        assert_eq!(
            view_mode & 0o050,
            0o050,
            "the view-root mask must carry `r-x`"
        );
    }

    /// A VM-scoped legacy swtpm plan: the trusted cgroup placement names the
    /// VM, and the plan's writable paths carry the per-VM state dir and the
    /// per-VM runtime dir `derive_paths` cross-checks against it.
    fn legacy_swtpm_plan(vms_root: &Path, vm: &str) -> SpawnRunnerPlan {
        let runtime_dir = vms_root.join(vm);
        let swtpm_dir = runtime_dir.join("swtpm");
        let mut plan = test_spawn_plan_with_argv(
            vec![
                "swtpm".to_owned(),
                "socket".to_owned(),
                "--tpm2".to_owned(),
                "--tpmstate".to_owned(),
                format!("dir={}", swtpm_dir.display()),
            ],
            "w1-swtpm",
        );
        plan.cgroup_placement.subtree = format!("d2b.slice/{vm}/swtpm");
        plan.mount_policy.writable_paths = vec![
            WritablePath {
                path: swtpm_dir.display().to_string(),
                purpose: "state".to_owned(),
            },
            WritablePath {
                path: runtime_dir.display().to_string(),
                purpose: "runtime".to_owned(),
            },
        ];
        plan
    }

    /// The per-VM device socket directory is opened to the launched worker
    /// principal (the swtpm/GPU rows run as their own intent principal, never
    /// as the daemon) with the same access-ACL shape the serving worker's
    /// socket directory gets - and only after the launch's typed fences
    /// passed, so the grant is applied on the success path alone.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn device_worker_socket_grant_opens_the_socket_directory_to_the_principal() {
        use std::os::unix::fs::PermissionsExt as _;

        if ![
            "/run/current-system/sw/bin/setfacl",
            "/usr/bin/setfacl",
            "/bin/setfacl",
        ]
        .iter()
        .any(|candidate| Path::new(candidate).exists())
        {
            eprintln!("skipping device-worker ACL application test: no setfacl binary");
            return;
        }

        let root = TestDir::new("device-worker-acl-apply");
        let runtime_root = root.join("run").join("d2b");
        let socket_dir = runtime_root.join("vms").join("acceptance-guest");
        std::fs::create_dir_all(&socket_dir).expect("create socket dir");
        std::fs::set_permissions(&runtime_root, std::fs::Permissions::from_mode(0o700))
            .expect("chmod runtime root");
        std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))
            .expect("chmod socket dir");

        let uid = 50_123;
        let grant = DeviceWorkerSocketGrant::for_guest(&runtime_root, "acceptance-guest")
            .expect("the trusted Guest scope names the socket directory");
        assert_eq!(grant.directory, socket_dir);
        grant.apply(uid).expect("grant ACLs");

        let fd = crate::sys::path_safe::open_dir_path_safe(&socket_dir).expect("open dir");
        assert_eq!(
            crate::sys::path_safe::fd_extended_acl_present(fd.as_fd()).expect("inspect ACL"),
            (true, false),
            "the device socket directory must carry an access ACL entry for the worker principal"
        );
        let mode = std::fs::metadata(&socket_dir)
            .expect("stat socket dir")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o070,
            0o070,
            "the socket-directory mask must carry `rwx`"
        );
    }

    /// The grant may only ever reach a directory strictly inside the broker's
    /// own runtime root, named by a plain Guest component: a Guest name that
    /// escapes the per-Guest directory - or a runtime root that is not an
    /// anchored absolute path - is refused instead of widened.
    #[test]
    fn device_worker_socket_grant_refuses_a_directory_outside_the_runtime_root() {
        let runtime_root = PathBuf::from("/run/d2b");
        for guest in ["", ".", "..", "../foreign", "a/b", "/etc"] {
            let error = DeviceWorkerSocketGrant::for_guest(&runtime_root, guest)
                .expect_err("an escaping Guest name must be refused");
            assert!(
                error.contains("not-a-plain-name") || error.contains("outside the runtime root"),
                "{guest}: {error}"
            );
        }
        for root in [PathBuf::from("run/d2b"), PathBuf::from("/run/d2b/../etc")] {
            let error = DeviceWorkerSocketGrant::for_guest(&root, "acceptance-guest")
                .expect_err("an unanchored runtime root must be refused");
            assert!(error.contains("not-anchored"), "{error}");
        }

        // A legacy plan whose trusted runtime directory is outside the broker
        // runtime root is refused the same way.
        let dir = TestDir::new("device-worker-legacy-foreign-root");
        let plan = legacy_swtpm_plan(&dir.join("outside").join("vms"), "acceptance-guest");
        let error = DeviceWorkerSocketGrant::for_legacy_plan(&plan, Path::new("/run/d2b"))
            .expect_err("a runtime directory outside the broker runtime root must be refused");
        assert!(
            error.contains("outside the broker runtime directory"),
            "{error}"
        );
    }

    /// The grant never reads the launch arguments: the directory opened to the
    /// worker is the one the plan's own trusted derivation names, so a launch
    /// whose argv points at another Guest's socket directory can only ever be
    /// granted its own - and a launch whose placement carries no trusted
    /// runtime directory at all is refused instead of granted.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn device_worker_socket_grant_never_reads_the_launch_arguments() {
        let dir = TestDir::new("device-worker-grant-argv");
        let runtime_root = dir.join("run").join("d2b");
        let trusted = runtime_root.join("vms").join("acceptance-guest");
        std::fs::create_dir_all(&trusted).expect("create trusted runtime dir");

        // The argv names a foreign socket directory; the plan's trusted
        // writable paths and cgroup placement name the Guest's own.
        let mut plan = legacy_swtpm_plan(&runtime_root.join("vms"), "acceptance-guest");
        plan.argv.push("--server".to_owned());
        plan.argv
            .push("type=unixio,path=/run/foreign/tpm.sock,mode=0660,uid=0,gid=0".to_owned());
        let grant = DeviceWorkerSocketGrant::for_legacy_plan(&plan, &runtime_root)
            .expect("the trusted runtime directory grants");
        assert_eq!(grant.directory, trusted);
        assert_ne!(grant.directory, PathBuf::from("/run/foreign"));

        // A role that binds no socket under the runtime root (the one-shot
        // flush, the video sidecar) resolves no grant at all.
        let no_socket = crate::ops::device_worker::DeviceWorkerLaunch {
            scope: None,
            binds_runtime_socket: false,
        };
        assert!(
            device_worker_socket_grant(&plan, &no_socket, &runtime_root)
                .expect("no socket bound")
                .is_none()
        );

        // The typed shape has no legacy plan to fall back to: a resource-backed
        // placement carries no VM name the derivation may read, so the grant is
        // refused rather than invented.
        let mut backed = plan.clone();
        backed.cgroup_placement.subtree = format!(
            "d2b.slice/{}/swtpm",
            "process-".to_owned() + &"c".repeat(64)
        );
        assert!(
            DeviceWorkerSocketGrant::for_legacy_plan(&backed, &runtime_root).is_err(),
            "a typed placement must not resolve a legacy runtime directory"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn current_path_dev_ino_tracks_inode_replacement() {
        // Inode pinning relies on re-stat detecting that a path now
        // resolves to a different inode than the one a prior fd mutated.
        // Use a rename to swap a fresh inode over the path deterministically
        // (remove+recreate can reuse the same inode number on some
        // filesystems, so don't rely on the allocator).
        let dir = TestDir::new("gc-restat");
        let path = dir.join("sock");
        let other = dir.join("other");
        std::fs::write(&path, b"a").expect("write path");
        std::fs::write(&other, b"bb").expect("write other");
        let path_id = current_path_dev_ino(&path)
            .expect("stat path")
            .expect("present");
        let other_id = current_path_dev_ino(&other)
            .expect("stat other")
            .expect("present");
        assert_ne!(
            path_id.1, other_id.1,
            "distinct files must have distinct inodes"
        );
        // Rename `other` over `path`: the path now resolves to other's inode.
        std::fs::rename(&other, &path).expect("rename over path");
        let after = current_path_dev_ino(&path)
            .expect("stat after")
            .expect("present");
        assert_eq!(
            after, other_id,
            "path must resolve to the replacement inode"
        );
        assert_ne!(
            after, path_id,
            "path must no longer resolve to the original inode"
        );
        assert_eq!(current_path_dev_ino(&dir.join("absent")), Ok(None));
    }

    #[test]
    fn parses_quoted_otel_host_bridge_ch_vsock_socket() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "d2b-otel-host-bridge".to_owned(),
                "-d".to_owned(),
                "-d".to_owned(),
                "UNIX-LISTEN:/run/d2b/otel/host-egress.sock,fork,reuseaddr,mode=0660"
                    .to_owned(),
                "EXEC:\"/run/current-system/sw/bin/d2b-ch-vsock-connect /var/lib/d2b/vms/sys-obs/vsock.sock 14317\""
                    .to_owned(),
            ],
            "w1-otel-host-bridge",
        );

        assert_eq!(
            ch_vsock_connect_socket_arg(&plan),
            Some(PathBuf::from("/var/lib/d2b/vms/sys-obs/vsock.sock"))
        );
    }

    #[test]
    fn parses_unquoted_vsock_relay_ch_vsock_socket() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "d2b-otel-relay@work-aad".to_owned(),
                "-d".to_owned(),
                "-d".to_owned(),
                "UNIX-LISTEN:/var/lib/d2b/vms/work-aad/vsock.sock_14317,fork,max-children=16,reuseaddr,mode=0660"
                    .to_owned(),
                "EXEC:/run/current-system/sw/bin/d2b-ch-vsock-connect /var/lib/d2b/vms/sys-obs/vsock.sock 14318"
                    .to_owned(),
            ],
            "w1-vsock-relay",
        );

        assert_eq!(
            ch_vsock_connect_socket_arg(&plan),
            Some(PathBuf::from("/var/lib/d2b/vms/sys-obs/vsock.sock"))
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn live_spawn_runner_applies_capability_and_net_namespace_when_privileged() {
        if rustix::process::geteuid().as_raw() != 0 {
            eprintln!("skipping privileged SpawnRunner isolation test: requires euid 0");
            return;
        }

        let dir = TestDir::new("spawn-runner-netns");
        let status_path = dir.join("status.txt");
        let netns_path = dir.join("netns.txt");
        let current_netns = tokio::fs::read_link("/proc/self/ns/net").await
            .expect("read current netns")
            .display()
            .to_string();
        let cmd = format!(
            "cat /proc/self/status > {}; readlink /proc/self/ns/net > {}",
            status_path.display(),
            netns_path.display()
        );
        let plan = SpawnRunnerPlanInput {
            binary_path: PathBuf::from("/bin/sh"),
            argv: vec!["sh".to_owned(), "-c".to_owned(), cmd],
            uid: 0,
            gid: 0,
            supplementary_groups: vec![],
            env: vec![],
            capabilities: vec!["CAP_NET_ADMIN".to_owned()],
            namespaces: NamespaceSet {
                mount: false,
                pid: false,
                net: true,
                ipc: false,
                uts: false,
                user: false,
            },
            seccomp_policy_ref: None,
            mount_policy: MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: false,
                hide_device_nodes_by_default: false,
                device_binds: Vec::new(),
                bind_mounts: Vec::new(),
            },
            cgroup_placement: test_cgroup_placement(),
            root_carve_out: true,
            skip_binary_exists_check: false,
            user_namespace: None,
            umask: None,
        };

        let outcome = live_spawn_runner(
            &plan,
            Vec::new(),
            Vec::new(),
            None,
            Path::new("/var/lib/d2b"),
            None,
            &crate::ops::device_worker::DeviceWorkerLaunch::default(),
            Path::new("/run/d2b"),
        )
        .await
        .expect("spawn privileged test child");
        let wait_status = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(outcome.pid), None)
            .expect("wait for test child");
        assert!(matches!(
            wait_status,
            nix::sys::wait::WaitStatus::Exited(_, 0)
        ));

        let status = tokio::fs::read_to_string(&status_path).await.expect("read child status");
        let netns = tokio::fs::read_to_string(&netns_path).await.expect("read child netns");
        let cap_eff = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("CapEff:\t")
                    .or_else(|| line.strip_prefix("CapEff:"))
            })
            .expect("CapEff line present")
            .trim();
        let cap_mask = u64::from_str_radix(cap_eff, 16).expect("parse CapEff hex");
        assert_ne!(cap_mask & (1u64 << 12), 0);
        assert_ne!(netns.trim(), current_netns);
    }

    fn wayland_proxy_plan(
        runtime_dir: Option<&str>,
        wayland_display: Option<&str>,
    ) -> SpawnRunnerPlan {
        let mut plan = test_spawn_plan_with_argv(
            vec!["d2b-wayland-proxy@personal-dev".to_owned()],
            "w1-wayland-proxy",
        );
        if let Some(dir) = runtime_dir {
            plan.env.push(format!("XDG_RUNTIME_DIR={dir}"));
        }
        if let Some(display) = wayland_display {
            plan.env.push(format!("WAYLAND_DISPLAY={display}"));
        }
        plan
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn wayland_proxy_acls_error_when_xdg_runtime_dir_not_set() {
        let plan = wayland_proxy_plan(None, None);
        let err = refresh_spawn_runner_acls(&plan, Path::new("/var/lib/d2b"))
            .await
            .expect_err("missing XDG_RUNTIME_DIR must fail");
        let detail = match err {
            LiveHandlerError::SpawnFailed { detail } => detail,
            other => panic!("expected SpawnFailed, got {other:?}"),
        };
        assert!(
            detail.contains("graphical-session-not-active"),
            "error must use graphical-session-not-active prefix: {detail}"
        );
        assert!(
            detail.contains("XDG_RUNTIME_DIR not set"),
            "error must name the missing env var: {detail}"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn wayland_proxy_acls_error_when_runtime_dir_absent() {
        let root = TestDir::new("wlproxy-absent-dir");
        // Point at a path that does not exist beneath the tempdir.
        let absent = root.join("run").join("user").join("1000");
        let plan = wayland_proxy_plan(Some(absent.to_str().unwrap()), None);
        let err = refresh_spawn_runner_acls(&plan, Path::new("/var/lib/d2b"))
            .await
            .expect_err("absent runtime dir must fail");
        let detail = match err {
            LiveHandlerError::SpawnFailed { detail } => detail,
            other => panic!("expected SpawnFailed, got {other:?}"),
        };
        assert!(
            detail.contains("graphical-session-not-active"),
            "error must use graphical-session-not-active prefix: {detail}"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn wayland_proxy_acls_error_when_runtime_dir_owner_mismatch() {
        let root = TestDir::new("wlproxy-owner-mismatch");
        // Create a dir owned by the current user but parsed path uid != current uid.
        // Pick a uid that is almost certainly not the running test uid.
        let current_uid = nix::unistd::Uid::current().as_raw();
        let mismatch_uid = if current_uid == 9999999 {
            9999998
        } else {
            9999999
        };
        // Path ends in mismatch_uid so the code expects owner == mismatch_uid,
        // but the dir is owned by current_uid.
        let runtime = root.join(&format!("{mismatch_uid}"));
        std::fs::create_dir(&runtime).expect("create runtime dir");
        let plan = wayland_proxy_plan(Some(runtime.to_str().unwrap()), None);
        let err = refresh_spawn_runner_acls(&plan, Path::new("/var/lib/d2b"))
            .await
            .expect_err("owner uid mismatch must fail");
        let detail = match err {
            LiveHandlerError::SpawnFailed { detail } => detail,
            other => panic!("expected SpawnFailed, got {other:?}"),
        };
        assert!(
            detail.contains("graphical-session-not-active"),
            "error must use graphical-session-not-active prefix: {detail}"
        );
        assert!(
            detail.contains("owner mismatch"),
            "error must name owner mismatch: {detail}"
        );
        assert!(
            detail.contains(&format!("{mismatch_uid}")),
            "error must name expected uid: {detail}"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn wayland_proxy_acls_error_when_wayland_socket_absent() {
        let root = TestDir::new("wlproxy-socket-absent");
        // Use the current uid so the owner check passes.
        let current_uid = nix::unistd::Uid::current().as_raw();
        let runtime = root.join(&format!("{current_uid}"));
        std::fs::create_dir(&runtime).expect("create runtime dir");
        // Do NOT create the socket. Use a known display name.
        let plan = wayland_proxy_plan(Some(runtime.to_str().unwrap()), Some("wayland-test-99"));
        let err = refresh_spawn_runner_acls(&plan, Path::new("/var/lib/d2b"))
            .await
            .expect_err("absent Wayland socket must fail");
        let detail = match err {
            LiveHandlerError::SpawnFailed { detail } => detail,
            other => panic!("expected SpawnFailed, got {other:?}"),
        };
        // The error could come from either:
        //   a) the traverse ACL step (fails to setfacl on real tempdir in CI)
        //   b) the socket-absent check.
        // Accept both: the important thing is we reached an error before
        // granting the wlproxy uid access to an unintended path.
        assert!(
            detail.contains("graphical-session-not-active")
                || detail.contains("traverse ACL")
                || detail.contains("wayland-proxy"),
            "must fail with a wayland-proxy-related error: {detail}"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn wayland_proxy_acls_error_when_socket_is_regular_file() {
        let root = TestDir::new("wlproxy-socket-wrong-type");
        let current_uid = nix::unistd::Uid::current().as_raw();
        let runtime = root.join(&format!("{current_uid}"));
        std::fs::create_dir(&runtime).expect("create runtime dir");
        // Plant a regular file where the socket should be.
        let socket_path = runtime.join("wayland-type-test");
        tokio::fs::write(&socket_path, b"not a socket").await.expect("write file");
        let plan = wayland_proxy_plan(Some(runtime.to_str().unwrap()), Some("wayland-type-test"));
        let err = refresh_spawn_runner_acls(&plan, Path::new("/var/lib/d2b"))
            .await
            .expect_err("regular file at socket location must fail");
        let detail = match err {
            LiveHandlerError::SpawnFailed { detail } => detail,
            other => panic!("expected SpawnFailed, got {other:?}"),
        };
        // Same as above: traverse ACL may fail first in CI without CAP_SETFCAP.
        assert!(
            detail.contains("graphical-session-not-active")
                || detail.contains("traverse ACL")
                || detail.contains("not a socket"),
            "must fail with graphical-session-not-active or type error: {detail}"
        );
    }

    /// One pending refresh per socket. A second caller while a refresh is
    /// waiting joins it rather than starting another, and the claim is
    /// released when the waiter ends - a leaked claim would leave that socket
    /// without an ACL refresh for the process's life.
    #[test]
    fn one_pending_obs_vsock_refresh_serves_every_caller() {
        let socket = PathBuf::from("/run/d2b/obs-single-flight.sock");
        let uid = 4242;
        assert!(
            claim_obs_vsock_acl_retry(uid, &socket),
            "the first caller claims the socket"
        );
        assert!(
            !claim_obs_vsock_acl_retry(uid, &socket),
            "a second caller joins the pending refresh"
        );
        assert!(
            claim_obs_vsock_acl_retry(uid + 1, &socket),
            "another principal is another grant, so it is its own claim"
        );
        for uid in [uid, uid + 1] {
            drop(PendingObsVsockAclRetry {
                uid,
                socket: socket.clone(),
            });
        }
        assert!(
            claim_obs_vsock_acl_retry(uid, &socket),
            "the claim is released when the refresh ends"
        );
        drop(PendingObsVsockAclRetry { uid, socket });
    }

    /// The canonical async ACL retry (plan U8 item 4) stops as soon as the
    /// attempt reports ready: exactly one attempt runs. Hermetic: the
    /// attempt is injected, so no setfacl shellout happens.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn acl_grant_retry_stops_on_first_success() {
        let background = crate::runtime::BrokerBackground {
            runtime: tokio::runtime::Handle::current(),
            dispatches: crate::runtime::DispatchPool::new(2),
        };
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempt_calls = Arc::clone(&calls);
        let attempt: Arc<dyn Fn() -> Result<bool, String> + Send + Sync> = Arc::new(move || {
            attempt_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(true)
        });
        retry_acl_grant(
            &background,
            Duration::from_secs(5),
            Duration::from_millis(10),
            "test",
            attempt,
        )
        .await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a ready grant must stop the retry after the first attempt"
        );
    }

    /// The canonical async ACL retry respects its deadline: a socket that
    /// never becomes ready keeps attempting until the window expires, then
    /// stops. Timing-tolerant by construction (R13): the assertions are
    /// lower bounds only - the retry must not give up before the window
    /// elapses and must have attempted more than once.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn acl_grant_retry_respects_deadline() {
        let background = crate::runtime::BrokerBackground {
            runtime: tokio::runtime::Handle::current(),
            dispatches: crate::runtime::DispatchPool::new(2),
        };
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempt_calls = Arc::clone(&calls);
        let attempt: Arc<dyn Fn() -> Result<bool, String> + Send + Sync> = Arc::new(move || {
            attempt_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(false)
        });
        let started = tokio::time::Instant::now();
        retry_acl_grant(
            &background,
            Duration::from_millis(150),
            Duration::from_millis(20),
            "test",
            attempt,
        )
        .await;
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(150),
            "the retry must not give up before its deadline: {elapsed:?}"
        );
        assert!(
            calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "the retry must keep attempting until the deadline"
        );
    }
}
