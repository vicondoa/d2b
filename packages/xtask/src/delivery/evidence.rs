use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::RepositoryProbe,
    model::{
        EvidenceResultClass, WaveSnapshot, ensure_schema, validate_identifier, validate_sha256,
    },
    snapshot::{SnapshotContext, load_snapshot_context},
    storage::{
        ensure_external_path, read_json, sha256_bytes, sha256_file, verify_json_digest,
        write_immutable_json,
    },
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceImportRequest {
    pub schema_version: u32,
    pub id: String,
    pub command: String,
    pub result_class: EvidenceResultClass,
    pub timestamp: String,
    pub tree_hash: String,
    pub payload: EvidencePayloadSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePayloadSource {
    pub path: Option<PathBuf>,
    pub sha256: Option<String>,
    pub external_locator: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub schema_version: u32,
    pub id: String,
    pub result_class: EvidenceResultClass,
    pub timestamp: String,
    pub tree_hash: String,
    pub snapshot_sha256: String,
    pub command_sha256: String,
    pub payload_sha256: String,
    pub external_locator_sha256: Option<String>,
}

pub fn import_evidence<P: RepositoryProbe>(
    probe: &P,
    snapshot_path: &Path,
    request_path: &Path,
) -> Result<PathBuf> {
    let context = load_snapshot_context(probe, snapshot_path, true)?;
    ensure_external_path(
        request_path,
        &context.repository_roots,
        &context.git_common_dirs,
    )?;
    let request: EvidenceImportRequest = read_json(request_path)?;
    let record = build_record(&context, &request)?;
    let path = context
        .layout
        .evidence_dir()
        .join(format!("{}.json", record.id));
    write_immutable_json(&path, &record)?;
    Ok(path)
}

pub fn verify_evidence<P: RepositoryProbe>(
    probe: &P,
    snapshot_path: &Path,
    evidence_path: &Path,
) -> Result<EvidenceRecord> {
    let context = load_snapshot_context(probe, snapshot_path, true)?;
    verify_evidence_in_context(&context, evidence_path)
}

pub(crate) fn verify_evidence_in_context(
    context: &SnapshotContext,
    evidence_path: &Path,
) -> Result<EvidenceRecord> {
    ensure_external_path(
        evidence_path,
        &context.repository_roots,
        &context.git_common_dirs,
    )?;
    let record: EvidenceRecord = read_json(evidence_path)?;
    validate_record(&context.snapshot, &context.digest, &record)?;
    let expected_path = context
        .layout
        .evidence_dir()
        .join(format!("{}.json", record.id));
    if super::storage::absolute_for_write(evidence_path)?
        != super::storage::absolute_for_write(&expected_path)?
    {
        return Err(DeliveryError::new(
            "evidence path is outside the snapshot validation directory",
        ));
    }
    verify_json_digest(evidence_path)?;
    Ok(record)
}

fn build_record(
    context: &SnapshotContext,
    request: &EvidenceImportRequest,
) -> Result<EvidenceRecord> {
    ensure_schema(request.schema_version, "evidence import request")?;
    validate_identifier(&request.id, "validation id")?;
    if request.command.is_empty() {
        return Err(DeliveryError::new("validation command must not be empty"));
    }
    if request.tree_hash != context.snapshot.root_repository.tree_hash {
        return Err(DeliveryError::new(
            "evidence tree does not match snapshot tree",
        ));
    }
    validate_timestamp(&request.timestamp)?;
    let required = context
        .snapshot
        .required_validations
        .iter()
        .find(|validation| validation.id == request.id)
        .ok_or_else(|| {
            DeliveryError::new(format!(
                "evidence {} is not required by the snapshot",
                request.id
            ))
        })?;
    let command_sha256 = sha256_bytes(request.command.as_bytes());
    if command_sha256 != required.command_sha256 {
        return Err(DeliveryError::new(format!(
            "command digest mismatch for validation {}",
            request.id
        )));
    }
    let payload_sha256 = payload_digest(context, &request.payload)?;
    let external_locator_sha256 = request
        .payload
        .external_locator
        .as_ref()
        .map(|locator| {
            if locator.trim().is_empty() {
                Err(DeliveryError::new(
                    "external locator must not be empty when supplied",
                ))
            } else {
                Ok(sha256_bytes(locator.as_bytes()))
            }
        })
        .transpose()?;
    Ok(EvidenceRecord {
        schema_version: DELIVERY_SCHEMA_VERSION,
        id: request.id.clone(),
        result_class: request.result_class,
        timestamp: request.timestamp.clone(),
        tree_hash: request.tree_hash.clone(),
        snapshot_sha256: context.digest.clone(),
        command_sha256,
        payload_sha256,
        external_locator_sha256,
    })
}

fn payload_digest(context: &SnapshotContext, payload: &EvidencePayloadSource) -> Result<String> {
    match (&payload.path, &payload.sha256) {
        (Some(path), None) => {
            ensure_external_path(path, &context.repository_roots, &context.git_common_dirs)?;
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                DeliveryError::new(format!(
                    "cannot inspect evidence payload {}: {error}",
                    path.display()
                ))
            })?;
            if !metadata.file_type().is_file() {
                return Err(DeliveryError::new(
                    "evidence payload must be a regular external file",
                ));
            }
            sha256_file(path)
        }
        (None, Some(digest)) => {
            validate_sha256(digest, "evidence payload digest")?;
            Ok(digest.clone())
        }
        (Some(_), Some(_)) => Err(DeliveryError::new(
            "evidence payload must specify path or sha256, not both",
        )),
        (None, None) => Err(DeliveryError::new(
            "evidence payload must specify path or sha256",
        )),
    }
}

pub(crate) fn validate_record(
    snapshot: &WaveSnapshot,
    snapshot_digest: &str,
    record: &EvidenceRecord,
) -> Result<()> {
    ensure_schema(record.schema_version, "evidence record")?;
    validate_identifier(&record.id, "validation id")?;
    validate_timestamp(&record.timestamp)?;
    validate_sha256(&record.snapshot_sha256, "snapshot digest")?;
    validate_sha256(&record.command_sha256, "command digest")?;
    validate_sha256(&record.payload_sha256, "payload digest")?;
    if let Some(locator) = &record.external_locator_sha256 {
        validate_sha256(locator, "external locator digest")?;
    }
    if record.tree_hash != snapshot.root_repository.tree_hash {
        return Err(DeliveryError::new(
            "evidence record tree does not match snapshot",
        ));
    }
    if record.snapshot_sha256 != snapshot_digest {
        return Err(DeliveryError::new(
            "evidence record snapshot digest does not match",
        ));
    }
    let required = snapshot
        .required_validations
        .iter()
        .find(|validation| validation.id == record.id)
        .ok_or_else(|| DeliveryError::new(format!("unexpected evidence record {}", record.id)))?;
    if required.command_sha256 != record.command_sha256 {
        return Err(DeliveryError::new(format!(
            "evidence command hash does not match required validation {}",
            record.id
        )));
    }
    Ok(())
}

fn validate_timestamp(timestamp: &str) -> Result<()> {
    let (date, time) = timestamp
        .strip_suffix('Z')
        .and_then(|without_zone| without_zone.split_once('T'))
        .ok_or_else(|| DeliveryError::new("evidence timestamp must be an RFC3339 UTC timestamp"))?;
    if date.len() != 10
        || date.as_bytes().get(4) != Some(&b'-')
        || date.as_bytes().get(7) != Some(&b'-')
    {
        return Err(DeliveryError::new("evidence timestamp date is invalid"));
    }
    let date_parts = date
        .split('-')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| DeliveryError::new("evidence timestamp date is invalid"))?;
    if date_parts.len() != 3 {
        return Err(DeliveryError::new("evidence timestamp date is invalid"));
    }
    let (seconds, fraction) = time
        .split_once('.')
        .map_or((time, None), |(seconds, fraction)| {
            (seconds, Some(fraction))
        });
    if fraction.is_some_and(|fraction| {
        fraction.is_empty()
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return Err(DeliveryError::new("evidence timestamp fraction is invalid"));
    }
    if seconds.len() != 8
        || seconds.as_bytes().get(2) != Some(&b':')
        || seconds.as_bytes().get(5) != Some(&b':')
    {
        return Err(DeliveryError::new("evidence timestamp time is invalid"));
    }
    let time_parts = seconds
        .split(':')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| DeliveryError::new("evidence timestamp time is invalid"))?;
    if time_parts.len() != 3 || time_parts[0] > 23 || time_parts[1] > 59 || time_parts[2] > 59 {
        return Err(DeliveryError::new("evidence timestamp time is invalid"));
    }
    let year = date_parts[0];
    let month = date_parts[1];
    let day = date_parts[2];
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > days {
        return Err(DeliveryError::new("evidence timestamp date is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_rfc3339_utc_timestamp() {
        validate_timestamp("2026-07-13T07:41:58.396Z").expect("valid timestamp");
    }

    #[test]
    fn rejects_impossible_or_non_utc_timestamp() {
        for timestamp in [
            "2026-02-30T00:00:00Z",
            "2026-07-13T24:00:00Z",
            "2026-07-13T00:00:00-07:00",
            "1-1-1T0:0:0Z",
        ] {
            validate_timestamp(timestamp).expect_err(timestamp);
        }
    }

    #[test]
    fn evidence_record_has_no_raw_command_or_locator_fields() {
        let record = EvidenceRecord {
            schema_version: DELIVERY_SCHEMA_VERSION,
            id: "unit".to_owned(),
            result_class: EvidenceResultClass::Passed,
            timestamp: "2026-07-13T07:41:58Z".to_owned(),
            tree_hash: "1".repeat(40),
            snapshot_sha256: "2".repeat(64),
            command_sha256: "3".repeat(64),
            payload_sha256: "4".repeat(64),
            external_locator_sha256: Some("5".repeat(64)),
        };
        let json = serde_json::to_string(&record).expect("serialize");
        assert!(!json.contains("\"command\""));
        assert!(!json.contains("\"external_locator\""));
        assert!(json.contains("external_locator_sha256"));
    }
}
