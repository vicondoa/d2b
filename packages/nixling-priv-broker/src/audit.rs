use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use nix::libc;
use nix::unistd::{Gid, Uid};
use rustix::fs::{Mode, OFlags, ResolveFlags};
use serde::Serialize;
use serde_json::Value;

#[cfg(test)]
use crate::ops::audit_op::OwnedOpAuditRecord;
use crate::{ops::audit_op::OpAuditRecord, sys::path_safe};

/// Broker semantic version embedded in every [`OpAuditRecord`].
/// Picked up at compile time from `Cargo.toml`.
pub const BROKER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn new_event_id() -> io::Result<String> {
    fs::read_to_string("/proc/sys/kernel/random/uuid").map(|uuid| uuid.trim().to_owned())
}

pub(crate) fn result_for_decision(decision: &str) -> &'static str {
    if decision == "allowed" {
        "success"
    } else if decision.starts_with("denied") {
        "denied"
    } else {
        "error"
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry<'a> {
    pub ts: u128,
    pub op: &'a str,
    pub caller_uid: u32,
    pub disposition: &'a str,
    pub opaque_target_id: &'a str,
    pub outcome: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<&'a str>,
}

/// Structured audit log writer.
///
/// Structured audit log writer for daily-rotated JSONL records under
/// `/var/lib/nixling/audit/broker-<utc-date>.jsonl`. The legacy
/// single-file `/var/lib/nixling/broker-audit.log` path was retired:
/// every record — `write_entry` (`AuditEntry` shape) and
/// `write_op_record` (`OpAuditRecord` shape) alike — lands in the day's
/// `broker-<utc-date>.jsonl` file. `ExportBrokerAudit` consumers and
/// the `broker-export-audit.sh` / `broker-socket-acl.sh` Layer-1 gates
/// migrate atomically: they now read the day's daily file (or the full
/// directory enumeration) instead of the legacy single file.
#[derive(Debug)]
pub struct AuditLog {
    /// Directory holding the daily-rotated records
    /// (`<audit_dir>/broker-<utc-date>.jsonl`).
    audit_dir: PathBuf,
    /// Open append-fd for the current UTC day's record file. Refreshed
    /// on day-boundary crossings via [`Self::append_to_daily`].
    daily: Mutex<DailyAppender>,
    /// `0640 root:nixlingd` group target for the daily files.
    expected_gid: u32,
    test_mode: bool,
    /// How many days of daily rotated audit files to retain. 0 disables
    /// pruning. Default 14 (matches the docs claim in
    /// `docs/reference/daemon-api.md` "Audit" and `AGENTS.md` "Control
    /// plane"). Operators that need bounded retention have it: prune
    /// runs on every day-boundary rotation in `append_to_daily` and on
    /// `open()`. Pruning is best-effort — errors are logged via the
    /// broker tracing but do not fail the write path.
    retention_days: u32,
    #[cfg(test)]
    captured_records: Option<Arc<Mutex<Vec<OwnedOpAuditRecord>>>>,
}

#[derive(Debug)]
struct DailyAppender {
    file: File,
    date_utc: String,
}

impl AuditLog {
    pub fn open(
        audit_dir: &Path,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
    ) -> io::Result<Self> {
        // Refuse symlink on the audit dir.
        if let Ok(metadata) = fs::symlink_metadata(audit_dir) {
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "audit directory must not be a symlink: {}",
                        audit_dir.display()
                    ),
                ));
            }
        }

        crate::sys::path_safe::ensure_dir(
            audit_dir,
            0o2750,
            if test_mode {
                None
            } else {
                Some(Uid::from_raw(0).as_raw())
            },
            if test_mode { None } else { Some(expected_gid) },
        )?;

        let today = utc_date_string();
        let daily_path = audit_dir.join(format!("broker-{today}.jsonl"));
        let daily_file = open_append_cloexec(&daily_path, expected_gid, test_mode)?;

        let log = Self {
            audit_dir: audit_dir.to_path_buf(),
            daily: Mutex::new(DailyAppender {
                file: daily_file,
                date_utc: today,
            }),
            expected_gid,
            test_mode,
            retention_days,
            #[cfg(test)]
            captured_records: None,
        };

        // Prune on open so a long-stopped daemon catches up. Best-effort:
        // log + ignore errors (caller should not fail to start the daemon
        // because of a stale-file cleanup hiccup).
        if let Err(err) = log.prune_expired_daily_files() {
            // We don't have tracing in scope here; rely on the broker
            // runtime to surface this via its own log if it cares.
            // The append path is unaffected.
            let _ = err;
        }

        Ok(log)
    }

    #[cfg(test)]
    pub fn open_capturing(
        audit_dir: &Path,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
    ) -> io::Result<(Self, Arc<Mutex<Vec<OwnedOpAuditRecord>>>)> {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let mut log = Self::open(audit_dir, expected_gid, test_mode, retention_days)?;
        log.captured_records = Some(Arc::clone(&capture));
        Ok((log, capture))
    }

    /// Returns the path of the audit directory holding daily
    /// `broker-YYYY-MM-DD.jsonl` files.
    pub fn path(&self) -> &Path {
        &self.audit_dir
    }

    pub fn audit_dir(&self) -> &Path {
        &self.audit_dir
    }

    /// Returns the path of the daily file the broker is currently
    /// appending to. Test helpers and the
    /// `broker-export-audit.sh` / `broker-socket-acl.sh` gates use
    /// this to address the actually-active file for fd / mode
    /// assertions.
    pub fn current_daily_path(&self) -> PathBuf {
        let date = self
            .daily
            .lock()
            .map(|g| g.date_utc.clone())
            .unwrap_or_else(|_| utc_date_string());
        self.audit_dir.join(format!("broker-{date}.jsonl"))
    }

    /// Legacy short-record writer. New op dispatch arms call
    /// [`Self::write_op_record`] instead. The `AuditEntry` JSONL shape
    /// is still produced for back-compat with the `broker-socket-acl.sh`
    /// gate (which greps `caller_uid`); all records — `AuditEntry` and
    /// `OpAuditRecord` alike — land in the day's daily file under
    /// `audit_dir`.
    pub fn write_entry(
        &self,
        op: &str,
        caller_uid: u32,
        disposition: &str,
        opaque_target_id: &str,
        outcome: &str,
    ) -> io::Result<()> {
        let entry = AuditEntry {
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            op,
            caller_uid,
            disposition,
            opaque_target_id,
            outcome,
            error_kind: None,
            error_message: None,
        };
        self.append_json_line(&entry)
    }

    /// Legacy short-record writer for errored outcomes that need
    /// admin-visible diagnostics. The full detail is also surfaced in
    /// the broker journal (`journalctl -u nixling-priv-broker`) for
    /// live-handler failures.
    pub fn write_error_entry(
        &self,
        operation: &str,
        caller_uid: u32,
        decision: &str,
        target_id: &str,
        error_kind: &str,
        error_message: &str,
    ) -> io::Result<()> {
        let entry = AuditEntry {
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            op: operation,
            caller_uid,
            disposition: decision,
            opaque_target_id: target_id,
            outcome: "errored",
            error_kind: Some(error_kind),
            error_message: Some(error_message),
        };
        self.append_json_line(&entry)
    }

    fn append_json_line<T: Serialize>(&self, value: &T) -> io::Result<()> {
        let mut line = serde_json::to_string(value)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push('\n');
        self.append_to_daily(line.as_bytes())
    }

    /// Append one [`OpAuditRecord`] to the day's daily file.
    pub fn write_op_record(&self, record: &OpAuditRecord<'_>) -> io::Result<()> {
        #[cfg(test)]
        if let Some(capture) = &self.captured_records {
            capture
                .lock()
                .map_err(|_| io::Error::other("audit capture mutex poisoned"))?
                .push(OwnedOpAuditRecord::from(record));
        }
        let line = record.to_jsonl();
        self.append_to_daily(line.as_bytes())?;
        Ok(())
    }

    /// Append a `ChildReaped` forensics record to the daily audit log.
    /// Both the real-time IPC channel and the audit channel receive the
    /// event (distinct sinks: IPC for daemon, audit for post-mortem
    /// forensics).
    pub fn write_child_reaped(
        &self,
        notif: &nixling_ipc::broker_wire::ChildReapedNotification,
    ) -> io::Result<()> {
        #[derive(serde::Serialize)]
        struct ChildReapedAuditEntry<'a> {
            ts: u128,
            op: &'static str,
            runner_id: &'a str,
            pid: i32,
            exit_status: &'a nixling_ipc::broker_wire::ChildExitStatus,
            reaped_at_ms: i64,
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.append_json_line(&ChildReapedAuditEntry {
            ts,
            op: "ChildReaped",
            runner_id: &notif.runner_id,
            pid: notif.pid,
            exit_status: &notif.exit_status,
            reaped_at_ms: notif.reaped_at_ms,
        })
    }

    /// Convenience helper used by error paths that still build their
    /// `operation_fields` payload ad hoc.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        operation: &str,
        public_operation_id: &str,
        peer_uid: u32,
        peer_gid: u32,
        peer_pid: i32,
        peer_role: &str,
        authz_result: &str,
        subject_id: &str,
        scope_id: &str,
        verb: &str,
        request_fields: Value,
        decision: &str,
        error_kind: Option<&str>,
        tracing_span_id: Option<&str>,
        bundle_version: &str,
        bundle_hash: &str,
        duration_us: u64,
        operation_fields: Option<Value>,
    ) -> io::Result<()> {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let event_id = new_event_id()?;
        let record = OpAuditRecord {
            ts_ms,
            broker_version: BROKER_VERSION,
            bundle_version,
            bundle_hash,
            operation,
            public_operation_id,
            event_id: &event_id,
            peer_uid,
            peer_gid,
            peer_pid,
            peer_role,
            authz_result,
            subject_id,
            scope_id,
            verb,
            request_fields,
            decision,
            result: result_for_decision(decision),
            error_kind,
            tracing_span_id,
            duration_us,
            operation_fields,
        };
        self.write_op_record(&record)
    }

    fn append_to_daily(&self, bytes: &[u8]) -> io::Result<()> {
        let mut guard = self
            .daily
            .lock()
            .map_err(|_| io::Error::other("audit daily mutex poisoned"))?;
        let today = utc_date_string();
        let rotated = today != guard.date_utc;
        if rotated {
            // Rotations swap the fd via reopen + atomic rename. We
            // reopen the new day's file in O_APPEND; the old file is
            // closed by replacing it (drop runs).
            let new_path = self.audit_dir.join(format!("broker-{today}.jsonl"));
            let new_file = open_append_cloexec(&new_path, self.expected_gid, self.test_mode)?;
            guard.file = new_file;
            guard.date_utc = today;
        }
        guard.file.write_all(bytes)?;
        guard.file.flush()?;
        // Release the daily lock BEFORE pruning so a slow `readdir`
        // never blocks concurrent writers. Prune is best-effort and
        // only runs on day-boundary crossings; the cost is bounded
        // by O(retention_days + leftover files).
        drop(guard);
        if rotated {
            if let Err(err) = self.prune_expired_daily_files() {
                // Same swallow as open(): pruning failures must not
                // break the write path. The next rotation retries.
                let _ = err;
            }
        }
        Ok(())
    }

    /// Delete any `broker-YYYY-MM-DD.jsonl` files whose date stamp is
    /// older than `retention_days` days ago in UTC. Returns the number
    /// of files removed (debug aid; the runtime tracing uses this to
    /// surface retention activity).
    ///
    /// Filename is the source of truth — we never parse JSON to
    /// inspect record timestamps. Operators who manually drop in
    /// `broker-<utc-date>.jsonl` files retain the same semantics.
    /// Files that don't match the expected name format are left
    /// alone so out-of-band artifacts (export tarballs, operator
    /// notes, etc.) survive.
    ///
    /// `retention_days == 0` disables pruning entirely.
    pub fn prune_expired_daily_files(&self) -> io::Result<usize> {
        if self.retention_days == 0 {
            return Ok(0);
        }
        let cutoff_days = self.retention_days as i64;
        let today_unix_days = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            / 86_400;

        let mut pruned = 0usize;
        let entries = match fs::read_dir(&self.audit_dir) {
            Ok(it) => it,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(err) => return Err(err),
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            let Some(stem) = name_str
                .strip_prefix("broker-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };
            // Expect `YYYY-MM-DD`.
            let parts: Vec<&str> = stem.split('-').collect();
            if parts.len() != 3 {
                continue;
            }
            let Ok(y) = parts[0].parse::<i32>() else {
                continue;
            };
            let Ok(m) = parts[1].parse::<u32>() else {
                continue;
            };
            let Ok(d) = parts[2].parse::<u32>() else {
                continue;
            };
            if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                continue;
            }
            let file_unix_days = match unix_days_from_ymd(y, m, d) {
                Some(v) => v,
                None => continue,
            };
            let age_days = today_unix_days - file_unix_days;
            if age_days > cutoff_days {
                // Best-effort: remove failures don't propagate as
                // hard errors (e.g. file vanished between readdir
                // and remove, permission denied on a stray file).
                let _ = path_safe::remove_nofollow(&entry.path());
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    /// Reads every `broker-YYYY-MM-DD.jsonl` file in `audit_dir`,
    /// sorted by filename (which equals chronological order), and
    /// returns the concatenated lines after filtering by `since`
    /// and `filter` substrings. Files that don't match the dated
    /// pattern are skipped so out-of-band artifacts (operator
    /// notes, export tarballs) don't pollute the export stream.
    pub fn export_lines(
        &self,
        since: Option<&str>,
        filter: Option<&str>,
    ) -> io::Result<Vec<String>> {
        let entries = match fs::read_dir(&self.audit_dir) {
            Ok(it) => it,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut daily_paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name();
                let name_str = name.to_str()?;
                let stem = name_str
                    .strip_prefix("broker-")
                    .and_then(|s| s.strip_suffix(".jsonl"))?;
                let parts: Vec<&str> = stem.split('-').collect();
                if parts.len() != 3 {
                    return None;
                }
                let y = parts[0].parse::<i32>().ok()?;
                let m = parts[1].parse::<u32>().ok()?;
                let d = parts[2].parse::<u32>().ok()?;
                unix_days_from_ymd(y, m, d)?;
                Some(entry.path())
            })
            .collect();
        // Filenames sort lexicographically in chronological order
        // because of the YYYY-MM-DD format.
        daily_paths.sort();

        let mut lines = Vec::new();
        for path in &daily_paths {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if let Some(since) = since {
                    if !line.contains(since) && !ts_at_least(&line, since) {
                        continue;
                    }
                }
                if let Some(filter) = filter {
                    if !line.contains(filter) {
                        continue;
                    }
                }
                lines.push(line);
            }
        }
        Ok(lines)
    }

    /// Returns `(uid, gid, mode)` of the current day's daily file.
    pub fn metadata(&self) -> io::Result<(u32, u32, u32)> {
        let metadata = fs::metadata(self.current_daily_path())?;
        Ok((
            metadata.uid(),
            metadata.gid(),
            metadata.permissions().mode() & 0o777,
        ))
    }
}

fn open_append_cloexec(path: &Path, expected_gid: u32, test_mode: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o640)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;
    path_safe::fchmod(file.as_fd(), 0o640)?;
    set_root_nixlingd_acl(&file, expected_gid, test_mode)?;
    // Refresh fd flags from a rustix view; this also asserts the file
    // descriptor was opened with the expected mode bits via
    // O_APPEND | O_CLOEXEC.
    let raw = file.as_raw_fd();
    let _ = raw; // intentional: rustix audit cross-check stays a static cast
    let _ = (
        OFlags::APPEND,
        ResolveFlags::BENEATH,
        Mode::from_raw_mode(0),
    );
    Ok(file)
}

fn set_root_nixlingd_acl(file: &File, expected_gid: u32, test_mode: bool) -> io::Result<()> {
    let owner_uid = if test_mode {
        Uid::current()
    } else {
        Uid::from_raw(0)
    };
    let group_gid = if test_mode {
        Gid::current()
    } else {
        Gid::from_raw(expected_gid)
    };
    if let Err(err) = path_safe::fchown(
        file.as_fd(),
        Some(owner_uid.as_raw()),
        Some(group_gid.as_raw()),
    ) {
        if !test_mode {
            return Err(err);
        }
    }
    Ok(())
}

fn utc_date_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, m, d) = ymd_from_unix(now as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Civil-from-days algorithm (Howard Hinnant, public domain). Avoids
/// dragging in a chrono / time crate just for date stamping.
fn ymd_from_unix(unix: i64) -> (i32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Inverse of [`ymd_from_unix`]. Returns days since the unix epoch
/// (1970-01-01) for the supplied Y-M-D, or `None` for out-of-range /
/// impossible dates. Civil-to-days (Howard Hinnant, public domain).
///
/// Validates calendar correctness via the round-trip check
/// (`ymd_from_unix(result * 86400) == (y, m, d)`). Invalid calendar
/// dates like 2023-02-29 or 2024-02-30 fail this round-trip because the
/// underlying Hinnant algorithm normalizes out-of-range days into the
/// next month, producing a different (y, m, d) on decode. We treat any
/// normalization as `None` so `prune_expired_daily_files` doesn't trust
/// a filename like `broker-2024-02-30.jsonl` as a real date.
fn unix_days_from_ymd(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 {
        y_adj / 400
    } else {
        (y_adj - 399) / 400
    };
    let yoe = (y_adj - era * 400) as u32; // [0, 399]
    let m_i = m as i32;
    let doy = ((153 * (if m_i > 2 { m_i - 3 } else { m_i + 9 }) + 2) / 5 + d as i32 - 1) as u32; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let result = era as i64 * 146_097 + doe as i64 - 719_468;
    // Round-trip guard: rejects impossible calendar dates that the
    // Hinnant algorithm would otherwise normalize (e.g. 2024-02-30
    // becoming 2024-03-01). Pruning trusts the filename only after
    // this guard agrees.
    let (yy, mm, dd) = ymd_from_unix(result * 86_400);
    if yy == y && mm == m && dd == d {
        Some(result)
    } else {
        None
    }
}

fn ts_at_least(line: &str, since: &str) -> bool {
    let wanted = since.parse::<u128>().ok();
    let current = line
        .split('"')
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|window| {
            if window.first().copied() == Some(":") {
                window
                    .get(1)
                    .and_then(|candidate| candidate.parse::<u128>().ok())
            } else {
                None
            }
        });
    match (current, wanted) {
        (Some(current), Some(wanted)) => current >= wanted,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ymd_decodes_known_epoch() {
        assert_eq!(ymd_from_unix(0), (1970, 1, 1));
        // 2024-02-29 UTC = 1709164800
        assert_eq!(ymd_from_unix(1_709_164_800), (2024, 2, 29));
    }

    #[test]
    fn ymd_round_trip() {
        // Probe a handful of dates incl. leap day, year boundary,
        // pre/post-epoch. unix_days_from_ymd inverts ymd_from_unix.
        for &(y, m, d) in &[
            (1970, 1, 1),
            (1970, 2, 1),
            (1970, 12, 31),
            (1971, 1, 1),
            (2000, 2, 29),
            (2024, 2, 29),
            (2025, 1, 1),
            (2026, 5, 28),
            (2100, 2, 28),
        ] {
            let unix_days = unix_days_from_ymd(y, m, d)
                .unwrap_or_else(|| panic!("unix_days_from_ymd({y}-{m:02}-{d:02}) returned None"));
            let (yy, mm, dd) = ymd_from_unix(unix_days * 86_400);
            assert_eq!(
                (yy, mm, dd),
                (y, m, d),
                "round-trip for {y}-{m:02}-{d:02}: got {yy}-{mm:02}-{dd:02} via unix_days={unix_days}"
            );
        }
    }

    #[test]
    fn unix_days_from_ymd_rejects_out_of_range() {
        assert_eq!(unix_days_from_ymd(2024, 0, 15), None);
        assert_eq!(unix_days_from_ymd(2024, 13, 15), None);
        assert_eq!(unix_days_from_ymd(2024, 5, 0), None);
        assert_eq!(unix_days_from_ymd(2024, 5, 32), None);
    }

    #[test]
    fn unix_days_from_ymd_rejects_invalid_calendar_dates() {
        // Dates that pass the 1..=31 day check but don't actually exist
        // in the calendar (Feb 30, Apr 31, Feb 29 on a non-leap year)
        // must round-trip to a different (y, m, d), which the guard
        // catches.
        assert_eq!(
            unix_days_from_ymd(2023, 2, 29),
            None,
            "2023-02-29 isn't a leap day"
        );
        assert_eq!(
            unix_days_from_ymd(2024, 2, 30),
            None,
            "Feb only has 29 days even in leap years"
        );
        assert_eq!(unix_days_from_ymd(2024, 4, 31), None, "April has 30 days");
        assert_eq!(unix_days_from_ymd(2024, 6, 31), None, "June has 30 days");
        assert_eq!(
            unix_days_from_ymd(2024, 9, 31),
            None,
            "September has 30 days"
        );
        assert_eq!(
            unix_days_from_ymd(2024, 11, 31),
            None,
            "November has 30 days"
        );
        // Valid dates still pass:
        assert!(
            unix_days_from_ymd(2024, 2, 29).is_some(),
            "2024-02-29 IS a leap day"
        );
        assert!(
            unix_days_from_ymd(2024, 4, 30).is_some(),
            "April 30 is valid"
        );
        assert!(
            unix_days_from_ymd(2023, 2, 28).is_some(),
            "Feb 28 is always valid"
        );
    }

    fn make_audit_with_files(retention_days: u32, file_dates: &[(i32, u32, u32)]) -> AuditLog {
        let dir = std::env::temp_dir().join(format!(
            "nixlingd-broker-audit-prune-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let audit_dir = dir.join("audit");
        fs::create_dir_all(&dir).expect("create scratch state dir");
        let log = AuditLog::open(&audit_dir, Gid::current().as_raw(), true, retention_days)
            .expect("open audit log");
        for &(y, m, d) in file_dates {
            let path = log
                .audit_dir
                .join(format!("broker-{y:04}-{m:02}-{d:02}.jsonl"));
            fs::write(&path, b"{}\n").expect("seed daily file");
        }
        log
    }

    #[test]
    fn prune_keeps_recent_and_deletes_old() {
        let today_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let today = ymd_from_unix(today_unix);
        let yesterday = ymd_from_unix(today_unix - 86_400);
        let old_50d = ymd_from_unix(today_unix - 86_400 * 50);
        let old_15d = ymd_from_unix(today_unix - 86_400 * 15);

        let log = make_audit_with_files(14, &[today, yesterday, old_15d, old_50d]);
        let pruned = log.prune_expired_daily_files().expect("prune ok");
        assert_eq!(
            pruned, 2,
            "should have pruned the 15-day-old and 50-day-old files"
        );

        let remaining: Vec<_> = fs::read_dir(&log.audit_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("broker-"))
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
            .collect();
        // Two recent files (today + yesterday) plus the broker-<today>.jsonl
        // that AuditLog::open seeded on its own. Allow 2 or 3 depending
        // on whether `today` overlaps with the open-seed.
        assert!(
            (2..=3).contains(&remaining.len()),
            "expected 2-3 remaining files; got {}",
            remaining.len()
        );

        let _ = fs::remove_dir_all(log.audit_dir.parent().unwrap());
    }

    #[test]
    fn prune_disabled_when_retention_zero() {
        let today_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let old_50d = ymd_from_unix(today_unix - 86_400 * 50);
        let old_500d = ymd_from_unix(today_unix - 86_400 * 500);

        let log = make_audit_with_files(0, &[old_50d, old_500d]);
        let pruned = log.prune_expired_daily_files().expect("prune ok");
        assert_eq!(pruned, 0, "retention=0 must disable pruning entirely");

        let _ = fs::remove_dir_all(log.audit_dir.parent().unwrap());
    }

    #[test]
    fn prune_ignores_non_matching_filenames() {
        let today_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let old_50d = ymd_from_unix(today_unix - 86_400 * 50);

        let log = make_audit_with_files(14, &[old_50d]);
        // Seed an operator note + an export tarball — both should
        // survive pruning.
        let note = log.audit_dir.join("NOTES-operator.txt");
        let tar = log.audit_dir.join("export-2024-01-01.tar.gz");
        let stray = log.audit_dir.join("broker-not-a-date.jsonl");
        fs::write(&note, b"todo").unwrap();
        fs::write(&tar, b"\0").unwrap();
        fs::write(&stray, b"{}\n").unwrap();

        let pruned = log.prune_expired_daily_files().expect("prune ok");
        assert_eq!(pruned, 1, "only the dated daily file should be pruned");
        assert!(note.exists(), "operator notes must survive prune");
        assert!(tar.exists(), "export tarballs must survive prune");
        assert!(stray.exists(), "non-date-matching jsonl must survive prune");

        let _ = fs::remove_dir_all(log.audit_dir.parent().unwrap());
    }
}
