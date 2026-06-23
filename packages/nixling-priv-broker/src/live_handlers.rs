//! Live broker request handlers.
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
//! The `pidfd_open` and `clone3` paths require a live kernel and are
//! exercised by the broker integration tests (broker-pidfd-adopt-roundtrip.sh
//! and broker-spawn-runner-smoke.sh).

use std::fs::{self, File};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ops::exec_reconcile::{
    GeneratedSshKey, IpRouteVerb, ReconcileExecError, ReconcileExecutor,
};
use crate::ops::spawn_runner::{
    SpawnRunnerError, SpawnRunnerPlan, SpawnRunnerPlanInput, build_cstring_vectors, preflight,
};
use nixling_core::bundle_resolver::{
    HostRuntime, ResolvedActivationIntent, ResolvedStoreViewIntent,
};
use nixling_core::minijail_profile::CgroupPlacement;
use nixling_host::hardlink_farm;
use nixling_ipc::broker_wire::ActivationMode;
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
/// This closes the critical pid-reuse race — the daemon's pre-call
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
pub fn live_apply_nftables(
    executor: &dyn ReconcileExecutor,
    nft_binary: &Path,
    nft_script: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .apply_nft_script(nft_binary, nft_script)
        .map_err(LiveHandlerError::ReconcileExec)
}

/// Live broker `ApplySysctl` handler.
pub fn live_apply_sysctl(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
) -> Result<(), LiveHandlerError> {
    executor
        .write_sysctl(key, value)
        .map_err(LiveHandlerError::ReconcileExec)
}

/// Live broker `UpdateHostsFile` handler. Atomic write with fsync via
/// the executor.
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

/// Live broker `ApplyRoute` handler.
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
    if let Some(generation_number) = intent.generation_number
        && generation_number != store_view_intent.generation
    {
        return Err(LiveHandlerError::Activation(format!(
            "activation/store-view generation mismatch: activation={} store-view={}",
            generation_number, store_view_intent.generation,
        )));
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
            let rollback_marker = hardlink_farm::read_generation_marker(&generation_dir)
                .map_err(|err| LiveHandlerError::Activation(err.to_string()))?;
            // The rolled-back generation may hold a DIFFERENT closure
            // than the current bundle intent, so the toplevel basename
            // for the target view must come from the rollback
            // generation's own marker — NOT the current intent's
            // basename (which would point at a non-existent
            // `generations/<old-gen>/<current-basename>`).
            let target_view_path = rollback_target_view_path(
                store_view_intent,
                rollback_generation,
                &rollback_marker,
            )?;
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

/// Resolve the target store-view path for a ROLLBACK to `generation`.
///
/// Unlike a forward activation, the rolled-back generation may hold a
/// different closure than the current bundle intent, so the toplevel
/// basename is recovered from that generation's own marker
/// (`closure_hash == "toplevel:<basename>"`, written by
/// [`ResolvedStoreViewIntent::closure_identity`]). Falls back to the
/// current intent's basename only for a marker written before the
/// `toplevel:` identity format (defensive; such markers never reached
/// activation in practice because store-view intents were absent).
fn rollback_target_view_path(
    intent: &ResolvedStoreViewIntent,
    generation: u64,
    marker: &hardlink_farm::GenerationMarker,
) -> Result<PathBuf, LiveHandlerError> {
    let dir = generation_dir(&intent.hardlink_farm_path, generation);
    if let Some(basename) = marker.closure_hash.strip_prefix("toplevel:") {
        // Must be a single, non-traversing path component (a Nix store
        // basename). Reject anything with separators / `.` / `..` so a
        // malformed marker can never escape the generation dir.
        if !basename.is_empty() && !basename.contains('/') && basename != "." && basename != ".." {
            return Ok(dir.join(basename));
        }
    }
    target_view_for_generation(intent, generation)
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

/// Live broker `ApplyNmUnmanaged` handler.
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

/// Live broker `UsbipBind` handler.
///
/// 1. Acquire `/run/nixling/locks/usbip/<bus_id>` for `vm_name`
///    (refuses if another VM already owns the busid).
/// 2. If sysfs already reports `usbip-host`, treat same-VM replay as
///    converged without shelling out.
/// 3. Otherwise run `usbip bind --busid <bus_id>` via the executor and
///    verify sysfs converged to `usbip-host`. On bind failure, release
///    the lock so a retried `UsbipBind` from the same VM can succeed.
pub fn live_usbip_bind(
    executor: &dyn ReconcileExecutor,
    usbip_binary: &Path,
    sysfs_root: &Path,
    bus_id: &str,
    lock_path: &Path,
    vm_name: &str,
    daemon_uid: u32,
    daemon_gid: u32,
) -> Result<(), LiveHandlerError> {
    crate::ops::usbip_lock::acquire_lock(lock_path, vm_name, daemon_uid, daemon_gid)
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?;
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?
    {
        crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost => {
            return Ok(());
        }
        crate::ops::usbip_host::UsbipDriverBinding::Unbound
        | crate::ops::usbip_host::UsbipDriverBinding::BoundToOtherDriver { .. } => {}
    }
    if let Err(e) = executor.run_usbip(
        usbip_binary,
        crate::ops::exec_reconcile::UsbipSubcommand::Bind,
        bus_id,
    ) {
        let _ = crate::ops::usbip_lock::release_lock(lock_path, vm_name);
        return Err(LiveHandlerError::ReconcileExec(e));
    }
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?
    {
        crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost => Ok(()),
        observed => Err(LiveHandlerError::UsbipLock(format!(
            "usbip bind did not converge to usbip-host for bus_id={bus_id}: observed {observed:?}"
        ))),
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
///    broker/nixlingd control path live.
/// 3. Leave `/run/nixling/locks/usbip/<bus_id>` in place. The dispatch layer
///    revokes the backend device ACL after successful unbind, then releases the
///    host-session claim last. Timeout/failure deliberately preserves the claim for
///    operator recovery.
pub fn live_usbip_unbind(
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
        .map_err(|e| LiveHandlerError::UsbipLock(e.to_string()))?
    {
        crate::ops::usbip_host::UsbipDriverBinding::Unbound => return Ok(()),
        crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost => {
            crate::ops::usbip_host::ensure_usbip_host_driver_unbind_supported(sysfs_root)
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
        .map_err(LiveHandlerError::ReconcileExec)?;
    executor
        .wait_usbip_stream_fd_release(sysfs_root, bus_id)
        .map_err(LiveHandlerError::ReconcileExec)?;
    executor
        .run_usbip(
            usbip_binary,
            crate::ops::exec_reconcile::UsbipSubcommand::Unbind,
            bus_id,
        )
        .map_err(LiveHandlerError::ReconcileExec)?;
    match crate::ops::usbip_host::inspect_usbip_driver_binding(sysfs_root, bus_id)
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

/// Run the bundle-resolved systemd unit install + `--enable` /
/// `--start` flow against the host's systemctl binary. Returns a typed
/// response the daemon echoes back to the operator.
///
/// The install path also writes the canonical `host-runtime.json`
/// artifact (the broker's view of the per-env / per-VM ifnames) so
/// downstream consumers can read it as a single source of truth.
pub fn live_run_host_install(
    executor: &dyn ReconcileExecutor,
    intent: &nixling_core::bundle_resolver::ResolvedInstallerIntent,
    enable: bool,
    start: bool,
    no_start: bool,
) -> Result<HostInstallOutcome, LiveHandlerError> {
    live_run_host_install_with_runtime(executor, intent, enable, start, no_start, None)
}

/// Variant that also accepts a host-runtime artifact to write alongside
/// the installer artifacts. The broker dispatch path constructs the
/// runtime from the loaded bundle resolver and passes it through.
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

    // Write the host-runtime.json snapshot. Always overwrites because
    // the broker is the single writer and the artifact is the runtime
    // view of the bundle (it tracks the live install).
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
        .env_remove("NOTIFY_SOCKET")
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
        .env_remove("NOTIFY_SOCKET")
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

/// Execute the bundle-resolved migration plan. Today writes a marker
/// file recording the migration record per VM; the daemon supervisor's
/// pidfd-table hand-off uses the supervisor live wiring.
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
        // v1.1.1: per-VM identifier no longer has a `.scope`
        // suffix; per-VM-interior + per-role-leaf taxonomy.
        vm.trim_end_matches(".scope").to_owned(),
        role_segments,
    )))
}

fn cgroup_leaf_path(parent_slice: &Path, vm: &str, role_segments: &[String]) -> PathBuf {
    let mut path = parent_slice.to_path_buf();
    // v1.1.1 per-VM-interior path (no `.scope` suffix); the role
    // segments build the per-role leaf under it.
    path.push(vm);
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
    // Always materialize the cgroup leaf dir tree even when
    // placement.delegated == false. The delegated flag is about
    // controller delegation (enabling subtree control), not whether the
    // directory exists. The broker spawn path always needs the leaf to
    // write the child pid into cgroup.procs.
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
    Ok(Some(leaf_path))
}

struct RunnerCgroupFds {
    dir_fd: OwnedFd,
    procs_fd: OwnedFd,
}

fn prepare_runner_cgroup_fds(
    placement: &CgroupPlacement,
) -> Result<Option<RunnerCgroupFds>, LiveHandlerError> {
    use nixling_host::cgroup::RealCgroupBackend;
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
) -> Option<&'static [nixling_host::devices::DeviceClass]> {
    use nixling_host::devices::DeviceClass;
    match policy_ref {
        // cloud-hypervisor-runner binds /dev/kvm + /dev/vhost-net + /dev/net/tun.
        // Returns empty (permissive BPF) — KVM uses 100+ ioctls and
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
        // host-reconcile, store-virtiofs-preflight, guest-control-health:
        // no device binds → permissive BPF. host-reconcile and
        // store-virtiofs-preflight run the nix toolchain (many ioctls for
        // terminal/file operations); guest-control-health is the daemon-side
        // authenticated Health probe, which speaks ttRPC over the guest-control
        // vsock and uses connect(2)/socket ioctls.
        "w1-host-reconcile" | "w1-store-virtiofs-preflight" | "w1-guest-control-health" => {
            Some(&[])
        }
        // swtpm is a software TPM emulator; no hardware device ioctls,
        // but it uses terminal/file ioctls during init → permissive BPF.
        "w1-swtpm" => Some(&[]),
        // gpu sidecar binds the full GPU device set.
        // Returns empty (permissive BPF) — same reasoning as KVM: DRM
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
            let compiled = nixling_host::seccomp::compile_ioctl_policy_to_bpf(classes);
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
            .map(|arg0| arg0.starts_with("nixling-qemu-media@"))
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
        bm.src.starts_with("/var/lib/nixling/media") || bm.dst.starts_with("/var/lib/nixling/media")
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

fn qemu_media_preflight_memlock_budget(required_bytes: u64) -> Result<(), LiveHandlerError> {
    let meminfo =
        fs::read_to_string("/proc/meminfo").map_err(|err| LiveHandlerError::SpawnFailed {
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
/// guest-control callers can build path-free, acl-spec-free error
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
/// `raw_os_error` fields let the guest-control path build a path-free,
/// acl-spec-free detail that satisfies the hash-only observability
/// contract. The guest-control formatter MUST NOT read `legacy_detail`.
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

    /// Path-free, acl-spec-free failure detail for the guest-control
    /// observability contract. Carries only the closed-set operation
    /// label, target class, daemon principal, failed stage, and the
    /// numeric errno / `io::ErrorKind`. Never the raw socket / state-dir
    /// path or the acl-spec string.
    fn guest_control_detail(&self, op_label: &str, target_class: &str) -> String {
        let errno = self
            .raw_os_error
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "guest-control vsock daemon ACL {op_label} on {target_class} failed: \
             principal={GUEST_CONTROL_DAEMON_PRINCIPAL} stage={} kind={:?} errno={errno}",
            self.stage_label(),
            self.errno_kind,
        )
    }

    /// Path-free detail for the guest-control fs-share (`nl-gctl`) consumer
    /// ACL path. The consumer is the per-VM cloud-hypervisor runner (not the
    /// daemon), so this reports the CH-runner principal class rather than
    /// `nixlingd`. Carries only closed-set op/target/stage/errno labels —
    /// never the raw socket/state-dir path, the acl-spec, or a uid-by-value.
    fn guest_control_fs_detail(&self, op_label: &str, target_class: &str) -> String {
        let errno = self
            .raw_os_error
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "guest-control fs-share consumer ACL {op_label} on {target_class} failed: \
             principal={GUEST_CONTROL_FS_CONSUMER_PRINCIPAL} stage={} kind={:?} errno={errno}",
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
/// the observability/device/session callers; guest-control callers use
/// [`setfacl_fd_safe_op_classed`] for a path-free detail instead.
fn setfacl_fd_safe_op(
    path: &Path,
    op: &str,
    acl_spec: &str,
    kind: AclPathKind,
) -> Result<Option<(u64, u64)>, String> {
    setfacl_fd_safe_op_classed(path, op, acl_spec, kind).map_err(|failure| failure.legacy_detail)
}

fn setfacl_verified_device(
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
                    .status();
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
        if !arg.contains("nixling-ch-vsock-connect") {
            return None;
        }
        let exec = arg.strip_prefix("EXEC:").unwrap_or(arg).trim_matches('"');
        let fields: Vec<&str> = exec.split_whitespace().collect();
        let helper_index = fields.iter().position(|field| {
            field
                .trim_matches('"')
                .ends_with("/nixling-ch-vsock-connect")
                || field.trim_matches('"') == "nixling-ch-vsock-connect"
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

fn spawn_obs_vsock_acl_retry(uid: u32, socket: PathBuf) {
    std::thread::spawn(move || {
        for _ in 0..120 {
            match grant_obs_vsock_acl_once(uid, &socket) {
                Ok(true) => return,
                Ok(false) => {}
                Err(err) => {
                    tracing::debug!(
                        path = %socket.display(),
                        error = %err,
                        "obs-vsock socket ACL refresh not ready yet",
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        tracing::warn!(
            path = %socket.display(),
            "obs-vsock socket ACL refresh timed out",
        );
    });
}

fn refresh_obs_vsock_acl(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
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
    match grant_obs_vsock_acl_once(uid, &socket) {
        Ok(true) => Ok(()),
        Ok(false) => {
            spawn_obs_vsock_acl_retry(uid, socket);
            Ok(())
        }
        Err(detail) => Err(LiveHandlerError::SpawnFailed {
            detail: format!("refresh obs-vsock ACL for runner uid {uid}: {detail}"),
        }),
    }
}

pub(crate) fn live_grant_verified_device_acl(
    path: &Path,
    uid: u32,
) -> Result<(), LiveHandlerError> {
    live_set_verified_device_acl(path, uid, "-m", &format!("u:{uid}:rw"), "grant", false)
}

pub(crate) fn live_revoke_verified_device_acl(
    path: &Path,
    uid: u32,
) -> Result<(), LiveHandlerError> {
    live_set_verified_device_acl(path, uid, "-x", &format!("u:{uid}"), "revoke", true)
}

fn live_set_verified_device_acl(
    path: &Path,
    uid: u32,
    operation: &str,
    acl_spec: &str,
    verb: &str,
    missing_ok: bool,
) -> Result<(), LiveHandlerError> {
    setfacl_verified_device(path, operation, acl_spec, missing_ok).map_err(|detail| {
        LiveHandlerError::Activation(format!(
            "{verb} USBIP device ACL for runner uid {uid}: {detail}"
        ))
    })
}

fn refresh_spawn_runner_acls(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
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
        // Wayland-proxy gets ACL on the real host compositor socket only.
        // PipeWire and Pulse sockets are explicitly revoked: the proxy
        // has no audio role and must not connect to them.
        let runtime_dir = env_value(plan, "XDG_RUNTIME_DIR");
        if let Some(runtime_dir) = runtime_dir {
            let runtime = Path::new(runtime_dir);
            let wayland_display = env_value(plan, "WAYLAND_DISPLAY").unwrap_or("wayland-0");
            setfacl_fd_safe(
                &runtime.join(wayland_display),
                &format!("u:{}:rwx", plan.uid),
                AclPathKind::Socket,
            )
            .map_err(|detail| LiveHandlerError::SpawnFailed {
                detail: format!(
                    "refresh host compositor socket ACL for wayland-proxy uid {}: {detail}",
                    plan.uid
                ),
            })?;
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
    refresh_obs_vsock_acl(plan)?;
    refresh_guest_control_vsock_acl(plan)?;
    refresh_guest_control_fs_acl(plan)?;

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

fn grant_daemon_api_socket_acl(api_socket: PathBuf) {
    std::thread::spawn(move || {
        for _ in 0..120 {
            if api_socket.exists() {
                match setfacl_fd_safe(&api_socket, "u:nixlingd:rwx", AclPathKind::Socket) {
                    Ok(()) => return,
                    Err(err) => {
                        tracing::debug!(
                            path = %api_socket.display(),
                            error = %err,
                            "cloud-hypervisor api socket ACL refresh not ready yet",
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        tracing::warn!(
            path = %api_socket.display(),
            "cloud-hypervisor api socket ACL refresh timed out",
        );
    });
}

/// The system principal the framework grants daemon-side guest-control
/// vsock access to. The `nixlingd` daemon owns the per-VM lifecycle DAG
/// and is the only host process that connects to the guest-control
/// vsock socket for the readiness probe / config-sync over the bridge.
const GUEST_CONTROL_DAEMON_PRINCIPAL: &str = "nixlingd";

/// The runner principal-class the framework grants guest-control fs-share
/// (`nl-gctl`) connect access to: the per-VM cloud-hypervisor runner. The
/// `nl-gctl` token virtiofs share is the only CROSS-PRINCIPAL fs share —
/// it is served by the narrower `gctlfs` principal (ADR 0021), so its 0700
/// socket is owned by `gctlfs`, not the CH runner. CH connects to that
/// vhost-user fs backend socket during device-init (the `--fs
/// socket=...,tag=nl-gctl` element emitted by `processes-json.nix`), but
/// the inherited `default:u:$ch_uid` grant is masked out by the 0700
/// socket's `mask::---`. This label names the consumer for the hash-only
/// audit without leaking a uid-by-value.
const GUEST_CONTROL_FS_CONSUMER_PRINCIPAL: &str = "cloud-hypervisor-runner";

/// The setfacl `-m` spec that lifts the `nl-gctl` socket's masked-out
/// CH-runner named entry: grant the runner uid `rw` AND pin the ACL mask
/// to `rw`. The explicit `m::rw` is load-bearing — without it the 0700
/// socket's `mask::---` keeps the named entry's effective perms at `---`
/// and CH's connect still EACCESes. The mask grants no execute and does
/// not touch owner/owning-group/other, so it raises effective perms for
/// the CH-runner named entry only.
fn guest_control_fs_socket_acl_spec(uid: u32) -> String {
    format!("u:{uid}:rw,m::rw")
}

/// Extract the cloud-hypervisor `--vsock socket=<path>` argument for a
/// CH runner plan. Gated on the CH runner's seccomp policy ref so the
/// daemon-vsock ACL is only ever attached to a real cloud-hypervisor
/// runner's vsock socket, never to any other role's argv.
fn cloud_hypervisor_vsock_socket_arg(plan: &SpawnRunnerPlan) -> Option<PathBuf> {
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

/// Path-free digest of a guest-control vsock ACL mutation, for audit.
///
/// Returns `sha256:<hex>` over the operation, the target class, and the
/// target's resolved `(dev, ino)` — never the raw socket / state-dir
/// path. The guest-control observability contract forbids raw vsock /
/// socket / state-dir paths in spans, logs, metrics, and audit.
fn guest_control_acl_diff_hash(op: &str, target_class: &str, dev: u64, ino: u64) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"guest-control-vsock-acl\0");
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

/// Emit a hash-only audit event for a guest-control vsock ACL mutation.
/// Closed-enum labels only; no raw paths, uids-by-value, or content.
fn audit_guest_control_vsock_acl(op: &str, target_class: &str, dev: u64, ino: u64) {
    tracing::info!(
        kind = "critical",
        subsystem = "guest-control-health",
        op = op,
        daemon_principal = GUEST_CONTROL_DAEMON_PRINCIPAL,
        target_class = target_class,
        acl_diff_hash = %guest_control_acl_diff_hash(op, target_class, dev, ino),
        result = "ok",
        "guest-control vsock daemon ACL mutation",
    );
}

/// Path-free wrapper over [`setfacl_fd_safe_op_classed`] for the
/// guest-control ACL path: on failure, builds a detail string carrying
/// only the closed-set op/target-class/stage/errno classification —
/// never the raw socket/state-dir path or the acl-spec string.
fn setfacl_guest_control(
    path: &Path,
    op: &str,
    acl_spec: &str,
    kind: AclPathKind,
    op_label: &str,
    target_class: &str,
) -> Result<Option<(u64, u64)>, String> {
    setfacl_fd_safe_op_classed(path, op, acl_spec, kind)
        .map_err(|failure| failure.guest_control_detail(op_label, target_class))
}

/// Path-free wrapper over [`setfacl_fd_safe_op_classed`] for the
/// guest-control fs-share (`nl-gctl`) CONSUMER ACL path: identical to
/// [`setfacl_guest_control`] but reports the cloud-hypervisor-runner
/// consumer principal class on failure instead of the daemon principal.
fn setfacl_guest_control_fs(
    path: &Path,
    op: &str,
    acl_spec: &str,
    kind: AclPathKind,
    op_label: &str,
    target_class: &str,
) -> Result<Option<(u64, u64)>, String> {
    setfacl_fd_safe_op_classed(path, op, acl_spec, kind)
        .map_err(|failure| failure.guest_control_fs_detail(op_label, target_class))
}

/// Emit a hash-only audit event for a guest-control fs-share consumer ACL
/// mutation (the cloud-hypervisor runner's connect grant on the `nl-gctl`
/// virtiofs socket / its parent dir). Closed-enum labels only; no raw
/// paths, uids-by-value, or content — same contract as
/// [`audit_guest_control_vsock_acl`].
fn audit_guest_control_fs_acl(op: &str, target_class: &str, dev: u64, ino: u64) {
    tracing::info!(
        kind = "critical",
        subsystem = "guest-control-health",
        op = op,
        consumer_principal = GUEST_CONTROL_FS_CONSUMER_PRINCIPAL,
        target_class = target_class,
        acl_diff_hash = %guest_control_acl_diff_hash(op, target_class, dev, ino),
        result = "ok",
        "guest-control fs-share consumer ACL mutation",
    );
}

/// Whether the daemon needs an explicit `--x` grant to traverse `path`.
///
/// `Ok(Some(true))` if `path` is a directory the daemon cannot already
/// search (world execute bit clear). `Ok(Some(false))` if it is a
/// world-traversable directory (no grant needed) or not a directory.
/// `Ok(None)` if the path is absent.
fn dir_needs_daemon_traverse(path: &Path) -> Result<Option<bool>, SetfaclFailure> {
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

/// Grant the daemon `u:nixlingd:--x` on every non-world-traversable
/// directory from the filesystem root down to `leaf` (inclusive), so the
/// daemon can `connect()` to the per-VM guest-control socket through the
/// full ancestor chain — not just the immediate parent. World-
/// traversable directories already grant search to everyone and are
/// skipped. The immediate per-VM leaf is audited as `state-dir`; higher
/// non-world-x ancestors as `ancestor`. These grants are additive and
/// idempotent; they are never revoked because sibling VMs and the
/// per-VM api-socket also depend on them.
fn grant_guest_control_traversal_acls(leaf: &Path) -> Result<(), String> {
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
        let needs = dir_needs_daemon_traverse(dir)
            .map_err(|failure| failure.guest_control_detail("grant", target_class))?;
        if needs == Some(true)
            && let Some((dev, ino)) = setfacl_guest_control(
                dir,
                "-m",
                &format!("u:{GUEST_CONTROL_DAEMON_PRINCIPAL}:--x"),
                AclPathKind::Directory,
                "grant",
                target_class,
            )?
        {
            audit_guest_control_vsock_acl("grant", target_class, dev, ino);
        }
    }
    Ok(())
}

/// Grant `u:nixlingd:rw` on the guest-control vsock socket inode, then
/// re-stat the path to confirm it still resolves to the same `(dev,
/// ino)` the fd-based setfacl mutated (inode pinning). If the
/// socket was replaced (or vanished) between the setfacl and the
/// re-stat, the grant landed on a now-stale inode: do not audit success
/// and report not-ready (`Ok(false)`) so the caller retries against the
/// current, live inode. Returns `Ok(false)` while the socket has not yet
/// been created by cloud-hypervisor.
fn grant_guest_control_socket_acl_once(socket: &Path) -> Result<bool, String> {
    let Some((dev, ino)) = setfacl_guest_control(
        socket,
        "-m",
        &format!("u:{GUEST_CONTROL_DAEMON_PRINCIPAL}:rw"),
        AclPathKind::Socket,
        "grant",
        "vsock-socket",
    )?
    else {
        return Ok(false);
    };
    match current_path_dev_ino(socket)
        .map_err(|failure| failure.guest_control_detail("grant", "vsock-socket"))?
    {
        Some(current) if current == (dev, ino) => {
            audit_guest_control_vsock_acl("grant", "vsock-socket", dev, ino);
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Revoke the daemon-principal guest-control vsock ACL from the socket
/// inode. Best-effort, idempotent, and path-free: a missing socket is a
/// no-op. Scoped to the per-VM socket inode only — the shared/ancestor
/// traversal grants are intentionally retained (the daemon also needs
/// them for the per-VM api-socket and sibling VMs depend on them). This
/// is the production revoke wiring: there is no CH-stop teardown hook
/// carrying the socket path (`SignalRunner` has only vm_id/role_id/
/// signal), so revoke runs as a revoke-then-grant at the next CH
/// (re-)spawn so a replaced/disabled socket cannot retain a stale grant.
fn revoke_guest_control_vsock_acl(socket: &Path) -> Result<(), String> {
    if let Some((dev, ino)) = setfacl_guest_control(
        socket,
        "-x",
        &format!("u:{GUEST_CONTROL_DAEMON_PRINCIPAL}"),
        AclPathKind::Socket,
        "revoke",
        "vsock-socket",
    )? {
        audit_guest_control_vsock_acl("revoke", "vsock-socket", dev, ino);
    }
    Ok(())
}

/// Retry the daemon-vsock socket ACL grant in a background thread until
/// the cloud-hypervisor process has created the vsock socket (bounded to
/// ~30s, matching the obs-vsock precedent). The traversal ACLs are
/// granted synchronously before this thread starts, so the retry only
/// re-attempts the socket grant. Logs are path-free.
fn spawn_guest_control_vsock_acl_retry(socket: PathBuf) {
    std::thread::spawn(move || {
        for _ in 0..120 {
            match grant_guest_control_socket_acl_once(&socket) {
                Ok(true) => return,
                Ok(false) => {}
                Err(_) => {
                    tracing::debug!(
                        subsystem = "guest-control-health",
                        "guest-control vsock daemon ACL refresh not ready yet",
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        tracing::warn!(
            subsystem = "guest-control-health",
            "guest-control vsock daemon ACL refresh timed out",
        );
    });
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
fn refresh_guest_control_vsock_acl(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
    let Some(socket) = cloud_hypervisor_vsock_socket_arg(plan) else {
        return Ok(());
    };
    let Some(parent) = socket.parent().map(Path::to_path_buf) else {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "guest-control vsock path has no parent".to_owned(),
        });
    };

    if let Err(detail) = revoke_guest_control_vsock_acl(&socket) {
        tracing::debug!(
            subsystem = "guest-control-health",
            detail = %detail,
            "pre-grant guest-control daemon ACL revoke (best-effort)",
        );
    }

    grant_guest_control_traversal_acls(&parent).map_err(|detail| {
        LiveHandlerError::SpawnFailed {
            detail: format!("refresh guest-control traversal ACLs: {detail}"),
        }
    })?;

    match grant_guest_control_socket_acl_once(&socket) {
        Ok(true) => Ok(()),
        Ok(false) => {
            spawn_guest_control_vsock_acl_retry(socket);
            Ok(())
        }
        Err(detail) => Err(LiveHandlerError::SpawnFailed {
            detail: format!("refresh guest-control vsock daemon ACL: {detail}"),
        }),
    }
}

/// Extract the cloud-hypervisor `--fs socket=<path>,tag=nl-gctl` argument
/// for a CH runner plan, or `None` for any non-CH runner / a CH plan
/// without the guest-control token share.
///
/// Cloud Hypervisor's `--fs` takes a variadic list of value elements
/// (`socket=...,tag=...`) — one per share — so this scans EVERY argv
/// element (never just the first after `--fs`) for the element whose
/// comma-separated fields include exactly `tag=nl-gctl`, and returns its
/// absolute `socket=` value. Gated on the CH runner's seccomp policy ref
/// so the grant is only ever attached to a real cloud-hypervisor runner.
fn cloud_hypervisor_guest_control_fs_socket_arg(plan: &SpawnRunnerPlan) -> Option<PathBuf> {
    if plan.seccomp_policy_ref.as_deref() != Some("w1-cloud-hypervisor-runner") {
        return None;
    }
    plan.argv.iter().find_map(|arg| {
        let arg = arg.trim_matches('"');
        let fields: Vec<&str> = arg.split(',').collect();
        if !fields.contains(&"tag=nl-gctl") {
            return None;
        }
        fields
            .iter()
            .find_map(|field| field.strip_prefix("socket="))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    })
}

/// Grant the CH runner uid execute (search) on the `nl-gctl` socket's
/// parent (the per-VM `guest-control` dir) so CH can traverse to the
/// socket. Idempotent with the host-activation `u:$ch_uid:--x` grant;
/// re-asserting it here makes the spawn path self-healing if the dir was
/// recreated. Path-free + hash-only audited.
fn grant_guest_control_fs_traversal_acl(parent: &Path, uid: u32) -> Result<(), String> {
    if let Some((dev, ino)) = setfacl_guest_control_fs(
        parent,
        "-m",
        &format!("u:{uid}:--x"),
        AclPathKind::Directory,
        "grant",
        "gctlfs-dir",
    )? {
        audit_guest_control_fs_acl("grant", "gctlfs-dir", dev, ino);
    }
    Ok(())
}

/// Grant `u:<ch_uid>:rw` on the `nl-gctl` virtiofs socket inode, with an
/// explicit `m::rw` mask so the 0700 socket's `mask::---` is deterministically
/// lifted to cover the CH-runner named entry (without enabling execute in the
/// mask). Then re-stat to confirm the same `(dev, ino)` (inode pinning): if
/// the socket was replaced/vanished between the setfacl and the re-stat, the
/// grant landed on a stale inode — do not audit success and report not-ready
/// (`Ok(false)`) so the caller retries the live inode. `Ok(false)` while the
/// socket has not yet been created by the gctlfs virtiofsd.
fn grant_guest_control_fs_socket_acl_once(socket: &Path, uid: u32) -> Result<bool, String> {
    let Some((dev, ino)) = setfacl_guest_control_fs(
        socket,
        "-m",
        &guest_control_fs_socket_acl_spec(uid),
        AclPathKind::Socket,
        "grant",
        "gctlfs-socket",
    )?
    else {
        return Ok(false);
    };
    match current_path_dev_ino(socket)
        .map_err(|failure| failure.guest_control_fs_detail("grant", "gctlfs-socket"))?
    {
        Some(current) if current == (dev, ino) => {
            audit_guest_control_fs_acl("grant", "gctlfs-socket", dev, ino);
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Retry the CH-runner `nl-gctl` socket ACL grant in a background thread
/// until the gctlfs virtiofsd has created the socket. Bounded to ~60s
/// (240 × 250ms) so it covers cloud-hypervisor's own ~1-minute vhost-user
/// backend connect-retry window. The traversal grant is applied
/// synchronously before this thread starts; this only re-attempts the
/// socket grant. Logs are path-free.
fn spawn_guest_control_fs_acl_retry(socket: PathBuf, uid: u32) {
    std::thread::spawn(move || {
        for _ in 0..240 {
            match grant_guest_control_fs_socket_acl_once(&socket, uid) {
                Ok(true) => return,
                Ok(false) => {}
                Err(_) => {
                    tracing::debug!(
                        subsystem = "guest-control-health",
                        consumer_principal = GUEST_CONTROL_FS_CONSUMER_PRINCIPAL,
                        "guest-control fs-share consumer ACL refresh not ready yet",
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        tracing::warn!(
            subsystem = "guest-control-health",
            consumer_principal = GUEST_CONTROL_FS_CONSUMER_PRINCIPAL,
            "guest-control fs-share consumer ACL refresh timed out",
        );
    });
}

/// Grant the cloud-hypervisor runner connect access to the `nl-gctl`
/// virtiofs socket. No-op for any non-CH runner or a CH plan without the
/// guest-control token share.
///
/// The `nl-gctl` share is the only CROSS-PRINCIPAL fs share (served by the
/// narrower `gctlfs` principal, so CH does not own its socket as it does
/// the runner-owned shares). The gctlfs virtiofsd runs in the broker's
/// user namespace (ADR 0021) where `--socket-group` does not take effect
/// on the host-visible socket, leaving it `0700 gctlfs:gctlfs` with
/// `mask::---` — which masks out the `default:u:$ch_uid` grant the
/// host-activation default ACL inherits onto the socket. Grant the CH
/// runner uid search on the parent and `rw` on the socket (lifting the
/// mask) so CH's vhost-user backend connect succeeds. The socket is a DAG
/// predecessor of cloud-hypervisor, so the synchronous grant normally
/// lands before CH spawns; a bounded retry covers the rare absent-socket
/// race.
fn refresh_guest_control_fs_acl(plan: &SpawnRunnerPlan) -> Result<(), LiveHandlerError> {
    let Some(socket) = cloud_hypervisor_guest_control_fs_socket_arg(plan) else {
        return Ok(());
    };
    let Some(parent) = socket.parent().map(Path::to_path_buf) else {
        return Err(LiveHandlerError::SpawnFailed {
            detail: "guest-control fs-share socket path has no parent".to_owned(),
        });
    };
    let uid = plan.uid;

    grant_guest_control_fs_traversal_acl(&parent, uid).map_err(|detail| {
        LiveHandlerError::SpawnFailed {
            detail: format!("refresh guest-control fs-share traversal ACL: {detail}"),
        }
    })?;

    match grant_guest_control_fs_socket_acl_once(&socket, uid) {
        Ok(true) => Ok(()),
        Ok(false) => {
            spawn_guest_control_fs_acl_retry(socket, uid);
            Ok(())
        }
        Err(detail) => Err(LiveHandlerError::SpawnFailed {
            detail: format!("refresh guest-control fs-share consumer ACL: {detail}"),
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
pub fn live_spawn_runner(
    plan_input: &SpawnRunnerPlanInput,
    mut pre_opened_device_fds: Vec<std::os::fd::OwnedFd>,
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

    let (binary, argv, env) =
        build_cstring_vectors(&plan).map_err(LiveHandlerError::SpawnPreflight)?;
    let seccomp_program = load_runner_seccomp(&plan)?;
    let cgroup_fds = prepare_runner_cgroup_fds(&plan.cgroup_placement)?;
    refresh_spawn_runner_acls(&plan)?;

    // swtpm-dir first-run hardening (issue #64). Gated on the
    // `w1-swtpm` role and run BEFORE clone3 so the persistent TPM2
    // NVRAM dir is provisioned + identity-bound (or fails closed)
    // before swtpm — which opens the NVRAM by pathname under its user
    // namespace — is ever spawned. ONLY the persistent state dir is
    // touched; the `/run` runtime-socket-dir posture is left intact.
    let swtpm_dir_audit = maybe_harden_swtpm_dir(&plan)?;
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
    // so the DAC permission check runs as the broker UID — the child's
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

    let memlock_guest_bytes = qemu_media_memlock_guest_bytes(&plan)?;
    let memlock_limit_bytes = memlock_guest_bytes.map(|guest_bytes| {
        guest_bytes.saturating_add(qemu_media_memlock_headroom_bytes(guest_bytes))
    });
    if let Some(guest_bytes) = memlock_guest_bytes {
        qemu_media_preflight_memlock_budget(qemu_media_memlock_preflight_required_bytes(
            guest_bytes,
        ))?;
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
fn maybe_harden_swtpm_dir(
    plan: &SpawnRunnerPlan,
) -> Result<Option<crate::ops::audit_op::SwtpmDirAudit>, LiveHandlerError> {
    if plan.seccomp_policy_ref.as_deref() != Some("w1-swtpm") {
        return Ok(None);
    }
    let paths = crate::ops::swtpm_dir::derive_paths(plan).map_err(|reason| {
        LiveHandlerError::SwtpmDirHardening {
            audit: crate::ops::swtpm_dir::SwtpmHardenError {
                reason,
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
            }
            .audit,
            reason,
        }
    })?;
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
    match crate::ops::swtpm_dir::harden(&paths, &cfg) {
        Ok(audit) => Ok(Some(audit)),
        Err(err) => Err(LiveHandlerError::SwtpmDirHardening {
            audit: err.audit,
            reason: err.reason,
        }),
    }
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
    use std::os::unix::fs::symlink;
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

    fn fake_unbound_usbip_sysfs(root: &TestDir, bus_id: &str) -> PathBuf {
        let sysfs_root = root.join("sys").join("bus").join("usb").join("devices");
        std::fs::create_dir_all(sysfs_root.join(bus_id)).expect("create fake usb device");
        sysfs_root
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
            target_view_path: root.join("store-view/live/alpha-system"),
            closure_paths: vec![source_view],
            db_dump_path: root.join("db.dump"),
        }
    }

    #[test]
    fn rollback_target_view_path_uses_rolled_back_marker_basename() {
        let intent = ResolvedStoreViewIntent {
            intent_id: "store-view:vm:alpha".to_owned(),
            vm: "alpha".to_owned(),
            generation: 99,
            hardlink_farm_path: PathBuf::from("/var/lib/nixling/vms/alpha/store-view"),
            target_view_path: PathBuf::from(
                "/var/lib/nixling/vms/alpha/store-view/generations/99/current-system",
            ),
            closure_paths: Vec::new(),
            db_dump_path: PathBuf::from("/nix/store/alpha-registration"),
        };
        let marker = hardlink_farm::GenerationMarker {
            closure_hash: "toplevel:old-system".to_owned(),
            nixling_version: "test".to_owned(),
            activated_at: "test".to_owned(),
            vm: "alpha".to_owned(),
            generation_number: 7,
        };
        // Rollback to gen 7 (closure `old-system`) must target the OLD
        // basename, not the current intent's `current-system`.
        assert_eq!(
            rollback_target_view_path(&intent, 7, &marker).unwrap(),
            PathBuf::from("/var/lib/nixling/vms/alpha/store-view/generations/7/old-system"),
        );

        // Defensive fallback: a pre-`toplevel:` marker format resolves
        // to the current intent's basename (old behavior).
        let legacy = hardlink_farm::GenerationMarker {
            closure_hash: "store-view:alpha:7".to_owned(),
            ..marker.clone()
        };
        assert_eq!(
            rollback_target_view_path(&intent, 7, &legacy).unwrap(),
            PathBuf::from("/var/lib/nixling/vms/alpha/store-view/generations/7/current-system"),
        );

        // A malformed marker with path traversal is rejected → fallback.
        let evil = hardlink_farm::GenerationMarker {
            closure_hash: "toplevel:../escape".to_owned(),
            ..marker.clone()
        };
        assert_eq!(
            rollback_target_view_path(&intent, 7, &evil).unwrap(),
            PathBuf::from("/var/lib/nixling/vms/alpha/store-view/generations/7/current-system"),
        );
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
    fn usbip_unbind_failure_preserves_claim_for_operator_recovery() {
        struct FailUsbipUnbind;
        impl ReconcileExecutor for FailUsbipUnbind {
            fn apply_nft_script(&self, _: &Path, _: &str) -> Result<(), ReconcileExecError> {
                unreachable!()
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
                subcommand: crate::ops::exec_reconcile::UsbipSubcommand,
                _: &str,
            ) -> Result<(), ReconcileExecError> {
                assert_eq!(
                    subcommand,
                    crate::ops::exec_reconcile::UsbipSubcommand::Unbind
                );
                Err(ReconcileExecError::TimedOut {
                    which: "usbip unbind".to_owned(),
                    timeout_ms: 1,
                    remediation: "manual recovery required".to_owned(),
                })
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

        let root = TestDir::new("usbip-unbind-preserves-claim");
        let lock_dir = root.join("locks");
        std::fs::create_dir_all(&lock_dir).expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Uid::current().as_raw(),
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
        .unwrap_err();

        assert!(matches!(err, LiveHandlerError::ReconcileExec(_)));
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&lock_path),
            Some("corp-vm".to_owned()),
            "failed or timed-out sysfs unbind must not falsely release the USBIP session claim"
        );
    }

    #[test]
    fn usbip_bind_same_vm_replay_skips_shellout_and_preserves_claim() {
        let root = TestDir::new("usbip-bind-same-vm-replay");
        let lock_dir = root.join("locks");
        std::fs::create_dir_all(&lock_dir).expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Uid::current().as_raw(),
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
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
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

    #[test]
    fn usbip_bind_shellout_failure_releases_claim_for_retry() {
        let root = TestDir::new("usbip-bind-failure-releases-claim");
        let lock_dir = root.join("locks");
        std::fs::create_dir_all(&lock_dir).expect("create lock dir");
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
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
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

    #[test]
    fn usbip_unbind_aborts_stream_before_driver_unbind_and_preserves_claim_for_acl_phase() {
        let root = TestDir::new("usbip-unbind-order");
        let lock_dir = root.join("locks");
        std::fs::create_dir_all(&lock_dir).expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Uid::current().as_raw(),
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

    #[test]
    fn usbip_unbind_fd_release_timeout_preserves_claim_without_driver_unbind() {
        let root = TestDir::new("usbip-unbind-release-timeout");
        let lock_dir = root.join("locks");
        std::fs::create_dir_all(&lock_dir).expect("create lock dir");
        let lock_path = lock_dir.join("1-2");
        let sysfs_root = fake_usbip_sysfs(&root, "1-2");
        crate::ops::usbip_lock::acquire_lock(
            &lock_path,
            "corp-vm",
            nix::unistd::Uid::current().as_raw(),
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
                .join("personal-dev")
                .join("virtiofsd-ro-store")
        );
        assert!(backend.directory_exists(&leaf));
        // DEFAULT_DELEGATED_PARENT_SLICE is the top-level
        // `/sys/fs/cgroup/nixling.slice` (systemd top-level slice naming
        // convention). The leaf path lives under that, so the slice MUST
        // exist for the leaf to exist.
        assert!(backend.directory_exists(Path::new("/sys/fs/cgroup/nixling.slice")));
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
            user_namespace: None,
            umask: None,
        };
        let err = live_spawn_runner(&plan, Vec::new()).unwrap_err();
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
            test_spawn_plan_with_argv(vec!["nixling-qemu-media@media".to_owned()], "w1-qemu-media");
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
                    path: "/run/nixling/vms/media".to_owned(),
                    purpose: "QMP socket".to_owned(),
                },
                WritablePath {
                    path: "/var/lib/nixling/vms/media".to_owned(),
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
            "nixling-qemu-media@media".to_owned(),
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
            "nixling-qemu-media@media".to_owned(),
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
                    "nixling-qemu-media@media".to_owned(),
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
            .push(nixling_core::minijail_profile::BindMount {
                src: "/var/lib/nixling/media/install.iso".to_owned(),
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
                "/var/lib/nixling/vms/corp-vm/api.sock".to_owned(),
                "--vsock".to_owned(),
                "cid=42,socket=/var/lib/nixling/vms/corp-vm/vsock.sock".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(
            cloud_hypervisor_vsock_socket_arg(&plan),
            Some(PathBuf::from("/var/lib/nixling/vms/corp-vm/vsock.sock"))
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
                "cid=42,socket=/var/lib/nixling/vms/corp-vm/vsock.sock".to_owned(),
            ],
            "w1-vsock-relay",
        );

        assert_eq!(cloud_hypervisor_vsock_socket_arg(&plan), None);
    }

    #[test]
    fn cloud_hypervisor_vsock_socket_absent_without_vsock_arg() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--api-socket".to_owned(),
                "/var/lib/nixling/vms/corp-vm/api.sock".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(cloud_hypervisor_vsock_socket_arg(&plan), None);
    }

    #[test]
    fn parses_guest_control_fs_socket_for_ch_runner_heavy_vm() {
        // CH `--fs` is variadic: many `socket=...,tag=...` value elements,
        // with `nl-gctl` LAST after ro-store / nl-meta / nl-hkeys. The
        // parser must scan every element, not just the first after `--fs`.
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--fs".to_owned(),
                "socket=/run/nixling/vms/work-aad/ro-store.sock,tag=ro-store".to_owned(),
                "socket=/run/nixling/vms/work-aad/nl-meta.sock,tag=nl-meta".to_owned(),
                "socket=/run/nixling/vms/work-aad/nl-hkeys.sock,tag=nl-hkeys".to_owned(),
                "socket=/run/nixling/vms/work-aad/guest-control/nl-gctl.sock,tag=nl-gctl"
                    .to_owned(),
                "--api-socket".to_owned(),
                "/var/lib/nixling/vms/work-aad/work-aad.sock".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(
            cloud_hypervisor_guest_control_fs_socket_arg(&plan),
            Some(PathBuf::from(
                "/run/nixling/vms/work-aad/guest-control/nl-gctl.sock"
            ))
        );
    }

    #[test]
    fn guest_control_fs_socket_gated_on_ch_runner_policy() {
        // Same nl-gctl share element but a non-CH seccomp policy: the
        // consumer ACL must never attach to a non-cloud-hypervisor runner.
        let plan = test_spawn_plan_with_argv(
            vec![
                "virtiofsd".to_owned(),
                "socket=/run/nixling/vms/work-aad/guest-control/nl-gctl.sock,tag=nl-gctl"
                    .to_owned(),
            ],
            "w1-virtiofsd",
        );

        assert_eq!(cloud_hypervisor_guest_control_fs_socket_arg(&plan), None);
    }

    #[test]
    fn guest_control_fs_socket_absent_without_nl_gctl_share() {
        // A guest-control-disabled VM: CH runner with fs shares but no
        // nl-gctl tag. The grant must be a no-op.
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--fs".to_owned(),
                "socket=/run/nixling/vms/work-aad/ro-store.sock,tag=ro-store".to_owned(),
                "socket=/run/nixling/vms/work-aad/nl-meta.sock,tag=nl-meta".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(cloud_hypervisor_guest_control_fs_socket_arg(&plan), None);
    }

    #[test]
    fn guest_control_fs_socket_rejects_relative_socket_path() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--fs".to_owned(),
                "socket=relative/nl-gctl.sock,tag=nl-gctl".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(cloud_hypervisor_guest_control_fs_socket_arg(&plan), None);
    }

    #[test]
    fn guest_control_fs_socket_parser_rejects_near_miss_tag() {
        // A share whose tag merely CONTAINS `nl-gctl` as a prefix
        // (`tag=nl-gctl-foo`) must not be mistaken for the token share:
        // the parser does an exact comma-field match, never a substring
        // match, so a future regression to substring matching would
        // mis-grant the consumer ACL onto the wrong share.
        let plan = test_spawn_plan_with_argv(
            vec![
                "cloud-hypervisor".to_owned(),
                "--fs".to_owned(),
                "socket=/run/nixling/vms/work-aad/nl-gctl-foo.sock,tag=nl-gctl-foo".to_owned(),
            ],
            "w1-cloud-hypervisor-runner",
        );

        assert_eq!(cloud_hypervisor_guest_control_fs_socket_arg(&plan), None);
    }

    #[test]
    fn guest_control_fs_socket_grant_not_ready_for_absent_socket() {
        // Hermetic: before the gctlfs virtiofsd creates the nl-gctl
        // socket, the consumer socket grant must report not-ready
        // (Ok(false)) so the caller retries — never erroring (which would
        // fail the cloud-hypervisor spawn outright) and never touching a
        // foreign inode. This is the exact race the bounded retry thread
        // exists to tolerate.
        let dir = TestDir::new("gc-fs-acl");
        let socket = dir.join("nl-gctl.sock");
        assert!(!socket.exists());
        assert_eq!(
            grant_guest_control_fs_socket_acl_once(&socket, 4242),
            Ok(false),
            "absent socket must report not-ready",
        );
    }

    #[test]
    fn guest_control_fs_socket_acl_spec_pins_mask() {
        // Regression guard: the `m::rw` mask token is load-bearing. The
        // 0700 nl-gctl socket's `mask::---` masks out the inherited
        // CH-runner named entry, so the grant MUST pin the mask to `rw`
        // (not the bare `u:<uid>:rw` the vsock precedent uses). Dropping
        // `,m::rw` would silently reintroduce the original EACCES/boot
        // hang with every other unit test still green. The mask must not
        // grant execute.
        let spec = guest_control_fs_socket_acl_spec(1628571);
        assert_eq!(spec, "u:1628571:rw,m::rw");
        assert!(spec.contains("m::rw"), "mask token missing: {spec}");
        assert!(
            !spec.contains('x'),
            "mask/entry must not grant execute: {spec}"
        );
    }

    #[test]
    fn guest_control_fs_detail_is_path_free() {
        // The fs-share consumer error formatter must never leak a
        // path-bearing legacy detail: it carries only closed-set class
        // tokens (op/target-class/stage), the CH-runner consumer
        // principal, the io::ErrorKind, and the numeric errno — same
        // path-free contract as the daemon `guest_control_detail` twin.
        let failure = SetfaclFailure {
            stage: SetfaclStage::Apply,
            errno_kind: std::io::ErrorKind::PermissionDenied,
            raw_os_error: Some(13),
            legacy_detail: "setfacl -m u:1628571:rw,m::rw on \
                 /run/nixling/vms/corp-vm/guest-control/nl-gctl.sock: denied"
                .to_owned(),
        };
        let detail = failure.guest_control_fs_detail("grant", "gctlfs-socket");
        assert!(
            !detail.contains('/'),
            "detail must not embed any path: {detail}"
        );
        assert!(
            !detail.contains("nl-gctl.sock"),
            "detail leaked socket name: {detail}"
        );
        assert!(!detail.contains(":rw"), "detail leaked acl spec: {detail}");
        assert!(
            !detail.contains("1628571"),
            "detail leaked uid-by-value: {detail}"
        );
        assert!(
            detail.contains("gctlfs-socket"),
            "missing target class: {detail}"
        );
        assert!(detail.contains("stage=apply"), "missing stage: {detail}");
        assert!(detail.contains("errno=13"), "missing errno: {detail}");
        assert!(
            detail.contains(GUEST_CONTROL_FS_CONSUMER_PRINCIPAL),
            "missing consumer principal: {detail}"
        );
    }

    #[test]
    fn guest_control_acl_diff_hash_is_path_free_and_stable() {
        let h1 = guest_control_acl_diff_hash("grant", "vsock-socket", 0x10, 0x20);
        let h2 = guest_control_acl_diff_hash("grant", "vsock-socket", 0x10, 0x20);
        let h3 = guest_control_acl_diff_hash("revoke", "vsock-socket", 0x10, 0x20);
        assert_eq!(h1, h2, "hash must be deterministic for identical inputs");
        assert_ne!(h1, h3, "op must affect the hash");
        assert!(h1.starts_with("sha256:"));
        assert_eq!(h1.len(), "sha256:".len() + 64);
        // The digest must not embed any raw path component.
        assert!(!h1.contains('/'));
    }

    #[test]
    fn guest_control_acl_grant_not_ready_for_absent_socket() {
        // Hermetic: before cloud-hypervisor creates the vsock socket,
        // the socket grant must report not-ready (Ok(false)) so the
        // caller retries — never erroring and never touching a foreign
        // inode.
        let dir = TestDir::new("gc-vsock-acl");
        let socket = dir.join("vsock.sock");
        assert!(!socket.exists());
        assert_eq!(
            grant_guest_control_socket_acl_once(&socket),
            Ok(false),
            "absent socket must report not-ready",
        );
        // Revoke against an absent socket is a no-op (short-circuits on
        // the absent inode before any setfacl). The traversal-skip
        // behaviour (no setfacl on world-traversable ancestors) is
        // covered hermetically by `dir_traverse_classification_world_x_vs_private`
        // without invoking the host setfacl binary on real ancestors —
        // which a TestDir rooted under a non-world-x CI path (e.g.
        // `/home/runner`, mode 0750) would otherwise trigger.
        revoke_guest_control_vsock_acl(&socket).expect("revoke of absent socket is a no-op");
    }

    #[test]
    fn setfacl_failure_guest_control_detail_is_path_free() {
        // A path-bearing legacy detail must never leak through the
        // guest-control formatter: it carries only closed-set class
        // tokens (op/target-class/stage), the daemon principal, the
        // io::ErrorKind, and the numeric errno.
        let failure = SetfaclFailure {
            stage: SetfaclStage::Apply,
            errno_kind: std::io::ErrorKind::PermissionDenied,
            raw_os_error: Some(13),
            legacy_detail:
                "setfacl -m u:nixlingd:rw on /var/lib/nixling/vms/corp-vm/vsock.sock: denied"
                    .to_owned(),
        };
        let detail = failure.guest_control_detail("grant", "vsock-socket");
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
            detail.contains(GUEST_CONTROL_DAEMON_PRINCIPAL),
            "missing daemon principal: {detail}"
        );
        // No-errno case renders a stable token.
        let mismatch = SetfaclFailure {
            stage: SetfaclStage::TypeMismatch,
            errno_kind: std::io::ErrorKind::InvalidInput,
            raw_os_error: None,
            legacy_detail: "refusing setfacl on /secret/path".to_owned(),
        };
        let detail = mismatch.guest_control_detail("revoke", "state-dir");
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

        assert_eq!(dir_needs_daemon_traverse(&world_x), Ok(Some(false)));
        assert_eq!(dir_needs_daemon_traverse(&private), Ok(Some(true)));
        assert_eq!(dir_needs_daemon_traverse(&file), Ok(Some(false)));
        assert_eq!(dir_needs_daemon_traverse(&dir.join("absent")), Ok(None));
    }

    #[test]
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
                "nixling-otel-host-bridge".to_owned(),
                "-d".to_owned(),
                "-d".to_owned(),
                "UNIX-LISTEN:/run/nixling/otel/host-egress.sock,fork,reuseaddr,mode=0660"
                    .to_owned(),
                "EXEC:\"/run/current-system/sw/bin/nixling-ch-vsock-connect /var/lib/nixling/vms/sys-obs/vsock.sock 14317\""
                    .to_owned(),
            ],
            "w1-otel-host-bridge",
        );

        assert_eq!(
            ch_vsock_connect_socket_arg(&plan),
            Some(PathBuf::from("/var/lib/nixling/vms/sys-obs/vsock.sock"))
        );
    }

    #[test]
    fn parses_unquoted_vsock_relay_ch_vsock_socket() {
        let plan = test_spawn_plan_with_argv(
            vec![
                "nixling-otel-relay@work-aad".to_owned(),
                "-d".to_owned(),
                "-d".to_owned(),
                "UNIX-LISTEN:/var/lib/nixling/vms/work-aad/vsock.sock_14317,fork,max-children=16,reuseaddr,mode=0660"
                    .to_owned(),
                "EXEC:/run/current-system/sw/bin/nixling-ch-vsock-connect /var/lib/nixling/vms/sys-obs/vsock.sock 14318"
                    .to_owned(),
            ],
            "w1-vsock-relay",
        );

        assert_eq!(
            ch_vsock_connect_socket_arg(&plan),
            Some(PathBuf::from("/var/lib/nixling/vms/sys-obs/vsock.sock"))
        );
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
                device_binds: Vec::new(),
                bind_mounts: Vec::new(),
            },
            cgroup_placement: test_cgroup_placement(),
            root_carve_out: true,
            skip_binary_exists_check: false,
            user_namespace: None,
            umask: None,
        };

        let outcome = live_spawn_runner(&plan, Vec::new()).expect("spawn privileged test child");
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
