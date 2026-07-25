//! Delivery CLI argument parsing, the `wave` subcommand table, and dispatch.
//!
//! `main.rs` forwards `cargo xtask delivery <args...>` here. Each workflow
//! stage owns its own module; this file only routes to it and fails closed for
//! any stage that has not landed.

use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    model::{CandidateDigests, validate_repository_id},
};

/// One stage of the wave delivery workflow.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WaveCommand {
    Help,
    Snapshot,
    ValidateImport,
    PanelRequest,
    PanelAttest,
    Seal,
    MergeEligibility,
}

/// Every wave subcommand, in workflow order.
pub const WAVE_COMMANDS: [WaveCommand; 7] = [
    WaveCommand::Snapshot,
    WaveCommand::ValidateImport,
    WaveCommand::PanelRequest,
    WaveCommand::PanelAttest,
    WaveCommand::Seal,
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
            Self::Seal | Self::MergeEligibility => "ADR046-delivery-006",
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
            Self::MergeEligibility => {
                "Confirm, per stacked pull request, that the seal still matches the current base \
                 and head and every required check is green."
            }
        }
    }

    pub fn required_options(self) -> &'static [&'static str] {
        match self {
            Self::Help => &[],
            Self::Snapshot => &["--program", "--wave", "--repo"],
            Self::ValidateImport => &["--snapshot", "--validation", "--result", "--repo"],
            Self::PanelRequest => &["--snapshot", "--repo"],
            Self::PanelAttest => &["--snapshot", "--records", "--repo"],
            Self::Seal => &["--snapshot", "--repo"],
            Self::MergeEligibility => &["--seal", "--target", "--repo"],
        }
    }

    pub fn optional_options(self) -> &'static [&'static str] {
        match self {
            Self::Help => &[],
            _ => &["--state-dir"],
        }
    }

    /// Whether this stage's implementation has landed.
    pub fn implemented(self) -> bool {
        matches!(
            self,
            Self::Help
                | Self::PanelRequest
                | Self::PanelAttest
                | Self::Seal
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<WorkflowCommandHelp>,
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

    /// Records the artifact this invocation produced. Only the external state
    /// path is reported; artifact content never reaches stdout.
    pub fn with_artifact(mut self, path: &std::path::Path) -> Result<Self> {
        self.artifact = Some(
            path.to_str()
                .ok_or_else(|| DeliveryError::new("delivery artifact path is not UTF-8"))?
                .to_owned(),
        );
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowCommandHelp {
    pub name: String,
    pub purpose: String,
    pub required_options: Vec<String>,
    pub optional_options: Vec<String>,
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
            output.commands = WAVE_COMMANDS
                .into_iter()
                .map(|command| WorkflowCommandHelp {
                    name: command.as_str().to_owned(),
                    purpose: command.purpose().to_owned(),
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
                    implemented: command.implemented(),
                    work_item: command.work_item().to_owned(),
                })
                .collect();
            Ok(output)
        }
        WaveCommand::PanelRequest => super::panel::run_request(rest),
        WaveCommand::PanelAttest => super::panel::run_attest(rest),
        WaveCommand::Seal => super::seal::run(rest),
        WaveCommand::MergeEligibility => super::eligibility::run(rest),
        other => Err(DeliveryError::unimplemented(
            other.as_str(),
            other.work_item(),
        )),
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
                "merge-eligibility",
                "help",
            ]
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
