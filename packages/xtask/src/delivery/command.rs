//! Delivery CLI argument parsing, the `wave` subcommand table, and dispatch.
//!
//! `main.rs` forwards `cargo xtask delivery <args...>` here. Each workflow
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
      \"number\": <pull request number>,
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
with no pull request.

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
  cargo xtask delivery wave merge-target \
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
            Self::Help => "cargo xtask delivery wave help",
            Self::Snapshot => {
                "cargo xtask delivery wave snapshot --program NAME --wave ID \
                 --repo LOGICAL_ID=CHECKOUT_ROOT --base LOGICAL_ID=REVISION \
                 [--head LOGICAL_ID=REVISION] [--edge FROM=TO] \
                 [--generated NAME=LOGICAL_ID:PATH] [--dependency NAME=LOGICAL_ID:PATH] \
                 [--contract NAME=LOGICAL_ID:PATH] [--state-dir DIR]"
            }
            Self::ValidateImport => {
                "cargo xtask delivery wave validate-import --snapshot PATH --validation NAME \
                 --result passed|failed --repo LOGICAL_ID=CHECKOUT_ROOT \
                 [--lane github-ci|local-host] [--command TEXT] [--log PATH] [--locator TEXT] \
                 [--candidate CANDIDATE_ID] [--state-dir DIR]"
            }
            Self::PanelRequest => {
                "cargo xtask delivery wave panel-request --snapshot PATH \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--state-dir DIR]"
            }
            Self::PanelAttest => {
                "cargo xtask delivery wave panel-attest --snapshot PATH --records DIR \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--state-dir DIR]"
            }
            Self::Seal => {
                "cargo xtask delivery wave seal --snapshot PATH --repo LOGICAL_ID=CHECKOUT_ROOT \
                 [--state-dir DIR]"
            }
            Self::MergeTarget => {
                "cargo xtask delivery wave merge-target --seal PATH --target PATH \
                 --repo LOGICAL_ID=CHECKOUT_ROOT [--state-dir DIR]"
            }
            Self::MergeEligibility => {
                "cargo xtask delivery wave merge-eligibility --seal PATH \
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

/// One JSON object describing the outcome of a delivery invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowOutput {
    pub schema_version: u32,
    pub operation: String,
    pub status: String,
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
                 example <state-root>/w0/<candidate-id>/snapshot.json"
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
            operation: operation.as_str().to_owned(),
            status: "ok".to_owned(),
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

/// Routes `cargo xtask delivery <args...>`.
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
    format!("usage: cargo xtask delivery wave <{stages}> [options]")
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
        assert_eq!(output.operation, "help");
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
            "cargo xtask delivery wave merge-target",
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
}
