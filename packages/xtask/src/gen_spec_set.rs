//! Generator for the ADR 0046 spec-set and work-item manifests.
//!
//! Reads every `docs/specs/ADR-046-*.md` and
//! `docs/specs/providers/ADR-046-provider-*.md` member, its metadata table, its
//! content digest, and every `### ADR046-<registered-prefix>-<ordinal>` work
//! item, then emits `docs/specs/ADR-046-spec-set.json` and
//! `docs/specs/ADR-046-work-items.json`.
//!
//! Generation is fail-closed: a malformed metadata table, a wrong-level or
//! malformed work-item heading, a duplicate or unregistered work-item id, a
//! missing or duplicated mandatory field, a free-form reuse action, or a
//! `create` item that names a reuse source aborts before any file is written.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const SPEC_SET_PATH: &str = "docs/specs/ADR-046-spec-set.json";
pub const WORK_ITEMS_PATH: &str = "docs/specs/ADR-046-work-items.json";

const SPECS_DIR: &str = "docs/specs";
const PROVIDERS_DIR: &str = "docs/specs/providers";
const SPEC_FILE_PREFIX: &str = "ADR-046-";
const WORK_ITEM_ID_PREFIX: &str = "ADR046-";
const PARENT_ADR: &str = "docs/adr/0046-d2b-3-provider-control-plane.md";
const ADR: &str = "0046";
const SPEC_SET_ARTIFACT_KIND: &str = "d2b-adr-spec-set";
const SPEC_SET_SCHEMA_VERSION: u32 = 3;
const WORK_ITEMS_ARTIFACT_KIND: &str = "d2b-adr-work-items";
const WORK_ITEMS_SCHEMA_VERSION: u32 = 1;

/// The ADR 0046 set is a fixed, closed corpus. A parser regression that
/// silently finds fewer members or work items must fail the generator rather
/// than quietly shrink the manifests.
const EXPECTED_MEMBERS: usize = 55;
const EXPECTED_WORK_ITEMS: usize = 543;

/// The four spellings a work-item heading uses. All four are load-bearing:
/// anchoring on one shape silently drops the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HeadingForm {
    /// `### ADR046-core-001`
    Bare,
    /// A title introduced by whitespace and, optionally, a dash of any kind:
    /// `### ADR046-core-001 - Some title`, the en-dash spelling, and the 19
    /// headings that introduce the same title with no dash at all. They spell
    /// one form because the dash is decoration, not grammar.
    Dash,
    /// `### ADR046-core-001: Some title`
    Colon,
    /// `### ADR046-core-001 (Some title)`
    Parenthetical,
}

/// The recorded heading-form distribution across the 543 work items. A parser
/// that stops matching one spelling changes this census, which fails the
/// generator instead of silently shrinking the manifest.
const EXPECTED_HEADING_FORMS: &[(HeadingForm, usize)] = &[
    (HeadingForm::Bare, 356),
    (HeadingForm::Dash, 112),
    (HeadingForm::Colon, 51),
    (HeadingForm::Parenthetical, 24),
];

impl HeadingForm {
    fn label(self) -> &'static str {
        match self {
            Self::Bare => "bare",
            Self::Dash => "dash title",
            Self::Colon => "colon title",
            Self::Parenthetical => "parenthetical title",
        }
    }
}

/// Classifies the heading remainder that follows a work-item id.
fn heading_form(rest: &str, id: &str) -> HeadingForm {
    let tail = rest
        .trim_start_matches('`')
        .strip_prefix(id)
        .unwrap_or("")
        .trim_start_matches('`');
    let trimmed = tail.trim_start();
    if trimmed.is_empty() {
        HeadingForm::Bare
    } else if trimmed.starts_with(':') {
        HeadingForm::Colon
    } else if trimmed.starts_with('(') {
        HeadingForm::Parenthetical
    } else {
        HeadingForm::Dash
    }
}

/// The closed `Reuse action` scalar domain.
const REUSE_ACTIONS: &[&str] = &[
    "adapt",
    "copy-unchanged",
    "create",
    "delete-after-cutover",
    "extract",
    "replace",
    "wrap",
];

/// Mandatory work-item table fields, in manifest field order.
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

/// A sentinel cell value meaning "no value", serialized as JSON `null`.
fn is_none_sentinel(value: &str) -> bool {
    value == "None"
        || value
            .strip_prefix("None")
            .is_some_and(|rest| rest.starts_with('.') || rest.starts_with(','))
}

#[derive(Debug)]
pub struct GenError(String);

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GenError {}

fn err<T>(message: impl Into<String>) -> Result<T, GenError> {
    Err(GenError(message.into()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecSetMember {
    pub depends_on: Vec<String>,
    pub path: String,
    pub sha256: String,
    pub spec_id: String,
    pub status: String,
    pub supersedes: Option<String>,
    pub version: u32,
    pub work_item_prefixes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecSetDoc {
    pub adr: String,
    pub artifact_kind: String,
    pub baseline: String,
    pub members: Vec<SpecSetMember>,
    pub parent: String,
    pub schema_version: u32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemEntry {
    pub current_source: String,
    pub data_migration: String,
    pub dependency_owner: String,
    pub destination: String,
    pub detailed_design: String,
    pub integration: String,
    pub removal_proof: String,
    pub reuse_action: String,
    pub reuse_source: Option<String>,
    pub spec_id: String,
    pub spec_path: String,
    pub validation: String,
    pub work_item_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemsDoc {
    pub adr: String,
    pub artifact_kind: String,
    pub items: Vec<WorkItemEntry>,
    pub schema_version: u32,
    pub status: String,
}

/// Both manifests, held together so callers validate one consistent parse.
#[derive(Debug, Clone)]
pub struct Manifests {
    pub spec_set: SpecSetDoc,
    pub work_items: WorkItemsDoc,
    /// Census of the heading spellings the parser recognized, so a shape that
    /// stops being matched is visible rather than silently absent.
    pub heading_forms: BTreeMap<HeadingForm, usize>,
}

/// Regenerates both manifests under `root` and returns the written paths.
pub fn generate(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let manifests = build(root)?;
    let spec_set_path = root.join(SPEC_SET_PATH);
    let work_items_path = root.join(WORK_ITEMS_PATH);
    fs::write(&spec_set_path, render_json(&manifests.spec_set)?)?;
    fs::write(&work_items_path, render_json(&manifests.work_items)?)?;
    let census = manifests
        .heading_forms
        .iter()
        .map(|(form, count)| format!("{count} {}", form.label()))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "spec-registry: {} members, {} work items ({census})",
        manifests.spec_set.members.len(),
        manifests.work_items.items.len()
    );
    Ok(vec![spec_set_path, work_items_path])
}

/// Parses and validates the member Markdown, returning both manifests.
pub fn build(root: &Path) -> Result<Manifests, Box<dyn std::error::Error>> {
    let specs = parse_members(root)?;
    let prefix_owner = registry(&specs)?;

    let mut members = Vec::with_capacity(specs.len());
    let mut items: Vec<WorkItemEntry> = Vec::new();
    let mut statuses = BTreeSet::new();
    let mut baselines = BTreeSet::new();
    let mut seen_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut heading_forms: BTreeMap<HeadingForm, usize> = BTreeMap::new();

    for spec in &specs {
        statuses.insert(spec.status.clone());
        baselines.insert(spec.baseline.clone());

        let mut prefixes: BTreeSet<String> = BTreeSet::new();
        for item in &spec.items {
            if let Some(previous) = seen_ids.insert(item.id.clone(), spec.spec_id.clone()) {
                return err(format!(
                    "duplicate work item id `{}` declared by `{previous}` and `{}`",
                    item.id, spec.spec_id
                ))?;
            }
            let owner = prefix_owner
                .get(&item.prefix)
                .expect("registry is complete");
            if owner != &spec.spec_id {
                return err(format!(
                    "work item `{}` uses prefix `{}` registered to `{owner}`, not `{}`",
                    item.id, item.prefix, spec.spec_id
                ))?;
            }
            prefixes.insert(item.prefix.clone());
            *heading_forms.entry(item.heading_form).or_default() += 1;
            items.push(item.to_entry(spec)?);
        }

        members.push(SpecSetMember {
            depends_on: spec.depends_on.iter().cloned().collect(),
            path: spec.rel_path.clone(),
            sha256: spec.sha256.clone(),
            spec_id: spec.spec_id.clone(),
            status: spec.status.clone(),
            supersedes: spec.supersedes.clone(),
            version: spec.version,
            work_item_prefixes: prefixes.into_iter().collect(),
        });
    }

    if statuses.len() != 1 {
        return err(format!(
            "the ADR 0046 set is atomic: every member must share one status, found {statuses:?}"
        ))?;
    }
    if baselines.len() != 1 {
        return err(format!(
            "every member must record one baseline commit, found {baselines:?}"
        ))?;
    }
    if members.len() != EXPECTED_MEMBERS {
        return err(format!(
            "expected exactly {EXPECTED_MEMBERS} ADR 0046 member specs, parsed {}; \
             a member was added, removed, or is no longer recognized by its metadata table",
            members.len()
        ))?;
    }
    if items.len() != EXPECTED_WORK_ITEMS {
        return err(format!(
            "expected exactly {EXPECTED_WORK_ITEMS} ADR 0046 work items, parsed {}; \
             a heading form or an item-owning section title is no longer recognized",
            items.len()
        ))?;
    }
    for (form, expected) in EXPECTED_HEADING_FORMS {
        let found = heading_forms.get(form).copied().unwrap_or(0);
        if found != *expected {
            return err(format!(
                "expected {expected} `{}` work-item headings, matched {found}; \
                 the heading parser no longer recognizes this spelling",
                form.label()
            ))?;
        }
    }

    members.sort_by(|a, b| a.spec_id.cmp(&b.spec_id));
    items.sort_by(|a, b| a.work_item_id.cmp(&b.work_item_id));

    let status = statuses.into_iter().next().expect("exactly one status");
    let baseline = baselines.into_iter().next().expect("exactly one baseline");

    Ok(Manifests {
        heading_forms,
        spec_set: SpecSetDoc {
            adr: ADR.to_string(),
            artifact_kind: SPEC_SET_ARTIFACT_KIND.to_string(),
            baseline,
            members,
            parent: PARENT_ADR.to_string(),
            schema_version: SPEC_SET_SCHEMA_VERSION,
            status: status.clone(),
        },
        work_items: WorkItemsDoc {
            adr: ADR.to_string(),
            artifact_kind: WORK_ITEMS_ARTIFACT_KIND.to_string(),
            items,
            schema_version: WORK_ITEMS_SCHEMA_VERSION,
            status,
        },
    })
}

// ---------------------------------------------------------------------------
// Markdown parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ParsedSpec {
    spec_id: String,
    rel_path: String,
    status: String,
    version: u32,
    baseline: String,
    depends_on: BTreeSet<String>,
    depends_globs: Vec<String>,
    supersedes: Option<String>,
    sha256: String,
    items: Vec<ParsedWorkItem>,
}

#[derive(Debug, Clone)]
struct ParsedWorkItem {
    id: String,
    prefix: String,
    heading_form: HeadingForm,
    fields: BTreeMap<String, String>,
}

impl ParsedWorkItem {
    fn field(&self, name: &str) -> Result<String, GenError> {
        self.fields
            .get(name)
            .cloned()
            .ok_or_else(|| GenError(format!("work item `{}` is missing `{name}`", self.id)))
    }

    fn to_entry(&self, spec: &ParsedSpec) -> Result<WorkItemEntry, GenError> {
        let reuse_action = self.field("Reuse action")?;
        if !REUSE_ACTIONS.contains(&reuse_action.as_str()) {
            return Err(GenError(format!(
                "work item `{}` declares free-form reuse action `{reuse_action}`; expected one of {REUSE_ACTIONS:?}",
                self.id
            )));
        }
        let reuse_source = match self.fields.get("Reuse source") {
            Some(value) if !is_none_sentinel(value) => Some(value.clone()),
            _ => None,
        };
        if reuse_action == "create" && reuse_source.is_some() {
            return Err(GenError(format!(
                "work item `{}` declares `create` with a reuse source",
                self.id
            )));
        }
        Ok(WorkItemEntry {
            current_source: self.field("Current source")?,
            data_migration: self.field("Data migration")?,
            dependency_owner: self.field("Dependency/owner")?,
            destination: self.field("Destination")?,
            detailed_design: self.field("Detailed design")?,
            integration: self.field("Integration")?,
            removal_proof: self.field("Removal proof")?,
            reuse_action,
            reuse_source,
            spec_id: spec.spec_id.clone(),
            spec_path: spec.rel_path.clone(),
            validation: self.field("Validation")?,
            work_item_id: self.id.clone(),
        })
    }
}

fn parse_members(root: &Path) -> Result<Vec<ParsedSpec>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    files.extend(member_files(&root.join(SPECS_DIR), SPECS_DIR)?);
    files.extend(member_files(&root.join(PROVIDERS_DIR), PROVIDERS_DIR)?);
    files.sort();

    let mut specs = Vec::new();
    for rel_path in files {
        let bytes = fs::read(root.join(&rel_path))?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| GenError(format!("{rel_path} is not valid UTF-8")))?;
        let Some(metadata) = metadata_table(&text) else {
            // A generated view (for example the implementation graph) carries no
            // metadata table and is not a member of the set.
            continue;
        };
        specs.push(parse_spec(&rel_path, &text, &bytes, &metadata)?);
    }

    let ids: BTreeSet<String> = specs.iter().map(|s| s.spec_id.clone()).collect();
    if ids.len() != specs.len() {
        return err("duplicate Spec ID declared across the ADR 0046 member set")?;
    }
    for spec in &mut specs {
        let mut resolved = BTreeSet::new();
        for dependency in &spec.depends_on {
            if !ids.contains(dependency.as_str()) {
                return err(format!(
                    "`{}` depends on `{dependency}`, which is not a member of the set",
                    spec.spec_id
                ))?;
            }
            resolved.insert(dependency.clone());
        }
        for glob in &spec.depends_globs {
            let matched: Vec<String> = ids
                .iter()
                .filter(|id| id.starts_with(glob.as_str()))
                .cloned()
                .collect();
            if matched.is_empty() {
                return err(format!(
                    "`{}` depends on `{glob}*`, which matches no member",
                    spec.spec_id
                ))?;
            }
            resolved.extend(matched);
        }
        resolved.remove(&spec.spec_id);
        spec.depends_on = resolved;
    }
    Ok(specs)
}

fn member_files(dir: &Path, rel_dir: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(SPEC_FILE_PREFIX) && name.ends_with(".md") {
            out.push(format!("{rel_dir}/{name}"));
        }
    }
    Ok(out)
}

/// Returns the first `| Field | Value |` table in the document, if any.
fn metadata_table(text: &str) -> Option<BTreeMap<String, String>> {
    let mut rows = BTreeMap::new();
    let mut in_table = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with('|') {
            in_table = true;
            if let Some((label, value)) = table_row(trimmed)
                && !label.chars().all(|c| c == '-' || c == ' ' || c == ':')
            {
                rows.insert(label, value);
            }
        } else if in_table {
            break;
        }
    }
    if rows.contains_key("Spec ID") {
        Some(rows)
    } else {
        None
    }
}

/// Splits `| label | value |` into its label and its raw value.
///
/// The value keeps every remaining character verbatim except a Markdown
/// cell-escaped `\|`, which is unescaped to the literal `|` it stands for. An
/// already-unescaped `|` inside prose (for example an inline `a|b|c` enum) is
/// preserved, so the first separator is the only structural pipe.
fn table_row(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_end();
    let inner = trimmed.strip_prefix('|')?.strip_suffix('|')?;
    let (label, value) = inner.split_once('|')?;
    Some((unescape_cell(label.trim()), unescape_cell(value.trim())))
}

fn unescape_cell(value: &str) -> String {
    value.replace("\\|", "|")
}

fn parse_spec(
    rel_path: &str,
    text: &str,
    bytes: &[u8],
    metadata: &BTreeMap<String, String>,
) -> Result<ParsedSpec, Box<dyn std::error::Error>> {
    let spec_id = strip_code(
        metadata
            .get("Spec ID")
            .ok_or_else(|| GenError(format!("{rel_path} has no `Spec ID` row")))?,
    );
    let expected = rel_path
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".md"))
        .unwrap_or_default();
    if spec_id != expected {
        return err(format!(
            "{rel_path} declares Spec ID `{spec_id}`, which does not match its filename"
        ))?;
    }
    let status = metadata
        .get("Status")
        .map(|s| strip_code(s))
        .ok_or_else(|| GenError(format!("{rel_path} has no `Status` row")))?;
    let version: u32 = metadata
        .get("Version")
        .map(|s| strip_code(s))
        .ok_or_else(|| GenError(format!("{rel_path} has no `Version` row")))?
        .parse()
        .map_err(|_| GenError(format!("{rel_path} has a non-integer `Version`")))?;
    let baseline = metadata
        .get("Baseline")
        .map(|s| strip_code(s))
        .ok_or_else(|| GenError(format!("{rel_path} has no `Baseline` row")))?;
    let supersedes = metadata
        .get("Supersedes")
        .filter(|value| !is_none_sentinel(value))
        .cloned();

    let depends_raw = metadata
        .get("Depends on")
        .ok_or_else(|| GenError(format!("{rel_path} has no `Depends on` row")))?;
    let (depends_on, depends_globs) = scan_spec_refs(depends_raw);

    let items = parse_work_items(rel_path, text)?;

    Ok(ParsedSpec {
        spec_id,
        rel_path: rel_path.to_string(),
        status,
        version,
        baseline,
        depends_on,
        depends_globs,
        supersedes,
        sha256: hex_digest(bytes),
        items,
    })
}

/// Extracts `ADR-046-*` spec references and `ADR-046-<prefix>*` globs.
fn scan_spec_refs(text: &str) -> (BTreeSet<String>, Vec<String>) {
    let mut ids = BTreeSet::new();
    let mut globs = Vec::new();
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    while let Some(offset) = text[cursor..].find(SPEC_FILE_PREFIX) {
        let start = cursor + offset;
        let mut end = start + SPEC_FILE_PREFIX.len();
        while end < bytes.len() {
            let ch = bytes[end] as char;
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' {
                end += 1;
            } else {
                break;
            }
        }
        let mut token = text[start..end].trim_end_matches('-').to_string();
        if end < bytes.len() && bytes[end] == b'*' {
            // A `ADR-046-provider-*` glob expands to every matching member.
            globs.push(text[start..end].to_string());
            token.clear();
        }
        if !token.is_empty() && token.len() > SPEC_FILE_PREFIX.len() {
            ids.insert(token);
        }
        cursor = end.max(start + 1);
    }
    globs.sort();
    globs.dedup();
    (ids, globs)
}

fn parse_work_items(
    rel_path: &str,
    text: &str,
) -> Result<Vec<ParsedWorkItem>, Box<dyn std::error::Error>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut items = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if let Some((level, rest)) = heading(line)
            && let Some(id) = leading_work_item_id(rest)
        {
            if level != 3 {
                return err(format!(
                    "{rel_path} declares work item `{id}` at heading level {level}; only `###` is valid"
                ))?;
            }
            let (item, next) =
                parse_work_item(rel_path, &lines, index + 1, &id, heading_form(rest, &id))?;
            items.push(item);
            index = next;
            continue;
        }
        index += 1;
    }
    Ok(items)
}

fn heading(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let level = line.chars().take_while(|c| *c == '#').count();
    let rest = line[level..].trim_start();
    Some((level, rest))
}

/// Returns the canonical work-item id a heading declares, if it declares one.
///
/// Extraction is anchored on the id grammar, never on a title separator. A
/// heading may introduce its title with an em-dash, an en-dash, a hyphen, a
/// colon, a parenthesis, or nothing but whitespace, and ids themselves contain
/// hyphens, so splitting on a separator character truncates the id. Instead,
/// take the leading run of id-shaped characters and return the *shortest*
/// anchored slice of it that satisfies the grammar; everything after that is
/// title text whatever punctuation introduces it.
fn leading_work_item_id(rest: &str) -> Option<String> {
    let text = rest.trim_start_matches('`');
    if !text.starts_with(WORK_ITEM_ID_PREFIX) {
        return None;
    }
    let body: String = text
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let cuts = body
        .match_indices('-')
        .map(|(index, _)| index)
        .chain(std::iter::once(body.len()));
    for cut in cuts {
        let candidate = &body[..cut];
        if is_work_item_id(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn is_work_item_id(token: &str) -> bool {
    split_work_item_id(token).is_some()
}

/// Splits `ADR046-<prefix>-<ordinal>` into its prefix and ordinal.
fn split_work_item_id(token: &str) -> Option<(String, u32)> {
    let body = token.strip_prefix(WORK_ITEM_ID_PREFIX)?;
    let (prefix, ordinal) = body.rsplit_once('-')?;
    if ordinal.len() != 3 || !ordinal.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u32 = ordinal.parse().ok()?;
    if value == 0 {
        return None;
    }
    if prefix.is_empty() {
        return None;
    }
    let valid = prefix.split('-').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    if !valid {
        return None;
    }
    Some((prefix.to_string(), value))
}

fn parse_work_item(
    rel_path: &str,
    lines: &[&str],
    start: usize,
    id: &str,
    heading_form: HeadingForm,
) -> Result<(ParsedWorkItem, usize), Box<dyn std::error::Error>> {
    let (prefix, _) = split_work_item_id(id).expect("caller validated the id");
    let mut index = start;
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }
    if index >= lines.len() || !lines[index].starts_with('|') {
        return err(format!(
            "{rel_path}: work item `{id}` is not followed by a field table"
        ))?;
    }
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    while index < lines.len() && lines[index].starts_with('|') {
        if let Some((label, value)) = table_row(lines[index]) {
            let separator = label.chars().all(|c| c == '-' || c == ' ' || c == ':');
            if !separator && label != "Field" && fields.insert(label.clone(), value).is_some() {
                return err(format!(
                    "{rel_path}: work item `{id}` declares `{label}` more than once"
                ))?;
            }
        }
        index += 1;
    }

    for field in MANDATORY_FIELDS {
        match fields.get(*field) {
            None => {
                return err(format!(
                    "{rel_path}: work item `{id}` is missing mandatory field `{field}`"
                ))?;
            }
            Some(value) if value.is_empty() => {
                return err(format!(
                    "{rel_path}: work item `{id}` has an empty `{field}`"
                ))?;
            }
            Some(_) => {}
        }
    }
    if let Some(declared) = fields.get("Work item ID") {
        let declared = strip_code(declared);
        if declared != id {
            return err(format!(
                "{rel_path}: work item heading `{id}` disagrees with its `Work item ID` row `{declared}`"
            ))?;
        }
    }

    Ok((
        ParsedWorkItem {
            id: id.to_string(),
            prefix,
            heading_form,
            fields,
        },
        index,
    ))
}

/// Builds the global prefix registry, rejecting any prefix owned twice.
fn registry(specs: &[ParsedSpec]) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    for spec in specs {
        for item in &spec.items {
            match owners.get(&item.prefix) {
                Some(owner) if owner != &spec.spec_id => {
                    return err(format!(
                        "work-item prefix `{}` is claimed by both `{owner}` and `{}`",
                        item.prefix, spec.spec_id
                    ))?;
                }
                _ => {
                    owners.insert(item.prefix.clone(), spec.spec_id.clone());
                }
            }
        }
    }
    Ok(owners)
}

fn strip_code(value: &str) -> String {
    value.trim().trim_matches('`').trim().to_string()
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// ---------------------------------------------------------------------------
// Deterministic JSON rendering
// ---------------------------------------------------------------------------

/// Serializes `value` in the committed manifest style: two-space indent, a
/// space on both sides of every object-key colon, and a trailing newline.
pub fn render_json<T: Serialize>(value: &T) -> Result<String, Box<dyn std::error::Error>> {
    let mut buffer = Vec::new();
    let formatter = SpacedPretty::default();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    value.serialize(&mut serializer)?;
    buffer.push(b'\n');
    Ok(String::from_utf8(buffer)?)
}

#[derive(Default)]
struct SpacedPretty<'a> {
    inner: serde_json::ser::PrettyFormatter<'a>,
}

impl serde_json::ser::Formatter for SpacedPretty<'_> {
    fn begin_array<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.begin_array(writer)
    }

    fn end_array<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.end_array(writer)
    }

    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.begin_array_value(writer, first)
    }

    fn end_array_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.end_array_value(writer)
    }

    fn begin_object<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.begin_object(writer)
    }

    fn end_object<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.end_object(writer)
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.begin_object_key(writer, first)
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b" : ")
    }

    fn end_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.inner.end_object_value(writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_heading_form_yields_the_same_work_item_id() {
        let cases = [
            ("### ADR046-core-001", HeadingForm::Bare),
            ("### ADR046-core-001 - Some title", HeadingForm::Dash),
            // Typographic dashes are banned in this repository's own prose, so
            // the separators this parser must still tolerate are written as
            // escapes rather than as literal characters.
            ("### ADR046-core-001 \u{2014} Some title", HeadingForm::Dash),
            ("### ADR046-core-001 \u{2013} Some title", HeadingForm::Dash),
            ("### ADR046-core-001 Some title", HeadingForm::Dash),
            ("### ADR046-core-001: Some title", HeadingForm::Colon),
            (
                "### ADR046-core-001 (Some title)",
                HeadingForm::Parenthetical,
            ),
            ("### `ADR046-core-001`", HeadingForm::Bare),
        ];
        for (line, expected) in cases {
            let (level, rest) = heading(line).expect("heading parses");
            assert_eq!(level, 3, "{line}");
            let id = leading_work_item_id(rest)
                .unwrap_or_else(|| panic!("`{line}` must declare a work item"));
            assert_eq!(id, "ADR046-core-001", "{line}");
            assert_eq!(heading_form(rest, &id), expected, "{line}");
        }
    }

    /// A hyphen introducing a title is indistinguishable, character by
    /// character, from the hyphens inside the id. Extraction is anchored on the
    /// id grammar so neither a spaced nor an unspaced hyphen can truncate a
    /// multi-segment prefix.
    #[test]
    fn a_hyphen_title_never_truncates_a_hyphenated_prefix() {
        let cases = [
            "### ADR046-security-key-012",
            "### ADR046-security-key-012 - Some title",
            "### ADR046-security-key-012 \u{2014} Some title",
            "### ADR046-security-key-012-Some title",
            "### ADR046-security-key-012: Some title",
            "### ADR046-security-key-012 (Some title)",
            "### `ADR046-security-key-012` - Some title",
        ];
        for line in cases {
            let (_, rest) = heading(line).expect("heading parses");
            let id = leading_work_item_id(rest)
                .unwrap_or_else(|| panic!("`{line}` must declare a work item"));
            assert_eq!(id, "ADR046-security-key-012", "{line}");
        }
    }

    #[test]
    fn the_expected_heading_census_totals_every_work_item() {
        let total: usize = EXPECTED_HEADING_FORMS.iter().map(|(_, n)| n).sum();
        assert_eq!(total, EXPECTED_WORK_ITEMS);
    }

    #[test]
    fn a_heading_that_only_looks_like_an_id_is_not_a_work_item() {
        for line in [
            "### ADR046-core-01",
            "### ADR046-core-0001",
            "### ADR046-core-000",
            "### Implementation work items",
            "### ADR046-Core-001",
            "### ADR-046-core-001",
            "### ADR046-001",
        ] {
            let (_, rest) = heading(line).expect("heading parses");
            assert!(
                leading_work_item_id(rest).is_none(),
                "`{line}` must not be read as a work item"
            );
        }
    }

    /// Shortest anchored match: once the grammar is satisfied the heading has
    /// declared its id, and trailing text is title whether or not a space
    /// separates it. `ADR046-core-001-002` would otherwise be read as prefix
    /// `core-001` with ordinal `002`.
    #[test]
    fn id_extraction_stops_at_the_shortest_grammatical_match() {
        for (line, expected) in [
            ("### ADR046-core-001-extra", "ADR046-core-001"),
            ("### ADR046-core-001-002", "ADR046-core-001"),
            ("### ADR046-core-001-0", "ADR046-core-001"),
        ] {
            let (_, rest) = heading(line).expect("heading parses");
            assert_eq!(
                leading_work_item_id(rest).as_deref(),
                Some(expected),
                "{line}"
            );
        }
    }

    #[test]
    fn the_reuse_action_domain_is_closed_and_sorted() {
        assert_eq!(
            REUSE_ACTIONS,
            [
                "adapt",
                "copy-unchanged",
                "create",
                "delete-after-cutover",
                "extract",
                "replace",
                "wrap",
            ]
        );
        for compound in ["extract and adapt", "refactor", "Create", ""] {
            assert!(
                !REUSE_ACTIONS.contains(&compound),
                "`{compound}` must not be an accepted reuse action"
            );
        }
    }

    #[test]
    fn table_row_keeps_unescaped_pipes_in_the_value() {
        let row = "| Detailed design | maps `Applied|Queued|MicQueueFull` into one state |";
        let (label, value) = table_row(row).expect("row parses");
        assert_eq!(label, "Detailed design");
        assert_eq!(value, "maps `Applied|Queued|MicQueueFull` into one state");
    }

    #[test]
    fn work_item_ids_require_a_three_digit_nonzero_ordinal() {
        assert!(is_work_item_id("ADR046-identities-001"));
        assert!(is_work_item_id("ADR046-security-key-029"));
        assert!(!is_work_item_id("ADR046-identities-01"));
        assert!(!is_work_item_id("ADR046-identities-0001"));
        assert!(!is_work_item_id("ADR046-identities-000"));
        assert!(!is_work_item_id("ADR-046-terminology-and-identities"));
        assert!(!is_work_item_id("ADR046--001"));
    }

    #[test]
    fn heading_ids_tolerate_both_title_separators() {
        assert_eq!(
            leading_work_item_id("ADR046-activation-001: Adapt the helper").as_deref(),
            Some("ADR046-activation-001")
        );
        assert_eq!(
            leading_work_item_id("ADR046-streamline-001 - Generated spec registry").as_deref(),
            Some("ADR046-streamline-001")
        );
        assert_eq!(leading_work_item_id("Purpose and scope"), None);
    }

    #[test]
    fn none_sentinels_become_json_null() {
        assert!(is_none_sentinel("None"));
        assert!(is_none_sentinel(
            "None. This spec is a new cross-cutting synthesis."
        ));
        assert!(!is_none_sentinel(
            "none required - this generator is specific to the manifest shape"
        ));
        assert!(!is_none_sentinel("Nonexistent owner"));
    }

    #[test]
    fn spec_refs_expand_globs_and_ignore_bare_prose() {
        let (ids, globs) = scan_spec_refs(
            "`ADR-046-decision-register`, `ADR-046-provider-state`, and all 27 \
             `docs/specs/providers/ADR-046-provider-*.md` dossiers",
        );
        assert!(ids.contains("ADR-046-decision-register"));
        assert!(ids.contains("ADR-046-provider-state"));
        assert_eq!(globs, vec!["ADR-046-provider-".to_string()]);
    }

    #[test]
    fn rendered_json_uses_the_committed_manifest_style() {
        #[derive(Serialize)]
        struct Sample {
            alpha: Vec<String>,
            beta: Vec<String>,
            gamma: Option<String>,
        }
        let rendered = render_json(&Sample {
            alpha: vec!["one".to_string()],
            beta: Vec::new(),
            gamma: None,
        })
        .expect("renders");
        assert_eq!(
            rendered,
            "{\n  \"alpha\" : [\n    \"one\"\n  ],\n  \"beta\" : [],\n  \"gamma\" : null\n}\n"
        );
    }
}
