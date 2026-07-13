use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use super::{
    DeliveryError, Result,
    command::RepositoryProbe,
    model::{validate_hash, validate_identifier, validate_sha256},
};

const STATE_DIRECTORY: &str = "d2b-delivery";
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateLayout {
    pub root: PathBuf,
    pub candidate: PathBuf,
}

impl StateLayout {
    pub fn create<P: RepositoryProbe>(
        probe: &P,
        root_repository: &Path,
        repository_roots: &[PathBuf],
        requested_root: Option<&Path>,
        wave: &str,
        tree_hash: &str,
    ) -> Result<Self> {
        validate_identifier(wave, "wave")?;
        validate_hash(tree_hash, "integrated tree")?;
        let common_dirs = repository_roots
            .iter()
            .map(|root| probe.git_common_dir(root))
            .collect::<Result<Vec<_>>>()?;
        let root = if let Some(requested) = requested_root {
            absolute_for_write(requested)?
        } else {
            probe.git_common_dir(root_repository)?.join(STATE_DIRECTORY)
        };
        ensure_external_path(&root, repository_roots, &common_dirs)?;
        let candidate = root.join(wave).join(tree_hash);
        ensure_external_path(&candidate, repository_roots, &common_dirs)?;
        fs::create_dir_all(&candidate)?;
        let root = fs::canonicalize(&root)?;
        let candidate = fs::canonicalize(&candidate)?;
        ensure_external_path(&candidate, repository_roots, &common_dirs)?;
        Ok(Self { root, candidate })
    }

    pub fn from_snapshot_path(snapshot_path: &Path, wave: &str, tree_hash: &str) -> Result<Self> {
        if snapshot_path.file_name().and_then(|name| name.to_str()) != Some("snapshot.json") {
            return Err(DeliveryError::new(
                "snapshot path must end in snapshot.json",
            ));
        }
        let candidate = snapshot_path
            .parent()
            .ok_or_else(|| DeliveryError::new("snapshot path has no candidate directory"))?
            .to_path_buf();
        if candidate.file_name().and_then(|name| name.to_str()) != Some(tree_hash) {
            return Err(DeliveryError::new(
                "snapshot directory is not addressed by its integrated tree hash",
            ));
        }
        let wave_dir = candidate
            .parent()
            .ok_or_else(|| DeliveryError::new("snapshot path has no wave directory"))?;
        if wave_dir.file_name().and_then(|name| name.to_str()) != Some(wave) {
            return Err(DeliveryError::new(
                "snapshot wave directory does not match snapshot wave",
            ));
        }
        let root = wave_dir
            .parent()
            .ok_or_else(|| DeliveryError::new("snapshot path has no state root"))?
            .to_path_buf();
        Ok(Self { root, candidate })
    }

    pub fn snapshot(&self) -> PathBuf {
        self.candidate.join("snapshot.json")
    }

    pub fn evidence_dir(&self) -> PathBuf {
        self.candidate.join("validation")
    }

    pub fn panel_dir(&self) -> PathBuf {
        self.candidate.join("panel")
    }

    pub fn seal(&self) -> PathBuf {
        self.candidate.join("seal.json")
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut file = fs::File::open(path)
        .map_err(|error| DeliveryError::new(format!("cannot read {}: {error}", path.display())))?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| DeliveryError::new(format!("cannot read {}: {error}", path.display())))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "JSON artifact exceeds {} bytes: {}",
            MAX_JSON_BYTES,
            path.display()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid JSON in {}: {error}", path.display())))
}

pub fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    render_digest(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|error| {
        DeliveryError::new(format!("cannot read payload {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            DeliveryError::new(format!("cannot read payload {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(render_digest(hasher.finalize()))
}

pub fn write_immutable_json<T: Serialize>(path: &Path, value: &T) -> Result<String> {
    let bytes = json_bytes(value)?;
    write_immutable(path, &bytes)?;
    let digest = sha256_bytes(&bytes);
    let sidecar = digest_path(path)?;
    write_immutable(&sidecar, format!("{digest}\n").as_bytes())?;
    Ok(digest)
}

pub fn verify_json_digest(path: &Path) -> Result<String> {
    let digest = sha256_file(path)?;
    let sidecar = digest_path(path)?;
    let recorded = fs::read_to_string(&sidecar).map_err(|error| {
        DeliveryError::new(format!(
            "cannot read digest sidecar {}: {error}",
            sidecar.display()
        ))
    })?;
    let recorded = recorded.trim();
    validate_sha256(recorded, "artifact digest")?;
    if recorded != digest {
        return Err(DeliveryError::new(format!(
            "digest mismatch for {}",
            path.display()
        )));
    }

    Ok(digest)
}

fn render_digest(digest: impl IntoIterator<Item = u8>) -> String {
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut rendered, "{byte:02x}").expect("writing to String cannot fail");
    }
    rendered
}

pub fn digest_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DeliveryError::new("artifact path has no UTF-8 filename"))?;
    let digest_name = match name.strip_suffix(".json") {
        Some(stem) => format!("{stem}.sha256"),
        None => format!("{name}.sha256"),
    };
    Ok(path.with_file_name(digest_name))
}

pub fn write_immutable(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| DeliveryError::new("artifact path has no parent"))?;
    fs::create_dir_all(parent)?;
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(path);
                return Err(DeliveryError::new(format!(
                    "cannot persist {}: {error}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path)?;
            if existing == bytes {
                Ok(())
            } else {
                Err(DeliveryError::new(format!(
                    "immutable artifact already exists with different content: {}",
                    path.display()
                )))
            }
        }
        Err(error) => Err(DeliveryError::new(format!(
            "cannot create immutable artifact {}: {error}",
            path.display()
        ))),
    }
}

pub fn ensure_external_path(
    path: &Path,
    repository_roots: &[PathBuf],
    allowed_git_common_dirs: &[PathBuf],
) -> Result<()> {
    let absolute = absolute_for_write(path)?;
    for common in allowed_git_common_dirs {
        let common = absolute_for_write(common)?;
        if absolute.starts_with(&common) {
            return Ok(());
        }
    }
    for root in repository_roots {
        let root = fs::canonicalize(root).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize repository root {}: {error}",
                root.display()
            ))
        })?;
        if absolute == root || absolute.starts_with(&root) {
            return Err(DeliveryError::new(format!(
                "delivery artifacts must not be stored in repository paths: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub fn absolute_for_write(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing = absolute.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| DeliveryError::new("path has no existing ancestor"))?;
        suffix.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| DeliveryError::new("path has no existing ancestor"))?;
    }
    let mut resolved = fs::canonicalize(existing)?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    Ok(normalize(&resolved))
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_repository_artifact_path() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let error = ensure_external_path(
            &root.join("delivery-state"),
            std::slice::from_ref(&root),
            &[],
        )
        .expect_err("repository path");
        assert!(error.to_string().contains("must not be stored"));
    }

    #[test]
    fn allows_git_common_directory() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let common = root.join(".git");
        if common.is_dir() {
            ensure_external_path(
                &common.join("d2b-delivery"),
                std::slice::from_ref(&root),
                &[common],
            )
            .expect("Git metadata is outside the reviewed content tree");
        }
    }

    #[test]
    fn immutable_write_rejects_changed_content() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let parent = root
            .parent()
            .expect("repository has parent")
            .join(format!(".d2b-xtask-storage-test-{}", std::process::id()));
        let path = parent.join("artifact");
        fs::create_dir_all(&parent).expect("create external test dir");
        write_immutable(&path, b"one").expect("initial write");
        let error = write_immutable(&path, b"two").expect_err("changed content");
        assert!(error.to_string().contains("different content"));
        fs::remove_dir_all(parent).expect("remove external test dir");
    }
}
