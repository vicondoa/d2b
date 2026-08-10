//! Work-item state gates for wave sealing and the wave exit boundary.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
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
    storage::{CandidateDir, MAX_JSON_BYTES, PANEL_REQUEST_FILE, SNAPSHOT_FILE},
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
    "82ba409d89ec7b4b41d74cd87b0f52b5dcc4c8c6a110ea1a0fcb3b0f1f24b02e";
const D2B_REPOSITORY_ID: &str = "github.com/vicondoa/d2b";

struct HistoricalPredecessorPolicy<'a> {
    repository_id: &'a str,
    retained_wave: &'a str,
    retained_candidate_id: &'a str,
    retained_request_sha256: &'a str,
    retained_snapshot_file_sha256: &'a str,
    predecessor_merge_oid: &'a str,
    constitution_sha256: &'a str,
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
/// state, proves the Wave 6 base and head descend from the merged Wave 5
/// boundary, and pins the amended constitution bytes at the candidate base.
pub fn require_adr046_w6_historical_predecessor_for_exit(
    candidate: &CandidateDir,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    if !is_adr046_w6(material) {
        return Ok(());
    }
    validate_historical_predecessor(
        &ADR046_W6_HISTORICAL_POLICY,
        candidate,
        material,
        repository_roots,
    )
}

fn is_adr046_w6(material: &CandidateMaterial) -> bool {
    matches!(
        (material.program.as_str(), material.wave.as_str()),
        ("ADR046", "W6") | ("ADR046", "adr046w6") | ("SPEC001", "spec001w6")
    )
}

fn validate_historical_predecessor(
    policy: &HistoricalPredecessorPolicy<'_>,
    candidate: &CandidateDir,
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
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

    for (label, descendant) in [
        ("base commit", repository.base_oid.as_str()),
        ("head commit", repository.head_oid.as_str()),
    ] {
        require_commit_ancestor(root, policy.predecessor_merge_oid, descendant, label)?;
    }

    let constitution = read_tree_object(
        root,
        policy.repository_id,
        &repository.base_oid,
        CONSTITUTION_PATH,
    )?
    .ok_or_else(|| {
        DeliveryError::new(format!(
            "ADR-046 Wave 6 base commit does not contain {CONSTITUTION_PATH}"
        ))
    })?;
    let constitution_sha256 = sha256_bytes(&constitution);
    if constitution_sha256 != policy.constitution_sha256 {
        return Err(DeliveryError::new(format!(
            "ADR-046 Wave 6 base commit has the wrong FR-036 constitution bytes: expected {}, got {constitution_sha256}",
            policy.constitution_sha256
        )));
    }

    let state_root = candidate
        .path()
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| DeliveryError::new("candidate path has no delivery state root"))?;
    let retained = state_root
        .join(policy.retained_wave)
        .join(policy.retained_candidate_id);
    let metadata = fs::symlink_metadata(&retained).map_err(|error| {
        DeliveryError::new(format!(
            "ADR-046 Wave 5 retained candidate is missing: {error}"
        ))
    })?;
    if !metadata.file_type().is_dir() {
        return Err(DeliveryError::new(
            "ADR-046 Wave 5 retained candidate is not a real directory",
        ));
    }

    let mut entries = BTreeSet::new();
    for entry in fs::read_dir(&retained).map_err(|error| {
        DeliveryError::new(format!(
            "cannot enumerate ADR-046 Wave 5 retained candidate: {error}"
        ))
    })? {
        let name = entry
            .map_err(|error| {
                DeliveryError::new(format!(
                    "cannot enumerate ADR-046 Wave 5 retained candidate: {error}"
                ))
            })?
            .file_name()
            .into_string()
            .map_err(|_| {
                DeliveryError::new("ADR-046 Wave 5 retained candidate contains a non-UTF-8 entry")
            })?;
        entries.insert(name);
    }
    let expected = ["evidence", PANEL_REQUEST_FILE, SNAPSHOT_FILE]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if entries != expected {
        return Err(DeliveryError::new(format!(
            "ADR-046 Wave 5 retained candidate entries differ from the accepted historical state: expected {expected:?}, got {entries:?}"
        )));
    }

    require_file_digest(
        &retained.join(PANEL_REQUEST_FILE),
        policy.retained_request_sha256,
        "retained panel request",
    )?;
    require_file_digest(
        &retained.join(SNAPSHOT_FILE),
        policy.retained_snapshot_file_sha256,
        "retained snapshot",
    )
}

fn require_file_digest(path: &Path, expected: &str, label: &str) -> Result<()> {
    let bytes = fs::read(path)
        .map_err(|error| DeliveryError::new(format!("cannot read {label}: {error}")))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} exceeds the delivery JSON size limit"
        )));
    }
    let actual = sha256_bytes(&bytes);
    if actual != expected {
        return Err(DeliveryError::new(format!(
            "{label} digest mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
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
            "ADR-046 Wave 6 {label} does not descend from merged Wave 5 boundary {ancestor}"
        ))),
        _ => Err(DeliveryError::environment(format!(
            "cannot verify ADR-046 Wave 5 merge ancestry: {}",
            String::from_utf8_lossy(&output.stderr).trim()
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

fn validate_state(wave: &str, gate: Gate, graph: &[u8], work_items: &[u8]) -> Result<()> {
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
            Gate::Exit => node_wave < current_wave,
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
    use crate::delivery::{snapshot::tests::GitFixture, storage::StateRoot};

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

    const RETAINED_REQUEST_BYTES: &[u8] = br#"{"roles":["software","test"]}"#;
    const RETAINED_SNAPSHOT_BYTES: &[u8] = br#"{"snapshot":"retained"}"#;

    struct HistoricalFixture {
        repository: GitFixture,
        state: StateRoot,
        current: CandidateDir,
        material: CandidateMaterial,
        roots: BTreeMap<String, PathBuf>,
        retained_candidate_id: String,
        request_sha256: String,
        snapshot_sha256: String,
        predecessor_merge_oid: String,
        constitution_sha256: String,
    }

    impl HistoricalFixture {
        fn new(label: &str) -> Self {
            let repository = GitFixture::new(label);
            let predecessor_merge_oid = repository.head();
            repository.write(CONSTITUTION_PATH, "constitution 3.1.0\n");
            repository.commit("constitution amendment");
            let current_oid = repository.head();
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
            let current_id = CandidateId::parse(&"a".repeat(64)).expect("current candidate id");
            let current = state
                .candidate("spec001w6", &current_id)
                .expect("current candidate");

            let retained_candidate_id = "b".repeat(64);
            let retained_id =
                CandidateId::parse(&retained_candidate_id).expect("retained candidate id");
            let retained = state
                .candidate(ADR046_W5_RETAINED_WAVE, &retained_id)
                .expect("retained candidate");
            fs::create_dir(retained.path().join("evidence")).expect("retained evidence directory");
            retained
                .write_bytes(PANEL_REQUEST_FILE, RETAINED_REQUEST_BYTES)
                .expect("retained request");
            retained
                .write_bytes(SNAPSHOT_FILE, RETAINED_SNAPSHOT_BYTES)
                .expect("retained snapshot");

            Self {
                repository,
                state,
                current,
                material,
                roots,
                retained_candidate_id,
                request_sha256: sha256_bytes(RETAINED_REQUEST_BYTES),
                snapshot_sha256: sha256_bytes(RETAINED_SNAPSHOT_BYTES),
                predecessor_merge_oid,
                constitution_sha256: sha256_bytes(b"constitution 3.1.0\n"),
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
            validate_historical_predecessor(policy, &self.current, &self.material, &self.roots)
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

        fs::write(retained.path().join("seal.json"), b"{}").expect("plant seal");
        let error = fixture
            .validate(&fixture.policy())
            .expect_err("an added seal must fail");
        assert!(error.message().contains("entries differ"), "{error}");
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
        assert!(error.message().contains("entries differ"), "{error}");
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
            error.message().contains("wrong FR-036 constitution bytes"),
            "{error}"
        );
    }

    #[test]
    fn non_adr046_w6_candidates_do_not_consume_the_historical_exception() {
        let fixture = HistoricalFixture::new("historical-predecessor-scope");
        let mut material = fixture.material.clone();
        "spec001w5".clone_into(&mut material.wave);
        require_adr046_w6_historical_predecessor_for_exit(
            &fixture.current,
            &material,
            &BTreeMap::new(),
        )
        .expect("another wave does not consume the one-time W6 exception");
    }

    #[test]
    fn committed_constitution_matches_the_historical_predecessor_policy() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/");
        let constitution = fs::read(root.join(CONSTITUTION_PATH)).expect("read constitution");
        assert_eq!(
            sha256_bytes(&constitution),
            ADR046_FR036_CONSTITUTION_SHA256
        );
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
