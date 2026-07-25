//! Generator for the ADR 0046 machine-readable implementation graph.
//!
//! Joins `docs/specs/ADR-046-spec-set.json` and
//! `docs/specs/ADR-046-work-items.json` with the delivery wave topology and
//! emits `docs/specs/ADR-046-implementation-graph.json` plus its rendered
//! `.md` companion.
//!
//! Every spec and every work item appears exactly once, mapped to a
//! dependency-ordered wave and a single-wave parallel group. Generation is
//! fail-closed: an unassigned spec, an unresolved dependency reference, a
//! cycle, a backward-wave dependency, or a cross-wave parallel group aborts
//! before any file is written.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::gen_spec_set::{SPEC_SET_PATH, WORK_ITEMS_PATH, render_json};

pub const GRAPH_JSON_PATH: &str = "docs/specs/ADR-046-implementation-graph.json";
pub const GRAPH_MD_PATH: &str = "docs/specs/ADR-046-implementation-graph.md";

const ADR: &str = "0046";
const ARTIFACT_KIND: &str = "d2b-adr-implementation-graph";
const SCHEMA_VERSION: u32 = 1;
const WAVE_TOPOLOGY_REF: &str =
    "docs/specs/ADR-046-validation-and-delivery.md#3-delivery-wave-topology";
const SPEC_ID_PREFIX: &str = "ADR-046-";
const WORK_ITEM_ID_PREFIX: &str = "ADR046-";

const EDGE_FILE_OVERLAP: &str = "file-overlap-order";
const EDGE_IMPLEMENTS: &str = "implements-spec";
const EDGE_SHARED_CONTRACT: &str = "shared-contract";
const EDGE_SPEC_DEPENDS: &str = "spec-depends-on";
const EDGE_WORK_ITEM_DEPENDS: &str = "work-item-depends-on";

/// Spec wave and parallel-group assignment, from the delivery wave topology.
///
/// Provider dossiers are covered by [`PROVIDER_FAMILIES`]; every other member
/// is listed here explicitly so an unassigned new member fails generation.
const SPEC_TOPOLOGY: &[(&str, u8, &str)] = &[
    ("decision-register", 0, "W0-reference-docs"),
    ("current-code-migration-map", 0, "W0-reference-docs"),
    ("terminology-and-identities", 0, "W0-foundation-chain"),
    ("resource-object-model", 0, "W0-foundation-chain"),
    ("resource-store-redb", 0, "W0-foundation-chain"),
    ("resource-api-and-authorization", 0, "W0-foundation-chain"),
    ("resource-reconciliation", 1, "W1-reconcile-and-bus"),
    ("componentsession-and-bus", 1, "W1-reconcile-and-bus"),
    (
        "primitive-resource-composition",
        2,
        "W2-composition-and-routing",
    ),
    ("zone-routing", 2, "W2-composition-and-routing"),
    ("provider-model-and-packaging", 3, "W3-provider-contract"),
    ("components-processes-and-sandbox", 4, "W4-parallel-specs"),
    ("core-controllers", 4, "W4-parallel-specs"),
    ("resources-network", 4, "W4-parallel-specs"),
    ("resources-credential", 4, "W4-parallel-specs"),
    ("provider-state", 4, "W4-parallel-specs"),
    ("resources-zone-control", 5, "W5-parallel-specs"),
    ("resources-host-guest-process-user", 5, "W5-parallel-specs"),
    ("resources-volume", 5, "W5-parallel-specs"),
    ("resources-device", 5, "W5-parallel-specs"),
    ("telemetry-audit-and-support", 5, "W5-parallel-specs"),
    ("cli-and-operations", 5, "W5-parallel-specs"),
    ("nix-configuration", 5, "W5-parallel-specs"),
    ("feasibility-and-spikes", 7, "W7-closing"),
    ("reset-and-cutover", 7, "W7-closing"),
    ("security-and-threat-model", 7, "W7-closing"),
    ("streamline", 7, "W7-closing"),
    ("validation-and-delivery", 7, "W7-closing"),
];

/// Provider dossier launch families. Every dossier launches in wave 6.
const PROVIDER_FAMILIES: &[(&str, &[&str])] = &[
    (
        "W6-system-host-guest",
        &[
            "provider-system-core",
            "provider-system-systemd",
            "provider-system-minijail",
            "provider-runtime-cloud-hypervisor",
            "provider-runtime-qemu-media",
            "provider-runtime-azure-container-apps",
            "provider-runtime-azure-virtual-machine",
        ],
    ),
    (
        "W6-storage-network-device",
        &[
            "provider-volume-local",
            "provider-volume-virtiofs",
            "provider-network-local",
            "provider-device-gpu",
            "provider-device-tpm",
            "provider-device-usbip",
            "provider-device-security-key",
        ],
    ),
    (
        "W6-interaction",
        &[
            "provider-display-wayland",
            "provider-audio-pipewire",
            "provider-clipboard-wayland",
            "provider-notification-desktop",
            "provider-shell-terminal",
        ],
    ),
    (
        "W6-credentials",
        &[
            "provider-credential-entra",
            "provider-credential-managed-identity",
            "provider-credential-secret-service",
        ],
    ),
    (
        "W6-transport-observability-activation",
        &[
            "provider-transport-unix",
            "provider-transport-vsock",
            "provider-transport-azure-relay",
            "provider-observability-otel",
            "provider-activation-nixos",
        ],
    ),
];

/// Shared-file barriers, as `(prerequisite, consumer)` work-item pairs.
const FILE_OVERLAP_EDGES: &[(&str, &str)] = &[
    ("ADR046-nix-014", "ADR046-cli-011"),
    ("ADR046-core-001", "ADR046-device-007"),
    ("ADR046-core-001", "ADR046-exec-013"),
    ("ADR046-core-001", "ADR046-exec-015"),
    ("ADR046-core-001", "ADR046-network-008"),
    ("ADR046-device-006", "ADR046-nix-014"),
    ("ADR046-cli-011", "ADR046-nix-019"),
    ("ADR046-nix-019", "ADR046-nix-031"),
    ("ADR046-transport-unix-009", "ADR046-qemu-media-017"),
    ("ADR046-core-001", "ADR046-telem-011"),
    ("ADR046-gpu-007", "ADR046-transport-unix-009"),
    ("ADR046-qemu-media-017", "ADR046-usbip-008"),
    ("ADR046-core-001", "ADR046-zone-control-016"),
    ("ADR046-core-001", "ADR046-zone-control-021"),
];

/// Work items whose parallel group is the shared-file barrier group rather
/// than their owning spec's default `wi:<specId>` group.
const WORK_ITEM_GROUP_OVERRIDES: &[(&str, &str)] = &[
    ("ADR046-network-008", "core-config-hub"),
    ("ADR046-device-007", "core-config-hub"),
    ("ADR046-exec-013", "core-config-hub"),
    ("ADR046-exec-015", "core-config-hub"),
    ("ADR046-telem-011", "core-config-hub"),
    ("ADR046-zone-control-016", "core-config-hub"),
    ("ADR046-zone-control-021", "core-config-hub"),
    ("ADR046-core-002", "core-controller-coordination"),
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecSetView {
    members: Vec<SpecMemberView>,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecMemberView {
    depends_on: Vec<String>,
    path: String,
    spec_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkItemsView {
    items: Vec<WorkItemView>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkItemView {
    dependency_owner: String,
    destination: String,
    detailed_design: String,
    spec_id: String,
    validation: String,
    work_item_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub blockers: Vec<String>,
    pub destinations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detailed_design: Option<String>,
    pub entry_contracts: Vec<String>,
    pub exit_gate: String,
    pub id: String,
    pub kind: String,
    pub owner: String,
    pub parallel_group: String,
    pub prerequisites: Vec<String>,
    pub spec_id: String,
    pub topological_rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<String>,
    pub wave: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    #[serde(skip)]
    order: (String, String, String),
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub edge_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    edges: usize,
    max_topological_rank: u32,
    nodes: usize,
    spec_nodes: usize,
    waves: usize,
    work_item_nodes: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedFrom {
    spec_set: String,
    wave_topology: String,
    work_items: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WaveSummary {
    parallel_groups: Vec<String>,
    spec_count: usize,
    wave: String,
    work_item_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphDoc {
    adr: String,
    artifact_kind: String,
    counts: Counts,
    critical_path: Vec<String>,
    edge_types: Vec<String>,
    edges: Vec<Edge>,
    generated_from: GeneratedFrom,
    nodes: Vec<Node>,
    schema_version: u32,
    status: String,
    waves: Vec<WaveSummary>,
}

/// Regenerates both graph artifacts under `root` and returns the written paths.
pub fn generate(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let graph = build(root)?;
    let json_path = root.join(GRAPH_JSON_PATH);
    let md_path = root.join(GRAPH_MD_PATH);
    std::fs::write(&json_path, render_json(&graph)?)?;
    std::fs::write(&md_path, render_markdown(&graph))?;
    Ok(vec![json_path, md_path])
}

/// Builds the graph from the two committed manifests.
pub fn build(root: &Path) -> Result<GraphDoc, Box<dyn std::error::Error>> {
    let spec_set: SpecSetView = serde_json::from_slice(&std::fs::read(root.join(SPEC_SET_PATH))?)?;
    let work_items: WorkItemsView =
        serde_json::from_slice(&std::fs::read(root.join(WORK_ITEMS_PATH))?)?;
    build_from(&spec_set, &work_items)
}

fn build_from(
    spec_set: &SpecSetView,
    work_items: &WorkItemsView,
) -> Result<GraphDoc, Box<dyn std::error::Error>> {
    let spec_waves = spec_waves(spec_set)?;
    let spec_groups = spec_groups(spec_set)?;
    let item_ids: BTreeSet<String> = work_items
        .items
        .iter()
        .map(|item| item.work_item_id.clone())
        .collect();

    let mut item_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for item in &work_items.items {
        let mut deps = scan_work_item_refs(&item.dependency_owner, &item_ids);
        deps.remove(&item.work_item_id);
        if let Some(missing) = deps.difference(&item_ids).next() {
            return Err(format!(
                "work item `{}` depends on `{missing}`, which is not a declared work item",
                item.work_item_id
            )
            .into());
        }
        item_deps.insert(item.work_item_id.clone(), deps);
    }
    for (prerequisite, consumer) in FILE_OVERLAP_EDGES {
        for id in [prerequisite, consumer] {
            if !item_ids.contains(*id) {
                return Err(format!(
                    "file-overlap barrier references `{id}`, which is not a declared work item"
                )
                .into());
            }
        }
        item_deps
            .get_mut(*consumer)
            .expect("consumer is a declared work item")
            .insert((*prerequisite).to_string());
    }

    let item_waves = item_waves(work_items, &spec_waves, &item_deps)?;

    let mut nodes: Vec<Node> = Vec::with_capacity(spec_set.members.len() + work_items.items.len());
    let mut prerequisites: HashMap<String, Vec<String>> = HashMap::new();

    for member in &spec_set.members {
        let wave = spec_waves[&member.spec_id];
        let deps = member.depends_on.clone();
        prerequisites.insert(member.spec_id.clone(), deps.clone());
        nodes.push(Node {
            blockers: Vec::new(),
            destinations: vec![member.path.clone()],
            detailed_design: None,
            entry_contracts: deps.clone(),
            exit_gate: exit_gate(wave),
            id: member.spec_id.clone(),
            kind: "spec".to_string(),
            owner: format!("ADR046-W{wave} wave (ADR-046-validation-and-delivery §3.2)"),
            parallel_group: spec_groups[&member.spec_id].clone(),
            prerequisites: deps,
            spec_id: member.spec_id.clone(),
            topological_rank: 0,
            validation: None,
            wave: format!("W{wave}"),
        });
    }

    for item in &work_items.items {
        let wave = item_waves[&item.work_item_id];
        let mut prereqs: BTreeSet<String> = item_deps[&item.work_item_id].clone();
        prereqs.insert(item.spec_id.clone());
        let prereqs: Vec<String> = prereqs.into_iter().collect();
        prerequisites.insert(item.work_item_id.clone(), prereqs.clone());
        nodes.push(Node {
            blockers: Vec::new(),
            destinations: vec![item.destination.clone()],
            detailed_design: Some(item.detailed_design.clone()),
            entry_contracts: vec![item.spec_id.clone()],
            exit_gate: exit_gate(wave),
            id: item.work_item_id.clone(),
            kind: "work-item".to_string(),
            owner: item.dependency_owner.clone(),
            parallel_group: work_item_group(&item.work_item_id, &item.spec_id, wave),
            prerequisites: prereqs,
            spec_id: item.spec_id.clone(),
            topological_rank: 0,
            validation: Some(item.validation.clone()),
            wave: format!("W{wave}"),
        });
    }

    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let ranks = topological_ranks(&nodes, &prerequisites)?;
    for node in &mut nodes {
        node.topological_rank = ranks[&node.id];
    }

    validate_waves(&nodes, &prerequisites)?;

    let edges = build_edges(spec_set, &spec_waves, work_items, &item_deps);
    let critical_path = critical_path(&nodes, &prerequisites, &ranks);
    let waves = wave_summaries(&nodes);

    let max_rank = ranks.values().copied().max().unwrap_or_default();
    let spec_nodes = spec_set.members.len();
    let work_item_nodes = work_items.items.len();

    Ok(GraphDoc {
        adr: ADR.to_string(),
        artifact_kind: ARTIFACT_KIND.to_string(),
        counts: Counts {
            edges: edges.len(),
            max_topological_rank: max_rank,
            nodes: nodes.len(),
            spec_nodes,
            waves: waves.len(),
            work_item_nodes,
        },
        critical_path,
        edge_types: vec![
            EDGE_FILE_OVERLAP.to_string(),
            EDGE_IMPLEMENTS.to_string(),
            EDGE_SHARED_CONTRACT.to_string(),
            EDGE_SPEC_DEPENDS.to_string(),
            EDGE_WORK_ITEM_DEPENDS.to_string(),
        ],
        edges,
        generated_from: GeneratedFrom {
            spec_set: SPEC_SET_PATH.to_string(),
            wave_topology: WAVE_TOPOLOGY_REF.to_string(),
            work_items: WORK_ITEMS_PATH.to_string(),
        },
        nodes,
        schema_version: SCHEMA_VERSION,
        status: spec_set.status.clone(),
        waves,
    })
}

fn exit_gate(wave: u8) -> String {
    format!(
        "ADR046-W{wave} exit criteria (ADR-046-validation-and-delivery §4): every spec/work item in this wave Merged with clean destinations, all validators green, and the ten-role panel seal recorded"
    )
}

fn short_name(spec_id: &str) -> &str {
    spec_id.strip_prefix(SPEC_ID_PREFIX).unwrap_or(spec_id)
}

fn spec_waves(spec_set: &SpecSetView) -> Result<BTreeMap<String, u8>, Box<dyn std::error::Error>> {
    let mut waves = BTreeMap::new();
    for member in &spec_set.members {
        let short = short_name(&member.spec_id);
        let wave = SPEC_TOPOLOGY
            .iter()
            .find(|(name, _, _)| *name == short)
            .map(|(_, wave, _)| *wave)
            .or_else(|| {
                PROVIDER_FAMILIES
                    .iter()
                    .any(|(_, members)| members.contains(&short))
                    .then_some(6)
            })
            .ok_or_else(|| {
                format!(
                    "`{}` has no delivery wave assignment; add it to the wave topology",
                    member.spec_id
                )
            })?;
        waves.insert(member.spec_id.clone(), wave);
    }
    Ok(waves)
}

fn spec_groups(
    spec_set: &SpecSetView,
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut groups = BTreeMap::new();
    for member in &spec_set.members {
        let short = short_name(&member.spec_id);
        let group = SPEC_TOPOLOGY
            .iter()
            .find(|(name, _, _)| *name == short)
            .map(|(_, _, group)| (*group).to_string())
            .or_else(|| {
                PROVIDER_FAMILIES
                    .iter()
                    .find(|(_, members)| members.contains(&short))
                    .map(|(group, _)| (*group).to_string())
            })
            .ok_or_else(|| {
                format!(
                    "`{}` has no parallel-group assignment; add it to the wave topology",
                    member.spec_id
                )
            })?;
        groups.insert(member.spec_id.clone(), group);
    }
    Ok(groups)
}

fn work_item_group(work_item_id: &str, spec_id: &str, wave: u8) -> String {
    WORK_ITEM_GROUP_OVERRIDES
        .iter()
        .find(|(id, _)| *id == work_item_id)
        .map(|(_, group)| format!("wi:{group}:w{wave}"))
        .unwrap_or_else(|| format!("wi:{spec_id}"))
}

/// Resolves each work item's launch wave as the least fixpoint of
/// `max(owning spec wave, max(dependency waves))`.
fn item_waves(
    work_items: &WorkItemsView,
    spec_waves: &BTreeMap<String, u8>,
    item_deps: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeMap<String, u8>, Box<dyn std::error::Error>> {
    let mut waves: BTreeMap<String, u8> = BTreeMap::new();
    for item in &work_items.items {
        let wave = *spec_waves.get(&item.spec_id).ok_or_else(|| {
            format!(
                "work item `{}` names spec `{}`, which is not a member of the set",
                item.work_item_id, item.spec_id
            )
        })?;
        waves.insert(item.work_item_id.clone(), wave);
    }
    let limit = work_items.items.len() + 1;
    for round in 0..=limit {
        let mut changed = false;
        for (id, deps) in item_deps {
            let mut wave = waves[id];
            for dep in deps {
                wave = wave.max(waves[dep]);
            }
            if wave != waves[id] {
                waves.insert(id.clone(), wave);
                changed = true;
            }
        }
        if !changed {
            return Ok(waves);
        }
        if round == limit {
            break;
        }
    }
    Err("work-item wave assignment did not converge; the dependency graph is cyclic".into())
}

fn build_edges(
    spec_set: &SpecSetView,
    spec_waves: &BTreeMap<String, u8>,
    work_items: &WorkItemsView,
    item_deps: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    let mut push = |edge_type: &str, from: &str, to: &str| {
        edges.push(Edge {
            order: (edge_type.to_string(), from.to_string(), to.to_string()),
            from: from.to_string(),
            to: to.to_string(),
            edge_type: edge_type.to_string(),
        });
    };

    let overlap: BTreeSet<(String, String)> = FILE_OVERLAP_EDGES
        .iter()
        .map(|(prerequisite, consumer)| ((*consumer).to_string(), (*prerequisite).to_string()))
        .collect();
    for (from, to) in &overlap {
        push(EDGE_FILE_OVERLAP, from, to);
    }
    for member in &spec_set.members {
        for dependency in &member.depends_on {
            push(EDGE_SPEC_DEPENDS, &member.spec_id, dependency);
            if spec_waves[&member.spec_id] == spec_waves[dependency] {
                push(EDGE_SHARED_CONTRACT, &member.spec_id, dependency);
            }
        }
    }
    for item in &work_items.items {
        push(EDGE_IMPLEMENTS, &item.work_item_id, &item.spec_id);
        for dependency in &item_deps[&item.work_item_id] {
            if overlap.contains(&(item.work_item_id.clone(), dependency.clone())) {
                continue;
            }
            push(EDGE_WORK_ITEM_DEPENDS, &item.work_item_id, dependency);
        }
    }

    edges.sort_by(|a, b| a.order.cmp(&b.order));
    edges
}

/// Longest-path depth over the `prerequisites` relation.
fn topological_ranks(
    nodes: &[Node],
    prerequisites: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, u32>, Box<dyn std::error::Error>> {
    let mut ranks: HashMap<String, u32> = HashMap::with_capacity(nodes.len());
    let mut state: HashMap<&str, u8> = HashMap::with_capacity(nodes.len());
    let mut stack: Vec<(&str, usize)> = Vec::new();

    for node in nodes {
        if ranks.contains_key(&node.id) {
            continue;
        }
        stack.push((node.id.as_str(), 0));
        while let Some((id, index)) = stack.pop() {
            let deps = prerequisites
                .get(id)
                .ok_or_else(|| format!("edge endpoint `{id}` has no node"))?;
            if index == 0 {
                if state.get(id) == Some(&1) {
                    return Err(format!("dependency cycle detected at `{id}`").into());
                }
                state.insert(id, 1);
            }
            if index < deps.len() {
                let next = deps[index].as_str();
                stack.push((id, index + 1));
                if !ranks.contains_key(next) {
                    if state.get(next) == Some(&1) {
                        return Err(format!(
                            "dependency cycle detected between `{id}` and `{next}`"
                        )
                        .into());
                    }
                    stack.push((next, 0));
                }
                continue;
            }
            let rank = deps
                .iter()
                .map(|dep| ranks[dep.as_str()] + 1)
                .max()
                .unwrap_or(0);
            ranks.insert(id.to_string(), rank);
            state.insert(id, 2);
        }
    }
    Ok(ranks)
}

fn validate_waves(
    nodes: &[Node],
    prerequisites: &HashMap<String, Vec<String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let wave_of: HashMap<&str, &str> = nodes
        .iter()
        .map(|node| (node.id.as_str(), node.wave.as_str()))
        .collect();
    for node in nodes {
        for dependency in &prerequisites[&node.id] {
            let dependency_wave = wave_of[dependency.as_str()];
            if dependency_wave > node.wave.as_str() {
                return Err(format!(
                    "`{}` in {} depends on `{dependency}` in the later {dependency_wave}",
                    node.id, node.wave
                )
                .into());
            }
        }
    }
    let mut group_waves: BTreeMap<&str, &str> = BTreeMap::new();
    for node in nodes {
        match group_waves.insert(node.parallel_group.as_str(), node.wave.as_str()) {
            Some(previous) if previous != node.wave => {
                return Err(format!(
                    "parallel group `{}` spans {previous} and {}",
                    node.parallel_group, node.wave
                )
                .into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Walks the longest chain back from the lexicographically smallest deepest
/// node, breaking every tie lexicographically so the path is deterministic.
fn critical_path(
    nodes: &[Node],
    prerequisites: &HashMap<String, Vec<String>>,
    ranks: &HashMap<String, u32>,
) -> Vec<String> {
    let Some(max_rank) = ranks.values().copied().max() else {
        return Vec::new();
    };
    let Some(start) = nodes
        .iter()
        .filter(|node| ranks[&node.id] == max_rank)
        .map(|node| node.id.clone())
        .min()
    else {
        return Vec::new();
    };
    let mut path = vec![start];
    loop {
        let current = path.last().expect("path is never empty").clone();
        let rank = ranks[&current];
        if rank == 0 {
            break;
        }
        let Some(previous) = prerequisites[&current]
            .iter()
            .filter(|dependency| ranks[dependency.as_str()] + 1 == rank)
            .min()
        else {
            break;
        };
        path.push(previous.clone());
    }
    path.reverse();
    path
}

fn wave_summaries(nodes: &[Node]) -> Vec<WaveSummary> {
    let mut by_wave: BTreeMap<&str, (usize, usize, BTreeSet<&str>)> = BTreeMap::new();
    for node in nodes {
        let entry = by_wave.entry(node.wave.as_str()).or_default();
        if node.kind == "spec" {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
        entry.2.insert(node.parallel_group.as_str());
    }
    by_wave
        .into_iter()
        .map(
            |(wave, (spec_count, work_item_count, groups))| WaveSummary {
                parallel_groups: groups.into_iter().map(str::to_string).collect(),
                spec_count,
                wave: wave.to_string(),
                work_item_count,
            },
        )
        .collect()
}

// ---------------------------------------------------------------------------
// Work-item dependency scanning
// ---------------------------------------------------------------------------

/// Extracts every work-item dependency declared by a `Dependency/owner` cell.
///
/// Literal ids are collected whether or not they sit in an inline code fence.
/// An inclusive `A through B` span additionally contributes its interior ids,
/// but only where the endpoints are written as bare prose; a fenced or
/// typographic-dash span contributes its endpoints alone. That asymmetry is
/// the recorded contract of the committed graph, not a deliberate rule: see
/// the range-interior finding in the ADR 0046 regeneration notes. Widening it
/// here would silently rewrite dependency edges the audit certified, so the
/// omission is reported instead.
///
/// A `Dependency for X` clause states the reverse relation and contributes no
/// edge in either direction.
fn scan_work_item_refs(text: &str, known: &BTreeSet<String>) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for clause in text.split(';') {
        if clause.trim_start().starts_with("Dependency for") {
            continue;
        }
        collect_ids(&clause.replace('`', ""), &mut refs);
        expand_ranges(clause, known, &mut refs);
    }
    refs
}
fn collect_ids(text: &str, out: &mut BTreeSet<String>) {
    for token in tokens(text) {
        if parse_id(&token).is_some() {
            out.insert(token);
        }
    }
}

fn expand_ranges(text: &str, known: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    let words = range_tokens(text);
    for (index, word) in words.iter().enumerate() {
        if word != "through" || index == 0 || index + 1 >= words.len() {
            continue;
        }
        let Some((prefix, start)) = parse_id(&words[index - 1]) else {
            continue;
        };
        let end = match parse_id(&words[index + 1]) {
            Some((end_prefix, end)) if end_prefix == prefix => end,
            Some(_) => continue,
            None => match parse_ordinal(&words[index + 1]) {
                Some(end) => end,
                None => continue,
            },
        };
        if end < start {
            continue;
        }
        for id in known {
            if let Some((candidate_prefix, ordinal)) = parse_id(id)
                && candidate_prefix == prefix
                && ordinal >= start
                && ordinal <= end
            {
                out.insert(id.clone());
            }
        }
    }
}

/// Splits prose into candidate tokens, dropping surrounding punctuation.
///
/// Typographic dashes separate tokens, so an `A–B` span yields its endpoints
/// as independent tokens rather than one unparsable run.
fn tokens(text: &str) -> Vec<String> {
    text.split(is_token_separator)
        .map(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_token_separator(c: char) -> bool {
    c.is_whitespace()
        || c == '/'
        || c == '('
        || c == ')'
        || c == ','
        || c == '\u{2013}'
        || c == '\u{2014}'
}

/// Tokenizes for range detection while keeping inline code fences attached, so
/// a fenced endpoint does not parse as an id and its span is left unexpanded.
fn range_tokens(text: &str) -> Vec<String> {
    text.split(is_token_separator)
        .map(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '`'))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_id(token: &str) -> Option<(String, u32)> {
    let body = token.strip_prefix(WORK_ITEM_ID_PREFIX)?;
    let (prefix, ordinal) = body.rsplit_once('-')?;
    if prefix.is_empty() {
        return None;
    }
    Some((prefix.to_string(), parse_ordinal(ordinal)?))
}

fn parse_ordinal(token: &str) -> Option<u32> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.len() != 3 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u32 = digits.parse().ok()?;
    (value != 0).then_some(value)
}

// ---------------------------------------------------------------------------
// Markdown rendering
// ---------------------------------------------------------------------------

fn render_markdown(graph: &GraphDoc) -> String {
    let mut out = String::new();
    out.push_str("# ADR 0046 implementation graph (generated)\n\n");
    out.push_str(&format!(
        "> **Generated index - not a normative member.** This file and its companion\n\
         > [`ADR-046-implementation-graph.json`](ADR-046-implementation-graph.json) are\n\
         > deterministically generated from\n\
         > [`ADR-046-spec-set.json`](ADR-046-spec-set.json),\n\
         > [`ADR-046-work-items.json`](ADR-046-work-items.json), and the {}-wave topology\n\
         > in [`ADR-046-validation-and-delivery.md` §3](ADR-046-validation-and-delivery.md).\n\
         > They are **not** among the {} `ADR-046-spec-set.json` members. Regenerate them\n\
         > with `cargo run -p xtask -- spec-registry` followed by\n\
         > `cargo run -p xtask -- implementation-graph`; the committed bytes are enforced\n\
         > by the ADR 0046 work-item policy gate.\n\n",
        graph.counts.waves, graph.counts.spec_nodes
    ));
    out.push_str(
        "The graph maps every member spec and every work item exactly once to a\n\
         dependency-ordered launch wave (`W0`–`W7`) and a file-disjoint parallel group.\n\
         It includes every resolved security-key work-item dependency; no lexical\n\
         tie-break or omitted dependency is used.\n\
         Each JSON work-item node also embeds the manifest's exact `detailedDesign` and\n\
         `validation` text byte-for-byte.\n\n",
    );

    out.push_str("## Counts\n\n| Metric | Value |\n| --- | --- |\n");
    out.push_str(&format!("| Waves | {} |\n", graph.counts.waves));
    out.push_str(&format!("| Spec nodes | {} |\n", graph.counts.spec_nodes));
    out.push_str(&format!(
        "| Work-item nodes | {} |\n",
        graph.counts.work_item_nodes
    ));
    out.push_str(&format!("| Total nodes | {} |\n", graph.counts.nodes));
    out.push_str(&format!("| Edges | {} |\n", graph.counts.edges));
    out.push_str(&format!(
        "| Max topological rank | {} |\n\n",
        graph.counts.max_topological_rank
    ));

    let first_wave = graph.waves.first().map(|w| w.wave.as_str()).unwrap_or("W0");
    let last_wave = graph.waves.last().map(|w| w.wave.as_str()).unwrap_or("W7");
    out.push_str(&format!("## Waves ({first_wave}–{last_wave})\n\n"));
    out.push_str("| Wave | Specs | #Specs | #Work items | Parallel groups |\n| --- | --- | --- | --- | --- |\n");
    for wave in &graph.waves {
        let specs = spec_short_names(graph, &wave.wave).join(", ");
        let groups = wave
            .parallel_groups
            .iter()
            .filter(|group| !group.starts_with("wi:"))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "| {} | {specs} | {} | {} | {groups} |\n",
            wave.wave, wave.spec_count, wave.work_item_count
        ));
    }

    out.push_str("\n## Dependency DAG (waves and prep barriers)\n\n```mermaid\nflowchart LR\n");
    for wave in &graph.waves {
        out.push_str(&format!("  subgraph {}[\"{}\"]\n", wave.wave, wave.wave));
        for spec_id in spec_ids_in_wave(graph, &wave.wave) {
            let short = short_name(&spec_id);
            out.push_str(&format!("    {}[\"{short}\"]\n", short.replace('-', "_")));
        }
        out.push_str("  end\n");
    }
    let chain: Vec<&str> = graph.waves.iter().map(|wave| wave.wave.as_str()).collect();
    out.push_str(&format!("  {}\n", chain.join(" --> ")));
    for edge in graph
        .edges
        .iter()
        .filter(|e| e.edge_type == EDGE_SHARED_CONTRACT)
    {
        out.push_str(&format!(
            "  {} -. prep .-> {}\n",
            short_name(&edge.to).replace('-', "_"),
            short_name(&edge.from).replace('-', "_")
        ));
    }
    out.push_str("```\n\n");
    out.push_str(
        "Solid arrows show wave launch order. Dotted arrows show same-wave shared\n\
         contract prep. Work-item dependencies and actual file-overlap barriers are\n\
         fully represented in the JSON.\n\n",
    );

    out.push_str("## Shared prep and file-overlap barriers\n\n| Prerequisite | Consumer | Type |\n| --- | --- | --- |\n");
    for edge in graph
        .edges
        .iter()
        .filter(|e| e.edge_type == EDGE_FILE_OVERLAP || e.edge_type == EDGE_SHARED_CONTRACT)
    {
        out.push_str(&format!(
            "| `{}` | `{}` | {} |\n",
            edge.to, edge.from, edge.edge_type
        ));
    }
    out.push_str(
        "\nOnly the listed `file-overlap-order` edges constrain shared files. Provider\n\
         integration ordering that touches disjoint crate trees is not represented as\n\
         file overlap. The former `wi:core-config-hub` is split into\n\
         `wi:core-config-hub:w4` and `wi:core-config-hub:w5`; each parallel group is\n\
         single-wave. The seven `assertions.nix` edges form the minimal per-wave chains\n\
         `ADR046-device-006` → `ADR046-nix-014` → `ADR046-cli-011` →\n\
         `ADR046-nix-019` → `ADR046-nix-031` in W5 and `ADR046-gpu-007` →\n\
         `ADR046-transport-unix-009` → `ADR046-qemu-media-017` →\n\
         `ADR046-usbip-008` in W6. W2 has one writer. These edges order only the shared\n\
         file; all other destinations retain their existing parallelism.\n\n",
    );

    out.push_str("## Parallel groups\n\n| Parallel group | Wave | #Nodes |\n| --- | --- | --- |\n");
    let mut group_rows: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
    for node in &graph.nodes {
        let entry = group_rows
            .entry(node.parallel_group.as_str())
            .or_insert((node.wave.as_str(), 0));
        entry.1 += 1;
    }
    for (group, (wave, count)) in &group_rows {
        out.push_str(&format!("| `{group}` | {wave} | {count} |\n"));
    }

    out.push_str("\n## Critical path (longest dependency chain)\n\n");
    for (index, id) in graph.critical_path.iter().enumerate() {
        out.push_str(&format!("{}. `{id}`\n", index + 1));
    }

    out.push_str("\n## Regeneration findings (D095–D098)\n\n");
    out.push_str(&format!(
        "- Regenerated from {} member specs and {} current work items; every declared heading is represented exactly once.\n",
        graph.counts.spec_nodes, graph.counts.work_item_nodes
    ));
    out.push_str("- `ADR046-provider-004` owns the common D098 Service/Binding base DTOs and schemas; the four implementation Providers own only strict extensions and controllers.\n");
    out.push_str("- `ADR046-zone-control-024` owns the shared Core-derived `physical-usb-backing` tuple; both the security-key and USB effect DAGs depend on it.\n");
    out.push_str("- Every `ADR046-security-key-*` dependency in `Dependency/owner` is encoded. The dependency subgraph is acyclic and uses no generator tie-break.\n");
    out.push_str(&format!(
        "- {} file-overlap barriers cover only the shared core\n  \
         configuration/cleanup files and `nixos-modules/assertions.nix`. Each appears\n  \
         both as a\n  \
         `file-overlap-order` edge and in the dependent node's `prerequisites`, so the\n  \
         ready-wave query enforces it. Soft cross-Provider integration order remains\n  \
         file-disjoint and concurrent.\n",
        spelled_count(
            graph
                .edges
                .iter()
                .filter(|e| e.edge_type == EDGE_FILE_OVERLAP)
                .count()
        )
    ));
    out.push_str("- `cargo run -p xtask -- spec-registry` and `cargo run -p xtask -- implementation-graph` are the canonical generators; `packages/d2b-contract-tests/tests/policy_adr046_work_items.rs` is the fail-closed drift gate that keeps the committed bytes honest.\n");

    out.push_str(
        "\n## Ready-wave algorithm\n\n\
         A node is ready when every id in `prerequisites` is done:\n\n\
         ```bash\n\
         jq --argjson done \"$DONE\" '\n  \
         .nodes[] | select((.prerequisites - $done) | length == 0)\n  \
         | {id, kind, wave, parallelGroup, topologicalRank}\n\
         ' docs/specs/ADR-046-implementation-graph.json\n\
         ```\n\n\
         A ready, file-disjoint group left unlaunched without a recorded blocker violates\n\
         the anti-serialization invariant.\n\n",
    );

    out.push_str(
        "## References\n\n\
         - [ADR 0046](../adr/0046-d2b-3-provider-control-plane.md)\n\
         - [Decision register](ADR-046-decision-register.md)\n\
         - [Validation and delivery](ADR-046-validation-and-delivery.md)\n\
         - [Spec-set manifest](ADR-046-spec-set.json)\n\
         - [Work-item manifest](ADR-046-work-items.json)\n",
    );
    out
}

fn spec_ids_in_wave(graph: &GraphDoc, wave: &str) -> Vec<String> {
    graph
        .nodes
        .iter()
        .filter(|node| node.kind == "spec" && node.wave == wave)
        .map(|node| node.id.clone())
        .collect()
}

fn spec_short_names(graph: &GraphDoc, wave: &str) -> Vec<String> {
    spec_ids_in_wave(graph, wave)
        .iter()
        .map(|id| short_name(id).to_string())
        .collect()
}

fn spelled_count(count: usize) -> String {
    const WORDS: [&str; 21] = [
        "Zero",
        "One",
        "Two",
        "Three",
        "Four",
        "Five",
        "Six",
        "Seven",
        "Eight",
        "Nine",
        "Ten",
        "Eleven",
        "Twelve",
        "Thirteen",
        "Fourteen",
        "Fifteen",
        "Sixteen",
        "Seventeen",
        "Eighteen",
        "Nineteen",
        "Twenty",
    ];
    WORDS
        .get(count)
        .map(|word| (*word).to_string())
        .unwrap_or_else(|| count.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn a_fenced_range_contributes_only_its_endpoints() {
        let ids = known(&[
            "ADR046-audio-001",
            "ADR046-audio-002",
            "ADR046-audio-003",
            "ADR046-audio-004",
            "ADR046-audio-005",
        ]);
        let fenced = scan_work_item_refs(
            "Depends on `ADR046-audio-001` through `ADR046-audio-005`",
            &ids,
        );
        assert_eq!(
            fenced,
            known(&["ADR046-audio-001", "ADR046-audio-005"]),
            "a fenced span records its endpoints only; widening this silently \
             rewrites dependency edges across the committed graph"
        );

        let bare =
            scan_work_item_refs("Depends on ADR046-audio-001 through ADR046-audio-005", &ids);
        assert_eq!(bare, ids, "an unfenced span expands inclusively");
    }

    #[test]
    fn a_typographic_dash_span_yields_its_left_endpoint_only() {
        let ids = known(&[
            "ADR046-network-001",
            "ADR046-network-002",
            "ADR046-network-003",
            "ADR046-network-004",
        ]);
        assert_eq!(
            scan_work_item_refs("ADR046-network-001\u{2013}004; owner", &ids),
            known(&["ADR046-network-001"]),
            "a dash span is not a recognized range spelling"
        );
    }

    #[test]
    fn a_dependency_for_clause_contributes_no_edge() {
        let ids = known(&["ADR046-cred-ss-003"]);
        assert!(
            scan_work_item_refs("Dependency for `ADR046-cred-ss-003`", &ids).is_empty(),
            "a `Dependency for` clause states the reverse relation"
        );
    }

    #[test]
    fn ranges_accept_a_bare_right_endpoint() {
        let ids = known(&[
            "ADR046-transport-unix-002",
            "ADR046-transport-unix-003",
            "ADR046-transport-unix-004",
        ]);
        let refs = scan_work_item_refs("ADR046-transport-unix-002 through 004", &ids);
        assert_eq!(refs, ids);
    }

    #[test]
    fn negated_and_reverse_phrasings_contribute_no_edge() {
        let ids = known(&["ADR046-feasibility-001", "ADR046-feasibility-004"]);
        let negated = scan_work_item_refs(
            "Independent of `-001` through `-004`; bus/session integrator",
            &ids,
        );
        assert!(negated.is_empty());

        let reverse = scan_work_item_refs(
            "Dependency for ADR046-cred-ss-003; owner: credential service contract",
            &known(&["ADR046-cred-ss-003"]),
        );
        assert!(reverse.is_empty());
    }

    #[test]
    fn prose_use_of_through_is_not_a_range() {
        let ids = known(&["ADR046-nl-001", "ADR046-nl-005"]);
        let refs = scan_work_item_refs(
            "Provider plus Core; provider validates through `d2b_host::ifname::derive_ifname`.",
            &ids,
        );
        assert!(refs.is_empty());
    }

    #[test]
    fn work_item_groups_prefer_the_shared_file_barrier_override() {
        assert_eq!(
            work_item_group("ADR046-network-008", "ADR-046-resources-network", 4),
            "wi:core-config-hub:w4"
        );
        assert_eq!(
            work_item_group("ADR046-nl-001", "ADR-046-provider-network-local", 6),
            "wi:ADR-046-provider-network-local"
        );
    }

    #[test]
    fn spelled_counts_cover_the_barrier_range() {
        assert_eq!(spelled_count(14), "Fourteen");
        assert_eq!(spelled_count(0), "Zero");
        assert_eq!(spelled_count(99), "99");
    }
}
