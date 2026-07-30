//! Work-item state gates for wave sealing and the wave exit boundary.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use super::{
    DeliveryError, Result,
    model::{CandidateMaterial, RepositoryRecord},
    storage::MAX_JSON_BYTES,
};

const GRAPH_PATH: &str = "docs/specs/ADR-046-implementation-graph.json";
const WORK_ITEMS_PATH: &str = "docs/specs/ADR-046-work-items.json";

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
    let object = format!("{}:{path}", repository.integration_tree_oid);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &object])
        .output()
        .map_err(|error| {
            DeliveryError::environment(format!(
                "cannot read delivery work-item state from repository {}: {error}",
                repository.id
            ))
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    if output.stdout.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{path} in repository {} exceeds the delivery JSON size limit",
            repository.id
        )));
    }
    Ok(Some(output.stdout))
}

fn wave_number(wave: &str) -> Result<u8> {
    wave.strip_prefix('W')
        .and_then(|number| number.parse::<u8>().ok())
        .filter(|number| *number <= 8)
        .ok_or_else(|| DeliveryError::new(format!("invalid delivery wave `{wave}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

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
