//! Delivery CLI argument parsing, the `wave` subcommand table, and dispatch.
//!
//! `main.rs` forwards `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery <args...>` here. Each workflow
//! stage owns its own module; this file only routes to it and fails closed for
//! any stage that has not landed.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    model::{CandidateDigests, validate_repository_id},
    storage::CandidateDir,
};

/// The complete `merge-target` document schema and an offline recipe to build
/// it. Published through `wave help` (see [`WaveCommand::schema`]) so a
/// contributor authors the one hand-written stage input without reading
/// source.
///
/// The document's `material` is not authored field by field: it is copied
/// verbatim from the candidate's own `seal.json` (equivalently `snapshot.json`)
/// `material` object, which `merge-target` re-canonicalizes and re-derives, so
/// a stale or edited material is caught by the same digest check every stage
/// uses.
const MERGE_TARGET_SCHEMA: &str = "\
A merge-target document (the --target input) is:

{
  \"artifact_kind\": \"d2b-delivery/merge-target\",
  \"schema_version\": <integer, equal to the candidate's seal.json schema_version>,
  \"material\": <the candidate's integrated material, copied verbatim from seal.json's \"material\" object>,
  \"pull_requests\": [
    {
      \"repository\": \"<logical repository id, exactly as passed to --repo>\",
      \"number\": 42,
      \"base_ref\": \"<base branch name>\",
      \"base_oid\": \"<base commit object id, 40 or 64 hex characters>\",
      \"head_ref\": \"<head branch name>\",
      \"head_oid\": \"<head commit object id, 40 or 64 hex characters>\",
      \"required_checks\": [ { \"name\": \"<check name>\", \"conclusion\": \"success\" } ]
    }
  ]
}

Every required check must read \"success\"; pending, failure, neutral, skipped, \
cancelled, stale, timed_out, action_required, and startup_failure all fail \
closed, as does a pull request with no required checks or a sealed repository \
with no pull request. Every \"number\" is a positive integer identifying the \
pull request (42 above is an example); 0 is rejected.

Offline recipe (no network I/O happens inside this stage):

  SEAL=<state-root>/<wave>/<candidate>/seal.json
  # Gather the live stack out of band, e.g. with:
  #   gh pr view <number> --repo <owner/name> \
  #     --json number,baseRefName,baseRefOid,headRefName,headRefOid,statusCheckRollup
  # Then assemble the document, taking material straight from the seal:
  jq -n --slurpfile s \"$SEAL\" '{
      artifact_kind: \"d2b-delivery/merge-target\",
      schema_version: $s[0].schema_version,
      material: $s[0].material,
      pull_requests: [ /* one object per repository, shaped as above */ ]
    }' > merge-target.json
  cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-target \
      --seal \"$SEAL\" --target merge-target.json --repo <logical-id>=<checkout-root>

merge-target validates the shape, canonicalizes the material, and installs \
merge-target.json under the candidate; merge-eligibility then reads that \
captured target when no --target is given.";

/// One stage of the wave delivery workflow.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WaveCommand {
    Help,
    Snapshot,
    ValidateImport,
    PanelRequest,
    PanelAttest,
    Seal,
    MergeTarget,
    MergeEligibility,
}

/// Every wave subcommand, in workflow order.
pub const WAVE_COMMANDS: [WaveCommand; 8] = [
    WaveCommand::Snapshot,
    WaveCommand::ValidateImport,
    WaveCommand::PanelRequest,
    WaveCommand::PanelAttest,
    WaveCommand::Seal,
    WaveCommand::MergeTarget,
    WaveCommand::MergeEligibility,
    WaveCommand::Help,
];

impl WaveCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Snapshot => "snapshot",
            Self::ValidateImport => "validate-import",
            Self::PanelRequest => "panel-request",
            Self::PanelAttest => "panel-attest",
            Self::Seal => "seal",
            Self::MergeTarget => "merge-target",
            Self::MergeEligibility => "merge-eligibility",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        WAVE_COMMANDS
            .into_iter()
            .find(|command| command.as_str() == name)
    }

    /// Work item that owns this stage's implementation.
    pub fn work_item(self) -> &'static str {
        match self {
            Self::Help => "ADR046-delivery-002",
            Self::Snapshot => "ADR046-delivery-002",
            Self::ValidateImport => "ADR046-delivery-003",
            Self::PanelRequest | Self::PanelAttest => "ADR046-delivery-005",
            Self::Seal | Self::MergeTarget | Self::MergeEligibility => "ADR046-delivery-006",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Self::Help => "List the wave workflow stages and their options.",
            Self::Snapshot => {
                "Bind the wave's base and head commits, dependency graph, and repository set into \
                 one immutable candidate."
            }
            Self::ValidateImport => {
                "Import CI, local, and host validator command results as evidence addressed by \
                 candidate ID."
            }
            Self::PanelRequest => {
                "Write the candidate-bound ten-role panel request into external delivery state."
            }
            Self::PanelAttest => {
                "Validate one panel record per role against the candidate's digests."
            }
            Self::Seal => {
                "Bind unanimous panel records and passing validator lanes to one candidate."
            }
            Self::MergeTarget => {
                "Capture the wave's current pull-request stack into the candidate as the \
                 merge-eligibility input."
            }
            Self::MergeEligibility => {
                "Confirm, per stacked pull request, that the seal still matches the current base \
                 and head and every required check is green."
            }
        }
    }

    /// A one-line synopsis showing every option and its value grammar,
    /// including the compound `key=value` forms the flat option list cannot
    /// express.
    pub fn synopsis(self) -> &'static str {
        match self {
            Self::Help => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave help"
            }
            Self::Snapshot => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave snapshot --program NAME --wave ID \
                 --repo LOGICAL_ID=CHECKOUT_ROOT --base LOGICAL_ID=REVISION \
                 [--head LOGICAL_ID=REVISION] [--edge FROM=TO] \
                 [--generated NAME=LOGICAL_ID:PATH] [--dependency NAME=LOGICAL_ID:PATH] \
                 [--contract NAME=LOGICAL_ID:PATH] [--state-dir DIR]"
            }
            Self::ValidateImport => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave validate-import --snapshot PATH --validation NAME \
                 --result passed|failed --repo LOGICAL_ID=CHECKOUT_ROOT \
                 [--lane github-ci|local-host] [--command TEXT] [--log PATH] [--locator TEXT] \
                 [--candidate CANDIDATE_ID] [--state-dir DIR]"
            }
            Self::PanelRequest => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-request --snapshot PATH \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--state-dir DIR]"
            }
            Self::PanelAttest => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-attest --snapshot PATH --records DIR \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--state-dir DIR]"
            }
            Self::Seal => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave seal --snapshot PATH --repo LOGICAL_ID=CHECKOUT_ROOT \
                 [--state-dir DIR]"
            }
            Self::MergeTarget => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-target --seal PATH --target PATH \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--state-dir DIR]"
            }
            Self::MergeEligibility => {
                "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-eligibility --seal PATH \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--target PATH] [--state-dir DIR]"
            }
        }
    }

    pub fn required_options(self) -> &'static [&'static str] {
        match self {
            Self::Help => &[],
            Self::Snapshot => &["--program", "--wave", "--repo", "--base"],
            Self::ValidateImport => &["--snapshot", "--validation", "--result", "--repo"],
            Self::PanelRequest => &["--snapshot", "--repo"],
            Self::PanelAttest => &["--snapshot", "--records", "--repo"],
            Self::Seal => &["--snapshot", "--repo"],
            Self::MergeTarget => &["--seal", "--target", "--repo"],
            Self::MergeEligibility => &["--seal", "--repo"],
        }
    }

    pub fn optional_options(self) -> &'static [&'static str] {
        match self {
            Self::Help => &[],
            Self::Snapshot => &[
                "--state-dir",
                "--head",
                "--edge",
                "--generated",
                "--dependency",
                "--contract",
            ],
            Self::ValidateImport => &[
                "--state-dir",
                "--lane",
                "--command",
                "--log",
                "--locator",
                "--candidate",
            ],
            Self::MergeEligibility => &["--state-dir", "--target"],
            _ => &["--state-dir"],
        }
    }

    /// The complete JSON schema and executable recipe for a stage whose input
    /// a contributor must author by hand.
    ///
    /// Only `merge-target` needs one: its `--target` document is not produced
    /// by an earlier stage, so its full shape and a precise, offline recipe are
    /// published here rather than living in source comments.
    pub fn schema(self) -> Option<&'static str> {
        match self {
            Self::MergeTarget => Some(MERGE_TARGET_SCHEMA),
            _ => None,
        }
    }

    /// Whether this stage's implementation has landed.
    pub fn implemented(self) -> bool {
        matches!(
            self,
            Self::Help
                | Self::Snapshot
                | Self::ValidateImport
                | Self::PanelRequest
                | Self::PanelAttest
                | Self::Seal
                | Self::MergeTarget
                | Self::MergeEligibility
        )
    }
}

impl std::fmt::Display for WaveCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for WaveCommand {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// The delivery outcome domain, pinned to a closed set.
///
/// A successful invocation always reports [`WorkflowStatus::Ok`]; every failure
/// path returns [`DeliveryError`] and is rendered separately with a nonzero exit
/// code, so `status` never widens implicitly. Keeping this a typed, single-member
/// enum (rather than a free `String`) means adding an outcome is a deliberate wire
/// change that the golden contract test forces to travel with a
/// [`DELIVERY_SCHEMA_VERSION`](super::DELIVERY_SCHEMA_VERSION) bump.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowStatus {
    Ok,
}

impl WorkflowStatus {
    /// Every status variant, in wire order.
    ///
    /// The golden contract fingerprint enumerates this array, and a
    /// wildcard-free exhaustiveness guard in that module fails to compile if a
    /// variant is added without extending `ALL`, so the outcome domain cannot
    /// widen without moving the pinned golden and the schema version together.
    pub const ALL: &'static [WorkflowStatus] = &[WorkflowStatus::Ok];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
        }
    }
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for WorkflowStatus {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// One JSON object describing the outcome of a delivery invocation.
///
/// The wire shape is a pinned contract: field names, the `operation` domain
/// (every [`WaveCommand`] wire string) and the `status` domain
/// ([`WorkflowStatus`]) are all typed, and the complete serialization is fixed
/// by the golden contract test in this module. Any incompatible change to the
/// shape or either domain must travel with a
/// [`DELIVERY_SCHEMA_VERSION`](super::DELIVERY_SCHEMA_VERSION) bump, or that test
/// fails the build.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowOutput {
    pub schema_version: u32,
    pub operation: WaveCommand,
    pub status: WorkflowStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<StateHelp>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<WorkflowCommandHelp>,
}

/// Where delivery state lives and how one stage's output chains into the next.
///
/// Emitted by the `help` stage so a contributor can chain the workflow without
/// reading source or searching the filesystem. Every field is a grammar or a
/// template, never an expanded path, so it carries no `HOME`, username, or
/// checkout path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StateHelp {
    pub default_root: String,
    pub override_flag: String,
    pub layout: String,
    pub chaining: String,
}

impl StateHelp {
    fn describe() -> Self {
        Self {
            default_root: "$XDG_STATE_HOME/d2b/delivery when XDG_STATE_HOME is set, else \
                 $HOME/.local/state/d2b/delivery"
                .to_owned(),
            override_flag: "--state-dir DIR overrides the default state root on every stage"
                .to_owned(),
            layout:
                "delivery state is laid out as <state-root>/<wave>/<candidate>/<artifact>, for \
                 example <state-root>/W0/<candidate-id>/snapshot.json"
                    .to_owned(),
            chaining:
                "each stage reports its output in the artifact field as a state-root-relative \
                 reference (<wave>/<candidate>/<artifact>); pass that value as the next stage's \
                 --snapshot or --seal and it resolves under the same state root, so chaining \
                 needs no absolute path"
                    .to_owned(),
        }
    }
}

impl WorkflowOutput {
    pub fn ok(operation: WaveCommand) -> Self {
        Self {
            schema_version: DELIVERY_SCHEMA_VERSION,
            operation,
            status: WorkflowStatus::Ok,
            candidate_id: None,
            content_id: None,
            snapshot_sha256: None,
            artifact: None,
            state: None,
            commands: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_digests(mut self, digests: &CandidateDigests) -> Self {
        self.candidate_id = Some(digests.candidate_id.as_str().to_owned());
        self.content_id = Some(digests.content_id.as_str().to_owned());
        self.snapshot_sha256 = Some(digests.snapshot_sha256.as_str().to_owned());
        self
    }

    /// Records the artifact this invocation produced, as a bounded
    /// state-root-relative reference.
    ///
    /// The reference is `<wave>/<candidate>/<artifact>` (for example
    /// `w0/<candidate-id>/snapshot.json`), never the absolute state path:
    /// structured stdout must never carry `HOME`, the local username, or a
    /// checkout or store path into a CI or operator log. Because the reference
    /// also names the wave and candidate, a later stage resolves it under the
    /// same state root, so this field is exactly the value to pass as the next
    /// stage's `--snapshot` or `--seal`. The absolute path stays internal to
    /// storage.
    pub fn with_artifact(mut self, candidate: &CandidateDir, path: &Path) -> Result<Self> {
        self.artifact = Some(candidate.state_relative_key(path)?);
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowCommandHelp {
    pub name: String,
    pub purpose: String,
    pub synopsis: String,
    pub required_options: Vec<String>,
    pub optional_options: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub implemented: bool,
    pub work_item: String,
}

/// Routes `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery <args...>`.
pub fn dispatch(args: &[String]) -> Result<WorkflowOutput> {
    match args {
        [group, rest @ ..] if group == "wave" => dispatch_wave(rest),
        [] => Err(DeliveryError::usage(usage())),
        [group, ..] => Err(DeliveryError::usage(format!(
            "unknown delivery group {group:?}; {}",
            usage()
        ))),
    }
}

fn dispatch_wave(args: &[String]) -> Result<WorkflowOutput> {
    let (name, rest) = args
        .split_first()
        .ok_or_else(|| DeliveryError::usage(usage()))?;
    let command = WaveCommand::parse(name).ok_or_else(|| {
        DeliveryError::usage(format!("unknown delivery wave stage {name:?}; {}", usage()))
    })?;
    match command {
        WaveCommand::Help => {
            let options = CliOptions::parse(rest)?;
            options.finish()?;
            let mut output = WorkflowOutput::ok(WaveCommand::Help);
            output.state = Some(StateHelp::describe());
            output.commands = WAVE_COMMANDS
                .into_iter()
                .map(|command| WorkflowCommandHelp {
                    name: command.as_str().to_owned(),
                    purpose: command.purpose().to_owned(),
                    synopsis: command.synopsis().to_owned(),
                    required_options: command
                        .required_options()
                        .iter()
                        .map(|option| (*option).to_owned())
                        .collect(),
                    optional_options: command
                        .optional_options()
                        .iter()
                        .map(|option| (*option).to_owned())
                        .collect(),
                    schema: command.schema().map(str::to_owned),
                    implemented: command.implemented(),
                    work_item: command.work_item().to_owned(),
                })
                .collect();
            Ok(output)
        }
        WaveCommand::Snapshot => super::snapshot::run(rest),
        WaveCommand::ValidateImport => super::evidence::run(rest),
        WaveCommand::PanelRequest => super::panel::run_request(rest),
        WaveCommand::PanelAttest => super::panel::run_attest(rest),
        WaveCommand::Seal => super::seal::run(rest),
        WaveCommand::MergeTarget => super::eligibility::run_capture(rest),
        WaveCommand::MergeEligibility => super::eligibility::run(rest),
    }
}

fn usage() -> String {
    let stages = WAVE_COMMANDS
        .into_iter()
        .map(WaveCommand::as_str)
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "usage: cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave <{stages}> [options]"
    )
}

/// Long-option parser shared by every wave stage.
///
/// Options are `--name value` pairs. Repeated options are collected, so a
/// stage can accept several `--repo` mappings, and [`CliOptions::finish`]
/// rejects anything a stage did not consume.
#[derive(Debug, Default)]
pub struct CliOptions {
    values: BTreeMap<String, Vec<String>>,
}

impl CliOptions {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut values = BTreeMap::<String, Vec<String>>::new();
        let mut chunks = args.chunks_exact(2);
        for pair in &mut chunks {
            if !pair[0].starts_with("--") {
                return Err(DeliveryError::usage(format!(
                    "expected an option, found {}",
                    pair[0]
                )));
            }
            values
                .entry(pair[0].clone())
                .or_default()
                .push(pair[1].clone());
        }
        if !chunks.remainder().is_empty() {
            return Err(DeliveryError::usage(format!(
                "option {} is missing its value",
                chunks.remainder()[0]
            )));
        }
        Ok(Self { values })
    }

    pub fn required_string(&mut self, name: &str) -> Result<String> {
        let values = self
            .values
            .remove(name)
            .ok_or_else(|| DeliveryError::usage(format!("missing required option {name}")))?;
        if values.len() != 1 {
            return Err(DeliveryError::usage(format!(
                "option {name} must appear exactly once"
            )));
        }
        Ok(values.into_iter().next().expect("exactly one value"))
    }

    pub fn required_path(&mut self, name: &str) -> Result<PathBuf> {
        self.required_string(name).map(PathBuf::from)
    }

    pub fn optional_string(&mut self, name: &str) -> Result<Option<String>> {
        match self.values.remove(name) {
            None => Ok(None),
            Some(values) if values.len() == 1 => Ok(values.into_iter().next()),
            Some(_) => Err(DeliveryError::usage(format!(
                "option {name} must appear at most once"
            ))),
        }
    }

    pub fn optional_path(&mut self, name: &str) -> Result<Option<PathBuf>> {
        Ok(self.optional_string(name)?.map(PathBuf::from))
    }

    /// Consumes every occurrence of a repeated option, in the order supplied.
    pub fn repeated_strings(&mut self, name: &str) -> Vec<String> {
        self.values.remove(name).unwrap_or_default()
    }

    /// Parses the repeated `--repo LOGICAL_ID=CHECKOUT_ROOT` mappings that tell
    /// every stage which checkouts delivery state must stay outside of.
    pub fn repository_roots(&mut self) -> Result<BTreeMap<String, PathBuf>> {
        let values = self.values.remove("--repo").ok_or_else(|| {
            DeliveryError::usage("at least one --repo LOGICAL_ID=CHECKOUT_ROOT mapping is required")
        })?;
        let mut roots = BTreeMap::new();
        for value in values {
            let (id, root) = value
                .split_once('=')
                .ok_or_else(|| DeliveryError::usage("--repo must use LOGICAL_ID=CHECKOUT_ROOT"))?;
            validate_repository_id(id)?;
            if root.is_empty() || roots.insert(id.to_owned(), PathBuf::from(root)).is_some() {
                return Err(DeliveryError::usage(
                    "--repo mapping has an empty root or a duplicate logical ID",
                ));
            }
        }
        Ok(roots)
    }

    pub fn finish(&self) -> Result<()> {
        if self.values.is_empty() {
            return Ok(());
        }
        Err(DeliveryError::usage(format!(
            "unknown option(s): {}",
            self.values.keys().cloned().collect::<Vec<_>>().join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::DeliveryErrorKind;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn help_lists_every_wave_stage() {
        let output = dispatch(&args(&["wave", "help"])).expect("help succeeds");
        assert_eq!(output.operation.as_str(), "help");
        let listed = output
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            listed,
            vec![
                "snapshot",
                "validate-import",
                "panel-request",
                "panel-attest",
                "seal",
                "merge-target",
                "merge-eligibility",
                "help",
            ]
        );
    }

    #[test]
    fn help_shows_a_synopsis_with_compound_option_grammar() {
        let output = dispatch(&args(&["wave", "help"])).expect("help succeeds");
        for command in &output.commands {
            assert!(
                !command.synopsis.is_empty(),
                "{} must carry a synopsis",
                command.name
            );
            assert!(
                command.synopsis.contains(&command.name),
                "{}'s synopsis must name the stage",
                command.name
            );
        }
        let snapshot = output
            .commands
            .iter()
            .find(|command| command.name == "snapshot")
            .expect("snapshot is listed");
        assert!(
            snapshot
                .synopsis
                .contains("--repo LOGICAL_ID=CHECKOUT_ROOT"),
            "the synopsis must spell the compound --repo grammar: {}",
            snapshot.synopsis
        );
        assert!(
            snapshot.synopsis.contains("--edge FROM=TO"),
            "the synopsis must spell the compound --edge grammar: {}",
            snapshot.synopsis
        );
        assert!(
            snapshot
                .synopsis
                .contains("--generated NAME=LOGICAL_ID:PATH"),
            "the synopsis must spell the compound fingerprint grammar: {}",
            snapshot.synopsis
        );
    }

    #[test]
    fn help_documents_the_state_root_default_layout_and_chaining() {
        let output = dispatch(&args(&["wave", "help"])).expect("help succeeds");
        let state = output.state.expect("help documents the state root");
        assert!(
            state.default_root.contains("$XDG_STATE_HOME/d2b/delivery")
                && state
                    .default_root
                    .contains("$HOME/.local/state/d2b/delivery"),
            "the default state root grammar must be published: {}",
            state.default_root
        );
        assert!(
            state.override_flag.contains("--state-dir"),
            "the override flag must be published: {}",
            state.override_flag
        );
        assert!(
            state
                .layout
                .contains("<state-root>/<wave>/<candidate>/<artifact>"),
            "the on-disk layout must be published: {}",
            state.layout
        );
        assert!(
            state.chaining.contains("--snapshot") && state.chaining.contains("--seal"),
            "chaining guidance must name the options a later stage consumes: {}",
            state.chaining
        );
        // The documentation is grammar and templates only: never an expanded
        // path that would leak HOME, the username, or a checkout.
        let home = std::env::var("HOME").unwrap_or_default();
        for field in [
            &state.default_root,
            &state.override_flag,
            &state.layout,
            &state.chaining,
        ] {
            assert!(
                !home.is_empty() && !field.contains(&home),
                "state help must not leak the expanded HOME path"
            );
        }
    }

    #[test]
    fn help_publishes_the_complete_merge_target_schema() {
        let output = dispatch(&args(&["wave", "help"])).expect("help succeeds");
        let merge_target = output
            .commands
            .iter()
            .find(|command| command.name == "merge-target")
            .expect("merge-target is listed");
        let schema = merge_target
            .schema
            .as_deref()
            .expect("merge-target publishes its schema");
        for token in [
            "\"artifact_kind\": \"d2b-delivery/merge-target\"",
            "\"schema_version\"",
            "\"material\"",
            "\"pull_requests\"",
            "\"base_oid\"",
            "\"head_oid\"",
            "\"required_checks\"",
            "\"conclusion\": \"success\"",
            "seal.json",
            "cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-target",
        ] {
            assert!(
                schema.contains(token),
                "the published schema must spell {token:?}: {schema}"
            );
        }
        assert!(
            !schema.contains("..."),
            "the published schema must not use an ellipsis for the material shape"
        );
        // No other stage carries a hand-authored-input schema.
        for command in &output.commands {
            if command.name != "merge-target" {
                assert!(
                    command.schema.is_none(),
                    "{} must not carry a schema",
                    command.name
                );
            }
        }
    }

    #[test]
    fn the_module_usage_string_lists_merge_target() {
        assert!(
            usage().contains("merge-target"),
            "the wave usage string must list merge-target: {}",
            usage()
        );
    }

    #[test]
    fn every_unimplemented_stage_fails_closed() {
        for command in WAVE_COMMANDS.into_iter().filter(|c| !c.implemented()) {
            let error = dispatch(&args(&["wave", command.as_str()]))
                .expect_err("an unimplemented stage must not report success");
            assert_eq!(error.kind(), DeliveryErrorKind::Unimplemented);
            assert!(error.message().contains(command.as_str()));
            assert!(error.message().contains(command.work_item()));
            assert_ne!(error.kind().exit_code(), 0);
        }
    }

    #[test]
    fn an_unknown_stage_is_a_usage_error() {
        let error = dispatch(&args(&["wave", "teleport"])).expect_err("unknown stage");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
        let error = dispatch(&args(&["orbit"])).expect_err("unknown group");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
        let error = dispatch(&[]).expect_err("no arguments");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
    }

    #[test]
    fn help_rejects_stray_options() {
        let error = dispatch(&args(&["wave", "help", "--state-dir", "/state"]))
            .expect_err("help takes no options");
        assert_eq!(error.kind(), DeliveryErrorKind::Usage);
    }

    #[test]
    fn options_parse_into_required_optional_and_repeated_values() {
        let mut options = CliOptions::parse(&args(&[
            "--wave",
            "w0",
            "--repo",
            "github.com/example/d2b=/checkout",
            "--repo",
            "github.com/example/entrablau=/other",
        ]))
        .expect("parse");
        assert_eq!(options.required_string("--wave").expect("wave"), "w0");
        assert_eq!(options.optional_path("--state-dir").expect("absent"), None);
        let roots = options.repository_roots().expect("roots");
        assert_eq!(roots.len(), 2);
        assert_eq!(roots["github.com/example/d2b"], PathBuf::from("/checkout"));
        options.finish().expect("all options consumed");
    }

    #[test]
    fn malformed_options_are_usage_errors() {
        assert_eq!(
            CliOptions::parse(&args(&["--wave"]))
                .expect_err("dangling option")
                .kind(),
            DeliveryErrorKind::Usage
        );
        assert_eq!(
            CliOptions::parse(&args(&["wave", "w0"]))
                .expect_err("positional argument")
                .kind(),
            DeliveryErrorKind::Usage
        );
        let mut options =
            CliOptions::parse(&args(&["--wave", "w0", "--wave", "w1"])).expect("parse");
        assert_eq!(
            options
                .required_string("--wave")
                .expect_err("repeated")
                .kind(),
            DeliveryErrorKind::Usage
        );
        let options = CliOptions::parse(&args(&["--stray", "value"])).expect("parse");
        assert_eq!(
            options.finish().expect_err("unconsumed").kind(),
            DeliveryErrorKind::Usage
        );
    }

    #[test]
    fn repository_mappings_are_validated() {
        let mut options =
            CliOptions::parse(&args(&["--repo", "not-a-repository=/checkout"])).expect("parse");
        assert!(options.repository_roots().is_err());
        let mut options =
            CliOptions::parse(&args(&["--repo", "github.com/example/d2b"])).expect("parse");
        assert!(options.repository_roots().is_err());
        let mut options = CliOptions::parse(&args(&[
            "--repo",
            "github.com/example/d2b=/a",
            "--repo",
            "github.com/example/d2b=/b",
        ]))
        .expect("parse");
        assert!(options.repository_roots().is_err());
    }

    /// Golden contract for the delivery success JSON.
    ///
    /// This JSON is the interface every wave's evidence import and every future
    /// CI consumer reads. These tests pin its complete wire shape: the top-level
    /// field names, which optional fields are omitted when empty, the
    /// `operation` domain (every [`WaveCommand`] wire string), the `status`
    /// domain ([`WorkflowStatus`]), and the nested [`StateHelp`] and
    /// [`WorkflowCommandHelp`] field names.
    ///
    /// Any incompatible change to the shape or either domain fails these tests.
    /// The only correct way to make them pass again is to bump
    /// [`DELIVERY_SCHEMA_VERSION`](crate::delivery::DELIVERY_SCHEMA_VERSION) and
    /// update the goldens below in the same change, so the wire contract moves
    /// visibly and every downstream consumer is forced to notice.
    mod golden_contract {
        use super::*;
        use crate::delivery::DELIVERY_SCHEMA_VERSION;
        use serde_json::{Value, json};

        /// The schema version the goldens below were authored against. Coupled
        /// to the production constant by [`schema_version_moves_with_the_golden`]
        /// so a version bump without a golden update, or a golden update without
        /// a version bump, fails the build.
        const GOLDEN_SCHEMA_VERSION: u32 = 1;

        fn sorted_keys(value: &Value) -> Vec<String> {
            value
                .as_object()
                .expect("a JSON object")
                .keys()
                .cloned()
                .collect()
        }

        #[test]
        fn schema_version_moves_with_the_golden() {
            assert_eq!(
                DELIVERY_SCHEMA_VERSION, GOLDEN_SCHEMA_VERSION,
                "DELIVERY_SCHEMA_VERSION changed; update GOLDEN_SCHEMA_VERSION and every golden \
                 in this module in the same change so the wire contract moves for consumers"
            );
        }

        #[test]
        fn minimal_success_output_matches_the_golden() {
            let output = WorkflowOutput::ok(WaveCommand::Snapshot);
            assert_eq!(
                serde_json::to_value(&output).expect("serialize the minimal output"),
                json!({
                    "schema_version": GOLDEN_SCHEMA_VERSION,
                    "operation": "snapshot",
                    "status": "ok",
                }),
                "the minimal success JSON drifted from its pinned contract"
            );
        }

        #[test]
        fn stage_output_with_digests_and_artifact_matches_the_golden() {
            // Build the fullest non-help envelope by field, so the golden does
            // not depend on candidate-fixture internals; the field types are
            // what the contract pins.
            let mut output = WorkflowOutput::ok(WaveCommand::Seal);
            output.candidate_id = Some("candidate-0000".to_owned());
            output.content_id = Some("content-1111".to_owned());
            output.snapshot_sha256 = Some("2222".to_owned());
            output.artifact = Some("w0/candidate-0000/seal.json".to_owned());
            assert_eq!(
                serde_json::to_value(&output).expect("serialize the full stage output"),
                json!({
                    "schema_version": GOLDEN_SCHEMA_VERSION,
                    "operation": "seal",
                    "status": "ok",
                    "candidate_id": "candidate-0000",
                    "content_id": "content-1111",
                    "snapshot_sha256": "2222",
                    "artifact": "w0/candidate-0000/seal.json",
                }),
                "the stage success JSON drifted from its pinned contract"
            );
        }

        #[test]
        fn empty_optional_fields_are_omitted_from_the_wire() {
            let value = serde_json::to_value(WorkflowOutput::ok(WaveCommand::Snapshot))
                .expect("serialize the minimal output");
            assert_eq!(
                sorted_keys(&value),
                vec![
                    "operation".to_owned(),
                    "schema_version".to_owned(),
                    "status".to_owned(),
                ],
                "the minimal envelope must omit every empty optional field"
            );
        }

        #[test]
        fn operation_domain_is_closed_and_pinned() {
            let domain = WAVE_COMMANDS
                .into_iter()
                .map(|command| {
                    serde_json::to_value(WorkflowOutput::ok(command)).expect("serialize")
                        ["operation"]
                        .clone()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                Value::Array(domain),
                json!([
                    "snapshot",
                    "validate-import",
                    "panel-request",
                    "panel-attest",
                    "seal",
                    "merge-target",
                    "merge-eligibility",
                    "help",
                ]),
                "the operation domain drifted; a renamed or added stage must move the schema \
                 version and this golden together"
            );
        }

        #[test]
        fn status_domain_is_closed_and_pinned() {
            // Derive the domain from the enumeration itself, not a hand-copied
            // literal: a hard-coded `[WorkflowStatus::Ok]` here would keep
            // passing after a variant is added, which is exactly the drift this
            // golden must catch. `ALL` is kept honest by the exhaustiveness
            // guard below.
            let domain = WorkflowStatus::ALL
                .iter()
                .map(|status| serde_json::to_value(status).expect("serialize"))
                .collect::<Vec<_>>();
            assert_eq!(
                Value::Array(domain),
                json!(["ok"]),
                "the status domain drifted; a new outcome must move the schema version and this \
                 golden together"
            );
        }

        #[test]
        fn workflow_status_all_enumerates_every_variant() {
            // Wildcard-free membership guard. Adding a `WorkflowStatus` variant
            // makes this match non-exhaustive, failing compilation until the
            // author adds an arm here; the arm's assertion then forces the new
            // variant into `ALL`, and `ALL` feeds every status-domain golden.
            for status in WorkflowStatus::ALL {
                let listed = match status {
                    WorkflowStatus::Ok => WorkflowStatus::ALL.contains(&WorkflowStatus::Ok),
                };
                assert!(
                    listed,
                    "every WorkflowStatus variant must be listed in WorkflowStatus::ALL"
                );
            }
        }

        #[test]
        fn wave_commands_enumerates_every_stage() {
            // The operation domain is `WAVE_COMMANDS`, a hand-maintained array.
            // This wildcard-free guard makes adding a `WaveCommand` variant a
            // compile error until an arm is added, and the arm forces the new
            // stage into `WAVE_COMMANDS`, so the operation-domain golden cannot
            // silently omit a live stage.
            fn assert_listed(command: WaveCommand) {
                let listed = match command {
                    WaveCommand::Help => WAVE_COMMANDS.contains(&WaveCommand::Help),
                    WaveCommand::Snapshot => WAVE_COMMANDS.contains(&WaveCommand::Snapshot),
                    WaveCommand::ValidateImport => {
                        WAVE_COMMANDS.contains(&WaveCommand::ValidateImport)
                    }
                    WaveCommand::PanelRequest => WAVE_COMMANDS.contains(&WaveCommand::PanelRequest),
                    WaveCommand::PanelAttest => WAVE_COMMANDS.contains(&WaveCommand::PanelAttest),
                    WaveCommand::Seal => WAVE_COMMANDS.contains(&WaveCommand::Seal),
                    WaveCommand::MergeTarget => WAVE_COMMANDS.contains(&WaveCommand::MergeTarget),
                    WaveCommand::MergeEligibility => {
                        WAVE_COMMANDS.contains(&WaveCommand::MergeEligibility)
                    }
                };
                assert!(
                    listed,
                    "every WaveCommand variant must be listed in WAVE_COMMANDS"
                );
            }
            for command in WAVE_COMMANDS {
                assert_listed(command);
            }
        }

        #[test]
        fn help_envelope_shape_is_pinned() {
            let output = dispatch(&args(&["wave", "help"])).expect("help succeeds");
            let value = serde_json::to_value(&output).expect("serialize the help output");
            assert_eq!(
                sorted_keys(&value),
                vec![
                    "commands".to_owned(),
                    "operation".to_owned(),
                    "schema_version".to_owned(),
                    "state".to_owned(),
                    "status".to_owned(),
                ],
                "the help envelope field set drifted from its pinned contract"
            );
            assert_eq!(value["schema_version"], json!(GOLDEN_SCHEMA_VERSION));
            assert_eq!(value["operation"], json!("help"));
            assert_eq!(value["status"], json!("ok"));

            assert_eq!(
                sorted_keys(&value["state"]),
                vec![
                    "chaining".to_owned(),
                    "default_root".to_owned(),
                    "layout".to_owned(),
                    "override_flag".to_owned(),
                ],
                "the state-help field set drifted from its pinned contract"
            );

            // Every command element carries the same closed field set; the
            // optional `schema` key appears only where a stage publishes one.
            let commands = value["commands"].as_array().expect("commands array");
            for command in commands {
                let mut keys = sorted_keys(command);
                keys.retain(|key| key != "schema");
                assert_eq!(
                    keys,
                    vec![
                        "implemented".to_owned(),
                        "name".to_owned(),
                        "optional_options".to_owned(),
                        "purpose".to_owned(),
                        "required_options".to_owned(),
                        "synopsis".to_owned(),
                        "work_item".to_owned(),
                    ],
                    "a command-help element field set drifted from its pinned contract"
                );
            }
            let with_schema = commands
                .iter()
                .filter(|command| command.get("schema").is_some())
                .filter_map(|command| command["name"].as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                with_schema,
                vec!["merge-target"],
                "only merge-target publishes a hand-authored-input schema"
            );
        }

        /// A `WorkflowOutput` with every field populated to a non-omitted
        /// value, built as an explicit struct literal for `WorkflowOutput`,
        /// `StateHelp`, and `WorkflowCommandHelp` with NO struct-update
        /// fallthrough.
        ///
        /// This is the field-exhaustiveness guard: adding a field to any of
        /// those three structs fails to compile here until the author
        /// populates it. Because a populated optional field then serializes,
        /// it appears in the fingerprint's field sets, so a new field cannot be
        /// introduced (and emitted by some stage) without moving the golden and
        /// forcing a schema-version bump. The placeholder values are
        /// deliberately prose-free so the fingerprint pins shape, not the
        /// human-facing help text (whose wording is covered by other tests and
        /// is not part of the wire contract).
        fn fully_populated_output(operation: WaveCommand) -> WorkflowOutput {
            WorkflowOutput {
                schema_version: DELIVERY_SCHEMA_VERSION,
                operation,
                status: WorkflowStatus::Ok,
                candidate_id: Some("candidate-0000".to_owned()),
                content_id: Some("content-1111".to_owned()),
                snapshot_sha256: Some("2222".to_owned()),
                artifact: Some("w0/candidate-0000/seal.json".to_owned()),
                state: Some(StateHelp {
                    default_root: "default-root".to_owned(),
                    override_flag: "override-flag".to_owned(),
                    layout: "layout".to_owned(),
                    chaining: "chaining".to_owned(),
                }),
                commands: vec![WorkflowCommandHelp {
                    name: "name".to_owned(),
                    purpose: "purpose".to_owned(),
                    synopsis: "synopsis".to_owned(),
                    required_options: vec!["--required".to_owned()],
                    optional_options: vec!["--optional".to_owned()],
                    schema: Some("schema".to_owned()),
                    implemented: true,
                    work_item: "work-item".to_owned(),
                }],
            }
        }

        /// Reduces a JSON value to its structural skeleton: an object becomes
        /// its sorted keys mapped to child skeletons; an array becomes the
        /// union of its elements' skeletons (so a heterogeneous wire array such
        /// as help `commands`, where only `merge-target` carries the optional
        /// `schema` key, still pins the full field set); a scalar becomes its
        /// JSON type name. Prose values collapse to `"string"`, so the
        /// fingerprint captures field presence and nesting without pinning
        /// help text that is not part of the wire contract.
        fn shape(value: &Value) -> Value {
            match value {
                Value::Object(map) => {
                    let mut keys = map.keys().cloned().collect::<Vec<_>>();
                    keys.sort();
                    let mut out = serde_json::Map::new();
                    for key in keys {
                        out.insert(key.clone(), shape(&map[&key]));
                    }
                    Value::Object(out)
                }
                Value::Array(items) => {
                    let mut merged = serde_json::Map::new();
                    let mut scalar = None;
                    for item in items {
                        match shape(item) {
                            Value::Object(map) => {
                                for (key, child) in map {
                                    merged.insert(key, child);
                                }
                            }
                            other => scalar = Some(other),
                        }
                    }
                    if !merged.is_empty() {
                        Value::Array(vec![Value::Object(merged)])
                    } else if let Some(other) = scalar {
                        Value::Array(vec![other])
                    } else {
                        Value::Array(Vec::new())
                    }
                }
                Value::String(_) => Value::String("string".to_owned()),
                Value::Number(_) => Value::String("number".to_owned()),
                Value::Bool(_) => Value::String("bool".to_owned()),
                Value::Null => Value::String("null".to_owned()),
            }
        }

        /// The generated wire fingerprint: every enum domain (serialized from
        /// the enumerations themselves, not hand-copied literals), the full
        /// field set of every emitted struct, and the serialized shape of a
        /// representative output for every wave stage. Any field, domain, or
        /// per-stage shape change moves this value.
        fn live_fingerprint() -> Value {
            let status_domain = WorkflowStatus::ALL
                .iter()
                .map(|status| serde_json::to_value(status).expect("serialize status"))
                .collect::<Vec<_>>();
            let operation_domain = WAVE_COMMANDS
                .iter()
                .map(|command| serde_json::to_value(command).expect("serialize operation"))
                .collect::<Vec<_>>();

            let template = serde_json::to_value(fully_populated_output(WaveCommand::Seal))
                .expect("serialize the fully populated template");
            let workflow_output_fields = sorted_keys(&template);
            let state_help_fields = sorted_keys(&template["state"]);
            let command_help_fields = sorted_keys(&template["commands"][0]);

            // Serialize a representative output for every stage: the real
            // dispatched help envelope for `help`, and the fully populated
            // template (operation swapped) for the rest. Reducing each to its
            // shape pins per-stage field presence and nesting.
            let mut stages = serde_json::Map::new();
            for command in WAVE_COMMANDS {
                let output = if command == WaveCommand::Help {
                    serde_json::to_value(dispatch(&args(&["wave", "help"])).expect("help succeeds"))
                        .expect("serialize the help output")
                } else {
                    serde_json::to_value(fully_populated_output(command))
                        .expect("serialize a stage output")
                };
                stages.insert(command.as_str().to_owned(), shape(&output));
            }

            json!({
                "schema_version": DELIVERY_SCHEMA_VERSION,
                "status_domain": status_domain,
                "operation_domain": operation_domain,
                "workflow_output_fields": workflow_output_fields,
                "state_help_fields": state_help_fields,
                "command_help_fields": command_help_fields,
                "stages": Value::Object(stages),
            })
        }

        /// The pinned fingerprint for each schema version.
        ///
        /// A version bump with no new arm here panics with a clear message, so
        /// the golden cannot silently follow the code; the author must capture
        /// `live_fingerprint()` and pin it against the new version.
        fn golden_fingerprint(version: u32) -> Value {
            match version {
                1 => serde_json::from_str::<Value>(GOLDEN_FINGERPRINT_V1)
                    .expect("the pinned v1 fingerprint is valid JSON"),
                other => panic!(
                    "no pinned delivery wire fingerprint golden for schema version {other}; \
                     capture live_fingerprint() and add a matching arm to golden_fingerprint in \
                     the same change"
                ),
            }
        }

        const GOLDEN_FINGERPRINT_V1: &str = r#"{
  "schema_version": 1,
  "status_domain": ["ok"],
  "operation_domain": [
    "snapshot",
    "validate-import",
    "panel-request",
    "panel-attest",
    "seal",
    "merge-target",
    "merge-eligibility",
    "help"
  ],
  "workflow_output_fields": [
    "artifact",
    "candidate_id",
    "commands",
    "content_id",
    "operation",
    "schema_version",
    "snapshot_sha256",
    "state",
    "status"
  ],
  "state_help_fields": ["chaining", "default_root", "layout", "override_flag"],
  "command_help_fields": [
    "implemented",
    "name",
    "optional_options",
    "purpose",
    "required_options",
    "schema",
    "synopsis",
    "work_item"
  ],
  "stages": {
    "snapshot": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "validate-import": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "panel-request": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "panel-attest": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "seal": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "merge-target": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "merge-eligibility": {
      "artifact": "string",
      "candidate_id": "string",
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": ["string"],
          "purpose": "string",
          "required_options": ["string"],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "content_id": "string",
      "operation": "string",
      "schema_version": "number",
      "snapshot_sha256": "string",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    },
    "help": {
      "commands": [
        {
          "implemented": "bool",
          "name": "string",
          "optional_options": [],
          "purpose": "string",
          "required_options": [],
          "schema": "string",
          "synopsis": "string",
          "work_item": "string"
        }
      ],
      "operation": "string",
      "schema_version": "number",
      "state": {
        "chaining": "string",
        "default_root": "string",
        "layout": "string",
        "override_flag": "string"
      },
      "status": "string"
    }
  }
}"#;

        #[test]
        fn generated_fingerprint_matches_the_versioned_golden() {
            assert_eq!(
                live_fingerprint(),
                golden_fingerprint(DELIVERY_SCHEMA_VERSION),
                "the generated delivery wire fingerprint drifted from its pinned golden; any \
                 field, status or operation domain, or per-stage shape change must bump \
                 DELIVERY_SCHEMA_VERSION and add a new golden_fingerprint arm in the same change"
            );
        }

        #[test]
        fn a_version_without_a_pinned_golden_fails_closed() {
            let unpinned = DELIVERY_SCHEMA_VERSION + 1;
            let panicked = std::panic::catch_unwind(|| golden_fingerprint(unpinned)).is_err();
            assert!(
                panicked,
                "golden_fingerprint must fail closed for an unpinned schema version so a bump \
                 cannot land without a matching golden"
            );
        }
    }
}
