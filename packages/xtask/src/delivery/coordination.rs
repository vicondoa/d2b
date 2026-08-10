//! Wave 6 entry, dispatch, and close receipts.
//!
//! The implementation worktree is not the authority for the Wave 6 launch
//! state.  That state is kept in a small set of external, candidate-bound
//! records:
//!
//! * a versioned dispatch ledger owns group and local-task transitions;
//! * command evidence keeps command identity, output digests, counts, and
//!   status without retaining raw output;
//! * a plan-approval receipt binds the selected lifecycle to the exact entry
//!   material.
//!
//! These records are process-correlation evidence.  They are deliberately not
//! authentication, a signature, or a new service boundary.  Their writers use
//! the same external-state and durable-write rules as [`StateRoot`] and
//! [`CandidateDir`].

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    DeliveryError, Result,
    model::{
        CandidateId, CandidateMaterial, PANEL_CURRENT_ROLES, PanelRole, PanelSelectionV1,
        SnapshotSha256, canonical_digest, qualified_wave_parts, sha256_bytes,
        validate_bounded_string, validate_hash, validate_identifier, validate_sha256,
    },
    storage::{MAX_JSON_BYTES, absolute_path, ensure_external_path},
};

pub const DISPATCH_LEDGER_ARTIFACT_KIND: &str = "d2b-feature-local/dispatch-ledger";
pub const COMMAND_EVIDENCE_ARTIFACT_KIND: &str = "d2b-feature-local/command-evidence";
pub const PLAN_APPROVAL_ARTIFACT_KIND: &str = "d2b-feature-local/plan-approval";
pub const FRESH_FETCH_ARTIFACT_KIND: &str = "d2b-feature-local/fresh-fetch";

pub const DISPATCH_LEDGER_SCHEMA_VERSION: u32 = 1;
pub const COMMAND_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const PLAN_APPROVAL_SCHEMA_VERSION: u32 = 1;
pub const FRESH_FETCH_SCHEMA_VERSION: u32 = 1;

pub const DISPATCH_LEDGER_ENV: &str = "D2B_W6_DISPATCH_LEDGER";
pub const COMMAND_EVIDENCE_ENV: &str = "D2B_W6_COMMAND_EVIDENCE_DIR";
pub const PLAN_APPROVAL_ENV: &str = "D2B_W6_PLAN_APPROVAL_RECEIPT";
pub const FRESH_FETCH_ENV: &str = "D2B_W6_FETCH_EVIDENCE";

const FEATURE_DIR: &str = "specs/001-adr046-d2b3-completion";
const GRAPH_PATH: &str = "docs/specs/ADR-046-implementation-graph.json";
const WORK_ITEMS_PATH: &str = "docs/specs/ADR-046-work-items.json";
const TASKS_PATH: &str = "specs/001-adr046-d2b3-completion/tasks.md";

const W6_MANIFEST_GROUPS: [&str; 29] = [
    "wi:ADR-046-provider-activation-nixos",
    "wi:ADR-046-provider-audio-pipewire",
    "wi:ADR-046-provider-clipboard-wayland",
    "wi:ADR-046-provider-credential-entra",
    "wi:ADR-046-provider-credential-managed-identity",
    "wi:ADR-046-provider-credential-secret-service",
    "wi:ADR-046-provider-device-gpu",
    "wi:ADR-046-provider-device-security-key",
    "wi:ADR-046-provider-device-tpm",
    "wi:ADR-046-provider-device-usbip",
    "wi:ADR-046-provider-display-wayland",
    "wi:ADR-046-provider-network-local",
    "wi:ADR-046-provider-notification-desktop",
    "wi:ADR-046-provider-observability-otel",
    "wi:ADR-046-provider-runtime-azure-container-apps",
    "wi:ADR-046-provider-runtime-azure-virtual-machine",
    "wi:ADR-046-provider-runtime-cloud-hypervisor",
    "wi:ADR-046-provider-runtime-qemu-media",
    "wi:ADR-046-provider-shell-terminal",
    "wi:ADR-046-provider-system-core",
    "wi:ADR-046-provider-system-minijail",
    "wi:ADR-046-provider-system-systemd",
    "wi:ADR-046-provider-transport-azure-relay",
    "wi:ADR-046-provider-transport-unix",
    "wi:ADR-046-provider-transport-vsock",
    "wi:ADR-046-provider-volume-local",
    "wi:ADR-046-provider-volume-virtiofs",
    "wi:core-controller-coordination:w6",
    "wi:process-provider-integration:w6",
];

pub const W6_LOCAL_GROUPS: [&str; 7] = [
    "feature-local:w6-shared-prep",
    "feature-local:w6-core-control-foundations",
    "feature-local:w6-storage-authority-foundations",
    "feature-local:w6-audit-telemetry-foundations",
    "feature-local:w6-operator-acceptance",
    "feature-local:w6-converge",
    "feature-local:w6-close",
];

pub const W6_GROUPS: [&str; 36] = [
    "wi:ADR-046-provider-activation-nixos",
    "wi:ADR-046-provider-audio-pipewire",
    "wi:ADR-046-provider-clipboard-wayland",
    "wi:ADR-046-provider-credential-entra",
    "wi:ADR-046-provider-credential-managed-identity",
    "wi:ADR-046-provider-credential-secret-service",
    "wi:ADR-046-provider-device-gpu",
    "wi:ADR-046-provider-device-security-key",
    "wi:ADR-046-provider-device-tpm",
    "wi:ADR-046-provider-device-usbip",
    "wi:ADR-046-provider-display-wayland",
    "wi:ADR-046-provider-network-local",
    "wi:ADR-046-provider-notification-desktop",
    "wi:ADR-046-provider-observability-otel",
    "wi:ADR-046-provider-runtime-azure-container-apps",
    "wi:ADR-046-provider-runtime-azure-virtual-machine",
    "wi:ADR-046-provider-runtime-cloud-hypervisor",
    "wi:ADR-046-provider-runtime-qemu-media",
    "wi:ADR-046-provider-shell-terminal",
    "wi:ADR-046-provider-system-core",
    "wi:ADR-046-provider-system-minijail",
    "wi:ADR-046-provider-system-systemd",
    "wi:ADR-046-provider-transport-azure-relay",
    "wi:ADR-046-provider-transport-unix",
    "wi:ADR-046-provider-transport-vsock",
    "wi:ADR-046-provider-volume-local",
    "wi:ADR-046-provider-volume-virtiofs",
    "wi:core-controller-coordination:w6",
    "wi:process-provider-integration:w6",
    "feature-local:w6-shared-prep",
    "feature-local:w6-core-control-foundations",
    "feature-local:w6-storage-authority-foundations",
    "feature-local:w6-audit-telemetry-foundations",
    "feature-local:w6-operator-acceptance",
    "feature-local:w6-converge",
    "feature-local:w6-close",
];

const W6_LOCAL_TASKS: [&str; 7] = ["T606", "T607", "T608", "T609", "T604", "T479", "T480"];

const REQUIRED_T221_COMMANDS: [&str; 8] = [
    "focused-guard-list",
    "focused-guard-ignored-list",
    "focused-guard-run",
    "gate0-test-drift",
    "test-policy",
    "test-unit",
    "heavy-gate-acquire",
    "predispatch-census",
];

const LOCAL_COMPLETION_EVIDENCE: [(&str, &[&str]); 7] = [
    (
        "feature-local:w6-shared-prep",
        &[
            "w6-shared-prep-inventory",
            "w6-shared-prep-shared-writers",
            "w6-shared-prep-lockfile-flake-packages",
        ],
    ),
    (
        "feature-local:w6-core-control-foundations",
        &[
            "w6-core-control-production-route",
            "w6-real-so-peercred-admission",
        ],
    ),
    (
        "feature-local:w6-storage-authority-foundations",
        &[
            "w6-typed-broker-host-effects",
            "w6-strict-resource-nix-validation",
            "w6-tpm-legacy-migration",
            "w6-host-global-authority",
        ],
    ),
    (
        "feature-local:w6-audit-telemetry-foundations",
        &[
            "w6-transactional-privileged-audit",
            "w6-forbidden-identity-redaction",
            "w6-bounded-telemetry",
            "w6-closed-metric-descriptors",
        ],
    ),
    (
        "feature-local:w6-operator-acceptance",
        &[
            "operator-nix-activation-cleanup-development",
            "daemon-restart-vm-survival-development",
        ],
    ),
    (
        "feature-local:w6-converge",
        &[
            "operator-nix-activation-cleanup",
            "w6-cloud-hypervisor-guest-acceptance",
        ],
    ),
    (
        "feature-local:w6-close",
        &[
            "w6-binding-panel-unanimous",
            "w6-protected-merge",
            "w6-post-merge-seal",
            "w6-merge-eligibility",
        ],
    ),
];

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn is_wave6(material: &CandidateMaterial) -> bool {
    let ordinal = if material.wave == "W6" {
        Some(6)
    } else {
        qualified_wave_parts(&material.wave).map(|(_, ordinal)| ordinal)
    };
    ordinal == Some(6)
        && (material.program.eq_ignore_ascii_case("ADR046")
            || material.program.eq_ignore_ascii_case("SPEC001"))
}

pub fn is_w6_entry_wave(material: &CandidateMaterial) -> bool {
    is_wave6(material)
}

pub fn is_wave6_entry_wave(material: &CandidateMaterial) -> bool {
    is_wave6(material)
}

pub fn is_wave6_identity(program: &str, wave: &str) -> bool {
    let ordinal = if wave == "W6" {
        Some(6)
    } else {
        qualified_wave_parts(wave).map(|(_, ordinal)| ordinal)
    };
    ordinal == Some(6)
        && (program.eq_ignore_ascii_case("ADR046") || program.eq_ignore_ascii_case("SPEC001"))
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn required_env_path(name: &str) -> Result<PathBuf> {
    let value = std::env::var_os(name).ok_or_else(|| {
        DeliveryError::environment(format!("{name} is required for Wave 6 delivery state"))
    })?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(DeliveryError::environment(format!(
            "{name} must be an absolute path"
        )));
    }
    Ok(path)
}

fn validate_external_file(
    path: &Path,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<PathBuf> {
    let path = absolute_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| DeliveryError::environment("external delivery record has no parent"))?;
    let roots = repository_roots.values().cloned().collect::<Vec<_>>();
    ensure_external_path(parent, &roots)?;
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && metadata.file_type().is_symlink()
    {
        return Err(DeliveryError::new(
            "external delivery record must not be a symlink",
        ));
    }
    Ok(path)
}

pub struct W6Paths {
    pub ledger: PathBuf,
    pub command_evidence_dir: PathBuf,
    pub plan_approval: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreshFetchEvidence {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub repository: String,
    pub remote: String,
    pub ref_name: String,
    pub fetched_oid: String,
    pub command_id: String,
    pub argv: Vec<String>,
    pub started_at_unix: u64,
    pub completed_at_unix: u64,
    pub exit_code: i32,
    pub result: CommandResult,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub output_bytes: u64,
    pub fetched_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_oid: Option<String>,
}

impl FreshFetchEvidence {
    pub fn validate(&self, root: &Path, expected_oid: &str) -> Result<()> {
        if self.artifact_kind != FRESH_FETCH_ARTIFACT_KIND {
            return Err(DeliveryError::new(
                "fresh-fetch evidence has an unexpected artifact kind",
            ));
        }
        if self.schema_version != FRESH_FETCH_SCHEMA_VERSION {
            return Err(DeliveryError::new(
                "fresh-fetch evidence has an unsupported schema version",
            ));
        }
        if self.repository != "github.com/vicondoa/d2b"
            || self.remote != "origin"
            || !matches!(self.ref_name.as_str(), "v3" | "refs/heads/v3")
        {
            return Err(DeliveryError::new(
                "fresh-fetch evidence must identify origin/v3 for the d2b repository",
            ));
        }
        validate_hash(&self.fetched_oid, "fresh-fetch OID")?;
        validate_hash(expected_oid, "entry base OID")?;
        if self.fetched_oid != expected_oid {
            return Err(DeliveryError::new(
                "fresh-fetch evidence names a different fetched tip than the entry base",
            ));
        }
        let plain_fetch = self.argv
            == ["git", "fetch", "origin", "v3"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
        let no_tags_fetch = self.argv
            == ["git", "fetch", "--no-tags", "origin", "v3"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
        if self.command_id != "git-fetch-origin-v3" || !(plain_fetch || no_tags_fetch) {
            return Err(DeliveryError::new(
                "fresh-fetch evidence does not identify the exact origin/v3 fetch command",
            ));
        }
        if self.completed_at_unix < self.started_at_unix
            || self.fetched_at_unix < self.completed_at_unix
            || self.fetched_at_unix == 0
        {
            return Err(DeliveryError::new(
                "fresh-fetch evidence timestamps are not ordered",
            ));
        }
        if self.result != CommandResult::Passed || self.exit_code != 0 {
            return Err(DeliveryError::new(
                "fresh-fetch evidence does not carry a successful fetch status",
            ));
        }
        validate_sha256(&self.stdout_sha256, "fresh-fetch stdout digest")?;
        validate_sha256(&self.stderr_sha256, "fresh-fetch stderr digest")?;
        if let Some(before_oid) = &self.before_oid {
            validate_hash(before_oid, "fresh-fetch previous OID")?;
        }
        let resolved = git_resolve_commit(root, &self.fetched_oid)?;
        if resolved != self.fetched_oid {
            return Err(DeliveryError::new(
                "fresh-fetch evidence names a commit that is not present in the checkout",
            ));
        }
        Ok(())
    }

    pub fn validate_for_close(
        &self,
        ledger: &DispatchLedger,
        command_evidence: &CommandEvidenceSet,
        feature_dir: &Path,
        panel_roles: Option<&[PanelRole]>,
    ) -> Result<()> {
        self.validate_shape()?;
        if let Some(panel_roles) = panel_roles {
            let roster = panel_roles
                .iter()
                .map(|role| role.as_str().to_owned())
                .collect::<Vec<_>>();
            if roster != self.selected_roster {
                return Err(DeliveryError::new(
                    "plan approval receipt selected roster differs from the panel request",
                ));
            }
        }
        if self.dispatch_ledger_sha256 != ledger.material_digest()? {
            return Err(DeliveryError::new(
                "plan approval receipt dispatch ledger material has changed",
            ));
        }
        if self.command_evidence_set_sha256 != command_evidence.digest() {
            return Err(DeliveryError::new(
                "plan approval receipt command evidence set has changed",
            ));
        }
        let expected_feature = feature_plan_material_digest(feature_dir)?;
        if self.feature_plan_material_sha256 != expected_feature {
            return Err(DeliveryError::new(
                "plan approval receipt feature material is stale; status-only checkbox updates \
                 are excluded, but requirement and guard changes invalidate approval",
            ));
        }
        Ok(())
    }
}

pub fn read_fresh_fetch_evidence(
    path: &Path,
    root: &Path,
    repository_roots: &BTreeMap<String, PathBuf>,
    expected_oid: &str,
) -> Result<FreshFetchEvidence> {
    let path = validate_external_file(path, repository_roots)?;
    let bytes = read_external_json(&path, "fresh-fetch evidence")?;
    let evidence: FreshFetchEvidence = serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid fresh-fetch evidence: {error}")))?;
    evidence.validate(root, expected_oid)?;
    Ok(evidence)
}

fn git_resolve_commit(root: &Path, oid: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", &format!("{oid}^{{commit}}")])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| DeliveryError::environment("cannot verify the fetched commit"))?;
    if !output.status.success() {
        return Err(DeliveryError::new(
            "fresh-fetch evidence names a commit absent from the checkout",
        ));
    }
    let resolved = String::from_utf8(output.stdout)
        .map_err(|_| DeliveryError::environment("fetched commit verification was not UTF-8"))?;
    Ok(resolved.trim().to_owned())
}

/// Performs the entry fetch when the caller did not supply an external
/// receipt.  The resulting record is validated against the fetched object and
/// is consumed by the historical guard; the remote-tracking ref is only used
/// to locate that object after this command succeeds.
pub fn refresh_origin_v3(root: &Path) -> Result<FreshFetchEvidence> {
    let started_at_unix = now_unix();
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["fetch", "--no-tags", "origin", "v3"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| DeliveryError::environment("cannot run the origin/v3 fetch"))?;
    let completed_at_unix = now_unix();
    let stdout_sha256 = sha256_bytes(&output.stdout);
    let stderr_sha256 = sha256_bytes(&output.stderr);
    if !output.status.success() {
        return Err(DeliveryError::environment(
            "the origin/v3 fetch did not complete successfully",
        ));
    }
    let fetched_oid = git_resolve_commit(root, "refs/remotes/origin/v3")?;
    let evidence = FreshFetchEvidence {
        artifact_kind: FRESH_FETCH_ARTIFACT_KIND.to_owned(),
        schema_version: FRESH_FETCH_SCHEMA_VERSION,
        repository: "github.com/vicondoa/d2b".to_owned(),
        remote: "origin".to_owned(),
        ref_name: "v3".to_owned(),
        fetched_oid: fetched_oid.clone(),
        command_id: "git-fetch-origin-v3".to_owned(),
        argv: ["git", "fetch", "--no-tags", "origin", "v3"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        started_at_unix,
        completed_at_unix,
        exit_code: output.status.code().unwrap_or(1),
        result: CommandResult::Passed,
        stdout_sha256,
        stderr_sha256,
        output_bytes: (output.stdout.len() + output.stderr.len()) as u64,
        fetched_at_unix: completed_at_unix,
        before_oid: None,
    };
    evidence.validate(root, &fetched_oid)?;
    Ok(evidence)
}

impl W6Paths {
    pub fn from_environment(repository_roots: &BTreeMap<String, PathBuf>) -> Result<Self> {
        let (ledger, command_evidence_dir) = Self::entry_from_environment(repository_roots)?;
        let plan_approval =
            validate_external_file(&required_env_path(PLAN_APPROVAL_ENV)?, repository_roots)?;
        Ok(Self {
            ledger,
            command_evidence_dir,
            plan_approval,
        })
    }

    fn entry_from_environment(
        repository_roots: &BTreeMap<String, PathBuf>,
    ) -> Result<(PathBuf, PathBuf)> {
        let ledger =
            validate_external_file(&required_env_path(DISPATCH_LEDGER_ENV)?, repository_roots)?;
        let command_evidence_dir = absolute_path(&required_env_path(COMMAND_EVIDENCE_ENV)?)?;
        let roots = repository_roots.values().cloned().collect::<Vec<_>>();
        ensure_external_path(&command_evidence_dir, &roots)?;
        if let Ok(metadata) = fs::symlink_metadata(&command_evidence_dir)
            && metadata.file_type().is_symlink()
        {
            return Err(DeliveryError::new(
                "external command evidence directory must not be a symlink",
            ));
        }
        Ok((ledger, command_evidence_dir))
    }

    pub fn fetch_evidence_from_environment(
        repository_roots: &BTreeMap<String, PathBuf>,
    ) -> Result<Option<PathBuf>> {
        let Some(value) = std::env::var_os(FRESH_FETCH_ENV) else {
            return Ok(None);
        };
        let path = validate_external_file(&PathBuf::from(value), repository_roots)?;
        Ok(Some(path))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum DispatchState {
    #[serde(rename = "NotLaunched")]
    NotLaunched,
    #[serde(rename = "Dispatched")]
    Dispatched,
    #[serde(rename = "Validated")]
    Validated,
    #[serde(rename = "Completed")]
    Completed,
    #[serde(rename = "Blocked")]
    Blocked,
}

impl DispatchState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotLaunched => "NotLaunched",
            Self::Dispatched => "Dispatched",
            Self::Validated => "Validated",
            Self::Completed => "Completed",
            Self::Blocked => "Blocked",
        }
    }

    fn is_done(self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum LocalTaskState {
    #[serde(rename = "Planned")]
    Planned,
    #[serde(rename = "Dispatched")]
    Dispatched,
    #[serde(rename = "Validated")]
    Validated,
    #[serde(rename = "Merged")]
    Merged,
}

impl LocalTaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "Planned",
            Self::Dispatched => "Dispatched",
            Self::Validated => "Validated",
            Self::Merged => "Merged",
        }
    }
}

pub fn transition_local_task_state(current: LocalTaskState, next: LocalTaskState) -> Result<()> {
    let allowed = match current {
        LocalTaskState::Planned => next == LocalTaskState::Dispatched,
        LocalTaskState::Dispatched => next == LocalTaskState::Validated,
        LocalTaskState::Validated => {
            next == LocalTaskState::Dispatched || next == LocalTaskState::Merged
        }
        LocalTaskState::Merged => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(DeliveryError::new(format!(
            "local task cannot transition from {} to {}",
            current.as_str(),
            next.as_str()
        )))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DispatchEntry {
    pub group: String,
    pub state: DispatchState,
    pub candidate_id: String,
    pub head_oid: String,
    pub dispatch_id: Option<String>,
    pub updated_at_unix: u64,
    pub completion_evidence_ids: Vec<String>,
    #[serde(default)]
    pub blocker: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DispatchLedger {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub entries: Vec<DispatchEntry>,
}

impl DispatchLedger {
    pub fn validate(&self) -> Result<()> {
        if self.artifact_kind != DISPATCH_LEDGER_ARTIFACT_KIND {
            return Err(DeliveryError::new(format!(
                "dispatch ledger artifact kind must be {DISPATCH_LEDGER_ARTIFACT_KIND:?}"
            )));
        }
        if self.schema_version != DISPATCH_LEDGER_SCHEMA_VERSION {
            return Err(DeliveryError::new(format!(
                "dispatch ledger schema version must be {DISPATCH_LEDGER_SCHEMA_VERSION}"
            )));
        }
        if self.entries.len() != W6_GROUPS.len() {
            return Err(DeliveryError::new(format!(
                "dispatch ledger must contain exactly {} group records",
                W6_GROUPS.len()
            )));
        }
        let expected = W6_GROUPS.into_iter().collect::<BTreeSet<_>>();
        let expected_order = W6_GROUPS;
        let mut actual = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            validate_bounded_string(&entry.group, "dispatch ledger group")?;
            if entry.group.chars().any(char::is_control) {
                return Err(DeliveryError::new(
                    "dispatch ledger group must not contain control characters",
                ));
            }
            if !expected.contains(entry.group.as_str()) {
                return Err(DeliveryError::new(format!(
                    "dispatch ledger contains unknown group {}",
                    entry.group
                )));
            }
            if !actual.insert(entry.group.as_str()) {
                return Err(DeliveryError::new(format!(
                    "dispatch ledger repeats group {}",
                    entry.group
                )));
            }
            if entry.group != expected_order[index] {
                return Err(DeliveryError::new(
                    "dispatch ledger entries must be in canonical group order",
                ));
            }
            validate_hash(&entry.head_oid, "dispatch ledger head OID")?;
            CandidateId::parse(entry.candidate_id.as_str())?;
            if let Some(dispatch_id) = &entry.dispatch_id {
                validate_identifier(dispatch_id, "dispatch identifier")?;
            }
            for evidence_id in &entry.completion_evidence_ids {
                validate_identifier(evidence_id, "completion evidence identifier")?;
            }
            if entry
                .completion_evidence_ids
                .windows(2)
                .any(|window| window[0] >= window[1])
            {
                return Err(DeliveryError::new(format!(
                    "completion evidence for group {} must be sorted and unique",
                    entry.group
                )));
            }
            match entry.state {
                DispatchState::NotLaunched => {
                    if entry.dispatch_id.is_some()
                        || !entry.completion_evidence_ids.is_empty()
                        || entry.blocker.is_some()
                    {
                        return Err(DeliveryError::new(format!(
                            "NotLaunched group {} must have no dispatch, completion, or blocker record",
                            entry.group
                        )));
                    }
                }
                DispatchState::Blocked => {
                    let blocker = entry.blocker.as_deref().ok_or_else(|| {
                        DeliveryError::new(format!(
                            "Blocked group {} must carry a durable blocker",
                            entry.group
                        ))
                    })?;
                    validate_bounded_string(blocker, "dispatch blocker")?;
                }
                DispatchState::Dispatched | DispatchState::Validated | DispatchState::Completed => {
                    if entry.dispatch_id.is_none() {
                        return Err(DeliveryError::new(format!(
                            "{} group {} must carry a dispatch identifier",
                            entry.state.as_str(),
                            entry.group
                        )));
                    }
                }
            }
        }
        if actual != expected {
            return Err(DeliveryError::new(
                "dispatch ledger group set does not match the closed Wave 6 census",
            ));
        }
        Ok(())
    }

    pub fn validate_for_candidate(&self, candidate_id: &CandidateId, head_oid: &str) -> Result<()> {
        self.validate()?;
        validate_hash(head_oid, "candidate head OID")?;
        for entry in &self.entries {
            if entry.candidate_id != candidate_id.as_str() {
                return Err(DeliveryError::new(format!(
                    "dispatch ledger group {} is bound to a different candidate",
                    entry.group
                )));
            }
            if entry.head_oid != head_oid {
                return Err(DeliveryError::new(format!(
                    "dispatch ledger group {} is bound to a different head",
                    entry.group
                )));
            }
        }
        Ok(())
    }

    pub fn entry(&self, group: &str) -> Result<&DispatchEntry> {
        self.entries
            .iter()
            .find(|entry| entry.group == group)
            .ok_or_else(|| DeliveryError::new(format!("dispatch ledger has no group {group}")))
    }

    fn entry_mut(&mut self, group: &str) -> Result<&mut DispatchEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.group == group)
            .ok_or_else(|| DeliveryError::new(format!("dispatch ledger has no group {group}")))
    }

    pub fn launched_groups(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.state != DispatchState::NotLaunched)
            .map(|entry| entry.group.clone())
            .collect()
    }

    pub fn material_digest(&self) -> Result<String> {
        #[derive(Serialize)]
        struct MaterialEntry<'a> {
            group: &'a str,
            candidate_id: &'a str,
            head_oid: &'a str,
        }
        let mut entries = self
            .entries
            .iter()
            .map(|entry| MaterialEntry {
                group: &entry.group,
                candidate_id: &entry.candidate_id,
                head_oid: &entry.head_oid,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.group.cmp(right.group));
        canonical_digest(b"d2b-feature-local-dispatch-material-v1\0", &entries)
    }

    pub fn ready_groups(&self, plan_approved: bool) -> Result<Vec<String>> {
        self.ready_groups_with_graph(plan_approved, &BTreeMap::new())
    }

    pub fn ready_groups_for_checkout(
        &self,
        plan_approved: bool,
        repository_root: &Path,
    ) -> Result<Vec<String>> {
        let prerequisites = graph_group_prerequisites(repository_root)?;
        self.ready_groups_with_graph(plan_approved, &prerequisites)
    }

    pub fn ready_groups_with_graph(
        &self,
        plan_approved: bool,
        graph_prerequisites: &BTreeMap<String, BTreeSet<String>>,
    ) -> Result<Vec<String>> {
        self.validate()?;
        if !plan_approved {
            return Ok(Vec::new());
        }

        let complete = |group: &str| {
            self.entry(group)
                .map(|entry| matches!(entry.state, DispatchState::Completed))
                .unwrap_or(false)
        };
        let mut ready = Vec::new();
        for group in W6_GROUPS {
            let entry = self.entry(group)?;
            if entry.state != DispatchState::NotLaunched {
                continue;
            }

            let is_ready = if group == "feature-local:w6-shared-prep" {
                true
            } else if group == "feature-local:w6-core-control-foundations"
                || group == "feature-local:w6-storage-authority-foundations"
                || group == "feature-local:w6-audit-telemetry-foundations"
            {
                complete("feature-local:w6-shared-prep")
            } else if group == "feature-local:w6-operator-acceptance" {
                complete("feature-local:w6-core-control-foundations")
                    && complete("feature-local:w6-storage-authority-foundations")
                    && complete("feature-local:w6-audit-telemetry-foundations")
            } else if group == "feature-local:w6-converge" {
                complete("feature-local:w6-operator-acceptance")
                    && W6_MANIFEST_GROUPS.into_iter().all(complete)
            } else if group == "feature-local:w6-close" {
                complete("feature-local:w6-converge")
            } else {
                let foundations = [
                    "feature-local:w6-shared-prep",
                    "feature-local:w6-core-control-foundations",
                    "feature-local:w6-storage-authority-foundations",
                    "feature-local:w6-audit-telemetry-foundations",
                ];
                foundations.into_iter().all(complete)
                    && graph_prerequisites
                        .get(group)
                        .into_iter()
                        .flatten()
                        .all(|dependency| complete(dependency))
            };
            if is_ready {
                ready.push(group.to_owned());
            }
        }
        Ok(ready)
    }

    pub fn require_pre_t221_state(&self) -> Result<()> {
        self.validate()?;
        if let Some(group) = self
            .entries
            .iter()
            .find(|entry| entry.state != DispatchState::NotLaunched)
        {
            return Err(DeliveryError::new(format!(
                "Wave 6 cannot enter T221: dispatch ledger already launched group {}",
                group.group
            )));
        }
        Ok(())
    }
}

pub fn graph_group_prerequisites(
    repository_root: &Path,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let graph: Value = serde_json::from_slice(
        &fs::read(repository_root.join(GRAPH_PATH))
            .map_err(|_| DeliveryError::environment("cannot read the implementation graph"))?,
    )?;
    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new("implementation graph has no nodes"))?;
    let edges = graph
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new("implementation graph has no edges"))?;
    let mut groups = BTreeMap::new();
    for node in nodes {
        if node.get("kind").and_then(Value::as_str) != Some("work-item")
            || node.get("wave").and_then(Value::as_str) != Some("W6")
        {
            continue;
        }
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DeliveryError::new("Wave 6 graph node has no id"))?;
        let group = node
            .get("parallelGroup")
            .and_then(Value::as_str)
            .ok_or_else(|| DeliveryError::new("Wave 6 graph node has no parallel group"))?;
        groups.insert(id.to_owned(), group.to_owned());
    }
    let mut prerequisites = BTreeMap::<String, BTreeSet<String>>::new();
    for edge in edges {
        let from = edge.get("from").and_then(Value::as_str);
        let to = edge.get("to").and_then(Value::as_str);
        let (Some(from), Some(to)) = (from, to) else {
            return Err(DeliveryError::new(
                "implementation graph edge has no endpoints",
            ));
        };
        let (Some(from_group), Some(to_group)) = (groups.get(from), groups.get(to)) else {
            continue;
        };
        if from_group != to_group {
            prerequisites
                .entry(to_group.clone())
                .or_default()
                .insert(from_group.clone());
        }
    }
    Ok(prerequisites)
}

pub fn initial_ledger(candidate_id: &CandidateId, head_oid: &str) -> Result<DispatchLedger> {
    validate_hash(head_oid, "dispatch ledger head OID")?;
    let entries = W6_GROUPS
        .into_iter()
        .map(|group| DispatchEntry {
            group: group.to_owned(),
            state: DispatchState::NotLaunched,
            candidate_id: candidate_id.as_str().to_owned(),
            head_oid: head_oid.to_owned(),
            dispatch_id: None,
            updated_at_unix: 0,
            completion_evidence_ids: Vec::new(),
            blocker: None,
        })
        .collect();
    let ledger = DispatchLedger {
        artifact_kind: DISPATCH_LEDGER_ARTIFACT_KIND.to_owned(),
        schema_version: DISPATCH_LEDGER_SCHEMA_VERSION,
        entries,
    };
    ledger.validate()?;
    Ok(ledger)
}

pub fn read_dispatch_ledger(
    path: &Path,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<DispatchLedger> {
    let path = validate_external_file(path, repository_roots)?;
    let bytes = read_external_json(&path, "dispatch ledger")?;
    let ledger: DispatchLedger = serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid dispatch ledger: {error}")))?;
    ledger.validate()?;
    Ok(ledger)
}

/// Create the immutable group-address material once, or compare it with an
/// existing ledger.  Status, dispatch identifiers, evidence locators, and
/// timestamps are never overwritten by this operation.
pub fn create_or_compare_ledger(
    path: &Path,
    candidate_id: &CandidateId,
    head_oid: &str,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<DispatchLedger> {
    let path = validate_external_file(path, repository_roots)?;
    if path.exists() {
        let ledger = read_dispatch_ledger(&path, repository_roots)?;
        ledger.validate_for_candidate(candidate_id, head_oid)?;
        return Ok(ledger);
    }
    let ledger = initial_ledger(candidate_id, head_oid)?;
    write_external_json_create(&path, &ledger)?;
    Ok(ledger)
}

pub fn transition_local_state(current: DispatchState, next: DispatchState) -> Result<()> {
    let allowed = match current {
        DispatchState::NotLaunched => next == DispatchState::Dispatched,
        DispatchState::Dispatched => next == DispatchState::Validated,
        DispatchState::Validated => {
            next == DispatchState::Dispatched || next == DispatchState::Completed
        }
        DispatchState::Completed | DispatchState::Blocked => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(DeliveryError::new(format!(
            "local delivery state cannot transition from {} to {}",
            current.as_str(),
            next.as_str()
        )))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_group(
    path: &Path,
    material: &CandidateMaterial,
    group: &str,
    next: DispatchState,
    dispatch_id: Option<&str>,
    completion_evidence_ids: &[String],
    blocker: Option<&str>,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<DispatchLedger> {
    if !is_wave6(material) {
        return Err(DeliveryError::new(
            "dispatch ledger updates are only available for Wave 6",
        ));
    }
    let digests = material.digests()?;
    let head = material
        .repository_set
        .first()
        .map(|repository| repository.head_oid.as_str())
        .ok_or_else(|| DeliveryError::new("Wave 6 material has no repository head"))?;
    let mut ledger = read_dispatch_ledger(path, repository_roots)?;
    ledger.validate_for_candidate(&digests.candidate_id, head)?;
    let existing = ledger.entry(group)?.clone();
    if existing.state == next
        && existing.dispatch_id.as_deref() == dispatch_id
        && existing.completion_evidence_ids == completion_evidence_ids
        && existing.blocker.as_deref() == blocker
    {
        return Ok(ledger);
    }
    if next != DispatchState::Blocked {
        if existing.state == DispatchState::Blocked {
            return Err(DeliveryError::new(
                "a Blocked group requires replacement plan approval before it can resume",
            ));
        }
        transition_local_state(existing.state, next)?;
    }
    if next == DispatchState::Dispatched && dispatch_id.is_none() {
        return Err(DeliveryError::usage(
            "a Dispatched group requires a dispatch identifier",
        ));
    }
    if next == DispatchState::Completed && completion_evidence_ids.is_empty() {
        return Err(DeliveryError::new(
            "a Completed group requires completion evidence identifiers",
        ));
    }
    if next == DispatchState::Blocked && blocker.is_none() {
        return Err(DeliveryError::usage(
            "a Blocked group requires a blocker description",
        ));
    }
    let entry = ledger.entry_mut(group)?;
    entry.state = next;
    entry.dispatch_id = dispatch_id.map(str::to_owned);
    entry.completion_evidence_ids = completion_evidence_ids.to_vec();
    entry.blocker = blocker.map(str::to_owned);
    entry.updated_at_unix = now_unix();
    ledger.validate_for_candidate(&digests.candidate_id, head)?;
    write_external_json_replace(path, &ledger, repository_roots)?;
    Ok(ledger)
}

pub fn block_group(
    path: &Path,
    material: &CandidateMaterial,
    group: &str,
    blocker: &str,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<DispatchLedger> {
    validate_bounded_string(blocker, "dispatch blocker")?;
    let digests = material.digests()?;
    let head = material
        .repository_set
        .first()
        .map(|repository| repository.head_oid.as_str())
        .ok_or_else(|| DeliveryError::new("Wave 6 material has no repository head"))?;
    let ledger = read_dispatch_ledger(path, repository_roots)?;
    ledger.validate_for_candidate(&digests.candidate_id, head)?;
    if ledger.entry(group)?.state == DispatchState::Completed {
        return Err(DeliveryError::new(
            "a Completed group cannot be replaced by a blocker",
        ));
    }
    update_group(
        path,
        material,
        group,
        DispatchState::Blocked,
        ledger.entry(group)?.dispatch_id.as_deref(),
        &ledger.entry(group)?.completion_evidence_ids,
        Some(blocker),
        repository_roots,
    )
}

pub fn dispatch_group(
    path: &Path,
    material: &CandidateMaterial,
    group: &str,
    dispatch_id: &str,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<DispatchLedger> {
    validate_identifier(dispatch_id, "dispatch identifier")?;
    let digests = material.digests()?;
    let head = material
        .repository_set
        .first()
        .map(|repository| repository.head_oid.as_str())
        .ok_or_else(|| DeliveryError::new("Wave 6 material has no repository head"))?;
    let ledger = read_dispatch_ledger(path, repository_roots)?;
    ledger.validate_for_candidate(&digests.candidate_id, head)?;
    ledger.require_pre_t221_state()?;
    require_plan_receipt(material, repository_roots, None, None)?;
    let ready = repository_roots
        .get("github.com/vicondoa/d2b")
        .map(|root| ledger.ready_groups_for_checkout(true, root))
        .transpose()?
        .unwrap_or(ledger.ready_groups(true)?);
    if ready != vec!["feature-local:w6-shared-prep".to_owned()] {
        return Err(DeliveryError::new(format!(
            "first Wave 6 dispatch is not ready; computed ready groups are {ready:?}"
        )));
    }
    if group != ready[0] {
        return Err(DeliveryError::new(format!(
            "group {group} is blocked until the computed ready group {} is dispatched",
            ready[0]
        )));
    }
    update_group(
        path,
        material,
        group,
        DispatchState::Dispatched,
        Some(dispatch_id),
        &[],
        None,
        repository_roots,
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandResult {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandEvidenceRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub command_id: String,
    pub argv: Vec<String>,
    pub working_tree_oid: String,
    pub started_at_unix: u64,
    pub completed_at_unix: u64,
    pub exit_code: i32,
    pub result: CommandResult,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub output_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_tests: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_tests: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_matches: Option<u64>,
}

impl CommandEvidenceRecord {
    pub fn validate(&self) -> Result<()> {
        if self.artifact_kind != COMMAND_EVIDENCE_ARTIFACT_KIND {
            return Err(DeliveryError::new(
                "command evidence has an unexpected artifact kind",
            ));
        }
        if self.schema_version != COMMAND_EVIDENCE_SCHEMA_VERSION {
            return Err(DeliveryError::new(
                "command evidence has an unsupported schema version",
            ));
        }
        validate_identifier(&self.command_id, "command evidence identity")?;
        if self.argv.is_empty() {
            return Err(DeliveryError::new(
                "command evidence argv must not be empty",
            ));
        }
        for argument in &self.argv {
            validate_bounded_string(argument, "command evidence argv")?;
            if argument.chars().any(char::is_control) {
                return Err(DeliveryError::new(
                    "command evidence argv must not contain control characters",
                ));
            }
        }
        validate_hash(&self.working_tree_oid, "command evidence working-tree OID")?;
        if self.completed_at_unix < self.started_at_unix {
            return Err(DeliveryError::new(
                "command evidence completion time precedes its start time",
            ));
        }
        validate_sha256(&self.stdout_sha256, "command evidence stdout digest")?;
        validate_sha256(&self.stderr_sha256, "command evidence stderr digest")?;
        let passed = self.result == CommandResult::Passed;
        if passed != (self.exit_code == 0) {
            return Err(DeliveryError::new(
                "command evidence result and exit status disagree",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEvidenceSet {
    pub records: Vec<CommandEvidenceRecord>,
    digest: String,
}

impl CommandEvidenceSet {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn record(&self, command_id: &str) -> Result<&CommandEvidenceRecord> {
        self.records
            .iter()
            .find(|record| record.command_id == command_id)
            .ok_or_else(|| DeliveryError::new(format!("command evidence is missing {command_id}")))
    }

    pub fn validate_t221(&self, working_tree_oid: &str) -> Result<()> {
        validate_hash(working_tree_oid, "T221 working-tree OID")?;
        let actual = self
            .records
            .iter()
            .map(|record| record.command_id.as_str())
            .collect::<BTreeSet<_>>();
        let expected = REQUIRED_T221_COMMANDS.into_iter().collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(DeliveryError::new(
                "T221 command evidence must contain exactly the closed required command set",
            ));
        }
        for command_id in REQUIRED_T221_COMMANDS {
            let record = self.record(command_id)?;
            if record.result != CommandResult::Passed
                || record.exit_code != 0
                || record.working_tree_oid != working_tree_oid
            {
                return Err(DeliveryError::new(format!(
                    "T221 command evidence {command_id} did not pass against the entry head"
                )));
            }
        }
        let list = self.record("focused-guard-list")?;
        if list.discovered_tests.unwrap_or(0) == 0 {
            return Err(DeliveryError::new(
                "focused guard command evidence discovered no tests",
            ));
        }
        if self
            .record("focused-guard-ignored-list")?
            .ignored_tests
            .unwrap_or(1)
            != 0
        {
            return Err(DeliveryError::new(
                "focused guard command evidence reports ignored tests",
            ));
        }
        if self.record("focused-guard-run")?.skip_matches.unwrap_or(1) != 0 {
            return Err(DeliveryError::new(
                "focused guard command evidence reports skipped results",
            ));
        }
        Ok(())
    }
}

pub fn read_command_evidence(
    directory: &Path,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<CommandEvidenceSet> {
    let directory = absolute_path(directory)?;
    let roots = repository_roots.values().cloned().collect::<Vec<_>>();
    ensure_external_path(&directory, &roots)?;
    let entries = fs::read_dir(&directory)
        .map_err(|_| DeliveryError::environment("cannot read command evidence directory"))?;
    let mut records = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|_| DeliveryError::environment("cannot read command evidence entry"))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            return Err(DeliveryError::new(
                "command evidence directory contains a non-JSON entry",
            ));
        }
        let bytes = read_external_json(&path, "command evidence")?;
        let record: CommandEvidenceRecord = serde_json::from_slice(&bytes)
            .map_err(|error| DeliveryError::new(format!("invalid command evidence: {error}")))?;
        record.validate()?;
        records.push(record);
    }
    records.sort_by(|left, right| left.command_id.cmp(&right.command_id));
    let mut ids = BTreeSet::new();
    if records
        .iter()
        .any(|record| !ids.insert(record.command_id.as_str()))
    {
        return Err(DeliveryError::new(
            "command evidence repeats a command identity",
        ));
    }
    let digest = canonical_digest(b"d2b-feature-local-command-evidence-set-v1\0", &records)?;
    Ok(CommandEvidenceSet { records, digest })
}

pub fn write_command_evidence(
    directory: &Path,
    record: &CommandEvidenceRecord,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    record.validate()?;
    let directory = absolute_path(directory)?;
    let roots = repository_roots.values().cloned().collect::<Vec<_>>();
    ensure_external_path(&directory, &roots)?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{}.json", record.command_id));
    let bytes = serde_json::to_vec(record)?;
    if path.exists() {
        let existing = read_external_json(&path, "command evidence")?;
        if existing != bytes {
            return Err(DeliveryError::new(
                "command evidence identity already exists with different bytes",
            ));
        }
        return Ok(());
    }
    write_external_json_create(&path, record)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanApprovalReceipt {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub entry_base_oid: String,
    pub feature_plan_material_sha256: String,
    pub entry_candidate_id: CandidateId,
    pub entry_content_id: super::model::ContentId,
    pub entry_snapshot_sha256: SnapshotSha256,
    pub selection_sha256: String,
    pub dispatch_ledger_sha256: String,
    pub command_evidence_set_sha256: String,
    pub selected_roster: Vec<String>,
    pub signoff_count: u32,
    pub recommendation_count: u32,
    pub result: String,
    pub durable_write_evidence_sha256: String,
    pub approved_at_unix: u64,
    #[serde(rename = "lifecycleApproval")]
    pub lifecycle_approval: Value,
    #[serde(rename = "seatRecords")]
    pub seat_records: BTreeMap<String, Value>,
}

impl PlanApprovalReceipt {
    pub fn validate_shape(&self) -> Result<()> {
        if self.artifact_kind != PLAN_APPROVAL_ARTIFACT_KIND {
            return Err(DeliveryError::new(
                "plan approval receipt has an unexpected artifact kind",
            ));
        }
        if self.schema_version != PLAN_APPROVAL_SCHEMA_VERSION {
            return Err(DeliveryError::new(
                "plan approval receipt has an unsupported schema version",
            ));
        }
        if self.program != "ADR046" || self.wave != "adr046w6" {
            return Err(DeliveryError::new(
                "plan approval receipt is not for ADR046 Wave 6",
            ));
        }
        validate_hash(&self.entry_base_oid, "plan approval entry base")?;
        for (value, label) in [
            (
                &self.feature_plan_material_sha256,
                "feature plan material digest",
            ),
            (&self.selection_sha256, "selection digest"),
            (&self.dispatch_ledger_sha256, "dispatch ledger digest"),
            (
                &self.command_evidence_set_sha256,
                "command evidence set digest",
            ),
            (
                &self.durable_write_evidence_sha256,
                "durable write evidence digest",
            ),
        ] {
            validate_sha256(value, label)?;
        }
        if self.selected_roster.is_empty() {
            return Err(DeliveryError::new(
                "plan approval receipt selected roster must not be empty",
            ));
        }
        let current = PANEL_CURRENT_ROLES
            .into_iter()
            .map(PanelRole::as_str)
            .collect::<BTreeSet<_>>();
        let mut roster = BTreeSet::new();
        for seat in &self.selected_roster {
            if !current.contains(seat.as_str()) || !roster.insert(seat.as_str()) {
                return Err(DeliveryError::new(
                    "plan approval receipt selected roster is not a unique current roster",
                ));
            }
        }
        if self.signoff_count != self.selected_roster.len() as u32 {
            return Err(DeliveryError::new(
                "plan approval signoff count does not equal the selected roster",
            ));
        }
        if self.recommendation_count != 0 || self.result != "approved" {
            return Err(DeliveryError::new(
                "plan approval receipt is not unanimous and recommendation-free",
            ));
        }
        let lifecycle_approved = self
            .lifecycle_approval
            .as_bool()
            .or_else(|| {
                self.lifecycle_approval
                    .get("approved")
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false);
        if !lifecycle_approved {
            return Err(DeliveryError::new(
                "plan approval receipt does not carry lifecycle approval",
            ));
        }
        let seats = self.seat_records.keys().collect::<BTreeSet<_>>();
        let expected = self.selected_roster.iter().collect::<BTreeSet<_>>();
        if seats != expected {
            return Err(DeliveryError::new(
                "plan approval receipt per-seat records do not match the selected roster",
            ));
        }
        for seat in &self.selected_roster {
            let value = self
                .seat_records
                .get(seat)
                .expect("seat record keys were checked");
            if value.get("signoff").and_then(Value::as_bool) != Some(true) {
                return Err(DeliveryError::new(format!(
                    "plan approval receipt seat {seat} is not signed off"
                )));
            }
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        material: &CandidateMaterial,
        selection: Option<&PanelSelectionV1>,
        ledger: &DispatchLedger,
        command_evidence: &CommandEvidenceSet,
        feature_dir: &Path,
        panel_roles: Option<&[PanelRole]>,
    ) -> Result<()> {
        self.validate_for_with_selection_bytes(
            material,
            selection,
            None,
            ledger,
            command_evidence,
            feature_dir,
            panel_roles,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn validate_for_with_selection_bytes(
        &self,
        material: &CandidateMaterial,
        selection: Option<&PanelSelectionV1>,
        selection_bytes: Option<&[u8]>,
        ledger: &DispatchLedger,
        command_evidence: &CommandEvidenceSet,
        feature_dir: &Path,
        panel_roles: Option<&[PanelRole]>,
    ) -> Result<()> {
        self.validate_shape()?;
        let digests = material.digests()?;
        let base = material
            .repository_set
            .first()
            .ok_or_else(|| DeliveryError::new("plan approval material has no repository"))?
            .base_oid
            .as_str();
        if self.entry_base_oid != base
            || self.entry_candidate_id != digests.candidate_id
            || self.entry_content_id != digests.content_id
            || self.entry_snapshot_sha256 != digests.snapshot_sha256
        {
            return Err(DeliveryError::new(
                "plan approval receipt is bound to a different entry base or production snapshot",
            ));
        }
        if let Some(selection) = selection {
            selection.validate_for_snapshot(
                material.program.as_str(),
                material.wave.as_str(),
                &digests,
            )?;
            let selection_digest = match selection_bytes {
                Some(bytes) => sha256_bytes(bytes),
                None => sha256_bytes(&serde_json::to_vec(selection)?),
            };
            if selection_digest != self.selection_sha256 {
                return Err(DeliveryError::new(
                    "plan approval receipt selection digest does not match the lifecycle selection",
                ));
            }
            let roster = selection
                .roster
                .iter()
                .map(|role| role.as_str().to_owned())
                .collect::<Vec<_>>();
            if roster != self.selected_roster {
                return Err(DeliveryError::new(
                    "plan approval receipt selected roster differs from the lifecycle selection",
                ));
            }
        } else if let Some(panel_roles) = panel_roles {
            let roster = panel_roles
                .iter()
                .map(|role| role.as_str().to_owned())
                .collect::<Vec<_>>();
            if roster != self.selected_roster {
                return Err(DeliveryError::new(
                    "plan approval receipt selected roster differs from the panel request",
                ));
            }
        }
        if self.dispatch_ledger_sha256 != ledger.material_digest()? {
            return Err(DeliveryError::new(
                "plan approval receipt dispatch ledger material has changed",
            ));
        }
        if self.command_evidence_set_sha256 != command_evidence.digest() {
            return Err(DeliveryError::new(
                "plan approval receipt command evidence set has changed",
            ));
        }
        let expected_feature = feature_plan_material_digest(feature_dir)?;
        if self.feature_plan_material_sha256 != expected_feature {
            return Err(DeliveryError::new(
                "plan approval receipt feature material is stale; status-only checkbox updates \
                 are excluded, but requirement and guard changes invalidate approval",
            ));
        }
        Ok(())
    }
}

pub fn read_plan_approval(
    path: &Path,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<PlanApprovalReceipt> {
    let path = validate_external_file(path, repository_roots)?;
    let bytes = read_external_json(&path, "plan approval receipt")?;
    let receipt: PlanApprovalReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid plan approval receipt: {error}")))?;
    receipt.validate_shape()?;
    Ok(receipt)
}

pub fn write_plan_approval(
    path: &Path,
    receipt: &PlanApprovalReceipt,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    receipt.validate_shape()?;
    let path = validate_external_file(path, repository_roots)?;
    write_external_json_replace(&path, receipt, repository_roots)
}

pub fn feature_plan_material_digest(feature_dir: &Path) -> Result<String> {
    let feature_dir = absolute_path(feature_dir)?;
    let mut files = Vec::new();
    collect_files(&feature_dir, &feature_dir, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut material = Vec::new();
    for (relative, path) in files {
        let bytes = fs::read(&path)
            .map_err(|_| DeliveryError::environment("cannot read feature plan material"))?;
        let normalized = normalize_status_only_updates(&bytes);
        material.push((relative, normalized));
    }
    canonical_digest(b"d2b-feature-local-plan-material-v1\0", &material)
}

fn collect_files(root: &Path, directory: &Path, output: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let entries = fs::read_dir(directory)
        .map_err(|_| DeliveryError::environment("cannot enumerate feature plan material"))?;
    for entry in entries {
        let entry = entry
            .map_err(|_| DeliveryError::environment("cannot enumerate feature plan material"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| DeliveryError::environment("cannot inspect feature plan material"))?;
        if metadata.file_type().is_symlink() {
            return Err(DeliveryError::new(
                "feature plan material must not contain symlinks",
            ));
        }
        if metadata.is_dir() {
            collect_files(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| DeliveryError::environment("feature plan material escaped its root"))?
                .to_str()
                .ok_or_else(|| DeliveryError::new("feature plan material path is not UTF-8"))?
                .to_owned();
            output.push((relative, path));
        }
    }
    Ok(())
}

fn normalize_status_only_updates(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    text.lines()
        .map(|line| {
            let mut normalized = line.to_owned();
            for marker in ["- [ ]", "- [x]", "- [X]"] {
                normalized = normalized.replace(marker, "- [?]");
            }
            normalized
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

pub fn require_entry_receipts(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<(DispatchLedger, CommandEvidenceSet)> {
    let (ledger_path, command_evidence_dir) = W6Paths::entry_from_environment(repository_roots)?;
    let digests = material.digests()?;
    let head = material
        .repository_set
        .first()
        .ok_or_else(|| DeliveryError::new("Wave 6 material has no repository"))?
        .head_oid
        .as_str()
        .to_owned();
    let ledger =
        create_or_compare_ledger(&ledger_path, &digests.candidate_id, &head, repository_roots)?;
    ledger.require_pre_t221_state()?;
    if ledger.ready_groups(true)? != vec!["feature-local:w6-shared-prep".to_owned()] {
        return Err(DeliveryError::new(
            "T221 entry requires T606 to be the only first-ready local group",
        ));
    }
    let evidence = read_command_evidence(&command_evidence_dir, repository_roots)?;
    evidence.validate_t221(&head)?;
    validate_w6_census(repository_roots)?;
    let feature_root = feature_root(repository_roots)?;
    let _ = feature_plan_material_digest(&feature_root)?;
    Ok((ledger, evidence))
}

pub fn require_plan_receipt(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
    selection: Option<&PanelSelectionV1>,
    panel_roles: Option<&[PanelRole]>,
) -> Result<PlanApprovalReceipt> {
    require_plan_receipt_with_selection_bytes(
        material,
        repository_roots,
        selection,
        None,
        panel_roles,
    )
}

pub fn require_plan_receipt_with_selection_bytes(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
    selection: Option<&PanelSelectionV1>,
    selection_bytes: Option<&[u8]>,
    panel_roles: Option<&[PanelRole]>,
) -> Result<PlanApprovalReceipt> {
    let paths = W6Paths::from_environment(repository_roots)?;
    let ledger = read_dispatch_ledger(&paths.ledger, repository_roots)?;
    let head = material
        .repository_set
        .first()
        .ok_or_else(|| DeliveryError::new("Wave 6 material has no repository"))?
        .head_oid
        .as_str()
        .to_owned();
    let digests = material.digests()?;
    ledger.validate_for_candidate(&digests.candidate_id, &head)?;
    let evidence = read_command_evidence(&paths.command_evidence_dir, repository_roots)?;
    evidence.validate_t221(&head)?;
    let receipt = read_plan_approval(&paths.plan_approval, repository_roots)?;
    let feature_root = feature_root(repository_roots)?;
    receipt.validate_for_with_selection_bytes(
        material,
        selection,
        selection_bytes,
        &ledger,
        &evidence,
        &feature_root,
        panel_roles,
    )?;
    Ok(receipt)
}

pub fn require_close_receipts(
    material: &CandidateMaterial,
    repository_roots: &BTreeMap<String, PathBuf>,
    panel_roles: Option<&[PanelRole]>,
    final_eligibility: bool,
) -> Result<DispatchLedger> {
    let paths = W6Paths::from_environment(repository_roots)?;
    let receipt = read_plan_approval(&paths.plan_approval, repository_roots)?;
    let ledger = read_dispatch_ledger(&paths.ledger, repository_roots)?;
    let head = ledger
        .entries
        .first()
        .ok_or_else(|| DeliveryError::new("dispatch ledger has no entries"))?
        .head_oid
        .as_str();
    ledger.validate_for_candidate(&receipt.entry_candidate_id, head)?;
    let evidence = read_command_evidence(&paths.command_evidence_dir, repository_roots)?;
    evidence.validate_t221(head)?;
    let feature_root = feature_root(repository_roots)?;
    receipt.validate_for_close(&ledger, &evidence, &feature_root, panel_roles)?;
    material.digests()?;
    for group in [
        "feature-local:w6-shared-prep",
        "feature-local:w6-core-control-foundations",
        "feature-local:w6-storage-authority-foundations",
        "feature-local:w6-audit-telemetry-foundations",
        "feature-local:w6-operator-acceptance",
        "feature-local:w6-converge",
    ] {
        let entry = ledger.entry(group)?;
        if !entry.state.is_done() {
            return Err(DeliveryError::new(format!(
                "Wave 6 close requires local group {group} to be Completed"
            )));
        }
        let required = LOCAL_COMPLETION_EVIDENCE
            .iter()
            .find(|(candidate, _)| *candidate == group)
            .map(|(_, evidence)| *evidence)
            .expect("closed local group has completion evidence");
        if !required.iter().all(|id| {
            entry
                .completion_evidence_ids
                .iter()
                .any(|actual| actual == id)
        }) {
            return Err(DeliveryError::new(format!(
                "Wave 6 local group {group} is Completed without every required completion evidence record"
            )));
        }
    }
    for group in W6_MANIFEST_GROUPS {
        let entry = ledger.entry(group)?;
        if entry.state != DispatchState::Completed {
            return Err(DeliveryError::new(format!(
                "Wave 6 seal requires manifest group {group} to be Completed"
            )));
        }
    }
    if final_eligibility {
        let entry = ledger.entry("feature-local:w6-close")?;
        if !entry.state.is_done() {
            return Err(DeliveryError::new(
                "Wave 6 eligibility requires the local close group to be Completed",
            ));
        }
        let required = LOCAL_COMPLETION_EVIDENCE
            .iter()
            .find(|(group, _)| *group == "feature-local:w6-close")
            .expect("close evidence");
        if !required.1.iter().all(|id| {
            entry
                .completion_evidence_ids
                .iter()
                .any(|actual| actual == id)
        }) {
            return Err(DeliveryError::new(
                "Wave 6 eligibility requires all local close evidence records",
            ));
        }
    }
    Ok(ledger)
}

fn feature_root(repository_roots: &BTreeMap<String, PathBuf>) -> Result<PathBuf> {
    let root = repository_roots
        .get("github.com/vicondoa/d2b")
        .or_else(|| repository_roots.values().next())
        .ok_or_else(|| DeliveryError::new("Wave 6 has no repository checkout"))?;
    let root = fs::canonicalize(root)
        .map_err(|_| DeliveryError::environment("cannot resolve the Wave 6 feature root"))?;
    let feature = root.join(FEATURE_DIR);
    if !feature.is_dir() {
        return Err(DeliveryError::new(
            "Wave 6 feature directory is missing from the entry tree",
        ));
    }
    Ok(feature)
}

fn validate_w6_census(repository_roots: &BTreeMap<String, PathBuf>) -> Result<()> {
    let root = repository_roots
        .get("github.com/vicondoa/d2b")
        .or_else(|| repository_roots.values().next())
        .ok_or_else(|| DeliveryError::new("Wave 6 has no repository checkout"))?;
    let graph: Value =
        serde_json::from_slice(&fs::read(root.join(GRAPH_PATH)).map_err(|_| {
            DeliveryError::environment("cannot read the Wave 6 implementation graph")
        })?)?;
    let work_items: Value =
        serde_json::from_slice(&fs::read(root.join(WORK_ITEMS_PATH)).map_err(|_| {
            DeliveryError::environment("cannot read the Wave 6 work-item manifest")
        })?)?;
    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new("Wave 6 implementation graph has no nodes"))?;
    let edges = graph
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new("Wave 6 implementation graph has no edges"))?;
    let w6 = nodes
        .iter()
        .filter(|node| node.get("kind").and_then(Value::as_str) == Some("work-item"))
        .filter(|node| node.get("wave").and_then(Value::as_str) == Some("W6"))
        .collect::<Vec<_>>();
    if nodes.len() != 600 || edges.len() != 1960 || w6.len() != 258 {
        return Err(DeliveryError::new(
            "Wave 6 pre-dispatch census does not match the committed graph",
        ));
    }
    let groups = w6
        .iter()
        .filter_map(|node| node.get("parallelGroup").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if groups != W6_MANIFEST_GROUPS.into_iter().collect::<BTreeSet<_>>() {
        return Err(DeliveryError::new(
            "Wave 6 graph group census does not match the closed foundation map",
        ));
    }
    let items = work_items
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| DeliveryError::new("Wave 6 work-item manifest has no items"))?;
    let mut manifest_ids = BTreeSet::new();
    let duplicate = items.iter().any(|item| {
        item.get("workItemId")
            .and_then(Value::as_str)
            .is_none_or(|id| !manifest_ids.insert(id))
    });
    let w6_ids = w6
        .iter()
        .filter_map(|node| node.get("id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let manifest_w6 = items
        .iter()
        .filter_map(|item| {
            (item.get("workItemId").and_then(Value::as_str))
                .map(|id| (id, item.get("implementationState").and_then(Value::as_str)))
        })
        .filter(|(id, _)| w6_ids.contains(id))
        .collect::<BTreeMap<_, _>>();
    if items.len() != 545
        || duplicate
        || manifest_w6.len() != w6_ids.len()
        || manifest_w6.values().any(|state| *state != Some("Planned"))
    {
        return Err(DeliveryError::new(
            "Wave 6 manifest state census is not all Planned at entry",
        ));
    }
    let tasks = fs::read_to_string(root.join(TASKS_PATH))
        .map_err(|_| DeliveryError::environment("cannot read the Wave 6 task contract"))?;
    for task in W6_LOCAL_TASKS {
        let marker = format!("] {task} ");
        let line = tasks
            .lines()
            .find(|line| line.contains(&marker))
            .ok_or_else(|| DeliveryError::new(format!("Wave 6 task contract has no {task} row")))?;
        if !line.contains("- [ ] ") {
            return Err(DeliveryError::new(format!(
                "Wave 6 local task {task} is not unchecked at T221 entry"
            )));
        }
    }
    Ok(())
}

fn read_external_json(path: &Path, label: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| DeliveryError::environment(format!("cannot read {label}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeliveryError::new(format!("{label} is not a regular file")));
    }
    let bytes =
        fs::read(path).map_err(|_| DeliveryError::environment(format!("cannot read {label}")))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn write_external_json_create<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| DeliveryError::environment("external record has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                DeliveryError::new("external record was created concurrently")
            } else {
                DeliveryError::environment("cannot create external delivery record")
            }
        })?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    sync_parent(parent)?;
    Ok(())
}

fn write_external_json_replace<T: Serialize>(
    path: &Path,
    value: &T,
    repository_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let path = validate_external_file(path, repository_roots)?;
    let bytes = serde_json::to_vec(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| DeliveryError::environment("external record has no parent"))?;
    fs::create_dir_all(parent)?;
    let ordinal = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".d2b-delivery-{ordinal}.tmp"));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| DeliveryError::environment("cannot create temporary delivery record"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
            .map_err(|_| DeliveryError::environment("cannot publish delivery record"))?;
        sync_parent(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::snapshot::tests::GitFixture;

    fn id(seed: char) -> String {
        std::iter::repeat_n(seed, 64).collect()
    }

    fn head(seed: char) -> String {
        std::iter::repeat_n(seed, 40).collect()
    }

    fn ledger() -> DispatchLedger {
        initial_ledger(&CandidateId::parse(id('a')).expect("candidate"), &head('b'))
            .expect("ledger")
    }

    #[test]
    fn initial_ledger_contains_the_closed_group_census() {
        let ledger = ledger();
        assert_eq!(ledger.entries.len(), 36);
        assert_eq!(ledger.launched_groups(), Vec::<String>::new());
        assert_eq!(
            ledger.ready_groups(true).expect("ready"),
            vec!["feature-local:w6-shared-prep".to_owned()]
        );
    }

    #[test]
    fn ledger_rejects_pre_t221_launches() {
        let mut ledger = ledger();
        ledger.entries[0].state = DispatchState::Dispatched;
        ledger.entries[0].dispatch_id = Some("dispatch-1".to_owned());
        ledger
            .validate()
            .expect("syntactically valid launched record");
        let error = ledger
            .require_pre_t221_state()
            .expect_err("pre-entry dispatch must block T221");
        assert!(error.message().contains("already launched"), "{error}");
    }

    #[test]
    fn local_states_are_monotonic_except_validation_retry() {
        transition_local_state(DispatchState::NotLaunched, DispatchState::Dispatched)
            .expect("dispatch");
        transition_local_state(DispatchState::Dispatched, DispatchState::Validated)
            .expect("validate");
        transition_local_state(DispatchState::Validated, DispatchState::Dispatched).expect("retry");
        transition_local_state(DispatchState::Completed, DispatchState::Dispatched)
            .expect_err("completed state cannot regress");
    }

    #[test]
    fn status_only_ledger_updates_keep_the_material_digest() {
        let before = ledger();
        let mut after = before.clone();
        after.entries[0].state = DispatchState::Dispatched;
        after.entries[0].dispatch_id = Some("dispatch-1".to_owned());
        after.entries[0].updated_at_unix = 42;
        assert_eq!(
            before.material_digest().expect("digest"),
            after.material_digest().expect("digest")
        );
    }

    #[test]
    fn checkbox_normalization_is_status_only() {
        let first = b"- [ ] T606\nrequirements stay material\n";
        let second = b"- [X] T606\nrequirements stay material\n";
        assert_eq!(
            normalize_status_only_updates(first),
            normalize_status_only_updates(second)
        );
        assert_ne!(
            normalize_status_only_updates(first),
            normalize_status_only_updates(b"- [ ] T606\nrequirements changed\n")
        );
    }

    #[test]
    fn fresh_fetch_evidence_verifies_the_fetched_object_and_command_identity() {
        let fixture = GitFixture::new("coordination-fresh-fetch");
        let head = fixture.head();
        let digest = sha256_bytes(&[]);
        let evidence = FreshFetchEvidence {
            artifact_kind: FRESH_FETCH_ARTIFACT_KIND.to_owned(),
            schema_version: FRESH_FETCH_SCHEMA_VERSION,
            repository: "github.com/vicondoa/d2b".to_owned(),
            remote: "origin".to_owned(),
            ref_name: "v3".to_owned(),
            fetched_oid: head.clone(),
            command_id: "git-fetch-origin-v3".to_owned(),
            argv: ["git", "fetch", "origin", "v3"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            started_at_unix: 1,
            completed_at_unix: 2,
            exit_code: 0,
            result: CommandResult::Passed,
            stdout_sha256: digest.clone(),
            stderr_sha256: digest,
            output_bytes: 0,
            fetched_at_unix: 2,
            before_oid: None,
        };
        evidence
            .validate(&fixture.repo(), &head)
            .expect("fresh-fetch evidence");

        let mut forged = evidence;
        forged.command_id = "remote-ref-only".to_owned();
        assert!(
            forged.validate(&fixture.repo(), &head).is_err(),
            "a remote ref name must not substitute for fetch evidence"
        );
    }
}
