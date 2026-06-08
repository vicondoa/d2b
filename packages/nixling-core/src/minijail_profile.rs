use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Typed minijail profile metadata referenced by process roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinijailProfile {
    /// Stable profile identifier used from processes.json.
    pub profile_id: String,
    /// Human-readable role the profile constrains.
    pub role: String,
    /// Numeric uid used after minijail drops privilege.
    pub uid: u32,
    /// Numeric gid used after minijail drops privilege.
    pub gid: u32,
    /// ADR or plan carve-out reference for uid/root-capable exceptions.
    #[serde(rename = "adr_carve_out")]
    pub adr_carve_out: Option<String>,
    /// Linux capabilities retained by the jailed process.
    pub capabilities: Vec<String>,
    /// Namespace isolation requested for this role.
    pub namespaces: NamespaceSet,
    /// Seccomp policy reference; kernel-version syscall allowlists are out of W1 scope.
    pub seccomp_policy_ref: Option<String>,
    /// Mount policy metadata, including writable paths.
    pub mount_policy: MountPolicy,
    /// Cgroup placement declaration for broker delegation.
    pub cgroup_placement: CgroupPlacement,
    /// Whether this long-lived profile may start as root under an ADR-listed exception.
    pub requires_start_root: bool,
    /// Optional ADR or plan reference justifying privileged exceptions.
    pub exception_ref: Option<String>,
}

/// Namespace flags for a minijail profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamespaceSet {
    /// Whether the mount namespace is isolated.
    pub mount: bool,
    /// Whether the PID namespace is isolated.
    pub pid: bool,
    /// Whether the network namespace is isolated.
    pub net: bool,
    /// Whether the IPC namespace is isolated.
    pub ipc: bool,
    /// Whether the UTS namespace is isolated.
    pub uts: bool,
    /// Whether a user namespace is used.
    pub user: bool,
}

/// Mount policy for a minijail role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MountPolicy {
    /// Read-only paths mounted into the jail.
    pub read_only_paths: Vec<String>,
    /// Writable paths explicitly documented for this role.
    pub writable_paths: Vec<WritablePath>,
    /// Whether `/nix/store` is visible read-only.
    pub nix_store_read_only: bool,
    /// Whether device nodes are hidden unless broker-opened fds are passed.
    pub hide_device_nodes_by_default: bool,
}

/// Writable path declaration with a purpose string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WritablePath {
    /// Absolute path made writable in the jail.
    pub path: String,
    /// Reason the path is writable.
    pub purpose: String,
}

/// Cgroup placement for a jailed role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CgroupPlacement {
    /// cgroup-v2 subtree requested for this role.
    pub subtree: String,
    /// Controllers requested by the role.
    pub controllers: Vec<String>,
    /// Whether the broker must delegate ownership to nixlingd.
    pub delegated: bool,
}
