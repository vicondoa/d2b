//! `DiskInit` broker op handler.
//!
//! Creates and pre-allocates a disk-image file for the per-VM
//! writable store overlay. The broker resolves every `DiskInit`
//! plan-op from the trusted bundle's `ProcessNode.plan_ops` for
//! the supplied `vm_id` and executes them in order.
//!
//! Security invariants:
//! - The target path MUST be absolute and under
//!   `/var/lib/nixling/vms/` (path-safety check).
//! - No `..` or symlink path components are permitted when opening or
//!   creating the held fd.
//! - The caller never supplies raw paths, sizes, modes, or
//!   ownership; all values come from the trusted bundle.
//!
//! Idempotency: when `if_absent = true` and the file already
//! exists, the broker validates that it is the expected nixling-owned
//! ext4 image before skipping. Empty sparse images may be formatted in
//! place; malformed or non-empty unknown data fails closed.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

use nixling_core::bundle_resolver::{BundleResolver, ResolvedDiskInitOp};

/// Permitted root prefix for all disk-init target paths.
const PERMITTED_ROOT: &str = "/var/lib/nixling/vms/";
const EXT4_MAGIC_OFFSET: u64 = 1024 + 0x38;
const EXT4_SUPER_MAGIC: u16 = 0xEF53;
const MKFS_STDERR_LIMIT: usize = 512;

/// Validate `mkfs.ext4` binary path resolution from
/// `NIXLING_BROKER_MKFS_EXT4_BINARY` (default
/// `/run/current-system/sw/bin/mkfs.ext4`). Rejects non-absolute paths
/// and any `..` segments to prevent env-var injection from steering the
/// root-running broker to an attacker-chosen binary.
fn validate_mkfs_ext4_binary(raw: &str) -> io::Result<&std::path::Path> {
    let p = std::path::Path::new(raw);
    if !p.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("disk-init: NIXLING_BROKER_MKFS_EXT4_BINARY must be absolute (got {raw:?})"),
        ));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "disk-init: NIXLING_BROKER_MKFS_EXT4_BINARY may not contain `..` (got {raw:?})"
            ),
        ));
    }
    Ok(p)
}

#[derive(Debug, Clone)]
struct MkfsTool {
    raw: String,
    path: std::path::PathBuf,
}

fn resolve_mkfs_ext4_binary() -> io::Result<MkfsTool> {
    let raw = std::env::var("NIXLING_BROKER_MKFS_EXT4_BINARY")
        .unwrap_or_else(|_| "/run/current-system/sw/bin/mkfs.ext4".to_owned());
    let path = validate_mkfs_ext4_binary(&raw)?.to_path_buf();
    Ok(MkfsTool { raw, path })
}

/// Outcome of a single `disk_init_one` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskInitOutcome {
    /// File was created and pre-allocated.
    Created,
    /// File already existed and `if_absent = true`; skipped.
    Skipped,
    /// Existing empty image was safely formatted in place.
    Repaired,
    /// Existing ext4 image had stale uid/gid/mode posture repaired.
    PostureRepaired,
    /// Existing empty image had stale posture repaired before formatting.
    RepairedWithPosture,
}

/// Aggregate result of running all disk-init ops for a VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInitSummary {
    pub ops_total: u32,
    pub ops_created: u32,
    pub ops_skipped: u32,
    pub ops_repaired: u32,
    pub ops_posture_repaired: u32,
    /// Stable hash of all target paths for the audit record.
    pub target_paths_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiskInitErrorKind {
    UnexpectedPosture,
    UnsafeRepair,
    FormatterFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiskInitError {
    kind: DiskInitErrorKind,
    reason: String,
}

impl DiskInitError {
    fn unexpected(reason: impl Into<String>) -> Self {
        Self {
            kind: DiskInitErrorKind::UnexpectedPosture,
            reason: reason.into(),
        }
    }

    fn unsafe_repair(reason: impl Into<String>) -> Self {
        Self {
            kind: DiskInitErrorKind::UnsafeRepair,
            reason: reason.into(),
        }
    }

    fn formatter(reason: impl Into<String>) -> Self {
        Self {
            kind: DiskInitErrorKind::FormatterFailed,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for DiskInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = match self.kind {
            DiskInitErrorKind::UnexpectedPosture => {
                "inspect the file ownership, permissions, and size, then restore the declared nixling image posture or remove the conflicting file before retrying"
            }
            DiskInitErrorKind::UnsafeRepair => {
                "inspect and back up the file; remove or explicitly reformat it before retrying if it is safe to discard"
            }
            DiskInitErrorKind::FormatterFailed => {
                "inspect mkfs.ext4 availability and the target image, then retry after correcting the formatter failure"
            }
        };
        write!(f, "disk-init: {} ({action})", self.reason)
    }
}

impl std::error::Error for DiskInitError {}

impl From<DiskInitError> for io::Error {
    fn from(value: DiskInitError) -> Self {
        let kind = match value.kind {
            DiskInitErrorKind::UnexpectedPosture | DiskInitErrorKind::UnsafeRepair => {
                io::ErrorKind::InvalidData
            }
            DiskInitErrorKind::FormatterFailed => io::ErrorKind::Other,
        };
        io::Error::new(kind, value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyImageEvidence {
    NoDataExtentsAfterSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataExtentClassification {
    Empty(EmptyImageEvidence),
    HasData,
    UnknownUnsupported(rustix::io::Errno),
}

fn safe_parent_and_name(path: &Path) -> io::Result<(OwnedFd, String)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("disk-init: target {} has no parent", path.display()),
        )
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("disk-init: target {} has no basename", path.display()),
            )
        })?
        .to_owned();
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: open parent directory {} safely: {e}",
                parent.display()
            ),
        )
    })?;
    Ok((parent_fd, name))
}

fn reopen_nofollow_rw(path: &Path) -> io::Result<File> {
    let (parent_fd, name) = safe_parent_and_name(path)?;
    let fd = crate::sys::path_safe::open_file_at_safe(
        &parent_fd,
        &name,
        nix::libc::O_RDWR | nix::libc::O_NONBLOCK,
    )?;
    Ok(File::from(fd))
}

fn apply_posture(file: &File, spec: &ResolvedDiskInitOp, phase: &str) -> io::Result<()> {
    crate::sys::path_safe::fchown(file.as_fd(), Some(spec.owner_uid), Some(spec.owner_gid))
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: {phase} fchown {}:{} on {}: {e}",
                    spec.owner_uid,
                    spec.owner_gid,
                    spec.target_path.display()
                ),
            )
        })?;
    crate::sys::path_safe::fchmod(file.as_fd(), spec.mode).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: {phase} fchmod {:04o} on {}: {e}",
                spec.mode,
                spec.target_path.display()
            ),
        )
    })?;
    Ok(())
}

fn quarantine_owner() -> (u32, u32) {
    #[cfg(test)]
    if nix::unistd::geteuid().as_raw() != 0 {
        return (
            nix::unistd::geteuid().as_raw(),
            nix::unistd::getegid().as_raw(),
        );
    }
    (0, 0)
}

fn apply_quarantine_posture(file: &File, spec: &ResolvedDiskInitOp, phase: &str) -> io::Result<()> {
    let (uid, gid) = quarantine_owner();
    crate::sys::path_safe::fchown(file.as_fd(), Some(uid), Some(gid)).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: {phase} quarantine fchown {uid}:{gid} on {}: {e}",
                spec.target_path.display()
            ),
        )
    })?;
    crate::sys::path_safe::fchmod(file.as_fd(), 0o600).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: {phase} quarantine fchmod 0600 on {}: {e}",
                spec.target_path.display()
            ),
        )
    })?;
    Ok(())
}

fn existing_metadata(
    file: &File,
    spec: &ResolvedDiskInitOp,
    phase: &str,
) -> Result<std::fs::Metadata, DiskInitError> {
    file.metadata().map_err(|e| {
        DiskInitError::unexpected(format!(
            "{phase} stat existing image {} failed: {e}",
            spec.target_path.display()
        ))
    })
}

fn validate_existing_type_and_link(
    meta: &std::fs::Metadata,
    spec: &ResolvedDiskInitOp,
) -> Result<(), DiskInitError> {
    if !meta.file_type().is_file() {
        let kind = if meta.file_type().is_symlink() {
            "symlink"
        } else if meta.file_type().is_dir() {
            "directory"
        } else if meta.file_type().is_block_device() {
            "block-device"
        } else {
            "non-regular"
        };
        return Err(DiskInitError::unexpected(format!(
            "existing image {} is {kind}, expected regular file",
            spec.target_path.display()
        )));
    }
    if meta.nlink() != 1 {
        return Err(DiskInitError::unsafe_repair(format!(
            "existing image {} link count {} is not safe to repair",
            spec.target_path.display(),
            meta.nlink()
        )));
    }
    Ok(())
}

fn validate_existing_size(
    meta: &std::fs::Metadata,
    spec: &ResolvedDiskInitOp,
) -> Result<(), DiskInitError> {
    if meta.len() != spec.size_bytes {
        return Err(DiskInitError::unexpected(format!(
            "existing image {} size {} does not match declared size {}",
            spec.target_path.display(),
            meta.len(),
            spec.size_bytes
        )));
    }
    Ok(())
}

fn validate_existing_identity(
    meta: &std::fs::Metadata,
    spec: &ResolvedDiskInitOp,
) -> Result<(), DiskInitError> {
    validate_existing_type_and_link(meta, spec)?;
    validate_existing_size(meta, spec)
}

fn posture_matches(meta: &std::fs::Metadata, spec: &ResolvedDiskInitOp) -> bool {
    meta.uid() == spec.owner_uid
        && meta.gid() == spec.owner_gid
        && (meta.mode() & 0o777) == spec.mode
}

fn validate_existing_posture(
    meta: &std::fs::Metadata,
    spec: &ResolvedDiskInitOp,
) -> Result<(), DiskInitError> {
    if meta.uid() != spec.owner_uid || meta.gid() != spec.owner_gid {
        return Err(DiskInitError::unexpected(format!(
            "existing image {} owner {}:{} does not match declared owner {}:{}",
            spec.target_path.display(),
            meta.uid(),
            meta.gid(),
            spec.owner_uid,
            spec.owner_gid
        )));
    }
    let mode = meta.mode() & 0o777;
    if mode != spec.mode {
        return Err(DiskInitError::unexpected(format!(
            "existing image {} mode {:04o} does not match declared mode {:04o}",
            spec.target_path.display(),
            mode,
            spec.mode
        )));
    }
    Ok(())
}

fn validate_quarantine_posture(
    meta: &std::fs::Metadata,
    spec: &ResolvedDiskInitOp,
) -> Result<(), DiskInitError> {
    let (uid, gid) = quarantine_owner();
    if meta.uid() != uid || meta.gid() != gid || (meta.mode() & 0o777) != 0o600 {
        return Err(DiskInitError::unexpected(format!(
            "existing image {} quarantine posture {}:{} mode {:04o} does not match expected {}:{} mode 0600",
            spec.target_path.display(),
            meta.uid(),
            meta.gid(),
            meta.mode() & 0o777,
            uid,
            gid,
        )));
    }
    Ok(())
}

fn repair_existing_posture_if_needed(
    file: &File,
    spec: &ResolvedDiskInitOp,
    meta: &std::fs::Metadata,
) -> Result<bool, DiskInitError> {
    if posture_matches(meta, spec) {
        return Ok(false);
    }
    apply_posture(file, spec, "posture-repair").map_err(|e| {
        DiskInitError::unexpected(format!(
            "repair declared posture on existing image {} failed: {e}",
            spec.target_path.display()
        ))
    })?;
    let repaired = existing_metadata(file, spec, "posture-repair")?;
    validate_existing_identity(&repaired, spec)?;
    validate_existing_posture(&repaired, spec)?;
    Ok(true)
}

fn quarantine_existing_for_mkfs(
    file: &File,
    spec: &ResolvedDiskInitOp,
) -> Result<(), DiskInitError> {
    apply_quarantine_posture(file, spec, "pre-mkfs").map_err(|e| {
        DiskInitError::unexpected(format!(
            "quarantine existing image {} before mkfs failed: {e}",
            spec.target_path.display()
        ))
    })?;
    let quarantined = existing_metadata(file, spec, "pre-mkfs-quarantine")?;
    validate_existing_identity(&quarantined, spec)?;
    validate_quarantine_posture(&quarantined, spec)?;
    verify_path_still_names_fd(file, spec, "pre-mkfs-quarantine")?;
    Ok(())
}

fn verify_path_still_names_fd(
    file: &File,
    spec: &ResolvedDiskInitOp,
    phase: &str,
) -> Result<(), DiskInitError> {
    let held = existing_metadata(file, spec, phase)?;
    let (parent_fd, name) = safe_parent_and_name(&spec.target_path).map_err(|e| {
        DiskInitError::unexpected(format!(
            "{phase} open parent for declared image {} identity check failed: {e}",
            spec.target_path.display()
        ))
    })?;
    let current = crate::sys::path_safe::fstatat_nofollow(&parent_fd, &name)
        .map_err(|e| {
            DiskInitError::unexpected(format!(
                "{phase} stat declared image {} by parent fd failed: {e}",
                spec.target_path.display()
            ))
        })?
        .ok_or_else(|| {
            DiskInitError::unexpected(format!(
                "{phase} declared image {} disappeared before spawn",
                spec.target_path.display()
            ))
        })?;
    if held.dev() != current.st_dev as u64 || held.ino() != current.st_ino as u64 {
        return Err(DiskInitError::unsafe_repair(format!(
            "declared image {} no longer names the validated inode; automatic repair bypassed",
            spec.target_path.display()
        )));
    }
    Ok(())
}

fn read_ext4_magic(file: &File, path: &Path) -> io::Result<Option<u16>> {
    use std::os::unix::fs::FileExt;
    let len = file
        .metadata()
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: stat ext4 superblock candidate {}: {e}",
                    path.display()
                ),
            )
        })?
        .len();
    if len < EXT4_MAGIC_OFFSET + 2 {
        return Ok(None);
    }
    let mut magic = [0u8; 2];
    file.read_exact_at(&mut magic, EXT4_MAGIC_OFFSET)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: read ext4 superblock magic from {} at offset {}: {e}",
                    path.display(),
                    EXT4_MAGIC_OFFSET
                ),
            )
        })?;
    Ok(Some(u16::from_le_bytes(magic)))
}

fn has_ext4_superblock(file: &File, path: &Path) -> io::Result<bool> {
    Ok(read_ext4_magic(file, path)? == Some(EXT4_SUPER_MAGIC))
}

fn classify_data_extents(
    file: &File,
    spec: &ResolvedDiskInitOp,
) -> Result<DataExtentClassification, DiskInitError> {
    file.sync_data().map_err(|e| {
        DiskInitError::unsafe_repair(format!(
            "disk-init: sync existing image {} before sparse-empty classification: {e}",
            spec.target_path.display()
        ))
    })?;
    match rustix::fs::seek(file, rustix::fs::SeekFrom::Data(0)) {
        Ok(_) => Ok(DataExtentClassification::HasData),
        Err(err) if err == rustix::io::Errno::NXIO => Ok(DataExtentClassification::Empty(
            EmptyImageEvidence::NoDataExtentsAfterSync,
        )),
        Err(err) => Ok(DataExtentClassification::UnknownUnsupported(err)),
    }
}

fn lock_existing_image(file: &File, path: &Path) -> io::Result<nix::fcntl::Flock<std::fs::File>> {
    let lock_file = file.try_clone().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("disk-init: clone fd for lock {}: {e}", path.display()),
        )
    })?;
    nix::fcntl::Flock::lock(lock_file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| io::Error::other(format!("disk-init: lock {}: {e}", path.display())))
}

fn acquire_exclusive_lease(
    file: &File,
    spec: &ResolvedDiskInitOp,
) -> Result<crate::sys::path_safe::FileWriteLease, DiskInitError> {
    crate::sys::path_safe::acquire_write_lease(file.as_fd()).map_err(|e| {
        DiskInitError::unsafe_repair(format!(
            "existing image {} is not exclusively available for repair: {e}",
            spec.target_path.display()
        ))
    })
}

fn fd_target_path(file: &File) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

fn set_declared_len(file: &File, spec: &ResolvedDiskInitOp, phase: &str) -> io::Result<()> {
    file.set_len(spec.size_bytes).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: {phase} set length {} bytes on {}: {e}",
                spec.size_bytes,
                spec.target_path.display()
            ),
        )
    })
}

fn reserve_declared_blocks_after_mkfs(file: &File, spec: &ResolvedDiskInitOp) -> io::Result<()> {
    rustix::fs::fallocate(
        file.as_fd(),
        rustix::fs::FallocateFlags::empty(),
        0,
        spec.size_bytes,
    )
    .map_err(|e| {
        io::Error::other(format!(
            "disk-init: fallocate {} bytes on {} after mkfs: {e}",
            spec.size_bytes,
            spec.target_path.display()
        ))
    })?;
    if !has_ext4_superblock(file, &spec.target_path)? {
        return Err(DiskInitError::formatter(format!(
            "post-mkfs fallocate did not preserve a valid ext4 superblock on {}",
            spec.target_path.display()
        ))
        .into());
    }
    Ok(())
}

fn run_mkfs_ext4_on_fd_with(file: &File, display_path: &Path, tool: &MkfsTool) -> io::Result<()> {
    let target = fd_target_path(file);
    let mut last_spawn_error = None;
    let mut mkfs_output = None;
    let args: [&std::ffi::OsStr; 5] = [
        std::ffi::OsStr::new("-q"),
        std::ffi::OsStr::new("-F"),
        std::ffi::OsStr::new("-E"),
        std::ffi::OsStr::new("lazy_itable_init=1,lazy_journal_init=1"),
        target.as_os_str(),
    ];
    for attempt in 0..5 {
        match crate::sys::path_safe::command_output_inheriting_fd(&tool.path, &args, file.as_fd()) {
            Ok(output) => {
                mkfs_output = Some(output);
                break;
            }
            Err(error) if error.raw_os_error() == Some(nix::libc::ETXTBSY) && attempt < 4 => {
                last_spawn_error = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => {
                return Err(DiskInitError::formatter(format!(
                    "mkfs.ext4 ({}) spawn on {} failed: {error}",
                    tool.raw,
                    display_path.display()
                ))
                .into());
            }
        }
    }
    let mkfs_output = match mkfs_output {
        Some(output) => output,
        None => {
            let error = last_spawn_error.expect("ETXTBSY error recorded");
            return Err(DiskInitError::formatter(format!(
                "mkfs.ext4 ({}) spawn on {} failed: {error}",
                tool.raw,
                display_path.display()
            ))
            .into());
        }
    };
    if !mkfs_output.status.success() {
        let stderr = bounded_lossy(&mkfs_output.stderr, MKFS_STDERR_LIMIT);
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!("; stderr: {stderr}")
        };
        return Err(DiskInitError::formatter(format!(
            "mkfs.ext4 exit={:?} on {}{}",
            mkfs_output.status.code(),
            display_path.display(),
            detail
        ))
        .into());
    }
    Ok(())
}

fn bounded_lossy(bytes: &[u8], limit: usize) -> String {
    let truncated = bytes.len() > limit;
    let prefix = if truncated { &bytes[..limit] } else { bytes };
    let mut text = String::from_utf8_lossy(prefix).trim().to_owned();
    if truncated {
        text.push_str("...[truncated]");
    }
    text
}

fn create_and_format(spec: &ResolvedDiskInitOp) -> io::Result<DiskInitOutcome> {
    let tool = resolve_mkfs_ext4_binary()?;
    create_and_format_with(spec, &tool)
}

fn create_and_format_with(
    spec: &ResolvedDiskInitOp,
    tool: &MkfsTool,
) -> io::Result<DiskInitOutcome> {
    let (parent_fd, name) = safe_parent_and_name(&spec.target_path)?;
    let fd = crate::sys::path_safe::create_file_at_safe(
        &parent_fd,
        &name,
        nix::libc::O_RDWR | nix::libc::O_CREAT | nix::libc::O_EXCL,
        0o600,
    )
    .map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("disk-init: open {} safely: {e}", spec.target_path.display()),
        )
    })?;
    let file = File::from(fd);

    set_declared_len(&file, spec, "create")?;
    let _lock = lock_existing_image(&file, &spec.target_path)?;
    run_mkfs_ext4_on_fd_with(&file, &spec.target_path, tool)?;
    if !has_ext4_superblock(&file, &spec.target_path)? {
        return Err(DiskInitError::formatter(format!(
            "mkfs.ext4 did not leave a valid ext4 superblock on {}",
            spec.target_path.display()
        ))
        .into());
    }
    reserve_declared_blocks_after_mkfs(&file, spec)?;
    apply_posture(&file, spec, "post-mkfs")?;
    verify_path_still_names_fd(&file, spec, "post-mkfs")?;
    Ok(DiskInitOutcome::Created)
}

fn validate_or_repair_existing(spec: &ResolvedDiskInitOp) -> io::Result<DiskInitOutcome> {
    let tool = resolve_mkfs_ext4_binary()?;
    validate_or_repair_existing_with(spec, &tool)
}

fn validate_or_repair_existing_with(
    spec: &ResolvedDiskInitOp,
    tool: &MkfsTool,
) -> io::Result<DiskInitOutcome> {
    let file = reopen_nofollow_rw(&spec.target_path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: open existing path-safe O_NOFOLLOW O_NONBLOCK {}: {e}",
                spec.target_path.display()
            ),
        )
    })?;
    let meta = existing_metadata(&file, spec, "pre-repair")?;
    validate_existing_type_and_link(&meta, spec)?;
    let lease = acquire_exclusive_lease(&file, spec)?;
    let _lock = lock_existing_image(&file, &spec.target_path)?;
    let mut meta = existing_metadata(&file, spec, "post-lease")?;
    validate_existing_type_and_link(&meta, spec)?;
    if meta.len() == 0 && spec.size_bytes != 0 {
        set_declared_len(&file, spec, "repair-empty")?;
        meta = existing_metadata(&file, spec, "post-repair-empty-resize")?;
        validate_existing_type_and_link(&meta, spec)?;
    }
    validate_existing_size(&meta, spec)?;
    let has_ext4 = has_ext4_superblock(&file, &spec.target_path)?;
    let empty = if has_ext4 {
        false
    } else {
        match classify_data_extents(&file, spec)? {
            DataExtentClassification::Empty(_) => true,
            DataExtentClassification::HasData => {
                return Err(DiskInitError::unsafe_repair(format!(
                    "existing image {} has data but no ext4 superblock; automatic posture repair bypassed",
                    spec.target_path.display()
                ))
                .into());
            }
            DataExtentClassification::UnknownUnsupported(errno) => {
                return Err(DiskInitError::unsafe_repair(format!(
                    "could not prove existing image {} is empty using filesystem extent metadata after sync: {errno}; automatic posture repair bypassed",
                    spec.target_path.display()
                ))
                .into());
            }
        }
    };
    if has_ext4 {
        let posture_repaired = repair_existing_posture_if_needed(&file, spec, &meta)?;
        verify_path_still_names_fd(&file, spec, "post-validation")?;
        return Ok(if posture_repaired {
            DiskInitOutcome::PostureRepaired
        } else {
            DiskInitOutcome::Skipped
        });
    }
    if empty {
        let posture_repaired = !posture_matches(&meta, spec);
        quarantine_existing_for_mkfs(&file, spec)?;
        drop(lease);
        run_mkfs_ext4_on_fd_with(&file, &spec.target_path, tool)?;
        if !has_ext4_superblock(&file, &spec.target_path)? {
            return Err(DiskInitError::formatter(format!(
                "mkfs.ext4 did not leave a valid ext4 superblock on {}",
                spec.target_path.display()
            ))
            .into());
        }
        reserve_declared_blocks_after_mkfs(&file, spec)?;
        apply_posture(&file, spec, "post-repair-mkfs")?;
        verify_path_still_names_fd(&file, spec, "post-repair-mkfs")?;
        return Ok(if posture_repaired {
            DiskInitOutcome::RepairedWithPosture
        } else {
            DiskInitOutcome::Repaired
        });
    }
    unreachable!("non-ext4 image classification returned neither data nor empty")
}

/// Execute a single disk-init spec.
///
/// Validates the path, creates the file (`O_CREAT|O_EXCL`),
/// pre-allocates `size_bytes` via `fallocate`, sets owner + mode via
/// fd-based `fchown` + `fchmod`, and formats it as ext4. Existing
/// images are skipped or repaired only after locked fd-bound identity
/// and ext4/proven-empty validation.
///
/// Returns an `io::Error` on any failure.
pub fn disk_init_one(spec: &ResolvedDiskInitOp) -> io::Result<DiskInitOutcome> {
    validate_target_path(&spec.target_path)?;
    if spec.if_absent {
        match std::fs::symlink_metadata(&spec.target_path) {
            Ok(_) => return validate_or_repair_existing(spec),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(io::Error::new(
                    err.kind(),
                    format!(
                        "disk-init: stat target {} before create: {err}",
                        spec.target_path.display()
                    ),
                ));
            }
        }
    }
    create_and_format(spec)
}

/// Execute all `DiskInit` plan-ops for `vm_id` from the trusted bundle.
///
/// Called from `runtime::dispatch_request_with_backend` for
/// `BrokerRequest::DiskInit`. Returns a [`DiskInitSummary`] for
/// the audit record or the first I/O error encountered.
pub fn live_disk_init(resolver: &BundleResolver, vm_id: &str) -> io::Result<DiskInitSummary> {
    let ops = resolver.resolve_disk_init_ops(vm_id);
    let ops_total = ops.len() as u32;
    let mut ops_created = 0u32;
    let mut ops_skipped = 0u32;
    let mut ops_repaired = 0u32;
    let mut ops_posture_repaired = 0u32;
    let mut paths_concat = String::new();

    for op in &ops {
        paths_concat.push_str(&op.target_path.display().to_string());
        paths_concat.push('\n');
        match disk_init_one(op)? {
            DiskInitOutcome::Created => ops_created += 1,
            DiskInitOutcome::Skipped => ops_skipped += 1,
            DiskInitOutcome::Repaired => ops_repaired += 1,
            DiskInitOutcome::PostureRepaired => {
                ops_skipped += 1;
                ops_posture_repaired += 1;
            }
            DiskInitOutcome::RepairedWithPosture => {
                ops_repaired += 1;
                ops_posture_repaired += 1;
            }
        }
    }

    Ok(DiskInitSummary {
        ops_total,
        ops_created,
        ops_skipped,
        ops_repaired,
        ops_posture_repaired,
        target_paths_hash: super::hosts::stable_hash_str(&paths_concat),
    })
}

/// Validate `path` is absolute and under [`PERMITTED_ROOT`], with
/// no `..` components. Returns an `io::Error` on violation.
fn validate_target_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path-safety-violation: disk-init target must be absolute, got {}",
                path.display()
            ),
        ));
    }
    if !path
        .to_str()
        .map(|s| s.starts_with(PERMITTED_ROOT))
        .unwrap_or(false)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path-safety-violation: disk-init target must be under {PERMITTED_ROOT}, got {}",
                path.display()
            ),
        ));
    }
    // Reject any `..` component after the prefix check (defense in depth).
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "path-safety-violation: disk-init target contains '..': {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::bundle_resolver::ResolvedDiskInitOp;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    fn scratch_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nixling-disk-init-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_spec(path: PathBuf, size_bytes: u64, if_absent: bool) -> ResolvedDiskInitOp {
        ResolvedDiskInitOp {
            target_path: path,
            size_bytes,
            mode: 0o600,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            if_absent,
        }
    }

    fn fake_mkfs_tool(scratch: &Path) -> MkfsTool {
        let script = scratch.join("fake-mkfs-ext4");
        fs::write(
            &script,
            format!(
            "#!/bin/sh\nlast=\"\"\nfor arg in \"$@\"; do last=\"$arg\"; done\nprintf '\\123\\357' | dd of=\"$last\" bs=1 seek={} conv=notrunc >/dev/null 2>&1\n",
            EXT4_MAGIC_OFFSET
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        MkfsTool {
            raw: script.display().to_string(),
            path: script,
        }
    }

    fn failing_mkfs_tool(scratch: &Path, stderr: &str) -> MkfsTool {
        let script = scratch.join("failing-mkfs-ext4");
        fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s' {:?} >&2\nexit 9\n", stderr),
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        MkfsTool {
            raw: script.display().to_string(),
            path: script,
        }
    }

    fn create_regular_image(path: &Path, size: u64, mode: u32) -> File {
        use std::os::unix::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)
            .unwrap();
        file.set_len(size).unwrap();
        file
    }

    fn write_ext4_magic(file: &File) {
        use std::os::unix::fs::FileExt;
        file.write_all_at(&EXT4_SUPER_MAGIC.to_le_bytes(), EXT4_MAGIC_OFFSET)
            .unwrap();
    }

    fn retry_on_transient_lease_contention<T>(
        mut f: impl FnMut() -> io::Result<T>,
    ) -> io::Result<T> {
        let mut last_err = None;
        for _ in 0..20 {
            match f() {
                Ok(value) => return Ok(value),
                Err(err)
                    if err.kind() == io::ErrorKind::InvalidData
                        && err.to_string().contains("Resource temporarily unavailable") =>
                {
                    last_err = Some(err);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.expect("retry loop records transient lease error"))
    }

    #[test]
    fn create_and_format_creates_ext4_image_when_absent() {
        let scratch = scratch_root();
        let target = scratch
            .join("vms")
            .join("test-vm")
            .join("store-overlay.img");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let outcome = create_and_format_with(&spec, &tool).expect("create and format succeeds");
        assert_eq!(outcome, DiskInitOutcome::Created);

        assert!(target.exists(), "file must exist after creation");

        let meta = fs::metadata(&target).expect("stat file");
        // Size should be at least `size_bytes` (fallocate may round up).
        assert!(
            meta.len() >= 4096,
            "file length {} < requested 4096",
            meta.len()
        );
        // Mode check: mask with 0o777 to get permission bits.
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "file mode should be 0o600"
        );
        // Owner check (we ran as current euid so fchown is a no-op).
        assert_eq!(meta.uid(), nix::unistd::geteuid().as_raw());
        assert!(has_ext4_superblock(&fs::File::open(&target).unwrap(), &target).unwrap());

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_valid_ext4_image_skips() {
        let scratch = scratch_root();
        let target = scratch.join("store-overlay.img");
        let file = create_regular_image(&target, 4096, 0o600);
        write_ext4_magic(&file);
        drop(file);

        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);
        let outcome =
            retry_on_transient_lease_contention(|| validate_or_repair_existing_with(&spec, &tool))
                .expect("existing image validates");
        assert_eq!(outcome, DiskInitOutcome::Skipped);

        let meta = fs::metadata(&target).expect("stat file");
        assert_eq!(meta.len(), 4096);

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_sparse_unformatted_image_is_repaired() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o600);
        drop(file);
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let outcome =
            retry_on_transient_lease_contention(|| validate_or_repair_existing_with(&spec, &tool))
                .expect("sparse image repairs");
        assert_eq!(outcome, DiskInitOutcome::Repaired);
        assert!(has_ext4_superblock(&fs::File::open(&target).unwrap(), &target).unwrap());

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_zero_length_unformatted_image_is_resized_and_repaired() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 0, 0o600);
        drop(file);
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let outcome =
            retry_on_transient_lease_contention(|| validate_or_repair_existing_with(&spec, &tool))
                .expect("zero-length image repairs");
        assert_eq!(outcome, DiskInitOutcome::Repaired);
        let meta = fs::metadata(&target).expect("stat repaired image");
        assert_eq!(meta.len(), 4096);
        assert!(has_ext4_superblock(&fs::File::open(&target).unwrap(), &target).unwrap());

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_non_ext4_with_data_fails_closed() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        fs::write(&target, vec![0xAA; 4096]).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let err = validate_or_repair_existing_with(&spec, &tool)
            .expect_err("non-ext4 data must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let message = err.to_string();
        assert!(
            message.contains("has data but no ext4 superblock")
                || message.contains("automatic posture repair bypassed")
                || message.contains("is not exclusively available for repair"),
            "unexpected error: {message}"
        );
        assert!(err.to_string().contains("inspect and back up"));

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_ext4_wrong_mode_is_repaired() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o644);
        write_ext4_magic(&file);
        drop(file);
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        // Full `cargo test` runs broker tests concurrently; unrelated
        // inherited opens can make Linux leases transiently EAGAIN in the
        // shared test process. Production cannot skip the lease, and the
        // lease helper itself is still covered by integration through the
        // normal non-skipped paths.
        let outcome =
            retry_on_transient_lease_contention(|| validate_or_repair_existing_with(&spec, &tool))
                .expect("safe stale posture repairs automatically");
        assert_eq!(outcome, DiskInitOutcome::PostureRepaired);
        let mode = fs::metadata(&target).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_sparse_wrong_mode_repairs_posture_then_formats() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o644);
        drop(file);
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let outcome =
            retry_on_transient_lease_contention(|| validate_or_repair_existing_with(&spec, &tool))
                .expect("sparse stale posture repairs and formats");
        assert_eq!(outcome, DiskInitOutcome::RepairedWithPosture);
        let meta = fs::metadata(&target).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o600);
        assert!(has_ext4_superblock(&fs::File::open(&target).unwrap(), &target).unwrap());

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_hardlinked_image_fails_closed_before_posture_repair() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let alias = scratch.join("alias.img");
        let file = create_regular_image(&target, 4096, 0o644);
        write_ext4_magic(&file);
        drop(file);
        fs::hard_link(&target, &alias).unwrap();
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let err = validate_or_repair_existing_with(&spec, &tool)
            .expect_err("multiply-linked image must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("link count"));
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o644);

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn path_identity_check_refuses_rename_swap() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let old = scratch.join("old-var.img");
        let replacement = scratch.join("replacement.img");
        let file = create_regular_image(&target, 4096, 0o600);
        write_ext4_magic(&file);
        let spec = test_spec(target.clone(), 4096, true);
        fs::rename(&target, &old).unwrap();
        let replacement_file = create_regular_image(&replacement, 4096, 0o600);
        write_ext4_magic(&replacement_file);
        drop(replacement_file);
        fs::rename(&replacement, &target).unwrap();

        let err = verify_path_still_names_fd(&file, &spec, "test")
            .expect_err("renamed replacement must not pass identity check");
        assert!(
            err.to_string()
                .contains("no longer names the validated inode")
        );

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn mkfs_failure_includes_bounded_stderr() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o600);
        drop(file);
        let spec = test_spec(target.clone(), 4096, true);
        let stderr = format!("permission denied {}", "x".repeat(MKFS_STDERR_LIMIT * 2));
        let tool = failing_mkfs_tool(&scratch, &stderr);

        let err = validate_or_repair_existing_with(&spec, &tool)
            .expect_err("failing mkfs must surface stderr");
        let rendered = err.to_string();
        assert!(rendered.contains("exit=Some(9)"));
        assert!(rendered.contains("permission denied"));
        assert!(rendered.contains("[truncated]"));
        assert!(
            rendered.len() < 900,
            "stderr should be bounded, got {} chars",
            rendered.len()
        );

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_symlink_fails_closed_at_open() {
        let scratch = scratch_root();
        let real = scratch.join("real.img");
        let link = scratch.join("link.img");
        fs::write(&real, b"x").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let spec = test_spec(link, 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let err =
            validate_or_repair_existing_with(&spec, &tool).expect_err("symlink must fail closed");
        assert!(err.to_string().contains("O_NOFOLLOW"));

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn parent_symlink_component_fails_closed() {
        let scratch = scratch_root();
        let real_dir = scratch.join("real");
        let link_dir = scratch.join("link");
        fs::create_dir(&real_dir).unwrap();
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
        let real = real_dir.join("var.img");
        let file = create_regular_image(&real, 4096, 0o600);
        write_ext4_magic(&file);
        drop(file);
        let target = link_dir.join("var.img");
        let spec = test_spec(target, 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let err = validate_or_repair_existing_with(&spec, &tool)
            .expect_err("intermediate symlink must fail closed");
        assert!(err.to_string().contains("open parent directory"));

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn fifo_target_fails_closed_without_blocking() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        nix::unistd::mkfifo(&target, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        let spec = test_spec(target, 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let err = validate_or_repair_existing_with(&spec, &tool)
            .expect_err("fifo must fail closed without blocking");
        assert!(
            err.to_string().contains("expected regular file")
                || err.to_string().contains("No such device or address")
                || err.to_string().contains("not a directory")
                || err.to_string().contains("Invalid argument")
        );

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn ext4_magic_is_little_endian_at_fixed_offset() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o600);

        assert_eq!(read_ext4_magic(&file, &target).unwrap(), Some(0));
        write_ext4_magic(&file);
        assert_eq!(
            read_ext4_magic(&file, &target).unwrap(),
            Some(EXT4_SUPER_MAGIC)
        );
        assert!(has_ext4_superblock(&file, &target).unwrap());

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn synced_sparse_blocks_zero_is_empty_evidence() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o600);
        let spec = test_spec(target, 4096, true);
        assert_eq!(
            classify_data_extents(&file, &spec).unwrap(),
            DataExtentClassification::Empty(EmptyImageEvidence::NoDataExtentsAfterSync)
        );
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn synced_allocated_blocks_are_data_evidence() {
        use std::io::Write as _;
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let mut file = create_regular_image(&target, 4096, 0o600);
        file.write_all(b"data").unwrap();
        file.sync_data().unwrap();
        let spec = test_spec(target, 4096, true);
        assert_eq!(
            classify_data_extents(&file, &spec).unwrap(),
            DataExtentClassification::HasData
        );
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn fd_target_points_at_broker_proc_fd() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o600);
        let fd_path = fd_target_path(&file);
        let rendered = fd_path.display().to_string();
        assert!(rendered.starts_with("/proc/self/fd/"));
        assert!(rendered.ends_with(&file.as_raw_fd().to_string()));

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn repair_lock_serializes_second_fd() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let first = create_regular_image(&target, 4096, 0o600);
        let second = reopen_nofollow_rw(&target).unwrap();

        let _first_lock = lock_existing_image(&first, &target).unwrap();

        let err = lock_existing_image(&second, &target)
            .expect_err("second fd must not acquire the repair lock while first holds it");
        assert!(err.to_string().contains("lock"));

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn disk_init_rejects_path_outside_permitted_root() {
        let spec = ResolvedDiskInitOp {
            target_path: PathBuf::from("/etc/nixling/evil.img"),
            size_bytes: 4096,
            mode: 0o600,
            owner_uid: 1000,
            owner_gid: 1000,
            if_absent: true,
        };
        let err = disk_init_one(&spec).expect_err("must reject path outside permitted root");
        assert_eq!(
            err.kind(),
            io::ErrorKind::PermissionDenied,
            "must return PermissionDenied for bad path"
        );
        assert!(
            err.to_string().contains("path-safety-violation"),
            "error message must mention path-safety-violation"
        );
    }

    // ----- mkfs path-safety regression tests -----

    #[test]
    fn mkfs_ext4_env_var_rejects_relative_path() {
        let err =
            validate_mkfs_ext4_binary("mkfs.ext4").expect_err("relative path must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("must be absolute"));
    }

    #[test]
    fn mkfs_ext4_env_var_rejects_parent_dir_traversal() {
        let err = validate_mkfs_ext4_binary("/run/current-system/sw/../../tmp/mkfs.ext4")
            .expect_err("`..` segment must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("may not contain"));
    }

    #[test]
    fn mkfs_ext4_env_var_accepts_canonical_absolute_path() {
        let p = validate_mkfs_ext4_binary("/run/current-system/sw/bin/mkfs.ext4")
            .expect("absolute path OK");
        assert!(p.is_absolute());
    }

    /// Post-mkfs reopen with O_NOFOLLOW refuses symlink swap. Proxy
    /// test for the TOCTOU race that the previous
    /// `std::fs::set_permissions` / `std::os::unix::fs::chown` variant
    /// allowed between mkfs returning and the chmod/chown reaching the
    /// inode. By re-opening with O_NOFOLLOW we ensure a concurrent
    /// attacker cannot redirect us to a sensitive target.
    #[test]
    fn post_mkfs_reopen_refuses_symlink() {
        use std::os::unix::fs::OpenOptionsExt;
        let scratch = scratch_root();
        let target_real = scratch.join("real.img");
        let target_link = scratch.join("link.img");
        std::fs::write(&target_real, b"x").unwrap();
        std::os::unix::fs::symlink(&target_real, &target_link).unwrap();
        let err = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&target_link)
            .expect_err("O_NOFOLLOW must refuse symlink terminal");
        // Linux: ELOOP for O_NOFOLLOW on a symlink terminal.
        assert_eq!(
            err.raw_os_error(),
            Some(40 /* ELOOP */),
            "expected ELOOP, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
