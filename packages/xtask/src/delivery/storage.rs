use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::fs::{MetadataExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use rustix::{
    fs::{
        FlockOperation, Mode, OFlags, RenameFlags, fchmod, fcntl_lock, mkdirat, open, openat,
        renameat_with, unlinkat,
    },
    io::Errno,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use super::{
    DeliveryError, Result,
    command::reject_symlink_components,
    model::{validate_identifier, validate_sha256},
};

const STATE_DIRECTORY: &str = "d2b/delivery";
pub const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMMUTABLE_BYTES: usize = MAX_PAYLOAD_BYTES;
const MAX_SIDECAR_BYTES: usize = 65;
static NEXT_PRIVATE_FILE: AtomicU64 = AtomicU64::new(1);
static NEXT_STAGING_DIRECTORY: AtomicU64 = AtomicU64::new(1);
static HELD_LOCKS: OnceLock<Mutex<BTreeSet<(u64, u64)>>> = OnceLock::new();

#[derive(Debug)]
pub struct StagedInput {
    path: PathBuf,
    digest: String,
}

impl StagedInput {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone)]
pub struct StateLayout {
    pub root: PathBuf,
    pub candidate: PathBuf,
    root_fd: Arc<OwnedFd>,
    candidate_fd: Arc<OwnedFd>,
}

impl fmt::Debug for StateLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateLayout")
            .field("root", &self.root)
            .field("candidate", &self.candidate)
            .finish_non_exhaustive()
    }
}

impl PartialEq for StateLayout {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.candidate == other.candidate
    }
}

impl Eq for StateLayout {}

impl StateLayout {
    pub fn create(
        repository_roots: &[PathBuf],
        requested_root: Option<&Path>,
        wave: &str,
        candidate_id: &str,
    ) -> Result<Self> {
        validate_identifier(wave, "wave")?;
        validate_sha256(candidate_id, "candidate ID")?;
        let root = prepare_state_root(repository_roots, requested_root)?;
        let wave_dir = root.join(wave);
        create_private_dir(&wave_dir)?;
        let candidate = wave_dir.join(candidate_id);
        create_private_dir(&candidate)?;
        Self::anchor(root, candidate)
    }

    pub fn from_snapshot_path(
        snapshot_path: &Path,
        wave: &str,
        candidate_id: &str,
    ) -> Result<Self> {
        if snapshot_path.file_name().and_then(OsStr::to_str) != Some("snapshot.json") {
            return Err(DeliveryError::new(
                "snapshot path must end in snapshot.json",
            ));
        }
        validate_identifier(wave, "wave")?;
        validate_sha256(candidate_id, "candidate ID")?;
        reject_symlink_components(snapshot_path)?;
        let candidate = snapshot_path
            .parent()
            .ok_or_else(|| DeliveryError::new("snapshot path has no candidate directory"))?
            .to_path_buf();
        if candidate.file_name().and_then(OsStr::to_str) != Some(candidate_id) {
            return Err(DeliveryError::new(
                "snapshot directory is not addressed by its candidate ID",
            ));
        }
        let wave_dir = candidate
            .parent()
            .ok_or_else(|| DeliveryError::new("snapshot path has no wave directory"))?;
        if wave_dir.file_name().and_then(OsStr::to_str) != Some(wave) {
            return Err(DeliveryError::new(
                "snapshot wave directory does not match snapshot wave",
            ));
        }
        let root = wave_dir
            .parent()
            .ok_or_else(|| DeliveryError::new("snapshot path has no state root"))?
            .to_path_buf();
        verify_private_directory(&root)?;
        verify_private_directory(wave_dir)?;
        verify_private_directory(&candidate)?;
        Self::anchor(root, candidate)
    }

    fn anchor(root: PathBuf, candidate: PathBuf) -> Result<Self> {
        let root_fd = Arc::new(open_directory_chain(&root, false)?);
        let candidate_fd = Arc::new(open_directory_chain(&candidate, false)?);
        secure_opened_directory(&root_fd, "delivery state root")?;
        secure_opened_directory(&candidate_fd, "delivery candidate directory")?;
        Ok(Self {
            root,
            candidate,
            root_fd,
            candidate_fd,
        })
    }

    fn relative_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.candidate.join(relative)
    }

    pub fn anchored_path(&self, relative: impl AsRef<Path>) -> Result<PathBuf> {
        self.verify_anchors()?;
        validate_anchored_relative(relative.as_ref())?;
        Ok(PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.candidate_fd.as_raw_fd()
        ))
        .join(relative))
    }

    pub fn write_candidate_json<T: Serialize>(
        &self,
        relative: impl AsRef<Path>,
        value: &T,
    ) -> Result<String> {
        self.verify_anchors()?;
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        let bytes = json_bytes(value)?;
        write_immutable_at(&self.candidate_fd, relative, &bytes)?;
        let digest = sha256_bytes(&bytes);
        let sidecar = digest_relative_path(relative)?;
        write_immutable_at(
            &self.candidate_fd,
            &sidecar,
            format!("{digest}\n").as_bytes(),
        )?;
        Ok(digest)
    }

    pub fn write_candidate_file(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> Result<String> {
        self.verify_anchors()?;
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        write_immutable_at(&self.candidate_fd, relative, bytes)?;
        let digest = sha256_bytes(bytes);
        let sidecar = digest_relative_path(relative)?;
        write_immutable_at(
            &self.candidate_fd,
            &sidecar,
            format!("{digest}\n").as_bytes(),
        )?;
        Ok(digest)
    }

    pub fn read_candidate_json<T: DeserializeOwned>(
        &self,
        relative: impl AsRef<Path>,
    ) -> Result<(T, String)> {
        self.verify_anchors()?;
        let relative = relative.as_ref();
        let bytes = read_limited_at(&self.candidate_fd, relative, MAX_JSON_BYTES, true)?;
        let digest = verify_digest_bytes_at(&self.candidate_fd, relative, &bytes)?;
        let value = serde_json::from_slice(&bytes).map_err(|error| {
            DeliveryError::new(format!(
                "invalid JSON in {}: {error}",
                self.relative_path(relative).display()
            ))
        })?;
        Ok((value, digest))
    }

    pub fn verify_candidate_digest(&self, relative: impl AsRef<Path>) -> Result<String> {
        self.verify_anchors()?;
        let relative = relative.as_ref();
        let bytes = read_limited_at(&self.candidate_fd, relative, MAX_PAYLOAD_BYTES, true)?;
        verify_digest_bytes_at(&self.candidate_fd, relative, &bytes)
    }

    pub fn retain_candidate_file(
        &self,
        source: &Path,
        relative: impl AsRef<Path>,
    ) -> Result<String> {
        self.verify_anchors()?;
        let relative = relative.as_ref();
        let bytes = read_limited(source, MAX_PAYLOAD_BYTES, true)?;
        write_immutable_at(&self.candidate_fd, relative, &bytes)?;
        let digest = sha256_bytes(&bytes);
        let sidecar = digest_relative_path(relative)?;
        write_immutable_at(
            &self.candidate_fd,
            &sidecar,
            format!("{digest}\n").as_bytes(),
        )?;
        Ok(digest)
    }

    pub fn stage_external_file(
        &self,
        source: &Path,
        label: &str,
        limit: usize,
    ) -> Result<StagedInput> {
        self.verify_anchors()?;
        validate_identifier(label, "staged input label")?;
        let bytes = read_limited(source, limit, false)?;
        let relative = PathBuf::from("input-staging").join(format!(
            "{}-{}-{}",
            std::process::id(),
            NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed),
            label
        ));
        write_immutable_at(&self.candidate_fd, &relative, &bytes)?;
        Ok(StagedInput {
            path: self.anchored_path(&relative)?,
            digest: sha256_bytes(&bytes),
        })
    }

    pub fn list_candidate_directory(&self, relative: impl AsRef<Path>) -> Result<Vec<OsString>> {
        self.verify_anchors()?;
        let relative = relative.as_ref();
        validate_anchored_relative(relative)?;
        let fd = open_relative_directory_chain(&self.candidate_fd, relative, false)?;
        let proc_path = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            fd.as_raw_fd()
        ));
        let mut names = Vec::new();
        for entry in fs::read_dir(proc_path)? {
            names.push(entry?.file_name());
        }
        names.sort();
        Ok(names)
    }

    fn verify_anchors(&self) -> Result<()> {
        for (fd, label) in [
            (&self.root_fd, "delivery state root"),
            (&self.candidate_fd, "delivery candidate directory"),
        ] {
            let metadata = File::from(fd.try_clone()?).metadata()?;
            verify_owner(&metadata, label)?;
            if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
                return Err(DeliveryError::new(format!(
                    "{label} anchor is no longer a private directory"
                )));
            }
        }
        Ok(())
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

    pub fn panel_trust_root(&self) -> PathBuf {
        self.panel_dir().join("trust-root.pem")
    }

    pub fn ci_attestation_dir(&self) -> PathBuf {
        self.candidate.join("ci-attestations")
    }

    pub fn ci_attestation_artifact(&self, validation_id: &str) -> PathBuf {
        self.ci_attestation_dir()
            .join(format!("{validation_id}.artifact.json"))
    }

    pub fn ci_attestation_bundle(&self, validation_id: &str) -> PathBuf {
        self.ci_attestation_dir()
            .join(format!("{validation_id}.bundle.jsonl"))
    }

    pub fn validation_execution_dir(&self, validation_id: &str, run_id: &str) -> PathBuf {
        self.candidate
            .join("execution")
            .join(validation_id)
            .join(run_id)
    }

    pub fn panel_request(&self) -> PathBuf {
        self.candidate.join("panel-request.json")
    }

    pub fn seal(&self) -> PathBuf {
        self.candidate.join("seal.json")
    }

    pub fn history_proof(&self) -> PathBuf {
        self.candidate.join("history-proof.json")
    }
}

#[derive(Debug)]
pub struct CandidateLock {
    file: File,
    identity: (u64, u64),
}

#[derive(Debug)]
enum CandidateLockFailure {
    Contended,
    Kernel(Errno),
}

impl CandidateLockFailure {
    fn into_delivery_error(self) -> DeliveryError {
        match self {
            Self::Contended => DeliveryError::new("candidate lock contention"),
            Self::Kernel(error) => {
                DeliveryError::new(format!("cannot acquire candidate OFD lock: {error}"))
            }
        }
    }
}

impl Drop for CandidateLock {
    fn drop(&mut self) {
        let _ = fcntl_lock(&self.file, FlockOperation::NonBlockingUnlock);
        if let Ok(mut held) = HELD_LOCKS.get_or_init(Default::default).lock() {
            held.remove(&self.identity);
        }
    }
}

pub fn acquire_candidate_lock(
    repository_roots: &[PathBuf],
    requested_root: Option<&Path>,
    wave: &str,
    lock_key: &str,
) -> Result<(PathBuf, CandidateLock)> {
    validate_identifier(wave, "wave")?;
    validate_sha256(lock_key, "candidate lock key")?;
    let root = prepare_state_root(repository_roots, requested_root)?;
    let lock_dir = root.join("locks");
    create_private_dir(&lock_dir)?;
    let lock_path = lock_dir.join(format!("{wave}-{lock_key}.lock"));
    let parent = open_parent(&lock_path, true)?;
    secure_opened_directory(&parent, "candidate lock directory")?;
    let name = file_name(&lock_path)?;
    let fd = match openat(
        parent.as_fd(),
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    ) {
        Ok(fd) => fd,
        Err(error) => {
            return Err(DeliveryError::new(format!(
                "cannot open candidate lock: {error}"
            )));
        }
    };
    fchmod(&fd, Mode::from_raw_mode(0o600))
        .map_err(|error| DeliveryError::new(format!("cannot secure candidate lock: {error}")))?;
    let file = File::from(fd);
    let metadata = file.metadata()?;
    verify_private_file_metadata(&metadata, "candidate lock")?;
    let identity = (metadata.dev(), metadata.ino());
    {
        let mut held = HELD_LOCKS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| DeliveryError::new("candidate lock registry is poisoned"))?;
        if !held.insert(identity) {
            return Err(CandidateLockFailure::Contended.into_delivery_error());
        }
    }
    fcntl_lock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        if let Ok(mut held) = HELD_LOCKS.get_or_init(Default::default).lock() {
            held.remove(&identity);
        }
        if matches!(error, Errno::AGAIN | Errno::ACCESS) {
            CandidateLockFailure::Contended
        } else {
            CandidateLockFailure::Kernel(error)
        }
        .into_delivery_error()
    })?;
    Ok((root, CandidateLock { file, identity }))
}

pub fn prepare_state_root(
    repository_roots: &[PathBuf],
    requested_root: Option<&Path>,
) -> Result<PathBuf> {
    let root = match requested_root {
        Some(path) => absolute_path(path)?,
        None => default_state_root()?,
    };
    ensure_external_path(&root, repository_roots)?;
    create_private_dir(&root)?;
    let root = fs::canonicalize(&root).map_err(|error| {
        DeliveryError::new(format!("cannot canonicalize delivery state root: {error}"))
    })?;
    ensure_external_path(&root, repository_roots)?;
    Ok(root)
}

fn default_state_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(DeliveryError::new("XDG_STATE_HOME must be absolute"));
        }
        return Ok(path.join(STATE_DIRECTORY));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| DeliveryError::new("HOME is required when XDG_STATE_HOME is unset"))?;
    if !home.is_absolute() {
        return Err(DeliveryError::new("HOME must be absolute"));
    }
    Ok(home.join(".local/state").join(STATE_DIRECTORY))
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = read_limited(path, MAX_JSON_BYTES, false)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid JSON in {}: {error}", path.display())))
}

pub fn read_json_with_digest<T: DeserializeOwned>(path: &Path) -> Result<(T, String)> {
    let bytes = read_limited(path, MAX_JSON_BYTES, false)?;
    let digest = sha256_bytes(&bytes);
    let value = serde_json::from_slice(&bytes).map_err(|error| {
        DeliveryError::new(format!("invalid JSON in {}: {error}", path.display()))
    })?;
    Ok((value, digest))
}

pub fn read_verified_json<T: DeserializeOwned>(path: &Path) -> Result<(T, String)> {
    let bytes = read_limited(path, MAX_JSON_BYTES, true)?;
    let digest = verify_digest_bytes(path, &bytes)?;
    let value = serde_json::from_slice(&bytes).map_err(|error| {
        DeliveryError::new(format!("invalid JSON in {}: {error}", path.display()))
    })?;
    Ok((value, digest))
}

pub fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    struct BoundedJson {
        bytes: Vec<u8>,
    }

    impl Write for BoundedJson {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let next = self
                .bytes
                .len()
                .checked_add(input.len())
                .ok_or_else(|| std::io::Error::other("JSON length overflow"))?;
            if next > MAX_JSON_BYTES.saturating_sub(1) {
                return Err(std::io::Error::other("JSON artifact exceeds hard bound"));
            }
            self.bytes.extend_from_slice(input);
            Ok(input.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut writer = BoundedJson {
        bytes: Vec::with_capacity(16 * 1024),
    };
    if let Err(error) = serde_json::to_writer_pretty(&mut writer, value) {
        return Err(DeliveryError::new(format!(
            "cannot serialize bounded JSON artifact: {error}"
        )));
    }
    writer.bytes.push(b'\n');
    Ok(writer.bytes)
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    render_digest(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    sha256_file_bounded(path, MAX_PAYLOAD_BYTES)
}

pub fn sha256_file_bounded(path: &Path, limit: usize) -> Result<String> {
    let mut file = open_regular_file(path, false)?;
    let size = file.metadata()?.len();
    if size > limit as u64 {
        return Err(DeliveryError::new(format!(
            "payload exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    let mut hasher = Sha256::new();
    let mut total = 0_usize;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            DeliveryError::new(format!("cannot read payload {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .ok_or_else(|| DeliveryError::new("payload length overflow"))?;
        if total > limit {
            return Err(DeliveryError::new(format!(
                "payload exceeds {limit} bytes: {}",
                path.display()
            )));
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

pub fn retain_immutable_file(source: &Path, destination: &Path) -> Result<String> {
    let bytes = read_limited(source, MAX_PAYLOAD_BYTES, false)?;
    write_immutable(destination, &bytes)?;
    let digest = sha256_bytes(&bytes);
    let sidecar = digest_path(destination)?;
    write_immutable(&sidecar, format!("{digest}\n").as_bytes())?;
    Ok(digest)
}

pub fn verify_json_digest(path: &Path) -> Result<String> {
    let bytes = read_limited(path, MAX_JSON_BYTES, true)?;
    verify_digest_bytes(path, &bytes)
}

pub fn verify_immutable_digest(path: &Path) -> Result<String> {
    let bytes = read_limited(path, MAX_PAYLOAD_BYTES, true)?;
    verify_digest_bytes(path, &bytes)
}

fn verify_digest_bytes(path: &Path, bytes: &[u8]) -> Result<String> {
    let digest = sha256_bytes(bytes);
    let sidecar = digest_path(path)?;
    let recorded = read_limited(&sidecar, MAX_SIDECAR_BYTES, true)?;
    let recorded = std::str::from_utf8(&recorded)
        .map_err(|_| DeliveryError::new("artifact digest sidecar is not UTF-8"))?
        .trim();
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
        .and_then(OsStr::to_str)
        .ok_or_else(|| DeliveryError::new("artifact path has no UTF-8 filename"))?;
    let digest_name = match name.strip_suffix(".json") {
        Some(stem) => format!("{stem}.sha256"),
        None => format!("{name}.sha256"),
    };
    Ok(path.with_file_name(digest_name))
}

fn digest_relative_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| DeliveryError::new("artifact path has no UTF-8 filename"))?;
    let digest_name = match name.strip_suffix(".json") {
        Some(stem) => format!("{stem}.sha256"),
        None => format!("{name}.sha256"),
    };
    Ok(path.with_file_name(digest_name))
}

pub fn write_immutable(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = open_parent(path, true)?;
    let destination_name = file_name(path)?;
    write_immutable_in(&parent, destination_name, bytes, path)
}

fn write_immutable_at(anchor: &OwnedFd, relative: &Path, bytes: &[u8]) -> Result<()> {
    validate_anchored_relative(relative)?;
    let (parent, destination_name) = open_relative_parent(anchor, relative, true)?;
    write_immutable_in(
        &parent,
        &destination_name,
        bytes,
        &PathBuf::from("<anchored-state>").join(relative),
    )
}

fn write_immutable_in(
    parent: &OwnedFd,
    destination_name: &OsStr,
    bytes: &[u8],
    display_path: &Path,
) -> Result<()> {
    if bytes.len() > MAX_IMMUTABLE_BYTES {
        return Err(DeliveryError::new(format!(
            "immutable artifact exceeds {MAX_IMMUTABLE_BYTES} bytes"
        )));
    }
    secure_opened_directory(parent, "immutable artifact directory")?;
    match read_existing_at(parent, destination_name, bytes.len())? {
        Some(existing) if existing == bytes => {
            verify_private_file_at(parent, destination_name)?;
            return Ok(());
        }
        Some(_) => {
            return Err(DeliveryError::new(format!(
                "immutable artifact already exists with different content: {}",
                display_path.display()
            )));
        }
        None => {}
    }

    let temporary_name = OsString::from(format!(
        ".{}.{}.{}.part",
        destination_name.to_string_lossy(),
        std::process::id(),
        NEXT_PRIVATE_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let fd = openat(
        parent.as_fd(),
        &temporary_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| {
        DeliveryError::new(format!(
            "cannot create private immutable artifact {}: {error}",
            display_path.display()
        ))
    })?;
    let mut file = File::from(fd);
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = unlinkat(
            parent.as_fd(),
            &temporary_name,
            rustix::fs::AtFlags::empty(),
        );
        return Err(DeliveryError::new(format!(
            "cannot persist {}: {error}",
            display_path.display()
        )));
    }
    match renameat_with(
        parent.as_fd(),
        &temporary_name,
        parent.as_fd(),
        destination_name,
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => {}
        Err(error) if error == Errno::EXIST => {
            let _ = unlinkat(
                parent.as_fd(),
                &temporary_name,
                rustix::fs::AtFlags::empty(),
            );
            match read_existing_at(parent, destination_name, bytes.len())? {
                Some(existing) if existing == bytes => {
                    verify_private_file_at(parent, destination_name)?;
                    return Ok(());
                }
                _ => {
                    return Err(DeliveryError::new(format!(
                        "immutable artifact appeared with different content: {}",
                        display_path.display()
                    )));
                }
            }
        }
        Err(error) => {
            let _ = unlinkat(
                parent.as_fd(),
                &temporary_name,
                rustix::fs::AtFlags::empty(),
            );
            return Err(DeliveryError::new(format!(
                "cannot atomically publish {}: {error}",
                display_path.display()
            )));
        }
    }
    File::from(parent.try_clone()?)
        .sync_all()
        .map_err(|error| {
            DeliveryError::new(format!(
                "cannot fsync artifact directory {}: {error}",
                display_path.display()
            ))
        })?;
    verify_private_file_at(parent, destination_name)
}

fn read_existing_at(
    parent: &OwnedFd,
    name: &OsStr,
    expected_len: usize,
) -> Result<Option<Vec<u8>>> {
    let fd = match openat(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(error) if error == Errno::NOENT => return Ok(None),
        Err(error) => {
            return Err(DeliveryError::new(format!(
                "cannot inspect existing immutable artifact: {error}"
            )));
        }
    };
    let mut file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(DeliveryError::new(
            "existing immutable artifact is not a regular file",
        ));
    }
    verify_private_file_metadata(&metadata, "existing immutable artifact")?;
    if metadata.len() > MAX_IMMUTABLE_BYTES as u64 || metadata.len() != expected_len as u64 {
        return Ok(Some(Vec::new()));
    }
    let mut bytes = vec![0_u8; expected_len];
    file.read_exact(&mut bytes)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(bytes))
}

fn verify_private_file_at(parent: &OwnedFd, name: &OsStr) -> Result<()> {
    let fd = openat(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| DeliveryError::new(format!("cannot open private artifact: {error}")))?;
    let file = File::from(fd);
    verify_private_file_metadata(&file.metadata()?, "immutable artifact")
}

pub fn ensure_external_path(path: &Path, repository_roots: &[PathBuf]) -> Result<()> {
    reject_symlink_components(path)?;
    let absolute = absolute_path(path)?;
    for root in repository_roots {
        reject_symlink_components(root)?;
        let root = fs::canonicalize(root).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize repository root {}: {error}",
                root.display()
            ))
        })?;
        if absolute == root || absolute.starts_with(&root) {
            return Err(DeliveryError::new(format!(
                "delivery artifacts must not be stored in repository or Git metadata paths: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(DeliveryError::new(format!(
                    "path contains parent traversal: {}",
                    path.display()
                )));
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    if !normalized.is_absolute() {
        return Err(DeliveryError::new("delivery path is not absolute"));
    }
    Ok(normalized)
}

pub fn secure_repository_subdir(root: &Path, relative: &Path) -> Result<PathBuf> {
    super::model::validate_repo_relative_path(relative)?;
    reject_symlink_components(root)?;
    let root = fs::canonicalize(root)?;
    let candidate = root.join(relative);
    reject_symlink_components(&candidate)?;
    let candidate = fs::canonicalize(&candidate).map_err(|error| {
        DeliveryError::new(format!(
            "cannot resolve logical validation cwd {}: {error}",
            relative.display()
        ))
    })?;
    if !candidate.starts_with(&root) || !candidate.is_dir() {
        return Err(DeliveryError::new(
            "logical validation cwd escapes its repository or is not a directory",
        ));
    }
    Ok(candidate)
}

pub fn reject_delivery_payload(path: &Path, state_root: &Path) -> Result<()> {
    reject_symlink_components(path)?;
    let path = fs::canonicalize(path).map_err(|error| {
        DeliveryError::new(format!(
            "cannot canonicalize evidence payload {}: {error}",
            path.display()
        ))
    })?;
    let state_root = fs::canonicalize(state_root)?;
    if path.starts_with(&state_root) {
        return Err(DeliveryError::new(
            "delivery-state artifacts cannot be validation evidence payloads",
        ));
    }
    if matches!(
        path.file_name().and_then(OsStr::to_str),
        Some("snapshot.json" | "seal.json" | "history-proof.json" | "panel-request.json")
    ) {
        return Err(DeliveryError::new(
            "delivery artifacts cannot be validation evidence payloads",
        ));
    }
    Ok(())
}

pub fn reject_delivery_payload_content(path: &Path) -> Result<()> {
    let bytes = read_limited(path, MAX_PAYLOAD_BYTES, true)?;
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && value
            .get("artifact_kind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind.starts_with("d2b-delivery/"))
    {
        return Err(DeliveryError::new(
            "delivery artifacts cannot be validation evidence payloads",
        ));
    }
    Ok(())
}

pub fn validate_payload_locator(locator: &str) -> Result<()> {
    if locator.is_empty()
        || locator.len() > 512
        || locator.contains("..")
        || locator.contains('\n')
        || locator.contains('\r')
        || locator.starts_with('/')
        || !locator.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'/' | b'.' | b'_' | b'-')
        })
        || !(locator.starts_with("github-artifact://")
            || locator.starts_with("discarded://")
            || locator.starts_with("private://")
            || locator.starts_with("oci://"))
    {
        return Err(DeliveryError::new(
            "payload locator must be bounded and use an approved privacy-safe scheme",
        ));
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    let absolute = absolute_path(path)?;
    let parent = open_parent(&absolute, true)?;
    let name = file_name(&absolute)?;
    let fd = match openat(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(error) if error == Errno::NOENT => {
            mkdirat(parent.as_fd(), name, Mode::from_raw_mode(0o700)).map_err(|error| {
                DeliveryError::new(format!(
                    "cannot create private directory {}: {error}",
                    absolute.display()
                ))
            })?;
            openat(
                parent.as_fd(),
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                DeliveryError::new(format!(
                    "cannot anchor private directory {}: {error}",
                    absolute.display()
                ))
            })?
        }
        Err(error) => {
            return Err(DeliveryError::new(format!(
                "private directory path is unsafe {}: {error}",
                absolute.display()
            )));
        }
    };
    let file = File::from(fd);
    let metadata = file.metadata()?;
    verify_owner(&metadata, "delivery state directory")?;
    file.set_permissions(fs::Permissions::from_mode(0o700))?;
    file.sync_all()?;
    Ok(())
}

pub fn create_private_directory(path: &Path) -> Result<()> {
    create_private_dir(path)
}

fn open_parent(path: &Path, create: bool) -> Result<OwnedFd> {
    let absolute = absolute_path(path)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| DeliveryError::new("path has no parent"))?;
    open_directory_chain(parent, create)
}

fn validate_anchored_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(DeliveryError::new(
            "anchored state path must be non-empty and relative",
        ));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(DeliveryError::new("anchored state path contains traversal"));
        }
    }
    Ok(())
}

fn open_relative_parent(
    anchor: &OwnedFd,
    relative: &Path,
    create: bool,
) -> Result<(OwnedFd, OsString)> {
    validate_anchored_relative(relative)?;
    let name = relative
        .file_name()
        .ok_or_else(|| DeliveryError::new("anchored state path has no filename"))?
        .to_os_string();
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let fd = open_relative_directory_chain(anchor, parent, create)?;
    Ok((fd, name))
}

fn open_relative_directory_chain(
    anchor: &OwnedFd,
    relative: &Path,
    create: bool,
) -> Result<OwnedFd> {
    let mut current = anchor.try_clone()?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(DeliveryError::new(
                "anchored directory path contains traversal",
            ));
        };
        current = match openat(
            current.as_fd(),
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(next) => next,
            Err(error) if create && error == Errno::NOENT => {
                mkdirat(current.as_fd(), name, Mode::from_raw_mode(0o700)).map_err(|error| {
                    DeliveryError::new(format!("cannot create anchored state directory: {error}"))
                })?;
                openat(
                    current.as_fd(),
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| {
                    DeliveryError::new(format!("cannot open new anchored state directory: {error}"))
                })?
            }
            Err(error) => {
                return Err(DeliveryError::new(format!(
                    "anchored state path contains a missing, symlink, or non-directory component: {error}"
                )));
            }
        };
        secure_opened_directory(&current, "delivery state directory")?;
    }
    Ok(current)
}

fn open_directory_chain(path: &Path, create: bool) -> Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(DeliveryError::new("anchored path must be absolute"));
    }
    let components = path.components().collect::<Vec<_>>();
    let own_pid = std::process::id().to_string();
    let proc_fd_prefix = match components.as_slice() {
        [
            Component::RootDir,
            Component::Normal(proc),
            Component::Normal(pid),
            Component::Normal(fd),
            Component::Normal(number),
            rest @ ..,
        ] if *proc == OsStr::new("proc")
            && *pid == OsStr::new(&own_pid)
            && *fd == OsStr::new("fd") =>
        {
            Some((*number, rest))
        }
        _ => None,
    };
    let (mut current, remaining): (OwnedFd, &[Component<'_>]) = if let Some((number, rest)) =
        proc_fd_prefix
    {
        let descriptor = PathBuf::from("/proc/self/fd").join(number);
        let fd = open(
            &descriptor,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            DeliveryError::new(format!(
                "cannot duplicate anchored state descriptor: {error}"
            ))
        })?;
        (fd, rest)
    } else {
        let fd = open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| DeliveryError::new(format!("cannot anchor filesystem root: {error}")))?;
        (fd, &components)
    };
    for component in remaining {
        let Component::Normal(name) = component else {
            continue;
        };
        current = match openat(
            current.as_fd(),
            *name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(next) => next,
            Err(error) if create && error == Errno::NOENT => {
                mkdirat(current.as_fd(), *name, Mode::from_raw_mode(0o700)).map_err(|error| {
                    DeliveryError::new(format!(
                        "cannot create anchored directory component: {error}"
                    ))
                })?;
                openat(
                    current.as_fd(),
                    *name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| {
                    DeliveryError::new(format!(
                        "cannot open newly-created directory component: {error}"
                    ))
                })?
            }
            Err(error) => {
                return Err(DeliveryError::new(format!(
                    "path contains a missing, symlink, or non-directory component: {error}"
                )));
            }
        };
    }
    Ok(current)
}

fn open_regular_file(path: &Path, require_private: bool) -> Result<File> {
    let parent = open_parent(path, false)?;
    let fd = openat(
        parent.as_fd(),
        file_name(path)?,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        DeliveryError::new(format!(
            "cannot open regular file {}: {error}",
            path.display()
        ))
    })?;
    let file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(DeliveryError::new(format!(
            "path is not a regular file: {}",
            path.display()
        )));
    }
    if require_private {
        verify_private_file_metadata(&metadata, "private artifact")?;
    }
    Ok(file)
}

fn read_limited(path: &Path, limit: usize, require_private: bool) -> Result<Vec<u8>> {
    let mut file = open_regular_file(path, require_private)?;
    let size = file.metadata()?.len();
    if size > limit as u64 {
        return Err(DeliveryError::new(format!(
            "artifact exceeds {limit} bytes: {}",
            path.display()
        )));
    }

    let size = usize::try_from(size)
        .map_err(|_| DeliveryError::new("artifact size does not fit in memory"))?;
    let mut bytes = vec![0_u8; size];
    file.read_exact(&mut bytes)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return Err(DeliveryError::new(format!(
            "artifact changed while reading: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_limited_at(
    anchor: &OwnedFd,
    relative: &Path,
    limit: usize,
    require_private: bool,
) -> Result<Vec<u8>> {
    let (parent, name) = open_relative_parent(anchor, relative, false)?;
    let fd = openat(
        parent.as_fd(),
        &name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        DeliveryError::new(format!(
            "cannot open anchored artifact {}: {error}",
            relative.display()
        ))
    })?;
    let mut file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(DeliveryError::new(
            "anchored artifact is not a regular file",
        ));
    }
    if require_private {
        verify_private_file_metadata(&metadata, "private anchored artifact")?;
    }
    if metadata.len() > limit as u64 {
        return Err(DeliveryError::new(format!(
            "anchored artifact exceeds {limit} bytes"
        )));
    }
    let size = usize::try_from(metadata.len())
        .map_err(|_| DeliveryError::new("anchored artifact size does not fit in memory"))?;
    let mut bytes = vec![0_u8; size];
    file.read_exact(&mut bytes)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return Err(DeliveryError::new(
            "anchored artifact changed while reading",
        ));
    }
    Ok(bytes)
}

fn verify_digest_bytes_at(anchor: &OwnedFd, path: &Path, bytes: &[u8]) -> Result<String> {
    let digest = sha256_bytes(bytes);
    let sidecar = digest_relative_path(path)?;
    let recorded = read_limited_at(anchor, &sidecar, MAX_SIDECAR_BYTES, true)?;
    let recorded = std::str::from_utf8(&recorded)
        .map_err(|_| DeliveryError::new("artifact digest sidecar is not UTF-8"))?
        .trim();
    validate_sha256(recorded, "artifact digest")?;
    if recorded != digest {
        return Err(DeliveryError::new(format!(
            "digest mismatch for anchored artifact {}",
            path.display()
        )));
    }
    Ok(digest)
}

pub fn verify_private_directory(path: &Path) -> Result<()> {
    let fd = open_directory_chain(path, false)?;
    let file = File::from(fd);
    let metadata = file.metadata()?;
    verify_owner(&metadata, "delivery state directory")?;
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(DeliveryError::new(
            "delivery state directory must have mode 0700",
        ));
    }
    Ok(())
}

fn secure_opened_directory(fd: &OwnedFd, label: &str) -> Result<()> {
    let metadata = File::from(fd.try_clone()?).metadata()?;
    verify_owner(&metadata, label)?;
    fchmod(fd, Mode::from_raw_mode(0o700))
        .map_err(|error| DeliveryError::new(format!("cannot secure {label}: {error}")))
}

fn verify_private_file_metadata(metadata: &fs::Metadata, label: &str) -> Result<()> {
    verify_owner(metadata, label)?;
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(DeliveryError::new(format!("{label} must have mode 0600")));
    }
    if metadata.nlink() != 1 {
        return Err(DeliveryError::new(format!(
            "{label} must not have hardlink aliases"
        )));
    }
    Ok(())
}

fn verify_owner(metadata: &fs::Metadata, label: &str) -> Result<()> {
    let current_uid = rustix::process::getuid().as_raw();
    if metadata.uid() != current_uid {
        return Err(DeliveryError::new(format!(
            "{label} is not owned by the current user"
        )));
    }
    Ok(())
}

fn file_name(path: &Path) -> Result<&OsStr> {
    path.file_name()
        .ok_or_else(|| DeliveryError::new("path has no final component"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn scratch(label: &str) -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repository");
        let path = repository.parent().expect("parent").join(format!(
            ".d2b-storage-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("scratch");
        path
    }

    #[test]
    fn private_immutable_write_is_atomic_and_bounded() {
        let root = scratch("immutable");
        let path = root.join("state/artifact");
        write_immutable(&path, b"one").expect("write");
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
        assert!(write_immutable(&path, b"two").is_err());
        assert!(write_immutable(&root.join("large"), &vec![0; MAX_IMMUTABLE_BYTES + 1]).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn oversized_sidecar_is_rejected_before_allocation() {
        let root = scratch("sidecar");
        let path = root.join("state/value.json");
        write_immutable_json(&path, &serde_json::json!({"ok": true})).expect("write");
        let sidecar = digest_path(&path).expect("sidecar");
        fs::remove_file(&sidecar).expect("remove");
        fs::write(&sidecar, vec![b'a'; MAX_SIDECAR_BYTES + 1]).expect("oversized");
        fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o600)).expect("mode");
        let error = verify_json_digest(&path).expect_err("oversized");
        assert!(error.to_string().contains("exceeds"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn candidate_lock_contends_between_separate_open_descriptions_and_releases() {
        let root = scratch("ofd-lock");
        let key = "a".repeat(64);
        let requested = root.join("state");
        let (_, first) =
            acquire_candidate_lock(&[], Some(&requested), "w1", &key).expect("first lock");
        let error = acquire_candidate_lock(&[], Some(&requested), "w1", &key)
            .expect_err("separate open description must contend");
        assert!(error.to_string().contains("contention"));
        drop(first);
        acquire_candidate_lock(&[], Some(&requested), "w1", &key)
            .expect("lock released with owning description");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn staged_input_survives_source_replacement() {
        let root = scratch("staged-source");
        let state = root.join("state");
        let source = root.join("source");
        fs::write(&source, b"trusted").expect("source");
        let layout = StateLayout::create(&[], Some(&state), "w1", &"b".repeat(64)).expect("layout");
        let staged = layout
            .stage_external_file(&source, "receipt", MAX_JSON_BYTES)
            .expect("stage");
        fs::remove_file(&source).expect("remove source");
        fs::write(root.join("replacement"), b"attacker").expect("replacement");
        symlink(root.join("replacement"), &source).expect("replace with symlink");
        assert_eq!(
            sha256_file(staged.path()).expect("staged digest"),
            staged.digest()
        );
        assert_eq!(
            read_limited(staged.path(), MAX_JSON_BYTES, true).expect("staged bytes"),
            b"trusted"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn candidate_anchor_survives_directory_rename_and_symlink_swap() {
        let root = scratch("anchor-swap");
        let state = root.join("state");
        let candidate_id = "c".repeat(64);
        let layout = StateLayout::create(&[], Some(&state), "w1", &candidate_id).expect("layout");
        let original = layout.candidate.clone();
        let moved = original.with_file_name("moved-candidate");
        fs::rename(&original, &moved).expect("rename candidate");
        let attacker = root.join("attacker");
        fs::create_dir(&attacker).expect("attacker");
        symlink(&attacker, &original).expect("replacement symlink");
        layout
            .write_candidate_file("anchored.bin", b"trusted")
            .expect("anchored write");
        assert_eq!(
            fs::read(moved.join("anchored.bin")).expect("moved data"),
            b"trusted"
        );
        assert!(!attacker.join("anchored.bin").exists());
        fs::remove_file(original).expect("remove symlink");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
