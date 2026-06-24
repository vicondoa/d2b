//! Daemon-internal USBIP reconciliation state model.
//!
//! This module does not add broker or public wire operations. It names the
//! state the daemon must compare as USB reconciliation grows from the existing
//! per-busid step machine into a restart-safe reconciler:
//!
//! * declared bundle intent and policy evaluation,
//! * the existing broker-mediated USB device claim,
//! * active host carrier / bind / proxy state,
//! * guest import state,
//! * physical topology identity, and
//! * public redaction boundaries for future status DTOs.
//!
//! Internal state may carry raw sysfs paths, bus numbers, port chains, and
//! serial-like observations so the daemon can avoid serial-only matching.
//! Public projections deliberately collapse those fields into coarse anchors;
//! privileged audit can later opt into a different DTO without weakening the
//! default status surface.

use std::{
    collections::HashMap,
    fs, io,
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// Maximum accepted lifecycle/correlation-id length for one reconcile attempt.
pub const USBIP_RECONCILE_CORRELATION_ID_MAX_LEN: usize = 48;

fn looks_like_trace_id(value: &str) -> bool {
    matches!(value.len(), 16 | 32) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn usbip_vm_source_shape_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .enumerate()
            .all(|(idx, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (idx > 0 && b == b'-'))
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && !looks_like_trace_id(value)
}

fn project_usbip_vm_label(value: &str) -> &str {
    if usbip_vm_source_shape_is_valid(value) {
        value
    } else {
        "other"
    }
}

fn deserialize_optional_usbip_event_source_vm<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if let Some(vm) = value.as_deref()
        && !usbip_vm_source_shape_is_valid(vm)
    {
        return Err(D::Error::custom("invalid USB event source VM shape"));
    }
    Ok(value)
}

/// Bundle-level desired claim for one physical USB device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipDesiredClaimState {
    /// No current bundle intent owns the device.
    Undeclared,
    /// The bundle declares this device for the target VM/env.
    Desired,
    /// A previous claim is being released; host and guest state should drain.
    Releasing,
}

/// Persisted state of the existing broker-mediated USB device claim.
///
/// The current backing artifact is the broker's per-busid USBIP lock file. The
/// daemon may observe that artifact for reconciliation/status, but only the
/// broker mutates it; this is not a second per-VM DAG lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipPersistedLockClaimState {
    /// No persisted broker claim exists for the device.
    Missing,
    /// The claim is held by the declared VM/env.
    HeldByDesiredOwner,
    /// The claim is held by a different env or VM.
    HeldByOtherOwner,
    /// The claim refers to an owner that is no longer active.
    StaleOwner,
    /// The persisted claim could not be parsed or validated.
    Corrupt,
}

/// Host-level carrier readiness for usbip-host and the per-env backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipActiveCarrierState {
    /// No usable carrier has been observed.
    Absent,
    /// The kernel module or backend prerequisite is not available.
    Unavailable,
    /// Non-owner runners are withheld while the owner reconciles.
    WithheldForOwner,
    /// The host carrier is ready for bind/proxy work.
    Ready,
    /// A previously-present device vanished while carrier state was probed.
    DepartedDuringProbe,
}

/// Host kernel driver binding state for the physical device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipHostBindState {
    /// Device is not bound to usbip-host.
    Unbound,
    /// Broker is currently binding the device.
    Binding,
    /// Device is bound to usbip-host for export.
    BoundToUsbipHost,
    /// Device is bound to some other kernel driver.
    BoundToUnexpectedDriver,
    /// Broker is currently unbinding the device.
    Unbinding,
    /// The device disappeared between bind preflight and bind completion.
    DepartedDuringBind,
}

/// Per-env proxy listener state for the exported device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProxyState {
    /// No proxy is declared for this desired claim.
    NotDeclared,
    /// Proxy is declared but not running/listening.
    Stopped,
    /// Proxy runner was spawned but has not reached readiness.
    Starting,
    /// Proxy is listening for the owning guest.
    Listening,
    /// Proxy belongs to an old owner/generation.
    Stale,
    /// Proxy failed readiness or exited unexpectedly.
    Failed,
}

/// Targeted cleanup support available to the current USBIP proxy implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipTargetedProxyCleanup {
    /// Current implementation: a generic L4 TCP forwarder with no USBIP/busid
    /// protocol parser and no proven host tuple for one selected busid stream.
    Unavailable,
    /// Future implementation: the reconciler has proven an exact conntrack/socket
    /// tuple for this busid, or the proxy itself is busid-aware.
    ConntrackOrSocketTuple,
}

/// Explicit operator-selected policy for actions that may bounce unrelated
/// same-env USBIP streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "policy")]
pub enum UsbipProxyDrainPolicy {
    /// Wait for currently accepted proxy connections to drain before forcing a
    /// recycle. The live implementation must enforce this with a bounded timer.
    BoundedDrain { grace_ms: u64 },
    /// Force a recycle immediately. This is never used for single-busid detach
    /// or normal VM restart.
    ForceNow,
}

/// Why the reconciler is synchronizing the per-env USBIP proxy/export path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum UsbipProxySynchronizationIntent {
    /// Normal attach or single-VM restart: refresh backend/export readiness only.
    RefreshExport,
    /// Release one selected busid from the current env.
    ReleaseBusid {
        targeted_cleanup: UsbipTargetedProxyCleanup,
    },
    /// Explicit env-level proxy recycle requested by an operator or follow-up
    /// reconciler. This is the only intent that may bounce same-env streams.
    ForceRecycleWithDrain { drain: UsbipProxyDrainPolicy },
}

/// Ordered action names for the daemon's USBIP proxy synchronization strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProxySynchronizationAction {
    OptimisticBackendExportRefresh,
    EnsureProxyListening,
    HostUnbind,
    WithdrawFirewallCarveout,
    TargetedConntrackOrSocketKill,
    TargetedConntrackDelete,
    TargetedTcpEstablishedSocketKill,
    SkipTcpSocketKillForUdp,
    RefuseSharedSocketKill,
    FailClosedRevocationNotIsolated,
    PreserveBusidLockForManualDrain,
    PreserveSameEnvStreams,
    BoundedDrainOrForce,
    AcquireExclusiveSocketLifecycleLock,
    RebindProxyListenerFdRelative,
}

/// Race-free plan for the current per-env generic L4 USBIP proxy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipProxySynchronizationPlan {
    pub intent: UsbipProxySynchronizationIntent,
    pub actions: Vec<UsbipProxySynchronizationAction>,
    pub may_bounce_same_env_streams: bool,
    pub claims_selective_busid_proxy_closure: bool,
}

/// Build the conservative synchronization plan for the current per-env generic
/// L4 proxy. The normal paths never stop/rebind the proxy: they optimistically
/// refresh backend/export readiness, withdraw/block per-busid ingress before any
/// host unbind, and fail closed if selective stream cleanup cannot be proven.
pub fn plan_usbip_proxy_synchronization(
    intent: UsbipProxySynchronizationIntent,
) -> UsbipProxySynchronizationPlan {
    use UsbipProxySynchronizationAction as Action;
    match &intent {
        UsbipProxySynchronizationIntent::RefreshExport => UsbipProxySynchronizationPlan {
            intent,
            actions: vec![
                Action::OptimisticBackendExportRefresh,
                Action::EnsureProxyListening,
                Action::PreserveSameEnvStreams,
            ],
            may_bounce_same_env_streams: false,
            claims_selective_busid_proxy_closure: false,
        },
        UsbipProxySynchronizationIntent::ReleaseBusid {
            targeted_cleanup: UsbipTargetedProxyCleanup::Unavailable,
        } => UsbipProxySynchronizationPlan {
            intent,
            actions: vec![
                Action::WithdrawFirewallCarveout,
                Action::FailClosedRevocationNotIsolated,
                Action::PreserveBusidLockForManualDrain,
                Action::PreserveSameEnvStreams,
            ],
            may_bounce_same_env_streams: false,
            claims_selective_busid_proxy_closure: false,
        },
        UsbipProxySynchronizationIntent::ReleaseBusid {
            targeted_cleanup: UsbipTargetedProxyCleanup::ConntrackOrSocketTuple,
        } => UsbipProxySynchronizationPlan {
            intent,
            actions: vec![
                Action::WithdrawFirewallCarveout,
                Action::TargetedConntrackOrSocketKill,
                Action::HostUnbind,
                Action::PreserveSameEnvStreams,
            ],
            may_bounce_same_env_streams: false,
            claims_selective_busid_proxy_closure: true,
        },
        UsbipProxySynchronizationIntent::ForceRecycleWithDrain { .. } => {
            UsbipProxySynchronizationPlan {
                intent,
                actions: vec![
                    Action::BoundedDrainOrForce,
                    Action::AcquireExclusiveSocketLifecycleLock,
                    Action::RebindProxyListenerFdRelative,
                ],
                may_bounce_same_env_streams: true,
                claims_selective_busid_proxy_closure: false,
            }
        }
    }
}

/// L4 protocol carried by a host-observed proxy flow tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProxyFlowProtocol {
    Tcp,
    Udp,
}

/// Whether the host-observed flow source proves the workload VM that owns the
/// USBIP stream. Env-level net-VM SNAT is not sufficient for single-busid
/// revocation because it can hide multiple same-env workload VMs behind one
/// source tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipProxyFlowSourceIdentity {
    ProvenVmSource,
    ObscuredBySnat,
    AntiSpoofNotProven,
}

impl UsbipProxyFlowSourceIdentity {
    const fn failure_reason(self) -> Option<UsbipRevocationFlowFailure> {
        match self {
            Self::ProvenVmSource => None,
            Self::ObscuredBySnat => Some(UsbipRevocationFlowFailure::SourceIdentityObscuredBySnat),
            Self::AntiSpoofNotProven => Some(UsbipRevocationFlowFailure::AntiSpoofNotProven),
        }
    }
}

/// Exact VM→proxy tuple safe to pass to conntrack/socket cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipProxyFlowTuple {
    pub protocol: UsbipProxyFlowProtocol,
    pub vm_addr: IpAddr,
    pub vm_port: u16,
    pub proxy_addr: IpAddr,
    pub proxy_port: u16,
}

/// What the reconciler can prove about active proxy traffic for one release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum UsbipProxyFlowObservation {
    /// No established session was observed for the selected VM/proxy tuple.
    NoEstablishedSession,
    /// An exact established tuple was proven. `conntrack_delete` means the
    /// tuple can be deleted without touching sibling env streams;
    /// `tcp_socket_kill` means the host can kill exactly this established TCP
    /// socket and not the shared listener.
    ExactEstablished {
        tuple: UsbipProxyFlowTuple,
        source_identity: UsbipProxyFlowSourceIdentity,
        conntrack_delete: bool,
        tcp_socket_kill: bool,
    },
    /// Only a shared listener was identified. Killing it would bounce unrelated
    /// same-env streams, so single-busid revocation must refuse.
    SharedListeningSocket { protocol: UsbipProxyFlowProtocol },
    /// Flow ownership could not be narrowed to one VM/proxy tuple.
    SharedOrAmbiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipTargetedTerminationMechanism {
    ConntrackDelete,
    TcpSocketKill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipRevocationFlowFailure {
    MissingExactTuple,
    CleanupUnsupported,
    SharedListeningSocket,
    AmbiguousSameEnvStreams,
    SourceIdentityObscuredBySnat,
    AntiSpoofNotProven,
}

impl UsbipRevocationFlowFailure {
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::MissingExactTuple => "missing-exact-tuple",
            Self::CleanupUnsupported => "cleanup-unsupported",
            Self::SharedListeningSocket => "shared-listening-socket",
            Self::AmbiguousSameEnvStreams => "ambiguous-same-env-streams",
            Self::SourceIdentityObscuredBySnat => "source-identity-obscured-by-snat",
            Self::AntiSpoofNotProven => "anti-spoof-not-proven",
        }
    }

    pub const fn summary(self) -> &'static str {
        match self {
            Self::MissingExactTuple => {
                "no exact VM-to-proxy flow tuple was proven for the selected USB stream"
            }
            Self::CleanupUnsupported => {
                "the selected USB stream could not be terminated by conntrack deletion or a targeted TCP socket kill"
            }
            Self::SharedListeningSocket => {
                "only the shared per-env USBIP proxy listener was identified"
            }
            Self::AmbiguousSameEnvStreams => {
                "active USBIP traffic could not be narrowed to one VM/proxy tuple"
            }
            Self::SourceIdentityObscuredBySnat => {
                "the host-observed USBIP source is obscured by SNAT and does not prove one workload VM"
            }
            Self::AntiSpoofNotProven => {
                "the host could not prove anti-spoofing for the selected USBIP source tuple"
            }
        }
    }

    pub const fn remediation(self) -> &'static str {
        match self {
            Self::MissingExactTuple | Self::AmbiguousSameEnvStreams => {
                "run `nixling usb probe`; if the stream is still ambiguous, stop the VM so it drains"
            }
            Self::CleanupUnsupported => {
                "enable host conntrack/TCP cleanup support or stop the VM so the stream drains"
            }
            Self::SharedListeningSocket => {
                "stop the VM, or use an explicit env-level drain/recycle if bouncing same-env USB streams is acceptable"
            }
            Self::SourceIdentityObscuredBySnat | Self::AntiSpoofNotProven => {
                "fix the per-VM source proof or stop the VM so the stream drains"
            }
        }
    }
}

/// Detailed flow-cleanup plan for one USBIP revocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipRevocationFlowTerminationPlan {
    pub observation: UsbipProxyFlowObservation,
    pub actions: Vec<UsbipProxySynchronizationAction>,
    pub mechanisms: Vec<UsbipTargetedTerminationMechanism>,
    pub fail_closed_reason: Option<UsbipRevocationFlowFailure>,
    pub may_bounce_same_env_streams: bool,
    pub preserves_busid_lock_for_manual_drain: bool,
}

impl UsbipRevocationFlowTerminationPlan {
    pub fn is_actionable_failure(&self) -> bool {
        self.fail_closed_reason.is_some() && self.preserves_busid_lock_for_manual_drain
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipRevocationFlowExecutionReport {
    pub completed: Vec<UsbipProxySynchronizationAction>,
    pub failed: Option<(UsbipProxySynchronizationAction, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbipRevocationFlowExecutionError {
    ActionFailed {
        action: UsbipProxySynchronizationAction,
        reason: String,
    },
    NotIsolated {
        reason: UsbipRevocationFlowFailure,
    },
    PlanInvariantViolation {
        reason: String,
    },
}

pub trait UsbipRevocationFlowExecutor {
    fn withdraw_firewall_carveout(
        &mut self,
        observation: &UsbipProxyFlowObservation,
    ) -> Result<(), String>;

    fn delete_conntrack_tuple(&mut self, tuple: &UsbipProxyFlowTuple) -> Result<(), String>;

    fn kill_tcp_established_socket(&mut self, tuple: &UsbipProxyFlowTuple) -> Result<(), String>;
}

fn exact_flow_tuple(
    observation: &UsbipProxyFlowObservation,
) -> Result<&UsbipProxyFlowTuple, UsbipRevocationFlowExecutionError> {
    match observation {
        UsbipProxyFlowObservation::ExactEstablished {
            tuple,
            source_identity,
            ..
        } => match source_identity.failure_reason() {
            None => Ok(tuple),
            Some(reason) => Err(UsbipRevocationFlowExecutionError::NotIsolated { reason }),
        },
        _ => Err(UsbipRevocationFlowExecutionError::PlanInvariantViolation {
            reason: "targeted cleanup action requires an exact established VM/proxy tuple"
                .to_owned(),
        }),
    }
}

fn mark_action_failed(
    report: &mut UsbipRevocationFlowExecutionReport,
    action: UsbipProxySynchronizationAction,
    reason: impl Into<String>,
) -> UsbipRevocationFlowExecutionError {
    let reason = reason.into();
    report.failed = Some((action, reason.clone()));
    UsbipRevocationFlowExecutionError::ActionFailed { action, reason }
}

/// Execute a revocation-flow plan without ever killing a shared per-env proxy.
///
/// Enforcement is deliberately stricter than the pure planner:
///
/// * the first side effect MUST withdraw/block the firewall carve-out;
/// * targeted conntrack/socket cleanup is allowed only after that side effect
///   has completed;
/// * fail-closed plans return `NotIsolated` after the firewall withdrawal and
///   before any flow-kill action runs, leaving the host-session busid claim for
///   manual recovery.
pub fn execute_usbip_revocation_flow_termination<E: UsbipRevocationFlowExecutor>(
    plan: &UsbipRevocationFlowTerminationPlan,
    executor: &mut E,
) -> Result<
    UsbipRevocationFlowExecutionReport,
    (
        UsbipRevocationFlowExecutionReport,
        UsbipRevocationFlowExecutionError,
    ),
> {
    let mut report = UsbipRevocationFlowExecutionReport {
        completed: Vec::with_capacity(plan.actions.len()),
        failed: None,
    };

    if plan.actions.first() != Some(&UsbipProxySynchronizationAction::WithdrawFirewallCarveout) {
        let reason =
            "revocation flow must withdraw the firewall carve-out before flow cleanup".to_owned();
        report.failed = Some((
            UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
            reason.clone(),
        ));
        return Err((
            report,
            UsbipRevocationFlowExecutionError::PlanInvariantViolation { reason },
        ));
    }

    for action in &plan.actions {
        use UsbipProxySynchronizationAction as Action;
        let result = match action {
            Action::WithdrawFirewallCarveout => {
                executor.withdraw_firewall_carveout(&plan.observation)
            }
            Action::TargetedConntrackDelete => match exact_flow_tuple(&plan.observation) {
                Ok(tuple) => executor.delete_conntrack_tuple(tuple),
                Err(err) => return Err((report, err)),
            },
            Action::TargetedTcpEstablishedSocketKill => match exact_flow_tuple(&plan.observation) {
                Ok(tuple) if tuple.protocol == UsbipProxyFlowProtocol::Tcp => {
                    executor.kill_tcp_established_socket(tuple)
                }
                Ok(_) => Err("TCP socket kill requested for a non-TCP USBIP flow".to_owned()),
                Err(err) => return Err((report, err)),
            },
            Action::FailClosedRevocationNotIsolated => {
                report.failed = Some((
                    *action,
                    plan.fail_closed_reason
                        .unwrap_or(UsbipRevocationFlowFailure::MissingExactTuple)
                        .summary()
                        .to_owned(),
                ));
                return Err((
                    report,
                    UsbipRevocationFlowExecutionError::NotIsolated {
                        reason: plan
                            .fail_closed_reason
                            .unwrap_or(UsbipRevocationFlowFailure::MissingExactTuple),
                    },
                ));
            }
            Action::PreserveSameEnvStreams
            | Action::PreserveBusidLockForManualDrain
            | Action::RefuseSharedSocketKill
            | Action::SkipTcpSocketKillForUdp => Ok(()),
            Action::OptimisticBackendExportRefresh
            | Action::EnsureProxyListening
            | Action::HostUnbind
            | Action::TargetedConntrackOrSocketKill
            | Action::BoundedDrainOrForce
            | Action::AcquireExclusiveSocketLifecycleLock
            | Action::RebindProxyListenerFdRelative => Err(format!(
                "action '{action:?}' is not valid in an immediate revocation flow"
            )),
        };

        match result {
            Ok(()) => report.completed.push(*action),
            Err(reason) => {
                let err = mark_action_failed(&mut report, *action, reason);
                return Err((report, err));
            }
        }
    }

    Ok(report)
}

/// Plan targeted established-flow cleanup after a firewall carve-out has been
/// withdrawn. The first action is always `WithdrawFirewallCarveout`, so a
/// successful conntrack/socket kill cannot immediately reconnect through a still
/// open TCP/3240 carve-out.
pub fn plan_usbip_revocation_flow_termination(
    observation: UsbipProxyFlowObservation,
) -> UsbipRevocationFlowTerminationPlan {
    use UsbipProxySynchronizationAction as Action;
    let mut actions = vec![Action::WithdrawFirewallCarveout];
    let mut mechanisms = Vec::new();
    let mut fail_closed_reason = None;
    let mut preserves_busid_lock_for_manual_drain = false;

    match &observation {
        UsbipProxyFlowObservation::NoEstablishedSession => {
            actions.push(Action::PreserveSameEnvStreams);
        }
        UsbipProxyFlowObservation::ExactEstablished {
            tuple,
            source_identity,
            conntrack_delete,
            tcp_socket_kill,
        } => {
            if let Some(reason) = source_identity.failure_reason() {
                actions.push(Action::FailClosedRevocationNotIsolated);
                actions.push(Action::PreserveBusidLockForManualDrain);
                actions.push(Action::PreserveSameEnvStreams);
                fail_closed_reason = Some(reason);
                preserves_busid_lock_for_manual_drain = true;
            } else {
                if *conntrack_delete {
                    actions.push(Action::TargetedConntrackDelete);
                    mechanisms.push(UsbipTargetedTerminationMechanism::ConntrackDelete);
                }
                match tuple.protocol {
                    UsbipProxyFlowProtocol::Tcp if *tcp_socket_kill => {
                        actions.push(Action::TargetedTcpEstablishedSocketKill);
                        mechanisms.push(UsbipTargetedTerminationMechanism::TcpSocketKill);
                    }
                    UsbipProxyFlowProtocol::Udp => actions.push(Action::SkipTcpSocketKillForUdp),
                    UsbipProxyFlowProtocol::Tcp => {}
                }
                if mechanisms.is_empty() {
                    actions.push(Action::FailClosedRevocationNotIsolated);
                    actions.push(Action::PreserveBusidLockForManualDrain);
                    actions.push(Action::PreserveSameEnvStreams);
                    fail_closed_reason = Some(UsbipRevocationFlowFailure::CleanupUnsupported);
                    preserves_busid_lock_for_manual_drain = true;
                } else {
                    actions.push(Action::PreserveSameEnvStreams);
                }
            }
        }
        UsbipProxyFlowObservation::SharedListeningSocket { .. } => {
            actions.push(Action::RefuseSharedSocketKill);
            actions.push(Action::FailClosedRevocationNotIsolated);
            actions.push(Action::PreserveBusidLockForManualDrain);
            actions.push(Action::PreserveSameEnvStreams);
            fail_closed_reason = Some(UsbipRevocationFlowFailure::SharedListeningSocket);
            preserves_busid_lock_for_manual_drain = true;
        }
        UsbipProxyFlowObservation::SharedOrAmbiguous => {
            actions.push(Action::FailClosedRevocationNotIsolated);
            actions.push(Action::PreserveBusidLockForManualDrain);
            actions.push(Action::PreserveSameEnvStreams);
            fail_closed_reason = Some(UsbipRevocationFlowFailure::AmbiguousSameEnvStreams);
            preserves_busid_lock_for_manual_drain = true;
        }
    }

    UsbipRevocationFlowTerminationPlan {
        observation,
        actions,
        mechanisms,
        fail_closed_reason,
        may_bounce_same_env_streams: false,
        preserves_busid_lock_for_manual_drain,
    }
}

/// Lifecycle path that requested active USBIP carrier cleanup for one VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipVmCarrierCleanupMode {
    /// VM stop or restart: tear down active guest/host carrier state but keep the
    /// broker-owned host-session busid claim for the same VM.
    VmStopOrRestart,
    /// Explicit `nixling usb detach`: tear down active state and release the
    /// host-session claim only after every earlier cleanup/revocation step succeeds.
    ExplicitDetach,
}

/// Ordered cleanup action for one VM/busid active USBIP carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipVmCarrierCleanupAction {
    /// Ask guestd to remove any imported USBIP device before the VM disappears.
    DetachGuestImport,
    /// Block/withdraw the host firewall carve-out before any flow termination.
    WithdrawFirewallCarveout,
    /// Delete one proven conntrack tuple.
    TargetedConntrackDelete,
    /// Kill one proven established TCP socket, never the shared listener.
    TargetedTcpEstablishedSocketKill,
    /// Record that UDP has no TCP socket-kill step.
    SkipTcpSocketKillForUdp,
    /// Record refusal to kill a shared per-env proxy listener.
    RefuseSharedSocketKill,
    /// Fail closed because single-VM/busid stream ownership was not isolated.
    FailClosedRevocationNotIsolated,
    /// Run the broker's safe host unbind path: stream-fd release, bounded helper
    /// unbind, and postcondition check.
    HostUnbind,
    /// Revoke the backend device ACL after explicit detach unbound successfully.
    RevokeBackendAcl,
    /// Release the broker-owned host-session busid claim after explicit detach
    /// unbound and revoked successfully.
    ReleaseDurableClaim,
    /// Preserve the host-session claim for same-VM restart or manual recovery.
    PreserveDurableClaim,
    /// Preserve per-env backend/proxy sidecars and unrelated same-env streams.
    PreserveSameEnvStreams,
    /// Surface manual recovery because active carrier cleanup did not converge.
    SurfaceManualRecovery,
}

/// Reusable VM lifecycle cleanup plan for active USBIP carrier state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipVmCarrierCleanupPlan {
    pub mode: UsbipVmCarrierCleanupMode,
    pub flow_observation: UsbipProxyFlowObservation,
    pub actions: Vec<UsbipVmCarrierCleanupAction>,
    pub may_bounce_same_env_streams: bool,
    pub releases_durable_claim_on_success: bool,
    pub preserves_durable_claim_on_success: bool,
    pub manual_recovery_on_failure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_closed_reason: Option<UsbipRevocationFlowFailure>,
}

/// Build the cleanup plan used by VM stop/restart and explicit USB detach.
///
/// The plan composes the immediate revocation flow with guest detach and host
/// unbind ordering. VM stop/restart deliberately omits ACL revoke and lock
/// release so a later same-VM start can reattach from the host-session claim.
/// Explicit detach appends ACL revoke and lock release only after guest detach,
/// firewall withdrawal/targeted flow cleanup, and host unbind have all
/// succeeded.
pub fn plan_usbip_vm_carrier_cleanup(
    mode: UsbipVmCarrierCleanupMode,
    flow_observation: UsbipProxyFlowObservation,
) -> UsbipVmCarrierCleanupPlan {
    let revocation = plan_usbip_revocation_flow_termination(flow_observation.clone());
    let mut actions = vec![UsbipVmCarrierCleanupAction::DetachGuestImport];
    let mut manual_recovery_on_failure = revocation.fail_closed_reason.is_some();

    for action in &revocation.actions {
        match action {
            UsbipProxySynchronizationAction::WithdrawFirewallCarveout => {
                actions.push(UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout)
            }
            UsbipProxySynchronizationAction::TargetedConntrackDelete => {
                actions.push(UsbipVmCarrierCleanupAction::TargetedConntrackDelete)
            }
            UsbipProxySynchronizationAction::TargetedTcpEstablishedSocketKill => {
                actions.push(UsbipVmCarrierCleanupAction::TargetedTcpEstablishedSocketKill)
            }
            UsbipProxySynchronizationAction::SkipTcpSocketKillForUdp => {
                actions.push(UsbipVmCarrierCleanupAction::SkipTcpSocketKillForUdp)
            }
            UsbipProxySynchronizationAction::RefuseSharedSocketKill => {
                actions.push(UsbipVmCarrierCleanupAction::RefuseSharedSocketKill)
            }
            UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated => {
                actions.push(UsbipVmCarrierCleanupAction::FailClosedRevocationNotIsolated)
            }
            UsbipProxySynchronizationAction::PreserveBusidLockForManualDrain => {
                actions.push(UsbipVmCarrierCleanupAction::PreserveDurableClaim);
                actions.push(UsbipVmCarrierCleanupAction::SurfaceManualRecovery);
            }
            UsbipProxySynchronizationAction::PreserveSameEnvStreams => {
                actions.push(UsbipVmCarrierCleanupAction::PreserveSameEnvStreams)
            }
            UsbipProxySynchronizationAction::HostUnbind
            | UsbipProxySynchronizationAction::OptimisticBackendExportRefresh
            | UsbipProxySynchronizationAction::EnsureProxyListening
            | UsbipProxySynchronizationAction::TargetedConntrackOrSocketKill
            | UsbipProxySynchronizationAction::BoundedDrainOrForce
            | UsbipProxySynchronizationAction::AcquireExclusiveSocketLifecycleLock
            | UsbipProxySynchronizationAction::RebindProxyListenerFdRelative => {}
        }
    }

    let revocation_converges = revocation.fail_closed_reason.is_none();
    if revocation_converges {
        actions.push(UsbipVmCarrierCleanupAction::HostUnbind);
        match mode {
            UsbipVmCarrierCleanupMode::VmStopOrRestart => {
                actions.push(UsbipVmCarrierCleanupAction::PreserveDurableClaim);
            }
            UsbipVmCarrierCleanupMode::ExplicitDetach => {
                actions.push(UsbipVmCarrierCleanupAction::RevokeBackendAcl);
                actions.push(UsbipVmCarrierCleanupAction::ReleaseDurableClaim);
            }
        }
    } else {
        manual_recovery_on_failure = true;
        if !actions.contains(&UsbipVmCarrierCleanupAction::PreserveDurableClaim) {
            actions.push(UsbipVmCarrierCleanupAction::PreserveDurableClaim);
        }
        if !actions.contains(&UsbipVmCarrierCleanupAction::SurfaceManualRecovery) {
            actions.push(UsbipVmCarrierCleanupAction::SurfaceManualRecovery);
        }
    }

    UsbipVmCarrierCleanupPlan {
        mode,
        flow_observation,
        actions,
        may_bounce_same_env_streams: false,
        releases_durable_claim_on_success: revocation_converges
            && matches!(mode, UsbipVmCarrierCleanupMode::ExplicitDetach),
        preserves_durable_claim_on_success: matches!(
            mode,
            UsbipVmCarrierCleanupMode::VmStopOrRestart
        ) || !revocation_converges,
        manual_recovery_on_failure,
        fail_closed_reason: revocation.fail_closed_reason,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipVmCarrierCleanupExecutionReport {
    pub completed: Vec<UsbipVmCarrierCleanupAction>,
    pub failed: Option<(UsbipVmCarrierCleanupAction, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbipVmCarrierCleanupExecutionError {
    ActionFailed {
        action: UsbipVmCarrierCleanupAction,
        reason: String,
    },
    NotIsolated {
        reason: UsbipRevocationFlowFailure,
    },
    PlanInvariantViolation {
        reason: String,
    },
}

pub trait UsbipVmCarrierCleanupExecutor {
    fn detach_guest_import(&mut self, plan: &UsbipVmCarrierCleanupPlan) -> Result<(), String>;
    fn withdraw_firewall_carveout(
        &mut self,
        observation: &UsbipProxyFlowObservation,
    ) -> Result<(), String>;
    fn delete_conntrack_tuple(&mut self, tuple: &UsbipProxyFlowTuple) -> Result<(), String>;
    fn kill_tcp_established_socket(&mut self, tuple: &UsbipProxyFlowTuple) -> Result<(), String>;
    fn host_unbind(&mut self, plan: &UsbipVmCarrierCleanupPlan) -> Result<(), String>;
    fn revoke_backend_acl(&mut self, plan: &UsbipVmCarrierCleanupPlan) -> Result<(), String>;
    fn release_durable_claim(&mut self, plan: &UsbipVmCarrierCleanupPlan) -> Result<(), String>;
}

fn mark_cleanup_action_failed(
    report: &mut UsbipVmCarrierCleanupExecutionReport,
    action: UsbipVmCarrierCleanupAction,
    reason: impl Into<String>,
) -> UsbipVmCarrierCleanupExecutionError {
    let reason = reason.into();
    report.failed.get_or_insert((action, reason.clone()));
    UsbipVmCarrierCleanupExecutionError::ActionFailed { action, reason }
}

fn detach_guest_import_failure_allows_host_cleanup(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("guest-control transport unavailable")
        || reason.contains("could not resolve guest-control transport")
        || reason.contains("cannot reach guest-control")
        || reason.contains("guest-control unreachable")
        || reason.contains("vm unreachable")
        || reason.contains("dead vm")
        || reason.contains("vm dead")
        || reason.contains("vm is dead")
        || reason.contains("vm is stopped")
        || reason.contains("vm is not running")
}

/// Execute a VM USBIP carrier cleanup plan without releasing the host-session claim
/// unless the plan is an explicit detach and every earlier cleanup step
/// succeeded. A guest-detach failure caused by a dead or unreachable VM remains
/// visible in the report but does not block host-side firewall/unbind cleanup.
pub fn execute_usbip_vm_carrier_cleanup<E: UsbipVmCarrierCleanupExecutor>(
    plan: &UsbipVmCarrierCleanupPlan,
    executor: &mut E,
) -> Result<
    UsbipVmCarrierCleanupExecutionReport,
    (
        UsbipVmCarrierCleanupExecutionReport,
        UsbipVmCarrierCleanupExecutionError,
    ),
> {
    let mut report = UsbipVmCarrierCleanupExecutionReport {
        completed: Vec::with_capacity(plan.actions.len()),
        failed: None,
    };
    let mut host_unbound = false;
    let mut acl_revoked = false;
    let mut deferred_error = None;

    for action in &plan.actions {
        use UsbipVmCarrierCleanupAction as Action;
        let result = match action {
            Action::DetachGuestImport => executor.detach_guest_import(plan),
            Action::WithdrawFirewallCarveout => {
                executor.withdraw_firewall_carveout(&plan.flow_observation)
            }
            Action::TargetedConntrackDelete => match exact_flow_tuple(&plan.flow_observation) {
                Ok(tuple) => executor.delete_conntrack_tuple(tuple),
                Err(UsbipRevocationFlowExecutionError::NotIsolated { reason }) => {
                    return Err((
                        report,
                        UsbipVmCarrierCleanupExecutionError::NotIsolated { reason },
                    ));
                }
                Err(err) => {
                    return Err((
                        report,
                        UsbipVmCarrierCleanupExecutionError::PlanInvariantViolation {
                            reason: format!("{err:?}"),
                        },
                    ));
                }
            },
            Action::TargetedTcpEstablishedSocketKill => {
                match exact_flow_tuple(&plan.flow_observation) {
                    Ok(tuple) if tuple.protocol == UsbipProxyFlowProtocol::Tcp => {
                        executor.kill_tcp_established_socket(tuple)
                    }
                    Ok(_) => Err("TCP socket kill requested for a non-TCP USBIP flow".to_owned()),
                    Err(UsbipRevocationFlowExecutionError::NotIsolated { reason }) => {
                        return Err((
                            report,
                            UsbipVmCarrierCleanupExecutionError::NotIsolated { reason },
                        ));
                    }
                    Err(err) => {
                        return Err((
                            report,
                            UsbipVmCarrierCleanupExecutionError::PlanInvariantViolation {
                                reason: format!("{err:?}"),
                            },
                        ));
                    }
                }
            }
            Action::FailClosedRevocationNotIsolated => {
                let reason = plan
                    .fail_closed_reason
                    .unwrap_or(UsbipRevocationFlowFailure::MissingExactTuple);
                report.failed = Some((*action, reason.summary().to_owned()));
                return Err((
                    report,
                    UsbipVmCarrierCleanupExecutionError::NotIsolated { reason },
                ));
            }
            Action::HostUnbind => {
                let result = executor.host_unbind(plan);
                if result.is_ok() {
                    host_unbound = true;
                }
                result
            }
            Action::RevokeBackendAcl => {
                if !host_unbound {
                    Err("backend ACL revoke requires successful host unbind first".to_owned())
                } else {
                    let result = executor.revoke_backend_acl(plan);
                    if result.is_ok() {
                        acl_revoked = true;
                    }
                    result
                }
            }
            Action::ReleaseDurableClaim => {
                if !host_unbound || !acl_revoked {
                    Err(
                        "session claim release requires successful host unbind and ACL revoke first"
                            .to_owned(),
                    )
                } else {
                    executor.release_durable_claim(plan)
                }
            }
            Action::PreserveDurableClaim
            | Action::PreserveSameEnvStreams
            | Action::SurfaceManualRecovery
            | Action::RefuseSharedSocketKill
            | Action::SkipTcpSocketKillForUdp => Ok(()),
        };

        match result {
            Ok(()) => report.completed.push(*action),
            Err(reason) => {
                if matches!(action, Action::DetachGuestImport)
                    && detach_guest_import_failure_allows_host_cleanup(&reason)
                {
                    deferred_error = Some(mark_cleanup_action_failed(&mut report, *action, reason));
                    continue;
                }
                let err = mark_cleanup_action_failed(&mut report, *action, reason);
                return Err((report, err));
            }
        }
    }

    if let Some(err) = deferred_error {
        Err((report, err))
    } else {
        Ok(report)
    }
}

/// Guest-side import state for the target VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipGuestImportState {
    /// No guest import has been requested.
    NotRequested,
    /// Guest import RPC is in flight.
    Importing,
    /// Guest reports the device imported.
    Imported,
    /// Guest detach RPC is in flight.
    Detaching,
    /// Guest reports the device detached.
    Detached,
    /// Guest import/detach failed.
    Failed,
    /// Guest state belongs to an old owner/generation.
    Stale,
    /// Guest returned a state this daemon does not understand.
    Unknown,
}

/// Policy failure detected before attempting host mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipPolicyFailure {
    /// USBIP support is disabled for this VM or environment.
    FeatureDisabled,
    /// Bundle has no USBIP bind/firewall intent for this busid.
    MissingBundleIntent,
    /// Device is not declared for the requested VM.
    DeviceNotDeclaredForVm,
    /// Device is not declared for the requested environment.
    DeviceNotDeclaredForEnv,
    /// Observed topology does not match the declared physical identity.
    TopologyMismatch,
    /// More than one physical device matches the declaration.
    AmbiguousPhysicalMatch,
    /// Caller is not authorized to mutate this claim.
    AuthorizationDenied,
}

/// Why a reconciliation row is degraded instead of converged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipDegradedReason {
    /// One or more policy checks failed.
    PolicyFailed(UsbipPolicyFailure),
    /// A desired device was not present at probe time.
    DeviceDepartedBeforeClaim,
    /// Device disappeared after the daemon/broker acquired the lock.
    DeviceDepartedAfterLock,
    /// Device disappeared during bind/import.
    DeviceDepartedDuringMutation,
    /// A device reappeared at the same logical busid with a different topology.
    DeviceReappearedWithDifferentTopology,
    /// Persisted lock is held by a different owner.
    LockHeldByOtherOwner,
    /// Persisted lock claim is stale or corrupt.
    InvalidPersistedLockClaim,
    /// Host carrier/backend is missing or unavailable.
    CarrierUnavailable,
    /// Host kernel bind is missing or points at an unexpected driver.
    HostBindUnavailable,
    /// Per-env proxy is missing, stale, or failed.
    ProxyUnavailable,
    /// Guest import has not converged.
    GuestImportUnavailable,
    /// Host has stale state for an undeclared/releasing claim.
    StaleHostState,
    /// Guest has stale state for an undeclared/releasing claim.
    StaleGuestState,
    /// Probe was incomplete; retry before mutating.
    ProbeIncomplete,
}

/// Closed public/status code for a degraded USB row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipDegradedReasonCode {
    PolicyFailed,
    DeviceDepartedBeforeClaim,
    DeviceDepartedAfterLock,
    DeviceDepartedDuringMutation,
    DeviceReappearedWithDifferentTopology,
    LockHeldByOtherOwner,
    InvalidPersistedLockClaim,
    CarrierUnavailable,
    HostBindUnavailable,
    ProxyUnavailable,
    GuestImportUnavailable,
    StaleHostState,
    StaleGuestState,
    ProbeIncomplete,
}

impl UsbipDegradedReasonCode {
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::PolicyFailed => "policy-failed",
            Self::DeviceDepartedBeforeClaim => "device-departed-before-claim",
            Self::DeviceDepartedAfterLock => "device-departed-after-lock",
            Self::DeviceDepartedDuringMutation => "device-departed-during-mutation",
            Self::DeviceReappearedWithDifferentTopology => "device-reappeared-different-topology",
            Self::LockHeldByOtherOwner => "lock-held-by-other-owner",
            Self::InvalidPersistedLockClaim => "invalid-persisted-lock-claim",
            Self::CarrierUnavailable => "carrier-unavailable",
            Self::HostBindUnavailable => "host-bind-unavailable",
            Self::ProxyUnavailable => "proxy-unavailable",
            Self::GuestImportUnavailable => "guest-import-unavailable",
            Self::StaleHostState => "stale-host-state",
            Self::StaleGuestState => "stale-guest-state",
            Self::ProbeIncomplete => "probe-incomplete",
        }
    }
}

impl UsbipPolicyFailure {
    pub const fn telemetry_label(&self) -> &'static str {
        match self {
            Self::FeatureDisabled => "feature-disabled",
            Self::MissingBundleIntent => "missing-bundle-intent",
            Self::DeviceNotDeclaredForVm => "device-not-declared-for-vm",
            Self::DeviceNotDeclaredForEnv => "device-not-declared-for-env",
            Self::TopologyMismatch => "topology-mismatch",
            Self::AmbiguousPhysicalMatch => "ambiguous-physical-match",
            Self::AuthorizationDenied => "authorization-denied",
        }
    }
}

/// Bounded telemetry/log labels projected from a degraded reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipTelemetryLabels {
    pub reason: &'static str,
    pub policy: &'static str,
}

/// Closed USB event type used for dedupe/rate-limit buckets and metric label
/// projection. Raw operation names, trace IDs, and process IDs must never create
/// additional buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipEventType {
    Degraded,
    StateTransition,
    SuppressedSummary,
    Other,
}

impl UsbipEventType {
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::Degraded => "degraded",
            Self::StateTransition => "state-transition",
            Self::SuppressedSummary => "suppressed-summary",
            Self::Other => "other",
        }
    }
}

/// Bounded source component for USB event buckets. Partition by VM/component
/// class, never by process ID, bus ID, sysfs path, trace ID, or serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipEventSourceKind {
    Vm,
    Host,
    Guest,
    Broker,
    Reconciler,
    Other,
}

impl UsbipEventSourceKind {
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::Vm => "vm",
            Self::Host => "host",
            Self::Guest => "guest",
            Self::Broker => "broker",
            Self::Reconciler => "reconciler",
            Self::Other => "other",
        }
    }
}

/// Bounded source projection for USB structured events and dedupe buckets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipEventSource {
    pub kind: UsbipEventSourceKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_usbip_event_source_vm"
    )]
    pub vm: Option<String>,
}

impl UsbipEventSource {
    pub fn vm(vm: impl AsRef<str>) -> Self {
        Self {
            kind: UsbipEventSourceKind::Vm,
            vm: Some(project_usbip_vm_label(vm.as_ref()).to_owned()),
        }
    }

    pub fn component(kind: UsbipEventSourceKind) -> Self {
        Self { kind, vm: None }
    }

    pub fn telemetry_labels(&self) -> UsbipEventSourceLabels<'_> {
        UsbipEventSourceLabels {
            source_kind: self.kind.telemetry_label(),
            vm: self.metric_vm_label(),
        }
    }

    fn metric_vm_label(&self) -> &'static str {
        match self.vm.as_deref() {
            None => "none",
            Some("other") => "other",
            Some(_) => "present",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipEventSourceLabels<'a> {
    pub source_kind: &'static str,
    pub vm: &'a str,
}

/// Bounded correlation identifier for one reconcile attempt. This is safe in
/// structured USB events but must never be projected into metric labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsbipReconcileCorrelationId(String);

impl UsbipReconcileCorrelationId {
    pub fn new(value: impl AsRef<str>) -> Option<Self> {
        let value = value.as_ref();
        let valid = !value.is_empty()
            && value.len() <= USBIP_RECONCILE_CORRELATION_ID_MAX_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            && !looks_like_trace_id(value);
        valid.then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for UsbipReconcileCorrelationId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UsbipReconcileCorrelationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).ok_or_else(|| D::Error::custom("invalid USB reconcile correlation id"))
    }
}

/// Bucketed lifecycle/correlation context attached to structured USB events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipReconcileAttemptContext {
    pub correlation_id: UsbipReconcileCorrelationId,
}

impl UsbipDegradedReason {
    pub fn code(&self) -> UsbipDegradedReasonCode {
        match self {
            Self::PolicyFailed(_) => UsbipDegradedReasonCode::PolicyFailed,
            Self::DeviceDepartedBeforeClaim => UsbipDegradedReasonCode::DeviceDepartedBeforeClaim,
            Self::DeviceDepartedAfterLock => UsbipDegradedReasonCode::DeviceDepartedAfterLock,
            Self::DeviceDepartedDuringMutation => {
                UsbipDegradedReasonCode::DeviceDepartedDuringMutation
            }
            Self::DeviceReappearedWithDifferentTopology => {
                UsbipDegradedReasonCode::DeviceReappearedWithDifferentTopology
            }
            Self::LockHeldByOtherOwner => UsbipDegradedReasonCode::LockHeldByOtherOwner,
            Self::InvalidPersistedLockClaim => UsbipDegradedReasonCode::InvalidPersistedLockClaim,
            Self::CarrierUnavailable => UsbipDegradedReasonCode::CarrierUnavailable,
            Self::HostBindUnavailable => UsbipDegradedReasonCode::HostBindUnavailable,
            Self::ProxyUnavailable => UsbipDegradedReasonCode::ProxyUnavailable,
            Self::GuestImportUnavailable => UsbipDegradedReasonCode::GuestImportUnavailable,
            Self::StaleHostState => UsbipDegradedReasonCode::StaleHostState,
            Self::StaleGuestState => UsbipDegradedReasonCode::StaleGuestState,
            Self::ProbeIncomplete => UsbipDegradedReasonCode::ProbeIncomplete,
        }
    }

    pub fn telemetry_labels(&self) -> UsbipTelemetryLabels {
        UsbipTelemetryLabels {
            reason: self.code().telemetry_label(),
            policy: match self {
                Self::PolicyFailed(policy) => policy.telemetry_label(),
                _ => "none",
            },
        }
    }

    pub fn summary(&self) -> &'static str {
        match self.code() {
            UsbipDegradedReasonCode::PolicyFailed => "USB policy does not allow this claim",
            UsbipDegradedReasonCode::DeviceDepartedBeforeClaim => {
                "the USB device was not present before claiming"
            }
            UsbipDegradedReasonCode::DeviceDepartedAfterLock => {
                "the USB device disappeared after broker claim acquisition"
            }
            UsbipDegradedReasonCode::DeviceDepartedDuringMutation => {
                "the USB device disappeared while host or guest state was changing"
            }
            UsbipDegradedReasonCode::DeviceReappearedWithDifferentTopology => {
                "a different USB device appeared at the expected location"
            }
            UsbipDegradedReasonCode::LockHeldByOtherOwner => {
                "another owner currently holds the USB claim"
            }
            UsbipDegradedReasonCode::InvalidPersistedLockClaim => {
                "the broker-mediated USB claim is missing, stale, or invalid"
            }
            UsbipDegradedReasonCode::CarrierUnavailable => {
                "the host USBIP carrier or backend is unavailable"
            }
            UsbipDegradedReasonCode::HostBindUnavailable => {
                "the host USB device is not bound for USBIP export"
            }
            UsbipDegradedReasonCode::ProxyUnavailable => {
                "the per-environment USBIP proxy is unavailable"
            }
            UsbipDegradedReasonCode::GuestImportUnavailable => {
                "the guest USBIP import has not converged"
            }
            UsbipDegradedReasonCode::StaleHostState => {
                "host USBIP state remains after the claim was removed"
            }
            UsbipDegradedReasonCode::StaleGuestState => {
                "guest USBIP state remains after the claim was removed"
            }
            UsbipDegradedReasonCode::ProbeIncomplete => {
                "USB probing did not produce a reconciliation-safe identity"
            }
        }
    }

    pub fn remediation(&self) -> &'static str {
        match self.code() {
            UsbipDegradedReasonCode::PolicyFailed => {
                "fix the USBIP declaration or caller authorization, rebuild the bundle, and retry the USB lifecycle verb"
            }
            UsbipDegradedReasonCode::DeviceDepartedBeforeClaim
            | UsbipDegradedReasonCode::DeviceDepartedAfterLock
            | UsbipDegradedReasonCode::DeviceDepartedDuringMutation => {
                "reconnect the physical device, wait for the host to observe it, then rerun the USB probe or lifecycle verb"
            }
            UsbipDegradedReasonCode::DeviceReappearedWithDifferentTopology => {
                "verify the physical device identity, update the declaration if intentional, and retry after the probe is stable"
            }
            UsbipDegradedReasonCode::LockHeldByOtherOwner => {
                "stop or detach the owning VM/environment before retrying this USB claim"
            }
            UsbipDegradedReasonCode::InvalidPersistedLockClaim => {
                "run the USB reconciler after confirming no active owner still uses the device; remove only broker-owned stale claim state"
            }
            UsbipDegradedReasonCode::CarrierUnavailable => {
                "ensure the usbip-host kernel module and per-environment backend are available, then retry"
            }
            UsbipDegradedReasonCode::HostBindUnavailable => {
                "for attach/start, rerun the USB lifecycle verb so the broker can bind the device to usbip-host; for detach/stop cleanup refused before unbind, stop the VM so USBIP streams drain or retry once a single targeted stream can be proven"
            }
            UsbipDegradedReasonCode::ProxyUnavailable => {
                "restart or reconcile the per-environment USBIP proxy before guest attach"
            }
            UsbipDegradedReasonCode::GuestImportUnavailable => {
                "check guest-control USBIP capability and retry the attach after host export is healthy"
            }
            UsbipDegradedReasonCode::StaleHostState => {
                "rerun USB detach/reconcile to drain host export and proxy state for the removed claim"
            }
            UsbipDegradedReasonCode::StaleGuestState => {
                "rerun USB detach/reconcile so guestd removes stale imported-device state"
            }
            UsbipDegradedReasonCode::ProbeIncomplete => {
                "retry the USB probe; if it repeats, verify the declaration has a stable physical selector"
            }
        }
    }

    pub fn to_public_reason(&self) -> UsbipPublicDegradedReason {
        UsbipPublicDegradedReason {
            code: self.code(),
            policy_failure: match self {
                Self::PolicyFailed(policy) => Some(*policy),
                _ => None,
            },
            summary: self.summary().to_owned(),
            remediation: self.remediation().to_owned(),
        }
    }
}

/// Structured, redacted status/probe reason detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipPublicDegradedReason {
    pub code: UsbipDegradedReasonCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_failure: Option<UsbipPolicyFailure>,
    pub summary: String,
    pub remediation: String,
}

/// Redacted event primitive for future live reconciliation/audit emission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipDegradedEvent {
    pub event_type: UsbipEventType,
    pub source: UsbipEventSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<UsbipReconcileCorrelationId>,
    pub claim_ref: String,
    pub reason: UsbipPublicDegradedReason,
    pub telemetry_reason: String,
    pub telemetry_policy: String,
}

/// Dedupe key for the bounded in-memory USB degraded-event limiter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipDegradedDedupeKey {
    pub event_type: UsbipEventType,
    pub source: UsbipEventSource,
    /// Internal-only claim partition; omitted from serialized events and metric
    /// labels so distinct claims do not share a rate-limit bucket.
    #[serde(skip)]
    pub claim_ref: String,
    pub reason: UsbipDegradedReasonCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_failure: Option<UsbipPolicyFailure>,
}

impl UsbipDegradedDedupeKey {
    fn bucket_key(&self) -> UsbipEventBucketKey {
        UsbipEventBucketKey {
            event_type: self.event_type,
            source_kind: self.source.kind,
            source_vm: UsbipEventSourceVmProjection::from_source(&self.source),
        }
    }

    fn limiter_key(&self) -> UsbipEventLimiterKey {
        UsbipEventLimiterKey {
            bucket: self.bucket_key(),
            claim_ref: self.claim_ref.clone(),
        }
    }
}

/// Capacity for the USB degraded/state event dedupe map. One bucket is reserved
/// for overflow so unbounded sources collapse to `other` instead of causing LRU
/// churn.
pub const USBIP_DEGRADED_DEDUPE_CAPACITY_HINT: usize = 256;
/// Per-key rate-limit window for USB degraded/state event emitters.
pub const USBIP_DEGRADED_RATE_LIMIT_WINDOW_SECS_HINT: u64 = 60;
/// Default number of identical USB degraded/state events emitted per bucket per
/// window before summaries take over.
pub const USBIP_DEGRADED_EVENTS_PER_WINDOW: u64 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipEventMetricLabels {
    pub event_type: &'static str,
    pub source_kind: &'static str,
    pub source_vm: &'static str,
    pub reason: &'static str,
    pub policy: &'static str,
}

/// Closed VM-source projection used by rate-limit buckets and metric labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipEventSourceVmProjection {
    None,
    Present,
    Other,
}

impl UsbipEventSourceVmProjection {
    fn from_source(source: &UsbipEventSource) -> Self {
        match source.vm.as_deref() {
            None => Self::None,
            Some("other") => Self::Other,
            Some(_) => Self::Present,
        }
    }

    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Present => "present",
            Self::Other => "other",
        }
    }
}

/// Strictly bounded metric bucket projection for USB event dedupe/rate limiting.
///
/// Metric labels deliberately exclude process IDs, trace IDs, raw VM names, bus
/// IDs, sysfs paths, serials, claim refs, and even closed reason codes. The
/// in-memory limiter additionally partitions by bounded internal claim ref so
/// distinct claims on one VM do not suppress each other.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipEventBucketKey {
    pub event_type: UsbipEventType,
    pub source_kind: UsbipEventSourceKind,
    pub source_vm: UsbipEventSourceVmProjection,
}

impl UsbipEventBucketKey {
    fn overflow() -> Self {
        Self {
            event_type: UsbipEventType::Other,
            source_kind: UsbipEventSourceKind::Other,
            source_vm: UsbipEventSourceVmProjection::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UsbipEventLimiterKey {
    bucket: UsbipEventBucketKey,
    claim_ref: String,
}

impl UsbipEventLimiterKey {
    fn overflow() -> Self {
        Self {
            bucket: UsbipEventBucketKey::overflow(),
            claim_ref: String::new(),
        }
    }
}

impl UsbipEventBucketKey {
    pub fn metric_labels(&self) -> UsbipEventMetricLabels {
        UsbipEventMetricLabels {
            event_type: self.event_type.telemetry_label(),
            source_kind: self.source_kind.telemetry_label(),
            source_vm: self.source_vm.telemetry_label(),
            reason: "none",
            policy: "none",
        }
    }
}

impl UsbipDegradedDedupeKey {
    pub fn metric_labels(&self) -> UsbipEventMetricLabels {
        UsbipEventMetricLabels {
            event_type: self.event_type.telemetry_label(),
            source_kind: self.source.kind.telemetry_label(),
            source_vm: self.source.metric_vm_label(),
            reason: self.reason.telemetry_label(),
            policy: self
                .policy_failure
                .as_ref()
                .map_or("none", UsbipPolicyFailure::telemetry_label),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipSuppressedEventSummary {
    pub bucket: UsbipEventBucketKey,
    pub suppressed_count: u64,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipEventLimiterDecision {
    pub emit_event: bool,
    pub bucket_key: UsbipEventBucketKey,
    pub suppressed_summary: Option<UsbipSuppressedEventSummary>,
}

#[derive(Debug, Clone)]
struct UsbipEventBucketState {
    emitted: u64,
    suppressed: u64,
    window_start: Duration,
}

/// Bounded in-memory dedupe/rate limiter for USB degraded/state events.
///
/// The limiter key is the closed event type + bounded source projection +
/// bounded internal claim ref. Metric labels still exclude claim refs, process
/// IDs, trace IDs, raw VM names, sysfs paths, serials, and reason strings. When
/// the bucket cap is reached, new keys are projected to a single `other`
/// overflow bucket.
#[derive(Debug, Clone)]
pub struct UsbipEventDedupeLimiter {
    buckets: HashMap<UsbipEventLimiterKey, UsbipEventBucketState>,
    max_buckets: usize,
    max_per_window: u64,
    window: Duration,
}

impl UsbipEventDedupeLimiter {
    pub fn new() -> Self {
        Self::with_limits(
            USBIP_DEGRADED_DEDUPE_CAPACITY_HINT,
            USBIP_DEGRADED_EVENTS_PER_WINDOW,
            Duration::from_secs(USBIP_DEGRADED_RATE_LIMIT_WINDOW_SECS_HINT),
        )
    }

    pub fn with_limits(max_buckets: usize, max_per_window: u64, window: Duration) -> Self {
        Self {
            buckets: HashMap::new(),
            max_buckets: max_buckets.max(1),
            max_per_window,
            window,
        }
    }

    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    pub fn observe(
        &mut self,
        now: Duration,
        key: UsbipDegradedDedupeKey,
    ) -> UsbipEventLimiterDecision {
        let limiter_key = self.project_key_for_capacity(key.limiter_key());
        let bucket_key = limiter_key.bucket.clone();
        let bucket = self
            .buckets
            .entry(limiter_key)
            .or_insert_with(|| UsbipEventBucketState {
                emitted: 0,
                suppressed: 0,
                window_start: now,
            });

        let suppressed_summary = if now.saturating_sub(bucket.window_start) >= self.window {
            let summary = suppressed_summary_for_bucket(&bucket_key, bucket, now);
            bucket.emitted = 0;
            bucket.suppressed = 0;
            bucket.window_start = now;
            summary
        } else {
            None
        };

        let emit_event = if bucket.emitted < self.max_per_window {
            bucket.emitted += 1;
            true
        } else {
            bucket.suppressed += 1;
            false
        };

        UsbipEventLimiterDecision {
            emit_event,
            bucket_key,
            suppressed_summary,
        }
    }

    pub fn flush_suppressed(&mut self, now: Duration) -> Vec<UsbipSuppressedEventSummary> {
        self.buckets
            .iter_mut()
            .filter_map(|(key, bucket)| {
                let summary = suppressed_summary_for_bucket(&key.bucket, bucket, now);
                bucket.suppressed = 0;
                bucket.window_start = now;
                summary
            })
            .collect()
    }

    fn project_key_for_capacity(&self, key: UsbipEventLimiterKey) -> UsbipEventLimiterKey {
        if self.buckets.contains_key(&key) {
            return key;
        }
        let reserve_threshold = self.max_buckets.saturating_sub(1);
        if self.buckets.len() >= reserve_threshold {
            UsbipEventLimiterKey::overflow()
        } else {
            key
        }
    }
}

impl Default for UsbipEventDedupeLimiter {
    fn default() -> Self {
        Self::new()
    }
}

fn suppressed_summary_for_bucket(
    key: &UsbipEventBucketKey,
    bucket: &UsbipEventBucketState,
    now: Duration,
) -> Option<UsbipSuppressedEventSummary> {
    (bucket.suppressed > 0).then(|| UsbipSuppressedEventSummary {
        bucket: key.clone(),
        suppressed_count: bucket.suppressed,
        window_start_ms: duration_millis_u64(bucket.window_start),
        window_end_ms: duration_millis_u64(now),
    })
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipStepFailureReasonKind {
    MissingBundleIntent,
    LockUnavailable,
    CarrierUnavailable,
    HostBindUnavailable,
    ProxyUnavailable,
    CommandTimeout,
    PermissionDenied,
    InvalidInput,
    TransportUnavailable,
    Other,
}

impl UsbipStepFailureReasonKind {
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::MissingBundleIntent => "missing-bundle-intent",
            Self::LockUnavailable => "lock-unavailable",
            Self::CarrierUnavailable => "carrier-unavailable",
            Self::HostBindUnavailable => "host-bind-unavailable",
            Self::ProxyUnavailable => "proxy-unavailable",
            Self::CommandTimeout => "command-timeout",
            Self::PermissionDenied => "permission-denied",
            Self::InvalidInput => "invalid-input",
            Self::TransportUnavailable => "transport-unavailable",
            Self::Other => "other",
        }
    }

    pub const fn summary(self) -> &'static str {
        match self {
            Self::MissingBundleIntent => "trusted bundle intent is missing",
            Self::LockUnavailable => "USB claim is unavailable",
            Self::CarrierUnavailable => "USBIP carrier is unavailable",
            Self::HostBindUnavailable => "host USBIP bind is unavailable",
            Self::ProxyUnavailable => "USBIP proxy is unavailable",
            Self::CommandTimeout => "USBIP command timed out",
            Self::PermissionDenied => "USBIP operation was denied",
            Self::InvalidInput => "USBIP operation input was invalid",
            Self::TransportUnavailable => "USBIP transport is unavailable",
            Self::Other => "USBIP step failed",
        }
    }

    pub const fn remediation(self) -> &'static str {
        match self {
            Self::MissingBundleIntent => {
                "rebuild the trusted bundle so the USBIP firewall/bind intent exists, then retry"
            }
            Self::LockUnavailable => {
                "stop the current owner or wait for stale claim reconciliation, then retry"
            }
            Self::CarrierUnavailable => {
                "ensure the usbip-host module and per-environment backend are available, then retry"
            }
            Self::HostBindUnavailable => {
                "rerun the lifecycle verb so the broker can bind the device for export"
            }
            Self::ProxyUnavailable => {
                "restart or reconcile the per-environment USBIP proxy, then retry"
            }
            Self::CommandTimeout => "retry after the host and guest USBIP services settle",
            Self::PermissionDenied => {
                "verify the caller and bundle policy permit this USB operation"
            }
            Self::InvalidInput => "retry with a valid declared USB selector",
            Self::TransportUnavailable => {
                "verify guest-control and USBIP transport readiness, then retry"
            }
            Self::Other => {
                "run `nixling usb probe` and retry the lifecycle verb after the reported posture is healthy"
            }
        }
    }
}

pub fn classify_usbip_step_failure(step: &str, detail: &str) -> UsbipStepFailureReasonKind {
    let detail = detail.to_ascii_lowercase();
    if detail.contains("timeout") || detail.contains("timed out") {
        return UsbipStepFailureReasonKind::CommandTimeout;
    }
    if detail.contains("permission")
        || detail.contains("denied")
        || detail.contains("not authorized")
    {
        return UsbipStepFailureReasonKind::PermissionDenied;
    }
    if detail.contains("invalid") || detail.contains("malformed") {
        return UsbipStepFailureReasonKind::InvalidInput;
    }
    if detail.contains("transport") || detail.contains("connection") || detail.contains("refused") {
        return UsbipStepFailureReasonKind::TransportUnavailable;
    }
    if detail.contains("trusted bundle has no") || detail.contains("missing bundle") {
        return UsbipStepFailureReasonKind::MissingBundleIntent;
    }

    match step {
        "lock" => UsbipStepFailureReasonKind::LockUnavailable,
        "modprobe" | "backend" => UsbipStepFailureReasonKind::CarrierUnavailable,
        "bind" => UsbipStepFailureReasonKind::HostBindUnavailable,
        "proxy" | "firewall" => UsbipStepFailureReasonKind::ProxyUnavailable,
        _ => UsbipStepFailureReasonKind::Other,
    }
}

/// USB vendor/product identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbVidPid {
    /// Four hexadecimal characters when known.
    pub vendor_id: Option<String>,
    /// Four hexadecimal characters when known.
    pub product_id: Option<String>,
}

impl UsbVidPid {
    /// Returns true when both vendor and product ids are present.
    pub fn is_complete(&self) -> bool {
        self.vendor_id.is_some() && self.product_id.is_some()
    }
}

/// Parsed physical USB bus/port topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbBusPortTopology {
    /// Kernel USB bus number.
    pub bus_number: u16,
    /// Downstream physical port chain from the root hub.
    pub port_chain: Vec<u8>,
    /// Canonical Linux USB busid, e.g. `1-2.4`.
    pub canonical_bus_id: String,
}

/// Topology parser failure. These values are daemon-internal and must not be
/// surfaced directly on public status DTOs because they may carry raw paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbTopologyParseError {
    /// Busid did not match the canonical Linux USB shape.
    InvalidBusId(String),
    /// Numeric bus field could not be parsed.
    InvalidBusNumber { field: &'static str, value: String },
    /// Numeric port field could not be parsed.
    InvalidPortNumber { field: &'static str, value: String },
    /// Vendor/product field was not a four-character hex id.
    InvalidVendorProduct { field: &'static str, value: String },
    /// Two observed topology sources disagreed on the bus number.
    ConflictingBusNumber { left: u16, right: u16 },
    /// Two observed topology sources disagreed on the port chain.
    ConflictingPortChain { left: Vec<u8>, right: Vec<u8> },
}

/// In-memory sysfs fields used by both the live reader and pure tests.
///
/// Broker host inspection can populate this from a privileged namespace later;
/// daemon restart reconciliation uses the same parser so host-session claims and live
/// observations are compared with one topology contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct UsbSysfsPhysicalFields<'a> {
    /// Raw sysfs device path. Stored only in daemon-internal state.
    pub sysfs_path: Option<&'a Path>,
    /// Linux USB busid such as `1-2.4`.
    pub bus_id: Option<&'a str>,
    /// `idVendor`.
    pub id_vendor: Option<&'a str>,
    /// `idProduct`.
    pub id_product: Option<&'a str>,
    /// `busnum` / bBusNumber-like field.
    pub busnum: Option<&'a str>,
    /// `devpath` / physical port-chain-like field.
    pub devpath: Option<&'a str>,
    /// `port_number` / bPortNumber-like field when available.
    pub port_number: Option<&'a str>,
    /// Serial-like descriptor. Supplemental only; never sufficient to match.
    pub serial: Option<&'a str>,
}

/// Strength of the internal physical topology observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbPhysicalTopologyClass {
    /// No physical anchor was observed.
    Missing,
    /// Only the bus number was observed; useful context but not unique enough.
    BusOnly,
    /// A downstream port chain or busid was observed without an explicit bus.
    PortChain,
    /// A raw sysfs physical path was observed without parsed bus/port fields.
    SysfsPath,
    /// Bus plus downstream port chain were observed.
    BusAndPortChain,
    /// Raw sysfs path plus parsed bus/port fields were observed.
    SysfsPathAndPortChain,
}

/// Result of comparing a host-session declared claim to an observed device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbDurableClaimMatch {
    /// Allowed vendor/product and physical topology match.
    Match,
    /// The host-session claim lacks a complete allowed vendor/product pair.
    MissingAllowedVendorProduct,
    /// Observed vendor/product does not match the allowed pair.
    VendorProductMismatch,
    /// The host-session claim has no spoof-resistant physical anchor.
    MissingDeclaredPhysicalAnchor,
    /// The observed device has no spoof-resistant physical anchor.
    MissingObservedPhysicalAnchor,
    /// Allowed vendor/product matched, but physical topology differed.
    PhysicalTopologyMismatch,
}

/// Classification of a sysfs attribute read race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSysfsAttrReadFailure {
    /// Attribute or device node was not found.
    NotFound,
    /// Kernel reported the USB device departed during the read.
    DepartedDevice,
    /// Non-departure I/O failure.
    Other {
        /// Portable error kind.
        kind: io::ErrorKind,
        /// Raw errno when available.
        raw_os_error: Option<i32>,
    },
}

/// Outcome of reading one trimmed sysfs attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbSysfsAttrRead {
    /// Attribute was present; trailing sysfs newline(s) were removed.
    Present(String),
    /// Attribute path was absent.
    Missing,
    /// Device departed while the attribute was read.
    Departed,
}

/// Non-departure I/O failure while reading sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbSysfsIoError {
    /// Attribute name being read.
    pub attr: String,
    /// Portable error kind.
    pub kind: io::ErrorKind,
    /// Raw errno when available.
    pub raw_os_error: Option<i32>,
}

/// Live sysfs probe failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbSysfsProbeError {
    /// Non-departure I/O failure.
    Io(UsbSysfsIoError),
    /// Sysfs fields were present but internally inconsistent or malformed.
    Parse(UsbTopologyParseError),
}

impl From<UsbSysfsIoError> for UsbSysfsProbeError {
    fn from(value: UsbSysfsIoError) -> Self {
        Self::Io(value)
    }
}

impl From<UsbTopologyParseError> for UsbSysfsProbeError {
    fn from(value: UsbTopologyParseError) -> Self {
        Self::Parse(value)
    }
}

/// Trim only sysfs line endings; do not perform broad whitespace cleanup.
pub fn trim_sysfs_value(raw: &str) -> String {
    raw.trim_end_matches(&['\r', '\n'][..]).to_owned()
}

/// Parse a Linux USB busid (`1-2.4`) into bus and physical port chain.
pub fn parse_usb_busid_topology(bus_id: &str) -> Result<UsbBusPortTopology, UsbTopologyParseError> {
    nixling_contracts::usbip::validate_bus_id(bus_id)
        .map_err(|_| UsbTopologyParseError::InvalidBusId(bus_id.to_owned()))?;
    let (bus, ports) = match bus_id.split_once('-') {
        Some((bus, ports)) => (bus, Some(ports)),
        None => (bus_id, None),
    };
    let bus_number = parse_bus_number("busid", bus)?;
    let port_chain = match ports {
        Some(ports) => parse_port_chain("busid", ports)?,
        None => Vec::new(),
    };
    Ok(UsbBusPortTopology {
        bus_number,
        port_chain,
        canonical_bus_id: bus_id.to_owned(),
    })
}

/// Parse a sysfs `devpath`/port-chain string such as `2.4`.
pub fn parse_usb_port_chain(raw: &str) -> Result<Vec<u8>, UsbTopologyParseError> {
    parse_port_chain("devpath", &trim_sysfs_value(raw))
}

/// Extract USB topology from a sysfs path or symlink target.
pub fn parse_usb_topology_from_sysfs_path(
    sysfs_path: &Path,
) -> Result<UsbBusPortTopology, UsbTopologyParseError> {
    for component in sysfs_path.components().rev() {
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        let device_name = name.split_once(':').map_or(name, |(device, _)| device);
        if let Ok(topology) = parse_usb_busid_topology(device_name) {
            return Ok(topology);
        }
    }
    Err(UsbTopologyParseError::InvalidBusId(
        sysfs_path.display().to_string(),
    ))
}

/// Build an internal identity from already-read sysfs fields.
pub fn identity_from_sysfs_fields(
    fields: UsbSysfsPhysicalFields<'_>,
) -> Result<UsbPhysicalTopologyIdentity, UsbTopologyParseError> {
    let mut bus_number = None;
    let mut port_chain: Option<Vec<u8>> = None;

    if let Some(bus_id) = fields.bus_id {
        let parsed = parse_usb_busid_topology(&trim_sysfs_value(bus_id))?;
        merge_bus_number(&mut bus_number, Some(parsed.bus_number))?;
        merge_port_chain(&mut port_chain, Some(parsed.port_chain))?;
    }
    if let Some(path) = fields.sysfs_path
        && let Ok(parsed) = parse_usb_topology_from_sysfs_path(path)
    {
        merge_bus_number(&mut bus_number, Some(parsed.bus_number))?;
        merge_port_chain(&mut port_chain, Some(parsed.port_chain))?;
    }
    if let Some(busnum) = fields.busnum {
        merge_bus_number(
            &mut bus_number,
            Some(parse_bus_number("busnum", &trim_sysfs_value(busnum))?),
        )?;
    }
    if let Some(devpath) = fields.devpath {
        merge_port_chain(
            &mut port_chain,
            Some(parse_port_chain("devpath", &trim_sysfs_value(devpath))?),
        )?;
    }
    if let Some(port_number) = fields.port_number {
        let port = parse_port_number("port_number", &trim_sysfs_value(port_number))?;
        match &mut port_chain {
            Some(chain) if !chain.is_empty() && chain.last() != Some(&port) => {
                return Err(UsbTopologyParseError::ConflictingPortChain {
                    left: chain.clone(),
                    right: vec![port],
                });
            }
            Some(chain) if chain.is_empty() => chain.push(port),
            Some(_) => {}
            None => port_chain = Some(vec![port]),
        }
    }

    Ok(UsbPhysicalTopologyIdentity {
        vid_pid: UsbVidPid {
            vendor_id: fields
                .id_vendor
                .map(|value| normalize_usb_hex4("idVendor", value))
                .transpose()?,
            product_id: fields
                .id_product
                .map(|value| normalize_usb_hex4("idProduct", value))
                .transpose()?,
        },
        bus_number,
        port_chain: port_chain.unwrap_or_default(),
        sysfs_path: fields.sysfs_path.map(Path::to_path_buf),
        serial_like: fields
            .serial
            .map(trim_sysfs_value)
            .filter(|s| !s.is_empty()),
    })
}

/// Classify sysfs read errors without collapsing disappeared devices into
/// generic I/O failures.
pub fn classify_usb_sysfs_read_error(error: &io::Error) -> UsbSysfsAttrReadFailure {
    if error.kind() == io::ErrorKind::NotFound {
        UsbSysfsAttrReadFailure::NotFound
    } else if error.raw_os_error() == Some(libc::ENODEV) {
        UsbSysfsAttrReadFailure::DepartedDevice
    } else {
        UsbSysfsAttrReadFailure::Other {
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        }
    }
}

/// Safely read and trim a sysfs attribute.
pub fn read_trimmed_sysfs_attr(
    device_path: &Path,
    attr: &str,
) -> Result<UsbSysfsAttrRead, UsbSysfsIoError> {
    match fs::read_to_string(device_path.join(attr)) {
        Ok(raw) => Ok(UsbSysfsAttrRead::Present(trim_sysfs_value(&raw))),
        Err(error) => match classify_usb_sysfs_read_error(&error) {
            UsbSysfsAttrReadFailure::NotFound => Ok(UsbSysfsAttrRead::Missing),
            UsbSysfsAttrReadFailure::DepartedDevice => Ok(UsbSysfsAttrRead::Departed),
            UsbSysfsAttrReadFailure::Other { kind, raw_os_error } => Err(UsbSysfsIoError {
                attr: attr.to_owned(),
                kind,
                raw_os_error,
            }),
        },
    }
}

/// Read a live sysfs USB device identity for restart reconciliation.
pub fn read_usb_sysfs_physical_identity(
    sysfs_path: &Path,
) -> Result<UsbipPhysicalPresence, UsbSysfsProbeError> {
    let id_vendor = match read_trimmed_sysfs_attr(sysfs_path, "idVendor")? {
        UsbSysfsAttrRead::Present(value) => value,
        UsbSysfsAttrRead::Missing | UsbSysfsAttrRead::Departed => {
            return Ok(UsbipPhysicalPresence::DepartedBeforeClaim);
        }
    };
    let id_product = match read_trimmed_sysfs_attr(sysfs_path, "idProduct")? {
        UsbSysfsAttrRead::Present(value) => value,
        UsbSysfsAttrRead::Missing | UsbSysfsAttrRead::Departed => {
            return Ok(UsbipPhysicalPresence::DepartedBeforeClaim);
        }
    };
    let busnum = read_optional_sysfs_attr(sysfs_path, "busnum")?;
    if busnum.departed {
        return Ok(UsbipPhysicalPresence::DepartedBeforeClaim);
    }
    let devpath = read_optional_sysfs_attr(sysfs_path, "devpath")?;
    if devpath.departed {
        return Ok(UsbipPhysicalPresence::DepartedBeforeClaim);
    }
    let port_number = read_optional_sysfs_attr(sysfs_path, "port_number")?;
    if port_number.departed {
        return Ok(UsbipPhysicalPresence::DepartedBeforeClaim);
    }
    let serial = read_optional_sysfs_attr(sysfs_path, "serial")?;
    if serial.departed {
        return Ok(UsbipPhysicalPresence::DepartedBeforeClaim);
    }

    let fields = UsbSysfsPhysicalFields {
        sysfs_path: Some(sysfs_path),
        bus_id: None,
        id_vendor: Some(&id_vendor),
        id_product: Some(&id_product),
        busnum: busnum.value.as_deref(),
        devpath: devpath.value.as_deref(),
        port_number: port_number.value.as_deref(),
        serial: serial.value.as_deref(),
    };
    Ok(UsbipPhysicalPresence::Present {
        identity: identity_from_sysfs_fields(fields)?,
    })
}

struct OptionalSysfsAttr {
    value: Option<String>,
    departed: bool,
}

fn read_optional_sysfs_attr(
    sysfs_path: &Path,
    attr: &str,
) -> Result<OptionalSysfsAttr, UsbSysfsIoError> {
    match read_trimmed_sysfs_attr(sysfs_path, attr)? {
        UsbSysfsAttrRead::Present(value) => Ok(OptionalSysfsAttr {
            value: Some(value),
            departed: false,
        }),
        UsbSysfsAttrRead::Missing => Ok(OptionalSysfsAttr {
            value: None,
            departed: false,
        }),
        UsbSysfsAttrRead::Departed => Ok(OptionalSysfsAttr {
            value: None,
            departed: true,
        }),
    }
}

fn parse_bus_number(field: &'static str, raw: &str) -> Result<u16, UsbTopologyParseError> {
    raw.parse::<u16>()
        .map_err(|_| UsbTopologyParseError::InvalidBusNumber {
            field,
            value: raw.to_owned(),
        })
}

fn parse_port_chain(field: &'static str, raw: &str) -> Result<Vec<u8>, UsbTopologyParseError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    raw.split('.')
        .map(|segment| parse_port_number(field, segment))
        .collect()
}

fn parse_port_number(field: &'static str, raw: &str) -> Result<u8, UsbTopologyParseError> {
    raw.parse::<u8>()
        .map_err(|_| UsbTopologyParseError::InvalidPortNumber {
            field,
            value: raw.to_owned(),
        })
}

fn normalize_usb_hex4(field: &'static str, raw: &str) -> Result<String, UsbTopologyParseError> {
    let value = trim_sysfs_value(raw).to_ascii_lowercase();
    if value.len() == 4 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(UsbTopologyParseError::InvalidVendorProduct { field, value })
    }
}

fn merge_bus_number(
    current: &mut Option<u16>,
    observed: Option<u16>,
) -> Result<(), UsbTopologyParseError> {
    let Some(observed) = observed else {
        return Ok(());
    };
    match *current {
        Some(existing) if existing != observed => {
            Err(UsbTopologyParseError::ConflictingBusNumber {
                left: existing,
                right: observed,
            })
        }
        Some(_) => Ok(()),
        None => {
            *current = Some(observed);
            Ok(())
        }
    }
}

fn merge_port_chain(
    current: &mut Option<Vec<u8>>,
    observed: Option<Vec<u8>>,
) -> Result<(), UsbTopologyParseError> {
    let Some(observed) = observed else {
        return Ok(());
    };
    match current {
        Some(existing) if *existing != observed => {
            Err(UsbTopologyParseError::ConflictingPortChain {
                left: existing.clone(),
                right: observed,
            })
        }
        Some(_) => Ok(()),
        None => {
            *current = Some(observed);
            Ok(())
        }
    }
}

fn vid_pid_matches(allowed: &UsbVidPid, observed: &UsbVidPid) -> bool {
    let (Some(allowed_vendor), Some(allowed_product)) = (&allowed.vendor_id, &allowed.product_id)
    else {
        return false;
    };
    let (Some(observed_vendor), Some(observed_product)) =
        (&observed.vendor_id, &observed.product_id)
    else {
        return false;
    };
    allowed_vendor.eq_ignore_ascii_case(observed_vendor)
        && allowed_product.eq_ignore_ascii_case(observed_product)
}

/// Internal physical identity. This is not a public/status DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbPhysicalTopologyIdentity {
    /// Vendor/product pair from descriptors.
    pub vid_pid: UsbVidPid,
    /// Kernel USB bus number when available.
    pub bus_number: Option<u16>,
    /// Physical downstream port chain, e.g. `[1, 4, 2]`.
    pub port_chain: Vec<u8>,
    /// Raw sysfs device path used only for daemon reconciliation/audit.
    pub sysfs_path: Option<PathBuf>,
    /// Serial-like descriptor, retained only as a supplemental hint.
    pub serial_like: Option<String>,
}

impl UsbPhysicalTopologyIdentity {
    /// True when the identity has a physical anchor besides serial.
    pub fn has_physical_anchor(&self) -> bool {
        self.bus_number.is_some() || !self.port_chain.is_empty() || self.sysfs_path.is_some()
    }

    /// True when the physical anchor is strong enough for restart reconciliation.
    pub fn has_spoof_resistant_anchor(&self) -> bool {
        self.sysfs_path.is_some() || (self.bus_number.is_some() && !self.port_chain.is_empty())
    }

    /// Strength of the internal physical topology observation.
    pub fn topology_class(&self) -> UsbPhysicalTopologyClass {
        match (
            self.sysfs_path.is_some(),
            self.bus_number.is_some(),
            self.port_chain.is_empty(),
        ) {
            (true, true, false) => UsbPhysicalTopologyClass::SysfsPathAndPortChain,
            (false, true, false) => UsbPhysicalTopologyClass::BusAndPortChain,
            (true, _, true) => UsbPhysicalTopologyClass::SysfsPath,
            (false, true, true) => UsbPhysicalTopologyClass::BusOnly,
            (false, false, false) => UsbPhysicalTopologyClass::PortChain,
            (true, false, false) => UsbPhysicalTopologyClass::SysfsPathAndPortChain,
            (false, false, true) => UsbPhysicalTopologyClass::Missing,
        }
    }

    /// True when the identity should be usable for reconciliation.
    pub fn is_reconciliation_safe(&self) -> bool {
        self.vid_pid.is_complete() && self.has_spoof_resistant_anchor()
    }

    /// Compare a host-session allowed claim against an observed device.
    ///
    /// Serial-like fields are deliberately ignored: an attacker can spoof them
    /// more easily than the combination of allowed VID/PID and a stable physical
    /// topology anchor.
    pub fn durable_claim_match(&self, observed: &Self) -> UsbDurableClaimMatch {
        if !self.vid_pid.is_complete() {
            return UsbDurableClaimMatch::MissingAllowedVendorProduct;
        }
        if !vid_pid_matches(&self.vid_pid, &observed.vid_pid) {
            return UsbDurableClaimMatch::VendorProductMismatch;
        }
        if !self.has_spoof_resistant_anchor() {
            return UsbDurableClaimMatch::MissingDeclaredPhysicalAnchor;
        }
        if !observed.has_spoof_resistant_anchor() {
            return UsbDurableClaimMatch::MissingObservedPhysicalAnchor;
        }
        if self.physical_topology_matches(observed) {
            UsbDurableClaimMatch::Match
        } else {
            UsbDurableClaimMatch::PhysicalTopologyMismatch
        }
    }

    /// Compare only physical anchors, ignoring VID/PID and serial-like fields.
    pub fn physical_topology_matches(&self, observed: &Self) -> bool {
        let bus_matches = match (self.bus_number, observed.bus_number) {
            (Some(left), Some(right)) if left != right => return false,
            (Some(_), Some(_)) => true,
            _ => false,
        };
        let ports_match = if !self.port_chain.is_empty() && !observed.port_chain.is_empty() {
            if self.port_chain != observed.port_chain {
                return false;
            }
            true
        } else {
            false
        };
        let sysfs_matches = match (&self.sysfs_path, &observed.sysfs_path) {
            (Some(left), Some(right)) if left != right => return false,
            (Some(_), Some(_)) => true,
            _ => false,
        };

        sysfs_matches || (bus_matches && ports_match)
    }

    /// Build the redacted identity exposed by future public/status DTOs.
    pub fn to_public_identity(&self) -> UsbipPublicDeviceIdentity {
        UsbipPublicDeviceIdentity {
            vendor_id: scrub_usb_hex_id(self.vid_pid.vendor_id.as_deref()),
            product_id: scrub_usb_hex_id(self.vid_pid.product_id.as_deref()),
            topology_anchor: if self.has_physical_anchor() {
                UsbipPublicTopologyAnchor::Observed
            } else {
                UsbipPublicTopologyAnchor::Missing
            },
            serial_observed: self.serial_like.is_some(),
        }
    }
}

/// Result of observing the physical device during reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum UsbipPhysicalPresence {
    /// Device is present with the given internal identity.
    Present {
        /// Internal topology identity.
        identity: UsbPhysicalTopologyIdentity,
    },
    /// Device was absent before any claim/bind side effect.
    DepartedBeforeClaim,
    /// Device was present for broker claim acquisition but absent later.
    DepartedAfterLock,
    /// Device vanished during bind/import mutation.
    DepartedDuringMutation,
    /// A later probe found a different device where the claim expected one.
    ReappearedDifferentTopology {
        /// Internal topology observed after reappearance.
        observed: UsbPhysicalTopologyIdentity,
    },
    /// Probe matched multiple physical devices.
    Ambiguous {
        /// Number of candidate devices.
        candidates: usize,
    },
    /// Probe could not complete reliably.
    ProbeIncomplete,
}

/// Declared desired claim from the trusted bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipDeclaredIntent {
    /// Desired claim state.
    pub desired: UsbipDesiredClaimState,
    /// Target environment when declared.
    pub env: Option<String>,
    /// Target VM when declared.
    pub vm: Option<String>,
    /// Daemon-internal busid. Do not expose on public status surfaces.
    pub bus_id: Option<String>,
    /// Declared physical identity if the bundle captured one.
    pub topology: Option<UsbPhysicalTopologyIdentity>,
    /// Policy failures found while resolving the declaration.
    pub policy_failures: Vec<UsbipPolicyFailure>,
}

/// Persisted broker-mediated USB device claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipPersistedLockClaim {
    /// Claim state as observed from the broker-owned artifact.
    pub state: UsbipPersistedLockClaimState,
    /// Owner environment recorded by the claim artifact, if parseable.
    pub env: Option<String>,
    /// Owner VM recorded by the claim artifact, if parseable.
    pub vm: Option<String>,
    /// Monotonic owner generation when available.
    pub generation: Option<u64>,
}

/// Host-observed carrier/bind/proxy state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipHostRuntimeState {
    /// Carrier/backend state.
    pub carrier: UsbipActiveCarrierState,
    /// Kernel bind state.
    pub bind: UsbipHostBindState,
    /// Per-env proxy listener state.
    pub proxy: UsbipProxyState,
}

/// Guest-observed import state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipGuestRuntimeState {
    /// Guest import state.
    pub import: UsbipGuestImportState,
    /// Owner generation echoed by guestd when available.
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipLifecycleClaim {
    pub vm: String,
    pub env: String,
    pub bus_id: String,
    pub host: String,
    pub claim_ref: String,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipLifecycleStep {
    HostBindReplay,
    ProxyReady,
    GuestStatus,
    GuestImport,
    GuestDetach,
    HostCarrierCleanup,
    ProxyReconcile,
    PreserveDurableClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipLifecycleFailureKind {
    RuntimeAbsent,
    PolicyMismatch,
    MissingBundleIntent,
    LockConflict,
    HostReplayFailed,
    ProxyFailed,
    GuestFailed,
}

impl UsbipLifecycleFailureKind {
    pub const fn degraded_reason(&self) -> UsbipDegradedReason {
        match self {
            Self::RuntimeAbsent => UsbipDegradedReason::DeviceDepartedBeforeClaim,
            Self::PolicyMismatch => {
                UsbipDegradedReason::PolicyFailed(UsbipPolicyFailure::TopologyMismatch)
            }
            Self::MissingBundleIntent => {
                UsbipDegradedReason::PolicyFailed(UsbipPolicyFailure::MissingBundleIntent)
            }
            Self::LockConflict => UsbipDegradedReason::LockHeldByOtherOwner,
            Self::HostReplayFailed => UsbipDegradedReason::HostBindUnavailable,
            Self::ProxyFailed => UsbipDegradedReason::ProxyUnavailable,
            Self::GuestFailed => UsbipDegradedReason::GuestImportUnavailable,
        }
    }

    pub const fn prevents_required_exposure(&self) -> bool {
        matches!(
            self,
            Self::PolicyMismatch | Self::MissingBundleIntent | Self::LockConflict
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipLifecycleStepError {
    pub kind: UsbipLifecycleFailureKind,
    pub detail: String,
}

impl UsbipLifecycleStepError {
    pub fn new(kind: UsbipLifecycleFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipLifecycleClaimReport {
    pub vm: String,
    pub env: String,
    pub bus_id: String,
    pub completed: Vec<UsbipLifecycleStep>,
    pub degraded: Vec<UsbipPublicDegradedReason>,
    pub fatal: bool,
}

impl UsbipLifecycleClaimReport {
    fn new(claim: &UsbipLifecycleClaim) -> Self {
        Self {
            vm: claim.vm.clone(),
            env: claim.env.clone(),
            bus_id: claim.bus_id.clone(),
            completed: Vec::new(),
            degraded: Vec::new(),
            fatal: false,
        }
    }

    fn push_degraded(&mut self, error: &UsbipLifecycleStepError, required: bool) {
        self.degraded
            .push(error.kind.degraded_reason().to_public_reason());
        self.fatal |= required && error.kind.prevents_required_exposure();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipLifecycleReconcileReport {
    pub claims: Vec<UsbipLifecycleClaimReport>,
}

impl UsbipLifecycleReconcileReport {
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    pub fn fatal(&self) -> bool {
        self.claims.iter().any(|claim| claim.fatal)
    }

    pub fn degraded_count(&self) -> usize {
        self.claims.iter().map(|claim| claim.degraded.len()).sum()
    }

    pub fn converged_count(&self) -> usize {
        self.claims
            .iter()
            .filter(|claim| claim.degraded.is_empty() && !claim.fatal)
            .count()
    }

    pub fn first_fatal_reason(&self) -> Option<&UsbipPublicDegradedReason> {
        self.claims
            .iter()
            .filter(|claim| claim.fatal)
            .find_map(|claim| claim.degraded.first())
    }
}

pub trait UsbipVmStartReconcileExecutor {
    fn replay_host_bind(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<(), UsbipLifecycleStepError>;
    fn ensure_proxy_ready(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<(), UsbipLifecycleStepError>;
    fn guest_status(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<UsbipGuestImportState, UsbipLifecycleStepError>;
    fn guest_import(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<(), UsbipLifecycleStepError>;
}

pub fn reconcile_usbip_vm_start_claims<E: UsbipVmStartReconcileExecutor>(
    claims: &[UsbipLifecycleClaim],
    attempt: &UsbipReconcileAttemptContext,
    executor: &mut E,
) -> UsbipLifecycleReconcileReport {
    let mut reports = Vec::with_capacity(claims.len());
    for claim in claims {
        let mut report = UsbipLifecycleClaimReport::new(claim);
        match executor.replay_host_bind(claim, attempt) {
            Ok(()) => report.completed.push(UsbipLifecycleStep::HostBindReplay),
            Err(error) => {
                report.push_degraded(&error, claim.required);
                reports.push(report);
                continue;
            }
        }
        match executor.ensure_proxy_ready(claim, attempt) {
            Ok(()) => report.completed.push(UsbipLifecycleStep::ProxyReady),
            Err(error) => {
                report.push_degraded(&error, claim.required);
                reports.push(report);
                continue;
            }
        }
        match executor.guest_status(claim, attempt) {
            Ok(UsbipGuestImportState::Imported) => {
                report.completed.push(UsbipLifecycleStep::GuestStatus);
            }
            Ok(_) => {
                report.completed.push(UsbipLifecycleStep::GuestStatus);
                match executor.guest_import(claim, attempt) {
                    Ok(()) => report.completed.push(UsbipLifecycleStep::GuestImport),
                    Err(error) => report.push_degraded(&error, claim.required),
                }
            }
            Err(error) => report.push_degraded(&error, claim.required),
        }
        reports.push(report);
    }
    UsbipLifecycleReconcileReport { claims: reports }
}

pub trait UsbipVmStopCarrierCleanup {
    fn observe_proxy_flow_for_cleanup(
        &mut self,
        _claim: &UsbipLifecycleClaim,
        _attempt: &UsbipReconcileAttemptContext,
    ) -> Result<UsbipProxyFlowObservation, UsbipLifecycleStepError> {
        Ok(UsbipProxyFlowObservation::SharedOrAmbiguous)
    }

    fn detach_guest_import(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<(), UsbipLifecycleStepError>;
    fn cleanup_host_carrier_preserve_claim(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<(), UsbipLifecycleStepError>;
    fn reconcile_proxy(
        &mut self,
        claim: &UsbipLifecycleClaim,
        attempt: &UsbipReconcileAttemptContext,
    ) -> Result<(), UsbipLifecycleStepError>;
}

pub fn cleanup_usbip_vm_stop_claims<E: UsbipVmStopCarrierCleanup>(
    claims: &[UsbipLifecycleClaim],
    attempt: &UsbipReconcileAttemptContext,
    executor: &mut E,
) -> UsbipLifecycleReconcileReport {
    let mut reports = Vec::with_capacity(claims.len());
    for claim in claims {
        let mut report = UsbipLifecycleClaimReport::new(claim);
        match executor.detach_guest_import(claim, attempt) {
            Ok(()) => report.completed.push(UsbipLifecycleStep::GuestDetach),
            Err(error) => report.push_degraded(&error, false),
        }

        let flow_observation = match executor.observe_proxy_flow_for_cleanup(claim, attempt) {
            Ok(observation) => observation,
            Err(error) => {
                report.push_degraded(&error, false);
                report
                    .completed
                    .push(UsbipLifecycleStep::PreserveDurableClaim);
                reports.push(report);
                continue;
            }
        };
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::VmStopOrRestart,
            flow_observation,
        );
        if !plan.preserves_durable_claim_on_success || plan.releases_durable_claim_on_success {
            report.push_degraded(
                &UsbipLifecycleStepError::new(
                    UsbipLifecycleFailureKind::HostReplayFailed,
                    "carrier cleanup plan does not preserve the session claim",
                ),
                claim.required,
            );
            reports.push(report);
            continue;
        }
        if let Some(reason) = plan.fail_closed_reason {
            report.push_degraded(
                &UsbipLifecycleStepError::new(
                    UsbipLifecycleFailureKind::HostReplayFailed,
                    format!(
                        "carrier cleanup refused before sysfs usbip-host unbind: {}; {}",
                        reason.summary(),
                        reason.remediation()
                    ),
                ),
                false,
            );
            report
                .completed
                .push(UsbipLifecycleStep::PreserveDurableClaim);
        } else {
            match executor.cleanup_host_carrier_preserve_claim(claim, attempt) {
                Ok(()) => {
                    report
                        .completed
                        .push(UsbipLifecycleStep::HostCarrierCleanup);
                    report
                        .completed
                        .push(UsbipLifecycleStep::PreserveDurableClaim);
                }
                Err(error) => {
                    report.push_degraded(&error, false);
                    report
                        .completed
                        .push(UsbipLifecycleStep::PreserveDurableClaim);
                }
            }
        }
        match executor.reconcile_proxy(claim, attempt) {
            Ok(()) => report.completed.push(UsbipLifecycleStep::ProxyReconcile),
            Err(error) => report.push_degraded(&error, false),
        }
        reports.push(report);
    }
    UsbipLifecycleReconcileReport { claims: reports }
}

/// Complete daemon-internal row used by the reconciler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipReconciliationState {
    /// Bundle declaration.
    pub declared: UsbipDeclaredIntent,
    /// Physical observation.
    pub physical: UsbipPhysicalPresence,
    /// Existing broker-mediated USB device claim; not a daemon DAG lock.
    pub lock: UsbipPersistedLockClaim,
    /// Host runtime state.
    pub host: UsbipHostRuntimeState,
    /// Guest runtime state.
    pub guest: UsbipGuestRuntimeState,
}

impl UsbipReconciliationState {
    /// Classify current degraded reasons. Empty means converged/no-op.
    pub fn degraded_reasons(&self) -> Vec<UsbipDegradedReason> {
        let mut reasons = Vec::new();
        reasons.extend(
            self.declared
                .policy_failures
                .iter()
                .cloned()
                .map(UsbipDegradedReason::PolicyFailed),
        );
        push_physical_degraded_reason(&self.physical, &mut reasons);

        match self.declared.desired {
            UsbipDesiredClaimState::Desired => {
                push_lock_degraded_reason(&self.lock, &mut reasons);
                push_desired_host_degraded_reasons(&self.host, &mut reasons);
                push_desired_guest_degraded_reason(&self.guest, &mut reasons);
            }
            UsbipDesiredClaimState::Undeclared | UsbipDesiredClaimState::Releasing => {
                if host_has_active_state(&self.host) {
                    reasons.push(UsbipDegradedReason::StaleHostState);
                }
                if guest_has_active_state(&self.guest) {
                    reasons.push(UsbipDegradedReason::StaleGuestState);
                }
            }
        }

        reasons
    }

    /// Redacted projection suitable for future public/status DTOs.
    pub fn to_public_status(&self, claim_ref: String) -> UsbipPublicStatus {
        let reasons = self.degraded_reasons();
        let posture = if reasons.is_empty() {
            UsbipPublicPosture::Known(KnownUsbipPublicPosture::Converged)
        } else {
            UsbipPublicPosture::Known(KnownUsbipPublicPosture::Degraded)
        };
        UsbipPublicStatus {
            claim_ref: scrub_public_claim_ref(&claim_ref),
            desired: UsbipPublicDesiredClaimState::Known(match self.declared.desired {
                UsbipDesiredClaimState::Undeclared => KnownUsbipPublicDesiredClaimState::Undeclared,
                UsbipDesiredClaimState::Desired => KnownUsbipPublicDesiredClaimState::Desired,
                UsbipDesiredClaimState::Releasing => KnownUsbipPublicDesiredClaimState::Releasing,
            }),
            posture,
            device: public_identity_from_presence(&self.physical),
            degraded_reason_count: reasons.len(),
            degraded_reasons: reasons
                .iter()
                .map(UsbipDegradedReason::to_public_reason)
                .collect(),
        }
    }

    pub fn to_degraded_events(&self, claim_ref: String) -> Vec<UsbipDegradedEvent> {
        self.to_degraded_events_for_attempt(
            claim_ref,
            UsbipEventSource::component(UsbipEventSourceKind::Reconciler),
            None,
        )
    }

    pub fn to_degraded_events_for_attempt(
        &self,
        claim_ref: String,
        source: UsbipEventSource,
        attempt: Option<&UsbipReconcileAttemptContext>,
    ) -> Vec<UsbipDegradedEvent> {
        let claim_ref = scrub_public_claim_ref(&claim_ref);
        self.degraded_reasons()
            .iter()
            .map(|reason| {
                let labels = reason.telemetry_labels();
                UsbipDegradedEvent {
                    event_type: UsbipEventType::Degraded,
                    source: source.clone(),
                    correlation_id: attempt.map(|attempt| attempt.correlation_id.clone()),
                    claim_ref: claim_ref.clone(),
                    reason: reason.to_public_reason(),
                    telemetry_reason: labels.reason.to_owned(),
                    telemetry_policy: labels.policy.to_owned(),
                }
            })
            .collect()
    }

    pub fn to_dedupe_keys(
        &self,
        claim_ref: String,
        source: UsbipEventSource,
    ) -> Vec<UsbipDegradedDedupeKey> {
        let claim_ref = normalize_dedupe_claim_ref(&claim_ref);
        self.degraded_reasons()
            .iter()
            .map(|reason| UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Degraded,
                source: source.clone(),
                claim_ref: claim_ref.clone(),
                reason: reason.code(),
                policy_failure: match reason {
                    UsbipDegradedReason::PolicyFailed(policy) => Some(*policy),
                    _ => None,
                },
            })
            .collect()
    }
}

fn push_physical_degraded_reason(
    physical: &UsbipPhysicalPresence,
    reasons: &mut Vec<UsbipDegradedReason>,
) {
    match physical {
        UsbipPhysicalPresence::Present { identity } if !identity.is_reconciliation_safe() => {
            reasons.push(UsbipDegradedReason::ProbeIncomplete);
        }
        UsbipPhysicalPresence::Present { .. } => {}
        UsbipPhysicalPresence::DepartedBeforeClaim => {
            reasons.push(UsbipDegradedReason::DeviceDepartedBeforeClaim);
        }
        UsbipPhysicalPresence::DepartedAfterLock => {
            reasons.push(UsbipDegradedReason::DeviceDepartedAfterLock);
        }
        UsbipPhysicalPresence::DepartedDuringMutation => {
            reasons.push(UsbipDegradedReason::DeviceDepartedDuringMutation);
        }
        UsbipPhysicalPresence::ReappearedDifferentTopology { .. } => {
            reasons.push(UsbipDegradedReason::DeviceReappearedWithDifferentTopology);
        }
        UsbipPhysicalPresence::Ambiguous { .. } | UsbipPhysicalPresence::ProbeIncomplete => {
            reasons.push(UsbipDegradedReason::ProbeIncomplete);
        }
    }
}

fn push_lock_degraded_reason(
    lock: &UsbipPersistedLockClaim,
    reasons: &mut Vec<UsbipDegradedReason>,
) {
    match lock.state {
        UsbipPersistedLockClaimState::Missing => {
            reasons.push(UsbipDegradedReason::InvalidPersistedLockClaim);
        }
        UsbipPersistedLockClaimState::HeldByDesiredOwner => {}
        UsbipPersistedLockClaimState::HeldByOtherOwner => {
            reasons.push(UsbipDegradedReason::LockHeldByOtherOwner);
        }
        UsbipPersistedLockClaimState::StaleOwner | UsbipPersistedLockClaimState::Corrupt => {
            reasons.push(UsbipDegradedReason::InvalidPersistedLockClaim);
        }
    }
}

fn push_desired_host_degraded_reasons(
    host: &UsbipHostRuntimeState,
    reasons: &mut Vec<UsbipDegradedReason>,
) {
    match host.carrier {
        UsbipActiveCarrierState::Ready | UsbipActiveCarrierState::WithheldForOwner => {}
        UsbipActiveCarrierState::DepartedDuringProbe => {
            reasons.push(UsbipDegradedReason::DeviceDepartedDuringMutation);
        }
        UsbipActiveCarrierState::Absent | UsbipActiveCarrierState::Unavailable => {
            reasons.push(UsbipDegradedReason::CarrierUnavailable);
        }
    }
    match host.bind {
        UsbipHostBindState::BoundToUsbipHost => {}
        UsbipHostBindState::DepartedDuringBind => {
            reasons.push(UsbipDegradedReason::DeviceDepartedDuringMutation);
        }
        UsbipHostBindState::Unbound
        | UsbipHostBindState::Binding
        | UsbipHostBindState::BoundToUnexpectedDriver
        | UsbipHostBindState::Unbinding => {
            reasons.push(UsbipDegradedReason::HostBindUnavailable);
        }
    }
    match host.proxy {
        UsbipProxyState::Listening => {}
        UsbipProxyState::NotDeclared
        | UsbipProxyState::Stopped
        | UsbipProxyState::Starting
        | UsbipProxyState::Stale
        | UsbipProxyState::Failed => {
            reasons.push(UsbipDegradedReason::ProxyUnavailable);
        }
    }
}

fn push_desired_guest_degraded_reason(
    guest: &UsbipGuestRuntimeState,
    reasons: &mut Vec<UsbipDegradedReason>,
) {
    match guest.import {
        UsbipGuestImportState::Imported => {}
        UsbipGuestImportState::NotRequested
        | UsbipGuestImportState::Importing
        | UsbipGuestImportState::Detaching
        | UsbipGuestImportState::Detached
        | UsbipGuestImportState::Failed
        | UsbipGuestImportState::Stale
        | UsbipGuestImportState::Unknown => {
            reasons.push(UsbipDegradedReason::GuestImportUnavailable);
        }
    }
}

fn host_has_active_state(host: &UsbipHostRuntimeState) -> bool {
    host.carrier != UsbipActiveCarrierState::Absent
        || !matches!(host.bind, UsbipHostBindState::Unbound)
        || !matches!(
            host.proxy,
            UsbipProxyState::NotDeclared | UsbipProxyState::Stopped
        )
}

fn guest_has_active_state(guest: &UsbipGuestRuntimeState) -> bool {
    matches!(
        guest.import,
        UsbipGuestImportState::Importing
            | UsbipGuestImportState::Imported
            | UsbipGuestImportState::Detaching
            | UsbipGuestImportState::Failed
            | UsbipGuestImportState::Stale
            | UsbipGuestImportState::Unknown
    )
}

fn public_identity_from_presence(physical: &UsbipPhysicalPresence) -> UsbipPublicDeviceIdentity {
    match physical {
        UsbipPhysicalPresence::Present { identity }
        | UsbipPhysicalPresence::ReappearedDifferentTopology { observed: identity } => {
            identity.to_public_identity()
        }
        UsbipPhysicalPresence::Ambiguous { .. } => UsbipPublicDeviceIdentity {
            vendor_id: None,
            product_id: None,
            topology_anchor: UsbipPublicTopologyAnchor::Ambiguous,
            serial_observed: false,
        },
        UsbipPhysicalPresence::DepartedBeforeClaim
        | UsbipPhysicalPresence::DepartedAfterLock
        | UsbipPhysicalPresence::DepartedDuringMutation => UsbipPublicDeviceIdentity {
            vendor_id: None,
            product_id: None,
            topology_anchor: UsbipPublicTopologyAnchor::Departed,
            serial_observed: false,
        },
        UsbipPhysicalPresence::ProbeIncomplete => UsbipPublicDeviceIdentity {
            vendor_id: None,
            product_id: None,
            topology_anchor: UsbipPublicTopologyAnchor::Missing,
            serial_observed: false,
        },
    }
}

/// Redacted public topology anchor. Does not expose sysfs, bus, port chain, or serial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipPublicTopologyAnchor {
    /// Physical anchor was observed internally.
    Observed,
    /// No reliable physical anchor was observed.
    Missing,
    /// Device departed before status was produced.
    Departed,
    /// Multiple candidates matched.
    Ambiguous,
}

/// Redacted device identity for future public/status DTOs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipPublicDeviceIdentity {
    /// USB vendor id; not unique enough to identify a physical device.
    pub vendor_id: Option<String>,
    /// USB product id; not unique enough to identify a physical device.
    pub product_id: Option<String>,
    /// Coarse topology state only.
    pub topology_anchor: UsbipPublicTopologyAnchor,
    /// Whether an internal serial-like value was seen, without revealing it.
    pub serial_observed: bool,
}

/// Known public desired-claim strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnownUsbipPublicDesiredClaimState {
    /// No public claim is desired.
    Undeclared,
    /// Device is desired for an owner.
    Desired,
    /// Device is draining/releasing.
    Releasing,
}

/// Forward-compatible public desired state.
///
/// Do not use `#[serde(other)]` for public round-tripped enums: unknown strings
/// must survive deserialize/serialize so newer daemons can talk to older tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UsbipPublicDesiredClaimState {
    /// Known state understood by this daemon.
    Known(KnownUsbipPublicDesiredClaimState),
    /// Unknown state preserved byte-for-byte by older clients.
    Unknown(String),
}

/// Known public reconciliation posture strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnownUsbipPublicPosture {
    /// All desired host and guest state has converged.
    Converged,
    /// Host side is still reconciling.
    PendingHost,
    /// Guest import is still reconciling.
    PendingGuest,
    /// Device is detached/no-op.
    Detached,
    /// One or more degraded reasons exist.
    Degraded,
}

/// Forward-compatible public posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UsbipPublicPosture {
    /// Known posture understood by this daemon.
    Known(KnownUsbipPublicPosture),
    /// Unknown posture preserved byte-for-byte by older clients.
    Unknown(String),
}

/// Redacted public/status DTO strategy for future wire promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipPublicStatus {
    /// Opaque public row id; not the raw busid/sysfs path/serial.
    pub claim_ref: String,
    /// Forward-compatible desired state.
    pub desired: UsbipPublicDesiredClaimState,
    /// Forward-compatible posture.
    pub posture: UsbipPublicPosture,
    /// Redacted device identity.
    pub device: UsbipPublicDeviceIdentity,
    /// Count only; detailed internal reasons may include sensitive topology.
    pub degraded_reason_count: usize,
    /// Structured, redacted reasons and user-visible remediation. No bus IDs,
    /// sysfs paths, serials, command output, trace IDs, or raw stderr.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_reasons: Vec<UsbipPublicDegradedReason>,
}

fn scrub_usb_hex_id(value: Option<&str>) -> Option<String> {
    nixling_contracts::usbip::sanitize_usb_hex_id(value)
}

fn scrub_public_claim_ref(value: &str) -> String {
    let valid_opaque = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');

    if valid_opaque
        && !looks_like_trace_id(value)
        && nixling_contracts::usbip::validate_bus_id(value).is_err()
    {
        value.to_owned()
    } else {
        "claim-redacted".to_owned()
    }
}

const USBIP_DEDUPE_CLAIM_REF_MAX_LEN: usize = 128;

fn normalize_dedupe_claim_ref(value: &str) -> String {
    if !value.is_empty() && value.len() <= USBIP_DEDUPE_CLAIM_REF_MAX_LEN {
        value.to_owned()
    } else {
        "claim-redacted".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safe_identity() -> UsbPhysicalTopologyIdentity {
        UsbPhysicalTopologyIdentity {
            vid_pid: UsbVidPid {
                vendor_id: Some("1050".to_owned()),
                product_id: Some("0407".to_owned()),
            },
            bus_number: Some(1),
            port_chain: vec![2, 4],
            sysfs_path: Some(PathBuf::from("/sys/devices/pci0000:00/usb1/1-2/1-2.4")),
            serial_like: Some("serial-redacted-in-public".to_owned()),
        }
    }

    fn converged_state() -> UsbipReconciliationState {
        UsbipReconciliationState {
            declared: UsbipDeclaredIntent {
                desired: UsbipDesiredClaimState::Desired,
                env: Some("work".to_owned()),
                vm: Some("corp".to_owned()),
                bus_id: Some("1-2.4".to_owned()),
                topology: Some(safe_identity()),
                policy_failures: Vec::new(),
            },
            physical: UsbipPhysicalPresence::Present {
                identity: safe_identity(),
            },
            lock: UsbipPersistedLockClaim {
                state: UsbipPersistedLockClaimState::HeldByDesiredOwner,
                env: Some("work".to_owned()),
                vm: Some("corp".to_owned()),
                generation: Some(7),
            },
            host: UsbipHostRuntimeState {
                carrier: UsbipActiveCarrierState::Ready,
                bind: UsbipHostBindState::BoundToUsbipHost,
                proxy: UsbipProxyState::Listening,
            },
            guest: UsbipGuestRuntimeState {
                import: UsbipGuestImportState::Imported,
                generation: Some(7),
            },
        }
    }

    #[test]
    fn complete_identity_requires_vid_pid_and_physical_anchor() {
        let mut identity = safe_identity();
        assert!(identity.is_reconciliation_safe());

        identity.bus_number = None;
        identity.port_chain.clear();
        identity.sysfs_path = None;
        assert!(!identity.has_physical_anchor());
        assert!(!identity.is_reconciliation_safe());

        identity.serial_like = Some("serial-alone-is-not-enough".to_owned());
        assert!(!identity.is_reconciliation_safe());

        identity.bus_number = Some(1);
        assert!(identity.has_physical_anchor());
        assert!(!identity.has_spoof_resistant_anchor());
        assert!(!identity.is_reconciliation_safe());
    }

    #[test]
    fn parses_busid_path_and_bbus_like_fields_into_topology() {
        let parsed = parse_usb_busid_topology("10-3.2").expect("busid parses");
        assert_eq!(parsed.bus_number, 10);
        assert_eq!(parsed.port_chain, vec![3, 2]);
        assert_eq!(parsed.canonical_bus_id, "10-3.2");

        let from_path = parse_usb_topology_from_sysfs_path(Path::new(
            "/sys/devices/pci0000:00/usb10/10-3/10-3.2/10-3.2:1.0",
        ))
        .expect("sysfs path parses");
        assert_eq!(from_path, parsed);

        let identity = identity_from_sysfs_fields(UsbSysfsPhysicalFields {
            sysfs_path: Some(Path::new(
                "/sys/devices/pci0000:00/usb10/10-3/10-3.2/10-3.2:1.0",
            )),
            bus_id: Some("10-3.2\n"),
            id_vendor: Some("1050\n"),
            id_product: Some("0407\n"),
            busnum: Some("10\n"),
            devpath: Some("3.2\n"),
            port_number: Some("2\n"),
            serial: Some("supplemental-serial\n"),
        })
        .expect("identity parses");

        assert_eq!(identity.vid_pid.vendor_id.as_deref(), Some("1050"));
        assert_eq!(identity.vid_pid.product_id.as_deref(), Some("0407"));
        assert_eq!(identity.bus_number, Some(10));
        assert_eq!(identity.port_chain, vec![3, 2]);
        assert_eq!(
            identity.topology_class(),
            UsbPhysicalTopologyClass::SysfsPathAndPortChain
        );
        assert_eq!(identity.serial_like.as_deref(), Some("supplemental-serial"));
        assert!(identity.is_reconciliation_safe());
    }

    #[test]
    fn rejects_conflicting_or_malformed_topology_fields() {
        let err = identity_from_sysfs_fields(UsbSysfsPhysicalFields {
            sysfs_path: None,
            bus_id: Some("1-2.4"),
            id_vendor: Some("1050"),
            id_product: Some("0407"),
            busnum: Some("2"),
            devpath: Some("2.4"),
            port_number: Some("4"),
            serial: None,
        })
        .expect_err("conflicting bus rejected");
        assert_eq!(
            err,
            UsbTopologyParseError::ConflictingBusNumber { left: 1, right: 2 }
        );

        let err = identity_from_sysfs_fields(UsbSysfsPhysicalFields {
            sysfs_path: None,
            bus_id: Some("1-2.4"),
            id_vendor: Some("1050"),
            id_product: Some("0407"),
            busnum: Some("1"),
            devpath: Some("2.5"),
            port_number: Some("5"),
            serial: None,
        })
        .expect_err("conflicting port chain rejected");
        assert_eq!(
            err,
            UsbTopologyParseError::ConflictingPortChain {
                left: vec![2, 4],
                right: vec![2, 5],
            }
        );

        assert!(matches!(
            parse_usb_busid_topology("01-2"),
            Err(UsbTopologyParseError::InvalidBusId(_))
        ));
        assert!(matches!(
            identity_from_sysfs_fields(UsbSysfsPhysicalFields {
                id_vendor: Some("yubi"),
                id_product: Some("0407"),
                ..UsbSysfsPhysicalFields::default()
            }),
            Err(UsbTopologyParseError::InvalidVendorProduct {
                field: "idVendor",
                ..
            })
        ));
    }

    #[test]
    fn durable_claim_match_requires_vid_pid_and_topology_not_serial() {
        let declared = safe_identity();
        let mut observed = safe_identity();
        observed.serial_like = Some("attacker-controlled-different-serial".to_owned());
        assert_eq!(
            declared.durable_claim_match(&observed),
            UsbDurableClaimMatch::Match
        );

        observed.vid_pid.product_id = Some("9999".to_owned());
        assert_eq!(
            declared.durable_claim_match(&observed),
            UsbDurableClaimMatch::VendorProductMismatch
        );

        observed = safe_identity();
        observed.port_chain = vec![2, 5];
        assert_eq!(
            declared.durable_claim_match(&observed),
            UsbDurableClaimMatch::PhysicalTopologyMismatch
        );

        let mut serial_only_declared = declared.clone();
        serial_only_declared.bus_number = None;
        serial_only_declared.port_chain.clear();
        serial_only_declared.sysfs_path = None;
        serial_only_declared.serial_like = Some("same-serial".to_owned());
        observed = safe_identity();
        observed.serial_like = Some("same-serial".to_owned());
        assert_eq!(
            serial_only_declared.durable_claim_match(&observed),
            UsbDurableClaimMatch::MissingDeclaredPhysicalAnchor
        );

        let mut bus_only_declared = declared;
        bus_only_declared.port_chain.clear();
        bus_only_declared.sysfs_path = None;
        assert_eq!(
            bus_only_declared.topology_class(),
            UsbPhysicalTopologyClass::BusOnly
        );
        assert_eq!(
            bus_only_declared.durable_claim_match(&observed),
            UsbDurableClaimMatch::MissingDeclaredPhysicalAnchor
        );
    }

    #[test]
    fn sysfs_helpers_trim_newlines_and_classify_departure_races() {
        assert_eq!(trim_sysfs_value("1050\n\n"), "1050");
        assert_eq!(trim_sysfs_value("0407\r\n"), "0407");
        assert_eq!(
            trim_sysfs_value("serial with spaces \n"),
            "serial with spaces "
        );

        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            classify_usb_sysfs_read_error(&not_found),
            UsbSysfsAttrReadFailure::NotFound
        );

        let departed = std::io::Error::from_raw_os_error(libc::ENODEV);
        assert_eq!(
            classify_usb_sysfs_read_error(&departed),
            UsbSysfsAttrReadFailure::DepartedDevice
        );
    }

    #[test]
    fn converged_desired_claim_has_no_degraded_reasons() {
        let state = converged_state();
        assert_eq!(state.degraded_reasons(), Vec::new());
        let public = state.to_public_status("claim-7".to_owned());
        assert_eq!(
            public.posture,
            UsbipPublicPosture::Known(KnownUsbipPublicPosture::Converged)
        );
        assert_eq!(public.degraded_reason_count, 0);
        assert!(public.degraded_reasons.is_empty());
    }

    #[test]
    fn proxy_refresh_plan_preserves_same_env_streams() {
        let plan = plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::RefreshExport);

        assert_eq!(
            plan.actions,
            vec![
                UsbipProxySynchronizationAction::OptimisticBackendExportRefresh,
                UsbipProxySynchronizationAction::EnsureProxyListening,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert!(!plan.may_bounce_same_env_streams);
        assert!(!plan.claims_selective_busid_proxy_closure);
        assert!(
            !plan
                .actions
                .contains(&UsbipProxySynchronizationAction::RebindProxyListenerFdRelative)
        );
    }

    #[test]
    fn generic_l4_release_without_targeted_cleanup_fails_closed() {
        let plan =
            plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::ReleaseBusid {
                targeted_cleanup: UsbipTargetedProxyCleanup::Unavailable,
            });

        assert_eq!(
            plan.actions,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated,
                UsbipProxySynchronizationAction::PreserveBusidLockForManualDrain,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert!(!plan.may_bounce_same_env_streams);
        assert!(!plan.claims_selective_busid_proxy_closure);
        assert!(
            !plan
                .actions
                .contains(&UsbipProxySynchronizationAction::TargetedConntrackOrSocketKill)
        );
        assert!(
            !plan
                .actions
                .contains(&UsbipProxySynchronizationAction::HostUnbind)
        );
    }

    #[test]
    fn targeted_release_never_rebinds_the_per_env_proxy() {
        let plan =
            plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::ReleaseBusid {
                targeted_cleanup: UsbipTargetedProxyCleanup::ConntrackOrSocketTuple,
            });

        assert!(plan.claims_selective_busid_proxy_closure);
        assert!(!plan.may_bounce_same_env_streams);
        assert!(
            plan.actions
                .contains(&UsbipProxySynchronizationAction::TargetedConntrackOrSocketKill)
        );
        assert!(
            plan.actions
                .contains(&UsbipProxySynchronizationAction::PreserveSameEnvStreams)
        );
        let target = plan
            .actions
            .iter()
            .position(|action| {
                *action == UsbipProxySynchronizationAction::TargetedConntrackOrSocketKill
            })
            .expect("targeted cleanup action");
        let host_unbind = plan
            .actions
            .iter()
            .position(|action| *action == UsbipProxySynchronizationAction::HostUnbind)
            .expect("host unbind after targeted cleanup");
        assert!(target < host_unbind);
        assert!(
            !plan
                .actions
                .contains(&UsbipProxySynchronizationAction::RebindProxyListenerFdRelative)
        );
    }

    fn flow_tuple(protocol: UsbipProxyFlowProtocol) -> UsbipProxyFlowTuple {
        UsbipProxyFlowTuple {
            protocol,
            vm_addr: "10.42.0.20".parse().unwrap(),
            vm_port: 49152,
            proxy_addr: "10.42.0.1".parse().unwrap(),
            proxy_port: 3240,
        }
    }

    #[derive(Default)]
    struct RevocationFixtureExecutor {
        calls: Vec<UsbipProxySynchronizationAction>,
        tuples: Vec<UsbipProxyFlowTuple>,
        fail_at: Option<(UsbipProxySynchronizationAction, &'static str)>,
    }

    impl RevocationFixtureExecutor {
        fn failing(action: UsbipProxySynchronizationAction, reason: &'static str) -> Self {
            Self {
                calls: Vec::new(),
                tuples: Vec::new(),
                fail_at: Some((action, reason)),
            }
        }

        fn dispatch(
            &mut self,
            action: UsbipProxySynchronizationAction,
            tuple: Option<&UsbipProxyFlowTuple>,
        ) -> Result<(), String> {
            self.calls.push(action);
            if let Some(tuple) = tuple {
                self.tuples.push(tuple.clone());
            }
            if let Some((target, reason)) = self.fail_at
                && target == action
            {
                return Err(reason.to_owned());
            }
            Ok(())
        }
    }

    impl UsbipRevocationFlowExecutor for RevocationFixtureExecutor {
        fn withdraw_firewall_carveout(
            &mut self,
            _: &UsbipProxyFlowObservation,
        ) -> Result<(), String> {
            self.dispatch(
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                None,
            )
        }

        fn delete_conntrack_tuple(&mut self, tuple: &UsbipProxyFlowTuple) -> Result<(), String> {
            self.dispatch(
                UsbipProxySynchronizationAction::TargetedConntrackDelete,
                Some(tuple),
            )
        }

        fn kill_tcp_established_socket(
            &mut self,
            tuple: &UsbipProxyFlowTuple,
        ) -> Result<(), String> {
            self.dispatch(
                UsbipProxySynchronizationAction::TargetedTcpEstablishedSocketKill,
                Some(tuple),
            )
        }
    }

    #[test]
    fn revocation_flow_cleanup_withdraws_firewall_before_killing_flows() {
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            });

        assert_eq!(
            plan.actions[0],
            UsbipProxySynchronizationAction::WithdrawFirewallCarveout
        );
        let firewall_idx = plan
            .actions
            .iter()
            .position(|action| *action == UsbipProxySynchronizationAction::WithdrawFirewallCarveout)
            .unwrap();
        let conntrack_idx = plan
            .actions
            .iter()
            .position(|action| *action == UsbipProxySynchronizationAction::TargetedConntrackDelete)
            .unwrap();
        let socket_idx = plan
            .actions
            .iter()
            .position(|action| {
                *action == UsbipProxySynchronizationAction::TargetedTcpEstablishedSocketKill
            })
            .unwrap();
        assert!(firewall_idx < conntrack_idx);
        assert!(firewall_idx < socket_idx);
        assert_eq!(
            plan.mechanisms,
            vec![
                UsbipTargetedTerminationMechanism::ConntrackDelete,
                UsbipTargetedTerminationMechanism::TcpSocketKill,
            ]
        );
        assert!(plan.fail_closed_reason.is_none());
        assert!(!plan.may_bounce_same_env_streams);
        assert!(
            !plan
                .actions
                .contains(&UsbipProxySynchronizationAction::RebindProxyListenerFdRelative)
        );
    }

    #[test]
    fn revocation_executor_applies_firewall_before_targeted_cleanup() {
        let tuple = flow_tuple(UsbipProxyFlowProtocol::Tcp);
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::ExactEstablished {
                tuple: tuple.clone(),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            });
        let mut executor = RevocationFixtureExecutor::default();

        let report = execute_usbip_revocation_flow_termination(&plan, &mut executor)
            .expect("exact tuple with both cleanup mechanisms should succeed");

        assert_eq!(report.failed, None);
        assert_eq!(
            &executor.calls[..3],
            &[
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::TargetedConntrackDelete,
                UsbipProxySynchronizationAction::TargetedTcpEstablishedSocketKill,
            ]
        );
        assert_eq!(executor.tuples, vec![tuple.clone(), tuple]);
    }

    #[test]
    fn revocation_executor_refuses_unisolated_stream_after_firewall_withdrawal() {
        let plan = plan_usbip_revocation_flow_termination(
            UsbipProxyFlowObservation::SharedListeningSocket {
                protocol: UsbipProxyFlowProtocol::Tcp,
            },
        );
        let mut executor = RevocationFixtureExecutor::default();

        let (report, err) = execute_usbip_revocation_flow_termination(&plan, &mut executor)
            .expect_err("shared listener must fail closed");

        assert_eq!(
            executor.calls,
            vec![UsbipProxySynchronizationAction::WithdrawFirewallCarveout]
        );
        assert_eq!(
            report.completed,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::RefuseSharedSocketKill,
            ]
        );
        assert_eq!(
            report.failed.as_ref().map(|(action, _)| *action),
            Some(UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated)
        );
        assert_eq!(
            err,
            UsbipRevocationFlowExecutionError::NotIsolated {
                reason: UsbipRevocationFlowFailure::SharedListeningSocket,
            }
        );
    }

    #[test]
    fn revocation_executor_never_kills_flows_if_firewall_withdrawal_fails() {
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            });
        let mut executor = RevocationFixtureExecutor::failing(
            UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
            "nft refused",
        );

        let (report, err) = execute_usbip_revocation_flow_termination(&plan, &mut executor)
            .expect_err("firewall withdrawal failure must halt");

        assert_eq!(
            executor.calls,
            vec![UsbipProxySynchronizationAction::WithdrawFirewallCarveout]
        );
        assert!(executor.tuples.is_empty());
        assert!(report.completed.is_empty());
        assert_eq!(
            report.failed.as_ref().map(|(action, _)| *action),
            Some(UsbipProxySynchronizationAction::WithdrawFirewallCarveout)
        );
        assert_eq!(
            err,
            UsbipRevocationFlowExecutionError::ActionFailed {
                action: UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                reason: "nft refused".to_owned(),
            }
        );
    }

    #[test]
    fn revocation_flow_cleanup_branches_udp_to_conntrack_only() {
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Udp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            });

        assert_eq!(
            plan.actions[0],
            UsbipProxySynchronizationAction::WithdrawFirewallCarveout
        );
        assert!(
            plan.actions
                .contains(&UsbipProxySynchronizationAction::TargetedConntrackDelete)
        );
        assert!(
            plan.actions
                .contains(&UsbipProxySynchronizationAction::SkipTcpSocketKillForUdp)
        );
        assert!(
            !plan
                .actions
                .contains(&UsbipProxySynchronizationAction::TargetedTcpEstablishedSocketKill),
            "UDP cleanup must not use the TCP socket-kill path"
        );
        assert_eq!(
            plan.mechanisms,
            vec![UsbipTargetedTerminationMechanism::ConntrackDelete]
        );
        assert!(plan.fail_closed_reason.is_none());
    }

    #[test]
    fn revocation_executor_does_not_run_tcp_socket_kill_for_udp() {
        let tuple = flow_tuple(UsbipProxyFlowProtocol::Udp);
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::ExactEstablished {
                tuple: tuple.clone(),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            });
        let mut executor = RevocationFixtureExecutor::default();

        let report = execute_usbip_revocation_flow_termination(&plan, &mut executor)
            .expect("UDP with conntrack delete should succeed without socket kill");

        assert_eq!(report.failed, None);
        assert_eq!(
            report.completed,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::TargetedConntrackDelete,
                UsbipProxySynchronizationAction::SkipTcpSocketKillForUdp,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert_eq!(
            executor.calls,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::TargetedConntrackDelete,
            ],
            "executor must not dispatch the TCP established-socket kill for UDP"
        );
        assert_eq!(executor.tuples, vec![tuple]);
    }

    #[test]
    fn revocation_flow_cleanup_refuses_shared_listener_kill() {
        let plan = plan_usbip_revocation_flow_termination(
            UsbipProxyFlowObservation::SharedListeningSocket {
                protocol: UsbipProxyFlowProtocol::Tcp,
            },
        );

        assert_eq!(
            plan.actions,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::RefuseSharedSocketKill,
                UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated,
                UsbipProxySynchronizationAction::PreserveBusidLockForManualDrain,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert_eq!(
            plan.fail_closed_reason,
            Some(UsbipRevocationFlowFailure::SharedListeningSocket)
        );
        assert!(plan.is_actionable_failure());
        assert!(plan.mechanisms.is_empty());
        assert!(!plan.may_bounce_same_env_streams);
    }

    #[test]
    fn revocation_flow_cleanup_fails_closed_for_snat_or_unproven_anti_spoofing() {
        for (source_identity, expected_reason) in [
            (
                UsbipProxyFlowSourceIdentity::ObscuredBySnat,
                UsbipRevocationFlowFailure::SourceIdentityObscuredBySnat,
            ),
            (
                UsbipProxyFlowSourceIdentity::AntiSpoofNotProven,
                UsbipRevocationFlowFailure::AntiSpoofNotProven,
            ),
        ] {
            let plan = plan_usbip_revocation_flow_termination(
                UsbipProxyFlowObservation::ExactEstablished {
                    tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                    source_identity,
                    conntrack_delete: true,
                    tcp_socket_kill: true,
                },
            );

            assert_eq!(
                plan.actions,
                vec![
                    UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                    UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated,
                    UsbipProxySynchronizationAction::PreserveBusidLockForManualDrain,
                    UsbipProxySynchronizationAction::PreserveSameEnvStreams,
                ]
            );
            assert_eq!(plan.fail_closed_reason, Some(expected_reason));
            assert!(plan.mechanisms.is_empty());
            assert!(plan.is_actionable_failure());
            assert!(!plan.may_bounce_same_env_streams);

            let mut executor = RevocationFixtureExecutor::default();
            let (report, err) = execute_usbip_revocation_flow_termination(&plan, &mut executor)
                .expect_err("SNAT/spoof-ambiguous tuple must fail closed");

            assert_eq!(
                executor.calls,
                vec![UsbipProxySynchronizationAction::WithdrawFirewallCarveout]
            );
            assert!(executor.tuples.is_empty());
            assert_eq!(
                report.failed.as_ref().map(|(action, _)| *action),
                Some(UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated)
            );
            assert_eq!(
                err,
                UsbipRevocationFlowExecutionError::NotIsolated {
                    reason: expected_reason,
                }
            );
        }
    }

    #[test]
    fn revocation_flow_cleanup_fails_closed_for_ambiguous_same_env_streams() {
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::SharedOrAmbiguous);

        assert_eq!(
            plan.actions,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated,
                UsbipProxySynchronizationAction::PreserveBusidLockForManualDrain,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert_eq!(
            plan.fail_closed_reason,
            Some(UsbipRevocationFlowFailure::AmbiguousSameEnvStreams)
        );
        assert!(plan.is_actionable_failure());
        assert!(plan.mechanisms.is_empty());
        assert!(!plan.may_bounce_same_env_streams);
    }

    #[test]
    fn revocation_flow_cleanup_fails_closed_when_established_stream_not_killable() {
        let plan =
            plan_usbip_revocation_flow_termination(UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: false,
                tcp_socket_kill: false,
            });

        assert_eq!(
            plan.actions,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated,
                UsbipProxySynchronizationAction::PreserveBusidLockForManualDrain,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert_eq!(
            plan.fail_closed_reason,
            Some(UsbipRevocationFlowFailure::CleanupUnsupported)
        );
        assert!(plan.is_actionable_failure());
        assert!(!plan.may_bounce_same_env_streams);
    }

    #[derive(Default)]
    struct VmCarrierCleanupFixtureExecutor {
        calls: Vec<UsbipVmCarrierCleanupAction>,
        fail_at: Option<(UsbipVmCarrierCleanupAction, &'static str)>,
    }

    impl VmCarrierCleanupFixtureExecutor {
        fn failing(action: UsbipVmCarrierCleanupAction, reason: &'static str) -> Self {
            Self {
                calls: Vec::new(),
                fail_at: Some((action, reason)),
            }
        }

        fn dispatch(&mut self, action: UsbipVmCarrierCleanupAction) -> Result<(), String> {
            self.calls.push(action);
            if let Some((target, reason)) = self.fail_at
                && target == action
            {
                return Err(reason.to_owned());
            }
            Ok(())
        }
    }

    impl UsbipVmCarrierCleanupExecutor for VmCarrierCleanupFixtureExecutor {
        fn detach_guest_import(&mut self, _: &UsbipVmCarrierCleanupPlan) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::DetachGuestImport)
        }

        fn withdraw_firewall_carveout(
            &mut self,
            _: &UsbipProxyFlowObservation,
        ) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout)
        }

        fn delete_conntrack_tuple(&mut self, _: &UsbipProxyFlowTuple) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::TargetedConntrackDelete)
        }

        fn kill_tcp_established_socket(&mut self, _: &UsbipProxyFlowTuple) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::TargetedTcpEstablishedSocketKill)
        }

        fn host_unbind(&mut self, _: &UsbipVmCarrierCleanupPlan) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::HostUnbind)
        }

        fn revoke_backend_acl(&mut self, _: &UsbipVmCarrierCleanupPlan) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::RevokeBackendAcl)
        }

        fn release_durable_claim(&mut self, _: &UsbipVmCarrierCleanupPlan) -> Result<(), String> {
            self.dispatch(UsbipVmCarrierCleanupAction::ReleaseDurableClaim)
        }
    }

    #[test]
    fn vm_stop_carrier_cleanup_preserves_durable_claim_after_teardown() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::VmStopOrRestart,
            UsbipProxyFlowObservation::NoEstablishedSession,
        );

        assert_eq!(
            plan.actions,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
                UsbipVmCarrierCleanupAction::PreserveSameEnvStreams,
                UsbipVmCarrierCleanupAction::HostUnbind,
                UsbipVmCarrierCleanupAction::PreserveDurableClaim,
            ]
        );
        assert!(!plan.may_bounce_same_env_streams);
        assert!(plan.preserves_durable_claim_on_success);
        assert!(!plan.releases_durable_claim_on_success);

        let mut executor = VmCarrierCleanupFixtureExecutor::default();
        let report = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect("stop cleanup succeeds without releasing claim");

        assert_eq!(
            executor.calls,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
                UsbipVmCarrierCleanupAction::HostUnbind,
            ]
        );
        assert!(report.failed.is_none());
        assert!(
            !report
                .completed
                .contains(&UsbipVmCarrierCleanupAction::ReleaseDurableClaim)
        );
        assert!(
            !report
                .completed
                .contains(&UsbipVmCarrierCleanupAction::RevokeBackendAcl)
        );
    }

    #[test]
    fn carrier_cleanup_orders_firewall_and_targeted_stream_cleanup_before_host_unbind() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            },
        );

        let expected_prefix = vec![
            UsbipVmCarrierCleanupAction::DetachGuestImport,
            UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
            UsbipVmCarrierCleanupAction::TargetedConntrackDelete,
            UsbipVmCarrierCleanupAction::TargetedTcpEstablishedSocketKill,
            UsbipVmCarrierCleanupAction::PreserveSameEnvStreams,
            UsbipVmCarrierCleanupAction::HostUnbind,
        ];
        assert_eq!(
            &plan.actions[..expected_prefix.len()],
            expected_prefix.as_slice()
        );
        let host_unbind = plan
            .actions
            .iter()
            .position(|action| *action == UsbipVmCarrierCleanupAction::HostUnbind)
            .expect("host unbind is present after targeted cleanup");
        let firewall = plan
            .actions
            .iter()
            .position(|action| *action == UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout)
            .expect("firewall withdrawal is present");
        let tcp_kill = plan
            .actions
            .iter()
            .position(|action| {
                *action == UsbipVmCarrierCleanupAction::TargetedTcpEstablishedSocketKill
            })
            .expect("TCP socket kill is present");
        assert!(firewall < host_unbind);
        assert!(tcp_kill < host_unbind);

        let mut executor = VmCarrierCleanupFixtureExecutor::default();
        let report = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect("targeted cleanup permits host unbind");
        assert_eq!(
            executor.calls,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
                UsbipVmCarrierCleanupAction::TargetedConntrackDelete,
                UsbipVmCarrierCleanupAction::TargetedTcpEstablishedSocketKill,
                UsbipVmCarrierCleanupAction::HostUnbind,
                UsbipVmCarrierCleanupAction::RevokeBackendAcl,
                UsbipVmCarrierCleanupAction::ReleaseDurableClaim,
            ]
        );
        assert!(report.failed.is_none());
    }

    #[test]
    fn carrier_cleanup_refuses_host_unbind_when_targeted_cleanup_cannot_be_guaranteed() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: false,
                tcp_socket_kill: false,
            },
        );

        assert_eq!(
            plan.fail_closed_reason,
            Some(UsbipRevocationFlowFailure::CleanupUnsupported)
        );
        assert!(plan.manual_recovery_on_failure);
        assert!(plan.preserves_durable_claim_on_success);
        assert!(
            !plan
                .actions
                .contains(&UsbipVmCarrierCleanupAction::HostUnbind)
        );
        assert!(
            !plan
                .actions
                .contains(&UsbipVmCarrierCleanupAction::ReleaseDurableClaim)
        );

        let mut executor = VmCarrierCleanupFixtureExecutor::default();
        let (report, err) = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect_err("unsupported targeted cleanup fails before host unbind");
        assert_eq!(
            err,
            UsbipVmCarrierCleanupExecutionError::NotIsolated {
                reason: UsbipRevocationFlowFailure::CleanupUnsupported,
            }
        );
        assert_eq!(
            executor.calls,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
            ]
        );
        assert_eq!(
            report.failed.as_ref().map(|(action, _)| *action),
            Some(UsbipVmCarrierCleanupAction::FailClosedRevocationNotIsolated)
        );
    }

    #[test]
    fn explicit_detach_releases_claim_only_after_successful_teardown() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::NoEstablishedSession,
        );

        assert_eq!(
            plan.actions,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
                UsbipVmCarrierCleanupAction::PreserveSameEnvStreams,
                UsbipVmCarrierCleanupAction::HostUnbind,
                UsbipVmCarrierCleanupAction::RevokeBackendAcl,
                UsbipVmCarrierCleanupAction::ReleaseDurableClaim,
            ]
        );
        assert!(plan.releases_durable_claim_on_success);
        assert!(!plan.preserves_durable_claim_on_success);

        let mut executor = VmCarrierCleanupFixtureExecutor::default();
        let report = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect("explicit detach cleanup succeeds");

        assert_eq!(
            executor.calls,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
                UsbipVmCarrierCleanupAction::HostUnbind,
                UsbipVmCarrierCleanupAction::RevokeBackendAcl,
                UsbipVmCarrierCleanupAction::ReleaseDurableClaim,
            ]
        );
        let host_unbind = report
            .completed
            .iter()
            .position(|action| *action == UsbipVmCarrierCleanupAction::HostUnbind)
            .unwrap();
        let acl_revoke = report
            .completed
            .iter()
            .position(|action| *action == UsbipVmCarrierCleanupAction::RevokeBackendAcl)
            .unwrap();
        let release = report
            .completed
            .iter()
            .position(|action| *action == UsbipVmCarrierCleanupAction::ReleaseDurableClaim)
            .unwrap();
        assert!(host_unbind < acl_revoke);
        assert!(acl_revoke < release);
    }

    #[test]
    fn carrier_cleanup_continues_host_teardown_when_guest_detach_vm_unreachable() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::NoEstablishedSession,
        );
        let mut executor = VmCarrierCleanupFixtureExecutor::failing(
            UsbipVmCarrierCleanupAction::DetachGuestImport,
            "guest-control USBIP import failed for vm 'corp-vm': guest-control transport unavailable",
        );

        let (report, err) = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect_err("unreachable guest detach stays degraded after host cleanup");

        assert_eq!(
            executor.calls,
            vec![
                UsbipVmCarrierCleanupAction::DetachGuestImport,
                UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout,
                UsbipVmCarrierCleanupAction::HostUnbind,
                UsbipVmCarrierCleanupAction::RevokeBackendAcl,
                UsbipVmCarrierCleanupAction::ReleaseDurableClaim,
            ]
        );
        assert_eq!(
            report.failed.as_ref().map(|(action, _)| *action),
            Some(UsbipVmCarrierCleanupAction::DetachGuestImport)
        );
        assert!(
            !report
                .completed
                .contains(&UsbipVmCarrierCleanupAction::DetachGuestImport)
        );
        assert!(
            report
                .completed
                .contains(&UsbipVmCarrierCleanupAction::WithdrawFirewallCarveout)
        );
        assert!(
            report
                .completed
                .contains(&UsbipVmCarrierCleanupAction::HostUnbind)
        );
        assert_eq!(
            err,
            UsbipVmCarrierCleanupExecutionError::ActionFailed {
                action: UsbipVmCarrierCleanupAction::DetachGuestImport,
                reason: "guest-control USBIP import failed for vm 'corp-vm': guest-control transport unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn carrier_cleanup_keeps_guest_detach_command_failure_fatal() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::NoEstablishedSession,
        );
        let mut executor = VmCarrierCleanupFixtureExecutor::failing(
            UsbipVmCarrierCleanupAction::DetachGuestImport,
            "guest usbip command failed",
        );

        let (report, err) = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect_err("guest command failure is not treated as a dead VM");

        assert_eq!(
            executor.calls,
            vec![UsbipVmCarrierCleanupAction::DetachGuestImport]
        );
        assert_eq!(
            report.failed.as_ref().map(|(action, _)| *action),
            Some(UsbipVmCarrierCleanupAction::DetachGuestImport)
        );
        assert_eq!(
            err,
            UsbipVmCarrierCleanupExecutionError::ActionFailed {
                action: UsbipVmCarrierCleanupAction::DetachGuestImport,
                reason: "guest usbip command failed".to_owned(),
            }
        );
    }

    #[test]
    fn carrier_cleanup_failure_preserves_claim_and_surfaces_manual_recovery() {
        let plan = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::ExactEstablished {
                tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                conntrack_delete: true,
                tcp_socket_kill: true,
            },
        );
        let mut executor = VmCarrierCleanupFixtureExecutor::failing(
            UsbipVmCarrierCleanupAction::HostUnbind,
            "kernel unbind helper timed out",
        );

        let (report, err) = execute_usbip_vm_carrier_cleanup(&plan, &mut executor)
            .expect_err("host unbind failure must preserve session claim");

        assert_eq!(
            report.failed.as_ref().map(|(action, _)| *action),
            Some(UsbipVmCarrierCleanupAction::HostUnbind)
        );
        assert_eq!(
            err,
            UsbipVmCarrierCleanupExecutionError::ActionFailed {
                action: UsbipVmCarrierCleanupAction::HostUnbind,
                reason: "kernel unbind helper timed out".to_owned(),
            }
        );
        assert!(
            !executor
                .calls
                .contains(&UsbipVmCarrierCleanupAction::RevokeBackendAcl)
        );
        assert!(
            !executor
                .calls
                .contains(&UsbipVmCarrierCleanupAction::ReleaseDurableClaim)
        );

        let ambiguous = plan_usbip_vm_carrier_cleanup(
            UsbipVmCarrierCleanupMode::ExplicitDetach,
            UsbipProxyFlowObservation::SharedOrAmbiguous,
        );
        assert!(ambiguous.manual_recovery_on_failure);
        assert!(ambiguous.preserves_durable_claim_on_success);
        assert!(
            ambiguous
                .actions
                .contains(&UsbipVmCarrierCleanupAction::SurfaceManualRecovery)
        );
        assert!(
            !ambiguous
                .actions
                .contains(&UsbipVmCarrierCleanupAction::ReleaseDurableClaim)
        );
    }

    #[test]
    fn vm_carrier_cleanup_never_bounces_same_env_sidecars() {
        let plans = [
            plan_usbip_vm_carrier_cleanup(
                UsbipVmCarrierCleanupMode::VmStopOrRestart,
                UsbipProxyFlowObservation::NoEstablishedSession,
            ),
            plan_usbip_vm_carrier_cleanup(
                UsbipVmCarrierCleanupMode::ExplicitDetach,
                UsbipProxyFlowObservation::ExactEstablished {
                    tuple: flow_tuple(UsbipProxyFlowProtocol::Tcp),
                    source_identity: UsbipProxyFlowSourceIdentity::ProvenVmSource,
                    conntrack_delete: true,
                    tcp_socket_kill: true,
                },
            ),
        ];

        for plan in plans {
            assert!(!plan.may_bounce_same_env_streams);
            assert!(
                plan.actions
                    .contains(&UsbipVmCarrierCleanupAction::PreserveSameEnvStreams)
            );
            assert!(!plan.actions.iter().any(|action| matches!(
                action,
                UsbipVmCarrierCleanupAction::RevokeBackendAcl
                    | UsbipVmCarrierCleanupAction::ReleaseDurableClaim
            ) && plan.mode
                == UsbipVmCarrierCleanupMode::VmStopOrRestart));
        }
    }

    struct LifecycleFixtureExecutor {
        calls: Vec<UsbipLifecycleStep>,
        attempts: Vec<String>,
        start_fail: Option<(UsbipLifecycleStep, UsbipLifecycleFailureKind)>,
        stop_fail: Option<(UsbipLifecycleStep, UsbipLifecycleFailureKind)>,
        guest_import_state: UsbipGuestImportState,
        stop_flow_observation: UsbipProxyFlowObservation,
    }

    impl Default for LifecycleFixtureExecutor {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                attempts: Vec::new(),
                start_fail: None,
                stop_fail: None,
                guest_import_state: UsbipGuestImportState::Detached,
                stop_flow_observation: UsbipProxyFlowObservation::NoEstablishedSession,
            }
        }
    }

    impl LifecycleFixtureExecutor {
        fn fail_start(
            step: UsbipLifecycleStep,
            kind: UsbipLifecycleFailureKind,
        ) -> LifecycleFixtureExecutor {
            Self {
                start_fail: Some((step, kind)),
                guest_import_state: UsbipGuestImportState::Detached,
                ..Self::default()
            }
        }

        fn maybe_fail(
            fail: &Option<(UsbipLifecycleStep, UsbipLifecycleFailureKind)>,
            step: UsbipLifecycleStep,
        ) -> Result<(), UsbipLifecycleStepError> {
            if let Some((target, kind)) = fail
                && *target == step
            {
                return Err(UsbipLifecycleStepError::new(
                    kind.clone(),
                    "fixture failure",
                ));
            }
            Ok(())
        }

        fn record_attempt(&mut self, attempt: &UsbipReconcileAttemptContext) {
            self.attempts
                .push(attempt.correlation_id.as_str().to_owned());
        }
    }

    impl UsbipVmStartReconcileExecutor for LifecycleFixtureExecutor {
        fn replay_host_bind(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<(), UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::HostBindReplay);
            Self::maybe_fail(&self.start_fail, UsbipLifecycleStep::HostBindReplay)
        }

        fn ensure_proxy_ready(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<(), UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::ProxyReady);
            Self::maybe_fail(&self.start_fail, UsbipLifecycleStep::ProxyReady)
        }

        fn guest_status(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<UsbipGuestImportState, UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::GuestStatus);
            Self::maybe_fail(&self.start_fail, UsbipLifecycleStep::GuestStatus)?;
            Ok(self.guest_import_state.clone())
        }

        fn guest_import(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<(), UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::GuestImport);
            Self::maybe_fail(&self.start_fail, UsbipLifecycleStep::GuestImport)
        }
    }

    impl UsbipVmStopCarrierCleanup for LifecycleFixtureExecutor {
        fn observe_proxy_flow_for_cleanup(
            &mut self,
            _: &UsbipLifecycleClaim,
            _: &UsbipReconcileAttemptContext,
        ) -> Result<UsbipProxyFlowObservation, UsbipLifecycleStepError> {
            Ok(self.stop_flow_observation.clone())
        }

        fn detach_guest_import(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<(), UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::GuestDetach);
            Self::maybe_fail(&self.stop_fail, UsbipLifecycleStep::GuestDetach)
        }

        fn cleanup_host_carrier_preserve_claim(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<(), UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::HostCarrierCleanup);
            Self::maybe_fail(&self.stop_fail, UsbipLifecycleStep::HostCarrierCleanup)
        }

        fn reconcile_proxy(
            &mut self,
            _: &UsbipLifecycleClaim,
            attempt: &UsbipReconcileAttemptContext,
        ) -> Result<(), UsbipLifecycleStepError> {
            self.record_attempt(attempt);
            self.calls.push(UsbipLifecycleStep::ProxyReconcile);
            Self::maybe_fail(&self.stop_fail, UsbipLifecycleStep::ProxyReconcile)
        }
    }

    fn lifecycle_claim() -> UsbipLifecycleClaim {
        UsbipLifecycleClaim {
            vm: "corp-vm".to_owned(),
            env: "work".to_owned(),
            bus_id: "1-2.3".to_owned(),
            host: "10.77.0.1".to_owned(),
            claim_ref: "usbip-bind:env:work:vm:corp-vm:bus:1-2.3".to_owned(),
            required: true,
        }
    }

    fn lifecycle_attempt() -> UsbipReconcileAttemptContext {
        UsbipReconcileAttemptContext {
            correlation_id: UsbipReconcileCorrelationId::new("usb-lifecycle-7")
                .expect("fixture correlation id is valid"),
        }
    }

    #[test]
    fn lifecycle_start_reattaches_same_vm_claim() {
        let mut executor = LifecycleFixtureExecutor {
            guest_import_state: UsbipGuestImportState::Detached,
            ..LifecycleFixtureExecutor::default()
        };

        let attempt = lifecycle_attempt();
        let report = reconcile_usbip_vm_start_claims(&[lifecycle_claim()], &attempt, &mut executor);

        assert!(!report.fatal());
        assert_eq!(report.degraded_count(), 0);
        assert_eq!(
            executor.calls,
            vec![
                UsbipLifecycleStep::HostBindReplay,
                UsbipLifecycleStep::ProxyReady,
                UsbipLifecycleStep::GuestStatus,
                UsbipLifecycleStep::GuestImport,
            ]
        );
    }

    #[test]
    fn lifecycle_start_missing_device_degrades_without_guest_exposure() {
        let mut executor = LifecycleFixtureExecutor::fail_start(
            UsbipLifecycleStep::HostBindReplay,
            UsbipLifecycleFailureKind::RuntimeAbsent,
        );

        let attempt = lifecycle_attempt();
        let report = reconcile_usbip_vm_start_claims(&[lifecycle_claim()], &attempt, &mut executor);

        assert!(!report.fatal());
        assert_eq!(report.degraded_count(), 1);
        assert_eq!(
            report.claims[0].degraded[0].code,
            UsbipDegradedReasonCode::DeviceDepartedBeforeClaim
        );
        assert_eq!(executor.calls, vec![UsbipLifecycleStep::HostBindReplay]);
    }

    #[test]
    fn lifecycle_start_policy_mismatch_fails_required_claim_before_import() {
        let mut executor = LifecycleFixtureExecutor::fail_start(
            UsbipLifecycleStep::HostBindReplay,
            UsbipLifecycleFailureKind::PolicyMismatch,
        );

        let attempt = lifecycle_attempt();
        let report = reconcile_usbip_vm_start_claims(&[lifecycle_claim()], &attempt, &mut executor);

        assert!(report.fatal());
        assert_eq!(report.degraded_count(), 1);
        assert_eq!(
            report.claims[0].degraded[0].code,
            UsbipDegradedReasonCode::PolicyFailed
        );
        assert_eq!(executor.calls, vec![UsbipLifecycleStep::HostBindReplay]);
    }

    #[test]
    fn lifecycle_stop_cleanup_preserves_claim() {
        let mut executor = LifecycleFixtureExecutor::default();
        let attempt = lifecycle_attempt();
        let report = cleanup_usbip_vm_stop_claims(&[lifecycle_claim()], &attempt, &mut executor);

        assert!(!report.fatal());
        assert_eq!(report.degraded_count(), 0);
        assert!(
            report.claims[0]
                .completed
                .contains(&UsbipLifecycleStep::PreserveDurableClaim)
        );
        assert_eq!(
            executor.calls,
            vec![
                UsbipLifecycleStep::GuestDetach,
                UsbipLifecycleStep::HostCarrierCleanup,
                UsbipLifecycleStep::ProxyReconcile,
            ]
        );
    }

    #[test]
    fn lifecycle_stop_preserves_claim_and_skips_host_unbind_when_flow_not_isolated() {
        let mut executor = LifecycleFixtureExecutor {
            stop_flow_observation: UsbipProxyFlowObservation::SharedOrAmbiguous,
            ..LifecycleFixtureExecutor::default()
        };
        let attempt = lifecycle_attempt();
        let report = cleanup_usbip_vm_stop_claims(&[lifecycle_claim()], &attempt, &mut executor);

        assert!(!report.fatal());
        assert_eq!(report.degraded_count(), 1);
        assert_eq!(
            report.claims[0].degraded[0].code,
            UsbipDegradedReasonCode::HostBindUnavailable
        );
        assert!(
            report.claims[0]
                .completed
                .contains(&UsbipLifecycleStep::PreserveDurableClaim)
        );
        assert_eq!(
            executor.calls,
            vec![
                UsbipLifecycleStep::GuestDetach,
                UsbipLifecycleStep::ProxyReconcile,
            ],
            "stop cleanup must not run sysfs host unbind when targeted stream cleanup is not proven"
        );
    }

    #[test]
    fn lifecycle_reconcile_threads_bounded_attempt_context_to_steps() {
        let attempt = lifecycle_attempt();
        let mut start_executor = LifecycleFixtureExecutor {
            guest_import_state: UsbipGuestImportState::Detached,
            ..LifecycleFixtureExecutor::default()
        };
        let start =
            reconcile_usbip_vm_start_claims(&[lifecycle_claim()], &attempt, &mut start_executor);
        assert!(!start.fatal());
        assert_eq!(
            start_executor.attempts,
            vec!["usb-lifecycle-7".to_owned(); start_executor.calls.len()]
        );

        let mut stop_executor = LifecycleFixtureExecutor::default();
        let stop = cleanup_usbip_vm_stop_claims(&[lifecycle_claim()], &attempt, &mut stop_executor);
        assert!(!stop.fatal());
        assert_eq!(
            stop_executor.attempts,
            vec!["usb-lifecycle-7".to_owned(); stop_executor.calls.len()]
        );
    }

    #[test]
    fn lifecycle_restart_composes_stop_cleanup_then_start_reimport() {
        let claim = lifecycle_claim();
        let stop_attempt = lifecycle_attempt();
        let mut stop_executor = LifecycleFixtureExecutor::default();
        let stop = cleanup_usbip_vm_stop_claims(
            std::slice::from_ref(&claim),
            &stop_attempt,
            &mut stop_executor,
        );
        let start_attempt = lifecycle_attempt();
        let mut start_executor = LifecycleFixtureExecutor {
            guest_import_state: UsbipGuestImportState::Detached,
            ..LifecycleFixtureExecutor::default()
        };
        let start = reconcile_usbip_vm_start_claims(&[claim], &start_attempt, &mut start_executor);

        assert!(!stop.fatal());
        assert!(!start.fatal());
        assert!(
            stop.claims[0]
                .completed
                .contains(&UsbipLifecycleStep::PreserveDurableClaim)
        );
        assert!(
            start_executor
                .calls
                .contains(&UsbipLifecycleStep::GuestImport)
        );
    }

    #[test]
    fn proxy_release_plan_blocks_and_targets_stream_before_host_unbind() {
        let release =
            plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::ReleaseBusid {
                targeted_cleanup: UsbipTargetedProxyCleanup::ConntrackOrSocketTuple,
            });

        assert_eq!(
            release.actions,
            vec![
                UsbipProxySynchronizationAction::WithdrawFirewallCarveout,
                UsbipProxySynchronizationAction::TargetedConntrackOrSocketKill,
                UsbipProxySynchronizationAction::HostUnbind,
                UsbipProxySynchronizationAction::PreserveSameEnvStreams,
            ]
        );
        assert!(!release.may_bounce_same_env_streams);
        assert!(release.claims_selective_busid_proxy_closure);

        let unsupported =
            plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::ReleaseBusid {
                targeted_cleanup: UsbipTargetedProxyCleanup::Unavailable,
            });
        assert!(
            !unsupported
                .actions
                .contains(&UsbipProxySynchronizationAction::HostUnbind)
        );
        assert!(
            unsupported
                .actions
                .contains(&UsbipProxySynchronizationAction::FailClosedRevocationNotIsolated)
        );
    }

    #[test]
    fn dynamic_busid_refresh_and_remove_do_not_bounce_sidecar_without_drain_policy() {
        let refresh =
            plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::RefreshExport);
        let release =
            plan_usbip_proxy_synchronization(UsbipProxySynchronizationIntent::ReleaseBusid {
                targeted_cleanup: UsbipTargetedProxyCleanup::Unavailable,
            });
        for plan in [&refresh, &release] {
            assert!(!plan.may_bounce_same_env_streams);
            assert!(
                !plan
                    .actions
                    .contains(&UsbipProxySynchronizationAction::RebindProxyListenerFdRelative)
            );
            assert!(
                !plan
                    .actions
                    .contains(&UsbipProxySynchronizationAction::BoundedDrainOrForce)
            );
        }
    }

    #[test]
    fn force_recycle_requires_explicit_drain_and_exclusive_rebind_lock() {
        let plan = plan_usbip_proxy_synchronization(
            UsbipProxySynchronizationIntent::ForceRecycleWithDrain {
                drain: UsbipProxyDrainPolicy::BoundedDrain { grace_ms: 250 },
            },
        );

        assert!(plan.may_bounce_same_env_streams);
        assert!(!plan.claims_selective_busid_proxy_closure);
        assert_eq!(
            plan.actions,
            vec![
                UsbipProxySynchronizationAction::BoundedDrainOrForce,
                UsbipProxySynchronizationAction::AcquireExclusiveSocketLifecycleLock,
                UsbipProxySynchronizationAction::RebindProxyListenerFdRelative,
            ]
        );
    }

    #[test]
    fn desired_claim_classifies_policy_toctou_host_and_guest_degradation() {
        let mut state = converged_state();
        state.declared.policy_failures = vec![UsbipPolicyFailure::TopologyMismatch];
        state.physical = UsbipPhysicalPresence::DepartedAfterLock;
        state.lock.state = UsbipPersistedLockClaimState::HeldByOtherOwner;
        state.host.carrier = UsbipActiveCarrierState::Unavailable;
        state.host.bind = UsbipHostBindState::BoundToUnexpectedDriver;
        state.host.proxy = UsbipProxyState::Failed;
        state.guest.import = UsbipGuestImportState::Failed;

        let reasons = state.degraded_reasons();
        assert!(reasons.contains(&UsbipDegradedReason::PolicyFailed(
            UsbipPolicyFailure::TopologyMismatch
        )));
        assert!(reasons.contains(&UsbipDegradedReason::DeviceDepartedAfterLock));
        assert!(reasons.contains(&UsbipDegradedReason::LockHeldByOtherOwner));
        assert!(reasons.contains(&UsbipDegradedReason::CarrierUnavailable));
        assert!(reasons.contains(&UsbipDegradedReason::HostBindUnavailable));
        assert!(reasons.contains(&UsbipDegradedReason::ProxyUnavailable));
        assert!(reasons.contains(&UsbipDegradedReason::GuestImportUnavailable));
    }

    #[test]
    fn degraded_reason_public_projection_has_static_labels_and_remediation() {
        let reason = UsbipDegradedReason::PolicyFailed(UsbipPolicyFailure::TopologyMismatch);
        assert_eq!(reason.code(), UsbipDegradedReasonCode::PolicyFailed);
        assert_eq!(
            reason.telemetry_labels(),
            UsbipTelemetryLabels {
                reason: "policy-failed",
                policy: "topology-mismatch",
            }
        );

        let public = reason.to_public_reason();
        assert_eq!(public.code, UsbipDegradedReasonCode::PolicyFailed);
        assert_eq!(
            public.policy_failure,
            Some(UsbipPolicyFailure::TopologyMismatch)
        );
        assert!(public.remediation.contains("rebuild"));
        let encoded = serde_json::to_string(&public).expect("public reason serializes");
        for forbidden in [
            "1-2.4",
            "/sys/",
            "serial-redacted-in-public",
            "stderr",
            "traceparent",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "leaked {forbidden}: {encoded}"
            );
        }
    }

    #[test]
    fn undeclared_claim_marks_active_host_and_guest_state_stale() {
        let mut state = converged_state();
        state.declared.desired = UsbipDesiredClaimState::Undeclared;

        let reasons = state.degraded_reasons();
        assert!(reasons.contains(&UsbipDegradedReason::StaleHostState));
        assert!(reasons.contains(&UsbipDegradedReason::StaleGuestState));
    }

    #[test]
    fn public_projection_redacts_raw_topology_and_serial() {
        let state = converged_state();
        let public = state.to_public_status("opaque-claim".to_owned());
        let encoded = serde_json::to_string(&public).expect("public status serializes");

        assert!(encoded.contains("1050"));
        assert!(encoded.contains("0407"));
        assert!(encoded.contains("serialObserved"));
        assert!(!encoded.contains("/sys/"));
        assert!(!encoded.contains("1-2.4"));
        assert!(!encoded.contains("serial-redacted-in-public"));
        assert!(!encoded.contains("busNumber"));
        assert!(!encoded.contains("portChain"));
    }

    #[test]
    fn public_status_scrubs_schema_string_fields_and_projects_events() {
        let mut state = converged_state();
        state.declared.policy_failures = vec![UsbipPolicyFailure::MissingBundleIntent];
        if let UsbipPhysicalPresence::Present { identity } = &mut state.physical {
            identity.vid_pid.vendor_id = Some("not-a-vid".to_owned());
            identity.vid_pid.product_id = Some("0407".to_owned());
        }

        let raw_trace_like_claim = "0123456789abcdef0123456789abcdef".to_owned();
        let public = state.to_public_status(raw_trace_like_claim.clone());
        assert_eq!(public.claim_ref, "claim-redacted");
        assert_eq!(public.device.vendor_id, None);
        assert_eq!(public.device.product_id.as_deref(), Some("0407"));
        assert_eq!(public.degraded_reason_count, public.degraded_reasons.len());
        assert!(
            public
                .degraded_reasons
                .iter()
                .any(|reason| reason.code == UsbipDegradedReasonCode::PolicyFailed)
        );

        let events = state.to_degraded_events("1-2.4".to_owned());
        assert!(!events.is_empty());
        assert!(
            events
                .iter()
                .all(|event| event.claim_ref == "claim-redacted")
        );
        assert!(
            events
                .iter()
                .any(|event| event.telemetry_reason == "policy-failed"
                    && event.telemetry_policy == "missing-bundle-intent")
        );

        let keys = state.to_dedupe_keys(raw_trace_like_claim, UsbipEventSource::vm("corp"));
        assert!(
            keys.iter()
                .all(|key| key.event_type == UsbipEventType::Degraded)
        );
        assert!(
            keys.iter()
                .all(|key| key.source.vm.as_deref() == Some("corp"))
        );

        let encoded = serde_json::to_string(&(public, events, keys)).expect("status serializes");
        for forbidden in [
            "/sys/",
            "1-2.4",
            "serial-redacted-in-public",
            "0123456789abcdef0123456789abcdef",
            "not-a-vid",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "leaked {forbidden}: {encoded}"
            );
        }
    }

    #[test]
    fn degraded_events_carry_bounded_reconcile_correlation_id() {
        let mut state = converged_state();
        state.host.proxy = UsbipProxyState::Failed;
        let attempt = UsbipReconcileAttemptContext {
            correlation_id: UsbipReconcileCorrelationId::new("usb-reconcile-7")
                .expect("bounded correlation id accepted"),
        };

        let events = state.to_degraded_events_for_attempt(
            "opaque-claim".to_owned(),
            UsbipEventSource::vm("corp"),
            Some(&attempt),
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.event_type, UsbipEventType::Degraded);
        assert_eq!(event.source.kind, UsbipEventSourceKind::Vm);
        assert_eq!(event.source.vm.as_deref(), Some("corp"));
        assert_eq!(
            event.correlation_id.as_ref().map(|id| id.as_str()),
            Some("usb-reconcile-7")
        );

        assert!(UsbipReconcileCorrelationId::new("0123456789abcdef0123456789abcdef").is_none());
        assert!(
            UsbipReconcileCorrelationId::new(
                "x".repeat(USBIP_RECONCILE_CORRELATION_ID_MAX_LEN + 1)
            )
            .is_none()
        );
        assert!(
            serde_json::from_str::<UsbipReconcileCorrelationId>(
                "\"0123456789abcdef0123456789abcdef\""
            )
            .is_err()
        );
    }

    #[test]
    fn usb_event_limiter_dedupes_repeats_and_summarizes_suppressed_window() {
        let key = UsbipDegradedDedupeKey {
            event_type: UsbipEventType::Degraded,
            source: UsbipEventSource::vm("corp"),
            claim_ref: String::new(),
            reason: UsbipDegradedReasonCode::ProxyUnavailable,
            policy_failure: None,
        };
        let mut limiter = UsbipEventDedupeLimiter::with_limits(8, 1, Duration::from_secs(60));

        let first = limiter.observe(Duration::from_secs(0), key.clone());
        assert!(first.emit_event);
        assert!(first.suppressed_summary.is_none());

        let duplicate = limiter.observe(Duration::from_secs(1), key.clone());
        assert!(!duplicate.emit_event);
        assert!(duplicate.suppressed_summary.is_none());

        let next_window = limiter.observe(Duration::from_secs(61), key);
        assert!(next_window.emit_event);
        let summary = next_window
            .suppressed_summary
            .expect("suppressed duplicate summarized when window rolls");
        assert_eq!(summary.suppressed_count, 1);
        assert_eq!(summary.window_start_ms, 0);
        assert_eq!(summary.window_end_ms, 61_000);
        assert_eq!(summary.bucket.event_type, UsbipEventType::Degraded);
        assert_eq!(summary.bucket.source_kind, UsbipEventSourceKind::Vm);
        assert_eq!(
            summary.bucket.source_vm,
            UsbipEventSourceVmProjection::Present
        );
    }

    #[test]
    fn usb_event_limiter_partitions_distinct_claim_refs_without_metric_labels() {
        let mut state = converged_state();
        state.host.proxy = UsbipProxyState::Failed;

        let key_a = state
            .to_dedupe_keys(
                "usbip-bind:env:work:vm:corp-vm:bus:1-2.3".to_owned(),
                UsbipEventSource::vm("corp-vm"),
            )
            .into_iter()
            .next()
            .expect("proxy failure emits a dedupe key");
        let key_b = state
            .to_dedupe_keys(
                "usbip-bind:env:work:vm:corp-vm:bus:1-2.4".to_owned(),
                UsbipEventSource::vm("corp-vm"),
            )
            .into_iter()
            .next()
            .expect("proxy failure emits a dedupe key");

        assert_ne!(key_a.limiter_key(), key_b.limiter_key());
        assert_eq!(key_a.bucket_key(), key_b.bucket_key());
        assert_eq!(key_a.metric_labels(), key_b.metric_labels());

        let encoded = serde_json::to_string(&(key_a.clone(), key_b.clone(), key_a.bucket_key()))
            .expect("dedupe keys serialize");
        assert!(!encoded.contains("1-2.3"));
        assert!(!encoded.contains("1-2.4"));

        let mut limiter = UsbipEventDedupeLimiter::with_limits(8, 1, Duration::from_secs(60));
        let first = limiter.observe(Duration::from_secs(0), key_a.clone());
        let repeated_same_claim = limiter.observe(Duration::from_secs(1), key_a);
        let distinct_claim = limiter.observe(Duration::from_secs(2), key_b);

        assert!(first.emit_event);
        assert!(!repeated_same_claim.emit_event);
        assert!(distinct_claim.emit_event);
        assert_eq!(limiter.bucket_count(), 2);
        assert_eq!(
            first.bucket_key.metric_labels(),
            distinct_claim.bucket_key.metric_labels()
        );
    }

    #[test]
    fn usb_event_limiter_caps_partitions_with_overflow_bucket() {
        let mut limiter = UsbipEventDedupeLimiter::with_limits(3, 1, Duration::from_secs(60));

        let first = limiter.observe(
            Duration::from_secs(0),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Degraded,
                source: UsbipEventSource::vm("corp-a"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::ProxyUnavailable,
                policy_failure: None,
            },
        );
        assert_eq!(first.bucket_key.event_type, UsbipEventType::Degraded);
        assert_eq!(first.bucket_key.source_kind, UsbipEventSourceKind::Vm);
        assert_eq!(
            first.bucket_key.source_vm,
            UsbipEventSourceVmProjection::Present
        );

        let second = limiter.observe(
            Duration::from_secs(0),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::StateTransition,
                source: UsbipEventSource::vm("corp-b"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::HostBindUnavailable,
                policy_failure: None,
            },
        );
        assert_eq!(
            second.bucket_key.event_type,
            UsbipEventType::StateTransition
        );
        assert_eq!(second.bucket_key.source_kind, UsbipEventSourceKind::Vm);
        assert_eq!(
            second.bucket_key.source_vm,
            UsbipEventSourceVmProjection::Present
        );
        assert_eq!(limiter.bucket_count(), 2);

        let third = limiter.observe(
            Duration::from_secs(1),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::StateTransition,
                source: UsbipEventSource::vm("corp-c"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::CarrierUnavailable,
                policy_failure: None,
            },
        );
        assert_eq!(third.bucket_key.event_type, UsbipEventType::StateTransition);
        assert!(
            !third.emit_event,
            "same event type/source projection bucket is rate limited regardless of reason or raw VM"
        );
        assert_eq!(limiter.bucket_count(), 2);
    }

    #[test]
    fn usb_event_limiter_has_strict_bucket_cap_without_lru_churn() {
        let mut limiter = UsbipEventDedupeLimiter::with_limits(4, 1, Duration::from_secs(60));
        let mut overflow_count = 0;

        for idx in 0..24 {
            let decision = limiter.observe(
                Duration::from_secs(idx),
                UsbipDegradedDedupeKey {
                    event_type: UsbipEventType::StateTransition,
                    source: UsbipEventSource::vm(format!("corp-{idx}")),
                    claim_ref: String::new(),
                    reason: UsbipDegradedReasonCode::ProxyUnavailable,
                    policy_failure: None,
                },
            );
            if decision.bucket_key.event_type == UsbipEventType::Other {
                overflow_count += 1;
                assert_eq!(decision.bucket_key.source_kind, UsbipEventSourceKind::Other);
                assert_eq!(decision.bucket_key.metric_labels().source_vm, "other");
            }
            assert!(
                limiter.bucket_count() <= 4,
                "bucket cap must be strict after source {idx}"
            );
        }

        assert_eq!(limiter.bucket_count(), 1);
        assert_eq!(
            overflow_count, 0,
            "raw VM names collapse to the source-present bucket instead of churning labels"
        );

        let duplicate = limiter.observe(
            Duration::from_secs(30),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::StateTransition,
                source: UsbipEventSource::vm("corp-overflow"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::CarrierUnavailable,
                policy_failure: None,
            },
        );
        assert_eq!(
            duplicate.bucket_key.event_type,
            UsbipEventType::StateTransition
        );
        assert!(!duplicate.emit_event);

        let summaries = limiter.flush_suppressed(Duration::from_secs(60));
        let vm_summary = summaries
            .iter()
            .find(|summary| summary.bucket.event_type == UsbipEventType::StateTransition)
            .expect("source-present bucket suppression summarized");
        assert_eq!(vm_summary.suppressed_count, 24);
        assert_eq!(vm_summary.window_start_ms, 0);
        assert_eq!(vm_summary.window_end_ms, 60_000);
    }

    #[test]
    fn usb_event_limiter_partitions_by_event_type_source_and_claim_ref() {
        let mut limiter = UsbipEventDedupeLimiter::with_limits(8, 1, Duration::from_secs(60));

        let first = limiter.observe(
            Duration::from_secs(0),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Degraded,
                source: UsbipEventSource::vm("corp-a"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::ProxyUnavailable,
                policy_failure: None,
            },
        );
        assert!(first.emit_event);

        let same_partition_different_reason_vm = limiter.observe(
            Duration::from_secs(1),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Degraded,
                source: UsbipEventSource::vm("corp-b"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::CarrierUnavailable,
                policy_failure: Some(UsbipPolicyFailure::TopologyMismatch),
            },
        );
        assert_eq!(
            same_partition_different_reason_vm.bucket_key,
            UsbipEventBucketKey {
                event_type: UsbipEventType::Degraded,
                source_kind: UsbipEventSourceKind::Vm,
                source_vm: UsbipEventSourceVmProjection::Present,
            }
        );
        assert!(
            !same_partition_different_reason_vm.emit_event,
            "reason/policy/raw VM must not split USB rate-limit buckets for one claim"
        );

        let different_event_type = limiter.observe(
            Duration::from_secs(2),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::StateTransition,
                source: UsbipEventSource::vm("corp-c"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::CarrierUnavailable,
                policy_failure: None,
            },
        );
        assert!(different_event_type.emit_event);

        let different_source_kind = limiter.observe(
            Duration::from_secs(3),
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Degraded,
                source: UsbipEventSource::component(UsbipEventSourceKind::Broker),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::CarrierUnavailable,
                policy_failure: None,
            },
        );
        assert!(different_source_kind.emit_event);
        assert_eq!(limiter.bucket_count(), 3);
    }

    #[test]
    fn usb_event_limiter_overflow_uses_single_static_other_bucket() {
        let mut limiter = UsbipEventDedupeLimiter::with_limits(3, 1, Duration::from_secs(60));

        for (idx, key) in [
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Degraded,
                source: UsbipEventSource::vm("corp"),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::ProxyUnavailable,
                policy_failure: None,
            },
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::StateTransition,
                source: UsbipEventSource::component(UsbipEventSourceKind::Broker),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::CarrierUnavailable,
                policy_failure: None,
            },
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::SuppressedSummary,
                source: UsbipEventSource::component(UsbipEventSourceKind::Guest),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::GuestImportUnavailable,
                policy_failure: None,
            },
            UsbipDegradedDedupeKey {
                event_type: UsbipEventType::Other,
                source: UsbipEventSource::component(UsbipEventSourceKind::Host),
                claim_ref: String::new(),
                reason: UsbipDegradedReasonCode::HostBindUnavailable,
                policy_failure: None,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let decision = limiter.observe(Duration::from_secs(idx as u64), key);
            assert!(
                limiter.bucket_count() <= 3,
                "bucket cap must remain strict after partition {idx}"
            );
            if idx >= 2 {
                assert_eq!(decision.bucket_key, UsbipEventBucketKey::overflow());
                assert_eq!(
                    decision.bucket_key.metric_labels(),
                    UsbipEventMetricLabels {
                        event_type: "other",
                        source_kind: "other",
                        source_vm: "other",
                        reason: "none",
                        policy: "none",
                    }
                );
            }
        }

        let summaries = limiter.flush_suppressed(Duration::from_secs(60));
        let overflow_summary = summaries
            .iter()
            .find(|summary| summary.bucket == UsbipEventBucketKey::overflow())
            .expect("overflow bucket summarized");
        assert_eq!(overflow_summary.suppressed_count, 1);
        assert_eq!(overflow_summary.window_start_ms, 2_000);
        assert_eq!(overflow_summary.window_end_ms, 60_000);
    }

    #[test]
    fn usb_event_metric_projection_uses_only_static_label_values() {
        let key = UsbipDegradedDedupeKey {
            event_type: UsbipEventType::Degraded,
            source: UsbipEventSource::vm("0123456789abcdef0123456789abcdef"),
            claim_ref: String::new(),
            reason: UsbipDegradedReasonCode::PolicyFailed,
            policy_failure: Some(UsbipPolicyFailure::TopologyMismatch),
        };

        assert_eq!(key.source.vm.as_deref(), Some("other"));
        assert_eq!(
            key.metric_labels(),
            UsbipEventMetricLabels {
                event_type: "degraded",
                source_kind: "vm",
                source_vm: "other",
                reason: "policy-failed",
                policy: "topology-mismatch",
            }
        );
        assert_eq!(
            UsbipEventSource::vm("corp").telemetry_labels(),
            UsbipEventSourceLabels {
                source_kind: "vm",
                vm: "present",
            }
        );
        assert_eq!(
            UsbipEventSource::vm("0123456789abcdef0123456789abcdef").telemetry_labels(),
            UsbipEventSourceLabels {
                source_kind: "vm",
                vm: "other",
            }
        );
    }

    #[test]
    fn usbip_step_failure_classification_uses_bounded_other_fallback() {
        assert_eq!(
            classify_usbip_step_failure("lock", "busy for /run/nixling/locks/usbip/1-2.4"),
            UsbipStepFailureReasonKind::LockUnavailable
        );
        assert_eq!(
            classify_usbip_step_failure("bind", "raw stderr: timed out for bus 1-2.4"),
            UsbipStepFailureReasonKind::CommandTimeout
        );
        assert_eq!(
            classify_usbip_step_failure("future-step", "unexpected trace 0123456789abcdef"),
            UsbipStepFailureReasonKind::Other
        );
        assert_eq!(UsbipStepFailureReasonKind::Other.telemetry_label(), "other");
    }

    #[test]
    fn public_open_enums_preserve_unknown_strings() {
        let source: UsbipEventSource = serde_json::from_str(r#"{"kind":"vm","vm":"future-vm"}"#)
            .expect("unknown-but-shaped VM source parses");
        assert_eq!(source.vm.as_deref(), Some("future-vm"));
        assert!(
            serde_json::from_str::<UsbipEventSource>(
                r#"{"kind":"vm","vm":"future-vm","pid":12345}"#
            )
            .is_err(),
            "field allowlisting rejects unknown fields rather than dropping values"
        );
        assert!(
            serde_json::from_str::<UsbipEventSource>(
                r#"{"kind":"vm","vm":"0123456789abcdef0123456789abcdef"}"#
            )
            .is_err(),
            "shape validation rejects unbounded trace-like VM labels"
        );

        let desired: UsbipPublicDesiredClaimState =
            serde_json::from_str("\"future-desired\"").expect("unknown desired state parses");
        assert_eq!(
            desired,
            UsbipPublicDesiredClaimState::Unknown("future-desired".to_owned())
        );
        assert_eq!(
            serde_json::to_string(&desired).expect("unknown desired state serializes"),
            "\"future-desired\""
        );

        let posture: UsbipPublicPosture =
            serde_json::from_str("\"future-posture\"").expect("unknown posture parses");
        assert_eq!(
            posture,
            UsbipPublicPosture::Unknown("future-posture".to_owned())
        );
        assert_eq!(
            serde_json::to_string(&posture).expect("unknown posture serializes"),
            "\"future-posture\""
        );
    }
}
