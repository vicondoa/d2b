//! Work-item state gates for wave sealing and the wave exit boundary.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use super::{
    DeliveryError, Result,
    model::{
        CandidateId, CandidateMaterial, MAX_WAVE_ORDINAL, RepositoryRecord, qualified_wave_parts,
        sha256_bytes,
    },
    storage::{MAX_JSON_BYTES, PANEL_REQUEST_FILE, SNAPSHOT_FILE, StateRoot},
};

const GRAPH_PATH: &str = "docs/specs/ADR-046-implementation-graph.json";
const WORK_ITEMS_PATH: &str = "docs/specs/ADR-046-work-items.json";
const CONSTITUTION_PATH: &str = ".specify/memory/constitution.md";

const ADR046_W5_RETAINED_WAVE: &str = "adr046w5";
const ADR046_W5_RETAINED_CANDIDATE: &str =
    "d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4";
const ADR046_W5_RETAINED_REQUEST_SHA256: &str =
    "15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7";
const ADR046_W5_RETAINED_SNAPSHOT_FILE_SHA256: &str =
    "dcf4d71a572bdf0766de557dde6b8ede7fd680eb9f85572238575d2ab5c82149";
const ADR046_W5_MERGED_BOUNDARY: &str = "177235ed37188b3be87525e7f016fb43401574c5";
const ADR046_FR036_CONSTITUTION_SHA256: &str =
    "f85a5ccf0175a7b6233e9f2828c221f48e564f71708569b6a585801ccf26db79";
const D2B_REPOSITORY_ID: &str = "github.com/vicondoa/d2b";
const D2B_INTEGRATION_REF: &str = "refs/remotes/origin/v3";
const ADR046_W5_RETAINED_EVIDENCE_SHA256: &str =
    "7deb84943d36962493422407ac74342fd598b2fea4970ea1a162942e25cfd33d";
const ADR046_W5_WORK_ITEM_PROJECTION_SHA256: &str =
    "3a7112ccade53a2a47f56e2d25abd9931e48723208ea0bf36787cd387df6ddf4";

struct HistoricalPredecessorPolicy<'a> {
    repository_id: &'a str,
    retained_wave: &'a str,
    retained_candidate_id: &'a str,
    retained_request_sha256: &'a str,
    retained_snapshot_file_sha256: &'a str,
    predecessor_merge_oid: &'a str,
    constitution_sha256: &'a str,
    retained_evidence_sha256: &'a str,
    integration_ref: &'a str,
}

const ADR046_W6_HISTORICAL_POLICY: HistoricalPredecessorPolicy<'static> =
    HistoricalPredecessorPolicy {
        repository_id: D2B_REPOSITORY_ID,
        retained_wave: ADR046_W5_RETAINED_WAVE,
        retained_candidate_id: ADR046_W5_RETAINED_CANDIDATE,
        retained_request_sha256: ADR046_W5_RETAINED_REQUEST_SHA256,
        retained_snapshot_file_sha256: ADR046_W5_RETAINED_SNAPSHOT_FILE_SHA256,
        predecessor_merge_oid: ADR046_W5_MERGED_BOUNDARY,
        constitution_sha256: ADR046_FR036_CONSTITUTION_SHA256,
        retained_evidence_sha256: ADR046_W5_RETAINED_EVIDENCE_SHA256,
        integration_ref: D2B_INTEGRATION_REF,
    };

#[derive(Deserialize)]
struct GraphView {
    nodes: Vec<NodeView>,
}

#[derive(Deserialize)]
struct NodeView {
    id: String,
    kind: String,
    wave: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkItemsView {
    items: Vec<WorkItemView>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkItemView {
    implementation_state: String,
    work_item_id: String,
}

/// Rejects a wave's exit boundary - panel request, seal, and merge
/// eligibility - while any prior-wave item is not `Merged`.
///
/// Under FR-036/FR-048 a wave's *implementation* may start before its
/// predecessor is sealed and merged, so `wave snapshot` no longer runs this
/// gate. FR-049 moves the predecessor-merged condition to the successor's
/// exit boundary, which is what this gate enforces.
///
/// This also carries the rebase-freshness condition of FR-049. The manifests
/// are read out of the snapshot's own `integration_tree_oid`, so a successor
/// that has not rebased onto the integration lineage since the predecessor
/// merged still carries the pre-merge manifest, in which the predecessor's
/// items are not `Merged`, and is refused here.
///
/// Limitation: that freshness property is a manifest-content proxy, not an
/// ancestry proof. All this gate asserts is that the tree at
/// `integration_tree_oid` marks every prior-wave item `Merged`. A successor
/// that hand-edited the work-item manifest, or cherry-picked the manifest
/// change, without ever rebasing onto the predecessor's merge would pass it.
/// Do not read a passing result as evidence that the snapshot's tree descends
/// from the predecessor's merge commit. Closing that gap needs the stronger
/// ancestry check - comparing the snapshot's tree against the predecessor's
/// recorded merge commit, e.g. `git merge-base --is-ancestor <predecessor
/// merge> <snapshot commit>` - and the exit path does not currently
/// carry the predecessor's merge commit, so the data for it is absent here.
pub fn require_prior_waves_merged_for_exit(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let wave = wave_number(&material.wave)?;
    if wave == 0 {
        return Ok(());
    }
    let (graph, work_items) = load_bound_state(material, repository_roots)?;
    validate_state(&material.wave, Gate::Exit, &graph, &work_items)
}

/// Applies the ordinary predecessor-state gate, with the one accepted
/// historical predecessor disposition integrated into that same decision.
///
/// The historical state is validated first. Only after that exact check passes
/// may the ordinary prior-item scan treat the disposed predecessor phase as
/// satisfied without rewriting its retained work-item states.
pub fn require_predecessor_state_for_exit(
    state_root: &StateRoot,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let disposed_wave = if is_adr046_post_w5(material) {
        require_adr046_historical_predecessor_for_exit(state_root, material, repository_roots)?;
        Some(5)
    } else {
        None
    };
    let wave = wave_number(&material.wave)?;
    if wave == 0 {
        return Ok(());
    }
    let (graph, work_items) = load_bound_state(material, repository_roots)?;
    if disposed_wave.is_some() {
        require_disposed_work_item_projection(
            &graph,
            &work_items,
            5,
            ADR046_W5_WORK_ITEM_PROJECTION_SHA256,
        )?;
    }
    validate_state_with_disposition(
        &material.wave,
        Gate::Exit,
        &graph,
        &work_items,
        disposed_wave,
    )
}

/// Validates the one-time ADR-046 Wave 5 historical predecessor disposition.
///
/// The ordinary work-item gate proves that every prior item is marked
/// `Merged`. ADR-046 Wave 6 additionally inherits a known historical delivery
/// exception: the retained Wave 5 request consumed its binding slot with zero
/// attestations and no seal before a later tree merged. The constitution
/// accepts exactly that state, not any equivalent-looking substitute.
///
/// This check is re-run at panel request, seal, and merge eligibility. It
/// binds the exact retained candidate bytes, rejects any added panel or seal
/// state, proves the Wave 6 base and head descend from the fetched integration
/// tip, and identifies the first-parent commit that introduced the accepted
/// constitution bytes. Later constitution amendments remain valid because the
/// accepted commit stays in the integration ancestry. This is deterministic
/// workflow validation for signoff tracking, not authentication or a security
/// boundary.
pub fn require_adr046_historical_predecessor_for_exit(
    state_root: &StateRoot,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    require_adr046_historical_predecessor(state_root, material, repository_roots, false)
}

/// Entry variant: the candidate base must be the fetched integration tip
/// before a snapshot can authorize implementation.
pub fn require_adr046_historical_predecessor_at_entry(
    state_root: &StateRoot,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    require_adr046_historical_predecessor(state_root, material, repository_roots, true)
}

fn require_adr046_historical_predecessor(
    state_root: &StateRoot,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
    require_current_tip: bool,
) -> Result<()> {
    if !is_adr046_post_w5(material) {
        return Ok(());
    }
    validate_historical_predecessor(
        &ADR046_W6_HISTORICAL_POLICY,
        state_root,
        material,
        repository_roots,
        require_current_tip,
    )
}

fn is_adr046_post_w5(material: &CandidateMaterial) -> bool {
    let Ok(ordinal) = wave_number(&material.wave) else {
        return false;
    };
    ordinal >= 6
        && (material.program.eq_ignore_ascii_case("ADR046")
            || material.program.eq_ignore_ascii_case("SPEC001"))
}

/// Freezes the historically dispositioned predecessor phase.
///
/// Its retained state is read-only process evidence. Production delivery
/// commands must not create a replacement candidate, append evidence, issue
/// another request, attest, seal, or register close artifacts for it.
pub fn reject_adr046_w5_mutation(material: &CandidateMaterial, operation: &str) -> Result<()> {
    let historical = (material.program.eq_ignore_ascii_case("ADR046")
        && matches!(material.wave.as_str(), "W5" | "adr046w5"))
        || (material.program.eq_ignore_ascii_case("SPEC001") && material.wave == "spec001w5");
    if historical {
        return Err(DeliveryError::new(format!(
            "{operation} is refused for immutable historical delivery state"
        )));
    }
    Ok(())
}

fn validate_historical_predecessor(
    policy: &HistoricalPredecessorPolicy<'_>,
    state_root: &StateRoot,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
    require_current_tip: bool,
) -> Result<()> {
    let repository = material
        .repository_set
        .iter()
        .filter(|repository| repository.id == policy.repository_id)
        .collect::<Vec<_>>();
    let [repository] = repository.as_slice() else {
        return Err(DeliveryError::new(format!(
            "ADR-046 Wave 6 historical predecessor validation requires exactly one `{}` repository record",
            policy.repository_id
        )));
    };
    let root = repository_roots.get(policy.repository_id).ok_or_else(|| {
        DeliveryError::new(format!(
            "ADR-046 Wave 6 historical predecessor validation has no checkout for `{}`",
            policy.repository_id
        ))
    })?;

    if require_current_tip {
        let integration_tip = resolve_commit(root, policy.integration_ref, "integration ref")?;
        if repository.base_oid != integration_tip {
            return Err(DeliveryError::new(
                "ADR-046 Wave 6 base commit is not the fetched integration tip",
            ));
        }
    }
    for (label, descendant) in [
        ("base commit", repository.base_oid.as_str()),
        ("head commit", repository.head_oid.as_str()),
    ] {
        require_commit_ancestor(root, policy.predecessor_merge_oid, descendant, label)?;
    }
    require_commit_ancestor(
        root,
        &repository.base_oid,
        &repository.head_oid,
        "head commit from its declared base",
    )?;

    let amendment_commit = find_first_parent_amendment(
        root,
        policy.repository_id,
        policy.predecessor_merge_oid,
        &repository.base_oid,
        policy.constitution_sha256,
    )?;
    require_commit_ancestor(
        root,
        &amendment_commit,
        &repository.head_oid,
        "head commit from the accepted FR-036 amendment",
    )?;

    let retained_id = CandidateId::parse(policy.retained_candidate_id)?;
    let retained = state_root
        .existing_candidate(policy.retained_wave, &retained_id)
        .map_err(|_| DeliveryError::new("ADR-046 Wave 5 retained candidate is missing"))?;
    let entries = utf8_entries(retained.list_root()?, "ADR-046 Wave 5 retained candidate")?;
    let expected = ["evidence", PANEL_REQUEST_FILE, SNAPSHOT_FILE]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if entries != expected {
        return Err(DeliveryError::new(
            "ADR-046 Wave 5 retained candidate entry set differs from the accepted historical state",
        ));
    }

    require_candidate_file_digest(
        &retained,
        PANEL_REQUEST_FILE,
        policy.retained_request_sha256,
        "retained panel request",
    )?;
    require_candidate_file_digest(
        &retained,
        SNAPSHOT_FILE,
        policy.retained_snapshot_file_sha256,
        "retained snapshot",
    )?;
    let evidence_sha256 = retained_evidence_digest(&retained)?;
    if evidence_sha256 != policy.retained_evidence_sha256 {
        return Err(DeliveryError::new("retained evidence tree digest mismatch"));
    }
    Ok(())
}

fn require_candidate_file_digest(
    candidate: &super::storage::CandidateDir,
    path: &str,
    expected: &str,
    label: &str,
) -> Result<()> {
    let bytes = candidate.read_bytes(path)?;
    let actual = sha256_bytes(&bytes);
    if actual != expected {
        return Err(DeliveryError::new(format!("{label} digest mismatch")));
    }
    Ok(())
}

fn retained_evidence_digest(candidate: &super::storage::CandidateDir) -> Result<String> {
    let expected_root = ["local-host"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let actual_root = utf8_entries(
        candidate
            .list("evidence")
            .map_err(|_| DeliveryError::new("retained evidence directory is unreadable"))?,
        "retained evidence directory",
    )?;
    if actual_root != expected_root {
        return Err(DeliveryError::new(
            "retained evidence directory entry set differs from the accepted historical state",
        ));
    }

    let files = utf8_entries(
        candidate.list("evidence/local-host").map_err(|_| {
            DeliveryError::new("retained local-host evidence directory is unreadable")
        })?,
        "retained local-host evidence directory",
    )?;
    let mut manifest = Vec::with_capacity(files.len());
    for name in files {
        let relative = format!("evidence/local-host/{name}");
        let bytes = candidate
            .read_bytes(&relative)
            .map_err(|_| DeliveryError::new("retained evidence entry is unreadable"))?;
        manifest.push((format!("local-host/{name}"), sha256_bytes(&bytes)));
    }
    Ok(sha256_bytes(&serde_json::to_vec(&manifest)?))
}

fn utf8_entries(entries: Vec<std::ffi::OsString>, label: &str) -> Result<BTreeSet<String>> {
    entries
        .into_iter()
        .map(|entry| {
            entry
                .into_string()
                .map_err(|_| DeliveryError::new(format!("{label} contains a non-UTF-8 entry")))
        })
        .collect()
}

fn find_first_parent_amendment(
    root: &Path,
    repository_id: &str,
    predecessor: &str,
    base: &str,
    expected_constitution_sha256: &str,
) -> Result<String> {
    let range = format!("{predecessor}..{base}");
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-list", "--first-parent", "--reverse", &range])
        .output()
        .map_err(|error| {
            DeliveryError::environment(format!(
                "cannot enumerate the ADR-046 integration lineage: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(DeliveryError::environment(
            "cannot enumerate the ADR-046 integration lineage",
        ));
    }
    for commit in String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
    {
        let Some(constitution) = read_tree_object(root, repository_id, commit, CONSTITUTION_PATH)?
        else {
            continue;
        };
        if sha256_bytes(&constitution) == expected_constitution_sha256 {
            return Ok(commit.to_owned());
        }
    }
    Err(DeliveryError::new(
        "delivery base has no accepted historical disposition on first-parent integration history",
    ))
}

fn resolve_commit(root: &Path, revision: &str, label: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", revision])
        .output()
        .map_err(|_| DeliveryError::environment(format!("cannot resolve {label}")))?;
    if !output.status.success() {
        return Err(DeliveryError::environment(format!(
            "cannot resolve {label}"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn require_commit_ancestor(
    root: &Path,
    ancestor: &str,
    descendant: &str,
    label: &str,
) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .map_err(|error| {
            DeliveryError::environment(format!(
                "cannot verify ADR-046 Wave 5 merge ancestry: {error}"
            ))
        })?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(DeliveryError::new(format!(
            "delivery {label} does not descend from the accepted historical boundary"
        ))),
        _ => Err(DeliveryError::environment(format!(
            "cannot verify ADR-046 Wave 5 merge ancestry for {label}"
        ))),
    }
}

/// Rejects sealing a wave while any item in that wave remains Planned.
pub fn require_current_wave_merged(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let (graph, work_items) = load_bound_state(material, repository_roots)?;
    validate_state(&material.wave, Gate::Seal, &graph, &work_items)
}

#[derive(Clone, Copy)]
enum Gate {
    /// Wave exit boundary - panel request, seal, and merge eligibility:
    /// every prior wave must be `Merged`.
    Exit,
    /// Seal boundary: every item in this wave must be `Merged`.
    Seal,
}

fn require_disposed_work_item_projection(
    graph: &[u8],
    work_items: &[u8],
    disposed_wave: u8,
    expected_sha256: &str,
) -> Result<()> {
    let graph: serde_json::Value = serde_json::from_slice(graph)?;
    let work_items: serde_json::Value = serde_json::from_slice(work_items)?;
    let nodes = graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| DeliveryError::new("implementation graph has no nodes array"))?;
    let items = work_items
        .get("items")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| DeliveryError::new("work-item manifest has no items array"))?;

    let mut graph_items = BTreeMap::new();
    for node in nodes {
        if node.get("kind").and_then(serde_json::Value::as_str) != Some("work-item") {
            continue;
        }
        let id = node
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| DeliveryError::new("work-item graph node has no id"))?;
        if graph_items.insert(id.to_owned(), node.clone()).is_some() {
            return Err(DeliveryError::new(
                "implementation graph repeats a work item",
            ));
        }
    }

    let mut manifest_items = BTreeMap::new();
    for item in items {
        let id = item
            .get("workItemId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| DeliveryError::new("work-item manifest row has no workItemId"))?;
        if manifest_items.insert(id.to_owned(), item.clone()).is_some() {
            return Err(DeliveryError::new("work-item manifest repeats a work item"));
        }
    }
    if graph_items.keys().collect::<Vec<_>>() != manifest_items.keys().collect::<Vec<_>>() {
        return Err(DeliveryError::new(
            "implementation graph and work-item manifest are not bijective",
        ));
    }

    let mut graph_projection = Vec::new();
    let mut manifest_projection = Vec::new();
    for (id, node) in graph_items {
        let wave = node
            .get("wave")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| DeliveryError::new("work-item graph node has no wave"))?;
        if wave_number(wave)? != disposed_wave {
            continue;
        }
        graph_projection.push(node);
        manifest_projection.push(
            manifest_items
                .get(&id)
                .expect("graph/manifest bijection checked")
                .clone(),
        );
    }
    let projection = serde_json::json!({
        "graphNodes": graph_projection,
        "workItems": manifest_projection,
    });
    if sha256_bytes(&serde_json::to_vec(&projection)?) != expected_sha256 {
        return Err(DeliveryError::new(
            "disposed work-item projection differs from the accepted historical boundary",
        ));
    }
    Ok(())
}

fn validate_state(wave: &str, gate: Gate, graph: &[u8], work_items: &[u8]) -> Result<()> {
    validate_state_with_disposition(wave, gate, graph, work_items, None)
}

fn validate_state_with_disposition(
    wave: &str,
    gate: Gate,
    graph: &[u8],
    work_items: &[u8],
    disposed_wave: Option<u8>,
) -> Result<()> {
    let current_wave = wave_number(wave)?;
    let graph: GraphView = serde_json::from_slice(graph)?;
    let work_items: WorkItemsView = serde_json::from_slice(work_items)?;

    let mut states = BTreeMap::new();
    for item in work_items.items {
        if states
            .insert(item.work_item_id.clone(), item.implementation_state)
            .is_some()
        {
            return Err(DeliveryError::new(format!(
                "work-item state manifest repeats `{}`",
                item.work_item_id
            )));
        }
    }

    let mut checked = BTreeSet::new();
    for node in graph.nodes {
        if node.kind != "work-item" {
            continue;
        }
        let node_wave = wave_number(&node.wave)?;
        let in_scope = match gate {
            Gate::Exit => node_wave < current_wave && Some(node_wave) != disposed_wave,
            Gate::Seal => node_wave == current_wave,
        };
        if !in_scope {
            continue;
        }
        if !checked.insert(node.id.clone()) {
            return Err(DeliveryError::new(format!(
                "implementation graph repeats work item `{}`",
                node.id
            )));
        }
        let state = states.get(&node.id).ok_or_else(|| {
            DeliveryError::new(format!(
                "implementation graph work item `{}` is absent from the work-item state manifest",
                node.id
            ))
        })?;
        if state != "Merged" {
            let action = match gate {
                Gate::Exit => format!(
                    "cannot request a panel for, seal, or merge {wave}: prior-wave work item \
                     `{}` in {} is `{state}`",
                    node.id, node.wave
                ),
                Gate::Seal => format!("cannot seal {wave}: work item `{}` is `{state}`", node.id),
            };
            return Err(DeliveryError::new(format!(
                "{action}; set its Implementation state to Merged with exact evidence, \
                 regenerate the work-item manifest and implementation graph, and take a new snapshot"
            )));
        }
    }
    Ok(())
}

fn load_bound_state(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut found = Vec::new();
    for repository in &material.repository_set {
        let root = repository_roots.get(&repository.id).ok_or_else(|| {
            DeliveryError::new(format!(
                "sealed repository {} has no matching --repo checkout",
                repository.id
            ))
        })?;
        let graph = read_tree_file(root, repository, GRAPH_PATH)?;
        let work_items = read_tree_file(root, repository, WORK_ITEMS_PATH)?;
        match (graph, work_items) {
            (Some(graph), Some(work_items)) => found.push((graph, work_items)),
            (None, None) => {}
            (Some(_), None) => {
                return Err(DeliveryError::new(format!(
                    "repository {} contains {GRAPH_PATH} but not {WORK_ITEMS_PATH} in the sealed tree",
                    repository.id
                )));
            }
            (None, Some(_)) => {
                return Err(DeliveryError::new(format!(
                    "repository {} contains {WORK_ITEMS_PATH} but not {GRAPH_PATH} in the sealed tree",
                    repository.id
                )));
            }
        }
    }
    match found.len() {
        1 => Ok(found.pop().expect("one state source")),
        0 => Err(DeliveryError::new(format!(
            "no sealed repository contains both {GRAPH_PATH} and {WORK_ITEMS_PATH}; \
             the delivery work-item state gate cannot run"
        ))),
        count => Err(DeliveryError::new(format!(
            "{count} sealed repositories contain the delivery work-item state manifests; \
             expected exactly one authoritative source"
        ))),
    }
}

fn read_tree_file(
    root: &Path,
    repository: &RepositoryRecord,
    path: &str,
) -> Result<Option<Vec<u8>>> {
    read_tree_object(root, &repository.id, &repository.integration_tree_oid, path)
}

fn read_tree_object(
    root: &Path,
    repository_id: &str,
    oid: &str,
    path: &str,
) -> Result<Option<Vec<u8>>> {
    let object = format!("{oid}:{path}");
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &object])
        .output()
        .map_err(|error| {
            DeliveryError::environment(format!(
                "cannot read delivery work-item state from repository {repository_id}: {error}"
            ))
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    if output.stdout.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{path} in repository {repository_id} exceeds the delivery JSON size limit"
        )));
    }
    Ok(Some(output.stdout))
}

/// Parses the wave ordinal from either wave form.
///
/// Ordering across waves is what enforces "wave N+1 cannot open a panel request
/// until wave N has merged", so both the legacy bare `W<N>` form and the
/// qualified `<program>w<N>` form must yield the same ordinal. The two forms
/// are never compared against each other in practice, because a work-item graph
/// belongs to one program, but the ordinal is the only thing this function
/// promises either way.
fn wave_number(wave: &str) -> Result<u8> {
    if let Some((_, ordinal)) = qualified_wave_parts(wave) {
        return Ok(ordinal);
    }
    wave.strip_prefix('W')
        .and_then(|number| number.parse::<u8>().ok())
        .filter(|number| *number <= MAX_WAVE_ORDINAL)
        .ok_or_else(|| DeliveryError::new(format!("invalid delivery wave `{wave}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        snapshot::tests::GitFixture,
        storage::{CandidateDir, StateRoot},
    };
    use std::fs;

    /// Wave ordering is what enforces "wave N+1 waits for wave N to merge", so
    /// both wave forms must yield the same ordinal. The legacy arm is asserted
    /// explicitly because a program is running against it.
    #[test]
    fn both_wave_forms_yield_the_same_ordinal() {
        for ordinal in 0u8..=8 {
            assert_eq!(
                wave_number(&format!("W{ordinal}")).expect("a legacy wave parses"),
                ordinal
            );
            assert_eq!(
                wave_number(&format!("adr046w{ordinal}")).expect("a qualified wave parses"),
                ordinal
            );
            assert_eq!(
                wave_number(&format!("spec001w{ordinal}")).expect("a qualified wave parses"),
                ordinal
            );
        }
        for wave in ["W9", "W10", "w1", "alice", "", "spec001w9", "spec001w10"] {
            assert!(
                wave_number(wave).is_err(),
                "an out-of-range or name-like wave must not yield an ordinal: {wave:?}"
            );
        }
    }

    fn graph() -> Vec<u8> {
        br#"{
          "nodes": [
            {"id":"ADR046-foundation-001","kind":"work-item","wave":"W0"},
            {"id":"ADR046-backend-001","kind":"work-item","wave":"W1"}
          ]
        }"#
        .to_vec()
    }

    fn work_items(foundation: &str, backend: &str) -> Vec<u8> {
        format!(
            r#"{{
              "items": [
                {{"workItemId":"ADR046-foundation-001","implementationState":"{foundation}"}},
                {{"workItemId":"ADR046-backend-001","implementationState":"{backend}"}}
              ]
            }}"#
        )
        .into_bytes()
    }

    #[test]
    fn seal_attempt_rejects_a_planned_item_in_the_current_wave() {
        let error = validate_state(
            "W0",
            Gate::Seal,
            &graph(),
            &work_items("Planned", "Planned"),
        )
        .expect_err("a Planned current-wave item must block sealing");
        let message = error.message();
        assert!(message.contains("cannot seal W0"), "{message}");
        assert!(message.contains("ADR046-foundation-001"), "{message}");
        assert!(
            message.contains("Implementation state to Merged"),
            "{message}"
        );
    }

    #[test]
    fn planned_items_in_the_current_wave_are_allowed_at_the_prior_wave_gate() {
        validate_state("W1", Gate::Exit, &graph(), &work_items("Merged", "Planned"))
            .expect("the prior-wave gate checks prior waves, not work in this wave");
    }

    #[test]
    fn exit_boundary_rejects_a_planned_prior_wave_item() {
        let error = validate_state(
            "W1",
            Gate::Exit,
            &graph(),
            &work_items("Planned", "Planned"),
        )
        .expect_err("a Planned prior-wave item must block the wave exit boundary");
        let message = error.message();
        assert!(
            message.contains("cannot request a panel for, seal, or merge W1"),
            "{message}"
        );
        assert!(message.contains("ADR046-foundation-001"), "{message}");
        assert!(message.contains("in W0 is `Planned`"), "{message}");
    }

    /// FR-036/FR-048: implementation of a successor wave may start while a
    /// predecessor item is still `Planned`. The end-to-end proof that snapshot
    /// entry runs no prior-wave gate lives in
    /// `snapshot::tests::snapshot_entry_is_permitted_while_a_prior_wave_item_is_planned`.
    #[test]
    fn the_exit_gate_is_the_only_prior_wave_gate() {
        validate_state("W1", Gate::Seal, &graph(), &work_items("Planned", "Merged"))
            .expect("a Planned prior-wave item never constrains the successor's own wave scope");
    }

    /// FR-049: the successor may not seal until its predecessor is sealed and
    /// merged, i.e. every prior-wave item is `Merged`.
    #[test]
    fn seal_is_refused_while_the_predecessor_wave_is_unsealed() {
        let error = validate_state("W1", Gate::Exit, &graph(), &work_items("Planned", "Merged"))
            .expect_err("an unmerged predecessor wave must block the successor's seal");
        assert!(
            error
                .message()
                .contains("cannot request a panel for, seal, or merge W1"),
            "{}",
            error.message()
        );
    }

    /// FR-049 rebase freshness: the manifests this gate reads come from the
    /// snapshot's own `integration_tree_oid`, so a successor that has not
    /// rebased onto the integration lineage since the predecessor merged still
    /// presents the pre-merge manifest and is refused. Rebasing brings in the
    /// post-merge manifest and the same gate then passes.
    #[test]
    fn seal_is_refused_until_the_successor_rebases_past_the_predecessor_merge() {
        let stale_tree = work_items("Planned", "Merged");
        validate_state("W1", Gate::Exit, &graph(), &stale_tree)
            .expect_err("a pre-rebase tree still carries the predecessor's pre-merge manifest");

        let rebased_tree = work_items("Merged", "Merged");
        validate_state("W1", Gate::Exit, &graph(), &rebased_tree)
            .expect("after rebasing past the predecessor merge the wave exit gate passes");
    }

    #[test]
    fn accepted_historical_predecessor_satisfies_only_its_prior_wave_rows() {
        let graph = br#"{
          "nodes": [
            {"id":"ADR046-history-001","kind":"work-item","wave":"W5"},
            {"id":"ADR046-current-001","kind":"work-item","wave":"W6"}
          ]
        }"#;
        let work_items = br#"{
          "items": [
            {"workItemId":"ADR046-history-001","implementationState":"Planned"},
            {"workItemId":"ADR046-current-001","implementationState":"Merged"}
          ]
        }"#;

        validate_state_with_disposition("spec001w6", Gate::Exit, graph, work_items, Some(5))
            .expect("the validated historical predecessor satisfies its exact prior rows");
        validate_state("spec001w6", Gate::Exit, graph, work_items)
            .expect_err("without the disposition the Planned predecessor remains blocking");

        let later_graph = br#"{
          "nodes": [
            {"id":"ADR046-history-001","kind":"work-item","wave":"W5"},
            {"id":"ADR046-current-001","kind":"work-item","wave":"W6"},
            {"id":"ADR046-next-001","kind":"work-item","wave":"W7"}
          ]
        }"#;
        let later_items = br#"{
          "items": [
            {"workItemId":"ADR046-history-001","implementationState":"Planned"},
            {"workItemId":"ADR046-current-001","implementationState":"Merged"},
            {"workItemId":"ADR046-next-001","implementationState":"Merged"}
          ]
        }"#;
        validate_state_with_disposition("spec001w8", Gate::Exit, later_graph, later_items, Some(5))
            .expect("later phases retain the exact historical predecessor disposition");

        let unmerged_successor = br#"{
          "items": [
            {"workItemId":"ADR046-history-001","implementationState":"Planned"},
            {"workItemId":"ADR046-current-001","implementationState":"Planned"},
            {"workItemId":"ADR046-next-001","implementationState":"Merged"}
          ]
        }"#;
        validate_state_with_disposition(
            "spec001w8",
            Gate::Exit,
            later_graph,
            unmerged_successor,
            Some(5),
        )
        .expect_err("the disposition must not waive later unmerged work");
    }

    #[test]
    fn immutable_historical_phase_rejects_all_supported_addresses() {
        for (program, wave) in [
            ("ADR046", "W5"),
            ("ADR046", "adr046w5"),
            ("adr046", "adr046w5"),
            ("SPEC001", "spec001w5"),
            ("spec001", "spec001w5"),
        ] {
            let mut material = crate::delivery::model::fixtures::material();
            program.clone_into(&mut material.program);
            wave.clone_into(&mut material.wave);
            let error = reject_adr046_w5_mutation(&material, "snapshot")
                .expect_err("historical delivery mutation must be refused");
            assert!(
                error
                    .message()
                    .contains("immutable historical delivery state"),
                "{program}/{wave}: {error}"
            );
        }

        let material = crate::delivery::model::fixtures::material();
        reject_adr046_w5_mutation(&material, "snapshot")
            .expect("unrelated delivery state remains mutable");
    }

    #[test]
    fn historical_mutation_guard_is_wired_to_every_mutating_delivery_command() {
        for (source, exact_call, publication) in [
            (
                include_str!("snapshot.rs"),
                "reject_adr046_w5_mutation(&material,\"snapshot\")",
                "letcandidate=root.candidate(",
            ),
            (
                include_str!("evidence.rs"),
                "reject_adr046_w5_mutation(&supplied.material,\"evidenceimport\")",
                "letcandidate=root.existing_candidate(",
            ),
            (
                include_str!("panel.rs"),
                "reject_adr046_w5_mutation(&snapshot.material,\"panelrequest\")",
                "matchselection_path{",
            ),
            (
                include_str!("panel.rs"),
                "reject_adr046_w5_mutation(&snapshot.material,\"panelattestation\")",
                "attest(&candidate,&snapshot,&records_dir)",
            ),
            (
                include_str!("seal.rs"),
                "reject_adr046_w5_mutation(&snapshot.material,\"seal\")",
                "seal_checked(&state,&candidate,&snapshot,&repository_roots)",
            ),
            (
                include_str!("eligibility.rs"),
                "reject_adr046_w5_mutation(&seal.material,\"mergetargetcapture\")",
                "capture(&candidate,target)",
            ),
            (
                include_str!("eligibility.rs"),
                "reject_adr046_w5_mutation(&seal.material,\"mergeeligibility\")",
                "evaluate_checked(&state,&candidate,&seal,&target,&repository_roots)",
            ),
        ] {
            let compact = source.split_whitespace().collect::<String>();
            let guard = compact
                .find(exact_call)
                .unwrap_or_else(|| panic!("missing exact historical mutation guard {exact_call}"));
            let publish = compact[guard..]
                .find(publication)
                .map(|offset| guard + offset)
                .unwrap_or_else(|| panic!("missing publication marker {publication}"));
            assert!(
                guard < publish,
                "historical mutation guard {exact_call} must precede {publication}"
            );
        }
    }

    const RETAINED_REQUEST_BYTES: &[u8] = br#"{"roles":["software","test"]}"#;
    const RETAINED_SNAPSHOT_BYTES: &[u8] = br#"{"snapshot":"retained"}"#;
    const TEST_RETAINED_EVIDENCE: [(&str, &str, &[u8]); 2] = [
        (
            "one.json",
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
            b"{}",
        ),
        (
            "two.json",
            "4062edaf750fb8074e7e83e0c9028c94e32468a8b6f1614774328ef045150f93",
            br#"{"ok":true}"#,
        ),
    ];

    struct HistoricalFixture {
        repository: GitFixture,
        state: StateRoot,
        material: CandidateMaterial,
        roots: BTreeMap<String, PathBuf>,
        retained_candidate_id: String,
        request_sha256: String,
        snapshot_sha256: String,
        predecessor_merge_oid: String,
        constitution_sha256: String,
        evidence_sha256: String,
    }

    impl HistoricalFixture {
        fn new(label: &str) -> Self {
            let repository = GitFixture::new(label);
            let predecessor_merge_oid = repository.head();
            repository.write(CONSTITUTION_PATH, "constitution 3.1.0\n");
            repository.commit("constitution amendment");
            let current_oid = repository.head();
            repository.git(&["update-ref", D2B_INTEGRATION_REF, &current_oid]);
            let integration_tree_oid = git_rev_parse(&repository, "HEAD^{tree}");

            let mut material = crate::delivery::model::fixtures::material();
            "SPEC001".clone_into(&mut material.program);
            "spec001w6".clone_into(&mut material.wave);
            let record = &mut material.repository_set[0];
            D2B_REPOSITORY_ID.clone_into(&mut record.id);
            current_oid.clone_into(&mut record.base_oid);
            current_oid.clone_into(&mut record.head_oid);
            integration_tree_oid.clone_into(&mut record.integration_tree_oid);
            current_oid.clone_into(&mut record.expected_pull_requests[0].head_oid);

            let roots = BTreeMap::from([(D2B_REPOSITORY_ID.to_owned(), repository.repo())]);
            let state = StateRoot::for_tests(&repository.state()).expect("state root");
            let current_id = CandidateId::parse("a".repeat(64)).expect("current candidate id");
            state
                .candidate("spec001w6", &current_id)
                .expect("current candidate");

            let retained_candidate_id = "b".repeat(64);
            let retained_id =
                CandidateId::parse(&retained_candidate_id).expect("retained candidate id");
            let retained = state
                .candidate(ADR046_W5_RETAINED_WAVE, &retained_id)
                .expect("retained candidate");
            retained
                .write_bytes(PANEL_REQUEST_FILE, RETAINED_REQUEST_BYTES)
                .expect("retained request");
            retained
                .write_bytes(SNAPSHOT_FILE, RETAINED_SNAPSHOT_BYTES)
                .expect("retained snapshot");
            for (name, _, bytes) in TEST_RETAINED_EVIDENCE {
                retained
                    .write_bytes(format!("evidence/local-host/{name}"), bytes)
                    .expect("retained evidence");
            }
            let evidence_sha256 =
                retained_evidence_digest(&retained).expect("retained evidence digest");

            Self {
                repository,
                state,
                material,
                roots,
                retained_candidate_id,
                request_sha256: sha256_bytes(RETAINED_REQUEST_BYTES),
                snapshot_sha256: sha256_bytes(RETAINED_SNAPSHOT_BYTES),
                predecessor_merge_oid,
                constitution_sha256: sha256_bytes(b"constitution 3.1.0\n"),
                evidence_sha256,
            }
        }

        fn policy(&self) -> HistoricalPredecessorPolicy<'_> {
            HistoricalPredecessorPolicy {
                repository_id: D2B_REPOSITORY_ID,
                retained_wave: ADR046_W5_RETAINED_WAVE,
                retained_candidate_id: &self.retained_candidate_id,
                retained_request_sha256: &self.request_sha256,
                retained_snapshot_file_sha256: &self.snapshot_sha256,
                predecessor_merge_oid: &self.predecessor_merge_oid,
                constitution_sha256: &self.constitution_sha256,
                retained_evidence_sha256: &self.evidence_sha256,
                integration_ref: D2B_INTEGRATION_REF,
            }
        }

        fn retained(&self) -> CandidateDir {
            self.state
                .existing_candidate(
                    ADR046_W5_RETAINED_WAVE,
                    &CandidateId::parse(&self.retained_candidate_id)
                        .expect("retained candidate id"),
                )
                .expect("retained candidate")
        }

        fn validate(&self, policy: &HistoricalPredecessorPolicy<'_>) -> Result<()> {
            validate_historical_predecessor(policy, &self.state, &self.material, &self.roots, true)
        }
    }

    fn git_rev_parse(repository: &GitFixture, revision: &str) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository.repo())
            .args(["rev-parse", revision])
            .output()
            .expect("git rev-parse");
        assert!(
            output.status.success(),
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    fn adr046_w6_historical_predecessor_accepts_only_the_exact_state() {
        let fixture = HistoricalFixture::new("historical-predecessor-positive");
        fixture
            .validate(&fixture.policy())
            .expect("the exact retained state passes");

        for wave in ["spec001w6", "spec001w7", "spec001w8"] {
            let mut material = fixture.material.clone();
            wave.clone_into(&mut material.wave);
            assert!(is_adr046_post_w5(&material));
            validate_historical_predecessor(
                &fixture.policy(),
                &fixture.state,
                &material,
                &fixture.roots,
                false,
            )
            .expect("the accepted historical predecessor carries through later phases");
        }

        let mut lowercase_spec = fixture.material.clone();
        "spec001".clone_into(&mut lowercase_spec.program);
        assert!(is_adr046_post_w5(&lowercase_spec));

        let mut lowercase_adr = fixture.material.clone();
        "adr046".clone_into(&mut lowercase_adr.program);
        "adr046w6".clone_into(&mut lowercase_adr.wave);
        assert!(is_adr046_post_w5(&lowercase_adr));
    }

    #[test]
    fn adr046_w6_historical_predecessor_rejects_wrong_candidate_and_artifacts() {
        let fixture = HistoricalFixture::new("historical-predecessor-artifacts");

        let missing_candidate = "c".repeat(64);
        let mut policy = fixture.policy();
        policy.retained_candidate_id = &missing_candidate;
        let error = fixture
            .validate(&policy)
            .expect_err("a different retained candidate must fail");
        assert!(
            error.message().contains("retained candidate is missing"),
            "{error}"
        );

        let retained = fixture.retained();
        fs::write(retained.path().join(PANEL_REQUEST_FILE), b"wrong roster")
            .expect("replace request");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("changed request bytes must fail");
        assert!(
            error.message().contains("request digest mismatch"),
            "{error}"
        );
        fs::write(
            retained.path().join(PANEL_REQUEST_FILE),
            RETAINED_REQUEST_BYTES,
        )
        .expect("restore request");

        fs::write(retained.path().join(SNAPSHOT_FILE), b"wrong head").expect("replace snapshot");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("changed snapshot bytes must fail");
        assert!(
            error.message().contains("snapshot digest mismatch"),
            "{error}"
        );
        fs::write(retained.path().join(SNAPSHOT_FILE), RETAINED_SNAPSHOT_BYTES)
            .expect("restore snapshot");

        let evidence = retained.path().join("evidence/local-host/one.json");
        fs::write(&evidence, b"changed evidence").expect("replace evidence");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("changed evidence bytes must fail");
        assert!(
            error.message().contains("evidence tree digest mismatch"),
            "{error}"
        );
        fs::write(&evidence, b"{}").expect("restore evidence");

        fs::remove_file(&evidence).expect("remove evidence");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("missing evidence must fail");
        assert!(
            error.message().contains("evidence tree digest mismatch"),
            "{error}"
        );
        fs::write(&evidence, b"{}").expect("restore evidence");

        fs::write(
            retained.path().join("evidence/local-host/extra.json"),
            b"{}",
        )
        .expect("add evidence");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("additional evidence must fail");
        assert!(
            error.message().contains("evidence tree digest mismatch"),
            "{error}"
        );
        fs::remove_file(retained.path().join("evidence/local-host/extra.json"))
            .expect("remove extra evidence");

        fs::write(retained.path().join("seal.json"), b"{}").expect("plant seal");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("an added seal must fail");
        assert!(error.message().contains("entry set differs"), "{error}");
        fs::remove_file(retained.path().join("seal.json")).expect("remove seal");

        fs::create_dir(retained.path().join("panel")).expect("plant panel directory");
        fs::write(
            retained.path().join("panel/software.json"),
            b"non-unanimous",
        )
        .expect("plant verdict");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("added panel state must fail");
        assert!(error.message().contains("entry set differs"), "{error}");
    }

    #[test]
    fn adr046_w6_historical_predecessor_rejects_wrong_ancestry_and_constitution() {
        let fixture = HistoricalFixture::new("historical-predecessor-lineage");

        fixture.repository.write("later.txt", "later\n");
        fixture.repository.commit("later non-ancestor");
        let later = fixture.repository.head();
        let mut policy = fixture.policy();
        policy.predecessor_merge_oid = &later;
        let error = fixture
            .validate(&policy)
            .expect_err("a non-ancestor boundary must fail");
        assert!(error.message().contains("does not descend"), "{error}");

        let wrong_constitution = sha256_bytes(b"wrong constitution");
        let mut policy = fixture.policy();
        policy.constitution_sha256 = &wrong_constitution;
        let error = fixture
            .validate(&policy)
            .expect_err("wrong constitution bytes must fail");
        assert!(
            error
                .message()
                .contains("no accepted historical disposition"),
            "{error}"
        );
    }

    #[test]
    fn adr046_w6_historical_predecessor_rejects_a_non_integration_base() {
        let fixture = HistoricalFixture::new("historical-predecessor-non-integration");
        fixture.repository.git(&[
            "checkout",
            "--quiet",
            "-b",
            "sibling",
            &fixture.predecessor_merge_oid,
        ]);
        fixture
            .repository
            .write(CONSTITUTION_PATH, "constitution 3.1.0\n");
        fixture.repository.commit("copied amendment bytes");
        let sibling = fixture.repository.head();
        let mut material = fixture.material.clone();
        sibling.clone_into(&mut material.repository_set[0].base_oid);
        sibling.clone_into(&mut material.repository_set[0].head_oid);
        material.repository_set[0].integration_tree_oid =
            git_rev_parse(&fixture.repository, "HEAD^{tree}");
        sibling.clone_into(&mut material.repository_set[0].expected_pull_requests[0].head_oid);
        let error = validate_historical_predecessor(
            &fixture.policy(),
            &fixture.state,
            &material,
            &fixture.roots,
            true,
        )
        .expect_err("copied bytes outside the integration tip must fail");
        assert!(
            error.message().contains("not the fetched integration tip"),
            "{error}"
        );
    }

    #[test]
    fn later_constitution_amendments_do_not_invalidate_the_accepted_predecessor() {
        let fixture = HistoricalFixture::new("historical-predecessor-later-constitution");
        fixture
            .repository
            .write(CONSTITUTION_PATH, "constitution 3.2.0\n");
        fixture.repository.commit("later constitution amendment");
        let later = fixture.repository.head();
        fixture
            .repository
            .git(&["update-ref", D2B_INTEGRATION_REF, &later]);
        let mut material = fixture.material.clone();
        later.clone_into(&mut material.repository_set[0].base_oid);
        later.clone_into(&mut material.repository_set[0].head_oid);
        material.repository_set[0].integration_tree_oid =
            git_rev_parse(&fixture.repository, "HEAD^{tree}");
        later.clone_into(&mut material.repository_set[0].expected_pull_requests[0].head_oid);
        validate_historical_predecessor(
            &fixture.policy(),
            &fixture.state,
            &material,
            &fixture.roots,
            true,
        )
        .expect("the first-parent history still contains the accepted amendment");
    }

    #[test]
    fn exit_rechecks_allow_the_integration_tip_to_advance_after_entry() {
        let fixture = HistoricalFixture::new("historical-predecessor-post-merge");
        fixture.repository.write("merged.txt", "merged\n");
        fixture.repository.commit("later integration tip");
        let later = fixture.repository.head();
        fixture
            .repository
            .git(&["update-ref", D2B_INTEGRATION_REF, &later]);

        validate_historical_predecessor(
            &fixture.policy(),
            &fixture.state,
            &fixture.material,
            &fixture.roots,
            false,
        )
        .expect("exit rechecks use the immutable entry base after the integration tip advances");
        validate_historical_predecessor(
            &fixture.policy(),
            &fixture.state,
            &fixture.material,
            &fixture.roots,
            true,
        )
        .expect_err("a new entry must use the advanced integration tip");
    }

    #[cfg(unix)]
    #[test]
    fn retained_evidence_read_failures_use_a_closed_diagnostic() {
        use std::os::unix::fs::symlink;

        let fixture = HistoricalFixture::new("historical-predecessor-diagnostic");
        let retained = fixture.retained();
        let evidence = retained.path().join("evidence/local-host/one.json");
        fs::remove_file(&evidence).expect("remove evidence");
        symlink("/definitely-not-readable", &evidence).expect("plant symlink");

        let error = fixture
            .validate(&fixture.policy())
            .expect_err("symlinked retained evidence must fail");
        assert_eq!(error.message(), "retained evidence entry is unreadable");
        assert!(!error.message().contains("one.json"), "{error}");
        assert!(!error.message().contains('/'), "{error}");
    }

    #[test]
    fn non_adr046_w6_candidates_do_not_consume_the_historical_exception() {
        let fixture = HistoricalFixture::new("historical-predecessor-scope");
        let mut material = fixture.material.clone();
        "spec001w5".clone_into(&mut material.wave);
        require_adr046_historical_predecessor_for_exit(&fixture.state, &material, &BTreeMap::new())
            .expect("another wave does not consume the one-time W6 exception");
    }

    #[test]
    fn committed_disposed_work_item_projection_is_exact() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/");
        let graph = fs::read(root.join(GRAPH_PATH)).expect("read graph");
        let work_items = fs::read(root.join(WORK_ITEMS_PATH)).expect("read work items");
        require_disposed_work_item_projection(
            &graph,
            &work_items,
            5,
            ADR046_W5_WORK_ITEM_PROJECTION_SHA256,
        )
        .expect("the accepted historical work-item projection matches");

        let mut graph_json: serde_json::Value =
            serde_json::from_slice(&graph).expect("parse graph");
        graph_json["nodes"]
            .as_array_mut()
            .expect("nodes")
            .push(serde_json::json!({
                "id": "ADR046-late-added-001",
                "kind": "work-item",
                "wave": "W5"
            }));
        let mut work_items_json: serde_json::Value =
            serde_json::from_slice(&work_items).expect("parse work items");
        work_items_json["items"]
            .as_array_mut()
            .expect("items")
            .push(serde_json::json!({
                "workItemId": "ADR046-late-added-001",
                "implementationState": "Planned"
            }));
        require_disposed_work_item_projection(
            &serde_json::to_vec(&graph_json).expect("graph bytes"),
            &serde_json::to_vec(&work_items_json).expect("work-item bytes"),
            5,
            ADR046_W5_WORK_ITEM_PROJECTION_SHA256,
        )
        .expect_err("an added historical row must invalidate the disposition");

        let graph_json: serde_json::Value = serde_json::from_slice(&graph).expect("parse graph");
        let first_id = graph_json["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find(|node| node["kind"] == "work-item" && node["wave"] == "W5")
            .and_then(|node| node["id"].as_str())
            .expect("historical work item");
        let mut changed_field: serde_json::Value =
            serde_json::from_slice(&work_items).expect("parse work items");
        let row = changed_field["items"]
            .as_array_mut()
            .expect("items")
            .iter_mut()
            .find(|item| item["workItemId"] == first_id)
            .expect("historical manifest row");
        row["detailedDesign"] = serde_json::Value::String("changed design".to_owned());
        require_disposed_work_item_projection(
            &graph,
            &serde_json::to_vec(&changed_field).expect("work-item bytes"),
            5,
            ADR046_W5_WORK_ITEM_PROJECTION_SHA256,
        )
        .expect_err("a changed historical field must invalidate the disposition");

        let mut manifest_only: serde_json::Value =
            serde_json::from_slice(&work_items).expect("parse work items");
        manifest_only["items"]
            .as_array_mut()
            .expect("items")
            .push(serde_json::json!({
                "workItemId": "ADR046-manifest-only-001",
                "implementationState": "Planned"
            }));
        require_disposed_work_item_projection(
            &graph,
            &serde_json::to_vec(&manifest_only).expect("work-item bytes"),
            5,
            ADR046_W5_WORK_ITEM_PROJECTION_SHA256,
        )
        .expect_err("a manifest-only work item must invalidate the disposition");
    }

    #[test]
    fn committed_first_delivery_wave_contains_exactly_the_shipped_items_and_seals() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/")
            .to_path_buf();
        let graph = std::fs::read(root.join(GRAPH_PATH)).expect("read implementation graph");
        let work_items = std::fs::read(root.join(WORK_ITEMS_PATH)).expect("read work-item states");

        validate_state("W1", Gate::Seal, &graph, &work_items)
            .expect("the committed first delivery wave must pass the seal membership gate");

        let graph: GraphView = serde_json::from_slice(&graph).expect("parse implementation graph");
        let actual = graph
            .nodes
            .into_iter()
            .filter(|node| node.kind == "work-item" && node.wave == "W1")
            .map(|node| node.id)
            .collect::<BTreeSet<_>>();
        let expected = [
            "ADR046-bus-001",
            "ADR046-feasibility-001",
            "ADR046-reconcile-001",
            "ADR046-reconcile-002",
            "ADR046-session-001",
            "ADR046-session-002",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }
}
