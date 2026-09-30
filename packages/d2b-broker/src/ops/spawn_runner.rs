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

/// The realization facet and refusal vocabulary of an admitted presentation.
///
/// Both live here, beside the launch plan they shape, because the fence and
/// the plan are the same decision: a mount the declared realization cannot
/// place is refused in [`preflight`], which runs before any descriptor is
/// opened and before any child exists.
mod presentation {
    pub use crate::sys::pidfd_sys::{PresentationBindSpec, PresentationRealization};
    pub use d2b_contracts_resource::v3::{BindingRealizationFacet, RequestedRights};
    pub use d2b_core::execution_plan::{ExecutionPlan, PlannedDestination, PrivateBacking};

    /// The one facet a filesystem presentation realizes.
    pub const FILESYSTEM_PRESENTATION: BindingRealizationFacet =
        BindingRealizationFacet::FilesystemPresentation;
}

pub use presentation::{
    BindingRealizationFacet, PresentationBindSpec, PresentationRealization, RequestedRights,
};
use presentation::{ExecutionPlan, PlannedDestination, PrivateBacking};

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
    /// How this launch's presentation is realized (KTD11).
    ///
    /// The facet comes from the trusted implementation contract, never from a
    /// role name, a seccomp label, or a family switch. There is no pre-graph
    /// posture: a launch always states the realization its declared
    /// capability names.
    pub presentation: PresentationRealization,
    /// The admitted destinations prepared for this launch, resolved from an
    /// exact execution plan by [`realize_presentation`].
    pub admitted_presentation: AdmittedPresentation,
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
    /// An admitted presentation was handed to a realization that prepares no
    /// private mount tree. The requested mount is refused rather than skipped
    /// (KTD11).
    PresentationRequiresFilesystemRealization,
    /// An admitted presentation named a relative private execution root, so
    /// its destinations could not be kept out of the host root.
    PresentationRequiresPrivateExecutionRoot,
    /// The launch's argv names a host path the admitted presentation does not
    /// grant. A worker reaches the source through its destination or through a
    /// declared inherited descriptor.
    PresentationArgvNamesHostSource,
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
            Self::PresentationRequiresFilesystemRealization => f.write_str(
                "an admitted presentation requires the filesystem-presentation realization, \
                 which prepares a private mount tree; this realization cannot place the \
                 requested mount",
            ),
            Self::PresentationRequiresPrivateExecutionRoot => f.write_str(
                "an admitted presentation requires an absolute private execution root so \
                 its destinations are prepared inside the runner's own mount tree",
            ),
            Self::PresentationArgvNamesHostSource => f.write_str(
                "launch argv names a host source path directly; a worker must address the \
                 admitted destination or a declared inherited descriptor",
            ),
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
    /// How this launch's presentation is realized (KTD11). See
    /// [`SpawnRunnerPlan::presentation`].
    pub presentation: PresentationRealization,
    /// The admitted destinations prepared for this launch. See
    /// [`SpawnRunnerPlan::admitted_presentation`].
    pub admitted_presentation: AdmittedPresentation,
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
    // KTD11: an admitted presentation is a request the declared realization
    // has to honor. Every check here is pure and runs before any descriptor
    // is opened and before any child exists, so a launch refused for a mount
    // it cannot realize leaves no mount, no prepared destination, and no
    // child behind it. There is no branch below that reports such a launch
    // as ready.
    let presentation = &input.admitted_presentation;
    if !presentation.binds.is_empty()
        && input.presentation != PresentationRealization::FilesystemPresentation
    {
        return Err(SpawnRunnerError::PresentationRequiresFilesystemRealization);
    }
    if !presentation.binds.is_empty() && !presentation.private_execution_root.is_absolute() {
        return Err(SpawnRunnerError::PresentationRequiresPrivateExecutionRoot);
    }
    if fence_presentation_argv(&input.argv, presentation).is_err() {
        return Err(SpawnRunnerError::PresentationArgvNamesHostSource);
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
        presentation: input.presentation,
        admitted_presentation: input.admitted_presentation.clone(),
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

// ---------------------------------------------------------------------------
// Effective Volume presentation (U11, KTD11)
// ---------------------------------------------------------------------------

/// The private execution root and the admitted binds prepared inside it.
///
/// The root is a directory the broker already owns; the child mounts a fresh
/// `tmpfs` on it inside its own mount namespace, so every destination
/// directory and every byte written through a presentation exists only there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedPresentation {
    pub private_execution_root: PathBuf,
    pub binds: Vec<PresentationBindSpec>,
}

/// Why one presentation was refused.
///
/// Every variant is a refusal, not a downgrade. The closed set is what makes
/// "unsupported presentation combinations refuse" checkable rather than a
/// claim: no variant means "do less than was asked".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresentationRefusal {
    /// The declaration is a namespace-first service source and the plan
    /// nonetheless carries a filesystem-presentation destination. ADR 0021's
    /// sandbox realizes a source, not a consumer destination, so this request
    /// cannot be honored and the launch is refused.
    ServiceSourceCannotPresentFilesystem { destination: PathBuf },
    /// A destination carries a presentation facet this realization does not
    /// implement.
    UnsupportedPresentationFacet {
        destination: PathBuf,
        facet: &'static str,
    },
    /// A filesystem presentation's source resolved to something that is not a
    /// filesystem backing. A device node, a socket, or a fabric is not
    /// bindable as a source tree, and approximating it would present
    /// something the graph never admitted.
    SourceBackingIsNotFilesystem { source: PathBuf },
    /// A destination lies outside the launch's private execution root, so it
    /// would be prepared in the host root and be visible to every process on
    /// the host.
    DestinationOutsidePrivateRoot { destination: PathBuf },
    /// The destination is writable but the admitted view's rights carry no
    /// mutating right. The broker refuses rather than presenting something
    /// the graph admitted read rights to.
    ReadWriteViewWithoutWriteRight { destination: PathBuf },
    /// The launch's argv names a host path the admitted presentation does not
    /// grant. A worker reaches the source through its destination or through a
    /// declared inherited descriptor.
    ArgvNamesHostSource { path: PathBuf },
}

impl std::fmt::Display for PresentationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceSourceCannotPresentFilesystem { destination } => write!(
                f,
                "namespace-first service source cannot present a filesystem mount at {}: \
                 the service realizes its admitted source inside its own verified sandbox, \
                 so the requested presentation is refused rather than skipped",
                destination.display()
            ),
            Self::UnsupportedPresentationFacet {
                destination,
                facet,
            } => write!(
                f,
                "presentation {facet} at {} is not a facet this realization supports",
                destination.display()
            ),
            Self::SourceBackingIsNotFilesystem { source } => write!(
                f,
                "presentation source {} is not a filesystem backing",
                source.display()
            ),
            Self::DestinationOutsidePrivateRoot { destination } => write!(
                f,
                "presentation destination {} is outside the launch's private execution root",
                destination.display()
            ),
            Self::ReadWriteViewWithoutWriteRight { destination } => write!(
                f,
                "presentation destination {} is writable but the admitted view carries no \
                 mutating right",
                destination.display()
            ),
            Self::ArgvNamesHostSource { path } => write!(
                f,
                "launch argv names the host source path {} directly; a worker reaches the \
                 source through its destination or a declared inherited descriptor",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PresentationRefusal {}

/// The facets a filesystem presentation may NOT carry.
///
/// A destination declaring anything other than
/// [`BindingRealizationFacet::FilesystemPresentation`] is refused by name,
/// with the contract's own kebab-case token, so an audit record names the same
/// facet the declaration did.
const fn forbidden_facet_token(facet: BindingRealizationFacet) -> Option<&'static str> {
    match facet {
        BindingRealizationFacet::FilesystemPresentation => None,
        BindingRealizationFacet::ConsumerDeviceSlot => Some("consumer-device-slot"),
        BindingRealizationFacet::DeviceAttachment => Some("device-attachment"),
        BindingRealizationFacet::NamespaceInterface => Some("namespace-interface"),
        BindingRealizationFacet::SharedFabric => Some("shared-fabric"),
        BindingRealizationFacet::EndpointDescriptor => Some("endpoint-descriptor"),
        BindingRealizationFacet::EndpointPathname => Some("endpoint-pathname"),
        BindingRealizationFacet::CredentialDelivery => Some("credential-delivery"),
    }
}

/// Whether one destination's admitted rights let its presentation be writable.
///
/// `Observe` and `Consume` are read-only uses; `Mutate`, `Share`, and
/// `Exclusive` each claim a writer, so any of them admits a writable mount.
const fn rights_admit_write(rights: RequestedRights) -> bool {
    matches!(
        rights,
        RequestedRights::Mutate | RequestedRights::Share | RequestedRights::Exclusive
    )
}

/// Resolve one destination against the plan's exact sources.
///
/// # Errors
///
/// Refuses a destination carrying a facet this realization does not
/// implement, a destination outside the private execution root, a plan with
/// no filesystem-backed source, and a writable destination whose admitted
/// view carries no mutating right.
fn resolve_destination(
    plan: &ExecutionPlan,
    destination: &PlannedDestination,
    private_execution_root: &Path,
    read_only: bool,
) -> Result<PresentationBindSpec, PresentationRefusal> {
    let destination_path = destination.path().as_path();
    if let Some(facet) = forbidden_facet_token(destination.presentation()) {
        return Err(PresentationRefusal::UnsupportedPresentationFacet {
            destination: destination_path.to_path_buf(),
            facet,
        });
    }
    let relative = destination_path
        .strip_prefix(private_execution_root)
        .map_err(|_| PresentationRefusal::DestinationOutsidePrivateRoot {
            destination: destination_path.to_path_buf(),
        })?;
    if relative.as_os_str().is_empty() {
        return Err(PresentationRefusal::DestinationOutsidePrivateRoot {
            destination: destination_path.to_path_buf(),
        });
    }
    let source = plan
        .sources()
        .iter()
        .find(|source| source.backing() == PrivateBacking::Filesystem)
        .ok_or_else(|| PresentationRefusal::SourceBackingIsNotFilesystem {
            source: destination_path.to_path_buf(),
        })?;
    if !read_only
        && source
            .views()
            .first()
            .is_some_and(|view| !rights_admit_write(view.rights()))
    {
        return Err(PresentationRefusal::ReadWriteViewWithoutWriteRight {
            destination: destination_path.to_path_buf(),
        });
    }
    Ok(PresentationBindSpec {
        source: source.backing_path().as_path().to_path_buf(),
        destination: destination_path.to_path_buf(),
        read_only,
    })
}

/// Turn one admitted plan into the presentation the broker will prepare.
///
/// `read_only` is the access the relationship itself admitted. It is never
/// inferred from the launch: a read-only binding mounts read-only, and write
/// access is confined to the admitted view, which is the only subtree the
/// bind exposes.
///
/// # Errors
///
/// Refuses the whole launch when the declaration cannot realize what the plan
/// asks for. A namespace-first service source handed a
/// filesystem-presentation destination is the load-bearing case (AE6): the
/// service realizes its admitted source inside its own verified sandbox, and
/// skipping the mount the consumer asked for is not an answer.
pub fn realize_presentation(
    plan: &ExecutionPlan,
    realization: PresentationRealization,
    private_execution_root: &Path,
    read_only: bool,
) -> Result<AdmittedPresentation, PresentationRefusal> {
    let destinations = plan.destinations();
    if realization == PresentationRealization::NamespaceFirstServiceSource {
        if let Some(destination) = destinations
            .iter()
            .find(|destination| destination.presentation() == presentation::FILESYSTEM_PRESENTATION)
        {
            return Err(PresentationRefusal::ServiceSourceCannotPresentFilesystem {
                destination: destination.path().as_path().to_path_buf(),
            });
        }
        // ADR 0021's zero-host-capability launch: the broker prepares no
        // private execution root, no destination, and no mount on this leg.
        return Ok(AdmittedPresentation {
            private_execution_root: PathBuf::new(),
            binds: Vec::new(),
        });
    }
    if !private_execution_root.is_absolute() {
        return Err(PresentationRefusal::DestinationOutsidePrivateRoot {
            destination: private_execution_root.to_path_buf(),
        });
    }
    let mut binds = Vec::with_capacity(destinations.len());
    for destination in destinations {
        binds.push(resolve_destination(
            plan,
            destination,
            private_execution_root,
            read_only,
        )?);
    }
    Ok(AdmittedPresentation {
        private_execution_root: private_execution_root.to_path_buf(),
        binds,
    })
}

/// Refuse a launch whose argv reaches the source by naming its host path.
///
/// The fence reads only the path-valued flags a worker uses to open what it
/// was given, so a shell script or a flag that merely contains a path is
/// untouched. What it catches is the spelling the graph does not grant: a
/// worker that would open a host path itself instead of reading the
/// destination the broker prepared for it.
///
/// Only the admitted DESTINATION is accepted, never the admitted source. The
/// source is what the graph grants a VIEW of; naming it would hand the worker
/// the whole backing tree the view was taken from, and would re-open the host
/// path the private execution root exists to hide. A declared inherited
/// descriptor (`/proc/self/fd/N`) names no host path and is untouched.
pub fn fence_presentation_argv(
    argv: &[String],
    presentation: &AdmittedPresentation,
) -> Result<(), PresentationRefusal> {
    if presentation.binds.is_empty() {
        return Ok(());
    }
    for argument in argv {
        let Some(value) = argument
            .strip_prefix("--shared-dir=")
            .or_else(|| argument.strip_prefix("--source="))
            .or_else(|| argument.strip_prefix("--dir="))
        else {
            continue;
        };
        let named = Path::new(value);
        if presentation
            .binds
            .iter()
            .any(|bind| bind.destination.as_path() == named)
        {
            continue;
        }
        if named.is_absolute() {
            return Err(PresentationRefusal::ArgvNamesHostSource {
                path: named.to_path_buf(),
            });
        }
    }
    Ok(())
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
        presentation: crate::ops::spawn_runner::PresentationRealization::NamespaceFirstServiceSource,
        admitted_presentation: crate::ops::spawn_runner::AdmittedPresentation {
            private_execution_root: std::path::PathBuf::new(),
            binds: Vec::new(),
        },
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

    #[test]
    fn user_namespace_with_zero_uid_is_allowed_in_plan_layer() {
        // The preflight does NOT validate the host UID - the
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
