use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::fs::PermissionsExt,
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{
        CommandLimits, CommandOutputAdapter, DEFAULT_COMMAND_OUTPUT_BYTES, GitProbe,
        RepositoryProbe,
    },
    model::{
        EVIDENCE_ARTIFACT_KIND, EvidenceResult, LogicalPath, RepositoryBinding,
        ValidationAuthority, ensure_schema, validate_bounded_string, validate_identifier,
        validate_sha256,
    },
    snapshot::{CurrentVerification, SnapshotContext, load_snapshot_context},
    storage::{
        MAX_JSON_BYTES, MAX_PAYLOAD_BYTES, create_private_directory, ensure_external_path,
        read_json, read_json_with_digest, reject_delivery_payload, reject_delivery_payload_content,
        secure_repository_subdir, sha256_file, validate_payload_locator,
    },
};
use rustix::{
    fs::{Mode, OFlags, fchmod, openat},
    io::Errno,
};
use serde::{Deserialize, Serialize};

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
    pub checkout: Option<ValidationCheckoutBinding>,
    pub result: EvidenceResult,
    pub exit_code: Option<i32>,
    pub captured_at_unix_seconds: u64,
    pub payload_locator: String,
    pub payload_sha256: String,
    pub output_capture: Option<EvidenceOutputCapture>,
    pub provenance: EvidenceProvenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceOutputCapture {
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationCheckoutBinding {
    pub repository: String,
    pub commit_oid: String,
    pub tree_oid: String,
    pub source_read_only: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
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
        conclusion: String,
        attestation_locator: String,
        attestation_artifact_sha256: String,
        attestation_bundle_sha256: String,
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
    pub artifact_sha256: String,
    pub bundle_sha256: String,
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
        artifact_path: &Path,
        bundle_path: &Path,
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
        artifact_path: &Path,
        bundle_path: &Path,
        policy: &CiAttestationPolicy,
    ) -> Result<VerifiedCiAttestation> {
        let repository = github_repo_arg(&policy.repository)?;
        let artifact = path_string(artifact_path)?;
        let bundle = path_string(bundle_path)?;
        let (claims_before, digest_before): (CiAttestationClaims, String) =
            read_json_with_digest(artifact_path)?;
        let bundle_before = sha256_file(bundle_path)?;
        let output = self.command.output(
            "gh",
            &[
                "attestation".to_owned(),
                "verify".to_owned(),
                artifact,
                "--repo".to_owned(),
                repository,
                "--bundle".to_owned(),
                bundle,
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
            read_json_with_digest(artifact_path)?;
        let bundle_after = sha256_file(bundle_path)?;
        if digest_before != digest_after
            || bundle_before != bundle_after
            || claims_before != claims_after
        {
            return Err(DeliveryError::new(
                "GitHub attestation artifact or bundle changed while it was being verified",
            ));
        }
        Ok(VerifiedCiAttestation {
            claims: claims_after,
            artifact_sha256: digest_after,
            bundle_sha256: bundle_after,
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
    let repository = context
        .snapshot
        .repository_set
        .iter()
        .find(|repository| repository.id == required.cwd.repository)
        .ok_or_else(|| DeliveryError::new("validation repository binding is missing"))?;
    let run_id = local_run_id()?;
    let execution_root = context
        .layout
        .validation_execution_dir(validation_id, &run_id);
    create_private_directory(&execution_root)?;
    let socket_root = PathBuf::from("/tmp").join(format!("d2b-validation-{}", &run_id[..24]));
    create_private_directory(&socket_root)?;
    let execution = ValidationExecution::new(execution_root.clone(), socket_root.clone());
    let source = execution_root.join("source");
    let output_root = execution_root.join("output");
    create_private_directory(&output_root)?;
    create_private_directory(&output_root.join("tmp"))?;
    create_private_directory(&output_root.join("home"))?;
    create_private_directory(&output_root.join("cargo-home"))?;
    create_private_directory(&output_root.join("cargo-target"))?;
    create_private_directory(&output_root.join("layer1-logs"))?;
    create_private_directory(&output_root.join("test-scratch"))?;
    run_checked(
        runner,
        "git",
        &[
            "clone".to_owned(),
            "--no-hardlinks".to_owned(),
            "--no-checkout".to_owned(),
            "--quiet".to_owned(),
            "--".to_owned(),
            path_string(repository_root)?,
            path_string(&source)?,
        ],
        None,
        "cannot create detached validation checkout",
    )?;
    run_checked(
        runner,
        "git",
        &[
            "-C".to_owned(),
            path_string(&source)?,
            "checkout".to_owned(),
            "--detach".to_owned(),
            "--quiet".to_owned(),
            repository.integration_oid.clone(),
        ],
        None,
        "cannot select validation checkout commit",
    )?;
    verify_checkout_identity(runner, &source, repository)?;
    make_source_read_only(&source)?;
    verify_source_read_only(&source)?;
    let cwd = secure_repository_subdir(&source, Path::new(&required.cwd.path))?;
    let environment = BTreeMap::from([
        (
            OsString::from("D2B_VALIDATION_OUTPUT_DIR"),
            output_root.as_os_str().to_owned(),
        ),
        (
            OsString::from("CARGO_TARGET_DIR"),
            output_root.join("cargo-target").into_os_string(),
        ),
        (
            OsString::from("CARGO_HOME"),
            output_root.join("cargo-home").into_os_string(),
        ),
        (
            OsString::from("D2B_LAYER1_LOG_DIR"),
            output_root.join("layer1-logs").into_os_string(),
        ),
        (
            OsString::from("D2B_VALIDATION_SOCKET_DIR"),
            socket_root.into_os_string(),
        ),
        (
            OsString::from("TMPDIR"),
            output_root.join("tmp").into_os_string(),
        ),
        (
            OsString::from("HOME"),
            output_root.join("home").into_os_string(),
        ),
    ]);
    let output = runner.output_with_environment(
        &required.argv[0],
        &required.argv[1..],
        Some(&cwd),
        &environment,
        CommandLimits {
            stdout_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            timeout: Duration::from_secs(required.timeout_seconds),
        },
    )?;
    verify_checkout_identity(runner, &source, repository)?;
    verify_source_read_only(&source)?;
    let result = if output.success {
        EvidenceResult::Passed
    } else {
        EvidenceResult::Failed
    };
    let payload = retained_output_payload(&output.stdout, &output.stderr)?;
    let payload_relative =
        Path::new("validation-output").join(format!("{validation_id}-{run_id}.bin"));
    let payload_sha256 = context
        .layout
        .write_candidate_file(&payload_relative, &payload)?;
    let output_capture = EvidenceOutputCapture {
        stdout_bytes: output.stdout.len() as u64,
        stderr_bytes: output.stderr.len() as u64,
        stdout_sha256: super::storage::sha256_bytes(&output.stdout),
        stderr_sha256: super::storage::sha256_bytes(&output.stderr),
        stdout_truncated: false,
        stderr_truncated: false,
    };
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
        checkout: Some(ValidationCheckoutBinding {
            repository: repository.id.clone(),
            commit_oid: repository.integration_oid.clone(),
            tree_oid: repository.integration_tree_oid.clone(),
            source_read_only: true,
        }),
        result,
        exit_code: output.exit_code,
        captured_at_unix_seconds: now_unix_seconds()?,
        payload_locator: format!("private://validation-output/{validation_id}-{run_id}.bin"),
        payload_sha256,
        output_capture: Some(output_capture),
        provenance: EvidenceProvenance::LocalRunner {
            runner: "xtask-local".to_owned(),
            runner_version: env!("CARGO_PKG_VERSION").to_owned(),
            run_id,
        },
    };
    validate_record(&context, &record)?;
    drop(execution);
    let path = evidence_path(&context, validation_id);
    context.layout.write_candidate_json(
        Path::new("validation").join(format!("{validation_id}.json")),
        &record,
    )?;
    Ok(path)
}

struct ValidationExecution {
    root: PathBuf,
    socket_root: PathBuf,
}

impl ValidationExecution {
    fn new(root: PathBuf, socket_root: PathBuf) -> Self {
        Self { root, socket_root }
    }
}

impl Drop for ValidationExecution {
    fn drop(&mut self) {
        let _ = make_tree_writable(&self.root);
        let _ = fs::remove_dir_all(&self.root);
        let _ = make_tree_writable(&self.socket_root);
        let _ = fs::remove_dir_all(&self.socket_root);
    }
}

fn run_checked<A: CommandOutputAdapter>(
    runner: &A,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    message: &str,
) -> Result<()> {
    let output = runner.output(program, args, cwd)?;
    if !output.success {
        return Err(DeliveryError::new(message));
    }
    Ok(())
}

fn verify_checkout_identity<A: CommandOutputAdapter>(
    runner: &A,
    source: &Path,
    repository: &super::model::RepositoryRecord,
) -> Result<()> {
    let probe = GitProbe::new(runner);
    let commit = probe.resolve_commit(source, "HEAD")?;
    let tree = probe.tree_for_commit(source, &commit)?;
    if commit != repository.integration_oid
        || tree != repository.integration_tree_oid
        || probe.is_dirty(source)?
    {
        return Err(DeliveryError::new(
            "detached validation checkout identity or cleanliness changed",
        ));
    }
    Ok(())
}

fn make_source_read_only(path: &Path) -> Result<()> {
    let fd = open_tree_root(path)?;
    chmod_tree_fd(&fd, false)
}

fn verify_source_read_only(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.permissions().mode() & 0o222 != 0 {
        return Err(DeliveryError::new(
            "validation checkout source is not read-only",
        ));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            verify_source_read_only(&entry?.path())?;
        }
    } else if !metadata.is_file() {
        return Err(DeliveryError::new(
            "validation checkout contains a non-regular filesystem entry",
        ));
    }
    Ok(())
}

fn make_tree_writable(path: &Path) -> Result<()> {
    let fd = open_tree_root(path)?;
    chmod_tree_fd(&fd, true)
}

fn open_tree_root(path: &Path) -> Result<OwnedFd> {
    let parent = path
        .parent()
        .ok_or_else(|| DeliveryError::new("validation tree path has no parent"))?;
    let parent_fd = rustix::fs::open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| DeliveryError::new(format!("cannot anchor validation tree: {error}")))?;
    openat(
        parent_fd.as_fd(),
        path.file_name()
            .ok_or_else(|| DeliveryError::new("validation tree path has no filename"))?,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| DeliveryError::new(format!("cannot open validation tree root: {error}")))
}

fn chmod_tree_fd(fd: &OwnedFd, writable: bool) -> Result<()> {
    let file = File::from(fd.try_clone()?);
    let metadata = file.metadata()?;
    if metadata.is_dir() {
        if writable {
            fchmod(fd, Mode::from_raw_mode(0o700)).map_err(|error| {
                DeliveryError::new(format!(
                    "cannot make validation directory writable: {error}"
                ))
            })?;
        }
        let proc_path = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            fd.as_raw_fd()
        ));
        let mut names = Vec::new();
        for entry in fs::read_dir(proc_path)? {
            names.push(entry?.file_name());
        }
        for name in names {
            let child = openat(
                fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                if error == Errno::LOOP {
                    DeliveryError::new("validation checkout contains a symlink")
                } else {
                    DeliveryError::new(format!(
                        "cannot open validation checkout entry without following links: {error}"
                    ))
                }
            })?;
            chmod_tree_fd(&child, writable)?;
        }
        if !writable {
            fchmod(fd, Mode::from_raw_mode(0o500)).map_err(|error| {
                DeliveryError::new(format!(
                    "cannot make validation directory read-only: {error}"
                ))
            })?;
        }
    } else if metadata.is_file() {
        let mode = if writable {
            0o600
        } else if metadata.permissions().mode() & 0o111 == 0 {
            0o400
        } else {
            0o500
        };
        fchmod(fd, Mode::from_raw_mode(mode)).map_err(|error| {
            DeliveryError::new(format!("cannot secure validation file mode: {error}"))
        })?;
    } else {
        return Err(DeliveryError::new(
            "validation checkout contains a non-regular filesystem entry",
        ));
    }
    Ok(())
}

pub fn import_ci_evidence<P: RepositoryProbe, V: CiAttestationVerifier>(
    probe: &P,
    verifier: &V,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    artifact_path: &Path,
    bundle_path: &Path,
    payload_path: Option<&Path>,
) -> Result<PathBuf> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    ensure_external_path(artifact_path, &context.external_exclusions)?;
    ensure_external_path(bundle_path, &context.external_exclusions)?;
    reject_delivery_payload(artifact_path, &context.layout.root)?;
    reject_delivery_payload(bundle_path, &context.layout.root)?;
    if super::storage::absolute_path(artifact_path)? == super::storage::absolute_path(bundle_path)?
    {
        return Err(DeliveryError::new(
            "CI attestation artifact and bundle must be distinct files",
        ));
    }
    let staged_artifact =
        context
            .layout
            .stage_external_file(artifact_path, "ci-artifact", MAX_JSON_BYTES)?;
    let staged_bundle =
        context
            .layout
            .stage_external_file(bundle_path, "ci-bundle", MAX_PAYLOAD_BYTES)?;
    let staged_payload = if let Some(payload) = payload_path {
        ensure_external_path(payload, &context.external_exclusions)?;
        reject_delivery_payload(payload, &context.layout.root)?;
        Some(
            context
                .layout
                .stage_external_file(payload, "ci-payload", MAX_PAYLOAD_BYTES)?,
        )
    } else {
        None
    };
    if let Some(payload) = &staged_payload {
        reject_delivery_payload_content(payload.path())?;
    }
    let asserted_claims: CiAttestationClaims = read_json(staged_artifact.path())?;
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
    let verified = verifier.verify(staged_artifact.path(), staged_bundle.path(), &policy)?;
    let claims = verified.claims;
    if claims != asserted_claims {
        return Err(DeliveryError::new(
            "verified CI claims changed during attestation verification",
        ));
    }
    validate_ci_claims(&context, required, &claims)?;
    if let Some(payload) = &staged_payload
        && payload.digest() != claims.payload_sha256
    {
        return Err(DeliveryError::new(
            "retrieved CI payload digest does not match signed attestation",
        ));
    }
    let retained_artifact_digest = context.layout.retain_candidate_file(
        staged_artifact.path(),
        Path::new("ci-attestations").join(format!("{}.artifact.json", claims.validation_id)),
    )?;
    let retained_bundle_digest = context.layout.retain_candidate_file(
        staged_bundle.path(),
        Path::new("ci-attestations").join(format!("{}.bundle.jsonl", claims.validation_id)),
    )?;
    if retained_artifact_digest != verified.artifact_sha256
        || retained_bundle_digest != verified.bundle_sha256
    {
        return Err(DeliveryError::new(
            "retained CI attestation artifact or bundle differs from verified inputs",
        ));
    }
    let result = if claims.exit_code == 0 && claims.conclusion == "success" {
        EvidenceResult::Passed
    } else {
        EvidenceResult::Failed
    };
    let attestation_locator = format!(
        "github-attestation://{}/{}",
        claims.repository, verified.bundle_sha256
    );
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
        checkout: None,
        result,
        exit_code: Some(claims.exit_code),
        captured_at_unix_seconds: claims.captured_at_unix_seconds,
        payload_locator: claims.payload_locator,
        payload_sha256: claims.payload_sha256,
        output_capture: None,
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
            conclusion: claims.conclusion,
            attestation_locator,
            attestation_artifact_sha256: verified.artifact_sha256,
            attestation_bundle_sha256: verified.bundle_sha256,
        },
    };
    validate_record(&context, &record)?;
    let path = evidence_path(&context, &record.id);
    context.layout.write_candidate_json(
        Path::new("validation").join(format!("{}.json", record.id)),
        &record,
    )?;
    Ok(path)
}

pub fn verify_evidence<P: RepositoryProbe>(
    probe: &P,
    verifier: &dyn CiAttestationVerifier,
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
    verify_evidence_in_context(&context, evidence_path, verifier)
}

pub(crate) fn verify_evidence_in_context(
    context: &SnapshotContext,
    path: &Path,
    verifier: &dyn CiAttestationVerifier,
) -> Result<EvidenceRecord> {
    ensure_external_path(path, &context.external_exclusions)?;
    let id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DeliveryError::new("evidence filename is not UTF-8"))?;
    validate_identifier(id, "validation id")?;
    let expected = evidence_path(context, id);
    if super::storage::absolute_path(path)? != super::storage::absolute_path(&expected)? {
        return Err(DeliveryError::new(
            "evidence path is outside its candidate validation directory",
        ));
    }
    let (record, _digest): (EvidenceRecord, String) = context
        .layout
        .read_candidate_json(Path::new("validation").join(format!("{id}.json")))?;
    validate_record(context, &record)?;
    if record.id != id {
        return Err(DeliveryError::new(
            "evidence filename does not match validation ID",
        ));
    }
    reverify_ci_attestation(context, &record, verifier)?;
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

fn reverify_ci_attestation(
    context: &SnapshotContext,
    record: &EvidenceRecord,
    verifier: &dyn CiAttestationVerifier,
) -> Result<()> {
    let EvidenceProvenance::GithubAttestation {
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
        conclusion,
        attestation_locator,
        attestation_artifact_sha256,
        attestation_bundle_sha256,
    } = &record.provenance
    else {
        return Ok(());
    };
    let required = required_validation(context, &record.id)?;
    let policy = CiAttestationPolicy {
        repository: repository.clone(),
        source_digest: source_digest.clone(),
        source_ref: source_ref.clone(),
        signer_workflow: signer_workflow.clone(),
    };
    let artifact_relative =
        Path::new("ci-attestations").join(format!("{}.artifact.json", record.id));
    let bundle_relative = Path::new("ci-attestations").join(format!("{}.bundle.jsonl", record.id));
    let retained_artifact = context.layout.anchored_path(&artifact_relative)?;
    let retained_bundle = context.layout.anchored_path(&bundle_relative)?;
    if context.layout.verify_candidate_digest(&artifact_relative)? != *attestation_artifact_sha256
        || context.layout.verify_candidate_digest(&bundle_relative)? != *attestation_bundle_sha256
    {
        return Err(DeliveryError::new(
            "retained CI attestation artifact or bundle digest changed",
        ));
    }
    let verified = verifier.verify(&retained_artifact, &retained_bundle, &policy)?;
    if verified.artifact_sha256 != *attestation_artifact_sha256
        || verified.bundle_sha256 != *attestation_bundle_sha256
    {
        return Err(DeliveryError::new(
            "CI attestation verifier returned different artifact or bundle digests",
        ));
    }
    validate_ci_claims(context, required, &verified.claims)?;
    let claims = &verified.claims;
    let expected_locator = format!("github-attestation://{repository}/{attestation_bundle_sha256}");
    if claims.candidate_id != record.candidate_id
        || claims.content_id != record.content_id
        || claims.snapshot_sha256 != record.snapshot_sha256
        || claims.validation_id != record.id
        || claims.argv != record.argv
        || claims.cwd != record.cwd
        || claims.repository_set != record.repository_set
        || Some(claims.exit_code) != record.exit_code
        || claims.captured_at_unix_seconds != record.captured_at_unix_seconds
        || claims.payload_locator != record.payload_locator
        || claims.payload_sha256 != record.payload_sha256
        || &claims.repository != repository
        || &claims.run_id != run_id
        || &claims.check_run_id != check_run_id
        || &claims.app_slug != app_slug
        || claims.app_id != *app_id
        || &claims.workflow != workflow
        || claims.workflow_id != *workflow_id
        || &claims.conclusion != conclusion
        || attestation_locator != &expected_locator
    {
        return Err(DeliveryError::new(
            "reverified CI attestation does not reproduce the derived evidence record",
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
    let repository = context
        .snapshot
        .repository_set
        .iter()
        .find(|repository| repository.id == required.cwd.repository)
        .ok_or_else(|| DeliveryError::new("evidence checkout repository is absent"))?;
    match (required.authority, &record.checkout) {
        (ValidationAuthority::LocalRunner, Some(checkout))
            if checkout.repository == repository.id
                && checkout.commit_oid == repository.integration_oid
                && checkout.tree_oid == repository.integration_tree_oid
                && checkout.source_read_only => {}
        (ValidationAuthority::GithubAttestation, None) => {}
        _ => {
            return Err(DeliveryError::new(
                "evidence checkout binding does not match validation authority",
            ));
        }
    }
    match (required.authority, &record.output_capture) {
        (ValidationAuthority::LocalRunner, Some(capture)) => {
            validate_sha256(&capture.stdout_sha256, "validation stdout digest")?;
            validate_sha256(&capture.stderr_sha256, "validation stderr digest")?;
            if capture.stdout_bytes > DEFAULT_COMMAND_OUTPUT_BYTES as u64
                || capture.stderr_bytes > DEFAULT_COMMAND_OUTPUT_BYTES as u64
                || capture.stdout_truncated
                || capture.stderr_truncated
                || !record
                    .payload_locator
                    .starts_with("private://validation-output/")
            {
                return Err(DeliveryError::new(
                    "local validation output capture metadata is invalid",
                ));
            }
        }
        (ValidationAuthority::GithubAttestation, None) => {}
        _ => {
            return Err(DeliveryError::new(
                "evidence output capture does not match validation authority",
            ));
        }
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
                conclusion,
                attestation_locator,
                attestation_artifact_sha256,
                attestation_bundle_sha256,
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
            validate_bounded_string(conclusion, "CI conclusion")?;
            if (record.result == EvidenceResult::Passed)
                != (record.exit_code == Some(0) && conclusion == "success")
            {
                return Err(DeliveryError::new(
                    "CI evidence result differs from its signed conclusion",
                ));
            }
            if !attestation_locator.starts_with("github-attestation://")
                || attestation_locator.len() > 512
                || attestation_locator.contains("..")
                || attestation_locator.contains(char::is_whitespace)
            {
                return Err(DeliveryError::new("CI attestation locator is invalid"));
            }
            validate_sha256(
                attestation_artifact_sha256,
                "CI attestation artifact digest",
            )?;
            validate_sha256(attestation_bundle_sha256, "CI attestation bundle digest")?;
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

fn retained_output_payload(stdout: &[u8], stderr: &[u8]) -> Result<Vec<u8>> {
    let capacity = 32_usize
        .checked_add(stdout.len())
        .and_then(|size| size.checked_add(stderr.len()))
        .ok_or_else(|| DeliveryError::new("validation output payload length overflow"))?;
    let mut payload = Vec::with_capacity(capacity);
    payload.extend_from_slice(b"d2b-output-v1\0");
    payload.extend_from_slice(&(stdout.len() as u64).to_be_bytes());
    payload.extend_from_slice(stdout);
    payload.extend_from_slice(&(stderr.len() as u64).to_be_bytes());
    payload.extend_from_slice(stderr);
    Ok(payload)
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
    use std::{cell::RefCell, fs, os::unix::fs::symlink};

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
    fn retained_output_payload_is_stream_distinct() {
        assert_ne!(
            retained_output_payload(b"ab", b"c").expect("payload"),
            retained_output_payload(b"a", b"bc").expect("payload")
        );
    }

    #[test]
    fn payload_locator_rejects_absolute_or_unbounded_values() {
        assert!(validate_payload_locator("/home/alice/output").is_err());
        assert!(validate_payload_locator(&format!("discarded://{}", "a".repeat(600))).is_err());
    }

    #[test]
    fn chmod_walk_never_follows_symlink_entries() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository")
            .parent()
            .expect("repository parent")
            .join(format!(".d2b-chmod-tree-test-{}", std::process::id()));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).expect("source");
        fs::write(&target, b"target").expect("target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("target mode");
        symlink(&target, source.join("link")).expect("symlink");
        let error = make_source_read_only(&source).expect_err("symlink rejected");
        assert!(error.to_string().contains("symlink"));
        assert_eq!(
            fs::metadata(&target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).expect("cleanup");
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
        let bundle = root.join(format!(
            ".d2b-ci-attestation-test-{}.bundle.jsonl",
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
        fs::write(
            &bundle,
            b"{\"mediaType\":\"application/vnd.dev.sigstore.bundle+json\"}\n",
        )
        .expect("write bundle");
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
            .verify(&path, &bundle, &policy)
            .expect("verified claims");
        let args = command.calls.borrow()[0].join(" ");
        for required in [
            "--signer-workflow",
            "--source-digest",
            "--source-ref",
            "--deny-self-hosted-runners",
            "--bundle",
        ] {
            assert!(args.contains(required), "missing {required}");
        }
        fs::remove_file(path).expect("cleanup");
        fs::remove_file(bundle).expect("cleanup");
    }
}
