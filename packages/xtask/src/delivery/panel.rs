use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::RepositoryProbe,
    model::{
        MAX_RECOMMENDATIONS, MAX_STRING_BYTES, PANEL_ATTESTATION_ARTIFACT_KIND, PANEL_MODEL_POLICY,
        PANEL_REQUEST_ARTIFACT_KIND, PANEL_ROLES, PanelRole, ensure_schema,
        validate_bounded_string, validate_sha256,
    },
    snapshot::{CurrentVerification, SnapshotContext, load_snapshot_context},
    storage::{ensure_external_path, read_json, read_verified_json, write_immutable_json},
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
    pub required_model_version: String,
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
    pub output_sha256: String,
    pub signoff: bool,
    pub recommendations: Vec<String>,
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
        required_model_version: PANEL_MODEL_POLICY.to_owned(),
    };
    let path = context.layout.panel_request();
    write_immutable_json(&path, &request)?;
    Ok(path)
}

pub fn validate_and_store_panel<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    records_dir: &Path,
) -> Result<Vec<PanelAttestation>> {
    let context = load_snapshot_context(
        probe,
        repository_roots,
        snapshot_path,
        CurrentVerification::ExactRefs,
    )?;
    ensure_external_path(records_dir, &context.external_exclusions)?;
    let records = read_and_validate_records(records_dir, &context)?;
    for record in &records {
        let path = context
            .layout
            .panel_dir()
            .join(format!("{}.json", record.role.as_str()));
        write_immutable_json(&path, record)?;
    }
    Ok(records)
}

pub(crate) fn read_stored_panel(context: &SnapshotContext) -> Result<Vec<PanelAttestation>> {
    let panel_dir = context.layout.panel_dir();
    ensure_external_path(&panel_dir, &context.external_exclusions)?;
    read_and_validate_records_mode(&panel_dir, context, true)
}

fn read_and_validate_records(
    records_dir: &Path,
    context: &SnapshotContext,
) -> Result<Vec<PanelAttestation>> {
    read_and_validate_records_mode(records_dir, context, false)
}

fn read_and_validate_records_mode(
    records_dir: &Path,
    context: &SnapshotContext,
    verified: bool,
) -> Result<Vec<PanelAttestation>> {
    let paths = json_files(records_dir)?;
    if paths.len() != PANEL_ROLES.len() {
        return Err(DeliveryError::new(format!(
            "panel must contain exactly {} JSON attestations, found {}",
            PANEL_ROLES.len(),
            paths.len()
        )));
    }
    let mut by_role = BTreeMap::new();
    let mut runs = BTreeSet::<(String, String)>::new();
    for path in paths {
        let record: PanelAttestation = if verified {
            read_verified_json(&path)?.0
        } else {
            read_json(&path)?
        };
        validate_record(context, &record)?;
        if !runs.insert((record.provider.clone(), record.run_id.clone())) {
            return Err(DeliveryError::new(
                "panel provenance repeats a provider/run ID",
            ));
        }
        if by_role.insert(record.role, record).is_some() {
            return Err(DeliveryError::new(format!(
                "duplicate panel role in {}",
                path.display()
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
        return Err(DeliveryError::new(
            "invalid panel attestation artifact_kind",
        ));
    }
    ensure_schema(record.schema_version, "panel attestation")?;
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
    if record.model_version != PANEL_MODEL_POLICY {
        return Err(DeliveryError::new(format!(
            "panel role {} did not use required model {}",
            record.role.as_str(),
            PANEL_MODEL_POLICY
        )));
    }
    validate_bounded_string(&record.provider, "panel provider")?;
    validate_bounded_string(&record.run_id, "panel run ID")?;
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

fn json_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        DeliveryError::new(format!(
            "cannot inspect panel directory {}: {error}",
            directory.display()
        ))
    })?;
    if !metadata.file_type().is_dir() {
        return Err(DeliveryError::new(
            "panel attestations path must be an external directory",
        ));
    }
    let mut json = Vec::with_capacity(PANEL_ROLES.len());
    let mut entries = 0_usize;
    for entry in fs::read_dir(directory)? {
        entries += 1;
        if entries > PANEL_ROLES.len() * 3 {
            return Err(DeliveryError::new(
                "panel directory contains too many entries",
            ));
        }
        if json.len() > PANEL_ROLES.len() {
            return Err(DeliveryError::new(
                "panel directory contains too many JSON entries",
            ));
        }
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(DeliveryError::new(format!(
                "panel attestation must not be a symlink: {}",
                path.display()
            )));
        }
        if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            if !file_type.is_file() {
                return Err(DeliveryError::new(format!(
                    "panel JSON entry is not a regular file: {}",
                    path.display()
                )));
            }
            json.push(path);
        }
    }
    json.sort();
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_policy_is_exact_and_roster_has_ten_unique_roles() {
        assert_eq!(PANEL_MODEL_POLICY, "gemini-3.1-pro-preview");
        assert_eq!(PANEL_ROLES.len(), 10);
        assert_eq!(PANEL_ROLES.into_iter().collect::<BTreeSet<_>>().len(), 10);
    }

    #[test]
    fn panel_metadata_is_external_attestation_metadata() {
        let record = PanelAttestation {
            artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            role: PanelRole::Rust,
            candidate_id: "a".repeat(64),
            content_id: "b".repeat(64),
            snapshot_sha256: "c".repeat(64),
            model_version: PANEL_MODEL_POLICY.to_owned(),
            provider: "provider".to_owned(),
            run_id: "run-1".to_owned(),
            output_sha256: "d".repeat(64),
            signoff: true,
            recommendations: vec![],
        };
        let json = serde_json::to_string(&record).expect("serialize");
        assert!(json.contains(PANEL_MODEL_POLICY));
        assert!(json.contains(PANEL_ATTESTATION_ARTIFACT_KIND));
    }
}
