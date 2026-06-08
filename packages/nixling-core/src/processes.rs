use crate::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-VM process DAG and lifecycle invariants from ADR 0004.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessesJson {
    /// Schema version used by this artifact.
    pub schema_version: String,
    /// Per-VM process DAGs.
    pub vms: Vec<VmProcessDag>,
}

/// Process DAG for one VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmProcessDag {
    /// VM name from the public manifest.
    pub vm: String,
    /// Ordered role nodes in the DAG.
    pub nodes: Vec<ProcessNode>,
    /// Dependency edges between DAG nodes.
    pub edges: Vec<DagEdge>,
    /// v0.4.0 invariants that must hold for this VM.
    pub invariants: VmProcessInvariants,
}

/// Stable DAG node identifier.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct NodeId(pub String);

/// Single process role in a VM DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessNode {
    /// Stable node id used by edges.
    pub id: NodeId,
    /// Role kind used by orchestration and minijail profile selection.
    pub role: ProcessRole,
    /// Optional systemd unit backing this node.
    pub unit: Option<String>,
    /// Absolute execve path for daemon-spawned roles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    /// Full argv for daemon-spawned roles; `argv[0]` is the process title.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    /// Typed minijail metadata for this role.
    pub profile: RoleProfile,
    /// Readiness predicates that mark the role available.
    pub readiness: Vec<ReadinessPredicate>,
}

/// Known role types in the ADR 0004 process graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessRole {
    /// Host reconciliation before VM-specific startup.
    HostReconcile,
    /// Store and virtiofs preflight validation.
    StoreVirtiofsPreflight,
    /// swtpm pre-start flush step.
    SwtpmPreStartFlush,
    /// swtpm sidecar.
    Swtpm,
    /// virtiofsd sidecar.
    Virtiofsd,
    /// Optional video sidecar.
    Video,
    /// Optional GPU/graphics sidecar.
    Gpu,
    /// Optional audio sidecar.
    Audio,
    /// Cloud Hypervisor runner.
    CloudHypervisorRunner,
    /// vsock relay sidecar.
    VsockRelay,
    /// Guest SSH readiness probe.
    GuestSshReadiness,
    /// USBIP proxy or attach helper.
    Usbip,
}

/// Role-level minijail metadata without kernel-version syscall allowlists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleProfile {
    /// Profile reference shared with minijail-profile.json.
    pub profile_id: String,
    /// Numeric uid used by the role.
    pub uid: u32,
    /// Numeric gid used by the role.
    pub gid: u32,
    /// ADR or plan carve-out reference for uid/root-capable exceptions.
    #[serde(rename = "adr_carve_out")]
    pub adr_carve_out: Option<String>,
    /// Linux capabilities retained by the role.
    pub caps: Vec<String>,
    /// Namespace isolation metadata.
    pub namespaces: NamespaceSet,
    /// Seccomp policy reference only; syscall allowlists are W3-owned.
    pub seccomp_policy_ref: Option<String>,
    /// Mount policy metadata.
    pub mount_policy: MountPolicy,
    /// cgroup-v2 placement and delegation metadata.
    pub cgroup_placement: CgroupPlacement,
}

/// Directed dependency edge in the per-VM DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DagEdge {
    /// Predecessor node.
    pub from: NodeId,
    /// Successor node.
    pub to: NodeId,
    /// Why the dependency exists.
    pub reason: String,
}

/// Readiness predicates used by orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "value")]
pub enum ReadinessPredicate {
    /// Cloud Hypervisor API socket info is available.
    ApiSocketInfo(String),
    /// Guest or sidecar sent a vsock readiness notification.
    VsockNotify(String),
    /// A Unix socket path exists.
    UnixSocketExists(String),
    /// A TCP port accepts connections.
    TcpPort { host: String, port: u16 },
    /// A command exits successfully.
    Command(Vec<String>),
    /// Component-specific predicate named by the emitter.
    ComponentSpecific(String),
}

/// v0.4.0 invariants preserved in the W1 process contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmProcessInvariants {
    /// swtpm pre-start uses `swtpm_ioctl -i` boot+shutdown flush.
    pub swtpm_pre_start_flush: bool,
    /// Every VM participates in the audit pipeline.
    pub per_vm_audit_pipeline: bool,
    /// USBIP is gated by host and VM opt-ins plus env scoping.
    pub usbip_gating: bool,
    /// TPM ownership migration avoids touching running VMs.
    pub tpm_ownership_migration_without_running_vm_mutation: bool,
}
