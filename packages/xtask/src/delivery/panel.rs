use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DeliveryError, Result,
    command::RepositoryProbe,
    model::{
        PANEL_ROLES, PanelRole, RepositoryTreeBinding, WaveSnapshot, ensure_schema, validate_sha256,
    },
    snapshot::{SnapshotContext, load_snapshot_context},
    storage::{ensure_external_path, read_json, verify_json_digest, write_immutable_json},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRecord {
    pub schema_version: u32,
    pub role: PanelRole,
    pub tree_hash: String,
    pub snapshot_sha256: String,
    pub repository_set: Vec<RepositoryTreeBinding>,
    pub signoff: bool,
    pub recommendations: Vec<String>,
}

pub fn validate_and_store_panel<P: RepositoryProbe>(
    probe: &P,
    snapshot_path: &Path,
    records_dir: &Path,
) -> Result<Vec<PanelRecord>> {
    let context = load_snapshot_context(probe, snapshot_path, true)?;
    ensure_external_path(
        records_dir,
        &context.repository_roots,
        &context.git_common_dirs,
    )?;
    let records = read_and_validate_records(records_dir, &context.snapshot, &context.digest)?;
    for record in &records {
        let path = context
            .layout
            .panel_dir()
            .join(format!("{}.json", record.role.as_str()));
        write_immutable_json(&path, record)?;
    }
    Ok(records)
}

pub(crate) fn read_stored_panel(context: &SnapshotContext) -> Result<Vec<PanelRecord>> {
    let panel_dir = context.layout.panel_dir();
    ensure_external_path(
        &panel_dir,
        &context.repository_roots,
        &context.git_common_dirs,
    )?;
    let records = read_and_validate_records(&panel_dir, &context.snapshot, &context.digest)?;
    for record in &records {
        let path = panel_dir.join(format!("{}.json", record.role.as_str()));
        verify_json_digest(&path)?;
    }
    Ok(records)
}

fn read_and_validate_records(
    records_dir: &Path,
    snapshot: &WaveSnapshot,
    snapshot_digest: &str,
) -> Result<Vec<PanelRecord>> {
    let paths = json_files(records_dir)?;
    if paths.len() != PANEL_ROLES.len() {
        return Err(DeliveryError::new(format!(
            "panel must contain exactly {} JSON records, found {}",
            PANEL_ROLES.len(),
            paths.len()
        )));
    }
    let mut by_role = BTreeMap::new();
    for path in paths {
        let mut record: PanelRecord = read_json(&path)?;
        validate_record(snapshot, snapshot_digest, &record)?;
        record.repository_set.sort();
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
        let missing = expected
            .difference(&actual)
            .map(|role| role.as_str())
            .collect::<Vec<_>>();
        return Err(DeliveryError::new(format!(
            "panel is missing roles: {}",
            missing.join(", ")
        )));
    }
    Ok(PANEL_ROLES
        .iter()
        .map(|role| {
            by_role
                .remove(role)
                .expect("all accepted roles were checked")
        })
        .collect())
}

fn validate_record(
    snapshot: &WaveSnapshot,
    snapshot_digest: &str,
    record: &PanelRecord,
) -> Result<()> {
    ensure_schema(record.schema_version, "panel record")?;
    validate_sha256(&record.snapshot_sha256, "panel snapshot digest")?;
    if record.tree_hash != snapshot.root_repository.tree_hash {
        return Err(DeliveryError::new(format!(
            "panel role {} is bound to a different tree",
            record.role.as_str()
        )));
    }
    if record.snapshot_sha256 != snapshot_digest {
        return Err(DeliveryError::new(format!(
            "panel role {} is bound to a different snapshot",
            record.role.as_str()
        )));
    }
    let mut repository_set = record.repository_set.clone();
    repository_set.sort();
    let expected = snapshot.repository_bindings();
    if repository_set != expected {
        return Err(DeliveryError::new(format!(
            "panel role {} has a different repository set",
            record.role.as_str()
        )));
    }
    if repository_set
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err(DeliveryError::new(format!(
            "panel role {} repeats a repository",
            record.role.as_str()
        )));
    }
    for recommendation in &record.recommendations {
        if recommendation.trim().is_empty() || recommendation.len() > 16_384 {
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
            "panel records path must be an external directory",
        ));
    }
    let mut files = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            let file_type = entry.file_type()?;
            Ok((entry.path(), file_type))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut json = Vec::new();
    for (path, file_type) in files {
        if file_type.is_symlink() {
            return Err(DeliveryError::new(format!(
                "panel record must not be a symlink: {}",
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
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> WaveSnapshot {
        WaveSnapshot {
            schema_version: super::super::DELIVERY_SCHEMA_VERSION,
            wave: "w1".to_owned(),
            root_repository: super::super::model::RootRepository {
                name: "example/d2b".to_owned(),
                root: "/external/repository".to_owned(),
                base_commit: "0".repeat(40),
                head_commit: "1".repeat(40),
                tree_hash: "2".repeat(40),
            },
            repository_set: vec![super::super::model::RepositoryRecord {
                name: "example/d2b".to_owned(),
                root: "/external/repository".to_owned(),
                head_commit: "1".repeat(40),
                tree_hash: "2".repeat(40),
            }],
            stack: vec![super::super::model::StackNode {
                id: "root".to_owned(),
                repository: "example/d2b".to_owned(),
                branch: "feature".to_owned(),
                pr: Some(42),
                head_commit: "1".repeat(40),
                depends_on: vec![],
            }],
            required_validations: vec![super::super::model::RequiredValidation {
                id: "unit".to_owned(),
                command_sha256: "3".repeat(64),
            }],
            required_checks: vec![super::super::model::RequiredCheck {
                node: "root".to_owned(),
                name: "unit".to_owned(),
            }],
            generated_artifacts: vec![],
            dependency_fingerprints: vec![],
            contract_fingerprints: vec![],
        }
    }

    fn record(role: PanelRole) -> PanelRecord {
        PanelRecord {
            schema_version: super::super::DELIVERY_SCHEMA_VERSION,
            role,
            tree_hash: "2".repeat(40),
            snapshot_sha256: "4".repeat(64),
            repository_set: vec![RepositoryTreeBinding {
                name: "example/d2b".to_owned(),
                tree_hash: "2".repeat(40),
            }],
            signoff: true,
            recommendations: vec![],
        }
    }

    #[test]
    fn signoff_matches_empty_recommendations() {
        let snapshot = snapshot();
        validate_record(&snapshot, &"4".repeat(64), &record(PanelRole::Rust)).expect("signoff");
        let mut finding = record(PanelRole::Rust);
        finding.signoff = false;
        finding.recommendations = vec!["fix the mismatch".to_owned()];
        validate_record(&snapshot, &"4".repeat(64), &finding)
            .expect("finding is structurally valid");
    }

    #[test]
    fn rejects_inconsistent_finding_record() {
        let snapshot = snapshot();
        let mut record = record(PanelRole::Security);
        record.recommendations = vec!["finding".to_owned()];
        let error = validate_record(&snapshot, &"4".repeat(64), &record).expect_err("inconsistent");
        assert!(error.to_string().contains("if and only if"));
    }

    #[test]
    fn rejects_model_metadata_in_panel_json() {
        let mut value = serde_json::to_value(record(PanelRole::Software)).expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .insert("model".to_owned(), serde_json::json!("forbidden"));
        let error = serde_json::from_value::<PanelRecord>(value).expect_err("unknown model");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn exact_role_set_has_ten_roles() {
        assert_eq!(PANEL_ROLES.len(), 10);
        assert_eq!(PANEL_ROLES.into_iter().collect::<BTreeSet<_>>().len(), 10);
    }
}
