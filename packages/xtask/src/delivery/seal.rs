use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::RepositoryProbe,
    evidence::verify_evidence_in_context,
    model::{
        ContentIdentity, EvidenceResultClass, PanelRole, RepositoryTreeBinding, WaveSnapshot,
        ensure_schema, validate_hash, validate_identifier, validate_sha256,
    },
    panel::read_stored_panel,
    snapshot::load_snapshot_context,
    storage::{ensure_external_path, read_json, verify_json_digest, write_immutable_json},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaveSeal {
    pub schema_version: u32,
    pub wave: String,
    pub tree_hash: String,
    pub snapshot_sha256: String,
    pub repository_set: Vec<RepositoryTreeBinding>,
    pub content_identity: ContentIdentity,
    pub validation_payloads: Vec<ValidationPayloadBinding>,
    pub panel_payloads: Vec<PanelPayloadBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationPayloadBinding {
    pub id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelPayloadBinding {
    pub role: PanelRole,
    pub sha256: String,
}

pub fn construct_seal<P: RepositoryProbe>(probe: &P, snapshot_path: &Path) -> Result<PathBuf> {
    let context = load_snapshot_context(probe, snapshot_path, true)?;
    let validation_payloads = collect_validation_payloads(&context)?;
    let panel_payloads = collect_panel_payloads(&context)?;
    let seal = WaveSeal {
        schema_version: DELIVERY_SCHEMA_VERSION,
        wave: context.snapshot.wave.clone(),
        tree_hash: context.snapshot.root_repository.tree_hash.clone(),
        snapshot_sha256: context.digest.clone(),
        repository_set: context.snapshot.repository_bindings(),
        content_identity: context.snapshot.content_identity(),
        validation_payloads,
        panel_payloads,
    };
    validate_seal(&seal, &context.snapshot, &context.digest)?;
    let path = context.layout.seal();
    write_immutable_json(&path, &seal)?;
    Ok(path)
}

pub fn verify_seal<P: RepositoryProbe>(probe: &P, seal_path: &Path) -> Result<WaveSeal> {
    if seal_path.file_name().and_then(|name| name.to_str()) != Some("seal.json") {
        return Err(DeliveryError::new("seal path must end in seal.json"));
    }
    let candidate = seal_path
        .parent()
        .ok_or_else(|| DeliveryError::new("seal path has no candidate directory"))?;
    let snapshot_path = candidate.join("snapshot.json");
    let context = load_snapshot_context(probe, &snapshot_path, true)?;
    ensure_external_path(
        seal_path,
        &context.repository_roots,
        &context.git_common_dirs,
    )?;
    if super::storage::absolute_for_write(seal_path)?
        != super::storage::absolute_for_write(&context.layout.seal())?
    {
        return Err(DeliveryError::new(
            "seal is outside its tree-addressed candidate directory",
        ));
    }
    let seal: WaveSeal = read_json(seal_path)?;
    validate_seal(&seal, &context.snapshot, &context.digest)?;
    let validation_payloads = collect_validation_payloads(&context)?;
    if validation_payloads != seal.validation_payloads {
        return Err(DeliveryError::new(
            "seal validation payload bindings changed",
        ));
    }
    let panel_payloads = collect_panel_payloads(&context)?;
    if panel_payloads != seal.panel_payloads {
        return Err(DeliveryError::new("seal panel payload bindings changed"));
    }
    verify_json_digest(seal_path)?;
    Ok(seal)
}

pub fn verify_history_only_equivalence(
    sealed: &WaveSnapshot,
    candidate: &WaveSnapshot,
) -> Result<()> {
    sealed.validate()?;
    candidate.validate()?;
    if sealed.content_identity() != candidate.content_identity() {
        return Err(DeliveryError::new(
            "candidate is not history-only: content, dependencies, contracts, \
             repository set, or required gates changed",
        ));
    }
    Ok(())
}

fn validate_seal(seal: &WaveSeal, snapshot: &WaveSnapshot, snapshot_digest: &str) -> Result<()> {
    ensure_schema(seal.schema_version, "wave seal")?;
    validate_identifier(&seal.wave, "wave")?;
    validate_hash(&seal.tree_hash, "seal tree")?;
    validate_sha256(&seal.snapshot_sha256, "seal snapshot digest")?;
    if seal.wave != snapshot.wave
        || seal.tree_hash != snapshot.root_repository.tree_hash
        || seal.snapshot_sha256 != snapshot_digest
    {
        return Err(DeliveryError::new(
            "seal identity does not match its snapshot",
        ));
    }
    if seal.repository_set != snapshot.repository_bindings() {
        return Err(DeliveryError::new(
            "seal repository set does not match its snapshot",
        ));
    }
    if seal.content_identity != snapshot.content_identity() {
        return Err(DeliveryError::new(
            "seal content identity does not match its snapshot",
        ));
    }

    let expected_validations = snapshot
        .required_validations
        .iter()
        .map(|validation| validation.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen_validations = BTreeSet::new();
    for payload in &seal.validation_payloads {
        validate_identifier(&payload.id, "validation id")?;
        validate_sha256(&payload.sha256, "validation payload digest")?;
        if !seen_validations.insert(payload.id.as_str()) {
            return Err(DeliveryError::new(format!(
                "seal repeats validation payload {}",
                payload.id
            )));
        }
    }
    if seen_validations != expected_validations {
        return Err(DeliveryError::new(
            "seal validation payload set is incomplete",
        ));
    }
    if !seal
        .validation_payloads
        .windows(2)
        .all(|pair| pair[0] < pair[1])
    {
        return Err(DeliveryError::new(
            "seal validation payloads must be sorted",
        ));
    }

    let expected_roles = super::model::PANEL_ROLES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut seen_roles = BTreeSet::new();
    for payload in &seal.panel_payloads {
        validate_sha256(&payload.sha256, "panel payload digest")?;
        if !seen_roles.insert(payload.role) {
            return Err(DeliveryError::new(format!(
                "seal repeats panel role {}",
                payload.role.as_str()
            )));
        }
    }
    if seen_roles != expected_roles {
        return Err(DeliveryError::new("seal panel payload set is incomplete"));
    }
    if !seal.panel_payloads.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DeliveryError::new("seal panel payloads must be sorted"));
    }
    Ok(())
}

fn collect_validation_payloads(
    context: &super::snapshot::SnapshotContext,
) -> Result<Vec<ValidationPayloadBinding>> {
    let directory = context.layout.evidence_dir();
    ensure_exact_json_set(
        &directory,
        context
            .snapshot
            .required_validations
            .iter()
            .map(|validation| validation.id.as_str()),
        "validation evidence",
    )?;
    let mut payloads = Vec::new();
    for required in &context.snapshot.required_validations {
        let path = directory.join(format!("{}.json", required.id));
        let record = verify_evidence_in_context(context, &path)?;
        if record.result_class != EvidenceResultClass::Passed {
            return Err(DeliveryError::new(format!(
                "validation evidence {} is not passed",
                required.id
            )));
        }
        payloads.push(ValidationPayloadBinding {
            id: required.id.clone(),
            sha256: verify_json_digest(&path)?,
        });
    }
    payloads.sort();
    Ok(payloads)
}

fn collect_panel_payloads(
    context: &super::snapshot::SnapshotContext,
) -> Result<Vec<PanelPayloadBinding>> {
    let records = read_stored_panel(context)?;
    let mut payloads = Vec::new();
    for record in records {
        if !record.signoff {
            return Err(DeliveryError::new(format!(
                "panel role {} has findings",
                record.role.as_str()
            )));
        }
        let path = context
            .layout
            .panel_dir()
            .join(format!("{}.json", record.role.as_str()));
        payloads.push(PanelPayloadBinding {
            role: record.role,
            sha256: verify_json_digest(&path)?,
        });
    }
    payloads.sort();
    Ok(payloads)
}

fn ensure_exact_json_set<'a>(
    directory: &Path,
    expected: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<()> {
    let expected = expected.map(str::to_owned).collect::<BTreeSet<_>>();
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        DeliveryError::new(format!(
            "cannot inspect {label} directory {}: {error}",
            directory.display()
        ))
    })?;
    if !metadata.file_type().is_dir() {
        return Err(DeliveryError::new(format!(
            "{label} path is not a directory"
        )));
    }
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            return Err(DeliveryError::new(format!(
                "{label} JSON is not a regular file: {}",
                path.display()
            )));
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| DeliveryError::new(format!("{label} filename is not UTF-8")))?;
        actual.insert(stem.to_owned());
    }
    if actual != expected {
        return Err(DeliveryError::new(format!(
            "{label} set does not exactly match the snapshot requirements"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::model::{
        RepositoryRecord, RequiredCheck, RequiredValidation, RootRepository, StackNode,
    };

    fn snapshot(tree: &str, contract: &str) -> WaveSnapshot {
        WaveSnapshot {
            schema_version: DELIVERY_SCHEMA_VERSION,
            wave: "w1".to_owned(),
            root_repository: RootRepository {
                name: "example/d2b".to_owned(),
                root: "/external/repository".to_owned(),
                base_commit: "0".repeat(40),
                head_commit: "1".repeat(40),
                tree_hash: tree.to_owned(),
            },
            repository_set: vec![RepositoryRecord {
                name: "example/d2b".to_owned(),
                root: "/external/repository".to_owned(),
                head_commit: "1".repeat(40),
                tree_hash: tree.to_owned(),
            }],
            stack: vec![StackNode {
                id: "root".to_owned(),
                repository: "example/d2b".to_owned(),
                branch: "feature".to_owned(),
                pr: Some(1),
                head_commit: "1".repeat(40),
                depends_on: vec![],
            }],
            required_validations: vec![RequiredValidation {
                id: "unit".to_owned(),
                command_sha256: "2".repeat(64),
            }],
            required_checks: vec![RequiredCheck {
                node: "root".to_owned(),
                name: "unit".to_owned(),
            }],
            generated_artifacts: vec![],
            dependency_fingerprints: vec![],
            contract_fingerprints: vec![super::super::model::Fingerprint {
                name: "contract".to_owned(),
                repository: "example/d2b".to_owned(),
                path: "contract.json".to_owned(),
                sha256: contract.to_owned(),
            }],
        }
    }

    #[test]
    fn history_only_allows_commit_and_graph_changes() {
        let mut candidate = snapshot(&"a".repeat(40), &"b".repeat(64));
        candidate.root_repository.base_commit = "3".repeat(40);
        candidate.root_repository.head_commit = "4".repeat(40);
        candidate.repository_set[0].head_commit = "4".repeat(40);
        candidate.stack[0].head_commit = "4".repeat(40);
        candidate.stack[0].branch = "retargeted".to_owned();
        verify_history_only_equivalence(&snapshot(&"a".repeat(40), &"b".repeat(64)), &candidate)
            .expect("history-only");
    }

    #[test]
    fn history_only_rejects_content_or_contract_change() {
        let sealed = snapshot(&"a".repeat(40), &"b".repeat(64));
        let changed_tree = snapshot(&"c".repeat(40), &"b".repeat(64));
        verify_history_only_equivalence(&sealed, &changed_tree).expect_err("tree changed");
        let changed_contract = snapshot(&"a".repeat(40), &"d".repeat(64));
        verify_history_only_equivalence(&sealed, &changed_contract).expect_err("contract changed");
    }

    #[test]
    fn seal_has_no_self_digest_field() {
        let snapshot = snapshot(&"a".repeat(40), &"b".repeat(64));
        let seal = WaveSeal {
            schema_version: DELIVERY_SCHEMA_VERSION,
            wave: "w1".to_owned(),
            tree_hash: "a".repeat(40),
            snapshot_sha256: "e".repeat(64),
            repository_set: snapshot.repository_bindings(),
            content_identity: snapshot.content_identity(),
            validation_payloads: vec![ValidationPayloadBinding {
                id: "unit".to_owned(),
                sha256: "f".repeat(64),
            }],
            panel_payloads: super::super::model::PANEL_ROLES
                .iter()
                .copied()
                .map(|role| PanelPayloadBinding {
                    role,
                    sha256: "1".repeat(64),
                })
                .collect(),
        };
        let json = serde_json::to_string(&seal).expect("serialize");
        assert!(!json.contains("seal_sha256"));
    }
}
