//! Broker-owned Zone resource-store opening.
//!
//! The wire carries one [`ZoneStoreId`].  The broker resolves the signed
//! storage-row artifact, derives the fixed Zone state layout from trusted
//! bundle data, and walks that layout through anchored descriptors.  The
//! database descriptor remains the synchronization handle: it is opened
//! `O_CLOEXEC`, locked with an OFD lock, verified by `F_GETFD`, and is the
//! single descriptor returned to the Zone runtime.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use d2b_contracts::broker_wire::{OpenZoneStoreResponse, ZoneStoreDisposition};
use d2b_contracts::v3::storage::{
    ZoneStoreDescriptorPublicationRequirement, ZoneStoreFilesystemRequirement,
    ZoneStoreFsyncRequirement, ZoneStoreId, ZoneStoreLockingRequirement, ZoneStorePrincipal,
    ZoneStoreReplacementDetection, ZoneStoreReplacementPublicationRequirement, ZoneStoreStorageRow,
};
use d2b_core::bundle_resolver::BundleResolver;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::libc;
use nix::unistd::{Gid, Group, Uid, User, fchown as nix_fchown};
use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, ResolveFlags, fchmod, fstat, fsync, mkdirat, open, openat,
    openat2, renameat, statat,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const STATE_ROOT_STORAGE_ID: &str = "path:state-root";
const ZONE_STORE_PREFIX: &str = "zone-store-";
const ZONE_STORE_PARENT_PREFIX: &str = "zone-store-parent-";
const ZONE_STORE_MARKER_PREFIX: &str = "zone-store-marker-";
const DATABASE_NAME: &str = "store.redb";
const MARKER_NAME: &str = ".d2b-store-marker";
const MARKER_VERSION: u32 = 1;
const PARENT_MODE: u32 = 0o750;
const MARKER_MODE: u32 = 0o640;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneStoreError {
    SignedRowMissing,
    SignedRowMismatch,
    InvalidStorageRow(&'static str),
    PathSafetyViolation(&'static str),
    DatabaseMissing,
    DatabaseReplaced,
    DatabaseCorrupt(&'static str),
    MarkerMissing,
    MarkerCorrupt,
    MarkerMismatch(&'static str),
    PrincipalUnresolved,
    LockUnavailable,
    Io(&'static str),
    CloexecMissing,
}

impl std::fmt::Display for ZoneStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::SignedRowMissing => "signed-storage-row-missing",
            Self::SignedRowMismatch => "signed-storage-row-mismatch",
            Self::InvalidStorageRow(reason) => reason,
            Self::PathSafetyViolation(reason) => reason,
            Self::DatabaseMissing => "database-missing-after-provision",
            Self::DatabaseReplaced => "database-replaced",
            Self::DatabaseCorrupt(reason) => reason,
            Self::MarkerMissing => "identity-marker-missing",
            Self::MarkerCorrupt => "identity-marker-corrupt",
            Self::MarkerMismatch(reason) => reason,
            Self::PrincipalUnresolved => "storage-principal-unresolved",
            Self::LockUnavailable => "ofd-lock-unavailable",
            Self::Io(reason) => reason,
            Self::CloexecMissing => "database-fd-cloexec-missing",
        };
        write!(formatter, "OpenZoneStore: {reason}")
    }
}

impl std::error::Error for ZoneStoreError {}

#[derive(Debug)]
pub struct ZoneStoreOutcome {
    pub response: OpenZoneStoreResponse,
    pub database_fd: OwnedFd,
}

#[derive(Debug, Clone)]
struct ResolvedZoneStoreRow {
    zone_store_id: ZoneStoreId,
    parent_directory: PathBuf,
    database_name: &'static str,
    marker_name: &'static str,
    identity_marker_id: String,
    owner_uid: u32,
    group_gid: u32,
    mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoreIdentityMarker {
    marker_version: u32,
    zone_store_id: String,
    identity_marker_id: String,
    store_identity: String,
    device: u64,
    inode: u64,
    owner_uid: u32,
    group_gid: u32,
    mode: u32,
    link_count: u64,
}

#[derive(Debug, Clone, Copy)]
struct DatabasePosture {
    device: u64,
    inode: u64,
    owner_uid: u32,
    group_gid: u32,
    mode: u32,
    link_count: u64,
}

/// Resolve and open the trusted Zone store named by `zone_store_id`.
///
/// The row is loaded from the broker's trusted bundle directory.  The
/// request id is compared with the row's signed `zoneStoreId`, and the
/// parent/marker ids are bound to the same Zone before any state path is
/// opened.
pub fn open_zone_store(
    resolver: &BundleResolver,
    zone_store_id: &ZoneStoreId,
) -> Result<ZoneStoreOutcome, ZoneStoreError> {
    let row = resolve_signed_row(resolver, zone_store_id)?;
    open_resolved_row(&row)
}

fn resolve_signed_row(
    resolver: &BundleResolver,
    requested_id: &ZoneStoreId,
) -> Result<ResolvedZoneStoreRow, ZoneStoreError> {
    let zone = zone_from_id(requested_id.as_str())?;
    let bundle_root = Path::new(&resolver.bundle.host_path)
        .parent()
        .ok_or(ZoneStoreError::SignedRowMissing)?;
    if !bundle_root.is_absolute() || has_parent_component(bundle_root) {
        return Err(ZoneStoreError::PathSafetyViolation(
            "trusted-bundle-root-invalid",
        ));
    }

    let row_path = bundle_root.join("zones").join(zone).join("storage.json");
    let bytes = read_anchored_file(&row_path)?;
    let row: ZoneStoreStorageRow =
        serde_json::from_slice(&bytes).map_err(|_| ZoneStoreError::SignedRowMismatch)?;
    if row.zone_store_id.as_str() != requested_id.as_str() {
        return Err(ZoneStoreError::SignedRowMismatch);
    }
    validate_row_binding(&row, zone)?;

    let owner_uid = resolve_user(&row.ownership.owner)?;
    let group_gid = resolve_group(&row.ownership.group)?;
    let mode = parse_mode(row.ownership.mode.as_str())?;

    let state_root = resolver
        .find_storage_path_spec(STATE_ROOT_STORAGE_ID)
        .map(|spec| PathBuf::from(spec.path_template.as_str()))
        .unwrap_or_else(|| PathBuf::from("/var/lib/d2b"));
    if !state_root.is_absolute()
        || has_parent_component(&state_root)
        || state_root
            .components()
            .any(|component| matches!(component, Component::CurDir))
    {
        return Err(ZoneStoreError::PathSafetyViolation(
            "state-root-not-anchorable",
        ));
    }

    Ok(ResolvedZoneStoreRow {
        zone_store_id: requested_id.clone(),
        parent_directory: state_root.join("zones").join(zone),
        database_name: DATABASE_NAME,
        marker_name: MARKER_NAME,
        identity_marker_id: row.marker.identity_marker_id.as_str().to_owned(),
        owner_uid,
        group_gid,
        mode,
    })
}

fn validate_row_binding(row: &ZoneStoreStorageRow, zone: &str) -> Result<(), ZoneStoreError> {
    let expected_parent = format!("{ZONE_STORE_PARENT_PREFIX}{zone}");
    let expected_marker = format!("{ZONE_STORE_MARKER_PREFIX}{zone}");
    if row.parent_directory_id.as_str() != expected_parent
        || row.marker.identity_marker_id.as_str() != expected_marker
        || row.storage_owner_principal.as_str() != row.ownership.owner.as_str()
    {
        return Err(ZoneStoreError::SignedRowMismatch);
    }
    if row.filesystem != ZoneStoreFilesystemRequirement::RegularFileAnchoredFdRelativeNoFollow {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-filesystem-invariant",
        ));
    }
    if row.locking != ZoneStoreLockingRequirement::OfdCloseOnExec {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-locking-invariant",
        ));
    }
    if row.replacement_detection
        != ZoneStoreReplacementDetection::FailClosedOnMissingReplacedOrIdentityMismatch
    {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-replacement-invariant",
        ));
    }
    if row.fsync != ZoneStoreFsyncRequirement::DatabaseAndParentDirectory {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-fsync-invariant",
        ));
    }
    if row.publication.descriptor
        != ZoneStoreDescriptorPublicationRequirement::OwnedDescriptorCloseOnExecVerifiedBeforeConcurrency
        || row.publication.replacement
            != ZoneStoreReplacementPublicationRequirement::AtomicRenameRetainPriorQuarantineAmbiguity
    {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-publication-invariant",
        ));
    }
    if row.ownership.link_count.get() != 1 {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-link-count-invariant",
        ));
    }
    Ok(())
}

fn open_resolved_row(row: &ResolvedZoneStoreRow) -> Result<ZoneStoreOutcome, ZoneStoreError> {
    let guard = open_lock()
        .lock()
        .map_err(|_| ZoneStoreError::LockUnavailable)?;

    // The state root is NixOS-owned.  It may not be silently recreated by a
    // broker operation; only the declared `zones/<zone>` descendants are
    // provisioned here.
    let state_root = row
        .parent_directory
        .parent()
        .and_then(Path::parent)
        .ok_or(ZoneStoreError::PathSafetyViolation("state-root-missing"))?;
    let _state_root_fd = open_anchored_directory(state_root, false)?;
    let parent_fd = open_anchored_directory(&row.parent_directory, true)?;
    let marker_present = marker_exists(&parent_fd, row.marker_name)?;
    let database_present = database_exists(&parent_fd, row.database_name)?;

    if database_present != marker_present {
        return Err(if database_present {
            ZoneStoreError::MarkerMissing
        } else {
            ZoneStoreError::DatabaseMissing
        });
    }

    let (database_fd, disposition) = if database_present {
        let database_fd = open_database(&parent_fd, row.database_name, false)?;
        verify_database_posture(&database_fd, row)?;
        (database_fd, ZoneStoreDisposition::Opened)
    } else {
        let database_fd = open_database(&parent_fd, row.database_name, true)?;
        prepare_new_database(&database_fd, row)?;
        (database_fd, ZoneStoreDisposition::Provisioned)
    };

    lock_database(&database_fd)?;
    let posture = verify_database_posture(&database_fd, row)?;
    let marker = if disposition == ZoneStoreDisposition::Provisioned {
        let marker = new_marker(row, posture)?;
        write_marker(&parent_fd, row, &marker)?;
        marker
    } else {
        read_marker(&parent_fd, row)?
    };
    validate_marker(&marker, row, posture)?;

    fsync(&database_fd).map_err(|_| ZoneStoreError::Io("database-fsync-failed"))?;
    fsync(&parent_fd).map_err(|_| ZoneStoreError::Io("parent-directory-fsync-failed"))?;
    verify_cloexec(&database_fd)?;

    drop(guard);
    Ok(ZoneStoreOutcome {
        response: OpenZoneStoreResponse {
            zone_store_id: row.zone_store_id.clone(),
            store_identity: marker.store_identity,
            disposition,
            fd_index: 0,
        },
        database_fd,
    })
}

fn open_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn open_database(parent: &OwnedFd, name: &str, create: bool) -> Result<OwnedFd, ZoneStoreError> {
    // The parent descriptor was walked from `/` with O_NOFOLLOW and the leaf
    // name is a single validated component. `openat(O_NOFOLLOW|O_CLOEXEC)`
    // is the stable fd-relative fallback for the regular-file leaf; it does
    // not re-resolve any caller path.
    let mut flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    if create {
        flags |= OFlags::CREATE | OFlags::EXCL;
    }
    openat(parent, name, flags, Mode::from_raw_mode(0o640)).map_err(|error| {
        if error == rustix::io::Errno::LOOP {
            ZoneStoreError::PathSafetyViolation("database-symlink-refused")
        } else if error == rustix::io::Errno::NOENT {
            ZoneStoreError::DatabaseMissing
        } else if error == rustix::io::Errno::EXIST {
            ZoneStoreError::DatabaseReplaced
        } else {
            ZoneStoreError::Io(if create {
                "database-create-failed"
            } else {
                "database-open-failed"
            })
        }
    })
}

fn prepare_new_database(
    database_fd: &OwnedFd,
    row: &ResolvedZoneStoreRow,
) -> Result<(), ZoneStoreError> {
    fchmod(database_fd, Mode::from_raw_mode(row.mode))
        .map_err(|_| ZoneStoreError::Io("database-chmod-failed"))?;
    nix_fchown(
        database_fd.as_raw_fd(),
        Some(Uid::from_raw(row.owner_uid)),
        Some(Gid::from_raw(row.group_gid)),
    )
    .map_err(|_| ZoneStoreError::Io("database-chown-failed"))?;
    Ok(())
}

fn verify_database_posture(
    database_fd: &OwnedFd,
    row: &ResolvedZoneStoreRow,
) -> Result<DatabasePosture, ZoneStoreError> {
    let stat =
        fstat(database_fd).map_err(|_| ZoneStoreError::DatabaseCorrupt("database-fstat-failed"))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(ZoneStoreError::DatabaseCorrupt("database-not-regular-file"));
    }
    let mode = (stat.st_mode as u32) & 0o777;
    let posture = DatabasePosture {
        device: stat.st_dev,
        inode: stat.st_ino,
        owner_uid: stat.st_uid,
        group_gid: stat.st_gid,
        mode,
        link_count: stat.st_nlink,
    };
    if posture.link_count != 1 {
        return Err(ZoneStoreError::DatabaseCorrupt(
            "database-link-count-mismatch",
        ));
    }
    if posture.owner_uid != row.owner_uid || posture.group_gid != row.group_gid {
        return Err(ZoneStoreError::DatabaseCorrupt("database-owner-mismatch"));
    }
    if posture.mode != row.mode {
        return Err(ZoneStoreError::DatabaseCorrupt("database-mode-mismatch"));
    }
    Ok(posture)
}

fn lock_database(database_fd: &OwnedFd) -> Result<(), ZoneStoreError> {
    let lock = libc::flock {
        l_type: libc::F_WRLCK as _,
        l_whence: libc::SEEK_SET as _,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(database_fd.as_raw_fd(), FcntlArg::F_OFD_SETLKW(&lock))
        .map(|_| ())
        .map_err(|_| ZoneStoreError::LockUnavailable)
}

fn verify_cloexec(database_fd: &OwnedFd) -> Result<(), ZoneStoreError> {
    let flags = fcntl(database_fd.as_raw_fd(), FcntlArg::F_GETFD)
        .map_err(|_| ZoneStoreError::CloexecMissing)?;
    if !FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC) {
        return Err(ZoneStoreError::CloexecMissing);
    }
    Ok(())
}

fn marker_exists(parent: &OwnedFd, name: &str) -> Result<bool, ZoneStoreError> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                return Err(ZoneStoreError::MarkerCorrupt);
            }
            if stat.st_nlink != 1 {
                return Err(ZoneStoreError::MarkerCorrupt);
            }
            Ok(true)
        }
        Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
        Err(error) if error == rustix::io::Errno::LOOP => Err(ZoneStoreError::PathSafetyViolation(
            "identity-marker-symlink-refused",
        )),
        Err(_) => Err(ZoneStoreError::Io("identity-marker-stat-failed")),
    }
}

fn database_exists(parent: &OwnedFd, name: &str) -> Result<bool, ZoneStoreError> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                return Err(ZoneStoreError::DatabaseCorrupt("database-not-regular-file"));
            }
            Ok(true)
        }
        Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
        Err(error) if error == rustix::io::Errno::LOOP => Err(ZoneStoreError::PathSafetyViolation(
            "database-symlink-refused",
        )),
        Err(_) => Err(ZoneStoreError::Io("database-stat-failed")),
    }
}

fn new_marker(
    row: &ResolvedZoneStoreRow,
    posture: DatabasePosture,
) -> Result<StoreIdentityMarker, ZoneStoreError> {
    let entropy = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_owned());
    let mut digest = Sha256::new();
    digest.update(row.zone_store_id.as_str().as_bytes());
    digest.update(row.identity_marker_id.as_bytes());
    digest.update(posture.device.to_le_bytes());
    digest.update(posture.inode.to_le_bytes());
    digest.update(entropy.as_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    let store_identity = format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    Ok(StoreIdentityMarker {
        marker_version: MARKER_VERSION,
        zone_store_id: row.zone_store_id.as_str().to_owned(),
        identity_marker_id: row.identity_marker_id.clone(),
        store_identity,
        device: posture.device,
        inode: posture.inode,
        owner_uid: posture.owner_uid,
        group_gid: posture.group_gid,
        mode: posture.mode,
        link_count: posture.link_count,
    })
}

fn write_marker(
    parent: &OwnedFd,
    row: &ResolvedZoneStoreRow,
    marker: &StoreIdentityMarker,
) -> Result<(), ZoneStoreError> {
    let body = serde_json::to_vec(marker).map_err(|_| ZoneStoreError::MarkerCorrupt)?;
    let temp_name = format!(
        ".d2b-store-marker.tmp-{}-{}",
        std::process::id(),
        marker.inode
    );
    let resolve = ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS;
    let temp_fd = openat2(
        parent,
        temp_name.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(MARKER_MODE),
        resolve,
    )
    .map_err(|_| ZoneStoreError::Io("identity-marker-temp-create-failed"))?;
    fchmod(&temp_fd, Mode::from_raw_mode(MARKER_MODE))
        .map_err(|_| ZoneStoreError::Io("identity-marker-chmod-failed"))?;
    nix_fchown(
        temp_fd.as_raw_fd(),
        Some(Uid::from_raw(row.owner_uid)),
        Some(Gid::from_raw(row.group_gid)),
    )
    .map_err(|_| ZoneStoreError::Io("identity-marker-chown-failed"))?;
    let mut file = File::from(temp_fd);
    file.write_all(&body)
        .map_err(|_| ZoneStoreError::Io("identity-marker-write-failed"))?;
    file.sync_all()
        .map_err(|_| ZoneStoreError::Io("identity-marker-fsync-failed"))?;
    drop(file);
    renameat(parent, temp_name.as_str(), parent, row.marker_name)
        .map_err(|_| ZoneStoreError::Io("identity-marker-publish-failed"))?;
    fsync(parent).map_err(|_| ZoneStoreError::Io("parent-directory-fsync-failed"))?;
    Ok(())
}

fn read_marker(
    parent: &OwnedFd,
    row: &ResolvedZoneStoreRow,
) -> Result<StoreIdentityMarker, ZoneStoreError> {
    let fd = openat2(
        parent,
        row.marker_name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        if error == rustix::io::Errno::LOOP {
            ZoneStoreError::PathSafetyViolation("identity-marker-symlink-refused")
        } else if error == rustix::io::Errno::NOENT {
            ZoneStoreError::MarkerMissing
        } else {
            ZoneStoreError::Io("identity-marker-open-failed")
        }
    })?;
    let stat = fstat(&fd).map_err(|_| ZoneStoreError::MarkerCorrupt)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(ZoneStoreError::MarkerCorrupt);
    }
    let marker_mode = (stat.st_mode as u32) & 0o777;
    if stat.st_uid != row.owner_uid || stat.st_gid != row.group_gid || marker_mode != MARKER_MODE {
        return Err(ZoneStoreError::MarkerMismatch(
            "identity-marker-posture-mismatch",
        ));
    }
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| ZoneStoreError::MarkerCorrupt)?;
    serde_json::from_slice(&bytes).map_err(|_| ZoneStoreError::MarkerCorrupt)
}

fn validate_marker(
    marker: &StoreIdentityMarker,
    row: &ResolvedZoneStoreRow,
    posture: DatabasePosture,
) -> Result<(), ZoneStoreError> {
    if marker.marker_version != MARKER_VERSION {
        return Err(ZoneStoreError::MarkerMismatch(
            "identity-marker-version-mismatch",
        ));
    }
    if marker.zone_store_id != row.zone_store_id.as_str() {
        return Err(ZoneStoreError::MarkerMismatch(
            "identity-marker-store-id-mismatch",
        ));
    }
    if marker.identity_marker_id != row.identity_marker_id {
        return Err(ZoneStoreError::MarkerMismatch(
            "identity-marker-id-mismatch",
        ));
    }
    if !is_store_identity(marker.store_identity.as_str()) {
        return Err(ZoneStoreError::MarkerCorrupt);
    }
    if marker.device != posture.device || marker.inode != posture.inode {
        return Err(ZoneStoreError::DatabaseReplaced);
    }
    if marker.owner_uid != posture.owner_uid
        || marker.group_gid != posture.group_gid
        || marker.mode != posture.mode
        || marker.link_count != posture.link_count
    {
        return Err(ZoneStoreError::MarkerMismatch(
            "identity-marker-posture-mismatch",
        ));
    }
    Ok(())
}

fn is_store_identity(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_anchored_file(path: &Path) -> Result<Vec<u8>, ZoneStoreError> {
    let parent = path.parent().ok_or(ZoneStoreError::PathSafetyViolation(
        "artifact-parent-missing",
    ))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ZoneStoreError::PathSafetyViolation("artifact-name-invalid"))?;
    let parent_fd = open_anchored_directory(parent, false)?;
    let fd = openat2(
        &parent_fd,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        if error == rustix::io::Errno::LOOP {
            ZoneStoreError::PathSafetyViolation("signed-row-symlink-refused")
        } else {
            ZoneStoreError::SignedRowMissing
        }
    })?;
    let stat = fstat(&fd).map_err(|_| ZoneStoreError::SignedRowMismatch)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_nlink != 1
        || (stat.st_mode as u32) & 0o002 != 0
    {
        return Err(ZoneStoreError::SignedRowMismatch);
    }
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| ZoneStoreError::SignedRowMismatch)?;
    Ok(bytes)
}

fn open_anchored_directory(path: &Path, create_missing: bool) -> Result<OwnedFd, ZoneStoreError> {
    let names = path_components(path)?;
    let mut current = open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ZoneStoreError::PathSafetyViolation("anchor-root-open-failed"))?;
    for name in names {
        current = if create_missing {
            open_or_create_directory(&current, &name)?
        } else {
            openat(
                &current,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                if error == rustix::io::Errno::LOOP {
                    ZoneStoreError::PathSafetyViolation("ancestor-symlink-refused")
                } else {
                    ZoneStoreError::PathSafetyViolation("ancestor-open-failed")
                }
            })?
        };
    }
    Ok(current)
}

fn open_or_create_directory(parent: &OwnedFd, name: &str) -> Result<OwnedFd, ZoneStoreError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match openat(parent, name, flags, Mode::empty()) {
        Ok(fd) => Ok(fd),
        Err(error) if error == rustix::io::Errno::NOENT => {
            mkdirat(parent, name, Mode::from_raw_mode(PARENT_MODE))
                .or_else(|error| {
                    if error == rustix::io::Errno::EXIST {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })
                .map_err(|_| ZoneStoreError::Io("parent-directory-create-failed"))?;
            fsync(parent).map_err(|_| ZoneStoreError::Io("parent-directory-fsync-failed"))?;
            openat(parent, name, flags, Mode::empty()).map_err(|error| {
                if error == rustix::io::Errno::LOOP {
                    ZoneStoreError::PathSafetyViolation("ancestor-symlink-refused")
                } else {
                    ZoneStoreError::Io("parent-directory-open-failed")
                }
            })
        }
        Err(error) if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR => {
            Err(ZoneStoreError::PathSafetyViolation(
                "ancestor-symlink-refused",
            ))
        }
        Err(_) => Err(ZoneStoreError::Io("parent-directory-open-failed")),
    }
}

fn path_components(path: &Path) -> Result<Vec<String>, ZoneStoreError> {
    if !path.is_absolute() {
        return Err(ZoneStoreError::PathSafetyViolation("path-must-be-absolute"));
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(
                name.to_str()
                    .ok_or(ZoneStoreError::PathSafetyViolation(
                        "path-component-not-utf8",
                    ))?
                    .to_owned(),
            ),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(ZoneStoreError::PathSafetyViolation(
                    "path-component-not-anchorable",
                ));
            }
        }
    }
    if names.is_empty() {
        return Err(ZoneStoreError::PathSafetyViolation("path-resolves-to-root"));
    }
    Ok(names)
}

fn has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn zone_from_id(id: &str) -> Result<&str, ZoneStoreError> {
    let zone = id
        .strip_prefix(ZONE_STORE_PREFIX)
        .filter(|zone| !zone.is_empty())
        .ok_or(ZoneStoreError::SignedRowMismatch)?;
    if !zone
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || zone.starts_with('-')
        || zone.ends_with('-')
    {
        return Err(ZoneStoreError::SignedRowMismatch);
    }
    Ok(zone)
}

fn parse_mode(mode: &str) -> Result<u32, ZoneStoreError> {
    if mode.len() != 4 || !mode.starts_with('0') {
        return Err(ZoneStoreError::InvalidStorageRow(
            "storage-row-mode-invariant",
        ));
    }
    u32::from_str_radix(&mode[1..], 8)
        .map_err(|_| ZoneStoreError::InvalidStorageRow("storage-row-mode-invariant"))
}

fn resolve_user(principal: &ZoneStorePrincipal) -> Result<u32, ZoneStoreError> {
    User::from_name(principal.as_str())
        .map_err(|_| ZoneStoreError::PrincipalUnresolved)?
        .map(|user| user.uid.as_raw())
        .ok_or(ZoneStoreError::PrincipalUnresolved)
}

fn resolve_group(principal: &ZoneStorePrincipal) -> Result<u32, ZoneStoreError> {
    Group::from_name(principal.as_str())
        .map_err(|_| ZoneStoreError::PrincipalUnresolved)?
        .map(|group| group.gid.as_raw())
        .ok_or(ZoneStoreError::PrincipalUnresolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::{Gid as NixGid, Uid as NixUid};
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::process::Command;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, ResolvedZoneStoreRow) {
        let temp = tempfile::Builder::new()
            .prefix("d2b-zone-store-")
            .tempdir()
            .expect("tempdir");
        let parent = temp.path().join("zones").join("local-root");
        fs::create_dir_all(&parent).expect("zone parent");
        let owner = NixUid::current().as_raw();
        let group = NixGid::current().as_raw();
        let id = ZoneStoreId::parse("zone-store-local-root").expect("id");
        (
            temp,
            ResolvedZoneStoreRow {
                zone_store_id: id,
                parent_directory: parent,
                database_name: DATABASE_NAME,
                marker_name: MARKER_NAME,
                identity_marker_id: "zone-store-marker-local-root".to_owned(),
                owner_uid: owner,
                group_gid: group,
                mode: 0o640,
            },
        )
    }

    fn signed_row() -> ZoneStoreStorageRow {
        serde_json::from_value(serde_json::json!({
            "zoneStoreId": "zone-store-local-root",
            "storageOwnerPrincipal": "d2b-zonert",
            "parentDirectoryId": "zone-store-parent-local-root",
            "ownership": {
                "owner": "d2b-zonert",
                "group": "d2b-zonert",
                "mode": "0640",
                "linkCount": 1
            },
            "filesystem": "regular-file-anchored-fd-relative-no-follow",
            "locking": "ofd-close-on-exec",
            "marker": {
                "identityMarkerId": "zone-store-marker-local-root"
            },
            "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
            "fsync": "database-and-parent-directory",
            "publication": {
                "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
                "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
            }
        }))
        .expect("signed row")
    }

    #[test]
    fn signed_row_parent_and_marker_mismatch_is_rejected() {
        let mut row = signed_row();
        assert_eq!(validate_row_binding(&row, "local-root"), Ok(()));
        row.parent_directory_id = d2b_contracts::v3::storage::ZoneStoreParentDirectoryId::parse(
            "zone-store-parent-other",
        )
        .expect("parent id");
        assert_eq!(
            validate_row_binding(&row, "local-root"),
            Err(ZoneStoreError::SignedRowMismatch)
        );
    }

    #[test]
    fn provision_and_reopen_are_idempotent_and_owned() {
        let (_temp, row) = fixture();
        let first = open_resolved_row(&row).expect("provision");
        assert_eq!(
            first.response.disposition,
            ZoneStoreDisposition::Provisioned
        );
        assert_eq!(first.response.fd_index, 0);
        assert_eq!(
            fcntl(first.database_fd.as_raw_fd(), FcntlArg::F_GETFD).expect("F_GETFD")
                & FdFlag::FD_CLOEXEC.bits(),
            FdFlag::FD_CLOEXEC.bits()
        );
        let identity = first.response.store_identity.clone();
        let first_stat = fstat(&first.database_fd).expect("database stat");
        drop(first);

        let second = open_resolved_row(&row).expect("reopen");
        assert_eq!(second.response.disposition, ZoneStoreDisposition::Opened);
        assert_eq!(second.response.store_identity, identity);
        let second_stat = fstat(&second.database_fd).expect("database stat");
        assert_eq!(first_stat.st_dev, second_stat.st_dev);
        assert_eq!(first_stat.st_ino, second_stat.st_ino);
        drop(second);
    }

    #[test]
    fn missing_or_corrupt_marker_fails_closed() {
        let (temp, row) = fixture();
        let opened = open_resolved_row(&row).expect("provision");
        drop(opened);
        fs::remove_file(temp.path().join("zones/local-root").join(MARKER_NAME))
            .expect("remove marker");
        assert_eq!(
            open_resolved_row(&row).expect_err("missing marker"),
            ZoneStoreError::MarkerMissing
        );

        let _ = open_resolved_row(&row);
        let marker_path = temp.path().join("zones/local-root").join(MARKER_NAME);
        fs::write(&marker_path, b"{\"markerVersion\":1}").expect("corrupt marker");
        fs::set_permissions(&marker_path, fs::Permissions::from_mode(0o640))
            .expect("corrupt marker");
        assert_eq!(
            open_resolved_row(&row).expect_err("corrupt marker"),
            ZoneStoreError::MarkerCorrupt
        );
    }

    #[test]
    fn replaced_database_is_rejected_by_inode_identity() {
        let (temp, row) = fixture();
        let opened = open_resolved_row(&row).expect("provision");
        drop(opened);
        let parent = temp.path().join("zones/local-root");
        let replacement = parent.join("replacement");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o640)
            .open(&replacement)
            .expect("replacement");
        drop(file);
        fs::rename(replacement, parent.join(DATABASE_NAME)).expect("replace database");
        assert_eq!(
            open_resolved_row(&row).expect_err("replacement must fail"),
            ZoneStoreError::DatabaseReplaced
        );
    }

    #[test]
    fn anchored_walk_rejects_path_injection_and_symlink_ancestors() {
        let (_temp, mut row) = fixture();
        row.parent_directory = row.parent_directory.join("..").join("escape");
        assert_eq!(
            open_resolved_row(&row).expect_err("parent traversal"),
            ZoneStoreError::PathSafetyViolation("path-component-not-anchorable")
        );

        let (temp, row) = fixture();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).expect("outside");
        let link = temp.path().join("zones-link");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");
        let mut linked = row;
        linked.parent_directory = link.join("local-root");
        assert_eq!(
            open_resolved_row(&linked).expect_err("symlink ancestor"),
            ZoneStoreError::PathSafetyViolation("ancestor-symlink-refused")
        );
    }

    #[test]
    fn direct_database_fd_does_not_inherit_across_exec() {
        let (_temp, row) = fixture();
        let opened = open_resolved_row(&row).expect("provision");
        let fd = opened.database_fd.as_raw_fd();
        let database_stat = fstat(&opened.database_fd).expect("database stat");
        let mut child = Command::new("sleep").arg("2").spawn().expect("exec probe");
        let child_fd = format!("/proc/{}/fd/{fd}", child.id());
        let child_comm = format!("/proc/{}/comm", child.id());
        let observation_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut observed_exec = false;
        while std::time::Instant::now() < observation_deadline {
            if let Ok(comm) = fs::read_to_string(&child_comm)
                && comm.trim() == "sleep"
            {
                observed_exec = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !observed_exec {
            let _ = child.kill();
            let _ = child.wait();
            panic!("exec probe did not reach sleep before timeout");
        }
        let child_database_identity = match fs::metadata(&child_fd) {
            Ok(metadata) => Some((metadata.dev(), metadata.ino())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("inspect child database fd: {error}"),
        };
        let _ = child.kill();
        let _ = child.wait();
        assert_ne!(
            child_database_identity,
            Some((database_stat.st_dev, database_stat.st_ino)),
            "database object inherited across exec"
        );
    }
}
