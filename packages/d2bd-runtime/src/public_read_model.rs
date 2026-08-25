use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use arc_swap::ArcSwapOption;
use d2b_contracts_control::public_wire;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{daemon_config::ArtifactPaths, typed_error::TypedError};

pub fn request_invalidates_public_status_model(request: &crate::wire::Request) -> bool {
    !matches!(
        request,
        crate::wire::Request::List(_)
            | crate::wire::Request::Status(_)
            | crate::wire::Request::Audit(_)
            | crate::wire::Request::HostCheck(_)
            | crate::wire::Request::AuthStatus
            | crate::wire::Request::KeysList
            | crate::wire::Request::KeysShow(_)
            | crate::wire::Request::Workload(public_wire::WorkloadOp::List(_))
            | crate::wire::Request::Workload(public_wire::WorkloadOp::Status(_))
            | crate::wire::Request::Audio(public_wire::AudioOp::Status(_))
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicArtifactFingerprint {
    pub current_system: Option<String>,
    pub pidfd_generation: u64,
    pub public_manifest: FileFingerprint,
    pub host: FileFingerprint,
    pub processes: FileFingerprint,
    pub bundle: FileFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileFingerprint {
    pub path: String,
    pub len: u64,
    pub modified_nanos: Option<u128>,
    pub symlink_target: Option<String>,
}

#[derive(Debug)]
pub struct CachedPublicFrame {
    pub fingerprint: PublicArtifactFingerprint,
    pub value: Value,
}

#[derive(Debug)]
pub struct PublicStatusReadModel {
    generation: AtomicU64,
    latest_published_generation: AtomicU64,
    list: ArcSwapOption<CachedPublicFrame>,
    status: ArcSwapOption<CachedPublicFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicReadModelKind {
    List,
    Status,
}

impl PublicStatusReadModel {
    pub fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            latest_published_generation: AtomicU64::new(0),
            list: ArcSwapOption::empty(),
            status: ArcSwapOption::empty(),
        }
    }

    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.list.store(None);
        self.status.store(None);
    }

    pub fn load_list(&self, pidfd_generation: u64) -> Option<Value> {
        self.load_if_fresh(pidfd_generation, &self.list)
    }

    pub fn load_status(&self, pidfd_generation: u64) -> Option<Value> {
        self.load_if_fresh(pidfd_generation, &self.status)
    }

    pub fn publish_if_unchanged(
        &self,
        kind: PublicReadModelKind,
        before: Option<PublicArtifactFingerprint>,
        current: Option<PublicArtifactFingerprint>,
        value: Value,
        kind_name: &'static str,
    ) -> Value {
        let Some(fingerprint) = before else {
            return value;
        };
        if current.as_ref() == Some(&fingerprint) {
            self.publish_stable(kind, value, fingerprint, kind_name)
        } else {
            value
        }
    }

    fn load_if_fresh(
        &self,
        pidfd_generation: u64,
        slot: &ArcSwapOption<CachedPublicFrame>,
    ) -> Option<Value> {
        let cached = slot.load_full()?;
        (cached.fingerprint.pidfd_generation == pidfd_generation).then(|| cached.value.clone())
    }

    fn publish_stable(
        &self,
        kind: PublicReadModelKind,
        value: Value,
        fingerprint: PublicArtifactFingerprint,
        kind_name: &'static str,
    ) -> Value {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let value = attach_read_model_metadata(value, &fingerprint, generation, kind_name);
        let mut observed = self.latest_published_generation.load(Ordering::Acquire);
        while generation > observed {
            match self.latest_published_generation.compare_exchange_weak(
                observed,
                generation,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let frame = Arc::new(CachedPublicFrame {
                        fingerprint,
                        value: value.clone(),
                    });
                    match kind {
                        PublicReadModelKind::List => self.list.store(Some(frame)),
                        PublicReadModelKind::Status => self.status.store(Some(frame)),
                    }
                    return value;
                }
                Err(next) => observed = next,
            }
        }
        tracing::debug!(
            read_model_kind = kind_name,
            generation,
            latest_generation = observed,
            "skipped stale public read-model publish"
        );
        value
    }
}

impl Default for PublicStatusReadModel {
    fn default() -> Self {
        Self::new()
    }
}

pub fn public_artifact_fingerprint(
    artifacts: &ArtifactPaths,
    pidfd_generation: u64,
) -> Result<PublicArtifactFingerprint, TypedError> {
    Ok(PublicArtifactFingerprint {
        current_system: fs::read_link("/run/current-system")
            .ok()
            .map(|path| path.display().to_string()),
        pidfd_generation,
        public_manifest: file_fingerprint(&artifacts.public_manifest_path)?,
        host: file_fingerprint(&artifacts.host_path)?,
        processes: file_fingerprint(&artifacts.processes_path)?,
        bundle: file_fingerprint(&artifacts.bundle_path)?,
    })
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint, TypedError> {
    let metadata = fs::metadata(path).map_err(|error| TypedError::InternalIo {
        context: format!("fingerprint {}", path.display()),
        detail: error.to_string(),
    })?;
    Ok(FileFingerprint {
        path: path.display().to_string(),
        len: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos()),
        symlink_target: fs::read_link(path)
            .ok()
            .map(|target| target.display().to_string()),
    })
}

fn attach_read_model_metadata(
    mut frame: Value,
    fingerprint: &PublicArtifactFingerprint,
    generation: u64,
    kind: &'static str,
) -> Value {
    let metadata = json!({
        "schemaVersion": 1,
        "kind": kind,
        "generation": generation,
        "sourceFingerprint": public_artifact_fingerprint_hash(fingerprint),
        "updatedAtUnixMs": system_unix_millis(),
        "freshness": "fresh",
        "deepRefresh": "available",
    });
    if kind == "status"
        && let Some(status) = frame.get_mut("status").and_then(Value::as_object_mut)
    {
        status.insert("readModel".to_owned(), metadata);
        return frame;
    }
    if let Some(object) = frame.as_object_mut() {
        object.insert("readModel".to_owned(), metadata);
    }
    frame
}

fn public_artifact_fingerprint_hash(fingerprint: &PublicArtifactFingerprint) -> String {
    let bytes = serde_json::to_vec(fingerprint).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn system_unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
