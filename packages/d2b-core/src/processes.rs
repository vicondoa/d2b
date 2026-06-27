use crate::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    /// v1.1.1fu11 (Option B): environment variables for the
    /// spawned runner in `KEY=VALUE` form. Used to thread
    /// session-resource paths (`PIPEWIRE_RUNTIME_DIR`,
    /// `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`) for audio/gpu/
    /// video sidecars whose libpipewire / libwayland clients
    /// can't auto-discover them under the ephemeral runner
    /// UID.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// v1.2 broker-executed plan operations that run before the runner
    /// spawns. The daemon sends only the opaque `vm_id`;
    /// the broker resolves these ops from the trusted bundle.
    ///
    /// Backward-compatible: omitted from JSON when empty (serde default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_ops: Vec<SpawnRunnerPlanOp>,
    /// Typed minijail metadata for this role.
    pub profile: RoleProfile,
    /// Readiness predicates that mark the role available.
    pub readiness: Vec<ReadinessPredicate>,
}

/// v1.2 additive plan operation emitted by `processes.json` for runner
/// nodes that require broker-side pre-spawn work.
///
/// Tag-dispatched via `#[serde(tag = "kind")]` so the manifest JSON
/// uses `{ "kind": "diskInit", ... }` shape (camelCase per bundle
/// schema conventions).
///
/// Backward-compatible per invariant I4: the field is absent from JSON
/// when empty (`skip_serializing_if = "Vec::is_empty"` on
/// `ProcessNode::plan_ops`), so older bundles always deserialize
/// cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SpawnRunnerPlanOp {
    /// Create and pre-allocate a disk image file if absent; validate and
    /// safely repair declared owner/mode posture drift or a trusted empty
    /// image when present.
    ///
    /// Used for d2b-owned raw ext4 volumes and the per-VM writable
    /// store overlay disk (`store-overlay.img`). The broker validates
    /// `target_path` is under `/var/lib/d2b/vms/`, creates absent
    /// files with `O_CREAT|O_EXCL`, pre-allocates `size_bytes` via
    /// `fallocate`, formats them as ext4, and sets mode + ownership.
    /// Existing files are not accepted on path existence alone: they
    /// must carry an ext4 superblock or be proven empty before broker-side
    /// repair, and stale owner/mode posture is repaired only after fd-bound
    /// identity checks.
    #[serde(rename_all = "camelCase")]
    DiskInit {
        /// Absolute host path for the new disk image.
        target_path: PathBuf,
        /// Pre-allocated size in bytes (broker calls `fallocate`).
        size_bytes: u64,
        /// Unix permission bits in octal (e.g. `0o600` = 384 decimal).
        mode: u32,
        /// Owner UID — typically the per-VM runner UID.
        owner_uid: u32,
        /// Owner GID.
        owner_gid: u32,
        /// When `true`, make re-runs idempotent by validating an
        /// existing file before skipping; declared posture drift is repaired
        /// only for a safe held fd, and a present but unformatted file is
        /// repaired only when it is proven empty. Otherwise the broker fails
        /// closed.
        if_absent: bool,
    },
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
    /// Optional GPU/graphics sidecar — render-node-only mode (fd-passing, no device bind-mounts).
    /// Selected when `graphics.renderNodeOnly = true`; uses broker-pre-NS pattern (ADR 0021).
    GpuRenderNode,
    /// Optional audio sidecar.
    Audio,
    /// Cloud Hypervisor runner.
    CloudHypervisorRunner,
    /// QEMU media runner.
    QemuMediaRunner,
    /// vsock relay sidecar.
    VsockRelay,
    /// Host-to-observability-VM OTLP bridge.
    OtelHostBridge,
    /// Guest SSH readiness probe.
    ///
    /// Retained for the SSH-compatibility window (old-generation VMs that
    /// predate guestd). New generations use [`ProcessRole::GuestControlHealth`]
    /// for framework readiness instead.
    GuestSshReadiness,
    /// Authenticated guest-control Health readiness probe.
    ///
    /// Replaces [`ProcessRole::GuestSshReadiness`] as the framework readiness
    /// gate on guest-control-capable VMs: readiness is a full authenticated
    /// Hello + token challenge-response + Health over the guest-control vsock,
    /// not a raw TCP-22 probe. It fails CLOSED.
    GuestControlHealth,
    /// USBIP proxy or attach helper.
    Usbip,
    /// Host-jailed Wayland filter proxy. Runs as `d2b-<vm>-wlproxy`
    /// with the real host compositor socket bound read/write at a fixed
    /// in-jail upstream path and the per-VM filter socket at
    /// `/run/d2b-wlproxy/<vm>`. Empty host capabilities; mandatory
    /// `seccompPolicyRef`; no PipeWire/Pulse socket access.
    WaylandProxy,
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
    /// Seccomp policy reference only; syscall allowlists are owned elsewhere.
    pub seccomp_policy_ref: Option<String>,
    /// Mount policy metadata.
    pub mount_policy: MountPolicy,
    /// cgroup-v2 placement and delegation metadata.
    pub cgroup_placement: CgroupPlacement,
    /// v1.1.1fu14 (ADR 0021): when `Some`, the broker
    /// pre-establishes a per-runner user namespace and writes
    /// uid_map/gid_map. The child runs fake-root inside the NS;
    /// host-side `caps` should be empty. Set by virtiofsd
    /// (v1.1.2 ADR 0021) and swtpm (v1.2) roles.
    /// Older bundles omit this field; deserialize defaults to None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_namespace: Option<RoleUserNamespace>,
    /// v1.1.2fu36: file-creation mask installed in the spawned
    /// child before execve. See `MinijailProfile::umask` for the
    /// rationale. Roles that bind shared Unix sockets
    /// (vhost-user-sound, crosvm-gpu, crosvm video, swtpm) declare
    /// `0o007` so downstream consumers (cloud-hypervisor) can
    /// connect via the per-VM-runtime default ACL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub umask: Option<u32>,
}

/// v1.1.1fu14 — single-entry uid_map/gid_map declaration for the
/// per-role user namespace. See ADR 0021.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleUserNamespace {
    pub host_uid_for_zero: u32,
    pub host_gid_for_zero: u32,
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
    /// A Unix stream socket is present and listening, checked non-destructively.
    UnixSocketListening(String),
    /// A TCP port accepts connections.
    TcpPort { host: String, port: u16 },
    /// A command exits successfully.
    Command(Vec<String>),
    /// Component-specific predicate named by the emitter.
    ComponentSpecific(String),
    /// Authenticated guest-control Health probe. Readiness requires a
    /// full Hello + token challenge-response + Health over the guest-control
    /// vsock — the host-side probe. Unlike [`Self::ComponentSpecific`]
    /// (which reports ready unconditionally and would fail OPEN), this predicate
    /// fails CLOSED: it is ready only when the daemon completes the
    /// authenticated probe and the guest reports a healthy/degraded state, and
    /// never ready for an old-generation / unreachable / auth-failed / timed-out
    /// guest. The daemon resolves the per-VM vsock socket, peer credentials, and
    /// broker-backed signer from its own trusted state.
    GuestControlHealth { vm: String },
}

/// v0.4.0 invariants preserved in the process contract.
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
