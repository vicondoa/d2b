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

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use d2b_contract_tests::{read_repo_file, repo_root};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// A mutation applied to a serialized work-item row in a negative fixture.
type MutateRow = fn(&mut Value);

const SPEC_SET: &str = "docs/specs/ADR-046-spec-set.json";
const WORK_ITEMS: &str = "docs/specs/ADR-046-work-items.json";
const GRAPH_JSON: &str = "docs/specs/ADR-046-implementation-graph.json";
const GRAPH_MD: &str = "docs/specs/ADR-046-implementation-graph.md";
const FEATURE_TASKS: &str = "specs/001-adr046-d2b3-completion/tasks.md";
/// SHA-256 of the compact, recursively key-sorted JSON contract in `tasks.md`.
/// The pin keeps the full contract exact without copying its long ownership
/// arrays into this policy.
const FEATURE_TASK_CONTRACT_SHA256: &str =
    "acd74e4458c8dcb167fb952678ae24094b5b252cd197a2973ac81d0b89b26353";

const EXPECTED_LOCAL_TASK_IDS: &[&str] = &["T606", "T607", "T608", "T609", "T604", "T479", "T480"];
const EXPECTED_PERMITTED_LOCAL_DEPENDENCY_IDS: &[&str] = &[
    "T221", "T606", "T607", "T608", "T609", "T604", "T479", "T480",
];
const EXPECTED_OWNED_TASK_IDS: &[&str] = &["T606", "T607", "T608", "T609", "T604", "T479"];
const EXPECTED_LOCAL_GROUP_IDS: &[&str] = &[
    "feature-local:w6-shared-prep",
    "feature-local:w6-core-control-foundations",
    "feature-local:w6-storage-authority-foundations",
    "feature-local:w6-audit-telemetry-foundations",
    "feature-local:w6-operator-acceptance",
    "feature-local:w6-converge",
    "feature-local:w6-close",
];
const EXPECTED_W6_WORK_ITEMS: usize = 258;
const EXPECTED_W6_MANIFEST_GROUPS: usize = 29;
const EXPECTED_W6_PROVIDER_GROUPS: usize = 27;
const EXPECTED_POST_ENTRY_GROUPS: usize = 36;
const EXPECTED_POST_ENTRY_RECORDS: usize = 265;
const EXPECTED_TASKS: usize = 609;
const EXPECTED_PARALLEL_TASKS: usize = 101;
const EXPECTED_MANIFEST_MERGED: usize = 68;
const EXPECTED_MANIFEST_PLANNED: usize = 477;

/// The normative member count, per `docs/specs/README.md`.
const EXPECTED_MEMBERS: usize = 55;
/// The normative work-item count. The corpus is closed; a parser or source
/// regression that changes it must fail rather than shrink the manifests.
const EXPECTED_WORK_ITEMS: usize = 545;
/// The certified graph shape. Pinned so a silent edge gain or loss fails here
/// even when the generator regenerates itself consistently.
const EXPECTED_NODES: u64 = 600;
const EXPECTED_EDGES: u64 = 1963;
const EXPECTED_MAX_RANK: u64 = 22;
const EXPECTED_WAVES: u64 = 8;
const EXPECTED_CRITICAL_PATH: usize = 23;
const EXPECTED_WORK_ITEMS_SCHEMA: u64 = 2;

const REUSE_ACTIONS: &[&str] = &[
    "adapt",
    "copy-unchanged",
    "create",
    "delete-after-cutover",
    "extract",
    "replace",
    "wrap",
];

const IMPLEMENTATION_STATES: &[&str] = &["Merged", "Planned"];

const MANDATORY_FIELDS: &[&str] = &[
    "Current source",
    "Data migration",
    "Dependency/owner",
    "Destination",
    "Detailed design",
    "Evidence",
    "Implementation state",
    "Integration",
    "Removal proof",
    "Reuse action",
    "Validation",
];

struct NoDuplicateValue;

impl<'de> DeserializeSeed<'de> for NoDuplicateValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateVisitor)
    }
}

struct NoDuplicateVisitor;

impl<'de> Visitor<'de> for NoDuplicateVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        NoDuplicateValue.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(NoDuplicateValue)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!(
                    "duplicate JSON object key `{key}`"
                )));
            }
            values.insert(key, object.next_value_seed(NoDuplicateValue)?);
        }
        Ok(Value::Object(values))
    }
}

fn parse_json_without_duplicates(source: &str) -> Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_str(source);
    let value = NoDuplicateValue
        .deserialize(&mut deserializer)
        .map_err(|error| error.to_string())?;
    deserializer.end().map_err(|error| error.to_string())?;
    Ok(value)
}

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
        "dash title"
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

/// Returns the work-item id a heading declares, anchored on the id grammar
/// rather than on whatever punctuation introduces the title. Work-item ids
/// contain hyphens, so a parser that splits the heading on a separator
/// truncates every id whose prefix has more than one segment.
fn leading_id(rest: &str) -> Option<String> {
    let text = rest.trim_start_matches('`');
    if !text.starts_with("ADR046-") {
        return None;
    }
    let body: String = text
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    body.match_indices('-')
        .map(|(index, _)| index)
        .chain(std::iter::once(body.len()))
        .map(|cut| body[..cut].to_string())
        .find(|candidate| split_id(candidate).is_some())
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
        let Some(token) = leading_id(rest) else {
            index += 1;
            continue;
        };
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
    spec_paths: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut findings = Vec::new();

    // Index the manifest by id so each Markdown declaration can be compared
    // field-for-field against its serialized row.
    let manifest_by_id: BTreeMap<&str, &Value> = manifest
        .iter()
        .filter_map(|entry| entry["workItemId"].as_str().map(|id| (id, entry)))
        .collect();

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
            if let Some((_, state)) = item
                .fields
                .iter()
                .find(|(label, _)| label == "Implementation state")
                && !IMPLEMENTATION_STATES.contains(&state.as_str())
            {
                findings.push(format!(
                    "work item `{}` declares free-form implementation state `{state}`",
                    item.id
                ));
            }

            if let Some(entry) = manifest_by_id.get(item.id.as_str()) {
                findings.extend(field_findings(item, spec_paths.get(spec_id), entry));
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
        let implementation_state = entry["implementationState"].as_str().unwrap_or_default();
        if !IMPLEMENTATION_STATES.contains(&implementation_state) {
            findings.push(format!(
                "`{id}` manifest declares free-form implementation state `{implementation_state}`"
            ));
        }
        if entry["evidence"].as_str().is_none_or(str::is_empty) {
            findings.push(format!("`{id}` manifest has missing or empty evidence"));
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

/// Mirrors the generator's `None`-sentinel rule so an empty reuse source is
/// recognized identically on both sides.
fn is_none_sentinel(value: &str) -> bool {
    value == "None"
        || value
            .strip_prefix("None")
            .is_some_and(|rest| rest.starts_with('.') || rest.starts_with(','))
}

/// Independently derives the row every serialized field should carry from a
/// Markdown declaration and reports each field whose manifest value disagrees.
///
/// The ID-membership and per-item structural rules above never look at the
/// *values* the generator wrote, so a generator regression that rewrites
/// `dependencyOwner`, `destination`, `detailedDesign`, `validation`,
/// `reuseAction`, `reuseSource`, or the member metadata would regenerate
/// cleanly and pass. This closes that hole by comparing every field.
fn field_findings(item: &Declaration, spec_path: Option<&String>, entry: &Value) -> Vec<String> {
    let mut findings = Vec::new();
    let id = &item.id;

    // Mirror the generator's `BTreeMap` semantics: a repeated label keeps the
    // last value. A duplicate is already reported as its own finding.
    let mut declared_fields: BTreeMap<&str, &str> = BTreeMap::new();
    for (label, value) in &item.fields {
        declared_fields.insert(label.as_str(), value.as_str());
    }

    const SCALARS: &[(&str, &str)] = &[
        ("currentSource", "Current source"),
        ("dataMigration", "Data migration"),
        ("dependencyOwner", "Dependency/owner"),
        ("destination", "Destination"),
        ("detailedDesign", "Detailed design"),
        ("evidence", "Evidence"),
        ("implementationState", "Implementation state"),
        ("integration", "Integration"),
        ("removalProof", "Removal proof"),
        ("reuseAction", "Reuse action"),
        ("validation", "Validation"),
    ];
    for (key, label) in SCALARS {
        let Some(expected) = declared_fields.get(label) else {
            continue;
        };
        let actual = entry.get(*key).and_then(Value::as_str).unwrap_or_default();
        if actual != *expected {
            findings.push(format!(
                "`{id}` manifest `{key}` is `{actual}` but its Markdown declares `{expected}`"
            ));
        }
    }

    // reuseSource is optional: an absent or `None`-sentinel field serializes as
    // JSON null; anything else must round-trip verbatim.
    let expected_source: Option<&str> = match declared_fields.get("Reuse source") {
        Some(value) if !is_none_sentinel(value) => Some(value),
        _ => None,
    };
    let actual_source = entry.get("reuseSource");
    let source_ok = match (expected_source, actual_source) {
        (None, None) => true,
        (None, Some(value)) => value.is_null(),
        (Some(expected), Some(value)) => value.as_str() == Some(expected),
        (Some(_), None) => false,
    };
    if !source_ok {
        let rendered = actual_source
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_else(|| "absent".to_string());
        let want = expected_source.unwrap_or("none");
        findings.push(format!(
            "`{id}` manifest reuseSource is `{rendered}` but its Markdown declares `{want}`"
        ));
    }

    // Member metadata: the row must carry the owning member's declared path.
    if let Some(expected_path) = spec_path {
        let actual_path = entry
            .get("specPath")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if actual_path != expected_path.as_str() {
            findings.push(format!(
                "`{id}` manifest specPath is `{actual_path}` but its member path is `{expected_path}`"
            ));
        }
    }

    findings
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
    parse_json_without_duplicates(&read_repo_file(rel))
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

struct MarkdownJsonFence {
    body: String,
    closed: bool,
}

fn markdown_fence(line: &str) -> Option<(char, usize, &str)> {
    let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
    if indentation > 3 || line.as_bytes().get(indentation) == Some(&b'\t') {
        return None;
    }
    let trimmed = &line[indentation..];
    let delimiter = trimmed.chars().next()?;
    if !matches!(delimiter, '`' | '~') {
        return None;
    }
    let length = trimmed
        .chars()
        .take_while(|char| *char == delimiter)
        .count();
    (length >= 3).then(|| (delimiter, length, &trimmed[length..]))
}

fn is_json_fence_info(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("json"))
}

fn markdown_json_fences(markdown: &str) -> Vec<MarkdownJsonFence> {
    let mut bodies = Vec::new();
    let mut body = String::new();
    let mut open_fence = None;

    for line in markdown.lines() {
        if let Some((delimiter, length)) = open_fence {
            let closes_fence = markdown_fence(line).is_some_and(
                |(candidate_delimiter, candidate_length, remainder)| {
                    candidate_delimiter == delimiter
                        && candidate_length >= length
                        && remainder.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
                },
            );
            if closes_fence {
                bodies.push(MarkdownJsonFence {
                    body: std::mem::take(&mut body),
                    closed: true,
                });
                open_fence = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
        } else if let Some((delimiter, length, info)) = markdown_fence(line)
            && is_json_fence_info(info)
        {
            open_fence = Some((delimiter, length));
            body.clear();
        }
    }
    if open_fence.is_some() {
        bodies.push(MarkdownJsonFence {
            body,
            closed: false,
        });
    }

    bodies
}

fn canonical_json(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => b"null".to_vec(),
        Value::Bool(value) => value.to_string().into_bytes(),
        Value::Number(value) => value.to_string().into_bytes(),
        Value::String(value) => {
            serde_json::to_vec(value).expect("JSON strings are always serializable")
        }
        Value::Array(values) => {
            let mut output = vec![b'['];
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                output.extend(canonical_json(value));
            }
            output.push(b']');
            output
        }
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));

            let mut output = vec![b'{'];
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                output.extend(
                    serde_json::to_vec(key).expect("JSON object keys are always serializable"),
                );
                output.push(b':');
                output.extend(canonical_json(value));
            }
            output.push(b'}');
            output
        }
    }
}

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for component in path {
        current = current.get(*component)?;
    }
    Some(current)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn string_array_has_duplicates(value: Option<&Value>) -> bool {
    let values = string_array(value);
    values.iter().collect::<BTreeSet<_>>().len() != values.len()
}

fn expected_strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn string_set(values: &[&str]) -> BTreeSet<String> {
    expected_strings(values).into_iter().collect()
}

fn object_keys(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_object)
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

fn expect_contract_value(
    contract: &Value,
    path: &[&str],
    expected: Value,
    label: &str,
    findings: &mut Vec<String>,
) {
    if value_at(contract, path) != Some(&expected) {
        findings.push(format!("feature-local contract {label} is incorrect"));
    }
}

fn expect_contract_string_array(
    contract: &Value,
    path: &[&str],
    expected: &[&str],
    label: &str,
    findings: &mut Vec<String>,
) {
    if string_array(value_at(contract, path)) != expected_strings(expected) {
        findings.push(format!("feature-local contract {label} is incorrect"));
    }
}

fn expected_local_dependencies() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        ("T606", vec!["T221"]),
        ("T607", vec!["T606"]),
        ("T608", vec!["T606"]),
        ("T609", vec!["T606"]),
        ("T604", vec!["T221", "T607", "T608", "T609"]),
        ("T479", vec!["T604", "T221"]),
        ("T480", vec!["T479"]),
    ])
}

fn expected_historical_foundation_adoption() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        (
            "T607",
            vec![
                "ADR046-cli-001",
                "ADR046-cli-009",
                "ADR046-exec-003",
                "ADR046-exec-004",
                "ADR046-exec-005",
                "ADR046-nix-003",
                "ADR046-zone-control-001",
            ],
        ),
        (
            "T608",
            vec![
                "ADR046-volume-001",
                "ADR046-volume-002",
                "ADR046-volume-004",
                "ADR046-zone-control-019",
                "ADR046-zone-control-020",
                "ADR046-zone-control-024",
            ],
        ),
        ("T609", vec!["ADR046-audit-001", "ADR046-telem-001"]),
    ])
}

fn expected_shared_file_order() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        ("Makefile", vec!["T606", "ADR046-ch-001", "T604"]),
        ("packages/Cargo.toml", vec!["T606", "T479"]),
        ("packages/Cargo.lock", vec!["T606", "T479"]),
        ("packages/d2b-priv-broker/Cargo.toml", vec!["T606", "T479"]),
        ("packages/d2b-priv-broker/Cargo.lock", vec!["T606", "T479"]),
        ("flake.nix", vec!["T606"]),
        ("packages/d2b-contracts/src/broker_wire.rs", vec!["T606"]),
        ("packages/d2b-priv-broker/src/runtime.rs", vec!["T606"]),
        ("packages/d2bd/src/lib.rs", vec!["T606"]),
        ("packages/d2b/src/lib.rs", vec!["T606"]),
    ])
}

fn expected_t604_manifest_dependencies() -> Vec<String> {
    let mut dependencies = vec![
        "ADR046-activation-001".to_owned(),
        "ADR046-activation-006".to_owned(),
        "ADR046-system-core-001".to_owned(),
        "ADR046-ch-001".to_owned(),
    ];
    dependencies.extend((1..=20).map(|ordinal| format!("ADR046-nl-{ordinal:03}")));
    dependencies.extend((1..=13).map(|ordinal| format!("ADR046-device-tpm-{ordinal:03}")));
    dependencies.extend((1..=13).map(|ordinal| format!("ADR046-vl-{ordinal:03}")));
    dependencies
}

fn expected_local_completion_evidence() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        (
            "T606",
            vec![
                "w6-shared-prep-inventory",
                "w6-shared-prep-shared-writers",
                "w6-shared-prep-lockfile-flake-packages",
            ],
        ),
        (
            "T607",
            vec![
                "w6-core-control-production-route",
                "w6-real-so-peercred-admission",
            ],
        ),
        (
            "T608",
            vec![
                "w6-typed-broker-host-effects",
                "w6-strict-resource-nix-validation",
                "w6-tpm-legacy-migration",
                "w6-host-global-authority",
            ],
        ),
        (
            "T609",
            vec![
                "w6-transactional-privileged-audit",
                "w6-forbidden-identity-redaction",
                "w6-bounded-telemetry",
                "w6-closed-metric-descriptors",
            ],
        ),
        (
            "T604",
            [
                "operator-nix-activation-cleanup-development",
                "daemon-restart-vm-survival-development",
            ]
            .to_vec(),
        ),
        (
            "T479",
            [
                "operator-nix-activation-cleanup",
                "w6-cloud-hypervisor-guest-acceptance",
                "w6-final-cargo-locks",
                "w6-changelog-fold",
            ]
            .to_vec(),
        ),
        (
            "T480",
            [
                "w6-binding-panel-unanimous",
                "w6-protected-merge",
                "w6-post-merge-seal",
                "w6-merge-eligibility",
            ]
            .to_vec(),
        ),
    ])
}

fn expected_scaffold_handoffs() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "packages/d2b-provider-activation-nixos/",
            "wi:ADR-046-provider-activation-nixos",
        ),
        (
            "packages/d2b-provider-audio-pipewire/",
            "wi:ADR-046-provider-audio-pipewire",
        ),
        (
            "packages/d2b-provider-clipboard-wayland/",
            "wi:ADR-046-provider-clipboard-wayland",
        ),
        (
            "packages/d2b-provider-display-wayland/",
            "wi:ADR-046-provider-display-wayland",
        ),
        (
            "packages/d2b-provider-notification-desktop/",
            "wi:ADR-046-provider-notification-desktop",
        ),
        (
            "packages/d2b-provider-runtime-azure-container-apps/",
            "wi:ADR-046-provider-runtime-azure-container-apps",
        ),
        (
            "packages/d2b-provider-runtime-azure-virtual-machine/",
            "wi:ADR-046-provider-runtime-azure-virtual-machine",
        ),
        (
            "packages/d2b-provider-runtime-cloud-hypervisor/",
            "wi:ADR-046-provider-runtime-cloud-hypervisor",
        ),
        (
            "packages/d2b-provider-runtime-qemu-media/",
            "wi:ADR-046-provider-runtime-qemu-media",
        ),
        (
            "packages/d2b-provider-shell-terminal/",
            "wi:ADR-046-provider-shell-terminal",
        ),
        (
            "packages/d2b-provider-transport-azure-relay/",
            "wi:ADR-046-provider-transport-azure-relay",
        ),
        (
            "packages/d2b-provider-transport-unix/",
            "wi:ADR-046-provider-transport-unix",
        ),
        (
            "packages/d2b-provider-transport-vsock/",
            "wi:ADR-046-provider-transport-vsock",
        ),
    ])
}

fn expected_local_task_label_prefixes() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("T606", "T606 [US2] **FEATURE-LOCAL FOUNDATION/COMPLETION -"),
        (
            "T607",
            "T607 [P] [US2] **FEATURE-LOCAL FOUNDATION/COMPLETION - PROSPECTIVELY",
        ),
        (
            "T608",
            "T608 [P] [US2] **FEATURE-LOCAL FOUNDATION/COMPLETION - PROSPECTIVELY",
        ),
        (
            "T609",
            "T609 [P] [US2] **FEATURE-LOCAL FOUNDATION/COMPLETION - PROSPECTIVELY",
        ),
        (
            "T604",
            "T604 [US1] **FEATURE-LOCAL COORDINATION/COMPLETION - author and development-validate operator activation and daemon-restart acceptance.**",
        ),
        (
            "T479",
            "T479 [US2] FEATURE-LOCAL COORDINATION/COMPLETION - W6 CONVERGE + FREEZE + OPERATOR/GUEST ACCEPTANCE -",
        ),
        (
            "T480",
            "T480 [US2] FEATURE-LOCAL COORDINATION/COMPLETION - W6 SINGLE BINDING WORK GATE + MERGE -",
        ),
    ])
}

fn split_top_level(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ';' | ',' if depth == 0 => {
                parts.push(text[start..index].to_owned());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(text[start..].to_owned());
    parts
}

fn expand_destination_braces(text: &str) -> Vec<String> {
    let Some(open) = text.find('{') else {
        return vec![text.to_owned()];
    };
    let mut depth = 0usize;
    let mut close = None;
    for (offset, character) in text[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![text.to_owned()];
    };

    let alternatives = split_top_level(&text[open + 1..close]);
    alternatives
        .into_iter()
        .flat_map(|alternative| {
            expand_destination_braces(&format!(
                "{}{}{}",
                &text[..open],
                alternative,
                &text[close + 1..]
            ))
        })
        .collect()
}

fn trim_destination_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|character: char| {
            matches!(
                character,
                '`' | '\'' | '"' | '(' | ')' | '[' | ']' | ',' | ';' | '.' | ':'
            )
        })
        .trim_start_matches("./")
        .to_owned()
}

fn is_path_like(token: &str) -> bool {
    token == "Makefile"
        || token == "flake.nix"
        || [
            "packages/",
            "nixos-modules/",
            "tests/",
            "integration/",
            "examples/",
            "templates/",
            "docs/",
            "proofs/",
            "changelog.d/",
        ]
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

fn is_provider_relative_path(token: &str) -> bool {
    token.starts_with("src/")
        || token.starts_with("tests/")
        || token.starts_with("integration/")
        || matches!(token, "README.md" | "Cargo.toml" | "Cargo.lock")
}

fn is_ignored_destination_root(token: &str) -> bool {
    matches!(
        token,
        "packages/" | "nixos-modules/" | "tests/" | "integration/"
    )
}

fn provider_root_for_group(group: Option<&str>) -> Option<String> {
    group
        .and_then(|group| group.strip_prefix("wi:ADR-046-provider-"))
        .map(|suffix| format!("packages/d2b-provider-{suffix}/"))
}

fn is_local_handoff_destination(destination: &str) -> bool {
    destination.trim_start().starts_with("T608-handoff")
}

fn canonical_destination_token(token: &str, provider_root: Option<&str>) -> Option<String> {
    let token = trim_destination_token(token);
    let token = if provider_root.is_some() && is_provider_relative_path(&token) {
        format!("{}{}", provider_root.expect("provider root"), token)
    } else if is_path_like(&token) {
        token
    } else {
        return None;
    };
    (!token.is_empty() && !is_ignored_destination_root(&token)).then_some(token)
}

/// Expand a manifest destination to canonical repository-relative paths.
///
/// Destination rows mix repository paths, brace groups, globs, and paths
/// relative to the Provider package named by the graph group. Keep globs
/// as patterns: dropping them loses the parent/child overlap that protects a
/// local file from an apparently unrelated `src/*` or Provider-relative row.
fn normalized_destination_atoms(
    destination: &str,
    provider_root: Option<&str>,
) -> BTreeSet<String> {
    let preserves_directory_roots = destination.to_ascii_lowercase().contains("scaffold");
    let mut candidates = Vec::new();
    let mut code = String::new();
    let mut in_code = false;
    for character in destination.chars() {
        if character == '`' {
            if in_code {
                candidates.extend(split_top_level(&code));
                code.clear();
            }
            in_code = !in_code;
        } else if in_code {
            code.push(character);
        }
    }

    // Some generated rows intentionally keep a path outside code spans (for
    // example the short Nix and broker operation rows). Include only tokens
    // anchored at a repository path root or at the selected Provider package
    // root so surrounding prose cannot become a false destination.
    for token in destination.split_whitespace() {
        let token = trim_destination_token(token);
        if is_path_like(&token) || provider_root.is_some_and(|_| is_provider_relative_path(&token))
        {
            candidates.push(token);
        }
    }

    let mut atoms = BTreeSet::new();
    for candidate in candidates {
        for part in split_top_level(&candidate) {
            for expanded in expand_destination_braces(&part) {
                if let Some(token) = canonical_destination_token(&expanded, provider_root) {
                    if !token.ends_with('/') || preserves_directory_roots {
                        atoms.insert(token);
                    }
                }
            }
        }
    }
    atoms
}

fn path_has_glob(path: &str) -> bool {
    path.contains('*') || path.contains("...") || path.contains('<')
}

fn glob_fixed_prefix(path: &str) -> Option<&str> {
    let first = ["*", "...", "<"]
        .iter()
        .filter_map(|marker| path.find(marker))
        .min()?;
    let prefix = &path[..first];
    (!prefix.is_empty()).then_some(prefix)
}

fn path_is_parent(parent: &str, child: &str) -> bool {
    parent.ends_with('/') && child.starts_with(parent)
}

/// Overlap is intentionally symmetric. A destination may be a parent of an
/// owned file, a child of an owned directory, or a glob whose fixed prefix
/// contains either side. The old one-way check silently dropped all three
/// cases.
fn destination_paths_overlap(left: &str, right: &str) -> bool {
    if left == right || path_is_parent(left, right) || path_is_parent(right, left) {
        return true;
    }
    if path_has_glob(left) {
        let Some(prefix) = glob_fixed_prefix(left) else {
            return false;
        };
        let other = glob_fixed_prefix(right).unwrap_or(right);
        if other.starts_with(prefix) || prefix.starts_with(other) {
            return true;
        }
    }
    if path_has_glob(right) {
        let Some(prefix) = glob_fixed_prefix(right) else {
            return false;
        };
        let other = glob_fixed_prefix(left).unwrap_or(left);
        if other.starts_with(prefix) || prefix.starts_with(other) {
            return true;
        }
    }
    false
}

fn local_path_overlaps_destination(local_path: &str, destination: &str) -> bool {
    destination_paths_overlap(local_path, destination)
}

fn local_owned_path_owners(contract: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let mut owners = BTreeMap::new();
    for task in EXPECTED_OWNED_TASK_IDS {
        if let Some(paths) = value_at(contract, &["owned_files", task]).and_then(Value::as_array) {
            for path in paths.iter().filter_map(Value::as_str) {
                owners
                    .entry(path.to_owned())
                    .or_insert_with(BTreeSet::new)
                    .insert((*task).to_owned());
            }
        }
    }
    owners
}

fn w6_manifest_groups(graph: &Value) -> BTreeSet<String> {
    graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["kind"] == "work-item" && node["wave"] == "W6")
        .filter_map(|node| node["parallelGroup"].as_str().map(str::to_owned))
        .collect()
}

fn is_sha256_hex(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|text| {
        text.len() == 64 && text.chars().all(|character| character.is_ascii_hexdigit())
    })
}

fn check_manifest_group_foundations(
    contract: &Value,
    graph: &Value,
    manifest: &Value,
    findings: &mut Vec<String>,
) {
    let groups = w6_manifest_groups(graph);
    if groups.len() != EXPECTED_W6_MANIFEST_GROUPS {
        findings.push(format!(
            "Wave 6 graph has {} manifest groups, expected {EXPECTED_W6_MANIFEST_GROUPS}",
            groups.len()
        ));
    }
    let provider_groups = groups
        .iter()
        .filter(|group| group.starts_with("wi:ADR-046-provider-"))
        .count();
    if provider_groups != EXPECTED_W6_PROVIDER_GROUPS {
        findings.push(format!(
            "Wave 6 graph has {provider_groups} Provider groups, expected {EXPECTED_W6_PROVIDER_GROUPS}"
        ));
    }

    let foundations = value_at(contract, &["manifest_group_foundations"]);
    if object_keys(foundations) != groups {
        findings.push(format!(
            "manifest_group_foundations keys do not equal the {EXPECTED_W6_MANIFEST_GROUPS} W6 graph groups"
        ));
    }
    let expected_foundations = expected_strings(&["T606", "T607", "T608", "T609"]);
    for group in &groups {
        if string_array(value_at(
            contract,
            &["manifest_group_foundations", group.as_str()],
        )) != expected_foundations
        {
            findings.push(format!(
                "manifest_group_foundations entry for `{group}` is not the exact T606/T607/T608/T609 substitution"
            ));
        }
    }

    let w6_nodes: Vec<&Value> = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["kind"] == "work-item" && node["wave"] == "W6")
        .collect();
    let mut w6_ids = BTreeSet::new();
    for node in &w6_nodes {
        let Some(id) = node["id"].as_str() else {
            findings.push("a W6 graph node has no workItemId".to_owned());
            continue;
        };
        if !w6_ids.insert(id.to_owned()) {
            findings.push(format!("W6 graph workItemId `{id}` appears more than once"));
        }
    }
    if w6_nodes.len() != EXPECTED_W6_WORK_ITEMS {
        findings.push(format!(
            "Wave 6 graph has {} work items, expected {EXPECTED_W6_WORK_ITEMS}",
            w6_nodes.len()
        ));
    }

    let mut manifest_by_id: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for item in manifest["items"].as_array().into_iter().flatten() {
        if let Some(id) = item["workItemId"].as_str() {
            manifest_by_id.entry(id).or_default().push(item);
        }
    }
    let mut w6_states = BTreeMap::<&str, usize>::new();
    for id in &w6_ids {
        match manifest_by_id.get(id.as_str()) {
            Some(items) if items.len() == 1 => {
                let item = items[0];
                let state = item["implementationState"].as_str().unwrap_or_default();
                *w6_states.entry(state).or_default() += 1;
                if state != "Planned" {
                    findings.push(format!(
                        "W6 manifest item `{id}` has state `{state}`, expected `Planned` at entry"
                    ));
                }
            }
            Some(items) => findings.push(format!(
                "W6 manifest item `{id}` resolves to {} rows instead of exactly one",
                items.len()
            )),
            None => findings.push(format!("W6 graph item `{id}` is absent from the manifest")),
        }
    }
    if w6_states != BTreeMap::from([("Planned", EXPECTED_W6_WORK_ITEMS)]) {
        findings.push(format!(
            "W6 manifest state census is {w6_states:?}, expected {EXPECTED_W6_WORK_ITEMS} Planned"
        ));
    }

    let mut post_entry_groups = groups;
    post_entry_groups.extend(
        EXPECTED_LOCAL_GROUP_IDS
            .iter()
            .map(|group| (*group).to_owned()),
    );
    if post_entry_groups.len() != EXPECTED_POST_ENTRY_GROUPS {
        findings.push(format!(
            "post-entry group census is {}, expected {EXPECTED_POST_ENTRY_GROUPS}",
            post_entry_groups.len()
        ));
    }
    if w6_ids.len() + EXPECTED_LOCAL_TASK_IDS.len() != EXPECTED_POST_ENTRY_RECORDS {
        findings.push(format!(
            "post-entry record census is {}, expected {EXPECTED_POST_ENTRY_RECORDS}",
            w6_ids.len() + EXPECTED_LOCAL_TASK_IDS.len()
        ));
    }
}

fn check_local_completion_contract(
    contract: &Value,
    graph: &Value,
    manifest: &Value,
    findings: &mut Vec<String>,
) {
    expect_contract_value(
        contract,
        &["adoption_substitution_semantics"],
        serde_json::json!({
            "scope": "W6 readiness and local completion only",
            "substitutes_execution_and_validation_obligation": true,
            "substitutes_manifest_identity": false,
            "substitutes_manifest_implementation_state": false,
            "mutates_historical_checkbox_or_delivery_state": false,
            "usable_for_prior_wave_seal_or_recovery": false,
            "satisfaction_rule": "every adopted id resolves to exactly one local task in historical_foundation_adoption; that task must reach Merged with all required completion evidence before any dependent manifest group is Ready"
        }),
        "adoption_substitution_semantics",
        findings,
    );
    expect_contract_value(
        contract,
        &["local_completion_state_machine"],
        serde_json::json!({
            "states": ["Planned", "Dispatched", "Validated", "Merged"],
            "initial_state": "Planned",
            "transitions": {
                "Planned": ["Dispatched"],
                "Dispatched": ["Validated"],
                "Validated": ["Dispatched", "Merged"],
                "Merged": []
            },
            "transition_authority": "external dispatch ledger plus structured evidence; checkbox rendering is a status projection only",
            "validated_requires_all_evidence": true,
            "merged_requires_validated_state_and_accepted_commit": true,
            "validation_failure_transition": ["Validated", "Dispatched"]
        }),
        "local_completion_state_machine",
        findings,
    );

    let expected_evidence = expected_local_completion_evidence();
    let expected_evidence_keys = expected_evidence
        .keys()
        .map(|task| (*task).to_owned())
        .collect::<BTreeSet<_>>();
    if object_keys(value_at(contract, &["required_local_completion_evidence"]))
        != expected_evidence_keys
    {
        findings.push("required_local_completion_evidence task set is incorrect".to_owned());
    }
    for (task, evidence) in expected_evidence {
        let expected = evidence
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        let actual = string_array(value_at(
            contract,
            &["required_local_completion_evidence", task],
        ));
        if actual != expected || actual.is_empty() {
            findings.push(format!(
                "required_local_completion_evidence for `{task}` is incomplete or reordered"
            ));
        }
        let mut unique = BTreeSet::new();
        if let Some(values) = value_at(contract, &["required_local_completion_evidence", task])
            .and_then(Value::as_array)
        {
            for value in values {
                if let Some(value) = value.as_str() {
                    if !unique.insert(value) {
                        findings.push(format!(
                            "required local completion evidence `{value}` is duplicated for `{task}`"
                        ));
                    }
                } else {
                    findings.push(format!(
                        "required local completion evidence for `{task}` contains a non-string"
                    ));
                }
            }
        }
    }

    expect_contract_value(
        contract,
        &["t608_strict_nix_and_tpm_prerequisites"],
        serde_json::json!({
            "nix_owners": [
                "packages/xtask/src/zone_schema.rs",
                "nixos-modules/generated/resource-types.nix",
                "nixos-modules/generated/options-zones-Zone.nix",
                "nixos-modules/generated/options-zones-ZoneLink.nix",
                "nixos-modules/options-resources.nix",
                "nixos-modules/options-zones-resources.nix",
                "nixos-modules/resource-schema-validation.nix",
                "nixos-modules/resource-compiler.nix",
                "nixos-modules/resources.nix",
                "nixos-modules/resources-bundle.nix",
                "nixos-modules/assertions.nix"
            ],
            "tpm_before_first_ensure": [
                "T606 shared broker and storage contract freeze is Merged",
                "T608 Volume contract and Host-global authority index are Validated",
                "legacy swtpm state and marker inventory is complete",
                "migration or exact-owner adoption is durable",
                "missing, replacement, ambiguity, and foreign-owner cases are refused"
            ]
        }),
        "t608_strict_nix_and_tpm_prerequisites",
        findings,
    );
    expect_contract_value(
        contract,
        &["t609_production_audit_wiring"],
        serde_json::json!({
            "single_foundation_owner": "T609",
            "surfaces": [
                "d2b-audit record, sink, segment, export, rotation, and prune",
                "resource-store transactional mutation audit",
                "daemon runtime and daemon audit",
                "broker privileged audit writer",
                "core authorization audit",
                "bus and session audit producers",
                "production boundary and failure-injection tests"
            ],
            "provider_rule": "Provider-specific emitters consume typed ports after T609; no Provider opens a writer or chooses durability"
        }),
        "t609_production_audit_wiring",
        findings,
    );

    check_manifest_group_foundations(contract, graph, manifest, findings);

    let dispatch = value_at(contract, &["dispatch_ledger_contract"]);
    let dispatch_keys = string_set(&[
        "path_environment",
        "must_be_absolute",
        "must_be_outside_git",
        "artifact_kind",
        "schema_version",
        "entry_key",
        "states",
        "entry_required_fields",
        "not_launched_null_fields",
        "required_groups",
        "local_group_ids",
        "readiness_rule",
        "pre_t221_rule",
        "not_authentication",
    ]);
    if object_keys(dispatch) != dispatch_keys {
        findings.push("dispatch_ledger_contract field set is incorrect".to_owned());
    }
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "path_environment"],
        serde_json::json!("D2B_W6_DISPATCH_LEDGER"),
        "dispatch_ledger_contract.path_environment",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "must_be_absolute"],
        serde_json::json!(true),
        "dispatch_ledger_contract.must_be_absolute",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "must_be_outside_git"],
        serde_json::json!(true),
        "dispatch_ledger_contract.must_be_outside_git",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "artifact_kind"],
        serde_json::json!("d2b-feature-local/dispatch-ledger"),
        "dispatch_ledger_contract.artifact_kind",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "schema_version"],
        serde_json::json!(1),
        "dispatch_ledger_contract.schema_version",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "entry_key"],
        serde_json::json!("group"),
        "dispatch_ledger_contract.entry_key",
        findings,
    );
    expect_contract_string_array(
        contract,
        &["dispatch_ledger_contract", "states"],
        &[
            "NotLaunched",
            "Dispatched",
            "Validated",
            "Completed",
            "Blocked",
        ],
        "dispatch_ledger_contract.states",
        findings,
    );
    expect_contract_string_array(
        contract,
        &["dispatch_ledger_contract", "entry_required_fields"],
        &[
            "group",
            "state",
            "candidateId",
            "headOid",
            "dispatchId",
            "updatedAtUnix",
            "completionEvidenceIds",
        ],
        "dispatch_ledger_contract.entry_required_fields",
        findings,
    );
    expect_contract_string_array(
        contract,
        &["dispatch_ledger_contract", "not_launched_null_fields"],
        &["dispatchId"],
        "dispatch_ledger_contract.not_launched_null_fields",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "required_groups"],
        serde_json::json!(EXPECTED_POST_ENTRY_GROUPS),
        "dispatch_ledger_contract.required_groups",
        findings,
    );
    expect_contract_string_array(
        contract,
        &["dispatch_ledger_contract", "local_group_ids"],
        EXPECTED_LOCAL_GROUP_IDS,
        "dispatch_ledger_contract.local_group_ids",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "readiness_rule"],
        serde_json::json!(
            "a group is Ready only when every local foundation in manifest_group_foundations and every graph prerequisite is Completed or Merged in the ledger"
        ),
        "dispatch_ledger_contract.readiness_rule",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "pre_t221_rule"],
        serde_json::json!(
            "derive launched groups from ledger entries whose state is not NotLaunched and require the derived set to be empty"
        ),
        "dispatch_ledger_contract.pre_t221_rule",
        findings,
    );
    expect_contract_value(
        contract,
        &["dispatch_ledger_contract", "not_authentication"],
        serde_json::json!(true),
        "dispatch_ledger_contract.not_authentication",
        findings,
    );
    let graph_groups = w6_manifest_groups(graph);
    let mut expected_post_entry_groups = graph_groups.clone();
    expected_post_entry_groups.extend(
        EXPECTED_LOCAL_GROUP_IDS
            .iter()
            .map(|group| (*group).to_owned()),
    );
    if expected_post_entry_groups.len() != EXPECTED_POST_ENTRY_GROUPS {
        findings.push("post-entry group derivation is not closed".to_owned());
    }

    let completion_states = string_array(value_at(
        contract,
        &["local_completion_state_machine", "states"],
    ));
    if object_keys(value_at(
        contract,
        &["local_completion_state_machine", "transitions"],
    )) != completion_states.iter().cloned().collect()
    {
        findings.push("local completion transition keys do not cover every state".to_owned());
    }
    for schema_path in [
        &["dispatch_ledger_contract", "states"][..],
        &["dispatch_ledger_contract", "entry_required_fields"][..],
        &["structured_command_evidence_contract", "required_fields"][..],
        &[
            "structured_command_evidence_contract",
            "required_t221_command_ids",
        ][..],
        &["plan_approval_receipt_contract", "required_fields"][..],
    ] {
        if string_array_has_duplicates(value_at(contract, schema_path)) {
            findings.push(format!(
                "receipt or ledger schema array `{}` contains duplicate values",
                schema_path.join(".")
            ));
        }
    }
    let command_evidence_count = value_at(
        contract,
        &[
            "structured_command_evidence_contract",
            "required_t221_command_ids",
        ],
    )
    .and_then(Value::as_array)
    .map(Vec::len);
    let import_cardinality = value_at(contract, &["entry_prepare_contract", "import_cardinality"])
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    if command_evidence_count != import_cardinality {
        findings.push(
            "entry-prepare import cardinality does not equal the closed command-evidence schema"
                .to_owned(),
        );
    }
    if value_at(
        contract,
        &[
            "entry_prepare_contract",
            "first_call_command_evidence_count",
        ],
    )
    .and_then(Value::as_u64)
        != Some(0)
    {
        findings.push(
            "entry-prepare first call must begin with zero imported command-evidence records"
                .to_owned(),
        );
    }

    expect_contract_value(
        contract,
        &["entry_prepare_contract"],
        serde_json::json!({
            "public_command": "delivery wave snapshot",
            "flag": "--entry-prepare true",
            "first_call": "fresh-fetch and discover candidate; atomically create-or-compare the 36-entry NotLaunched dispatch ledger and create-or-compare an empty command-evidence directory; emit digests; do not write snapshot.json",
            "first_call_command_evidence_count": 0,
            "import_flag": "--command-evidence PATH",
            "import_cardinality": 8,
            "import_rule": "repeat entry preparation with one flag per closed command identity; strict-parse and create-or-compare every record",
            "ordinary_snapshot_rule": "omit --entry-prepare and --command-evidence; validate ledger and exact eight-record evidence set before writing snapshot.json",
            "records_must_preexist_before_discovery": false
        }),
        "entry_prepare_contract",
        findings,
    );

    expect_contract_value(
        contract,
        &["structured_command_evidence_contract"],
        serde_json::json!({
            "artifact_kind": "d2b-feature-local/command-evidence",
            "schema_version": 1,
            "required_fields": [
                "commandId",
                "argv",
                "workingTreeOid",
                "startedAtUnix",
                "completedAtUnix",
                "exitCode",
                "result",
                "stdoutSha256",
                "stderrSha256",
                "outputBytes"
            ],
            "required_t221_command_ids": [
                "focused-guard-list",
                "focused-guard-ignored-list",
                "focused-guard-run",
                "gate0-test-drift",
                "test-policy",
                "test-unit",
                "heavy-gate-acquire",
                "predispatch-census"
            ],
            "profile_runner": "delivery wave run-command-profile --profile <closed-id>",
            "command_profiles": {
                "focused-guard-list": [
                    "cargo", "test", "--manifest-path", "packages/Cargo.toml", "-p",
                    "xtask", "delivery::work_item_state::tests", "--", "--list"
                ],
                "focused-guard-ignored-list": [
                    "cargo", "test", "--manifest-path", "packages/Cargo.toml", "-p",
                    "xtask", "delivery::work_item_state::tests", "--", "--list", "--ignored"
                ],
                "focused-guard-run": [
                    "cargo", "test", "--manifest-path", "packages/Cargo.toml", "-p",
                    "xtask", "delivery::work_item_state::tests", "--", "--nocapture"
                ],
                "gate0-test-drift": ["make", "test-drift"],
                "test-policy": ["make", "test-policy"],
                "test-unit": ["make", "test-unit"],
                "heavy-gate-acquire": [
                    "cargo", "run", "--quiet", "--manifest-path", "packages/Cargo.toml",
                    "-p", "xtask", "--", "heavy-gate", "--", "true"
                ],
                "predispatch-census": [
                    "cargo", "run", "--quiet", "--manifest-path", "packages/Cargo.toml",
                    "-p", "xtask", "--", "delivery", "wave", "entry-census",
                    "--program", "ADR046", "--wave", "adr046w6"
                ]
            },
            "layer1_membership_for_test_unit": [
                "test-flake", "test-nix-unit", "test-runtime-ledger"
            ],
            "argv_rule": "production profile runner owns argv; evidence argv must exactly equal the selected closed profile and caller-supplied replacement argv is refused",
            "focused_fields": ["discoveredTests", "ignoredTests", "skipMatches"],
            "result_values": ["passed", "failed"],
            "raw_output_persisted_in_git": false,
            "not_authentication": true
        }),
        "structured_command_evidence_contract",
        findings,
    );
    expect_contract_value(
        contract,
        &["plan_approval_receipt_contract"],
        serde_json::json!({
            "path_environment": "D2B_W6_PLAN_APPROVAL_RECEIPT",
            "must_be_absolute": true,
            "must_be_outside_git": true,
            "artifact_kind": "d2b-feature-local/plan-approval",
            "schema_version": 1,
            "required_fields": [
                "artifactKind",
                "schemaVersion",
                "program",
                "wave",
                "entryBaseOid",
                "featurePlanMaterialSha256",
                "entryCandidateId",
                "entryContentId",
                "entrySnapshotSha256",
                "selectionSha256",
                "dispatchLedgerSha256",
                "commandEvidenceSetSha256",
                "selectedRoster",
                "signoffCount",
                "recommendationCount",
                "result",
                "durableWriteEvidenceSha256",
                "approvedAtUnix",
                "lifecycleApproval",
                "seatRecords"
            ],
            "required_values": {
                "program": "ADR046",
                "wave": "adr046w6",
                "recommendationCount": 0,
                "result": "approved"
            },
            "lifecycle_approval_fields": [
                "artifactKind",
                "schemaVersion",
                "lifecycleId",
                "phase",
                "candidateId",
                "contentId",
                "snapshotSha256",
                "selectionSha256",
                "approved"
            ],
            "lifecycle_approval_required_values": {
                "phase": "plan",
                "approved": true
            },
            "seat_record_rule": "seatRecords is an exact key map for selectedRoster; every value is candidate/selection-bound, has signoff true, recommendations [], and carries its completion-bound recordSha256",
            "production_writer": "delivery wave entry-plan-approval write",
            "production_verifier": "delivery wave entry-plan-approval verify",
            "feature_plan_material_digest": "SHA-256 over the ordered FEATURE_DIR files with only entry_plan_invalidation_policy.status_only_updates normalized to fixed placeholders; requirements, machine contract, dependencies, ownership, validation, readiness, census, and guards remain byte-significant",
            "durable_write": [
                "create same-directory temporary file",
                "write canonical JSON",
                "fsync temporary file",
                "rename over target",
                "fsync parent directory"
            ],
            "correlation_only": true,
            "not_authentication": true
        }),
        "plan_approval_receipt_contract",
        findings,
    );
    expect_contract_value(
        contract,
        &["entry_plan_invalidation_policy"],
        serde_json::json!({
            "boundary": "first Dispatched transition in the external dispatch ledger",
            "pre_first_dispatch_material_changes_invalidate": [
                "entry base or ancestry",
                "retained predecessor material",
                "guard implementation or command evidence",
                "requirements or success criteria",
                "local task contract",
                "dependencies or launch/readiness rules",
                "ownership or shared-writer handoffs",
                "validation or completion evidence",
                "manifest group membership or foundation mapping"
            ],
            "status_only_updates": [
                "checkbox projection derived from the dispatch ledger",
                "local completion state projection",
                "evidence result, digest, byte count, or external locator",
                "dispatch, validation, merge, or seal status",
                "timestamps and non-authorizing progress summaries"
            ],
            "status_only_must_not_change": [
                "requirements",
                "dependencies",
                "owners",
                "destinations",
                "validation",
                "state transitions",
                "launch census",
                "readiness rules",
                "guard predicates"
            ],
            "status_only_updates_do_not_invalidate_after_first_dispatch": true,
            "material_change_after_first_dispatch": "stop affected dispatch, record Blocked in the ledger, replace the plan material and approval receipt before further launch"
        }),
        "entry_plan_invalidation_policy",
        findings,
    );
    if !is_sha256_hex(value_at(
        contract,
        &[
            "local_to_manifest_shared_writer_handoffs",
            "work_items_sha256",
        ],
    )) || !is_sha256_hex(value_at(
        contract,
        &[
            "local_to_manifest_shared_writer_handoffs",
            "implementation_graph_sha256",
        ],
    )) {
        findings.push("shared-writer handoff source digests must be SHA-256 values".to_owned());
    }
    for (field, path) in [
        ("work_items_sha256", "docs/specs/ADR-046-work-items.json"),
        (
            "implementation_graph_sha256",
            "docs/specs/ADR-046-implementation-graph.json",
        ),
    ] {
        let actual = std::fs::read(repo_root().join(path))
            .map(|bytes| hex_sha256(&bytes))
            .ok();
        let expected = value_at(
            contract,
            &["local_to_manifest_shared_writer_handoffs", field],
        )
        .and_then(Value::as_str);
        if actual.as_deref() != expected {
            findings.push(format!(
                "shared-writer handoff `{field}` does not match `{path}`"
            ));
        }
    }
}

fn manifest_group_by_id(graph: &Value) -> BTreeMap<String, String> {
    graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["kind"] == "work-item" && node["wave"] == "W6")
        .filter_map(|node| {
            Some((
                node["id"].as_str()?.to_owned(),
                node["parallelGroup"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

fn graph_precedes(graph: &Value, before: &str, after: &str) -> bool {
    let nodes: BTreeMap<&str, &Value> = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| Some((node["id"].as_str()?, node)))
        .collect();
    let mut pending = vec![after.to_owned()];
    let mut visited = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        let Some(node) = nodes.get(current.as_str()) else {
            continue;
        };
        for prerequisite in node["prerequisites"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if prerequisite == before {
                return true;
            }
            pending.push(prerequisite.to_owned());
        }
    }
    false
}

fn local_dependency_precedes(contract: &Value, before: &str, after: &str) -> bool {
    let mut pending = vec![after.to_owned()];
    let mut visited = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        for dependency in string_array(value_at(
            contract,
            &["required_local_dependencies", current.as_str()],
        )) {
            if dependency == before {
                return true;
            }
            pending.push(dependency);
        }
    }
    false
}

fn manifest_dependency_contains(
    contract: &Value,
    task: &str,
    dependency: &str,
    w6_ids: &BTreeSet<String>,
) -> bool {
    if string_array(value_at(
        contract,
        &["required_manifest_dependencies", task],
    ))
    .iter()
    .any(|candidate| candidate == dependency)
    {
        return true;
    }
    task == "T479"
        && w6_ids.contains(dependency)
        && value_at(contract, &["required_manifest_dependency_queries", "T479"])
            .and_then(|query| query["complete_for_task"].as_bool())
            .unwrap_or(false)
}

fn handoff_edge_is_executable(
    contract: &Value,
    graph: &Value,
    from: &str,
    to: &str,
    local_ids: &BTreeSet<String>,
    manifest_ids: &BTreeSet<&str>,
    manifest_groups: &BTreeMap<String, String>,
    w6_ids: &BTreeSet<String>,
) -> bool {
    let from_local = local_ids.contains(from);
    let to_local = local_ids.contains(to);
    let from_manifest = manifest_ids.contains(from);
    let to_manifest = manifest_ids.contains(to);

    match (from_local, to_local, from_manifest, to_manifest) {
        (true, true, false, false) => local_dependency_precedes(contract, from, to),
        (true, false, false, true) => manifest_groups
            .get(to)
            .map(|group| {
                string_array(value_at(
                    contract,
                    &["manifest_group_foundations", group.as_str()],
                ))
                .iter()
                .any(|foundation| foundation == from)
            })
            .unwrap_or(false),
        (false, true, true, false) => manifest_dependency_contains(contract, to, from, w6_ids),
        (false, false, true, true) => {
            if graph_precedes(graph, from, to) {
                true
            } else if graph_precedes(graph, to, from) {
                false
            } else {
                let Some(from_group) = manifest_groups.get(from) else {
                    return false;
                };
                let Some(to_group) = manifest_groups.get(to) else {
                    return false;
                };
                let from_foundations = string_array(value_at(
                    contract,
                    &["manifest_group_foundations", from_group.as_str()],
                ));
                let to_foundations = string_array(value_at(
                    contract,
                    &["manifest_group_foundations", to_group.as_str()],
                ));
                from_foundations
                    .iter()
                    .any(|foundation| to_foundations.contains(foundation))
            }
        }
        _ => false,
    }
}

fn canonical_handoff_path(local_path: &str, declared_paths: &BTreeSet<String>) -> String {
    declared_paths
        .iter()
        .filter(|path| {
            path.as_str() == local_path
                || (path.ends_with('/') && local_path.starts_with(path.as_str()))
        })
        .max_by_key(|path| path.len())
        .cloned()
        .unwrap_or_else(|| local_path.to_owned())
}

fn handoff_writers(
    path: &str,
    owners: &BTreeMap<String, BTreeSet<String>>,
    manifest: &Value,
    w6_ids: &BTreeSet<String>,
    manifest_groups: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut writers = BTreeSet::new();
    for (local_path, local_owners) in owners {
        if destination_paths_overlap(path, local_path) {
            writers.extend(local_owners.iter().cloned());
        }
    }
    for item in manifest["items"].as_array().into_iter().flatten() {
        let Some(id) = item["workItemId"].as_str() else {
            continue;
        };
        if !w6_ids.contains(id) {
            continue;
        }
        let Some(destination_text) = item["destination"].as_str() else {
            continue;
        };
        if is_local_handoff_destination(destination_text) {
            continue;
        }
        let provider_root = manifest_groups
            .get(id)
            .and_then(|group| provider_root_for_group(Some(group.as_str())));
        let atoms = item["destination"]
            .as_str()
            .map(|destination| normalized_destination_atoms(destination, provider_root.as_deref()))
            .unwrap_or_default();
        if atoms
            .iter()
            .filter(|destination| !path_has_glob(destination))
            .any(|destination| destination_paths_overlap(path, destination))
        {
            writers.insert(id.to_owned());
        }
    }
    writers
}

fn check_shared_writer_handoffs(
    contract: &Value,
    graph: &Value,
    manifest: &Value,
    findings: &mut Vec<String>,
) {
    let owners = local_owned_path_owners(contract);
    let scaffold_roots = object_keys(value_at(
        contract,
        &[
            "local_to_manifest_shared_writer_handoffs",
            "scaffold_handoffs",
        ],
    ));
    let manifest_ids = manifest["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["workItemId"].as_str())
        .collect::<BTreeSet<_>>();
    let w6_ids = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["kind"] == "work-item" && node["wave"] == "W6")
        .filter_map(|node| node["id"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let local_ids = string_set(EXPECTED_LOCAL_TASK_IDS);
    let manifest_groups = manifest_group_by_id(graph);
    let declared_handoff_paths = value_at(
        contract,
        &["local_to_manifest_shared_writer_handoffs", "handoffs"],
    )
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .flat_map(|handoff| handoff["paths"].as_array().into_iter().flatten())
    .filter_map(Value::as_str)
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let mut writers_by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in owners.keys().filter(|path| !scaffold_roots.contains(*path)) {
        for item in manifest["items"].as_array().into_iter().flatten() {
            let Some(id) = item["workItemId"].as_str() else {
                continue;
            };
            if !w6_ids.contains(id) {
                continue;
            }
            let Some(destination_text) = item["destination"].as_str() else {
                continue;
            };
            if is_local_handoff_destination(destination_text) {
                continue;
            }
            let provider_root = manifest_groups
                .get(id)
                .and_then(|group| provider_root_for_group(Some(group.as_str())));
            let atoms = item["destination"]
                .as_str()
                .map(|destination| {
                    normalized_destination_atoms(destination, provider_root.as_deref())
                })
                .unwrap_or_default();
            if atoms
                .iter()
                .filter(|destination| !path_has_glob(destination))
                .any(|destination| local_path_overlaps_destination(path, destination))
            {
                let destination = atoms
                    .iter()
                    .filter(|destination| {
                        !path_has_glob(destination)
                            && local_path_overlaps_destination(path, destination)
                    })
                    .max_by_key(|destination| destination.len())
                    .expect("overlapping destination");
                let raw_path = if destination.ends_with('/')
                    && !path_has_glob(destination)
                    && path.starts_with(destination)
                {
                    destination.to_owned()
                } else {
                    path.to_owned()
                };
                let canonical = canonical_handoff_path(&raw_path, &declared_handoff_paths);
                writers_by_path
                    .entry(canonical.clone())
                    .or_default()
                    .extend(owners.get(path).into_iter().flatten().cloned());
                writers_by_path
                    .entry(canonical)
                    .or_default()
                    .insert(id.to_owned());
            }
        }
    }

    let handoffs = value_at(
        contract,
        &["local_to_manifest_shared_writer_handoffs", "handoffs"],
    )
    .and_then(Value::as_array);
    let Some(handoffs) = handoffs else {
        findings.push("local_to_manifest_shared_writer_handoffs.handoffs is missing".to_owned());
        return;
    };
    if handoffs.len() != 16 {
        findings.push(format!(
            "shared-writer handoff count is {}, expected 16",
            handoffs.len()
        ));
    }

    let mut handoff_paths = BTreeSet::new();
    let mut path_locations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for handoff in handoffs {
        let surface = handoff["surface"].as_str().unwrap_or_default();
        if surface.is_empty() {
            findings.push("a shared-writer handoff has no surface".to_owned());
        }
        let paths = handoff["paths"].as_array();
        let order = handoff["order"].as_array();
        let (Some(paths), Some(order)) = (paths, order) else {
            findings.push(format!(
                "shared-writer handoff `{surface}` is missing paths or order"
            ));
            continue;
        };
        let mut paths_in_handoff = BTreeSet::new();
        for path in paths {
            let Some(path) = path.as_str() else {
                findings.push(format!(
                    "shared-writer handoff `{surface}` contains a non-string path"
                ));
                continue;
            };
            if !paths_in_handoff.insert(path.to_owned()) {
                findings.push(format!(
                    "shared-writer handoff `{surface}` repeats path `{path}`"
                ));
            }
            path_locations
                .entry(path.to_owned())
                .or_default()
                .push(surface.to_owned());
            handoff_paths.insert(path.to_owned());
            let path_writers = handoff_writers(path, &owners, manifest, &w6_ids, &manifest_groups);
            if path_writers.is_empty() {
                findings.push(format!(
                    "shared-writer handoff `{surface}` names a path without a local/manifest overlap `{path}`"
                ));
                continue;
            }
            let path_owners = owners
                .iter()
                .filter(|(local_path, _)| destination_paths_overlap(path, local_path))
                .flat_map(|(_, owners)| owners.iter())
                .collect::<BTreeSet<_>>();
            if !path_owners.is_empty() {
                let Some(first) = order.first().and_then(Value::as_str) else {
                    findings.push(format!(
                        "shared-writer handoff `{surface}` has an empty order"
                    ));
                    continue;
                };
                if !path_owners.iter().any(|owner| owner.as_str() == first) {
                    findings.push(format!(
                        "shared-writer handoff `{surface}` starts `{first}` instead of a local owner of `{path}`"
                    ));
                }
            }
            let mut seen_order = BTreeSet::new();
            for endpoint in order {
                let Some(endpoint) = endpoint.as_str() else {
                    findings.push(format!(
                        "shared-writer handoff `{surface}` contains a non-string order endpoint"
                    ));
                    continue;
                };
                if !seen_order.insert(endpoint.to_owned()) {
                    findings.push(format!(
                        "shared-writer handoff `{surface}` repeats order endpoint `{endpoint}`"
                    ));
                }
                if !local_ids.contains(endpoint) && !manifest_ids.contains(endpoint) {
                    findings.push(format!(
                        "shared-writer handoff `{surface}` has unresolved order endpoint `{endpoint}`"
                    ));
                }
            }
            if !path_writers
                .iter()
                .all(|writer| seen_order.contains(writer))
            {
                findings.push(format!(
                    "shared-writer handoff `{surface}` order for `{path}` omits a local or manifest owner"
                ));
            }
            if order.is_empty() {
                findings.push(format!(
                    "shared-writer handoff `{surface}` has an incomplete order"
                ));
            }
            for pair in order.windows(2) {
                let Some(from) = pair[0].as_str() else {
                    continue;
                };
                let Some(to) = pair[1].as_str() else {
                    continue;
                };
                if !handoff_edge_is_executable(
                    contract,
                    graph,
                    from,
                    to,
                    &local_ids,
                    &manifest_ids,
                    &manifest_groups,
                    &w6_ids,
                ) {
                    findings.push(format!(
                        "shared-writer handoff `{surface}` adjacent order `{from}` -> `{to}` is absent from graph/readiness ordering"
                    ));
                }
            }
        }
    }

    for (path, surfaces) in &path_locations {
        if surfaces.len() > 1 {
            findings.push(format!(
                "shared-writer path `{path}` appears in multiple handoffs: {}",
                surfaces.join(", ")
            ));
        }
    }
    for path in writers_by_path.keys() {
        if !handoff_paths.contains(path) {
            findings.push(format!(
                "manifest destination overlap `{path}` has no shared-writer handoff"
            ));
        }
    }
    for path in &handoff_paths {
        let writers = handoff_writers(path, &owners, manifest, &w6_ids, &manifest_groups);
        if writers.iter().any(|writer| !local_ids.contains(writer))
            && !writers_by_path.contains_key(path)
        {
            findings.push(format!(
                "shared-writer handoff path `{path}` has no derived local/manifest overlap"
            ));
        }
    }

    let expected_scaffolds = expected_scaffold_handoffs();
    if object_keys(value_at(
        contract,
        &[
            "local_to_manifest_shared_writer_handoffs",
            "scaffold_handoffs",
        ],
    )) != expected_scaffolds
        .keys()
        .map(|key| (*key).to_owned())
        .collect()
    {
        findings.push("scaffold handoff roots are missing or extra".to_owned());
    }
    let derived_scaffolds = owners
        .iter()
        .filter(|(path, path_owners)| {
            path.ends_with('/')
                && path.starts_with("packages/d2b-provider-")
                && path_owners.contains("T606")
        })
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    if derived_scaffolds
        != expected_scaffolds
            .keys()
            .map(|key| (*key).to_owned())
            .collect()
    {
        findings.push("T606 Provider scaffold roots are not closed".to_owned());
    }
    for (root, group) in expected_scaffolds {
        let actual = value_at(
            contract,
            &[
                "local_to_manifest_shared_writer_handoffs",
                "scaffold_handoffs",
                root,
            ],
        )
        .and_then(Value::as_str);
        if actual != Some(group) {
            findings.push(format!(
                "scaffold handoff `{root}` does not map to `{group}`"
            ));
        }
        if !w6_manifest_groups(graph).contains(group) {
            findings.push(format!(
                "scaffold handoff `{root}` maps to unresolved W6 group `{group}`"
            ));
        }
    }
}

fn contract_task_row(line: &str, task_ids: &BTreeSet<String>) -> Option<(String, bool)> {
    let rest = line.strip_prefix("- ")?;
    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let marker = &rest[..=close];
    let id = rest[close + 1..]
        .split_whitespace()
        .next()?
        .trim_matches('`');
    task_ids
        .contains(id)
        .then_some((id.to_owned(), marker != "[ ]"))
}

fn task_row_id(line: &str) -> Option<String> {
    let rest = line.strip_prefix("- ")?;
    let close = rest.find(']')?;
    let marker = &rest[..=close];
    if !matches!(marker, "[ ]" | "[x]" | "[X]") {
        return None;
    }
    let id = rest[close + 1..].split_whitespace().next()?;
    (id.starts_with('T') && id[1..].chars().all(|character| character.is_ascii_digit()))
        .then_some(id.to_owned())
}

fn check_task_census(markdown: &str, findings: &mut Vec<String>) {
    let expected_labels = expected_local_task_label_prefixes();
    let expected_ids = string_set(EXPECTED_LOCAL_TASK_IDS);
    let mut task_count = 0usize;
    let mut parallel_count = 0usize;
    let mut ids = BTreeSet::new();
    let mut local_lines = BTreeMap::new();

    for line in markdown.lines() {
        if !line.starts_with("- [") {
            continue;
        }
        let Some(close) = line.find(']') else {
            findings.push("task row has an unterminated checkbox".to_owned());
            continue;
        };
        let marker = &line[2..=close];
        let id = task_row_id(line);
        if !matches!(marker, "[ ]" | "[x]" | "[X]") {
            if id.is_some() {
                findings.push(format!(
                    "task row `{}` has an invalid checkbox marker `{marker}`",
                    id.as_deref().unwrap_or_default()
                ));
            }
            continue;
        }
        let Some(id) = id else {
            findings.push(format!("task row has no numeric task id: `{line}`"));
            continue;
        };
        task_count += 1;
        if line.contains("[P]") {
            parallel_count += 1;
        }
        if !ids.insert(id.clone()) {
            findings.push(format!("task id `{id}` is declared more than once"));
        }
        if expected_ids.contains(&id) {
            local_lines.insert(id, line.to_owned());
        } else if line.contains("FEATURE-LOCAL") {
            findings.push(format!("unregistered feature-local task label on `{id}`"));
        }
    }

    if task_count != EXPECTED_TASKS {
        findings.push(format!(
            "task census is {task_count}, expected {EXPECTED_TASKS}"
        ));
    }
    if parallel_count != EXPECTED_PARALLEL_TASKS {
        findings.push(format!(
            "parallel task census is {parallel_count}, expected {EXPECTED_PARALLEL_TASKS}"
        ));
    }
    for (task, expected) in expected_labels {
        let Some(line) = local_lines.get(task) else {
            findings.push(format!("feature-local task label `{task}` is missing"));
            continue;
        };
        let Some(close) = line.find(']') else {
            continue;
        };
        let actual = line[close + 2..].trim_start();
        if !actual.starts_with(expected) {
            findings.push(format!(
                "feature-local task label `{task}` is not the current authoritative label"
            ));
        }
    }
}

fn check_local_coordination_tasks(markdown: &str, graph: &Value) -> Vec<String> {
    let mut findings = Vec::new();
    check_task_census(markdown, &mut findings);
    let mut contracts = Vec::new();
    for fence in markdown_json_fences(markdown) {
        if !fence.closed {
            findings.push("JSON task contract fence is not closed".to_owned());
            continue;
        }
        match parse_json_without_duplicates(&fence.body) {
            Ok(value) if value["artifact_kind"] == "d2b-feature-local-task-contract" => {
                contracts.push(value);
            }
            Ok(_) => {}
            Err(error) => {
                findings.push(format!("JSON task contract fence is invalid: {error}"));
            }
        }
    }
    if contracts.len() != 1 {
        findings.push(format!(
            "expected exactly one feature-local task contract, found {}",
            contracts.len()
        ));
    }
    let Some(contract) = contracts.into_iter().next() else {
        return findings;
    };

    let contract_hash = hex_sha256(&canonical_json(&contract));
    if contract_hash != FEATURE_TASK_CONTRACT_SHA256 {
        findings.push(format!(
            "feature-local task contract canonical SHA-256 pin mismatch (got {contract_hash})"
        ));
    }
    let manifest = load(WORK_ITEMS);
    check_local_completion_contract(&contract, graph, &manifest, &mut findings);
    check_shared_writer_handoffs(&contract, graph, &manifest, &mut findings);

    expect_contract_value(
        &contract,
        &["artifact_kind"],
        serde_json::json!("d2b-feature-local-task-contract"),
        "artifact_kind",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["schema_version"],
        serde_json::json!(1),
        "schema_version",
        &mut findings,
    );
    expect_contract_string_array(
        &contract,
        &["task_ids"],
        EXPECTED_LOCAL_TASK_IDS,
        "task_ids",
        &mut findings,
    );
    expect_contract_string_array(
        &contract,
        &["unchecked_task_ids"],
        EXPECTED_LOCAL_TASK_IDS,
        "unchecked_task_ids",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["outside_retired_fences"],
        serde_json::json!(true),
        "outside_retired_fences",
        &mut findings,
    );
    expect_contract_string_array(
        &contract,
        &["permitted_local_dependency_ids"],
        EXPECTED_PERMITTED_LOCAL_DEPENDENCY_IDS,
        "permitted_local_dependency_ids",
        &mut findings,
    );

    let query_nodes = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["kind"] == "work-item" && node["wave"] == "W6")
        .collect::<Vec<_>>();
    let query_ids = query_nodes
        .iter()
        .filter_map(|node| node["id"].as_str())
        .collect::<BTreeSet<_>>();
    if query_ids.len() != query_nodes.len() {
        findings.push("feature-local T479 W6 query projects duplicate or missing ids".to_owned());
    }
    if query_nodes.len() != 258 {
        findings.push(format!(
            "feature-local T479 W6 query expected 258 rows, got {}",
            query_nodes.len(),
        ));
    }
    let graph_ids = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| node["id"].as_str())
        .collect::<BTreeSet<_>>();

    let expected_local = string_set(EXPECTED_LOCAL_TASK_IDS);
    let expected_local_dependencies = expected_local_dependencies();
    if object_keys(value_at(&contract, &["required_local_dependencies"])) != expected_local {
        findings.push("feature-local local dependency task set is incorrect".to_owned());
    }
    let permitted_dependencies =
        string_array(value_at(&contract, &["permitted_local_dependency_ids"]))
            .into_iter()
            .collect::<BTreeSet<_>>();
    for (task, expected) in &expected_local_dependencies {
        let actual = string_array(value_at(&contract, &["required_local_dependencies", *task]));
        let expected = expected
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        if actual != expected {
            findings.push(format!(
                "feature-local contract {task} local dependencies are incorrect"
            ));
        }
    }
    if let Some(dependencies) =
        value_at(&contract, &["required_local_dependencies"]).and_then(Value::as_object)
    {
        for (task, values) in dependencies {
            for dependency in string_array(Some(values)) {
                if !permitted_dependencies.contains(&dependency) {
                    findings.push(format!(
                        "feature-local contract {task} uses unpermitted dependency `{dependency}`"
                    ));
                }
            }
        }
    }

    let expected_adoption = expected_historical_foundation_adoption();
    let expected_adoption_keys = {
        let mut keys = string_set(&["source_wave_label", "execution_wave"]);
        keys.insert("mutates_historical_state".to_owned());
        keys.extend(expected_adoption.keys().map(|key| (*key).to_owned()));
        keys
    };
    if object_keys(value_at(&contract, &["historical_foundation_adoption"]))
        != expected_adoption_keys
    {
        findings.push("feature-local historical adoption task set is incorrect".to_owned());
    }
    expect_contract_value(
        &contract,
        &["historical_foundation_adoption", "source_wave_label"],
        serde_json::json!("W5"),
        "historical_foundation_adoption.source_wave_label",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["historical_foundation_adoption", "execution_wave"],
        serde_json::json!("W6"),
        "historical_foundation_adoption.execution_wave",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["historical_foundation_adoption", "mutates_historical_state"],
        serde_json::json!(false),
        "historical_foundation_adoption.mutates_historical_state",
        &mut findings,
    );
    for (task, expected) in &expected_adoption {
        let actual = string_array(value_at(
            &contract,
            &["historical_foundation_adoption", *task],
        ));
        let expected = expected
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        if actual != expected {
            findings.push(format!(
                "feature-local historical adoption set for {task} is incorrect"
            ));
        }
    }

    let expected_t604_manifest = expected_t604_manifest_dependencies();
    if object_keys(value_at(&contract, &["required_manifest_dependencies"]))
        != string_set(&["T604"])
    {
        findings.push("feature-local manifest dependency task set is incorrect".to_owned());
    }
    if string_array(value_at(
        &contract,
        &["required_manifest_dependencies", "T604"],
    )) != expected_t604_manifest
    {
        findings.push("feature-local T604 manifest dependency set is incorrect".to_owned());
    }
    for dependency in &expected_t604_manifest {
        if !graph_ids.contains(dependency.as_str()) {
            findings.push(format!(
                "feature-local T604 dependency `{dependency}` is absent from the graph"
            ));
        }
    }

    let expected_query = serde_json::json!({
        "artifact": "docs/specs/ADR-046-implementation-graph.json",
        "where": {"kind": "work-item", "wave": "W6"},
        "project": "id",
        "project_semantics": "workItemId",
        "expected_count": 258,
        "cardinality": "exact",
        "complete_for_task": true
    });
    expect_contract_value(
        &contract,
        &["required_manifest_dependency_queries", "T479"],
        expected_query,
        "required_manifest_dependency_queries.T479",
        &mut findings,
    );
    if object_keys(value_at(
        &contract,
        &["required_manifest_dependency_queries"],
    )) != string_set(&["T479"])
    {
        findings.push("feature-local manifest query task set is incorrect".to_owned());
    }

    let expected_shared = expected_shared_file_order();
    let expected_shared_keys = expected_shared
        .keys()
        .map(|key| (*key).to_owned())
        .collect::<BTreeSet<_>>();
    if object_keys(value_at(&contract, &["shared_file_order"])) != expected_shared_keys {
        findings.push("feature-local shared-file ownership set is incorrect".to_owned());
    }
    for (path, expected) in &expected_shared {
        let actual = string_array(value_at(&contract, &["shared_file_order", *path]));
        let expected = expected
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        if actual != expected {
            findings.push(format!(
                "feature-local shared-file order for `{path}` is incorrect"
            ));
        }
    }

    let expected_owned = string_set(EXPECTED_OWNED_TASK_IDS);
    if object_keys(value_at(&contract, &["owned_files"])) != expected_owned {
        findings.push("feature-local owned task set is incorrect".to_owned());
    }
    let expected_owned_counts = BTreeMap::from([
        ("T606", 39usize),
        ("T607", 15usize),
        ("T608", 28usize),
        ("T609", 21usize),
        ("T604", 8usize),
        ("T479", 5usize),
    ]);
    for (task, expected_count) in expected_owned_counts {
        let actual_count = string_array(value_at(&contract, &["owned_files", task])).len();
        if actual_count != expected_count {
            findings.push(format!(
                "feature-local owned file count for {task} expected {expected_count}, got {actual_count}"
            ));
        }
    }

    expect_contract_string_array(
        &contract,
        &["case_id_fixture_paths"],
        &[
            "tests/golden/delivery/host-generation-pre-start-case-ids.txt",
            "tests/golden/delivery/host-generation-unit-census-case-ids.txt",
        ],
        "case_id_fixture_paths",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["validator_identity_literals"],
        serde_json::json!({"T604": ["operator-nix-activation-cleanup"]}),
        "validator_identity_literals",
        &mut findings,
    );
    expect_contract_string_array(
        &contract,
        &["acceptance_resource_identities"],
        &[
            "Volume/acceptance-state",
            "Network/acceptance-net",
            "Device/acceptance-tpm",
        ],
        "acceptance_resource_identities",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["candidate_evidence_literals"],
        serde_json::json!({
            "T479": [
                "operator-nix-activation-cleanup",
                "w6-cloud-hypervisor-guest-acceptance"
            ]
        }),
        "candidate_evidence_literals",
        &mut findings,
    );
    expect_contract_string_array(
        &contract,
        &["t479_candidate_execution_order"],
        &[
            "converge-f6",
            "freeze-f6",
            "invoke-t604-operator-validator",
            "execute-t604-authored-daemon-restart-case-with-cloud-hypervisor-case",
            "emit-both-candidate-records",
        ],
        "t479_candidate_execution_order",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["operator_acceptance"],
        serde_json::json!({
            "validator_author": "T604",
            "candidate_executor": "T479",
            "candidate_evidence_owner": "T479",
            "candidate_evidence_literal": "operator-nix-activation-cleanup",
            "candidate_record_count": 1,
            "t604_pre_f6_candidate_evidence_emission": false,
            "close_revalidator": "T480"
        }),
        "operator_acceptance",
        &mut findings,
    );
    expect_contract_value(
        &contract,
        &["fr075"],
        serde_json::json!({
            "case_author": "T604",
            "candidate_executor": "T479",
            "candidate_evidence_owner": "T479",
            "candidate_evidence_literal": "w6-cloud-hypervisor-guest-acceptance",
            "candidate_record_count": 1,
            "t604_candidate_bound_evidence": false,
            "close_revalidator": "T480"
        }),
        "fr075",
        &mut findings,
    );

    let contract_task_ids = string_array(value_at(&contract, &["task_ids"]))
        .into_iter()
        .collect::<BTreeSet<_>>();
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut line_retired_depths = Vec::with_capacity(lines.len());
    let mut retired_depth = 0usize;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("<!-- RETIRED-") && trimmed.ends_with("-BEGIN -->") {
            retired_depth += 1;
        }
        line_retired_depths.push(retired_depth);
        if trimmed.starts_with("<!-- RETIRED-") && trimmed.ends_with("-END -->") {
            retired_depth = retired_depth.saturating_sub(1);
        }
    }

    let mut blocks = BTreeMap::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];

        if let Some((id, checked)) = contract_task_row(line, &contract_task_ids) {
            let start = index;
            index += 1;
            while index < lines.len() && !lines[index].starts_with("- [") {
                index += 1;
            }
            let block = lines[start..index].join("\n");
            if blocks
                .insert(
                    id.clone(),
                    (line.to_owned(), block, line_retired_depths[start], checked),
                )
                .is_some()
            {
                findings.push(format!(
                    "feature-local task {id} is declared more than once"
                ));
            }
            continue;
        }
        index += 1;
    }

    let actual = blocks.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected_local {
        findings.push(format!(
            "feature-local task set must be exactly {expected_local:?}, got {actual:?}"
        ));
    }

    let requirements = BTreeMap::from([
        (
            "T606",
            ["T221", "machine-readable local task contract"].as_slice(),
        ),
        ("T607", ["T606", "local task contract"].as_slice()),
        ("T608", ["T606", "local task contract"].as_slice()),
        ("T609", ["T606", "local task contract"].as_slice()),
        (
            "T604",
            [
                "T221",
                "ADR046-ch-001",
                "operator-nix-activation-cleanup",
                "T479",
            ]
            .as_slice(),
        ),
        (
            "T479",
            [
                "T604",
                "T221",
                "operator-nix-activation-cleanup",
                "w6-cloud-hypervisor-guest-acceptance",
            ]
            .as_slice(),
        ),
        (
            "T480",
            [
                "T479",
                "operator-nix-activation-cleanup",
                "w6-cloud-hypervisor-guest-acceptance",
            ]
            .as_slice(),
        ),
    ]);
    for (id, required) in requirements {
        let Some((heading, block, depth, _checked)) = blocks.get(id) else {
            continue;
        };
        if !heading.starts_with("- [ ]")
            && !heading.starts_with("- [x]")
            && !heading.starts_with("- [X]")
        {
            findings.push(format!(
                "feature-local task {id} has an invalid checkbox projection"
            ));
        }
        if *depth != 0 {
            findings.push(format!(
                "feature-local task {id} must not be inside a retired fence"
            ));
        }
        for literal in required {
            if !block.contains(literal) {
                findings.push(format!(
                    "feature-local task {id} is missing required contract literal `{literal}`"
                ));
            }
        }
    }

    for (task, obligations) in expected_adoption {
        let Some((_, block, _, _)) = blocks.get(task) else {
            continue;
        };
        for obligation in obligations {
            if !block.contains(obligation) {
                findings.push(format!(
                    "feature-local task {task} is missing adopted obligation `{obligation}`"
                ));
            }
        }
    }

    findings.sort();
    findings.dedup();
    findings
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
    assert_eq!(
        work_items["schemaVersion"].as_u64(),
        Some(EXPECTED_WORK_ITEMS_SCHEMA),
        "the work-item artifact schema must be version {EXPECTED_WORK_ITEMS_SCHEMA}"
    );
    let mut manifest_state_counts = BTreeMap::new();
    for item in work_items["items"].as_array().expect("items array") {
        let state = item["implementationState"]
            .as_str()
            .expect("implementationState");
        *manifest_state_counts.entry(state).or_insert(0usize) += 1;
    }
    assert_eq!(
        manifest_state_counts,
        BTreeMap::from([
            ("Merged", EXPECTED_MANIFEST_MERGED),
            ("Planned", EXPECTED_MANIFEST_PLANNED),
        ]),
        "the current manifest implementation-state census changed"
    );

    let mut census: BTreeMap<&str, usize> = BTreeMap::new();
    for item in declared.values().flatten() {
        *census.entry(item.form).or_default() += 1;
    }

    assert_eq!(
        census,
        BTreeMap::from([
            ("bare", 358),
            ("dash title", 112),
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

    let spec_paths: BTreeMap<String, String> = members
        .iter()
        .map(|member| {
            (
                member["specId"].as_str().expect("specId").to_string(),
                member["path"].as_str().expect("path").to_string(),
            )
        })
        .collect();

    let findings = check_work_items(
        &declared,
        work_items["items"].as_array().expect("items array"),
        &registry,
        &spec_paths,
    );
    assert!(
        findings.is_empty(),
        "ADR 0046 work-item bijection violations:\n{}",
        findings.join("\n")
    );
}

#[test]
fn markdown_json_fences_require_commonmark_delimiters() {
    let fences = markdown_json_fences("````json\n{\"artifact_kind\":\"other\"}\n```\n~~~\n`````\n");
    assert_eq!(fences.len(), 1);
    assert!(fences[0].closed);
    assert_eq!(fences[0].body, "{\"artifact_kind\":\"other\"}\n```\n~~~\n");

    let tasks = read_repo_file(FEATURE_TASKS);
    let graph = load(GRAPH_JSON);
    for indentation in 0..=3 {
        let spaces = " ".repeat(indentation);
        let duplicate = format!(
            "{tasks}\n{spaces}```json\n\
             {{\"artifact_kind\":\"d2b-feature-local-task-contract\"}}\n\
             {spaces}```\n"
        );
        assert!(
            !check_local_coordination_tasks(&duplicate, &graph).is_empty(),
            "{indentation}-space duplicate local-task contract unexpectedly passed"
        );
    }
}

#[test]
fn feature_local_coordination_tasks_are_closed_and_authoritative() {
    let findings =
        check_local_coordination_tasks(&read_repo_file(FEATURE_TASKS), &load(GRAPH_JSON));
    assert!(
        findings.is_empty(),
        "feature-local coordination task policy failed:\n{}",
        findings.join("\n")
    );
}

#[test]
fn feature_local_coordination_contract_rejects_load_bearing_mutations() {
    let tasks = read_repo_file(FEATURE_TASKS);
    let graph = load(GRAPH_JSON);
    let material_contract = |markdown: &str| {
        markdown_json_fences(markdown)
            .into_iter()
            .find_map(|fence| {
                (fence.closed)
                    .then(|| parse_json_without_duplicates(&fence.body).expect("contract JSON"))
            })
            .expect("feature-local contract")
    };
    let original_material_contract = material_contract(&tasks);
    for (from, to) in [
        ("\"schema_version\": 1", "\"schema_version\": 2"),
        (
            "\"task_ids\": [\"T606\", \"T607\", \"T608\", \"T609\", \"T604\", \"T479\", \"T480\"]",
            "\"task_ids\": [\"T606\", \"T607\", \"T608\", \"T609\", \"T604\", \"T479\"]",
        ),
        ("\"T607\": [\"T606\"]", "\"T607\": []"),
        (
            "\"T604\": [\"T221\", \"T607\", \"T608\", \"T609\"]",
            "\"T604\": [\"T221\"]",
        ),
        ("\"Merged\": []", "\"Merged\": [\"Dispatched\"]"),
        ("\"w6-shared-prep-inventory\"", "\"w6-shared-prep-missing\""),
        (
            "\"wi:ADR-046-provider-activation-nixos\": [\"T606\", \"T607\", \"T608\", \"T609\"]",
            "\"wi:ADR-046-provider-activation-nixos\": [\"T606\"]",
        ),
        (
            "\"artifact_kind\": \"d2b-feature-local/dispatch-ledger\"",
            "\"artifact_kind\": \"d2b-feature-local/dispatch-ledger-v2\"",
        ),
        (
            "\"NotLaunched\", \"Dispatched\", \"Validated\", \"Completed\", \"Blocked\"",
            "\"NotLaunched\", \"Completed\", \"Validated\", \"Dispatched\", \"Blocked\"",
        ),
        (
            "\"first_call_command_evidence_count\": 0",
            "\"first_call_command_evidence_count\": 1",
        ),
        ("\"import_cardinality\": 8", "\"import_cardinality\": 7"),
        (
            "\"records_must_preexist_before_discovery\": false",
            "\"records_must_preexist_before_discovery\": true",
        ),
        (
            "\"artifact_kind\": \"d2b-feature-local/plan-approval\"",
            "\"artifact_kind\": \"d2b-feature-local/plan-approval-v2\"",
        ),
        ("\"result\": \"approved\"", "\"result\": \"rejected\""),
        (
            "\"status_only_updates_do_not_invalidate_after_first_dispatch\": true",
            "\"status_only_updates_do_not_invalidate_after_first_dispatch\": false",
        ),
        (
            "\"packages/d2b-priv-broker/src/audit.rs\"",
            "\"packages/d2b-priv-broker/src/other.rs\"",
        ),
        (
            "\"packages/d2b-provider-activation-nixos/\": \"wi:ADR-046-provider-activation-nixos\"",
            "\"packages/d2b-provider-activation-nixos/\": \"wi:ADR-046-provider-audio-pipewire\"",
        ),
        (
            "\"single_foundation_owner\": \"T609\"",
            "\"single_foundation_owner\": \"T608\"",
        ),
        (
            "\"tpm_before_first_ensure\": [",
            "\"tpm_before_first_ensure\": []",
        ),
        (
            "\"source_wave_label\": \"W5\"",
            "\"source_wave_label\": \"W4\"",
        ),
        ("\"ADR046-cli-001\"", "\"ADR046-cli-999\""),
        ("\"ADR046-ch-001\"", "\"ADR046-ch-999\""),
        ("\"expected_count\": 258", "\"expected_count\": 257"),
        (
            "\"Makefile\": [\"T606\", \"ADR046-ch-001\", \"T604\"]",
            "\"Makefile\": [\"T606\", \"T604\", \"ADR046-ch-001\"]",
        ),
        (
            "\"packages/Cargo.toml\": [\"T606\", \"T479\"]",
            "\"packages/Cargo.toml\": [\"T604\"]",
        ),
        (
            "\"T606\": [\n      \"packages/Cargo.toml\"",
            "\"T610\": [\n      \"packages/Cargo.toml\"",
        ),
        (
            "\"candidate_record_count\": 1",
            "\"candidate_record_count\": 2",
        ),
        (
            "\"close_revalidator\": \"T480\"",
            "\"close_revalidator\": \"T479\"",
        ),
        ("\"freeze-f6\"", "\"emit-before-freeze\""),
        (
            "\"t604_candidate_bound_evidence\": false",
            "\"t604_candidate_bound_evidence\": true",
        ),
        ("\"Volume/acceptance-state\"", "\"Volume/acceptance-other\""),
    ] {
        assert!(tasks.contains(from), "mutation source missing: {from}");
        let mutated = tasks.replacen(from, to, 1);
        assert!(
            !check_local_coordination_tasks(&mutated, &graph).is_empty(),
            "mutation unexpectedly passed: {from} -> {to}"
        );
    }

    let without_legacy_phrase = tasks.replace(
        "FEATURE-LOCAL COORDINATION/COMPLETION",
        "LOCAL COORDINATION",
    );
    let phrase_findings = check_local_coordination_tasks(&without_legacy_phrase, &graph);
    assert!(
        !phrase_findings.is_empty(),
        "a changed feature-local task label unexpectedly passed"
    );

    for task_id in EXPECTED_LOCAL_TASK_IDS {
        let prefix = format!("- [ ] {task_id}");
        let original_line = tasks
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("local task row missing from fixture: {task_id}"));
        let checked_line = original_line.replacen("- [ ]", "- [x]", 1);
        let checked = tasks.replacen(original_line, &checked_line, 1);
        let checked_material_contract = material_contract(&checked);
        assert!(
            canonical_json(&checked_material_contract)
                == canonical_json(&original_material_contract),
            "a checked local task projection changed canonical material contract: {task_id}"
        );

        let fenced = tasks.replacen(
            original_line,
            &format!(
                "<!-- RETIRED-CONTRACT-TEST-BEGIN -->\n{original_line}\n\
                 <!-- RETIRED-CONTRACT-TEST-END -->"
            ),
            1,
        );
        assert!(
            !check_local_coordination_tasks(&fenced, &graph).is_empty(),
            "retired-fenced local task row unexpectedly passed: {task_id}"
        );
    }

    let duplicated = format!(
        "{tasks}\n```json\n{{\"artifact_kind\":\"d2b-feature-local-task-contract\"}}\n```\n"
    );
    assert!(
        !check_local_coordination_tasks(&duplicated, &graph).is_empty(),
        "duplicate local-task contract unexpectedly passed"
    );
    let duplicate_key = tasks.replacen(
        "\"schema_version\": 1",
        "\"schema_version\": 1,\n  \"schema_version\": 1",
        1,
    );
    assert!(
        !check_local_coordination_tasks(&duplicate_key, &graph).is_empty(),
        "duplicate JSON key unexpectedly passed"
    );
    let malformed = format!(
        "{tasks}\n```json\n{{\"artifact_kind\":\"d2b-feature-local-task-contract\",\n```\n"
    );
    assert!(
        !check_local_coordination_tasks(&malformed, &graph).is_empty(),
        "malformed competing local-task contract unexpectedly passed"
    );
    let spaced_fence = tasks.replacen("```json", "``` json", 1)
        + "\n``` json\n{\"artifact_kind\":\"d2b-feature-local-task-contract\"}\n```\n";
    assert!(
        !check_local_coordination_tasks(&spaced_fence, &graph).is_empty(),
        "spaced duplicate local-task contract unexpectedly passed"
    );
    let escaped_kind = format!(
        "{tasks}\n```json\n{{\"artifact_kind\":\"d2b-feature-local-task-\\u0063ontract\"}}\n```\n"
    );
    assert!(
        !check_local_coordination_tasks(&escaped_kind, &graph).is_empty(),
        "escaped duplicate local-task contract unexpectedly passed"
    );
    let malformed_escaped_kind = format!(
        "{tasks}\n``` json\n{{\"artifact_kind\":\"d2b-feature-local-task-\\u0063ontract\",\n```\n"
    );
    assert!(
        !check_local_coordination_tasks(&malformed_escaped_kind, &graph).is_empty(),
        "malformed escaped competing local-task contract unexpectedly passed"
    );
    for (label, opening, closing) in [
        ("long backtick", "````json", "````"),
        ("tilde", "~~~ json", "~~~"),
        (
            "case-insensitive attributed",
            "````` JSON contract=local",
            "`````",
        ),
    ] {
        let duplicate = format!(
            "{tasks}\n{opening}\n{{\"artifact_kind\":\"d2b-feature-local-task-contract\"}}\n{closing}\n"
        );
        assert!(
            !check_local_coordination_tasks(&duplicate, &graph).is_empty(),
            "{label} duplicate local-task contract unexpectedly passed"
        );
        let malformed = format!(
            "{tasks}\n{opening}\n{{\"artifact_kind\":\"d2b-feature-local-task-\\u0063ontract\",\n{closing}\n"
        );
        assert!(
            !check_local_coordination_tasks(&malformed, &graph).is_empty(),
            "{label} malformed local-task contract unexpectedly passed"
        );
    }
    let unclosed =
        format!("{tasks}\n````json\n{{\"artifact_kind\":\"d2b-feature-local-task-contract\"}}\n");
    assert!(
        !check_local_coordination_tasks(&unclosed, &graph).is_empty(),
        "unclosed local-task contract unexpectedly passed"
    );
    for (label, pseudo_closer) in [
        ("overindented", "    ````"),
        ("tab-indented", "\t````"),
        ("unicode-whitespace", "````\u{00a0}"),
    ] {
        let malformed = format!(
            "{tasks}\n````json\n{{\"artifact_kind\":\"other\"}}\n{pseudo_closer}\n\
             {{\"artifact_kind\":\"d2b-feature-local-task-contract\"}}\n`````\n"
        );
        assert!(
            !check_local_coordination_tasks(&malformed, &graph).is_empty(),
            "{label} pseudo-closer unexpectedly hid a competing contract"
        );
    }
}

#[test]
fn feature_local_semantic_branches_reject_independent_mutations() {
    let tasks = read_repo_file(FEATURE_TASKS);
    let contract = markdown_json_fences(&tasks)
        .into_iter()
        .find_map(|fence| {
            (fence.closed)
                .then(|| parse_json_without_duplicates(&fence.body).expect("contract JSON"))
        })
        .expect("feature-local contract");
    let graph = load(GRAPH_JSON);
    let manifest = load(WORK_ITEMS);

    let mut findings = Vec::new();
    check_local_completion_contract(&contract, &graph, &manifest, &mut findings);
    // The feature editor is consolidating the shared-writer contract in a
    // follow-up. Keep the mutation assertions below independent of that
    // pending material change rather than treating today's handoff draft as
    // a valid fixture.

    let mut state_mutation = contract.clone();
    state_mutation["local_completion_state_machine"]["transitions"]["Planned"] =
        serde_json::json!(["Validated"]);
    findings.clear();
    check_local_completion_contract(&state_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "a non-monotonic local transition unexpectedly passed"
    );

    let mut entry_prepare_mutation = contract.clone();
    entry_prepare_mutation["entry_prepare_contract"]["import_cardinality"] = serde_json::json!(7);
    findings.clear();
    check_local_completion_contract(&entry_prepare_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "an incomplete entry-prepare import contract unexpectedly passed"
    );

    let mut foundation_mutation = contract.clone();
    foundation_mutation["manifest_group_foundations"]
        .as_object_mut()
        .expect("foundation map")
        .remove("wi:ADR-046-provider-activation-nixos");
    findings.clear();
    check_local_completion_contract(&foundation_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "a missing manifest foundation mapping unexpectedly passed"
    );

    let mut dispatch_mutation = contract.clone();
    dispatch_mutation["dispatch_ledger_contract"]["entry_required_fields"] =
        serde_json::json!(["group"]);
    findings.clear();
    check_local_completion_contract(&dispatch_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "an incomplete dispatch-ledger entry schema unexpectedly passed"
    );

    let mut receipt_mutation = contract.clone();
    receipt_mutation["plan_approval_receipt_contract"]["required_values"]["result"] =
        serde_json::json!("rejected");
    findings.clear();
    check_local_completion_contract(&receipt_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "a rejected plan-approval receipt schema unexpectedly passed"
    );

    let mut handoff_mutation = contract.clone();
    handoff_mutation["local_to_manifest_shared_writer_handoffs"]["handoffs"][0]["order"][1] =
        serde_json::json!("T999");
    findings.clear();
    check_shared_writer_handoffs(&handoff_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "an unresolved shared-writer order endpoint unexpectedly passed"
    );

    let mut scaffold_mutation = contract.clone();
    scaffold_mutation["local_to_manifest_shared_writer_handoffs"]["scaffold_handoffs"]["packages/d2b-provider-activation-nixos/"] =
        serde_json::json!("wi:ADR-046-provider-audio-pipewire");
    findings.clear();
    check_shared_writer_handoffs(&scaffold_mutation, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "a wrong scaffold-to-group handoff unexpectedly passed"
    );

    let mut missing_overlap = contract.clone();
    missing_overlap["local_to_manifest_shared_writer_handoffs"]["handoffs"][0]["paths"]
        .as_array_mut()
        .expect("broker handoff paths")
        .retain(|path| path != "packages/d2b-contracts/src/broker_wire.rs");
    findings.clear();
    check_shared_writer_handoffs(&missing_overlap, &graph, &manifest, &mut findings);
    assert!(
        !findings.is_empty(),
        "a missing normalized local-owned overlap unexpectedly passed"
    );

    let mut extra_overlap = manifest.clone();
    let item = extra_overlap["items"]
        .as_array_mut()
        .expect("manifest items")
        .iter_mut()
        .find(|item| item["workItemId"] == "ADR046-vl-011")
        .expect("provider policy item");
    item["destination"] = serde_json::json!(
        "`packages/xtask/src/provider_crate_policy.rs`; `packages/d2b-contracts/src/v3/volume.rs`"
    );
    findings.clear();
    check_shared_writer_handoffs(&contract, &graph, &extra_overlap, &mut findings);
    assert!(
        !findings.is_empty(),
        "an extra normalized local-owned overlap unexpectedly passed"
    );
}

#[test]
fn destination_overlap_resolution_covers_parent_child_glob_and_provider_paths() {
    assert!(destination_paths_overlap(
        "packages/example/src/file.rs",
        "packages/example/"
    ));
    assert!(destination_paths_overlap(
        "packages/example/src/",
        "packages/example/src/file.rs"
    ));
    assert!(destination_paths_overlap(
        "packages/example/src/file.rs",
        "packages/example/src/*"
    ));
    assert!(destination_paths_overlap(
        "packages/d2b-provider-demo/src/file.rs",
        "packages/d2b-provider-<base>-<implementation>/"
    ));
    assert!(!destination_paths_overlap(
        "packages/other/src/file.rs",
        "packages/example/src/*"
    ));

    let atoms = normalized_destination_atoms(
        "`src/controller.rs`; `tests/controller.rs`; `integration/real.rs`",
        Some("packages/d2b-provider-demo/"),
    );
    assert!(atoms.contains("packages/d2b-provider-demo/src/controller.rs"));
    assert!(atoms.contains("packages/d2b-provider-demo/tests/controller.rs"));
    assert!(atoms.contains("packages/d2b-provider-demo/integration/real.rs"));
    assert!(!atoms.contains("src/controller.rs"));
}

#[test]
fn shared_writer_policy_rejects_duplicate_paths_missing_owners_and_graph_gaps() {
    let tasks = read_repo_file(FEATURE_TASKS);
    let contract = markdown_json_fences(&tasks)
        .into_iter()
        .find_map(|fence| {
            (fence.closed)
                .then(|| parse_json_without_duplicates(&fence.body).expect("contract JSON"))
        })
        .expect("feature-local contract");
    let mut graph = load(GRAPH_JSON);
    let manifest = load(WORK_ITEMS);

    let mut duplicate = contract.clone();
    let duplicate_path =
        duplicate["local_to_manifest_shared_writer_handoffs"]["handoffs"][0]["paths"][0].clone();
    duplicate["local_to_manifest_shared_writer_handoffs"]["handoffs"][1]["paths"]
        .as_array_mut()
        .expect("audit handoff paths")
        .push(duplicate_path);
    let mut findings = Vec::new();
    check_shared_writer_handoffs(&duplicate, &graph, &manifest, &mut findings);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("appears in multiple handoffs")),
        "duplicate shared path was not rejected: {findings:?}"
    );

    let mut missing_owner = contract.clone();
    missing_owner["local_to_manifest_shared_writer_handoffs"]["handoffs"][0]["order"]
        .as_array_mut()
        .expect("broker handoff order")
        .retain(|endpoint| endpoint != "ADR046-transport-unix-006");
    findings.clear();
    check_shared_writer_handoffs(&missing_owner, &graph, &manifest, &mut findings);
    assert!(
        findings
            .iter()
            .any(|finding| { finding.contains("omits a local or manifest owner") }),
        "incomplete total writer order was not rejected: {findings:?}"
    );

    let transport = graph["nodes"]
        .as_array_mut()
        .expect("graph nodes")
        .iter_mut()
        .find(|node| node["id"] == "ADR046-transport-unix-006")
        .expect("transport handoff node");
    transport["prerequisites"] = serde_json::json!([]);
    let mut graph_gap_contract = contract.clone();
    graph_gap_contract["manifest_group_foundations"]["wi:ADR-046-provider-transport-unix"] =
        serde_json::json!([]);
    findings.clear();
    check_shared_writer_handoffs(&graph_gap_contract, &graph, &manifest, &mut findings);
    assert!(
        findings.iter().any(|finding| {
            finding.contains("adjacent order") && finding.contains("graph/readiness ordering")
        }),
        "graph gap between adjacent handoff writers was not rejected: {findings:?}"
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
        counts["waves"].as_u64(),
        Some(EXPECTED_WAVES),
        "the certified graph has {EXPECTED_WAVES} waves"
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
        graph["waves"].as_array().map(|waves| waves.len() as u64),
        Some(EXPECTED_WAVES),
        "the emitted graph must contain {EXPECTED_WAVES} waves"
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
        // The reviewing model must remain distinct from the model that writes
        // the code. Both are now GPT-5.6 siblings, so pin the authoring sibling
        // rather than rejecting the whole family and accidentally rejecting
        // the current `gpt-5.6-sol` panel binding. The comparison folds case so
        // a manifest spelling `GPT-5.6-Luna` cannot slip past it.
        assert!(
            !text.to_ascii_lowercase().contains("gpt-5.6-luna"),
            "`{rel}` references a coding-model binding; the panel model must stay \
             distinct from the model that writes the code"
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
            (
                "Evidence",
                "Destination and validation remain outstanding.".to_string(),
            ),
            ("Implementation state", "Planned".to_string()),
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

    /// The repository replaced every em-dash with a plain hyphen, which
    /// rewrote 112 of the 543 headings into a spelling whose separator is
    /// character-identical to the hyphens inside the ids. All five title
    /// spellings, dashed or not, must keep yielding the declared id intact;
    /// the em-dash spelling stays covered (as an escape, never a literal)
    /// because the parser still has to tolerate it in inbound Markdown.
    #[test]
    fn every_title_spelling_yields_the_declared_id() {
        for id in ["ADR046-core-001", "ADR046-security-key-012"] {
            let titles = [
                String::new(),
                format!(" \u{2014} {id} title"),
                format!(" - {id} title"),
                format!(": {id} title"),
                format!(" ({id} title)"),
                format!(" {id} title"),
            ];
            for title in titles {
                let doc = format!("### {id}{title}\n\n{}\n", table(id, &[]));
                let found: Vec<String> =
                    declarations(&doc).into_iter().map(|item| item.id).collect();
                assert_eq!(
                    found,
                    vec![id.to_string()],
                    "heading `### {id}{title}` must declare exactly `{id}`"
                );
            }
        }
    }

    fn manifest_row(id: &str, action: &str, reuse: Value) -> Value {
        serde_json::json!({
            "currentSource": "none",
            "dataMigration": "None",
            "dependencyOwner": "fixture owner",
            "destination": "`packages/fixture/src/lib.rs`",
            "detailedDesign": "fixture design",
            "evidence": "Destination and validation remain outstanding.",
            "implementationState": "Planned",
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

    fn spec_paths() -> BTreeMap<String, String> {
        BTreeMap::from([(
            SPEC.to_string(),
            "docs/specs/ADR-046-fixture.md".to_string(),
        )])
    }

    fn declared(markdown: &str) -> BTreeMap<String, Vec<Declaration>> {
        BTreeMap::from([(SPEC.to_string(), declarations(markdown))])
    }

    fn run(md: &str, rows: Vec<Value>, prefixes: &[&str]) -> Vec<String> {
        check_work_items(&declared(md), &rows, &registry(prefixes), &spec_paths())
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
            &spec_paths(),
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
            &spec_paths(),
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
    fn a_missing_delivery_field_or_free_form_implementation_state_fails() {
        let valid = markdown("###", "ADR046-fixture-001", &[]);
        let row = manifest_row("ADR046-fixture-001", "create", Value::Null);
        for (field, row_text) in [
            ("Implementation state", "Planned"),
            ("Evidence", "Destination and validation remain outstanding."),
        ] {
            let missing_field = valid.replace(&format!("| {field} | {row_text} |\n"), "");
            let missing = run(&missing_field, vec![row.clone()], &["fixture"]);
            assert!(
                missing.iter().any(|finding| finding.contains(&format!(
                    "work item `ADR046-fixture-001` is missing mandatory field `{field}`"
                ))),
                "a missing {field} must fail, got {missing:?}"
            );
        }

        let free_form_state = valid.replace(
            "| Implementation state | Planned |",
            "| Implementation state | In progress |",
        );
        let mut invalid_row = row;
        invalid_row["implementationState"] = Value::String("In progress".to_string());
        let invalid = run(&free_form_state, vec![invalid_row], &["fixture"]);
        assert!(
            invalid
                .iter()
                .any(|finding| finding.contains("free-form implementation state `In progress`")),
            "a free-form implementation state must fail, got {invalid:?}"
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

    #[test]
    fn a_manifest_scalar_that_disagrees_with_the_markdown_fails() {
        // Every serialized scalar must round-trip from the Markdown declaration;
        // a generator regression that rewrites one silently must be caught.
        let cases: &[(&str, MutateRow)] = &[
            ("currentSource", |row| {
                row["currentSource"] = Value::String("drifted source".to_string())
            }),
            ("dataMigration", |row| {
                row["dataMigration"] = Value::String("drifted migration".to_string())
            }),
            ("dependencyOwner", |row| {
                row["dependencyOwner"] = Value::String("drifted owner".to_string())
            }),
            ("destination", |row| {
                row["destination"] = Value::String("`packages/drifted/src/lib.rs`".to_string())
            }),
            ("detailedDesign", |row| {
                row["detailedDesign"] = Value::String("drifted design".to_string())
            }),
            ("evidence", |row| {
                row["evidence"] = Value::String("drifted evidence".to_string())
            }),
            ("implementationState", |row| {
                row["implementationState"] = Value::String("Merged".to_string())
            }),
            ("integration", |row| {
                row["integration"] = Value::String("drifted integration".to_string())
            }),
            ("removalProof", |row| {
                row["removalProof"] = Value::String("drifted proof".to_string())
            }),
            ("validation", |row| {
                row["validation"] = Value::String("drifted validation".to_string())
            }),
        ];
        for (key, mutate) in cases {
            let md = markdown("###", "ADR046-fixture-001", &[]);
            let mut row = manifest_row("ADR046-fixture-001", "create", Value::Null);
            mutate(&mut row);
            let findings = run(&md, vec![row], &["fixture"]);
            assert!(
                findings
                    .iter()
                    .any(|f| f.contains(&format!("manifest `{key}`"))),
                "a drifted `{key}` must be reported, got {findings:?}"
            );
        }
    }

    #[test]
    fn a_manifest_reuse_action_that_disagrees_with_the_markdown_fails() {
        // A *valid* action that differs from the Markdown passes the vocabulary
        // check but must still fail the value comparison.
        let md = markdown("###", "ADR046-fixture-001", &[("Reuse action", "adapt")]);
        let row = manifest_row("ADR046-fixture-001", "wrap", Value::Null);
        let findings = run(&md, vec![row], &["fixture"]);
        assert!(
            findings
                .iter()
                .any(|f| f.contains("manifest `reuseAction`")),
            "a valid-but-mismatched reuse action must be reported, got {findings:?}"
        );
    }

    #[test]
    fn a_manifest_reuse_source_that_disagrees_with_the_markdown_fails() {
        // Markdown declares a reuse source under a non-create action; the
        // manifest must carry that exact string, not null or a different value.
        let md = markdown(
            "###",
            "ADR046-fixture-001",
            &[
                ("Reuse action", "adapt"),
                ("Reuse source", "`packages/original/src/lib.rs`"),
            ],
        );
        let dropped = run(
            &md,
            vec![manifest_row("ADR046-fixture-001", "adapt", Value::Null)],
            &["fixture"],
        );
        assert!(
            dropped.iter().any(|f| f.contains("manifest reuseSource")),
            "a dropped reuse source must be reported, got {dropped:?}"
        );

        let rewritten = run(
            &md,
            vec![manifest_row(
                "ADR046-fixture-001",
                "adapt",
                Value::String("`packages/wrong/src/lib.rs`".to_string()),
            )],
            &["fixture"],
        );
        assert!(
            rewritten.iter().any(|f| f.contains("manifest reuseSource")),
            "a rewritten reuse source must be reported, got {rewritten:?}"
        );
    }

    #[test]
    fn a_manifest_spec_path_that_disagrees_with_the_member_fails() {
        let md = markdown("###", "ADR046-fixture-001", &[]);
        let mut row = manifest_row("ADR046-fixture-001", "create", Value::Null);
        row["specPath"] = Value::String("docs/specs/ADR-046-wrong.md".to_string());
        let findings = run(&md, vec![row], &["fixture"]);
        assert!(
            findings.iter().any(|f| f.contains("manifest specPath")),
            "a drifted specPath must be reported, got {findings:?}"
        );
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
