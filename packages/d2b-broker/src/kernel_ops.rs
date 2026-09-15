//! The broker's own-process kernel seam (U10).
//!
//! The U10 sandwich serves each process-family operation's privileged,
//! resource-agnostic kernel in-broker as a broker-generic committed row
//! while the family operation itself stays forwarded to the declaring
//! process. A kernel is the narrowest responsible layer of one family
//! operation: the syscall or /proc probe that touches no family knowledge
//! (no runner ids, no bundle intents, no VM topology). The daemon-side
//! family handler validates the typed request against its own resolver and
//! invokes the kernel as a nested envelope call, so the broker never grows
//! family code and the family side never re-implements a privileged
//! syscall.
//!
//! Every kernel is a closure over the broker's serve-time config (state
//! dir, runtime root, daemon uid/gid) registered on the envelope's
//! [`HandlerTable`]. The mixed [`KernelDispatcher`] routes these rows to
//! this table and every other committed row to the forward carrier, and
//! the envelope's audit rule records the in-broker leg broker-side
//! (KTD6).
//!
//! The kernels reuse the same live helpers the retired typed arms used
//! (`live_handlers`, `ops::state_dir`, `ops::cgroup`, the runner pidfd
//! registry cell, the reap buffer); they add no family knowledge of their
//! own.

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use d2b_contracts_resource::v3::{CanonicalJsonObject, CanonicalJsonValue};

use crate::envelope::{DirectInvocation, DispatchFailure, DispatchOutcome, HandlerTable};
use crate::ops::spawn_runner::{SpawnRunnerPlanInput, UserNamespaceSpec};
use crate::ops::state_dir::{DirKind, PrepareDirRequest};

/// The committed broker-generic kernel rows this table serves.
pub const OPEN_PIDFD: &str = "open-pidfd";
pub const OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET: &str = "open-peer-pidfd-from-accepted-socket";
pub const POLL_CHILD_REAPED: &str = "poll-child-reaped";
pub const PREPARE_DIRECTORY: &str = "prepare-directory";
pub const KILL_CGROUP: &str = "kill-cgroup";
pub const SIGNAL_PIDFD: &str = "signal-pidfd";
pub const DEREGISTER_PIDFD: &str = "deregister-pidfd";
pub const SPAWN_PROCESS: &str = "spawn-process";
pub const DELEGATE_CGROUP_V2: &str = "delegate-cgroup-v2";
pub const OPEN_CGROUP_DIR: &str = "open-cgroup-dir";
pub const OBSERVE_PROCESS: &str = "observe-process";

/// The fixed process values one kernel closure captures at serve time.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// The broker's state root (the daemon state dir): the tree the
    /// swtpm-dir first-run hardening and the state-cell store live under.
    pub state_dir: PathBuf,
    /// The broker runtime root (the private socket's directory): the tree
    /// a Device-owned worker's per-Guest socket directory must strictly
    /// live under before the broker opens it to the worker's principal.
    pub runtime_root: PathBuf,
    /// The daemon's uid, as the broker resolved it from its configuration.
    pub daemon_uid: u32,
    /// The daemon's gid, as the broker resolved it from its configuration.
    pub daemon_gid: u32,
    /// The configured trusted bundle path. The spawn kernel loads the
    /// bundle resolver from it per invocation when a launch needs bundle
    /// knowledge (the USBIP backend device-bind extension), mirroring the
    /// broker's per-request bundle reload authority; every other kernel
    /// stays bundle-free.
    pub bundle_path: PathBuf,
}

/// The kernel handler table over one serve-time config.
///
/// Every kernel is registered under its committed broker-generic row name;
/// the mixed [`KernelDispatcher`](crate::envelope::KernelDispatcher) routes
/// exactly those names to this table and forwards every other operation.
pub fn kernel_table(config: &KernelConfig) -> HandlerTable {
    let config = Arc::new(config.clone());
    HandlerTable::new()
        .with(OPEN_PIDFD, {
            let config = Arc::clone(&config);
            move |invocation| open_pidfd(&config, invocation)
        })
        .with(OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET, {
            move |invocation| open_peer_pidfd_from_accepted_socket(invocation)
        })
        .with(POLL_CHILD_REAPED, {
            move |invocation| poll_child_reaped(invocation)
        })
        .with(PREPARE_DIRECTORY, {
            move |invocation| prepare_directory(invocation)
        })
        .with(KILL_CGROUP, {
            move |invocation| kill_cgroup(invocation)
        })
        .with(SIGNAL_PIDFD, {
            move |invocation| signal_pidfd(invocation)
        })
        .with(DEREGISTER_PIDFD, {
            move |invocation| deregister_pidfd(invocation)
        })
        .with(SPAWN_PROCESS, {
            let config = Arc::clone(&config);
            move |invocation| spawn_process(&config, invocation)
        })
        .with(DELEGATE_CGROUP_V2, {
            let config = Arc::clone(&config);
            move |invocation| delegate_cgroup_v2(&config, invocation)
        })
        .with(OPEN_CGROUP_DIR, {
            move |invocation| open_cgroup_dir(invocation)
        })
        .with(OBSERVE_PROCESS, {
            move |invocation| observe_process(invocation)
        })
}

/// The pidfd-open kernel: `pidfd_open(pid)` plus the start-time
/// verification that closes the pid-reuse race, exactly as the retired
/// `OpenPidfd` arm's live handler ran it. The pidfd travels back over the
/// fd leg.
fn open_pidfd(
    _config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let pid = field_i64(invocation.payload, "pid")? as i32;
    let expected_start_time_ticks = field_i64(invocation.payload, "expectedStartTimeTicks")? as u64;
    let outcome = crate::live_handlers::live_open_pidfd(pid, expected_start_time_ticks)
        .map_err(|error| errored(format!("open-pidfd: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "pid": outcome.pid,
            "verifiedStartTimeTicks": outcome.verified_start_time_ticks,
        }))?,
        fds: vec![outcome.pidfd],
    })
}

/// The peer-pidfd kernel: derive the accepted socket's peer pidfd via
/// `SO_PEERPIDFD`, exactly as the retired `OpenPeerPidfdFromAcceptedSocket`
/// arm's sys layer ran it. The pidfd travels back over the fd leg.
fn open_peer_pidfd_from_accepted_socket(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let socket = request_fd(invocation, 0)?;
    let pidfd = crate::sys::peer_pidfd_from_accepted_socket(socket.as_raw_fd())
        .map_err(|error| errored(format!("open-peer-pidfd-from-accepted-socket: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: vec![pidfd],
    })
}

/// The signal kernel: `pidfd_send_signal` on the attached pidfd, exactly
/// as the retired `SignalRunner` arm's sys layer ran it.
fn signal_pidfd(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let signal = field_i64(invocation.payload, "signal")? as nix::libc::c_int;
    let pidfd = request_fd(invocation, 0)?;
    crate::sys::pidfd_sys::pidfd_send_signal(pidfd.as_fd(), signal)
        .map_err(|error| errored(format!("signal-pidfd: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "signaled": true }))?,
        fds: Vec::new(),
    })
}

/// The deregister kernel: remove the invocation's runner-pidfd registry
/// record (the pidfd cell remove path). The record is keyed by the
/// invocation id the spawn-process kernel registered under, so the
/// resource-agnostic kernel never learns a runner id.
fn deregister_pidfd(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let removed = crate::runtime::runner_pidfds().remove(invocation.ctx.invocation_id);
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "removed": removed }))?,
        fds: Vec::new(),
    })
}

/// The reap-probe kernel: a pidfd-keyed non-blocking `waitid` probe over
/// the broker's reap cell, exactly as the retired `PollChildReaped` arm's
/// targeted reap ran it. The broker reaps the child it spawned, records
/// the notification under the invocation id, and returns the outcome; a
/// child the SIGCHLD loop already reaped is answered from the recorded
/// notification.
fn poll_child_reaped(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    use d2b_contracts_broker::broker_wire::{ChildExitKind, ChildExitStatus, ChildReapedNotification};
    use nix::errno::Errno;
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};

    let pidfd = request_fd(invocation, 0)?;
    let invocation_id = invocation.ctx.invocation_id;
    let wait_flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG;
    match waitid(Id::PIDFd(pidfd.as_fd()), wait_flags) {
        Ok(WaitStatus::Exited(pid, code)) => {
            let notification = ChildReapedNotification {
                runner_id: invocation_id.to_owned(),
                pid: pid.as_raw(),
                exit_status: ChildExitStatus {
                    kind: ChildExitKind::Exited,
                    code: Some(code),
                    signal: None,
                },
                reaped_at_ms: reaped_at_ms_now(),
            };
            crate::runtime::runner_pidfds().remove(invocation_id);
            crate::runtime::push_child_reap_notification(notification.clone());
            reap_result(notification, "reaped")
        }
        Ok(WaitStatus::Signaled(pid, sig, _)) => {
            let sig_num = sig as nix::libc::c_int;
            let notification = ChildReapedNotification {
                runner_id: invocation_id.to_owned(),
                pid: pid.as_raw(),
                exit_status: ChildExitStatus {
                    kind: if sig_num == nix::libc::SIGKILL {
                        ChildExitKind::Killed
                    } else {
                        ChildExitKind::Signaled
                    },
                    code: None,
                    signal: Some(sig_num),
                },
                reaped_at_ms: reaped_at_ms_now(),
            };
            crate::runtime::runner_pidfds().remove(invocation_id);
            crate::runtime::push_child_reap_notification(notification.clone());
            reap_result(notification, "reaped")
        }
        Ok(WaitStatus::StillAlive) | Ok(_) => {
            // Still running; a notification can only exist if the SIGCHLD
            // loop reaped it between the probe and the drain.
            match drain_notification(invocation_id) {
                Some(notification) => reap_result(notification, "reaped"),
                None => reap_result_absent("stillAlive"),
            }
        }
        Err(Errno::ECHILD) => {
            // The SIGCHLD loop already reaped it: the recorded notification
            // is the answer, and a lost notification (buffer overflow) is
            // still a terminal already-reaped outcome.
            match drain_notification(invocation_id) {
                Some(notification) => reap_result(notification, "alreadyReaped"),
                None => reap_result_absent("alreadyReaped"),
            }
        }
        Err(error) => Err(errored(format!("poll-child-reaped: {error}"))),
    }
}

/// The directory-prepare kernel: the generic state/runtime directory
/// creation with the path-safety posture of the retired
/// `PrepareStateDir`/`PrepareRuntimeDir` arms' shared helper.
fn prepare_directory(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let kind = match field_str(invocation.payload, "kind")? {
        "runtime" => DirKind::RuntimeDir,
        "state" => DirKind::StateDir,
        other => return Err(refused(format!("kind: unknown directory kind {other}"))),
    };
    let base_dir = PathBuf::from(field_str(invocation.payload, "baseDir")?);
    let vm_id_or_scope = field_str(invocation.payload, "vmIdOrScope")?.to_owned();
    let mode = field_i64(invocation.payload, "mode")? as u32;
    let owner_uid = field_i64(invocation.payload, "ownerUid")? as u32;
    let owner_gid = field_i64(invocation.payload, "ownerGid")? as u32;
    let created_paths = optional_str_array(invocation.payload, "createdPaths")?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let audit = crate::ops::state_dir::prepare_dir(&PrepareDirRequest {
        kind,
        base_dir,
        vm_id_or_scope,
        mode,
        owner_uid,
        owner_gid,
        created_paths,
    })
    .map_err(|error| errored(format!("prepare-directory: {error}")))?;
    let result = serde_json::to_value(&audit)
        .map_err(|error| errored(format!("prepare-directory result: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(result)?,
        fds: Vec::new(),
    })
}

/// The cgroup-kill kernel: kill exactly the named leaf under the
/// delegated slice, refusing any path outside it - the resource-agnostic
/// core of the retired `CgroupKill` arm.
fn kill_cgroup(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let cgroup_path = PathBuf::from(field_str(invocation.payload, "cgroupPath")?);
    if !strictly_inside_delegated_slice(&cgroup_path) {
        return Err(refused(
            "cgroupPath: outside the delegated d2b.slice subtree".to_owned(),
        ));
    }
    let backend = d2b_host::cgroup::RealCgroupBackend::new();
    d2b_host::cgroup::cgroup_kill_leaf_only(&backend, &cgroup_path, std::slice::from_ref(&cgroup_path))
        .map_err(|error| errored(format!("kill-cgroup: {}", error.code())))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The cgroup-delegation kernel: enable the delegated slice's controllers
/// and chown the subtree to the daemon principal, exactly as the retired
/// `DelegateCgroupV2` arm's live helper ran it.
fn delegate_cgroup_v2(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let path = PathBuf::from(field_str(invocation.payload, "path")?);
    if !path.starts_with(Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE)) {
        return Err(refused(
            "path: outside the delegated d2b.slice subtree".to_owned(),
        ));
    }
    let backend = d2b_host::cgroup::RealCgroupBackend::new();
    crate::ops::cgroup::create_d2b_slice(
        &backend,
        Path::new("/sys/fs/cgroup"),
        &path,
        config.daemon_uid,
        config.daemon_gid,
    )
    .map_err(|error| errored(format!("delegate-cgroup-v2: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The cgroup-dir kernel: open the named cgroup directory under the
/// delegated slice with the path-safe `O_PATH` open, returning the
/// descriptor over the fd leg - the resource-agnostic core of the retired
/// `OpenCgroupDir` arm.
fn open_cgroup_dir(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let path = PathBuf::from(field_str(invocation.payload, "path")?);
    if !path.starts_with(Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE)) {
        return Err(refused(
            "path: outside the delegated d2b.slice subtree".to_owned(),
        ));
    }
    let fd = crate::sys::path_safe::open_dir_path_safe(&path)
        .map_err(|error| errored(format!("open-cgroup-dir: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "path": path.display().to_string() }))?,
        fds: vec![fd],
    })
}

/// The observe kernel: the /proc observation probe (presence, state,
/// start time, executable) plus the registry-cell check that the
/// invocation's registered pidfd names the observed pid. Expectation
/// comparison stays on the family side.
fn observe_process(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let pid = field_i64(invocation.payload, "pid")? as i32;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    let (present, state, start_time_ticks) = match &stat {
        Ok(stat) => (
            true,
            parse_proc_state(stat),
            crate::ops::pidfd::parse_proc_stat_start_time(stat),
        ),
        Err(_) => (false, None, None),
    };
    let executable = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|path| path.display().to_string());
    let invocation_id = invocation.ctx.invocation_id;
    let registered = crate::runtime::runner_pidfds().contains_key(invocation_id)
        && crate::runtime::runner_pidfds()
            .get(invocation_id)
            .is_some_and(|pidfd| pidfd_pid(pidfd.as_fd()) == Some(pid));
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "pid": pid,
            "present": present,
            "state": state,
            "startTimeTicks": start_time_ticks,
            "executable": executable,
            "registered": registered,
        }))?,
        fds: Vec::new(),
    })
}

/// The spawn kernel: the privileged spawn of one fully-resolved runner
/// plan. The daemon-side family handler validates the typed request
/// against its resolver and carries the resolved plan (plus the
/// activation input, the swtpm identity, and the Device-worker scope it
/// derived) in the payload; this kernel runs the same live spawn the
/// retired `SpawnRunner` arm ran, registers the spawned pidfd under the
/// invocation id so the broker's SIGCHLD reaper owns the child, and
/// returns the pidfd and any extra descriptors over the fd leg.
fn spawn_process(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let mut plan_input = parse_plan(invocation.payload)?;
    let role = parse_role(invocation.payload)?;
    let serving_worker = optional_field_bool(invocation.payload, "servingWorker")?
        .unwrap_or(false);
    let identity = parse_runner_identity(invocation.payload)?;
    let activation_input: Option<d2b_contracts_resource::v3::ActivationRunnerInput> =
        optional_parse_field(invocation.payload, "activationInput")?;
    let swtpm_identity = optional_parse_swtpm_identity(invocation.payload)?;
    let device_worker = parse_device_worker(invocation.payload)?;
    let request_fds = invocation
        .fds
        .iter()
        .map(|fd| {
            fd.try_clone()
                .map_err(|error| errored(format!("spawn-process request fd: {error}")))
        })
        .collect::<Result<Vec<OwnedFd>, DispatchFailure>>()?;
    // The USBIP backend device binds (the retired arm's
    // `extend_usbip_backend_device_binds`): the kernel loads the bundle
    // resolver from the captured bundle path - the same per-request
    // reload authority the broker's answer path uses - and extends the
    // parsed plan's mount policy with the live locked busid device nodes
    // before the sandbox plan builds, so the kernel produces the same
    // mountPolicy the retired arm did.
    if matches!(role, d2b_contracts_broker::broker_wire::RunnerRole::Usbip) {
        let resolver = match crate::runtime::load_kernel_resolver(&config.bundle_path) {
            crate::runtime::BundleSlot::Loaded(resolver) => resolver,
            crate::runtime::BundleSlot::Unavailable => {
                return Err(refused("spawn-process: usbip backend bundle resolver unavailable"));
            }
            crate::runtime::BundleSlot::Tampered { .. } => {
                return Err(refused("spawn-process: usbip backend bundle tampered"));
            }
        };
        crate::runtime::extend_usbip_backend_device_binds(
            &resolver,
            &identity.vm_id,
            &identity.role_id,
            &role,
            &mut plan_input.mount_policy,
        )
        .map_err(|error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        })?;
    }
    // The stale-socket preflight cleanups (the retired arm's three
    // `cleanup_*_stale_socket` calls), on the final argv the daemon-side
    // handler composed.
    crate::runtime::cleanup_cloud_hypervisor_stale_sockets(&role, &plan_input.argv).map_err(
        |error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        },
    )?;
    crate::runtime::cleanup_video_stale_socket(&role, &plan_input.argv).map_err(|error| {
        errored(format!(
            "spawn-process: {}",
            crate::runtime::broker_error_kernel_detail(error)
        ))
    })?;
    crate::runtime::cleanup_otel_host_bridge_stale_socket(&role, &plan_input.argv).map_err(
        |error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        },
    )?;
    // The serving-worker ACL grant (the retired arm's
    // `prepare_runner_launch_identity` serving posture): before the
    // spawn and before any descriptor is passed to the child, open the
    // two ticket-named trees to the runner principal the plan
    // establishes.
    if serving_worker {
        crate::live_handlers::grant_serving_worker_launch_acls(
            &plan_input.argv,
            plan_input.uid,
            &config.runtime_root,
        )
        .map_err(|error| errored(format!("spawn-process: {error}")))?;
    }
    // The duplicate-runner guard (the retired arm's
    // `reserve_runner_id_for_spawn`): refuse a second live spawn for the
    // same runner BEFORE the child is spawned so a duplicate is never
    // created - a rejected-after-spawn duplicate would leak an orphan
    // child and pollute the existing registration.
    let runner_id = crate::runtime::runner_registry_key(
        &identity.vm_id,
        &identity.role_id,
        identity.resource_ref.as_ref(),
        identity.resource_uid.as_ref(),
        identity.zone_uid.as_ref(),
        identity.runtime_scope,
    );
    if let Err(error) = crate::runtime::reserve_runner_id_for_spawn(&runner_id) {
        return Err(match error {
            crate::runtime::BrokerError::Protocol(message) => refused(message),
            other => errored(format!("spawn-process reserve: {other:?}")),
        });
    }
    let outcome = crate::live_handlers::live_spawn_runner(
        &plan_input,
        Vec::new(),
        request_fds,
        activation_input.as_ref(),
        &config.state_dir,
        swtpm_identity.as_ref(),
        &device_worker,
        &config.runtime_root,
    )
    .map_err(|error| errored(format!("spawn-process: {error}")))?;
    // Register the spawned child under the invocation id so the SIGCHLD
    // reaper owns it; a registration failure reaps the child right here
    // so it cannot zombie.
    let invocation_id = invocation.ctx.invocation_id;
    if let Err(error) = crate::runtime::runner_pidfds().insert(
        invocation_id,
        duplicate(&outcome.pidfd).map_err(|error| errored(format!("spawn-process: {error}")))?,
    ) {
        crate::runtime::targeted_reap_runner(invocation_id, outcome.pidfd.as_fd());
        return Err(errored(format!("spawn-process registry: {error:?}")));
    }
    // Register the runner-id-keyed pidfd and metadata the broker's
    // existing removal paths (the SIGCHLD reaper, down/stop) key on -
    // the retired arm's `register_runner_pidfd` plus
    // `register_runner_metadata` - so those paths and the runner
    // observation surface work for kernel-spawned runners. A failure
    // rolls the spawn back exactly like the retired arm's
    // `cleanup_spawned_runner_after_failure`.
    if let Err(error) = crate::runtime::runner_pidfds().insert(
        &runner_id,
        duplicate(&outcome.pidfd).map_err(|error| errored(format!("spawn-process: {error}")))?,
    ) {
        crate::runtime::cleanup_spawned_runner_after_failure(&runner_id, outcome.pidfd.as_fd());
        let _ = crate::runtime::runner_pidfds().remove(invocation_id);
        return Err(errored(format!("spawn-process registry: {error:?}")));
    }
    let registration = crate::runtime::RunnerRegistration {
        vm_id: identity.vm_id.clone(),
        role_id: identity.role_id.clone(),
        resource_ref: identity.resource_ref.clone(),
        resource_uid: identity.resource_uid.clone(),
        zone_uid: identity.zone_uid.clone(),
        generation: identity.generation,
        runtime_scope: identity.runtime_scope,
        owner_ref: identity.owner_ref.clone(),
        provider_ref: identity.provider_ref.clone(),
        provider_identity: identity.provider_identity,
        template_identity: identity.template_identity,
        role,
        bundle_runner_intent_ref: identity.bundle_runner_intent_ref.clone(),
        pid: outcome.pid,
        start_time_ticks: outcome.start_time_ticks,
        binary_path: plan_input.binary_path.clone(),
        cgroup_subtree: plan_input.cgroup_placement.subtree.clone(),
        guest_execution: identity.guest_execution.clone(),
    };
    let replaced = crate::runtime::runner_metadata_registry()
        .lock()
        .map_err(|_| errored("spawn-process: runner metadata registry mutex poisoned".to_owned()))?
        .insert(runner_id.clone(), registration);
    if replaced.is_some() {
        // The reserve guard ran before the spawn, so a pre-existing
        // registration is a concurrent duplicate that slipped in between
        // the guard and the insert; roll the spawn back rather than
        // overwrite the live registration.
        crate::runtime::cleanup_spawned_runner_after_failure(&runner_id, outcome.pidfd.as_fd());
        let _ = crate::runtime::runner_pidfds().remove(invocation_id);
        return Err(errored(format!(
            "spawn-process metadata registry: runner {runner_id} already registered"
        )));
    }
    // Close the registration-window race: a child that exited between
    // clone3 and the registry insertion is reaped here and its
    // notification recorded under the invocation id for the reap probe.
    crate::runtime::targeted_reap_runner(invocation_id, outcome.pidfd.as_fd());
    let mut result = serde_json::json!({
        "pid": outcome.pid,
        "startTimeTicks": outcome.start_time_ticks,
        "usedForkFallback": outcome.used_fork_fallback,
        "pidfdIndex": 0,
        "extraFdIndexes": (1..=outcome.extra_response_fds.len() as u32)
            .collect::<Vec<u32>>(),
    });
    // The final plan's device binds (the USBIP backend extension the
    // retired arm applied to the mount policy): observable so the
    // sandbox plan the kernel built can be asserted.
    if !plan_input.mount_policy.device_binds.is_empty() {
        result["deviceBinds"] = serde_json::to_value(&plan_input.mount_policy.device_binds)
            .map_err(|error| errored(format!("spawn-process device binds: {error}")))?;
    }
    if let Some(audit) = &outcome.swtpm_dir_audit {
        result["swtpmDirAudit"] = serde_json::to_value(audit)
            .map_err(|error| errored(format!("spawn-process swtpm audit: {error}")))?;
    }
    let mut fds = Vec::with_capacity(1 + outcome.extra_response_fds.len());
    fds.push(outcome.pidfd);
    fds.extend(outcome.extra_response_fds);
    Ok(DispatchOutcome {
        result: canonical(result)?,
        fds,
    })
}

// ---------------------------------------------------------------------------
// Payload parsing
// ---------------------------------------------------------------------------

fn field<'a>(
    payload: &'a CanonicalJsonObject,
    key: &str,
) -> Result<&'a CanonicalJsonValue, DispatchFailure> {
    payload
        .get(key)
        .ok_or_else(|| refused(format!("{key}: missing")))
}

fn field_i64(payload: &CanonicalJsonObject, key: &str) -> Result<i64, DispatchFailure> {
    match field(payload, key)? {
        CanonicalJsonValue::Integer(value) => Ok(*value),
        _ => Err(refused(format!("{key}: expected an integer"))),
    }
}

fn field_str<'a>(
    payload: &'a CanonicalJsonObject,
    key: &str,
) -> Result<&'a str, DispatchFailure> {
    match field(payload, key)? {
        CanonicalJsonValue::String(value) => Ok(value.as_str()),
        _ => Err(refused(format!("{key}: expected a string"))),
    }
}

fn optional_field_bool(
    payload: &CanonicalJsonObject,
    key: &str,
) -> Result<Option<bool>, DispatchFailure> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    match value {
        CanonicalJsonValue::Bool(value) => Ok(Some(*value)),
        _ => Err(refused(format!("{key}: expected a boolean"))),
    }
}

fn field_str_array(payload: &CanonicalJsonObject, key: &str) -> Result<Vec<String>, DispatchFailure> {
    match field(payload, key)? {
        CanonicalJsonValue::Array(values) => values
            .iter()
            .map(|value| match value {
                CanonicalJsonValue::String(value) => Ok(value.clone()),
                _ => Err(refused(format!("{key}: expected an array of strings"))),
            })
            .collect(),
        _ => Err(refused(format!("{key}: expected an array of strings"))),
    }
}

fn field_i64_array(payload: &CanonicalJsonObject, key: &str) -> Result<Vec<i64>, DispatchFailure> {
    match field(payload, key)? {
        CanonicalJsonValue::Array(values) => values
            .iter()
            .map(|value| match value {
                CanonicalJsonValue::Integer(value) => Ok(*value),
                _ => Err(refused(format!("{key}: expected an array of integers"))),
            })
            .collect(),
        _ => Err(refused(format!("{key}: expected an array of integers"))),
    }
}

fn optional_str_array(
    payload: &CanonicalJsonObject,
    key: &str,
) -> Result<Vec<String>, DispatchFailure> {
    let Some(value) = payload.get(key) else {
        return Ok(Vec::new());
    };
    match value {
        CanonicalJsonValue::Array(values) => values
            .iter()
            .map(|value| match value {
                CanonicalJsonValue::String(value) => Ok(value.clone()),
                _ => Err(refused(format!("{key}: expected an array of strings"))),
            })
            .collect(),
        _ => Err(refused(format!("{key}: expected an array of strings"))),
    }
}

fn value_to_serde(value: &CanonicalJsonValue) -> Result<serde_json::Value, DispatchFailure> {
    serde_json::to_value(value).map_err(|error| errored(format!("payload value: {error}")))
}

fn parse_field<T: serde::de::DeserializeOwned>(
    payload: &CanonicalJsonObject,
    key: &str,
) -> Result<T, DispatchFailure> {
    let json = value_to_serde(field(payload, key)?)?;
    serde_json::from_value(json).map_err(|error| refused(format!("{key}: {error}")))
}

fn optional_parse_field<T: serde::de::DeserializeOwned>(
    payload: &CanonicalJsonObject,
    key: &str,
) -> Result<Option<T>, DispatchFailure> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    let json = value_to_serde(value)?;
    serde_json::from_value(json)
        .map(Some)
        .map_err(|error| refused(format!("{key}: {error}")))
}

/// The optional seccomp policy reference: a plain string or a `{"ref":
/// string}` object, absent meaning no policy.
fn optional_seccomp_ref(
    payload: &CanonicalJsonObject,
) -> Result<Option<String>, DispatchFailure> {
    let Some(value) = payload.get("seccompPolicyRef") else {
        return Ok(None);
    };
    match value {
        CanonicalJsonValue::String(value) => Ok(Some(value.clone())),
        CanonicalJsonValue::Object(fields) => match fields.get("ref") {
            Some(CanonicalJsonValue::String(value)) => Ok(Some(value.clone())),
            _ => Err(refused("seccompPolicyRef: expected {ref: string}")),
        },
        _ => Err(refused("seccompPolicyRef: expected a string or {ref: string}")),
    }
}

/// The optional umask: a plain integer or a `{"mask": integer}` object.
fn optional_umask(payload: &CanonicalJsonObject) -> Result<Option<u32>, DispatchFailure> {
    let Some(value) = payload.get("umask") else {
        return Ok(None);
    };
    let mask = match value {
        CanonicalJsonValue::Integer(mask) => *mask,
        CanonicalJsonValue::Object(fields) => match fields.get("mask") {
            Some(CanonicalJsonValue::Integer(mask)) => *mask,
            _ => return Err(refused("umask: expected {mask: integer}")),
        },
        _ => return Err(refused("umask: expected an integer or {mask: integer}")),
    };
    Ok(Some(mask as u32))
}

/// The optional user-namespace spec: `{hostUidForZero, hostGidForZero}`.
fn optional_user_namespace(
    payload: &CanonicalJsonObject,
) -> Result<Option<UserNamespaceSpec>, DispatchFailure> {
    let Some(value) = payload.get("userNamespace") else {
        return Ok(None);
    };
    let CanonicalJsonValue::Object(fields) = value else {
        return Err(refused("userNamespace: expected an object"));
    };
    let host_uid_for_zero = integer_field(fields, "hostUidForZero")? as u32;
    let host_gid_for_zero = integer_field(fields, "hostGidForZero")? as u32;
    Ok(Some(UserNamespaceSpec {
        host_uid_for_zero,
        host_gid_for_zero,
    }))
}

fn integer_field(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<i64, DispatchFailure> {
    match fields.get(key) {
        Some(CanonicalJsonValue::Integer(value)) => Ok(*value),
        _ => Err(refused(format!("{key}: expected an integer"))),
    }
}

/// The optional swtpm identity: `{guest, stateRoot, stateVolume}`.
fn optional_parse_swtpm_identity(
    payload: &CanonicalJsonObject,
) -> Result<Option<crate::ops::swtpm_dir::ResourceBackedSwtpm>, DispatchFailure> {
    let Some(value) = payload.get("swtpmIdentity") else {
        return Ok(None);
    };
    let CanonicalJsonValue::Object(fields) = value else {
        return Err(refused("swtpmIdentity: expected an object"));
    };
    let guest = match fields.get("guest") {
        Some(CanonicalJsonValue::String(guest)) => guest.clone(),
        _ => return Err(refused("swtpmIdentity.guest: expected a string")),
    };
    let state_root = match fields.get("stateRoot") {
        Some(CanonicalJsonValue::String(root)) => PathBuf::from(root),
        _ => return Err(refused("swtpmIdentity.stateRoot: expected a string")),
    };
    let state_volume = match fields.get("stateVolume") {
        Some(CanonicalJsonValue::String(volume)) => Some(volume.clone()),
        Some(CanonicalJsonValue::Null) | None => None,
        _ => return Err(refused("swtpmIdentity.stateVolume: expected a string or null")),
    };
    Ok(Some(crate::ops::swtpm_dir::ResourceBackedSwtpm {
        guest,
        state_root,
        state_volume,
    }))
}

/// The Device-worker launch scope: `{scope: {zoneUid, deviceRef,
/// deviceUid, guest} | null, bindsRuntimeSocket: bool}`, absent meaning
/// the default (no scope, no runtime socket).
fn parse_device_worker(
    payload: &CanonicalJsonObject,
) -> Result<crate::ops::device_worker::DeviceWorkerLaunch, DispatchFailure> {
    let Some(value) = payload.get("deviceWorker") else {
        return Ok(crate::ops::device_worker::DeviceWorkerLaunch::default());
    };
    let CanonicalJsonValue::Object(fields) = value else {
        return Err(refused("deviceWorker: expected an object"));
    };
    let scope = match fields.get("scope") {
        None | Some(CanonicalJsonValue::Null) => None,
        Some(CanonicalJsonValue::Object(scope)) => {
            let zone_uid = match scope.get("zoneUid") {
                Some(CanonicalJsonValue::String(uid)) => {
                    d2b_contracts::identity::ResourceUid::parse(uid.clone())
                        .map_err(|error| refused(format!("deviceWorker.scope.zoneUid: {error}")))?
                }
                _ => return Err(refused("deviceWorker.scope.zoneUid: expected a string")),
            };
            let device_ref = match scope.get("deviceRef") {
                Some(CanonicalJsonValue::String(reference)) => {
                    d2b_contracts::identity::ResourceRef::parse(reference)
                        .map_err(|error| refused(format!("deviceWorker.scope.deviceRef: {error}")))?
                }
                _ => return Err(refused("deviceWorker.scope.deviceRef: expected a string")),
            };
            let device_uid = match scope.get("deviceUid") {
                Some(CanonicalJsonValue::String(uid)) => {
                    d2b_contracts::identity::ResourceUid::parse(uid.clone())
                        .map_err(|error| refused(format!("deviceWorker.scope.deviceUid: {error}")))?
                }
                _ => return Err(refused("deviceWorker.scope.deviceUid: expected a string")),
            };
            let guest = match scope.get("guest") {
                Some(CanonicalJsonValue::String(guest)) => guest.clone(),
                _ => return Err(refused("deviceWorker.scope.guest: expected a string")),
            };
            Some(crate::ops::device_worker::DeviceWorkerScope {
                zone_uid,
                device_ref,
                device_uid,
                guest,
            })
        }
        _ => return Err(refused("deviceWorker.scope: expected an object or null")),
    };
    let binds_runtime_socket = match fields.get("bindsRuntimeSocket") {
        Some(CanonicalJsonValue::Bool(binds)) => *binds,
        None => false,
        _ => return Err(refused("deviceWorker.bindsRuntimeSocket: expected a boolean")),
    };
    Ok(crate::ops::device_worker::DeviceWorkerLaunch {
        scope,
        binds_runtime_socket,
    })
}

/// The fully-resolved spawn plan, parsed from the payload the daemon-side
/// family handler carried.
fn parse_plan(payload: &CanonicalJsonObject) -> Result<SpawnRunnerPlanInput, DispatchFailure> {
    Ok(SpawnRunnerPlanInput {
        binary_path: PathBuf::from(field_str(payload, "binaryPath")?),
        argv: field_str_array(payload, "argv")?,
        uid: field_i64(payload, "uid")? as u32,
        gid: field_i64(payload, "gid")? as u32,
        supplementary_groups: field_i64_array(payload, "supplementaryGroups")?
            .into_iter()
            .map(|value| value as u32)
            .collect(),
        env: field_str_array(payload, "env")?,
        capabilities: field_str_array(payload, "capabilities")?,
        namespaces: parse_field(payload, "namespaces")?,
        seccomp_policy_ref: optional_seccomp_ref(payload)?,
        mount_policy: parse_field(payload, "mountPolicy")?,
        cgroup_placement: parse_field(payload, "cgroupPlacement")?,
        root_carve_out: optional_field_bool(payload, "rootCarveOut")?.unwrap_or(false),
        skip_binary_exists_check: optional_field_bool(payload, "skipBinaryExistsCheck")?
            .unwrap_or(false),
        user_namespace: optional_user_namespace(payload)?,
        umask: optional_umask(payload)?,
    })
}

/// The runner role one spawn-process payload carries, in the wire
/// vocabulary's kebab-case spelling.
fn parse_role(payload: &CanonicalJsonObject) -> Result<d2b_contracts_broker::broker_wire::RunnerRole, DispatchFailure> {
    parse_field(payload, "role")
}

/// The runner identity one spawn-process payload carries: the registry
/// key fields plus the metadata the kernel registers under the runner id
/// (the retired arm's `register_runner_metadata`), all derived
/// daemon-side from the verified bundle and the typed request.
#[derive(Debug, Clone)]
struct RunnerIdentity {
    vm_id: String,
    role_id: String,
    resource_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    resource_uid: Option<d2b_contracts_resource::v3::ResourceUid>,
    zone_uid: Option<d2b_contracts_resource::v3::ResourceUid>,
    generation: Option<u64>,
    runtime_scope: Option<[u8; 32]>,
    owner_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    provider_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    provider_identity: Option<[u8; 32]>,
    template_identity: Option<[u8; 32]>,
    bundle_runner_intent_ref: String,
    guest_execution: Option<d2b_contracts_broker::broker_wire::GuestExecutionBinding>,
}

fn string_field(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<String, DispatchFailure> {
    match fields.get(key) {
        Some(CanonicalJsonValue::String(value)) => Ok(value.clone()),
        _ => Err(refused(format!("{key}: expected a string"))),
    }
}

fn optional_string_field(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<String>, DispatchFailure> {
    let Some(value) = fields.get(key) else {
        return Ok(None);
    };
    match value {
        CanonicalJsonValue::String(value) => Ok(Some(value.clone())),
        CanonicalJsonValue::Null => Ok(None),
        _ => Err(refused(format!("{key}: expected a string or null"))),
    }
}

fn optional_u64_field(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<u64>, DispatchFailure> {
    let Some(value) = fields.get(key) else {
        return Ok(None);
    };
    match value {
        CanonicalJsonValue::Integer(value) if *value >= 0 => Ok(Some(*value as u64)),
        CanonicalJsonValue::Null => Ok(None),
        _ => Err(refused(format!("{key}: expected a non-negative integer or null"))),
    }
}

/// An optional 32-byte identity digest, carried as an array of 32
/// uint8 integers (the wire vocabulary's spelling of `[u8; 32]`).
fn optional_byte_array(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<[u8; 32]>, DispatchFailure> {
    let Some(value) = fields.get(key) else {
        return Ok(None);
    };
    if matches!(value, CanonicalJsonValue::Null) {
        return Ok(None);
    }
    let CanonicalJsonValue::Array(values) = value else {
        return Err(refused(format!("{key}: expected an array of 32 integers or null")));
    };
    if values.len() != 32 {
        return Err(refused(format!("{key}: expected 32 integers")));
    }
    let mut out = [0u8; 32];
    for (index, value) in values.iter().enumerate() {
        match value {
            CanonicalJsonValue::Integer(value) if (0..=255).contains(value) => {
                out[index] = *value as u8;
            }
            _ => return Err(refused(format!("{key}: expected uint8 integers"))),
        }
    }
    Ok(Some(out))
}

fn optional_resource_ref(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<d2b_contracts_resource::v3::ResourceRef>, DispatchFailure> {
    optional_string_field(fields, key)?
        .map(|value| {
            d2b_contracts::identity::ResourceRef::parse(value.as_str()).map_err(|error| {
                refused(format!("{key}: {error}"))
            })
        })
        .transpose()
}

fn optional_resource_uid(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<d2b_contracts_resource::v3::ResourceUid>, DispatchFailure> {
    optional_string_field(fields, key)?
        .map(|value| {
            d2b_contracts::identity::ResourceUid::parse(value).map_err(|error| {
                refused(format!("{key}: {error}"))
            })
        })
        .transpose()
}

/// The optional Guest execution binding, carried as the wire vocabulary's
/// camelCase object (or null).
fn optional_guest_execution(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
) -> Result<Option<d2b_contracts_broker::broker_wire::GuestExecutionBinding>, DispatchFailure> {
    let Some(value) = fields.get("guestExecution") else {
        return Ok(None);
    };
    if matches!(value, CanonicalJsonValue::Null) {
        return Ok(None);
    }
    let json = value_to_serde(value)?;
    serde_json::from_value(json)
        .map(Some)
        .map_err(|error| refused(format!("guestExecution: {error}")))
}

/// The runner identity object one spawn-process payload carries.
fn parse_runner_identity(payload: &CanonicalJsonObject) -> Result<RunnerIdentity, DispatchFailure> {
    let CanonicalJsonValue::Object(fields) = field(payload, "runnerIdentity")? else {
        return Err(refused("runnerIdentity: expected an object"));
    };
    Ok(RunnerIdentity {
        vm_id: string_field(fields, "vmId")?,
        role_id: string_field(fields, "roleId")?,
        resource_ref: optional_resource_ref(fields, "resourceRef")?,
        resource_uid: optional_resource_uid(fields, "resourceUid")?,
        zone_uid: optional_resource_uid(fields, "zoneUid")?,
        generation: optional_u64_field(fields, "generation")?,
        runtime_scope: optional_byte_array(fields, "runtimeScope")?,
        owner_ref: optional_resource_ref(fields, "ownerRef")?,
        provider_ref: optional_resource_ref(fields, "providerRef")?,
        provider_identity: optional_byte_array(fields, "providerIdentity")?,
        template_identity: optional_byte_array(fields, "templateIdentity")?,
        bundle_runner_intent_ref: string_field(fields, "bundleRunnerIntentRef")?,
        guest_execution: optional_guest_execution(fields)?,
    })
}

// ---------------------------------------------------------------------------
// Result helpers
// ---------------------------------------------------------------------------

fn canonical(value: serde_json::Value) -> Result<CanonicalJsonObject, DispatchFailure> {
    serde_json::from_value(value).map_err(|error| errored(format!("result: {error}")))
}

fn refused(detail: impl Into<String>) -> DispatchFailure {
    DispatchFailure::with_detail(crate::envelope::HANDLER_REFUSED, detail)
}

fn errored(detail: impl Into<String>) -> DispatchFailure {
    DispatchFailure::with_detail(crate::envelope::HANDLER_ERRORED, detail)
}

fn request_fd<'a>(
    invocation: &'a DirectInvocation<'_>,
    index: usize,
) -> Result<&'a OwnedFd, DispatchFailure> {
    invocation.fds.get(index).ok_or_else(|| {
        DispatchFailure::with_detail(
            crate::envelope::FD_LEG,
            format!("request fd {index} missing"),
        )
    })
}

fn duplicate(fd: &OwnedFd) -> std::io::Result<OwnedFd> {
    nix::unistd::dup(fd.as_raw_fd())
        .map(crate::sys::owned_fd_from_raw)
        .map_err(|error| std::io::Error::from_raw_os_error(error as i32))
}

fn reaped_at_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// The wire vocabulary spelling of one exit kind.
fn exit_kind_str(kind: &d2b_contracts_broker::broker_wire::ChildExitKind) -> &'static str {
    match kind {
        d2b_contracts_broker::broker_wire::ChildExitKind::Exited => "exited",
        d2b_contracts_broker::broker_wire::ChildExitKind::Signaled => "signaled",
        d2b_contracts_broker::broker_wire::ChildExitKind::Killed => "killed",
    }
}

/// The pid of one pidfd, read from its fdinfo entry (`Pid:` line).
fn pidfd_pid(pidfd: std::os::fd::BorrowedFd<'_>) -> Option<i32> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd())).ok()?;
    info.lines().find_map(|line| {
        line.strip_prefix("Pid:").and_then(|value| value.trim().parse::<i32>().ok())
    })
}

/// The process state character (field 3) of one `/proc/<pid>/stat` line.
fn parse_proc_state(stat: &str) -> Option<String> {
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 1..)?;
    // The first token after the closing `)` of the comm is field 3, the
    // process state character.
    let state = rest.split_whitespace().next()?;
    Some(state.to_owned())
}

/// The reap notification recorded under one invocation id, drained from
/// the broker's reap cell.
fn drain_notification(invocation_id: &str) -> Option<d2b_contracts_broker::broker_wire::ChildReapedNotification> {
    let mut buffer = crate::runtime::child_reap_buffer()
        .lock()
        .ok()?;
    let index = buffer.iter().position(|notification| notification.runner_id == invocation_id)?;
    buffer.remove(index)
}

/// Render one reap outcome as the canonical result.
fn reap_result(
    notification: d2b_contracts_broker::broker_wire::ChildReapedNotification,
    outcome: &str,
) -> Result<DispatchOutcome, DispatchFailure> {
    let exit_status = notification.exit_status;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "reaped": true,
            "alreadyReaped": outcome == "alreadyReaped",
            "stillAlive": false,
            "pid": notification.pid,
            "exitKind": exit_kind_str(&exit_status.kind),
            "exitCode": exit_status.code,
            "exitSignal": exit_status.signal,
            "reapedAtMs": notification.reaped_at_ms,
        }))?,
        fds: Vec::new(),
    })
}

/// Render the terminal still-alive / already-reaped (no notification)
/// outcome as the canonical result.
fn reap_result_absent(outcome: &str) -> Result<DispatchOutcome, DispatchFailure> {
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "reaped": false,
            "alreadyReaped": outcome == "alreadyReaped",
            "stillAlive": outcome == "stillAlive",
            "pid": null,
            "exitKind": null,
            "exitCode": null,
            "exitSignal": null,
            "reapedAtMs": null,
        }))?,
        fds: Vec::new(),
    })
}

/// Whether one cgroup path is strictly inside the delegated slice (never
/// the slice root itself, so an ancestor kill cannot name every guest).
fn strictly_inside_delegated_slice(path: &Path) -> bool {
    let delegated = Path::new(crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE);
    path.starts_with(delegated) && path != delegated
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::CanonicalJsonValue;

    fn object(entries: &[(&str, CanonicalJsonValue)]) -> CanonicalJsonObject {
        let body = serde_json::to_string(
            &std::collections::BTreeMap::from_iter(
                entries
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), serde_json::to_value(value).expect("value")))
            )
        ).expect("object serializes");
        CanonicalJsonObject::parse(body.as_bytes()).expect("canonical object parses")
    }

    fn integer(value: i64) -> CanonicalJsonValue {
        CanonicalJsonValue::Integer(value)
    }

    fn string(value: &str) -> CanonicalJsonValue {
        CanonicalJsonValue::String(value.to_owned())
    }

    #[test]
    fn parse_plan_reads_every_committed_field() {
        let payload = object(&[
            ("binaryPath", string("/nix/store/x/bin/runner")),
            ("argv", CanonicalJsonValue::Array(vec![string("/nix/store/x/bin/runner"), string("--flag")])),
            ("uid", integer(1000)),
            ("gid", integer(1000)),
            ("supplementaryGroups", CanonicalJsonValue::Array(vec![integer(100), integer(200)])),
            ("env", CanonicalJsonValue::Array(vec![string("A=B")])),
            ("capabilities", CanonicalJsonValue::Array(vec![string("CAP_NET_ADMIN")])),
            (
                "namespaces",
                CanonicalJsonValue::Object(
                    [
                        ("mount", CanonicalJsonValue::Bool(true)),
                        ("pid", CanonicalJsonValue::Bool(true)),
                        ("net", CanonicalJsonValue::Bool(true)),
                        ("ipc", CanonicalJsonValue::Bool(true)),
                        ("uts", CanonicalJsonValue::Bool(true)),
                        ("user", CanonicalJsonValue::Bool(false)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
            ("seccompPolicyRef", string("w1-wayland-proxy")),
            (
                "mountPolicy",
                CanonicalJsonValue::Object(
                    [
                        ("readOnlyPaths", CanonicalJsonValue::Array(vec![])),
                        ("writablePaths", CanonicalJsonValue::Array(vec![])),
                        ("nixStoreReadOnly", CanonicalJsonValue::Bool(true)),
                        ("hideDeviceNodesByDefault", CanonicalJsonValue::Bool(true)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
            (
                "cgroupPlacement",
                CanonicalJsonValue::Object(
                    [
                        ("subtree", string("d2b.slice/zone-a/guest-1/role")),
                        ("controllers", CanonicalJsonValue::Array(vec![string("cpu")])),
                        ("delegated", CanonicalJsonValue::Bool(true)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
            ("rootCarveOut", CanonicalJsonValue::Bool(true)),
            ("skipBinaryExistsCheck", CanonicalJsonValue::Bool(false)),
            (
                "userNamespace",
                CanonicalJsonValue::Object(
                    [
                        ("hostUidForZero", integer(1000)),
                        ("hostGidForZero", integer(1000)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
            ("umask", integer(0o077)),
        ]);
        let plan = parse_plan(&payload).expect("the committed plan shape parses");
        assert_eq!(plan.binary_path, PathBuf::from("/nix/store/x/bin/runner"));
        assert_eq!(plan.argv, vec!["/nix/store/x/bin/runner", "--flag"]);
        assert_eq!(plan.uid, 1000);
        assert_eq!(plan.gid, 1000);
        assert_eq!(plan.supplementary_groups, vec![100, 200]);
        assert_eq!(plan.env, vec!["A=B"]);
        assert_eq!(plan.capabilities, vec!["CAP_NET_ADMIN"]);
        assert!(plan.namespaces.mount && plan.namespaces.pid && !plan.namespaces.user);
        assert_eq!(plan.seccomp_policy_ref.as_deref(), Some("w1-wayland-proxy"));
        assert!(plan.mount_policy.nix_store_read_only);
        assert_eq!(plan.cgroup_placement.subtree, "d2b.slice/zone-a/guest-1/role");
        assert!(plan.root_carve_out);
        assert!(!plan.skip_binary_exists_check);
        assert_eq!(
            plan.user_namespace,
            Some(UserNamespaceSpec {
                host_uid_for_zero: 1000,
                host_gid_for_zero: 1000,
            })
        );
        assert_eq!(plan.umask, Some(0o077));
    }

    #[test]
    fn parse_plan_tolerates_absent_optionals() {
        let payload = object(&[
            ("binaryPath", string("/bin/true")),
            ("argv", CanonicalJsonValue::Array(vec![string("/bin/true")])),
            ("uid", integer(1000)),
            ("gid", integer(1000)),
            ("supplementaryGroups", CanonicalJsonValue::Array(vec![])),
            ("env", CanonicalJsonValue::Array(vec![])),
            ("capabilities", CanonicalJsonValue::Array(vec![])),
            (
                "namespaces",
                CanonicalJsonValue::Object(
                    [
                        ("mount", CanonicalJsonValue::Bool(false)),
                        ("pid", CanonicalJsonValue::Bool(false)),
                        ("net", CanonicalJsonValue::Bool(false)),
                        ("ipc", CanonicalJsonValue::Bool(false)),
                        ("uts", CanonicalJsonValue::Bool(false)),
                        ("user", CanonicalJsonValue::Bool(false)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
            (
                "mountPolicy",
                CanonicalJsonValue::Object(
                    [
                        ("readOnlyPaths", CanonicalJsonValue::Array(vec![])),
                        ("writablePaths", CanonicalJsonValue::Array(vec![])),
                        ("nixStoreReadOnly", CanonicalJsonValue::Bool(false)),
                        ("hideDeviceNodesByDefault", CanonicalJsonValue::Bool(false)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
            (
                "cgroupPlacement",
                CanonicalJsonValue::Object(
                    [
                        ("subtree", string("d2b.slice/zone-a/guest-1/role")),
                        ("controllers", CanonicalJsonValue::Array(vec![])),
                        ("delegated", CanonicalJsonValue::Bool(false)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
        ]);
        let plan = parse_plan(&payload).expect("the minimal plan shape parses");
        assert_eq!(plan.seccomp_policy_ref, None);
        assert!(!plan.root_carve_out);
        assert_eq!(plan.user_namespace, None);
        assert_eq!(plan.umask, None);
    }

    #[test]
    fn parse_role_and_runner_identity_read_every_committed_field() {
        use d2b_contracts_broker::broker_wire::{GuestExecutionBinding, RunnerRole};

        let payload = object(&[
            ("role", string("cloud-hypervisor")),
            ("servingWorker", CanonicalJsonValue::Bool(false)),
            (
                "runnerIdentity",
                CanonicalJsonValue::Object(
                    [
                        ("vmId", string("vm-a")),
                        ("roleId", string("ch-runner")),
                        ("resourceRef", string("Process/vol-abcd")),
                        ("resourceUid", string("00000000-0000-4000-8000-000000000001")),
                        ("zoneUid", string("00000000-0000-4000-8000-000000000002")),
                        ("generation", integer(7)),
                        (
                            "runtimeScope",
                            CanonicalJsonValue::Array(
                                (0..32).map(|index| integer(index as i64)).collect(),
                            ),
                        ),
                        ("ownerRef", string("Volume/vol-abcd")),
                        ("providerRef", string("Provider/volume-virtiofs")),
                        (
                            "providerIdentity",
                            CanonicalJsonValue::Array(
                                (0..32).map(|index| integer(index as i64)).collect(),
                            ),
                        ),
                        (
                            "templateIdentity",
                            CanonicalJsonValue::Array(
                                (0..32).map(|index| integer(255 - index as i64)).collect(),
                            ),
                        ),
                        ("bundleRunnerIntentRef", string("runner:vm-a:ch-runner")),
                        (
                            "guestExecution",
                            CanonicalJsonValue::Object(
                                [
                                    ("targetUid", string("00000000-0000-4000-8000-000000000003")),
                                    (
                                        "bootIdentityDigest",
                                        CanonicalJsonValue::Array(
                                            (0..32).map(|index| integer(index as i64)).collect(),
                                        ),
                                    ),
                                    ("sessionGeneration", integer(1)),
                                    ("assignmentEpoch", integer(2)),
                                    ("providerGeneration", integer(3)),
                                    ("controllerGeneration", integer(4)),
                                ]
                                .into_iter()
                                .map(|(key, value)| (key.to_owned(), value))
                                .collect(),
                            ),
                        ),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
        ]);
        assert_eq!(
            parse_role(&payload).expect("role parses"),
            RunnerRole::CloudHypervisor
        );
        let identity = parse_runner_identity(&payload).expect("runner identity parses");
        assert_eq!(identity.vm_id, "vm-a");
        assert_eq!(identity.role_id, "ch-runner");
        assert_eq!(
            identity.resource_ref.as_ref().map(|reference| reference.to_canonical_string()),
            Some("Process/vol-abcd".to_owned())
        );
        assert_eq!(
            identity.resource_uid.as_ref().map(|uid| uid.as_str()),
            Some("00000000-0000-4000-8000-000000000001")
        );
        assert_eq!(
            identity.zone_uid.as_ref().map(|uid| uid.as_str()),
            Some("00000000-0000-4000-8000-000000000002")
        );
        assert_eq!(identity.generation, Some(7));
        assert_eq!(
            identity.runtime_scope,
            Some(std::array::from_fn(|index| index as u8))
        );
        assert_eq!(
            identity.owner_ref.as_ref().map(|reference| reference.to_canonical_string()),
            Some("Volume/vol-abcd".to_owned())
        );
        assert_eq!(
            identity.provider_ref.as_ref().map(|reference| reference.to_canonical_string()),
            Some("Provider/volume-virtiofs".to_owned())
        );
        assert_eq!(
            identity.provider_identity,
            Some(std::array::from_fn(|index| index as u8))
        );
        assert_eq!(
            identity.template_identity,
            Some(std::array::from_fn(|index| 255 - index as u8))
        );
        assert_eq!(identity.bundle_runner_intent_ref, "runner:vm-a:ch-runner");
        assert_eq!(
            identity.guest_execution,
            Some(GuestExecutionBinding {
                target_uid: d2b_contracts::identity::ResourceUid::parse(
                    "00000000-0000-4000-8000-000000000003",
                )
                .expect("target uid parses"),
                boot_identity_digest: std::array::from_fn(|index| index as u8),
                session_generation: 1,
                assignment_epoch: 2,
                provider_generation: 3,
                controller_generation: 4,
            })
        );
    }

    #[test]
    fn parse_runner_identity_tolerates_absent_optionals() {
        let payload = object(&[
            ("role", string("usbip")),
            ("servingWorker", CanonicalJsonValue::Bool(true)),
            (
                "runnerIdentity",
                CanonicalJsonValue::Object(
                    [
                        ("vmId", string("sys-work-usbipd")),
                        ("roleId", string("backend")),
                        ("resourceRef", CanonicalJsonValue::Null),
                        ("resourceUid", CanonicalJsonValue::Null),
                        ("zoneUid", CanonicalJsonValue::Null),
                        ("generation", CanonicalJsonValue::Null),
                        ("runtimeScope", CanonicalJsonValue::Null),
                        ("ownerRef", CanonicalJsonValue::Null),
                        ("providerRef", CanonicalJsonValue::Null),
                        ("providerIdentity", CanonicalJsonValue::Null),
                        ("templateIdentity", CanonicalJsonValue::Null),
                        ("bundleRunnerIntentRef", string("runner:sys-work-usbipd:backend")),
                        ("guestExecution", CanonicalJsonValue::Null),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
        ]);
        let identity = parse_runner_identity(&payload).expect("minimal identity parses");
        assert_eq!(identity.vm_id, "sys-work-usbipd");
        assert_eq!(identity.role_id, "backend");
        assert_eq!(identity.resource_ref, None);
        assert_eq!(identity.resource_uid, None);
        assert_eq!(identity.zone_uid, None);
        assert_eq!(identity.generation, None);
        assert_eq!(identity.runtime_scope, None);
        assert_eq!(identity.owner_ref, None);
        assert_eq!(identity.provider_ref, None);
        assert_eq!(identity.provider_identity, None);
        assert_eq!(identity.template_identity, None);
        assert_eq!(identity.guest_execution, None);
        // A malformed byte array is refused, not truncated.
        let bad = object(&[
            ("role", string("usbip")),
            ("servingWorker", CanonicalJsonValue::Bool(false)),
            (
                "runnerIdentity",
                CanonicalJsonValue::Object(
                    [
                        ("vmId", string("sys-work-usbipd")),
                        ("roleId", string("backend")),
                        (
                            "runtimeScope",
                            CanonicalJsonValue::Array(vec![integer(1), integer(2)]),
                        ),
                        ("bundleRunnerIntentRef", string("runner:x")),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                ),
            ),
        ]);
        assert!(
            parse_runner_identity(&bad).is_err(),
            "a short runtimeScope must be refused"
        );
    }

    #[test]
    fn the_delegated_slice_guard_refuses_the_slice_root_and_foreign_paths() {
        assert!(!strictly_inside_delegated_slice(Path::new(
            "/sys/fs/cgroup/d2b.slice"
        )));
        assert!(!strictly_inside_delegated_slice(Path::new("/sys/fs/cgroup/other.slice")));
        assert!(!strictly_inside_delegated_slice(Path::new("/etc/passwd")));
        assert!(strictly_inside_delegated_slice(Path::new(
            "/sys/fs/cgroup/d2b.slice/zone-a/guest-1/role"
        )));
    }

    #[test]
    fn proc_state_parses_the_third_field_after_the_comm() {
        assert_eq!(
            parse_proc_state("1 (systemd) S 2 1 1 0 -1 4194560 1 2 3 4 5 0 0 0 0 0 0 0 0 0 1 2 3"),
            Some("S".to_owned())
        );
        assert_eq!(
            parse_proc_state("42 (a comm with spaces) Z 1 2 3 4"),
            Some("Z".to_owned())
        );
        assert_eq!(parse_proc_state("no close paren"), None);
    }
}