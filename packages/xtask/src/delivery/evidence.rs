use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CommandLimits, CommandOutputAdapter, DEFAULT_COMMAND_OUTPUT_BYTES, RepositoryProbe},
    model::{
        EVIDENCE_ARTIFACT_KIND, EvidenceResult, LogicalPath, RepositoryBinding,
        ValidationAuthority, ensure_schema, validate_bounded_string, validate_identifier,
        validate_sha256,
    },
    snapshot::{CurrentVerification, SnapshotContext, load_snapshot_context},
    storage::{
        ensure_external_path, read_json, read_json_with_digest, read_verified_json,
        reject_delivery_payload, secure_repository_subdir, sha256_file, validate_payload_locator,
        write_immutable_json,
    },
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub candidate_id: String,
    pub content_id: String,
    pub snapshot_sha256: String,
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: LogicalPath,
    pub repository_set: Vec<RepositoryBinding>,
    pub result: EvidenceResult,
    pub exit_code: Option<i32>,
    pub captured_at_unix_seconds: u64,
    pub payload_locator: String,
    pub payload_sha256: String,
    pub provenance: EvidenceProvenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceProvenance {
    LocalRunner {
        runner: String,
        runner_version: String,
        run_id: String,
    },
    GithubAttestation {
        repository: String,
        run_id: String,
        check_run_id: String,
        app_slug: String,
        app_id: u64,
        workflow: String,
        workflow_id: u64,
        signer_workflow: String,
        source_digest: String,
        source_ref: String,
        attestation_sha256: String,
    },
}

static NEXT_LOCAL_RUN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CiAttestationClaims {
    pub candidate_id: String,
    pub content_id: String,
    pub snapshot_sha256: String,
    pub validation_id: String,
    pub argv: Vec<String>,
    pub cwd: LogicalPath,
    pub repository_set: Vec<RepositoryBinding>,
    pub exit_code: i32,
    pub conclusion: String,
    pub captured_at_unix_seconds: u64,
    pub payload_locator: String,
    pub payload_sha256: String,
    pub repository: String,
    pub run_id: String,
    pub check_run_id: String,
    pub app_slug: String,
    pub app_id: u64,
    pub workflow: String,
    pub workflow_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCiAttestation {
    pub claims: CiAttestationClaims,
    pub attestation_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CiAttestationPolicy {
    pub repository: String,
    pub source_digest: String,
    pub source_ref: String,
    pub signer_workflow: String,
}

pub trait CiAttestationVerifier {
    fn verify(
        &self,
        attestation_path: &Path,
        policy: &CiAttestationPolicy,
    ) -> Result<VerifiedCiAttestation>;
}

#[derive(Debug)]
pub struct GithubAttestationVerifier<'a, A> {
    command: &'a A,
}

impl<'a, A> GithubAttestationVerifier<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> CiAttestationVerifier for GithubAttestationVerifier<'_, A> {
    fn verify(
        &self,
        attestation_path: &Path,
        policy: &CiAttestationPolicy,
    ) -> Result<VerifiedCiAttestation> {
        let repository = github_repo_arg(&policy.repository)?;
        let path = path_string(attestation_path)?;
        let (claims_before, digest_before): (CiAttestationClaims, String) =
            read_json_with_digest(attestation_path)?;
        let output = self.command.output(
            "gh",
            &[
                "attestation".to_owned(),
                "verify".to_owned(),
                path,
                "--repo".to_owned(),
                repository,
                "--format".to_owned(),
                "json".to_owned(),
                "--signer-workflow".to_owned(),
                policy.signer_workflow.clone(),
                "--source-digest".to_owned(),
                policy.source_digest.clone(),
                "--source-ref".to_owned(),
                policy.source_ref.clone(),
                "--deny-self-hosted-runners".to_owned(),
            ],
            None,
        )?;
        if !output.success {
            return Err(DeliveryError::new("GitHub attestation verification failed"));
        }
        let verified: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
            .map_err(|error| DeliveryError::new(format!("invalid gh attestation JSON: {error}")))?;
        if verified.is_empty()
            || !verified
                .iter()
                .any(|entry| verification_contains_digest(entry, &digest_before))
        {
            return Err(DeliveryError::new(
                "GitHub attestation output did not bind the claims artifact digest",
            ));
        }
        let (claims_after, digest_after): (CiAttestationClaims, String) =
            read_json_with_digest(attestation_path)?;
        if digest_before != digest_after || claims_before != claims_after {
            return Err(DeliveryError::new(
                "GitHub attestation changed while it was being verified",
            ));
        }
        Ok(VerifiedCiAttestation {
            claims: claims_after,
            attestation_sha256: digest_after,
        })
    }
}

fn verification_contains_digest(entry: &serde_json::Value, digest: &str) -> bool {
    entry
        .pointer("/verificationResult/statement/subject")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|subjects| {
            subjects.iter().any(|subject| {
                subject
                    .pointer("/digest/sha256")
                    .and_then(serde_json::Value::as_str)
                    == Some(digest)
            })
        })
}

pub fn run_validation<P: RepositoryProbe, A: CommandOutputAdapter>(
    probe: &P,
    runner: &A,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    validation_id: &str,
) -> Result<PathBuf> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    validate_identifier(validation_id, "validation id")?;
    let required = required_validation(&context, validation_id)?;
    if required.authority != ValidationAuthority::LocalRunner {
        return Err(DeliveryError::new(format!(
            "validation {validation_id} requires verified GitHub attestation import"
        )));
    }
    let repository_root = context
        .repository_roots
        .get(&required.cwd.repository)
        .ok_or_else(|| DeliveryError::new("validation cwd repository mapping is missing"))?;
    let cwd = secure_repository_subdir(repository_root, Path::new(&required.cwd.path))?;
    let output = runner.output_with_limits(
        &required.argv[0],
        &required.argv[1..],
        Some(&cwd),
        CommandLimits {
            stdout_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            timeout: Duration::from_secs(required.timeout_seconds),
        },
    )?;
    let result = if output.success {
        EvidenceResult::Passed
    } else {
        EvidenceResult::Failed
    };
    let payload_sha256 = discarded_output_digest(&output.stdout, &output.stderr);
    let record = EvidenceRecord {
        artifact_kind: EVIDENCE_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        candidate_id: context.snapshot.candidate_id.clone(),
        content_id: context.snapshot.content_id.clone(),
        snapshot_sha256: context.digest.clone(),
        id: required.id.clone(),
        argv: required.argv.clone(),
        cwd: required.cwd.clone(),
        repository_set: context.snapshot.repository_bindings(),
        result,
        exit_code: output.exit_code,
        captured_at_unix_seconds: now_unix_seconds()?,
        payload_locator: "discarded://stdout-stderr".to_owned(),
        payload_sha256,
        provenance: EvidenceProvenance::LocalRunner {
            runner: "xtask-local".to_owned(),
            runner_version: env!("CARGO_PKG_VERSION").to_owned(),
            run_id: local_run_id()?,
        },
    };
    validate_record(&context, &record)?;
    let path = evidence_path(&context, validation_id);
    write_immutable_json(&path, &record)?;
    Ok(path)
}

pub fn import_ci_evidence<P: RepositoryProbe, V: CiAttestationVerifier>(
    probe: &P,
    verifier: &V,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    attestation_path: &Path,
    payload_path: Option<&Path>,
) -> Result<PathBuf> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    ensure_external_path(attestation_path, &context.external_exclusions)?;
    reject_delivery_payload(attestation_path, &context.layout.root)?;
    let asserted_claims: CiAttestationClaims = read_json(attestation_path)?;
    let required = required_validation(&context, &asserted_claims.validation_id)?;
    if required.authority != ValidationAuthority::GithubAttestation {
        return Err(DeliveryError::new(format!(
            "validation {} is not authorized for CI attestation import",
            asserted_claims.validation_id
        )));
    }
    let repository = context
        .snapshot
        .repository_set
        .iter()
        .find(|repository| repository.id == required.cwd.repository)
        .ok_or_else(|| DeliveryError::new("CI validation repository is absent"))?;
    let policy = CiAttestationPolicy {
        repository: repository.id.clone(),
        source_digest: repository.integration_oid.clone(),
        source_ref: format!("refs/heads/{}", repository.integration_ref),
        signer_workflow: required
            .ci_signer_workflow
            .clone()
            .ok_or_else(|| DeliveryError::new("CI signer workflow policy is absent"))?,
    };
    let verified = verifier.verify(attestation_path, &policy)?;
    let claims = verified.claims;
    if claims != asserted_claims {
        return Err(DeliveryError::new(
            "verified CI claims changed during attestation verification",
        ));
    }
    validate_ci_claims(&context, required, &claims)?;
    if let Some(payload) = payload_path {
        ensure_external_path(payload, &context.external_exclusions)?;
        reject_delivery_payload(payload, &context.layout.root)?;
        if sha256_file(payload)? != claims.payload_sha256 {
            return Err(DeliveryError::new(
                "retrieved CI payload digest does not match signed attestation",
            ));
        }
    }
    let result = if claims.exit_code == 0 && claims.conclusion == "success" {
        EvidenceResult::Passed
    } else {
        EvidenceResult::Failed
    };
    let record = EvidenceRecord {
        artifact_kind: EVIDENCE_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        candidate_id: claims.candidate_id,
        content_id: claims.content_id,
        snapshot_sha256: claims.snapshot_sha256,
        id: claims.validation_id,
        argv: claims.argv,
        cwd: claims.cwd,
        repository_set: claims.repository_set,
        result,
        exit_code: Some(claims.exit_code),
        captured_at_unix_seconds: claims.captured_at_unix_seconds,
        payload_locator: claims.payload_locator,
        payload_sha256: claims.payload_sha256,
        provenance: EvidenceProvenance::GithubAttestation {
            repository: claims.repository,
            run_id: claims.run_id,
            check_run_id: claims.check_run_id,
            app_slug: claims.app_slug,
            app_id: claims.app_id,
            workflow: claims.workflow,
            workflow_id: claims.workflow_id,
            signer_workflow: policy.signer_workflow,
            source_digest: policy.source_digest,
            source_ref: policy.source_ref,
            attestation_sha256: verified.attestation_sha256,
        },
    };
    validate_record(&context, &record)?;
    let path = evidence_path(&context, &record.id);
    write_immutable_json(&path, &record)?;
    Ok(path)
}

pub fn verify_evidence<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    evidence_path: &Path,
) -> Result<EvidenceRecord> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    verify_evidence_in_context(&context, evidence_path)
}

pub(crate) fn verify_evidence_in_context(
    context: &SnapshotContext,
    path: &Path,
) -> Result<EvidenceRecord> {
    ensure_external_path(path, &context.external_exclusions)?;
    let (record, _digest): (EvidenceRecord, String) = read_verified_json(path)?;
    validate_record(context, &record)?;
    let expected = evidence_path(context, &record.id);
    if super::storage::absolute_path(path)? != super::storage::absolute_path(&expected)? {
        return Err(DeliveryError::new(
            "evidence path is outside its candidate validation directory",
        ));
    }
    Ok(record)
}

fn validate_ci_claims(
    context: &SnapshotContext,
    required: &super::model::RequiredValidation,
    claims: &CiAttestationClaims,
) -> Result<()> {
    if claims.candidate_id != context.snapshot.candidate_id
        || claims.content_id != context.snapshot.content_id
        || claims.snapshot_sha256 != context.digest
        || claims.argv != required.argv
        || claims.cwd != required.cwd
        || claims.repository_set != context.snapshot.repository_bindings()
    {
        return Err(DeliveryError::new(
            "signed CI attestation does not exactly bind the snapshot, argv, cwd, and repositories",
        ));
    }
    validate_timestamp(claims.captured_at_unix_seconds)?;
    validate_payload_locator(&claims.payload_locator)?;
    validate_sha256(&claims.payload_sha256, "CI payload digest")?;
    validate_bounded_string(&claims.repository, "CI repository provenance")?;
    validate_bounded_string(&claims.run_id, "CI run ID")?;
    validate_bounded_string(&claims.check_run_id, "CI check run ID")?;
    validate_bounded_string(&claims.app_slug, "CI app slug")?;
    if claims.app_id == 0 {
        return Err(DeliveryError::new("CI app ID must be non-zero"));
    }
    validate_bounded_string(&claims.workflow, "CI workflow")?;
    if claims.workflow_id == 0 {
        return Err(DeliveryError::new("CI workflow ID must be non-zero"));
    }
    let expected_publisher = required
        .ci_publisher
        .as_ref()
        .ok_or_else(|| DeliveryError::new("authoritative CI validation has no publisher policy"))?;
    if claims.app_slug != expected_publisher.app_slug
        || claims.app_id != expected_publisher.app_id
        || claims.workflow != expected_publisher.workflow
        || claims.workflow_id != expected_publisher.workflow_id
        || claims.repository != required.cwd.repository
    {
        return Err(DeliveryError::new(
            "signed CI attestation publisher/repository does not match authoritative matrix",
        ));
    }
    if !matches!(
        claims.conclusion.as_str(),
        "success" | "failure" | "cancelled"
    ) {
        return Err(DeliveryError::new(
            "signed CI attestation has an unknown conclusion",
        ));
    }
    Ok(())
}

pub(crate) fn validate_record(context: &SnapshotContext, record: &EvidenceRecord) -> Result<()> {
    if record.artifact_kind != EVIDENCE_ARTIFACT_KIND {
        return Err(DeliveryError::new("invalid evidence artifact_kind"));
    }
    ensure_schema(record.schema_version, "evidence record")?;
    validate_identifier(&record.id, "validation id")?;
    validate_sha256(&record.candidate_id, "evidence candidate ID")?;
    validate_sha256(&record.content_id, "evidence content ID")?;
    validate_sha256(&record.snapshot_sha256, "evidence snapshot digest")?;
    validate_sha256(&record.payload_sha256, "evidence payload digest")?;
    validate_payload_locator(&record.payload_locator)?;
    validate_timestamp(record.captured_at_unix_seconds)?;
    let required = required_validation(context, &record.id)?;
    if record.candidate_id != context.snapshot.candidate_id
        || record.content_id != context.snapshot.content_id
        || record.snapshot_sha256 != context.digest
        || record.argv != required.argv
        || record.cwd != required.cwd
        || record.repository_set != context.snapshot.repository_bindings()
    {
        return Err(DeliveryError::new(
            "evidence does not exactly match candidate authority",
        ));
    }
    match (&record.provenance, required.authority) {
        (
            EvidenceProvenance::LocalRunner {
                runner,
                runner_version,
                run_id,
            },
            ValidationAuthority::LocalRunner,
        ) => {
            if runner != "xtask-local" {
                return Err(DeliveryError::new(
                    "local evidence runner provenance is not xtask-local",
                ));
            }
            validate_bounded_string(runner_version, "runner version")?;
            validate_sha256(run_id, "local run ID")?;
        }
        (
            EvidenceProvenance::GithubAttestation {
                repository,
                run_id,
                check_run_id,
                app_slug,
                app_id,
                workflow,
                workflow_id,
                signer_workflow,
                source_digest,
                source_ref,
                attestation_sha256,
            },
            ValidationAuthority::GithubAttestation,
        ) => {
            validate_bounded_string(repository, "CI repository")?;
            validate_bounded_string(run_id, "CI run ID")?;
            validate_bounded_string(check_run_id, "CI check run ID")?;
            validate_bounded_string(app_slug, "CI app slug")?;
            if *app_id == 0 || *workflow_id == 0 {
                return Err(DeliveryError::new(
                    "CI app and workflow IDs must be non-zero",
                ));
            }
            let repository = context
                .snapshot
                .repository_set
                .iter()
                .find(|repository| repository.id == required.cwd.repository)
                .ok_or_else(|| DeliveryError::new("CI provenance repository is absent"))?;
            if required.ci_signer_workflow.as_ref() != Some(signer_workflow)
                || source_digest != &repository.integration_oid
                || source_ref != &format!("refs/heads/{}", repository.integration_ref)
            {
                return Err(DeliveryError::new(
                    "CI attestation signer/source provenance differs from candidate authority",
                ));
            }
            validate_bounded_string(workflow, "CI workflow")?;
            validate_sha256(attestation_sha256, "CI attestation digest")?;
        }
        _ => {
            return Err(DeliveryError::new(
                "evidence provenance does not match authoritative validation authority",
            ));
        }
    }
    if record.result == EvidenceResult::Passed && record.exit_code != Some(0) {
        return Err(DeliveryError::new(
            "passed evidence must carry exit status 0",
        ));
    }
    if record.result == EvidenceResult::Failed && record.exit_code == Some(0) {
        return Err(DeliveryError::new(
            "failed evidence cannot carry exit status 0",
        ));
    }
    Ok(())
}

fn required_validation<'a>(
    context: &'a SnapshotContext,
    id: &str,
) -> Result<&'a super::model::RequiredValidation> {
    context
        .snapshot
        .required_validations
        .iter()
        .find(|validation| validation.id == id)
        .ok_or_else(|| DeliveryError::new(format!("validation {id} is not required")))
}

fn evidence_path(context: &SnapshotContext, id: &str) -> PathBuf {
    context.layout.evidence_dir().join(format!("{id}.json"))
}

fn discarded_output_digest(stdout: &[u8], stderr: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b-delivery-discarded-output-v1\0");
    hasher.update((stdout.len() as u64).to_be_bytes());
    hasher.update(stdout);
    hasher.update((stderr.len() as u64).to_be_bytes());
    hasher.update(stderr);
    let mut rendered = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        write!(&mut rendered, "{byte:02x}").expect("String write");
    }
    rendered
}

fn validate_timestamp(timestamp: u64) -> Result<()> {
    if timestamp == 0 {
        return Err(DeliveryError::new("evidence timestamp must be non-zero"));
    }
    let now = now_unix_seconds()?;
    if timestamp > now.saturating_add(300) {
        return Err(DeliveryError::new(
            "evidence timestamp is too far in the future",
        ));
    }
    Ok(())
}

fn now_unix_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| DeliveryError::new("system time is before the Unix epoch"))
}

fn local_run_id() -> Result<String> {
    let timestamp = now_unix_seconds()?;
    let nonce = NEXT_LOCAL_RUN.fetch_add(1, Ordering::Relaxed);
    Ok(super::storage::sha256_bytes(
        format!(
            "d2b-delivery-local-run-v1\0{}\0{}\0{}\0{}\0{}\0{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::process::id(),
            timestamp,
            nonce
        )
        .as_bytes(),
    ))
}

fn github_repo_arg(repository: &str) -> Result<String> {
    let parts = repository.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["github.com", owner, name] => Ok(format!("{owner}/{name}")),
        _ => Err(DeliveryError::new("invalid GitHub repository identity")),
    }
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("attestation path is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        command::{CommandOutput, CommandOutputAdapter},
        model::GitObjectFormat,
    };
    use std::{cell::RefCell, fs};

    struct FakeCommand {
        output: CommandOutput,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl CommandOutputAdapter for FakeCommand {
        fn output_with_limits(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
            _limits: CommandLimits,
        ) -> Result<CommandOutput> {
            if program != "gh" {
                return Err(DeliveryError::new("unexpected program"));
            }
            self.calls.borrow_mut().push(args.to_vec());
            Ok(self.output.clone())
        }
    }

    #[test]
    fn discarded_output_hash_is_stream_distinct() {
        assert_ne!(
            discarded_output_digest(b"ab", b"c"),
            discarded_output_digest(b"a", b"bc")
        );
    }

    #[test]
    fn payload_locator_rejects_absolute_or_unbounded_values() {
        assert!(validate_payload_locator("/home/alice/output").is_err());
        assert!(validate_payload_locator(&format!("discarded://{}", "a".repeat(600))).is_err());
    }

    #[test]
    fn github_attestation_adapter_enforces_signer_source_and_subject_digest() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repository")
            .parent()
            .expect("repository parent");
        let path = root.join(format!(
            ".d2b-ci-attestation-test-{}.json",
            std::process::id()
        ));
        let claims = CiAttestationClaims {
            candidate_id: "a".repeat(64),
            content_id: "b".repeat(64),
            snapshot_sha256: "c".repeat(64),
            validation_id: "unit".to_owned(),
            argv: vec!["cargo".to_owned(), "test".to_owned()],
            cwd: LogicalPath {
                repository: "github.com/example/d2b".to_owned(),
                path: ".".to_owned(),
            },
            repository_set: vec![RepositoryBinding {
                id: "github.com/example/d2b".to_owned(),
                object_format: GitObjectFormat::Sha1,
                commit_oid: "d".repeat(40),
                tree_oid: "e".repeat(40),
            }],
            exit_code: 0,
            conclusion: "success".to_owned(),
            captured_at_unix_seconds: now_unix_seconds().expect("time"),
            payload_locator: "github-artifact://run/output".to_owned(),
            payload_sha256: "f".repeat(64),
            repository: "github.com/example/d2b".to_owned(),
            run_id: "1".to_owned(),
            check_run_id: "2".to_owned(),
            app_slug: "github-actions".to_owned(),
            app_id: 15368,
            workflow: "Layer 1".to_owned(),
            workflow_id: 321,
        };
        fs::write(
            &path,
            serde_json::to_vec(&claims).expect("serialize claims"),
        )
        .expect("write claims");
        let digest = sha256_file(&path).expect("claims digest");
        let output = serde_json::to_vec(&serde_json::json!([{
            "attestation": {},
            "verificationResult": {
                "statement": {
                    "subject": [{"digest": {"sha256": digest}}]
                }
            }
        }]))
        .expect("verification JSON");
        let command = FakeCommand {
            output: CommandOutput {
                success: true,
                exit_code: Some(0),
                stdout: output,
                stderr: vec![],
            },
            calls: RefCell::new(vec![]),
        };
        let policy = CiAttestationPolicy {
            repository: "github.com/example/d2b".to_owned(),
            source_digest: "d".repeat(40),
            source_ref: "refs/heads/feature".to_owned(),
            signer_workflow: "github.com/example/d2b/.github/workflows/layer1.yml".to_owned(),
        };
        GithubAttestationVerifier::new(&command)
            .verify(&path, &policy)
            .expect("verified claims");
        let args = command.calls.borrow()[0].join(" ");
        for required in [
            "--signer-workflow",
            "--source-digest",
            "--source-ref",
            "--deny-self-hosted-runners",
        ] {
            assert!(args.contains(required), "missing {required}");
        }
        fs::remove_file(path).expect("cleanup");
    }
}
