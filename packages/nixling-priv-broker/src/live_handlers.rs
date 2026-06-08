//! W4-fu live broker request handlers.
//!
//! These functions execute the side-effectful work for the broker's
//! production dispatch path — `pidfd_open(2)` + start-time
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
//! The `pidfd_open` and `clone3` paths require a live kernel and
//! are exercised by the broker integration tests (broker-pidfd-
//! adopt-roundtrip.sh, W4-fu broker-spawn-runner-smoke.sh).

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use crate::ops::exec_reconcile::{
    GeneratedSshKey, IpRouteVerb, ReconcileExecError, ReconcileExecutor,
};
use crate::ops::spawn_runner::{
    build_cstring_vectors, preflight, SpawnRunnerError, SpawnRunnerPlan, SpawnRunnerPlanInput,
};
use nixling_core::bundle_resolver::{
    HostRuntime, ResolvedActivationIntent, ResolvedStoreViewIntent,
};
use nixling_core::minijail_profile::CgroupPlacement;
use nixling_host::hardlink_farm;
use nixling_ipc::broker_wire::ActivationMode;

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
    /// W13 (W6-fu) per-busid lock failure (already-held, owner
    /// mismatch on release, or I/O error on lock root).
    UsbipLock(String),
    /// W15 (W9-fu) host install / migrate writer failure.
    HostInstall(String),
    /// W14 LiveNative activation / GC / key-management failures that
    /// are not raw executor errors.
    Activation(String),
    Gc(String),
    KeysRotate(String),
    HostKey(String),
    /// W12 NetworkManager reload failure after writing the unmanaged
    /// config snippet.
    NmReload(String),
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
            Self::ProcStatReadFailed { pid, detail } => write!(
                f,
                "/proc/{pid}/stat read after pidfd_open: {detail}"
            ),
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

/// W4-fu live broker `OpenPidfd` handler.
///
/// Performs the open-AND-verify atomically:
/// 1. `pidfd_open(pid)`.
/// 2. `/proc/<pid>/stat` field-22 read.
/// 3. Compare against `expected_start_time_ticks`.
/// 4. On match: return the pidfd.
/// 5. On mismatch: drop the pidfd (closing it) and return
///    [`LiveHandlerError::PidfdRace`].
///
/// This closes the W*-fu GPT-5.5 panel CRITICAL pid-reuse race
/// — the daemon's pre-call /proc read is augmented by the broker's
/// post-open re-check so the returned pidfd is provably bound to
/// the original process.
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

/// W4-fu live broker `ApplyNftables` handler. Wraps the
/// `exec_reconcile::ReconcileExecutor::apply_nft_script` call.
pub fn live_apply_nftables(
    executor: &dyn ReconcileExecutor,
    nft_binary: &Path,
    nft_script: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .apply_nft_script(nft_binary, nft_script)
        .map_err(LiveHandlerError::ReconcileExec)
}

/// W4-fu live broker `ApplySysctl` handler.
pub fn live_apply_sysctl(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .write_sysctl(key, value)
        .map_err(LiveHandlerError::ReconcileExec)
}

/// W4-fu live broker `UpdateHostsFile` handler. Atomic write with
/// fsync via the executor.
pub fn live_update_hosts_file(
    executor: &dyn ReconcileExecutor,
    path: &Path,
    contents: &[u8],
    mode: u32,
) -> Result<(), LiveHandlerError> {
    executor
        .write_atomic_file(path, contents, mode)
        .map_err(LiveHandlerError::ReconcileExec)
}

/// W4-fu live broker `ApplyRoute` handler.
pub fn live_apply_route(
    executor: &dyn ReconcileExecutor,
    ip_binary: &Path,
    verb: IpRouteVerb,
    route_spec: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .ip_route(ip_binary, verb, route_spec)
        .map_err(LiveHandlerError::ReconcileExec)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreViewOutcome {
    pub vm: String,
    pub generation: u64,
    pub hardlink_farm_path: PathBuf,
    pub target_view_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountNamespaceOutcome {
    pub vm: String,
    pub role_id: String,
    pub mount_root: PathBuf,
    pub mount_view_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationOutcome {
    pub mode: ActivationMode,
    pub vm: String,
    pub generation_number: Option<u64>,
    pub summary: String,
    pub prepared_store_view: Option<StoreViewOutcome>,
    pub mount_namespace: MountNamespaceOutcome,
    pub activation_script_path: PathBuf,
    pub activation_script_mode: String,
    pub rollback_marker_written: Option<u64>,
    pub current_generation_updated: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcOutcome {
    pub keep_generations: Option<u32>,
    pub retained_store_path_count: u32,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysRotateOutcome {
    pub vm: String,
    pub key_path: PathBuf,
    pub public_key_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyTrustOutcome {
    pub vm: String,
    pub static_ip: String,
    pub known_hosts_path: PathBuf,
    pub updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateKnownHostOutcome {
    pub vm: String,
    pub static_ip: String,
    pub known_hosts_path: PathBuf,
    pub removed: bool,
}

const ACTIVATION_MOUNT_ROLE_ID: &str = "activation";

pub fn live_prepare_store_view(
    executor: &dyn ReconcileExecutor,
    intent: &ResolvedStoreViewIntent,
) -> Result<StoreViewOutcome, LiveHandlerError> {
    executor
        .prepare_store_view(intent)
        .map_err(LiveHandlerError::ReconcileExec)?;
    Ok(StoreViewOutcome {
        vm: intent.vm.clone(),
        generation: intent.generation,
        hardlink_farm_path: intent.hardlink_farm_path.clone(),
        target_view_path: intent.target_view_path.clone(),
    })
}

pub fn live_setup_mount_namespace(
    executor: &dyn ReconcileExecutor,
    vm: &str,
    hardlink_farm_path: &Path,
    role_id: &str,
    source_view_path: &Path,
) -> Result<MountNamespaceOutcome, LiveHandlerError> {
    let mount_root = mount_root_for(hardlink_farm_path, role_id)?;
    let mount_view_path = executor
        .setup_mount_namespace(vm, role_id, source_view_path, &mount_root)
        .map_err(LiveHandlerError::ReconcileExec)?;
    Ok(MountNamespaceOutcome {
        vm: vm.to_owned(),
        role_id: role_id.to_owned(),
        mount_root,
        mount_view_path,
    })
}

pub fn live_run_activation(
    executor: &dyn ReconcileExecutor,
    intent: &ResolvedActivationIntent,
    store_view_intent: &ResolvedStoreViewIntent,
    mode: ActivationMode,
) -> Result<ActivationOutcome, LiveHandlerError> {
    if intent.vm != store_view_intent.vm {
        return Err(LiveHandlerError::Activation(format!(
            "activation/store-view vm mismatch: activation={} store-view={}",
            intent.vm, store_view_intent.vm,
        )));
    }
    if let Some(generation_number) = intent.generation_number {
        if generation_number != store_view_intent.generation {
            return Err(LiveHandlerError::Activation(format!(
                "activation/store-view generation mismatch: activation={} store-view={}",
                generation_number, store_view_intent.generation,
            )));
        }
    }

    let previous_current = read_current_generation(&store_view_intent.hardlink_farm_path)?;
    let mut prepared_store_view = None;
    let (target_generation, target_view_path) = match mode {
        ActivationMode::Rollback => {
            let rollback_generation = read_rollback_marker(&store_view_intent.hardlink_farm_path)?
                .ok_or_else(|| {
                    LiveHandlerError::Activation(format!(
                        "rollback marker missing at {}",
                        rollback_marker_path(&store_view_intent.hardlink_farm_path).display(),
                    ))
                })?;
            let generation_dir =
                generation_dir(&store_view_intent.hardlink_farm_path, rollback_generation);
            hardlink_farm::read_generation_marker(&generation_dir)
                .map_err(|err| LiveHandlerError::Activation(err.to_string()))?;
            let target_view_path =
                target_view_for_generation(store_view_intent, rollback_generation)?;
            if !target_view_path.exists() {
                return Err(LiveHandlerError::Activation(format!(
                    "rollback target store view missing at {}",
                    target_view_path.display(),
                )));
            }
            (rollback_generation, target_view_path)
        }
        _ => {
            let store_view = live_prepare_store_view(executor, store_view_intent)?;
            let target_generation = store_view.generation;
            let target_view_path = store_view.target_view_path.clone();
            prepared_store_view = Some(store_view);
            (target_generation, target_view_path)
        }
    };

    let mount_namespace = live_setup_mount_namespace(
        executor,
        &intent.vm,
        &store_view_intent.hardlink_farm_path,
        ACTIVATION_MOUNT_ROLE_ID,
        &target_view_path,
    )?;
    let activation_script_mode = activation_script_mode(mode).to_owned();
    let activation_script_path = mount_namespace
        .mount_view_path
        .join("bin/switch-to-configuration");
    let summary = executor
        .run_activation_script(
            &activation_script_mode,
            &target_view_path,
            &mount_namespace.mount_view_path,
        )
        .map_err(LiveHandlerError::ReconcileExec)?;

    let mut rollback_marker_written = None;
    let mut current_generation_updated = None;
    match mode {
        ActivationMode::Test => {
            if let Some(previous_generation) = previous_current.filter(|g| *g != target_generation)
            {
                write_rollback_marker(&store_view_intent.hardlink_farm_path, previous_generation)?;
                rollback_marker_written = Some(previous_generation);
            }
        }
        ActivationMode::Switch | ActivationMode::Boot => {
            if let Some(previous_generation) = previous_current.filter(|g| *g != target_generation)
            {
                write_rollback_marker(&store_view_intent.hardlink_farm_path, previous_generation)?;
                rollback_marker_written = Some(previous_generation);
            }
            swap_current_generation(&store_view_intent.hardlink_farm_path, target_generation)?;
            current_generation_updated = Some(target_generation);
        }
        ActivationMode::Rollback => {
            swap_current_generation(&store_view_intent.hardlink_farm_path, target_generation)?;
            current_generation_updated = Some(target_generation);
            clear_rollback_marker(&store_view_intent.hardlink_farm_path)?;
        }
    }

    Ok(ActivationOutcome {
        mode,
        vm: intent.vm.clone(),
        generation_number: Some(target_generation),
        summary,
        prepared_store_view,
        mount_namespace,
        activation_script_path,
        activation_script_mode,
        rollback_marker_written,
        current_generation_updated,
    })
}

fn activation_script_mode(mode: ActivationMode) -> &'static str {
    match mode {
        ActivationMode::Switch => "switch",
        ActivationMode::Boot => "boot",
        ActivationMode::Test => "test",
        ActivationMode::Rollback => "switch",
    }
}

fn mount_root_for(hardlink_farm_path: &Path, role_id: &str) -> Result<PathBuf, LiveHandlerError> {
    let vm_root = hardlink_farm_path.parent().ok_or_else(|| {
        LiveHandlerError::Activation(format!(
            "store-view root {} lacks a VM state-dir parent",
            hardlink_farm_path.display(),
        ))
    })?;
    Ok(vm_root.join("mount-ns").join(role_id))
}

fn generation_dir(hardlink_farm_path: &Path, generation: u64) -> PathBuf {
    hardlink_farm_path
        .join("generations")
        .join(generation.to_string())
}

fn target_view_for_generation(
    intent: &ResolvedStoreViewIntent,
    generation: u64,
) -> Result<PathBuf, LiveHandlerError> {
    let target_name = intent.target_view_path.file_name().ok_or_else(|| {
        LiveHandlerError::Activation(format!(
            "store-view target {} lacks a basename",
            intent.target_view_path.display(),
        ))
    })?;
    Ok(generation_dir(&intent.hardlink_farm_path, generation).join(target_name))
}

fn read_current_generation(hardlink_farm_path: &Path) -> Result<Option<u64>, LiveHandlerError> {
    read_generation_pointer(&hardlink_farm_path.join("current"))
}

fn read_generation_pointer(path: &Path) -> Result<Option<u64>, LiveHandlerError> {
    match std::fs::read_link(path) {
        Ok(target) => parse_generation_pointer(path, &target).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(LiveHandlerError::Activation(format!(
            "readlink {}: {error}",
            path.display(),
        ))),
    }
}

fn parse_generation_pointer(path: &Path, target: &Path) -> Result<u64, LiveHandlerError> {
    let generation = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            LiveHandlerError::Activation(format!(
                "generation pointer {} has invalid target {}",
                path.display(),
                target.display(),
            ))
        })?;
    generation.parse::<u64>().map_err(|error| {
        LiveHandlerError::Activation(format!(
            "generation pointer {} targets non-numeric generation {}: {error}",
            path.display(),
            target.display(),
        ))
    })
}

fn rollback_marker_path(hardlink_farm_path: &Path) -> PathBuf {
    hardlink_farm_path.join("rollback-marker")
}

fn read_rollback_marker(hardlink_farm_path: &Path) -> Result<Option<u64>, LiveHandlerError> {
    let marker_path = rollback_marker_path(hardlink_farm_path);
    match std::fs::read_to_string(&marker_path) {
        Ok(contents) => contents.trim().parse::<u64>().map(Some).map_err(|error| {
            LiveHandlerError::Activation(format!(
                "rollback marker {} is invalid: {error}",
                marker_path.display(),
            ))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(LiveHandlerError::Activation(format!(
            "read rollback marker {}: {error}",
            marker_path.display(),
        ))),
    }
}

fn write_rollback_marker(
    hardlink_farm_path: &Path,
    generation: u64,
) -> Result<(), LiveHandlerError> {
    let marker_path = rollback_marker_path(hardlink_farm_path);
    std::fs::create_dir_all(hardlink_farm_path).map_err(|error| {
        LiveHandlerError::Activation(format!(
            "create store-view root {}: {error}",
            hardlink_farm_path.display(),
        ))
    })?;
    std::fs::write(&marker_path, format!("{generation}\n")).map_err(|error| {
        LiveHandlerError::Activation(format!(
            "write rollback marker {}: {error}",
            marker_path.display(),
        ))
    })
}

fn clear_rollback_marker(hardlink_farm_path: &Path) -> Result<(), LiveHandlerError> {
    let marker_path = rollback_marker_path(hardlink_farm_path);
    match std::fs::remove_file(&marker_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LiveHandlerError::Activation(format!(
            "remove rollback marker {}: {error}",
            marker_path.display(),
        ))),
    }
}

fn swap_current_generation(
    hardlink_farm_path: &Path,
    generation: u64,
) -> Result<(), LiveHandlerError> {
    let generation = u32::try_from(generation).map_err(|_| {
        LiveHandlerError::Activation(format!("generation {generation} exceeds u32"))
    })?;
    hardlink_farm::swap_current_symlink(hardlink_farm_path, generation)
        .map_err(|error| LiveHandlerError::Activation(error.to_string()))
}

pub fn live_run_gc(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedGcIntent,
    keep_generations: Option<u32>,
) -> Result<GcOutcome, LiveHandlerError> {
    let summary = executor
        .run_gc(keep_generations)
        .map_err(LiveHandlerError::ReconcileExec)?;
    Ok(GcOutcome {
        keep_generations,
        retained_store_path_count: intent.retained_store_paths.len() as u32,
        summary,
    })
}

pub fn live_run_keys_rotate(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedKeysRotateIntent,
) -> Result<KeysRotateOutcome, LiveHandlerError> {
    let GeneratedSshKey {
        public_key_fingerprint,
    } = executor
        .run_ssh_keygen(&intent.key_path, &format!("nixling:{}", intent.vm))
        .map_err(LiveHandlerError::ReconcileExec)?;
    Ok(KeysRotateOutcome {
        vm: intent.vm.clone(),
        key_path: intent.key_path.clone(),
        public_key_fingerprint,
    })
}

pub fn live_run_trust(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedHostKeyTrustIntent,
) -> Result<HostKeyTrustOutcome, LiveHandlerError> {
    let host_public_key = read_required_single_line(&intent.host_public_key_path)?;
    let existing = read_known_hosts_lines(&intent.known_hosts_path)?;
    let rendered_line = format!("{} {}", intent.static_ip, host_public_key);
    let updated = !existing.iter().any(|line| line.trim() == rendered_line);
    let rewritten = rewrite_known_hosts_lines(&existing, &intent.static_ip, Some(&rendered_line));
    executor
        .write_atomic_file(
            &intent.known_hosts_path,
            &render_known_hosts_lines(&rewritten),
            0o644,
        )
        .map_err(LiveHandlerError::ReconcileExec)?;
    Ok(HostKeyTrustOutcome {
        vm: intent.vm.clone(),
        static_ip: intent.static_ip.clone(),
        known_hosts_path: intent.known_hosts_path.clone(),
        updated,
    })
}

pub fn live_run_rotate_known_host(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedRotateKnownHostIntent,
) -> Result<RotateKnownHostOutcome, LiveHandlerError> {
    let existing = read_known_hosts_lines(&intent.known_hosts_path)?;
    let rewritten = rewrite_known_hosts_lines(&existing, &intent.static_ip, None);
    let removed = rewritten.len() != existing.len();
    if removed || intent.known_hosts_path.exists() {
        executor
            .write_atomic_file(
                &intent.known_hosts_path,
                &render_known_hosts_lines(&rewritten),
                0o644,
            )
            .map_err(LiveHandlerError::ReconcileExec)?;
    }
    Ok(RotateKnownHostOutcome {
        vm: intent.vm.clone(),
        static_ip: intent.static_ip.clone(),
        known_hosts_path: intent.known_hosts_path.clone(),
        removed,
    })
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

/// W12 live broker `ApplyNmUnmanaged` handler.
pub fn live_apply_nm_unmanaged(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedNmUnmanagedIntent,
) -> Result<(), LiveHandlerError> {
    if let Some(method) = live_apply_nm_unmanaged_with_reloaders(
        executor,
        intent,
        networkmanager_reload_via_dbus,
        systemctl_invoke,
    )? {
        tracing::info!(
            reload_method = method.as_str(),
            file_path = %intent.file_path.display(),
            "reloaded NetworkManager after writing unmanaged config"
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn live_apply_nm_unmanaged_with_reload<F>(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedNmUnmanagedIntent,
    mut reload: F,
) -> Result<(), LiveHandlerError>
where
    F: FnMut(&[&str]) -> Result<(), String>,
{
    executor
        .write_atomic_file(&intent.file_path, intent.contents.as_bytes(), intent.mode)
        .map_err(LiveHandlerError::ReconcileExec)?;
    if intent.reload_behavior == "atomic-reload" {
        reload(&["reload", "NetworkManager"]).map_err(LiveHandlerError::NmReload)?;
        tracing::info!(
            reload_method = "custom",
            file_path = %intent.file_path.display(),
            "reloaded NetworkManager after writing unmanaged config"
        );
    }
    Ok(())
}

fn live_apply_nm_unmanaged_with_reloaders<D, F>(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedNmUnmanagedIntent,
    mut dbus_reload: D,
    mut fallback_reload: F,
) -> Result<Option<NmReloadMethod>, LiveHandlerError>
where
    D: FnMut() -> Result<(), String>,
    F: FnMut(&[&str]) -> Result<(), String>,
{
    executor
        .write_atomic_file(&intent.file_path, intent.contents.as_bytes(), intent.mode)
        .map_err(LiveHandlerError::ReconcileExec)?;
    if intent.reload_behavior != "atomic-reload" {
        return Ok(None);
    }
    match dbus_reload() {
        Ok(()) => Ok(Some(NmReloadMethod::Dbus)),
        Err(dbus_err) => {
            tracing::warn!(
                reload_method = "dbus",
                file_path = %intent.file_path.display(),
                error = %dbus_err,
                "NetworkManager DBus reload failed; falling back to systemctl"
            );
            fallback_reload(&["reload", "NetworkManager"]).map_err(|systemctl_err| {
                LiveHandlerError::NmReload(format!(
                    "dbus Reload(0) failed: {dbus_err}; systemctl fallback failed: {systemctl_err}"
                ))
            })?;
            Ok(Some(NmReloadMethod::SystemctlFallback))
        }
    }
}

/// W13 (W6-fu) live broker `UsbipBind` handler.
///
/// 1. Acquire `/run/nixling/locks/usbip/<bus_id>` for `vm_name`
///    (refuses if another VM already owns the busid).
/// 2. Run `usbip bind --busid <bus_id>` via the executor.
/// 3. On bind failure, release the lock (so a retried `UsbipBind`
///    from the same VM can succeed).
pub fn live_usbip_bind(
    executor: &dyn ReconcileExecutor,
    usbip_binary: &Path,
    bus_id: &str,
    lock_path: &Path,
    vm_name: &str,
    daemon_uid: u32,
    daemon_gid: u32,
) -> Result<(), LiveHandlerError> {
    crate::ops::usbip_lock::acquire_lock(lock_path, vm_name, daemon_uid, daemon_gid)
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?;
    if let Err(e) = executor.run_usbip(
        usbip_binary,
        crate::ops::exec_reconcile::UsbipSubcommand::Bind,
        bus_id,
    ) {
        let _ = crate::ops::usbip_lock::release_lock(lock_path, vm_name);
        return Err(LiveHandlerError::ReconcileExec(e));
    }
    Ok(())
}

/// W13 (W6-fu) live broker `UsbipUnbind` handler.
///
/// 1. Verify the lock's recorded owner matches `vm_name` BEFORE
///    touching usbip. This is the kernel-panel HIGH fix: pre-W14b
///    the unbind shellout ran first, so a stale-but-authenticated
///    request from VM A could detach VM B's device before the
///    owner-mismatch was caught at release_lock time.
/// 2. Run `usbip unbind --busid <bus_id>` via the executor.
/// 3. Release `/run/nixling/locks/usbip/<bus_id>` (re-verifies
///    owner; missing lock is idempotent).
pub fn live_usbip_unbind(
    executor: &dyn ReconcileExecutor,
    usbip_binary: &Path,
    bus_id: &str,
    lock_path: &Path,
    vm_name: &str,
) -> Result<(), LiveHandlerError> {
    if let Some(observed) = crate::ops::usbip_lock::peek_owner(lock_path) {
        if observed != vm_name {
            return Err(LiveHandlerError::UsbipLock(format!(
                "usbip unbind refused: bus_id={bus_id} lock at {} owned by {observed} but caller is {vm_name}",
                lock_path.display()
            )));
        }
    }
    // No lock present → idempotent unbind (already released). Still
    // run the shellout so the kernel state catches up.
    executor
        .run_usbip(
            usbip_binary,
            crate::ops::exec_reconcile::UsbipSubcommand::Unbind,
            bus_id,
        )
        .map_err(LiveHandlerError::ReconcileExec)?;
    crate::ops::usbip_lock::release_lock(lock_path, vm_name)
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?;
    Ok(())
}

/// W15 (W9-fu live host install): run the bundle-resolved
/// systemd unit install + `--enable` / `--start` flow against
/// the host's systemctl binary. Returns a typed response the
/// daemon echoes back to the operator.
///
/// W16 (W3 ifname unification): the W15 install path now also
/// writes the canonical `host-runtime.json` artifact (the broker's
/// view of the per-env / per-VM ifnames) so downstream consumers
/// can read it as a single source of truth.
pub fn live_run_host_install(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedInstallerIntent,
    enable: bool,
    start: bool,
    no_start: bool,
) -> Result<HostInstallOutcome, LiveHandlerError> {
    live_run_host_install_with_runtime(executor, intent, enable, start, no_start, None)
}

/// W16 variant that also accepts a host-runtime artifact to write
/// alongside the installer artifacts. The W12+W15 broker dispatch
/// path constructs the runtime from the loaded bundle resolver and
/// passes it through.
pub fn live_run_host_install_with_runtime(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedInstallerIntent,
    enable: bool,
    start: bool,
    no_start: bool,
    host_runtime: Option<&nixling_core::bundle_resolver::HostRuntimeArtifact>,
) -> Result<HostInstallOutcome, LiveHandlerError> {
    live_run_host_install_with_runtime_and_systemctl(
        executor,
        intent,
        enable,
        start,
        no_start,
        host_runtime,
        systemctl_invoke,
    )
}

fn live_run_host_install_with_runtime_and_systemctl<F>(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedInstallerIntent,
    enable: bool,
    start: bool,
    no_start: bool,
    host_runtime: Option<&nixling_core::bundle_resolver::HostRuntimeArtifact>,
    mut systemctl: F,
) -> Result<HostInstallOutcome, LiveHandlerError>
where
    F: FnMut(&[&str]) -> Result<(), String>,
{
    if start && no_start {
        return Err(LiveHandlerError::HostInstall(
            "--start and --no-start are mutually exclusive".to_owned(),
        ));
    }
    let mut artifacts_written = Vec::new();
    for artifact in &intent.artifacts {
        let parent = artifact.path.parent().ok_or_else(|| {
            LiveHandlerError::HostInstall(format!(
                "artifact path has no parent: {}",
                artifact.path.display()
            ))
        })?;
        ensure_dir_tree(parent, 0o755).map_err(|e| {
            LiveHandlerError::HostInstall(format!(
                "failed to create parent {} for artifact {}: {e}",
                parent.display(),
                artifact.path.display()
            ))
        })?;
        if !artifact.path.try_exists().map_err(|e| {
            LiveHandlerError::HostInstall(format!(
                "failed to inspect artifact {}: {e}",
                artifact.path.display()
            ))
        })? {
            let body = format!("# {}\n# {}\n", intent.service_name, artifact.purpose);
            executor
                .write_atomic_file(&artifact.path, body.as_bytes(), artifact.mode)
                .map_err(|e| {
                    LiveHandlerError::HostInstall(format!(
                        "failed to write artifact {}: {e}",
                        artifact.path.display()
                    ))
                })?;
        }
        artifacts_written.push(artifact.path.display().to_string());
    }

    // W16: write the host-runtime.json snapshot. Always overwrites
    // because the broker is the single writer and the artifact is
    // the runtime view of the bundle (it tracks the live install).
    if let Some(runtime) = host_runtime {
        let parent = runtime.path.parent().ok_or_else(|| {
            LiveHandlerError::HostInstall(format!(
                "host-runtime path has no parent: {}",
                runtime.path.display()
            ))
        })?;
        ensure_dir_tree(parent, 0o755).map_err(|e| {
            LiveHandlerError::HostInstall(format!(
                "failed to create host-runtime parent {}: {e}",
                parent.display()
            ))
        })?;
        let body = runtime
            .render_json()
            .map_err(|e| LiveHandlerError::HostInstall(format!("host-runtime render: {e}")))?;
        executor
            .write_atomic_file(&runtime.path, body.as_bytes(), 0o644)
            .map_err(|e| {
                LiveHandlerError::HostInstall(format!(
                    "failed to write host-runtime {}: {e}",
                    runtime.path.display()
                ))
            })?;
        artifacts_written.push(runtime.path.display().to_string());
    }

    if enable {
        systemctl(&["enable", &intent.service_name]).map_err(LiveHandlerError::HostInstall)?;
    }
    let started = if start && !no_start {
        systemctl(&["start", &intent.service_name]).map_err(LiveHandlerError::HostInstall)?;
        true
    } else {
        false
    };
    Ok(HostInstallOutcome {
        installed: true,
        enabled: enable,
        started,
        artifacts_written,
    })
}

#[derive(Debug, Clone)]
pub struct HostInstallOutcome {
    pub installed: bool,
    pub enabled: bool,
    pub started: bool,
    pub artifacts_written: Vec<String>,
}

fn ensure_dir_tree(path: &Path, mode: u32) -> std::io::Result<()> {
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
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_existing_dir(path, &metadata),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("directory path has no parent: {}", path.display()),
                )
            })?;
            ensure_dir_tree(parent, mode)?;
            if production_path(path) {
                crate::sys::path_safe::refuse_non_root_parent(path)?;
            }
            crate::sys::path_safe::refuse_world_writable_parent(path)?;
            crate::sys::path_safe::ensure_dir(path, mode, None, None).map(|_| ())
        }
        Err(err) => Err(err),
    }
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
    path.starts_with("/etc") || path.starts_with("/run") || path.starts_with("/var/lib/nixling")
}

fn read_required_single_line(path: &Path) -> Result<String, LiveHandlerError> {
    let contents = crate::sys::path_safe::read_to_string_nofollow(path).map_err(|e| {
        LiveHandlerError::HostKey(format!("failed to read {}: {e}", path.display()))
    })?;
    contents
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
        .ok_or_else(|| {
            LiveHandlerError::HostKey(format!(
                "{} did not contain a public key line",
                path.display()
            ))
        })
}

fn read_known_hosts_lines(path: &Path) -> Result<Vec<String>, LiveHandlerError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = crate::sys::path_safe::read_to_string_nofollow(path).map_err(|e| {
        LiveHandlerError::HostKey(format!("failed to read {}: {e}", path.display()))
    })?;
    Ok(contents.lines().map(str::to_owned).collect())
}

fn rewrite_known_hosts_lines(
    existing: &[String],
    static_ip: &str,
    replacement: Option<&str>,
) -> Vec<String> {
    let mut lines: Vec<String> = existing
        .iter()
        .filter(|line| !known_hosts_line_matches(line, static_ip))
        .cloned()
        .collect();
    if let Some(replacement) = replacement {
        lines.push(replacement.to_owned());
    }
    lines
}

fn known_hosts_line_matches(line: &str, static_ip: &str) -> bool {
    let host_field = line.split_whitespace().next().unwrap_or_default();
    let bracketed = format!("[{static_ip}]:22");
    host_field
        .split(',')
        .any(|host| host == static_ip || host == bracketed)
}

fn render_known_hosts_lines(lines: &[String]) -> Vec<u8> {
    if lines.is_empty() {
        Vec::new()
    } else {
        let mut rendered = lines.join("\n");
        rendered.push('\n');
        rendered.into_bytes()
    }
}

fn systemctl_invoke(args: &[&str]) -> Result<(), String> {
    use std::process::Command;

    let output = Command::new("/usr/bin/systemctl")
        .args(args)
        .output()
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

fn networkmanager_reload_via_dbus() -> Result<(), String> {
    use std::process::Command;

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
        .output()
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

fn read_host_runtime(path: &Path) -> Result<Option<HostRuntime>, ReconcileExecError> {
    let contents = match if path.is_absolute() {
        crate::sys::path_safe::read_to_string_nofollow(path)
    } else {
        std::fs::read_to_string(path)
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

pub(crate) fn read_host_runtime_nft_hash(
    path: &Path,
) -> Result<Option<String>, ReconcileExecError> {
    Ok(read_host_runtime(path)?.and_then(|runtime| runtime.nft_applied_hash))
}

pub(crate) fn update_host_runtime_nft_hash(
    path: &Path,
    nft_hash: Option<&str>,
) -> Result<(), ReconcileExecError> {
    if !path.is_absolute() {
        return Err(ReconcileExecError::InvalidInput {
            detail: format!("host-runtime path must be absolute: {}", path.display()),
        });
    }
    let mut runtime = read_host_runtime(path)?.ok_or_else(|| ReconcileExecError::Io {
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
    ensure_dir_tree(parent, 0o755).map_err(|err| ReconcileExecError::Io {
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

/// W15 (W9-fu migrate writer): execute the bundle-resolved
/// migration plan. Today writes a marker file recording the
/// migration record per VM; the daemon supervisor's pidfd-table
/// hand-off ships with the W4-fu-fu supervisor live wiring.
pub fn live_run_migrate(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedMigrateIntent,
) -> Result<MigrateOutcome, LiveHandlerError> {
    live_run_migrate_with_marker_dir(executor, intent, Path::new("/var/lib/nixling/migrate"))
}

fn live_run_migrate_with_marker_dir(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedMigrateIntent,
    marker_dir: &Path,
) -> Result<MigrateOutcome, LiveHandlerError> {
    ensure_dir_tree(marker_dir, 0o755).map_err(|e| {
        LiveHandlerError::HostInstall(format!(
            "failed to create migrate marker dir {}: {e}",
            marker_dir.display()
        ))
    })?;
    for vm in &intent.vms {
        let marker = marker_dir.join(format!("{vm}.json"));
        let body = serde_json::json!({
            "vm": vm,
            "migratedAt": chrono_like_utc_now_string(),
            "wave": "W15",
        });
        executor
            .write_atomic_file(&marker, body.to_string().as_bytes(), 0o644)
            .map_err(|e| {
                LiveHandlerError::HostInstall(format!(
                    "failed to write migrate marker {}: {e}",
                    marker.display()
                ))
            })?;
    }
    Ok(MigrateOutcome {
        migrated_vm_count: intent.vms.len() as u32,
        notes: intent.notes.clone(),
    })
}

#[derive(Debug, Clone)]
pub struct MigrateOutcome {
    pub migrated_vm_count: u32,
    pub notes: Vec<String>,
}

fn chrono_like_utc_now_string() -> String {
    // Avoid pulling in chrono for one timestamp; derive from SystemTime.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix-{secs}")
}

/// W13 (W6-fu) live broker `UsbipProxyReconcile` handler.
///
/// Walks each (bus_id, vm_name, lock_path) tuple from the trusted
/// bundle and asserts the lockfile (if present) records the
/// expected owner. Returns the first mismatch encountered, or `Ok`
/// if every present lock matches expected ownership. Missing locks
/// are treated as "not bound" (no-op).
///
/// W13 deliberately does NOT auto-rebind: reconcile-after-restart
/// is the daemon's responsibility (the daemon classifies each
/// existing claim against the bundle and either issues `UsbipBind`
/// or `UsbipUnbind` to resolve drift). This handler is the
/// validation half.
pub fn live_usbip_proxy_reconcile(
    expectations: &[(String, String, std::path::PathBuf)],
) -> Result<(), LiveHandlerError> {
    for (bus_id, vm_name, lock_path) in expectations {
        if let Some(observed) = crate::ops::usbip_lock::peek_owner(lock_path) {
            if &observed != vm_name {
                return Err(LiveHandlerError::UsbipLock(format!(
                    "usbip proxy reconcile: bus_id={bus_id} lock at {} owned by {observed} but bundle expects {vm_name}",
                    lock_path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Result of [`live_spawn_runner`].
#[derive(Debug)]
pub struct SpawnRunnerResult {
    pub pidfd: OwnedFd,
    pub pid: i32,
    pub start_time_ticks: u64,
    /// True if the broker fell back to `fork(2)` + `pidfd_open(2)`
    /// instead of the preferred `clone3(CLONE_PIDFD)`. Used for
    /// audit-record bookkeeping.
    pub used_fork_fallback: bool,
}

fn parse_runner_cgroup_subtree(
    subtree: &str,
) -> Result<Option<(String, Vec<String>)>, LiveHandlerError> {
    if subtree.is_empty() {
        return Ok(None);
    }
    let normalized = subtree
        .strip_prefix("nixling.slice/")
        .or_else(|| subtree.strip_prefix("nixling/"))
        .ok_or_else(|| LiveHandlerError::SpawnFailed {
            detail: format!("unsupported cgroup subtree root: {subtree}"),
        })?;
    let mut segments = normalized.split('/').filter(|segment| !segment.is_empty());
    let vm = segments
        .next()
        .ok_or_else(|| LiveHandlerError::SpawnFailed {
            detail: format!("missing vm segment in cgroup subtree: {subtree}"),
        })?;
    if vm.contains('\0') {
        return Err(LiveHandlerError::SpawnFailed {
            detail: format!("invalid NUL byte in cgroup subtree vm segment: {subtree}"),
        });
    }
    let mut role_segments = Vec::new();
    for segment in segments {
        if segment.contains('\0') {
            return Err(LiveHandlerError::SpawnFailed {
                detail: format!("invalid NUL byte in cgroup subtree segment: {subtree}"),
            });
        }
        role_segments.push(segment.to_owned());
    }
    Ok(Some((
        vm.trim_end_matches(".scope").to_owned(),
        role_segments,
    )))
}

fn cgroup_leaf_path(parent_slice: &Path, vm: &str, role_segments: &[String]) -> PathBuf {
    let mut path = parent_slice.to_path_buf();
    path.push(format!("{vm}.scope"));
    for segment in role_segments {
        path.push(segment);
    }
    path
}

fn ensure_runner_cgroup_leaf<B: nixling_host::cgroup::CgroupBackend>(
    backend: &B,
    placement: &CgroupPlacement,
    unified_hierarchy_root: &Path,
    parent_slice: &Path,
    uid: u32,
    gid: u32,
) -> Result<Option<PathBuf>, LiveHandlerError> {
    use nixling_host::cgroup::create_vm_subtree;

    let Some((vm, role_segments)) = parse_runner_cgroup_subtree(&placement.subtree)? else {
        return Ok(None);
    };
    let leaf_path = cgroup_leaf_path(parent_slice, &vm, &role_segments);
    if placement.delegated {
        let slice = crate::ops::cgroup::create_nixling_slice(
            backend,
            unified_hierarchy_root,
            parent_slice,
            uid,
            gid,
        )
        .map_err(|err| LiveHandlerError::SpawnFailed {
            detail: format!("delegate cgroup slice: {err}"),
        })?;
        let mut cursor = create_vm_subtree(backend, &slice, &vm, uid, gid).map_err(|err| {
            LiveHandlerError::SpawnFailed {
                detail: format!("delegate cgroup subtree for {vm}: {err}"),
            }
        })?;
        for segment in &role_segments {
            cursor.push(segment);
            if !backend.exists(&cursor) {
                backend
                    .mkdir(&cursor)
                    .map_err(|err| LiveHandlerError::SpawnFailed {
                        detail: format!("create cgroup leaf {}: {err}", cursor.display()),
                    })?;
            }
            backend
                .fchown(&cursor, uid, gid)
                .map_err(|err| LiveHandlerError::SpawnFailed {
                    detail: format!("chown cgroup leaf {}: {err}", cursor.display()),
                })?;
        }
    }
    Ok(Some(leaf_path))
}

fn prepare_runner_cgroup_fd(
    placement: &CgroupPlacement,
) -> Result<Option<OwnedFd>, LiveHandlerError> {
    use nixling_host::cgroup::RealCgroupBackend;
    use rustix::fs::{open, Mode, OFlags};

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
    )? else {
        return Ok(None);
    };
    let fd = open(
        leaf_path.join("cgroup.procs"),
        OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|err| LiveHandlerError::SpawnFailed {
        detail: format!("open {}: {err}", leaf_path.join("cgroup.procs").display()),
    })?;
    Ok(Some(fd))
}

fn load_runner_seccomp(
    plan: &SpawnRunnerPlan,
) -> Result<Option<crate::sys::pidfd_sys::SeccompProgram>, LiveHandlerError> {
    match plan.seccomp_policy_ref.as_deref() {
        Some(policy_path) if Path::new(policy_path).is_absolute() => {
            crate::sys::pidfd_sys::load_seccomp_program(Path::new(policy_path))
                .map(Some)
                .map_err(|err| LiveHandlerError::SpawnFailed {
                    detail: format!("load seccomp program {policy_path}: {err}"),
                })
        }
        Some(policy_ref) => {
            tracing::debug!(
                seccomp_policy_ref = %policy_ref,
                "spawn runner seccomp policy ref not resolved to an absolute path; skipping filter load"
            );
            Ok(None)
        }
        None => Ok(None),
    }
}

/// W4-fu live broker `SpawnRunner` handler.
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
pub fn live_spawn_runner(
    plan_input: &SpawnRunnerPlanInput,
) -> Result<SpawnRunnerResult, LiveHandlerError> {
    let plan = preflight(plan_input).map_err(LiveHandlerError::SpawnPreflight)?;
    let (binary, argv, env) =
        build_cstring_vectors(&plan).map_err(LiveHandlerError::SpawnPreflight)?;
    let seccomp_program = load_runner_seccomp(&plan)?;
    let cgroup_procs_fd = prepare_runner_cgroup_fd(&plan.cgroup_placement)?;
    let isolation = crate::sys::pidfd_sys::RunnerIsolationSpec {
        capabilities: plan.capabilities.clone(),
        namespaces: plan.namespaces.clone(),
        seccomp_program,
        mount_policy: plan.mount_policy.clone(),
        cgroup_procs_fd,
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

    let start_time_ticks =
        crate::sys::pidfd_sys::read_proc_stat_start_time(outcome.pid).map_err(|e| {
            LiveHandlerError::ProcStatReadFailed {
                pid: outcome.pid,
                detail: e.to_string(),
            }
        })?;

    Ok(SpawnRunnerResult {
        pidfd: outcome.pidfd,
        pid: outcome.pid,
        start_time_ticks,
        used_fork_fallback: outcome.used_fork_fallback,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};
    use nixling_core::bundle_resolver::{
        HostRuntime, HostRuntimeArtifact, HostRuntimeIfName, InstallerArtifact,
        ResolvedActivationIntent, ResolvedGcIntent, ResolvedHostKeyTrustIntent,
        ResolvedInstallerIntent, ResolvedKeysRotateIntent, ResolvedMigrateIntent,
        ResolvedNmUnmanagedIntent, ResolvedRotateKnownHostIntent,
    };
    use nixling_core::minijail_profile::{
        CgroupPlacement, MountPolicy, NamespaceSet, WritablePath,
    };
    use nixling_host::cgroup::fake::FakeCgroupBackend;
    use nixling_ipc::broker_wire::ActivationMode;
    use std::path::{Path, PathBuf};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
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
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sample_installer_intent(root: &TestDir) -> ResolvedInstallerIntent {
        ResolvedInstallerIntent {
            intent_id: "installer-host".to_owned(),
            unit_path: root.join("nixlingd.service"),
            service_name: "nixlingd.service".to_owned(),
            daemon_config_path: root.join("daemon-config.json"),
            bundle_path: root.join("bundle.json"),
            artifacts: vec![
                InstallerArtifact {
                    path: root.join("nixlingd.service"),
                    mode: 0o644,
                    purpose: "nixlingd systemd unit".to_owned(),
                },
                InstallerArtifact {
                    path: root.join("daemon-config.json"),
                    mode: 0o640,
                    purpose: "daemon configuration".to_owned(),
                },
            ],
        }
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

    fn sample_activation_intent(root: &TestDir) -> ResolvedActivationIntent {
        ResolvedActivationIntent {
            intent_id: "activation:vm:alpha".to_owned(),
            vm: "alpha".to_owned(),
            target_generation_path: root.join("generation"),
            generation_number: Some(42),
        }
    }

    fn sample_nm_unmanaged_intent(root: &TestDir) -> ResolvedNmUnmanagedIntent {
        ResolvedNmUnmanagedIntent {
            intent_id: "nm-unmanaged:work".to_owned(),
            file_path: root.join("00-nixling-unmanaged.conf"),
            contents: concat!(
                "# nixling-managed begin\n",
                "[keyfile]\n",
                "unmanaged-devices=interface-name:nl-*\n",
                "# nixling-managed end\n"
            )
            .to_owned(),
            mode: 0o644,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            reload_behavior: "atomic-reload".to_owned(),
        }
    }

    fn sample_store_view_intent(root: &TestDir) -> ResolvedStoreViewIntent {
        let source_view = root.join("source-view/alpha-system");
        std::fs::create_dir_all(source_view.join("bin")).unwrap();
        std::fs::write(
            source_view.join("bin/switch-to-configuration"),
            b"#!/bin/sh\n",
        )
        .unwrap();
        ResolvedStoreViewIntent {
            intent_id: "store-view:vm:alpha".to_owned(),
            vm: "alpha".to_owned(),
            generation: 42,
            hardlink_farm_path: root.join("store-view"),
            target_view_path: root.join("store-view/generations/42/alpha-system"),
            closure_paths: vec![source_view],
        }
    }

    fn sample_gc_intent() -> ResolvedGcIntent {
        ResolvedGcIntent {
            intent_id: "gc:host".to_owned(),
            retained_store_paths: vec![
                PathBuf::from("/nix/store/aaaaaaaaaaaaaaaa-alpha"),
                PathBuf::from("/nix/store/bbbbbbbbbbbbbbbb-beta"),
            ],
        }
    }

    fn sample_keys_rotate_intent(root: &TestDir) -> ResolvedKeysRotateIntent {
        ResolvedKeysRotateIntent {
            intent_id: "keys-rotate:vm:alpha".to_owned(),
            vm: "alpha".to_owned(),
            key_path: root.join("alpha_ed25519"),
        }
    }

    fn sample_host_key_trust_intent(root: &TestDir) -> ResolvedHostKeyTrustIntent {
        ResolvedHostKeyTrustIntent {
            intent_id: "trust:vm:alpha".to_owned(),
            vm: "alpha".to_owned(),
            static_ip: "10.20.0.10".to_owned(),
            known_hosts_path: root.join("known_hosts.nixling"),
            host_public_key_path: root.join("ssh_host_ed25519_key.pub"),
        }
    }

    fn sample_rotate_known_host_intent(root: &TestDir) -> ResolvedRotateKnownHostIntent {
        ResolvedRotateKnownHostIntent {
            intent_id: "rotate-known-host:vm:alpha".to_owned(),
            vm: "alpha".to_owned(),
            static_ip: "10.20.0.10".to_owned(),
            known_hosts_path: root.join("known_hosts.nixling"),
        }
    }

    #[test]
    fn live_apply_nftables_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_apply_nftables(&exec, Path::new("/usr/sbin/nft"), "table inet nixling {}").unwrap();
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::ApplyNftScript { binary, script } => {
                assert!(binary.ends_with("nft"));
                assert!(script.contains("inet nixling"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn live_apply_sysctl_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_apply_sysctl(&exec, "net.ipv4.ip_forward", "1").unwrap();
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteSysctl { key, value }
                if key == "net.ipv4.ip_forward" && value == "1"
        ));
    }

    #[test]
    fn live_update_hosts_file_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_update_hosts_file(
            &exec,
            Path::new("/etc/hosts"),
            b"127.0.0.1 localhost\n",
            0o644,
        )
        .unwrap();
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFile { mode: 0o644, .. }
        ));
    }

    #[test]
    fn live_apply_route_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        live_apply_route(
            &exec,
            Path::new("/usr/sbin/ip"),
            IpRouteVerb::Add,
            "10.0.0.0/24 dev tap0",
        )
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

    #[test]
    fn live_run_activation_sequences_native_steps() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("live-run-activation");
        let intent = sample_activation_intent(&root);
        let store_view_intent = sample_store_view_intent(&root);
        let outcome =
            live_run_activation(&exec, &intent, &store_view_intent, ActivationMode::Switch)
                .unwrap();
        assert_eq!(outcome.vm, "alpha");
        assert_eq!(outcome.generation_number, Some(42));
        assert_eq!(outcome.current_generation_updated, Some(42));
        assert_eq!(outcome.activation_script_mode, "switch");
        assert_eq!(
            std::fs::read_link(store_view_intent.hardlink_farm_path.join("current")).unwrap(),
            PathBuf::from("generations/42")
        );
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::PrepareStoreView { vm, generation, .. }
                if vm == "alpha" && *generation == 42
        ));
        assert!(matches!(
            &log[1],
            ReconcileOp::SetupMountNamespace { vm, role_id, .. }
                if vm == "alpha" && role_id == "activation"
        ));
        assert!(matches!(
            &log[2],
            ReconcileOp::RunActivationScript { mode_arg, .. } if mode_arg == "switch"
        ));
    }

    #[test]
    fn live_run_gc_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        let intent = sample_gc_intent();
        let outcome = live_run_gc(&exec, &intent, Some(2)).unwrap();
        assert_eq!(outcome.retained_store_path_count, 2);
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::RunGc {
                keep_generations: Some(2)
            }
        ));
    }

    #[test]
    fn live_run_keys_rotate_drives_executor() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("live-run-keys-rotate");
        let intent = sample_keys_rotate_intent(&root);
        let outcome = live_run_keys_rotate(&exec, &intent).unwrap();
        assert_eq!(outcome.vm, "alpha");
        assert!(outcome.public_key_fingerprint.starts_with("SHA256:fake:"));
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::RunSshKeygen { key_path, comment }
                if key_path == &intent.key_path && comment == "nixling:alpha"
        ));
    }

    #[test]
    fn live_run_trust_rewrites_known_hosts_atomically() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("live-run-trust");
        let intent = sample_host_key_trust_intent(&root);
        std::fs::write(
            &intent.host_public_key_path,
            b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKE alpha@nixling\n",
        )
        .expect("write host pubkey");
        std::fs::write(
            &intent.known_hosts_path,
            b"10.20.0.10 ssh-ed25519 AAAAOLD alpha@nixling\n192.0.2.1 ssh-ed25519 AAAAOTHER beta@nixling\n",
        )
        .expect("seed known_hosts");
        let outcome = live_run_trust(&exec, &intent).unwrap();
        assert!(outcome.updated);
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFile { path, mode: 0o644, contents }
                if path == &intent.known_hosts_path
                    && String::from_utf8_lossy(contents).contains("10.20.0.10 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKE alpha@nixling")
                    && String::from_utf8_lossy(contents).contains("192.0.2.1 ssh-ed25519 AAAAOTHER beta@nixling")
        ));
    }

    #[test]
    fn live_run_rotate_known_host_rewrites_known_hosts_atomically() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("live-run-rotate-known-host");
        let intent = sample_rotate_known_host_intent(&root);
        std::fs::write(
            &intent.known_hosts_path,
            b"10.20.0.10 ssh-ed25519 AAAAOLD alpha@nixling\n192.0.2.1 ssh-ed25519 AAAAOTHER beta@nixling\n",
        )
        .expect("seed known_hosts");
        let outcome = live_run_rotate_known_host(&exec, &intent).unwrap();
        assert!(outcome.removed);
        let log = exec.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFile { path, mode: 0o644, contents }
                if path == &intent.known_hosts_path
                    && !String::from_utf8_lossy(contents).contains("10.20.0.10 ssh-ed25519 AAAAOLD alpha@nixling")
                    && String::from_utf8_lossy(contents).contains("192.0.2.1 ssh-ed25519 AAAAOTHER beta@nixling")
        ));
    }

    /// A failing reconcile executor surfaces ReconcileExec
    /// LiveHandlerError variant.
    #[test]
    fn live_apply_propagates_executor_error() {
        struct FailExec;
        impl ReconcileExecutor for FailExec {
            fn apply_nft_script(
                &self,
                _nft: &Path,
                _script: &str,
            ) -> Result<(), ReconcileExecError> {
                Err(ReconcileExecError::NonZeroExit {
                    which: "nft".to_owned(),
                    exit_code: 1,
                    stderr: "fail".to_owned(),
                })
            }
            fn write_sysctl(&self, _: &str, _: &str) -> Result<(), ReconcileExecError> {
                unreachable!()
            }
            fn write_atomic_file(
                &self,
                _: &Path,
                _: &[u8],
                _: u32,
            ) -> Result<(), ReconcileExecError> {
                unreachable!()
            }
            fn write_path_value(&self, _: &Path, _: &str) -> Result<(), ReconcileExecError> {
                unreachable!()
            }
            fn read_path_value(&self, _: &Path) -> Result<String, ReconcileExecError> {
                unreachable!()
            }
            fn ip_route(
                &self,
                _: &Path,
                _: IpRouteVerb,
                _: &str,
            ) -> Result<(), ReconcileExecError> {
                unreachable!()
            }
            fn run_usbip(
                &self,
                _: &Path,
                _: crate::ops::exec_reconcile::UsbipSubcommand,
                _: &str,
            ) -> Result<(), ReconcileExecError> {
                unreachable!()
            }
            fn prepare_store_view(
                &self,
                _: &ResolvedStoreViewIntent,
            ) -> Result<(), ReconcileExecError> {
                unreachable!()
            }
            fn setup_mount_namespace(
                &self,
                _: &str,
                _: &str,
                _: &Path,
                _: &Path,
            ) -> Result<PathBuf, ReconcileExecError> {
                unreachable!()
            }
            fn run_activation_script(
                &self,
                _: &str,
                _: &Path,
                _: &Path,
            ) -> Result<String, ReconcileExecError> {
                unreachable!()
            }
            fn run_gc(&self, _: Option<u32>) -> Result<String, ReconcileExecError> {
                unreachable!()
            }
            fn run_ssh_keygen(
                &self,
                _: &Path,
                _: &str,
            ) -> Result<crate::ops::exec_reconcile::GeneratedSshKey, ReconcileExecError>
            {
                unreachable!()
            }
        }
        let err = live_apply_nftables(&FailExec, Path::new("/usr/sbin/nft"), "x").unwrap_err();
        assert!(matches!(err, LiveHandlerError::ReconcileExec(_)));
    }

    #[test]
    fn live_run_host_install_propagates_systemctl_failure() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("live-run-host-install");
        let intent = sample_installer_intent(&root);
        let runtime = sample_host_runtime(root.join("host-runtime.json"));
        let calls = std::cell::RefCell::new(Vec::new());

        let err = live_run_host_install_with_runtime_and_systemctl(
            &exec,
            &intent,
            true,
            true,
            false,
            Some(&runtime),
            |args| {
                calls.borrow_mut().push(args.join(" "));
                Err("systemctl enable failed: nixlingd.service: permission denied".to_owned())
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            LiveHandlerError::HostInstall(ref detail)
                if detail == "systemctl enable failed: nixlingd.service: permission denied"
        ));
        assert_eq!(
            calls.into_inner(),
            vec!["enable nixlingd.service".to_owned()]
        );

        let log = exec.take_log();
        assert_eq!(
            log.iter()
                .filter(|op| matches!(op, ReconcileOp::WriteAtomicFile { .. }))
                .count(),
            3
        );
        assert!(log.iter().any(|op| matches!(
            op,
            ReconcileOp::WriteAtomicFile {
                path,
                mode: 0o644,
                contents,
            } if path == &intent.artifacts[0].path
                && String::from_utf8_lossy(contents).contains("# nixlingd.service")
        )));
        assert!(log.iter().any(|op| matches!(
            op,
            ReconcileOp::WriteAtomicFile {
                path,
                mode: 0o640,
                contents,
            } if path == &intent.artifacts[1].path
                && String::from_utf8_lossy(contents).contains("daemon configuration")
        )));
        assert!(log.iter().any(|op| matches!(
            op,
            ReconcileOp::WriteAtomicFile {
                path,
                mode: 0o644,
                contents,
            } if path == &runtime.path
                && String::from_utf8_lossy(contents).contains("\"schemaVersion\": \"v2\"")
        )));
    }

    #[test]
    fn live_run_migrate_uses_atomic_write() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("live-run-migrate");
        let marker_dir = root.join("markers");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        let existing_marker = marker_dir.join("alpha.json");
        std::fs::write(&existing_marker, b"").expect("seed empty marker");
        let intent = ResolvedMigrateIntent {
            intent_id: "migrate-host".to_owned(),
            vms: vec!["alpha".to_owned(), "beta".to_owned()],
            notes: vec!["rewrite markers".to_owned()],
        };

        let outcome = live_run_migrate_with_marker_dir(&exec, &intent, &marker_dir).unwrap();
        assert_eq!(outcome.migrated_vm_count, 2);

        let log = exec.take_log();
        assert_eq!(log.len(), 2);
        assert!(log.iter().any(|op| matches!(
            op,
            ReconcileOp::WriteAtomicFile {
                path,
                mode: 0o644,
                contents,
            } if path == &existing_marker
                && String::from_utf8_lossy(contents).contains("\"vm\":\"alpha\"")
        )));
        assert!(log.iter().any(|op| matches!(
            op,
            ReconcileOp::WriteAtomicFile {
                path,
                mode: 0o644,
                contents,
            } if path == &marker_dir.join("beta.json")
                && String::from_utf8_lossy(contents).contains("\"vm\":\"beta\"")
        )));
    }

    #[test]
    fn live_apply_nm_unmanaged_prefers_dbus_reload() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-dbus");
        let intent = sample_nm_unmanaged_intent(&root);
        let dbus_calls = std::cell::Cell::new(0);
        let fallback_calls = std::cell::RefCell::new(Vec::new());

        let method = live_apply_nm_unmanaged_with_reloaders(
            &exec,
            &intent,
            || {
                dbus_calls.set(dbus_calls.get() + 1);
                Ok(())
            },
            |args| {
                fallback_calls.borrow_mut().push(args.join(" "));
                Ok(())
            },
        )
        .expect("nm unmanaged apply succeeds");

        assert_eq!(method, Some(NmReloadMethod::Dbus));
        assert_eq!(dbus_calls.get(), 1);
        assert!(fallback_calls.borrow().is_empty());
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFile {
                path,
                mode: 0o644,
                contents,
            } if path == &intent.file_path
                && contents.as_slice() == intent.contents.as_bytes()
        ));
    }

    #[test]
    fn live_apply_nm_unmanaged_falls_back_to_systemctl() {
        let exec = FakeReconcileExecutor::new();
        let root = TestDir::new("nm-unmanaged-fallback");
        let intent = sample_nm_unmanaged_intent(&root);
        let fallback_calls = std::cell::RefCell::new(Vec::new());

        let method = live_apply_nm_unmanaged_with_reloaders(
            &exec,
            &intent,
            || Err("dbus unavailable".to_owned()),
            |args| {
                fallback_calls.borrow_mut().push(args.join(" "));
                Ok(())
            },
        )
        .expect("systemctl fallback succeeds");

        assert_eq!(method, Some(NmReloadMethod::SystemctlFallback));
        assert_eq!(
            fallback_calls.into_inner(),
            vec!["reload NetworkManager".to_owned()]
        );
    }

    #[test]
    fn update_host_runtime_nft_hash_rewrites_runtime_json() {
        let root = TestDir::new("host-runtime-nft-hash");
        let runtime = sample_host_runtime(root.join("host-runtime.json"));
        std::fs::create_dir_all(runtime.path.parent().expect("runtime parent")).unwrap();
        std::fs::write(
            &runtime.path,
            runtime.render_json().expect("render runtime"),
        )
        .unwrap();

        update_host_runtime_nft_hash(&runtime.path, Some("0123456789abcdef"))
            .expect("update host runtime hash");

        assert_eq!(
            read_host_runtime_nft_hash(&runtime.path).expect("read host runtime hash"),
            Some("0123456789abcdef".to_owned())
        );
        let updated = std::fs::read_to_string(&runtime.path).expect("read updated runtime");
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
                path: "/var/lib/nixling/vms/test".to_owned(),
                purpose: "test".to_owned(),
            }],
            nix_store_read_only: false,
            hide_device_nodes_by_default: false,
        }
    }

    fn test_cgroup_placement() -> CgroupPlacement {
        CgroupPlacement {
            subtree: String::new(),
            controllers: vec![],
            delegated: false,
        }
    }

    #[test]
    fn ensure_runner_cgroup_leaf_uses_delegated_parent_slice() {
        let backend = FakeCgroupBackend::new(1000);
        let root = Path::new("/sys/fs/cgroup");
        let parent = Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE);
        backend.seed_unified(root);
        nixling_host::cgroup::CgroupBackend::mkdir(&backend, parent)
            .expect("seed delegated parent slice");
        let placement = CgroupPlacement {
            subtree: "nixling.slice/personal-dev/virtiofsd-ro-store".to_owned(),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: true,
        };

        let leaf = ensure_runner_cgroup_leaf(&backend, &placement, root, parent, 1000, 1000)
            .expect("prepare delegated cgroup leaf")
            .expect("leaf path");

        assert_eq!(
            leaf,
            Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE)
                .join("personal-dev.scope")
                .join("virtiofsd-ro-store")
        );
        assert!(backend.directory_exists(&leaf));
        assert!(!backend.directory_exists(Path::new("/sys/fs/cgroup/nixling.slice")));
        assert_eq!(
            backend
                .file_contents(&root.join("cgroup.subtree_control"))
                .as_deref(),
            Some("")
        );
    }

    /// live_spawn_runner preflight failure surfaces SpawnPreflight.
    #[test]
    fn live_spawn_runner_propagates_preflight_error() {
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
        };
        let err = live_spawn_runner(&plan).unwrap_err();
        assert!(matches!(err, LiveHandlerError::SpawnPreflight(_)));
    }

    #[test]
    fn live_spawn_runner_applies_capability_and_net_namespace_when_privileged() {
        if rustix::process::geteuid().as_raw() != 0 {
            eprintln!("skipping privileged SpawnRunner isolation test: requires euid 0");
            return;
        }

        let dir = TestDir::new("spawn-runner-netns");
        let status_path = dir.join("status.txt");
        let netns_path = dir.join("netns.txt");
        let current_netns = std::fs::read_link("/proc/self/ns/net")
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
            },
            cgroup_placement: test_cgroup_placement(),
            root_carve_out: true,
            skip_binary_exists_check: false,
        };

        let outcome = live_spawn_runner(&plan).expect("spawn privileged test child");
        let wait_status = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(outcome.pid), None)
            .expect("wait for test child");
        assert!(matches!(
            wait_status,
            nix::sys::wait::WaitStatus::Exited(_, 0)
        ));

        let status = std::fs::read_to_string(&status_path).expect("read child status");
        let netns = std::fs::read_to_string(&netns_path).expect("read child netns");
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
}
