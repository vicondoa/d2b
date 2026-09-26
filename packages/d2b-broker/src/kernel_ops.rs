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
pub const TAKE_CONTROLLER_BOOTSTRAP: &str = "take-controller-bootstrap";
pub const CONSUME_CELL: &str = "consume-cell";
pub const COMPLETE_CELL: &str = "complete-cell";
/// The U12 network-fds family kernels: the privileged, resource-agnostic
/// cores of the thirteen retired network-family wire arms, served
/// in-broker as broker-generic committed rows. The daemon-side family
/// handler (or the daemon's legacy effect-port/host-prep legs) resolves
/// the trusted bundle intents and carries the resolved values in the
/// payload; each kernel runs the same ops-module helper the retired arm
/// ran.
pub const APPLY_NFTABLES: &str = "apply-nftables";
pub const APPLY_NFTABLES_PROJECTION: &str = "apply-nftables-projection";
pub const APPLY_NM_UNMANAGED: &str = "apply-nm-unmanaged";
pub const APPLY_ROUTE: &str = "apply-route";
pub const APPLY_SYSCTL: &str = "apply-sysctl";
pub const CREATE_BRIDGE: &str = "create-bridge";
pub const DELETE_BRIDGE: &str = "delete-bridge";
pub const CREATE_PERSISTENT_TAP: &str = "create-persistent-tap";
pub const DELETE_PERSISTENT_TAP: &str = "delete-persistent-tap";
pub const CREATE_TAP_FD: &str = "create-tap-fd";
pub const SET_BRIDGE_PORT_FLAGS: &str = "set-bridge-port-flags";
pub const UPDATE_HOSTS_FILE: &str = "update-hosts-file";
pub const SEED_DNSMASQ_LEASE: &str = "seed-dnsmasq-lease";

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
pub fn kernel_table(config: KernelConfig) -> HandlerTable {
    let config = Arc::new(config);
    HandlerTable::new()
        .with(OPEN_PIDFD, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { open_pidfd(&config, invocation).await })
            }
        })
        .with(OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET, {
            move |invocation| Box::pin(open_peer_pidfd_from_accepted_socket(invocation))
        })
        .with(POLL_CHILD_REAPED, {
            move |invocation| Box::pin(poll_child_reaped(invocation))
        })
        .with(PREPARE_DIRECTORY, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { prepare_directory(&config)(invocation) })
            }
        })
        .with(KILL_CGROUP, {
            move |invocation| Box::pin(kill_cgroup(invocation))
        })
        .with(SIGNAL_PIDFD, {
            move |invocation| Box::pin(signal_pidfd(invocation))
        })
        .with(DEREGISTER_PIDFD, {
            move |invocation| Box::pin(deregister_pidfd(invocation))
        })
        .with(SPAWN_PROCESS, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { spawn_process(&config, invocation).await })
            }
        })
        .with(DELEGATE_CGROUP_V2, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { delegate_cgroup_v2(&config, invocation).await })
            }
        })
        .with(OPEN_CGROUP_DIR, {
            move |invocation| Box::pin(open_cgroup_dir(invocation))
        })
        .with(OBSERVE_PROCESS, {
            move |invocation| Box::pin(observe_process(invocation))
        })
        .with(TAKE_CONTROLLER_BOOTSTRAP, {
            move |invocation| Box::pin(take_controller_bootstrap(invocation))
        })
        .with(CONSUME_CELL, {
            move |invocation| Box::pin(consume_cell(invocation))
        })
        .with(COMPLETE_CELL, {
            move |invocation| Box::pin(complete_cell(invocation))
        })
        .with(APPLY_NFTABLES, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { apply_nftables(&config, invocation).await })
            }
        })
        .with(APPLY_NFTABLES_PROJECTION, {
            move |invocation| Box::pin(apply_nftables_projection(invocation))
        })
        .with(APPLY_NM_UNMANAGED, {
            move |invocation| Box::pin(apply_nm_unmanaged(invocation))
        })
        .with(APPLY_ROUTE, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { apply_route(&config, invocation).await })
            }
        })
        .with(APPLY_SYSCTL, {
            move |invocation| Box::pin(apply_sysctl(invocation))
        })
        .with(CREATE_BRIDGE, {
            move |invocation| Box::pin(create_bridge(invocation))
        })
        .with(DELETE_BRIDGE, {
            move |invocation| Box::pin(delete_bridge(invocation))
        })
        .with(CREATE_PERSISTENT_TAP, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { create_persistent_tap(&config, invocation).await })
            }
        })
        .with(DELETE_PERSISTENT_TAP, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { delete_persistent_tap(&config, invocation).await })
            }
        })
        .with(CREATE_TAP_FD, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { create_tap_fd(&config, invocation).await })
            }
        })
        .with(SET_BRIDGE_PORT_FLAGS, {
            let config = Arc::clone(&config);
            move |invocation| {
                let config = Arc::clone(&config);
                Box::pin(async move { set_bridge_port_flags(&config, invocation).await })
            }
        })
        .with(UPDATE_HOSTS_FILE, {
            move |invocation| Box::pin(update_hosts_file(invocation))
        })
        .with(SEED_DNSMASQ_LEASE, {
            move |invocation| Box::pin(seed_dnsmasq_lease(invocation))
        })
}

/// The pidfd-open kernel: `pidfd_open(pid)` plus the start-time
/// verification that closes the pid-reuse race, exactly as the retired
/// `OpenPidfd` arm's live handler ran it. The pidfd travels back over the
/// fd leg.
async fn open_pidfd(
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
async fn open_peer_pidfd_from_accepted_socket(
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
async fn signal_pidfd(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
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
async fn deregister_pidfd(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
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
async fn poll_child_reaped(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    use d2b_contracts_broker::broker_wire::{
        ChildExitKind, ChildExitStatus, ChildReapedNotification,
    };
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
fn prepare_directory(config: &KernelConfig) -> impl Fn(&DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let daemon_uid = config.daemon_uid;
    move |invocation: &DirectInvocation<'_>| {
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
        daemon_uid: Some(daemon_uid),
    })
    .map_err(|error| errored(format!("prepare-directory: {error}")))?;
    let result = serde_json::to_value(&audit)
        .map_err(|error| errored(format!("prepare-directory result: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(result)?,
        fds: Vec::new(),
    })
    }
}

/// The cgroup-kill kernel: kill exactly the named leaf under the
/// delegated slice, refusing any path outside it - the resource-agnostic
/// core of the retired `CgroupKill` arm.
async fn kill_cgroup(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let cgroup_path = PathBuf::from(field_str(invocation.payload, "cgroupPath")?);
    if !strictly_inside_delegated_slice(&cgroup_path) {
        return Err(refused(
            "cgroupPath: outside the delegated d2b.slice subtree".to_owned(),
        ));
    }
    let backend = d2b_host::cgroup::RealCgroupBackend::new();
    d2b_host::cgroup::cgroup_kill_leaf_only(
        &backend,
        &cgroup_path,
        std::slice::from_ref(&cgroup_path),
    )
    .map_err(|error| errored(format!("kill-cgroup: {}", error.code())))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The cgroup-delegation kernel: enable the delegated slice's controllers
/// and chown the subtree to the daemon principal, exactly as the retired
/// `DelegateCgroupV2` arm's live helper ran it.
async fn delegate_cgroup_v2(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let path = PathBuf::from(field_str(invocation.payload, "path")?);
    if !path.starts_with(Path::new(
        crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE,
    )) {
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
async fn open_cgroup_dir(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let path = PathBuf::from(field_str(invocation.payload, "path")?);
    if !path.starts_with(Path::new(
        crate::ops::cgroup::DEFAULT_DELEGATED_PARENT_SLICE,
    )) {
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
async fn observe_process(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let pid = field_i64(invocation.payload, "pid")? as i32;
    let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await;
    let (present, state, start_time_ticks) = match &stat {
        Ok(stat) => (
            true,
            parse_proc_state(stat),
            crate::ops::pidfd::parse_proc_stat_start_time(stat),
        ),
        Err(_) => (false, None, None),
    };
    // The registered binary is the spawn-time record of what the kernel
    // ACTUALLY exec'd (the daemon's resolved plan carried in the payload).
    // /proc/<pid>/exe can be unreadable from the broker's context even for
    // a present process (the runner runs under its own uid/namespace), so
    // the family side prefers this record and uses the readlink only as a
    // cross-check when both are readable.
    let executable = tokio::fs::read_link(format!("/proc/{pid}/exe"))
        .await
        .ok()
        .map(|path| path.display().to_string());
    let registered_binary =
        optional_parse_identity_fields(invocation.payload).and_then(|identity| {
            // Non-blocking try-lock (plan U8): a Busy collision reports no
            // registered binary, exactly like the old poisoned path.
            crate::runtime::runner_metadata_registry()
                .try_lock()
                .ok()
                .and_then(|registry| {
                    registry
                        .get(&crate::runtime::runner_registry_key(
                            &identity.vm_id,
                            &identity.role_id,
                            identity.resource_ref.as_ref(),
                            identity.resource_uid.as_ref(),
                            identity.zone_uid.as_ref(),
                            identity.runtime_scope,
                        ))
                        .map(|registration| registration.binary_path.display().to_string())
                })
        });
    let invocation_id = invocation.ctx.invocation_id;
    let registered = crate::runtime::runner_pidfds().contains_key(invocation_id)
        && match crate::runtime::runner_pidfds().get(invocation_id) {
            Some(pidfd) => pidfd_pid(pidfd.as_fd()).await == Some(pid),
            None => false,
        };
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "pid": pid,
            "present": present,
            "state": state,
            "startTimeTicks": start_time_ticks,
            "executable": executable,
            "registeredBinary": registered_binary,
            "registered": registered,
        }))?,
        fds: Vec::new(),
    })
}

/// The controller-bootstrap escrow surrender.
///
/// A daemon adopting a still-running ProviderController after its own
/// restart takes the escrow the spawn-process kernel retained at launch:
/// the controller's bootstrap sends have been landing in it, so the
/// daemon's bootstrap wait can consume them and the session acceptor can
/// establish the controller session. One-time: the escrow is removed from
/// the registry. An absent escrow is an absent result (the caller's
/// adoption then classifies ControllerBootstrapMissing and replaces the
/// runner), never a dispatch error.
async fn take_controller_bootstrap(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let Some(identity) = optional_parse_identity_fields(invocation.payload) else {
        return Err(refused(
            "take-controller-bootstrap: runner identity fields missing",
        ));
    };
    let key = crate::runtime::runner_registry_key(
        &identity.vm_id,
        &identity.role_id,
        identity.resource_ref.as_ref(),
        identity.resource_uid.as_ref(),
        identity.zone_uid.as_ref(),
        identity.runtime_scope,
    );
    // Non-blocking try-lock (plan U8): a Busy collision (a concurrent
    // spawn-process escrow insert, sub-microsecond) fails the take closed;
    // the daemon retries the adoption.
    let escrow = crate::runtime::controller_bootstrap_registry()
        .try_lock()
        .map_err(|_| errored("take-controller-bootstrap: registry busy (tokio try-lock)".to_owned()))?
        .remove(&key);
    let Some(escrow) = escrow else {
        return Ok(DispatchOutcome {
            result: canonical(serde_json::json!({ "taken": false }))?,
            fds: Vec::new(),
        });
    };
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "taken": true }))?,
        fds: vec![escrow],
    })
}

/// The optional runner-identity fields an observation carries, when the
/// caller attached them.
#[derive(Default)]
struct ObserveIdentityFields {
    vm_id: String,
    role_id: String,
    resource_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    resource_uid: Option<d2b_contracts_resource::v3::ResourceUid>,
    zone_uid: Option<d2b_contracts_resource::v3::ResourceUid>,
    runtime_scope: Option<[u8; 32]>,
}

fn optional_parse_identity_fields(payload: &CanonicalJsonObject) -> Option<ObserveIdentityFields> {
    let vm_id = field_str(payload, "vmId").ok()?.to_owned();
    let role_id = field_str(payload, "roleId").ok()?.to_owned();
    let resource_ref = match optional_field_str(payload, "resourceRef")? {
        None => None,
        Some(value) => Some(d2b_contracts_resource::v3::ResourceRef::parse(&value).ok()?),
    };
    let resource_uid = match optional_field_str(payload, "resourceUid")? {
        None => None,
        Some(value) => Some(d2b_contracts_resource::v3::ResourceUid::parse(value).ok()?),
    };
    let zone_uid = match optional_field_str(payload, "zoneUid")? {
        None => None,
        Some(value) => Some(d2b_contracts_resource::v3::ResourceUid::parse(value).ok()?),
    };
    let runtime_scope = optional_field_bytes32(payload, "runtimeScope")?;
    Some(ObserveIdentityFields {
        vm_id,
        role_id,
        resource_ref,
        resource_uid,
        zone_uid,
        runtime_scope,
    })
}

fn optional_field_str(payload: &CanonicalJsonObject, key: &str) -> Option<Option<String>> {
    match payload.get(key) {
        None => Some(None),
        Some(CanonicalJsonValue::String(value)) => Some(Some(value.clone())),
        Some(CanonicalJsonValue::Null) => Some(None),
        Some(_) => None,
    }
}

fn optional_field_bytes32(payload: &CanonicalJsonObject, key: &str) -> Option<Option<[u8; 32]>> {
    match payload.get(key) {
        None => Some(None),
        Some(CanonicalJsonValue::Null) => Some(None),
        Some(CanonicalJsonValue::Array(values)) => {
            let mut bytes = [0u8; 32];
            if values.len() != 32 {
                return None;
            }
            for (index, value) in values.iter().enumerate() {
                match value {
                    CanonicalJsonValue::Integer(byte) if (0..=255).contains(byte) => {
                        bytes[index] = *byte as u8;
                    }
                    _ => return None,
                }
            }
            Some(Some(bytes))
        }
        Some(_) => None,
    }
}

/// The cell-consume kernel: compare-and-consume one declared one-time
/// state cell under the canonical identity the payload carries and the
/// envelope-attested initiating principal. The committed row is the
/// declaration surface: the cell name and its durability facet flow from
/// the row's own `stateCell`/`cellDurability` facets (U3/KTD3), so the
/// kernel is generic - it never learns a lease shape. The durable
/// pre-commit happens before `Granted` returns, so a broker crash between
/// the commit and the completion leaves an `unknown` record the retried
/// invocation reconciles under its id; a completed record refuses
/// re-consume across broker restarts (AE2).
async fn consume_cell(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let row = crate::catalog::BrokerOperationRow::find(CONSUME_CELL)
        .expect("consume-cell is a committed row");
    let cell = row
        .state_cell
        .expect("consume-cell declares its state cell");
    let durability = row
        .cell_durability
        .expect("consume-cell declares its cell durability");
    let identity = cell_identity(invocation.payload)?;
    let principal = initiating_principal(invocation);
    let decision = crate::state_cells::broker_store()
        .consume(cell, &identity, &principal, durability)
        .map_err(|error| errored(format!("consume-cell: {error}")))?;
    match decision {
        crate::state_cells::ConsumeDecision::Granted
        | crate::state_cells::ConsumeDecision::Reconciled => Ok(DispatchOutcome {
            result: canonical(serde_json::json!({ "consumed": true }))?,
            fds: Vec::new(),
        }),
        crate::state_cells::ConsumeDecision::InProgress => Err(refused("cell-in-progress")),
        crate::state_cells::ConsumeDecision::Replayed => Err(refused("cell-replayed")),
        // Replay under a different principal refuses: the invocation id
        // alone never gates a one-time grant (KTD3).
        crate::state_cells::ConsumeDecision::ForeignPrincipal => Err(refused("cell-caller-denied")),
    }
}

/// The cell-complete kernel: record completion for one claimed one-time
/// state cell under the same canonical identity and initiating principal
/// the consume leg used. The durable completed marker is what refuses a
/// replayed consume across broker restarts (AE2).
async fn complete_cell(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let row = crate::catalog::BrokerOperationRow::find(COMPLETE_CELL)
        .expect("complete-cell is a committed row");
    let cell = row
        .state_cell
        .expect("complete-cell declares its state cell");
    let identity = cell_identity(invocation.payload)?;
    let principal = initiating_principal(invocation);
    crate::state_cells::broker_store()
        .complete(cell, &identity, &principal)
        .map_err(|error| errored(format!("complete-cell: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "completed": true }))?,
        fds: Vec::new(),
    })
}

/// The canonical per-invocation identity of one cell call: the validated
/// payload object itself, serialized canonically. The row's declared
/// payload schema is the identity contract - the caller carries the full
/// identity fields (the lease's zone/guest/generations/policy revision/
/// operation id/operation/stop-only key, KTD3) and the envelope refuses
/// any payload outside the declared shape, so consume and complete of one
/// key always derive the same identity.
pub(crate) fn cell_identity(payload: &CanonicalJsonObject) -> Result<String, DispatchFailure> {
    serde_json::to_string(payload).map_err(|error| errored(format!("cell identity: {error}")))
}

/// The initiating principal as attested at the envelope boundary, rendered
/// for cell keys. Invocation ids appear in audit records and are not
/// secrets, so the principal is the replay gate - never the id alone
/// (KTD3).
fn initiating_principal(invocation: &DirectInvocation<'_>) -> String {
    invocation.ctx.chain.initiating_identity().to_owned()
}

/// The spawn kernel: the privileged spawn of one fully-resolved runner
/// plan. The daemon-side family handler validates the typed request
/// against its resolver and carries the resolved plan (plus the
/// activation input, the swtpm identity, and the Device-worker scope it
/// derived) in the payload; this kernel runs the same live spawn the
/// retired `SpawnRunner` arm ran, registers the spawned pidfd under the
/// invocation id so the broker's SIGCHLD reaper owns the child, and
/// returns the pidfd and any extra descriptors over the fd leg.
async fn spawn_process(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let mut plan_input = parse_plan(invocation.payload)?;
    let role = parse_role(invocation.payload)?;
    let serving_worker = optional_field_bool(invocation.payload, "servingWorker")?.unwrap_or(false);
    let identity = parse_runner_identity(invocation.payload)?;
    let preflight_socket_paths: Vec<std::path::PathBuf> =
        optional_parse_field(invocation.payload, "preflightSocketPaths")?
            .unwrap_or_default();
    let activation_input: Option<d2b_contracts_resource::v3::ActivationRunnerInput> =
        optional_parse_field(invocation.payload, "activationInput")?;
    let swtpm_identity = optional_parse_swtpm_identity(invocation.payload)?;
    let device_worker = parse_device_worker(invocation.payload)?;
    let mut request_fds = invocation
        .fds
        .iter()
        .map(|fd| {
            fd.try_clone()
                .map_err(|error| errored(format!("spawn-process request fd: {error}")))
        })
        .collect::<Result<Vec<OwnedFd>, DispatchFailure>>()?;
    // The ProviderController escrow (the retired arm's bootstrap handoff):
    // the daemon's launch path pre-arms a seqpacket pair per controller,
    // the child end rides the request to the child and the daemon end is
    // the escrow this kernel retains for the broker-side controller-
    // bootstrap registry, returning a clone so the daemon's marker can
    // re-arm its endpoint after the spawn.
    let posture_is_controller = matches!(
        role,
        d2b_contracts_broker::broker_wire::RunnerRole::ProviderController
    ) && !serving_worker;
    // The daemon's launch path pre-arms a seqpacket pair per controller,
    // the child end rides the request to the child and the daemon end is
    // the escrow this kernel retains for the broker-side controller-
    // bootstrap registry (custody: the pair stays open until the
    // controller's bootstrap send). No duplicate rides the answer leg:
    // a dup of the caller's own descriptor would trip the forward
    // carrier's anti-replay fence (a peer returning the caller's own
    // fd is refused with the fd-leg code), and the daemon already keeps
    // its own copy of the daemon end to wait on.
    let retained_controller_bootstrap = if posture_is_controller {
        let retained = request_fds.pop().ok_or_else(|| {
            errored("spawn-process: ProviderController bootstrap escrow fd is missing".to_owned())
        })?;
        Some(retained)
    } else {
        None
    };
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
                return Err(refused(
                    "spawn-process: usbip backend bundle resolver unavailable",
                ));
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
        .await
        .map_err(|error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        })?;
    }
    // The stale-socket preflight cleanups (the retired arm's three
    // `cleanup_*_stale_socket` calls). The guest runtime Provider declares
    // its socket-carrying argv paths; the kernel unlinks provably-stale
    // sockets before the spawn proceeds.
    crate::runtime::cleanup_stale_sockets(&preflight_socket_paths)
        .await
        .map_err(|error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        })?;
    crate::runtime::cleanup_video_stale_socket(&role, &plan_input.argv)
        .await
        .map_err(|error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        })?;
    crate::runtime::cleanup_otel_host_bridge_stale_socket(&role, &plan_input.argv)
        .await
        .map_err(|error| {
            errored(format!(
                "spawn-process: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        })?;
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
    .await
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
        crate::runtime::cleanup_spawned_runner_after_failure(&runner_id, outcome.pidfd.as_fd())
            .await;
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
    // Non-blocking try-lock (plan U8): a Busy collision fails the spawn
    // closed exactly like the old poisoned path (the child is rolled back
    // by the caller).
    let replaced = crate::runtime::runner_metadata_registry()
        .try_lock()
        .map_err(|_| errored("spawn-process: runner metadata registry busy (tokio try-lock)".to_owned()))?
        .insert(runner_id.clone(), registration);
    if replaced.is_some() {
        // The reserve guard ran before the spawn, so a pre-existing
        // registration is a concurrent duplicate that slipped in between
        // the guard and the insert; roll the spawn back rather than
        // overwrite the live registration.
        crate::runtime::cleanup_spawned_runner_after_failure(&runner_id, outcome.pidfd.as_fd())
            .await;
        let _ = crate::runtime::runner_pidfds().remove(invocation_id);
        return Err(errored(format!(
            "spawn-process metadata registry: runner {runner_id} already registered"
        )));
    }
    // Close the registration-window race: a child that exited between
    // clone3 and the registry insertion is reaped here and its
    // notification recorded under the invocation id for the reap probe.
    crate::runtime::targeted_reap_runner(invocation_id, outcome.pidfd.as_fd());
    // The retired arm delivered preopened response fds (console sockets,
    // controller bootstrap) as extra descriptors; the kernel path mints
    // none - live_spawn_runner starts with an empty extra set and the
    // kernel never extends it - so the result advertises the pidfd alone
    // rather than an always-empty extraFdIndexes range (the handler keeps
    // only fd 0 and would silently close any advertised extra).
    if let Some(bootstrap) = retained_controller_bootstrap {
        crate::runtime::controller_bootstrap_registry()
            .try_lock()
            .map_err(|_| {
                errored(
                    "spawn-process: controller bootstrap registry busy (tokio try-lock)".to_owned(),
                )
            })?
            .insert(runner_id.clone(), bootstrap);
    }
    let mut result = serde_json::json!({
        "pid": outcome.pid,
        "startTimeTicks": outcome.start_time_ticks,
        "usedForkFallback": outcome.used_fork_fallback,
        "pidfdIndex": 0,
        "controllerBootstrapFdIndex": null,
        "extraFdIndexes": [],
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
    Ok(DispatchOutcome {
        result: canonical(result)?,
        fds: vec![outcome.pidfd],
    })
}

// ---------------------------------------------------------------------------
// The U12 network-fds kernels
// ---------------------------------------------------------------------------

/// The broker-side bundle resolver for one kernel invocation, loaded from
/// the captured bundle path exactly as the spawn kernel loads it (the same
/// per-request reload authority the broker's answer path uses). The
/// tap/bridge kernels need bundle knowledge to re-derive the trusted tap
/// intent and the installed generation fence.
fn kernel_resolver(
    config: &KernelConfig,
    operation: &str,
) -> Result<Arc<d2b_core::bundle_resolver::BundleResolver>, DispatchFailure> {
    match crate::runtime::load_kernel_resolver(&config.bundle_path) {
        crate::runtime::BundleSlot::Loaded(resolver) => Ok(resolver),
        crate::runtime::BundleSlot::Unavailable => {
            Err(refused(format!("{operation}: bundle resolver unavailable")))
        }
        crate::runtime::BundleSlot::Tampered { .. } => {
            Err(refused(format!("{operation}: bundle tampered")))
        }
    }
}

/// The apply-nftables kernel: install or flush the framework's own
/// `inet d2b` table with the coexistence fence and the persisted-hash
/// drift check, exactly as the retired `ApplyNftables` arm's live backend
/// ran it. The daemon-side caller resolves the trusted nft intent and
/// carries the resolved script body, ownership id, and coexistence policy
/// in the payload.
async fn apply_nftables(
    _config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    use crate::ops::nft::ApplyWithCoexistenceError;
    let family = field_str(invocation.payload, "family")?.to_owned();
    let table = field_str(invocation.payload, "table")?.to_owned();
    let script_body = field_str(invocation.payload, "scriptBody")?.to_owned();
    let ownership_id = field_str(invocation.payload, "ownershipId")?.to_owned();
    let destroy = optional_field_bool(invocation.payload, "destroy")?.unwrap_or(false);
    let desired_hash = optional_str(invocation.payload, "desiredHash")?;
    let table_hash_after_apply = optional_str(invocation.payload, "tableHashAfterApply")?;
    let coexistence_policy: Option<d2b_core::host_w3::FirewallCoexistencePolicy> =
        optional_parse_field(invocation.payload, "coexistencePolicy")?;
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    let nft_binary = crate::runtime::nft_binary_path();
    let script_body = if destroy {
        crate::runtime::render_nft_destroy_script(&family, &table)
    } else {
        script_body
    };
    let persisted_hash = crate::runtime::persisted_nft_hash()
        .await
        .map_err(|error| errored(format!("apply-nftables: {error}")))?;
    let expected_hash = if destroy {
        None
    } else {
        desired_hash.or(persisted_hash).or(table_hash_after_apply)
    };
    crate::ops::nft::apply_with_coexistence(
        &exec,
        &nft_binary,
        &script_body,
        &ownership_id,
        coexistence_policy.as_ref(),
        expected_hash.as_deref(),
    )
    .await
    .map_err(|error| match error {
        ApplyWithCoexistenceError::CoexistenceRefused { manager, rationale } => {
            refused(format!("coexistence-refused: {manager:?}: {rationale}"))
        }
        ApplyWithCoexistenceError::ParseFailed(error) => {
            errored(format!("nft-script-parse-failed: {error}"))
        }
        ApplyWithCoexistenceError::CarveoutOrderingViolation(error) => {
            errored(format!("carveout-ordering-violation: {error}"))
        }
        ApplyWithCoexistenceError::DriftDetected { expected, observed } => errored(format!(
            "nftables-drift-detected: expected {expected}, observed {observed}"
        )),
        ApplyWithCoexistenceError::ForeignOwnership => refused("foreign-nft-ownership"),
        ApplyWithCoexistenceError::ReconcileExec(error) => errored(error.to_string()),
    })?;
    crate::ops::nft::persist_live_nft_hash(
        &exec,
        &nft_binary,
        &family,
        &table,
        &crate::runtime::nft_hash_sidecar_path(),
    )
    .await
    .map_err(|error| errored(format!("apply-nftables: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The apply-nftables-projection kernel: install or remove one
/// Provider-owned nftables projection under its ownership marker with the
/// installed-generation and desired-hash fences, exactly as the retired
/// `ApplyNftablesProjection` arm ran it.
async fn apply_nftables_projection(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let script_body = field_str(invocation.payload, "scriptBody")?.to_owned();
    let marker = field_str(invocation.payload, "marker")?.to_owned();
    let trusted_hash = field_str(invocation.payload, "trustedHash")?.to_owned();
    let caller_hash = optional_str(invocation.payload, "callerHash")?;
    let expected_generation = field_str(invocation.payload, "expectedGenerationId")?.to_owned();
    let installed_generation = field_str(invocation.payload, "installedGenerationId")?.to_owned();
    let action: d2b_contracts_broker::broker_wire::NftablesProjectionAction =
        match field_str(invocation.payload, "action")? {
            "apply" => d2b_contracts_broker::broker_wire::NftablesProjectionAction::Apply,
            "remove" => d2b_contracts_broker::broker_wire::NftablesProjectionAction::Remove,
            other => {
                return Err(refused(format!(
                    "apply-nftables-projection: unknown action {other}"
                )));
            }
        };
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    let projection_digest = crate::ops::nft::apply_nftables_projection(
        &exec,
        &crate::runtime::nft_binary_path(),
        &script_body,
        &marker,
        &trusted_hash,
        caller_hash.as_deref(),
        &expected_generation,
        &installed_generation,
        action,
    )
    .await
    .map(|result| result.projection_digest)
    .map_err(|error| errored(format!("apply-nftables-projection: {}", error.code())))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "projectionDigest": projection_digest }))?,
        fds: Vec::new(),
    })
}

/// The apply-nm-unmanaged kernel: write or remove the NetworkManager
/// unmanaged drop-in file with the reload behavior, exactly as the retired
/// `ApplyNmUnmanaged` arm's live backend ran it.
async fn apply_nm_unmanaged(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let destroy = optional_field_bool(invocation.payload, "destroy")?.unwrap_or(false);
    let intent = d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent {
        intent_id: field_str(invocation.payload, "intentId")?.to_owned(),
        file_path: PathBuf::from(field_str(invocation.payload, "filePath")?),
        contents: field_str(invocation.payload, "contents")?.to_owned(),
        mode: field_i64(invocation.payload, "mode")? as u32,
        owner: field_str(invocation.payload, "owner")?.to_owned(),
        group: field_str(invocation.payload, "group")?.to_owned(),
        reload_behavior: parse_field(invocation.payload, "reloadBehavior")?,
    };
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    if destroy {
        crate::ops::nm::remove_with_reload(&intent)
            .await
            .map_err(|error| errored(format!("apply-nm-unmanaged: {error}")))?;
    } else {
        crate::ops::nm::apply_with_reload(&exec, &intent)
            .await
            .map_err(|error| errored(format!("apply-nm-unmanaged: {error}")))?;
    }
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The apply-route kernel: apply or remove one ownership-marked route with
/// the durable UID-bound marker-record preflight, exactly as the retired
/// `ApplyRoute` arm's live backend ran it.
async fn apply_route(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let destroy = optional_field_bool(invocation.payload, "destroy")?.unwrap_or(false);
    let provenance: d2b_contracts_resource::v3::NetworkProvenance =
        parse_field(invocation.payload, "provenance")?;
    let intent = d2b_core::bundle_resolver::ResolvedRouteIntent {
        intent_id: field_str(invocation.payload, "intentId")?.to_owned(),
        route_spec: field_str(invocation.payload, "routeSpec")?.to_owned(),
        destination: field_str(invocation.payload, "destination")?.to_owned(),
        via: optional_str(invocation.payload, "via")?,
        device: optional_str(invocation.payload, "device")?,
        table: optional_str(invocation.payload, "table")?,
        owned: optional_field_bool(invocation.payload, "owned")?.unwrap_or(false),
        route_name: optional_str(invocation.payload, "routeName")?,
        provenance: Some(provenance.clone()),
        ownership_marker: optional_str(invocation.payload, "ownershipMarker")?,
    };
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    crate::ops::route::apply_with_preflight_owned(
        &exec,
        &crate::runtime::ip_binary_path(),
        &config.state_dir,
        &intent,
        &provenance,
        destroy,
    )
    .await
    .map_err(|error| errored(format!("apply-route: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The apply-sysctl kernel: write one key with readback verification, or
/// restore the destroy default, exactly as the retired `ApplySysctl` arm's
/// live backend ran it.
async fn apply_sysctl(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let key = field_str(invocation.payload, "key")?.to_owned();
    let destroy = optional_field_bool(invocation.payload, "destroy")?.unwrap_or(false);
    let value = if destroy {
        crate::runtime::destroy_sysctl_value(&key)
            .map_err(|error| {
                errored(format!(
                    "apply-sysctl: {}",
                    crate::runtime::broker_error_kernel_detail(error)
                ))
            })?
            .to_owned()
    } else {
        field_str(invocation.payload, "value")?.to_owned()
    };
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    crate::ops::sysctl::apply_with_readback(&exec, &key, &value)
        .await
        .map_err(|error| errored(format!("apply-sysctl: {error}")))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The create-bridge kernel: create the framework-owned bridge from the
/// resolved intent with the ownership-marker fence, exactly as the retired
/// `CreateBridge` arm ran it.
async fn create_bridge(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let intent = parse_resolved_bridge_intent(invocation)?;
    let bridge_intent_digest =
        crate::ops::network::create_bridge(&crate::ops::network::SystemBridgeBackend, &intent)
            .await
            .map_err(|error| errored(format!("create-bridge: {}", error.code())))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "bridgeIntentDigest": bridge_intent_digest }))?,
        fds: Vec::new(),
    })
}

/// The delete-bridge kernel: remove the framework-owned bridge after its
/// TAP removals, exactly as the retired `DeleteBridge` arm ran it.
async fn delete_bridge(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    let intent = parse_resolved_bridge_intent(invocation)?;
    let bridge_intent_digest =
        crate::ops::network::delete_bridge(&crate::ops::network::SystemBridgeBackend, &intent)
            .await
            .map_err(|error| errored(format!("delete-bridge: {}", error.code())))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "bridgeIntentDigest": bridge_intent_digest }))?,
        fds: Vec::new(),
    })
}

/// The resolved bridge intent one bridge kernel reconstructs from the
/// payload the daemon-side caller resolved from the trusted bundle.
fn parse_resolved_bridge_intent(
    invocation: &DirectInvocation<'_>,
) -> Result<d2b_core::bundle_resolver::ResolvedBridgeIntent, DispatchFailure> {
    Ok(d2b_core::bundle_resolver::ResolvedBridgeIntent {
        intent_id: field_str(invocation.payload, "intentId")?.to_owned(),
        scope_label: field_str(invocation.payload, "scopeLabel")?.to_owned(),
        bridge_ifname: d2b_contracts_resource::v3::IfName::parse(field_str(
            invocation.payload,
            "bridgeIfname",
        )?)
        .map_err(|error| refused(format!("bridgeIfname: {error}")))?,
        mtu: field_i64(invocation.payload, "mtu")? as u16,
        stp_disabled: optional_field_bool(invocation.payload, "stpDisabled")?.unwrap_or(false),
        multicast_snooping_disabled: optional_field_bool(
            invocation.payload,
            "multicastSnoopingDisabled",
        )?
        .unwrap_or(false),
        ipv6_suppressed: optional_field_bool(invocation.payload, "ipv6Suppressed")?
            .unwrap_or(false),
        ipv4_address: optional_str(invocation.payload, "ipv4Address")?
            .map(|value| {
                d2b_contracts_resource::v3::network::Ipv4Cidr::parse(&value).map_err(|_| {
                    refused(format!(
                        "ipv4Address: expected a validated IPv4 CIDR, got {value}"
                    ))
                })
            })
            .transpose()?,
        provenance: optional_parse_field(invocation.payload, "provenance")?,
        ownership_marker: optional_str(invocation.payload, "ownershipMarker")?,
    })
}

/// The create-persistent-tap kernel: create the persistent TAP from the
/// typed request, re-deriving the trusted tap intent from the broker's own
/// bundle copy, and persist the realization record - exactly what the
/// retired `CreatePersistentTap` arm ran.
async fn create_persistent_tap(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let req: d2b_contracts_broker::broker_wire::CreatePersistentTapRequest =
        typed_payload(invocation, "create-persistent-tap")?;
    let resolver = kernel_resolver(config, "create-persistent-tap")?;
    let exec =
        crate::ops::exec_reconcile::SystemLiveExec::new(config.daemon_uid, config.daemon_gid);
    let outcome = crate::ops::tap::live_create_persistent_tap(&exec, &resolver, &req, None)
        .await
        .map_err(|error| errored(format!("create-persistent-tap: {error}")))?;
    if let Err(error) = crate::ops::network::persist_persistent_tap_realization(
        &config.state_dir,
        &req,
        &outcome.tap_ifname,
    )
    .await
    {
        let cleanup = crate::ops::network::PersistentTapBackend::delete_tap(
            &crate::ops::network::SystemPersistentTapBackend,
            outcome.tap_ifname.as_str(),
        )
        .await;
        if let Err(cleanup) = cleanup {
            return Err(errored(format!(
                "create-persistent-tap: {} (cleanup failed: {})",
                error.code(),
                cleanup.code()
            )));
        }
        let _ = crate::ops::network::remove_persistent_tap_realization(
            &config.state_dir,
            &req.attachment_id,
        )
        .await;
        return Err(errored(format!("create-persistent-tap: {}", error.code())));
    }
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "bridge": outcome.bridge_ifname.as_ref().map(|ifname| ifname.as_str()),
            "tap": outcome.tap_ifname.as_str(),
        }))?,
        fds: Vec::new(),
    })
}

/// The delete-persistent-tap kernel: remove one trusted attachment
/// realization under the exact generation fences, exactly what the retired
/// `DeletePersistentTap` arm ran.
async fn delete_persistent_tap(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let req: d2b_contracts_broker::broker_wire::DeletePersistentTapRequest =
        typed_payload(invocation, "delete-persistent-tap")?;
    let resolver = kernel_resolver(config, "delete-persistent-tap")?;
    let installed = resolver
        .installed_generation_identity()
        .ok_or_else(|| refused("delete-persistent-tap: installed-generation-unavailable"))?;
    if installed.as_str() != req.expected_bundle_generation.as_str() {
        return Err(refused(
            "delete-persistent-tap: stale-projection-generation",
        ));
    }
    let realization = crate::ops::network::load_persistent_tap_realization(&config.state_dir, &req)
        .await
        .map_err(|error| errored(format!("delete-persistent-tap: {}", error.code())))?;
    let attachment_digest = crate::ops::network::delete_persistent_tap(
        &crate::ops::network::SystemPersistentTapBackend,
        &realization,
        &req,
    )
    .await
    .map_err(|error| errored(format!("delete-persistent-tap: {}", error.code())))?;
    crate::ops::network::mark_persistent_tap_realization_deleted(
        &config.state_dir,
        &req.attachment_id,
    )
    .await
    .map_err(|error| errored(format!("delete-persistent-tap: {}", error.code())))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "attachmentDigest": attachment_digest }))?,
        fds: Vec::new(),
    })
}

/// The create-tap-fd kernel: create one VMM TAP and return its descriptor
/// over the fd leg, exactly what the retired `CreateTapFd` arm ran. This is
/// the ONLY fd-bearing network kernel; the row declares the `any` fd kind.
async fn create_tap_fd(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let req: d2b_contracts_broker::broker_wire::CreateTapFdRequest =
        typed_payload(invocation, "create-tap-fd")?;
    let resolver = kernel_resolver(config, "create-tap-fd")?;
    let exec =
        crate::ops::exec_reconcile::SystemLiveExec::new(config.daemon_uid, config.daemon_gid);
    let outcome = crate::ops::tap::live_create_tap_fd(&exec, &resolver, &req, None)
        .await
        .map_err(|error| errored(format!("create-tap-fd: {error}")))?;
    let fd = outcome
        .fd
        .ok_or_else(|| errored("create-tap-fd: produced no tap fd".to_owned()))?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "bridge": outcome.bridge_ifname.as_ref().map(|ifname| ifname.as_str()),
            "tap": outcome.tap_ifname.as_str(),
            "fdIndex": 0,
        }))?,
        fds: vec![fd],
    })
}

/// The set-bridge-port-flags kernel: apply the trusted per-role bridge
/// port flag set under the installed-generation fence, exactly what the
/// retired `SetBridgePortFlags` arm's live backend ran.
async fn set_bridge_port_flags(
    config: &KernelConfig,
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let req: d2b_contracts_broker::broker_wire::SetBridgePortFlagsRequest =
        typed_payload(invocation, "set-bridge-port-flags")?;
    let resolver = kernel_resolver(config, "set-bridge-port-flags")?;
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    let response = crate::runtime::dispatch_set_bridge_port_flags_inner(&req, &resolver, &exec)
        .await
        .map_err(|error| {
            errored(format!(
                "set-bridge-port-flags: {}",
                crate::runtime::broker_error_kernel_detail(error)
            ))
        })?;
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({
            "bridge": response.bridge.as_str(),
            "port": response.port.as_str(),
            "isolated": response.isolated,
            "neighSuppress": response.neigh_suppress,
        }))?,
        fds: Vec::new(),
    })
}

/// The update-hosts-file kernel: write or remove the managed /etc/hosts
/// marker block from the resolved intent, exactly what the retired
/// `UpdateHostsFile` arm's live backend ran.
async fn update_hosts_file(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let destroy = optional_field_bool(invocation.payload, "destroy")?.unwrap_or(false);
    let intent = d2b_core::bundle_resolver::ResolvedHostsIntent {
        intent_id: field_str(invocation.payload, "intentId")?.to_owned(),
        path: PathBuf::from(field_str(invocation.payload, "path")?),
        managed_block: field_str(invocation.payload, "managedBlock")?.to_owned(),
        start_marker: field_str(invocation.payload, "startMarker")?.to_owned(),
        end_marker: field_str(invocation.payload, "endMarker")?.to_owned(),
        mode: field_i64(invocation.payload, "mode")? as u32,
        provenance: optional_parse_field(invocation.payload, "provenance")?,
        ownership_marker: optional_str(invocation.payload, "ownershipMarker")?,
    };
    let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
    if destroy {
        crate::ops::hosts::remove_marker_block(&exec, &intent)
            .await
            .map_err(|error| errored(format!("update-hosts-file: {error}")))?;
    } else {
        crate::ops::hosts::write_marker_block(&exec, &intent)
            .await
            .map_err(|error| errored(format!("update-hosts-file: {error}")))?;
    }
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({}))?,
        fds: Vec::new(),
    })
}

/// The seed-dnsmasq-lease kernel: the retired `SeedDnsmasqLease` arm's
/// live core was its admission check - the per-VM dnsmasq lease row is
/// derived, never caller-supplied, so the kernel re-derives the expected
/// child VM name from the admitted Network identity and refuses a
/// mismatch. The lease file write itself remains a committed follow-up;
/// the arm's observable contract (admission + ack) is preserved.
async fn seed_dnsmasq_lease(
    invocation: &DirectInvocation<'_>,
) -> Result<DispatchOutcome, DispatchFailure> {
    let scope_id = field_str(invocation.payload, "scopeId")?;
    let zone_uid: d2b_contracts_resource::v3::ResourceUid =
        parse_field(invocation.payload, "zoneUid")?;
    let network_uid: d2b_contracts_resource::v3::ResourceUid =
        parse_field(invocation.payload, "networkUid")?;
    let network_generation: d2b_contracts_resource::v3::ResourceGeneration =
        parse_field(invocation.payload, "networkGeneration")?;
    let attachment_generation: d2b_contracts_resource::v3::ResourceGeneration =
        parse_field(invocation.payload, "attachmentGeneration")?;
    let bundle_generation: d2b_contracts_resource::v3::ResourceBundleGenerationId =
        parse_field(invocation.payload, "bundleGeneration")?;
    let vm_id = field_str(invocation.payload, "vmId")?;
    if scope_id.starts_with("network:") {
        let expected_scope = format!("network:{}:{}", zone_uid.as_str(), network_uid.as_str());
        if scope_id != expected_scope
            || network_generation.get() == 0
            || attachment_generation.get() == 0
            || bundle_generation.as_str().is_empty()
        {
            return Err(refused("seed-dnsmasq-lease: network-admission-mismatch"));
        }
    }
    let expected_vm = d2b_contracts_resource::v3::derive_network_child_name(&network_uid, "vm");
    if vm_id != expected_vm.as_str() {
        return Err(refused("seed-dnsmasq-lease: network-admission-mismatch"));
    }
    Ok(DispatchOutcome {
        result: canonical(serde_json::json!({ "seeded": true }))?,
        fds: Vec::new(),
    })
}

/// Deserialize the whole payload as one typed wire request. The payload
/// schema of the committed kernel row is the typed request's camelCase
/// shape, so the typed parse is the kernel's payload validation.
fn typed_payload<T: serde::de::DeserializeOwned>(
    invocation: &DirectInvocation<'_>,
    operation: &str,
) -> Result<T, DispatchFailure> {
    let json = serde_json::to_value(invocation.payload)
        .map_err(|error| errored(format!("{operation}: payload value: {error}")))?;
    serde_json::from_value(json).map_err(|error| refused(format!("{operation}: {error}")))
}

/// The optional string field of one payload.
fn optional_str(
    payload: &CanonicalJsonObject,
    key: &str,
) -> Result<Option<String>, DispatchFailure> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    match value {
        CanonicalJsonValue::String(value) => Ok(Some(value.clone())),
        CanonicalJsonValue::Null => Ok(None),
        _ => Err(refused(format!("{key}: expected a string"))),
    }
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

fn field_str<'a>(payload: &'a CanonicalJsonObject, key: &str) -> Result<&'a str, DispatchFailure> {
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

fn field_str_array(
    payload: &CanonicalJsonObject,
    key: &str,
) -> Result<Vec<String>, DispatchFailure> {
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
    if matches!(value, CanonicalJsonValue::Null) {
        return Ok(None);
    }
    let json = value_to_serde(value)?;
    serde_json::from_value(json)
        .map(Some)
        .map_err(|error| refused(format!("{key}: {error}")))
}

/// The optional seccomp policy reference: a plain string or a `{"ref":
/// string}` object, absent meaning no policy.
fn optional_seccomp_ref(payload: &CanonicalJsonObject) -> Result<Option<String>, DispatchFailure> {
    let Some(value) = payload.get("seccompPolicyRef") else {
        return Ok(None);
    };
    match value {
        CanonicalJsonValue::String(value) => Ok(Some(value.clone())),
        CanonicalJsonValue::Object(fields) => match fields.get("ref") {
            Some(CanonicalJsonValue::String(value)) => Ok(Some(value.clone())),
            _ => Err(refused("seccompPolicyRef: expected {ref: string}")),
        },
        _ => Err(refused(
            "seccompPolicyRef: expected a string or {ref: string}",
        )),
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
    // The daemon emits absent optionals as JSON null (notably from the
    // typed request's Option fields); null means None here too, so an
    // absent namespace never refuses a spawn.
    if matches!(value, CanonicalJsonValue::Null) {
        return Ok(None);
    }
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
    // The daemon's family handler always carries the derived field, so an
    // absent resolver derivation serializes as JSON null - treat it as
    // absent like the kernel's other optional parsers.
    if matches!(value, CanonicalJsonValue::Null) {
        return Ok(None);
    }
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
        _ => {
            return Err(refused(
                "swtpmIdentity.stateVolume: expected a string or null",
            ));
        }
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
                    d2b_contracts::identity::ResourceRef::parse(reference).map_err(|error| {
                        refused(format!("deviceWorker.scope.deviceRef: {error}"))
                    })?
                }
                _ => return Err(refused("deviceWorker.scope.deviceRef: expected a string")),
            };
            let device_uid = match scope.get("deviceUid") {
                Some(CanonicalJsonValue::String(uid)) => {
                    d2b_contracts::identity::ResourceUid::parse(uid.clone()).map_err(|error| {
                        refused(format!("deviceWorker.scope.deviceUid: {error}"))
                    })?
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
        _ => {
            return Err(refused(
                "deviceWorker.bindsRuntimeSocket: expected a boolean",
            ));
        }
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
fn parse_role(
    payload: &CanonicalJsonObject,
) -> Result<d2b_contracts_broker::broker_wire::RunnerRole, DispatchFailure> {
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
        _ => Err(refused(format!(
            "{key}: expected a non-negative integer or null"
        ))),
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
        return Err(refused(format!(
            "{key}: expected an array of 32 integers or null"
        )));
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
            d2b_contracts::identity::ResourceRef::parse(value.as_str())
                .map_err(|error| refused(format!("{key}: {error}")))
        })
        .transpose()
}

fn optional_resource_uid(
    fields: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<d2b_contracts_resource::v3::ResourceUid>, DispatchFailure> {
    optional_string_field(fields, key)?
        .map(|value| {
            d2b_contracts::identity::ResourceUid::parse(value)
                .map_err(|error| refused(format!("{key}: {error}")))
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
async fn pidfd_pid(pidfd: std::os::fd::BorrowedFd<'_>) -> Option<i32> {
    let info =
        tokio::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd()))
            .await
            .ok()?;
    info.lines().find_map(|line| {
        line.strip_prefix("Pid:")
            .and_then(|value| value.trim().parse::<i32>().ok())
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
fn drain_notification(
    invocation_id: &str,
) -> Option<d2b_contracts_broker::broker_wire::ChildReapedNotification> {
    // Non-blocking try-lock (plan U8): a Busy collision (the reap task's
    // push) returns None and the notification stays buffered for the next
    // poll - the reap outcome is never lost, only deferred.
    let mut buffer = crate::runtime::child_reap_buffer().try_lock().ok()?;
    let index = buffer
        .iter()
        .position(|notification| notification.runner_id == invocation_id)?;
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
        let body = serde_json::to_string(&std::collections::BTreeMap::from_iter(
            entries.iter().map(|(key, value)| {
                (
                    (*key).to_owned(),
                    serde_json::to_value(value).expect("value"),
                )
            }),
        ))
        .expect("object serializes");
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
            (
                "argv",
                CanonicalJsonValue::Array(vec![
                    string("/nix/store/x/bin/runner"),
                    string("--flag"),
                ]),
            ),
            ("uid", integer(1000)),
            ("gid", integer(1000)),
            (
                "supplementaryGroups",
                CanonicalJsonValue::Array(vec![integer(100), integer(200)]),
            ),
            ("env", CanonicalJsonValue::Array(vec![string("A=B")])),
            (
                "capabilities",
                CanonicalJsonValue::Array(vec![string("CAP_NET_ADMIN")]),
            ),
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
                        (
                            "controllers",
                            CanonicalJsonValue::Array(vec![string("cpu")]),
                        ),
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
        assert_eq!(
            plan.cgroup_placement.subtree,
            "d2b.slice/zone-a/guest-1/role"
        );
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
                        (
                            "resourceUid",
                            string("00000000-0000-4000-8000-000000000001"),
                        ),
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
            identity
                .resource_ref
                .as_ref()
                .map(|reference| reference.to_canonical_string()),
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
            identity
                .owner_ref
                .as_ref()
                .map(|reference| reference.to_canonical_string()),
            Some("Volume/vol-abcd".to_owned())
        );
        assert_eq!(
            identity
                .provider_ref
                .as_ref()
                .map(|reference| reference.to_canonical_string()),
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
                        (
                            "bundleRunnerIntentRef",
                            string("runner:sys-work-usbipd:backend"),
                        ),
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
        assert!(!strictly_inside_delegated_slice(Path::new(
            "/sys/fs/cgroup/other.slice"
        )));
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
