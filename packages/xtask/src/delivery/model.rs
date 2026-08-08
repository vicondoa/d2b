//! Shared delivery identifiers, digests, and validators.
//!
//! Spec section 12.1 binds every wave candidate to three digests:
//!
//! * `content_id` - digest of the wave's integrated tree. It is deliberately
//!   free of commit history, so a history-only rebase that preserves every
//!   byte of integrated content reproduces the same `content_id` (section
//!   12.6).
//! * `candidate_id` - digest of `content_id` plus the wave's dependency graph
//!   and repository set.
//! * `snapshot_sha256` - digest covering the same inputs byte-for-byte,
//!   including the exact base and head commits the snapshot binds. It is the
//!   only one of the three that changes under a history-only rebase, which is
//!   what makes it useful for detecting one.
//!
//! A consequence worth stating outright, because it is deliberate and easy to
//! mistake for a bug: a history-only rebase that preserves every byte of
//! integrated content reproduces both `content_id` and `candidate_id`. The
//! wave therefore keeps the same candidate address, and re-snapshotting
//! rewrites `snapshot.json` in place rather than creating a second candidate
//! directory. Spec section 12.6 only permits reusing panel evidence across
//! such a rebase, so excluding commit history from `candidate_id` is the
//! behaviour that makes that clause coherent. Do not "fix" it by folding
//! base or head object IDs into either identifier.
//!
//! Every digest is produced by [`canonical_digest`], which prefixes a
//! per-purpose domain tag and a big-endian length so material from one
//! purpose can never be reinterpreted as material for another.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Component, Path},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{DELIVERY_SCHEMA_VERSION, DeliveryError, Result};

pub const SNAPSHOT_ARTIFACT_KIND: &str = "d2b-delivery/wave-snapshot";
pub const SEAL_ARTIFACT_KIND: &str = "d2b-delivery/wave-seal";
pub const HISTORY_PROOF_ARTIFACT_KIND: &str = "d2b-delivery/history-proof";
pub const PANEL_REQUEST_ARTIFACT_KIND: &str = "d2b-delivery/panel-request";
pub const PANEL_ATTESTATION_ARTIFACT_KIND: &str = "d2b-delivery/panel-receipt";
pub const EVIDENCE_ARTIFACT_KIND: &str = "d2b-delivery/validation-evidence";

/// Provider, model, and reasoning effort every panel role is bound to by spec
/// section 12.3. These live only inside the external delivery-state directory;
/// section 12.5 forbids them from reaching Git, a PR body, or a release
/// archive. The constants below are validator policy, not authorship
/// attribution, so they are committed here for the attestation check to
/// compare against.
pub const PANEL_PROVIDER_POLICY: &str = "github-copilot";
pub const PANEL_MODEL_POLICY: &str = "gpt-5.6-sol";
pub const PANEL_REASONING_EFFORT_POLICY: &str = "xhigh";
/// Historical panel records used this exact model and effort pair. Keep it
/// readable so existing delivery state remains attestable after the policy
/// moves forward; new panel requests always use the current constants above.
pub const PANEL_LEGACY_MODEL_POLICY: &str = "gemini-3.1-pro-preview";
pub const PANEL_LEGACY_REASONING_EFFORT_POLICY: &str = "high";

pub const MAX_REPOSITORIES: usize = 16;
/// Upper bound on the pull requests a single repository may bind, shared by the
/// snapshot's expected set and the merge target's declared stack so the two are
/// compared against the same ceiling.
pub const MAX_PULL_REQUESTS: usize = 64;
pub const MAX_DEPENDENCY_EDGES: usize = 512;
pub const MAX_FINGERPRINTS: usize = 512;
pub const MAX_STRING_BYTES: usize = 4 * 1024;

const CONTENT_DOMAIN: &[u8] = b"d2b-delivery-content-v1\0";
const CANDIDATE_DOMAIN: &[u8] = b"d2b-delivery-candidate-v1\0";
const SNAPSHOT_DOMAIN: &[u8] = b"d2b-delivery-snapshot-v1\0";

/// The historical ten-role panel from spec section 12.3.
///
/// This is deliberately retained as a separate compatibility roster. Current
/// panel selection uses [`PANEL_CURRENT_ROLES`] and never includes `rust`.
pub const PANEL_ROLES: [PanelRole; 10] = [
    PanelRole::Software,
    PanelRole::Test,
    PanelRole::Nixos,
    PanelRole::Networking,
    PanelRole::Security,
    PanelRole::Rust,
    PanelRole::Product,
    PanelRole::Docs,
    PanelRole::Observability,
    PanelRole::Kernel,
];

/// Current panel role domain used by the version-1 selected-roster format.
///
/// A request stores a selected subset of this ordered domain. `rust` is not a
/// current seat: Rust review is a profile on the `software` seat, while the
/// legacy roster below remains readable unchanged.
pub const PANEL_CURRENT_ROLES: [PanelRole; 13] = [
    PanelRole::Software,
    PanelRole::Test,
    PanelRole::Product,
    PanelRole::Docs,
    PanelRole::Security,
    PanelRole::Observability,
    PanelRole::Simplicity,
    PanelRole::Reliability,
    PanelRole::Agentic,
    PanelRole::Nixos,
    PanelRole::Networking,
    PanelRole::Kernel,
    PanelRole::Build,
];

pub const PANEL_SELECTION_ARTIFACT_KIND: &str = "d2b-panel/lifecycle-selection";
pub const PANEL_SELECTION_SCHEMA_VERSION: u32 = 1;
pub const PANEL_SELECTION_TABLE_VERSION: u32 = 2;

/// The panel selector's table is a repository contract, not a second Rust
/// configuration surface. Including the checked-in table makes every delivery
/// consumer validate the same mandatory seats, triggers, floors, order, and
/// profiles as the lifecycle helper.
const AUTHORITATIVE_SELECTION_TABLE: &str =
    include_str!("../../../../.github/skills/d2b-panel-round/selection-table.json");

macro_rules! digest_identifier {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Accepts an existing lowercase SHA-256 digest.
            pub fn parse(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate_sha256(&value, $label)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        // Deserialize routes through `parse` rather than the transparent
        // derive so serde cannot construct a digest from a malformed length,
        // uppercase hex, or arbitrary text: the validator that guards every
        // constructed value guards a decoded one too.
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

digest_identifier!(ContentId, "content ID");
digest_identifier!(CandidateId, "candidate ID");
digest_identifier!(SnapshotSha256, "snapshot digest");

/// The three digests a candidate snapshot is addressed by.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDigests {
    pub content_id: ContentId,
    pub candidate_id: CandidateId,
    pub snapshot_sha256: SnapshotSha256,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

impl GitObjectFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }

    pub fn hash_len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
        }
    }
}

/// One repository participating in a wave, with the commits the snapshot binds
/// and the tree the wave integrates.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRecord {
    /// Logical identity, always `github.com/owner/repository`.
    pub id: String,
    pub object_format: GitObjectFormat,
    /// Exact base commit the wave's stack sits on.
    pub base_oid: String,
    /// Exact head commit of the wave's topmost open pull request.
    pub head_oid: String,
    /// Tree the wave integrates to. History-only rebases preserve it.
    pub integration_tree_oid: String,
    /// Complete set of open pull requests the wave binds for this repository,
    /// one per slice. Merge eligibility requires the merge target to name
    /// exactly this set, so a wave of parallel same-repository pull requests
    /// cannot be declared eligible while a slice (and its required checks) is
    /// silently missing. The topmost pull request's head is
    /// [`head_oid`](Self::head_oid).
    pub expected_pull_requests: Vec<ExpectedPullRequest>,
}

impl RepositoryRecord {
    pub fn validate(&self) -> Result<()> {
        validate_repository_id(&self.id)?;
        validate_hash_for_format(&self.base_oid, self.object_format, "base commit")?;
        validate_hash_for_format(&self.head_oid, self.object_format, "head commit")?;
        validate_hash_for_format(
            &self.integration_tree_oid,
            self.object_format,
            "integration tree",
        )?;
        self.validate_expected_pull_requests()
    }

    /// The expected pull-request set must be non-empty, within the pull-request
    /// bound, free of repeated numbers or head commits, and must include the
    /// sealed topmost head. Binding the full set is what lets eligibility
    /// demand exact equality; a repository with no bound pull request could
    /// otherwise accept a merge target that omits every slice.
    fn validate_expected_pull_requests(&self) -> Result<()> {
        if self.expected_pull_requests.is_empty()
            || self.expected_pull_requests.len() > MAX_PULL_REQUESTS
        {
            return Err(DeliveryError::new(format!(
                "repository {} must bind between 1 and {MAX_PULL_REQUESTS} expected pull \
                 requests; binding none would let a merge target silently omit every slice",
                self.id
            )));
        }
        let mut numbers = BTreeSet::new();
        let mut heads = BTreeSet::new();
        for pull_request in &self.expected_pull_requests {
            if pull_request.number == 0 {
                return Err(DeliveryError::new(format!(
                    "repository {} binds an expected pull request numbered 0",
                    self.id
                )));
            }
            validate_hash_for_format(
                &pull_request.head_oid,
                self.object_format,
                "expected pull request head",
            )?;
            if !numbers.insert(pull_request.number) {
                return Err(DeliveryError::new(format!(
                    "repository {} binds pull request {} more than once",
                    self.id, pull_request.number
                )));
            }
            if !heads.insert(pull_request.head_oid.as_str()) {
                return Err(DeliveryError::new(format!(
                    "repository {} binds two expected pull requests at the same head commit {}",
                    self.id, pull_request.head_oid
                )));
            }
        }
        if !heads.contains(self.head_oid.as_str()) {
            return Err(DeliveryError::new(format!(
                "repository {} binds a topmost head commit that is not one of its expected pull \
                 request heads",
                self.id
            )));
        }
        Ok(())
    }
}

/// One open pull request the wave's stack is expected to carry for a
/// repository, bound by its number and the head commit it pointed at when the
/// snapshot was taken.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedPullRequest {
    /// Positive pull-request number; unique within a repository.
    pub number: u64,
    /// Head commit the pull request pointed at when the snapshot was taken.
    pub head_oid: String,
}

/// One edge of the wave's dependency graph from spec section 3.4.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyEdge {
    /// Node that must land first.
    pub from: String,
    /// Node that depends on `from`.
    pub to: String,
}

impl DependencyEdge {
    pub fn validate(&self) -> Result<()> {
        validate_identifier(&self.from, "dependency edge source")?;
        validate_identifier(&self.to, "dependency edge target")?;
        if self.from == self.to {
            return Err(DeliveryError::new(format!(
                "dependency edge {} depends on itself",
                self.from
            )));
        }
        Ok(())
    }
}

/// Digest of one tracked file whose content participates in the wave's
/// identity: a generated artifact, a dependency lockfile, or a contract index.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub name: String,
    pub repository: String,
    pub path: String,
    pub sha256: String,
}

impl Fingerprint {
    pub fn validate(&self) -> Result<()> {
        validate_identifier(&self.name, "fingerprint name")?;
        validate_repository_id(&self.repository)?;
        validate_repo_relative_path(Path::new(&self.path))?;
        validate_sha256(&self.sha256, "fingerprint digest")
    }

    fn sort_key(&self) -> (&str, &str, &str) {
        (
            self.name.as_str(),
            self.repository.as_str(),
            self.path.as_str(),
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelRole {
    Software,
    Test,
    Nixos,
    Networking,
    Security,
    Rust,
    Product,
    Docs,
    Observability,
    Kernel,
    Simplicity,
    Reliability,
    Agentic,
    Build,
}

impl PanelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Test => "test",
            Self::Nixos => "nixos",
            Self::Networking => "networking",
            Self::Security => "security",
            Self::Rust => "rust",
            Self::Product => "product",
            Self::Docs => "docs",
            Self::Observability => "observability",
            Self::Kernel => "kernel",
            Self::Simplicity => "simplicity",
            Self::Reliability => "reliability",
            Self::Agentic => "agentic",
            Self::Build => "build",
        }
    }

    pub(crate) fn is_current(self) -> bool {
        PANEL_CURRENT_ROLES.contains(&self)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionTable {
    artifact_kind: String,
    selection_table_version: u32,
    mandatory_seats: Vec<String>,
    optional_seats: Vec<String>,
    floors: BTreeMap<String, u32>,
    fill_order: Vec<String>,
    seats: BTreeMap<String, SelectionSeat>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionSeat {
    class: String,
    focus: String,
    triggers: Vec<SelectionTrigger>,
    profiles: BTreeMap<String, SelectionProfile>,
    #[serde(default)]
    citation_only_prose_does_not_trigger: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionProfile {
    paths: Vec<String>,
    signals: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionTrigger {
    kind: String,
    #[serde(default)]
    patterns: Option<Vec<String>>,
    #[serde(default)]
    values: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
struct SelectionInputs {
    changed_paths: Vec<String>,
    signals: Vec<String>,
    candidate_class: String,
    ambiguous: bool,
    full_candidate: Option<Box<Self>>,
    fix_delta: Option<Box<Self>>,
}

const CANDIDATE_CLASSES: [&str; 4] = ["code", "configuration", "documentation", "ambiguous"];
const CANDIDATE_CLASS_PRECEDENCE: [&str; 4] =
    ["ambiguous", "code", "configuration", "documentation"];

fn authoritative_selection_table() -> Result<SelectionTable> {
    let table: SelectionTable =
        serde_json::from_str(AUTHORITATIVE_SELECTION_TABLE).map_err(|error| {
            DeliveryError::new(format!(
                "authoritative panel selection table is invalid: {error}"
            ))
        })?;
    validate_selection_table(&table)?;
    Ok(table)
}

fn validate_selection_table(table: &SelectionTable) -> Result<()> {
    if table.artifact_kind != "d2b-panel/selection-table" {
        return Err(DeliveryError::new(
            "authoritative panel selection table has an unexpected artifact kind",
        ));
    }
    if table.selection_table_version != PANEL_SELECTION_TABLE_VERSION {
        return Err(DeliveryError::new(format!(
            "authoritative panel selection table version must be {PANEL_SELECTION_TABLE_VERSION}"
        )));
    }
    if table.mandatory_seats.is_empty() || table.optional_seats.is_empty() {
        return Err(DeliveryError::new(
            "authoritative panel selection table must define mandatory and optional seats",
        ));
    }
    if table.fill_order != table.optional_seats {
        return Err(DeliveryError::new(
            "authoritative panel selection table fill_order must exactly match optional_seats",
        ));
    }

    let mut all_seats = BTreeSet::new();
    for seat in table
        .mandatory_seats
        .iter()
        .chain(table.optional_seats.iter())
    {
        if !all_seats.insert(seat.as_str()) {
            return Err(DeliveryError::new(format!(
                "authoritative panel selection table repeats seat {seat}"
            )));
        }
        if current_role_named(seat).is_none() {
            return Err(DeliveryError::new(format!(
                "authoritative panel selection table names unsupported seat {seat}"
            )));
        }
    }
    if table.seats.len() != all_seats.len()
        || table
            .seats
            .keys()
            .any(|seat| !all_seats.contains(seat.as_str()))
    {
        return Err(DeliveryError::new(
            "authoritative panel selection table seat definitions do not match its seat domain",
        ));
    }

    for seat in &table.mandatory_seats {
        validate_selection_seat(table, seat, "mandatory")?;
    }
    for seat in &table.optional_seats {
        validate_selection_seat(table, seat, "optional")?;
    }

    if table.floors.len() != CANDIDATE_CLASSES.len()
        || CANDIDATE_CLASSES
            .iter()
            .any(|candidate_class| !table.floors.contains_key(*candidate_class))
    {
        return Err(DeliveryError::new(
            "authoritative panel selection table floors must define exactly the four candidate classes",
        ));
    }
    for candidate_class in CANDIDATE_CLASSES {
        let floor = table
            .floors
            .get(candidate_class)
            .copied()
            .expect("checked above");
        if floor < table.mandatory_seats.len() as u32 {
            return Err(DeliveryError::new(format!(
                "authoritative panel selection table floor for {candidate_class} is below its \
                 mandatory seat count"
            )));
        }
    }

    let table_order = table
        .mandatory_seats
        .iter()
        .chain(table.fill_order.iter())
        .map(|seat| current_role_named(seat).expect("seat domain was checked above"))
        .collect::<Vec<_>>();
    if table_order != PANEL_CURRENT_ROLES {
        return Err(DeliveryError::new(
            "authoritative panel selection table order does not match the current role domain",
        ));
    }
    Ok(())
}

fn validate_selection_seat(table: &SelectionTable, seat: &str, expected_class: &str) -> Result<()> {
    let definition = table.seats.get(seat).ok_or_else(|| {
        DeliveryError::new(format!(
            "authoritative panel selection table has no definition for seat {seat}"
        ))
    })?;
    if definition.class != expected_class {
        return Err(DeliveryError::new(format!(
            "authoritative panel selection table class for {seat} is not {expected_class}"
        )));
    }
    validate_bounded_string(&definition.focus, "selection-table seat focus")?;
    for trigger in &definition.triggers {
        match trigger.kind.as_str() {
            "always" => {
                if trigger.patterns.is_some() || trigger.values.is_some() {
                    return Err(DeliveryError::new(format!(
                        "authoritative panel selection table always trigger for {seat} has \
                         unexpected fields"
                    )));
                }
            }
            "path" => {
                let patterns = trigger.patterns.as_ref().ok_or_else(|| {
                    DeliveryError::new(format!(
                        "authoritative panel selection table path trigger for {seat} has no patterns"
                    ))
                })?;
                if patterns.is_empty() || trigger.values.is_some() {
                    return Err(DeliveryError::new(format!(
                        "authoritative panel selection table path trigger for {seat} is malformed"
                    )));
                }
                for pattern in patterns {
                    validate_bounded_string(pattern, "selection-table path pattern")?;
                }
            }
            "signal" => {
                let values = trigger.values.as_ref().ok_or_else(|| {
                    DeliveryError::new(format!(
                        "authoritative panel selection table signal trigger for {seat} has no values"
                    ))
                })?;
                if values.is_empty() || trigger.patterns.is_some() {
                    return Err(DeliveryError::new(format!(
                        "authoritative panel selection table signal trigger for {seat} is malformed"
                    )));
                }
                for value in values {
                    validate_bounded_string(value, "selection-table signal trigger")?;
                }
            }
            other => {
                return Err(DeliveryError::new(format!(
                    "authoritative panel selection table has unknown trigger kind {other:?}"
                )));
            }
        }
    }
    for (profile, definition) in &definition.profiles {
        validate_bounded_string(profile, "selection-table profile name")?;
        for path in &definition.paths {
            validate_bounded_string(path, "selection-table profile path")?;
        }
        for signal in &definition.signals {
            validate_bounded_string(signal, "selection-table profile signal")?;
        }
    }
    Ok(())
}

fn current_role_named(name: &str) -> Option<PanelRole> {
    PANEL_CURRENT_ROLES
        .iter()
        .copied()
        .find(|role| role.as_str() == name)
}

fn table_roles(names: &[String], label: &str) -> Result<Vec<PanelRole>> {
    names
        .iter()
        .map(|name| {
            current_role_named(name).ok_or_else(|| {
                DeliveryError::new(format!(
                    "authoritative panel selection table {label} names unsupported seat {name}"
                ))
            })
        })
        .collect()
}

/// Mirrors POSIX `path.normalize` behavior for the JavaScript lifecycle
/// helper. Backslash is a literal filename character; only `/` separates
/// path segments.
fn normalize_classification_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let trailing_separator = path.ends_with('/');
    let mut components = Vec::new();

    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components
                    .last()
                    .is_some_and(|component| *component != "..")
                {
                    components.pop();
                } else if !absolute {
                    components.push(component);
                }
            }
            component => components.push(component),
        }
    }

    let mut normalized = if absolute {
        "/".to_owned()
    } else {
        String::new()
    };
    normalized.push_str(&components.join("/"));
    if normalized.is_empty() {
        normalized.push('.');
    }
    if trailing_separator && normalized != "/" {
        normalized.push('/');
    }
    normalized
}

fn contains_c0_c1_control(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(
            character,
            '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}'
        )
    })
}

fn is_documentation_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    if path.starts_with("docs/") || path.starts_with("changelog.d/") {
        return true;
    }
    if path.contains('/') {
        return false;
    }
    if path == "readme"
        || path.starts_with("readme.")
        || path == "changelog"
        || path.starts_with("changelog.")
    {
        return true;
    }
    [".md", ".mdx", ".rst", ".txt"]
        .iter()
        .any(|suffix| path.ends_with(suffix) && path.len() > suffix.len())
}

fn candidate_class_precedence(classes: &[&str]) -> &'static str {
    CANDIDATE_CLASS_PRECEDENCE
        .iter()
        .copied()
        .find(|candidate_class| classes.contains(candidate_class))
        .expect("nested classification exists")
}

fn parse_classification_inputs(
    value: &Value,
    label: &str,
    allow_nested: bool,
    allow_empty_fix_delta_paths: bool,
) -> Result<SelectionInputs> {
    let object = value
        .as_object()
        .ok_or_else(|| DeliveryError::new(format!("{label} must be an object")))?;
    for key in object.keys() {
        let known = matches!(
            key.as_str(),
            "changed_paths" | "signals" | "candidate_class" | "ambiguous"
        ) || (allow_nested && matches!(key.as_str(), "full_candidate" | "fix_delta"));
        if !known {
            return Err(DeliveryError::new(format!(
                "{label} contains unknown field {key:?}"
            )));
        }
    }
    for key in ["changed_paths", "signals", "candidate_class", "ambiguous"] {
        if !object.contains_key(key) {
            return Err(DeliveryError::new(format!("{label} must contain {key}")));
        }
    }

    let raw_changed_paths = object
        .get("changed_paths")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new(format!("{label} changed_paths must be an array")))?
        .iter()
        .map(|value| {
            let path = value.as_str().ok_or_else(|| {
                DeliveryError::new(format!("{label} changed_paths entries must be strings"))
            })?;
            validate_bounded_string(path, &format!("{label} changed path"))?;
            if contains_c0_c1_control(path) {
                return Err(DeliveryError::new(format!(
                    "{label} changed paths must not contain control characters"
                )));
            }
            Ok(path.to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    let canonical_changed_paths = raw_changed_paths
        .iter()
        .map(|path| {
            let canonical = normalize_classification_path(path);
            if canonical.as_str() != path
                || canonical == "."
                || canonical.starts_with('/')
                || canonical.ends_with('/')
            {
                return Err(DeliveryError::new(format!(
                    "{label} changed_paths must contain canonical normalized paths"
                )));
            }
            Ok(canonical)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut sorted_changed_paths = canonical_changed_paths.clone();
    sorted_changed_paths.sort();
    sorted_changed_paths.dedup();
    if canonical_changed_paths != sorted_changed_paths {
        return Err(DeliveryError::new(format!(
            "{label} changed_paths must be unique and sorted"
        )));
    }

    let raw_signals = object
        .get("signals")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new(format!("{label} signals must be an array")))?
        .iter()
        .map(|value| {
            let signal = value.as_str().ok_or_else(|| {
                DeliveryError::new(format!("{label} signals entries must be strings"))
            })?;
            validate_bounded_string(signal, &format!("{label} signal"))?;
            if contains_c0_c1_control(signal) {
                return Err(DeliveryError::new(format!(
                    "{label} signals must not contain control characters"
                )));
            }
            Ok(signal.to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    let canonical_signals = raw_signals
        .iter()
        .map(|signal| signal.trim().to_lowercase())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if raw_signals != canonical_signals {
        return Err(DeliveryError::new(format!(
            "{label} signals must be unique, lowercase, and sorted"
        )));
    }

    let candidate_class = object
        .get("candidate_class")
        .and_then(Value::as_str)
        .ok_or_else(|| DeliveryError::new(format!("{label} candidate_class must be a string")))?;
    validate_bounded_string(candidate_class, &format!("{label} candidate class"))?;
    if !CANDIDATE_CLASSES.contains(&candidate_class) {
        return Err(DeliveryError::new(format!(
            "{label} candidate_class {candidate_class:?} is unsupported"
        )));
    }
    if candidate_class == "documentation"
        && canonical_changed_paths
            .iter()
            .any(|path| !is_documentation_path(path))
    {
        return Err(DeliveryError::new(format!(
            "{label} candidate_class documentation cannot narrow actual non-documentation paths"
        )));
    }
    let ambiguous = object
        .get("ambiguous")
        .and_then(Value::as_bool)
        .ok_or_else(|| DeliveryError::new(format!("{label} ambiguous must be boolean")))?;
    // A verification selection with no supplied delta carries an empty
    // fix_delta sentinel using the full candidate class with a non-widening
    // ambiguity bit, so that sentinel is the one permitted exception.
    if ambiguous != (candidate_class == "ambiguous")
        && !(allow_empty_fix_delta_paths
            && raw_changed_paths.is_empty()
            && raw_signals.is_empty()
            && candidate_class == "ambiguous"
            && !ambiguous)
    {
        return Err(DeliveryError::new(format!(
            "{label} candidate_class and ambiguous disagree"
        )));
    }
    if matches!(candidate_class, "code" | "configuration")
        && raw_changed_paths.is_empty()
        && (!allow_empty_fix_delta_paths || !raw_signals.is_empty())
    {
        return Err(DeliveryError::new(format!(
            "{label} {candidate_class} classification must contain changed paths"
        )));
    }

    let full_candidate = object
        .get("full_candidate")
        .map(|value| {
            parse_classification_inputs(value, &format!("{label}.full_candidate"), false, false)
                .map(Box::new)
        })
        .transpose()?;
    let fix_delta = object
        .get("fix_delta")
        .map(|value| {
            parse_classification_inputs(value, &format!("{label}.fix_delta"), false, true)
                .map(Box::new)
        })
        .transpose()?;

    Ok(SelectionInputs {
        changed_paths: canonical_changed_paths,
        signals: canonical_signals,
        candidate_class: candidate_class.to_owned(),
        ambiguous,
        full_candidate,
        fix_delta,
    })
}

fn selection_inputs(value: &Value, phase: &str) -> Result<SelectionInputs> {
    parse_classification_inputs(
        value,
        "panel selection classification_inputs",
        phase == "verification",
        false,
    )
}

fn validate_nested_classification_consistency(inputs: &SelectionInputs) -> Result<()> {
    if inputs.full_candidate.is_none() && inputs.fix_delta.is_none() {
        return Ok(());
    }

    let mut changed_paths = BTreeSet::new();
    let mut signals = BTreeSet::new();
    let mut nested_classes = Vec::new();
    let mut ambiguous = false;
    for nested in [
        inputs.full_candidate.as_deref(),
        inputs.fix_delta.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        changed_paths.extend(nested.changed_paths.iter().cloned());
        signals.extend(nested.signals.iter().cloned());
        nested_classes.push(nested.candidate_class.as_str());
        ambiguous |= nested.ambiguous;
    }
    let changed_paths = changed_paths.into_iter().collect::<Vec<_>>();
    if inputs.changed_paths != changed_paths {
        return Err(DeliveryError::new(
            "panel selection classification_inputs changed_paths must equal the union of its \
             nested full_candidate and fix_delta paths",
        ));
    }
    let signals = signals.into_iter().collect::<Vec<_>>();
    if inputs.signals != signals {
        return Err(DeliveryError::new(
            "panel selection classification_inputs signals must equal the union of its nested \
             full_candidate and fix_delta signals",
        ));
    }
    if inputs.ambiguous != ambiguous {
        return Err(DeliveryError::new(
            "panel selection classification_inputs ambiguous must equal nested classifications",
        ));
    }
    let expected_class = candidate_class_precedence(&nested_classes);
    if inputs.candidate_class != expected_class {
        return Err(DeliveryError::new(
            "panel selection classification_inputs candidate_class must agree with nested \
             classifications",
        ));
    }
    Ok(())
}

fn trigger_matches(trigger: &SelectionTrigger, inputs: &SelectionInputs) -> bool {
    match trigger.kind.as_str() {
        "always" => true,
        "path" => trigger.patterns.as_ref().is_some_and(|patterns| {
            inputs
                .changed_paths
                .iter()
                .any(|path| patterns.iter().any(|pattern| glob_matches(path, pattern)))
        }),
        "signal" => trigger.values.as_ref().is_some_and(|values| {
            values.iter().any(|value| {
                let value = value.to_lowercase();
                inputs.signals.iter().any(|signal| signal == &value)
            })
        }),
        _ => false,
    }
}

fn glob_matches(path: &str, pattern: &str) -> bool {
    fn visit(
        path: &[u8],
        pattern: &[u8],
        path_index: usize,
        pattern_index: usize,
        memo: &mut BTreeMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(path_index, pattern_index)) {
            return *result;
        }
        let result = if pattern_index == pattern.len() {
            path_index == path.len()
        } else if pattern[pattern_index] == b'*' {
            if pattern.get(pattern_index + 1) == Some(&b'*') {
                let next = pattern_index + 2;
                if pattern.get(next) == Some(&b'/') {
                    let next_segment = path[path_index..]
                        .iter()
                        .position(|byte| *byte == b'/')
                        .map(|offset| path_index + offset + 1);
                    visit(path, pattern, path_index, next + 1, memo)
                        || next_segment.is_some_and(|path_index| {
                            visit(path, pattern, path_index, pattern_index, memo)
                        })
                } else {
                    visit(path, pattern, path_index, next, memo)
                        || (path_index < path.len()
                            && visit(path, pattern, path_index + 1, pattern_index, memo))
                }
            } else {
                visit(path, pattern, path_index, pattern_index + 1, memo)
                    || (path_index < path.len()
                        && path[path_index] != b'/'
                        && visit(path, pattern, path_index + 1, pattern_index, memo))
            }
        } else if pattern[pattern_index] == b'?' {
            path_index < path.len()
                && path[path_index] != b'/'
                && visit(path, pattern, path_index + 1, pattern_index + 1, memo)
        } else {
            path_index < path.len()
                && path[path_index].eq_ignore_ascii_case(&pattern[pattern_index])
                && visit(path, pattern, path_index + 1, pattern_index + 1, memo)
        };
        memo.insert((path_index, pattern_index), result);
        result
    }

    visit(
        path.as_bytes(),
        pattern.as_bytes(),
        0,
        0,
        &mut BTreeMap::new(),
    )
}

/// The candidate-bound selection artifact shared by the lifecycle helper and
/// the delivery request writer.
///
/// The nested classification input is intentionally retained as JSON because
/// the lifecycle helper adds full-candidate and fix-delta details over time.
/// Its required top-level shape is checked by [`Self::validate_for_snapshot`], while the
/// top-level DTO remains closed so a misspelled selection field cannot pass.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSelectionV1 {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub lifecycle_id: String,
    pub phase: String,
    pub program: String,
    pub wave: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub selection_table_version: u32,
    pub candidate_class: String,
    pub classification_inputs: Value,
    pub ambiguity_widened: bool,
    pub profiles: BTreeMap<String, Vec<String>>,
    pub roster: Vec<PanelRole>,
}

impl PanelSelectionV1 {
    /// Validates a selection against the immutable snapshot it is intended to
    /// request. The returned roster is already the only roster a current
    /// request may store.
    pub fn validate_for_snapshot(
        &self,
        program: &str,
        wave: &str,
        digests: &CandidateDigests,
    ) -> Result<()> {
        if self.artifact_kind != PANEL_SELECTION_ARTIFACT_KIND {
            return Err(DeliveryError::new(
                "panel selection artifact kind is not d2b-panel/lifecycle-selection",
            ));
        }
        if self.schema_version != PANEL_SELECTION_SCHEMA_VERSION {
            return Err(DeliveryError::new(format!(
                "panel selection schema version must be {PANEL_SELECTION_SCHEMA_VERSION}"
            )));
        }
        validate_bounded_string(&self.lifecycle_id, "panel selection lifecycle identifier")?;
        if self.lifecycle_id == "."
            || self.lifecycle_id == ".."
            || self.lifecycle_id.contains('/')
            || self.lifecycle_id.contains('\\')
            || self.lifecycle_id.chars().any(char::is_control)
        {
            return Err(DeliveryError::new(
                "panel selection lifecycle identifier must be one safe path component",
            ));
        }
        if self.phase != "discovery" && self.phase != "verification" {
            return Err(DeliveryError::new(
                "panel selection phase must be discovery or verification",
            ));
        }
        validate_program_wave(&self.program, &self.wave)?;
        if self.program != program || self.wave != wave {
            return Err(DeliveryError::new(
                "panel selection program and wave must match the candidate snapshot",
            ));
        }
        if self.candidate_id != digests.candidate_id
            || self.content_id != digests.content_id
            || self.snapshot_sha256 != digests.snapshot_sha256
        {
            return Err(DeliveryError::new(
                "panel selection candidate digests must exactly match the candidate snapshot",
            ));
        }
        let table = authoritative_selection_table()?;
        if self.selection_table_version != table.selection_table_version {
            return Err(DeliveryError::new(format!(
                "panel selection table version must be {}",
                table.selection_table_version
            )));
        }
        if !table.floors.contains_key(&self.candidate_class) {
            return Err(DeliveryError::new(
                "panel selection candidate class is not supported",
            ));
        }
        let inputs = selection_inputs(&self.classification_inputs, &self.phase)?;
        if self.phase == "verification"
            && (inputs.full_candidate.is_none() || inputs.fix_delta.is_none())
        {
            return Err(DeliveryError::new(
                "verification selection classification_inputs must contain both \
                 full_candidate and fix_delta",
            ));
        }
        if self.candidate_class != inputs.candidate_class {
            return Err(DeliveryError::new(
                "panel selection candidate_class disagrees with classification_inputs",
            ));
        }
        if self.ambiguity_widened != inputs.ambiguous {
            return Err(DeliveryError::new(
                "panel selection ambiguity_widened disagrees with classification_inputs",
            ));
        }
        validate_nested_classification_consistency(&inputs)?;
        let table_order = table
            .mandatory_seats
            .iter()
            .chain(table.fill_order.iter())
            .map(|seat| current_role_named(seat).expect("validated selection table seat"))
            .collect::<Vec<_>>();
        let mandatory = table_roles(&table.mandatory_seats, "mandatory_seats")?;
        let triggered_optional = table
            .optional_seats
            .iter()
            .filter_map(|seat| {
                let definition = table
                    .seats
                    .get(seat)
                    .expect("validated selection table seat definition");
                trigger_matches_for_seat(definition, &inputs)
                    .then(|| current_role_named(seat).expect("validated selection table seat"))
            })
            .collect::<Vec<_>>();

        if self.roster.is_empty() {
            return Err(DeliveryError::new(
                "panel selection roster must contain at least one current seat",
            ));
        }
        let mut seen = BTreeSet::new();
        for role in &self.roster {
            if !role.is_current() {
                return Err(DeliveryError::new(format!(
                    "panel selection roster contains legacy or unknown seat {}",
                    role.as_str()
                )));
            }
            if !seen.insert(*role) {
                return Err(DeliveryError::new(format!(
                    "panel selection roster repeats seat {}",
                    role.as_str()
                )));
            }
        }
        for role in mandatory {
            if !seen.contains(&role) {
                return Err(DeliveryError::new(format!(
                    "panel selection roster omits mandatory seat {}",
                    role.as_str()
                )));
            }
        }
        for role in triggered_optional {
            if !seen.contains(&role) {
                return Err(DeliveryError::new(format!(
                    "panel selection roster omits triggered optional seat {}",
                    role.as_str()
                )));
            }
        }
        let canonical = table_order
            .iter()
            .copied()
            .filter(|role| seen.contains(role))
            .collect::<Vec<_>>();
        if self.roster != canonical {
            return Err(DeliveryError::new(
                "panel selection roster is not in selection-table order",
            ));
        }
        let floor = table
            .floors
            .get(&self.candidate_class)
            .copied()
            .expect("candidate class was checked against the authoritative table")
            as usize;
        if self.roster.len() < floor {
            return Err(DeliveryError::new(format!(
                "panel selection roster has {} seats but this candidate class requires at least {floor}",
                self.roster.len()
            )));
        }

        if self.profiles.len() != self.roster.len() {
            return Err(DeliveryError::new(
                "panel selection profiles must have exactly one entry per selected seat",
            ));
        }
        for role in &self.roster {
            let profiles = self.profiles.get(role.as_str()).ok_or_else(|| {
                DeliveryError::new(format!(
                    "panel selection profiles are missing selected seat {}",
                    role.as_str()
                ))
            })?;
            let definition = table
                .seats
                .get(role.as_str())
                .expect("validated selection table seat definition");
            let mut profile_names = BTreeSet::new();
            for profile in profiles {
                validate_bounded_string(profile, "panel selection profile")?;
                if !profile_names.insert(profile) {
                    return Err(DeliveryError::new(format!(
                        "panel selection repeats profile for seat {}",
                        role.as_str()
                    )));
                }
                if !definition.profiles.contains_key(profile) {
                    return Err(DeliveryError::new(format!(
                        "panel selection profile {}/{} is not defined by the selection table",
                        role.as_str(),
                        profile
                    )));
                }
            }
            for (profile, profile_definition) in &definition.profiles {
                let required = profile_definition.paths.iter().any(|pattern| {
                    inputs
                        .changed_paths
                        .iter()
                        .any(|path| glob_matches(path, pattern))
                }) || profile_definition.signals.iter().any(|signal| {
                    let signal = signal.to_lowercase();
                    inputs.signals.iter().any(|input| input == &signal)
                });
                if required && !profile_names.contains(profile) {
                    return Err(DeliveryError::new(format!(
                        "panel selection profile {}/{} is missing for its classification inputs",
                        role.as_str(),
                        profile
                    )));
                }
            }
        }
        if self
            .profiles
            .keys()
            .any(|role| !self.roster.iter().any(|selected| selected.as_str() == role))
        {
            return Err(DeliveryError::new(
                "panel selection profiles contain an unselected seat",
            ));
        }
        Ok(())
    }
}

fn trigger_matches_for_seat(definition: &SelectionSeat, inputs: &SelectionInputs) -> bool {
    definition
        .triggers
        .iter()
        .any(|trigger| trigger_matches(trigger, inputs))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceResult {
    Passed,
    Failed,
}

/// Everything a wave candidate's identity is derived from.
///
/// Construct it, call [`CandidateMaterial::digests`], and address every
/// downstream artifact by the resulting [`CandidateId`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateMaterial {
    /// Delivery program the wave belongs to; always `ADR046`.
    pub program: String,
    /// Wave identifier from spec section 3.2, one of `W0`..`W8`.
    pub wave: String,
    pub repository_set: Vec<RepositoryRecord>,
    pub dependency_graph: Vec<DependencyEdge>,
    /// Digests of generated artifacts (schemas, spec-set manifests, rendered
    /// Nix fixtures) that spec section 12.6 treats as wave content.
    pub generated_artifacts: Vec<Fingerprint>,
    /// Digests of dependency metadata, for example lockfiles.
    pub dependency_fingerprints: Vec<Fingerprint>,
    /// Digests of contract or index content.
    pub contract_fingerprints: Vec<Fingerprint>,
}

impl CandidateMaterial {
    /// Sorts every list into canonical order and rejects duplicates, so two
    /// callers that supply the same set in a different order derive the same
    /// digests.
    pub fn canonicalize(&mut self) -> Result<()> {
        ensure_count(
            self.repository_set.len(),
            1,
            MAX_REPOSITORIES,
            "repositories",
        )?;
        ensure_count(
            self.dependency_graph.len(),
            0,
            MAX_DEPENDENCY_EDGES,
            "dependency edges",
        )?;
        validate_program_wave(&self.program, &self.wave)?;

        for repository in &mut self.repository_set {
            repository.expected_pull_requests.sort();
        }
        self.repository_set.sort();
        self.dependency_graph.sort();
        for list in [
            &mut self.generated_artifacts,
            &mut self.dependency_fingerprints,
            &mut self.contract_fingerprints,
        ] {
            ensure_count(list.len(), 0, MAX_FINGERPRINTS, "fingerprints")?;
            list.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        }

        let mut repositories = BTreeSet::new();
        for repository in &self.repository_set {
            repository.validate()?;
            if !repositories.insert(repository.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "repository set repeats {}",
                    repository.id
                )));
            }
        }

        let mut edges = BTreeSet::new();
        for edge in &self.dependency_graph {
            edge.validate()?;
            if !edges.insert((edge.from.as_str(), edge.to.as_str())) {
                return Err(DeliveryError::new(format!(
                    "dependency graph repeats edge {} -> {}",
                    edge.from, edge.to
                )));
            }
        }

        for (label, list) in [
            ("generated artifact", &self.generated_artifacts),
            ("dependency fingerprint", &self.dependency_fingerprints),
            ("contract fingerprint", &self.contract_fingerprints),
        ] {
            let mut names = BTreeSet::new();
            for fingerprint in list {
                fingerprint.validate()?;
                if !repositories.contains(fingerprint.repository.as_str()) {
                    return Err(DeliveryError::new(format!(
                        "{label} {} names repository {} outside the repository set",
                        fingerprint.name, fingerprint.repository
                    )));
                }
                if !names.insert(fingerprint.name.as_str()) {
                    return Err(DeliveryError::new(format!(
                        "{label} list repeats {}",
                        fingerprint.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Derives all three candidate digests. Canonicalizes and validates first,
    /// so the result is independent of the caller's input ordering.
    pub fn digests(&self) -> Result<CandidateDigests> {
        let mut canonical = self.clone();
        canonical.canonicalize()?;
        let content_id = canonical.content_id()?;
        let candidate_id = canonical.candidate_id(&content_id)?;
        let snapshot_sha256 = canonical.snapshot_sha256(&content_id, &candidate_id)?;
        Ok(CandidateDigests {
            content_id,
            candidate_id,
            snapshot_sha256,
        })
    }

    /// Digest of the wave's integrated tree. Commit history is excluded on
    /// purpose: spec section 12.6 requires a history-only rebase to reproduce
    /// the same value.
    fn content_id(&self) -> Result<ContentId> {
        #[derive(Serialize)]
        struct ContentRepository<'a> {
            id: &'a str,
            object_format: GitObjectFormat,
            integration_tree_oid: &'a str,
        }
        #[derive(Serialize)]
        struct ContentMaterial<'a> {
            schema_version: u32,
            program: &'a str,
            wave: &'a str,
            repositories: Vec<ContentRepository<'a>>,
            generated_artifacts: &'a [Fingerprint],
            dependency_fingerprints: &'a [Fingerprint],
            contract_fingerprints: &'a [Fingerprint],
        }
        let repositories = self
            .repository_set
            .iter()
            .map(|repository| ContentRepository {
                id: &repository.id,
                object_format: repository.object_format,
                integration_tree_oid: &repository.integration_tree_oid,
            })
            .collect();
        canonical_digest(
            CONTENT_DOMAIN,
            &ContentMaterial {
                schema_version: DELIVERY_SCHEMA_VERSION,
                program: &self.program,
                wave: &self.wave,
                repositories,
                generated_artifacts: &self.generated_artifacts,
                dependency_fingerprints: &self.dependency_fingerprints,
                contract_fingerprints: &self.contract_fingerprints,
            },
        )
        .map(ContentId)
    }

    /// Digest of `content_id` plus the dependency graph and repository set.
    fn candidate_id(&self, content_id: &ContentId) -> Result<CandidateId> {
        #[derive(Serialize)]
        struct RepositoryMembership<'a> {
            id: &'a str,
            object_format: GitObjectFormat,
        }
        #[derive(Serialize)]
        struct CandidateMaterialDigest<'a> {
            schema_version: u32,
            program: &'a str,
            wave: &'a str,
            content_id: &'a ContentId,
            dependency_graph: &'a [DependencyEdge],
            repository_set: Vec<RepositoryMembership<'a>>,
        }
        let repository_set = self
            .repository_set
            .iter()
            .map(|repository| RepositoryMembership {
                id: &repository.id,
                object_format: repository.object_format,
            })
            .collect();
        canonical_digest(
            CANDIDATE_DOMAIN,
            &CandidateMaterialDigest {
                schema_version: DELIVERY_SCHEMA_VERSION,
                program: &self.program,
                wave: &self.wave,
                content_id,
                dependency_graph: &self.dependency_graph,
                repository_set,
            },
        )
        .map(CandidateId)
    }

    /// Digest covering the same inputs byte-for-byte, including the exact base
    /// and head commits and both derived identifiers.
    fn snapshot_sha256(
        &self,
        content_id: &ContentId,
        candidate_id: &CandidateId,
    ) -> Result<SnapshotSha256> {
        #[derive(Serialize)]
        struct SnapshotMaterial<'a> {
            schema_version: u32,
            content_id: &'a ContentId,
            candidate_id: &'a CandidateId,
            material: &'a CandidateMaterial,
        }
        canonical_digest(
            SNAPSHOT_DOMAIN,
            &SnapshotMaterial {
                schema_version: DELIVERY_SCHEMA_VERSION,
                content_id,
                candidate_id,
                material: self,
            },
        )
        .map(SnapshotSha256)
    }
}

/// Domain-separated SHA-256 over the canonical JSON encoding of `value`.
///
/// The domain tag and the big-endian payload length are hashed before the
/// payload, so material serialized for one purpose cannot collide with
/// material serialized for another.
pub fn canonical_digest(domain: &[u8], value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    Ok(render_digest(hasher.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    render_digest(hasher.finalize())
}

fn render_digest(digest: impl IntoIterator<Item = u8>) -> String {
    use std::fmt::Write as _;
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        write!(&mut rendered, "{byte:02x}").expect("writing to a String cannot fail");
    }
    rendered
}

pub fn validate_repo_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DeliveryError::new(format!(
            "path must be repository-relative without traversal: {}",
            path.display()
        )));
    }
    let rendered = path
        .to_str()
        .ok_or_else(|| DeliveryError::new("repository-relative path is not UTF-8"))?;
    validate_bounded_string(rendered, "repository-relative path")
}

pub fn validate_hash(value: &str, label: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64) || !is_lower_hex(value) {
        return Err(DeliveryError::new(format!(
            "{label} must be a full lowercase Git object hash"
        )));
    }
    Ok(())
}

pub fn validate_hash_for_format(value: &str, format: GitObjectFormat, label: &str) -> Result<()> {
    if value.len() != format.hash_len() || !is_lower_hex(value) {
        return Err(DeliveryError::new(format!(
            "{label} does not match Git object format {}",
            format.as_str()
        )));
    }
    Ok(())
}

pub fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !is_lower_hex(value) {
        return Err(DeliveryError::new(format!(
            "{label} must be a lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(DeliveryError::new(format!(
            "{label} must use lowercase ASCII letters, digits, '.', '_' or '-'"
        )));
    }
    Ok(())
}

/// The delivery program the legacy wave namespace belongs to: ADR 0046. Spec
/// section 3.2 (`docs/specs/ADR-046-validation-and-delivery.md`) defines a
/// closed wave namespace `ADR046-W0`..`ADR046-W8`, split there into the fixed
/// program component and the closed wave component.
pub const ADR046_PROGRAM: &str = "ADR046";

/// The closed set of legacy ADR 0046 wave identifiers, `W0` through `W8`.
///
/// A bare wave in this set always means program [`ADR046_PROGRAM`] and always
/// resolves to its existing `<state-root>/W<N>/...` directory. This form is not
/// deprecated and is not on a timer: ADR 0046 runs to completion in it, because
/// re-addressing a wave would invalidate the candidate digests binding its
/// existing snapshots, seals, and panel records.
pub const ADR046_WAVES: [&str; 9] = ["W0", "W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];

/// Highest wave ordinal any program may use, matching the legacy closed set.
pub const MAX_WAVE_ORDINAL: u8 = 8;

/// Bounds on the program component of a qualified wave token.
const MIN_QUALIFIED_PROGRAM_LEN: usize = 3;
const MAX_QUALIFIED_PROGRAM_LEN: usize = 16;

/// Splits a qualified wave token into its lowercase program component and its
/// wave ordinal, or returns `None` if the token is not a qualified wave.
///
/// The canonical form fuses the program and the wave into a single lowercase
/// token with no separator: `adr046w1`, `spec001w1`. Fusing them rather than
/// adding a `<program>/<wave>` path component is deliberate. The delivery state
/// layout is `<state-root>/<wave>/<candidate-id>/...`, in which the program is
/// **not** a path component, so with two programs in flight a bare `W1` from
/// each would name the same state directory. A fused token makes uniqueness
/// intrinsic to the identifier, so it survives being copied into an artifact
/// reference, a commit subject, a panel record, or a checkpoint, none of which
/// have a path structure to lean on. It also requires no state-layout change,
/// which is what keeps the in-flight program safe.
///
/// The accepted shape is `[a-z][a-z0-9]*` followed by `w` and a single ordinal
/// digit `0`..=[`MAX_WAVE_ORDINAL`], with the program component bounded in
/// length. The split is taken at the **final** `w`, so a program whose own name
/// contains `w` or ends in a digit is still parsed correctly.
pub fn qualified_wave_parts(wave: &str) -> Option<(&str, u8)> {
    // Matched on bytes rather than by byte-offset slicing, because a
    // byte-offset split of an arbitrary operator string can land inside a
    // multi-byte character and panic. A slice pattern cannot.
    let [program @ .., b'w', digit @ b'0'..=b'9'] = wave.as_bytes() else {
        return None;
    };
    let ordinal = digit - b'0';
    if ordinal > MAX_WAVE_ORDINAL {
        return None;
    }
    if !(MIN_QUALIFIED_PROGRAM_LEN..=MAX_QUALIFIED_PROGRAM_LEN).contains(&program.len()) {
        return None;
    }
    let [first, rest @ ..] = program else {
        return None;
    };
    if !first.is_ascii_lowercase() {
        return None;
    }
    if !rest
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return None;
    }
    // Every accepted byte is ASCII, so the prefix is a character boundary.
    Some((&wave[..program.len()], ordinal))
}

/// Rejects any wave outside the legacy closed set and the qualified namespace.
///
/// The wave becomes a path component (`<state-root>/<wave>/<candidate>/...`)
/// and is echoed verbatim in every stage's `artifact` reference, so a
/// free-form identifier would let a username or other operator string leak into
/// structured output. Two forms are accepted, and neither admits one:
///
/// * a member of the legacy [`ADR046_WAVES`] closed set, which continues to
///   mean program [`ADR046_PROGRAM`]; or
/// * a qualified token accepted by [`qualified_wave_parts`], which is a bounded
///   lowercase ASCII alphanumeric string ending in `w` and one ordinal digit.
///
/// The legacy form keeps its closed-set guarantee exactly. The qualified form
/// is a strict bounded pattern rather than a nine-element set, which is the one
/// property that genuinely widens here: it still cannot express a path
/// separator, a relative traversal, an absolute path, a control character,
/// whitespace, uppercase, or an unbounded length, so the state path component
/// remains a short lowercase alphanumeric token in every accepted case.
pub fn validate_wave(wave: &str) -> Result<()> {
    if ADR046_WAVES.contains(&wave) || qualified_wave_parts(wave).is_some() {
        return Ok(());
    }
    Err(DeliveryError::new(format!(
        "delivery wave must be one of {} - the closed ADR 0046 wave namespace - \
         or a qualified lowercase token such as `spec001w1`",
        ADR046_WAVES.join(", ")
    )))
}

/// Rejects any program/wave pair the delivery namespace does not admit.
///
/// This is the single gate every stage runs before it creates delivery state or
/// emits an artifact reference, so no name-like value can reach a state
/// directory name or structured stdout. Three rules, in order:
///
/// * A legacy bare wave requires the program to be exactly [`ADR046_PROGRAM`].
///   This is the pre-existing behaviour, unchanged.
/// * A qualified wave requires its embedded program to equal the `--program`
///   argument case-insensitively, so `--program SPEC001 --wave adr046w1` is
///   rejected as the inconsistency it is rather than silently trusting one side.
/// * Any other wave is rejected by [`validate_wave`].
pub fn validate_program_wave(program: &str, wave: &str) -> Result<()> {
    if let Some((qualified_program, _)) = qualified_wave_parts(wave) {
        if !program.eq_ignore_ascii_case(qualified_program) {
            return Err(DeliveryError::new(format!(
                "delivery wave `{wave}` names program `{qualified_program}`, \
                 which disagrees with the requested program `{program}`"
            )));
        }
        return Ok(());
    }
    if program != ADR046_PROGRAM {
        return Err(DeliveryError::new(format!(
            "delivery program must be {ADR046_PROGRAM} for a bare wave - \
             a new program must use a qualified wave such as `spec001w1`"
        )));
    }
    validate_wave(wave)
}

pub fn validate_repository_id(id: &str) -> Result<()> {
    validate_bounded_string(id, "repository identity")?;
    let parts = id.split('/').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "github.com"
        || parts[1].is_empty()
        || parts[2].is_empty()
        || !parts[1]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !parts[2]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(DeliveryError::new(format!(
            "logical repository identity must be github.com/owner/repository: {id:?}"
        )));
    }
    Ok(())
}

pub fn validate_git_ref(value: &str, label: &str) -> Result<()> {
    validate_bounded_string(value, label)?;
    if value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("..")
        || value.contains("@{")
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(DeliveryError::new(format!("invalid {label}")));
    }
    Ok(())
}

pub fn validate_bounded_string(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > MAX_STRING_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} must be non-empty and at most {MAX_STRING_BYTES} bytes"
        )));
    }
    Ok(())
}

pub fn ensure_schema(version: u32, label: &str) -> Result<()> {
    if version != DELIVERY_SCHEMA_VERSION {
        return Err(DeliveryError::new(format!(
            "unsupported {label} schema version {version}"
        )));
    }
    Ok(())
}

fn ensure_count(count: usize, minimum: usize, maximum: usize, label: &str) -> Result<()> {
    if count < minimum || count > maximum {
        return Err(DeliveryError::new(format!(
            "{label} count must be between {minimum} and {maximum}, found {count}"
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn oid(seed: u8) -> String {
        (0..40).map(|_| char::from(b'0' + seed % 10)).collect()
    }

    pub fn fingerprint(name: &str, path: &str, digest_seed: u8) -> Fingerprint {
        Fingerprint {
            name: name.to_owned(),
            repository: "github.com/example/d2b".to_owned(),
            path: path.to_owned(),
            sha256: sha256_bytes(&[digest_seed]),
        }
    }

    pub fn material() -> CandidateMaterial {
        CandidateMaterial {
            program: "ADR046".to_owned(),
            wave: "W0".to_owned(),
            repository_set: vec![RepositoryRecord {
                id: "github.com/example/d2b".to_owned(),
                object_format: GitObjectFormat::Sha1,
                base_oid: oid(1),
                head_oid: oid(2),
                integration_tree_oid: oid(3),
                expected_pull_requests: vec![ExpectedPullRequest {
                    number: 1,
                    head_oid: oid(2),
                }],
            }],
            dependency_graph: vec![
                DependencyEdge {
                    from: "adr046-w0".to_owned(),
                    to: "adr046-w1".to_owned(),
                },
                DependencyEdge {
                    from: "adr046-w1".to_owned(),
                    to: "adr046-w2".to_owned(),
                },
            ],
            generated_artifacts: vec![
                fingerprint("spec-set", "docs/specs/ADR-046-spec-set.json", 1),
                fingerprint("work-items", "docs/specs/ADR-046-work-items.json", 2),
            ],
            dependency_fingerprints: vec![fingerprint("cargo-lock", "packages/Cargo.lock", 3)],
            contract_fingerprints: vec![fingerprint("privileges", "docs/reference/schemas/v2", 4)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fixtures::*, *};

    #[test]
    fn identical_inputs_produce_identical_digests() {
        let first = material().digests().expect("digests");
        let second = material().digests().expect("digests");
        assert_eq!(first, second);
    }

    #[test]
    fn input_ordering_does_not_change_digests() {
        let ordered = material().digests().expect("digests");
        let mut shuffled = material();
        shuffled.dependency_graph.reverse();
        shuffled.generated_artifacts.reverse();
        let shuffled = shuffled.digests().expect("digests");
        assert_eq!(ordered, shuffled);
    }

    #[test]
    fn a_single_byte_content_change_changes_the_content_id() {
        let baseline = material().digests().expect("digests");
        let mut changed = material();
        let digest = changed.generated_artifacts[0].sha256.clone();
        let mut bytes = digest.into_bytes();
        // Flip exactly one hex character of one fingerprint.
        bytes[63] = if bytes[63] == b'a' { b'b' } else { b'a' };
        changed.generated_artifacts[0].sha256 =
            String::from_utf8(bytes).expect("hex digest stays UTF-8");
        let changed = changed.digests().expect("digests");
        assert_ne!(baseline.content_id, changed.content_id);
        assert_ne!(baseline.candidate_id, changed.candidate_id);
        assert_ne!(baseline.snapshot_sha256, changed.snapshot_sha256);
    }

    #[test]
    fn a_changed_integration_tree_changes_the_content_id() {
        let baseline = material().digests().expect("digests");
        let mut changed = material();
        changed.repository_set[0].integration_tree_oid = oid(4);
        let changed = changed.digests().expect("digests");
        assert_ne!(baseline.content_id, changed.content_id);
    }

    #[test]
    fn a_history_only_rebase_preserves_the_content_id() {
        let baseline = material().digests().expect("digests");
        let mut rebased = material();
        rebased.repository_set[0].base_oid = oid(5);
        rebased.repository_set[0].head_oid = oid(6);
        rebased.repository_set[0].expected_pull_requests[0].head_oid = oid(6);
        let rebased = rebased.digests().expect("digests");
        assert_eq!(baseline.content_id, rebased.content_id);
        assert_eq!(baseline.candidate_id, rebased.candidate_id);
        assert_ne!(baseline.snapshot_sha256, rebased.snapshot_sha256);
    }

    #[test]
    fn a_changed_dependency_graph_changes_the_candidate_id_only() {
        let baseline = material().digests().expect("digests");
        let mut changed = material();
        changed.dependency_graph.push(DependencyEdge {
            from: "adr046-w2".to_owned(),
            to: "adr046-w3".to_owned(),
        });
        let changed = changed.digests().expect("digests");
        assert_eq!(baseline.content_id, changed.content_id);
        assert_ne!(baseline.candidate_id, changed.candidate_id);
    }

    #[test]
    fn a_changed_repository_set_changes_the_candidate_id() {
        let baseline = material().digests().expect("digests");
        let mut changed = material();
        changed.repository_set.push(RepositoryRecord {
            id: "github.com/example/entrablau".to_owned(),
            object_format: GitObjectFormat::Sha1,
            base_oid: oid(7),
            head_oid: oid(8),
            integration_tree_oid: oid(9),
            expected_pull_requests: vec![ExpectedPullRequest {
                number: 1,
                head_oid: oid(8),
            }],
        });
        let changed = changed.digests().expect("digests");
        assert_ne!(baseline.candidate_id, changed.candidate_id);
    }

    #[test]
    fn domain_separation_keeps_identical_material_distinct() {
        let digests = material().digests().expect("digests");
        assert_ne!(digests.content_id.as_str(), digests.candidate_id.as_str());
        assert_ne!(
            digests.candidate_id.as_str(),
            digests.snapshot_sha256.as_str()
        );
    }

    #[test]
    fn derived_identifiers_are_lowercase_sha256() {
        let digests = material().digests().expect("digests");
        for value in [
            digests.content_id.as_str(),
            digests.candidate_id.as_str(),
            digests.snapshot_sha256.as_str(),
        ] {
            validate_sha256(value, "derived identifier").expect("derived identifier is a digest");
        }
    }

    #[test]
    fn identifiers_reject_a_non_digest_string() {
        assert!(CandidateId::parse("not-a-digest").is_err());
        assert!(ContentId::parse("A".repeat(64)).is_err());
        assert!(SnapshotSha256::parse("0".repeat(63)).is_err());
        assert!(CandidateId::parse("0".repeat(64)).is_ok());
    }

    #[test]
    fn deserializing_a_malformed_digest_is_rejected() {
        // Deserialize must run the same validator `parse` runs, so a hand-edited
        // artifact cannot smuggle a malformed digest past the newtype.
        for malformed in [
            "\"not-a-digest\"",
            "\"\"",
            &format!("\"{}\"", "A".repeat(64)),
            &format!("\"{}\"", "0".repeat(63)),
            &format!("\"{}\"", "0".repeat(65)),
            "\"g0000000000000000000000000000000000000000000000000000000000000000\"",
            "42",
        ] {
            assert!(
                serde_json::from_str::<ContentId>(malformed).is_err(),
                "ContentId accepted {malformed}"
            );
            assert!(
                serde_json::from_str::<CandidateId>(malformed).is_err(),
                "CandidateId accepted {malformed}"
            );
            assert!(
                serde_json::from_str::<SnapshotSha256>(malformed).is_err(),
                "SnapshotSha256 accepted {malformed}"
            );
        }
    }

    #[test]
    fn deserializing_a_well_formed_digest_round_trips() {
        let digest = format!("\"{}\"", "a".repeat(64));
        let content: ContentId = serde_json::from_str(&digest).expect("content id");
        let candidate: CandidateId = serde_json::from_str(&digest).expect("candidate id");
        let snapshot: SnapshotSha256 = serde_json::from_str(&digest).expect("snapshot digest");
        assert_eq!(content.as_str(), "a".repeat(64));
        assert_eq!(
            serde_json::to_string(&candidate).expect("serialize"),
            digest
        );
        assert_eq!(serde_json::to_string(&snapshot).expect("serialize"), digest);
    }

    #[test]
    fn the_legacy_wave_namespace_is_unchanged() {
        // Every closed wave is accepted.
        for wave in ADR046_WAVES {
            validate_wave(wave).expect("a closed wave is accepted");
            validate_program_wave(ADR046_PROGRAM, wave).expect("the closed pair is accepted");
        }
        // Name-like and free-form waves are rejected: a username, a branch
        // name, the historical lowercase spelling, and an out-of-range number.
        for wave in [
            "alice",
            "feature-branch",
            "w0",
            "W9",
            "",
            "W0 ",
            "adr046-w0",
        ] {
            assert!(
                validate_wave(wave).is_err(),
                "a wave outside the closed set must be rejected: {wave:?}"
            );
            assert!(
                validate_program_wave(ADR046_PROGRAM, wave).is_err(),
                "the pair must be rejected for a name-like wave: {wave:?}"
            );
        }
        // The program is fixed for a bare wave: a name-like program is rejected
        // even with a valid wave.
        for program in ["alice", "adr046", "ADR-046", "ADR047", ""] {
            assert!(
                validate_program_wave(program, "W0").is_err(),
                "a program outside the closed namespace must be rejected: {program:?}"
            );
        }
    }

    #[test]
    fn a_qualified_wave_carries_its_own_program() {
        for (wave, program, ordinal) in [
            ("adr046w0", "adr046", 0u8),
            ("adr046w1", "adr046", 1),
            ("spec001w1", "spec001", 1),
            ("spec001w8", "spec001", 8),
            // A program whose own name contains `w` still splits at the final
            // one, which is why the parser scans from the end.
            ("workflow2w3", "workflow2", 3),
        ] {
            assert_eq!(
                qualified_wave_parts(wave),
                Some((program, ordinal)),
                "a qualified wave must split into its program and ordinal: {wave:?}"
            );
            validate_wave(wave).expect("a qualified wave is accepted");
            validate_program_wave(&program.to_ascii_uppercase(), wave)
                .expect("a qualified wave agrees with its own program, spelled either case");
            validate_program_wave(program, wave)
                .expect("the comparison is case-insensitive in both directions");
        }
    }

    #[test]
    fn a_qualified_wave_is_rejected_when_it_could_name_a_path_or_a_free_form_string() {
        for wave in [
            // Separators, traversal, and absolute paths can never appear.
            "spec/001w1",
            "../w1",
            "..w1",
            "/etc/w1",
            "spec.001w1",
            "spec-001w1",
            "spec_001w1",
            // Case, whitespace, and control characters can never appear.
            "SPEC001w1",
            "spec001W1",
            "spec 001w1",
            "spec001w1\n",
            "\tspec001w1",
            // The ordinal stays bounded and single-digit.
            "spec001w9",
            "spec001w10",
            "spec001w",
            "spec001w-1",
            // The program component stays bounded and starts with a letter.
            "w1",
            "0spec001w1",
            "aw1",
            "abw1",
            "thisprogramnameisfartoolongw1",
            // A multi-byte character must not panic the parser.
            "spec001w\u{20ac}",
            "\u{20ac}w1",
            "spec\u{20ac}01w1",
        ] {
            assert!(
                qualified_wave_parts(wave).is_none(),
                "an unsafe qualified wave must not parse: {wave:?}"
            );
            assert!(
                validate_wave(wave).is_err(),
                "an unsafe qualified wave must be rejected: {wave:?}"
            );
        }
    }

    #[test]
    fn a_qualified_wave_disagreeing_with_its_program_is_rejected() {
        // The inconsistency is caught rather than one side silently winning,
        // because either choice would write state under an address the operator
        // did not ask for.
        for (program, wave) in [
            ("SPEC001", "adr046w1"),
            ("ADR046", "spec001w1"),
            ("SPEC002", "spec001w1"),
            ("", "spec001w1"),
        ] {
            let error = validate_program_wave(program, wave)
                .expect_err("a disagreeing program and wave must be rejected");
            assert!(
                error
                    .to_string()
                    .contains("disagrees with the requested program"),
                "the error must name the disagreement: {error}"
            );
        }
    }

    #[test]
    fn a_name_like_wave_is_rejected_by_canonicalize() {
        let mut named = material();
        "alice".clone_into(&mut named.wave);
        let error = named
            .digests()
            .expect_err("a name-like wave must not derive digests");
        assert!(
            error.message().contains("closed ADR 0046 wave namespace"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn a_duplicate_repository_is_rejected() {
        let mut duplicated = material();
        let existing = duplicated.repository_set[0].clone();
        duplicated.repository_set.push(existing);
        assert!(duplicated.digests().is_err());
    }

    #[test]
    fn a_fingerprint_outside_the_repository_set_is_rejected() {
        let mut stray = material();
        stray.generated_artifacts[0].repository = "github.com/example/other".to_owned();
        assert!(stray.digests().is_err());
    }

    #[test]
    fn the_panel_roster_is_the_ten_role_default_panel() {
        assert_eq!(PANEL_ROLES.len(), 10);
        let names = PANEL_ROLES.map(PanelRole::as_str);
        assert_eq!(
            names,
            [
                "software",
                "test",
                "nixos",
                "networking",
                "security",
                "rust",
                "product",
                "docs",
                "observability",
                "kernel",
            ]
        );
    }

    #[test]
    fn the_current_panel_domain_is_thirteen_roles_without_rust() {
        assert_eq!(PANEL_CURRENT_ROLES.len(), 13);
        assert_eq!(
            PANEL_CURRENT_ROLES.map(PanelRole::as_str),
            [
                "software",
                "test",
                "product",
                "docs",
                "security",
                "observability",
                "simplicity",
                "reliability",
                "agentic",
                "nixos",
                "networking",
                "kernel",
                "build",
            ]
        );
        assert!(!PANEL_CURRENT_ROLES.contains(&PanelRole::Rust));
    }

    #[test]
    fn the_panel_binding_pins_provider_model_and_reasoning_effort() {
        assert_eq!(PANEL_PROVIDER_POLICY, "github-copilot");
        assert_eq!(PANEL_MODEL_POLICY, "gpt-5.6-sol");
        assert_eq!(PANEL_REASONING_EFFORT_POLICY, "xhigh");
        assert_eq!(PANEL_LEGACY_MODEL_POLICY, "gemini-3.1-pro-preview");
        assert_eq!(PANEL_LEGACY_REASONING_EFFORT_POLICY, "high");
    }

    fn selection(
        roster: &[PanelRole],
        candidate_class: &str,
        changed_paths: &[&str],
        signals: &[&str],
        software_profiles: &[&str],
    ) -> PanelSelectionV1 {
        let material = material();
        let digests = material.digests().expect("digests");
        let changed_paths =
            if changed_paths.is_empty() && matches!(candidate_class, "code" | "configuration") {
                vec!["src/panel.txt"]
            } else {
                changed_paths.to_vec()
            };
        PanelSelectionV1 {
            artifact_kind: PANEL_SELECTION_ARTIFACT_KIND.to_owned(),
            schema_version: PANEL_SELECTION_SCHEMA_VERSION,
            lifecycle_id: "selection-tests".to_owned(),
            phase: "discovery".to_owned(),
            program: material.program,
            wave: material.wave,
            candidate_id: digests.candidate_id,
            content_id: digests.content_id,
            snapshot_sha256: digests.snapshot_sha256,
            selection_table_version: PANEL_SELECTION_TABLE_VERSION,
            candidate_class: candidate_class.to_owned(),
            classification_inputs: serde_json::json!({
                "changed_paths": changed_paths,
                "signals": signals,
                "candidate_class": candidate_class,
                "ambiguous": candidate_class == "ambiguous",
            }),
            ambiguity_widened: candidate_class == "ambiguous",
            profiles: roster
                .iter()
                .map(|role| {
                    let profiles = if *role == PanelRole::Software {
                        software_profiles
                            .iter()
                            .map(|profile| (*profile).to_owned())
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (role.as_str().to_owned(), profiles)
                })
                .collect(),
            roster: roster.to_vec(),
        }
    }

    #[test]
    fn selection_validation_uses_table_mandatory_seats_and_floors() {
        let mandatory_only = selection(&PANEL_CURRENT_ROLES[..7], "code", &[], &[], &[]);
        let error = mandatory_only
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("code selection below the table floor must fail");
        assert!(error.message().contains("requires at least 10"), "{error}");

        let missing_software = selection(
            &[
                PanelRole::Test,
                PanelRole::Product,
                PanelRole::Docs,
                PanelRole::Security,
                PanelRole::Observability,
                PanelRole::Simplicity,
                PanelRole::Reliability,
                PanelRole::Agentic,
                PanelRole::Nixos,
                PanelRole::Networking,
            ],
            "code",
            &[],
            &[],
            &[],
        );
        let error = missing_software
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("a selection omitting a mandatory seat must fail");
        assert!(
            error.message().contains("mandatory seat software"),
            "{error}"
        );

        let documentation = selection(&PANEL_CURRENT_ROLES[..8], "documentation", &[], &[], &[]);
        documentation
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("documentation floor and mandatory seats come from the table");
    }

    #[test]
    fn selection_validation_requires_every_triggered_optional_seat() {
        let cases = [
            (
                "src/state-machine.rs",
                &[
                    PanelRole::Software,
                    PanelRole::Test,
                    PanelRole::Product,
                    PanelRole::Docs,
                    PanelRole::Security,
                    PanelRole::Observability,
                    PanelRole::Simplicity,
                    PanelRole::Agentic,
                    PanelRole::Nixos,
                    PanelRole::Networking,
                ][..],
                "reliability",
            ),
            (
                ".github/agents/panel-test.agent.md",
                &[
                    PanelRole::Software,
                    PanelRole::Test,
                    PanelRole::Product,
                    PanelRole::Docs,
                    PanelRole::Security,
                    PanelRole::Observability,
                    PanelRole::Simplicity,
                    PanelRole::Reliability,
                    PanelRole::Nixos,
                    PanelRole::Networking,
                ][..],
                "agentic",
            ),
            (
                "configuration.nix",
                &[
                    PanelRole::Software,
                    PanelRole::Test,
                    PanelRole::Product,
                    PanelRole::Docs,
                    PanelRole::Security,
                    PanelRole::Observability,
                    PanelRole::Simplicity,
                    PanelRole::Reliability,
                    PanelRole::Agentic,
                    PanelRole::Networking,
                ][..],
                "nixos",
            ),
            (
                "src/network-firewall.rs",
                &[
                    PanelRole::Software,
                    PanelRole::Test,
                    PanelRole::Product,
                    PanelRole::Docs,
                    PanelRole::Security,
                    PanelRole::Observability,
                    PanelRole::Simplicity,
                    PanelRole::Reliability,
                    PanelRole::Agentic,
                    PanelRole::Nixos,
                ][..],
                "networking",
            ),
            (
                "src/syscall.rs",
                &[
                    PanelRole::Software,
                    PanelRole::Test,
                    PanelRole::Product,
                    PanelRole::Docs,
                    PanelRole::Security,
                    PanelRole::Observability,
                    PanelRole::Simplicity,
                    PanelRole::Reliability,
                    PanelRole::Agentic,
                    PanelRole::Nixos,
                ][..],
                "kernel",
            ),
        ];
        for (path, roster, seat) in cases {
            let selection = selection(roster, "code", &[path], &[], &[]);
            let error = selection
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .expect_err("a triggered optional seat cannot be omitted");
            assert!(
                error.message().contains(&format!("optional seat {seat}")),
                "{seat}: {error}"
            );
        }

        let build = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &["Cargo.toml"],
            &[],
            &["rust"],
        );
        let error = build
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("Cargo.toml must trigger the optional build seat");
        assert!(error.message().contains("optional seat build"), "{error}");

        let build_signal = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &[],
            &["build-contract"],
            &[],
        );
        let error = build_signal
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("a build classification signal must trigger build");
        assert!(error.message().contains("optional seat build"), "{error}");

        let mut complete = build;
        complete.roster.push(PanelRole::Build);
        complete.profiles.insert("build".to_owned(), Vec::new());
        complete
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("the triggered build seat is accepted when present");
    }

    #[test]
    fn selection_validation_requires_table_profiles_and_rejects_unknown_profiles() {
        let mut missing_rust = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &["packages/xtask/src/main.rs"],
            &[],
            &[],
        );
        missing_rust.roster.push(PanelRole::Build);
        missing_rust.profiles.insert("build".to_owned(), Vec::new());
        let error = missing_rust
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("Rust is a required software profile for a Rust path");
        assert!(error.message().contains("software/rust"), "{error}");

        let unknown = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &[],
            &[],
            &["not-a-table-profile"],
        );
        let error = unknown
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("profiles outside the table must fail");
        assert!(
            error
                .message()
                .contains("not defined by the selection table"),
            "{error}"
        );

        let duplicate = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &[],
            &[],
            &["rust", "rust"],
        );
        let error = duplicate
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("duplicate profiles must fail closed");
        assert!(error.message().contains("repeats profile"), "{error}");
    }

    #[test]
    fn selection_validation_rejects_inconsistent_classification_metadata() {
        let mut class_mismatch = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &["src/panel.txt"],
            &[],
            &[],
        );
        class_mismatch.classification_inputs["candidate_class"] =
            serde_json::json!("documentation");
        assert!(
            class_mismatch
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "top-level and nested candidate classes must agree"
        );

        let mut ambiguity_mismatch = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &["src/panel.txt"],
            &[],
            &[],
        );
        ambiguity_mismatch.ambiguity_widened = true;
        assert!(
            ambiguity_mismatch
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "top-level and nested ambiguity must agree"
        );

        for candidate_class in ["code", "configuration"] {
            let mut empty_paths = selection(
                &PANEL_CURRENT_ROLES[..10],
                candidate_class,
                &["src/panel.txt"],
                &[],
                &[],
            );
            empty_paths.classification_inputs["changed_paths"] = serde_json::json!([]);
            assert!(
                empty_paths
                    .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                    .is_err(),
                "{candidate_class} classifications need a changed path"
            );
        }

        let mut unknown_field = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &["src/panel.txt"],
            &[],
            &[],
        );
        unknown_field.classification_inputs["unexpected"] = serde_json::json!(true);
        assert!(
            unknown_field
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "unknown classification fields must fail closed"
        );
    }

    #[test]
    fn selection_validation_checks_nested_full_candidate_and_fix_delta_classifications() {
        let mut verification = selection(
            &PANEL_CURRENT_ROLES,
            "code",
            &["docs/guide.md", "src/panel.txt"],
            &[],
            &["markdown"],
        );
        verification.phase = "verification".to_owned();
        verification.classification_inputs = serde_json::json!({
            "changed_paths": ["docs/guide.md", "src/panel.txt"],
            "signals": [],
            "candidate_class": "code",
            "ambiguous": false,
            "full_candidate": {
                "changed_paths": ["src/panel.txt"],
                "signals": [],
                "candidate_class": "code",
                "ambiguous": false,
            },
            "fix_delta": {
                "changed_paths": ["docs/guide.md"],
                "signals": [],
                "candidate_class": "documentation",
                "ambiguous": false,
            },
        });
        verification
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("consistent nested classifications");

        let mut stale_union = verification.clone();
        stale_union.classification_inputs["changed_paths"] = serde_json::json!(["src/panel.txt"]);
        assert!(
            stale_union
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "top-level paths must include both nested classifications"
        );

        let mut unknown_nested_field = verification.clone();
        unknown_nested_field.classification_inputs["full_candidate"]["unexpected"] =
            serde_json::json!(true);
        assert!(
            unknown_nested_field
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "nested classification fields must be closed"
        );

        let mut empty_full_candidate = verification.clone();
        empty_full_candidate.classification_inputs["full_candidate"]["changed_paths"] =
            serde_json::json!([]);
        assert!(
            empty_full_candidate
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "a code full-candidate classification needs a changed path"
        );

        let mut inconsistent_nested_ambiguity = verification.clone();
        inconsistent_nested_ambiguity.classification_inputs["fix_delta"]["candidate_class"] =
            serde_json::json!("ambiguous");
        assert!(
            inconsistent_nested_ambiguity
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .is_err(),
            "nested candidate class and ambiguity must agree"
        );

        let mut empty_fix_delta =
            selection(&PANEL_CURRENT_ROLES, "code", &["src/panel.txt"], &[], &[]);
        empty_fix_delta.phase = "verification".to_owned();
        empty_fix_delta.classification_inputs = serde_json::json!({
            "changed_paths": ["src/panel.txt"],
            "signals": [],
            "candidate_class": "code",
            "ambiguous": false,
            "full_candidate": {
                "changed_paths": ["src/panel.txt"],
                "signals": [],
                "candidate_class": "code",
                "ambiguous": false,
            },
            "fix_delta": {
                "changed_paths": [],
                "signals": [],
                "candidate_class": "code",
                "ambiguous": false,
            },
        });
        empty_fix_delta
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("the standard no-op fix delta remains readable");

        let mut missing_nested_classification = verification.clone();
        missing_nested_classification
            .classification_inputs
            .as_object_mut()
            .expect("classification inputs object")
            .remove("fix_delta");
        let error = missing_nested_classification
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("verification selections require both nested classifications");
        assert!(
            error
                .message()
                .contains("must contain both full_candidate and fix_delta"),
            "{error}"
        );
    }

    #[test]
    fn selection_validation_matches_javascript_classification_parity() {
        assert_eq!(
            candidate_class_precedence(&["documentation", "configuration"]),
            "configuration"
        );
        assert_eq!(
            candidate_class_precedence(&["documentation", "code"]),
            "code"
        );
        assert_eq!(
            candidate_class_precedence(&["configuration", "ambiguous"]),
            "ambiguous"
        );

        let mut documentation_full_configuration_delta = selection(
            &PANEL_CURRENT_ROLES,
            "configuration",
            &["docs/full-review.md", "nixos/panel.nix"],
            &[],
            &["markdown", "nix"],
        );
        documentation_full_configuration_delta.phase = "verification".to_owned();
        documentation_full_configuration_delta.classification_inputs = serde_json::json!({
            "changed_paths": ["docs/full-review.md", "nixos/panel.nix"],
            "signals": [],
            "candidate_class": "configuration",
            "ambiguous": false,
            "full_candidate": {
                "changed_paths": ["docs/full-review.md"],
                "signals": [],
                "candidate_class": "documentation",
                "ambiguous": false,
            },
            "fix_delta": {
                "changed_paths": ["nixos/panel.nix"],
                "signals": [],
                "candidate_class": "configuration",
                "ambiguous": false,
            },
        });
        documentation_full_configuration_delta
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("configuration must take precedence over documentation");

        let non_documentation = selection(
            &PANEL_CURRENT_ROLES[..8],
            "documentation",
            &["src/panel.rs"],
            &[],
            &[],
        );
        let error = non_documentation
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("documentation must not narrow a non-documentation path");
        assert!(
            error
                .message()
                .contains("cannot narrow actual non-documentation paths"),
            "{error}"
        );

        let literal_backslash = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &[r"src\panel.rs"],
            &[],
            &["rust"],
        );
        literal_backslash
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("a backslash is a literal POSIX filename character");

        for path in ["./src/panel.rs", "src//panel.rs", "src/panel.rs/"] {
            let noncanonical = selection(&PANEL_CURRENT_ROLES[..10], "code", &[path], &[], &[]);
            let error = noncanonical
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .expect_err("classification paths must already be canonical");
            assert!(
                error.message().contains("canonical normalized paths"),
                "{path:?}: {error}"
            );
        }

        let canonical_signal = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &["src/panel.rs"],
            &["rust"],
            &["rust"],
        );
        canonical_signal
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("canonical lowercase signals remain accepted");
        for signal in ["Rust", " rust", "rust "] {
            let noncanonical_signal = selection(
                &PANEL_CURRENT_ROLES[..10],
                "code",
                &["src/panel.rs"],
                &[signal],
                &["rust"],
            );
            let error = noncanonical_signal
                .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
                .expect_err("classification signals must already be canonical");
            assert!(
                error.message().contains("lowercase, and sorted"),
                "{signal:?}: {error}"
            );
        }
    }

    #[test]
    fn selection_validation_rejects_c0_and_c1_controls_in_paths_and_signals() {
        for control in ['\u{0000}', '\u{0080}', '\u{009f}'] {
            let path = format!("src/panel{control}.rs");
            let path_error = selection(
                &PANEL_CURRENT_ROLES[..10],
                "code",
                &[path.as_str()],
                &[],
                &[],
            )
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("C0 and C1 controls must be rejected in paths");
            assert!(
                path_error.message().contains("control characters"),
                "{control:?}: {path_error}"
            );

            let signal = format!("rust{control}");
            let signal_error = selection(
                &PANEL_CURRENT_ROLES[..10],
                "code",
                &["src/panel.rs"],
                &[signal.as_str()],
                &[],
            )
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("C0 and C1 controls must be rejected in signals");
            assert!(
                signal_error.message().contains("control characters"),
                "{control:?}: {signal_error}"
            );
        }
    }

    #[test]
    fn glob_matching_matches_javascript_segment_and_literal_parity() {
        let cargo_pattern = "**/Cargo.toml";
        assert!(glob_matches("Cargo.toml", cargo_pattern));
        assert!(glob_matches("dir/Cargo.toml", cargo_pattern));
        assert!(!glob_matches("xCargo.toml", cargo_pattern));

        assert!(!glob_matches(r"dir\Cargo.toml", cargo_pattern));
        assert!(glob_matches(r"dir\Cargo.toml", r"dir\Cargo.toml"));
        assert!(glob_matches("\u{e000}/Cargo.toml", cargo_pattern));
        assert!(glob_matches("\u{10000}/Cargo.toml", cargo_pattern));
    }

    #[test]
    fn unicode_classification_keeps_rust_byte_order_and_signal_case_parity() {
        let bmp = "\u{e000}";
        let non_bmp = "\u{10000}";
        assert!(bmp.as_bytes() < non_bmp.as_bytes());

        let ordered = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &[bmp, non_bmp],
            &[bmp, non_bmp],
            &[],
        );
        ordered
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect("classification paths and signals use UTF-8 byte ordering");

        let reversed = selection(
            &PANEL_CURRENT_ROLES[..10],
            "code",
            &[non_bmp, bmp],
            &[non_bmp, bmp],
            &[],
        );
        let error = reversed
            .validate_for_snapshot("ADR046", "W0", &mandatory_only_snapshot_digests())
            .expect_err("classification ordering must remain UTF-8 byte lexicographic");
        assert!(error.message().contains("unique and sorted"), "{error}");

        let inputs = SelectionInputs {
            changed_paths: Vec::new(),
            signals: vec!["ä".to_owned(), "\u{10428}".to_owned()],
            candidate_class: "code".to_owned(),
            ambiguous: false,
            full_candidate: None,
            fix_delta: None,
        };
        let trigger = SelectionTrigger {
            kind: "signal".to_owned(),
            patterns: None,
            values: Some(vec!["Ä".to_owned(), "\u{10400}".to_owned()]),
        };
        assert!(trigger_matches(&trigger, &inputs));
    }

    fn mandatory_only_snapshot_digests() -> CandidateDigests {
        material().digests().expect("digests")
    }
}
