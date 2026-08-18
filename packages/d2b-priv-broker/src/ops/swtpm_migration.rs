//! Crash-safe, fd-relative legacy swtpm migration.
//!
//! The migration owner never mutates a path after a path check. Every source,
//! destination, marker, journal, and lock operation is relative to held
//! directory descriptors opened with no-follow and beneath constraints.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::fd::{AsFd, AsRawFd, OwnedFd},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use nix::fcntl::{FlockArg, flock};
use nix::libc;
use rustix::fs::{
    AtFlags, Dir, Mode, OFlags, RenameFlags, fsync, mkdirat, openat, renameat_with, unlinkat,
};

const MAX_TREE_ENTRIES: usize = 16_384;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const LOCK_NAME: &str = ".d2b-legacy-swtpm.lock";
const MARKER_MODE: u32 = 0o600;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

/// Ordered journal phases. Each phase is durable before the next mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LegacyMigrationPhase {
    Prepared,
    PayloadStaged,
    MarkerPublished,
    Committed,
    SourceRetired,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMigrationObservation {
    Unmigrated,
    Committed,
    MissingMarker,
    ReplacementDetected,
    Ambiguous,
    ForeignOwner,
}

/// Typed result returned by the broker migration operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMigrationOutcome {
    Migrated,
    AlreadyMigrated,
    NotApplicable,
    Pending,
    Failed,
    Ambiguous,
}

/// One broker mutation selected by journal replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMigrationAction {
    PrepareJournal,
    StagePayload,
    PublishMarker,
    Commit,
    RetireSource,
    AlreadyMigrated,
    Quarantine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyFileIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}

/// Durable identity row for one migration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMigrationJournal {
    source_digest: [u8; 32],
    destination_digest: [u8; 32],
    marker_digest: [u8; 32],
    marker_first_provisioned_ms: u64,
    phase: Option<LegacyMigrationPhase>,
    source_identity: Option<LegacyFileIdentity>,
    destination_identity: Option<LegacyFileIdentity>,
}

impl LegacyMigrationJournal {
    pub fn new(
        source_digest: [u8; 32],
        destination_digest: [u8; 32],
        marker_digest: [u8; 32],
    ) -> Result<Self, LegacyMigrationError> {
        if source_digest == [0; 32] || destination_digest == [0; 32] || marker_digest == [0; 32] {
            return Err(LegacyMigrationError::IdentityInvalid);
        }
        Ok(Self {
            source_digest,
            destination_digest,
            marker_digest,
            marker_first_provisioned_ms: now_ms(),
            phase: None,
            source_identity: None,
            destination_identity: None,
        })
    }

    pub const fn phase(&self) -> Option<LegacyMigrationPhase> {
        self.phase
    }

    pub(crate) const fn destination_digest(&self) -> [u8; 32] {
        self.destination_digest
    }

    pub(crate) const fn marker_digest(&self) -> [u8; 32] {
        self.marker_digest
    }

    fn marker_first_provisioned_ms(&self) -> u64 {
        self.marker_first_provisioned_ms
    }

    fn source_identity(&self) -> Option<LegacyFileIdentity> {
        self.source_identity
    }

    fn destination_identity(&self) -> Option<LegacyFileIdentity> {
        self.destination_identity
    }

    fn set_source_identity(&mut self, identity: LegacyFileIdentity) {
        self.source_identity = Some(identity);
    }

    fn set_destination_identity(&mut self, identity: LegacyFileIdentity) {
        self.destination_identity = Some(identity);
    }

    fn set_marker_digest(&mut self, marker_digest: [u8; 32]) {
        self.marker_digest = marker_digest;
    }

    pub(crate) fn validates_identities(
        &self,
        source_digest: [u8; 32],
        destination_digest: [u8; 32],
    ) -> bool {
        self.source_digest == source_digest && self.destination_digest == destination_digest
    }

    fn has_valid_durable_shape(&self) -> bool {
        let valid_identity = |identity: LegacyFileIdentity| identity.dev != 0 && identity.ino != 0;
        match self.phase {
            Some(LegacyMigrationPhase::Prepared) => {
                self.source_identity.is_some_and(valid_identity)
                    && self.destination_identity.is_none()
            }
            Some(
                LegacyMigrationPhase::PayloadStaged
                | LegacyMigrationPhase::MarkerPublished
                | LegacyMigrationPhase::Committed
                | LegacyMigrationPhase::SourceRetired,
            ) => {
                self.source_identity.is_some_and(valid_identity)
                    && self.destination_identity.is_some_and(valid_identity)
            }
            None => false,
        }
    }

    pub(crate) const fn next_action(
        &self,
        observation: LegacyMigrationObservation,
    ) -> LegacyMigrationAction {
        match observation {
            LegacyMigrationObservation::MissingMarker
            | LegacyMigrationObservation::ReplacementDetected
            | LegacyMigrationObservation::Ambiguous
            | LegacyMigrationObservation::ForeignOwner => LegacyMigrationAction::Quarantine,
            LegacyMigrationObservation::Unmigrated => match self.phase {
                None => LegacyMigrationAction::PrepareJournal,
                Some(LegacyMigrationPhase::Prepared) => LegacyMigrationAction::StagePayload,
                Some(LegacyMigrationPhase::PayloadStaged) => LegacyMigrationAction::PublishMarker,
                Some(LegacyMigrationPhase::MarkerPublished) => LegacyMigrationAction::Commit,
                Some(LegacyMigrationPhase::Committed) => LegacyMigrationAction::RetireSource,
                Some(LegacyMigrationPhase::SourceRetired) => LegacyMigrationAction::AlreadyMigrated,
            },
            LegacyMigrationObservation::Committed => match self.phase {
                Some(LegacyMigrationPhase::SourceRetired) => LegacyMigrationAction::AlreadyMigrated,
                Some(LegacyMigrationPhase::Committed) => LegacyMigrationAction::RetireSource,
                _ => LegacyMigrationAction::Quarantine,
            },
        }
    }

    pub fn advance(&mut self, next: LegacyMigrationPhase) -> Result<(), LegacyMigrationError> {
        let expected = match self.phase {
            None => LegacyMigrationPhase::Prepared,
            Some(LegacyMigrationPhase::Prepared) => LegacyMigrationPhase::PayloadStaged,
            Some(LegacyMigrationPhase::PayloadStaged) => LegacyMigrationPhase::MarkerPublished,
            Some(LegacyMigrationPhase::MarkerPublished) => LegacyMigrationPhase::Committed,
            Some(LegacyMigrationPhase::Committed) => LegacyMigrationPhase::SourceRetired,
            Some(LegacyMigrationPhase::SourceRetired) => {
                return Err(LegacyMigrationError::AlreadyTerminal);
            }
        };
        if expected != next {
            return Err(LegacyMigrationError::PhaseOrder);
        }
        self.phase = Some(next);
        Ok(())
    }

    pub const fn derived_status(&self) -> LegacyMigrationStatus {
        match self.phase {
            Some(LegacyMigrationPhase::Committed | LegacyMigrationPhase::SourceRetired) => {
                LegacyMigrationStatus::Adopted
            }
            _ => LegacyMigrationStatus::Pending,
        }
    }

    pub(crate) const fn status_for(
        &self,
        observation: LegacyMigrationObservation,
    ) -> LegacyMigrationStatus {
        match observation {
            LegacyMigrationObservation::MissingMarker
            | LegacyMigrationObservation::ReplacementDetected
            | LegacyMigrationObservation::Ambiguous
            | LegacyMigrationObservation::ForeignOwner => LegacyMigrationStatus::Quarantined,
            LegacyMigrationObservation::Unmigrated => self.derived_status(),
            LegacyMigrationObservation::Committed => match self.derived_status() {
                LegacyMigrationStatus::Adopted => LegacyMigrationStatus::Adopted,
                LegacyMigrationStatus::Pending | LegacyMigrationStatus::Quarantined => {
                    LegacyMigrationStatus::Quarantined
                }
            },
        }
    }
}

impl core::fmt::Debug for LegacyMigrationJournal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LegacyMigrationJournal")
            .field("phase", &self.phase)
            .field("has_source_digest", &true)
            .field("has_destination_digest", &true)
            .field("has_marker_digest", &true)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMigrationStatus {
    Pending,
    Adopted,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMigrationError {
    IdentityInvalid,
    PhaseOrder,
    AlreadyTerminal,
    InventoryInvalid,
    ForeignOwner,
    Durability,
    LockUnavailable,
    BudgetExceeded,
}

impl core::fmt::Display for LegacyMigrationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::IdentityInvalid => "legacy-migration-identity-invalid",
            Self::PhaseOrder => "legacy-migration-phase-order-invalid",
            Self::AlreadyTerminal => "legacy-migration-already-terminal",
            Self::InventoryInvalid => "legacy-migration-inventory-invalid",
            Self::ForeignOwner => "legacy-migration-foreign-owner",
            Self::Durability => "legacy-migration-durability-failure",
            Self::LockUnavailable => "legacy-migration-lock-unavailable",
            Self::BudgetExceeded => "legacy-migration-budget-exceeded",
        })
    }
}

impl std::error::Error for LegacyMigrationError {}

/// Resolved paths are created only by the trusted broker bundle adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyMigrationPaths {
    source: PathBuf,
    destination: PathBuf,
    journal: PathBuf,
    marker: PathBuf,
    expected_owner: (u32, u32),
}

impl LegacyMigrationPaths {
    pub(crate) fn new(
        source: PathBuf,
        destination: PathBuf,
        journal: PathBuf,
        marker: PathBuf,
        expected_owner: (u32, u32),
    ) -> Result<Self, LegacyMigrationError> {
        let paths = [
            source.as_path(),
            destination.as_path(),
            journal.as_path(),
            marker.as_path(),
        ];
        if paths.iter().any(|path| {
            !path.is_absolute()
                || path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
                || path.file_name().and_then(|name| name.to_str()).is_none()
        }) {
            return Err(LegacyMigrationError::InventoryInvalid);
        }
        if !cfg!(test)
            && !migration_paths_match_marker_contract(&source, &destination, &journal, &marker)
        {
            return Err(LegacyMigrationError::InventoryInvalid);
        }
        Ok(Self {
            source,
            destination,
            journal,
            marker,
            expected_owner,
        })
    }
}

fn migration_paths_match_marker_contract(
    source: &Path,
    destination: &Path,
    journal: &Path,
    marker: &Path,
) -> bool {
    let Some(vm_root) = destination.parent() else {
        return false;
    };
    let Some(state_root) = vm_root.parent().and_then(Path::parent) else {
        return false;
    };
    marker.parent().and_then(Path::parent) == Some(state_root)
        && marker
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("swtpm-markers")
        && marker.file_name() == vm_root.file_name()
        && source.parent() == Some(vm_root)
        && journal.parent() == Some(vm_root)
        && source.file_name().and_then(|name| name.to_str()) == Some("swtpm-legacy")
        && destination.file_name().and_then(|name| name.to_str()) == Some("swtpm")
        && journal.file_name().and_then(|name| name.to_str()) == Some(".d2b-legacy-swtpm.journal")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyInventoryState {
    NeverProvisioned,
    ValidLegacy,
    AlreadyCommitted,
    Missing,
    Replaced,
    Ambiguous,
    Foreign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyInventory {
    pub state: LegacyInventoryState,
    pub source_digest: Option<[u8; 32]>,
    pub destination_digest: Option<[u8; 32]>,
    pub marker_digest: Option<[u8; 32]>,
}

struct AnchoredPath {
    parent: OwnedFd,
    name: String,
}

struct AnchoredPaths {
    source: AnchoredPath,
    destination: AnchoredPath,
    journal: AnchoredPath,
    marker: Option<AnchoredPath>,
    lock: AnchoredPath,
}

struct MigrationLock {
    parent: OwnedFd,
    name: String,
    fd: OwnedFd,
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = unlinkat(&self.parent, self.name.as_str(), AtFlags::empty());
        let _ = fsync(&self.parent);
    }
}

fn anchored(path: &Path) -> Result<AnchoredPath, LegacyMigrationError> {
    let parent = path
        .parent()
        .ok_or(LegacyMigrationError::InventoryInvalid)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LegacyMigrationError::InventoryInvalid)?
        .to_owned();
    let parent = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    Ok(AnchoredPath { parent, name })
}

fn anchored_optional(path: &Path) -> Result<Option<AnchoredPath>, LegacyMigrationError> {
    let parent = path
        .parent()
        .ok_or(LegacyMigrationError::InventoryInvalid)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LegacyMigrationError::InventoryInvalid)?
        .to_owned();
    let parent = match crate::sys::path_safe::open_dir_path_safe(parent) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LegacyMigrationError::InventoryInvalid),
    };
    Ok(Some(AnchoredPath { parent, name }))
}

fn anchored_paths(paths: &LegacyMigrationPaths) -> Result<AnchoredPaths, LegacyMigrationError> {
    let lock_path = paths
        .source
        .parent()
        .ok_or(LegacyMigrationError::InventoryInvalid)?
        .join(LOCK_NAME);
    Ok(AnchoredPaths {
        source: anchored(&paths.source)?,
        destination: anchored(&paths.destination)?,
        journal: anchored(&paths.journal)?,
        marker: anchored_optional(&paths.marker)?,
        lock: anchored(&lock_path)?,
    })
}

fn child_gid_is_trusted(
    parent: std::os::fd::BorrowedFd<'_>,
    child_gid: u32,
) -> std::io::Result<bool> {
    let stat = crate::sys::path_safe::fstat_fd(parent)?;
    Ok(child_gid_is_trusted_metadata(
        stat.st_mode,
        stat.st_gid,
        nix::unistd::getegid().as_raw(),
        child_gid,
    ))
}

fn child_gid_is_trusted_metadata(
    parent_mode: libc::mode_t,
    parent_gid: u32,
    effective_gid: u32,
    child_gid: u32,
) -> bool {
    child_gid == effective_gid
        || (parent_mode & libc::S_IFMT == libc::S_IFDIR
            && parent_mode & 0o002 == 0
            && parent_mode & 0o2000 != 0
            && parent_gid == child_gid)
}

fn child_stat(path: &AnchoredPath) -> Result<Option<libc::stat>, LegacyMigrationError> {
    crate::sys::path_safe::fstatat_nofollow(&path.parent, &path.name)
        .map_err(|_| LegacyMigrationError::InventoryInvalid)
}

fn open_child_dir(path: &AnchoredPath) -> Result<OwnedFd, LegacyMigrationError> {
    crate::sys::path_safe::open_at(
        path.parent.as_fd(),
        Path::new(&path.name),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )
    .map_err(|_| LegacyMigrationError::InventoryInvalid)
}

fn verify_regular_or_directory(stat: &libc::stat) -> Result<(), LegacyMigrationError> {
    let kind = stat.st_mode & libc::S_IFMT;
    if kind != libc::S_IFREG && kind != libc::S_IFDIR {
        return Err(LegacyMigrationError::InventoryInvalid);
    }

    if stat.st_mode & 0o002 != 0 {
        return Err(LegacyMigrationError::ForeignOwner);
    }

    Ok(())
}

fn file_identity(stat: &libc::stat) -> LegacyFileIdentity {
    LegacyFileIdentity {
        dev: stat.st_dev,
        ino: stat.st_ino,
        uid: stat.st_uid,
        gid: stat.st_gid,
        mode: stat.st_mode & 0o7777,
    }
}

fn digest_fd_tree(
    fd: &OwnedFd,
    expected_owner: Option<(u32, u32)>,
) -> Result<[u8; 32], LegacyMigrationError> {
    let mut entries = Vec::new();
    let mut count = 0usize;
    digest_directory(
        fd.as_fd(),
        &mut Vec::new(),
        &mut entries,
        &mut count,
        expected_owner,
    )?;
    entries.sort_by(|left: &(String, Vec<u8>), right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, bytes) in entries {
        digest.update((relative.len() as u64).to_be_bytes());
        digest.update(relative.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    Ok(digest.finalize().into())
}

fn digest_directory(
    fd: std::os::fd::BorrowedFd<'_>,
    prefix: &mut Vec<String>,
    entries: &mut Vec<(String, Vec<u8>)>,
    count: &mut usize,
    expected_owner: Option<(u32, u32)>,
) -> Result<(), LegacyMigrationError> {
    let directory = Dir::read_from(fd).map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    for entry in directory {
        let entry = entry.map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        *count = count.saturating_add(1);
        if *count > MAX_TREE_ENTRIES {
            return Err(LegacyMigrationError::BudgetExceeded);
        }
        let name = std::str::from_utf8(raw)
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?
            .to_owned();
        let child = crate::sys::path_safe::open_at(
            fd,
            Path::new(&name),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        )
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let stat = crate::sys::path_safe::fstat_fd(child.as_fd())
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        verify_regular_or_directory(&stat)?;
        if expected_owner.is_some_and(|(uid, gid)| stat.st_uid != uid || stat.st_gid != gid) {
            return Err(LegacyMigrationError::ForeignOwner);
        }
        prefix.push(name);
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            digest_directory(child.as_fd(), prefix, entries, count, expected_owner)?;
        } else {
            let file = std::fs::File::from(child);
            let mut bytes = Vec::new();
            file.take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(LegacyMigrationError::BudgetExceeded);
            }
            entries.push((prefix.join("/"), bytes));
        }
        prefix.pop();
    }
    Ok(())
}

/// Calculate a digest over a trusted, anchored directory.
pub(crate) fn digest_tree(root: &Path) -> Result<[u8; 32], LegacyMigrationError> {
    let root = anchored(root)?;
    let stat = child_stat(&root)?.ok_or(LegacyMigrationError::InventoryInvalid)?;
    verify_regular_or_directory(&stat)?;
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(LegacyMigrationError::InventoryInvalid);
    }
    digest_fd_tree(&open_child_dir(&root)?, None)
}

fn read_digest(path: &AnchoredPath) -> Result<Option<[u8; 32]>, LegacyMigrationError> {
    let Some(bytes) = read_owned_file(path)? else {
        return Ok(None);
    };
    Ok(Some(Sha256::digest(bytes).into()))
}

fn read_owned_file(path: &AnchoredPath) -> Result<Option<Vec<u8>>, LegacyMigrationError> {
    let Some(stat) = child_stat(path)? else {
        return Ok(None);
    };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || stat.st_mode & 0o7777 != MARKER_MODE
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || !child_gid_is_trusted(path.parent.as_fd(), stat.st_gid)
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?
    {
        return Err(LegacyMigrationError::ForeignOwner);
    }
    let fd = crate::sys::path_safe::open_at(
        path.parent.as_fd(),
        Path::new(&path.name),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )
    .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    let file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(LegacyMigrationError::BudgetExceeded);
    }
    Ok(Some(bytes))
}

fn read_journal(
    path: &AnchoredPath,
) -> Result<Option<LegacyMigrationJournal>, LegacyMigrationError> {
    let Some(stat) = child_stat(path)? else {
        return Ok(None);
    };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || stat.st_mode & 0o7777 != MARKER_MODE
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || !child_gid_is_trusted(path.parent.as_fd(), stat.st_gid)
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?
    {
        return Err(LegacyMigrationError::ForeignOwner);
    }
    let fd = crate::sys::path_safe::open_at(
        path.parent.as_fd(),
        Path::new(&path.name),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )
    .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    let file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| LegacyMigrationError::InventoryInvalid)
}

/// Inspect all migration artifacts without mutation.
pub(crate) fn inventory(
    paths: &LegacyMigrationPaths,
) -> Result<LegacyInventory, LegacyMigrationError> {
    let anchored = anchored_paths(paths)?;
    let source = child_stat(&anchored.source)?;
    let destination = child_stat(&anchored.destination)?;
    let (uid, gid) = paths.expected_owner;
    if [source.as_ref(), destination.as_ref()]
        .into_iter()
        .flatten()
        .any(|stat| stat.st_uid != uid || stat.st_gid != gid)
    {
        return Err(LegacyMigrationError::ForeignOwner);
    }
    if source
        .as_ref()
        .is_some_and(|stat| stat.st_mode & libc::S_IFMT != libc::S_IFDIR)
    {
        return Ok(LegacyInventory {
            state: LegacyInventoryState::Foreign,
            source_digest: None,
            destination_digest: None,
            marker_digest: None,
        });
    }
    let source_digest = source
        .as_ref()
        .filter(|stat| stat.st_mode & libc::S_IFMT == libc::S_IFDIR)
        .map(|_| {
            digest_fd_tree(
                &open_child_dir(&anchored.source)?,
                Some(paths.expected_owner),
            )
        })
        .transpose()?;
    let destination_digest = destination
        .as_ref()
        .filter(|stat| stat.st_mode & libc::S_IFMT == libc::S_IFDIR)
        .map(|_| {
            digest_fd_tree(
                &open_child_dir(&anchored.destination)?,
                Some(paths.expected_owner),
            )
        })
        .transpose()?;
    let marker_digest = anchored
        .marker
        .as_ref()
        .map(read_digest)
        .transpose()?
        .flatten();
    let journal = read_journal(&anchored.journal)?;
    if journal
        .as_ref()
        .is_some_and(|journal| !journal.has_valid_durable_shape())
    {
        return Err(LegacyMigrationError::InventoryInvalid);
    }
    let state = match (
        source.is_some(),
        destination.is_some(),
        journal.as_ref(),
        marker_digest,
    ) {
        (false, false, None, None) => LegacyInventoryState::NeverProvisioned,
        (false, _, Some(journal), Some(_))
            if matches!(
                journal.phase(),
                Some(LegacyMigrationPhase::Committed | LegacyMigrationPhase::SourceRetired)
            ) =>
        {
            LegacyInventoryState::AlreadyCommitted
        }
        (true, false, None, None) => LegacyInventoryState::ValidLegacy,
        (true, false, Some(journal), None)
            if journal.phase() == Some(LegacyMigrationPhase::Prepared) =>
        {
            LegacyInventoryState::ValidLegacy
        }
        (true, true, None, _) => LegacyInventoryState::Ambiguous,
        (false, true, None, None) => LegacyInventoryState::Missing,
        (true, true, Some(journal), None)
            if matches!(journal.phase(), Some(LegacyMigrationPhase::PayloadStaged)) =>
        {
            LegacyInventoryState::ValidLegacy
        }
        (true, true, Some(journal), Some(_))
            if journal.phase() == Some(LegacyMigrationPhase::PayloadStaged) =>
        {
            LegacyInventoryState::ValidLegacy
        }
        (true, true, Some(journal), Some(_))
            if journal.phase() == Some(LegacyMigrationPhase::MarkerPublished) =>
        {
            LegacyInventoryState::ValidLegacy
        }
        (true, true, Some(journal), Some(_))
            if journal.phase() == Some(LegacyMigrationPhase::Committed) =>
        {
            LegacyInventoryState::ValidLegacy
        }
        _ => LegacyInventoryState::Foreign,
    };
    Ok(LegacyInventory {
        state,
        source_digest,
        destination_digest,
        marker_digest,
    })
}

/// Probe the broker-owned migration inventory without changing any state.
///
/// The short-lived lock gives the Core caller a stable classification while a
/// migration attempt cannot publish a journal or marker concurrently.
pub(crate) fn probe(
    paths: &LegacyMigrationPaths,
) -> Result<LegacyInventoryState, LegacyMigrationError> {
    let anchored = match anchored_paths(paths) {
        Ok(anchored) => anchored,
        Err(error) => {
            tracing::warn!(?error, "TPM migration probe path anchoring failed");
            return Err(error);
        }
    };
    let _lock = match acquire_lock(&anchored.lock) {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!(?error, "TPM migration probe lock failed");
            return Err(error);
        }
    };
    let current = match inventory(paths) {
        Ok(current) => current,
        Err(error) => {
            tracing::warn!(?error, "TPM migration probe inventory failed");
            return Err(error);
        }
    };
    tracing::warn!(
        ?current,
        marker_parent_present = anchored.marker.is_some(),
        "TPM migration inventory probe state"
    );
    if !matches!(current.state, LegacyInventoryState::AlreadyCommitted) {
        return Ok(current.state);
    }
    let Some(journal) = read_journal(&anchored.journal)? else {
        return Ok(LegacyInventoryState::Ambiguous);
    };
    if destination_and_marker_match(paths, &anchored, &journal)? {
        Ok(LegacyInventoryState::AlreadyCommitted)
    } else {
        Ok(LegacyInventoryState::Ambiguous)
    }
}

fn acquire_lock(path: &AnchoredPath) -> Result<MigrationLock, LegacyMigrationError> {
    let fd = openat(
        path.parent.as_fd(),
        path.name.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    let stat = crate::sys::path_safe::fstat_fd(fd.as_fd())
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    // The per-VM state parent is setgid `users`, so a newly-created lock
    // inherits that group even though the broker owns the file. The lock is
    // mode 0600 and anchored below the broker-owned VM directory; the owner
    // and mode are the security boundary, not the inherited group.
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || stat.st_mode & 0o077 != 0
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || !child_gid_is_trusted(path.parent.as_fd(), stat.st_gid)
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?
    {
        return Err(LegacyMigrationError::ForeignOwner);
    }
    flock(fd.as_raw_fd(), FlockArg::LockExclusiveNonblock)
        .map_err(|_| LegacyMigrationError::LockUnavailable)?;
    Ok(MigrationLock {
        parent: path
            .parent
            .try_clone()
            .map_err(|_| LegacyMigrationError::Durability)?,
        name: path.name.clone(),
        fd,
    })
}

fn write_atomic(
    parent: &std::os::fd::OwnedFd,
    name: &str,
    bytes: &[u8],
) -> Result<(), LegacyMigrationError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(LegacyMigrationError::BudgetExceeded);
    }
    let temp = format!(
        ".d2b-migration-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    );
    let fd = openat(
        parent.as_fd(),
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(MARKER_MODE),
    )
    .map_err(|_| LegacyMigrationError::Durability)?;
    let temp_stat = crate::sys::path_safe::fstat_fd(fd.as_fd())
        .map_err(|_| LegacyMigrationError::Durability)?;
    if temp_stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || temp_stat.st_uid != nix::unistd::geteuid().as_raw()
        || !child_gid_is_trusted(parent.as_fd(), temp_stat.st_gid)
            .map_err(|_| LegacyMigrationError::Durability)?
        || temp_stat.st_mode & 0o7777 != MARKER_MODE
    {
        let _ = unlinkat(parent.as_fd(), temp.as_str(), AtFlags::empty());
        return Err(LegacyMigrationError::ForeignOwner);
    }
    let result = (|| {
        let mut file = std::fs::File::from(fd);
        file.write_all(bytes)
            .map_err(|_| LegacyMigrationError::Durability)?;
        file.sync_all()
            .map_err(|_| LegacyMigrationError::Durability)?;
        renameat_with(
            parent.as_fd(),
            temp.as_str(),
            parent.as_fd(),
            name,
            RenameFlags::empty(),
        )
        .map_err(|_| LegacyMigrationError::Durability)?;
        fsync(parent.as_fd()).map_err(|_| LegacyMigrationError::Durability)
    })();
    if result.is_err() {
        let _ = unlinkat(parent.as_fd(), temp.as_str(), AtFlags::empty());
    }
    result
}

fn planned_marker_digest(source: [u8; 32], destination: [u8; 32]) -> [u8; 32] {
    let bytes = serde_json::json!({
        "version": 1,
        "sourceDigest": hex_encode(source),
        "destinationDigest": hex_encode(destination),
    })
    .to_string()
    .as_bytes()
    .to_vec();
    Sha256::digest(bytes).into()
}

fn marker_vm(paths: &LegacyMigrationPaths) -> Result<&str, LegacyMigrationError> {
    paths
        .marker
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(LegacyMigrationError::InventoryInvalid)
}

fn exact_marker_payload(
    paths: &LegacyMigrationPaths,
    journal: &LegacyMigrationJournal,
) -> Result<Vec<u8>, LegacyMigrationError> {
    let identity = journal
        .destination_identity()
        .ok_or(LegacyMigrationError::IdentityInvalid)?;
    crate::ops::swtpm_dir::marker_payload(
        marker_vm(paths)?,
        crate::ops::swtpm_dir::MarkerOrigin::LegacyMigration,
        identity.dev,
        identity.ino,
        identity.uid,
        identity.gid,
        identity.mode,
        journal.marker_first_provisioned_ms(),
    )
    .map_err(|_| LegacyMigrationError::Durability)
}

fn destination_matches(
    paths: &LegacyMigrationPaths,
    anchored: &AnchoredPaths,
    journal: &LegacyMigrationJournal,
) -> Result<bool, LegacyMigrationError> {
    let Some(expected) = journal.destination_identity() else {
        return Ok(false);
    };
    let Some(stat) = child_stat(&anchored.destination)? else {
        return Ok(false);
    };
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR
        || file_identity(&stat) != expected
        || (expected.uid, expected.gid) != paths.expected_owner
        || expected.mode != 0o700
    {
        return Ok(false);
    }
    let destination_fd = match open_child_dir(&anchored.destination) {
        Ok(fd) => fd,
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let digest = match digest_fd_tree(&destination_fd, Some(paths.expected_owner)) {
        Ok(digest) => digest,
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    Ok(digest == journal.destination_digest())
}

fn marker_matches(
    paths: &LegacyMigrationPaths,
    anchored: &AnchoredPaths,
    journal: &LegacyMigrationJournal,
) -> Result<bool, LegacyMigrationError> {
    let Some(marker) = anchored.marker.as_ref() else {
        return Ok(false);
    };
    let bytes = match read_owned_file(marker) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(false),
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let Some(expected) = journal.destination_identity() else {
        return Ok(false);
    };
    let payload_digest: [u8; 32] = Sha256::digest(&bytes).into();
    Ok(payload_digest == journal.marker_digest()
        && crate::ops::swtpm_dir::marker_matches(
            &bytes,
            marker_vm(paths)?,
            crate::ops::swtpm_dir::MarkerOrigin::LegacyMigration,
            expected.dev,
            expected.ino,
            expected.uid,
            expected.gid,
            expected.mode,
        ))
}

fn destination_and_marker_match(
    paths: &LegacyMigrationPaths,
    anchored: &AnchoredPaths,
    journal: &LegacyMigrationJournal,
) -> Result<bool, LegacyMigrationError> {
    Ok(destination_matches(paths, anchored, journal)? && marker_matches(paths, anchored, journal)?)
}

pub(crate) fn validate_committed_for_harden(
    source: &Path,
    destination: &Path,
    journal: &Path,
    marker: &Path,
    expected_owner: (u32, u32),
) -> Result<bool, LegacyMigrationError> {
    let paths = LegacyMigrationPaths::new(
        source.to_path_buf(),
        destination.to_path_buf(),
        journal.to_path_buf(),
        marker.to_path_buf(),
        expected_owner,
    )?;
    let anchored = anchored_paths(&paths)?;
    let Some(journal_record) = read_journal(&anchored.journal)? else {
        return Ok(false);
    };
    if !journal_record.has_valid_durable_shape()
        || !matches!(
            journal_record.phase(),
            Some(LegacyMigrationPhase::Committed | LegacyMigrationPhase::SourceRetired)
        )
        || !destination_and_marker_match(&paths, &anchored, &journal_record)?
    {
        return Ok(false);
    }

    let Some(source_stat) = child_stat(&anchored.source)? else {
        return Ok(true);
    };
    if source_stat.st_mode & libc::S_IFMT != libc::S_IFDIR
        || journal_record
            .source_identity()
            .is_none_or(|identity| file_identity(&source_stat) != identity)
    {
        return Ok(false);
    }
    let source_fd = match open_child_dir(&anchored.source) {
        Ok(fd) => fd,
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let source_digest = match digest_fd_tree(&source_fd, Some(expected_owner)) {
        Ok(digest) => digest,
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    Ok(source_digest == journal_record.source_digest)
}

fn hex_encode(bytes: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn copy_tree(
    source: std::os::fd::BorrowedFd<'_>,
    destination: std::os::fd::BorrowedFd<'_>,
    count: &mut usize,
    expected_owner: Option<(u32, u32)>,
) -> Result<(), LegacyMigrationError> {
    let directory = Dir::read_from(source).map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    for entry in directory {
        let entry = entry.map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        *count = count.saturating_add(1);
        if *count > MAX_TREE_ENTRIES {
            return Err(LegacyMigrationError::BudgetExceeded);
        }
        let name = std::str::from_utf8(raw)
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?
            .to_owned();
        let source_fd = crate::sys::path_safe::open_at(
            source,
            Path::new(&name),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        )
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let stat = crate::sys::path_safe::fstat_fd(source_fd.as_fd())
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        verify_regular_or_directory(&stat)?;
        if expected_owner.is_some_and(|(uid, gid)| stat.st_uid != uid || stat.st_gid != gid) {
            return Err(LegacyMigrationError::ForeignOwner);
        }
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            mkdirat(destination, name.as_str(), Mode::from_raw_mode(0o700))
                .map_err(|_| LegacyMigrationError::Durability)?;
            let destination_fd = crate::sys::path_safe::open_at(
                destination,
                Path::new(&name),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            )
            .map_err(|_| LegacyMigrationError::Durability)?;
            if let Some((uid, gid)) = expected_owner {
                crate::sys::path_safe::fchown(destination_fd.as_fd(), Some(uid), Some(gid))
                    .map_err(|_| LegacyMigrationError::ForeignOwner)?;
            }
            copy_tree(
                source_fd.as_fd(),
                destination_fd.as_fd(),
                count,
                expected_owner,
            )?;
            fsync(destination_fd.as_fd()).map_err(|_| LegacyMigrationError::Durability)?;
        } else {
            let source_file = std::fs::File::from(source_fd);
            let temp = format!(
                ".d2b-copy-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            );
            let destination_fd = openat(
                destination,
                temp.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_raw_mode((stat.st_mode & 0o777) as u32),
            )
            .map_err(|_| LegacyMigrationError::Durability)?;
            let result = (|| {
                let mut destination_file = std::fs::File::from(destination_fd);
                let mut bytes = Vec::new();
                source_file
                    .take((MAX_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
                if bytes.len() > MAX_FILE_BYTES {
                    return Err(LegacyMigrationError::BudgetExceeded);
                }
                destination_file
                    .write_all(&bytes)
                    .map_err(|_| LegacyMigrationError::Durability)?;
                if let Some((uid, gid)) = expected_owner {
                    crate::sys::path_safe::fchown(destination_file.as_fd(), Some(uid), Some(gid))
                        .map_err(|_| LegacyMigrationError::ForeignOwner)?;
                }
                destination_file
                    .sync_all()
                    .map_err(|_| LegacyMigrationError::Durability)?;
                renameat_with(
                    destination,
                    temp.as_str(),
                    destination,
                    name.as_str(),
                    RenameFlags::empty(),
                )
                .map_err(|_| LegacyMigrationError::Durability)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = unlinkat(destination, temp.as_str(), AtFlags::empty());
            }
            result?;
        }
    }
    fsync(destination).map_err(|_| LegacyMigrationError::Durability)
}

fn remove_tree_fd(fd: std::os::fd::BorrowedFd<'_>) -> Result<(), LegacyMigrationError> {
    let directory = Dir::read_from(fd).map_err(|_| LegacyMigrationError::InventoryInvalid)?;
    for entry in directory {
        let entry = entry.map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        let child = std::str::from_utf8(raw).map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let child_fd = crate::sys::path_safe::open_at(
            fd,
            Path::new(child),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        )
        .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        let stat = crate::sys::path_safe::fstat_fd(child_fd.as_fd())
            .map_err(|_| LegacyMigrationError::InventoryInvalid)?;
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            remove_tree_fd(child_fd.as_fd())?;
            unlinkat(fd, child, AtFlags::REMOVEDIR)
                .map_err(|_| LegacyMigrationError::Durability)?;
        } else {
            unlinkat(fd, child, AtFlags::empty()).map_err(|_| LegacyMigrationError::Durability)?;
        }
    }
    fsync(fd).map_err(|_| LegacyMigrationError::Durability)
}

fn persist_journal_at(
    path: &AnchoredPath,
    journal: &LegacyMigrationJournal,
) -> Result<(), LegacyMigrationError> {
    let bytes = serde_json::to_vec(journal).map_err(|_| LegacyMigrationError::Durability)?;
    write_atomic(&path.parent, &path.name, &bytes)
}

fn publish_marker_at(path: &AnchoredPath, payload: &[u8]) -> Result<(), LegacyMigrationError> {
    if child_stat(path)?.is_some() {
        let existing = read_digest(path)?.ok_or(LegacyMigrationError::InventoryInvalid)?;
        let expected: [u8; 32] = Sha256::digest(payload).into();
        if existing != expected {
            return Err(LegacyMigrationError::ForeignOwner);
        }
        return Ok(());
    }
    write_atomic(&path.parent, &path.name, payload)
}

/// Execute or replay the byte-preserving migration.
pub(crate) fn migrate(
    paths: &LegacyMigrationPaths,
) -> Result<LegacyMigrationOutcome, LegacyMigrationError> {
    let anchored = anchored_paths(paths)?;
    let _lock = match acquire_lock(&anchored.lock) {
        Ok(lock) => lock,
        Err(LegacyMigrationError::LockUnavailable) => {
            return Ok(LegacyMigrationOutcome::Pending);
        }
        Err(error) => return Err(error),
    };
    let current = match inventory(paths) {
        Ok(current) => current,
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(LegacyMigrationOutcome::Ambiguous);
        }
        Err(error) => return Err(error),
    };
    if matches!(current.state, LegacyInventoryState::NeverProvisioned) {
        // Absence alone is not a Core-sealed never-provisioned decision.
        // The broker must quarantine rather than mint NotApplicable.
        return Ok(LegacyMigrationOutcome::Ambiguous);
    }
    if matches!(
        current.state,
        LegacyInventoryState::Missing
            | LegacyInventoryState::Replaced
            | LegacyInventoryState::Ambiguous
            | LegacyInventoryState::Foreign
    ) {
        return Ok(LegacyMigrationOutcome::Ambiguous);
    }
    if matches!(current.state, LegacyInventoryState::AlreadyCommitted) {
        let Some(mut journal) = read_journal(&anchored.journal)? else {
            return Ok(LegacyMigrationOutcome::Ambiguous);
        };
        if !destination_and_marker_match(paths, &anchored, &journal)? {
            return Ok(LegacyMigrationOutcome::Ambiguous);
        }
        if journal.phase() == Some(LegacyMigrationPhase::Committed) {
            journal.advance(LegacyMigrationPhase::SourceRetired)?;
            persist_journal_at(&anchored.journal, &journal)?;
            return Ok(LegacyMigrationOutcome::Migrated);
        }
        return Ok(LegacyMigrationOutcome::AlreadyMigrated);
    }
    let source_digest = current
        .source_digest
        .ok_or(LegacyMigrationError::InventoryInvalid)?;
    let source_stat = match child_stat(&anchored.source)? {
        Some(stat) => stat,
        None => return Ok(LegacyMigrationOutcome::Ambiguous),
    };
    let source_identity = file_identity(&source_stat);
    let source_fd = match open_child_dir(&anchored.source) {
        Ok(fd) => fd,
        Err(LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner) => {
            return Ok(LegacyMigrationOutcome::Ambiguous);
        }
        Err(error) => return Err(error),
    };
    let existing_journal = read_journal(&anchored.journal)?;
    if existing_journal
        .as_ref()
        .and_then(LegacyMigrationJournal::source_identity)
        .is_some_and(|identity| identity != source_identity)
    {
        return Ok(LegacyMigrationOutcome::Ambiguous);
    }
    let destination_digest = existing_journal
        .as_ref()
        .map(LegacyMigrationJournal::destination_digest)
        .unwrap_or_else(|| current.destination_digest.unwrap_or(source_digest));
    let marker_digest = planned_marker_digest(source_digest, destination_digest);
    let mut journal = match existing_journal {
        Some(journal) if !journal.validates_identities(source_digest, destination_digest) => {
            return Ok(LegacyMigrationOutcome::Ambiguous);
        }
        Some(journal) => journal,
        None => LegacyMigrationJournal::new(source_digest, destination_digest, marker_digest)?,
    };
    if journal.phase() == Some(LegacyMigrationPhase::Prepared)
        && journal.marker_digest() != marker_digest
    {
        return Ok(LegacyMigrationOutcome::Ambiguous);
    }
    if matches!(
        journal.phase(),
        Some(
            LegacyMigrationPhase::PayloadStaged
                | LegacyMigrationPhase::MarkerPublished
                | LegacyMigrationPhase::Committed
                | LegacyMigrationPhase::SourceRetired
        )
    ) {
        let payload = exact_marker_payload(paths, &journal)?;
        let payload_digest: [u8; 32] = Sha256::digest(payload).into();
        if payload_digest != journal.marker_digest() {
            return Ok(LegacyMigrationOutcome::Ambiguous);
        }
    }
    let mut changed = false;
    loop {
        match journal.next_action(LegacyMigrationObservation::Unmigrated) {
            LegacyMigrationAction::PrepareJournal => {
                journal.set_source_identity(source_identity);
                journal.advance(LegacyMigrationPhase::Prepared)?;
                persist_journal_at(&anchored.journal, &journal)?;
                changed = true;
            }
            LegacyMigrationAction::StagePayload => {
                let destination_stat = child_stat(&anchored.destination)?;
                // A Prepared journal has no recorded destination identity.
                // Any destination that already exists is therefore
                // untrusted and must be quarantined without deletion or
                // restaging.
                if destination_stat.is_some() || journal.destination_identity().is_some() {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                mkdirat(
                    anchored.destination.parent.as_fd(),
                    anchored.destination.name.as_str(),
                    Mode::from_raw_mode(0o700),
                )
                .map_err(|_| LegacyMigrationError::Durability)?;
                let destination = open_child_dir(&anchored.destination)?;
                let (uid, gid) = paths.expected_owner;
                crate::sys::path_safe::fchown(destination.as_fd(), Some(uid), Some(gid))
                    .map_err(|_| LegacyMigrationError::ForeignOwner)?;
                crate::sys::path_safe::fchmod(destination.as_fd(), 0o700)
                    .map_err(|_| LegacyMigrationError::ForeignOwner)?;
                let mut count = 0;
                copy_tree(
                    source_fd.as_fd(),
                    destination.as_fd(),
                    &mut count,
                    Some(paths.expected_owner),
                )?;
                let actual = digest_fd_tree(
                    &open_child_dir(&anchored.destination)?,
                    Some(paths.expected_owner),
                )?;
                if actual != destination_digest {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                let destination_stat = child_stat(&anchored.destination)?
                    .ok_or(LegacyMigrationError::InventoryInvalid)?;
                let destination_identity = file_identity(&destination_stat);
                if destination_identity.uid != uid
                    || destination_identity.gid != gid
                    || destination_identity.mode != 0o700
                {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                journal.set_destination_identity(destination_identity);
                let payload = exact_marker_payload(paths, &journal)?;
                journal.set_marker_digest(Sha256::digest(payload).into());
                journal.advance(LegacyMigrationPhase::PayloadStaged)?;
                persist_journal_at(&anchored.journal, &journal)?;
                changed = true;
            }
            LegacyMigrationAction::PublishMarker => {
                if !destination_matches(paths, &anchored, &journal)? {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                let payload = exact_marker_payload(paths, &journal)?;
                let payload_digest: [u8; 32] = Sha256::digest(&payload).into();
                if payload_digest != journal.marker_digest() {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                let Some(marker) = anchored.marker.as_ref() else {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                };
                match publish_marker_at(marker, &payload) {
                    Ok(()) => {}
                    Err(
                        LegacyMigrationError::InventoryInvalid | LegacyMigrationError::ForeignOwner,
                    ) => {
                        return Ok(LegacyMigrationOutcome::Ambiguous);
                    }
                    Err(error) => return Err(error),
                }
                journal.advance(LegacyMigrationPhase::MarkerPublished)?;
                persist_journal_at(&anchored.journal, &journal)?;
                changed = true;
            }
            LegacyMigrationAction::Commit => {
                if !destination_and_marker_match(paths, &anchored, &journal)? {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                journal.advance(LegacyMigrationPhase::Committed)?;
                persist_journal_at(&anchored.journal, &journal)?;
                changed = true;
            }
            LegacyMigrationAction::RetireSource => {
                if !destination_and_marker_match(paths, &anchored, &journal)? {
                    return Ok(LegacyMigrationOutcome::Ambiguous);
                }
                let current_source = child_stat(&anchored.source)?;
                if let Some(current_source) = current_source {
                    let Some(expected_source) = journal.source_identity() else {
                        return Ok(LegacyMigrationOutcome::Ambiguous);
                    };
                    if file_identity(&current_source) != expected_source {
                        return Ok(LegacyMigrationOutcome::Ambiguous);
                    }
                    let held_identity = file_identity(
                        &crate::sys::path_safe::fstat_fd(source_fd.as_fd())
                            .map_err(|_| LegacyMigrationError::InventoryInvalid)?,
                    );
                    if held_identity != expected_source {
                        return Ok(LegacyMigrationOutcome::Ambiguous);
                    }
                    let actual = digest_fd_tree(&source_fd, Some(paths.expected_owner))?;
                    if actual != source_digest {
                        return Ok(LegacyMigrationOutcome::Ambiguous);
                    }
                    let quarantine_name = format!(
                        ".d2b-tpm-retire-{}-{}",
                        std::process::id(),
                        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
                    );
                    if child_stat(&AnchoredPath {
                        parent: anchored
                            .source
                            .parent
                            .try_clone()
                            .map_err(|_| LegacyMigrationError::Durability)?,
                        name: quarantine_name.clone(),
                    })?
                    .is_some()
                    {
                        return Ok(LegacyMigrationOutcome::Ambiguous);
                    }
                    renameat_with(
                        anchored.source.parent.as_fd(),
                        anchored.source.name.as_str(),
                        anchored.source.parent.as_fd(),
                        quarantine_name.as_str(),
                        RenameFlags::empty(),
                    )
                    .map_err(|_| LegacyMigrationError::Durability)?;
                    remove_tree_fd(source_fd.as_fd())?;
                    unlinkat(
                        anchored.source.parent.as_fd(),
                        quarantine_name.as_str(),
                        AtFlags::REMOVEDIR,
                    )
                    .map_err(|_| LegacyMigrationError::Durability)?;
                    fsync(anchored.source.parent.as_fd())
                        .map_err(|_| LegacyMigrationError::Durability)?;
                }
                journal.advance(LegacyMigrationPhase::SourceRetired)?;
                persist_journal_at(&anchored.journal, &journal)?;
                changed = true;
            }
            LegacyMigrationAction::AlreadyMigrated => {
                return Ok(if changed {
                    LegacyMigrationOutcome::Migrated
                } else {
                    LegacyMigrationOutcome::AlreadyMigrated
                });
            }
            LegacyMigrationAction::Quarantine => return Ok(LegacyMigrationOutcome::Ambiguous),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!("legacy-migration-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(root.join("state")).unwrap();
            Self(root)
        }

        fn paths(&self) -> LegacyMigrationPaths {
            LegacyMigrationPaths::new(
                self.0.join("legacy"),
                self.0.join("state/swtpm"),
                self.0.join("state/migration.journal"),
                self.0.join("state/marker"),
                (
                    nix::unistd::geteuid().as_raw(),
                    nix::unistd::getegid().as_raw(),
                ),
            )
            .unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn committed_fixture(label: &str) -> (Scratch, LegacyMigrationPaths) {
        let scratch = Scratch::new(label);
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"legacy").unwrap();
        fs::create_dir_all(&paths.destination).unwrap();
        fs::set_permissions(&paths.destination, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(paths.destination.join("nvram"), b"legacy").unwrap();

        let source_digest = digest_tree(&paths.source).unwrap();
        let destination_digest = digest_tree(&paths.destination).unwrap();
        let mut journal = LegacyMigrationJournal::new(
            source_digest,
            destination_digest,
            planned_marker_digest(source_digest, destination_digest),
        )
        .unwrap();
        let source_stat = fs::symlink_metadata(&paths.source).unwrap();
        let destination_stat = fs::symlink_metadata(&paths.destination).unwrap();
        journal.set_source_identity(LegacyFileIdentity {
            dev: source_stat.dev(),
            ino: source_stat.ino(),
            uid: source_stat.uid(),
            gid: source_stat.gid(),
            mode: source_stat.permissions().mode() & 0o7777,
        });
        journal.advance(LegacyMigrationPhase::Prepared).unwrap();
        journal.set_destination_identity(LegacyFileIdentity {
            dev: destination_stat.dev(),
            ino: destination_stat.ino(),
            uid: destination_stat.uid(),
            gid: destination_stat.gid(),
            mode: destination_stat.permissions().mode() & 0o7777,
        });
        let payload = exact_marker_payload(&paths, &journal).unwrap();
        journal.set_marker_digest(Sha256::digest(payload.clone()).into());
        journal
            .advance(LegacyMigrationPhase::PayloadStaged)
            .unwrap();
        journal
            .advance(LegacyMigrationPhase::MarkerPublished)
            .unwrap();
        journal.advance(LegacyMigrationPhase::Committed).unwrap();
        fs::write(
            &paths.marker,
            exact_marker_payload(&paths, &journal).unwrap(),
        )
        .unwrap();
        fs::set_permissions(&paths.marker, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(
            &paths.journal,
            serde_json::to_vec(&journal).expect("journal serializes"),
        )
        .unwrap();
        fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();
        (scratch, paths)
    }

    #[test]
    fn production_marker_contract_matches_harden_tree_only() {
        assert!(migration_paths_match_marker_contract(
            Path::new("/var/lib/d2b/vms/work/swtpm-legacy"),
            Path::new("/var/lib/d2b/vms/work/swtpm"),
            Path::new("/var/lib/d2b/vms/work/.d2b-legacy-swtpm.journal"),
            Path::new("/var/lib/d2b/swtpm-markers/work"),
        ));
        assert!(!migration_paths_match_marker_contract(
            Path::new("/var/lib/d2b/vms/work/swtpm-legacy"),
            Path::new("/var/lib/d2b/vms/work/swtpm"),
            Path::new("/var/lib/d2b/vms/work/.d2b-legacy-swtpm.journal"),
            Path::new("/var/lib/d2b/vms/work/.d2b-legacy-swtpm.marker"),
        ));
    }

    #[test]
    fn child_gid_is_trusted_accepts_effective_and_safe_setgid_inheritance_only() {
        let effective_gid = 100;
        let inherited_gid = 200;
        let directory = libc::S_IFDIR;

        assert!(child_gid_is_trusted_metadata(
            directory | 0o0700,
            999,
            effective_gid,
            effective_gid,
        ));
        assert!(child_gid_is_trusted_metadata(
            directory | 0o2700,
            inherited_gid,
            effective_gid,
            inherited_gid,
        ));
        assert!(!child_gid_is_trusted_metadata(
            directory | 0o2702,
            inherited_gid,
            effective_gid,
            inherited_gid,
        ));
        assert!(!child_gid_is_trusted_metadata(
            directory | 0o0700,
            inherited_gid,
            effective_gid,
            inherited_gid,
        ));
    }

    #[test]
    fn committed_replay_requires_proven_source_absence_or_identity() {
        let (scratch, paths) = committed_fixture("committed-absent");
        fs::remove_dir_all(&paths.source).unwrap();
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Migrated);
        drop(scratch);

        let (scratch, paths) = committed_fixture("committed-file");
        fs::remove_dir_all(&paths.source).unwrap();
        fs::write(&paths.source, b"wrong-type").unwrap();
        let journal_before = fs::read(&paths.journal).unwrap();
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert_eq!(fs::read(&paths.journal).unwrap(), journal_before);
        assert!(paths.source.is_file());
        drop(scratch);

        let (scratch, paths) = committed_fixture("committed-symlink");
        let outside = scratch.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::remove_dir_all(&paths.source).unwrap();
        symlink(&outside, &paths.source).unwrap();
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert!(
            fs::symlink_metadata(&paths.source)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        drop(scratch);

        let (scratch, paths) = committed_fixture("committed-replaced");
        let original = fs::symlink_metadata(&paths.source).unwrap();
        let stash = paths.source.with_file_name("swtpm-legacy-stash");
        fs::rename(&paths.source, &stash).unwrap();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"replacement").unwrap();
        assert_ne!(
            fs::symlink_metadata(&paths.source).unwrap().ino(),
            original.ino()
        );
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert!(paths.source.is_dir());
        assert!(stash.is_dir());
        drop(scratch);
    }

    #[test]
    fn migration_replays_byte_preserving_commit_and_source_retirement() {
        let scratch = Scratch::new("positive");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("tpm2-00.permall"), b"legacy-nvram").unwrap();
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Migrated);
        assert_eq!(
            fs::read(paths.destination.join("tpm2-00.permall")).unwrap(),
            b"legacy-nvram"
        );
        assert!(!paths.source.exists());
        assert!(paths.marker.exists());
        assert!(paths.journal.exists());
        assert_eq!(
            migrate(&paths).unwrap(),
            LegacyMigrationOutcome::AlreadyMigrated
        );
    }

    #[test]
    fn missing_replaced_or_foreign_state_is_quarantined_without_mutation() {
        let scratch = Scratch::new("foreign");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        symlink(scratch.0.join("outside"), paths.source.join("state")).unwrap();
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert!(!paths.destination.exists());
        assert!(!paths.journal.exists());
    }

    #[test]
    fn never_provisioned_state_is_quarantined_without_core_decision() {
        let scratch = Scratch::new("absent");
        let paths = scratch.paths();
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert!(!paths.destination.exists());
        assert!(!paths.marker.exists());
    }

    #[test]
    fn inventory_probe_distinguishes_fresh_and_legacy_state_without_mutation() {
        let scratch = Scratch::new("probe-fresh");
        let paths = scratch.paths();
        assert_eq!(
            probe(&paths).unwrap(),
            LegacyInventoryState::NeverProvisioned
        );
        assert!(!paths.destination.exists());
        drop(scratch);

        let scratch = Scratch::new("probe-legacy");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"legacy").unwrap();
        assert_eq!(probe(&paths).unwrap(), LegacyInventoryState::ValidLegacy);
        assert!(paths.source.exists());
        assert!(!paths.destination.exists());
    }

    #[test]
    fn inventory_probe_accepts_fresh_state_before_marker_root_bootstrap() {
        let scratch = Scratch::new("probe-marker-root-absent");
        let paths = LegacyMigrationPaths::new(
            scratch.0.join("legacy"),
            scratch.0.join("state/swtpm"),
            scratch.0.join("state/migration.journal"),
            scratch.0.join("swtpm-markers/work"),
            (
                nix::unistd::geteuid().as_raw(),
                nix::unistd::getegid().as_raw(),
            ),
        )
        .unwrap();
        assert_eq!(
            probe(&paths).unwrap(),
            LegacyInventoryState::NeverProvisioned
        );
        assert!(!scratch.0.join("swtpm-markers").exists());
    }

    #[test]
    fn inventory_probe_requires_matching_committed_destination_evidence() {
        let (scratch, paths) = committed_fixture("probe-committed");
        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Migrated);
        assert_eq!(
            probe(&paths).unwrap(),
            LegacyInventoryState::AlreadyCommitted
        );
        fs::remove_dir_all(&paths.destination).unwrap();
        assert_eq!(probe(&paths).unwrap(), LegacyInventoryState::Ambiguous);
        drop(scratch);
    }

    #[test]
    fn prepared_phase_replays_with_destination_absent() {
        let scratch = Scratch::new("prepared-absent");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"complete").unwrap();
        let source_digest = digest_tree(&paths.source).unwrap();
        let marker_digest = planned_marker_digest(source_digest, source_digest);
        let mut journal =
            LegacyMigrationJournal::new(source_digest, source_digest, marker_digest).unwrap();
        let source_stat = std::fs::symlink_metadata(&paths.source).unwrap();
        journal.set_source_identity(LegacyFileIdentity {
            dev: source_stat.dev(),
            ino: source_stat.ino(),
            uid: source_stat.uid(),
            gid: source_stat.gid(),
            mode: source_stat.permissions().mode() & 0o7777,
        });
        journal.advance(LegacyMigrationPhase::Prepared).unwrap();
        fs::write(
            &paths.journal,
            serde_json::to_vec(&journal).expect("journal serializes"),
        )
        .unwrap();
        fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            inventory(&paths).unwrap().state,
            LegacyInventoryState::ValidLegacy
        );

        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Migrated);
        assert_eq!(
            fs::read(paths.destination.join("nvram")).unwrap(),
            b"complete"
        );
        assert!(!paths.source.exists());
    }

    #[test]
    fn prepared_destination_without_identity_is_quarantined_without_mutation() {
        let scratch = Scratch::new("prepared-existing");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"complete").unwrap();
        fs::create_dir_all(&paths.destination).unwrap();
        fs::write(paths.destination.join("nvram"), b"untrusted").unwrap();
        let source_digest = digest_tree(&paths.source).unwrap();
        let marker_digest = planned_marker_digest(source_digest, source_digest);
        let mut journal =
            LegacyMigrationJournal::new(source_digest, source_digest, marker_digest).unwrap();
        let source_stat = fs::symlink_metadata(&paths.source).unwrap();
        journal.set_source_identity(LegacyFileIdentity {
            dev: source_stat.dev(),
            ino: source_stat.ino(),
            uid: source_stat.uid(),
            gid: source_stat.gid(),
            mode: source_stat.permissions().mode() & 0o7777,
        });
        journal.advance(LegacyMigrationPhase::Prepared).unwrap();
        fs::write(
            &paths.journal,
            serde_json::to_vec(&journal).expect("journal serializes"),
        )
        .unwrap();
        fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert_eq!(
            fs::read(paths.destination.join("nvram")).unwrap(),
            b"untrusted"
        );
        assert!(paths.source.exists());
    }

    #[test]
    fn replaced_destination_with_recorded_identity_is_quarantined_without_deletion() {
        let scratch = Scratch::new("destination-replaced");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"complete").unwrap();
        fs::create_dir_all(&paths.destination).unwrap();
        fs::write(paths.destination.join("nvram"), b"complete").unwrap();
        fs::set_permissions(&paths.destination, fs::Permissions::from_mode(0o700)).unwrap();

        let source_digest = digest_tree(&paths.source).unwrap();
        let destination_digest = digest_tree(&paths.destination).unwrap();
        let mut journal = LegacyMigrationJournal::new(
            source_digest,
            destination_digest,
            planned_marker_digest(source_digest, destination_digest),
        )
        .unwrap();
        let source_stat = fs::symlink_metadata(&paths.source).unwrap();
        let destination_stat = fs::symlink_metadata(&paths.destination).unwrap();
        journal.set_source_identity(LegacyFileIdentity {
            dev: source_stat.dev(),
            ino: source_stat.ino(),
            uid: source_stat.uid(),
            gid: source_stat.gid(),
            mode: source_stat.permissions().mode() & 0o7777,
        });
        journal.advance(LegacyMigrationPhase::Prepared).unwrap();
        journal.set_destination_identity(LegacyFileIdentity {
            dev: destination_stat.dev(),
            ino: destination_stat.ino(),
            uid: destination_stat.uid(),
            gid: destination_stat.gid(),
            mode: destination_stat.permissions().mode() & 0o7777,
        });
        let payload = exact_marker_payload(&paths, &journal).unwrap();
        journal.set_marker_digest(Sha256::digest(payload).into());
        journal
            .advance(LegacyMigrationPhase::PayloadStaged)
            .unwrap();
        fs::write(
            &paths.journal,
            serde_json::to_vec(&journal).expect("journal serializes"),
        )
        .unwrap();
        fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();

        let stash = paths.destination.with_file_name("swtpm-stash");
        fs::rename(&paths.destination, &stash).unwrap();
        fs::create_dir_all(&paths.destination).unwrap();
        fs::write(paths.destination.join("nvram"), b"replacement").unwrap();

        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert_eq!(
            fs::read(paths.destination.join("nvram")).unwrap(),
            b"replacement"
        );
        assert!(paths.source.exists());
        assert!(stash.exists());
    }

    #[test]
    fn later_phase_journal_missing_identity_is_rejected() {
        let scratch = Scratch::new("missing-identity");
        let paths = scratch.paths();
        fs::create_dir_all(&paths.source).unwrap();
        fs::write(paths.source.join("nvram"), b"legacy").unwrap();
        let source_digest = digest_tree(&paths.source).unwrap();
        let mut journal = LegacyMigrationJournal::new(
            source_digest,
            source_digest,
            planned_marker_digest(source_digest, source_digest),
        )
        .unwrap();
        let source_stat = fs::symlink_metadata(&paths.source).unwrap();
        journal.set_source_identity(LegacyFileIdentity {
            dev: source_stat.dev(),
            ino: source_stat.ino(),
            uid: source_stat.uid(),
            gid: source_stat.gid(),
            mode: source_stat.permissions().mode() & 0o7777,
        });
        journal.advance(LegacyMigrationPhase::Prepared).unwrap();
        let mut value = serde_json::to_value(&journal).unwrap();
        value["phase"] = serde_json::Value::String("committed".to_owned());
        value
            .as_object_mut()
            .expect("journal object")
            .remove("destinationIdentity");
        fs::write(&paths.journal, serde_json::to_vec(&value).unwrap()).unwrap();
        fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(migrate(&paths).unwrap(), LegacyMigrationOutcome::Ambiguous);
        assert!(paths.source.exists());
        assert!(!paths.destination.exists());
    }
}
