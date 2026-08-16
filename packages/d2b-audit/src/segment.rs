//! Append-only audit segment rotation and retention.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::AuditHash;
use crate::record_types::AuditRecord;
use rustix::{
    fs::{FlockOperation, flock},
    process::{getegid, geteuid},
};

/// Default maximum segment size.
pub const DEFAULT_MAX_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
/// Default retention period.
pub const DEFAULT_RETENTION_DAYS: u64 = 30;
/// Maximum directory entries inspected by one bounded retention/rotation scan.
pub const MAX_SEGMENT_SCAN_ENTRIES: usize = 4096;
/// Maximum records inspected by one cumulative retention scan.
pub const MAX_RETENTION_SCAN_LINES: usize = 200_000;
/// Maximum bytes inspected by one cumulative retention scan.
pub const MAX_RETENTION_SCAN_BYTES: usize = 64 * 1024 * 1024;

/// Failure points used by durability tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePoint {
    /// Fail before writing record bytes.
    Append,
    /// Fail after writing record bytes but before file synchronization.
    DataSync,
    /// Fail before synchronizing the containing directory.
    ParentSync,
    /// Fail while rotating to a new segment.
    Rotation,
    /// Fail when a retention scan starts.
    PruneScan,
    /// Fail while publishing a retention checkpoint.
    PruneCheckpoint,
    /// Fail after the retention checkpoint is prepared.
    PruneDelete,
    /// Fail after retention deletes segments but before clearing its checkpoint.
    PruneFinalize,
}

struct RetentionScanBudget {
    lines: usize,
    bytes: usize,
}

impl RetentionScanBudget {
    fn consume_line(&mut self, bytes: usize) -> io::Result<()> {
        self.lines = self.lines.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.lines > MAX_RETENTION_SCAN_LINES || self.bytes > MAX_RETENTION_SCAN_BYTES {
            return Err(io::Error::other("audit-retention-scan-limit"));
        }
        Ok(())
    }
}

struct OwnedSegment {
    name: String,
    path: PathBuf,
    // Keep the active path as a sort-order boundary without selecting it.
    identity: Option<(u64, u64)>,
}

/// Deterministic fault injector for append and rotation tests.
#[derive(Debug, Clone, Default)]
pub struct FailureInjector {
    append: Arc<AtomicU8>,
    data_sync: Arc<AtomicU8>,
    parent_sync: Arc<AtomicU8>,
    rotation: Arc<AtomicU8>,
    prune_scan: Arc<AtomicU8>,
    prune_checkpoint: Arc<AtomicU8>,
    prune_delete: Arc<AtomicU8>,
    prune_finalize: Arc<AtomicU8>,
}

impl FailureInjector {
    /// Arrange for the next operation at `point` to fail.
    pub fn fail_next(&self, point: FailurePoint) {
        let slot = match point {
            FailurePoint::Append => &self.append,
            FailurePoint::DataSync => &self.data_sync,
            FailurePoint::ParentSync => &self.parent_sync,
            FailurePoint::Rotation => &self.rotation,
            FailurePoint::PruneScan => &self.prune_scan,
            FailurePoint::PruneCheckpoint => &self.prune_checkpoint,
            FailurePoint::PruneDelete => &self.prune_delete,
            FailurePoint::PruneFinalize => &self.prune_finalize,
        };
        slot.store(1, Ordering::SeqCst);
    }

    fn take(&self, point: FailurePoint) -> bool {
        let slot = match point {
            FailurePoint::Append => &self.append,
            FailurePoint::DataSync => &self.data_sync,
            FailurePoint::ParentSync => &self.parent_sync,
            FailurePoint::Rotation => &self.rotation,
            FailurePoint::PruneScan => &self.prune_scan,
            FailurePoint::PruneCheckpoint => &self.prune_checkpoint,
            FailurePoint::PruneDelete => &self.prune_delete,
            FailurePoint::PruneFinalize => &self.prune_finalize,
        };
        slot.swap(0, Ordering::SeqCst) != 0
    }
}

/// Append-only segment writer.
pub struct SegmentWriter {
    directory: PathBuf,
    directory_file: File,
    _lock_file: File,
    directory_identity: (u64, u64),
    file: File,
    file_identity: (u64, u64),
    path: PathBuf,
    bytes: u64,
    opened_day: u64,
    sequence: u32,
    max_bytes: u64,
    retention_days: u64,
    injector: Option<FailureInjector>,
    retention_degraded: AtomicBool,
}

impl core::fmt::Debug for SegmentWriter {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SegmentWriter")
            .field("bytes", &self.bytes)
            .field("sequence", &self.sequence)
            .field("max_bytes", &self.max_bytes)
            .field("retention_days", &self.retention_days)
            .field(
                "retention_degraded",
                &self.retention_degraded.load(Ordering::Acquire),
            )
            .finish()
    }
}

impl SegmentWriter {
    /// Open the current segment in a directory.
    pub fn open(
        directory: impl AsRef<Path>,
        max_bytes: u64,
        retention_days: u64,
    ) -> io::Result<Self> {
        fs::create_dir_all(directory.as_ref())?;
        Self::open_at(directory, max_bytes.max(1), retention_days, now_ms())
    }

    /// Open at a supplied timestamp, useful for deterministic tests.
    pub fn open_at(
        directory: impl AsRef<Path>,
        max_bytes: u64,
        retention_days: u64,
        timestamp_ms: u64,
    ) -> io::Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit-directory-not-absolute",
            ));
        }
        fs::create_dir_all(&directory)?;
        let directory_file = open_directory(&directory)?;
        let directory_metadata = directory_file.metadata()?;
        validate_directory_metadata(&directory_metadata)?;
        let lock_file = open_lock(&directory, directory_metadata.gid())?;
        repair_pending_checkpoint(&directory, &directory_file)?;
        let opened_day = day_number(timestamp_ms);
        let sequence = next_sequence(&directory, timestamp_ms)?;
        let path = directory.join(segment_name(timestamp_ms, sequence));
        let file = open_append(&path)?;
        let file_metadata = file.metadata()?;
        validate_segment_metadata(&file_metadata)?;
        directory_file.sync_all()?;
        let bytes = file.metadata()?.len();
        let writer = Self {
            directory,
            directory_file,
            _lock_file: lock_file,
            directory_identity: (directory_metadata.dev(), directory_metadata.ino()),
            file,
            file_identity: (file_metadata.dev(), file_metadata.ino()),
            path,
            bytes,
            opened_day,
            sequence,
            max_bytes: max_bytes.max(1),
            retention_days,
            injector: None,
            retention_degraded: AtomicBool::new(false),
        };
        writer.prune_old(timestamp_ms)?;
        Ok(writer)
    }

    /// Open a writer with deterministic durability fault injection.
    pub fn open_at_with_injector(
        directory: impl AsRef<Path>,
        max_bytes: u64,
        retention_days: u64,
        timestamp_ms: u64,
        injector: FailureInjector,
    ) -> io::Result<Self> {
        let mut writer = Self::open_at(directory, max_bytes, retention_days, timestamp_ms)?;
        writer.injector = Some(injector);
        Ok(writer)
    }

    /// Append a record and rotate before crossing a size or UTC-day boundary.
    pub fn append(&mut self, record: &AuditRecord) -> io::Result<PathBuf> {
        self.append_at(record, now_ms())
    }

    /// Append a record at a supplied timestamp.
    ///
    /// The timestamp is used only for deterministic rotation decisions; the
    /// audit record's own timestamp remains part of the caller-supplied
    /// record and is never rewritten by the segment writer.
    pub fn append_at(&mut self, record: &AuditRecord, timestamp_ms: u64) -> io::Result<PathBuf> {
        self.prepare_append()?;
        let line = record.to_json_line().map_err(io::Error::other)?;
        self.append_serialized_at(&line, timestamp_ms)
    }

    /// Append a line already serialized and validated by the audit record.
    pub(crate) fn append_serialized(&mut self, line: &[u8]) -> io::Result<PathBuf> {
        self.prepare_append()?;
        self.append_serialized_at(line, now_ms())
    }

    fn prepare_append(&self) -> io::Result<()> {
        self.validate_live_inodes()?;
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::Append))
        {
            return Err(io::Error::other("audit-append-injected"));
        }
        Ok(())
    }

    fn append_serialized_at(&mut self, line: &[u8], timestamp_ms: u64) -> io::Result<PathBuf> {
        let current_day = day_number(timestamp_ms);
        let rotated = current_day != self.opened_day
            || (self.bytes > 0 && self.bytes.saturating_add(line.len() as u64) > self.max_bytes);
        if rotated {
            self.rotate(timestamp_ms)?;
        }
        let offset = self.file.metadata()?.len();
        if let Err(error) = self.file.write_all(line) {
            let _ = rollback_append(&mut self.file, offset, &self.directory_file);
            return Err(error);
        }
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::DataSync))
        {
            rollback_append(&mut self.file, offset, &self.directory_file)?;
            return Err(io::Error::other("audit-data-sync-injected"));
        }
        if let Err(error) = self.file.sync_data() {
            rollback_append(&mut self.file, offset, &self.directory_file)?;
            return Err(error);
        }
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::ParentSync))
        {
            rollback_append(&mut self.file, offset, &self.directory_file)?;
            return Err(io::Error::other("audit-parent-sync-injected"));
        }
        if let Err(error) = self.directory_file.sync_all() {
            rollback_append(&mut self.file, offset, &self.directory_file)?;
            return Err(error);
        }
        self.bytes = self.bytes.saturating_add(line.len() as u64);
        if rotated {
            let _ = self.prune_old(timestamp_ms);
        }
        Ok(self.path.clone())
    }

    /// Force the current segment to disk.
    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_all()?;
        self.directory_file.sync_all()
    }

    /// Current segment path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Directory owned by this writer.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Whether a failed append left the current segment at its tracked offset.
    pub(crate) fn append_state_consistent(&self) -> bool {
        self.file
            .metadata()
            .map(|metadata| metadata.len() == self.bytes)
            .unwrap_or(false)
    }

    /// Whether the last automatic retention attempt needs a retry.
    pub fn retention_degraded(&self) -> bool {
        self.retention_degraded.load(Ordering::Acquire)
    }

    /// Remove segments older than the retention floor.
    ///
    /// Only files matching the owned `audit-*.jsonl` shape are considered.
    pub fn prune_old(&self, now_ms: u64) -> io::Result<usize> {
        let result = self.prune_old_inner(now_ms);
        self.retention_degraded
            .store(result.is_err(), Ordering::Release);
        result
    }

    fn prune_old_inner(&self, now_ms: u64) -> io::Result<usize> {
        if self.retention_days == 0 {
            return Ok(0);
        }
        repair_pending_checkpoint(&self.directory, &self.directory_file)?;
        let floor = now_ms.saturating_sub(self.retention_days.saturating_mul(24 * 60 * 60 * 1000));
        let mut budget = RetentionScanBudget { lines: 0, bytes: 0 };
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::PruneScan))
        {
            return Err(io::Error::other("audit-prune-scan-injected"));
        }
        let cutoff_date = utc_stamp(floor)
            .get(..8)
            .ok_or_else(|| io::Error::other("audit-retention-date-invalid"))?
            .to_owned();
        let inventory = owned_segment_inventory(&self.directory, &self.path)?;
        let mut prefix = Vec::new();
        for segment in inventory {
            let segment_date = segment
                .name
                .get(6..14)
                .ok_or_else(|| io::Error::other("audit-segment-name-invalid"))?;
            if segment.identity.is_none() || segment_date >= cutoff_date.as_str() {
                break;
            }
            prefix.push(segment);
        }
        if prefix.is_empty() {
            return Ok(0);
        }

        let start_anchor = checkpoint_anchor(&self.directory)?;
        let mut anchor = start_anchor.clone();
        let mut checkpoint_segments = Vec::with_capacity(prefix.len());
        for segment in &prefix {
            let metadata = fs::symlink_metadata(&segment.path)?;
            validate_segment_metadata(&metadata)?;
            let Some(identity) = segment.identity else {
                return Err(io::Error::other("audit-segment-identity-invalid"));
            };
            if (metadata.dev(), metadata.ino()) != identity {
                return Err(io::Error::other("audit-segment-identity-invalid"));
            }
            let previous = anchor.clone();
            let tail = segment_tail_hash(&segment.path, &previous, &mut budget)?;
            checkpoint_segments.push(RetentionSegment {
                name: segment.name.clone(),
                dev: metadata.dev(),
                ino: metadata.ino(),
                previous,
                tail: tail.clone(),
            });
            anchor = tail;
        }
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::PruneCheckpoint))
        {
            return Err(io::Error::other("audit-prune-checkpoint-injected"));
        }
        write_checkpoint(
            &self.directory,
            &start_anchor,
            &anchor,
            &checkpoint_segments,
            RetentionCheckpointPhase::Prepared,
        )?;
        self.directory_file.sync_all()?;
        write_checkpoint(
            &self.directory,
            &start_anchor,
            &anchor,
            &checkpoint_segments,
            RetentionCheckpointPhase::Deleting,
        )?;
        self.directory_file.sync_all()?;
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::PruneDelete))
        {
            return Err(io::Error::other("audit-prune-delete-injected"));
        }
        let mut removed = 0;
        for segment in &checkpoint_segments {
            if remove_checkpoint_segment(&self.directory, segment)? {
                removed += 1;
            }
        }
        self.directory_file.sync_all()?;
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::PruneFinalize))
        {
            return Err(io::Error::other("audit-prune-finalize-injected"));
        }
        clear_checkpoint(&self.directory, &anchor)?;
        self.directory_file.sync_all()?;
        Ok(removed)
    }

    fn rotate(&mut self, timestamp_ms: u64) -> io::Result<()> {
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::Rotation))
        {
            return Err(io::Error::other("audit-rotation-injected"));
        }
        self.file.sync_all()?;
        let opened_day = day_number(timestamp_ms);
        let sequence = self.sequence.saturating_add(1);
        let path = self.directory.join(segment_name(timestamp_ms, sequence));
        let file = open_append(&path)?;
        if let Err(error) = self.directory_file.sync_all() {
            drop(file);
            let _ = fs::remove_file(&path);
            let _ = self.directory_file.sync_all();
            return Err(error);
        }
        self.opened_day = opened_day;
        self.sequence = sequence;
        self.path = path;
        self.file = file;
        let metadata = self.file.metadata()?;
        validate_segment_metadata(&metadata)?;
        self.file_identity = (metadata.dev(), metadata.ino());
        self.bytes = metadata.len();
        Ok(())
    }

    fn validate_live_inodes(&self) -> io::Result<()> {
        let directory = self.directory_file.metadata()?;
        if (directory.dev(), directory.ino()) != self.directory_identity {
            return Err(io::Error::other("audit-directory-inode-changed"));
        }
        let file = self.file.metadata()?;
        if (file.dev(), file.ino()) != self.file_identity {
            return Err(io::Error::other("audit-segment-inode-changed"));
        }
        validate_directory_metadata(&directory)?;
        validate_segment_metadata(&file)
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .mode(0o640)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

fn open_directory(directory: &Path) -> io::Result<File> {
    let directory_fd = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?;
    Ok(directory_fd)
}

fn open_lock(directory: &Path, expected_gid: u32) -> io::Result<File> {
    let path = directory.join("audit.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !(metadata.uid() == geteuid().as_raw() || metadata.uid() == 0)
        || !(metadata.gid() == getegid().as_raw() || metadata.gid() == expected_gid)
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "audit-lock-ownership-invalid",
        ));
    }
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "audit-lock-held"))?;
    Ok(file)
}

fn validate_directory_metadata(metadata: &std::fs::Metadata) -> io::Result<()> {
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !(metadata.uid() == geteuid().as_raw() || metadata.uid() == 0)
        || metadata.permissions().mode() & 0o0022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "audit-directory-ownership-invalid",
        ));
    }
    Ok(())
}

fn validate_segment_metadata(metadata: &std::fs::Metadata) -> io::Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::other("audit-segment-identity-invalid"));
    }
    Ok(())
}

fn rollback_append(file: &mut File, offset: u64, directory: &File) -> io::Result<()> {
    file.set_len(offset)?;
    file.seek(SeekFrom::Start(offset))?;
    file.sync_all()?;
    directory.sync_all()
}

fn checkpoint_path(directory: &Path) -> PathBuf {
    directory.join("audit-checkpoint.json")
}

fn checkpoint_next_path(directory: &Path) -> PathBuf {
    directory.join("audit-checkpoint.json.next")
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionCheckpoint {
    anchor: AuditHash,
    pending: bool,
    #[serde(default)]
    start_anchor: Option<AuditHash>,
    #[serde(default)]
    segments: Vec<RetentionSegment>,
    #[serde(default)]
    phase: Option<RetentionCheckpointPhase>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RetentionCheckpointPhase {
    Prepared,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionSegment {
    name: String,
    dev: u64,
    ino: u64,
    previous: AuditHash,
    tail: AuditHash,
}

fn read_checkpoint_file(path: &Path) -> io::Result<Option<RetentionCheckpoint>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    validate_segment_metadata(&metadata)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    if file.metadata()?.len() > 1024 * 1024 {
        return Err(io::Error::other("audit-retention-checkpoint-limit"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let checkpoint = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::other("audit-retention-checkpoint-invalid"))?;
    Ok(Some(checkpoint))
}

fn read_checkpoint(directory: &Path) -> io::Result<Option<RetentionCheckpoint>> {
    read_checkpoint_with_directory(directory, None)
}

fn read_checkpoint_with_directory(
    directory: &Path,
    directory_file: Option<&File>,
) -> io::Result<Option<RetentionCheckpoint>> {
    let checkpoint = read_checkpoint_file(&checkpoint_path(directory))?;
    if let Some(checkpoint) = checkpoint.as_ref() {
        validate_checkpoint(checkpoint)?;
    }
    let staged = match read_checkpoint_file(&checkpoint_next_path(directory)) {
        Ok(staged) => staged,
        Err(error) if is_discardable_checkpoint_scratch_error(&error) => {
            discard_checkpoint_scratch(directory, directory_file)?;
            None
        }
        Err(error) => return Err(error),
    };
    let Some(staged) = staged else {
        return Ok(checkpoint);
    };
    if let Err(error) = validate_checkpoint(&staged) {
        if is_discardable_checkpoint_scratch_error(&error) {
            discard_checkpoint_scratch(directory, directory_file)?;
            return Ok(checkpoint);
        }
        return Err(error);
    }
    let checkpoint = match checkpoint {
        None => Some(staged),
        Some(committed) => {
            let compatible = match (&committed.pending, &staged.pending) {
                (false, true) => staged.start_anchor.as_ref() == Some(&committed.anchor),
                (true, false) => staged.anchor == committed.anchor,
                (true, true) => {
                    committed.anchor == staged.anchor
                        && committed.start_anchor == staged.start_anchor
                        && committed.segments == staged.segments
                        && matches!(
                            (committed.phase, staged.phase),
                            (
                                Some(RetentionCheckpointPhase::Prepared),
                                Some(RetentionCheckpointPhase::Deleting)
                            )
                        )
                }
                (false, false) => committed.anchor == staged.anchor,
            };
            if !compatible {
                discard_checkpoint_scratch(directory, directory_file)?;
                return Ok(Some(committed));
            }
            Some(staged)
        }
    };
    Ok(checkpoint)
}

fn is_discardable_checkpoint_scratch_error(error: &io::Error) -> bool {
    matches!(
        error.to_string().as_str(),
        "audit-retention-checkpoint-invalid"
            | "audit-retention-checkpoint-limit"
            | "audit-retention-checkpoint-unverifiable"
    )
}

fn discard_checkpoint_scratch(directory: &Path, directory_file: Option<&File>) -> io::Result<()> {
    let path = checkpoint_next_path(directory);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    validate_segment_metadata(&metadata)?;
    fs::remove_file(path)?;
    match directory_file {
        Some(directory_file) => directory_file.sync_all(),
        None => open_directory(directory)?.sync_all(),
    }
}

fn validate_checkpoint(checkpoint: &RetentionCheckpoint) -> io::Result<()> {
    if !checkpoint.pending {
        if checkpoint.start_anchor.is_some()
            || !checkpoint.segments.is_empty()
            || checkpoint.phase.is_some()
        {
            return Err(io::Error::other("audit-retention-checkpoint-unverifiable"));
        }
        return Ok(());
    }
    let Some(start_anchor) = checkpoint.start_anchor.as_ref() else {
        return Err(io::Error::other("audit-retention-checkpoint-unverifiable"));
    };
    let Some(phase) = checkpoint.phase else {
        return Err(io::Error::other("audit-retention-checkpoint-unverifiable"));
    };
    if checkpoint.segments.is_empty() || checkpoint.segments.len() > MAX_SEGMENT_SCAN_ENTRIES {
        return Err(io::Error::other("audit-retention-checkpoint-unverifiable"));
    }
    let mut previous_name = None;
    let mut previous = start_anchor.clone();
    for segment in &checkpoint.segments {
        if !crate::export::is_segment_name(&segment.name)
            || previous_name.is_some_and(|name| name >= segment.name.as_str())
            || segment.previous != previous
        {
            return Err(io::Error::other("audit-retention-checkpoint-unverifiable"));
        }
        previous_name = Some(segment.name.as_str());
        previous = segment.tail.clone();
    }
    if checkpoint.anchor != previous {
        return Err(io::Error::other("audit-retention-checkpoint-unverifiable"));
    }
    match phase {
        RetentionCheckpointPhase::Prepared | RetentionCheckpointPhase::Deleting => Ok(()),
    }
}

fn repair_pending_checkpoint(directory: &Path, directory_file: &File) -> io::Result<()> {
    match fs::symlink_metadata(checkpoint_next_path(directory)) {
        Ok(metadata) => {
            validate_segment_metadata(&metadata)?;
            read_checkpoint_with_directory(directory, Some(directory_file))?;
            match fs::symlink_metadata(checkpoint_next_path(directory)) {
                Ok(metadata) => {
                    validate_segment_metadata(&metadata)?;
                    fs::rename(checkpoint_next_path(directory), checkpoint_path(directory))?;
                    directory_file.sync_all()?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let Some(checkpoint) = read_checkpoint_with_directory(directory, Some(directory_file))? else {
        return Ok(());
    };
    if !checkpoint.pending {
        return Ok(());
    }
    let phase = checkpoint
        .phase
        .ok_or_else(|| io::Error::other("audit-retention-checkpoint-unverifiable"))?;
    let allow_missing = phase == RetentionCheckpointPhase::Deleting;
    let mut budget = RetentionScanBudget { lines: 0, bytes: 0 };
    for segment in &checkpoint.segments {
        let path = directory.join(&segment.name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound && allow_missing => continue,
            Err(error) => return Err(error),
        };
        validate_segment_metadata(&metadata)?;
        if (metadata.dev(), metadata.ino()) != (segment.dev, segment.ino) {
            return Err(io::Error::other("audit-segment-identity-invalid"));
        }
        let tail = segment_tail_hash(&path, &segment.previous, &mut budget)?;
        if tail != segment.tail {
            return Err(io::Error::other("audit-retention-checkpoint-chain-invalid"));
        }
    }
    if phase == RetentionCheckpointPhase::Prepared {
        write_checkpoint(
            directory,
            checkpoint
                .start_anchor
                .as_ref()
                .ok_or_else(|| io::Error::other("audit-retention-checkpoint-unverifiable"))?,
            &checkpoint.anchor,
            &checkpoint.segments,
            RetentionCheckpointPhase::Deleting,
        )?;
        directory_file.sync_all()?;
    }
    for segment in &checkpoint.segments {
        let _ = remove_checkpoint_segment(directory, segment)?;
    }
    directory_file.sync_all()?;
    clear_checkpoint(directory, &checkpoint.anchor)?;
    directory_file.sync_all()
}

fn remove_checkpoint_segment(directory: &Path, segment: &RetentionSegment) -> io::Result<bool> {
    let path = directory.join(&segment.name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    validate_segment_metadata(&metadata)?;
    if (metadata.dev(), metadata.ino()) != (segment.dev, segment.ino) {
        return Err(io::Error::other("audit-segment-identity-invalid"));
    }
    fs::remove_file(path)?;
    Ok(true)
}

pub(crate) fn checkpoint_pending(directory: &Path) -> io::Result<bool> {
    Ok(read_checkpoint(directory)?.is_some_and(|checkpoint| checkpoint.pending))
}

pub(crate) fn checkpoint_anchor(directory: &Path) -> io::Result<AuditHash> {
    match read_checkpoint(directory)? {
        Some(checkpoint) if checkpoint.pending => {
            Err(io::Error::other("audit-retention-checkpoint-pending"))
        }
        Some(checkpoint) => Ok(checkpoint.anchor),
        None => Ok(crate::genesis_hash()),
    }
}

fn write_checkpoint(
    directory: &Path,
    start_anchor: &AuditHash,
    anchor: &AuditHash,
    segments: &[RetentionSegment],
    phase: RetentionCheckpointPhase,
) -> io::Result<()> {
    let path = checkpoint_path(directory);
    let tmp = checkpoint_next_path(directory);
    let bytes = serde_json::to_vec(&RetentionCheckpoint {
        anchor: anchor.clone(),
        pending: true,
        start_anchor: Some(start_anchor.clone()),
        segments: segments.to_vec(),
        phase: Some(phase),
    })
    .map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?
        .sync_all()?;
    Ok(())
}

fn clear_checkpoint(directory: &Path, anchor: &AuditHash) -> io::Result<()> {
    let path = checkpoint_path(directory);
    let tmp = checkpoint_next_path(directory);
    let bytes = serde_json::to_vec(&RetentionCheckpoint {
        anchor: anchor.clone(),
        pending: false,
        start_anchor: None,
        segments: Vec::new(),
        phase: None,
    })
    .map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?
        .sync_all()
}

fn owned_segment_inventory(directory: &Path, active_path: &Path) -> io::Result<Vec<OwnedSegment>> {
    let mut inventory = Vec::new();
    let mut scanned = 0usize;
    for entry in fs::read_dir(directory)? {
        scanned = scanned.saturating_add(1);
        if scanned > MAX_SEGMENT_SCAN_ENTRIES {
            return Err(io::Error::other("audit-retention-scan-limit"));
        }
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !crate::export::is_segment_name(name) {
            continue;
        }
        let name = name.to_owned();
        let path = entry.path();
        let identity = if path == active_path {
            None
        } else {
            let metadata = fs::symlink_metadata(&path)?;
            validate_segment_metadata(&metadata)?;
            Some((metadata.dev(), metadata.ino()))
        };
        inventory.push(OwnedSegment {
            name,
            path,
            identity,
        });
    }
    inventory.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(inventory)
}

fn segment_tail_hash(
    path: &Path,
    previous: &AuditHash,
    budget: &mut RetentionScanBudget,
) -> io::Result<AuditHash> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    let mut reader = BufReader::new(file);
    let mut current = previous.clone();
    while let Some(line) = read_bounded_line(&mut reader)? {
        budget.consume_line(line.len())?;
        if line.is_empty() {
            continue;
        }
        let record: AuditRecord = serde_json::from_slice(&line)
            .map_err(|_| io::Error::other("audit-segment-record-invalid"))?;
        record
            .verify(&current)
            .map_err(|_| io::Error::other("audit-segment-chain-invalid"))?;
        current = record.record_hash().clone();
    }
    Ok(current)
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::other("audit-segment-line-truncated"))
            };
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > crate::export::MAX_EXPORT_LINE_BYTES {
            return Err(io::Error::other("audit-segment-line-limit"));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            bytes.pop();
            return Ok(Some(bytes));
        }
    }
}

fn next_sequence(directory: &Path, timestamp_ms: u64) -> io::Result<u32> {
    let prefix = format!("audit-{}", utc_stamp(timestamp_ms));
    let mut max = 0;
    let mut scanned = 0usize;
    for entry in fs::read_dir(directory)? {
        scanned = scanned.saturating_add(1);
        if scanned > MAX_SEGMENT_SCAN_ENTRIES {
            return Err(io::Error::other("audit-segment-scan-limit"));
        }
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".jsonl"))
            && let Ok(sequence) = rest.parse::<u32>()
        {
            max = max.max(sequence);
        }
    }
    Ok(max)
}

fn segment_name(timestamp_ms: u64, sequence: u32) -> String {
    format!("audit-{}{:06}.jsonl", utc_stamp(timestamp_ms), sequence)
}

fn utc_stamp(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1000;
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = seconds_of_day / 3600;
    let minute = seconds_of_day % 3600 / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}")
}

fn day_number(timestamp_ms: u64) -> u64 {
    timestamp_ms / (86_400 * 1000)
}

fn now_ms() -> u64 {
    system_time_ms(SystemTime::now()).unwrap_or(0)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

// Howard Hinnant's civil calendar conversion, kept local to avoid a datetime
// dependency in the append-only audit writer.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash_chain::genesis_hash;
    use crate::record_types::{AuditRecord, AuditRecordFields, ProcessEffectFields};

    fn writable_manifest_dir() -> std::path::PathBuf {
        std::env::var_os("TEST_TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }

    fn sample() -> AuditRecord {
        AuditRecord::new(
            1,
            "work",
            "op",
            "corr",
            None,
            "test",
            genesis_hash(),
            AuditRecordFields::ProcessEffect(ProcessEffectFields {
                event: "launch".to_owned(),
                provider: "systemd".to_owned(),
                domain: "system".to_owned(),
                no_isolation: false,
                execution_ref_digest:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                        .to_owned(),
                process_uid: "uid".to_owned(),
                outcome: "ok".to_owned(),
                exit_class: None,
            }),
        )
        .unwrap()
    }

    fn old_segment(directory: &Path) -> PathBuf {
        let path = directory.join("audit-19700101000000000000.jsonl");
        fs::write(&path, b"").unwrap();
        path
    }

    fn test_directory(name: &str) -> PathBuf {
        writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-{name}-{}", std::process::id()))
    }

    fn write_staged_checkpoint(
        directory: &Path,
        start_anchor: &AuditHash,
        anchor: &AuditHash,
        segments: &[RetentionSegment],
        phase: RetentionCheckpointPhase,
    ) {
        let bytes = serde_json::to_vec(&RetentionCheckpoint {
            anchor: anchor.clone(),
            pending: true,
            start_anchor: Some(start_anchor.clone()),
            segments: segments.to_vec(),
            phase: Some(phase),
        })
        .unwrap();
        fs::write(checkpoint_next_path(directory), bytes).unwrap();
    }

    #[test]
    fn names_are_owned_and_rotation_is_size_bounded() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-segment-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut writer = SegmentWriter::open_at(&directory, 1, 30, 1_700_000_000_000).unwrap();
        let first = writer.append(&sample()).unwrap();
        let second = writer.append(&sample()).unwrap();
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("audit-")
        );
        assert_ne!(first, second);
        assert!(second.extension().is_some_and(|value| value == "jsonl"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pruning_ignores_unowned_jsonl_names() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let writer = SegmentWriter::open_at(&directory, 1024, 1, 1_700_000_000_000).unwrap();
        let unowned = directory.join("audit-not-a-segment.jsonl");
        fs::write(&unowned, b"not an audit segment\n").unwrap();
        let removed = writer.prune_old(1_900_000_000_000).unwrap();
        assert_eq!(removed, 0);
        assert!(unowned.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pruning_rejects_invalid_owned_artifacts() {
        for kind in ["directory", "symlink"] {
            let directory = writable_manifest_dir()
                .join("target")
                .join(format!("d2b-audit-invalid-{kind}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&directory);
            let writer = SegmentWriter::open_at(&directory, 1024, 1, 1_700_000_000_000).unwrap();
            let invalid = directory.join("audit-19700101000000000000.jsonl");
            match kind {
                "directory" => fs::create_dir(&invalid).unwrap(),
                "symlink" => std::os::unix::fs::symlink("missing", &invalid).unwrap(),
                _ => unreachable!(),
            }
            let error = writer.prune_old(1_900_000_000_000).unwrap_err();
            assert_eq!(error.to_string(), "audit-segment-identity-invalid");
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn pruning_excludes_the_active_segment() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-active-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let writer = SegmentWriter::open_at(&directory, 1024, 1, 1_700_000_000_000).unwrap();
        let active = writer.path().to_path_buf();
        let old = old_segment(&directory);

        assert_eq!(writer.prune_old(1_900_000_000_000).unwrap(), 1);
        assert!(active.exists());
        assert!(!old.exists());
        let later_old = directory.join("audit-20240101000000000000.jsonl");
        fs::write(&later_old, b"").unwrap();
        assert_eq!(writer.prune_old(1_900_000_000_000).unwrap(), 0);
        assert!(later_old.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pruning_stops_at_the_first_non_expired_segment() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-prefix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let now_ms = 1_700_000_000_000 + 10 * 86_400_000;
        let writer = SegmentWriter::open_at(&directory, 1024, 1, now_ms + 10 * 86_400_000).unwrap();
        let old = old_segment(&directory);
        let gap = directory.join(format!("audit-{}000000.jsonl", utc_stamp(now_ms)));
        fs::write(&gap, b"").unwrap();

        assert_eq!(writer.prune_old(now_ms).unwrap(), 1);
        assert!(!old.exists());
        assert!(gap.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pruning_fails_closed_when_directory_scan_budget_is_exceeded() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-scan-budget-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let writer = SegmentWriter::open_at(&directory, 1024, 1, 1_700_000_000_000).unwrap();
        for index in 0..=MAX_SEGMENT_SCAN_ENTRIES {
            fs::write(
                directory.join(format!("foreign-retention-artifact-{index}")),
                b"foreign",
            )
            .unwrap();
        }

        let error = writer.prune_old(1_900_000_000_000).unwrap_err();
        assert_eq!(error.to_string(), "audit-retention-scan-limit");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durability_failures_never_report_a_successful_append() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-faults-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let injector = FailureInjector::default();
        let mut writer = SegmentWriter::open_at_with_injector(
            &directory,
            1024,
            30,
            1_700_000_000_000,
            injector.clone(),
        )
        .unwrap();

        for point in [
            FailurePoint::Append,
            FailurePoint::DataSync,
            FailurePoint::ParentSync,
        ] {
            injector.fail_next(point);
            let result = writer.append(&sample());
            assert!(result.is_err(), "failure point {point:?} was ignored");
        }
        writer.append(&sample()).unwrap();
        injector.fail_next(FailurePoint::Rotation);
        assert!(
            writer
                .append_at(&sample(), 1_700_000_000_000 + 86_400_000)
                .is_err()
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_append_survives_retention_failure_and_reports_degradation() {
        let directory = writable_manifest_dir().join("target").join(format!(
            "d2b-audit-retention-after-append-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let injector = FailureInjector::default();
        let mut writer = SegmentWriter::open_at_with_injector(
            &directory,
            1,
            1,
            1_900_000_000_000,
            injector.clone(),
        )
        .unwrap();
        let old = old_segment(&directory);
        writer.append_at(&sample(), 1_900_000_000_000).unwrap();
        injector.fail_next(FailurePoint::PruneCheckpoint);
        assert!(writer.append_at(&sample(), 1_900_000_000_000).is_ok());
        assert!(writer.retention_degraded());
        assert!(writer.append_state_consistent());
        assert!(old.exists());
        assert_eq!(writer.prune_old(1_900_000_000_000).unwrap(), 1);
        assert!(!writer.retention_degraded());
        assert!(!old.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pending_retention_checkpoint_repairs_on_restart_across_delete_boundaries() {
        for point in [FailurePoint::PruneDelete, FailurePoint::PruneFinalize] {
            let directory = writable_manifest_dir().join("target").join(format!(
                "d2b-audit-checkpoint-{point:?}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&directory);
            let injector = FailureInjector::default();
            let mut writer = SegmentWriter::open_at_with_injector(
                &directory,
                1,
                1,
                1_900_000_000_000,
                injector.clone(),
            )
            .unwrap();
            let old = old_segment(&directory);
            writer.append_at(&sample(), 1_900_000_000_000).unwrap();
            injector.fail_next(point);
            assert!(writer.append_at(&sample(), 1_900_000_000_000).is_ok());
            assert!(writer.retention_degraded());
            drop(writer);

            let writer = SegmentWriter::open_at(&directory, 1, 1, 1_900_000_000_000).unwrap();
            assert!(!writer.retention_degraded());
            assert!(!old.exists());
            assert!(!checkpoint_pending(&directory).unwrap());
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn truncated_checkpoint_scratch_is_discarded_before_recovery() {
        let directory = test_directory("truncated-checkpoint-next");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        clear_checkpoint(&directory, &genesis_hash()).unwrap();
        fs::write(checkpoint_next_path(&directory), b"{").unwrap();

        let writer = SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).unwrap();

        assert!(!checkpoint_next_path(&directory).exists());
        assert_eq!(checkpoint_anchor(&directory).unwrap(), genesis_hash());
        drop(writer);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn garbage_checkpoint_scratch_without_commit_is_discarded() {
        let directory = test_directory("garbage-checkpoint-next");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(checkpoint_next_path(&directory), b"garbage").unwrap();

        let writer = SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).unwrap();

        assert!(!checkpoint_path(&directory).exists());
        assert!(!checkpoint_next_path(&directory).exists());
        assert_eq!(checkpoint_anchor(&directory).unwrap(), genesis_hash());
        drop(writer);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unsafe_checkpoint_scratch_identity_fails_closed() {
        for kind in ["directory", "symlink"] {
            let directory = test_directory(&format!("unsafe-checkpoint-next-{kind}"));
            let _ = fs::remove_dir_all(&directory);
            fs::create_dir_all(&directory).unwrap();
            match kind {
                "directory" => fs::create_dir(checkpoint_next_path(&directory)).unwrap(),
                "symlink" => {
                    std::os::unix::fs::symlink("missing", checkpoint_next_path(&directory)).unwrap()
                }
                _ => unreachable!(),
            }

            let error =
                SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).unwrap_err();

            assert_eq!(error.to_string(), "audit-segment-identity-invalid");
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn invalid_committed_checkpoint_fails_closed_even_with_garbage_scratch() {
        let directory = test_directory("invalid-committed-checkpoint");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(checkpoint_path(&directory), b"invalid").unwrap();
        fs::write(checkpoint_next_path(&directory), b"garbage").unwrap();

        let error = SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).unwrap_err();

        assert_eq!(error.to_string(), "audit-retention-checkpoint-invalid");
        assert!(checkpoint_next_path(&directory).exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn staged_checkpoint_publish_wins_atomically() {
        let directory = test_directory("staged-checkpoint-publish");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        clear_checkpoint(&directory, &genesis_hash()).unwrap();

        let segment = directory.join("audit-19700101000000000000.jsonl");
        let record = sample();
        let line = record.to_json_line().unwrap();
        fs::write(&segment, line).unwrap();
        let metadata = fs::symlink_metadata(&segment).unwrap();
        let anchor = record.record_hash().clone();
        let checkpoint_segment = RetentionSegment {
            name: "audit-19700101000000000000.jsonl".to_owned(),
            dev: metadata.dev(),
            ino: metadata.ino(),
            previous: genesis_hash(),
            tail: anchor.clone(),
        };
        write_staged_checkpoint(
            &directory,
            &genesis_hash(),
            &anchor,
            &[checkpoint_segment],
            RetentionCheckpointPhase::Prepared,
        );

        let writer = SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).unwrap();

        assert!(!checkpoint_next_path(&directory).exists());
        assert!(!segment.exists());
        assert_eq!(checkpoint_anchor(&directory).unwrap(), anchor);
        drop(writer);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unverifiable_pending_retention_checkpoint_fails_closed_on_restart() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-checkpoint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        write_checkpoint(
            &directory,
            &crate::genesis_hash(),
            &crate::genesis_hash(),
            &[],
            RetentionCheckpointPhase::Prepared,
        )
        .unwrap();
        assert!(SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn utc_day_rotation_occurs_for_an_empty_segment() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-day-rotation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let first_day = 1_700_000_000_000;
        let second_day = first_day + 86_400_000;
        let mut writer = SegmentWriter::open_at(&directory, 1024, 30, first_day).unwrap();
        let first = writer.path().to_path_buf();
        assert_eq!(
            writer.append_at(&sample(), second_day).unwrap(),
            writer.path()
        );
        assert_ne!(first, writer.path());
        assert!(
            writer
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("audit-{}", utc_stamp(second_day)))
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn ordinary_append_does_not_run_a_retention_scan() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-no-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut writer = SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).unwrap();
        for index in 0..=MAX_SEGMENT_SCAN_ENTRIES {
            fs::write(
                directory.join(format!("retention-no-scan-{index}")),
                b"unowned",
            )
            .unwrap();
        }
        assert!(writer.append_at(&sample(), 1_700_000_000_000).is_ok());
        assert!(!writer.retention_degraded());
        let _ = fs::remove_dir_all(directory);
    }
}
