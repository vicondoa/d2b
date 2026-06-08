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
    /// Seccomp policy reference; kernel-version syscall allowlists are out of scope.
    pub seccomp_policy_ref: Option<String>,
    /// Mount policy metadata, including writable paths.
    pub mount_policy: MountPolicy,
    /// Cgroup placement declaration for broker delegation.
    pub cgroup_placement: CgroupPlacement,
    /// Whether this long-lived profile may start as root under an ADR-listed exception.
    pub requires_start_root: bool,
    /// Optional ADR or plan reference justifying privileged exceptions.
    pub exception_ref: Option<String>,
    /// v1.1.1fu14 (ADR 0021): when `Some`, the broker pre-establishes
    /// a single-entry user namespace for runners using this profile.
    /// The child is fake-root inside the namespace (all caps within
    /// the user-NS scope) and the host-side `capabilities` set
    /// should typically be empty. Currently consumed by virtiofsd
    /// roles for least-privilege FS serving without CAP_DAC_*.
    /// Profiles that don't need a user NS omit this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_namespace: Option<UserNamespaceProfile>,
    /// v1.1.2fu36: file-creation mask the broker installs in the
    /// spawned child before execve. Lets profiles that bind shared
    /// sockets (vhost-user-sound, crosvm-gpu, crosvm video, swtpm)
    /// declare an umask of `0o007` so the resulting sockets have
    /// group r-w (mode 0660/0770) and the existing default ACL on the
    /// per-VM runtime dir yields effective r-w for named-user entries
    /// (cloud-hypervisor uid). Without this,
    /// the default 0o022 umask yields socket mode 0644 — but
    /// vhost-user/swtpm bind() typically restricts to 0600 — which
    /// derives ACL mask=0 and named-user entries become ineffective.
    /// None means "inherit the broker's umask" (current behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub umask: Option<u32>,
}

/// v1.1.1fu14 — single-entry uid_map/gid_map declaration for the
/// per-role user namespace. The values are the host UIDs/GIDs that
/// in-NS UID 0 (and GID 0) map to. See ADR 0021.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserNamespaceProfile {
    pub host_uid_for_zero: u32,
    pub host_gid_for_zero: u32,
}

// v1.1.2fu19 panel-software R2 should-fix: provide `From` impls
// across the duplicate `UserNamespaceSpec` types so layer
// boundaries don't drift if one struct adds a field and the
// others don't. The conversions are infallible — both source
// and target are the same two `u32`s — so we use `From`, not
// `TryFrom`.
impl From<crate::processes::RoleUserNamespace> for UserNamespaceProfile {
    fn from(rn: crate::processes::RoleUserNamespace) -> Self {
        Self {
            host_uid_for_zero: rn.host_uid_for_zero,
            host_gid_for_zero: rn.host_gid_for_zero,
        }
    }
}

impl From<UserNamespaceProfile> for crate::processes::RoleUserNamespace {
    fn from(p: UserNamespaceProfile) -> Self {
        Self {
            host_uid_for_zero: p.host_uid_for_zero,
            host_gid_for_zero: p.host_gid_for_zero,
        }
    }
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
    /// v1.1.1 live-deploy fu9: closed-set device bind paths the
    /// broker opens on behalf of the role (e.g. `/dev/kvm`,
    /// `/dev/dri/renderD128`, `/dev/nvidia*`). Optional for
    /// backward-compat with older bundles; empty for roles that
    /// don't need device access.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_binds: Vec<String>,
    /// v1.1.1 live-deploy fu9: cross-domain bind mounts the broker
    /// performs into the role's mount namespace (e.g. a Wayland
    /// socket for the GPU sidecar). Each entry is a `{src, dst}`
    /// pair. Optional for backward-compat with older bundles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bind_mounts: Vec<BindMount>,
}

/// Cross-domain bind mount declaration for a minijail role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindMount {
    /// Source path on the host (broker opens on the caller's behalf).
    pub src: String,
    /// Destination path inside the role's mount namespace.
    pub dst: String,
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
