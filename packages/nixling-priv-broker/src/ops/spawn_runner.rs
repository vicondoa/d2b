//! W4-fu broker SpawnRunner preflight + spawn helper.
//!
//! The broker's `SpawnRunner` dispatch resolves the daemon's opaque
//! `bundle_runner_intent_ref` into the full launch context (binary
//! path, argv, arg0, uid/gid, supplementary groups, environment,
//! seccomp profile, cgroup placement). This module owns the
//! **post-resolution** validation primitive:
//!
//! - [`SpawnRunnerPlan`]: the validated launch plan the broker
//!   feeds to the spawn syscall.
//! - [`preflight`]: pure data validation. Refuses non-absolute
//!   binary paths, empty argv, NUL bytes anywhere, missing
//!   binaries, uid 0 without an ADR carve-out, malformed env.
//! - [`build_cstring_vectors`]: converts the plan into the
//!   `CString` triple (`binary`, `argv`, `env`) the execve syscall
//!   in `crate::sys::pidfd_sys::clone3_pidfd_or_fork_fallback`
//!   expects.
//!
//! The actual spawn lives in `sys::pidfd_sys` (the broker's only
//! unsafe-quarantined module); this preflight + cstring builder is
//! pure data so it's fully unit-tested without root.

use std::ffi::{CString, NulError};
use std::path::{Path, PathBuf};

use nixling_core::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};

/// Validated launch plan. Produced by [`preflight`] from a
/// bundle-resolved row; consumed by `clone3_pidfd_or_fork_fallback`.
#[derive(Debug, Clone)]
pub struct SpawnRunnerPlan {
    pub binary_path: PathBuf,
    pub argv: Vec<String>,
    pub uid: u32,
    pub gid: u32,
    pub supplementary_groups: Vec<u32>,
    pub env: Vec<String>,
    pub capabilities: Vec<String>,
    pub namespaces: NamespaceSet,
    pub seccomp_policy_ref: Option<String>,
    pub mount_policy: MountPolicy,
    pub cgroup_placement: CgroupPlacement,
    /// v1.1.1fu14 (ADR 0021): when `Some`, the broker
    /// pre-establishes a single-entry user namespace for this
    /// runner. The child is fake-root inside the namespace
    /// (all caps within the user-NS scope) and the host-side
    /// `capabilities` set should be empty. Currently consumed
    /// by virtiofsd roles for least-privilege FS serving.
    pub user_namespace: Option<UserNamespaceSpec>,
    /// v1.1.2fu36: file-creation mask the broker installs in the
    /// spawned child before execve. See `MinijailProfile::umask`.
    pub umask: Option<u32>,
}

/// Single-entry uid/gid mapping for a runner's user namespace.
/// The child sees `0` mapped to `host_uid_for_zero` on the
/// host (and `host_gid_for_zero` for groups). All other UIDs
/// inside the namespace map to overflowuid (65534). This is
/// the minimal mapping needed for virtiofsd to operate as
/// fake-root over its `--shared-dir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserNamespaceSpec {
    pub host_uid_for_zero: u32,
    pub host_gid_for_zero: u32,
}

/// Errors the preflight + cstring conversion can return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnRunnerError {
    InvalidBinaryPath {
        path: String,
    },
    EmptyArgv,
    EmptyArg0,
    Arg0WithNul {
        arg0: String,
    },
    ArgvEntryWithNul {
        index: usize,
    },
    EnvEntryWithNul {
        index: usize,
    },
    BinaryNotFound {
        path: String,
    },
    /// uid 0 without an ADR carve-out. The W3 ADR 0003 §"per-role
    /// minijail" rule pins that long-lived runners do not start
    /// as root; carve-outs require an explicit `adr_carve_out`
    /// field on the bundle row, surfaced as
    /// `SpawnRunnerPlanInput::root_carve_out`.
    RootRequiresCarveOut,
    /// `supplementary_groups` contained the primary gid; redundant
    /// and ambiguous, refuse.
    SupplementaryGroupContainsPrimaryGid {
        gid: u32,
    },
    /// Env entry doesn't match `KEY=VALUE` or `KEY` is empty.
    InvalidEnvEntry {
        index: usize,
        entry: String,
    },
}

impl std::fmt::Display for SpawnRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBinaryPath { path } => {
                write!(f, "binary path {path:?} must be absolute")
            }
            Self::EmptyArgv => f.write_str("argv must be non-empty"),
            Self::EmptyArg0 => f.write_str("argv[0] must be non-empty"),
            Self::Arg0WithNul { arg0 } => write!(f, "argv[0] contains NUL: {arg0:?}"),
            Self::ArgvEntryWithNul { index } => write!(f, "argv[{index}] contains NUL"),
            Self::EnvEntryWithNul { index } => write!(f, "env[{index}] contains NUL"),
            Self::BinaryNotFound { path } => write!(f, "binary {path} does not exist"),
            Self::RootRequiresCarveOut => {
                f.write_str("uid 0 requires an explicit ADR carve-out (W3 ADR 0003)")
            }
            Self::SupplementaryGroupContainsPrimaryGid { gid } => write!(
                f,
                "supplementary_groups contains the primary gid {gid}; remove the duplicate"
            ),
            Self::InvalidEnvEntry { index, entry } => {
                write!(f, "env[{index}] {entry:?} is not KEY=VALUE")
            }
        }
    }
}

impl std::error::Error for SpawnRunnerError {}

/// Input to [`preflight`]. Pure data; no syscalls.
#[derive(Debug, Clone)]
pub struct SpawnRunnerPlanInput {
    pub binary_path: PathBuf,
    pub argv: Vec<String>,
    pub uid: u32,
    pub gid: u32,
    pub supplementary_groups: Vec<u32>,
    pub env: Vec<String>,
    pub capabilities: Vec<String>,
    pub namespaces: NamespaceSet,
    pub seccomp_policy_ref: Option<String>,
    pub mount_policy: MountPolicy,
    pub cgroup_placement: CgroupPlacement,
    /// Set by the broker dispatch when the bundle row's
    /// `adr_carve_out` field is non-null (e.g. for the swtpm
    /// pre-start flush which legitimately runs as root).
    pub root_carve_out: bool,
    /// Set to `true` only by unit tests so the preflight skips
    /// the binary-exists check.
    pub skip_binary_exists_check: bool,
    /// v1.1.1fu14 (ADR 0021): when `Some`, broker creates a
    /// per-runner user namespace and writes uid_map/gid_map. The
    /// in-NS UID 0 maps to the supplied host UID. virtiofsd
    /// roles set this to gain fake-root semantics with zero
    /// host-side caps.
    pub user_namespace: Option<UserNamespaceSpec>,
    /// v1.1.2fu36: optional umask installed before execve.
    pub umask: Option<u32>,
}
/// [`SpawnRunnerPlan`].
pub fn preflight(input: &SpawnRunnerPlanInput) -> Result<SpawnRunnerPlan, SpawnRunnerError> {
    if !input
        .binary_path
        .to_str()
        .map(|s| s.starts_with('/'))
        .unwrap_or(false)
    {
        return Err(SpawnRunnerError::InvalidBinaryPath {
            path: input.binary_path.display().to_string(),
        });
    }
    if input.argv.is_empty() {
        return Err(SpawnRunnerError::EmptyArgv);
    }
    if input.argv[0].is_empty() {
        return Err(SpawnRunnerError::EmptyArg0);
    }
    if input.argv[0].contains('\0') {
        return Err(SpawnRunnerError::Arg0WithNul {
            arg0: input.argv[0].clone(),
        });
    }
    for (i, a) in input.argv.iter().enumerate() {
        if a.contains('\0') {
            return Err(SpawnRunnerError::ArgvEntryWithNul { index: i });
        }
    }
    for (i, e) in input.env.iter().enumerate() {
        if e.contains('\0') {
            return Err(SpawnRunnerError::EnvEntryWithNul { index: i });
        }
        match e.split_once('=') {
            Some((k, _)) if !k.is_empty() => {}
            _ => {
                return Err(SpawnRunnerError::InvalidEnvEntry {
                    index: i,
                    entry: e.clone(),
                })
            }
        }
    }
    if input.uid == 0 && !input.root_carve_out {
        return Err(SpawnRunnerError::RootRequiresCarveOut);
    }
    if input.supplementary_groups.contains(&input.gid) {
        return Err(SpawnRunnerError::SupplementaryGroupContainsPrimaryGid { gid: input.gid });
    }
    if !input.skip_binary_exists_check && !input.binary_path.exists() {
        return Err(SpawnRunnerError::BinaryNotFound {
            path: input.binary_path.display().to_string(),
        });
    }
    Ok(SpawnRunnerPlan {
        binary_path: input.binary_path.clone(),
        argv: input.argv.clone(),
        uid: input.uid,
        gid: input.gid,
        supplementary_groups: input.supplementary_groups.clone(),
        env: input.env.clone(),
        capabilities: input.capabilities.clone(),
        namespaces: input.namespaces.clone(),
        seccomp_policy_ref: input.seccomp_policy_ref.clone(),
        mount_policy: input.mount_policy.clone(),
        cgroup_placement: input.cgroup_placement.clone(),
        user_namespace: input.user_namespace,
        umask: input.umask,
    })
}

/// Convert the plan into the `(binary, argv, env)` CString triple
/// the execve syscall expects. Pure.
pub fn build_cstring_vectors(
    plan: &SpawnRunnerPlan,
) -> Result<(CString, Vec<CString>, Vec<CString>), SpawnRunnerError> {
    let binary =
        path_to_cstring(&plan.binary_path).map_err(|_| SpawnRunnerError::InvalidBinaryPath {
            path: plan.binary_path.display().to_string(),
        })?;
    let mut argv: Vec<CString> = Vec::with_capacity(plan.argv.len());
    for (i, a) in plan.argv.iter().enumerate() {
        argv.push(
            CString::new(a.as_bytes())
                .map_err(|_| SpawnRunnerError::ArgvEntryWithNul { index: i })?,
        );
    }
    let mut env: Vec<CString> = Vec::with_capacity(plan.env.len());
    for (i, e) in plan.env.iter().enumerate() {
        env.push(
            CString::new(e.as_bytes())
                .map_err(|_| SpawnRunnerError::EnvEntryWithNul { index: i })?,
        );
    }
    Ok((binary, argv, env))
}

fn path_to_cstring(path: &Path) -> Result<CString, NulError> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(path.as_os_str().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::minijail_profile::WritablePath;

    fn test_namespaces() -> NamespaceSet {
        NamespaceSet {
            mount: true,
            pid: false,
            net: false,
            ipc: true,
            uts: false,
            user: false,
        }
    }

    fn test_mount_policy() -> MountPolicy {
        MountPolicy {
            read_only_paths: vec!["/nix/store".to_owned()],
            writable_paths: vec![WritablePath {
                path: "/var/lib/nixling/vms/corp-vm".to_owned(),
                purpose: "runner state".to_owned(),
            }],
            nix_store_read_only: true,
            hide_device_nodes_by_default: true,
                    device_binds: Vec::new(),
                    bind_mounts: Vec::new(),
        }
    }

    fn test_cgroup_placement() -> CgroupPlacement {
        CgroupPlacement {
            subtree: "nixling.slice/corp-vm/cloud-hypervisor".to_owned(),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: false,
        }
    }

    fn good_input() -> SpawnRunnerPlanInput {
        SpawnRunnerPlanInput {
            binary_path: PathBuf::from("/nix/store/abc/bin/cloud-hypervisor"),
            argv: vec!["microvm@corp-vm".to_owned(), "--api-socket".to_owned()],
            uid: 1100,
            gid: 1100,
            supplementary_groups: vec![27],
            env: vec!["PATH=/usr/bin".to_owned(), "TERM=dumb".to_owned()],
            capabilities: vec!["CAP_NET_ADMIN".to_owned()],
            namespaces: test_namespaces(),
            seccomp_policy_ref: Some("/work/seccomp/cloud-hypervisor.bpf".to_owned()),
            mount_policy: test_mount_policy(),
            cgroup_placement: test_cgroup_placement(),
            root_carve_out: false,
            skip_binary_exists_check: true,
            user_namespace: None,
            umask: None,
        }
    }

    #[test]
    fn happy_path_validates_and_emits_plan() {
        let plan = preflight(&good_input()).unwrap();
        assert_eq!(plan.uid, 1100);
        assert_eq!(plan.argv.len(), 2);
        assert_eq!(plan.capabilities, vec!["CAP_NET_ADMIN".to_owned()]);
        assert!(plan.namespaces.mount);
        assert_eq!(
            plan.seccomp_policy_ref.as_deref(),
            Some("/work/seccomp/cloud-hypervisor.bpf")
        );
        assert_eq!(
            plan.mount_policy.read_only_paths,
            vec!["/nix/store".to_owned()]
        );
        assert_eq!(
            plan.cgroup_placement.subtree,
            "nixling.slice/corp-vm/cloud-hypervisor"
        );
    }

    #[test]
    fn rejects_non_absolute_binary() {
        let mut i = good_input();
        i.binary_path = PathBuf::from("cloud-hypervisor");
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::InvalidBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_argv() {
        let mut i = good_input();
        i.argv.clear();
        assert!(matches!(preflight(&i), Err(SpawnRunnerError::EmptyArgv)));
    }

    #[test]
    fn rejects_empty_arg0() {
        let mut i = good_input();
        i.argv[0].clear();
        assert!(matches!(preflight(&i), Err(SpawnRunnerError::EmptyArg0)));
    }

    #[test]
    fn rejects_arg0_with_nul() {
        let mut i = good_input();
        i.argv[0] = "bad\0name".to_owned();
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::Arg0WithNul { .. })
        ));
    }

    #[test]
    fn rejects_argv_entry_with_nul() {
        let mut i = good_input();
        i.argv.push("evil\0arg".to_owned());
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::ArgvEntryWithNul { index: 2 })
        ));
    }

    #[test]
    fn rejects_env_with_nul() {
        let mut i = good_input();
        i.env.push("KEY=val\0ue".to_owned());
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::EnvEntryWithNul { index: 2 })
        ));
    }

    #[test]
    fn rejects_env_missing_equals() {
        let mut i = good_input();
        i.env.push("NOEQUALS".to_owned());
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::InvalidEnvEntry { index: 2, .. })
        ));
    }

    #[test]
    fn rejects_env_with_empty_key() {
        let mut i = good_input();
        i.env.push("=value".to_owned());
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::InvalidEnvEntry { index: 2, .. })
        ));
    }

    #[test]
    fn rejects_uid_zero_without_carve_out() {
        let mut i = good_input();
        i.uid = 0;
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::RootRequiresCarveOut)
        ));
    }

    #[test]
    fn accepts_uid_zero_with_carve_out() {
        let mut i = good_input();
        i.uid = 0;
        i.root_carve_out = true;
        assert!(preflight(&i).is_ok());
    }

    #[test]
    fn rejects_primary_gid_in_supplementary_set() {
        let mut i = good_input();
        i.supplementary_groups.push(i.gid);
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::SupplementaryGroupContainsPrimaryGid { gid: 1100 })
        ));
    }

    #[test]
    fn rejects_missing_binary_when_not_skipped() {
        let mut i = good_input();
        i.skip_binary_exists_check = false;
        i.binary_path = PathBuf::from("/tmp/nonexistent-nixling-binary-xyzzy");
        assert!(matches!(
            preflight(&i),
            Err(SpawnRunnerError::BinaryNotFound { .. })
        ));
    }

    #[test]
    fn build_cstring_vectors_round_trips() {
        let plan = preflight(&good_input()).unwrap();
        let (bin, argv, env) = build_cstring_vectors(&plan).unwrap();
        assert!(bin.to_string_lossy().ends_with("/cloud-hypervisor"));
        assert_eq!(argv.len(), 2);
        assert_eq!(env.len(), 2);
        assert_eq!(argv[0].to_string_lossy(), "microvm@corp-vm");
    }

    // v1.1.1fu14 (ADR 0021) — user_namespace round-trips.
    //
    // The preflight is pure data; we only verify that the
    // user_namespace field round-trips from the input to the
    // resulting plan unchanged. Actual broker spawn behaviour
    // is exercised in `sys::tests::clone3_spawn_runner_*` and
    // in the integration tests under `live_handlers.rs`.

    #[test]
    fn user_namespace_round_trips_none() {
        let plan = preflight(&good_input()).unwrap();
        assert_eq!(plan.user_namespace, None);
    }

    #[test]
    fn user_namespace_round_trips_some() {
        let mut input = good_input();
        input.user_namespace = Some(UserNamespaceSpec {
            host_uid_for_zero: 11_032_050,
            host_gid_for_zero: 11_032_050,
        });
        let plan = preflight(&input).unwrap();
        assert_eq!(
            plan.user_namespace,
            Some(UserNamespaceSpec {
                host_uid_for_zero: 11_032_050,
                host_gid_for_zero: 11_032_050,
            })
        );
    }

    #[test]
    fn user_namespace_with_zero_uid_is_allowed_in_plan_layer() {
        // The preflight does NOT validate the host UID — the
        // broker dispatch is responsible for refusing UID 0
        // mappings when adr_carve_out is absent (separately
        // enforced in runtime.rs). This test pins the plan
        // layer's pass-through semantics.
        let mut input = good_input();
        input.user_namespace = Some(UserNamespaceSpec {
            host_uid_for_zero: 0,
            host_gid_for_zero: 0,
        });
        let plan = preflight(&input).unwrap();
        assert_eq!(plan.user_namespace.unwrap().host_uid_for_zero, 0);
    }
}
