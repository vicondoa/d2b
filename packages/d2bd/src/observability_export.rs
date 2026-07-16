use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use d2b_contracts::{
    public_wire::OBSERVABILITY_EXPORT_INSPECT_MAX_BYTES,
    v2_provider::{
        Fingerprint, MAX_OBSERVABILITY_QUERY_BYTES, ObservabilityExportFormat, OperationId,
    },
};
use sha2::{Digest, Sha256};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);
const EXPORT_DIRECTORY: &str = "observability-exports";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservabilityExportStoreError {
    BoundsExceeded,
    StorageUnavailable,
    CompletionAmbiguous,
    NotFound,
    InvalidArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservabilityExportInspection {
    pub(crate) encoded_bytes: u32,
    pub(crate) format: ObservabilityExportFormat,
    pub(crate) digest: Fingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservabilityExportChunk {
    pub(crate) inspection: ObservabilityExportInspection,
    pub(crate) offset: u32,
    pub(crate) bytes: Vec<u8>,
    pub(crate) complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ObservabilityExportLookup {
    Missing,
    Available(ObservabilityExportChunk),
}

#[derive(Clone)]
pub(crate) struct ObservabilityExportStore {
    root: PathBuf,
    directory_sync: fn(&Path) -> std::io::Result<()>,
}

impl std::fmt::Debug for ObservabilityExportStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservabilityExportStore")
            .finish_non_exhaustive()
    }
}

struct TempArtifact {
    path: PathBuf,
    committed: bool,
}

impl Drop for TempArtifact {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl ObservabilityExportStore {
    pub(crate) fn new(daemon_state_dir: &Path) -> Self {
        Self {
            root: daemon_state_dir.join(EXPORT_DIRECTORY),
            directory_sync: Self::sync_directory,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_directory_sync(
        daemon_state_dir: &Path,
        directory_sync: fn(&Path) -> std::io::Result<()>,
    ) -> Self {
        Self {
            root: daemon_state_dir.join(EXPORT_DIRECTORY),
            directory_sync,
        }
    }

    fn sync_directory(path: &Path) -> std::io::Result<()> {
        let directory = File::open(path)?;
        directory.sync_all()
    }

    fn extension(format: ObservabilityExportFormat) -> &'static str {
        match format {
            ObservabilityExportFormat::JsonLines => "jsonl",
            ObservabilityExportFormat::OtlpProtobuf => "otlp.pb",
        }
    }

    fn artifact_path(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
    ) -> PathBuf {
        self.root.join(format!(
            "{}.{}",
            operation_id.as_str(),
            Self::extension(format)
        ))
    }

    fn temp_path(&self, operation_id: &OperationId, format: ObservabilityExportFormat) -> PathBuf {
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        self.root.join(format!(
            ".{}.{}.{}.{}.tmp",
            operation_id.as_str(),
            Self::extension(format),
            std::process::id(),
            sequence
        ))
    }

    pub(crate) fn persist(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
        payload: &[u8],
        record_count: u16,
        max_records: u16,
        max_bytes: u32,
    ) -> Result<ObservabilityExportInspection, ObservabilityExportStoreError> {
        if record_count > max_records
            || payload.len() > usize::try_from(max_bytes).unwrap_or(usize::MAX)
        {
            return Err(ObservabilityExportStoreError::BoundsExceeded);
        }

        fs::create_dir_all(&self.root)
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;

        let temp_path = self.temp_path(operation_id, format);
        let mut temp = TempArtifact {
            path: temp_path.clone(),
            committed: false,
        };
        {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temp_path)
                .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
            file.write_all(payload)
                .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
            file.sync_all()
                .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        }

        let artifact_path = self.artifact_path(operation_id, format);
        fs::rename(&temp_path, &artifact_path)
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        temp.committed = true;
        (self.directory_sync)(&self.root)
            .map_err(|_| ObservabilityExportStoreError::CompletionAmbiguous)?;
        self.inspect(operation_id, format)
    }

    pub(crate) fn inspect(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
    ) -> Result<ObservabilityExportInspection, ObservabilityExportStoreError> {
        self.read_artifact(operation_id, format)?
            .map(|artifact| artifact.inspection)
            .ok_or(ObservabilityExportStoreError::NotFound)
    }

    pub(crate) fn lookup(
        &self,
        operation_id: &OperationId,
        offset: u32,
        max_bytes: u32,
    ) -> Result<ObservabilityExportLookup, ObservabilityExportStoreError> {
        if max_bytes == 0 || max_bytes > OBSERVABILITY_EXPORT_INSPECT_MAX_BYTES {
            return Err(ObservabilityExportStoreError::BoundsExceeded);
        }
        let json = self.read_artifact(operation_id, ObservabilityExportFormat::JsonLines)?;
        let otlp = self.read_artifact(operation_id, ObservabilityExportFormat::OtlpProtobuf)?;
        let artifact = match (json, otlp) {
            (None, None) => return Ok(ObservabilityExportLookup::Missing),
            (Some(artifact), None) | (None, Some(artifact)) => artifact,
            (Some(_), Some(_)) => return Err(ObservabilityExportStoreError::InvalidArtifact),
        };
        let start =
            usize::try_from(offset).map_err(|_| ObservabilityExportStoreError::BoundsExceeded)?;
        if start > artifact.bytes.len() {
            return Err(ObservabilityExportStoreError::BoundsExceeded);
        }
        let length = usize::try_from(max_bytes)
            .map_err(|_| ObservabilityExportStoreError::BoundsExceeded)?;
        let end = start.saturating_add(length).min(artifact.bytes.len());
        Ok(ObservabilityExportLookup::Available(
            ObservabilityExportChunk {
                inspection: artifact.inspection,
                offset,
                bytes: artifact.bytes[start..end].to_vec(),
                complete: end == artifact.bytes.len(),
            },
        ))
    }

    fn read_artifact(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
    ) -> Result<Option<StoredArtifact>, ObservabilityExportStoreError> {
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.artifact_path(operation_id, format))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ObservabilityExportStoreError::StorageUnavailable),
        };
        let metadata = file
            .metadata()
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(ObservabilityExportStoreError::InvalidArtifact);
        }
        if metadata.len() > u64::from(MAX_OBSERVABILITY_QUERY_BYTES) {
            return Err(ObservabilityExportStoreError::BoundsExceeded);
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(metadata.len())
                .map_err(|_| ObservabilityExportStoreError::BoundsExceeded)?,
        );
        Read::by_ref(&mut file)
            .take(u64::from(MAX_OBSERVABILITY_QUERY_BYTES) + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        if bytes.len() > usize::try_from(MAX_OBSERVABILITY_QUERY_BYTES).unwrap_or(usize::MAX) {
            return Err(ObservabilityExportStoreError::BoundsExceeded);
        }
        if u64::try_from(bytes.len()).ok() != Some(metadata.len()) {
            return Err(ObservabilityExportStoreError::InvalidArtifact);
        }
        let digest = Fingerprint::parse(format!("{:x}", Sha256::digest(&bytes)))
            .map_err(|_| ObservabilityExportStoreError::InvalidArtifact)?;
        Ok(Some(StoredArtifact {
            inspection: ObservabilityExportInspection {
                encoded_bytes: u32::try_from(bytes.len())
                    .map_err(|_| ObservabilityExportStoreError::BoundsExceeded)?,
                format,
                digest,
            },
            bytes,
        }))
    }

    #[cfg(test)]
    pub(crate) fn read(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
    ) -> std::io::Result<Vec<u8>> {
        self.read_artifact(operation_id, format)
            .map_err(|error| std::io::Error::other(format!("{error:?}")))?
            .map(|artifact| artifact.bytes)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }
}

struct StoredArtifact {
    inspection: ObservabilityExportInspection,
    bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_export_is_private_inspectable_and_cleans_failed_temp() {
        let state = tempfile::tempdir().expect("state directory");
        let store = ObservabilityExportStore::new(state.path());
        let operation = OperationId::parse("export-operation").expect("operation id");
        let payload = br#"{"kind":"bounded"}\n"#;
        let inspection = store
            .persist(
                &operation,
                ObservabilityExportFormat::JsonLines,
                payload,
                1,
                1,
                1_024,
            )
            .expect("persist export");
        assert_eq!(
            inspection.encoded_bytes,
            u32::try_from(payload.len()).expect("payload length")
        );
        assert_eq!(inspection.format, ObservabilityExportFormat::JsonLines);
        assert_eq!(
            inspection.digest.as_str(),
            format!("{:x}", Sha256::digest(payload))
        );
        assert_eq!(
            store
                .read(&operation, ObservabilityExportFormat::JsonLines)
                .expect("read export"),
            payload
        );
        let metadata =
            fs::metadata(store.artifact_path(&operation, ObservabilityExportFormat::JsonLines))
                .expect("export metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(
            fs::read_dir(&store.root)
                .expect("list export store")
                .all(|entry| !entry
                    .expect("export entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );

        let blocked = OperationId::parse("blocked-export").expect("operation id");
        fs::create_dir_all(store.artifact_path(&blocked, ObservabilityExportFormat::OtlpProtobuf))
            .expect("block final rename");
        assert_eq!(
            store.persist(
                &blocked,
                ObservabilityExportFormat::OtlpProtobuf,
                b"payload",
                1,
                1,
                1_024,
            ),
            Err(ObservabilityExportStoreError::StorageUnavailable)
        );
        assert!(
            fs::read_dir(&store.root)
                .expect("list export store")
                .all(|entry| !entry
                    .expect("export entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn post_rename_sync_failure_is_ambiguous_and_lookup_resolves_artifact() {
        fn fail_directory_sync(_path: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other("injected directory sync failure"))
        }

        let state = tempfile::tempdir().expect("state directory");
        let store =
            ObservabilityExportStore::with_directory_sync(state.path(), fail_directory_sync);
        let operation = OperationId::parse("ambiguous-export").expect("operation id");
        let payload = b"bounded-export";
        assert_eq!(
            store.persist(
                &operation,
                ObservabilityExportFormat::JsonLines,
                payload,
                1,
                1,
                1_024,
            ),
            Err(ObservabilityExportStoreError::CompletionAmbiguous)
        );

        let lookup = ObservabilityExportStore::new(state.path())
            .lookup(&operation, 0, 7)
            .expect("lookup ambiguous completion");
        let ObservabilityExportLookup::Available(first) = lookup else {
            panic!("renamed artifact must be available");
        };
        assert_eq!(first.bytes, b"bounded");
        assert!(!first.complete);
        assert_eq!(first.inspection.encoded_bytes, payload.len() as u32);
        assert_eq!(
            first.inspection.digest.as_str(),
            format!("{:x}", Sha256::digest(payload))
        );

        let second = ObservabilityExportStore::new(state.path())
            .lookup(&operation, 7, OBSERVABILITY_EXPORT_INSPECT_MAX_BYTES)
            .expect("lookup remainder");
        let ObservabilityExportLookup::Available(second) = second else {
            panic!("renamed artifact must be available");
        };
        assert_eq!(second.bytes, b"-export");
        assert!(second.complete);
        assert_eq!(
            ObservabilityExportStore::new(state.path()).lookup(
                &OperationId::parse("missing-export").expect("operation id"),
                0,
                1,
            ),
            Ok(ObservabilityExportLookup::Missing)
        );
        assert_eq!(
            ObservabilityExportStore::new(state.path()).lookup(
                &operation,
                u32::try_from(payload.len() + 1).expect("offset"),
                1,
            ),
            Err(ObservabilityExportStoreError::BoundsExceeded)
        );
    }
}
