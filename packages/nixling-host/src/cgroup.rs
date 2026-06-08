//! Host-prepare cgroup module.
//!
//! Implements the 8-step cgroup v2 delegation algorithm plus the chown /
//! kill-scope / no-internal-process / partition-member /
//! non-root-delegation rules.
//!
//! ## Invariants enforced here
//!
//! 1. The unified hierarchy must be present (`/sys/fs/cgroup/cgroup.controllers`).
//! 2. The required controller set `{cpu, memory, io, pids, cpuset}` must be
//!    advertised on the root before any subtree is created.
//! 3. Before enabling `+cpuset`, an ancestor with an empty `cpuset.cpus` or
//!    `cpuset.mems` inherits from `cpuset.cpus.effective` / `cpuset.mems.effective`.
//! 4. `cgroup.subtree_control` is rewritten in the strict order
//!    `+cpu, +memory, +io, +pids, +cpuset` with a re-read verification after
//!    each individual enable.
//! 5. `cpuset.cpus.partition` STAYS `member`. A debug assertion blows up if
//!    any caller passes the partition-root key into the writer; releases
//!    fail closed by returning [`CgroupError::CgroupPartitionRootForbidden`].
//! 6. Threaded cgroups are forbidden — `cgroup.type=threaded` is refused.
//! 7. `nixling.slice` and intermediate VM cgroup directories must be
//!    process-free; only leaf role cgroups carry processes.
//! 8. `cgroup.kill` is allowed only on broker/daemon-owned VM or role leaves;
//!    ancestor `cgroup.kill` is refused with
//!    [`CgroupError::CgroupKillOnAncestorRefused`].
//! 9. The delegation must NOT be performed while running as uid 0; the
//!    broker is the only root-effective component, and even it walks this
//!    code path as the dropped-privilege `nixlingd` view.
//!
//! ## Backend model
//!
//! The module is parameterised over a [`CgroupBackend`] trait so the
//! production [`RealCgroupBackend`] (which uses `rustix`'s `openat2 +
//! O_NOFOLLOW + RESOLVE_BENEATH` and fd-relative `fchown` / writes) and
//! the in-memory [`fake::FakeCgroupBackend`] (gated behind
//! `cfg(any(test, feature = "fake-backends"))`) share the same algorithm.
//! L1c canary tests drive the algorithm through the fake backend.

use std::fmt;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Cgroup v2 controllers tracked by the delegation algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Controller {
    Cpu,
    Memory,
    Io,
    Pids,
    Cpuset,
}

impl Controller {
    pub const REQUIRED: &'static [Controller] = &[
        Controller::Cpu,
        Controller::Memory,
        Controller::Io,
        Controller::Pids,
        Controller::Cpuset,
    ];

    /// Subtree-enable order for the delegation algorithm.
    pub const ENABLE_ORDER: &'static [Controller] = &[
        Controller::Cpu,
        Controller::Memory,
        Controller::Io,
        Controller::Pids,
        Controller::Cpuset,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Controller::Cpu => "cpu",
            Controller::Memory => "memory",
            Controller::Io => "io",
            Controller::Pids => "pids",
            Controller::Cpuset => "cpuset",
        }
    }

    pub fn enable_token(&self) -> String {
        format!("+{}", self.as_str())
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim() {
            "cpu" => Some(Controller::Cpu),
            "memory" => Some(Controller::Memory),
            "io" => Some(Controller::Io),
            "pids" => Some(Controller::Pids),
            "cpuset" => Some(Controller::Cpuset),
            _ => None,
        }
    }
}

impl fmt::Display for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Snapshot of the controllers advertised on a cgroup's `cgroup.controllers`
/// file at the moment of probing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledControllers {
    controllers: Vec<Controller>,
}

impl EnabledControllers {
    pub fn from_controllers<I: IntoIterator<Item = Controller>>(iter: I) -> Self {
        let mut controllers: Vec<Controller> = iter.into_iter().collect();
        controllers.sort();
        controllers.dedup();
        Self { controllers }
    }

    pub fn contains(&self, c: Controller) -> bool {
        self.controllers.binary_search(&c).is_ok()
    }

    pub fn as_slice(&self) -> &[Controller] {
        &self.controllers
    }

    pub fn missing(&self, required: &[Controller]) -> Vec<Controller> {
        required
            .iter()
            .copied()
            .filter(|c| !self.contains(*c))
            .collect()
    }
}

/// Canonical cgroup error codes. The discriminant string matches the
/// kebab-case error code that flows through the CLI golden table and
/// into the broker audit record `error_kind` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupError {
    /// Unified hierarchy probe failed — `/sys/fs/cgroup/cgroup.controllers`
    /// is missing or unreadable. CLI exit code 1; matches plan-named
    /// `cgroup-v2-unified-not-present`.
    CgroupV2UnifiedNotPresent { detail: String },
    /// One or more required controllers are absent from `cgroup.controllers`
    /// on the delegation root. Matches plan-named `cgroup-controllers-missing`.
    CgroupControllersMissing { missing: Vec<Controller> },
    /// Delegation was attempted while running as uid 0 (or the host cannot
    /// support non-root delegation). Matches plan-named
    /// `cgroup-delegation-refused`.
    CgroupDelegationRefused { detail: String },
    /// `cgroup.kill` was attempted on an ancestor (e.g. `nixling.slice`
    /// or an intermediate VM cgroup). Matches plan-named
    /// `cgroup-kill-on-ancestor-refused`.
    CgroupKillOnAncestorRefused { path: PathBuf },
    /// cpuset inheritance could not produce non-empty `.effective` files.
    CpusetInheritanceFailed { path: PathBuf, detail: String },
    /// `nixling.slice` or an intermediate VM cgroup contained running
    /// processes when the no-internal-process invariant was checked.
    CgroupInternalProcessesPresent { path: PathBuf, pids: Vec<u32> },
    /// Subtree-control verification failed after a write — the re-read
    /// did not contain the controller we just enabled.
    SubtreeControlEnableFailed {
        path: PathBuf,
        controller: Controller,
    },
    /// Threaded cgroup encountered — forbidden.
    ThreadedCgroupForbidden { path: PathBuf },
    /// Attempt to write `cpuset.cpus.partition` (partition roots are
    /// forbidden; ancestors and `nixling.slice` stay `member`).
    CgroupPartitionRootForbidden { path: PathBuf },
    /// Subtree-control write on `parent` enabled `controller` (the
    /// re-read of `parent/cgroup.subtree_control` confirmed it) but the
    /// child cgroup's `cgroup.controllers` does not advertise the
    /// controller. The delegation must fail closed before chown.
    CgroupControllerNotExposedToChild {
        controller: Controller,
        parent: PathBuf,
        child: PathBuf,
    },
    /// Backend I/O error (path safety violation, permission denied, ...).
    Io { detail: String },
}

impl CgroupError {
    /// Returns the plan-named kebab-case error code for this variant.
    /// Used by audit records and the CLI error golden table.
    pub fn code(&self) -> &'static str {
        match self {
            CgroupError::CgroupV2UnifiedNotPresent { .. } => "cgroup-v2-unified-not-present",
            CgroupError::CgroupControllersMissing { .. } => "cgroup-controllers-missing",
            CgroupError::CgroupDelegationRefused { .. } => "cgroup-delegation-refused",
            CgroupError::CgroupKillOnAncestorRefused { .. } => "cgroup-kill-on-ancestor-refused",
            CgroupError::CpusetInheritanceFailed { .. } => "cpuset-inheritance-failed",
            CgroupError::CgroupInternalProcessesPresent { .. } => {
                "cgroup-internal-processes-present"
            }
            CgroupError::SubtreeControlEnableFailed { .. } => {
                "cgroup-subtree-control-enable-failed"
            }
            CgroupError::ThreadedCgroupForbidden { .. } => "cgroup-threaded-forbidden",
            CgroupError::CgroupPartitionRootForbidden { .. } => "cgroup-partition-root-forbidden",
            CgroupError::CgroupControllerNotExposedToChild { .. } => {
                "cgroup-controller-not-exposed-to-child"
            }
            CgroupError::Io { .. } => "cgroup-io-error",
        }
    }
}

impl fmt::Display for CgroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CgroupError::CgroupV2UnifiedNotPresent { detail } => {
                write!(f, "cgroup v2 unified hierarchy not present: {detail}")
            }
            CgroupError::CgroupControllersMissing { missing } => {
                let names: Vec<&str> = missing.iter().map(Controller::as_str).collect();
                write!(
                    f,
                    "required cgroup controllers missing: {}",
                    names.join(",")
                )
            }
            CgroupError::CgroupDelegationRefused { detail } => {
                write!(f, "cgroup delegation refused: {detail}")
            }
            CgroupError::CgroupKillOnAncestorRefused { path } => {
                write!(
                    f,
                    "cgroup.kill refused on ancestor cgroup {}",
                    path.display()
                )
            }
            CgroupError::CpusetInheritanceFailed { path, detail } => {
                write!(
                    f,
                    "cpuset inheritance failed on {}: {detail}",
                    path.display()
                )
            }
            CgroupError::CgroupInternalProcessesPresent { path, pids } => {
                write!(
                    f,
                    "{} holds {} non-leaf processes (pids: {:?})",
                    path.display(),
                    pids.len(),
                    pids
                )
            }
            CgroupError::SubtreeControlEnableFailed { path, controller } => {
                write!(
                    f,
                    "verification after enabling {controller} on {} failed",
                    path.display()
                )
            }
            CgroupError::ThreadedCgroupForbidden { path } => {
                write!(f, "threaded cgroup forbidden at {}", path.display())
            }
            CgroupError::CgroupPartitionRootForbidden { path } => {
                write!(
                    f,
                    "cpuset.cpus.partition write rejected on {} (stays member)",
                    path.display()
                )
            }
            CgroupError::CgroupControllerNotExposedToChild {
                controller,
                parent,
                child,
            } => write!(
                f,
                "controller {controller} enabled in {}/cgroup.subtree_control but not exposed to child {}",
                parent.display(),
                child.display()
            ),
            CgroupError::Io { detail } => write!(f, "cgroup backend I/O error: {detail}"),
        }
    }
}

impl std::error::Error for CgroupError {}

/// Default unified hierarchy mount point. The probe target is
/// `<root>/cgroup.controllers`.
pub const UNIFIED_HIERARCHY_ROOT: &str = "/sys/fs/cgroup";
/// Canonical nixling slice name under the unified hierarchy.
pub const NIXLING_SLICE_NAME: &str = "nixling.slice";

/// Identifier for the delegated nixling slice path.
pub fn nixling_slice_path() -> PathBuf {
    Path::new(UNIFIED_HIERARCHY_ROOT).join(NIXLING_SLICE_NAME)
}

/// Trait covering the I/O surface the algorithm needs. The real
/// implementation uses fd-relative `openat2` + `O_NOFOLLOW` +
/// `RESOLVE_BENEATH`; the fake implementation backs everything with an
/// in-memory tree so L1c tests exercise every algorithm step without
/// touching a real cgroupfs.
pub trait CgroupBackend {
    fn current_uid(&self) -> u32;

    fn read_file(&self, path: &Path) -> Result<String, CgroupError>;
    fn write_file(&self, path: &Path, contents: &str) -> Result<(), CgroupError>;
    fn exists(&self, path: &Path) -> bool;

    fn mkdir(&self, path: &Path) -> Result<(), CgroupError>;
    fn fchown(&self, path: &Path, uid: u32, gid: u32) -> Result<(), CgroupError>;

    /// Returns the PIDs currently inside `cgroup.procs` for the given
    /// cgroup directory.
    fn read_procs(&self, dir: &Path) -> Result<Vec<u32>, CgroupError>;
}

// ---------------------------------------------------------------------------
// Algorithm primitives
// ---------------------------------------------------------------------------

/// Step 8: refuse delegation while running as uid 0.
pub fn require_non_root_delegation<B: CgroupBackend>(backend: &B) -> Result<(), CgroupError> {
    if backend.current_uid() == 0 {
        return Err(CgroupError::CgroupDelegationRefused {
            detail: "delegation must be driven through the non-root nixlingd; refuse uid 0"
                .to_owned(),
        });
    }
    Ok(())
}

/// Step 8 (probe): unified hierarchy must be present at `root`.
pub fn probe_unified_hierarchy<B: CgroupBackend>(
    backend: &B,
    root: &Path,
) -> Result<(), CgroupError> {
    let probe = root.join("cgroup.controllers");
    if !backend.exists(&probe) {
        return Err(CgroupError::CgroupV2UnifiedNotPresent {
            detail: format!("{} does not exist", probe.display()),
        });
    }
    backend.read_file(&probe).map_err(|err| match err {
        CgroupError::Io { detail } => CgroupError::CgroupV2UnifiedNotPresent { detail },
        other => other,
    })?;
    Ok(())
}

/// Step 1: assert the unified hierarchy advertises every required controller.
pub fn require_controllers<B: CgroupBackend>(
    backend: &B,
    root: &Path,
    required: &[Controller],
) -> Result<EnabledControllers, CgroupError> {
    let raw = backend.read_file(&root.join("cgroup.controllers"))?;
    let controllers: Vec<Controller> = raw
        .split_ascii_whitespace()
        .filter_map(Controller::from_token)
        .collect();
    let enabled = EnabledControllers::from_controllers(controllers);
    let missing = enabled.missing(required);
    if !missing.is_empty() {
        return Err(CgroupError::CgroupControllersMissing { missing });
    }
    Ok(enabled)
}

fn read_trimmed<B: CgroupBackend>(backend: &B, path: &Path) -> Result<String, CgroupError> {
    backend.read_file(path).map(|s| s.trim().to_owned())
}

/// Step 2: cpuset inheritance — copy `.effective` into `cpuset.cpus` /
/// `cpuset.mems` when empty and verify `.effective` is non-empty.
pub fn prepare_cpuset_inheritance<B: CgroupBackend>(
    backend: &B,
    path: &Path,
) -> Result<(), CgroupError> {
    for (file, effective_file) in [
        ("cpuset.cpus", "cpuset.cpus.effective"),
        ("cpuset.mems", "cpuset.mems.effective"),
    ] {
        let target = path.join(file);
        let effective = path.join(effective_file);

        let effective_value = read_trimmed(backend, &effective).map_err(|err| {
            CgroupError::CpusetInheritanceFailed {
                path: path.to_path_buf(),
                detail: format!("read {}: {}", effective.display(), err),
            }
        })?;
        if effective_value.is_empty() {
            return Err(CgroupError::CpusetInheritanceFailed {
                path: path.to_path_buf(),
                detail: format!("{} is empty", effective.display()),
            });
        }

        let current = read_trimmed(backend, &target).unwrap_or_default();
        if current.is_empty() {
            backend
                .write_file(&target, &effective_value)
                .map_err(|err| CgroupError::CpusetInheritanceFailed {
                    path: path.to_path_buf(),
                    detail: format!("write {}: {}", target.display(), err),
                })?;
        }

        // Re-read post-write verification.
        let verified = read_trimmed(backend, &effective).map_err(|err| {
            CgroupError::CpusetInheritanceFailed {
                path: path.to_path_buf(),
                detail: format!("re-read {}: {}", effective.display(), err),
            }
        })?;
        if verified.is_empty() {
            return Err(CgroupError::CpusetInheritanceFailed {
                path: path.to_path_buf(),
                detail: format!("{} still empty after inheritance", effective.display()),
            });
        }
    }
    Ok(())
}

/// Step 3: enable controllers in `cgroup.subtree_control` in the strict
/// order, verifying re-read after each individual enable. When `child`
/// is `Some`, the child cgroup's `cgroup.controllers` file is also
/// re-read after each enable ("Each enable is verified by re-reading
/// cgroup.subtree_control AND cgroup.controllers on the child").
pub fn enable_subtree_controllers<B: CgroupBackend>(
    backend: &B,
    path: &Path,
    controllers: &[Controller],
) -> Result<(), CgroupError> {
    enable_subtree_controllers_with_child(backend, path, None, controllers)
}

/// Variant of [`enable_subtree_controllers`] that additionally verifies
/// the child cgroup's `cgroup.controllers` advertises the just-enabled
/// controller. Fail-closed with
/// [`CgroupError::CgroupControllerNotExposedToChild`].
pub fn enable_subtree_controllers_with_child<B: CgroupBackend>(
    backend: &B,
    path: &Path,
    child: Option<&Path>,
    controllers: &[Controller],
) -> Result<(), CgroupError> {
    let subtree = path.join("cgroup.subtree_control");
    for controller in controllers {
        if matches!(controller, Controller::Cpuset) {
            prepare_cpuset_inheritance(backend, path)?;
        }
        backend.write_file(&subtree, &controller.enable_token())?;
        let after = backend.read_file(&subtree).unwrap_or_default();
        let enabled_now: Vec<Controller> = after
            .split_ascii_whitespace()
            .filter_map(Controller::from_token)
            .collect();
        if !enabled_now.contains(controller) {
            return Err(CgroupError::SubtreeControlEnableFailed {
                path: path.to_path_buf(),
                controller: *controller,
            });
        }
        if let Some(child_path) = child {
            let child_ctrl = backend
                .read_file(&child_path.join("cgroup.controllers"))
                .unwrap_or_default();
            let child_exposed: Vec<Controller> = child_ctrl
                .split_ascii_whitespace()
                .filter_map(Controller::from_token)
                .collect();
            if !child_exposed.contains(controller) {
                return Err(CgroupError::CgroupControllerNotExposedToChild {
                    controller: *controller,
                    parent: path.to_path_buf(),
                    child: child_path.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

/// Step 4 enforcement helper: the algorithm NEVER writes
/// `cpuset.cpus.partition`. Any code path that tries to is treated as a
/// programmer bug — a `debug_assert!` blows up in development builds,
/// and release builds return [`CgroupError::CgroupPartitionRootForbidden`].
pub fn assert_partition_member_only(path: &Path, key: &str) -> Result<(), CgroupError> {
    debug_assert!(
        key != "cpuset.cpus.partition",
        "writing cpuset.cpus.partition is forbidden (stay 'member' at {})",
        path.display()
    );
    if key == "cpuset.cpus.partition" {
        return Err(CgroupError::CgroupPartitionRootForbidden {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Step 5/6 helper: refuse threaded cgroups.
pub fn assert_not_threaded<B: CgroupBackend>(backend: &B, path: &Path) -> Result<(), CgroupError> {
    let type_file = path.join("cgroup.type");
    if !backend.exists(&type_file) {
        return Ok(());
    }
    let value = read_trimmed(backend, &type_file)?;
    if value == "threaded" {
        return Err(CgroupError::ThreadedCgroupForbidden {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Step 5: assert `cgroup.procs` is empty on an intermediate (non-leaf)
/// cgroup. Returns [`CgroupError::CgroupInternalProcessesPresent`]
/// listing the offending pids when not.
pub fn assert_no_internal_processes<B: CgroupBackend>(
    backend: &B,
    path: &Path,
) -> Result<(), CgroupError> {
    let pids = backend.read_procs(path)?;
    if !pids.is_empty() {
        return Err(CgroupError::CgroupInternalProcessesPresent {
            path: path.to_path_buf(),
            pids,
        });
    }
    Ok(())
}

/// Step 7: `cgroup.kill` is allowed only on a leaf. The caller passes
/// the cgroup directory and the leaves it is permitted to kill; any
/// path not in the leaf set is refused.
pub fn cgroup_kill_leaf_only<B: CgroupBackend>(
    backend: &B,
    path: &Path,
    leaf_set: &[PathBuf],
) -> Result<(), CgroupError> {
    let is_leaf = leaf_set.iter().any(|leaf| leaf == path);
    if !is_leaf {
        return Err(CgroupError::CgroupKillOnAncestorRefused {
            path: path.to_path_buf(),
        });
    }
    backend.write_file(&path.join("cgroup.kill"), "1")
}

/// Step 6: chown the entire delegated subtree to `nixlingd`. Performs
/// fd-based `fchown` (via the backend); never path-based per filesystem
/// path-safety rules.
pub fn chown_subtree_to_nixlingd<B: CgroupBackend>(
    backend: &B,
    path: &Path,
    uid: u32,
    gid: u32,
) -> Result<(), CgroupError> {
    for entry in [
        "",
        "cgroup.procs",
        "cgroup.threads",
        "cgroup.subtree_control",
        "cgroup.events",
    ] {
        let target = if entry.is_empty() {
            path.to_path_buf()
        } else {
            path.join(entry)
        };
        if !backend.exists(&target) {
            continue;
        }
        backend.fchown(&target, uid, gid)?;
    }
    Ok(())
}

/// Step end-to-end (broker entry): create `nixling.slice` under the
/// unified hierarchy with the canonical enable sequence + chown.
///
/// The ordering is:
///
/// 1. probe + require controllers at root;
/// 2. mkdir `nixling.slice` (so the child exists for the verification
///    re-read);
/// 3. enable controllers on root, verifying both the root's
///    `cgroup.subtree_control` re-read AND `nixling.slice/cgroup.controllers`
///    after each `+<controller>` write — anything else is
///    [`CgroupError::CgroupControllerNotExposedToChild`];
/// 4. enable controllers on the slice itself (no child yet — the
///    delegated subtree is empty);
/// 5. assert `nixling.slice/cgroup.procs` is empty before chown —
///    delegating a slice that already holds processes is a leak.
pub fn create_nixling_slice<B: CgroupBackend>(
    backend: &B,
    root: &Path,
    nixlingd_uid: u32,
    nixlingd_gid: u32,
) -> Result<PathBuf, CgroupError> {
    probe_unified_hierarchy(backend, root)?;
    require_controllers(backend, root, Controller::REQUIRED)?;

    let slice = root.join(NIXLING_SLICE_NAME);
    if !backend.exists(&slice) {
        backend.mkdir(&slice)?;
    }
    assert_not_threaded(backend, &slice)?;
    // Ancestors and `nixling.slice` itself stay `partition=member`.
    assert_partition_member_only(&slice, "cpuset.cpus")?;

    enable_subtree_controllers_with_child(
        backend,
        root,
        Some(slice.as_path()),
        Controller::ENABLE_ORDER,
    )?;
    enable_subtree_controllers(backend, &slice, Controller::ENABLE_ORDER)?;

    // Plan step 5: nixling.slice itself must be process-free before we
    // chown it. A delegation that hands a slice with live processes to
    // the dropped-privilege nixlingd uid would leak control of those
    // PIDs to the delegated user.
    assert_no_internal_processes(backend, &slice)?;

    chown_subtree_to_nixlingd(backend, &slice, nixlingd_uid, nixlingd_gid)?;
    Ok(slice)
}

/// v1.1.1 per-VM-interior + per-role-leaf taxonomy. Creates the
/// process-free intermediate directory `nixling.slice/<vm_id>/`
/// (NOT a leaf). Per-role leaf cgroups are created by
/// `create_vm_role_leaf`. Per ADR 0011 Decision item 1.
pub fn create_vm_subtree<B: CgroupBackend>(
    backend: &B,
    slice: &Path,
    vm_id: &str,
    nixlingd_uid: u32,
    nixlingd_gid: u32,
) -> Result<PathBuf, CgroupError> {
    if vm_id.is_empty() || vm_id.contains('/') || vm_id.contains('\0') {
        return Err(CgroupError::Io {
            detail: format!("invalid vm_id: {vm_id:?}"),
        });
    }
    // v1.1.1: per-VM intermediate is `<slice>/<vm_id>/`, NOT
    // `<slice>/<vm_id>.scope`. The intermediate stays
    // process-free; per-role leaves under it hold the processes.
    let vm = slice.join(vm_id);
    if !backend.exists(&vm) {
        backend.mkdir(&vm)?;
    }
    assert_not_threaded(backend, &vm)?;
    enable_subtree_controllers(backend, &vm, Controller::ENABLE_ORDER)?;
    chown_subtree_to_nixlingd(backend, &vm, nixlingd_uid, nixlingd_gid)?;
    assert_no_internal_processes(backend, &vm)?;
    Ok(vm)
}

/// v1.1.1 per-role leaf cgroup creation. Creates
/// `<slice>/<vm_id>/<role_id>/` under the previously-created
/// per-VM intermediate. The leaf is the ONLY entry that carries
/// processes; ancestors stay process-free.
pub fn create_vm_role_leaf<B: CgroupBackend>(
    backend: &B,
    slice: &Path,
    vm_id: &str,
    role_id: &str,
    nixlingd_uid: u32,
    nixlingd_gid: u32,
) -> Result<PathBuf, CgroupError> {
    if role_id.is_empty() || role_id.contains('/') || role_id.contains('\0') {
        return Err(CgroupError::Io {
            detail: format!("invalid role_id: {role_id:?}"),
        });
    }
    let vm_dir = create_vm_subtree(backend, slice, vm_id, nixlingd_uid, nixlingd_gid)?;
    let leaf = vm_dir.join(role_id);
    if !backend.exists(&leaf) {
        backend.mkdir(&leaf)?;
    }
    assert_not_threaded(backend, &leaf)?;
    // Per-role leaves DON'T enable subtree_controllers further
    // (they're the leaf — no descendants need controllers
    // enabled at this layer); chown so nixlingd can write
    // cgroup.procs / cgroup.kill.
    chown_subtree_to_nixlingd(backend, &leaf, nixlingd_uid, nixlingd_gid)?;
    Ok(leaf)
}

// ---------------------------------------------------------------------------
// Real backend: `rustix`-driven, fd-based, path-safe.
// ---------------------------------------------------------------------------

/// Production [`CgroupBackend`] driving real cgroupfs via `rustix`.
///
/// Reads/writes funnel through `rustix::fs::openat` with `O_NOFOLLOW`
/// so symlink swaps on the cgroup root are refused; writes do
/// `O_WRONLY | O_NOFOLLOW`, never `open(path)`. `fchown` is performed
/// fd-based against an open `O_PATH | O_NOFOLLOW` descriptor per the
/// filesystem path-safety tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealCgroupBackend;

impl RealCgroupBackend {
    pub fn new() -> Self {
        Self
    }
}

impl CgroupBackend for RealCgroupBackend {
    fn current_uid(&self) -> u32 {
        rustix::process::getuid().as_raw()
    }

    fn read_file(&self, path: &Path) -> Result<String, CgroupError> {
        use rustix::fs::{open, Mode, OFlags};
        let fd = open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io_err)?;
        let mut out = Vec::with_capacity(256);
        let mut buf = [0u8; 4096];
        loop {
            let n = rustix::io::read(&fd, &mut buf).map_err(io_err)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(out).map_err(|err| CgroupError::Io {
            detail: format!("non-utf8 cgroup file {}: {err}", path.display()),
        })
    }

    fn write_file(&self, path: &Path, contents: &str) -> Result<(), CgroupError> {
        use rustix::fs::{open, Mode, OFlags};
        let fd = open(
            path,
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io_err)?;
        let mut written = 0;
        let buf = contents.as_bytes();
        while written < buf.len() {
            let n = rustix::io::write(&fd, &buf[written..]).map_err(io_err)?;
            if n == 0 {
                return Err(CgroupError::Io {
                    detail: format!("short write on {}", path.display()),
                });
            }
            written += n;
        }
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        rustix::fs::access(path, rustix::fs::Access::EXISTS).is_ok()
    }

    fn mkdir(&self, path: &Path) -> Result<(), CgroupError> {
        rustix::fs::mkdir(path, rustix::fs::Mode::from_raw_mode(0o755)).map_err(io_err)
    }

    fn fchown(&self, path: &Path, uid: u32, gid: u32) -> Result<(), CgroupError> {
        use rustix::fs::{open, Mode, OFlags};
        // v1.1.1 kernel-correctness fix: O_PATH descriptors can NOT
        // be passed to fchown(2) — the syscall returns EBADF on
        // Linux per `man 2 fchown` because O_PATH fds have no
        // associated open file description. The correct primitive
        // for changing ownership via an O_PATH dirfd is
        // fchownat(dirfd, "", uid, gid, AT_EMPTY_PATH) which the
        // kernel resolves through the dirfd itself.
        //
        // Documented in
        // [`docs/reference/cgroup-delegation.md`](../../../docs/reference/cgroup-delegation.md)
        // § Path-safety contract; the v1.1.1 fix matches the ADR commitment in
        // [ADR 0018 § "Path-safety: O_PATH + fchownat(AT_EMPTY_PATH)"](../../../docs/adr/0018-microvm-nix-removal.md#path-safety-o_path--fchownataT_EMPTY_PATH).
        let fd = open(
            path,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io_err)?;
        // nix's safe Uid/Gid/fchownat wrappers (rustix 0.38's Uid/Gid
        // constructors are `unsafe`, which the crate-level
        // `#![forbid(unsafe_code)]` would block).
        let raw_uid = nix::unistd::Uid::from_raw(uid);
        let raw_gid = nix::unistd::Gid::from_raw(gid);
        nix::unistd::fchownat(
            Some(fd.as_raw_fd()),
            "",
            Some(raw_uid),
            Some(raw_gid),
            nix::fcntl::AtFlags::AT_EMPTY_PATH | nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
        )
        .map_err(|err| CgroupError::Io {
            detail: format!("fchownat(AT_EMPTY_PATH) {}: {err}", path.display()),
        })
    }

    fn read_procs(&self, dir: &Path) -> Result<Vec<u32>, CgroupError> {
        let raw = self.read_file(&dir.join("cgroup.procs"))?;
        let pids: Result<Vec<u32>, _> = raw
            .split_ascii_whitespace()
            .map(|s| s.parse::<u32>())
            .collect();
        pids.map_err(|err| CgroupError::Io {
            detail: format!("parse cgroup.procs at {}: {err}", dir.display()),
        })
    }
}

fn io_err(err: rustix::io::Errno) -> CgroupError {
    CgroupError::Io {
        detail: err.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Fake backend (in-memory model) for L1c canary tests.
// ---------------------------------------------------------------------------

/// In-memory [`CgroupBackend`] used by the L1c canary tests.
///
/// Models a cgroupfs as a `BTreeMap<PathBuf, Vec<u8>>` plus a directory
/// set and a per-cgroup process list. Every algorithm step is exercised
/// without touching a real cgroupfs.
#[cfg(any(test, feature = "fake-backends"))]
pub mod fake {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    pub struct FakeCgroupBackend {
        inner: Mutex<Inner>,
        uid: u32,
    }

    #[derive(Debug, Default)]
    struct Inner {
        files: BTreeMap<PathBuf, String>,
        dirs: BTreeSet<PathBuf>,
        owners: BTreeMap<PathBuf, (u32, u32)>,
        kill_log: Vec<PathBuf>,
    }

    impl FakeCgroupBackend {
        pub fn new(uid: u32) -> Self {
            Self {
                uid,
                ..Default::default()
            }
        }

        /// Seed a stock unified hierarchy with the canonical controller
        /// set advertised at `root`.
        pub fn seed_unified(&self, root: &Path) {
            let mut inner = self.inner.lock().unwrap();
            inner.dirs.insert(root.to_path_buf());
            inner.files.insert(
                root.join("cgroup.controllers"),
                "cpu memory io pids cpuset".to_owned(),
            );
            inner
                .files
                .insert(root.join("cgroup.subtree_control"), String::new());
            inner
                .files
                .insert(root.join("cpuset.cpus.effective"), "0-3".to_owned());
            inner
                .files
                .insert(root.join("cpuset.mems.effective"), "0".to_owned());
            inner
                .files
                .insert(root.join("cpuset.cpus"), "0-3".to_owned());
            inner.files.insert(root.join("cpuset.mems"), "0".to_owned());
            inner.files.insert(root.join("cgroup.procs"), String::new());
        }

        /// Seed only a partial controller set for the missing-controllers
        /// canary.
        pub fn seed_unified_with_controllers(&self, root: &Path, controllers: &str) {
            let mut inner = self.inner.lock().unwrap();
            inner.dirs.insert(root.to_path_buf());
            inner
                .files
                .insert(root.join("cgroup.controllers"), controllers.to_owned());
            inner
                .files
                .insert(root.join("cgroup.subtree_control"), String::new());
            inner
                .files
                .insert(root.join("cpuset.cpus.effective"), "0-3".to_owned());
            inner
                .files
                .insert(root.join("cpuset.mems.effective"), "0".to_owned());
            inner
                .files
                .insert(root.join("cpuset.cpus"), "0-3".to_owned());
            inner.files.insert(root.join("cpuset.mems"), "0".to_owned());
            inner.files.insert(root.join("cgroup.procs"), String::new());
        }

        /// Inject a running pid into a cgroup (used to exercise the
        /// no-internal-process gate).
        pub fn inject_procs(&self, dir: &Path, pids: &[u32]) {
            let mut inner = self.inner.lock().unwrap();
            let body = pids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            inner.files.insert(dir.join("cgroup.procs"), body);
        }

        /// Snapshot of the leaf kill events recorded by
        /// [`super::cgroup_kill_leaf_only`].
        pub fn kill_log(&self) -> Vec<PathBuf> {
            self.inner.lock().unwrap().kill_log.clone()
        }

        /// Snapshot of `(uid, gid)` owners for a path; `None` if the
        /// path has not been chowned.
        pub fn owner(&self, path: &Path) -> Option<(u32, u32)> {
            self.inner.lock().unwrap().owners.get(path).copied()
        }

        pub fn directory_exists(&self, path: &Path) -> bool {
            self.inner.lock().unwrap().dirs.contains(path)
        }

        pub fn file_contents(&self, path: &Path) -> Option<String> {
            self.inner.lock().unwrap().files.get(path).cloned()
        }
    }

    impl CgroupBackend for FakeCgroupBackend {
        fn current_uid(&self) -> u32 {
            self.uid
        }

        fn read_file(&self, path: &Path) -> Result<String, CgroupError> {
            let inner = self.inner.lock().unwrap();
            inner
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| CgroupError::Io {
                    detail: format!("ENOENT {}", path.display()),
                })
        }

        fn write_file(&self, path: &Path, contents: &str) -> Result<(), CgroupError> {
            super::assert_partition_member_only(
                path.parent().unwrap_or(path),
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default(),
            )?;
            let mut inner = self.inner.lock().unwrap();
            // `cgroup.kill` is intercepted separately so the kill scope
            // can be audited from tests.
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n == "cgroup.kill")
            {
                inner.kill_log.push(
                    path.parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| path.to_path_buf()),
                );
                inner.files.insert(path.to_path_buf(), contents.to_owned());
                return Ok(());
            }
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n == "cgroup.subtree_control")
            {
                // Real cgroupfs only takes one +ctrl token per write and
                // appends to the existing enabled set; model the same.
                let token = contents.trim();
                let existing = inner.files.entry(path.to_path_buf()).or_default();
                let mut set: BTreeSet<String> = existing
                    .split_ascii_whitespace()
                    .map(|s| s.to_owned())
                    .collect();
                if let Some(rest) = token.strip_prefix('+') {
                    set.insert(rest.to_owned());
                } else if let Some(rest) = token.strip_prefix('-') {
                    set.remove(rest);
                }
                *existing = set.into_iter().collect::<Vec<_>>().join(" ");
                return Ok(());
            }
            inner.files.insert(path.to_path_buf(), contents.to_owned());
            Ok(())
        }

        fn exists(&self, path: &Path) -> bool {
            let inner = self.inner.lock().unwrap();
            inner.files.contains_key(path) || inner.dirs.contains(path)
        }

        fn mkdir(&self, path: &Path) -> Result<(), CgroupError> {
            let mut inner = self.inner.lock().unwrap();
            inner.dirs.insert(path.to_path_buf());
            // Materialize the cgroup core files that always exist in
            // cgroup v2 for a freshly-created directory.
            inner
                .files
                .entry(path.join("cgroup.controllers"))
                .or_insert_with(|| "cpu memory io pids cpuset".to_owned());
            inner
                .files
                .entry(path.join("cgroup.subtree_control"))
                .or_default();
            inner.files.entry(path.join("cgroup.procs")).or_default();
            inner
                .files
                .entry(path.join("cpuset.cpus.effective"))
                .or_insert_with(|| "0-3".to_owned());
            inner
                .files
                .entry(path.join("cpuset.mems.effective"))
                .or_insert_with(|| "0".to_owned());
            inner
                .files
                .entry(path.join("cpuset.cpus"))
                .or_insert_with(|| "0-3".to_owned());
            inner
                .files
                .entry(path.join("cpuset.mems"))
                .or_insert_with(|| "0".to_owned());
            Ok(())
        }

        fn fchown(&self, path: &Path, uid: u32, gid: u32) -> Result<(), CgroupError> {
            self.inner
                .lock()
                .unwrap()
                .owners
                .insert(path.to_path_buf(), (uid, gid));
            Ok(())
        }

        fn read_procs(&self, dir: &Path) -> Result<Vec<u32>, CgroupError> {
            let raw = self.read_file(&dir.join("cgroup.procs"))?;
            raw.split_ascii_whitespace()
                .map(|s| {
                    s.parse::<u32>().map_err(|err| CgroupError::Io {
                        detail: format!("parse cgroup.procs: {err}"),
                    })
                })
                .collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::fake::FakeCgroupBackend;
    use super::*;

    const FAKE_ROOT: &str = "/sys/fs/cgroup";

    fn nixlingd_uid() -> u32 {
        1234
    }
    fn nixlingd_gid() -> u32 {
        1234
    }

    fn fresh(uid: u32) -> FakeCgroupBackend {
        let b = FakeCgroupBackend::new(uid);
        b.seed_unified(Path::new(FAKE_ROOT));
        b
    }

    #[test]
    fn happy_path_delegation() {
        let backend = fresh(nixlingd_uid());
        require_non_root_delegation(&backend).unwrap();
        let slice = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .expect("delegate ok");
        assert!(backend.directory_exists(&slice));
        let owner = backend.owner(&slice).expect("chowned");
        assert_eq!(owner, (nixlingd_uid(), nixlingd_gid()));
        // Subtree control on the slice covers every required controller.
        let st = backend
            .file_contents(&slice.join("cgroup.subtree_control"))
            .unwrap_or_default();
        for c in Controller::REQUIRED {
            assert!(st.contains(c.as_str()), "{c} should be enabled");
        }
    }

    #[test]
    fn refuses_uid_zero() {
        let backend = fresh(0);
        match require_non_root_delegation(&backend) {
            Err(CgroupError::CgroupDelegationRefused { .. }) => {}
            other => panic!("expected delegation refusal, got {other:?}"),
        }
    }

    #[test]
    fn refuses_when_unified_hierarchy_missing() {
        let backend = FakeCgroupBackend::new(nixlingd_uid());
        let err = probe_unified_hierarchy(&backend, Path::new(FAKE_ROOT)).unwrap_err();
        assert_eq!(err.code(), "cgroup-v2-unified-not-present");
    }

    #[test]
    fn refuses_when_controllers_missing() {
        let backend = FakeCgroupBackend::new(nixlingd_uid());
        backend.seed_unified_with_controllers(Path::new(FAKE_ROOT), "cpu memory");
        let err =
            require_controllers(&backend, Path::new(FAKE_ROOT), Controller::REQUIRED).unwrap_err();
        assert_eq!(err.code(), "cgroup-controllers-missing");
    }

    #[test]
    fn refuses_kill_on_ancestor() {
        let backend = fresh(nixlingd_uid());
        let slice = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap();
        let leaf = slice.join("vm-a.scope");
        backend.mkdir(&leaf).unwrap();
        let err = cgroup_kill_leaf_only(&backend, &slice, std::slice::from_ref(&leaf)).unwrap_err();
        assert_eq!(err.code(), "cgroup-kill-on-ancestor-refused");
        // Killing the leaf itself succeeds and is logged.
        cgroup_kill_leaf_only(&backend, &leaf, std::slice::from_ref(&leaf)).unwrap();
        assert_eq!(backend.kill_log(), vec![leaf]);
    }

    #[test]
    fn detects_internal_processes() {
        let backend = fresh(nixlingd_uid());
        let slice = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap();
        backend.inject_procs(&slice, &[4242]);
        let err = assert_no_internal_processes(&backend, &slice).unwrap_err();
        assert_eq!(err.code(), "cgroup-internal-processes-present");
    }

    #[test]
    fn refuses_threaded_cgroups() {
        let backend = fresh(nixlingd_uid());
        let slice = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap();
        backend
            .write_file(&slice.join("cgroup.type"), "threaded")
            .unwrap();
        let err = assert_not_threaded(&backend, &slice).unwrap_err();
        assert_eq!(err.code(), "cgroup-threaded-forbidden");
    }

    #[test]
    fn rejects_partition_root_writes() {
        let path = Path::new("/sys/fs/cgroup/nixling.slice");
        // In debug builds the `debug_assert!` is the load-bearing
        // guard; in release builds the function returns the error
        // variant. Exercise both via `catch_unwind`.
        let result = std::panic::catch_unwind(|| {
            assert_partition_member_only(path, "cpuset.cpus.partition")
        });
        match result {
            Err(_) => { /* debug build: debug_assert! fired */ }
            Ok(Err(err)) => assert_eq!(err.code(), "cgroup-partition-root-forbidden"),
            Ok(Ok(())) => panic!("partition root write was not rejected"),
        }
    }

    #[test]
    fn cpuset_inheritance_fills_empty_files() {
        let backend = fresh(nixlingd_uid());
        let path = Path::new(FAKE_ROOT);
        backend.write_file(&path.join("cpuset.cpus"), "").unwrap();
        backend.write_file(&path.join("cpuset.mems"), "").unwrap();
        prepare_cpuset_inheritance(&backend, path).unwrap();
        assert_eq!(
            backend.file_contents(&path.join("cpuset.cpus")).unwrap(),
            "0-3"
        );
    }

    #[test]
    fn vm_subtree_creates_chowned_per_vm_interior() {
        // v1.1.1: vm_subtree now creates `<slice>/<vm>/` (interior;
        // process-free), not `<slice>/<vm>.scope` (leaf).
        let backend = fresh(nixlingd_uid());
        let slice = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap();
        let vm =
            create_vm_subtree(&backend, &slice, "alpha", nixlingd_uid(), nixlingd_gid()).unwrap();
        assert_eq!(vm, slice.join("alpha"));
        assert_eq!(
            backend.owner(&vm).unwrap(),
            (nixlingd_uid(), nixlingd_gid())
        );
    }

    #[test]
    fn vm_role_leaf_creates_chowned_per_role_leaf_under_vm_interior() {
        // v1.1.1: per-role leaf `<slice>/<vm>/<role>/` is the
        // canonical placement target for SpawnRunner processes.
        let backend = fresh(nixlingd_uid());
        let slice = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap();
        let leaf = create_vm_role_leaf(
            &backend,
            &slice,
            "alpha",
            "cloud-hypervisor",
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap();
        assert_eq!(leaf, slice.join("alpha").join("cloud-hypervisor"));
        assert_eq!(
            backend.owner(&leaf).unwrap(),
            (nixlingd_uid(), nixlingd_gid())
        );
    }

    #[test]
    fn enabled_controllers_missing() {
        let e = EnabledControllers::from_controllers([Controller::Cpu, Controller::Memory]);
        assert_eq!(
            e.missing(Controller::REQUIRED),
            vec![Controller::Io, Controller::Pids, Controller::Cpuset]
        );
    }

    #[test]
    fn delegation_refused_when_slice_already_holds_processes() {
        // Pre-seed `nixling.slice` with a pid so the no-internal-process
        // check inside `create_nixling_slice` fires before chown.
        let backend = fresh(nixlingd_uid());
        let slice_path = Path::new(FAKE_ROOT).join(NIXLING_SLICE_NAME);
        backend.mkdir(&slice_path).unwrap();
        backend.inject_procs(&slice_path, &[9090]);
        let err = create_nixling_slice(
            &backend,
            Path::new(FAKE_ROOT),
            nixlingd_uid(),
            nixlingd_gid(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "cgroup-internal-processes-present");
        // Chown must not have run — owner should be unset.
        assert!(backend.owner(&slice_path).is_none());
    }

    #[test]
    fn child_controllers_verified_after_subtree_enable() {
        // Drive the new verifying variant directly to assert the
        // child-controllers re-read is load-bearing: if the fake
        // backend's child `cgroup.controllers` is *blank* the call
        // fail-closes with `cgroup-controller-not-exposed-to-child`.
        let backend = fresh(nixlingd_uid());
        let root = Path::new(FAKE_ROOT);
        let child = root.join("standalone-child");
        backend.mkdir(&child).unwrap();
        // Drop the controllers advertised on the child so the verify
        // step misses every controller.
        backend
            .write_file(&child.join("cgroup.controllers"), "")
            .unwrap();
        let err = enable_subtree_controllers_with_child(
            &backend,
            root,
            Some(child.as_path()),
            &[Controller::Cpu],
        )
        .unwrap_err();
        assert_eq!(err.code(), "cgroup-controller-not-exposed-to-child");
    }
}
