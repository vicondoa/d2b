//! Append-only audit segment rotation and retention.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::record_types::AuditRecord;

/// Default maximum segment size.
pub const DEFAULT_MAX_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
/// Default retention period.
pub const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Append-only segment writer.
#[derive(Debug)]
pub struct SegmentWriter {
    directory: PathBuf,
    file: File,
    path: PathBuf,
    bytes: u64,
    opened_day: u64,
    sequence: u32,
    max_bytes: u64,
    retention_days: u64,
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
        fs::create_dir_all(&directory)?;
        let opened_day = day_number(timestamp_ms);
        let sequence = next_sequence(&directory, timestamp_ms)?;
        let path = directory.join(segment_name(timestamp_ms, sequence));
        let file = open_append(&path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            directory,
            file,
            path,
            bytes,
            opened_day,
            sequence,
            max_bytes: max_bytes.max(1),
            retention_days,
        })
    }

    /// Append a record and rotate before crossing a size or UTC-day boundary.
    pub fn append(&mut self, record: &AuditRecord) -> io::Result<PathBuf> {
        let line = record.to_json_line().map_err(io::Error::other)?;
        let current_day = day_number(now_ms());
        if self.bytes > 0
            && (self.bytes.saturating_add(line.len() as u64) > self.max_bytes
                || current_day != self.opened_day)
        {
            self.rotate(now_ms())?;
        }
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        self.bytes = self.bytes.saturating_add(line.len() as u64);
        Ok(self.path.clone())
    }

    /// Force the current segment to disk.
    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// Current segment path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Remove segments older than the retention floor.
    ///
    /// Only files matching the owned `audit-*.jsonl` shape are considered.
    pub fn prune_old(&self, now_ms: u64) -> io::Result<usize> {
        let floor = now_ms.saturating_sub(self.retention_days.saturating_mul(24 * 60 * 60 * 1000));
        let mut removed = 0;
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with("audit-") || !name.ends_with(".jsonl") || path == self.path {
                continue;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(system_time_ms)
                .unwrap_or(now_ms);
            if modified < floor {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn rotate(&mut self, timestamp_ms: u64) -> io::Result<()> {
        self.file.sync_all()?;
        self.opened_day = day_number(timestamp_ms);
        self.sequence = self.sequence.saturating_add(1);
        self.path = self
            .directory
            .join(segment_name(timestamp_ms, self.sequence));
        self.file = open_append(&self.path)?;
        self.bytes = self.file.metadata()?.len();
        Ok(())
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .mode(0o640)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
}

fn next_sequence(directory: &Path, timestamp_ms: u64) -> io::Result<u32> {
    let prefix = format!("audit-{}", utc_stamp(timestamp_ms));
    let mut max = 0;
    for entry in fs::read_dir(directory)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".jsonl"))
        {
            if let Ok(sequence) = rest.parse::<u32>() {
                max = max.max(sequence);
            }
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
                execution_ref_digest: "sha256:exec".to_owned(),
                process_uid: "uid".to_owned(),
                outcome: "ok".to_owned(),
                exit_class: None,
            }),
        )
        .unwrap()
    }

    #[test]
    fn names_are_owned_and_rotation_is_size_bounded() {
        let directory =
            std::env::temp_dir().join(format!("d2b-audit-segment-{}", std::process::id()));
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
}
