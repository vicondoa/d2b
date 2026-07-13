use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use serde::Serialize;

pub mod command;
pub mod eligibility;
pub mod evidence;
pub mod model;
pub mod panel;
pub mod seal;
pub mod snapshot;
pub mod storage;

pub use command::{
    GIT_TOWN_LOCKED_VERSION, GIT_TOWN_SUPPORTED_MAJOR, GhMergeSource, GhStatusSource, GitProbe,
    GitTownStackSource, ProcessCommandOutput, StackCapability, check_git_town_capability,
};
pub use eligibility::{
    MergeEligibility, atomic_history_merge, atomic_merge, check_history_merge_eligibility,
    check_merge_eligibility, evaluate_merge_eligibility,
};
pub use evidence::{
    CiAttestationClaims, CiAttestationPolicy, CiAttestationVerifier, EvidenceProvenance,
    EvidenceRecord, GithubAttestationVerifier, VerifiedCiAttestation, import_ci_evidence,
    run_validation, verify_evidence,
};
pub use model::*;
pub use panel::{
    OpenSslPanelReceiptVerifier, PanelAttestation, PanelRequest, create_panel_request,
    validate_and_store_panel,
};
pub use seal::{
    HistoryProof, PanelPayloadBinding, ValidationPayloadBinding, WaveSeal, construct_history_proof,
    construct_seal, verify_history_only_equivalence, verify_history_proof, verify_seal,
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

#[derive(Serialize)]
struct WorkflowOutput {
    schema_version: u32,
    operation: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    integration_points: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commands: Vec<WorkflowCommandHelp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack_capability: Option<StackCapability>,
}

#[derive(Serialize)]
struct WorkflowCommandHelp {
    name: String,
    purpose: String,
    required_options: Vec<String>,
    optional_options: Vec<String>,
}

pub fn run_cli(args: &[String]) -> std::process::ExitCode {
    match run_cli_inner(args) {
        Ok(output) => match serde_json::to_string(&output) {
            Ok(json) => {
                println!("{json}");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("delivery failed: cannot render result: {error}");
                std::process::ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("delivery failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_cli_inner(args: &[String]) -> Result<WorkflowOutput> {
    let command = ProcessCommandOutput;
    let probe = GitProbe::new(command);
    let graph = GitTownStackSource::new(&command);
    let github = GhStatusSource::new(&command);
    let ci_verifier = GithubAttestationVerifier::new(&command);
    let panel_verifier = OpenSslPanelReceiptVerifier::new(&command);
    match args {
        [area, action] if area == "wave" && action == "help" => Ok(WorkflowOutput {
            schema_version: DELIVERY_SCHEMA_VERSION,
            operation: "help".to_owned(),
            status: "ok".to_owned(),
            candidate_id: None,
            artifact: None,
            stages: vec![
                "snapshot".to_owned(),
                "validation-run".to_owned(),
                "validation-import".to_owned(),
                "panel-request".to_owned(),
                "panel-attest".to_owned(),
                "seal".to_owned(),
                "verify".to_owned(),
                "eligibility".to_owned(),
                "history-proof".to_owned(),
                "retarget-preflight".to_owned(),
                "merge".to_owned(),
            ],
            integration_points: vec![
                "checked-in-authoritative-manifest".to_owned(),
                "git-town-parent-configuration".to_owned(),
                "ordinary-github-pull-requests".to_owned(),
                "github-check-suite-run-authority".to_owned(),
                "offline-github-attestation-bundle".to_owned(),
                "signed-external-panel-receipts".to_owned(),
                "exact-base-head-merge-authority".to_owned(),
                "external-layer1-renderer".to_owned(),
            ],
            commands: workflow_command_help(),
            stack_capability: None,
        }),
        [area, action, rest @ ..] if area == "stack" && action == "capability" => {
            let mut options = CliOptions::parse(rest)?;
            let repository_id = options.required_string("--repository")?;
            options.finish()?;
            validate_repository_id(&repository_id)?;
            let repository = repository_id.strip_prefix("github.com/").ok_or_else(|| {
                DeliveryError::new("authority repository is not hosted by GitHub")
            })?;
            let capability = check_git_town_capability(&ProcessCommandOutput, repository)?;
            Ok(WorkflowOutput {
                schema_version: DELIVERY_SCHEMA_VERSION,
                operation: "stack-capability".to_owned(),
                status: "ok".to_owned(),
                candidate_id: None,
                artifact: None,
                stages: vec![],
                integration_points: vec![
                    format!("git-town-{}-supported-major", capability.supported_major),
                    "ordinary-github-pull-request-api".to_owned(),
                ],
                commands: vec![],
                stack_capability: Some(capability),
            })
        }
        [area, action, rest @ ..] if area == "wave" && action == "snapshot" => {
            let mut options = CliOptions::parse(rest)?;
            let authority_repository = options.required_string("--authority-repository")?;
            let authority_ref = options.required_string("--authority-ref")?;
            let manifest_path = options.required_path("--manifest-path")?;
            let repository_roots = options.repository_roots()?;
            let state_root = options.optional_path("--state-dir")?;
            options.finish()?;
            let request = SnapshotRequest {
                authority_repository,
                authority_ref,
                manifest_path,
                repository_roots,
                state_root,
            };
            let path = create_snapshot(&probe, &graph, &github, &request)?;
            output_for_artifact("snapshot", &path)
        }
        [area, action, rest @ ..] if area == "wave" && action == "validation-run" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot = options.required_path("--snapshot")?;
            let validation = options.required_string("--validation")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let path = run_validation(&probe, &command, &roots, &snapshot, &validation)?;
            output_for_artifact("validation-run", &path)
        }
        [area, action, rest @ ..] if area == "wave" && action == "validation-import" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot = options.required_path("--snapshot")?;
            let artifact = options.required_path("--artifact")?;
            let bundle = options.required_path("--bundle")?;
            let payload = options.optional_path("--payload")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let path = import_ci_evidence(
                &probe,
                &ci_verifier,
                &roots,
                &snapshot,
                &artifact,
                &bundle,
                payload.as_deref(),
            )?;
            output_for_artifact("validation-import", &path)
        }
        [area, action, rest @ ..] if area == "wave" && action == "panel-request" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot = options.required_path("--snapshot")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let path = create_panel_request(&probe, &roots, &snapshot)?;
            output_for_artifact("panel-request", &path)
        }
        [area, action, rest @ ..] if area == "wave" && action == "panel-attest" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot = options.required_path("--snapshot")?;
            let records = options.required_path("--records")?;
            let trust_root = options.required_path("--trust-root")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            validate_and_store_panel(
                &probe,
                &panel_verifier,
                &roots,
                &snapshot,
                &records,
                &trust_root,
            )?;
            output_for_candidate("panel-attest", &snapshot, None)
        }
        [area, action, rest @ ..] if area == "wave" && action == "seal" => {
            let mut options = CliOptions::parse(rest)?;
            let snapshot = options.required_path("--snapshot")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let path = construct_seal(
                &probe,
                &github,
                &ci_verifier,
                &panel_verifier,
                &roots,
                &snapshot,
            )?;
            output_for_artifact("seal", &path)
        }
        [area, action, rest @ ..] if area == "wave" && action == "verify" => {
            let mut options = CliOptions::parse(rest)?;
            let seal = options.required_path("--seal")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let verified = verify_seal(
                &probe,
                &github,
                &ci_verifier,
                &panel_verifier,
                &roots,
                &seal,
            )?;
            Ok(WorkflowOutput {
                schema_version: DELIVERY_SCHEMA_VERSION,
                operation: "verify".to_owned(),
                status: "verified".to_owned(),
                candidate_id: Some(verified.candidate_id),
                artifact: Some(path_string(&seal)?),
                stages: vec![],
                integration_points: vec![],
                commands: vec![],
                stack_capability: None,
            })
        }
        [area, action, rest @ ..]
            if area == "wave"
                && matches!(action.as_str(), "history-proof" | "retarget-preflight") =>
        {
            let mut options = CliOptions::parse(rest)?;
            let old_seal = options.required_path("--old-seal")?;
            let new_snapshot = options.required_path("--new-snapshot")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let path = construct_history_proof(
                &probe,
                &ci_verifier,
                &panel_verifier,
                &roots,
                &old_seal,
                &new_snapshot,
            )?;
            output_for_artifact(action, &path)
        }
        [area, action, rest @ ..] if area == "wave" && action == "eligibility" => {
            let mut options = CliOptions::parse(rest)?;
            let seal = options.required_path("--seal")?;
            let target = options.required_string("--target")?;
            let snapshot = options.optional_path("--new-snapshot")?;
            let proof = options.optional_path("--history-proof")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let eligible = match (snapshot, proof) {
                (None, None) => check_merge_eligibility(
                    &probe,
                    &github,
                    &ci_verifier,
                    &panel_verifier,
                    &roots,
                    &seal,
                    &target,
                )?,
                (Some(snapshot), Some(proof)) => check_history_merge_eligibility(
                    &probe,
                    &github,
                    &ci_verifier,
                    &panel_verifier,
                    &roots,
                    &seal,
                    &snapshot,
                    &proof,
                    &target,
                )?,
                _ => {
                    return Err(DeliveryError::new(
                        "--new-snapshot and --history-proof must be supplied together",
                    ));
                }
            };
            output_for_eligibility("eligibility", eligible)
        }
        [area, action, rest @ ..] if area == "wave" && action == "merge" => {
            let mut options = CliOptions::parse(rest)?;
            let seal = options.required_path("--seal")?;
            let target = options.required_string("--target")?;
            let snapshot = options.optional_path("--new-snapshot")?;
            let proof = options.optional_path("--history-proof")?;
            let roots = options.repository_roots()?;
            options.finish()?;
            let merger = GhMergeSource::new(&command);
            let merged = match (snapshot, proof) {
                (None, None) => atomic_merge(
                    &probe,
                    &github,
                    &ci_verifier,
                    &panel_verifier,
                    &merger,
                    &roots,
                    &seal,
                    &target,
                )?,
                (Some(snapshot), Some(proof)) => atomic_history_merge(
                    &probe,
                    &github,
                    &ci_verifier,
                    &panel_verifier,
                    &merger,
                    &roots,
                    &seal,
                    &snapshot,
                    &proof,
                    &target,
                )?,
                _ => {
                    return Err(DeliveryError::new(
                        "--new-snapshot and --history-proof must be supplied together",
                    ));
                }
            };
            output_for_eligibility("merge", merged)
        }
        _ => Err(DeliveryError::new(
            "usage: cargo xtask delivery <stack capability --repository github.com/OWNER/REPOSITORY|wave <help|snapshot|validation-run|validation-import|panel-request|panel-attest|seal|verify|eligibility|history-proof|retarget-preflight|merge> [options]>",
        )),
    }
}

fn output_for_artifact(operation: &str, path: &Path) -> Result<WorkflowOutput> {
    let candidate = candidate_id_from_artifact(path)?;
    Ok(WorkflowOutput {
        schema_version: DELIVERY_SCHEMA_VERSION,
        operation: operation.to_owned(),
        status: "ok".to_owned(),
        candidate_id: candidate,
        artifact: Some(path_string(path)?),
        stages: vec![],
        integration_points: vec![],
        commands: vec![],
        stack_capability: None,
    })
}

fn output_for_candidate(
    operation: &str,
    snapshot: &Path,
    artifact: Option<&Path>,
) -> Result<WorkflowOutput> {
    let snapshot = read_snapshot(snapshot)?;
    Ok(WorkflowOutput {
        schema_version: DELIVERY_SCHEMA_VERSION,
        operation: operation.to_owned(),
        status: "ok".to_owned(),
        candidate_id: Some(snapshot.candidate_id),
        artifact: artifact.map(path_string).transpose()?,
        stages: vec![],
        integration_points: vec![],
        commands: vec![],
        stack_capability: None,
    })
}

fn output_for_eligibility(
    operation: &str,
    eligibility: MergeEligibility,
) -> Result<WorkflowOutput> {
    Ok(WorkflowOutput {
        schema_version: DELIVERY_SCHEMA_VERSION,
        operation: operation.to_owned(),
        status: if operation == "merge" {
            "merge-request-accepted".to_owned()
        } else {
            "eligible-preflight-only".to_owned()
        },
        candidate_id: Some(eligibility.candidate_id),
        artifact: None,
        stages: vec![],
        integration_points: vec![],
        commands: vec![],
        stack_capability: None,
    })
}

fn workflow_command_help() -> Vec<WorkflowCommandHelp> {
    [
        (
            "snapshot",
            "Import checked-in authority, Git Town parent topology, and ordinary GitHub PR state into an immutable candidate.",
            &[
                "--authority-repository",
                "--authority-ref",
                "--manifest-path",
                "--repo",
            ][..],
            &["--state-dir"][..],
        ),
        (
            "validation-run",
            "Execute one authoritative argv in a read-only detached checkout, post-check it, and capture bounded evidence.",
            &["--snapshot", "--validation", "--repo"],
            &[],
        ),
        (
            "validation-import",
            "Offline-verify and retain a GitHub CI artifact plus signed attestation bundle and optional payload.",
            &[
                "--snapshot",
                "--artifact",
                "--bundle",
                "--repo",
            ],
            &["--payload"],
        ),
        (
            "panel-request",
            "Write the external ten-role request bound to the candidate.",
            &["--snapshot", "--repo"],
            &[],
        ),
        (
            "panel-attest",
            "Verify and retain ten externally signed panel receipts against an out-of-band trust root.",
            &["--snapshot", "--records", "--trust-root", "--repo"],
            &[],
        ),
        (
            "seal",
            "Bind passing evidence, signed panel receipts, exact PRs, and nested GitHub check-suite run authority.",
            &["--snapshot", "--repo"],
            &[],
        ),
        (
            "verify",
            "Re-verify retained CI bundles and panel signatures plus current refs and GitHub authority.",
            &["--seal", "--repo"],
            &[],
        ),
        (
            "eligibility",
            "Run a preflight only; atomic merge repeats every authority check.",
            &["--seal", "--target", "--repo"],
            &["--new-snapshot", "--history-proof"],
        ),
        (
            "history-proof",
            "Prove commit-history or merged-stack progression with content identity and require newer CI run IDs and timestamps.",
            &["--old-seal", "--new-snapshot", "--repo"],
            &[],
        ),
        (
            "retarget-preflight",
            "Run history-only retarget proof before changing merge-train state.",
            &["--old-seal", "--new-snapshot", "--repo"],
            &[],
        ),
        (
            "merge",
            "Recheck authority and fail closed unless the backend provides exact base+head CAS or verified merge-group authority.",
            &["--seal", "--target", "--repo"],
            &["--new-snapshot", "--history-proof"],
        ),
    ]
    .into_iter()
    .map(
        |(name, purpose, required_options, optional_options)| WorkflowCommandHelp {
            name: name.to_owned(),
            purpose: purpose.to_owned(),
            required_options: required_options
                .iter()
                .map(|option| (*option).to_owned())
                .collect(),
            optional_options: optional_options
                .iter()
                .map(|option| (*option).to_owned())
                .collect(),
        },
    )
    .collect()
}

fn candidate_id_from_artifact(path: &Path) -> Result<Option<String>> {
    if path.file_name().and_then(|name| name.to_str()) == Some("snapshot.json") {
        return Ok(Some(read_snapshot(path)?.candidate_id));
    }
    let candidate = path.ancestors().find_map(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| {
                name.len() == 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .map(str::to_owned)
    });
    Ok(candidate)
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("workflow path is not UTF-8"))
}

#[derive(Debug)]
struct CliOptions {
    values: BTreeMap<String, Vec<String>>,
}

impl CliOptions {
    fn parse(args: &[String]) -> Result<Self> {
        let mut values = BTreeMap::<String, Vec<String>>::new();
        let mut chunks = args.chunks_exact(2);
        for pair in &mut chunks {
            if !pair[0].starts_with("--") {
                return Err(DeliveryError::new(format!(
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
            return Err(DeliveryError::new("option is missing its value"));
        }
        Ok(Self { values })
    }

    fn required_string(&mut self, name: &str) -> Result<String> {
        let values = self
            .values
            .remove(name)
            .ok_or_else(|| DeliveryError::new(format!("missing required option {name}")))?;
        if values.len() != 1 {
            return Err(DeliveryError::new(format!(
                "option {name} must appear exactly once"
            )));
        }
        Ok(values.into_iter().next().expect("one value"))
    }

    fn required_path(&mut self, name: &str) -> Result<PathBuf> {
        self.required_string(name).map(Into::into)
    }

    fn optional_path(&mut self, name: &str) -> Result<Option<PathBuf>> {
        match self.values.remove(name) {
            None => Ok(None),
            Some(values) if values.len() == 1 => Ok(values.into_iter().next().map(PathBuf::from)),
            Some(_) => Err(DeliveryError::new(format!(
                "option {name} must appear at most once"
            ))),
        }
    }

    fn repository_roots(&mut self) -> Result<BTreeMap<String, PathBuf>> {
        let values = self.values.remove("--repo").ok_or_else(|| {
            DeliveryError::new("at least one --repo LOGICAL_ID=CHECKOUT_ROOT mapping is required")
        })?;
        let mut roots = BTreeMap::new();
        for value in values {
            let (id, root) = value
                .split_once('=')
                .ok_or_else(|| DeliveryError::new("--repo must use LOGICAL_ID=CHECKOUT_ROOT"))?;
            validate_repository_id(id)?;
            if root.is_empty() || roots.insert(id.to_owned(), PathBuf::from(root)).is_some() {
                return Err(DeliveryError::new(
                    "--repo mapping has an empty root or duplicate logical ID",
                ));
            }
        }
        Ok(roots)
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
