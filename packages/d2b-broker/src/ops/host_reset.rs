//! The one-shot ownership-bounded reset runner (U32, KTD15).
//!
//! # A reset is an ownership operation, not a migration
//!
//! The clean break (R43-R48, KD6) starts the new release from fresh d2b
//! state. This runner is how that happens: `d2b host reset` invokes it
//! offline, without a running daemon and without ever opening the previous
//! release's SpecStore. Nothing here reads a desired row, replays a policy,
//! or imports a record. The only inputs are the verified new deployment
//! graph and live evidence read off this host.
//!
//! # Why this file never mentions a SpecStore
//!
//! Reading the old store to learn what to delete would be migration, and
//! migration is exactly what the clean break removed. The deletion
//! inventory is therefore derived from one thing only: the exact ownership
//! description the verified new deployment graph carries. An old store
//! that is unreadable, in an old format, or absent changes nothing about
//! what this runner deletes, because the runner never looks.
//!
//! # The order of operations is the safety property
//!
//! 1. Verify the new deployment graph (self-hash). A tampered graph is
//!    refused before anything is read from the host.
//! 2. Admit the reset Operation against the prior accepted graph built
//!    from that document's canonical authority rows, under the exact
//!    local operator authority. One evaluator, no bypass.
//! 3. Observe the deployment root no-follow and decide the ownership
//!    boundary plus the drain proof. Any unresolved question is a refusal.
//! 4. Only then unlink, strictly inside the verified inventory.
//! 5. Establish the fresh deployment root and a NEW incarnation, so the
//!    next boot initializes rather than resynchronizing a rollback.
//!
//! # The runner unlinks names; it never repairs permissions
//!
//! A store-view farm shares inodes with the system store. A chmod or
//! chown there is a permission change on every name of that inode, so the
//! inventory is unlink-only and a hardlink farm is never descended into.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use d2b_contracts_resource::v3::{
    AdmissionDecision, AdmissionStage, AuthoritySubject, AuthoritySubjectKind, CanonicalJsonObject,
    ResourceRef, StoreIncarnation, ZoneId, canonical_digest, canonical_json_bytes,
};
use d2b_core::resource_authority::{
    AcceptedGraph, AcceptedGraphError, GraphAuthority, GraphMutation, MutationKind,
    MutationSubjectEvidence, ProjectionRow, TransportIdentity,
};
use d2b_host::ownership_matrix::{
    DrainEvidence, OwnedPath, ResetOwnership, ResetRefusal, VerifiedResetOwnership,
    observe_reset, render_ownership_marker, verify_reset, OWNERSHIP_MARKER_FILE,
};
use serde::{Deserialize, Serialize};

/// The deployment-root-relative name of the verified new deployment graph.
///
/// It lives at the deployment root beside the ownership marker and is
/// deliberately outside every owned entry: reset reads it to learn what it
/// may remove, then rewrites it with the new incarnation.
pub const DEPLOYMENT_GRAPH_FILE: &str = "deployment-graph.json";

/// The deployment-root-relative name of the fresh-incarnation record reset
/// establishes after the removal.
pub const INCARNATION_FILE: &str = "incarnation.json";

/// The broker projection state directory: the projection cursor and
/// digest, the prepared fences, the effect journal, and the reservation
/// journal.
pub const AUTHORITY_STATE_DIR: &str = "authority";

/// The broker's durable declared state cells, including the binding
/// reservation ledger.
pub const STATE_CELLS_DIR: &str = "state-cells";

/// The broker's host-generation handoff journal.
pub const HOST_GENERATION_JOURNAL_DIR: &str = "host-generation-handoffs";

/// The bounded read for the deployment graph document.
pub const MAX_DEPLOYMENT_GRAPH_BYTES: usize = 64 * 1024;

/// The domain separator for the deployment graph self-hash.
pub const DEPLOYMENT_GRAPH_DIGEST_DOMAIN: &str = "d2b:v3:deployment-graph";

/// The reset Operation the graph declares and this runner admits.
pub const RESET_OPERATION_REF: &str = "Operation/local-reset";

/// The domain separator for the fresh-incarnation record.
pub const INCARNATION_DIGEST_DOMAIN: &str = "d2b:v3:deployment-incarnation";

// ---------------------------------------------------------------------------
// The verified new deployment graph
// ---------------------------------------------------------------------------

/// One owned path as the deployment graph declares it.
///
/// The paths are absolute and, by construction, strictly beneath the
/// deployment root: a graph that names anything else describes an
/// ownership this reset does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclaredOwnedPath {
    /// Absolute path beneath the deployment root.
    pub path: PathBuf,
    /// The treatment the reset may use: `tree`, `file`, or
    /// `hardlinkFarm`.
    pub kind: String,
}

/// The verified new deployment graph, as published beside the deployment
/// root.
///
/// The document is self-hashed: `graphDigest` covers the canonical bytes
/// of everything else in it, so a graph whose ownership description was
/// edited after verification fails closed before a single path is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentGraph {
    /// The document's own schema tag.
    pub schema_version: String,
    /// The Zone the graph describes.
    pub zone: ZoneId,
    /// The store incarnation the new deployment is verified in.
    pub store_incarnation: StoreIncarnation,
    /// The ownership id the deployment root's marker must carry.
    pub ownership_id: String,
    /// The exact owned path inventory, in declaration order.
    pub owned: Vec<DeclaredOwnedPath>,
    /// Volume sources the operator owns outside the deployment root.
    pub external_sources: Vec<PathBuf>,
    /// The accepted `Role` rows, as canonical bytes.
    pub roles: Vec<GraphAuthorityRow>,
    /// The accepted `RoleBinding` rows, as canonical bytes.
    pub role_bindings: Vec<GraphAuthorityRow>,
    /// `sha256:` over the canonical bytes of this document without
    /// `graphDigest`.
    pub graph_digest: String,
}

/// One accepted authority row of the deployment graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphAuthorityRow {
    /// The exact resource reference the row was accepted for.
    pub reference: String,
    /// The row's canonical admitted bytes.
    pub admitted: serde_json::Value,
}

/// The document schema tag this runner verifies.
pub const DEPLOYMENT_GRAPH_SCHEMA: &str = "d2b-deployment-graph/1";

impl DeploymentGraph {
    /// The exact ownership description this graph declares.
    ///
    /// An unknown treatment is a refusal rather than a default: a graph
    /// that cannot say how a path may be touched does not get to have it
    /// touched.
    pub fn ownership(&self) -> Result<ResetOwnership, ResetFailure> {
        let mut owned = Vec::with_capacity(self.owned.len());
        for entry in &self.owned {
            let kind = match entry.kind.as_str() {
                "tree" => d2b_host::ownership_matrix::OwnedPathKind::Tree,
                "file" => d2b_host::ownership_matrix::OwnedPathKind::File,
                "hardlinkFarm" => d2b_host::ownership_matrix::OwnedPathKind::HardlinkFarm,
                other => {
                    return Err(ResetFailure::Ownership(
                        ResetRefusal::OwnershipKindMismatch {
                            path: entry.path.clone(),
                            expected: "tree | file | hardlinkFarm".to_owned(),
                            observed: other.to_owned(),
                        },
                    ));
                }
            };
            owned.push(OwnedPath {
                path: entry.path.clone(),
                kind,
            });
        }
        Ok(ResetOwnership {
            owned,
            external_sources: self.external_sources.clone(),
        })
    }

    /// Verify the document's self-hash over its own canonical bytes.
    pub fn verify_digest(&self) -> Result<(), ResetFailure> {
        let mut without_digest = self.clone();
        without_digest.graph_digest = String::new();
        let bytes = canonical_json_bytes(&without_digest)
            .map_err(|error| ResetFailure::Graph(format!("graph-canonical-bytes: {error}")))?;
        if canonical_digest(DEPLOYMENT_GRAPH_DIGEST_DOMAIN, &bytes) != self.graph_digest {
            return Err(ResetFailure::Graph("deployment-graph-digest-mismatch".to_owned()));
        }
        Ok(())
    }
}

/// The prior accepted graph the reset Operation is admitted against.
///
/// Built from the deployment graph's own canonical rows through
/// [`AcceptedGraph::from_canonical_rows`], so it decides exactly what the
/// same rows decide anywhere else. The deployment root subject is the
/// exact local operator authority this invocation presents: the admission
/// is an exact-identity one-shot, not a bootstrap class that admits
/// whatever a future caller names.
fn prior_accepted_graph(graph: &DeploymentGraph) -> Result<AcceptedGraph, ResetFailure> {
    let root = AuthoritySubject::unresourced(AuthoritySubjectKind::Operator);
    // The decoded rows are collected first so the borrowed projections
    // below all outlive the call, rather than pointing at a `reference`
    // that a loop iteration already dropped.
    let mut decoded: Vec<(ResourceRef, CanonicalJsonObject)> =
        Vec::with_capacity(graph.roles.len() + graph.role_bindings.len());
    for rows in [&graph.roles, &graph.role_bindings] {
        for row in rows {
            let reference = ResourceRef::parse(row.reference.as_str()).map_err(|error| {
                ResetFailure::Graph(format!("graph-row-reference: {error}"))
            })?;
            let admitted = serde_json::from_value::<CanonicalJsonObject>(row.admitted.clone())
                .map_err(|error| ResetFailure::Graph(format!("graph-row-bytes: {error}")))?;
            decoded.push((reference, admitted));
        }
    }
    let rows = decoded
        .iter()
        .map(|(reference, admitted)| ProjectionRow::new(reference, admitted));
    AcceptedGraph::from_canonical_rows(
        graph.zone.clone(),
        graph.store_incarnation.clone(),
        root,
        rows,
    )
    .map_err(|error: AcceptedGraphError| ResetFailure::Graph(format!("graph-rows: {error}")))
}

/// Admit the reset Operation through the one evaluator.
///
/// The target is the exact `Operation` reference the deployment graph
/// declares, so a caller cannot redirect the reset at another resource,
/// and the initiating subject is unresourced explicit local operator
/// authority, which the evaluator admits only against a graph whose
/// deployment root is that exact subject.
fn admit_reset(graph: &DeploymentGraph, accepted: &AcceptedGraph) -> Result<(), ResetFailure> {
    let target = ResourceRef::parse(RESET_OPERATION_REF)
        .map_err(|error| ResetFailure::Graph(format!("reset-operation-ref: {error}")))?;
    let request = GraphMutation::new(
        graph.zone.clone(),
        MutationSubjectEvidence::new(
            AuthoritySubject::unresourced(AuthoritySubjectKind::Operator),
            TransportIdentity::OperatorConsole,
        ),
        MutationKind::Delete,
        target,
    );
    match GraphAuthority::admit_mutation(&request, accepted) {
        AdmissionDecision::Admitted => Ok(()),
        AdmissionDecision::Refused { stage, reason } => Err(ResetFailure::Admission {
            stage,
            // `RefusalReason` is a closed kebab-case contract enum, so its
            // serialized spelling is the stable label; the runtime never
            // carries caller text inside one.
            reason: serde_json::to_value(reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "refusal-reason-unrenderable".to_owned()),
        }),
    }
}

// ---------------------------------------------------------------------------
// Live drain evidence
// ---------------------------------------------------------------------------

/// Where the runner reads live drain evidence from.
///
/// Every path is derived from the deployment root plus the host's own
/// cgroup hierarchy; none is a caller-supplied host path, so a reset
/// cannot be pointed at someone else's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainProbes {
    /// The delegated cgroup subtree whose live members are live
    /// workloads.
    pub cgroup_root: PathBuf,
    /// Durable managed-runner records. A present record names a managed
    /// process that has not been retired.
    pub runner_records: PathBuf,
    /// The lease and lock files a held lock makes live.
    pub lock_root: PathBuf,
    /// The active host-generation record.
    pub host_generation: PathBuf,
    /// Managed markers that must be absent before removal.
    pub markers: Vec<PathBuf>,
}

impl DrainProbes {
    /// Derive the probe set from the deployment root and the host cgroup
    /// hierarchy root.
    pub fn under(deployment_root: &Path, cgroup_root: PathBuf) -> Self {
        Self {
            cgroup_root,
            runner_records: deployment_root.join("runtime/runners"),
            lock_root: deployment_root.join("locks"),
            host_generation: deployment_root.join("host-generation/active"),
            markers: vec![deployment_root.join("runtime/host-runtime.json")],
        }
    }
}

/// The bounded read for one drain probe listing.
const MAX_DRAIN_LISTING_BYTES: usize = 64 * 1024;

/// Collect the live evidence the reset refuses to ignore.
///
/// Reading absence is not reading evidence: a directory that happens to
/// be empty contributes nothing, and only a positive observation - a live
/// cgroup member, a named runner record, a lock that will not take, a
/// present host-generation record - becomes a refusal.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn collect_drain_evidence(probes: &DrainProbes) -> DrainEvidence {
    DrainEvidence {
        live_cgroup_members: live_cgroup_members(&probes.cgroup_root),
        live_processes: live_runner_records(&probes.runner_records),
        live_markers: present_paths(&probes.markers),
        held_leases: held_locks(&probes.lock_root),
        active_host_generation: fs::read_to_string(&probes.host_generation)
            .ok()
            .map(|body| body.trim().to_owned())
            .filter(|generation| !generation.is_empty()),
    }
}

/// Every leaf under `root` that still has live members.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn live_cgroup_members(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_cgroup(root, &mut found, 0);
    found
}

/// The walk is bounded so a pathological hierarchy cannot turn a refusal
/// into an unbounded scan.
const CGROUP_WALK_DEPTH: usize = 8;

/// Read a `cgroup.procs` at each level; a non-empty file is live
/// membership even when its processes are leafless.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn walk_cgroup(path: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > CGROUP_WALK_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        let Ok(meta) = fs::symlink_metadata(&child) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        if let Ok(procs) = fs::read_to_string(child.join("cgroup.procs"))
            && !procs.trim().is_empty()
        {
            out.push(child.clone());
        }
        walk_cgroup(&child, out, depth + 1);
    }
}

/// Every managed-runner record the previous deployment left behind.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn live_runner_records(root: &Path) -> Vec<PathBuf> {
    list_files_bounded(root)
}

/// Every lease or lock file whose `flock(2)` a live holder still has.
///
/// The lock root is walked rather than listed flat because the declared
/// layout nests one level (`locks/usbip/<busid>`), and a lease nobody
/// looks for is not a lease the boundary saw.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn held_locks(root: &Path) -> Vec<PathBuf> {
    let mut held = Vec::new();
    for path in list_files_bounded(root) {
        if lock_is_held(&path) {
            held.push(path);
        }
    }
    held
}

/// Whether any live process holds the advisory lock on `path`.
///
/// The probe is non-blocking and never blocks the runner: a lock that
/// cannot be taken immediately is held by somebody, and somebody holding
/// it is exactly the lease a reset must not cut.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn lock_is_held(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = fs::OpenOptions::new()
        .create(false)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
    else {
        return false;
    };
    match nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockExclusiveNonblock) {
        Ok(()) => {
            // It was free; give it straight back so the reset never holds
            // a lock another process is about to take.
            let _ = nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::Unlock);
            false
        }
        Err(_) => true,
    }
}

/// Bounded no-follow walk of the regular files under `root`.
///
/// Depth-bounded and byte-budgeted so a pathological tree turns into a
/// truncated probe rather than an unbounded scan, and no-follow at every
/// level so a symlink planted under a drain root is listed as itself and
/// never entered.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn list_files_bounded(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut budget = MAX_DRAIN_LISTING_BYTES;
    walk_drain_files(root, DRAIN_WALK_DEPTH, &mut found, &mut budget);
    found
}

/// The bounded walk depth for a drain probe root.
const DRAIN_WALK_DEPTH: usize = 8;

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn walk_drain_files(
    dir: &Path,
    remaining_depth: usize,
    out: &mut Vec<PathBuf>,
    budget: &mut usize,
) {
    if *budget == 0 || remaining_depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_drain_files(&path, remaining_depth - 1, out, budget);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        *budget = budget.saturating_sub(meta.len() as usize);
        out.push(path);
    }
}

/// The subset of `paths` that currently exist.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn present_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

/// Whether the invocation inspects or applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// Verify and report the exact inventory. Nothing is removed.
    Inspect,
    /// Verify, remove the inventory, and establish the fresh root.
    Apply,
}

/// One reset invocation's inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetRequest {
    /// The deployment root this reset is bounded to.
    pub deployment_root: PathBuf,
    /// Inspect or apply.
    pub mode: ResetMode,
    /// The cgroup hierarchy root the drain probe walks.
    pub cgroup_root: PathBuf,
}

impl ResetRequest {
    /// An inspect request against `deployment_root`.
    pub fn inspect(deployment_root: impl Into<PathBuf>, cgroup_root: impl Into<PathBuf>) -> Self {
        Self {
            deployment_root: deployment_root.into(),
            mode: ResetMode::Inspect,
            cgroup_root: cgroup_root.into(),
        }
    }

    /// The same request, applying.
    pub fn applying(mut self) -> Self {
        self.mode = ResetMode::Apply;
        self
    }
}

/// The stable envelope one reset renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetReport {
    /// Always true; a refusal is a separate value.
    pub ok: bool,
    /// `"inspect"` or `"apply"`.
    pub mode: &'static str,
    /// The Zone the verified graph describes.
    pub zone: String,
    /// The store incarnation this reset establishes.
    pub store_incarnation: String,
    /// The ownership id the deployment root's marker carried.
    pub ownership_id: String,
    /// The verified deployment root.
    pub deployment_root: PathBuf,
    /// The complete unlink inventory, in declaration order. An entry with
    /// `present: false` is already gone, which is what makes a repeated
    /// completed reset safe.
    pub inventory: Vec<ResetInventoryEntry>,
    /// External Volume sources the reset proved it leaves alone.
    pub external_sources: Vec<PathBuf>,
    /// The fresh incarnation record, in apply mode only.
    pub incarnation: Option<IncarnationRecord>,
}

/// One inventory line: exactly what the reset may unlink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetInventoryEntry {
    /// The owned path.
    pub path: PathBuf,
    /// The treatment it received: `tree`, `file`, or `hardlinkFarm`.
    pub kind: &'static str,
    /// Whether the path existed when the boundary was verified.
    pub present: bool,
    /// Whether apply mode actually removed it.
    pub removed: bool,
}

/// The fresh-incarnation record the reset establishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncarnationRecord {
    /// The document schema tag.
    pub schema_version: String,
    /// The Zone the fresh root describes.
    pub zone: ZoneId,
    /// The new store incarnation. It differs from the verified graph's,
    /// so the next boot initializes instead of resynchronizing.
    pub store_incarnation: StoreIncarnation,
    /// The ownership id the fresh root carries.
    pub ownership_id: String,
    /// The verified deployment graph's digest this incarnation followed.
    pub previous_graph_digest: String,
    /// `sha256:` over the canonical bytes of this record without
    /// `incarnation_digest`.
    pub incarnation_digest: String,
}

/// The document schema tag the fresh-incarnation record carries.
pub const INCARNATION_SCHEMA: &str = "d2b-deployment-incarnation/1";

/// Why a reset invocation failed.
///
/// A refusal and a graph failure are different things: a refusal is the
/// ownership or drain boundary declining to act, a graph failure is the
/// verified new deployment graph refusing to be used at all. Neither
/// ever means "carry on with what could be read".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetFailure {
    /// The ownership or drain boundary refused.
    Ownership(ResetRefusal),
    /// The verified deployment graph could not be used.
    Graph(String),
    /// The reset Operation was not admitted.
    Admission {
        stage: AdmissionStage,
        reason: String,
    },
    /// The deployment root could not be read.
    Io { path: PathBuf, detail: String },
}

impl core::fmt::Display for ResetFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ownership(refusal) => write!(formatter, "{}", refusal.code()),
            Self::Graph(detail) => write!(formatter, "reset-graph: {detail}"),
            Self::Admission { stage, reason } => {
                write!(formatter, "reset-admission: {stage:?}:{reason}")
            }
            Self::Io { path, detail } => {
                write!(formatter, "reset-io: {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for ResetFailure {}

/// The stable envelope a failure renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetFailureReport {
    /// Always false.
    pub ok: bool,
    /// The stable refusal or failure code.
    pub code: String,
    /// The stage the refusal came from, when the evaluator named one.
    pub stage: Option<String>,
    /// The path the refusal is about, when it names one.
    pub path: Option<PathBuf>,
    /// The operator-facing detail.
    pub detail: String,
}

impl ResetFailure {
    /// The stable envelope one failure renders.
    pub fn report(&self) -> ResetFailureReport {
        let (code, path, detail) = match self {
            Self::Ownership(refusal) => {
                let path = refusal.path().map(Path::to_path_buf);
                (refusal.code().to_owned(), path, refusal.code().to_owned())
            }
            Self::Graph(detail) => ("reset-graph-invalid".to_owned(), None, detail.clone()),
            Self::Admission { stage, reason } => (
                "reset-not-admitted".to_owned(),
                None,
                format!("{stage:?}:{reason}"),
            ),
            Self::Io { path, detail } => (
                "reset-io".to_owned(),
                Some(path.clone()),
                detail.clone(),
            ),
        };
        ResetFailureReport {
            ok: false,
            code,
            stage: match self {
                Self::Admission { stage, .. } => Some(format!("{stage:?}")),
                _ => None,
            },
            path,
            detail,
        }
    }
}

/// The exit code a reset refusal reports: the same
/// `--apply-or-dry-run-required` class the CLI already owns, because a
/// reset that would have needed a change nobody asked for is exactly what
/// that code means.
pub const RESET_REFUSAL_EXIT: u8 = 78;

/// Run one reset: verify, decide, and (only when asked) remove.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn run_owned_reset(request: &ResetRequest) -> Result<ResetReport, ResetFailure> {
    // 1. The verified new deployment graph, read self-hashed and bounded.
    let graph = read_deployment_graph(&request.deployment_root)?;
    graph.verify_digest()?;

    // 2. Admit the reset Operation against the graph's own prior state.
    let accepted = prior_accepted_graph(&graph)?;
    admit_reset(&graph, &accepted)?;

    // 3. The exact ownership boundary plus the live drain proof.
    let ownership = graph.ownership()?;
    let observation = observe_reset(&request.deployment_root, &graph.ownership_id, &ownership)
        .map_err(ResetFailure::Ownership)?;
    let probes = DrainProbes::under(&request.deployment_root, request.cgroup_root.clone());
    let evidence = collect_drain_evidence(&probes);
    let verified = verify_reset(&observation, &ownership, &evidence)
        .map_err(ResetFailure::Ownership)?;

    let inventory: Vec<ResetInventoryEntry> = verified
        .entries
        .iter()
        .map(|entry| ResetInventoryEntry {
            path: entry.path.clone(),
            kind: owned_path_kind_str(entry.kind),
            present: entry.present,
            removed: false,
        })
        .collect();

    if request.mode == ResetMode::Inspect {
        return Ok(ResetReport {
            ok: true,
            mode: "inspect",
            zone: graph.zone.as_str().to_owned(),
            store_incarnation: graph.store_incarnation.as_str().to_owned(),
            ownership_id: verified.ownership_id.clone(),
            deployment_root: verified.deployment_root.clone(),
            inventory,
            external_sources: verified.external_sources.clone(),
            incarnation: None,
        });
    }

    // 4. Remove strictly inside the verified inventory.
    let removed = remove_verified(&verified)?;
    let inventory: Vec<ResetInventoryEntry> = verified
        .entries
        .iter()
        .zip(removed)
        .map(|(entry, removed)| ResetInventoryEntry {
            path: entry.path.clone(),
            kind: owned_path_kind_str(entry.kind),
            present: entry.present,
            removed,
        })
        .collect();

    // 5. The fresh deployment root and a NEW incarnation.
    let incarnation = establish_fresh_root(&verified, &graph)?;

    Ok(ResetReport {
        ok: true,
        mode: "apply",
        zone: graph.zone.as_str().to_owned(),
        store_incarnation: incarnation.store_incarnation.as_str().to_owned(),
        ownership_id: verified.ownership_id.clone(),
        deployment_root: verified.deployment_root.clone(),
        inventory,
        external_sources: verified.external_sources.clone(),
        incarnation: Some(incarnation),
    })
}

/// The stable treatment label one inventory line carries.
const fn owned_path_kind_str(kind: d2b_host::ownership_matrix::OwnedPathKind) -> &'static str {
    match kind {
        d2b_host::ownership_matrix::OwnedPathKind::Tree => "tree",
        d2b_host::ownership_matrix::OwnedPathKind::File => "file",
        d2b_host::ownership_matrix::OwnedPathKind::HardlinkFarm => "hardlinkFarm",
    }
}

/// Read and decode the verified deployment graph, bounded and no-follow.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn read_deployment_graph(deployment_root: &Path) -> Result<DeploymentGraph, ResetFailure> {
    let path = deployment_root.join(DEPLOYMENT_GRAPH_FILE);
    let meta = fs::symlink_metadata(&path).map_err(|error| ResetFailure::Io {
        path: path.clone(),
        detail: error.to_string(),
    })?;
    if meta.file_type().is_symlink() {
        return Err(ResetFailure::Io {
            path,
            detail: "the deployment graph is a symlink".to_owned(),
        });
    }
    if meta.len() as usize > MAX_DEPLOYMENT_GRAPH_BYTES {
        return Err(ResetFailure::Io {
            path,
            detail: "the deployment graph exceeds the bounded read".to_owned(),
        });
    }
    let bytes = fs::read(&path).map_err(|error| ResetFailure::Io {
        path: path.clone(),
        detail: error.to_string(),
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ResetFailure::Graph(format!("deployment-graph-parse: {error}")))
}

/// Unlink exactly the verified inventory, nothing else.
///
/// The traversal is no-follow at every level: a `Tree` is emptied by
/// removing its own children by name and never re-resolves a link, and a
/// `HardlinkFarm` is emptied one level only, so a `gcroots` link into the
/// system store is unlinked as a name and never entered. No inode is
/// chmod-ed or chown-ed anywhere in here.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn remove_verified(verified: &VerifiedResetOwnership) -> Result<Vec<bool>, ResetFailure> {
    let mut removed = Vec::with_capacity(verified.entries.len());
    for entry in &verified.entries {
        removed.push(if entry.present {
            remove_owned_path(entry)?;
            true
        } else {
            false
        });
    }
    Ok(removed)
}

/// Remove one verified owned path and everything the declaration covers.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn remove_owned_path(entry: &d2b_host::ownership_matrix::VerifiedOwnedPath) -> Result<(), ResetFailure>
{
    match entry.kind {
        d2b_host::ownership_matrix::OwnedPathKind::File => remove_nofollow(&entry.path),
        d2b_host::ownership_matrix::OwnedPathKind::Tree => remove_tree(&entry.path),
        d2b_host::ownership_matrix::OwnedPathKind::HardlinkFarm => remove_shallow(&entry.path),
    }
    .map_err(|detail| ResetFailure::Io {
        path: entry.path.clone(),
        detail,
    })
}

/// Unlink one path without following it. A symlink is unlinked as itself.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn remove_nofollow(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => remove_tree(path),
        Ok(_) => fs::remove_file(path).map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Remove a directory and everything under it, bounded and no-follow.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn remove_tree(path: &Path) -> Result<(), String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if meta.file_type().is_symlink() {
        return fs::remove_file(path).map_err(|error| error.to_string());
    }
    if meta.is_dir() {
        let entries = fs::read_dir(path).map_err(|error| error.to_string())?;
        for entry in entries {
            let child = entry.map_err(|error| error.to_string())?.path();
            remove_tree(&child)?;
        }
        return fs::remove_dir(path).map_err(|error| error.to_string());
    }
    fs::remove_file(path).map_err(|error| error.to_string())
}

/// Remove a hardlink farm's own entries, one level deep.
///
/// The farm is never descended into. That is the whole point: its
/// inodes are shared with the system store, and its sibling metadata
/// (`gcroots/`, `meta/`, `sync.lock`) is removed by its own declaration
/// rather than by being swept from here.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn remove_shallow(path: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let child = entry.map_err(|error| error.to_string())?.path();
        let meta = match fs::symlink_metadata(&child) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if meta.is_dir() && !meta.file_type().is_symlink() {
            remove_tree(&child)?;
        } else {
            fs::remove_file(&child).map_err(|error| error.to_string())?;
        }
    }
    fs::remove_dir(path).map_err(|error| error.to_string())
}

/// Re-establish the deployment root and write a NEW incarnation.
///
/// The new incarnation is derived from the verified graph's, never from
/// the store that just went away: it is the new model's first identity,
/// so the next boot initializes instead of resynchronizing a rollback.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn establish_fresh_root(
    verified: &VerifiedResetOwnership,
    graph: &DeploymentGraph,
) -> Result<IncarnationRecord, ResetFailure> {
    let root = &verified.deployment_root;
    fs::create_dir_all(root).map_err(|error| ResetFailure::Io {
        path: root.clone(),
        detail: error.to_string(),
    })?;
    let marker = root.join(OWNERSHIP_MARKER_FILE);
    write_atomic(&marker, render_ownership_marker(&verified.ownership_id).as_bytes())?;

    let mut incarnation = IncarnationRecord {
        schema_version: INCARNATION_SCHEMA.to_owned(),
        zone: graph.zone.clone(),
        store_incarnation: next_incarnation(graph)?,
        ownership_id: verified.ownership_id.clone(),
        previous_graph_digest: graph.graph_digest.clone(),
        incarnation_digest: String::new(),
    };
    let bytes = canonical_json_bytes(&incarnation)
        .map_err(|error| ResetFailure::Graph(format!("incarnation-bytes: {error}")))?;
    incarnation.incarnation_digest = canonical_digest(INCARNATION_DIGEST_DOMAIN, &bytes);
    let rendered = serde_json::to_vec_pretty(&incarnation)
        .map_err(|error| ResetFailure::Graph(format!("incarnation-json: {error}")))?;
    write_atomic(&root.join(INCARNATION_FILE), &rendered)?;
    Ok(incarnation)
}

/// The incarnation the fresh root is established in.
///
/// An identity, not a counter: the new incarnation is derived from the
/// verified graph's own identity, so two resets of the same graph produce
/// the same one and a graph that was never verified produces none.
fn next_incarnation(graph: &DeploymentGraph) -> Result<StoreIncarnation, ResetFailure> {
    let current = graph.store_incarnation.as_str();
    let suffix = current
        .rsplit_once('-')
        .and_then(|(_, tail)| tail.parse::<u64>().ok());
    let next = match suffix {
        Some(value) => value.saturating_add(1),
        None => 1,
    };
    let base = match suffix {
        Some(_) => current.rsplit_once('-').map(|(head, _)| head).unwrap_or(current),
        None => current,
    };
    StoreIncarnation::parse(format!("{base}-{next}"))
        .map_err(|error| ResetFailure::Graph(format!("incarnation-token: {error}")))
}

/// Write one bounded document, replacing atomically.
///
/// No permission is changed on any existing inode here: a fresh file is
/// created and renamed over, so nothing shared is ever chmod-ed.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn write_atomic(path: &Path, body: &[u8]) -> Result<(), ResetFailure> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, body).map_err(|error| ResetFailure::Io {
        path: temporary.clone(),
        detail: error.to_string(),
    })?;
    fs::rename(&temporary, path).map_err(|error| ResetFailure::Io {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })
}
