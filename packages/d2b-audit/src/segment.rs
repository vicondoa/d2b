//! Append-only audit segment rotation and retention.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
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
    /// Fail while publishing a retention checkpoint.
    PruneCheckpoint,
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

/// Deterministic fault injector for append and rotation tests.
#[derive(Debug, Clone, Default)]
pub struct FailureInjector {
    append: Arc<AtomicU8>,
    data_sync: Arc<AtomicU8>,
    parent_sync: Arc<AtomicU8>,
    rotation: Arc<AtomicU8>,
    prune_checkpoint: Arc<AtomicU8>,
}

impl FailureInjector {
    /// Arrange for the next operation at `point` to fail.
    pub fn fail_next(&self, point: FailurePoint) {
        let slot = match point {
            FailurePoint::Append => &self.append,
            FailurePoint::DataSync => &self.data_sync,
            FailurePoint::ParentSync => &self.parent_sync,
            FailurePoint::Rotation => &self.rotation,
            FailurePoint::PruneCheckpoint => &self.prune_checkpoint,
        };
        slot.store(1, Ordering::SeqCst);
    }

    fn take(&self, point: FailurePoint) -> bool {
        let slot = match point {
            FailurePoint::Append => &self.append,
            FailurePoint::DataSync => &self.data_sync,
            FailurePoint::ParentSync => &self.parent_sync,
            FailurePoint::Rotation => &self.rotation,
            FailurePoint::PruneCheckpoint => &self.prune_checkpoint,
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
}

impl core::fmt::Debug for SegmentWriter {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SegmentWriter")
            .field("bytes", &self.bytes)
            .field("sequence", &self.sequence)
            .field("max_bytes", &self.max_bytes)
            .field("retention_days", &self.retention_days)
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
        if checkpoint_pending(&directory)? {
            return Err(io::Error::other("audit-retention-checkpoint-pending"));
        }
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
        self.validate_live_inodes()?;
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::Append))
        {
            return Err(io::Error::other("audit-append-injected"));
        }
        let line = record.to_json_line().map_err(io::Error::other)?;
        let current_day = day_number(timestamp_ms);
        if self.bytes > 0
            && (self.bytes.saturating_add(line.len() as u64) > self.max_bytes
                || current_day != self.opened_day)
        {
            self.rotate(timestamp_ms)?;
        }
        let offset = self.file.metadata()?.len();
        if let Err(error) = self.file.write_all(&line) {
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
        self.prune_old(timestamp_ms)?;
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

    /// Remove segments older than the retention floor.
    ///
    /// Only files matching the owned `audit-*.jsonl` shape are considered.
    pub fn prune_old(&self, now_ms: u64) -> io::Result<usize> {
        if self.retention_days == 0 {
            return Ok(0);
        }
        let floor = now_ms.saturating_sub(self.retention_days.saturating_mul(24 * 60 * 60 * 1000));
        let mut candidates = Vec::new();
        let mut budget = RetentionScanBudget { lines: 0, bytes: 0 };
        let cutoff_date = utc_stamp(floor)
            .get(..8)
            .ok_or_else(|| io::Error::other("audit-retention-date-invalid"))?
            .to_owned();
        let mut scanned = 0usize;
        for entry in fs::read_dir(&self.directory)? {
            scanned = scanned.saturating_add(1);
            if scanned > MAX_SEGMENT_SCAN_ENTRIES {
                return Err(io::Error::other("audit-retention-scan-limit"));
            }
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !crate::export::is_segment_name(name) || path == self.path {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io::Error::other("audit-segment-identity-invalid"));
            }
            let segment_date = name
                .get(6..14)
                .ok_or_else(|| io::Error::other("audit-segment-name-invalid"))?;
            if segment_date < cutoff_date.as_str() {
                candidates.push((name.to_owned(), path));
            }
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        let owned_names = owned_segment_names(&self.directory, &mut budget)?;
        let mut prefix = Vec::new();
        for (name, path) in candidates {
            if owned_names.get(prefix.len()).map(String::as_str) != Some(name.as_str()) {
                break;
            }
            prefix.push(path);
        }
        if prefix.is_empty() {
            return Ok(0);
        }

        let mut anchor = checkpoint_anchor(&self.directory)?;
        for path in &prefix {
            anchor = segment_tail_hash(path, &anchor, &mut budget)?;
        }
        if self
            .injector
            .as_ref()
            .is_some_and(|injector| injector.take(FailurePoint::PruneCheckpoint))
        {
            return Err(io::Error::other("audit-prune-checkpoint-injected"));
        }
        write_checkpoint(&self.directory, &anchor, true)?;
        self.directory_file.sync_all()?;
        let mut removed = 0;
        for path in &prefix {
            if fs::symlink_metadata(path).is_ok() {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        self.directory_file.sync_all()?;
        write_checkpoint(&self.directory, &anchor, false)?;
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

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionCheckpoint {
    anchor: AuditHash,
    pending: bool,
}

pub(crate) fn checkpoint_pending(directory: &Path) -> io::Result<bool> {
    let path = checkpoint_path(directory);
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<RetentionCheckpoint>(&bytes)
            .map(|checkpoint| checkpoint.pending)
            .map_err(|_| io::Error::other("audit-retention-checkpoint-invalid")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn checkpoint_anchor(directory: &Path) -> io::Result<AuditHash> {
    let path = checkpoint_path(directory);
    match fs::read(path) {
        Ok(bytes) => {
            let checkpoint = serde_json::from_slice::<RetentionCheckpoint>(&bytes)
                .map_err(|_| io::Error::other("audit-retention-checkpoint-invalid"))?;
            if checkpoint.pending {
                return Err(io::Error::other("audit-retention-checkpoint-pending"));
            }
            Ok(checkpoint.anchor)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(crate::genesis_hash()),
        Err(error) => Err(error),
    }
}

fn write_checkpoint(directory: &Path, anchor: &AuditHash, pending: bool) -> io::Result<()> {
    let path = checkpoint_path(directory);
    let tmp = directory.join("audit-checkpoint.json.next");
    let bytes = serde_json::to_vec(&RetentionCheckpoint {
        anchor: anchor.clone(),
        pending,
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

fn owned_segment_names(
    directory: &Path,
    _budget: &mut RetentionScanBudget,
) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    let mut scanned = 0usize;
    for entry in fs::read_dir(directory)? {
        scanned = scanned.saturating_add(1);
        if scanned > MAX_SEGMENT_SCAN_ENTRIES {
            return Err(io::Error::other("audit-retention-scan-limit"));
        }
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if crate::export::is_segment_name(&name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
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

    #[test]
    fn names_are_owned_and_rotation_is_size_bounded() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    fn durability_failures_never_report_a_successful_append() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    fn pending_retention_checkpoint_fails_closed_on_restart() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("d2b-audit-checkpoint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        write_checkpoint(&directory, &crate::genesis_hash(), true).unwrap();
        assert!(SegmentWriter::open_at(&directory, 1024, 30, 1_700_000_000_000).is_err());
        let _ = fs::remove_dir_all(directory);
    }
}
