//! Broker-side cgroup operation handlers (`DelegateCgroupV2`,
//! `OpenCgroupDir`).
//!
//! Implements the broker contract for cgroup v2 delegation:
//!
//! - paths are re-derived from the trusted bundle (`BundlePaths`),
//!   never from caller input; the wire request only names the
//!   subject (`subtree`, `vm_id`) and the broker maps that name to
//!   the canonical delegated subtree (default
//!   `/sys/fs/cgroup/d2b.slice`) plus per-VM interiors and
//!   per-role leaves beneath it;
//! - the 8-step delegation algorithm runs through
//!   [`d2b_host::cgroup`];
//! - `OpenCgroupDir` returns an `O_PATH | O_NOFOLLOW` fd opened with
//!   the path-safety contract (`openat2` + `RESOLVE_BENEATH` on real
//!   builds; the fake handler returns a placeholder fd identifier so
//!   L1c tests can observe the call without performing real cgroupfs
//!   I/O).
//!
//! See `docs/reference/cgroup-delegation.md` for the audit record
//! schema and the per-variant fields populated below.

use std::fmt;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use d2b_contracts::broker_wire::OpenCgroupDirRequest;
use d2b_contracts::types::{PathClass as BrokerPathClass, ScopeId};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_host::cgroup::{
    self as host_cgroup, CgroupBackend, CgroupError, Controller, D2B_SLICE_NAME,
};

use crate::ops::exec_reconcile::SystemLiveExec;

use super::AuditDecision;

pub(crate) const DEFAULT_DELEGATED_PARENT_SLICE: &str = "/sys/fs/cgroup/d2b.slice";

/// Sub-error for [`super::OpError::Cgroup`]. Stays kebab-case to match
/// the audit `error_kind` field.
#[derive(Debug)]
pub enum CgroupOpError {
    /// The host-side cgroup algorithm returned an error.
    Host(CgroupError),
    /// The operator has not pre-created / delegated the parent slice
    /// that the broker is allowed to manage.
    CgroupNotDelegated { expected_parent: PathBuf },
    /// `OpenCgroupDir` was asked about a path that does not resolve
    /// under the delegated d2b.slice (`path-class = foreign`).
    PathClassForeign { requested: PathBuf },
    /// `cgroup.kill` was attempted on a non-leaf path.
    KillAncestor { requested: PathBuf },
}

impl fmt::Display for CgroupOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CgroupOpError::Host(err) => write!(f, "{}: {err}", err.code()),
            CgroupOpError::CgroupNotDelegated { expected_parent } => write!(
                f,
                "cgroup-not-delegated: expected systemd-delegated parent slice at {}",
                expected_parent.display()
            ),
            CgroupOpError::PathClassForeign { requested } => write!(
                f,
                "OpenCgroupDir: requested path outside d2b.slice: {}",
                requested.display()
            ),
            CgroupOpError::KillAncestor { requested } => write!(
                f,
                "cgroup.kill refused on non-leaf path {}",
                requested.display()
            ),
        }
    }
}

impl CgroupOpError {
    fn code(&self) -> &'static str {
        match self {
            CgroupOpError::Host(err) => err.code(),
            CgroupOpError::CgroupNotDelegated { .. } => "cgroup-not-delegated",
            CgroupOpError::PathClassForeign { .. } => "path-class-foreign",
            CgroupOpError::KillAncestor { .. } => "cgroup-kill-on-ancestor-refused",
        }
    }
}

impl std::error::Error for CgroupOpError {}

impl From<CgroupError> for CgroupOpError {
    fn from(err: CgroupError) -> Self {
        CgroupOpError::Host(err)
    }
}

impl From<CgroupError> for super::OpError {
    fn from(err: CgroupError) -> Self {
        super::OpError::Cgroup(CgroupOpError::Host(err))
    }
}

impl From<CgroupOpError> for super::OpError {
    fn from(err: CgroupOpError) -> Self {
        super::OpError::Cgroup(err)
    }
}

/// Paths extracted from the trusted bundle that the cgroup handlers
/// resolve to. Constructed by the integrator-managed bundle loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupBundleContext {
    pub unified_hierarchy_root: PathBuf,
    /// Systemd-managed delegated parent slice. The broker only writes
    /// inside this subtree; it never touches `/sys/fs/cgroup` root.
    pub parent_slice: PathBuf,
    pub d2bd_uid: u32,
    pub d2bd_gid: u32,
    /// VM ids advertised by the trusted bundle. Any `OpenCgroupDir`
    /// request whose `vm_id` is not in this set is refused with
    /// `path-class = unknown-subject`.
    pub known_vms: Vec<String>,
}

impl CgroupBundleContext {
    pub fn slice_path(&self) -> PathBuf {
        self.parent_slice.clone()
    }

    /// v1.1.1 per-VM-interior + per-role-leaf taxonomy per ADR 0011
    /// Decision item 1: `vm_interior_path` returns the
    /// process-free intermediate directory `d2b.slice/<vm_id>/`.
    /// Per-role leaf cgroups (`d2b.slice/<vm_id>/<role>/`) are
    /// the only entries that carry processes.
    pub fn vm_interior_path(&self, vm_id: &str) -> PathBuf {
        self.slice_path().join(vm_id)
    }

    /// v1.1.1 per-role leaf cgroup path
    /// `d2b.slice/<vm_id>/<role_id>/`. Processes for the
    /// `(vm_id, role_id)` SpawnRunner instance are placed here via
    /// `clone3(CLONE_INTO_CGROUP)` at spawn time.
    pub fn vm_role_leaf_path(&self, vm_id: &str, role_id: &str) -> PathBuf {
        self.vm_interior_path(vm_id).join(role_id)
    }

    /// v1.0 backward-compat alias: returns the per-VM INTERIOR
    /// `<slice>/<vm_id>/` (NOT the legacy `<vm_id>.scope` leaf).
    /// The interior path is the one create_vm_subtree actually
    /// materializes in v1.1+; read-side callers
    /// (`handle_open_cgroup_dir`) work transparently against the
    /// interior. Write-side callers should migrate to
    /// `vm_role_leaf_path(vm_id, role_id)` for per-role-leaf
    /// granularity.
    #[deprecated(
        since = "1.1.1",
        note = "v1.1.1 migrated to per-VM-interior + per-role-leaf; use vm_interior_path or vm_role_leaf_path"
    )]
    pub fn vm_leaf_path(&self, vm_id: &str) -> PathBuf {
        self.vm_interior_path(vm_id)
    }

    pub fn knows_vm(&self, vm_id: &str) -> bool {
        self.known_vms.iter().any(|known| known == vm_id)
    }
}

/// Outcome of a `DelegateCgroupV2` call: includes the slice path,
/// owner uid, and the controllers enabled by this delegation pass.
#[derive(Debug, Clone)]
pub struct DelegateCgroupV2Outcome {
    pub slice_path: PathBuf,
    pub owner_uid: u32,
    pub controllers_enabled: Vec<Controller>,
}

/// Outcome of an `OpenCgroupDir` call.
#[derive(Debug, Clone)]
pub struct OpenCgroupDirOutcome {
    pub cgroup_path: PathBuf,
    pub cgroup_id: String,
    pub path_class: PathClass,
}

/// Audit-recording trait. The integrator wires this up to the live
/// [`crate::audit::AuditLog`]; the fake harness for L1c tests records
/// to an in-memory `Vec<AuditFields>`.
pub trait AuditSink {
    fn record(
        &self,
        operation: &'static str,
        decision: AuditDecision,
        fields: &AuditFields,
        error_kind: Option<&str>,
    );
}

/// Per-variant audit fields for `DelegateCgroupV2`/`OpenCgroupDir` rows.
#[derive(Debug, Clone, Default)]
pub struct AuditFields {
    pub slice_path: Option<PathBuf>,
    pub controllers_enabled: Vec<Controller>,
    pub owner_uid: Option<u32>,
    pub cgroup_id: Option<String>,
    pub path_class: Option<PathClass>,
}

/// `path_class` discriminant per plan.md `OpenCgroupDir` audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    D2bSlice,
    VmLeaf,
    Foreign,
    Unknown,
}

impl PathClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            PathClass::D2bSlice => "d2b-slice",
            PathClass::VmLeaf => "vm-leaf",
            PathClass::Foreign => "foreign",
            PathClass::Unknown => "unknown-subject",
        }
    }
}

/// `DelegateCgroupV2`: runs the cgroup delegation algorithm against a
/// systemd-managed parent slice that has already been delegated to the
/// broker/daemon. The broker never writes `/sys/fs/cgroup` root; the
/// operator must pre-create + `Delegate=yes` the parent slice (default
/// `/sys/fs/cgroup/d2b.slice`) and then the broker
/// enables controllers / chowns only within that subtree. Per the
/// broker variant table, `destructive: no`, `secret: no`, audit
/// decision `allowed` on success.
pub fn handle_delegate_cgroup_v2<B, A>(
    backend: &B,
    context: &CgroupBundleContext,
    audit: &A,
) -> Result<DelegateCgroupV2Outcome, super::OpError>
where
    B: CgroupBackend,
    A: AuditSink,
{
    let root = context.unified_hierarchy_root.as_path();

    let mut fields = AuditFields {
        slice_path: Some(context.slice_path()),
        controllers_enabled: Vec::new(),
        owner_uid: Some(context.d2bd_uid),
        ..AuditFields::default()
    };

    if let Err(err) = host_cgroup::require_non_root_delegation(backend) {
        audit.record(
            "DelegateCgroupV2",
            AuditDecision::DeniedRefused,
            &fields,
            Some(err.code()),
        );
        return Err(super::OpError::Cgroup(CgroupOpError::Host(err)));
    }

    let slice = create_d2b_slice(
        backend,
        root,
        context.parent_slice.as_path(),
        context.d2bd_uid,
        context.d2bd_gid,
    )
    .map_err(|err| {
        audit.record(
            "DelegateCgroupV2",
            classify_decision(&err),
            &fields,
            Some(err.code()),
        );
        super::OpError::Cgroup(err)
    })?;

    fields.slice_path = Some(slice.clone());
    fields.controllers_enabled = Controller::ENABLE_ORDER.to_vec();

    audit.record("DelegateCgroupV2", AuditDecision::Allowed, &fields, None);

    Ok(DelegateCgroupV2Outcome {
        slice_path: slice,
        owner_uid: context.d2bd_uid,
        controllers_enabled: Controller::ENABLE_ORDER.to_vec(),
    })
}

/// `OpenCgroupDir`: returns the resolved (bundle-trusted) cgroup path
/// for a VM/role. The wire request only carries a subject name; the
/// broker re-derives the path from `CgroupBundleContext` and refuses
/// any path that escapes the delegated slice.
///
/// The actual fd-open + `O_PATH | O_NOFOLLOW` happens in the runtime
/// dispatch; this handler returns the canonical path plus a stable
/// `cgroup_id` derived from the unified subject name so the audit
/// record matches the plan-named field.
pub fn handle_open_cgroup_dir<B, A>(
    backend: &B,
    context: &CgroupBundleContext,
    requested_subject: &str,
    audit: &A,
) -> Result<OpenCgroupDirOutcome, super::OpError>
where
    B: CgroupBackend,
    A: AuditSink,
{
    let mut fields = AuditFields {
        cgroup_id: Some(requested_subject.to_owned()),
        ..AuditFields::default()
    };

    // Subject classification: the wire request carries a logical
    // subject name (e.g. "d2b-slice" or a vm id). The broker
    // maps that to a canonical path under d2b.slice.
    let (canonical_path, class) =
        if requested_subject == D2B_SLICE_NAME || requested_subject == "d2b-slice" {
            (context.slice_path(), PathClass::D2bSlice)
        } else if context.knows_vm(requested_subject) {
            (context.vm_leaf_path(requested_subject), PathClass::VmLeaf)
        } else {
            fields.path_class = Some(PathClass::Unknown);
            audit.record(
                "OpenCgroupDir",
                AuditDecision::DeniedUnknown,
                &fields,
                Some("unknown-subject"),
            );
            return Err(super::OpError::UnknownSubject {
                operation: "OpenCgroupDir",
                subject: requested_subject.to_owned(),
            });
        };

    fields.path_class = Some(class);
    fields.cgroup_id = Some(canonical_path.display().to_string());

    if !is_under_slice(&canonical_path, &context.slice_path()) {
        audit.record(
            "OpenCgroupDir",
            AuditDecision::DeniedRefused,
            &fields,
            Some("path-class-foreign"),
        );
        return Err(super::OpError::Cgroup(CgroupOpError::PathClassForeign {
            requested: canonical_path,
        }));
    }

    if !backend.exists(&canonical_path) {
        let kind = "ENOENT";
        audit.record("OpenCgroupDir", AuditDecision::Errored, &fields, Some(kind));
        return Err(super::OpError::Cgroup(CgroupOpError::Host(
            CgroupError::Io {
                detail: format!("{} missing", canonical_path.display()),
            },
        )));
    }

    audit.record("OpenCgroupDir", AuditDecision::Allowed, &fields, None);
    Ok(OpenCgroupDirOutcome {
        cgroup_path: canonical_path.clone(),
        cgroup_id: canonical_path.display().to_string(),
        path_class: class,
    })
}

/// Cgroup-kill handler: refuses ancestor kills with `cgroup-kill-on-
/// ancestor-refused`. Only callable from teardown paths in the runtime.
pub fn handle_cgroup_kill<B, A>(
    backend: &B,
    context: &CgroupBundleContext,
    requested_subject: &str,
    audit: &A,
) -> Result<(), super::OpError>
where
    B: CgroupBackend,
    A: AuditSink,
{
    let mut fields = AuditFields {
        cgroup_id: Some(requested_subject.to_owned()),
        ..AuditFields::default()
    };

    if !context.knows_vm(requested_subject) {
        fields.path_class = Some(PathClass::Unknown);
        audit.record(
            "CgroupKill",
            AuditDecision::DeniedUnknown,
            &fields,
            Some("unknown-subject"),
        );
        return Err(super::OpError::UnknownSubject {
            operation: "CgroupKill",
            subject: requested_subject.to_owned(),
        });
    }

    let leaf = context.vm_leaf_path(requested_subject);
    fields.path_class = Some(PathClass::VmLeaf);
    fields.cgroup_id = Some(leaf.display().to_string());

    host_cgroup::cgroup_kill_leaf_only(backend, &leaf, std::slice::from_ref(&leaf)).map_err(
        |err| {
            audit.record(
                "CgroupKill",
                AuditDecision::DeniedRefused,
                &fields,
                Some(err.code()),
            );
            super::OpError::Cgroup(CgroupOpError::Host(err))
        },
    )?;
    audit.record("CgroupKill", AuditDecision::Allowed, &fields, None);
    Ok(())
}

pub(crate) fn create_d2b_slice<B: CgroupBackend>(
    backend: &B,
    unified_hierarchy_root: &Path,
    parent_slice: &Path,
    d2bd_uid: u32,
    d2bd_gid: u32,
) -> Result<PathBuf, CgroupOpError> {
    host_cgroup::probe_unified_hierarchy(backend, unified_hierarchy_root)?;
    // Systemd must pre-create + delegate this slice. The broker only
    // enables controllers and changes ownership within that subtree;
    // it never writes `/sys/fs/cgroup/cgroup.subtree_control`.
    if !backend.exists(parent_slice) {
        return Err(CgroupOpError::CgroupNotDelegated {
            expected_parent: parent_slice.to_path_buf(),
        });
    }
    host_cgroup::assert_not_threaded(backend, parent_slice)?;
    host_cgroup::assert_no_internal_processes(backend, parent_slice)?;
    host_cgroup::require_controllers(backend, parent_slice, Controller::REQUIRED)?;
    host_cgroup::enable_subtree_controllers(backend, parent_slice, Controller::ENABLE_ORDER)?;
    host_cgroup::chown_subtree_to_d2bd(backend, parent_slice, d2bd_uid, d2bd_gid)?;
    Ok(parent_slice.to_path_buf())
}

fn classify_decision(err: &CgroupOpError) -> AuditDecision {
    match err {
        CgroupOpError::Host(
            CgroupError::CgroupDelegationRefused { .. }
            | CgroupError::CgroupKillOnAncestorRefused { .. }
            | CgroupError::CgroupPartitionRootForbidden { .. }
            | CgroupError::ThreadedCgroupForbidden { .. },
        )
        | CgroupOpError::CgroupNotDelegated { .. }
        | CgroupOpError::PathClassForeign { .. }
        | CgroupOpError::KillAncestor { .. } => AuditDecision::DeniedRefused,
        _ => AuditDecision::Errored,
    }
}

fn is_under_slice(candidate: &Path, slice: &Path) -> bool {
    candidate.starts_with(slice)
}

// ---------------------------------------------------------------------------
// Test harness (in-memory audit sink + fake backend integration).
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "fake-backends"))]
pub mod test_harness {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    pub struct RecordingAuditSink {
        pub entries: Mutex<Vec<RecordedEntry>>,
    }

    #[derive(Debug, Clone)]
    pub struct RecordedEntry {
        pub operation: &'static str,
        pub decision: AuditDecision,
        pub fields: AuditFields,
        pub error_kind: Option<String>,
    }

    impl AuditSink for RecordingAuditSink {
        fn record(
            &self,
            operation: &'static str,
            decision: AuditDecision,
            fields: &AuditFields,
            error_kind: Option<&str>,
        ) {
            self.entries.lock().unwrap().push(RecordedEntry {
                operation,
                decision,
                fields: fields.clone(),
                error_kind: error_kind.map(str::to_owned),
            });
        }
    }
}

#[derive(Debug)]
pub struct LiveOpenCgroupDirOutcome {
    pub cgroup_path: PathBuf,
    pub fd: OwnedFd,
}

pub fn live_delegate_cgroup_v2(
    exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts::broker_wire::DelegateCgroupV2Request,
    _audit_log: &crate::audit::AuditLog,
) -> Result<(), super::OpError> {
    let subject = req.scope_id.as_str().trim_start_matches("vm:");
    if subject != "runtime" && resolver.find_manifest_vm(subject).is_none() {
        return Err(super::OpError::UnknownSubject {
            operation: "DelegateCgroupV2",
            subject: req.scope_id.as_str().to_owned(),
        });
    }
    let backend = host_cgroup::RealCgroupBackend::new();
    create_d2b_slice(
        &backend,
        Path::new("/sys/fs/cgroup"),
        Path::new(DEFAULT_DELEGATED_PARENT_SLICE),
        exec.d2bd_uid(),
        exec.d2bd_gid(),
    )
    .map_err(super::OpError::from)?;
    Ok(())
}

pub fn live_open_cgroup_dir(
    exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &OpenCgroupDirRequest,
    audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenCgroupDirOutcome, super::OpError> {
    let cgroup_path = match req.path_class {
        BrokerPathClass::Runtime => {
            live_delegate_cgroup_v2(
                exec,
                resolver,
                &d2b_contracts::broker_wire::DelegateCgroupV2Request {
                    scope_id: ScopeId::new("runtime"),
                    tracing_span_id: req.tracing_span_id.clone(),
                },
                audit_log,
            )?;
            PathBuf::from(DEFAULT_DELEGATED_PARENT_SLICE)
        }
        BrokerPathClass::Vm => {
            let vm_name = req.scope_id.as_str().trim_start_matches("vm:");
            if resolver.find_manifest_vm(vm_name).is_none() {
                return Err(super::OpError::UnknownSubject {
                    operation: "OpenCgroupDir",
                    subject: vm_name.to_owned(),
                });
            }
            live_delegate_cgroup_v2(
                exec,
                resolver,
                &d2b_contracts::broker_wire::DelegateCgroupV2Request {
                    scope_id: ScopeId::new(vm_name),
                    tracing_span_id: req.tracing_span_id.clone(),
                },
                audit_log,
            )?;
            let backend = host_cgroup::RealCgroupBackend::new();
            host_cgroup::create_vm_subtree(
                &backend,
                Path::new(DEFAULT_DELEGATED_PARENT_SLICE),
                vm_name,
                exec.d2bd_uid(),
                exec.d2bd_gid(),
            )
            .map_err(super::OpError::from)?
        }
    };
    let fd = crate::sys::path_safe::open_dir_path_safe(&cgroup_path).map_err(|e| {
        super::OpError::Io {
            path: cgroup_path.clone(),
            detail: e.to_string(),
        }
    })?;
    Ok(LiveOpenCgroupDirOutcome { cgroup_path, fd })
}

#[cfg(test)]
mod tests {
    use super::test_harness::RecordingAuditSink;
    use super::*;
    use d2b_host::cgroup::fake::FakeCgroupBackend;

    const ROOT: &str = "/sys/fs/cgroup";

    fn context(known: &[&str]) -> CgroupBundleContext {
        CgroupBundleContext {
            unified_hierarchy_root: PathBuf::from(ROOT),
            parent_slice: PathBuf::from(DEFAULT_DELEGATED_PARENT_SLICE),
            d2bd_uid: 1234,
            d2bd_gid: 1234,
            known_vms: known.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn backend(uid: u32) -> FakeCgroupBackend {
        let b = FakeCgroupBackend::new(uid);
        b.seed_unified(Path::new(ROOT));
        host_cgroup::CgroupBackend::mkdir(&b, Path::new(DEFAULT_DELEGATED_PARENT_SLICE))
            .expect("seed delegated parent slice");
        b
    }

    fn undelegated_backend(uid: u32) -> FakeCgroupBackend {
        let b = FakeCgroupBackend::new(uid);
        b.seed_unified(Path::new(ROOT));
        b
    }

    #[test]
    fn delegate_happy_path() {
        let b = backend(1234);
        let ctx = context(&[]);
        let audit = RecordingAuditSink::default();
        let outcome = handle_delegate_cgroup_v2(&b, &ctx, &audit).unwrap();
        assert_eq!(
            outcome.slice_path,
            Path::new(DEFAULT_DELEGATED_PARENT_SLICE)
        );
        assert_eq!(outcome.controllers_enabled.len(), 5);
        let entries = audit.entries.lock().unwrap();
        let last = entries.last().expect("at least one record");
        assert_eq!(last.operation, "DelegateCgroupV2");
        assert_eq!(last.decision, AuditDecision::Allowed);
        assert_eq!(last.fields.owner_uid, Some(1234));
    }

    #[test]
    fn delegate_refused_uid_zero() {
        let b = backend(0);
        let ctx = context(&[]);
        let audit = RecordingAuditSink::default();
        let err = handle_delegate_cgroup_v2(&b, &ctx, &audit).unwrap_err();
        match err {
            super::super::OpError::Cgroup(CgroupOpError::Host(
                CgroupError::CgroupDelegationRefused { .. },
            )) => {}
            other => panic!("unexpected: {other:?}"),
        }
        let entries = audit.entries.lock().unwrap();
        let last = entries.last().unwrap();
        assert_eq!(last.decision, AuditDecision::DeniedRefused);
        assert_eq!(
            last.error_kind.as_deref(),
            Some("cgroup-delegation-refused")
        );
    }

    #[test]
    fn delegate_refuses_when_parent_not_delegated() {
        let b = undelegated_backend(1234);
        let ctx = context(&[]);
        let audit = RecordingAuditSink::default();
        let err = handle_delegate_cgroup_v2(&b, &ctx, &audit).unwrap_err();
        match err {
            super::super::OpError::Cgroup(CgroupOpError::CgroupNotDelegated {
                expected_parent,
            }) => assert_eq!(expected_parent, Path::new(DEFAULT_DELEGATED_PARENT_SLICE)),
            other => panic!("unexpected: {other:?}"),
        }
        let entries = audit.entries.lock().unwrap();
        let last = entries.last().unwrap();
        assert_eq!(last.decision, AuditDecision::DeniedRefused);
        assert_eq!(last.error_kind.as_deref(), Some("cgroup-not-delegated"));
    }

    #[test]
    fn open_unknown_subject_audited() {
        let b = backend(1234);
        let ctx = context(&["alpha"]);
        let audit = RecordingAuditSink::default();
        let err = handle_open_cgroup_dir(&b, &ctx, "unknown", &audit).unwrap_err();
        match err {
            super::super::OpError::UnknownSubject { operation, subject } => {
                assert_eq!(operation, "OpenCgroupDir");
                assert_eq!(subject, "unknown");
            }
            other => panic!("unexpected: {other:?}"),
        }
        let entries = audit.entries.lock().unwrap();
        let last = entries.last().unwrap();
        assert_eq!(last.decision, AuditDecision::DeniedUnknown);
        assert_eq!(last.fields.path_class, Some(PathClass::Unknown));
    }

    #[test]
    fn open_known_vm_leaf_after_delegation() {
        let b = backend(1234);
        let ctx = context(&["alpha"]);
        let audit = RecordingAuditSink::default();
        handle_delegate_cgroup_v2(&b, &ctx, &audit).unwrap();
        host_cgroup::create_vm_subtree(&b, &ctx.slice_path(), "alpha", ctx.d2bd_uid, ctx.d2bd_gid)
            .unwrap();
        let outcome = handle_open_cgroup_dir(&b, &ctx, "alpha", &audit).unwrap();
        assert_eq!(outcome.path_class, PathClass::VmLeaf);
    }

    #[test]
    fn cgroup_kill_ancestor_refused() {
        let b = backend(1234);
        let ctx = context(&["alpha"]);
        let audit = RecordingAuditSink::default();
        handle_delegate_cgroup_v2(&b, &ctx, &audit).unwrap();
        // Asking the slice itself isn't a known vm — refused with
        // `unknown-subject` per the bundle gate.
        let err = handle_cgroup_kill(&b, &ctx, "d2b.slice", &audit).unwrap_err();
        match err {
            super::super::OpError::UnknownSubject { .. } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
