//! Explicit live-pool verification for ADR 0027 store-view.
//!
//! The CLI reaches this only through nixlingd -> broker. Verification reads
//! the broker-owned split layout (`state/`, `meta/`, `live/`) under the same
//! `sync.lock` file used by StoreSync and writes host-only integrity records.
//! It deliberately performs only the W6 top-level readiness/manifest check:
//! deep recursive package verification and real repair are later waves.

use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::io::Write as _;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use nix::fcntl::{flock, FlockArg};
use nixling_core::bundle_resolver::ResolvedStoreViewIntent;
use nixling_host::hardlink_farm;
use nixling_ipc::broker_wire::{StoreVerifyResponse, StoreVerifyStatus, StoreVerifyUnknownReason};
use serde::{Deserialize, Serialize};

use crate::ops::store_sync::run_store_sync_repair;
use crate::ops::store_view_posture::{posture_host_only_file, posture_store_view_matrix_paths};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntegrityState {
    Ok,
    Suspect,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegrityRecord {
    generation_id: Option<String>,
    state: IntegrityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drift_signature: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unknown_reason: Option<StoreVerifyUnknownReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audit_ref: Option<String>,
    repair_attempted: bool,
}

impl IntegrityRecord {
    fn ok(generation_id: &str) -> Self {
        Self {
            generation_id: Some(generation_id.to_owned()),
            state: IntegrityState::Ok,
            drift_signature: None,
            unknown_reason: None,
            audit_ref: None,
            repair_attempted: false,
        }
    }

    fn suspect(generation_id: &str, drift_signature: Vec<String>) -> Self {
        assert!(!drift_signature.is_empty(), "suspect requires drift");
        Self {
            generation_id: Some(generation_id.to_owned()),
            state: IntegrityState::Suspect,
            drift_signature: Some(drift_signature),
            unknown_reason: None,
            audit_ref: None,
            repair_attempted: false,
        }
    }

    fn unknown(generation_id: Option<&str>, unknown_reason: StoreVerifyUnknownReason) -> Self {
        debug_assert!(
            generation_id.is_none()
                == matches!(
                    unknown_reason,
                    StoreVerifyUnknownReason::GenerationIdentityUnavailable
                ),
            "VM-level unknown is only valid for generation_identity_unavailable"
        );
        Self {
            generation_id: generation_id.map(str::to_owned),
            state: IntegrityState::Unknown,
            drift_signature: None,
            unknown_reason: Some(unknown_reason),
            audit_ref: None,
            repair_attempted: false,
        }
    }

    fn validate(&self) -> Result<(), &'static str> {
        match self.state {
            IntegrityState::Ok => {
                if self.generation_id.is_none()
                    || self.drift_signature.is_some()
                    || self.unknown_reason.is_some()
                {
                    return Err("ok integrity record shape");
                }
            }
            IntegrityState::Suspect => {
                if self.generation_id.is_none()
                    || self
                        .drift_signature
                        .as_ref()
                        .map(|sig| sig.is_empty())
                        .unwrap_or(true)
                    || self.unknown_reason.is_some()
                {
                    return Err("suspect integrity record shape");
                }
            }
            IntegrityState::Unknown => {
                if self.drift_signature.is_some() || self.unknown_reason.is_none() {
                    return Err("unknown integrity record shape");
                }
                let reason = self.unknown_reason.unwrap();
                if self.generation_id.is_none()
                    && reason != StoreVerifyUnknownReason::GenerationIdentityUnavailable
                {
                    return Err("VM-level unknown reason shape");
                }
                if self.generation_id.is_some()
                    && reason == StoreVerifyUnknownReason::GenerationIdentityUnavailable
                {
                    return Err("generation-scoped unknown reason shape");
                }
            }
        }
        Ok(())
    }
}

pub fn run_store_verify(intent: &ResolvedStoreViewIntent, repair: bool) -> StoreVerifyResponse {
    let initial = run_store_verify_read_only(intent, repair);
    if repair
        && matches!(
            initial.status,
            StoreVerifyStatus::Drift | StoreVerifyStatus::Unknown
        )
    {
        return repair_store_view(intent, initial);
    }
    initial
}

fn run_store_verify_read_only(
    intent: &ResolvedStoreViewIntent,
    repair: bool,
) -> StoreVerifyResponse {
    let lock = match acquire_verify_lock(&intent.hardlink_farm_path) {
        Ok(lock) => lock,
        Err(detail) => {
            return failed(&intent.vm, format!("verify lock failed: {detail}"));
        }
    };
    let response = verify_locked(intent, repair);
    if let Err(err) = posture_store_view_matrix_paths(&intent.hardlink_farm_path, &intent.vm) {
        return failed(&intent.vm, format!("posture store-view metadata: {err}"));
    }
    drop(lock);
    response
}

fn repair_store_view(
    intent: &ResolvedStoreViewIntent,
    initial: StoreVerifyResponse,
) -> StoreVerifyResponse {
    match run_store_sync_repair(intent) {
        Ok(_) => {}
        Err(err) => {
            return StoreVerifyResponse {
                vm: initial.vm,
                status: StoreVerifyStatus::Failed,
                checked: initial.checked,
                drifted: initial.drifted,
                repaired: 0,
                unknown_reason: initial.unknown_reason,
                audit_ref: initial.audit_ref,
                remediation: Some(format!(
                    "repair incomplete; inspect audit_ref and broker logs ({err})"
                )),
            };
        }
    }

    let after = run_store_verify_read_only(intent, false);
    match after.status {
        StoreVerifyStatus::Ok => StoreVerifyResponse {
            vm: after.vm,
            status: StoreVerifyStatus::Repaired,
            checked: after.checked,
            drifted: 0,
            repaired: initial.drifted,
            unknown_reason: None,
            audit_ref: after.audit_ref,
            remediation: None,
        },
        StoreVerifyStatus::Drift => StoreVerifyResponse {
            remediation: Some("repair incomplete; inspect audit_ref and broker logs".to_owned()),
            ..after
        },
        StoreVerifyStatus::Unknown => {
            let remediation = after.remediation.clone().unwrap_or_else(|| {
                "repair incomplete; inspect audit_ref and broker logs".to_owned()
            });
            StoreVerifyResponse {
                remediation: Some(remediation),
                ..after
            }
        }
        _ => after,
    }
}

fn verify_locked(intent: &ResolvedStoreViewIntent, repair: bool) -> StoreVerifyResponse {
    let store_root = &intent.hardlink_farm_path;
    let vm = intent.vm.as_str();
    let Some(generation_id) = hardlink_farm::read_state_current_id(store_root) else {
        let record = IntegrityRecord::unknown(
            None,
            StoreVerifyUnknownReason::GenerationIdentityUnavailable,
        );
        if let Err(err) = write_integrity_record(&vm_unknown_integrity_path(store_root), &record) {
            return failed(vm, format!("write VM-level unknown integrity: {err}"));
        }
        return unknown(
            vm,
            0,
            StoreVerifyUnknownReason::GenerationIdentityUnavailable,
        );
    };

    let meta_current = hardlink_farm::read_meta_current_id(store_root);
    if meta_current.as_deref() != Some(generation_id.as_str()) {
        if meta_current.is_none() {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestMissing,
            );
        }
        return write_drift(
            vm,
            store_root,
            &generation_id,
            0,
            vec!["meta/current".to_owned()],
            repair,
        );
    }

    let state_gen = hardlink_farm::state_generation_dir(store_root, &generation_id);
    match hardlink_farm::read_generation_marker(&state_gen) {
        Ok(marker) if marker.vm == vm => {}
        Ok(_) => {
            return write_drift(
                vm,
                store_root,
                &generation_id,
                0,
                vec!["state/generations/current/marker.json".to_owned()],
                repair,
            );
        }
        Err(hardlink_farm::HardlinkFarmError::MarkerMissing { .. }) => {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestMissing,
            );
        }
        Err(_) => {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestUnreadable,
            );
        }
    }

    let live_marker = hardlink_farm::live_dir(store_root).join(format!(".nixling-marker-{vm}"));
    match std::fs::symlink_metadata(&live_marker) {
        Ok(meta) if meta.len() == 0 => {}
        Ok(_) => {
            return write_drift(
                vm,
                store_root,
                &generation_id,
                0,
                vec![format!("live/.nixling-marker-{vm}")],
                repair,
            );
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestMissing,
            );
        }
        Err(_) => {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestUnreadable,
            );
        }
    }

    let store_paths_path =
        hardlink_farm::meta_generation_dir(store_root, &generation_id).join("store-paths");
    let store_paths = match std::fs::read_to_string(&store_paths_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestMissing,
            );
        }
        Err(_) => {
            return write_unknown(
                vm,
                store_root,
                &generation_id,
                0,
                StoreVerifyUnknownReason::MarkerOrManifestUnreadable,
            );
        }
    };

    let live = hardlink_farm::live_dir(store_root);
    let mut checked = 0u32;
    let mut drift = Vec::new();
    for line in store_paths.lines().filter(|line| !line.trim().is_empty()) {
        checked = checked.saturating_add(1);
        let path = Path::new(line);
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            drift.push(format!("manifest:{line}"));
            continue;
        };
        let live_path = live.join(name);
        if std::fs::symlink_metadata(&live_path).is_err() {
            drift.push(name.to_owned());
            continue;
        }
        if verify_tree(source_path_for_line(line), &live_path).is_err() {
            drift.push(name.to_owned());
        }
    }

    if !drift.is_empty() {
        drift.sort();
        drift.dedup();
        return write_drift(vm, store_root, &generation_id, checked, drift, repair);
    }

    fn source_path_for_line(line: &str) -> &Path {
        Path::new(line)
    }

    fn verify_tree(source: &Path, live: &Path) -> Result<(), ()> {
        let source_meta = std::fs::symlink_metadata(source).map_err(|_| ())?;
        let live_meta = std::fs::symlink_metadata(live).map_err(|_| ())?;
        if source_meta.file_type().is_symlink() {
            if !live_meta.file_type().is_symlink() {
                return Err(());
            }
            return (std::fs::read_link(source).map_err(|_| ())?
                == std::fs::read_link(live).map_err(|_| ())?)
            .then_some(())
            .ok_or(());
        }
        if source_meta.is_dir() {
            if !live_meta.is_dir() {
                return Err(());
            }
            let mut source_entries = std::collections::BTreeSet::new();
            for entry in std::fs::read_dir(source).map_err(|_| ())? {
                let entry = entry.map_err(|_| ())?;
                source_entries.insert(entry.file_name());
                verify_tree(&entry.path(), &live.join(entry.file_name()))?;
            }
            for entry in std::fs::read_dir(live).map_err(|_| ())? {
                let entry = entry.map_err(|_| ())?;
                if !source_entries.contains(&entry.file_name()) {
                    return Err(());
                }
            }
            return Ok(());
        }
        if source_meta.is_file() {
            if !live_meta.is_file() {
                return Err(());
            }
            if (source_meta.mode() & 0o111) != (live_meta.mode() & 0o111) {
                return Err(());
            }
            if source_meta.dev() == live_meta.dev() && source_meta.ino() == live_meta.ino() {
                return Ok(());
            }
            return files_equal(source, live).then_some(()).ok_or(());
        }
        Err(())
    }

    fn files_equal(a: &Path, b: &Path) -> bool {
        let (Ok(a_meta), Ok(b_meta)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
            return false;
        };
        if a_meta.len() != b_meta.len() {
            return false;
        }
        let (Ok(mut a_file), Ok(mut b_file)) = (File::open(a), File::open(b)) else {
            return false;
        };
        let mut a_buf = [0u8; 8192];
        let mut b_buf = [0u8; 8192];
        loop {
            let Ok(a_n) = a_file.read(&mut a_buf) else {
                return false;
            };
            let Ok(b_n) = b_file.read(&mut b_buf) else {
                return false;
            };
            if a_n != b_n || a_buf[..a_n] != b_buf[..b_n] {
                return false;
            }
            if a_n == 0 {
                return true;
            }
        }
    }

    let record = IntegrityRecord::ok(&generation_id);
    if let Err(err) = write_integrity_record(
        &generation_integrity_path(store_root, &generation_id),
        &record,
    ) {
        return failed(vm, format!("write generation integrity: {err}"));
    }
    StoreVerifyResponse {
        vm: vm.to_owned(),
        status: StoreVerifyStatus::Ok,
        checked,
        drifted: 0,
        repaired: 0,
        unknown_reason: None,
        audit_ref: None,
        remediation: None,
    }
}

fn write_drift(
    vm: &str,
    store_root: &Path,
    generation_id: &str,
    checked: u32,
    drift: Vec<String>,
    repair: bool,
) -> StoreVerifyResponse {
    let drifted = u32::try_from(drift.len()).unwrap_or(u32::MAX);
    let record = IntegrityRecord::suspect(generation_id, drift);
    if let Err(err) = write_integrity_record(
        &generation_integrity_path(store_root, generation_id),
        &record,
    ) {
        return failed(vm, format!("write suspect integrity: {err}"));
    }
    StoreVerifyResponse {
        vm: vm.to_owned(),
        status: StoreVerifyStatus::Drift,
        checked,
        drifted,
        repaired: 0,
        unknown_reason: None,
        audit_ref: None,
        remediation: Some(if repair {
            "repair path not available yet; inspect audit_ref and broker logs".to_owned()
        } else {
            "rerun with --repair to repair live-pool drift".to_owned()
        }),
    }
}

fn write_unknown(
    vm: &str,
    store_root: &Path,
    generation_id: &str,
    checked: u32,
    reason: StoreVerifyUnknownReason,
) -> StoreVerifyResponse {
    let record = IntegrityRecord::unknown(Some(generation_id), reason);
    if let Err(err) = write_integrity_record(
        &generation_integrity_path(store_root, generation_id),
        &record,
    ) {
        return failed(vm, format!("write unknown integrity: {err}"));
    }
    unknown(vm, checked, reason)
}

fn unknown(vm: &str, checked: u32, reason: StoreVerifyUnknownReason) -> StoreVerifyResponse {
    StoreVerifyResponse {
        vm: vm.to_owned(),
        status: StoreVerifyStatus::Unknown,
        checked,
        drifted: 0,
        repaired: 0,
        unknown_reason: Some(reason),
        audit_ref: None,
        remediation: Some(match reason {
            StoreVerifyUnknownReason::MarkerOrManifestMissing => {
                "run with --repair or activate a new generation to recreate marker/manifest state"
            }
            StoreVerifyUnknownReason::MarkerOrManifestUnreadable => {
                "fix permissions or storage errors, then rerun verify"
            }
            StoreVerifyUnknownReason::OlderHostGeneration => {
                "activate a current store-view-capable generation, then rerun verify"
            }
            StoreVerifyUnknownReason::GenerationIdentityUnavailable => {
                "restore state/current or activate a new generation, then rerun verify"
            }
        }
        .to_owned()),
    }
}

fn failed(vm: &str, detail: String) -> StoreVerifyResponse {
    StoreVerifyResponse {
        vm: vm.to_owned(),
        status: StoreVerifyStatus::Failed,
        checked: 0,
        drifted: 0,
        repaired: 0,
        unknown_reason: None,
        audit_ref: None,
        remediation: Some(format!(
            "inspect audit_ref and broker logs, then retry ({detail})"
        )),
    }
}

pub fn not_found(vm: &str) -> StoreVerifyResponse {
    StoreVerifyResponse {
        vm: vm.to_owned(),
        status: StoreVerifyStatus::NotFound,
        checked: 0,
        drifted: 0,
        repaired: 0,
        unknown_reason: None,
        audit_ref: None,
        remediation: Some("check the VM name, declaration, and authorization".to_owned()),
    }
}

fn acquire_verify_lock(farm_root: &Path) -> Result<File, String> {
    std::fs::create_dir_all(farm_root).map_err(|err| format!("create farm root: {err}"))?;
    let path = hardlink_farm::sync_lock_path(farm_root);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| format!("open {}: {err}", path.display()))?;
    flock(file.as_raw_fd(), FlockArg::LockExclusive)
        .map_err(|err| format!("lock {}: {err}", path.display()))?;
    Ok(file)
}

fn generation_integrity_path(store_root: &Path, generation_id: &str) -> PathBuf {
    hardlink_farm::state_generation_dir(store_root, generation_id).join("integrity.json")
}

fn vm_unknown_integrity_path(store_root: &Path) -> PathBuf {
    hardlink_farm::state_dir(store_root).join("integrity-unknown.json")
}

fn write_integrity_record(path: &Path, record: &IntegrityRecord) -> std::io::Result<()> {
    record
        .validate()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    posture_host_only_file(path).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("posture integrity record: {err}"),
        )
    })?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_host::hardlink_farm::{GenerationMarker, StoreViewLinkCounts};
    use tempfile::tempdir;

    fn fake_closure(root: &Path, names: &[&str]) -> Vec<PathBuf> {
        let store = root.join("nix-store");
        std::fs::create_dir_all(&store).unwrap();
        names
            .iter()
            .map(|name| {
                let path = store.join(name);
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("payload"), name).unwrap();
                path
            })
            .collect()
    }

    fn intent(root: &Path, vm: &str, closure: Vec<PathBuf>) -> ResolvedStoreViewIntent {
        let farm = root.join("vms").join(vm).join("store-view");
        let db_dump_path = root.join("db.dump");
        std::fs::write(&db_dump_path, b"db").unwrap();
        ResolvedStoreViewIntent {
            intent_id: format!("store-view:vm:{vm}"),
            vm: vm.to_owned(),
            generation: 7,
            hardlink_farm_path: farm,
            target_view_path: root.join("target"),
            closure_paths: closure,
            db_dump_path,
        }
    }

    fn publish(intent: &ResolvedStoreViewIntent) -> String {
        let generation_id = crate::ops::store_sync::generation_id_for_intent(intent);
        let marker = GenerationMarker {
            closure_hash: intent.closure_identity(),
            nixling_version: "test".to_owned(),
            activated_at: "test".to_owned(),
            vm: intent.vm.clone(),
            generation_number: 7,
        };
        let counts = hardlink_farm::build_store_view(
            &intent.hardlink_farm_path,
            &generation_id,
            &intent.closure_paths,
            &marker,
        )
        .unwrap();
        assert_eq!(
            counts,
            StoreViewLinkCounts {
                linked: 2,
                skipped: 0
            }
        );
        hardlink_farm::write_meta_db_dump(
            &intent.hardlink_farm_path,
            &generation_id,
            &intent.db_dump_path,
        )
        .unwrap();
        hardlink_farm::swap_state_current(&intent.hardlink_farm_path, &generation_id).unwrap();
        hardlink_farm::swap_meta_current(&intent.hardlink_farm_path, &generation_id).unwrap();
        hardlink_farm::plant_live_marker(&intent.hardlink_farm_path, &intent.vm).unwrap();
        generation_id
    }

    #[test]
    fn clean_generation_returns_ok_and_writes_integrity() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Ok);
        assert_eq!(response.checked, 2);
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("\"state\": \"ok\""));
    }

    #[test]
    fn missing_live_basename_returns_drift_without_repair() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        std::fs::remove_dir_all(
            hardlink_farm::live_dir(&intent.hardlink_farm_path).join("aaaaaaaaaaaaaaaa-alpha"),
        )
        .unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Drift);
        assert_eq!(response.drifted, 1);
        assert_eq!(response.repaired, 0);
        assert!(response
            .remediation
            .as_deref()
            .unwrap()
            .contains("rerun with --repair"));
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("\"state\": \"suspect\""));
        assert!(raw.contains("aaaaaaaaaaaaaaaa-alpha"));
    }

    #[test]
    fn repair_missing_live_basename_returns_repaired_after_second_verify() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        let missing =
            hardlink_farm::live_dir(&intent.hardlink_farm_path).join("aaaaaaaaaaaaaaaa-alpha");
        std::fs::remove_dir_all(&missing).unwrap();

        let response = run_store_verify(&intent, true);
        assert_eq!(response.status, StoreVerifyStatus::Repaired);
        assert_eq!(response.checked, 2);
        assert_eq!(response.drifted, 0);
        assert_eq!(response.repaired, 1);
        assert!(
            missing.exists(),
            "repair should re-materialize missing top-level basename"
        );
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("\"state\": \"ok\""));
    }

    #[test]
    fn internal_live_tree_drift_returns_drift() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        let live_alpha =
            hardlink_farm::live_dir(&intent.hardlink_farm_path).join("aaaaaaaaaaaaaaaa-alpha");
        std::fs::remove_dir_all(&live_alpha).unwrap();
        std::fs::create_dir_all(&live_alpha).unwrap();
        std::fs::write(live_alpha.join("payload"), b"drifted").unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Drift);
        assert_eq!(response.checked, 2);
        assert_eq!(response.drifted, 1);
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("aaaaaaaaaaaaaaaa-alpha"));
    }

    #[test]
    fn repair_internal_live_tree_drift_stays_drift_until_replace_wave() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        publish(&intent);
        let live_alpha =
            hardlink_farm::live_dir(&intent.hardlink_farm_path).join("aaaaaaaaaaaaaaaa-alpha");
        std::fs::remove_dir_all(&live_alpha).unwrap();
        std::fs::create_dir_all(&live_alpha).unwrap();
        std::fs::write(live_alpha.join("payload"), b"drifted").unwrap();

        let response = run_store_verify(&intent, true);
        assert_eq!(response.status, StoreVerifyStatus::Drift);
        assert_eq!(response.drifted, 1);
        assert_eq!(response.repaired, 0);
        assert!(response
            .remediation
            .as_deref()
            .unwrap()
            .contains("repair incomplete"));
    }

    #[test]
    fn missing_current_returns_vm_level_unknown() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        std::fs::create_dir_all(&intent.hardlink_farm_path).unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Unknown);
        assert_eq!(
            response.unknown_reason,
            Some(StoreVerifyUnknownReason::GenerationIdentityUnavailable)
        );
        let raw =
            std::fs::read_to_string(vm_unknown_integrity_path(&intent.hardlink_farm_path)).unwrap();
        assert!(raw.contains("\"generation_id\": null"));
        assert!(raw.contains("generation_identity_unavailable"));
    }

    #[test]
    fn meta_current_divergence_returns_drift() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        let other_id = "g-other";
        std::fs::create_dir_all(hardlink_farm::meta_generation_dir(
            &intent.hardlink_farm_path,
            other_id,
        ))
        .unwrap();
        hardlink_farm::swap_meta_current(&intent.hardlink_farm_path, other_id).unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Drift);
        assert_eq!(response.drifted, 1);
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("\"state\": \"suspect\""));
        assert!(raw.contains("meta/current"));
    }

    #[test]
    fn marker_vm_mismatch_returns_drift() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        let state_gen =
            hardlink_farm::state_generation_dir(&intent.hardlink_farm_path, &generation_id);
        let mut marker = hardlink_farm::read_generation_marker(&state_gen).unwrap();
        marker.vm = "other-vm".to_owned();
        hardlink_farm::write_generation_marker(&state_gen, &marker).unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Drift);
        assert_eq!(response.drifted, 1);
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("marker.json"));
    }

    #[test]
    fn missing_marker_returns_generation_scoped_unknown() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        std::fs::remove_file(
            hardlink_farm::state_generation_dir(&intent.hardlink_farm_path, &generation_id)
                .join("marker.json"),
        )
        .unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Unknown);
        assert_eq!(
            response.unknown_reason,
            Some(StoreVerifyUnknownReason::MarkerOrManifestMissing)
        );
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("\"state\": \"unknown\""));
        assert!(raw.contains("marker_or_manifest_missing"));
    }

    #[test]
    fn unreadable_marker_returns_generation_scoped_unknown() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        std::fs::write(
            hardlink_farm::state_generation_dir(&intent.hardlink_farm_path, &generation_id)
                .join("marker.json"),
            b"not-json",
        )
        .unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Unknown);
        assert_eq!(
            response.unknown_reason,
            Some(StoreVerifyUnknownReason::MarkerOrManifestUnreadable)
        );
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains("marker_or_manifest_unreadable"));
    }

    #[test]
    fn nonzero_live_marker_returns_drift() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        std::fs::write(
            hardlink_farm::live_dir(&intent.hardlink_farm_path).join(".nixling-marker-vm-a"),
            b"payload-is-drift",
        )
        .unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Drift);
        assert_eq!(response.drifted, 1);
        let raw = std::fs::read_to_string(generation_integrity_path(
            &intent.hardlink_farm_path,
            &generation_id,
        ))
        .unwrap();
        assert!(raw.contains(".nixling-marker-vm-a"));
    }

    #[test]
    fn repair_nonzero_live_marker_replants_zero_length_marker() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        publish(&intent);
        let marker =
            hardlink_farm::live_dir(&intent.hardlink_farm_path).join(".nixling-marker-vm-a");
        std::fs::write(&marker, b"payload-is-drift").unwrap();

        let response = run_store_verify(&intent, true);
        assert_eq!(response.status, StoreVerifyStatus::Repaired);
        assert_eq!(std::fs::metadata(&marker).unwrap().len(), 0);
    }

    #[test]
    fn missing_store_paths_returns_unknown() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        std::fs::remove_file(
            hardlink_farm::meta_generation_dir(&intent.hardlink_farm_path, &generation_id)
                .join("store-paths"),
        )
        .unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Unknown);
        assert_eq!(
            response.unknown_reason,
            Some(StoreVerifyUnknownReason::MarkerOrManifestMissing)
        );
    }

    #[test]
    fn repair_missing_store_paths_recreates_manifest() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        let store_paths =
            hardlink_farm::meta_generation_dir(&intent.hardlink_farm_path, &generation_id)
                .join("store-paths");
        std::fs::remove_file(&store_paths).unwrap();

        let response = run_store_verify(&intent, true);
        assert_eq!(response.status, StoreVerifyStatus::Repaired);
        assert!(
            store_paths.is_file(),
            "repair should rewrite guest manifest"
        );
    }

    #[test]
    fn unreadable_store_paths_returns_unknown() {
        let tmp = tempdir().unwrap();
        let closure = fake_closure(
            tmp.path(),
            &["aaaaaaaaaaaaaaaa-alpha", "bbbbbbbbbbbbbbbb-beta"],
        );
        let intent = intent(tmp.path(), "vm-a", closure);
        let generation_id = publish(&intent);
        let store_paths =
            hardlink_farm::meta_generation_dir(&intent.hardlink_farm_path, &generation_id)
                .join("store-paths");
        std::fs::remove_file(&store_paths).unwrap();
        std::fs::create_dir(&store_paths).unwrap();

        let response = run_store_verify(&intent, false);
        assert_eq!(response.status, StoreVerifyStatus::Unknown);
        assert_eq!(
            response.unknown_reason,
            Some(StoreVerifyUnknownReason::MarkerOrManifestUnreadable)
        );
    }

    #[test]
    fn not_found_response_has_signed_remediation() {
        let response = not_found("missing-vm");
        assert_eq!(response.status, StoreVerifyStatus::NotFound);
        assert_eq!(response.vm, "missing-vm");
        assert!(response.remediation.unwrap().contains("VM name"));
    }
}
