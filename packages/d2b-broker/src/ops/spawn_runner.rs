//! Broker SpawnRunner preflight + spawn helper.
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
use std::fmt;
use std::path::{Path, PathBuf};

use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};

/// Validated launch plan. Produced by [`preflight`] from a
/// bundle-resolved row; consumed by `clone3_pidfd_or_fork_fallback`.
#[derive(Clone)]
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
    /// When `Some`, the broker pre-establishes a single-entry user
    /// namespace for this runner. The child is fake-root inside the
    /// namespace (all caps within the user-NS scope) and the host-side
    /// `capabilities` set should be empty. Currently consumed by
    /// virtiofsd roles for least-privilege FS serving (ADR 0021).
    pub user_namespace: Option<UserNamespaceSpec>,
    /// File-creation mask the broker installs in the spawned child
    /// before execve. See `SandboxProfile::umask`.
    pub umask: Option<u32>,
}

impl fmt::Debug for SpawnRunnerPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SpawnRunnerPlan(<redacted>)")
    }
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
    /// uid 0 without an ADR carve-out. ADR 0003 §"per-role minijail"
    /// pins that long-lived runners do not start as root; carve-outs
    /// require an explicit `adr_carve_out` field on the bundle row,
    /// surfaced as `SpawnRunnerPlanInput::root_carve_out`.
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
                f.write_str("uid 0 requires an explicit ADR carve-out (ADR 0003)")
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
#[derive(Clone)]
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
    /// When `Some`, broker creates a per-runner user namespace and
    /// writes uid_map/gid_map. The in-NS UID 0 maps to the supplied host
    /// UID. virtiofsd roles set this to gain fake-root semantics with
    /// zero host-side caps (ADR 0021).
    pub user_namespace: Option<UserNamespaceSpec>,
    /// Optional umask installed before execve.
    pub umask: Option<u32>,
}

impl fmt::Debug for SpawnRunnerPlanInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SpawnRunnerPlanInput(<redacted>)")
    }
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
                });
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

// -- The trust boundary: the launch payload against the bundle's own row --

/// The closed set of reasons one launch plan is refused because it disagrees
/// with - or is absent from - the bundle-resolved runner intent it names.
///
/// Every variant renders as a path-free, value-free slug: the field that
/// disagreed is the whole of the report, and the payload's value for it is
/// recorded nowhere. A refusal is a correct outcome of this fence, not a
/// failure to launch: where the trusted declaration is absent, the plan is
/// refused rather than completed with a synthesized default, and a plan that
/// matches is the only thing that spawns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanIntentMismatch {
    /// `binaryPath` - the executable the broker `execve`s - is not the one
    /// the verified bundle declares. No other fence can catch this one: the
    /// argv fence reads what the worker *opens*, and the binary is what the
    /// worker *is*.
    BinaryPath,
    /// The host uid the plan runs as is not the intent's.
    Uid,
    /// The host gid the plan runs as is not the intent's.
    Gid,
    /// The supplementary group set the broker `setgroups`es to.
    SupplementaryGroups,
    /// The capability list `apply_capabilities` raises on the child.
    Capabilities,
    /// The namespace classes the broker clones.
    Namespaces,
    /// The seccomp policy class. This is load-bearing beyond the filter
    /// itself: `w1-swtpm` is the marker the swtpm argv fence and the state
    /// directory grant key off, so a plan that renamed it would drop both.
    SeccompPolicy,
    /// The mount policy - the read-only enforcement, the device mask, the
    /// private `/proc` and the secret masks.
    MountPolicy,
    /// The file-creation mask installed before `execve`.
    Umask,
    /// The ADR 0003 root carve-out, which is what `preflight`'s uid-0 refusal
    /// is keyed on. Asserting it the bundle row does not declare is how a
    /// payload would run as host root outside a user namespace.
    RootCarveOut,
    /// The plan pre-establishes a user namespace the trusted intent declares
    /// no mapping for.
    UserNamespaceUndeclared,
    /// The trusted intent declares a user-namespace mapping the plan drops.
    UserNamespaceWithheld,
    /// Both declare a mapping and the two mappings are not the same one.
    UserNamespaceMapping,
    /// The mapping sends in-namespace root (`0`) to host root (`0`) with no
    /// ADR 0003 carve-out in the bundle row, so the broker would write
    /// `0 0 1` into the child's `uid_map` and in-namespace root would carry
    /// host-root DAC - with the payload's own capability list raised on top.
    HostUidForZeroIsHostRoot,
    /// The same for the group side of the mapping.
    HostGidForZeroIsHostRoot,
    /// A broker-pre-NS launch whose plan asks for the root secret masks
    /// (`/etc`, `/var`, `/root`, `/run`, ...) that the user-namespace path
    /// drops without a word - see [`DroppedMountProtection`].
    UserNamespaceDropsRootSecretMasks,
}

impl std::fmt::Display for PlanIntentMismatch {
    /// The stable static-code spelling the launch-failure envelope surfaces
    /// and the audit record carries.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BinaryPath => "spawn-plan-binary-path-mismatch",
            Self::Uid => "spawn-plan-uid-mismatch",
            Self::Gid => "spawn-plan-gid-mismatch",
            Self::SupplementaryGroups => "spawn-plan-supplementary-groups-mismatch",
            Self::Capabilities => "spawn-plan-capabilities-mismatch",
            Self::Namespaces => "spawn-plan-namespaces-mismatch",
            Self::SeccompPolicy => "spawn-plan-seccomp-policy-mismatch",
            Self::MountPolicy => "spawn-plan-mount-policy-mismatch",
            Self::Umask => "spawn-plan-umask-mismatch",
            Self::RootCarveOut => "spawn-plan-root-carve-out-mismatch",
            Self::UserNamespaceUndeclared => "spawn-plan-user-namespace-undeclared",
            Self::UserNamespaceWithheld => "spawn-plan-user-namespace-withheld",
            Self::UserNamespaceMapping => "spawn-plan-user-namespace-mapping-mismatch",
            Self::HostUidForZeroIsHostRoot => "spawn-plan-host-uid-for-zero-is-host-root",
            Self::HostGidForZeroIsHostRoot => "spawn-plan-host-gid-for-zero-is-host-root",
            Self::UserNamespaceDropsRootSecretMasks => {
                "spawn-plan-user-namespace-drops-root-secret-masks"
            }
        })
    }
}

/// Cross-check one launch plan against the bundle-resolved runner intent it
/// names, and refuse anything the verified bundle does not itself declare.
///
/// The spawn payload reaches the broker as a resolved plan: binary, uid, gid,
/// groups, capabilities, namespaces, seccomp class, mount policy, umask and
/// the user-namespace mapping are all the daemon's word, and every one of
/// them becomes a host credential. The bundle is the broker's own verified
/// copy of the same row, so it - not the payload - decides. Each field is
/// compared to the intent's and a disagreement is refused by name, in the
/// shape [`crate::ops::device_worker::state_volume_leaf_for_launch`] already
/// uses for the state-directory presence policy.
///
/// The user-namespace mapping gets two checks. Equality with the trusted
/// mapping, because the payload chooses which host identity in-namespace
/// root lands on; and a refusal of a host-root mapping on its own, because
/// `0 -> 0` makes in-namespace root host root and there is no `input.uid`
/// check that sees it - a user-namespace launch forces the in-namespace
/// credential to `0` regardless. A bundle row that genuinely wants host root
/// says so with the same ADR 0003 `adrCarveOut` the uid-0 refusal is keyed
/// on, and the plan may not claim that carve-out itself.
///
/// `argv`, `env` and the cgroup subtree are deliberately not compared: the
/// daemon composes all three per launch (bounded controller-supplied launch
/// arguments, the audio runtime properties, the private cgroup placement), so
/// an exact match would refuse every well-formed launch. The arguments
/// themselves are fenced where they matter -
/// `swtpm_identity::verify_argv_names_only_trusted_paths` for the Device
/// worker that opens its state by pathname, the per-family preflights for the
/// rest.
pub fn verify_plan_against_intent(
    plan_input: &SpawnRunnerPlanInput,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
) -> Result<(), PlanIntentMismatch> {
    if plan_input.binary_path != intent.binary_path {
        return Err(PlanIntentMismatch::BinaryPath);
    }
    if plan_input.uid != intent.uid {
        return Err(PlanIntentMismatch::Uid);
    }
    if plan_input.gid != intent.gid {
        return Err(PlanIntentMismatch::Gid);
    }
    if plan_input.supplementary_groups != intent.supplementary_groups {
        return Err(PlanIntentMismatch::SupplementaryGroups);
    }
    if plan_input.capabilities != intent.capabilities {
        return Err(PlanIntentMismatch::Capabilities);
    }
    if plan_input.namespaces != intent.namespaces {
        return Err(PlanIntentMismatch::Namespaces);
    }
    if plan_input.seccomp_policy_ref != intent.seccomp_policy_ref {
        return Err(PlanIntentMismatch::SeccompPolicy);
    }
    if plan_input.mount_policy != intent.mount_policy {
        return Err(PlanIntentMismatch::MountPolicy);
    }
    if plan_input.umask != intent.umask {
        return Err(PlanIntentMismatch::Umask);
    }
    if plan_input.root_carve_out != intent.root_carve_out {
        return Err(PlanIntentMismatch::RootCarveOut);
    }
    match (plan_input.user_namespace, intent.user_namespace) {
        (None, None) => {}
        (Some(_), None) => return Err(PlanIntentMismatch::UserNamespaceUndeclared),
        (None, Some(_)) => return Err(PlanIntentMismatch::UserNamespaceWithheld),
        (Some(claimed), Some(declared)) => {
            if claimed.host_uid_for_zero != declared.host_uid_for_zero
                || claimed.host_gid_for_zero != declared.host_gid_for_zero
            {
                return Err(PlanIntentMismatch::UserNamespaceMapping);
            }
            if claimed.host_uid_for_zero == 0 && !intent.root_carve_out {
                return Err(PlanIntentMismatch::HostUidForZeroIsHostRoot);
            }
            if claimed.host_gid_for_zero == 0 && !intent.root_carve_out {
                return Err(PlanIntentMismatch::HostGidForZeroIsHostRoot);
            }
        }
    }
    Ok(())
}

/// The protections a launch's mount block applies, and the broker-pre-NS
/// (ADR 0021 user-namespace) path drops.
///
/// `sys::pidfd_sys` gates the whole block - `apply_mount_actions`,
/// `apply_root_secret_masks`, `apply_device_mask_and_binds` and
/// `apply_private_procfs` - on `!in_ns_credentials`, because a path that
/// belongs to a mount inherited from the parent namespace cannot be
/// bind-mounted inside a user namespace. That is a real kernel constraint,
/// but it is not only a set of *grants* going missing: the block also carries
/// every *protection* the plan asked for, and a plan preflighted as if they
/// applied would run without them and say nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DroppedMountProtection {
    /// `apply_mount_actions`: the read-only paths the profile declares
    /// (`readOnlyPaths`, and `/nix/store` under `nixStoreReadOnly`). The
    /// namespace clones the broker's mount tree, so a plan that asked for a
    /// path read-only gets it read-write.
    ReadOnlyPaths,
    /// The `writablePaths`, `deviceBinds` and `bindMounts` the broker would
    /// have materialized. They silently never appear; the worker fails later
    /// on a path that is not there.
    MountGrants,
    /// `apply_device_mask_and_binds` plus `apply_private_procfs`: the `/dev`
    /// mask and the private `/proc` a `hideDeviceNodesByDefault` profile with
    /// a private pid namespace asks for. The device nodes stay visible and
    /// the `/proc` stays the broker's.
    DeviceMasking,
    /// `apply_root_secret_masks`: `/etc`, `/var`, `/home`, `/root`, `/run`,
    /// `/tmp`, `/boot`, `/mnt`, `/media`, `/srv` and `/opt` for a profile
    /// that runs as host root with the device mask on. These are the masks
    /// that keep a root runner from reading host state through a path its
    /// profile never mentioned.
    RootSecretMasks,
}

impl std::fmt::Display for DroppedMountProtection {
    /// The stable static-code spelling the audit record carries.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ReadOnlyPaths => "read-only-paths",
            Self::MountGrants => "mount-grants",
            Self::DeviceMasking => "device-masking",
            Self::RootSecretMasks => "root-secret-masks",
        })
    }
}

/// Decide what a broker-pre-NS launch silently loses to its mount policy.
///
/// Returns [`PlanIntentMismatch::UserNamespaceDropsRootSecretMasks`] for the
/// one drop that must not be a drop at all: a plan that runs as host root
/// with the device mask on, inside a user namespace where the masks are
/// skipped. That is the combination where the dropped masks are the only
/// thing between the runner and `/etc`, `/root` and `/var` on the host, and
/// no trusted role declares it - so it is refused rather than recorded.
/// Everything else the launch drops is returned for the caller to record,
/// because the trusted ADR 0021 roles (the Device workers, virtiofsd) are
/// *designed* for the user-namespace path: their render node arrives as a
/// pre-opened descriptor instead of a bind mount, and refusing them would
/// take away the isolation they exist for.
pub fn verify_user_namespace_mount_posture(
    plan_input: &SpawnRunnerPlanInput,
) -> Result<Vec<DroppedMountProtection>, PlanIntentMismatch> {
    if plan_input.user_namespace.is_none() {
        return Ok(Vec::new());
    }
    let policy = &plan_input.mount_policy;
    let mut dropped = Vec::new();
    if !policy.read_only_paths.is_empty() || policy.nix_store_read_only {
        dropped.push(DroppedMountProtection::ReadOnlyPaths);
    }
    if !policy.writable_paths.is_empty()
        || !policy.device_binds.is_empty()
        || !policy.bind_mounts.is_empty()
    {
        dropped.push(DroppedMountProtection::MountGrants);
    }
    if policy.hide_device_nodes_by_default && plan_input.namespaces.pid {
        dropped.push(DroppedMountProtection::DeviceMasking);
    }
    if policy.hide_device_nodes_by_default && plan_input.uid == 0 {
        return Err(PlanIntentMismatch::UserNamespaceDropsRootSecretMasks);
    }
    Ok(dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::sandbox_profile::WritablePath;

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
                path: "/var/lib/d2b/vms/corp-vm".to_owned(),
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
            subtree: "d2b.slice/corp-vm/cloud-hypervisor".to_owned(),
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
            "d2b.slice/corp-vm/cloud-hypervisor"
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
        i.binary_path = PathBuf::from("/tmp/nonexistent-d2b-binary-xyzzy");
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

    #[test]
    fn launch_plan_debug_redacts_host_values() {
        let input = good_input();
        let plan = preflight(&input).unwrap();
        let plan_debug = format!("{plan:?}");
        let input_debug = format!("{input:?}");
        for rendered in [plan_debug, input_debug] {
            assert!(!rendered.contains("/nix/store/abc"));
            assert!(!rendered.contains("PATH=/usr/bin"));
            assert!(!rendered.contains("cloud-hypervisor"));
            assert!(rendered.contains("<redacted>"));
        }
    }

    /// A TPM worker's trusted row, as the bundle resolves it: the swtpm
    /// executable, the worker's own principal, the `w1-swtpm` seccomp class
    /// and the ADR 0021 user-namespace mapping onto that same principal.
    fn swtpm_intent() -> d2b_core::bundle_resolver::ResolvedRunnerIntent {
        use d2b_core::bundle_resolver::UserNamespaceSpec;
        d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("runner:vm:host-system:role:swtpm-tpm")
            .with_vm_name("host-system")
            .with_role_id("swtpm-tpm")
            .with_role(d2b_core::processes::ProcessRole::Swtpm)
            .with_binary_path(PathBuf::from(
                "/nix/store/swtpm/bin/swtpm",
            ))
            .with_uid(60100)
            .with_gid(60100)
            .with_seccomp_policy_ref(Some("w1-swtpm"))
            .with_mount_policy(test_mount_policy())
            .with_namespaces(NamespaceSet {
                user: true,
                ..test_namespaces()
            })
            .with_user_namespace(Some(UserNamespaceSpec {
                host_uid_for_zero: 60100,
                host_gid_for_zero: 60100,
            }))
            .with_umask(Some(0o007))
            .build()
    }

    /// The plan the daemon's launch arm sends for [`swtpm_intent`]: the same
    /// executable, principal, seccomp class and mapping, plus the trusted
    /// state path in `argv` that the swtpm argv fence reads.
    fn swtpm_plan_input() -> SpawnRunnerPlanInput {
        SpawnRunnerPlanInput {
            binary_path: PathBuf::from("/nix/store/swtpm/bin/swtpm"),
            argv: vec![
                "swtpm".to_owned(),
                "--tpmstate".to_owned(),
                "dir=/var/lib/d2b/tpm/device-tpm0-tpm-state".to_owned(),
            ],
            uid: 60100,
            gid: 60100,
            supplementary_groups: Vec::new(),
            env: vec!["D2B_VM=host-system".to_owned()],
            capabilities: Vec::new(),
            namespaces: NamespaceSet {
                user: true,
                ..test_namespaces()
            },
            seccomp_policy_ref: Some("w1-swtpm".to_owned()),
            mount_policy: test_mount_policy(),
            cgroup_placement: test_cgroup_placement(),
            root_carve_out: false,
            skip_binary_exists_check: true,
            user_namespace: Some(UserNamespaceSpec {
                host_uid_for_zero: 60100,
                host_gid_for_zero: 60100,
            }),
            umask: Some(0o007),
        }
    }

    #[test]
    fn plan_matching_the_bundle_intent_is_admitted() {
        let mut plan = swtpm_plan_input();
        let intent = swtpm_intent();
        plan.umask = intent.umask;
        assert_eq!(verify_plan_against_intent(&plan, &intent), Ok(()));
    }

    /// The exploit from #619: the arguments name a trusted state path, so
    /// the argv fence is satisfied, while `binaryPath` - which becomes
    /// `argv[0]` and is what the worker actually *is* - names a different
    /// executable. The fence that catches it is the plan cross-check, and it
    /// refuses before any child exists.
    #[test]
    fn a_binary_path_that_is_not_the_intent_binary_is_refused() {
        let mut plan = swtpm_plan_input();
        let intent = swtpm_intent();
        plan.umask = intent.umask;
        plan.binary_path = PathBuf::from("/bin/cat");
        plan.argv[0] = "/bin/cat".to_owned();
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::BinaryPath)
        );
    }

    #[test]
    fn a_uid_gid_and_capability_list_the_intent_does_not_declare_are_refused() {
        let mut plan = swtpm_plan_input();
        let intent = swtpm_intent();
        plan.uid = 0;
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::Uid)
        );
        plan.uid = intent.uid;
        plan.gid = 0;
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::Gid)
        );
        plan.gid = intent.gid;
        plan.capabilities = vec!["CAP_SYS_ADMIN".to_owned()];
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::Capabilities)
        );
    }

    /// A payload that renames the seccomp class would drop the swtpm argv
    /// fence and the state-directory grant along with the filter, so the
    /// class is a plan field the bundle decides.
    #[test]
    fn a_renamed_seccomp_class_is_refused() {
        let mut plan = swtpm_plan_input();
        let intent = swtpm_intent();
        plan.seccomp_policy_ref = Some("w1-wayland-proxy".to_owned());
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::SeccompPolicy)
        );
    }

    /// The payload cannot assert the ADR 0003 root carve-out for itself: it
    /// is exactly the switch `preflight`'s uid-0 refusal is keyed on.
    #[test]
    fn a_self_asserted_root_carve_out_is_refused() {
        let mut plan = swtpm_plan_input();
        let intent = swtpm_intent();
        // The principal is the bundle's; only the carve-out is the payload's,
        // so the disagreement reported is the one under test.
        plan.root_carve_out = true;
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::RootCarveOut)
        );
    }

    #[test]
    fn a_user_namespace_the_intent_does_not_declare_is_refused() {
        let mut plan = good_input();
        plan.user_namespace = Some(UserNamespaceSpec {
            host_uid_for_zero: 0,
            host_gid_for_zero: 0,
        });
        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_binary_path(plan.binary_path.clone())
            .with_uid(plan.uid)
            .with_gid(plan.gid)
            .with_supplementary_groups(plan.supplementary_groups.clone())
            .with_capabilities(plan.capabilities.clone())
            .with_namespaces(plan.namespaces.clone())
            .with_seccomp_policy_ref(plan.seccomp_policy_ref.clone())
            .with_mount_policy(plan.mount_policy.clone())
            .build();
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::UserNamespaceUndeclared)
        );
    }

    /// Even a bundle row that itself declared a `0 -> 0` mapping does not
    /// get one for free: in-namespace root would be host root, so the
    /// refusal is keyed on the trusted `adrCarveOut`, not on the payload.
    #[test]
    fn a_user_namespace_mapping_root_onto_host_root_is_refused_without_a_carve_out() {
        let mut plan = swtpm_plan_input();
        let mut intent = swtpm_intent();
        let host_root = UserNamespaceSpec {
            host_uid_for_zero: 0,
            host_gid_for_zero: 0,
        };
        plan.user_namespace = Some(host_root);
        intent.user_namespace = Some(d2b_core::bundle_resolver::UserNamespaceSpec {
            host_uid_for_zero: 0,
            host_gid_for_zero: 0,
        });
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::HostUidForZeroIsHostRoot)
        );
        intent.root_carve_out = true;
        plan.root_carve_out = true;
        assert_eq!(verify_plan_against_intent(&plan, &intent), Ok(()));
    }

    #[test]
    fn a_user_namespace_mapping_that_disagrees_with_the_intent_is_refused() {
        let mut plan = swtpm_plan_input();
        let intent = swtpm_intent();
        plan.user_namespace = Some(UserNamespaceSpec {
            host_uid_for_zero: 60101,
            host_gid_for_zero: 60100,
        });
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::UserNamespaceMapping)
        );
    }

    #[test]
    fn a_user_namespace_launch_the_bundle_did_not_declare_is_refused() {
        let mut plan = good_input();
        plan.user_namespace = Some(UserNamespaceSpec {
            host_uid_for_zero: 1100,
            host_gid_for_zero: 1100,
        });
        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_binary_path(plan.binary_path.clone())
            .with_uid(plan.uid)
            .with_gid(plan.gid)
            .with_supplementary_groups(plan.supplementary_groups.clone())
            .with_capabilities(plan.capabilities.clone())
            .with_namespaces(plan.namespaces.clone())
            .with_seccomp_policy_ref(plan.seccomp_policy_ref.clone())
            .with_mount_policy(plan.mount_policy.clone())
            .build();
        assert_eq!(
            verify_plan_against_intent(&plan, &intent),
            Err(PlanIntentMismatch::UserNamespaceUndeclared)
        );
    }

    /// The mount block's masks are protections, not just grants, and the
    /// user-namespace path skips all of them. A plan that asks for the root
    /// secret masks is refused; the read-only and device drops the trusted
    /// ADR 0021 roles accept are reported so the caller records them.
    #[test]
    fn a_user_namespace_launch_that_drops_the_root_secret_masks_is_refused() {
        let mut plan = swtpm_plan_input();
        plan.uid = 0;
        plan.mount_policy.hide_device_nodes_by_default = true;
        assert_eq!(
            verify_user_namespace_mount_posture(&plan),
            Err(PlanIntentMismatch::UserNamespaceDropsRootSecretMasks)
        );
    }

    #[test]
    fn a_user_namespace_launch_reports_the_protections_the_mount_block_drops() {
        let mut plan = swtpm_plan_input();
        plan.namespaces.pid = true;
        plan.mount_policy.device_binds = vec!["/dev/kvm".to_owned()];
        assert_eq!(
            verify_user_namespace_mount_posture(&plan),
            Ok(vec![
                DroppedMountProtection::ReadOnlyPaths,
                DroppedMountProtection::MountGrants,
                DroppedMountProtection::DeviceMasking,
            ])
        );
    }

    #[test]
    fn a_launch_without_a_user_namespace_drops_nothing() {
        let mut plan = swtpm_plan_input();
        plan.user_namespace = None;
        plan.namespaces.pid = true;
        assert_eq!(verify_user_namespace_mount_posture(&plan), Ok(Vec::new()));
    }

    #[test]
    fn user_namespace_with_zero_uid_is_allowed_in_plan_layer() {
        // The preflight itself does not decide the host mapping: the plan
        // layer passes it through, and
        // `verify_plan_against_intent` is what refuses a `0 -> 0` mapping
        // the bundle row did not carve out. This test pins the plan layer's
        // pass-through semantics so the two layers are not confused.
        let mut input = good_input();
        input.user_namespace = Some(UserNamespaceSpec {
            host_uid_for_zero: 0,
            host_gid_for_zero: 0,
        });
        let plan = preflight(&input).unwrap();
        assert_eq!(plan.user_namespace.unwrap().host_uid_for_zero, 0);
    }
}
