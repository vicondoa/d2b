//! Broker-owned cutover artifact quarantine and finalization.
//!
//! All paths are derived from opaque operation/artifact ids under the broker
//! state root. Marker contents are checked before any rename or removal.
//! The broker is the only repair owner for these operation-scoped paths.

use std::{
    fs::{self, File},
    io::Read as _,
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use d2b_contracts_broker::broker_wire::CanonicalAuditDigest;
use d2b_contracts::{types::BundleOpId};
use d2b_contracts_zone_session::v3::ArtifactId;
use nix::unistd::geteuid;
use rustix::fs::{
    AtFlags, CWD, FileType, Mode, OFlags, ResolveFlags, fstat, fsync, openat2, statat, unlinkat,
};
use sha2::{Digest as _, Sha256};

const STAGED_MARKER: &str = ".d2b-staged-marker";
const LEGACY_MARKER: &str = ".d2b-legacy-marker";
const VOLUME_MARKER: &str = ".d2b-durable-volume-marker";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutoverArtifactError {
    InvalidId,
    ForeignOwner,
    Missing,
    Marker,
    Digest,
    SourceMissing,
    DestinationExists,
    Io,
}

impl std::fmt::Display for CutoverArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidId => "cutover-artifact-invalid-id",
            Self::ForeignOwner => "cutover-artifact-foreign-owner",
            Self::Missing => "cutover-artifact-missing",
            Self::Marker => "cutover-artifact-marker-invalid",
            Self::Digest => "cutover-artifact-digest-mismatch",
            Self::SourceMissing => "cutover-artifact-source-missing",
            Self::DestinationExists => "cutover-artifact-destination-exists",
            Self::Io => "cutover-artifact-io",
        })
    }
}

impl std::error::Error for CutoverArtifactError {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedMarker {
    operation_id: String,
    staged_id: String,
    source_id: String,
    marker_digest: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyMarker {
    operation_id: String,
    artifact_id: String,
    disposition_digest: String,
    kind: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VolumeMarker {
    operation_id: String,
    storage_ref: String,
    marker_digest: String,
    source_preserved: bool,
}

/// Move one operation-owned staged destination into its quarantine area.
pub fn quarantine_staged_destination(
    state_dir: &Path,
    operation_id: &BundleOpId,
    staged_id: &BundleOpId,
    source_id: &BundleOpId,
    marker_digest: &CanonicalAuditDigest,
) -> Result<(), CutoverArtifactError> {
    validate_id(operation_id.as_str())?;
    validate_id(staged_id.as_str())?;
    validate_id(source_id.as_str())?;
    let operation_root = operation_root(state_dir, operation_id)?;
    let staged = operation_root.join("staged").join(staged_id.as_str());
    let source = operation_root.join("sources").join(source_id.as_str());
    let staged_fd = open_owned_directory(&staged)?;
    let marker_bytes = read_owned_file_at(&staged_fd, STAGED_MARKER)?;
    let marker: StagedMarker =
        serde_json::from_slice(&marker_bytes).map_err(|_| CutoverArtifactError::Marker)?;
    let supplied_marker_digest = marker.marker_digest.clone();
    let mut unsigned_marker = marker.clone();
    unsigned_marker.marker_digest.clear();
    let unsigned_bytes =
        serde_json::to_vec(&unsigned_marker).map_err(|_| CutoverArtifactError::Marker)?;
    if marker.operation_id != operation_id.as_str()
        || marker.staged_id != staged_id.as_str()
        || marker.source_id != source_id.as_str()
        || digest_bytes("d2b:cutover:staged-marker:v1", &unsigned_bytes)?.as_str()
            != supplied_marker_digest
        || supplied_marker_digest != marker_digest.as_str()
    {
        return Err(CutoverArtifactError::Digest);
    }
    require_owned_directory(&source)?;
    let quarantine_root = operation_root.join("quarantine");
    ensure_owned_directory(&quarantine_root)?;
    let destination = quarantine_root.join(staged_id.as_str());
    if destination.exists() {
        return Err(CutoverArtifactError::DestinationExists);
    }
    let staged_parent =
        open_owned_directory(staged.parent().ok_or(CutoverArtifactError::Missing)?)?;
    let quarantine_parent = open_owned_directory(&quarantine_root)?;
    rustix::fs::renameat(
        &staged_parent,
        staged_id.as_str(),
        &quarantine_parent,
        staged_id.as_str(),
    )
    .map_err(|_| CutoverArtifactError::Io)?;
    fsync(&quarantine_parent).map_err(|_| CutoverArtifactError::Io)?;
    Ok(())
}

/// Finalize only the approved, operation-owned legacy artifacts.
pub fn finalize_legacy_artifacts(
    state_dir: &Path,
    operation_id: &BundleOpId,
    artifacts: &[ArtifactId],
    disposition_digest: &CanonicalAuditDigest,
    consent_digest: &CanonicalAuditDigest,
) -> Result<(), CutoverArtifactError> {
    validate_id(operation_id.as_str())?;
    if artifacts.is_empty() || consent_digest.as_str().is_empty() {
        return Err(CutoverArtifactError::InvalidId);
    }
    let operation_root = operation_root(state_dir, operation_id)?;
    let legacy_root = operation_root.join("legacy");
    let finalized_root = operation_root.join("finalized");
    ensure_owned_directory(&finalized_root)?;
    let finalized_parent = open_owned_directory(&finalized_root)?;
    for artifact in artifacts {
        validate_id(artifact.as_str())?;
        let source = legacy_root.join(artifact.as_str());
        let source_metadata =
            fs::symlink_metadata(&source).map_err(|_| CutoverArtifactError::Missing)?;
        if source_metadata.uid() != geteuid().as_raw()
            || (!source_metadata.is_dir() && !source_metadata.is_file())
        {
            return Err(CutoverArtifactError::ForeignOwner);
        }
        let marker_path = if source_metadata.is_dir() {
            source.join(LEGACY_MARKER)
        } else {
            legacy_root.join(format!(".{}-legacy-marker", artifact.as_str()))
        };
        let marker_bytes = if source_metadata.is_dir() {
            let source_fd = open_owned_directory(&source)?;
            read_owned_file_at(&source_fd, LEGACY_MARKER)?
        } else {
            let legacy_parent =
                open_owned_directory(source.parent().ok_or(CutoverArtifactError::Missing)?)?;
            let marker_name = marker_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(CutoverArtifactError::Missing)?;
            read_owned_file_at(&legacy_parent, marker_name)?
        };
        let marker: LegacyMarker =
            serde_json::from_slice(&marker_bytes).map_err(|_| CutoverArtifactError::Marker)?;
        if marker.operation_id != operation_id.as_str()
            || marker.artifact_id != artifact.as_str()
            || marker.disposition_digest != disposition_digest.as_str()
            || !matches!(marker.kind.as_str(), "directory" | "regular-file")
        {
            return Err(CutoverArtifactError::Digest);
        }
        if marker.kind == "directory" {
            let destination = finalized_root.join(artifact.as_str());
            if destination.exists() {
                return Err(CutoverArtifactError::DestinationExists);
            }
            let source_fd = open_owned_directory(&source)?;
            let legacy_parent =
                open_owned_directory(source.parent().ok_or(CutoverArtifactError::Missing)?)?;
            rustix::fs::renameat(
                &legacy_parent,
                artifact.as_str(),
                &finalized_parent,
                artifact.as_str(),
            )
            .map_err(|_| CutoverArtifactError::Io)?;
            delete_tree_fd(&source_fd)?;
            unlinkat(&finalized_parent, artifact.as_str(), AtFlags::REMOVEDIR)
                .map_err(|_| CutoverArtifactError::Io)?;
        } else {
            let legacy_parent =
                open_owned_directory(source.parent().ok_or(CutoverArtifactError::Missing)?)?;
            rustix::fs::unlinkat(
                &legacy_parent,
                artifact.as_str(),
                rustix::fs::AtFlags::empty(),
            )
            .map_err(|_| CutoverArtifactError::Io)?;
            if marker_path != source {
                let marker_parent = open_owned_directory(
                    marker_path.parent().ok_or(CutoverArtifactError::Missing)?,
                )?;
                let marker_name = marker_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or(CutoverArtifactError::Missing)?;
                rustix::fs::unlinkat(&marker_parent, marker_name, rustix::fs::AtFlags::empty())
                    .map_err(|_| CutoverArtifactError::Io)?;
            }
        }
    }
    fsync(&finalized_parent).map_err(|_| CutoverArtifactError::Io)?;
    Ok(())
}

/// Destroy exactly one durable Volume after separate destructive consent.
pub fn destroy_durable_volume(
    state_dir: &Path,
    operation_id: &BundleOpId,
    storage_ref: &BundleOpId,
    marker_digest: &CanonicalAuditDigest,
    consent_digest: &CanonicalAuditDigest,
) -> Result<(), CutoverArtifactError> {
    validate_id(operation_id.as_str())?;
    validate_id(storage_ref.as_str())?;
    if consent_digest.as_str().is_empty() {
        return Err(CutoverArtifactError::InvalidId);
    }
    let operation_root = operation_root(state_dir, operation_id)?;
    let volume = operation_root.join("volumes").join(storage_ref.as_str());
    let volume_fd = open_owned_directory(&volume)?;
    let volume_parent =
        open_owned_directory(volume.parent().ok_or(CutoverArtifactError::Missing)?)?;
    let marker_bytes = read_owned_file_at(&volume_fd, VOLUME_MARKER)?;
    let marker: VolumeMarker =
        serde_json::from_slice(&marker_bytes).map_err(|_| CutoverArtifactError::Marker)?;
    let supplied_marker_digest = marker.marker_digest.clone();
    let mut unsigned_marker = marker.clone();
    unsigned_marker.marker_digest.clear();
    let unsigned_bytes =
        serde_json::to_vec(&unsigned_marker).map_err(|_| CutoverArtifactError::Marker)?;
    if marker.operation_id != operation_id.as_str()
        || marker.storage_ref != storage_ref.as_str()
        || digest_bytes("d2b:cutover:volume-marker:v1", &unsigned_bytes)?.as_str()
            != supplied_marker_digest
        || supplied_marker_digest != marker_digest.as_str()
        || !marker.source_preserved
    {
        return Err(CutoverArtifactError::Digest);
    }
    // The marker has authenticated the opened directory fd. Removal is
    // scoped to that one derived Volume root; no recursive sweep is allowed.
    delete_tree_fd(&volume_fd)?;
    unlinkat(&volume_parent, storage_ref.as_str(), AtFlags::REMOVEDIR)
        .map_err(|_| CutoverArtifactError::Io)?;
    fsync(&volume_parent).map_err(|_| CutoverArtifactError::Io)?;
    Ok(())
}

fn operation_root(
    state_dir: &Path,
    operation_id: &BundleOpId,
) -> Result<PathBuf, CutoverArtifactError> {
    let _operation_fd = open_operation_directory(state_dir, operation_id)?;
    let root = state_dir.join("cutover").join(operation_id.as_str());
    require_owned_directory(&root)?;
    Ok(root)
}

fn open_operation_directory(
    state_dir: &Path,
    operation_id: &BundleOpId,
) -> Result<OwnedFd, CutoverArtifactError> {
    require_owned_directory(state_dir)?;
    let state_fd = open_owned_directory(state_dir)?;
    let cutover_fd = openat2(
        &state_fd,
        "cutover",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| CutoverArtifactError::Io)?;
    openat2(
        &cutover_fd,
        operation_id.as_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| CutoverArtifactError::Io)
}

fn validate_id(value: &str) -> Result<(), CutoverArtifactError> {
    if value.is_empty()
        || value.len() > 128
        || value.contains('/')
        || value.contains('\\')
        || value.contains("..")
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(CutoverArtifactError::InvalidId)
    } else {
        Ok(())
    }
}

fn require_owned_directory(path: &Path) -> Result<(), CutoverArtifactError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CutoverArtifactError::Missing
        } else {
            CutoverArtifactError::Io
        }
    })?;
    if !metadata.is_dir() || metadata.uid() != geteuid().as_raw() {
        return Err(CutoverArtifactError::ForeignOwner);
    }
    Ok(())
}

fn ensure_owned_directory(path: &Path) -> Result<(), CutoverArtifactError> {
    if path.exists() {
        return require_owned_directory(path);
    }
    let parent = path.parent().ok_or(CutoverArtifactError::Missing)?;
    require_owned_directory(parent)?;
    fs::create_dir(path).map_err(|_| CutoverArtifactError::Io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| CutoverArtifactError::Io)?;
    Ok(())
}

fn read_owned_file_at(parent: &OwnedFd, name: &str) -> Result<Vec<u8>, CutoverArtifactError> {
    let metadata = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| CutoverArtifactError::Missing)?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile
        || metadata.st_uid != geteuid().as_raw()
        || metadata.st_mode & 0o777 != 0o600
    {
        return Err(CutoverArtifactError::ForeignOwner);
    }
    let fd = openat2(
        parent,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| CutoverArtifactError::Io)?;
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| CutoverArtifactError::Io)?;
    Ok(bytes)
}

fn open_owned_directory(path: &Path) -> Result<OwnedFd, CutoverArtifactError> {
    require_owned_directory(path)?;
    openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| CutoverArtifactError::Io)
}

fn delete_tree_fd(directory: &OwnedFd) -> Result<(), CutoverArtifactError> {
    let root_stat = fstat(directory).map_err(|_| CutoverArtifactError::Io)?;
    if FileType::from_raw_mode(root_stat.st_mode) != FileType::Directory
        || root_stat.st_uid != geteuid().as_raw()
    {
        return Err(CutoverArtifactError::ForeignOwner);
    }
    let root_device = root_stat.st_dev;
    let proc_path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    let entries = std::fs::read_dir(proc_path).map_err(|_| CutoverArtifactError::Io)?;
    for entry in entries {
        let entry = entry.map_err(|_| CutoverArtifactError::Io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| CutoverArtifactError::Io)?;
        let stat = statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| CutoverArtifactError::Io)?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        if stat.st_dev != root_device || stat.st_uid != geteuid().as_raw() {
            return Err(CutoverArtifactError::ForeignOwner);
        }
        match file_type {
            FileType::Directory => {
                let child = openat2(
                    directory,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                    Mode::empty(),
                    ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
                )
                .map_err(|_| CutoverArtifactError::Io)?;
                delete_tree_fd(&child)?;
                unlinkat(directory, &name, AtFlags::REMOVEDIR)
                    .map_err(|_| CutoverArtifactError::Io)?;
            }
            FileType::RegularFile => {
                if stat.st_nlink != 1 {
                    return Err(CutoverArtifactError::ForeignOwner);
                }
                unlinkat(directory, &name, AtFlags::empty())
                    .map_err(|_| CutoverArtifactError::Io)?;
            }
            _ => return Err(CutoverArtifactError::Io),
        }
    }
    Ok(())
}

fn digest_bytes(domain: &str, bytes: &[u8]) -> Result<CanonicalAuditDigest, CutoverArtifactError> {
    let digest = format!(
        "sha256:{:x}",
        Sha256::digest([domain.as_bytes(), bytes].concat())
    );
    CanonicalAuditDigest::parse(digest).map_err(|_| CutoverArtifactError::Digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::libc;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    fn root(label: &str) -> PathBuf {
        let root = PathBuf::from(".scratch")
            .join(format!("cutover-artifacts-{label}-{}", std::process::id()));
        fs::create_dir_all(&root).expect("root");
        root
    }

    fn write_owned(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .mode(0o600)
            .open(path)
            .expect("marker");
        file.write_all(bytes).expect("marker bytes");
        file.sync_all().expect("marker sync");
    }

    fn staged_marker(
        operation_id: &str,
        staged_id: &str,
        source_id: &str,
    ) -> (Vec<u8>, CanonicalAuditDigest) {
        let mut marker = StagedMarker {
            operation_id: operation_id.to_owned(),
            staged_id: staged_id.to_owned(),
            source_id: source_id.to_owned(),
            marker_digest: String::new(),
        };
        let unsigned = serde_json::to_vec(&marker).expect("marker JSON");
        let digest =
            digest_bytes("d2b:cutover:staged-marker:v1", &unsigned).expect("marker digest");
        marker.marker_digest = digest.as_str().to_owned();
        (serde_json::to_vec(&marker).expect("marker JSON"), digest)
    }

    #[test]
    fn quarantine_moves_only_marked_staging_and_preserves_source() {
        let root = root("quarantine");
        let operation = BundleOpId::new("op-quarantine");
        let staged = BundleOpId::new("staged-one");
        let source = BundleOpId::new("source-one");
        let op_root = root.join("cutover").join(operation.as_str());
        fs::create_dir_all(op_root.join("staged").join(staged.as_str())).expect("staged");
        fs::create_dir_all(op_root.join("sources").join(source.as_str())).expect("source");
        fs::write(
            op_root.join("staged").join(staged.as_str()).join("data"),
            b"staged",
        )
        .expect("staged data");
        let (marker_bytes, digest) =
            staged_marker(operation.as_str(), staged.as_str(), source.as_str());
        write_owned(
            &op_root
                .join("staged")
                .join(staged.as_str())
                .join(STAGED_MARKER),
            &marker_bytes,
        );
        quarantine_staged_destination(&root, &operation, &staged, &source, &digest)
            .expect("quarantine");
        assert!(!op_root.join("staged").join(staged.as_str()).exists());
        assert!(op_root.join("quarantine").join(staged.as_str()).exists());
        assert!(op_root.join("sources").join(source.as_str()).exists());
        assert_eq!(
            quarantine_staged_destination(&root, &operation, &staged, &source, &digest),
            Err(CutoverArtifactError::Missing)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quarantine_refuses_foreign_marker_and_replay() {
        let root = root("quarantine-refuse");
        let operation = BundleOpId::new("op-quarantine-refuse");
        let staged = BundleOpId::new("staged-one");
        let source = BundleOpId::new("source-one");
        let staged_path = root
            .join("cutover")
            .join(operation.as_str())
            .join("staged")
            .join(staged.as_str());
        fs::create_dir_all(&staged_path).expect("staged");
        fs::create_dir_all(
            root.join("cutover")
                .join(operation.as_str())
                .join("sources")
                .join(source.as_str()),
        )
        .expect("source");
        let (bytes, digest) = staged_marker(operation.as_str(), staged.as_str(), source.as_str());
        write_owned(&staged_path.join(STAGED_MARKER), &bytes);
        fs::remove_file(staged_path.join(STAGED_MARKER)).expect("remove marker");
        std::os::unix::fs::symlink("/etc/passwd", staged_path.join(STAGED_MARKER))
            .expect("foreign marker");
        assert_eq!(
            quarantine_staged_destination(&root, &operation, &staged, &source, &digest),
            Err(CutoverArtifactError::ForeignOwner)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finalization_removes_only_approved_marked_artifacts() {
        let root = root("finalization");
        let operation = BundleOpId::new("op-finalize");
        let artifact = ArtifactId::parse("legacy-one").expect("artifact");
        let disposition = CanonicalAuditDigest::parse("sha256:".to_owned() + &"a".repeat(64))
            .expect("disposition");
        let legacy = root
            .join("cutover")
            .join(operation.as_str())
            .join("legacy")
            .join(artifact.as_str());
        fs::create_dir_all(&legacy).expect("legacy");
        let marker = LegacyMarker {
            operation_id: operation.to_string(),
            artifact_id: artifact.as_str().to_owned(),
            disposition_digest: disposition.as_str().to_owned(),
            kind: "directory".to_owned(),
        };
        write_owned(
            &legacy.join(LEGACY_MARKER),
            &serde_json::to_vec(&marker).expect("marker"),
        );
        fs::write(legacy.join("data"), b"legacy").expect("data");
        let consent =
            CanonicalAuditDigest::parse("sha256:".to_owned() + &"b".repeat(64)).expect("consent");
        finalize_legacy_artifacts(&root, &operation, &[artifact], &disposition, &consent)
            .expect("finalize");
        assert!(!legacy.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn destroy_volume_requires_source_preservation_marker() {
        let root = root("destroy");
        let operation = BundleOpId::new("op-destroy");
        let storage = BundleOpId::new("path:durable-volume");
        let volume = root
            .join("cutover")
            .join(operation.as_str())
            .join("volumes")
            .join(storage.as_str());
        fs::create_dir_all(&volume).expect("volume");
        let marker = VolumeMarker {
            operation_id: operation.to_string(),
            storage_ref: storage.to_string(),
            marker_digest: String::new(),
            source_preserved: true,
        };
        let unsigned = serde_json::to_vec(&marker).expect("marker");
        let digest = digest_bytes("d2b:cutover:volume-marker:v1", &unsigned).expect("digest");
        let marker = VolumeMarker {
            marker_digest: digest.as_str().to_owned(),
            ..marker
        };
        write_owned(
            &volume.join(VOLUME_MARKER),
            &serde_json::to_vec(&marker).expect("marker"),
        );
        let consent =
            CanonicalAuditDigest::parse("sha256:".to_owned() + &"c".repeat(64)).expect("consent");
        destroy_durable_volume(&root, &operation, &storage, &digest, &consent).expect("destroy");
        assert!(!volume.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recursive_destroy_refuses_nested_symlink_and_hardlink_children() {
        let root = root("destroy-tree-refuse");
        let operation = BundleOpId::new("op-destroy-tree-refuse");
        let storage = BundleOpId::new("path:durable-volume");
        let volume = root
            .join("cutover")
            .join(operation.as_str())
            .join("volumes")
            .join(storage.as_str());
        fs::create_dir_all(&volume).expect("volume");
        let marker = VolumeMarker {
            operation_id: operation.to_string(),
            storage_ref: storage.to_string(),
            marker_digest: String::new(),
            source_preserved: true,
        };
        let unsigned = serde_json::to_vec(&marker).expect("marker");
        let digest = digest_bytes("d2b:cutover:volume-marker:v1", &unsigned).expect("digest");
        let marker = VolumeMarker {
            marker_digest: digest.as_str().to_owned(),
            ..marker
        };
        write_owned(
            &volume.join(VOLUME_MARKER),
            &serde_json::to_vec(&marker).expect("marker"),
        );
        fs::write(volume.join("data"), b"data").expect("data");
        fs::hard_link(volume.join("data"), volume.join("hardlink")).expect("hardlink");
        let consent =
            CanonicalAuditDigest::parse("sha256:".to_owned() + &"d".repeat(64)).expect("consent");
        assert_eq!(
            destroy_durable_volume(&root, &operation, &storage, &digest, &consent),
            Err(CutoverArtifactError::ForeignOwner)
        );
        assert!(volume.exists());
        fs::remove_file(volume.join("hardlink")).expect("hardlink cleanup");
        std::os::unix::fs::symlink("/etc/passwd", volume.join("nested-link")).expect("nested link");
        assert_eq!(
            destroy_durable_volume(&root, &operation, &storage, &digest, &consent),
            Err(CutoverArtifactError::Io)
        );
        assert!(volume.exists());
        fs::remove_file(volume.join("nested-link")).expect("nested link cleanup");
        nix::unistd::mkfifo(
            &volume.join("fifo"),
            nix::sys::stat::Mode::from_bits_truncate(0o600),
        )
        .expect("fifo");
        assert_eq!(
            destroy_durable_volume(&root, &operation, &storage, &digest, &consent),
            Err(CutoverArtifactError::Io)
        );
        let _ = fs::remove_dir_all(root);
    }
}
