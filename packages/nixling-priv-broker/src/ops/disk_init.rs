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
//! - No `..` path components are permitted.
//! - The caller never supplies raw paths, sizes, modes, or
//!   ownership; all values come from the trusted bundle.
//!
//! Idempotency: when `if_absent = true` and the file already
//! exists, the broker validates that it is the expected nixling-owned
//! ext4 image before skipping. Empty sparse images may be formatted in
//! place; malformed or non-empty unknown data fails closed.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
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
}

/// Aggregate result of running all disk-init ops for a VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInitSummary {
    pub ops_total: u32,
    pub ops_created: u32,
    pub ops_skipped: u32,
    pub ops_repaired: u32,
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
    NoDataExtents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataExtentClassification {
    Empty(EmptyImageEvidence),
    HasData,
    UnknownUnsupported(rustix::io::Errno),
}

fn reopen_nofollow_rw(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
}

fn apply_posture(file: &File, spec: &ResolvedDiskInitOp, phase: &str) -> io::Result<()> {
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
    Ok(())
}

fn validate_existing_posture(file: &File, spec: &ResolvedDiskInitOp) -> Result<(), DiskInitError> {
    let meta = file.metadata().map_err(|e| {
        DiskInitError::unexpected(format!(
            "stat existing image {} failed: {e}",
            spec.target_path.display()
        ))
    })?;
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
    if meta.len() != spec.size_bytes {
        return Err(DiskInitError::unexpected(format!(
            "existing image {} size {} does not match declared size {}",
            spec.target_path.display(),
            meta.len(),
            spec.size_bytes
        )));
    }
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

fn classify_seek_data_result(result: rustix::io::Result<u64>) -> DataExtentClassification {
    match result {
        Ok(_) => DataExtentClassification::HasData,
        Err(err) if err == rustix::io::Errno::NXIO => {
            DataExtentClassification::Empty(EmptyImageEvidence::NoDataExtents)
        }
        Err(err) => DataExtentClassification::UnknownUnsupported(err),
    }
}

fn classify_data_extents(file: &File) -> DataExtentClassification {
    classify_seek_data_result(rustix::fs::seek(file, rustix::fs::SeekFrom::Data(0)))
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

fn fd_target_path(file: &File) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        file.as_raw_fd()
    ))
}

fn run_mkfs_ext4_on_fd_with(file: &File, display_path: &Path, tool: &MkfsTool) -> io::Result<()> {
    let target = fd_target_path(file);
    let mkfs_output = std::process::Command::new(&tool.path)
        .arg("-q")
        .arg("-F")
        .arg("-E")
        .arg("lazy_itable_init=1,lazy_journal_init=1")
        .arg(&target)
        .output()
        .map_err(|e| {
            DiskInitError::formatter(format!(
                "mkfs.ext4 ({}) spawn on {} failed: {e}",
                tool.raw,
                display_path.display()
            ))
        })?;
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
    if let Some(parent) = spec.target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("disk-init: create parent dir {}: {e}", parent.display()),
            )
        })?;
    }

    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&spec.target_path)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("disk-init: open {}: {e}", spec.target_path.display()),
            )
        })?;

    rustix::fs::fallocate(
        file.as_fd(),
        rustix::fs::FallocateFlags::empty(),
        0,
        spec.size_bytes,
    )
    .map_err(|e| {
        io::Error::other(format!(
            "disk-init: fallocate {} bytes on {}: {e}",
            spec.size_bytes,
            spec.target_path.display()
        ))
    })?;

    apply_posture(&file, spec, "create")?;
    let _lock = lock_existing_image(&file, &spec.target_path)?;
    run_mkfs_ext4_on_fd_with(&file, &spec.target_path, tool)?;
    if !has_ext4_superblock(&file, &spec.target_path)? {
        return Err(DiskInitError::formatter(format!(
            "mkfs.ext4 did not leave a valid ext4 superblock on {}",
            spec.target_path.display()
        ))
        .into());
    }
    apply_posture(&file, spec, "post-mkfs")?;
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
                "disk-init: open existing O_NOFOLLOW {}: {e}",
                spec.target_path.display()
            ),
        )
    })?;
    let _lock = lock_existing_image(&file, &spec.target_path)?;
    validate_existing_posture(&file, spec)?;
    if has_ext4_superblock(&file, &spec.target_path)? {
        return Ok(DiskInitOutcome::Skipped);
    }
    match classify_data_extents(&file) {
        DataExtentClassification::Empty(_) => {
            run_mkfs_ext4_on_fd_with(&file, &spec.target_path, tool)?;
            if !has_ext4_superblock(&file, &spec.target_path)? {
                return Err(DiskInitError::formatter(format!(
                    "mkfs.ext4 did not leave a valid ext4 superblock on {}",
                    spec.target_path.display()
                ))
                .into());
            }
            apply_posture(&file, spec, "post-repair-mkfs")?;
            Ok(DiskInitOutcome::Repaired)
        }
        DataExtentClassification::HasData => Err(DiskInitError::unsafe_repair(format!(
            "existing image {} has data but no ext4 superblock",
            spec.target_path.display()
        ))
        .into()),
        DataExtentClassification::UnknownUnsupported(errno) => {
            Err(DiskInitError::unsafe_repair(format!(
                "could not prove existing image {} is empty using filesystem extent metadata: {errno}",
                spec.target_path.display()
            ))
            .into())
        }
    }
}

/// Execute a single disk-init spec.
///
/// Validates the path, creates the file (O_CREAT|O_EXCL),
/// pre-allocates `size_bytes` via `fallocate`, sets mode + owner via
/// `fchmod` + `fchown`, and formats it as ext4. Existing images are
/// only skipped after locked posture and ext4-superblock validation.
///
/// Returns an `io::Error` on any failure.
pub fn disk_init_one(spec: &ResolvedDiskInitOp) -> io::Result<DiskInitOutcome> {
    validate_target_path(&spec.target_path)?;
    if spec.if_absent && spec.target_path.exists() {
        return validate_or_repair_existing(spec);
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
    let mut paths_concat = String::new();

    for op in &ops {
        paths_concat.push_str(&op.target_path.display().to_string());
        paths_concat.push('\n');
        match disk_init_one(op)? {
            DiskInitOutcome::Created => ops_created += 1,
            DiskInitOutcome::Skipped => ops_skipped += 1,
            DiskInitOutcome::Repaired => ops_repaired += 1,
        }
    }

    Ok(DiskInitSummary {
        ops_total,
        ops_created,
        ops_skipped,
        ops_repaired,
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
            validate_or_repair_existing_with(&spec, &tool).expect("existing image validates");
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

        let outcome = validate_or_repair_existing_with(&spec, &tool).expect("sparse image repairs");
        assert_eq!(outcome, DiskInitOutcome::Repaired);
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
        assert!(err.to_string().contains("has data but no ext4 superblock"));
        assert!(err.to_string().contains("inspect and back up"));

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn existing_wrong_mode_fails_closed() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o644);
        write_ext4_magic(&file);
        drop(file);
        let spec = test_spec(target.clone(), 4096, true);
        let tool = fake_mkfs_tool(&scratch);

        let err = validate_or_repair_existing_with(&spec, &tool)
            .expect_err("wrong mode must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("mode 0644"));
        assert!(err.to_string().contains(&target.display().to_string()));

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
    fn seek_data_enxio_is_no_data_and_other_errors_are_unknown() {
        assert_eq!(
            classify_seek_data_result(Err(rustix::io::Errno::NXIO)),
            DataExtentClassification::Empty(EmptyImageEvidence::NoDataExtents)
        );
        assert_eq!(
            classify_seek_data_result(Err(rustix::io::Errno::INVAL)),
            DataExtentClassification::UnknownUnsupported(rustix::io::Errno::INVAL)
        );
        assert_eq!(
            classify_seek_data_result(Ok(0)),
            DataExtentClassification::HasData
        );
    }

    #[test]
    fn fd_target_points_at_broker_proc_fd() {
        let scratch = scratch_root();
        let target = scratch.join("var.img");
        let file = create_regular_image(&target, 4096, 0o600);
        let fd_path = fd_target_path(&file);
        let rendered = fd_path.display().to_string();
        assert!(rendered.starts_with(&format!("/proc/{}/fd/", std::process::id())));
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
