//! `CreateTapFd` + `CreatePersistentTap` + `SetBridgePortFlags` ops.
//!
//! - `CreateTapFd`: broker opens `/dev/net/tun` + `TUNSETIFF` and
//!   returns the fd via `SCM_RIGHTS`. The runner has no
//!   `CAP_NET_ADMIN`. Implementation lives behind the netlink
//!   backend trait (`nixling_host::netlink::NetlinkBackend`) so the
//!   L1c canary tests can drive the path with a fake backend.
//! - `CreatePersistentTap`: fallback when CH does not support
//!   `tap-fd`. Broker calls `TUNSETPERSIST` + `TUNSETOWNER` /
//!   `TUNSETGROUP` so the runner uid/gid can open the device node.
//! - `SetBridgePortFlags`: every flag, every role with readback.
//!
//! NetworkManager unmanaged gate: the broker MUST NOT create the link
//! until the prior `ApplyNmUnmanaged` op has either confirmed every
//! declared ifname is `unmanaged` (NM present + reload + readback ok),
//! or recorded a `NotApplicable` outcome because NM was absent and the
//! bundle's firewall coexistence policy permits a clean coexistence (no
//! NM detected → `coexist`). The previous implementation hardcoded
//! `nm_unmanaged_applied = true`; the actual prior-op outcome is now
//! threaded into [`TapCreateGate`] and a missing gate refuses with
//! `nm-unmanaged-pre-create-required`.

use crate::ops::exec_reconcile::{ReconcileExecError, ReconcileExecutor, SystemLiveExec};
use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::host::{HostJson, TapRole};
use nixling_core::host_w3::TapRoleW3;
use nixling_core::manifest_v04::VmEntry;
use nixling_host::bridge_port::BridgePortFlagSet;
use nixling_host::ifname::{DerivedRole, IfName, derive_from_env_vm};
use nixling_host::netlink::{
    LinkKind, LinkSpec, NetlinkBackend, NetlinkError, TapOwner, fake::FakeBackend,
    ipv6_off_sequence, readback_bridge_port_flags,
};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Outcome of the prior `ApplyNmUnmanaged` op for the same ifname
/// set, threaded into [`create_tap`] via [`TapCreateGate`]. The wire
/// request never carries this directly — the runtime fetches it from
/// the bundle session state (per H1 wire-refactor) and supplies it
/// alongside [`CreateTapRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmUnmanagedOutcome {
    /// `ApplyNmUnmanaged` ran successfully and every declared ifname
    /// is in state `unmanaged` per `nmcli ... device status`.
    Applied,
    /// NetworkManager is absent on this host AND the bundle's
    /// firewall coexistence policy declares clean coexistence (no
    /// manager detected → `coexist`). The gate is satisfied without
    /// the `ApplyNmUnmanaged` op needing to run.
    NotApplicableNmAbsentConfiguredCoexist,
    /// `ApplyNmUnmanaged` has not yet been threaded through for this
    /// ifname; the broker refuses link creation with
    /// `nm-unmanaged-pre-create-required`.
    NotApplied,
}

impl NmUnmanagedOutcome {
    pub fn satisfied(self) -> bool {
        matches!(
            self,
            NmUnmanagedOutcome::Applied
                | NmUnmanagedOutcome::NotApplicableNmAbsentConfiguredCoexist
        )
    }
}

/// Gate carried by every TAP-create operation so the runtime can
/// thread the prior `ApplyNmUnmanaged` outcome from the bundle
/// session state without round-tripping it over the wire request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapCreateGate {
    pub nm_unmanaged_applied: NmUnmanagedOutcome,
}

impl TapCreateGate {
    /// Convenience constructor for the no-NM-detected coexist case.
    pub const fn nm_absent_coexist() -> Self {
        Self {
            nm_unmanaged_applied: NmUnmanagedOutcome::NotApplicableNmAbsentConfiguredCoexist,
        }
    }
    pub const fn applied() -> Self {
        Self {
            nm_unmanaged_applied: NmUnmanagedOutcome::Applied,
        }
    }
    pub const fn not_applied() -> Self {
        Self {
            nm_unmanaged_applied: NmUnmanagedOutcome::NotApplied,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateTapRequest {
    pub env: String,
    pub vm: Option<String>,
    pub mtu: Option<u32>,
    pub mac: Option<[u8; 6]>,
    /// Persistent mode: bind `TUNSETOWNER`/`TUNSETGROUP` to this
    /// uid/gid. None means `CreateTapFd` (transient + SCM_RIGHTS).
    pub persistent_owner: Option<TapOwner>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTapAudit {
    pub ifname_derived: String,
    pub role: &'static str,
    pub persistent: bool,
    /// Echoes the gate the broker enforced. Surfaced in the audit
    /// record so post-mortem analysis can prove the link-create did
    /// not race the NM unmanaged op.
    pub nm_unmanaged_outcome: NmUnmanagedOutcome,
}

pub fn derive_tap_ifname(req: &CreateTapRequest) -> Result<IfName, NetlinkError> {
    derive_from_env_vm(&req.env, req.vm.as_deref(), DerivedRole::Tap, None)
        .map_err(|e| NetlinkError::Backend(e.to_string()))
}

pub fn create_tap<B: NetlinkBackend>(
    backend: &mut B,
    req: &CreateTapRequest,
    gate: TapCreateGate,
) -> Result<CreateTapAudit, NetlinkError> {
    if !gate.nm_unmanaged_applied.satisfied() {
        return Err(NetlinkError::Backend(
            "nm-unmanaged-pre-create-required".to_owned(),
        ));
    }
    let ifname = derive_tap_ifname(req)?;
    let spec = LinkSpec {
        ifname: ifname.clone(),
        kind: LinkKind::Tap,
        mtu: req.mtu,
        mac: req.mac,
        tap_owner: req.persistent_owner,
    };
    ipv6_off_sequence(backend, &spec, true)?;
    Ok(CreateTapAudit {
        ifname_derived: ifname.as_str().to_owned(),
        role: if req.persistent_owner.is_some() {
            "persistent-tap"
        } else {
            "tap-fd"
        },
        persistent: req.persistent_owner.is_some(),
        nm_unmanaged_outcome: gate.nm_unmanaged_applied,
    })
}

#[derive(Debug, Clone)]
pub struct SetBridgePortFlagsRequest {
    pub ifname: IfName,
    pub role: TapRoleW3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetBridgePortFlagsAudit {
    pub ifname: String,
    pub role: TapRoleW3,
    pub flags_after: BridgePortFlagSet,
}

pub fn set_bridge_port_flags<B: NetlinkBackend>(
    backend: &mut B,
    req: &SetBridgePortFlagsRequest,
) -> Result<SetBridgePortFlagsAudit, NetlinkError> {
    let flags = BridgePortFlagSet::defaults_for(req.role.clone());
    backend.write_bridge_port_flags(&req.ifname, flags)?;
    let observed = readback_bridge_port_flags(backend, &req.ifname, req.role.clone())?;
    Ok(SetBridgePortFlagsAudit {
        ifname: req.ifname.as_str().to_owned(),
        role: req.role.clone(),
        flags_after: observed,
    })
}

#[derive(Debug)]
pub struct LiveCreateTapOutcome {
    pub bridge_ifname: Option<nixling_core::host::IfName>,
    pub tap_ifname: nixling_core::host::IfName,
    pub fd: Option<OwnedFd>,
}

pub fn live_create_tap_fd(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &nixling_ipc::broker_wire::CreateTapFdRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveCreateTapOutcome, super::OpError> {
    let intent = resolver
        .resolve_tap_intent(req.vm_id.as_str(), req.role_id.as_str())
        .ok_or_else(|| super::OpError::UnknownSubject {
            operation: "CreateTapFd",
            subject: req.vm_id.as_str().to_owned(),
        })?;
    let dev_net =
        crate::sys::path_safe::open_dir_path_safe(Path::new("/dev/net")).map_err(|e| {
            super::OpError::Io {
                path: PathBuf::from("/dev/net"),
                detail: e.to_string(),
            }
        })?;
    let tun_fd =
        crate::sys::path_safe::open_at(dev_net.as_fd(), Path::new("tun"), rustix::fs::OFlags::RDWR)
            .map_err(|e| super::OpError::Io {
                path: PathBuf::from("/dev/net/tun"),
                detail: e.to_string(),
            })?;
    crate::sys::tun_create_tap_fd(&tun_fd, intent.tap_ifname.as_str()).map_err(|e| {
        super::OpError::Io {
            path: PathBuf::from("/dev/net/tun"),
            detail: e.to_string(),
        }
    })?;
    attach_tap_to_bridge(&intent.tap_ifname, &intent.bridge_ifname)?;
    Ok(LiveCreateTapOutcome {
        bridge_ifname: Some(intent.bridge_ifname),
        tap_ifname: intent.tap_ifname,
        fd: Some(tun_fd),
    })
}

pub fn live_create_persistent_tap(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &nixling_ipc::broker_wire::CreatePersistentTapRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveCreateTapOutcome, super::OpError> {
    let intent = resolver
        .resolve_tap_intent(req.vm_id.as_str(), req.role_id.as_str())
        .ok_or_else(|| super::OpError::UnknownSubject {
            operation: "CreatePersistentTap",
            subject: req.vm_id.as_str().to_owned(),
        })?;
    let dev_net =
        crate::sys::path_safe::open_dir_path_safe(Path::new("/dev/net")).map_err(|e| {
            super::OpError::Io {
                path: PathBuf::from("/dev/net"),
                detail: e.to_string(),
            }
        })?;
    let tun_fd =
        crate::sys::path_safe::open_at(dev_net.as_fd(), Path::new("tun"), rustix::fs::OFlags::RDWR)
            .map_err(|e| super::OpError::Io {
                path: PathBuf::from("/dev/net/tun"),
                detail: e.to_string(),
            })?;
    crate::sys::tun_create_tap_fd(&tun_fd, intent.tap_ifname.as_str()).map_err(|e| {
        super::OpError::Io {
            path: PathBuf::from("/dev/net/tun"),
            detail: e.to_string(),
        }
    })?;
    crate::sys::tun_set_owner(&tun_fd, intent.owner_uid).map_err(|e| super::OpError::Io {
        path: PathBuf::from("/dev/net/tun"),
        detail: e.to_string(),
    })?;
    crate::sys::tun_set_group(&tun_fd, intent.owner_gid).map_err(|e| super::OpError::Io {
        path: PathBuf::from("/dev/net/tun"),
        detail: e.to_string(),
    })?;
    crate::sys::tun_set_persist(&tun_fd, true).map_err(|e| super::OpError::Io {
        path: PathBuf::from("/dev/net/tun"),
        detail: e.to_string(),
    })?;
    attach_tap_to_bridge(&intent.tap_ifname, &intent.bridge_ifname)?;
    Ok(LiveCreateTapOutcome {
        bridge_ifname: Some(intent.bridge_ifname),
        tap_ifname: intent.tap_ifname,
        fd: None,
    })
}

fn attach_tap_to_bridge(
    tap_ifname: &nixling_core::host::IfName,
    bridge_ifname: &nixling_core::host::IfName,
) -> Result<(), super::OpError> {
    let ip = ip_binary_path();
    run_ip_link(
        &ip,
        &[
            "link",
            "set",
            "dev",
            tap_ifname.as_str(),
            "master",
            bridge_ifname.as_str(),
        ],
    )?;
    run_ip_link(&ip, &["link", "set", "dev", tap_ifname.as_str(), "up"])
}

fn run_ip_link(ip: &Path, args: &[&str]) -> Result<(), super::OpError> {
    let output = Command::new(ip)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| super::OpError::Io {
            path: ip.to_path_buf(),
            detail: err.to_string(),
        })?;
    if !output.status.success() {
        return Err(super::OpError::Io {
            path: ip.to_path_buf(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSetBridgePortFlagsError {
    Resolve(String),
    ReconcileExec(ReconcileExecError),
    ReadbackMismatch {
        path: PathBuf,
        expected: String,
        observed: String,
    },
}

impl std::fmt::Display for LiveSetBridgePortFlagsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resolve(detail) => write!(f, "set-bridge-port-flags resolve: {detail}"),
            Self::ReconcileExec(err) => write!(f, "set-bridge-port-flags: {err}"),
            Self::ReadbackMismatch {
                path,
                expected,
                observed,
            } => write!(
                f,
                "set-bridge-port-flags readback mismatch at {}: expected {:?}, observed {:?}",
                path.display(),
                expected,
                observed
            ),
        }
    }
}

impl std::error::Error for LiveSetBridgePortFlagsError {}

struct LiveBridgePortTarget {
    bridge: String,
    port: String,
    role: TapRoleW3,
}

pub fn live_set_bridge_port_flags(
    _executor: &dyn ReconcileExecutor,
    resolver: &BundleResolver,
    req: &nixling_ipc::broker_wire::SetBridgePortFlagsRequest,
) -> Result<nixling_ipc::broker_wire::BridgePortFlagsResponse, LiveSetBridgePortFlagsError> {
    let target = resolve_live_bridge_port_target(resolver, req)?;
    let ip_binary = ip_binary_path();
    live_set_bridge_port_flags_with_ops(
        &target,
        |port, flags| apply_bridge_port_flags_via_ip(&ip_binary, port, flags),
        |port| read_bridge_port_flags_via_ip(&ip_binary, port),
    )
}

fn live_set_bridge_port_flags_with_ops<F, G>(
    target: &LiveBridgePortTarget,
    mut apply: F,
    mut readback: G,
) -> Result<nixling_ipc::broker_wire::BridgePortFlagsResponse, LiveSetBridgePortFlagsError>
where
    F: FnMut(&str, BridgePortFlagSet) -> Result<(), ReconcileExecError>,
    G: FnMut(&str) -> Result<BridgePortFlagSet, ReconcileExecError>,
{
    let desired = target.desired_flags();
    apply(target.port.as_str(), desired).map_err(LiveSetBridgePortFlagsError::ReconcileExec)?;
    let observed =
        readback(target.port.as_str()).map_err(LiveSetBridgePortFlagsError::ReconcileExec)?;
    nixling_host::bridge_port::validate_readback(target.role.clone(), observed).map_err(
        |drift| {
            let first = drift
                .differences
                .first()
                .expect("bridge-port drift must include at least one flag");
            LiveSetBridgePortFlagsError::ReadbackMismatch {
                path: bridge_port_readback_path(target.port.as_str(), first.flag),
                expected: bool_to_ip_value(first.expected).to_owned(),
                observed: bool_to_ip_value(first.actual).to_owned(),
            }
        },
    )?;
    Ok(nixling_ipc::broker_wire::BridgePortFlagsResponse {
        bridge: nixling_core::host::IfName::new(&target.bridge).map_err(|err| {
            LiveSetBridgePortFlagsError::Resolve(format!("resolved bridge ifname invalid: {err}"))
        })?,
        isolated: desired.isolated,
        neigh_suppress: desired.neigh_suppress,
        port: nixling_core::host::IfName::new(&target.port).map_err(|err| {
            LiveSetBridgePortFlagsError::Resolve(format!("resolved port ifname invalid: {err}"))
        })?,
    })
}

fn resolve_live_bridge_port_target(
    resolver: &BundleResolver,
    req: &nixling_ipc::broker_wire::SetBridgePortFlagsRequest,
) -> Result<LiveBridgePortTarget, LiveSetBridgePortFlagsError> {
    let vm_name = req.vm_id.as_str();
    let manifest_vm =
        resolver.manifest.vms.get(vm_name).ok_or_else(|| {
            LiveSetBridgePortFlagsError::Resolve(format!("unknown vm_id {vm_name}"))
        })?;
    let env_name = manifest_vm.env.as_deref().ok_or_else(|| {
        LiveSetBridgePortFlagsError::Resolve(format!("vm {vm_name} is not attached to an env"))
    })?;
    let env = resolver
        .host
        .environments
        .iter()
        .find(|env| env.env == env_name)
        .ok_or_else(|| {
            LiveSetBridgePortFlagsError::Resolve(format!("host.json missing env {env_name}"))
        })?;
    let role = tap_role_from_role_id(req.role_id.as_str())?;
    let row = env
        .bridge_port_flags
        .iter()
        .find(|row| row.role == role)
        .ok_or_else(|| {
            LiveSetBridgePortFlagsError::Resolve(format!(
                "host.json missing bridgePortFlags row for env {env_name} role {}",
                req.role_id.as_str()
            ))
        })?;
    let (bridge, port) = match role {
        TapRole::WorkloadLan => (
            resolved_bundle_ifname(
                &resolver.host,
                env_name,
                None,
                TapRole::NetVmLan,
                env.bridge.as_str(),
            ),
            resolved_bundle_ifname(
                &resolver.host,
                env_name,
                Some(vm_name),
                TapRole::WorkloadLan,
                &manifest_vm.tap,
            ),
        ),
        TapRole::NetVmLan => (
            resolved_bundle_ifname(
                &resolver.host,
                env_name,
                None,
                TapRole::NetVmLan,
                env.bridge.as_str(),
            ),
            resolved_bundle_ifname(
                &resolver.host,
                env_name,
                Some(vm_name),
                TapRole::NetVmLan,
                &format!("{env_name}-l1"),
            ),
        ),
        TapRole::Uplink => {
            let net_vm = net_vm_for_env(resolver, env_name)?;
            let uplink_bridge = net_vm.bridge.as_deref().ok_or_else(|| {
                LiveSetBridgePortFlagsError::Resolve(format!(
                    "net VM for env {env_name} has no bridge field"
                ))
            })?;
            (
                resolved_bundle_ifname(
                    &resolver.host,
                    env_name,
                    None,
                    TapRole::Uplink,
                    uplink_bridge,
                ),
                resolved_bundle_ifname(
                    &resolver.host,
                    env_name,
                    Some(&net_vm.name),
                    TapRole::Uplink,
                    &net_vm.tap,
                ),
            )
        }
    };
    let role = match role {
        TapRole::NetVmLan => TapRoleW3::NetVmLan,
        TapRole::Uplink => TapRoleW3::UplinkP2P,
        TapRole::WorkloadLan if row.isolated => TapRoleW3::WorkloadLanIsolated,
        TapRole::WorkloadLan => TapRoleW3::WorkloadLanEastWest,
    };
    Ok(LiveBridgePortTarget { bridge, port, role })
}

fn tap_role_from_role_id(role_id: &str) -> Result<TapRole, LiveSetBridgePortFlagsError> {
    match role_id {
        "workload-lan" => Ok(TapRole::WorkloadLan),
        "net-vm-lan" => Ok(TapRole::NetVmLan),
        "uplink" => Ok(TapRole::Uplink),
        other => Err(LiveSetBridgePortFlagsError::Resolve(format!(
            "unsupported bridge-port role_id {other}"
        ))),
    }
}

fn resolved_bundle_ifname(
    host: &HostJson,
    env: &str,
    vm: Option<&str>,
    role: TapRole,
    fallback: &str,
) -> String {
    host.if_name_mappings
        .iter()
        .find(|mapping| {
            mapping.env == env
                && mapping.vm.as_deref() == vm
                && mapping.role == role
                && mapping.user_visible_name == fallback
        })
        .map(|mapping| mapping.derived_ifname.as_str().to_owned())
        .unwrap_or_else(|| fallback.to_owned())
}

fn net_vm_for_env<'a>(
    resolver: &'a BundleResolver,
    env: &str,
) -> Result<&'a VmEntry, LiveSetBridgePortFlagsError> {
    resolver
        .manifest
        .vms
        .values()
        .find(|vm| vm.is_net_vm && vm.env.as_deref() == Some(env))
        .ok_or_else(|| {
            LiveSetBridgePortFlagsError::Resolve(format!("manifest missing net VM for env {env}"))
        })
}

fn ip_binary_path() -> PathBuf {
    std::env::var_os("NIXLING_BROKER_IP_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/sbin/ip"))
}

/// Drive bridge-slave flags through iproute2's netlink frontend rather
/// than the legacy sysfs compatibility files. The nested broker
/// workspace does not currently wire `rtnetlink` directly, but `ip link
/// ... type bridge_slave ...` emits the same `RTM_SETLINK`
/// `IFLA_PROTINFO` updates without reaching into `/sys`.
fn apply_bridge_port_flags_via_ip(
    ip_binary: &Path,
    port: &str,
    flags: BridgePortFlagSet,
) -> Result<(), ReconcileExecError> {
    if !ip_binary.is_absolute() {
        return Err(ReconcileExecError::InvalidInput {
            detail: format!("ip binary must be absolute: {}", ip_binary.display()),
        });
    }
    let args = build_bridge_port_ip_args(port, flags);
    let output = Command::new(ip_binary)
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| ReconcileExecError::BinaryMissing {
            which: "ip link set".to_owned(),
            detail: err.to_string(),
        })?;
    if !output.status.success() {
        return Err(ReconcileExecError::NonZeroExit {
            which: "ip link set".to_owned(),
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn read_bridge_port_flags_via_ip(
    ip_binary: &Path,
    port: &str,
) -> Result<BridgePortFlagSet, ReconcileExecError> {
    if !ip_binary.is_absolute() {
        return Err(ReconcileExecError::InvalidInput {
            detail: format!("ip binary must be absolute: {}", ip_binary.display()),
        });
    }
    let output = Command::new(ip_binary)
        .args(["-d", "-j", "link", "show", "dev", port])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| ReconcileExecError::BinaryMissing {
            which: "ip link show".to_owned(),
            detail: err.to_string(),
        })?;
    if !output.status.success() {
        return Err(ReconcileExecError::NonZeroExit {
            which: "ip link show".to_owned(),
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    parse_bridge_port_flags_json(&String::from_utf8_lossy(&output.stdout), port)
}

fn build_bridge_port_ip_args(port: &str, flags: BridgePortFlagSet) -> Vec<String> {
    [
        "link".to_owned(),
        "set".to_owned(),
        "dev".to_owned(),
        port.to_owned(),
        "type".to_owned(),
        "bridge_slave".to_owned(),
        "hairpin".to_owned(),
        bool_to_ip_value(flags.hairpin_mode).to_owned(),
        "learning".to_owned(),
        bool_to_ip_value(flags.learning).to_owned(),
        "flood".to_owned(),
        bool_to_ip_value(flags.unicast_flood).to_owned(),
        "mcast_flood".to_owned(),
        bool_to_ip_value(flags.multicast_flood).to_owned(),
        "neigh_suppress".to_owned(),
        bool_to_ip_value(flags.neigh_suppress).to_owned(),
        "guard".to_owned(),
        bool_to_ip_value(flags.bpdu_guard).to_owned(),
        "root_block".to_owned(),
        bool_to_ip_value(flags.root_block).to_owned(),
        "fastleave".to_owned(),
        bool_to_ip_value(flags.fast_leave).to_owned(),
        "isolated".to_owned(),
        bool_to_ip_value(flags.isolated).to_owned(),
    ]
    .to_vec()
}

fn parse_bridge_port_flags_json(
    json: &str,
    port: &str,
) -> Result<BridgePortFlagSet, ReconcileExecError> {
    let links = serde_json::from_str::<Vec<serde_json::Value>>(json).map_err(|err| {
        ReconcileExecError::InvalidInput {
            detail: format!("invalid ip -j link output for {port}: {err}"),
        }
    })?;
    let link = links
        .iter()
        .find(|link| link.get("ifname").and_then(serde_json::Value::as_str) == Some(port))
        .or_else(|| links.first())
        .ok_or_else(|| ReconcileExecError::InvalidInput {
            detail: format!("ip -j link output missing bridge port {port}"),
        })?;
    let info = link
        .pointer("/linkinfo/info_slave_data")
        .or_else(|| link.pointer("/linkinfo/info_data"))
        .ok_or_else(|| ReconcileExecError::InvalidInput {
            detail: format!("ip -j link output missing bridge slave info for {port}"),
        })?;
    Ok(BridgePortFlagSet {
        isolated: json_bool_field(info, &["isolated"], port)?,
        hairpin_mode: json_bool_field(info, &["hairpin", "hairpin_mode"], port)?,
        learning: json_bool_field(info, &["learning"], port)?,
        unicast_flood: json_bool_field(info, &["flood", "unicast_flood"], port)?,
        multicast_flood: json_bool_field(info, &["mcast_flood", "multicast_flood"], port)?,
        neigh_suppress: json_bool_field(info, &["neigh_suppress"], port)?,
        bpdu_guard: json_bool_field(info, &["guard", "bpdu_guard"], port)?,
        root_block: json_bool_field(info, &["root_block"], port)?,
        fast_leave: json_bool_field(info, &["fastleave", "fast_leave"], port)?,
    })
}

fn json_bool_field(
    info: &serde_json::Value,
    names: &[&str],
    port: &str,
) -> Result<bool, ReconcileExecError> {
    for name in names {
        if let Some(value) = info.get(*name) {
            return match value {
                serde_json::Value::Bool(value) => Ok(*value),
                serde_json::Value::Number(value) => value
                    .as_u64()
                    .map(|value| value != 0)
                    .ok_or_else(|| ReconcileExecError::InvalidInput {
                        detail: format!("invalid bridge-port {name} value for {port}: {value}"),
                    }),
                serde_json::Value::String(value) => match value.as_str() {
                    "on" | "true" | "1" => Ok(true),
                    "off" | "false" | "0" => Ok(false),
                    other => Err(ReconcileExecError::InvalidInput {
                        detail: format!("invalid bridge-port {name} string for {port}: {other}"),
                    }),
                },
                other => Err(ReconcileExecError::InvalidInput {
                    detail: format!("invalid bridge-port {name} JSON for {port}: {other}"),
                }),
            };
        }
    }
    Err(ReconcileExecError::InvalidInput {
        detail: format!("missing bridge-port field {} for {port}", names.join("/")),
    })
}

fn bridge_port_readback_path(port: &str, leaf: &str) -> PathBuf {
    PathBuf::from(format!("bridge-port:{port}:{leaf}"))
}

impl LiveBridgePortTarget {
    fn desired_flags(&self) -> BridgePortFlagSet {
        BridgePortFlagSet::defaults_for(self.role.clone())
    }
}

const fn bool_to_ip_value(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Convenience constructor used by tests / fuzzers.
pub fn fake_backend() -> FakeBackend {
    FakeBackend::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::bundle::{Bundle, BundleGeneration};
    use nixling_core::host::{
        BridgePortFlags as HostBridgePortFlags, HostJson as BundleHostJson, IfName as BundleIfName,
        IfNameMapping, LanPolicy, NetEnv, NftablesModel, SitePolicy, TapRole, UsbipBusidLock,
        UsbipLockOwner, UsbipLockScope,
    };
    use nixling_core::manifest_v04::ManifestV04;
    use nixling_core::processes::ProcessesJson;

    fn live_bridge_flag_resolver() -> BundleResolver {
        let bundle = Bundle {
            bundle_version: 4,
            schema_version: "v2".to_owned(),
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            sync_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: Some("2025-05-30T00:00:00Z".to_owned()),
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        let host = BundleHostJson {
            schema_version: "v2".to_owned(),
            site: SitePolicy {
                allow_unsafe_east_west: false,
            },
            environments: vec![NetEnv {
                env: "work".to_owned(),
                bridge: BundleIfName::new("br-work-lan").expect("bridge ifname"),
                mtu: 1500,
                mss_clamp: None,
                lan: LanPolicy {
                    allow_east_west: false,
                    effective_east_west: false,
                },
                net_vm_forward_blocklist: Vec::new(),
                bridge_port_flags: vec![HostBridgePortFlags {
                    role: TapRole::WorkloadLan,
                    isolated: true,
                    neigh_suppress: true,
                    learning: Some(true),
                    unicast_flood: Some(false),
                    rule: "workload isolation".to_owned(),
                }],
                ipv6_sysctls: vec![nixling_core::host::Ipv6SysctlEntry {
                    if_name: BundleIfName::new("work-l10").expect("tap ifname"),
                    disable_ipv6: 1,
                    accept_ra: 0,
                    autoconf: 0,
                    addr_gen_mode: 1,
                    arp_ignore: 1,
                }],
                usbip_busid_locks: vec![UsbipBusidLock {
                    vm: "corp-vm".to_owned(),
                    lock_owner: UsbipLockOwner::Daemon,
                    scope: UsbipLockScope::PerBusid,
                    bus_ids: Vec::new(),
                    vendor_product_allowlist: Vec::new(),
                }],
            }],
            nftables: NftablesModel {
                family: "inet".to_owned(),
                table: "nixling".to_owned(),
                chains: Vec::new(),
                table_hash_after_apply: None,
                ownership_id: "nixling-test".to_owned(),
            },
            network_manager: nixling_core::host::NetworkManagerUnmanaged {
                file_path: "/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf".to_owned(),
                match_criteria: Vec::new(),
                ownership: nixling_core::host::OwnershipRule {
                    owner: "root".to_owned(),
                    group: "root".to_owned(),
                    mode: "0644".to_owned(),
                    drift_policy: "replace".to_owned(),
                },
                reload_behavior: "reload".to_owned(),
            },
            hosts_file: nixling_core::host::HostsFileOwnership {
                start_marker: "# nixling-managed begin".to_owned(),
                end_marker: "# nixling-managed end".to_owned(),
                rule: "marker-block-only".to_owned(),
            },
            kernel_modules: Vec::new(),
            fd_ownership: Vec::new(),
            runtime_providers: Vec::new(),
            vm_runtimes: Vec::new(),
            cloud_hypervisor_capabilities: Vec::new(),
            if_name_mappings: vec![
                IfNameMapping {
                    env: "work".to_owned(),
                    vm: None,
                    role: TapRole::NetVmLan,
                    user_visible_name: "br-work-lan".to_owned(),
                    derived_ifname: BundleIfName::new("nl-bWORK000").expect("derived bridge"),
                },
                IfNameMapping {
                    env: "work".to_owned(),
                    vm: Some("corp-vm".to_owned()),
                    role: TapRole::WorkloadLan,
                    user_visible_name: "work-l10".to_owned(),
                    derived_ifname: BundleIfName::new("nl-tWORK010").expect("derived tap"),
                },
            ],
            qemu_media: None,
            ch: None,
            firewall_coexistence_policy: None,
        };
        let manifest = ManifestV04::from_slice(br#"{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318},"corp-vm":{"apiSocket":"/run/nixling/corp-vm.sock","audio":false,"audioService":"nixling-corp-vm-snd.service","audioStateFile":"/var/lib/nixling/vms/corp-vm/state/audio-state.json","bridge":"br-work-lan","env":"work","gpuSocket":"/run/nixling/corp-vm-gpu.sock","graphics":false,"isNetVm":false,"name":"corp-vm","netVm":"sys-work-net","observability":{"agentSocket":"/run/nixling/otlp.sock","enabled":false,"vsockCid":110,"vsockHostSocket":"/run/nixling/corp-vm-vsock.sock"},"runtime":{"kind":"nixos","provider":{"id":"local-cloud-hypervisor","type":"local","driver":"cloud-hypervisor"},"capabilities":{"lifecycle":true,"display":true,"usbHotplug":true,"guestControl":true,"exec":true,"configSync":true,"ssh":true,"storeSync":true,"keys":true,"inGuestObservability":true}},"sshUser":"alice","stateDir":"/var/lib/nixling/vms/corp-vm","staticIp":"10.20.0.10","tap":"work-l10","tpm":false,"tpmSocket":"/run/swtpm/corp-vm/sock","usbipYubikey":false,"usbipdHostIp":"192.0.2.1"}}"#.as_slice()).expect("manifest");
        BundleResolver::from_artifacts(
            bundle,
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
        )
    }

    #[test]
    fn create_tap_fd_runs_ipv6_off_sequence() {
        let mut be = fake_backend();
        let req = CreateTapRequest {
            env: "e".into(),
            vm: Some("v".into()),
            mtu: Some(1500),
            mac: None,
            persistent_owner: None,
        };
        let audit = create_tap(&mut be, &req, TapCreateGate::applied()).unwrap();
        assert_eq!(audit.role, "tap-fd");
        assert!(!audit.ifname_derived.is_empty());
        assert_eq!(audit.nm_unmanaged_outcome, NmUnmanagedOutcome::Applied);
    }

    #[test]
    fn create_persistent_tap_sets_owner() {
        let mut be = fake_backend();
        let req = CreateTapRequest {
            env: "e".into(),
            vm: Some("v".into()),
            mtu: None,
            mac: None,
            persistent_owner: Some(TapOwner {
                uid: 1000,
                gid: 1000,
            }),
        };
        let audit = create_tap(&mut be, &req, TapCreateGate::nm_absent_coexist()).unwrap();
        assert_eq!(audit.role, "persistent-tap");
        assert!(audit.persistent);
        assert_eq!(
            audit.nm_unmanaged_outcome,
            NmUnmanagedOutcome::NotApplicableNmAbsentConfiguredCoexist
        );
    }

    #[test]
    fn create_tap_refuses_without_nm_unmanaged_gate() {
        let mut be = fake_backend();
        let req = CreateTapRequest {
            env: "e".into(),
            vm: Some("v".into()),
            mtu: None,
            mac: None,
            persistent_owner: None,
        };
        let err = create_tap(&mut be, &req, TapCreateGate::not_applied()).unwrap_err();
        match err {
            NetlinkError::Backend(msg) => {
                assert_eq!(msg, "nm-unmanaged-pre-create-required");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn set_bridge_port_flags_readback_matches() {
        let mut be = fake_backend();
        let req = CreateTapRequest {
            env: "e".into(),
            vm: Some("v".into()),
            mtu: None,
            mac: None,
            persistent_owner: None,
        };
        let _ = create_tap(&mut be, &req, TapCreateGate::applied()).unwrap();
        let ifname = derive_tap_ifname(&req).unwrap();
        let audit = set_bridge_port_flags(
            &mut be,
            &SetBridgePortFlagsRequest {
                ifname,
                role: TapRoleW3::WorkloadLanIsolated,
            },
        )
        .unwrap();
        assert!(audit.flags_after.isolated);
    }

    #[test]
    fn set_bridge_port_flags_readback_drift_fails_closed() {
        let mut be = fake_backend();
        let req = CreateTapRequest {
            env: "e".into(),
            vm: Some("v".into()),
            mtu: None,
            mac: None,
            persistent_owner: None,
        };
        let _ = create_tap(&mut be, &req, TapCreateGate::applied()).unwrap();
        // Force a foreign actor to flip the flags on readback.
        be.force_bridge_port_flags = Some(BridgePortFlagSet::ALL_OFF);
        let ifname = derive_tap_ifname(&req).unwrap();
        let err = set_bridge_port_flags(
            &mut be,
            &SetBridgePortFlagsRequest {
                ifname,
                role: TapRoleW3::WorkloadLanIsolated,
            },
        )
        .unwrap_err();
        assert!(matches!(err, NetlinkError::BridgePortFlagDrift(_)));
    }

    #[test]
    fn build_bridge_port_ip_args_uses_bridge_slave_netlink_surface() {
        let rendered = build_bridge_port_ip_args(
            "nl-tWORK010",
            BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanIsolated),
        )
        .join(" ");
        assert!(rendered.starts_with("link set dev nl-tWORK010 type bridge_slave"));
        for needle in [
            "hairpin off",
            "learning on",
            "flood off",
            "mcast_flood off",
            "neigh_suppress on",
            "guard on",
            "root_block on",
            "fastleave on",
            "isolated on",
        ] {
            assert!(rendered.contains(needle), "missing {needle} in {rendered}");
        }
    }

    #[test]
    fn parse_bridge_port_flags_json_reads_bridge_slave_readback() {
        let flags = parse_bridge_port_flags_json(
            r#"[
                {
                    "ifname": "nl-tWORK010",
                    "linkinfo": {
                        "info_slave_data": {
                            "isolated": true,
                            "hairpin": false,
                            "learning": true,
                            "flood": false,
                            "mcast_flood": false,
                            "neigh_suppress": true,
                            "guard": true,
                            "root_block": true,
                            "fastleave": true
                        }
                    }
                }
            ]"#,
            "nl-tWORK010",
        )
        .expect("parse bridge slave readback");
        assert_eq!(
            flags,
            BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanIsolated)
        );
    }

    #[test]
    fn live_set_bridge_port_flags_uses_netlink_apply_and_readback() {
        let resolver = live_bridge_flag_resolver();
        let req = nixling_ipc::broker_wire::SetBridgePortFlagsRequest {
            vm_id: nixling_ipc::types::VmId::new("corp-vm"),
            role_id: nixling_ipc::types::RoleId::new("workload-lan"),
            tracing_span_id: None,
        };
        let target = resolve_live_bridge_port_target(&resolver, &req).expect("resolve bridge port");
        let applied = std::cell::RefCell::new(Vec::new());

        let response = live_set_bridge_port_flags_with_ops(
            &target,
            |port, flags| {
                applied.borrow_mut().push((port.to_owned(), flags));
                Ok(())
            },
            |port| {
                assert_eq!(port, "nl-tWORK010");
                Ok(BridgePortFlagSet::defaults_for(
                    TapRoleW3::WorkloadLanIsolated,
                ))
            },
        )
        .expect("live bridge flags");

        assert_eq!(response.bridge.as_str(), "nl-bWORK000");
        assert_eq!(response.port.as_str(), "nl-tWORK010");
        assert!(response.isolated);
        assert!(response.neigh_suppress);
        assert_eq!(
            applied.into_inner(),
            vec![(
                "nl-tWORK010".to_owned(),
                BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanIsolated),
            )]
        );
    }

    #[test]
    fn live_set_bridge_port_flags_fails_closed_on_readback_drift() {
        let resolver = live_bridge_flag_resolver();
        let req = nixling_ipc::broker_wire::SetBridgePortFlagsRequest {
            vm_id: nixling_ipc::types::VmId::new("corp-vm"),
            role_id: nixling_ipc::types::RoleId::new("workload-lan"),
            tracing_span_id: None,
        };
        let target = resolve_live_bridge_port_target(&resolver, &req).expect("resolve bridge port");
        let err = live_set_bridge_port_flags_with_ops(
            &target,
            |_port, _flags| Ok(()),
            |_port| Ok(BridgePortFlagSet::ALL_OFF),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            LiveSetBridgePortFlagsError::ReadbackMismatch { path, .. }
                if path == PathBuf::from("bridge-port:nl-tWORK010:isolated")
        ));
    }
}
