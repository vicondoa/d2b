use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CommandOutputAdapter, RepositoryProbe},
    model::{
        MAX_RECOMMENDATIONS, MAX_STRING_BYTES, PANEL_ATTESTATION_ARTIFACT_KIND, PANEL_MODEL_POLICY,
        PANEL_PROVIDER_POLICY, PANEL_REQUEST_ARTIFACT_KIND, PANEL_ROLES, PANEL_SIGNATURE_POLICY,
        PanelRole, ensure_schema, validate_bounded_string, validate_sha256,
    },
    snapshot::{CurrentVerification, SnapshotContext, load_snapshot_context},
    storage::{
        MAX_JSON_BYTES, MAX_PAYLOAD_BYTES, ensure_external_path, read_json_with_digest, sha256_file,
    },
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRequest {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub candidate_id: String,
    pub content_id: String,
    pub snapshot_sha256: String,
    pub required_roles: Vec<PanelRole>,
    pub required_provider: String,
    pub required_model_version: String,
    pub required_signature_algorithm: String,
    pub required_trust_root_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelAttestation {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub role: PanelRole,
    pub candidate_id: String,
    pub content_id: String,
    pub snapshot_sha256: String,
    pub model_version: String,
    pub provider: String,
    pub run_id: String,
    pub receipt_locator: String,
    pub output_sha256: String,
    pub signoff: bool,
    pub recommendations: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPanelReceipt {
    pub claims: PanelAttestation,
    pub receipt_sha256: String,
    pub signature_sha256: String,
    pub trust_root_sha256: String,
}

pub trait PanelReceiptVerifier {
    fn verify(
        &self,
        receipt_path: &Path,
        signature_path: &Path,
        trust_root_path: &Path,
    ) -> Result<VerifiedPanelReceipt>;
}

#[derive(Debug)]
pub struct OpenSslPanelReceiptVerifier<'a, A> {
    command: &'a A,
}

impl<'a, A> OpenSslPanelReceiptVerifier<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> PanelReceiptVerifier for OpenSslPanelReceiptVerifier<'_, A> {
    fn verify(
        &self,
        receipt_path: &Path,
        signature_path: &Path,
        trust_root_path: &Path,
    ) -> Result<VerifiedPanelReceipt> {
        let (claims_before, receipt_before): (PanelAttestation, String) =
            read_json_with_digest(receipt_path)?;
        let signature_before = sha256_file(signature_path)?;
        let trust_before = sha256_file(trust_root_path)?;
        let key_check = self.command.output(
            "openssl",
            &[
                "rsa".to_owned(),
                "-pubin".to_owned(),
                "-in".to_owned(),
                path_string(trust_root_path)?,
                "-noout".to_owned(),
            ],
            None,
        )?;
        if !key_check.success {
            return Err(DeliveryError::new(
                "panel trust root is not an externally supplied RSA public key",
            ));
        }
        let output = self.command.output(
            "openssl",
            &[
                "dgst".to_owned(),
                "-sha256".to_owned(),
                "-verify".to_owned(),
                path_string(trust_root_path)?,
                "-signature".to_owned(),
                path_string(signature_path)?,
                path_string(receipt_path)?,
            ],
            None,
        )?;
        if !output.success {
            return Err(DeliveryError::new(
                "panel receipt detached-signature verification failed",
            ));
        }
        let (claims_after, receipt_after): (PanelAttestation, String) =
            read_json_with_digest(receipt_path)?;
        let signature_after = sha256_file(signature_path)?;
        let trust_after = sha256_file(trust_root_path)?;
        if claims_before != claims_after
            || receipt_before != receipt_after
            || signature_before != signature_after
            || trust_before != trust_after
        {
            return Err(DeliveryError::new(
                "panel receipt inputs changed during signature verification",
            ));
        }
        Ok(VerifiedPanelReceipt {
            claims: claims_after,
            receipt_sha256: receipt_after,
            signature_sha256: signature_after,
            trust_root_sha256: trust_after,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredPanelReceipt {
    pub claims: PanelAttestation,
    pub receipt_sha256: String,
    pub signature_sha256: String,
    pub trust_root_sha256: String,
}

pub fn create_panel_request<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
) -> Result<PathBuf> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    let request = PanelRequest {
        artifact_kind: PANEL_REQUEST_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        candidate_id: context.snapshot.candidate_id.clone(),
        content_id: context.snapshot.content_id.clone(),
        snapshot_sha256: context.digest,
        required_roles: PANEL_ROLES.to_vec(),
        required_provider: PANEL_PROVIDER_POLICY.to_owned(),
        required_model_version: PANEL_MODEL_POLICY.to_owned(),
        required_signature_algorithm: PANEL_SIGNATURE_POLICY.to_owned(),
        required_trust_root_sha256: context.snapshot.panel_trust_root_sha256.clone(),
    };
    let path = context.layout.panel_request();
    context
        .layout
        .write_candidate_json("panel-request.json", &request)?;
    Ok(path)
}

pub fn validate_and_store_panel<P: RepositoryProbe>(
    probe: &P,
    verifier: &dyn PanelReceiptVerifier,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    records_dir: &Path,
    trust_root_path: &Path,
) -> Result<Vec<PanelAttestation>> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    ensure_external_path(records_dir, &context.external_exclusions)?;
    ensure_external_path(trust_root_path, &context.external_exclusions)?;
    let staged_trust =
        context
            .layout
            .stage_external_file(trust_root_path, "panel-trust", MAX_PAYLOAD_BYTES)?;
    if staged_trust.digest() != context.snapshot.panel_trust_root_sha256 {
        return Err(DeliveryError::new(
            "panel trust root does not match checked-in candidate authority",
        ));
    }
    let mut staged_pairs = Vec::with_capacity(PANEL_ROLES.len());
    for receipt_path in receipt_files(records_dir)? {
        let role_name = receipt_path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DeliveryError::new("panel receipt filename is not UTF-8"))?;
        let signature_path = receipt_path.with_extension("sig");
        ensure_external_path(&receipt_path, &context.external_exclusions)?;
        ensure_external_path(&signature_path, &context.external_exclusions)?;
        let receipt = context.layout.stage_external_file(
            &receipt_path,
            &format!("panel-{role_name}-receipt"),
            MAX_JSON_BYTES,
        )?;
        let signature = context.layout.stage_external_file(
            &signature_path,
            &format!("panel-{role_name}-signature"),
            MAX_PAYLOAD_BYTES,
        )?;
        staged_pairs.push((role_name.to_owned(), receipt, signature));
    }
    let verification_paths = staged_pairs
        .iter()
        .map(|(_, receipt, signature)| {
            (receipt.path().to_path_buf(), signature.path().to_path_buf())
        })
        .collect();
    let records =
        read_and_validate_paths(verification_paths, staged_trust.path(), &context, verifier)?;
    let trust_root_sha256 = context
        .layout
        .retain_candidate_file(staged_trust.path(), "panel/trust-root.pem")?;
    for record in &records {
        let role = record.claims.role.as_str();
        let (_, receipt, signature) = staged_pairs
            .iter()
            .find(|(name, _, _)| name == role)
            .ok_or_else(|| DeliveryError::new("panel receipt filename does not match role"))?;
        let receipt_sha256 = context.layout.retain_candidate_file(
            receipt.path(),
            Path::new("panel").join(format!("{role}.json")),
        )?;
        let signature_sha256 = context.layout.retain_candidate_file(
            signature.path(),
            Path::new("panel").join(format!("{role}.sig")),
        )?;
        if receipt_sha256 != record.receipt_sha256
            || signature_sha256 != record.signature_sha256
            || trust_root_sha256 != record.trust_root_sha256
        {
            return Err(DeliveryError::new(
                "panel receipt inputs changed before immutable retention",
            ));
        }
    }
    read_stored_panel(&context, verifier)?;
    Ok(records.into_iter().map(|record| record.claims).collect())
}

pub(crate) fn read_stored_panel(
    context: &SnapshotContext,
    verifier: &dyn PanelReceiptVerifier,
) -> Result<Vec<StoredPanelReceipt>> {
    let panel_dir = context.layout.panel_dir();
    let trust_relative = Path::new("panel/trust-root.pem");
    let trust_root = context.layout.anchored_path(trust_relative)?;
    ensure_external_path(&panel_dir, &context.external_exclusions)?;
    let pairs = PANEL_ROLES
        .iter()
        .map(|role| {
            let role = role.as_str();
            Ok((
                context
                    .layout
                    .anchored_path(Path::new("panel").join(format!("{role}.json")))?,
                context
                    .layout
                    .anchored_path(Path::new("panel").join(format!("{role}.sig")))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let records = read_and_validate_paths(pairs, &trust_root, context, verifier)?;
    let trust_digest = context.layout.verify_candidate_digest(trust_relative)?;
    if trust_digest != context.snapshot.panel_trust_root_sha256 {
        return Err(DeliveryError::new(
            "stored panel trust root differs from checked-in candidate authority",
        ));
    }
    for record in &records {
        let role = record.claims.role.as_str();
        if context
            .layout
            .verify_candidate_digest(Path::new("panel").join(format!("{role}.json")))?
            != record.receipt_sha256
            || context
                .layout
                .verify_candidate_digest(Path::new("panel").join(format!("{role}.sig")))?
                != record.signature_sha256
            || record.trust_root_sha256 != trust_digest
        {
            return Err(DeliveryError::new(
                "stored panel receipt digest binding changed",
            ));
        }
    }
    Ok(records)
}

fn read_and_validate_paths(
    paths: Vec<(PathBuf, PathBuf)>,
    trust_root_path: &Path,
    context: &SnapshotContext,
    verifier: &dyn PanelReceiptVerifier,
) -> Result<Vec<StoredPanelReceipt>> {
    let mut by_role = BTreeMap::new();
    let mut runs = BTreeSet::<(String, String)>::new();
    for (receipt_path, signature_path) in paths {
        let verified = verifier.verify(&receipt_path, &signature_path, trust_root_path)?;
        validate_record(context, &verified.claims)?;
        if !runs.insert((
            verified.claims.provider.clone(),
            verified.claims.run_id.clone(),
        )) {
            return Err(DeliveryError::new(
                "panel provenance repeats a provider/run ID",
            ));
        }
        let stored = StoredPanelReceipt {
            claims: verified.claims,
            receipt_sha256: verified.receipt_sha256,
            signature_sha256: verified.signature_sha256,
            trust_root_sha256: verified.trust_root_sha256,
        };
        let role = stored.claims.role;
        if by_role.insert(role, stored).is_some() {
            return Err(DeliveryError::new(format!(
                "duplicate panel role in {}",
                receipt_path.display()
            )));
        }
    }
    let expected = PANEL_ROLES.into_iter().collect::<BTreeSet<_>>();
    let actual = by_role.keys().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(DeliveryError::new(
            "panel does not contain the exact ten-role roster",
        ));
    }
    Ok(PANEL_ROLES
        .iter()
        .map(|role| {
            by_role
                .remove(role)
                .expect("all exact panel roles were checked")
        })
        .collect())
}

fn validate_record(context: &SnapshotContext, record: &PanelAttestation) -> Result<()> {
    if record.artifact_kind != PANEL_ATTESTATION_ARTIFACT_KIND {
        return Err(DeliveryError::new("invalid panel receipt artifact_kind"));
    }
    ensure_schema(record.schema_version, "panel receipt")?;
    validate_sha256(&record.candidate_id, "panel candidate ID")?;
    validate_sha256(&record.content_id, "panel content ID")?;
    validate_sha256(&record.snapshot_sha256, "panel snapshot digest")?;
    validate_sha256(&record.output_sha256, "panel output digest")?;
    if record.candidate_id != context.snapshot.candidate_id
        || record.content_id != context.snapshot.content_id
        || record.snapshot_sha256 != context.digest
    {
        return Err(DeliveryError::new(format!(
            "panel role {} is bound to a different candidate",
            record.role.as_str()
        )));
    }
    if record.provider != PANEL_PROVIDER_POLICY || record.model_version != PANEL_MODEL_POLICY {
        return Err(DeliveryError::new(format!(
            "panel role {} violates provider/model program policy",
            record.role.as_str()
        )));
    }
    validate_bounded_string(&record.provider, "panel provider")?;
    validate_bounded_string(&record.run_id, "panel run ID")?;
    validate_receipt_locator(&record.receipt_locator)?;
    if record.provider.contains(char::is_whitespace) || record.run_id.contains(char::is_whitespace)
    {
        return Err(DeliveryError::new(
            "panel provider and run ID must be machine identifiers",
        ));
    }
    if record.recommendations.len() > MAX_RECOMMENDATIONS {
        return Err(DeliveryError::new(
            "panel recommendation count is oversized",
        ));
    }
    for recommendation in &record.recommendations {
        if recommendation.trim().is_empty() || recommendation.len() > MAX_STRING_BYTES {
            return Err(DeliveryError::new(format!(
                "panel role {} has an empty or oversized recommendation",
                record.role.as_str()
            )));
        }
    }
    if record.signoff != record.recommendations.is_empty() {
        return Err(DeliveryError::new(format!(
            "panel role {} must sign off if and only if recommendations are empty",
            record.role.as_str()
        )));
    }
    Ok(())
}

fn validate_receipt_locator(locator: &str) -> Result<()> {
    if !locator.starts_with("github-copilot://")
        || locator.len() > 512
        || locator.contains("..")
        || locator.contains(char::is_whitespace)
        || !locator.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'/' | b'.' | b'_' | b'-')
        })
    {
        return Err(DeliveryError::new(
            "panel receipt locator is not a bounded GitHub Copilot provider locator",
        ));
    }
    Ok(())
}

fn receipt_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        DeliveryError::new(format!(
            "cannot inspect panel receipt directory {}: {error}",
            directory.display()
        ))
    })?;
    if !metadata.file_type().is_dir() {
        return Err(DeliveryError::new(
            "panel receipts path must be an external directory",
        ));
    }
    let mut json = Vec::with_capacity(PANEL_ROLES.len());
    let mut signatures = BTreeSet::new();
    let mut entries = 0_usize;
    for entry in fs::read_dir(directory)? {
        entries += 1;
        if entries > PANEL_ROLES.len() * 4 + 3 {
            return Err(DeliveryError::new(
                "panel receipt directory contains too many entries",
            ));
        }
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(DeliveryError::new(format!(
                "panel receipt input must not be a symlink: {}",
                path.display()
            )));
        }
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("json") => {
                if !file_type.is_file() {
                    return Err(DeliveryError::new(
                        "panel receipt JSON is not a regular file",
                    ));
                }
                json.push(path);
            }
            Some("sig") => {
                if !file_type.is_file() {
                    return Err(DeliveryError::new(
                        "panel receipt signature is not a regular file",
                    ));
                }
                signatures.insert(path);
            }
            _ => {}
        }
    }
    json.sort();
    if json.len() != PANEL_ROLES.len() || signatures.len() != PANEL_ROLES.len() {
        return Err(DeliveryError::new(format!(
            "panel must contain exactly {} receipt/signature pairs",
            PANEL_ROLES.len()
        )));
    }
    if json
        .iter()
        .any(|path| !signatures.contains(&path.with_extension("sig")))
    {
        return Err(DeliveryError::new(
            "panel receipt directory has an unmatched receipt or signature",
        ));
    }
    Ok(json)
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("panel receipt path is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::command::{CommandLimits, CommandOutput};
    use std::{cell::RefCell, fs};

    struct FakeCommand {
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl CommandOutputAdapter for FakeCommand {
        fn output_with_limits(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
            _limits: CommandLimits,
        ) -> Result<CommandOutput> {
            self.calls
                .borrow_mut()
                .push((program.to_owned(), args.to_vec()));
            Ok(CommandOutput {
                success: true,
                exit_code: Some(0),
                stdout: b"Verified OK\n".to_vec(),
                stderr: vec![],
            })
        }
    }

    #[test]
    fn program_policy_is_exact_and_roster_has_ten_unique_roles() {
        assert_eq!(PANEL_PROVIDER_POLICY, "github-copilot");
        assert_eq!(PANEL_MODEL_POLICY, "gemini-3.1-pro-preview");
        assert_eq!(PANEL_SIGNATURE_POLICY, "rsa-sha256");
        assert_eq!(PANEL_ROLES.len(), 10);
        assert_eq!(PANEL_ROLES.into_iter().collect::<BTreeSet<_>>().len(), 10);
    }

    #[test]
    fn openssl_verifier_binds_receipt_signature_and_trust_root() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository")
            .parent()
            .expect("repository parent")
            .join(format!(".d2b-panel-signature-test-{}", std::process::id()));
        fs::create_dir(&root).expect("scratch");
        let receipt = root.join("software.json");
        let signature = root.join("software.sig");
        let trust_root = root.join("trust-root.pem");
        let claims = PanelAttestation {
            artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            role: PanelRole::Software,
            candidate_id: "a".repeat(64),
            content_id: "b".repeat(64),
            snapshot_sha256: "c".repeat(64),
            model_version: PANEL_MODEL_POLICY.to_owned(),
            provider: PANEL_PROVIDER_POLICY.to_owned(),
            run_id: "provider-run-1".to_owned(),
            receipt_locator: "github-copilot://runs/provider-run-1/software".to_owned(),
            output_sha256: "d".repeat(64),
            signoff: true,
            recommendations: vec![],
        };
        fs::write(&receipt, serde_json::to_vec(&claims).expect("receipt JSON")).expect("receipt");
        fs::write(&signature, b"detached signature").expect("signature");
        fs::write(&trust_root, b"public key").expect("trust root");
        let command = FakeCommand {
            calls: RefCell::new(vec![]),
        };
        let verified = OpenSslPanelReceiptVerifier::new(&command)
            .verify(&receipt, &signature, &trust_root)
            .expect("verified receipt");
        assert_eq!(verified.claims, claims);
        let calls = command.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "openssl");
        assert_eq!(calls[0].1[0], "rsa");
        assert!(calls[0].1.contains(&"-pubin".to_owned()));
        let arguments = calls[1].1.join(" ");
        assert!(arguments.contains("dgst -sha256 -verify"));
        assert!(arguments.contains("-signature"));
        assert!(arguments.contains(receipt.to_str().expect("receipt path")));
        drop(calls);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
