use std::fmt;

pub mod command;
pub mod eligibility;
pub mod evidence;
pub mod model;
pub mod panel;
pub mod seal;
pub mod snapshot;
pub mod storage;

pub use command::{
    GH_STACK_VERSION, GhStatusSource, GitProbe, ProcessCommandOutput,
    check_gh_stack_private_preview,
};
pub use eligibility::{check_merge_eligibility, evaluate_merge_eligibility};
pub use evidence::{
    EvidenceImportRequest, EvidencePayloadSource, EvidenceRecord, import_evidence, verify_evidence,
};
pub use model::*;
pub use panel::{PanelRecord, validate_and_store_panel};
pub use seal::{
    PanelPayloadBinding, ValidationPayloadBinding, WaveSeal, construct_seal,
    verify_history_only_equivalence, verify_seal,
};
pub use snapshot::{create_snapshot, read_snapshot};

pub const DELIVERY_SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::result::Result<T, DeliveryError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryError {
    message: String,
}

impl DeliveryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for DeliveryError {}

impl From<std::io::Error> for DeliveryError {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("I/O error: {error}"))
    }
}

impl From<serde_json::Error> for DeliveryError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("JSON error: {error}"))
    }
}

pub fn run_cli(args: &[String]) -> std::process::ExitCode {
    match run_cli_inner(args) {
        Ok(message) => {
            println!("{message}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("delivery failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_cli_inner(args: &[String]) -> Result<String> {
    let probe = GitProbe::new(ProcessCommandOutput);
    match args {
        [area, action, rest @ ..] if area == "stack" && action == "validate" => {
            let mut options = CliOptions::parse(rest)?;
            let manifest_path = options.required_path("--manifest")?;
            options.finish()?;
            let manifest: StackManifest = storage::read_json(&manifest_path)?;
            manifest.validate()?;
            Ok("delivery stack manifest is valid".to_owned())
        }
        [area, action, rest @ ..] if area == "stack" && action == "capability" => {
            let mut options = CliOptions::parse(rest)?;
            let manifest_path = options.required_path("--manifest")?;
            options.finish()?;
            let manifest: StackManifest = storage::read_json(&manifest_path)?;
            manifest.validate()?;
            check_gh_stack_private_preview(&ProcessCommandOutput, &manifest.root_repository.name)?;
            Ok(format!(
                "official gh-stack {GH_STACK_VERSION} private preview is available"
            ))
        }
        [area, action, rest @ ..] if area == "wave" && action == "snapshot" => {
            let mut options = CliOptions::parse(rest)?;
            let manifest_path = options.required_path("--manifest")?;
            let state_dir = options.optional_path("--state-dir")?;
            options.finish()?;
            let manifest: StackManifest = storage::read_json(&manifest_path)?;
            let path = create_snapshot(&probe, &manifest, state_dir.as_deref())?;
            Ok(format!("wrote immutable snapshot {}", path.display()))
        }
        [area, action, rest @ ..] if area == "evidence" && action == "import" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot_path = options.required_path("--snapshot")?;
            let request_path = options.required_path("--request")?;
            options.finish()?;
            let path = import_evidence(&probe, &snapshot_path, &request_path)?;
            Ok(format!("imported validation evidence {}", path.display()))
        }
        [area, action, rest @ ..] if area == "evidence" && action == "verify" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot_path = options.required_path("--snapshot")?;
            let evidence_path = options.required_path("--evidence")?;
            options.finish()?;
            verify_evidence(&probe, &snapshot_path, &evidence_path)?;
            Ok("validation evidence is valid".to_owned())
        }
        [area, action, rest @ ..] if area == "panel" && action == "validate" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot_path = options.required_path("--snapshot")?;
            let records_dir = options.required_path("--records")?;
            options.finish()?;
            let records = validate_and_store_panel(&probe, &snapshot_path, &records_dir)?;
            let signed_off = records.iter().filter(|record| record.signoff).count();
            Ok(format!(
                "validated {} panel records ({signed_off} signoffs)",
                records.len()
            ))
        }
        [area, action, rest @ ..] if area == "wave" && action == "seal" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot_path = options.required_path("--snapshot")?;
            options.finish()?;
            let path = construct_seal(&probe, &snapshot_path)?;
            Ok(format!("wrote immutable wave seal {}", path.display()))
        }
        [area, action, rest @ ..] if area == "wave" && action == "verify" => {
            let mut options = CliOptions::parse(rest)?;
            let seal_path = options.required_path("--seal")?;
            options.finish()?;
            verify_seal(&probe, &seal_path)?;
            Ok("wave seal and current content are valid".to_owned())
        }
        [area, action, rest @ ..] if area == "wave" && action == "history-only" => {
            let mut options = CliOptions::parse(rest)?;
            let sealed_path = options.required_path("--sealed-snapshot")?;
            let candidate_path = options.required_path("--candidate-snapshot")?;
            options.finish()?;
            let sealed = read_snapshot(&sealed_path)?;
            let candidate = read_snapshot(&candidate_path)?;
            verify_history_only_equivalence(&sealed, &candidate)?;
            Ok("snapshots are history-only equivalent; rerun required CI".to_owned())
        }
        [area, action, rest @ ..] if area == "merge" && action == "eligibility" => {
            let mut options = CliOptions::parse(rest)?;
            let seal_path = options.required_path("--seal")?;
            let node = options.required_string("--node")?;
            options.finish()?;
            let command = ProcessCommandOutput;
            let status = GhStatusSource::new(&command);
            check_merge_eligibility(&probe, &status, &seal_path, &node)?;
            Ok(format!("stack node {node} is eligible to merge"))
        }
        _ => Err(DeliveryError::new(
            "usage: cargo xtask [delivery] <stack validate --manifest PATH|stack capability \
             --manifest PATH|wave snapshot \
             --manifest PATH [--state-dir PATH]|evidence import --snapshot PATH \
             --request PATH|evidence verify --snapshot PATH --evidence PATH|panel \
             validate --snapshot PATH --records DIR|wave seal --snapshot PATH|wave \
             verify --seal PATH|wave history-only --sealed-snapshot PATH \
             --candidate-snapshot PATH|merge eligibility --seal PATH --node ID>",
        )),
    }
}

#[derive(Debug)]
struct CliOptions {
    values: std::collections::BTreeMap<String, String>,
}

impl CliOptions {
    fn parse(args: &[String]) -> Result<Self> {
        let mut values = std::collections::BTreeMap::new();
        let mut chunks = args.chunks_exact(2);
        for pair in &mut chunks {
            if !pair[0].starts_with("--") {
                return Err(DeliveryError::new(format!(
                    "expected an option, found {}",
                    pair[0]
                )));
            }
            if values.insert(pair[0].clone(), pair[1].clone()).is_some() {
                return Err(DeliveryError::new(format!("duplicate option {}", pair[0])));
            }
        }
        if !chunks.remainder().is_empty() {
            return Err(DeliveryError::new("option is missing its value"));
        }
        Ok(Self { values })
    }

    fn required_string(&mut self, name: &str) -> Result<String> {
        self.values
            .remove(name)
            .ok_or_else(|| DeliveryError::new(format!("missing required option {name}")))
    }

    fn required_path(&mut self, name: &str) -> Result<std::path::PathBuf> {
        self.required_string(name).map(Into::into)
    }

    fn optional_path(&mut self, name: &str) -> Result<Option<std::path::PathBuf>> {
        Ok(self.values.remove(name).map(Into::into))
    }

    fn finish(&self) -> Result<()> {
        if self.values.is_empty() {
            Ok(())
        } else {
            Err(DeliveryError::new(format!(
                "unknown option(s): {}",
                self.values.keys().cloned().collect::<Vec<_>>().join(", ")
            )))
        }
    }
}
