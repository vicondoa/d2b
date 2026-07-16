//! Test fixture builders for downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production
//! consumers never pull this in.

use crate::bundle_resolver::{ResolvedRunnerIntent, UserNamespaceSpec};
use crate::minijail_profile::{
    BindMount, CgroupPlacement, MountPolicy, NamespaceSet, WritablePath,
};
use crate::processes::{ProcessRole, RoleProfile, RoleUserNamespace};
use std::path::PathBuf;

// ── RoleProfileBuilder ──────────────────────────────────────────────────────

/// Builder for [`RoleProfile`] test fixtures.
///
/// All fields start from sensible defaults; call `with_<field>` methods to
/// override only the fields that are semantically load-bearing for the test.
pub struct RoleProfileBuilder {
    profile_id: String,
    uid: u32,
    gid: u32,
    adr_carve_out: Option<String>,
    caps: Vec<String>,
    namespaces: NamespaceSet,
    seccomp_policy_ref: Option<String>,
    mount_policy: MountPolicy,
    cgroup_placement: CgroupPlacement,
    user_namespace: Option<RoleUserNamespace>,
    umask: Option<u32>,
}

impl RoleProfileBuilder {
    /// Create a builder with sensible defaults for unit tests.
    pub fn new() -> Self {
        Self {
            profile_id: "test-profile".to_owned(),
            uid: 1000,
            gid: 1000,
            adr_carve_out: None,
            caps: vec![],
            namespaces: NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: false,
            },
            seccomp_policy_ref: None,
            mount_policy: MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_placement: CgroupPlacement {
                subtree: "d2b.slice/test".to_owned(),
                controllers: vec![],
                delegated: false,
            },
            user_namespace: None,
            umask: None,
        }
    }

    pub fn with_profile_id(mut self, id: impl Into<String>) -> Self {
        self.profile_id = id.into();
        self
    }

    pub fn with_uid(mut self, uid: u32) -> Self {
        self.uid = uid;
        self
    }

    pub fn with_gid(mut self, gid: u32) -> Self {
        self.gid = gid;
        self
    }

    pub fn with_adr_carve_out(mut self, v: Option<impl Into<String>>) -> Self {
        self.adr_carve_out = v.map(Into::into);
        self
    }

    pub fn with_caps(mut self, caps: Vec<String>) -> Self {
        self.caps = caps;
        self
    }

    pub fn with_namespaces(mut self, ns: NamespaceSet) -> Self {
        self.namespaces = ns;
        self
    }

    pub fn with_seccomp_policy_ref(mut self, r: Option<impl Into<String>>) -> Self {
        self.seccomp_policy_ref = r.map(Into::into);
        self
    }

    pub fn with_read_only_paths(mut self, paths: Vec<String>) -> Self {
        self.mount_policy.read_only_paths = paths;
        self
    }

    pub fn with_writable_paths(mut self, paths: Vec<WritablePath>) -> Self {
        self.mount_policy.writable_paths = paths;
        self
    }

    pub fn with_nix_store_read_only(mut self, v: bool) -> Self {
        self.mount_policy.nix_store_read_only = v;
        self
    }

    pub fn with_hide_device_nodes_by_default(mut self, v: bool) -> Self {
        self.mount_policy.hide_device_nodes_by_default = v;
        self
    }

    pub fn with_device_binds(mut self, binds: Vec<String>) -> Self {
        self.mount_policy.device_binds = binds;
        self
    }

    pub fn with_bind_mounts(mut self, mounts: Vec<BindMount>) -> Self {
        self.mount_policy.bind_mounts = mounts;
        self
    }

    /// Replace the entire mount policy at once (useful when several
    /// non-default fields must be set together).
    pub fn with_mount_policy(mut self, mp: MountPolicy) -> Self {
        self.mount_policy = mp;
        self
    }

    pub fn with_cgroup_subtree(mut self, subtree: impl Into<String>) -> Self {
        self.cgroup_placement.subtree = subtree.into();
        self
    }

    pub fn with_cgroup_controllers(mut self, controllers: Vec<String>) -> Self {
        self.cgroup_placement.controllers = controllers;
        self
    }

    pub fn with_cgroup_delegated(mut self, delegated: bool) -> Self {
        self.cgroup_placement.delegated = delegated;
        self
    }

    /// Replace the entire cgroup placement at once.
    pub fn with_cgroup_placement(mut self, cp: CgroupPlacement) -> Self {
        self.cgroup_placement = cp;
        self
    }

    pub fn with_user_namespace(mut self, un: Option<RoleUserNamespace>) -> Self {
        self.user_namespace = un;
        self
    }

    pub fn with_umask(mut self, umask: Option<u32>) -> Self {
        self.umask = umask;
        self
    }

    pub fn build(self) -> RoleProfile {
        RoleProfile {
            profile_id: self.profile_id,
            uid: self.uid,
            gid: self.gid,
            adr_carve_out: self.adr_carve_out,
            caps: self.caps,
            namespaces: self.namespaces,
            seccomp_policy_ref: self.seccomp_policy_ref,
            mount_policy: self.mount_policy,
            cgroup_placement: self.cgroup_placement,
            user_namespace: self.user_namespace,
            umask: self.umask,
        }
    }
}

impl Default for RoleProfileBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── ResolvedRunnerIntentBuilder ─────────────────────────────────────────────

/// Builder for [`ResolvedRunnerIntent`] test fixtures.
///
/// All fields start from sensible defaults; call `with_<field>` methods to
/// override only the fields that are semantically load-bearing for the test.
pub struct ResolvedRunnerIntentBuilder {
    intent_id: String,
    vm_name: String,
    role_id: String,
    role: ProcessRole,
    binary_path: PathBuf,
    argv: Vec<String>,
    env: Vec<String>,
    uid: u32,
    gid: u32,
    supplementary_groups: Vec<u32>,
    capabilities: Vec<String>,
    namespaces: NamespaceSet,
    seccomp_policy_ref: Option<String>,
    mount_policy: MountPolicy,
    cgroup_placement: CgroupPlacement,
    root_carve_out: bool,
    profile_id: String,
    user_namespace: Option<UserNamespaceSpec>,
    umask: Option<u32>,
}

impl ResolvedRunnerIntentBuilder {
    /// Create a builder with sensible defaults for unit tests.
    pub fn new() -> Self {
        Self {
            intent_id: "test-intent".to_owned(),
            vm_name: "test-vm".to_owned(),
            role_id: "test-role".to_owned(),
            role: ProcessRole::CloudHypervisorRunner,
            binary_path: PathBuf::from("/bin/test"),
            argv: vec!["test".to_owned()],
            env: vec![],
            uid: 1000,
            gid: 1000,
            supplementary_groups: vec![],
            capabilities: vec![],
            namespaces: NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: false,
            },
            seccomp_policy_ref: None,
            mount_policy: MountPolicy {
                read_only_paths: vec![],
                writable_paths: vec![],
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
                device_binds: vec![],
                bind_mounts: vec![],
            },
            cgroup_placement: CgroupPlacement {
                subtree: "d2b.slice/test".to_owned(),
                controllers: vec![],
                delegated: false,
            },
            root_carve_out: false,
            profile_id: "test-profile".to_owned(),
            user_namespace: None,
            umask: None,
        }
    }

    pub fn with_intent_id(mut self, id: impl Into<String>) -> Self {
        self.intent_id = id.into();
        self
    }

    pub fn with_vm_name(mut self, name: impl Into<String>) -> Self {
        self.vm_name = name.into();
        self
    }

    pub fn with_role_id(mut self, id: impl Into<String>) -> Self {
        self.role_id = id.into();
        self
    }

    pub fn with_role(mut self, role: ProcessRole) -> Self {
        self.role = role;
        self
    }

    pub fn with_binary_path(mut self, path: PathBuf) -> Self {
        self.binary_path = path;
        self
    }

    pub fn with_argv(mut self, argv: Vec<String>) -> Self {
        self.argv = argv;
        self
    }

    pub fn with_env(mut self, env: Vec<String>) -> Self {
        self.env = env;
        self
    }

    pub fn with_uid(mut self, uid: u32) -> Self {
        self.uid = uid;
        self
    }

    pub fn with_gid(mut self, gid: u32) -> Self {
        self.gid = gid;
        self
    }

    pub fn with_supplementary_groups(mut self, groups: Vec<u32>) -> Self {
        self.supplementary_groups = groups;
        self
    }

    pub fn with_capabilities(mut self, caps: Vec<String>) -> Self {
        self.capabilities = caps;
        self
    }

    pub fn with_namespaces(mut self, ns: NamespaceSet) -> Self {
        self.namespaces = ns;
        self
    }

    pub fn with_seccomp_policy_ref(mut self, r: Option<impl Into<String>>) -> Self {
        self.seccomp_policy_ref = r.map(Into::into);
        self
    }

    /// Replace the entire mount policy at once.
    pub fn with_mount_policy(mut self, mp: MountPolicy) -> Self {
        self.mount_policy = mp;
        self
    }

    /// Replace the entire cgroup placement at once.
    pub fn with_cgroup_placement(mut self, cp: CgroupPlacement) -> Self {
        self.cgroup_placement = cp;
        self
    }

    pub fn with_root_carve_out(mut self, v: bool) -> Self {
        self.root_carve_out = v;
        self
    }

    pub fn with_profile_id(mut self, id: impl Into<String>) -> Self {
        self.profile_id = id.into();
        self
    }

    pub fn with_user_namespace(mut self, un: Option<UserNamespaceSpec>) -> Self {
        self.user_namespace = un;
        self
    }

    pub fn with_umask(mut self, umask: Option<u32>) -> Self {
        self.umask = umask;
        self
    }

    pub fn build(self) -> ResolvedRunnerIntent {
        ResolvedRunnerIntent {
            intent_id: self.intent_id,
            vm_name: self.vm_name,
            role_id: self.role_id,
            role: self.role,
            source: crate::bundle_resolver::ResolvedRunnerSource::ExplicitProcessNode,
            binary_path: self.binary_path,
            argv: self.argv,
            env: self.env,
            uid: self.uid,
            gid: self.gid,
            supplementary_groups: self.supplementary_groups,
            capabilities: self.capabilities,
            namespaces: self.namespaces,
            seccomp_policy_ref: self.seccomp_policy_ref,
            mount_policy: self.mount_policy,
            cgroup_placement: self.cgroup_placement,
            root_carve_out: self.root_carve_out,
            profile_id: self.profile_id,
            user_namespace: self.user_namespace,
            umask: self.umask,
        }
    }
}

impl Default for ResolvedRunnerIntentBuilder {
    fn default() -> Self {
        Self::new()
    }
}
