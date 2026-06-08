//! P2 host-prep DAG: per-VM host preparation steps that the daemon
//! executes via the broker before invoking the per-VM process DAG
//! (`nixlingd::supervisor::dag`).
//!
//! Background
//! ----------
//!
//! Pre-P2 the framework relied on per-VM systemd templates
//! (`microvm-tap-interfaces@<vm>.service`, `microvm-setup@<vm>.service`,
//! `microvm-pci-devices@<vm>.service`, ...) to do tap-fd creation,
//! vhost-net fd open, dnsmasq lease seeding, store-view bind-mounts,
//! nftables rule installation, and ownership-matrix enforcement
//! before the cloud-hypervisor runner could start.
//!
//! P2 collapses those into a single typed DAG that the daemon walks
//! in topo order on every VM start. Every step dispatches a typed
//! broker op (per plan.md "P2 daemon-side host-prep replaces per-VM
//! systemd templates"); failures surface as the typed
//! [`HostPrepStepFailed`] error.
//!
//! Scope split with sibling P2 deliverables
//! ----------------------------------------
//!
//! - [`HostPrepStepKind::OwnershipMatrixCheck`] is implemented by the
//!   `ph2-p2-ownership-matrix` sibling agent
//!   (`packages/nixling-host/src/ownership_matrix.rs`).
//! - [`HostPrepStepKind::SshHostKeyPreflight`] is implemented by the
//!   `ph2-p2-ssh-host-key-preflight` sibling agent.
//!
//! This module defines the typed enum variants and dependency edges
//! so other agents can wire the broker dispatch independently. The
//! variants are *not* terminal-only; the DAG executor calls into them
//! by `HostPrepStepKind` discriminant.
//!
//! Broker-op mapping
//! -----------------
//!
//! | Step                       | Broker op                                            |
//! | -------------------------- | ---------------------------------------------------- |
//! | `BringUpTapInterface`      | [`BrokerRequest::CreateTapFd`] / `CreatePersistentTap` |
//! | `PreOpenVhostNetFd`        | [`BrokerRequest::OpenVhostNet`] (or `OpenDevice`)    |
//! | `SeedDnsmasqLease`         | [`BrokerRequest::SeedDnsmasqLease`] *(P2 stub)*      |
//! | `BindMountFromHardlinkFarm`| [`BrokerRequest::BindMountFromHardlinkFarm`] *(P2)*  |
//! | `ApplyNftablesRules`       | [`BrokerRequest::ApplyNftables`]                     |
//! | `OwnershipMatrixCheck`     | [`BrokerRequest::OwnershipMatrixCheck`] *(P2 stub)*  |
//! | `SshHostKeyPreflight`      | [`BrokerRequest::SshHostKeyPreflight`] *(P2 stub)*   |
//!
//! [`BrokerRequest::CreateTapFd`]: nixling_ipc::broker_wire::BrokerRequest::CreateTapFd
//! [`BrokerRequest::OpenVhostNet`]: nixling_ipc::broker_wire::BrokerRequest::OpenVhostNet
//! [`BrokerRequest::ApplyNftables`]: nixling_ipc::broker_wire::BrokerRequest::ApplyNftables
//! [`BrokerRequest::SeedDnsmasqLease`]: nixling_ipc::broker_wire::BrokerRequest::SeedDnsmasqLease
//! [`BrokerRequest::BindMountFromHardlinkFarm`]: nixling_ipc::broker_wire::BrokerRequest::BindMountFromHardlinkFarm
//! [`BrokerRequest::OwnershipMatrixCheck`]: nixling_ipc::broker_wire::BrokerRequest::OwnershipMatrixCheck
//! [`BrokerRequest::SshHostKeyPreflight`]: nixling_ipc::broker_wire::BrokerRequest::SshHostKeyPreflight
//!
//! Topological order (minimal fixture VM)
//! --------------------------------------
//!
//! ```text
//! SshHostKeyPreflight   OwnershipMatrixCheck
//!         \                   /
//!          \                 /
//!           +---> ApplyNftablesRules ---> BringUpTapInterface ---> PreOpenVhostNetFd
//!                       \                       /
//!                        +---> SeedDnsmasqLease (net-VM only) -> BindMountFromHardlinkFarm
//! ```
//!
//! The exact graph and the per-VM set is derived in
//! [`build_host_prep_dag`].

use nixling_core::bundle_resolver::BundleResolver;
use nixling_ipc::types::{BundleOpId, ScopeId, VmId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Stable identifier for a single host-prep step within one VM's
/// DAG. Used as the topo-sort key and as the audit/error correlator.
///
/// The string form `<vm>:<kind>` is deterministic; the broker side
/// can derive the audit `opaque_target_id` from it.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct HostPrepStepId(pub String);

impl HostPrepStepId {
    pub fn new(vm: &str, kind: HostPrepStepKind) -> Self {
        Self(format!("{vm}:{}", kind.as_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostPrepStepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Typed kind discriminant for a host-prep step.
///
/// Every variant corresponds to a documented broker op (see the
/// module-level mapping table). New variants land here in lockstep
/// with the broker wire variant they dispatch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum HostPrepStepKind {
    /// Tap-fd creation via broker. Replaces the setup body of
    /// `microvm-tap-interfaces@<vm>.service`. Dispatches
    /// `BrokerRequest::CreateTapFd` (or `CreatePersistentTap` for
    /// the persistent-tap shape). The broker chooses the runner uid
    /// / gid via `TUNSETOWNER` / `TUNSETGROUP` per the bundle's
    /// runner intent (graphics VMs need the `nixling-<vm>-gpu` uid,
    /// not `microvm`).
    BringUpTapInterface,
    /// vhost-net fd open via broker `OpenVhostNet` (or
    /// `OpenDevice(vhost-net)`). The fd is later handed off through
    /// SCM_RIGHTS to the cloud-hypervisor runner so the runner
    /// itself does not need `CAP_NET_ADMIN`.
    PreOpenVhostNetFd,
    /// Per-VM dnsmasq lease seeding. Replaces the leaves of
    /// `microvm-setup@<vm>.service` that wrote
    /// `/var/lib/nixling/dnsmasq/<vm>.leases`.
    ///
    /// **Net-VM-only**: workload VMs do not run dnsmasq, so the
    /// builder skips this step for them.
    SeedDnsmasqLease,
    /// Per-VM `/var/lib/nixling/vms/<vm>/store-view` bind-mount
    /// from the per-VM hardlink farm (`store/`). Dispatches the
    /// `BindMountFromHardlinkFarm` broker op (P2 stub).
    BindMountFromHardlinkFarm,
    /// nftables fragment apply via broker `ApplyNftables`. Already
    /// fully typed in W3; here we just register it as a DAG step.
    /// The per-VM rules live in the bundle's `nft:env:<env>` intent.
    ApplyNftablesRules,
    /// Defence-in-depth ownership/mode/setgid invariants for
    /// `/var/lib/nixling/vms/<vm>/`. Owned by sibling P2 agent
    /// (`ph2-p2-ownership-matrix`). The dispatch arm here is a
    /// typed placeholder; the sibling lands the real check.
    OwnershipMatrixCheck,
    /// VM start preflight: refuse if
    /// `/var/lib/nixling/vms/<vm>/sshd-host-keys/ssh_host_*_key`
    /// opened with `O_NOFOLLOW` drifts from `root:root 0400`
    /// (rejects symlinks, owner/group mismatch, mode mismatch).
    /// Owned by sibling P2 agent (`ph2-p2-ssh-host-key-preflight`).
    SshHostKeyPreflight,
    // ---- P2fu1 kernel-r1-1 closure: explicit ordering for the
    // microvm-tap-interfaces / microvm-setup tap path ----
    /// Mark the per-VM tap parent bridge interfaces as unmanaged in
    /// NetworkManager via broker `ApplyNmUnmanaged`. Must run BEFORE
    /// tap creation so NetworkManager doesn't race the broker's
    /// `TUNSETIFF` + immediate `dev set master` and pull the link
    /// down between create + attach. Replaces the
    /// `NetworkManager.conf.d/00-nixling-unmanaged.conf` materializer
    /// leaf of `microvm-setup@<vm>.service`.
    ApplyNmUnmanaged,
    /// Apply the per-VM sysctl set (RP filter, forwarding, MSS clamp
    /// thresholds, ARP responder mode). Must run AFTER tap creation
    /// (so per-tap sysctls under
    /// `/proc/sys/net/ipv4/conf/<ifname>/` exist) and BEFORE
    /// SetBridgePortFlags. Replaces the sysctl-apply leaf of
    /// `microvm-setup@<vm>.service`.
    ApplySysctl,
    /// Set bridge-port flags on the tap (e.g. `learning off`,
    /// `flood off`, `mcast_to_unicast off`) after tap attach.
    /// Replaces the `bridge link set` leaf of
    /// `microvm-tap-interfaces@<vm>.service`. Must run AFTER
    /// tap creation + bridge attach + sysctl apply.
    SetBridgePortFlags,
    /// P3 `ph3-p3-net-route-degraded-mode`: daemon-side host-scope
    /// preflight that verifies each env's LAN bridge exists and is
    /// administratively up. Replaces the legacy
    /// `nixling-net-route-preflight.service` host singleton
    /// (scheduled for removal in P6). This is a typed-only step in
    /// the host-prep enum: the daemon executes the check in its
    /// startup path and in `dispatch_broker_host_reconcile`; it is
    /// NOT scheduled per-VM in the DAG today.
    HostNetRoutePreflight,
}

impl HostPrepStepKind {
    /// Stable kebab-case string used in audit records, error
    /// envelopes, and [`HostPrepStepId`] keys.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BringUpTapInterface => "bring-up-tap-interface",
            Self::PreOpenVhostNetFd => "pre-open-vhost-net-fd",
            Self::SeedDnsmasqLease => "seed-dnsmasq-lease",
            Self::BindMountFromHardlinkFarm => "bind-mount-from-hardlink-farm",
            Self::ApplyNftablesRules => "apply-nftables-rules",
            Self::OwnershipMatrixCheck => "ownership-matrix-check",
            Self::SshHostKeyPreflight => "ssh-host-key-preflight",
            Self::ApplyNmUnmanaged => "apply-nm-unmanaged",
            Self::ApplySysctl => "apply-sysctl",
            Self::SetBridgePortFlags => "set-bridge-port-flags",
            Self::HostNetRoutePreflight => "host-net-route-preflight",
        }
    }

    /// Name of the broker op this step dispatches. Used in
    /// [`HostPrepStepFailed::op_kind`] for the operator-facing
    /// error envelope.
    pub fn broker_op_name(&self) -> &'static str {
        match self {
            Self::BringUpTapInterface => "CreateTapFd",
            Self::PreOpenVhostNetFd => "OpenVhostNet",
            Self::SeedDnsmasqLease => "SeedDnsmasqLease",
            Self::BindMountFromHardlinkFarm => "BindMountFromHardlinkFarm",
            Self::ApplyNftablesRules => "ApplyNftables",
            Self::OwnershipMatrixCheck => "OwnershipMatrixCheck",
            Self::SshHostKeyPreflight => "SshHostKeyPreflight",
            Self::ApplyNmUnmanaged => "ApplyNmUnmanaged",
            Self::ApplySysctl => "ApplySysctl",
            Self::SetBridgePortFlags => "SetBridgePortFlags",
            // No broker op: executed inline in the daemon (see
            // `nixlingd::net_route_preflight`). Reported here for
            // audit symmetry with other host-scope steps.
            Self::HostNetRoutePreflight => "HostNetRoutePreflight",
        }
    }
}

/// Opaque pointer into the trusted bundle for a single host-prep
/// step (per W4-H5 bundle-resolved intent contract: the daemon
/// never names raw paths/uids/argv on the broker wire).
///
/// The shape mirrors the W3 `BundleOpId` discipline; each step
/// carries the IDs the broker needs to look up its intent row in
/// its own copy of the bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleStepRef {
    /// Opaque VM id this step targets (resolved against
    /// `bundle.vms[<vm_id>]`).
    pub vm_id: VmId,
    /// Optional authorization scope (env / VM). Present for steps
    /// that scope to an env (`ApplyNftablesRules` uses
    /// `ScopeId::new("env:<env>")`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<ScopeId>,
    /// Optional opaque intent reference for steps that look up a
    /// specific bundle intent row (e.g. `nft:env:<env>` for
    /// `ApplyNftablesRules`, `runner:vm:<vm>:role:<role>` for tap
    /// ownership derivation). `None` for steps whose entire
    /// payload is derived from `vm_id` alone (e.g.
    /// `SshHostKeyPreflight`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_op_id: Option<BundleOpId>,
}

/// One node in the host-prep DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPrepStep {
    pub id: HostPrepStepId,
    /// Edges expressed as "this step depends on these step ids".
    /// Topo-sort guarantees all of these have completed before the
    /// step is dispatched.
    #[serde(default)]
    pub depends_on: Vec<HostPrepStepId>,
    pub kind: HostPrepStepKind,
    pub bundle_ref: BundleStepRef,
}

/// Typed error surfaced when a host-prep step fails. The daemon
/// converts this into the public error envelope; the operator sees
/// `step_id`, the broker op that was dispatched, and the raw broker
/// error string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPrepStepFailed {
    pub step_id: HostPrepStepId,
    pub op_kind: String,
    pub broker_error: String,
}

impl fmt::Display for HostPrepStepFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "host-prep step {} ({}) failed: {}",
            self.step_id, self.op_kind, self.broker_error
        )
    }
}

impl std::error::Error for HostPrepStepFailed {}

/// Cycle / unknown-edge error from [`topo_sort`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum CycleError {
    /// At least one dependency cycle exists; `residual` is the set
    /// of step ids that could not be sequenced.
    Cycle { residual: Vec<HostPrepStepId> },
    /// A `depends_on` entry referenced an id not present in the
    /// step list.
    UnknownDependency {
        step_id: HostPrepStepId,
        missing: HostPrepStepId,
    },
    /// Two steps share an id.
    DuplicateStep { step_id: HostPrepStepId },
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cycle { residual } => {
                write!(f, "host-prep DAG has a cycle; residual nodes: {residual:?}")
            }
            Self::UnknownDependency { step_id, missing } => write!(
                f,
                "host-prep step {step_id} depends on unknown id {missing}"
            ),
            Self::DuplicateStep { step_id } => {
                write!(f, "host-prep DAG has duplicate step id {step_id}")
            }
        }
    }
}

impl std::error::Error for CycleError {}

/// Build the host-prep DAG for one VM, returning the topo-sorted
/// step list.
///
/// The set of steps is derived from the VM's properties in the
/// trusted bundle:
///
/// - Every VM emits `SshHostKeyPreflight`, `OwnershipMatrixCheck`,
///   `ApplyNftablesRules`, `BringUpTapInterface`, `PreOpenVhostNetFd`,
///   and `BindMountFromHardlinkFarm`.
/// - Net VMs (`VmEntry::is_net_vm`) additionally emit
///   `SeedDnsmasqLease`.
///
/// Steps unrelated to the VM's optional sidecars (obs / usbip /
/// gpu) are intentionally not part of the host-prep DAG — those are
/// handled inside the per-VM process DAG (`supervisor::dag`) once
/// host-prep completes.
///
/// # Panics
///
/// Does not panic. Returns an empty vector if the VM is unknown to
/// the resolver (the daemon-side caller is responsible for
/// surfacing that as a typed error).
pub fn build_host_prep_dag(vm: &str, resolver: &BundleResolver) -> Vec<HostPrepStep> {
    let Some(vm_entry) = resolver.find_manifest_vm(vm) else {
        return Vec::new();
    };
    build_host_prep_dag_for(vm, vm_entry.is_net_vm, vm_entry.env.as_deref())
}

/// Bundle-free constructor used by unit tests and integrators that
/// already know the VM's net-VM flag + env. Keeps the production
/// `build_host_prep_dag` thin and tests hermetic.
pub fn build_host_prep_dag_for(vm: &str, is_net_vm: bool, env: Option<&str>) -> Vec<HostPrepStep> {
    let vm_id = VmId::new(vm.to_string());
    let env_scope = env.map(|e| ScopeId::new(format!("env:{e}")));
    let nft_intent = env.map(|e| BundleOpId::new(format!("nft:env:{e}")));

    let id = |k: HostPrepStepKind| HostPrepStepId::new(vm, k);

    let mut steps = Vec::with_capacity(10);

    // Preflights — no upstream deps; siblings of one another.
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::SshHostKeyPreflight),
        depends_on: vec![],
        kind: HostPrepStepKind::SshHostKeyPreflight,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: None,
            bundle_op_id: None,
        },
    });
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::OwnershipMatrixCheck),
        depends_on: vec![],
        kind: HostPrepStepKind::OwnershipMatrixCheck,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: None,
            bundle_op_id: None,
        },
    });

    // P2fu1 kernel-r1-1: NetworkManager unmanage must run BEFORE tap
    // creation so NM doesn't race the broker's TUNSETIFF + master-set
    // and drop the link between create + attach. Replaces the
    // 00-nixling-unmanaged.conf materializer leaf of microvm-setup.
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::ApplyNmUnmanaged),
        depends_on: vec![],
        kind: HostPrepStepKind::ApplyNmUnmanaged,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: env_scope.clone(),
            bundle_op_id: None,
        },
    });

    // Nftables: per-env scope, gated on both preflights + NM unmanage
    // (so the chain exists AND the tap-parent bridge is daemon-owned
    // before the tap is added to it).
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::ApplyNftablesRules),
        depends_on: vec![
            id(HostPrepStepKind::SshHostKeyPreflight),
            id(HostPrepStepKind::OwnershipMatrixCheck),
            id(HostPrepStepKind::ApplyNmUnmanaged),
        ],
        kind: HostPrepStepKind::ApplyNftablesRules,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: env_scope.clone(),
            bundle_op_id: nft_intent,
        },
    });

    // Tap: depends on nftables (so the chain exists before the tap
    // is added to the bridge).
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::BringUpTapInterface),
        depends_on: vec![id(HostPrepStepKind::ApplyNftablesRules)],
        kind: HostPrepStepKind::BringUpTapInterface,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: env_scope.clone(),
            // Runner intent carries TUNSETOWNER target uid/gid.
            bundle_op_id: Some(BundleOpId::new(format!("runner:vm:{vm}:role:ch"))),
        },
    });

    // P2fu1 kernel-r1-1: sysctl-apply runs AFTER tap creation so the
    // per-tap /proc/sys/net/ipv4/conf/<ifname>/ entries exist when
    // the sysctl writer iterates them. Replaces sysctl-apply leaf
    // of microvm-setup.
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::ApplySysctl),
        depends_on: vec![id(HostPrepStepKind::BringUpTapInterface)],
        kind: HostPrepStepKind::ApplySysctl,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: env_scope.clone(),
            bundle_op_id: None,
        },
    });

    // P2fu1 kernel-r1-1: bridge-port flags (learning off, flood off,
    // mcast_to_unicast off) AFTER sysctls so the flag set reflects
    // the final per-tap config. Replaces bridge-link-set leaf of
    // microvm-tap-interfaces.
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::SetBridgePortFlags),
        depends_on: vec![id(HostPrepStepKind::ApplySysctl)],
        kind: HostPrepStepKind::SetBridgePortFlags,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: env_scope.clone(),
            bundle_op_id: None,
        },
    });

    // vhost-net fd: depends on the tap (and post-tap bridge flags
    // are now in their stable state) so the runner gets both fds
    // together with the bridge-port flags already pinned.
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::PreOpenVhostNetFd),
        depends_on: vec![id(HostPrepStepKind::SetBridgePortFlags)],
        kind: HostPrepStepKind::PreOpenVhostNetFd,
        bundle_ref: BundleStepRef {
            vm_id: vm_id.clone(),
            scope_id: env_scope.clone(),
            bundle_op_id: None,
        },
    });

    if is_net_vm {
        steps.push(HostPrepStep {
            id: id(HostPrepStepKind::SeedDnsmasqLease),
            depends_on: vec![id(HostPrepStepKind::ApplyNftablesRules)],
            kind: HostPrepStepKind::SeedDnsmasqLease,
            bundle_ref: BundleStepRef {
                vm_id: vm_id.clone(),
                scope_id: env_scope.clone(),
                bundle_op_id: None,
            },
        });
    }

    // Per-VM store-view bind: depends on ownership matrix (parent
    // dir must already be correct mode/owner). Tap-independent.
    steps.push(HostPrepStep {
        id: id(HostPrepStepKind::BindMountFromHardlinkFarm),
        depends_on: vec![id(HostPrepStepKind::OwnershipMatrixCheck)],
        kind: HostPrepStepKind::BindMountFromHardlinkFarm,
        bundle_ref: BundleStepRef {
            vm_id,
            scope_id: None,
            bundle_op_id: None,
        },
    });

    topo_sort(steps).expect("static host-prep DAG is acyclic")
}

/// Pure topological sort with cycle + dangling-edge detection.
/// Tie-break is by [`HostPrepStepId`] string order so the output
/// is deterministic across daemon restarts.
pub fn topo_sort(steps: Vec<HostPrepStep>) -> Result<Vec<HostPrepStep>, CycleError> {
    let mut by_id: BTreeMap<HostPrepStepId, HostPrepStep> = BTreeMap::new();
    for step in steps {
        if by_id.contains_key(&step.id) {
            return Err(CycleError::DuplicateStep {
                step_id: step.id.clone(),
            });
        }
        by_id.insert(step.id.clone(), step);
    }

    let mut indeg: BTreeMap<HostPrepStepId, usize> = BTreeMap::new();
    let mut rev: BTreeMap<HostPrepStepId, BTreeSet<HostPrepStepId>> = BTreeMap::new();
    for id in by_id.keys() {
        indeg.entry(id.clone()).or_insert(0);
    }
    for step in by_id.values() {
        for dep in &step.depends_on {
            if !by_id.contains_key(dep) {
                return Err(CycleError::UnknownDependency {
                    step_id: step.id.clone(),
                    missing: dep.clone(),
                });
            }
            *indeg.entry(step.id.clone()).or_insert(0) += 1;
            rev.entry(dep.clone()).or_default().insert(step.id.clone());
        }
    }

    let mut ready: VecDeque<HostPrepStepId> = indeg
        .iter()
        .filter_map(|(id, n)| (*n == 0).then(|| id.clone()))
        .collect();
    // BTreeMap iteration is already ordered, so `ready` is sorted.

    let mut out: Vec<HostPrepStep> = Vec::with_capacity(by_id.len());
    while let Some(id) = ready.pop_front() {
        if let Some(step) = by_id.remove(&id) {
            out.push(step);
        }
        if let Some(downstream) = rev.remove(&id) {
            // BTreeSet iteration is sorted; sort siblings deterministically.
            for next in downstream {
                if let Some(n) = indeg.get_mut(&next) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        ready.push_back(next);
                    }
                }
            }
        }
    }

    if !by_id.is_empty() {
        let residual: Vec<HostPrepStepId> = by_id.into_keys().collect();
        return Err(CycleError::Cycle { residual });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_vm_minimal_fixture_step_set_and_order() {
        let steps = build_host_prep_dag_for("work", false, Some("work"));
        let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        // Net-VM-only step must be absent.
        assert!(!ids.contains(&"work:seed-dnsmasq-lease"));
        // All others present.
        for k in [
            HostPrepStepKind::SshHostKeyPreflight,
            HostPrepStepKind::OwnershipMatrixCheck,
            HostPrepStepKind::ApplyNftablesRules,
            HostPrepStepKind::BringUpTapInterface,
            HostPrepStepKind::PreOpenVhostNetFd,
            HostPrepStepKind::BindMountFromHardlinkFarm,
        ] {
            let expected = format!("work:{}", k.as_str());
            assert!(
                ids.iter().any(|i| *i == expected),
                "missing step {expected} in {ids:?}"
            );
        }
        assert_topo_valid(&steps);
        // Specific ordering invariants from the doc.
        assert_before(
            &steps,
            "work:apply-nftables-rules",
            "work:bring-up-tap-interface",
        );
        assert_before(
            &steps,
            "work:bring-up-tap-interface",
            "work:pre-open-vhost-net-fd",
        );
        assert_before(
            &steps,
            "work:ssh-host-key-preflight",
            "work:apply-nftables-rules",
        );
        assert_before(
            &steps,
            "work:ownership-matrix-check",
            "work:apply-nftables-rules",
        );
        assert_before(
            &steps,
            "work:ownership-matrix-check",
            "work:bind-mount-from-hardlink-farm",
        );
    }

    #[test]
    fn net_vm_fixture_adds_seed_dnsmasq_lease() {
        let steps = build_host_prep_dag_for("sys-work-net", true, Some("work"));
        let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"sys-work-net:seed-dnsmasq-lease"));
        assert_topo_valid(&steps);
        assert_before(
            &steps,
            "sys-work-net:apply-nftables-rules",
            "sys-work-net:seed-dnsmasq-lease",
        );
    }

    #[test]
    fn topo_sort_detects_cycle() {
        let a = HostPrepStepId("a".to_string());
        let b = HostPrepStepId("b".to_string());
        let steps = vec![
            HostPrepStep {
                id: a.clone(),
                depends_on: vec![b.clone()],
                kind: HostPrepStepKind::OwnershipMatrixCheck,
                bundle_ref: BundleStepRef {
                    vm_id: VmId::new("vm"),
                    scope_id: None,
                    bundle_op_id: None,
                },
            },
            HostPrepStep {
                id: b.clone(),
                depends_on: vec![a.clone()],
                kind: HostPrepStepKind::OwnershipMatrixCheck,
                bundle_ref: BundleStepRef {
                    vm_id: VmId::new("vm"),
                    scope_id: None,
                    bundle_op_id: None,
                },
            },
        ];
        let err = topo_sort(steps).expect_err("cycle must surface");
        match err {
            CycleError::Cycle { residual } => {
                assert!(residual.contains(&a) && residual.contains(&b));
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn topo_sort_rejects_unknown_dependency() {
        let a = HostPrepStepId("a".to_string());
        let missing = HostPrepStepId("ghost".to_string());
        let steps = vec![HostPrepStep {
            id: a.clone(),
            depends_on: vec![missing.clone()],
            kind: HostPrepStepKind::OwnershipMatrixCheck,
            bundle_ref: BundleStepRef {
                vm_id: VmId::new("vm"),
                scope_id: None,
                bundle_op_id: None,
            },
        }];
        let err = topo_sort(steps).expect_err("must reject");
        assert!(matches!(err, CycleError::UnknownDependency { .. }));
    }

    #[test]
    fn topo_sort_rejects_duplicate_id() {
        let a = HostPrepStepId("a".to_string());
        let step = HostPrepStep {
            id: a.clone(),
            depends_on: vec![],
            kind: HostPrepStepKind::OwnershipMatrixCheck,
            bundle_ref: BundleStepRef {
                vm_id: VmId::new("vm"),
                scope_id: None,
                bundle_op_id: None,
            },
        };
        let err = topo_sort(vec![step.clone(), step]).expect_err("must reject");
        assert!(matches!(err, CycleError::DuplicateStep { .. }));
    }

    #[test]
    fn step_kind_serde_round_trip() {
        for k in [
            HostPrepStepKind::BringUpTapInterface,
            HostPrepStepKind::PreOpenVhostNetFd,
            HostPrepStepKind::SeedDnsmasqLease,
            HostPrepStepKind::BindMountFromHardlinkFarm,
            HostPrepStepKind::ApplyNftablesRules,
            HostPrepStepKind::OwnershipMatrixCheck,
            HostPrepStepKind::SshHostKeyPreflight,
        ] {
            let json = serde_json::to_string(&k).expect("serialize kind");
            let back: HostPrepStepKind = serde_json::from_str(&json).expect("deserialize kind");
            assert_eq!(k, back, "round-trip kind={k:?} json={json}");
        }
    }

    #[test]
    fn step_serde_round_trip_through_wire() {
        let steps = build_host_prep_dag_for("work", false, Some("work"));
        let json = serde_json::to_string(&steps).expect("serialize");
        let back: Vec<HostPrepStep> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(steps, back);
    }

    #[test]
    fn step_id_format_is_vm_colon_kind() {
        let id = HostPrepStepId::new("work", HostPrepStepKind::BringUpTapInterface);
        assert_eq!(id.as_str(), "work:bring-up-tap-interface");
    }

    #[test]
    fn step_failed_implements_error_trait() {
        let e = HostPrepStepFailed {
            step_id: HostPrepStepId::new("work", HostPrepStepKind::ApplyNftablesRules),
            op_kind: "ApplyNftables".into(),
            broker_error: "broker returned Unimplemented".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("work:apply-nftables-rules"));
        assert!(s.contains("ApplyNftables"));
    }

    fn assert_before(steps: &[HostPrepStep], earlier: &str, later: &str) {
        let pos_e = steps.iter().position(|s| s.id.as_str() == earlier);
        let pos_l = steps.iter().position(|s| s.id.as_str() == later);
        match (pos_e, pos_l) {
            (Some(e), Some(l)) => assert!(e < l, "{earlier} must precede {later}"),
            other => panic!("missing step in DAG: {other:?}"),
        }
    }

    fn assert_topo_valid(steps: &[HostPrepStep]) {
        let mut seen: BTreeSet<HostPrepStepId> = BTreeSet::new();
        for step in steps {
            for dep in &step.depends_on {
                assert!(
                    seen.contains(dep),
                    "step {} dispatched before dep {dep}",
                    step.id
                );
            }
            seen.insert(step.id.clone());
        }
    }
}
