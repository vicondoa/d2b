//! Fail-closed policy gate for the generated ADR 0046 spec-set, work-item, and
//! implementation-graph artifacts.
//!
//! The byte-level drift gate lives in `tests/unit/gates/drift-check.sh`, which
//! regenerates the artifacts with `cargo xtask spec-registry` /
//! `cargo xtask implementation-graph` and requires a clean `git diff`. This
//! module is the *semantic* half: it re-derives the Markdown/manifest bijection
//! independently of the generator, so a generator bug cannot certify itself.
//!
//! Every helper below takes plain in-memory inputs, which lets the negative
//! fixtures exercise the same code path the real tree is checked with.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contract_tests::{read_repo_file, repo_root};
use serde_json::Value;

const SPEC_SET: &str = "docs/specs/ADR-046-spec-set.json";
const WORK_ITEMS: &str = "docs/specs/ADR-046-work-items.json";
const GRAPH_JSON: &str = "docs/specs/ADR-046-implementation-graph.json";
const GRAPH_MD: &str = "docs/specs/ADR-046-implementation-graph.md";

/// The normative member count, per `docs/specs/README.md`.
const EXPECTED_MEMBERS: usize = 55;
/// The normative work-item count. The corpus is closed; a parser or source
/// regression that changes it must fail rather than shrink the manifests.
const EXPECTED_WORK_ITEMS: usize = 543;
/// The certified graph shape. Pinned so a silent edge gain or loss fails here
/// even when the generator regenerates itself consistently.
const EXPECTED_NODES: u64 = 598;
const EXPECTED_EDGES: u64 = 1940;
const EXPECTED_MAX_RANK: u64 = 22;
const EXPECTED_CRITICAL_PATH: usize = 23;

const REUSE_ACTIONS: &[&str] = &[
    "adapt",
    "copy-unchanged",
    "create",
    "delete-after-cutover",
    "extract",
    "replace",
    "wrap",
];

const MANDATORY_FIELDS: &[&str] = &[
    "Current source",
    "Data migration",
    "Dependency/owner",
    "Destination",
    "Detailed design",
    "Integration",
    "Removal proof",
    "Reuse action",
    "Validation",
];

// ---------------------------------------------------------------------------
// Markdown side
// ---------------------------------------------------------------------------

/// A work-item declaration found in a member's Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Declaration {
    id: String,
    level: usize,
    form: &'static str,
    fields: Vec<(String, String)>,
}

/// Classifies the heading remainder that follows a work-item id. Derived here
/// independently of the generator so the two must agree.
fn classify(rest: &str, id: &str) -> &'static str {
    let tail = rest
        .trim_start_matches('`')
        .strip_prefix(id)
        .unwrap_or("")
        .trim_start_matches('`');
    let trimmed = tail.trim_start();
    if trimmed.is_empty() {
        "bare"
    } else if trimmed.starts_with(':') {
        "colon title"
    } else if trimmed.starts_with('(') {
        "parenthetical title"
    } else {
        "em-dash title"
    }
}

fn split_row(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_end();
    let inner = trimmed.strip_prefix('|')?.strip_suffix('|')?;
    let (label, value) = inner.split_once('|')?;
    Some((
        label.trim().replace("\\|", "|"),
        value.trim().replace("\\|", "|"),
    ))
}

fn is_separator(label: &str) -> bool {
    !label.is_empty() && label.chars().all(|c| c == '-' || c == ' ' || c == ':')
}

/// Splits `ADR046-<prefix>-<ordinal>` into its prefix and ordinal, rejecting a
/// two-digit, four-digit, or zero ordinal.
fn split_id(token: &str) -> Option<(String, u32)> {
    let body = token.strip_prefix("ADR046-")?;
    let (prefix, ordinal) = body.rsplit_once('-')?;
    if prefix.is_empty() || ordinal.len() != 3 || !ordinal.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u32 = ordinal.parse().ok()?;
    if value == 0 {
        return None;
    }
    let well_formed = prefix.split('-').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    well_formed.then_some((prefix.to_string(), value))
}

/// Collects every ADR 0046 work-item declaration in one Markdown document,
/// including declarations at an invalid heading level so the caller can reject
/// them explicitly rather than silently skipping them.
fn declarations(markdown: &str) -> Vec<Declaration> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if !line.starts_with('#') {
            index += 1;
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        let rest = line[level..].trim_start();
        let token: String = rest
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ':')
            .collect();
        let token = token.trim_matches('`').to_string();
        if split_id(&token).is_none() {
            index += 1;
            continue;
        }
        let mut cursor = index + 1;
        while cursor < lines.len() && lines[cursor].trim().is_empty() {
            cursor += 1;
        }
        let mut fields = Vec::new();
        while cursor < lines.len() && lines[cursor].starts_with('|') {
            if let Some((label, value)) = split_row(lines[cursor])
                && !is_separator(&label)
                && label != "Field"
            {
                fields.push((label, value));
            }
            cursor += 1;
        }
        out.push(Declaration {
            form: classify(rest, &token),
            id: token,
            level,
            fields,
        });
        index = cursor.max(index + 1);
    }
    out
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// Verifies the exact Markdown-to-manifest bijection and every per-item
/// structural rule. Returns one message per violation.
fn check_work_items(
    declared: &BTreeMap<String, Vec<Declaration>>,
    manifest: &[Value],
    prefix_registry: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut findings = Vec::new();

    let mut markdown_ids: BTreeSet<String> = BTreeSet::new();
    let mut owner_of: BTreeMap<String, String> = BTreeMap::new();
    for (spec_id, items) in declared {
        for item in items {
            if item.level != 3 {
                findings.push(format!(
                    "`{}` declares `{}` at heading level {}; only `###` is valid",
                    spec_id, item.id, item.level
                ));
            }
            if !markdown_ids.insert(item.id.clone()) {
                findings.push(format!("work item `{}` is declared twice", item.id));
            }
            owner_of.insert(item.id.clone(), spec_id.clone());

            let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
            for (label, value) in &item.fields {
                *seen.entry(label.as_str()).or_default() += 1;
                if label == "Work item ID" {
                    let declared_id = value.trim().trim_matches('`');
                    if declared_id != item.id {
                        findings.push(format!(
                            "work item `{}` disagrees with its `Work item ID` row `{declared_id}`",
                            item.id
                        ));
                    }
                }
            }
            for field in MANDATORY_FIELDS {
                match seen.get(field) {
                    None => findings.push(format!(
                        "work item `{}` is missing mandatory field `{field}`",
                        item.id
                    )),
                    Some(1) => {}
                    Some(count) => findings.push(format!(
                        "work item `{}` declares `{field}` {count} times",
                        item.id
                    )),
                }
            }
            if item.fields.iter().any(|(label, value)| {
                MANDATORY_FIELDS.contains(&label.as_str()) && value.is_empty()
            }) {
                findings.push(format!(
                    "work item `{}` has an empty mandatory field",
                    item.id
                ));
            }
        }
    }

    // Global one-member prefix ownership, resolved through the registry.
    let mut registered: BTreeMap<&str, &str> = BTreeMap::new();
    for (spec_id, prefixes) in prefix_registry {
        let sorted: Vec<&String> = {
            let mut copy: Vec<&String> = prefixes.iter().collect();
            copy.sort();
            copy
        };
        if sorted.iter().copied().ne(prefixes.iter()) {
            findings.push(format!(
                "`{spec_id}` has an unsorted `workItemPrefixes` list"
            ));
        }
        for prefix in prefixes {
            if let Some(other) = registered.insert(prefix.as_str(), spec_id.as_str()) {
                findings.push(format!(
                    "prefix `{prefix}` is registered to both `{other}` and `{spec_id}`"
                ));
            }
        }
    }
    for (spec_id, items) in declared {
        if items.is_empty() {
            continue;
        }
        let prefixes = prefix_registry.get(spec_id);
        if list_is_empty(prefixes) {
            findings.push(format!(
                "`{spec_id}` owns work items but registers no `workItemPrefixes`"
            ));
        }
        for item in items {
            let Some((prefix, _)) = split_id(&item.id) else {
                findings.push(format!("work item `{}` has a malformed id", item.id));
                continue;
            };
            match registered.get(prefix.as_str()) {
                Some(owner) if *owner == spec_id.as_str() => {}
                Some(owner) => findings.push(format!(
                    "work item `{}` uses prefix `{prefix}` registered to `{owner}`",
                    item.id
                )),
                None => findings.push(format!(
                    "work item `{}` uses unregistered prefix `{prefix}`",
                    item.id
                )),
            }
        }
    }

    let mut manifest_ids: BTreeSet<String> = BTreeSet::new();
    for entry in manifest {
        let id = entry["workItemId"].as_str().unwrap_or_default().to_string();
        if !manifest_ids.insert(id.clone()) {
            findings.push(format!("manifest lists `{id}` more than once"));
        }
        let action = entry["reuseAction"].as_str().unwrap_or_default();
        if !REUSE_ACTIONS.contains(&action) {
            findings.push(format!("`{id}` declares free-form reuse action `{action}`"));
        }
        if action == "create" && !entry["reuseSource"].is_null() {
            findings.push(format!("`{id}` declares `create` with a reuse source"));
        }
        if let Some(owner) = owner_of.get(&id) {
            let spec_id = entry["specId"].as_str().unwrap_or_default();
            if spec_id != owner {
                findings.push(format!(
                    "`{id}` records owner `{spec_id}` but is declared by `{owner}`"
                ));
            }
        }
    }

    for missing in markdown_ids.difference(&manifest_ids) {
        findings.push(format!(
            "`{missing}` is declared but absent from the manifest"
        ));
    }
    for extra in manifest_ids.difference(&markdown_ids) {
        findings.push(format!("`{extra}` is in the manifest but declared nowhere"));
    }
    findings.sort();
    findings.dedup();
    findings
}

fn list_is_empty(prefixes: Option<&Vec<String>>) -> bool {
    prefixes.is_none_or(Vec::is_empty)
}

/// Verifies the graph's node/edge closure, acyclicity, wave monotonicity, and
/// single-wave parallel groups.
fn check_graph(graph: &Value, expected_nodes: &BTreeSet<String>) -> Vec<String> {
    let mut findings = Vec::new();
    let nodes = graph["nodes"].as_array().cloned().unwrap_or_default();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut wave_of: BTreeMap<String, String> = BTreeMap::new();
    let mut group_wave: BTreeMap<String, String> = BTreeMap::new();
    let mut prerequisites: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for node in &nodes {
        let id = node["id"].as_str().unwrap_or_default().to_string();
        if !seen.insert(id.clone()) {
            findings.push(format!("graph lists node `{id}` more than once"));
        }
        let wave = node["wave"].as_str().unwrap_or_default().to_string();
        wave_of.insert(id.clone(), wave.clone());
        let group = node["parallelGroup"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match group_wave.insert(group.clone(), wave.clone()) {
            Some(previous) if previous != wave => findings.push(format!(
                "parallel group `{group}` spans {previous} and {wave}"
            )),
            _ => {}
        }
        prerequisites.insert(
            id,
            node["prerequisites"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|value| value.as_str().unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default(),
        );
    }

    for missing in expected_nodes.difference(&seen) {
        findings.push(format!("graph is missing node `{missing}`"));
    }
    for extra in seen.difference(expected_nodes) {
        findings.push(format!("graph has unexpected node `{extra}`"));
    }

    for (id, deps) in &prerequisites {
        for dep in deps {
            match wave_of.get(dep) {
                None => findings.push(format!("`{id}` depends on unresolved `{dep}`")),
                Some(dep_wave) if dep_wave.as_str() > wave_of[id].as_str() => {
                    findings.push(format!(
                        "`{id}` in {} depends on `{dep}` in the later {dep_wave}",
                        wave_of[id]
                    ));
                }
                Some(_) => {}
            }
        }
    }

    for edge in graph["edges"].as_array().cloned().unwrap_or_default() {
        for end in ["from", "to"] {
            let id = edge[end].as_str().unwrap_or_default();
            if !seen.contains(id) {
                findings.push(format!("edge endpoint `{id}` resolves to no node"));
            }
        }
    }

    if let Some(cycle) = find_cycle(&prerequisites) {
        findings.push(format!("dependency cycle through `{cycle}`"));
    }

    findings.sort();
    findings.dedup();
    findings
}

/// Iterative depth-first search returning a node on a cycle, if any exists.
fn find_cycle(prerequisites: &BTreeMap<String, Vec<String>>) -> Option<String> {
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    for root in prerequisites.keys() {
        if state.get(root.as_str()) == Some(&2) {
            continue;
        }
        let mut stack: Vec<(&str, usize)> = vec![(root.as_str(), 0)];
        while let Some((id, index)) = stack.pop() {
            if index == 0 {
                if state.get(id) == Some(&2) {
                    continue;
                }
                state.insert(id, 1);
            }
            let deps = prerequisites.get(id).map(Vec::as_slice).unwrap_or_default();
            if index < deps.len() {
                stack.push((id, index + 1));
                let next = deps[index].as_str();
                match state.get(next) {
                    Some(1) => return Some(next.to_string()),
                    Some(2) => {}
                    _ => stack.push((next, 0)),
                }
                continue;
            }
            state.insert(id, 2);
        }
    }
    None
}

fn hex_sha256(bytes: &[u8]) -> String {
    // A small local SHA-256 so the policy gate verifies the committed digests
    // independently of the generator's hashing crate.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bit_len = (bytes.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for (index, word) in chunk.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for index in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let temp1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let temp2 = s0.wrapping_add(maj);
            v = [
                temp1.wrapping_add(temp2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(temp1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (slot, value) in h.iter_mut().zip(v) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

// ---------------------------------------------------------------------------
// Real-tree assertions
// ---------------------------------------------------------------------------

fn load(rel: &str) -> Value {
    serde_json::from_str(&read_repo_file(rel))
        .unwrap_or_else(|error| panic!("policy-lint: {rel} is not valid JSON: {error}"))
}

fn real_tree() -> (Value, Value, BTreeMap<String, Vec<Declaration>>) {
    let spec_set = load(SPEC_SET);
    let work_items = load(WORK_ITEMS);
    let mut declared = BTreeMap::new();
    for member in spec_set["members"].as_array().expect("members array") {
        let spec_id = member["specId"].as_str().expect("specId").to_string();
        let path = member["path"].as_str().expect("path");
        declared.insert(spec_id, declarations(&read_repo_file(path)));
    }
    (spec_set, work_items, declared)
}

#[test]
fn the_real_spec_tree_declares_every_work_item_exactly_once() {
    let (spec_set, work_items, declared) = real_tree();
    let members = spec_set["members"].as_array().expect("members array");
    assert_eq!(
        members.len(),
        EXPECTED_MEMBERS,
        "docs/specs/README.md pins the ADR 0046 set at {EXPECTED_MEMBERS} members"
    );
    let total: usize = declared.values().map(Vec::len).sum();
    assert_eq!(
        total, EXPECTED_WORK_ITEMS,
        "the ADR 0046 corpus is closed at {EXPECTED_WORK_ITEMS} work items"
    );

    let mut census: BTreeMap<&str, usize> = BTreeMap::new();
    for item in declared.values().flatten() {
        *census.entry(item.form).or_default() += 1;
    }
    assert_eq!(
        census,
        BTreeMap::from([
            ("bare", 356),
            ("em-dash title", 112),
            ("colon title", 51),
            ("parenthetical title", 24),
        ]),
        "a work-item heading spelling stopped being recognized"
    );

    let registry: BTreeMap<String, Vec<String>> = members
        .iter()
        .map(|member| {
            (
                member["specId"].as_str().expect("specId").to_string(),
                member["workItemPrefixes"]
                    .as_array()
                    .expect("workItemPrefixes")
                    .iter()
                    .map(|value| value.as_str().expect("prefix").to_string())
                    .collect(),
            )
        })
        .collect();

    let findings = check_work_items(
        &declared,
        work_items["items"].as_array().expect("items array"),
        &registry,
    );
    assert!(
        findings.is_empty(),
        "ADR 0046 work-item bijection violations:\n{}",
        findings.join("\n")
    );
}

#[test]
fn every_member_records_its_current_content_digest_and_a_single_status() {
    let spec_set = load(SPEC_SET);
    let mut statuses = BTreeSet::new();
    for member in spec_set["members"].as_array().expect("members array") {
        let path = member["path"].as_str().expect("path");
        let bytes = std::fs::read(repo_root().join(path))
            .unwrap_or_else(|error| panic!("policy-lint: cannot read {path}: {error}"));
        assert_eq!(
            member["sha256"].as_str().expect("sha256"),
            hex_sha256(&bytes),
            "`{path}` digest is stale; rerun `cargo run -p xtask -- spec-registry`"
        );
        statuses.insert(member["status"].as_str().expect("status").to_string());
    }
    assert_eq!(
        statuses.len(),
        1,
        "the ADR 0046 set is atomic: every member must share one status, found {statuses:?}"
    );
    let status = statuses.into_iter().next().expect("one status");
    for rel in [SPEC_SET, WORK_ITEMS, GRAPH_JSON] {
        assert_eq!(
            load(rel)["status"].as_str().expect("status"),
            status,
            "`{rel}` records a status the member set does not agree with"
        );
    }
}

#[test]
fn the_implementation_graph_is_closed_acyclic_and_wave_monotonic() {
    let spec_set = load(SPEC_SET);
    let work_items = load(WORK_ITEMS);
    let graph = load(GRAPH_JSON);

    let mut expected: BTreeSet<String> = spec_set["members"]
        .as_array()
        .expect("members array")
        .iter()
        .map(|member| member["specId"].as_str().expect("specId").to_string())
        .collect();
    expected.extend(
        work_items["items"]
            .as_array()
            .expect("items array")
            .iter()
            .map(|item| item["workItemId"].as_str().expect("workItemId").to_string()),
    );

    let findings = check_graph(&graph, &expected);
    assert!(
        findings.is_empty(),
        "ADR 0046 implementation-graph violations:\n{}",
        findings.join("\n")
    );

    let counts = &graph["counts"];
    assert_eq!(counts["specNodes"].as_u64(), Some(EXPECTED_MEMBERS as u64));
    assert_eq!(
        counts["workItemNodes"].as_u64(),
        Some(EXPECTED_WORK_ITEMS as u64)
    );
    assert_eq!(
        counts["nodes"].as_u64(),
        Some(EXPECTED_NODES),
        "the certified graph has {EXPECTED_NODES} nodes"
    );
    assert_eq!(
        counts["edges"].as_u64(),
        Some(EXPECTED_EDGES),
        "the certified graph has {EXPECTED_EDGES} edges; a change here rewrites \
         the ready-wave schedule and needs an explicit decision, not a regeneration"
    );
    assert_eq!(
        counts["maxTopologicalRank"].as_u64(),
        Some(EXPECTED_MAX_RANK)
    );
    assert_eq!(
        graph["criticalPath"].as_array().map(Vec::len),
        Some(EXPECTED_CRITICAL_PATH)
    );
    assert_eq!(
        counts["nodes"].as_u64(),
        Some(expected.len() as u64),
        "counts.nodes must equal the spec + work-item node total"
    );
    assert_eq!(
        counts["edges"].as_u64(),
        graph["edges"].as_array().map(|edges| edges.len() as u64),
        "counts.edges must equal the emitted edge count"
    );
    assert_eq!(
        counts["waves"].as_u64(),
        graph["waves"].as_array().map(|waves| waves.len() as u64),
        "counts.waves must equal the emitted wave count"
    );
}

#[test]
fn work_item_nodes_embed_the_manifest_text_byte_for_byte() {
    let work_items = load(WORK_ITEMS);
    let graph = load(GRAPH_JSON);
    let nodes: BTreeMap<&str, &Value> = graph["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|node| (node["id"].as_str().expect("id"), node))
        .collect();
    for item in work_items["items"].as_array().expect("items array") {
        let id = item["workItemId"].as_str().expect("workItemId");
        let node = nodes
            .get(id)
            .unwrap_or_else(|| panic!("graph is missing work-item node `{id}`"));
        for field in ["detailedDesign", "validation"] {
            assert_eq!(
                node[field], item[field],
                "graph node `{id}` does not embed the manifest's `{field}` byte-for-byte"
            );
        }
        assert_eq!(
            node["destinations"],
            serde_json::json!([item["destination"]]),
            "graph node `{id}` must carry its manifest destination unsplit"
        );
    }
}

#[test]
fn generated_artifacts_are_deterministic_and_carry_no_superseded_bindings() {
    for rel in [SPEC_SET, WORK_ITEMS, GRAPH_JSON, GRAPH_MD] {
        let text = read_repo_file(rel);
        assert!(
            text.ends_with('\n'),
            "`{rel}` must end with exactly one trailing newline"
        );
        assert!(
            !text.to_ascii_lowercase().contains("gemini"),
            "`{rel}` still references the superseded panel model binding"
        );
        let root = repo_root();
        let root_str = root.to_string_lossy();
        assert!(
            !text.contains(root_str.as_ref()),
            "`{rel}` embeds the absolute checkout path; generated artifacts must be portable"
        );
        for host_path in ["/nix/store/", "/run/user/", "/var/tmp/"] {
            assert!(
                !text.contains(host_path),
                "`{rel}` leaks the host path `{host_path}`; generated artifacts must be portable"
            );
        }
    }
    let graph = load(GRAPH_JSON);
    let md = read_repo_file(GRAPH_MD);
    for (metric, key) in [
        ("Spec nodes", "specNodes"),
        ("Work-item nodes", "workItemNodes"),
        ("Total nodes", "nodes"),
        ("Edges", "edges"),
    ] {
        let expected = format!("| {metric} | {} |", graph["counts"][key].as_u64().unwrap());
        assert!(
            md.contains(&expected),
            "`{GRAPH_MD}` is out of sync with the JSON counts; expected `{expected}`"
        );
    }
}

#[test]
fn every_mermaid_node_id_is_a_valid_identifier() {
    let md = read_repo_file(GRAPH_MD);
    let mermaid = md
        .split("```mermaid")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("the graph document renders a mermaid block");
    for line in mermaid.lines() {
        let line = line.trim();
        let Some(id) = line.split(['[', ' ']).next() else {
            continue;
        };
        if id.is_empty()
            || line.starts_with("flowchart")
            || line.starts_with("subgraph")
            || line.starts_with("end")
            || line.starts_with("W0 -->")
        {
            continue;
        }
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && id.starts_with(|c: char| c.is_ascii_alphabetic()),
            "mermaid node id `{id}` is not a valid identifier"
        );
    }
}

#[test]
fn the_drift_gate_regenerates_every_adr046_artifact() {
    let gate = read_repo_file("tests/unit/gates/drift-check.sh");
    for needle in ["run_xtask spec-registry", "run_xtask implementation-graph"] {
        assert!(
            gate.contains(needle),
            "tests/unit/gates/drift-check.sh must run `{needle}`"
        );
    }
    for rel in [SPEC_SET, WORK_ITEMS, GRAPH_JSON, GRAPH_MD] {
        assert!(
            gate.contains(rel),
            "tests/unit/gates/drift-check.sh must diff `{rel}`"
        );
    }
}

// ---------------------------------------------------------------------------
// Negative fixtures
// ---------------------------------------------------------------------------

#[cfg(test)]
mod fixtures {
    use super::*;

    const SPEC: &str = "ADR-046-fixture";

    fn table(id: &str, extra: &[(&str, &str)]) -> String {
        let mut rows = vec![
            ("Work item ID", format!("`{id}`")),
            ("Dependency/owner", "fixture owner".to_string()),
            ("Current source", "none".to_string()),
            ("Reuse action", "create".to_string()),
            ("Destination", "`packages/fixture/src/lib.rs`".to_string()),
            ("Detailed design", "fixture design".to_string()),
            ("Integration", "fixture integration".to_string()),
            ("Data migration", "None".to_string()),
            ("Validation", "fixture validation".to_string()),
            ("Removal proof", "Not applicable".to_string()),
        ];
        for (label, value) in extra {
            rows.push(((*label), (*value).to_string()));
        }
        let body: String = rows
            .iter()
            .map(|(label, value)| format!("| {label} | {value} |\n"))
            .collect();
        format!("| Field | Value |\n| --- | --- |\n{body}")
    }

    fn markdown(heading: &str, id: &str, extra: &[(&str, &str)]) -> String {
        format!("{heading} {id}: fixture\n\n{}\n", table(id, extra))
    }

    fn manifest_row(id: &str, action: &str, reuse: Value) -> Value {
        serde_json::json!({
            "currentSource": "none",
            "dataMigration": "None",
            "dependencyOwner": "fixture owner",
            "destination": "`packages/fixture/src/lib.rs`",
            "detailedDesign": "fixture design",
            "integration": "fixture integration",
            "removalProof": "Not applicable",
            "reuseAction": action,
            "reuseSource": reuse,
            "specId": SPEC,
            "specPath": "docs/specs/ADR-046-fixture.md",
            "validation": "fixture validation",
            "workItemId": id,
        })
    }

    fn registry(prefixes: &[&str]) -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([(
            SPEC.to_string(),
            prefixes.iter().map(|p| (*p).to_string()).collect(),
        )])
    }

    fn declared(markdown: &str) -> BTreeMap<String, Vec<Declaration>> {
        BTreeMap::from([(SPEC.to_string(), declarations(markdown))])
    }

    fn run(md: &str, rows: Vec<Value>, prefixes: &[&str]) -> Vec<String> {
        check_work_items(&declared(md), &rows, &registry(prefixes))
    }

    #[test]
    fn the_happy_fixture_passes() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let rows = vec![manifest_row("ADR046-fixture-001", "create", Value::Null)];
        assert!(run(&md, rows, &["fixture"]).is_empty());
    }

    #[test]
    fn a_dropped_heading_fails() {
        let rows = vec![manifest_row("ADR046-fixture-001", "create", Value::Null)];
        let findings = run("# Fixture\n", rows, &["fixture"]);
        assert!(findings.iter().any(|f| f.contains("declared nowhere")));
    }

    #[test]
    fn a_level_two_or_four_item_heading_fails() {
        for heading in ["##", "####"] {
            let md = markdown(heading, "ADR046-fixture-001", &[]);
            let rows = vec![manifest_row("ADR046-fixture-001", "create", Value::Null)];
            let findings = run(&md, rows, &["fixture"]);
            assert!(
                findings.iter().any(|f| f.contains("only `###` is valid")),
                "{heading} heading must be rejected, got {findings:?}"
            );
        }
    }

    #[test]
    fn an_extra_manifest_row_fails() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let rows = vec![
            manifest_row("ADR046-fixture-001", "create", Value::Null),
            manifest_row("ADR046-fixture-002", "create", Value::Null),
        ];
        let findings = run(&md, rows, &["fixture"]);
        assert!(findings.iter().any(|f| f.contains("declared nowhere")));
    }

    #[test]
    fn a_duplicate_id_fails() {
        let md = format!(
            "{}\n{}",
            markdown("###", "ADR046-fixture-001", &[]),
            markdown("###", "ADR046-fixture-001", &[])
        );
        let rows = vec![
            manifest_row("ADR046-fixture-001", "create", Value::Null),
            manifest_row("ADR046-fixture-001", "create", Value::Null),
        ];
        let findings = run(&md, rows, &["fixture"]);
        assert!(findings.iter().any(|f| f.contains("declared twice")));
        assert!(findings.iter().any(|f| f.contains("more than once")));
    }

    #[test]
    fn a_prefix_claimed_by_two_members_fails() {
        let registry = BTreeMap::from([
            (SPEC.to_string(), vec!["fixture".to_string()]),
            ("ADR-046-other".to_string(), vec!["fixture".to_string()]),
        ]);
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let findings = check_work_items(
            &declared(&md),
            &[manifest_row("ADR046-fixture-001", "create", Value::Null)],
            &registry,
        );
        assert!(findings.iter().any(|f| f.contains("registered to both")));
    }

    #[test]
    fn an_unsorted_or_empty_prefix_registry_fails() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let rows = vec![manifest_row("ADR046-fixture-001", "create", Value::Null)];
        let unsorted = check_work_items(
            &declared(&md),
            &rows,
            &BTreeMap::from([(
                SPEC.to_string(),
                vec!["zulu".to_string(), "fixture".to_string()],
            )]),
        );
        assert!(unsorted.iter().any(|f| f.contains("unsorted")));

        let empty = run(&md, rows, &[]);
        assert!(empty.iter().any(|f| f.contains("registers no")));
    }

    #[test]
    fn an_unregistered_or_heuristic_prefix_match_fails() {
        let md = markdown("###", "ADR046-fixture-extra-001", &[]);
        let rows = vec![manifest_row(
            "ADR046-fixture-extra-001",
            "create",
            Value::Null,
        )];
        let findings = run(&md, rows, &["fixture"]);
        assert!(
            findings
                .iter()
                .any(|f| f.contains("unregistered prefix `fixture-extra`")),
            "a longest-prefix heuristic must not resolve `fixture-extra` to `fixture`: {findings:?}"
        );
    }

    #[test]
    fn a_wrong_owner_record_fails() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let mut row = manifest_row("ADR046-fixture-001", "create", Value::Null);
        row["specId"] = Value::String("ADR-046-other".to_string());
        let findings = run(&md, vec![row], &["fixture"]);
        assert!(findings.iter().any(|f| f.contains("records owner")));
    }

    #[test]
    fn a_two_digit_or_zero_ordinal_is_not_a_work_item() {
        assert!(split_id("ADR046-fixture-01").is_none());
        assert!(split_id("ADR046-fixture-0001").is_none());
        assert!(split_id("ADR046-fixture-000").is_none());
        assert!(split_id("ADR046-fixture-001").is_some());
        // A malformed heading declares nothing, so the manifest row is orphaned.
        let md = markdown("###", "ADR046-fixture-01", &[]);
        let rows = vec![manifest_row("ADR046-fixture-01", "create", Value::Null)];
        let findings = run(&md, rows, &["fixture"]);
        assert!(findings.iter().any(|f| f.contains("declared nowhere")));
    }

    #[test]
    fn a_missing_or_duplicated_mandatory_field_fails() {
        let md = "### ADR046-fixture-001: fixture\n\n| Field | Value |\n| --- | --- |\n| Current source | none |\n";
        let rows = vec![manifest_row("ADR046-fixture-001", "create", Value::Null)];
        let missing = run(md, rows.clone(), &["fixture"]);
        assert!(
            missing
                .iter()
                .any(|f| f.contains("missing mandatory field"))
        );

        let duplicated = markdown("###", "ADR046-fixture-001", &[("Validation", "again")]);
        let findings = run(&duplicated, rows, &["fixture"]);
        assert!(
            findings
                .iter()
                .any(|f| f.contains("declares `Validation` 2 times"))
        );
    }

    #[test]
    fn a_free_form_or_compound_action_fails() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        for action in ["extract and adapt", "refactor"] {
            let rows = vec![manifest_row("ADR046-fixture-001", action, Value::Null)];
            let findings = run(&md, rows, &["fixture"]);
            assert!(
                findings
                    .iter()
                    .any(|f| f.contains("free-form reuse action")),
                "`{action}` must be rejected"
            );
        }
    }

    #[test]
    fn create_with_a_reuse_source_fails() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let rows = vec![manifest_row(
            "ADR046-fixture-001",
            "create",
            Value::String("packages/d2b/src/lib.rs".to_string()),
        )];
        let findings = run(&md, rows, &["fixture"]);
        assert!(findings.iter().any(|f| f.contains("with a reuse source")));
    }

    fn node(id: &str, wave: &str, group: &str, prerequisites: &[&str]) -> Value {
        serde_json::json!({
            "id": id,
            "wave": wave,
            "parallelGroup": group,
            "prerequisites": prerequisites,
        })
    }

    fn graph(nodes: Vec<Value>, edges: Vec<Value>) -> Value {
        serde_json::json!({ "nodes": nodes, "edges": edges })
    }

    fn ids(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn a_dangling_dependency_fails() {
        let graph = graph(vec![node("a", "W0", "g", &["missing"])], vec![]);
        let findings = check_graph(&graph, &ids(&["a"]));
        assert!(findings.iter().any(|f| f.contains("unresolved `missing`")));
    }

    #[test]
    fn a_cyclic_dag_fails() {
        let graph = graph(
            vec![node("a", "W0", "g", &["b"]), node("b", "W0", "g", &["a"])],
            vec![],
        );
        let findings = check_graph(&graph, &ids(&["a", "b"]));
        assert!(findings.iter().any(|f| f.contains("dependency cycle")));
    }

    #[test]
    fn a_backward_wave_dependency_fails() {
        let graph = graph(
            vec![node("a", "W0", "g0", &["b"]), node("b", "W1", "g1", &[])],
            vec![],
        );
        let findings = check_graph(&graph, &ids(&["a", "b"]));
        assert!(findings.iter().any(|f| f.contains("in the later W1")));
    }

    #[test]
    fn a_cross_wave_parallel_group_fails() {
        let graph = graph(
            vec![
                node("a", "W0", "shared", &[]),
                node("b", "W1", "shared", &[]),
            ],
            vec![],
        );
        let findings = check_graph(&graph, &ids(&["a", "b"]));
        assert!(findings.iter().any(|f| f.contains("spans W0 and W1")));
    }

    #[test]
    fn an_unresolved_edge_endpoint_fails() {
        let graph = graph(
            vec![node("a", "W0", "g", &[])],
            vec![serde_json::json!({ "from": "a", "to": "ghost", "type": "spec-depends-on" })],
        );
        let findings = check_graph(&graph, &ids(&["a"]));
        assert!(findings.iter().any(|f| f.contains("resolves to no node")));
    }

    #[test]
    fn a_missing_or_extra_graph_node_fails() {
        let missing = check_graph(&graph(vec![], vec![]), &ids(&["a"]));
        assert!(missing.iter().any(|f| f.contains("missing node `a`")));
        let extra = check_graph(
            &graph(vec![node("a", "W0", "g", &[])], vec![]),
            &BTreeSet::new(),
        );
        assert!(extra.iter().any(|f| f.contains("unexpected node `a`")));
    }

    #[test]
    fn the_local_digest_matches_known_sha256_vectors() {
        assert_eq!(
            hex_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
