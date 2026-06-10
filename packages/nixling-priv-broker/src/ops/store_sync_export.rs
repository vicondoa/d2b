//! StoreSync-only observability JSONL export (ADR 0027).
//!
//! The host-confidential broker audit record
//! ([`StoreSyncAuditFields`](super::store_sync_audit::StoreSyncAuditFields))
//! is `0640 root:nixlingd` and carries host-only context the
//! observability plane must never see: `caller_principal`,
//! `retained_generations`, the host `hardlink_farm_path`, the opaque
//! `bundle_closure_ref`, and any future host-only field. Grafana Alloy
//! is deliberately NOT granted read access to that record, nor to the
//! unified `/var/lib/nixling/audit/broker-*.jsonl` stream.
//!
//! Instead, every terminal StoreSync attempt also emits a narrow,
//! positive-allow-list projection here, written to
//! `<export-dir>/store-sync-<utc-date>.jsonl` (default
//! `/var/lib/nixling/observability/store-sync/`). The host Nix/Alloy
//! wiring grants the `alloy` identity focused read/traverse on THAT
//! directory only and tails the daily-rotated glob.
//!
//! Redaction is structural, not advisory: the exported surface is a
//! dedicated [`StoreSyncObservabilityRecord`] struct that simply does
//! not carry the host-only fields, so no serializer ever receives the
//! full audit struct — [`StoreSyncObservabilityRecord::from_audit_fields`]
//! reads only the allow-listed fields and the rest cannot leak. The
//! `#[serde(deny_unknown_fields)]` round-trip test plus the
//! [`EXPORTED_KEYS`] key-set test pin the contract.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use nix::libc;
use serde::{Deserialize, Serialize};

use super::store_sync_audit::{
    AuthzOutcome, CleanupReason, CleanupStatus, ErrorStage, StoreSyncAuditFields, SyncStatus,
};

/// Default directory for the StoreSync observability export. The host
/// Nix module grants `alloy` read/traverse here and nowhere else in the
/// broker's confidential state. Override via `--store-sync-export-dir`.
pub const DEFAULT_STORE_SYNC_EXPORT_DIR: &str = "/var/lib/nixling/observability/store-sync";

/// The exact, ordered positive allow-list of JSON keys the export
/// surface emits. The redaction test asserts a serialized record's
/// key-set equals this slice; the host-only audit fields
/// (`caller_principal`, `retained_generations`, `bundle_closure_ref`,
/// `hardlink_farm_path`, the nested `timings` object, and the raw
/// `vm`/`env` keys) are intentionally absent.
pub const EXPORTED_KEYS: &[&str] = &[
    "schema_version",
    "target_vm",
    "vm_id",
    "target_env",
    "generation_id",
    "generation_token",
    "sync_status",
    "error_stage",
    "cleanup_status",
    "cleanup_reason",
    "authz_outcome",
    "closure_count",
    "linked_count",
    "skipped_count",
    "swept_count",
    "fast_path",
    "total_ms",
    "lock_wait_ms",
    "lock_hold_ms",
    "probe_ms",
    "verify_ms",
    "stage_ms",
    "metadata_ms",
    "sweep_ms",
    "cleanup_ms",
];

/// Host-audit fields that MUST NOT appear on the export surface. Pinned
/// here so the redaction test fails closed if a future field is added to
/// the projection by mistake.
pub const REDACTED_KEYS: &[&str] = &[
    "caller_principal",
    "retained_generations",
    "bundle_closure_ref",
    "hardlink_farm_path",
    "timings",
    "vm",
    "env",
];

/// The signed StoreSync observability export record: a positive
/// allow-list projection of the terminal audit record.
///
/// `vm` is exported as `target_vm` and `env` as `target_env` so the
/// observability plane treats them as JSON content, never as Loki
/// stream labels (the stream stays a host singleton: `vm="host"`,
/// `role="host"`, `source="store-sync-audit"`). The per-phase timings
/// are flattened to the top level. `deny_unknown_fields` keeps the
/// surface closed under round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSyncObservabilityRecord {
    pub schema_version: u32,
    pub target_vm: String,
    pub vm_id: String,
    /// Target env of the synced VM. Stays in JSON content (never a Loki
    /// label) and is always serialized — `null` until env attribution is
    /// threaded — so the exported key-set is stable.
    pub target_env: Option<String>,
    pub generation_id: String,
    pub generation_token: u32,
    pub sync_status: SyncStatus,
    pub error_stage: ErrorStage,
    pub cleanup_status: CleanupStatus,
    pub cleanup_reason: CleanupReason,
    pub authz_outcome: AuthzOutcome,
    pub closure_count: u32,
    pub linked_count: u32,
    pub skipped_count: u32,
    pub swept_count: u32,
    pub fast_path: bool,
    pub total_ms: u64,
    pub lock_wait_ms: u64,
    pub lock_hold_ms: u64,
    pub probe_ms: u64,
    pub verify_ms: u64,
    pub stage_ms: u64,
    pub metadata_ms: u64,
    pub sweep_ms: u64,
    pub cleanup_ms: u64,
}

impl StoreSyncObservabilityRecord {
    /// Project the host-confidential terminal audit record down to the
    /// signed observability allow-list.
    ///
    /// This reads ONLY the allow-listed fields. The host-only fields on
    /// [`StoreSyncAuditFields`] (`caller_principal`,
    /// `retained_generations`, `bundle_closure_ref`,
    /// `hardlink_farm_path`, and any field added in a future wave) are
    /// never copied, and because [`StoreSyncObservabilityRecord`] does
    /// not carry them, they can never reach the serialized surface.
    pub fn from_audit_fields(fields: &StoreSyncAuditFields) -> Self {
        let timings = &fields.timings;
        Self {
            schema_version: fields.schema_version,
            target_vm: fields.vm.clone(),
            vm_id: fields.vm_id.clone(),
            target_env: fields.env.clone(),
            generation_id: fields.generation_id.clone(),
            generation_token: fields.generation_token,
            sync_status: fields.sync_status,
            error_stage: fields.error_stage,
            cleanup_status: fields.cleanup_status,
            cleanup_reason: fields.cleanup_reason,
            authz_outcome: fields.authz_outcome,
            closure_count: fields.closure_count,
            linked_count: fields.linked_count,
            skipped_count: fields.skipped_count,
            swept_count: fields.swept_count,
            fast_path: fields.fast_path,
            total_ms: timings.total_ms,
            lock_wait_ms: timings.lock_wait_ms,
            lock_hold_ms: timings.lock_hold_ms,
            probe_ms: timings.probe_ms,
            verify_ms: timings.verify_ms,
            stage_ms: timings.stage_ms,
            metadata_ms: timings.metadata_ms,
            sweep_ms: timings.sweep_ms,
            cleanup_ms: timings.cleanup_ms,
        }
    }

    /// Serialize the record as one JSONL line (trailing newline).
    pub fn to_jsonl(&self) -> io::Result<String> {
        let mut line = serde_json::to_string(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push('\n');
        Ok(line)
    }
}

/// Ensure the export directory exists without disturbing an existing
/// one's ownership/permissions/ACLs.
///
/// In production the directory is created by the observability host
/// module's `systemd.tmpfiles` rule (mode `0750` + a focused `alloy`
/// read/traverse ACL and a default ACL so broker-created `0640` files
/// inherit `user:alloy:r`). The broker must NOT chmod/chown/setfacl an
/// existing directory — doing so would clobber that grant. We only
/// create the tree when it is missing (standalone broker tests, or a
/// first run before tmpfiles ran). Refuses a symlinked leaf.
fn ensure_export_dir(export_dir: &Path) -> io::Result<()> {
    match fs::symlink_metadata(export_dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "store-sync export directory must not be a symlink: {}",
                        export_dir.display()
                    ),
                ));
            }
            if !meta.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "store-sync export path exists but is not a directory: {}",
                        export_dir.display()
                    ),
                ));
            }
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(export_dir)?;
            // Tighten the leaf we just created (best-effort; the
            // canonical posture is the tmpfiles rule in production).
            let perms = std::os::unix::fs::PermissionsExt::from_mode(0o750);
            let _ = fs::set_permissions(export_dir, perms);
            Ok(())
        }
        Err(err) => Err(err),
    }
}

/// Append one projected StoreSync observability record to the day's
/// rotated export file (`<export-dir>/store-sync-<utc-date>.jsonl`).
///
/// Files are created `0640` (owner-write, group-read). In production the
/// directory's default ACL grants `alloy` read on new files; the broker
/// does not chown to or know about the `alloy` gid. Daily rotation is by
/// filename, so a long-lived broker that crosses midnight simply opens
/// the next day's file. The host Alloy `local.file_match` globs the
/// directory and follows new files + truncation.
///
/// Call-site contract: this is best-effort observability. The
/// host-confidential audit record is the source of truth; a failed
/// export write must never fail the StoreSync operation. The
/// `io::Result` is returned only so the caller can log it.
pub fn append_export_record(
    export_dir: &Path,
    record: &StoreSyncObservabilityRecord,
) -> io::Result<()> {
    ensure_export_dir(export_dir)?;
    let date = crate::audit::utc_date_string();
    let path = export_dir.join(format!("store-sync-{date}.jsonl"));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o640)
        .custom_flags(libc::O_CLOEXEC)
        .open(&path)?;
    let line = record.to_jsonl()?;
    file.write_all(line.as_bytes())?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::store_sync_audit::{StoreSyncAuditContext, StoreSyncTimings};

    fn ctx() -> StoreSyncAuditContext {
        StoreSyncAuditContext {
            vm: "corp-vm".to_owned(),
            vm_id: "store-view:vm:corp-vm".to_owned(),
            env: Some("work".to_owned()),
            bundle_closure_ref: "store-view:vm:corp-vm".to_owned(),
            hardlink_farm_path: "/var/lib/nixling/vms/corp-vm/store-view".to_owned(),
            generation_id: "g-deadbeef".to_owned(),
            generation_token: 42,
            caller_principal: Some("uid:998/role:daemon".to_owned()),
            closure_count: 17,
            timings: StoreSyncTimings {
                total_ms: 12,
                lock_wait_ms: 1,
                lock_hold_ms: 2,
                probe_ms: 3,
                verify_ms: 4,
                stage_ms: 5,
                metadata_ms: 6,
                sweep_ms: 7,
                cleanup_ms: 8,
            },
        }
    }

    fn keys_of(value: &serde_json::Value) -> Vec<String> {
        value
            .as_object()
            .expect("record serializes to a JSON object")
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn exported_keys_equal_the_allow_list() {
        let audit = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42, 41]);
        let export = StoreSyncObservabilityRecord::from_audit_fields(&audit);
        let value = serde_json::to_value(&export).expect("serialize export record");

        let mut actual = keys_of(&value);
        actual.sort();
        let mut expected: Vec<String> = EXPORTED_KEYS.iter().map(|k| (*k).to_owned()).collect();
        expected.sort();
        assert_eq!(actual, expected, "export key-set must equal EXPORTED_KEYS");
    }

    #[test]
    fn redaction_fields_are_absent_from_every_terminal_shape() {
        // Cover each constructor so no terminal shape leaks a host-only
        // field through the projection.
        let records = vec![
            StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42, 41]),
            StoreSyncAuditFields::ok_fast_path(ctx(), vec![42]),
            StoreSyncAuditFields::ok_cleanup_failed(ctx(), 17, 0, vec![42, 41], 0),
            StoreSyncAuditFields::failed(ctx(), ErrorStage::Probe),
            StoreSyncAuditFields::denied(ctx()),
        ];
        for audit in &records {
            let export = StoreSyncObservabilityRecord::from_audit_fields(audit);
            let value = serde_json::to_value(&export).expect("serialize export record");
            let keys = keys_of(&value);
            for redacted in REDACTED_KEYS {
                assert!(
                    !keys.iter().any(|k| k == redacted),
                    "redacted key {redacted:?} leaked into export surface; keys={keys:?}"
                );
            }
            // The caller-principal value must never appear anywhere in
            // the serialized text either (defensive against renames).
            let text = serde_json::to_string(&export).expect("serialize export text");
            assert!(
                !text.contains("uid:998/role:daemon"),
                "caller principal value leaked into export text: {text}"
            );
            assert!(
                !text.contains("/var/lib/nixling/vms/corp-vm/store-view"),
                "host farm path leaked into export text: {text}"
            );
        }
    }

    #[test]
    fn vm_and_env_are_renamed_to_target_fields() {
        let audit = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42]);
        let export = StoreSyncObservabilityRecord::from_audit_fields(&audit);
        assert_eq!(export.target_vm, "corp-vm");
        assert_eq!(export.vm_id, "store-view:vm:corp-vm");
        assert_eq!(export.target_env.as_deref(), Some("work"));
    }

    #[test]
    fn target_env_key_is_present_even_when_unknown() {
        let mut audit = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42]);
        audit.env = None;
        let export = StoreSyncObservabilityRecord::from_audit_fields(&audit);
        let value = serde_json::to_value(&export).expect("serialize export record");
        let obj = value.as_object().expect("object");
        assert!(
            obj.contains_key("target_env"),
            "target_env key must persist"
        );
        assert!(
            obj["target_env"].is_null(),
            "unknown env serializes as null"
        );
    }

    #[test]
    fn timings_are_flattened_to_top_level() {
        let audit = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42]);
        let export = StoreSyncObservabilityRecord::from_audit_fields(&audit);
        assert_eq!(export.total_ms, 12);
        assert_eq!(export.lock_wait_ms, 1);
        assert_eq!(export.cleanup_ms, 8);
        let value = serde_json::to_value(&export).expect("serialize");
        assert!(value.get("timings").is_none(), "no nested timings object");
        assert_eq!(value["total_ms"], 12);
    }

    #[test]
    fn round_trips_through_json_with_deny_unknown_fields() {
        let audit = StoreSyncAuditFields::failed(ctx(), ErrorStage::Stage);
        let export = StoreSyncObservabilityRecord::from_audit_fields(&audit);
        let line = export.to_jsonl().expect("jsonl");
        assert!(line.ends_with('\n'));
        let parsed: StoreSyncObservabilityRecord =
            serde_json::from_str(line.trim_end()).expect("round-trip");
        assert_eq!(parsed, export);
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let base = std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        base.join("store-sync-export-tests")
            .join(format!("{name}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn append_export_record_writes_one_jsonl_line() {
        let dir = scratch_dir("writer");
        let _ = fs::remove_dir_all(&dir);
        let audit = StoreSyncAuditFields::ok_non_fast_path(ctx(), 5, 12, vec![42]);
        let export = StoreSyncObservabilityRecord::from_audit_fields(&audit);

        append_export_record(&dir, &export).expect("first append");
        append_export_record(&dir, &export).expect("second append");

        let date = crate::audit::utc_date_string();
        let path = dir.join(format!("store-sync-{date}.jsonl"));
        let contents = fs::read_to_string(&path).expect("read export file");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "each append adds exactly one line");
        let first: StoreSyncObservabilityRecord =
            serde_json::from_str(lines[0]).expect("parse exported line");
        assert_eq!(first, export);

        let mode = fs::metadata(&path).expect("stat export file").permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o640,
            "export file is 0640"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
