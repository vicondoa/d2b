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
    collections::BTreeSet,
    fmt,
    path::{Component, Path},
};

use serde::{Deserialize, Serialize};
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
pub const PANEL_MODEL_POLICY: &str = "gemini-3.1-pro-preview";
pub const PANEL_REASONING_EFFORT_POLICY: &str = "xhigh";

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

/// The ten-role default panel from spec section 12.3.
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
        }
    }
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

/// The one delivery program this tooling serves: ADR 0046. Spec section 3.2
/// (`docs/specs/ADR-046-validation-and-delivery.md`) defines a closed wave
/// namespace `ADR046-W0`..`ADR046-W8`, split here into the fixed program
/// component and the closed wave component.
pub const ADR046_PROGRAM: &str = "ADR046";

/// The closed set of ADR 0046 wave identifiers, `W0` through `W8`. There is no
/// generic or operator-chosen wave: a value outside this set - a username, a
/// branch name, or any free-form string - is rejected before it can name a
/// state directory or be emitted in an artifact reference.
pub const ADR046_WAVES: [&str; 9] = ["W0", "W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];

/// Rejects any wave outside the closed ADR 0046 namespace.
///
/// The wave becomes a path component (`<state-root>/<wave>/<candidate>/...`)
/// and is echoed verbatim in every stage's `artifact` reference, so allowing a
/// free-form identifier would let a username or other operator string leak
/// into structured output. Membership in the fixed [`ADR046_WAVES`] set is the
/// only accepted form.
pub fn validate_wave(wave: &str) -> Result<()> {
    if !ADR046_WAVES.contains(&wave) {
        return Err(DeliveryError::new(format!(
            "delivery wave must be one of {} - the closed ADR 0046 wave namespace",
            ADR046_WAVES.join(", ")
        )));
    }
    Ok(())
}

/// Rejects any program/wave pair outside the closed ADR 0046 namespace.
///
/// The program must be exactly [`ADR046_PROGRAM`] and the wave must be a member
/// of [`ADR046_WAVES`]. This is the single gate every stage runs before it
/// creates delivery state or emits an artifact reference, so no name-like value
/// can reach a state directory name or structured stdout.
pub fn validate_program_wave(program: &str, wave: &str) -> Result<()> {
    if program != ADR046_PROGRAM {
        return Err(DeliveryError::new(format!(
            "delivery program must be {ADR046_PROGRAM} - the only supported ADR 0046 program"
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
    fn the_wave_namespace_is_the_closed_adr046_set() {
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
        // The program is fixed: a name-like program is rejected even with a
        // valid wave.
        for program in ["alice", "adr046", "ADR-046", "ADR047", ""] {
            assert!(
                validate_program_wave(program, "W0").is_err(),
                "a program outside the closed namespace must be rejected: {program:?}"
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
    fn the_panel_binding_pins_provider_model_and_reasoning_effort() {
        assert_eq!(PANEL_PROVIDER_POLICY, "github-copilot");
        assert_eq!(PANEL_MODEL_POLICY, "gemini-3.1-pro-preview");
        assert_eq!(PANEL_REASONING_EFFORT_POLICY, "xhigh");
    }
}
