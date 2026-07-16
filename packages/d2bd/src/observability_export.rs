use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use d2b_contracts::v2_provider::{ObservabilityExportFormat, OperationId};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);
const EXPORT_DIRECTORY: &str = "observability-exports";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservabilityExportStoreError {
    BoundsExceeded,
    StorageUnavailable,
    InvalidArtifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ObservabilityExportInspection {
    pub(crate) encoded_bytes: u32,
}

#[derive(Clone)]
pub(crate) struct ObservabilityExportStore {
    root: PathBuf,
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
        }
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
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        self.inspect(operation_id, format)
    }

    pub(crate) fn inspect(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
    ) -> Result<ObservabilityExportInspection, ObservabilityExportStoreError> {
        let metadata = fs::symlink_metadata(self.artifact_path(operation_id, format))
            .map_err(|_| ObservabilityExportStoreError::StorageUnavailable)?;
        if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(ObservabilityExportStoreError::InvalidArtifact);
        }
        let encoded_bytes = u32::try_from(metadata.len())
            .map_err(|_| ObservabilityExportStoreError::BoundsExceeded)?;
        Ok(ObservabilityExportInspection { encoded_bytes })
    }

    #[cfg(test)]
    pub(crate) fn read(
        &self,
        operation_id: &OperationId,
        format: ObservabilityExportFormat,
    ) -> std::io::Result<Vec<u8>> {
        fs::read(self.artifact_path(operation_id, format))
    }
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
}
