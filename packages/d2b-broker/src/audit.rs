use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::libc;
use nix::unistd::{Gid, Uid};
use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock};
use serde::Serialize;
use serde_json::Value;


#[cfg(test)]
use crate::ops::audit_op::OwnedOpAuditRecord;
use crate::{
    ops::audit_op::{BrokerAuditRecordClass, OpAuditRecord},
    sys::path_safe,
};
use d2b_audit::evidence_chain::ChainRecord;
use d2b_contracts_broker::broker_wire::{
    AuditExportCursor, AuditExportEntry, AuditExportErrorCode, BrokerAuditFilter,
    BrokerAuditSeverity, ExportBrokerAuditResponse,
};

/// Broker semantic version embedded in every [`OpAuditRecord`].
/// Picked up at compile time from `Cargo.toml`.
pub const BROKER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
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

const DEFAULT_AUDIT_WRITES_PER_SECOND: u32 = 4096;
const AUDIT_WRITE_WINDOW: Duration = Duration::from_secs(1);
/// The bound on admitted-but-unstarted audit worker commands (plan U6: the
/// append queue is bounded; a full queue drops unprivileged appends with
/// accounting and backpressures privileged ones).
const AUDIT_WORKER_QUEUE_DEPTH: usize = 256;
const MAX_EXPORTED_AUDIT_BYTES: usize = 768 * 1024;
const MAX_EXPORTED_AUDIT_LINE_BYTES: usize = 64 * 1024;
const MAX_EXPORTED_AUDIT_PAGE_RECORDS: u32 = 1024;
const MAX_LEGACY_EXPORT_RECORDS: usize = 16 * 1024;
const MAX_LEGACY_EXPORT_BYTES: usize = 512 * 1024;
const MAX_AUDIT_DIRECTORY_ENTRIES: usize = 4096;
const AUDIT_RECONCILE_CHUNK_BYTES: usize = 8 * 1024;
const MAX_QUARANTINE_NAME_ATTEMPTS: usize = 64;
const MAX_EXPORTED_AUDIT_DISCARD_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditWriteClass {
    Privileged,
    Unprivileged,
}

impl AuditWriteClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Privileged => "privileged",
            Self::Unprivileged => "unprivileged",
        }
    }
}

/// Aggregated audit-drop counts: how many privileged and unprivileged
/// records were rate-limited rather than durably written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuditDropSummary {
    /// Privileged-class records dropped by the rate limiter.

    pub privileged_rate_limited: u64,
    /// Unprivileged-class records dropped by the rate limiter.



    pub unprivileged_rate_limited: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuditDropWarning {
    dropped_total: u64,
    dropped_since_previous_warning: u64,
}

#[derive(Debug, Clone, Default)]
struct AuditDropWarningState {
    privileged_reported: u64,
    unprivileged_reported: u64,
}

impl AuditDropWarningState {
    fn observe(
        &mut self,
        audit_class: AuditWriteClass,
        dropped_total: u64,
    ) -> Option<AuditDropWarning> {
        if dropped_total == 0 || !dropped_total.is_power_of_two() {
            return None;
        }
        let previous = match audit_class {
            AuditWriteClass::Privileged => &mut self.privileged_reported,
            AuditWriteClass::Unprivileged => &mut self.unprivileged_reported,
        };
        let warning = AuditDropWarning {
            dropped_total,
            dropped_since_previous_warning: dropped_total.saturating_sub(*previous),
        };
        *previous = dropped_total;
        Some(warning)
    }
}

/// One legacy JSONL audit record, consumed by the socket-acl gate:
/// `ts` / `op` identify the operation, `disposition` the authz outcome class
/// (`allowed` / `denied-*` / `errored`), `outcome` the finer result
/// spelling, and the optional error fields carry the failure detail.
#[derive(Clone)]
pub struct AuditEntry<'a> {
    /// Monotonic timestamp, microseconds since an arbitrary epoch.

    pub ts: u128,
    /// The audited operation name.

    pub op: &'a str,
    /// The caller's uid.


    pub caller_uid: u32,
    /// The caller's gid, when known.



    pub caller_gid: Option<u32>,
    /// The authz outcome class.



    pub disposition: &'a str,
    /// The opaque target operation id, when the operation names one.



    pub opaque_target_id: &'a str,
    /// The finer result spelling (`ok`/refusal/error kind).)



    pub outcome: &'a str,
    /// The error kind, when the outcome is an error.



    pub error_kind: Option<&'a str>,
    /// The error detail, when present.


    pub error_message: Option<&'a str>,
}

impl Serialize for AuditEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut value = serde_json::Map::from_iter([
            ("ts".to_owned(), serde_json::json!(self.ts)),
            ("op".to_owned(), serde_json::json!(self.op)),
            ("caller_uid".to_owned(), serde_json::json!(self.caller_uid)),
            (
                "disposition".to_owned(),
                serde_json::json!(self.disposition),
            ),
            (
                "opaque_target_id".to_owned(),
                serde_json::json!(self.opaque_target_id),
            ),
            ("outcome".to_owned(), serde_json::json!(self.outcome)),
        ]);
        if let Some(caller_gid) = self.caller_gid {
            value.insert("caller_gid".to_owned(), serde_json::json!(caller_gid));
        }
        if let Some(error_kind) = self.error_kind {
            value.insert("error_kind".to_owned(), serde_json::json!(error_kind));
        }
        if let Some(error_message) = self.error_message {
            value.insert("error_message".to_owned(), serde_json::json!(error_message));
        }
        sanitize_audit_value(serde_json::Value::Object(value)).serialize(serializer)
    }
}

impl core::fmt::Debug for AuditEntry<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuditEntry(<redacted>)")
    }
}

/// Structured audit log writer.
///
/// Structured audit log writer for daily-rotated JSONL records under
/// `/var/lib/d2b/audit/broker-<utc-date>.jsonl`. The legacy
/// single-file `/var/lib/d2b/broker-audit.log` path was retired:
/// every record - `write_entry` (`AuditEntry` shape) and
/// `write_op_record` (`OpAuditRecord` shape) alike - lands in the day's
/// `broker-<utc-date>.jsonl` file. `ExportBrokerAudit` consumers and
/// the `broker-export-audit.sh` / `broker-socket-acl.sh` Layer-1 gates
/// migrate atomically: they now read the day's daily file (or the full
/// directory enumeration) instead of the legacy single file. Every
/// append is rolled back to its pre-append offset after a write, flush,
/// or synchronization error; if rollback cannot be synchronized, the
/// writer is poisoned until a fresh open. A truncated final line found
/// on any owned daily file is quarantined before pruning or export.
pub struct AuditLog {
    /// Directory holding the daily-rotated records
    /// (`<audit_dir>/broker-<utc-date>.jsonl`).
    audit_dir: PathBuf,
    /// `0640 root:d2bd` group target for the daily files.
    expected_gid: u32,
    test_mode: bool,
    /// How many days of daily rotated audit files to retain. 0 disables
    /// pruning. Default 30 (matches the docs claim in
    /// `docs/reference/daemon-api.md` "Audit" and `AGENTS.md` "Control
    /// plane"). Operators that need bounded retention have it: prune
    /// runs on every day-boundary rotation in `append_to_daily` and on
    /// `open()`. Pruning is best-effort - errors are logged via the
    /// broker tracing but do not fail the write path.
    retention_days: u32,
    /// The bounded channel to the dedicated audit worker (plan U6 / R4):
    /// the worker owns the daily appender, limiter, drop accounting, and
    /// the directory lock, and executes every append transaction serially.
    writer: AuditWriter,
    #[cfg(test)]
    captured_records: Option<Arc<Mutex<Vec<OwnedOpAuditRecord>>>>,
}

impl core::fmt::Debug for AuditLog {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuditLog")
            .field("retention_days", &self.retention_days)
            .field("test_mode", &self.test_mode)
            .finish()
    }
}

/// The caller-side handle of the dedicated audit worker.
///
/// The worker owns limiter-check, drop-accounting, write, fsync,
/// rollback/poison, and rotation+prune as one serialized FIFO unit, so a
/// caller cancelled mid-append can never leak a partial JSONL line and a
/// record is fsynced before the next write starts (crash preserves a
/// prefix). The queue is bounded; a full queue drops unprivileged appends
/// with accounting (matching [`AuditDropSummary`]) and backpressures
/// privileged appends (the mutation-success boundary never drops).
struct AuditWriter {
    commands: SyncSender<AuditCommand>,
    /// Caller-side queue-full drop accounting (the worker cannot see an
    /// append that never reached it). Merged into [`AuditDropSummary`] on
    /// read.
    dropped_privileged: AtomicU64,
    dropped_unprivileged: AtomicU64,
}

/// One serialized command the audit worker executes in FIFO order.
enum AuditCommand {
    /// Open the directory lock, reconcile owned daily files, open the
    /// current day's appender, and prune - the whole `open()` barrier on
    /// the worker, so a fresh writer is usable only after the reply.
    Bootstrap {
        audit_dir: PathBuf,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
        #[cfg(test)]
        capture: Option<Arc<Mutex<Vec<OwnedOpAuditRecord>>>>,
        reply: SyncSender<io::Result<()>>,
    },
    /// One append transaction: limiter-check, rotation, write, flush,
    /// fsync, directory sync, rollback/poison on failure, then prune.
    Append {
        audit_class: AuditWriteClass,
        operation: String,
        bytes: Vec<u8>,
        #[cfg(test)]
        capture_record: Option<OwnedOpAuditRecord>,
        reply: SyncSender<io::Result<()>>,
    },
    ExportPage {
        since: Option<String>,
        filter: Option<String>,
        cursor: Option<AuditExportCursor>,
        limit: u32,
        reply: SyncSender<io::Result<ExportBrokerAuditResponse>>,
    },
    Prune {
        reply: SyncSender<io::Result<usize>>,
    },
    DropSummary {
        reply: SyncSender<AuditDropSummary>,
    },
    #[cfg(test)]
    DropWarningState {
        reply: SyncSender<AuditDropWarningState>,
    },
    CurrentDailyPath {
        reply: SyncSender<PathBuf>,
    },
    Metadata {
        reply: SyncSender<io::Result<(u32, u32, u32)>>,
    },
    #[cfg(test)]
    SetWriteLimit {
        writes_per_second: u32,
        /// Opt-in per test: freeze the rate-limit window (deterministic
        /// assertions); `false` keeps the real clock.
        freeze_window: bool,
        reply: SyncSender<io::Result<()>>,
    },
    #[cfg(test)]
    InjectIoFailure {
        failure: InjectedAuditIoFailure,
        reply: SyncSender<io::Result<()>>,
    },
    /// Arm a crash between two records (U6 invariant test): the next append
    /// writes a partial line and the worker exits without sync, rollback, or
    /// reply - the durable prefix is all that survives a restart.
    #[cfg(test)]
    InjectCrashAfterWrite {
        reply: SyncSender<io::Result<()>>,
    },
    Shutdown {
        exited: SyncSender<()>,
    },
}

/// The audit worker's whole state: every field below is touched only on the
/// worker thread, so the append transaction never tears.
struct AuditWorkerState {
    audit_dir: PathBuf,
    expected_gid: u32,
    test_mode: bool,
    retention_days: u32,
    /// Exclusive directory ownership lock held for the lifetime of the
    /// worker. Reconciliation, pruning, export, and append therefore share
    /// one cross-process mutation boundary; the lock releases only when the
    /// worker exits (after the Shutdown ack).
    directory_lock: File,
    /// Open append-fd for the current UTC day's record file. Refreshed on
    /// day-boundary crossings.
    daily: DailyAppender,
    write_limiter: AuditWriteLimiter,
    drop_summary: AuditDropSummary,
    drop_warning_state: AuditDropWarningState,
    #[cfg(test)]
    capture: Option<Arc<Mutex<Vec<OwnedOpAuditRecord>>>>,
    #[cfg(test)]
    crash_after_write: bool,
}

#[derive(Debug)]
struct DailyAppender {
    file: File,
    date_utc: String,
    poisoned: bool,
    #[cfg(test)]
    io_failure: Option<InjectedAuditIoFailure>,
}

#[cfg(test)]
#[derive(Debug)]
enum InjectedAuditIoFailure {
    PartialWrite,
    Flush,
    Sync { remaining: u32 },
    /// Park the worker inside the append transaction until the test releases
    /// it: the caller can then cancel (drop its reply) mid-append and prove
    /// the worker owns the append to completion.
    Stall {
        stalled: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    },
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
impl DailyAppender {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        {
            let stalling = matches!(self.io_failure, Some(InjectedAuditIoFailure::Stall { .. }));
            if stalling {
                let Some(InjectedAuditIoFailure::Stall { stalled, release }) =
                    self.io_failure.take()
                else {
                    unreachable!("stalling implies the stall variant")
                };
                let _ = stalled.send(());
                let _ = release.blocking_recv();
            }
        }
        #[cfg(test)]
        if matches!(self.io_failure, Some(InjectedAuditIoFailure::PartialWrite)) {
            self.io_failure = None;
            let partial_len = (bytes.len() / 2).max(1).min(bytes.len());
            self.file.write_all(&bytes[..partial_len])?;
            return Err(io::Error::other("injected-audit-write-failure"));
        }
        self.file.write_all(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        #[cfg(test)]
        if matches!(self.io_failure, Some(InjectedAuditIoFailure::Flush)) {
            self.io_failure = None;
            return Err(io::Error::other("injected-audit-flush-failure"));
        }
        self.file.flush()
    }

    fn sync_all(&mut self) -> io::Result<()> {
        #[cfg(test)]
        if let Some(InjectedAuditIoFailure::Sync { remaining }) = &self.io_failure
            && *remaining > 0
        {
            self.io_failure = (*remaining > 1).then_some(InjectedAuditIoFailure::Sync {
                remaining: remaining - 1,
            });
            return Err(io::Error::other("injected-audit-sync-failure"));
        }
        self.file.sync_all()
    }

    fn rollback_to(&mut self, offset: u64, audit_dir: &Path) -> io::Result<()> {
        self.file.set_len(offset)?;
        self.sync_all()?;
        sync_directory(audit_dir)
    }
}

impl AuditLog {
    /// Open the broker's audit log under `audit_dir`, the daemon's entry
    /// point: runs the full bootstrap/poison barrier (symlink refusal,
    /// directory lock, reconciliation, appender setup, prune) on the
    /// worker before returning, so a fresh writer never observes a
    /// half-opened directory.

    pub fn open(
        audit_dir: &Path,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
    ) -> io::Result<Self> {
        Self::open_inner(
            audit_dir,
            expected_gid,
            test_mode,
            retention_days,
            #[cfg(test)]
            None,
        )
    }

    /// Spawn the audit worker and run the whole `open()` barrier on it:
    /// symlink refusal, directory lock, reconciliation of every owned daily
    /// file, the current day's appender, and prune-on-open. The store is
    /// usable only after the bootstrap reply, so a fresh writer can never
    /// observe a half-opened directory.
    fn open_inner(
        audit_dir: &Path,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
        #[cfg(test)]
        capture: Option<Arc<Mutex<Vec<OwnedOpAuditRecord>>>>,
    ) -> io::Result<Self> {
        let (commands, receiver) = mpsc::sync_channel::<AuditCommand>(AUDIT_WORKER_QUEUE_DEPTH);
        std::thread::Builder::new()
            .name("d2b-broker-audit-writer".to_owned())
            .spawn(move || audit_worker_loop(receiver))
            .map_err(|error| io::Error::other(format!("audit worker spawn failed: {error}")))?;
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        commands
            .send(AuditCommand::Bootstrap {
                audit_dir: audit_dir.to_path_buf(),
                expected_gid,
                test_mode,
                retention_days,
                #[cfg(test)]
                capture: capture.clone(),
                reply: reply_tx,
            })
            .map_err(|_| io::Error::other("audit worker unavailable"))?;
        #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
        reply_rx
            .recv()
            .map_err(|_| io::Error::other("audit worker unavailable"))??;
        Ok(Self {
            audit_dir: audit_dir.to_path_buf(),
            expected_gid,
            test_mode,
            retention_days,
            writer: AuditWriter {
                commands,
                dropped_privileged: AtomicU64::new(0),
                dropped_unprivileged: AtomicU64::new(0),
            },
            #[cfg(test)]
            captured_records: capture,
        })
    }

    #[cfg(test)]
    pub fn open_capturing(
        audit_dir: &Path,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
    ) -> io::Result<(Self, Arc<Mutex<Vec<OwnedOpAuditRecord>>>)> {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let log = Self::open_inner(
            audit_dir,
            expected_gid,
            test_mode,
            retention_days,
            Some(Arc::clone(&capture)),
        )?;
        Ok((log, capture))
    }

    #[cfg(test)]
    pub fn open_with_write_limit(
        audit_dir: &Path,
        expected_gid: u32,
        test_mode: bool,
        retention_days: u32,
        writes_per_second: u32,
        freeze_window: bool,
    ) -> io::Result<Self> {
        // Freezing is the caller's explicit, per-test choice: `true` makes
        // the rate-limit window deterministic, `false` (the default stance)
        // keeps the worker on the real clock.
        let log = Self::open(audit_dir, expected_gid, test_mode, retention_days)?;
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        log.submit(
            AuditCommand::SetWriteLimit {
                writes_per_second,
                freeze_window,
                reply: reply_tx,
            },
            reply_rx,
        )??;
        Ok(log)
    }

    #[cfg(test)]
    fn inject_io_failure(&self, failure: InjectedAuditIoFailure) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(
            AuditCommand::InjectIoFailure {
                failure,
                reply: reply_tx,
            },
            reply_rx,
        )?

    }

    /// Arm a crash between two records (U6 invariant test): the next append
    /// writes a partial line and the worker exits without sync, rollback, or
    /// reply, so only the durable prefix survives a restart.
    #[cfg(test)]
    fn inject_crash_after_write(&self) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(
            AuditCommand::InjectCrashAfterWrite { reply: reply_tx },
            reply_rx,
        )?

    }

    /// The worker's drop-warning cursors (U6 test hook; the warning state is
    /// worker-owned).
    #[cfg(test)]
    fn drop_warning_state_snapshot(&self) -> AuditDropWarningState {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(
            AuditCommand::DropWarningState { reply: reply_tx },
            reply_rx,
        )
        .unwrap_or_default()
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
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(
            AuditCommand::CurrentDailyPath { reply: reply_tx },
            reply_rx,
        )
        .unwrap_or_else(|_| {
            self.audit_dir
                .join(format!("broker-{}.jsonl", utc_date_string()))
        })
    }

    /// Legacy short-record writer. New op dispatch arms call
    /// [`Self::write_op_record`] instead. The `AuditEntry` JSONL shape
    /// is still produced for back-compat with the `broker-socket-acl.sh`
    /// gate (which greps numeric `caller_uid` attribution). Authenticated
    /// caller UID/GID pairs use [`Self::write_entry_with_caller_ids`];
    /// all records - `AuditEntry` and `OpAuditRecord` alike - land in the
    /// day's daily file under `audit_dir`.
    pub fn write_entry(
        &self,
        op: &str,
        caller_uid: u32,
        disposition: &str,
        opaque_target_id: &str,
        outcome: &str,
    ) -> io::Result<()> {
        self.write_entry_with_class(
            AuditWriteClass::Privileged,
            op,
            caller_uid,
            disposition,
            opaque_target_id,
            outcome,
        )
    }

    pub(crate) fn write_entry_with_caller_ids(
        &self,
        op: &str,
        caller_uid: u32,
        caller_gid: u32,
        disposition: &str,
        opaque_target_id: &str,
        outcome: &str,
    ) -> io::Result<()> {
        self.write_entry_with_class_and_caller_ids(
            AuditWriteClass::Privileged,
            op,
            caller_uid,
            caller_gid,
            disposition,
            opaque_target_id,
            outcome,
        )
    }

    pub(crate) fn write_entry_with_class(
        &self,
        audit_class: AuditWriteClass,
        op: &str,
        caller_uid: u32,
        disposition: &str,
        opaque_target_id: &str,
        outcome: &str,
    ) -> io::Result<()> {
        self.write_entry_with_class_and_optional_caller_gid(
            audit_class,
            op,
            caller_uid,
            None,
            disposition,
            opaque_target_id,
            outcome,
        )
    }

    pub(crate) fn write_entry_with_class_and_caller_ids(
        &self,
        audit_class: AuditWriteClass,
        op: &str,
        caller_uid: u32,
        caller_gid: u32,
        disposition: &str,
        opaque_target_id: &str,
        outcome: &str,
    ) -> io::Result<()> {
        self.write_entry_with_class_and_optional_caller_gid(
            audit_class,
            op,
            caller_uid,
            Some(caller_gid),
            disposition,
            opaque_target_id,
            outcome,
        )
    }

    fn write_entry_with_class_and_optional_caller_gid(
        &self,
        audit_class: AuditWriteClass,
        op: &str,
        caller_uid: u32,
        caller_gid: Option<u32>,
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
            caller_gid,
            disposition,
            opaque_target_id,
            outcome,
            error_kind: None,
            error_message: None,
        };
        self.append_json_line(audit_class, op, &entry)
    }

    /// Legacy short-record writer for errored outcomes that need
    /// admin-visible diagnostics. The full detail is also surfaced in
    /// the broker journal (`journalctl -u d2b-broker`) for
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
        self.write_error_entry_with_optional_caller_gid(
            operation,
            caller_uid,
            None,
            decision,
            target_id,
            error_kind,
            error_message,
        )
    }

    pub(crate) fn write_error_entry_with_caller_ids(
        &self,
        operation: &str,
        caller_uid: u32,
        caller_gid: u32,
        decision: &str,
        target_id: &str,
        error_kind: &str,
        error_message: &str,
    ) -> io::Result<()> {
        self.write_error_entry_with_optional_caller_gid(
            operation,
            caller_uid,
            Some(caller_gid),
            decision,
            target_id,
            error_kind,
            error_message,
        )
    }

    fn write_error_entry_with_optional_caller_gid(
        &self,
        operation: &str,
        caller_uid: u32,
        caller_gid: Option<u32>,
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
            caller_gid,
            disposition: decision,
            opaque_target_id: target_id,
            outcome: "errored",
            error_kind: Some(error_kind),
            error_message: Some(error_message),
        };
        self.append_json_line(AuditWriteClass::Privileged, operation, &entry)
    }

    fn append_json_line<T: Serialize>(
        &self,
        audit_class: AuditWriteClass,
        operation: &str,
        value: &T,
    ) -> io::Result<()> {
        let value = serde_json::to_value(value)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let mut line = serde_json::to_string(&sanitize_audit_value(value))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push('\n');
        self.append_to_daily(
            audit_class,
            operation,
            line.as_bytes(),
            #[cfg(test)]
            None,
        )
    }

    /// Append one [`OpAuditRecord`] to the day's daily file.
    pub fn write_op_record(&self, record: &OpAuditRecord<'_>) -> io::Result<()> {
        record
            .operation_identity()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "audit operation invalid"))?;
        let expected_key = d2b_audit::ZoneOperationKey::new(
            record.zone_id.clone(),
            record.operation_identity.clone(),
        );
        if record.zone_operation_key != expected_key {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit operation join mismatch",
            ));
        }
        let line = serde_json::to_string(&sanitize_audit_value(
            serde_json::to_value(record)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
        ))
        .map(|mut line| {
            line.push('\n');
            line
        })
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        // The capture push happens on the worker after the durable append
        // (same ordering the daily lock gave it before the U6 conversion).
        #[cfg(test)]
        let capture_record = self
            .captured_records
            .as_ref()
            .map(|_| OwnedOpAuditRecord::from(record));
        self.append_to_daily(
            AuditWriteClass::Privileged,
            record.operation,
            line.as_bytes(),
            #[cfg(test)]
            capture_record,
        )
    }

    /// Append one nested-call evidence-chain record (KTD6) to the day's
    /// daily file.
    ///
    /// The record shape is the shared chain record (`d2b_audit::
    /// evidence_chain::ChainRecord`): the broker writes it for the legs its
    /// own process executes (in-broker executions only), the daemon writes
    /// the same shape for the legs it executes, and the two sides never
    /// both record one leg. A consumer restores the full picture of one
    /// invocation by counting exactly one root record per invocation id
    /// and keying the correlation records on the id plus depth.
    pub fn write_chain_record(&self, record: &ChainRecord) -> io::Result<()> {
        self.append_json_line(AuditWriteClass::Privileged, &record.operation, record)
    }

    /// Append a `ChildReaped` forensics record to the daily audit log.
    /// Both the real-time IPC channel and the audit channel receive the
    /// event (distinct sinks: IPC for daemon, audit for post-mortem
    /// forensics).
    pub fn write_child_reaped(
        &self,
        notif: &d2b_contracts_broker::broker_wire::ChildReapedNotification,
    ) -> io::Result<()> {
        #[derive(serde::Serialize)]
        struct ChildReapedAuditEntry<'a> {
            ts: u128,
            op: &'static str,
            runner_id: &'a str,
            pid: i32,
            exit_status: &'a d2b_contracts_broker::broker_wire::ChildExitStatus,
            reaped_at_ms: i64,
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.append_json_line(
            AuditWriteClass::Privileged,
            "ChildReaped",
            &ChildReapedAuditEntry {
                ts,
                op: "ChildReaped",
                runner_id: &notif.runner_id,
                pid: notif.pid,
                exit_status: &notif.exit_status,
                reaped_at_ms: notif.reaped_at_ms,
            },
        )
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
        self.record_with_join(
            operation,
            public_operation_id,
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
            error_kind,
            tracing_span_id,
            bundle_version,
            bundle_hash,
            duration_us,
            operation_fields,
            None,
        )
    }

    /// Append a typed durability record using the caller-supplied canonical
    /// join. The broker never derives this key from a display target or a
    /// serialized request.
    #[allow(clippy::too_many_arguments)]
    pub fn record_with_join(
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
        supplied_join: Option<(&str, &str)>,
    ) -> io::Result<()> {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let event_id = new_event_id()?;
        let expected_operation = d2b_audit::OperationIdentity::parse(public_operation_id)
            .or_else(|_| d2b_audit::OperationIdentity::derive(public_operation_id))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "audit operation invalid"))?;
        let (zone_id, operation_identity, public_operation_id, audit_scope_id, zone_operation_key) =
            if let Some((join_zone_id, join_operation_identity)) = supplied_join {
                let zone_id = d2b_audit::ZoneId::parse(join_zone_id).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "audit zone invalid")
                })?;
                let operation_identity =
                    d2b_audit::OperationIdentity::parse(join_operation_identity).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "audit operation invalid")
                    })?;
                let key =
                    d2b_audit::ZoneOperationKey::new(zone_id.clone(), operation_identity.clone());
                (
                    zone_id.clone(),
                    operation_identity.clone(),
                    operation_identity.as_str().to_owned(),
                    zone_id.as_str().to_owned(),
                    key,
                )
            } else {
                let zone_id = d2b_audit::ZoneId::derive(scope_id)
                    .or_else(|_| d2b_audit::ZoneId::derive(&d2b_audit::opaque_identity(scope_id)))
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "audit zone invalid")
                    })?;
                let key =
                    d2b_audit::ZoneOperationKey::new(zone_id.clone(), expected_operation.clone());
                (
                    zone_id,
                    expected_operation,
                    public_operation_id.to_owned(),
                    scope_id.to_owned(),
                    key,
                )
            };
        let record = OpAuditRecord {
            record_class: BrokerAuditRecordClass::Durability,
            ts_ms,
            broker_version: BROKER_VERSION,
            bundle_version,
            bundle_hash,
            operation,
            public_operation_id: &public_operation_id,
            zone_id: &zone_id,
            operation_identity: &operation_identity,
            event_id: &event_id,
            peer_uid,
            peer_gid,
            peer_pid,
            peer_role,
            authz_result,
            subject_id,
            scope_id: &audit_scope_id,
            verb,
            request_fields,
            decision,
            result: result_for_decision(decision),
            error_kind,
            tracing_span_id,
            duration_us,
            operation_fields,
            zone_operation_key,
        };
        self.write_op_record(&record)
    }

    /// Query the rate-limited drop counters, merging the worker-side counts
    /// with the caller-side queue-full drops the worker never saw.
    pub fn audit_drop_summary(&self) -> io::Result<AuditDropSummary> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let mut summary = self.submit(
            AuditCommand::DropSummary { reply: reply_tx },
            reply_rx,
        )?;
        // Merge the caller-side queue-full drops the worker never saw.
        summary.privileged_rate_limited = summary
            .privileged_rate_limited
            .saturating_add(self.writer.dropped_privileged.load(Ordering::Relaxed));
        summary.unprivileged_rate_limited = summary
            .unprivileged_rate_limited
            .saturating_add(self.writer.dropped_unprivileged.load(Ordering::Relaxed));
        Ok(summary)
    }

    /// Submit one append transaction to the worker.
    ///
    /// The worker owns limiter-check, drop-accounting, write, fsync,
    /// rollback/poison, and rotation+prune as one serialized FIFO unit, so a
    /// caller cancelled mid-append cannot leak a partial JSONL line. The
    /// queue is bounded: a full queue drops unprivileged appends with
    /// accounting (matching [`AuditDropSummary`]) and backpressures
    /// privileged appends - the mutation-success boundary never drops.
    fn append_to_daily(
        &self,
        audit_class: AuditWriteClass,
        operation: &str,
        bytes: &[u8],
        #[cfg(test)]
        capture_record: Option<OwnedOpAuditRecord>,
    ) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let command = AuditCommand::Append {
            audit_class,
            operation: operation.to_owned(),
            bytes: bytes.to_vec(),
            #[cfg(test)]
            capture_record,
            reply: reply_tx,
        };
        self.enqueue_append(audit_class, operation, command, reply_rx)
    }

    /// Submit one command and wait for its reply over the dedicated bounded
    /// worker boundary (plan R4: the caller side of the audit worker's
    /// `sync_channel`, with one bounded reply channel per request).
    #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
    fn submit<T>(&self, command: AuditCommand, reply: mpsc::Receiver<T>) -> io::Result<T> {
        self.writer
            .commands
            .send(command)
            .map_err(|_| io::Error::other("audit worker unavailable"))?;
        reply
            .recv()
            .map_err(|_| io::Error::other("audit worker unavailable"))
    }

    /// Enqueue one append on the bounded worker boundary.
    ///
    /// Privileged appends use the blocking send (backpressure: the
    /// mutation-success boundary never drops a record). Unprivileged appends
    /// use the non-blocking send; a full queue drops them with accounting
    /// (matching [`AuditDropSummary`]) and the caller sees the WouldBlock
    /// refusal immediately.
    #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
    fn enqueue_append(
        &self,
        audit_class: AuditWriteClass,
        operation: &str,
        command: AuditCommand,
        reply: mpsc::Receiver<io::Result<()>>,
    ) -> io::Result<()> {
        match audit_class {
            AuditWriteClass::Privileged => {
                self.writer
                    .commands
                    .send(command)
                    .map_err(|_| io::Error::other("audit-writer-poisoned"))?;
            }
            AuditWriteClass::Unprivileged => {
                match self.writer.commands.try_send(command) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        return Err(io::Error::other("audit-writer-poisoned"));
                    }
                    Err(mpsc::TrySendError::Full(_)) => {
                        self.account_queue_full_drop(audit_class, operation);
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "audit write rate limit exceeded",
                        ));
                    }
                }
            }
        }
        reply
            .recv()
            .map_err(|_| io::Error::other("audit-writer-poisoned"))?
    }

    /// Account one append the bounded queue refused (drop-with-accounting,
    /// matching [`AuditDropSummary`]). The worker cannot see an append that
    /// never reached it, so the counter lives caller-side and is merged on
    /// read.
    fn account_queue_full_drop(&self, audit_class: AuditWriteClass, operation: &str) {
        let counter = match audit_class {
            AuditWriteClass::Privileged => &self.writer.dropped_privileged,
            AuditWriteClass::Unprivileged => &self.writer.dropped_unprivileged,
        };
        let dropped_total = counter.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        tracing::warn!(
            audit_drop_reason = "worker_queue_full",
            audit_class = audit_class.as_str(),
            operation = %operation,
            dropped_total,
            "broker audit record dropped by the bounded audit worker queue"
        );
    }

    /// Delete any `broker-YYYY-MM-DD.jsonl` files whose date stamp is
    /// older than `retention_days` days ago in UTC. Returns the number
    /// of files removed (debug aid; the runtime tracing uses this to
    /// surface retention activity).
    ///
    /// Filename is the source of truth - we never parse JSON to
    /// inspect record timestamps. Operators who manually drop in
    /// `broker-<utc-date>.jsonl` files retain the same semantics.
    /// Foreign files that don't enter the owned namespace are left alone so
    /// out-of-band artifacts (export tarballs, operator notes, etc.) survive.
    /// An invalid name in the owned namespace fails closed. Reconciled
    /// truncated tails use the dated quarantine form and follow the same
    /// retention window.
    ///
    /// `retention_days == 0` disables pruning entirely.
    pub fn prune_expired_daily_files(&self) -> io::Result<usize> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(AuditCommand::Prune { reply: reply_tx }, reply_rx)?
    }

    /// Reads one bounded, typed page from the broker audit chain.
    ///
    /// The read runs on the audit worker, so it cannot observe a file set
    /// while rotation or pruning is in flight (the serialization the daily
    /// lock used to give before the U6 conversion).
    pub fn export_page(
        &self,
        since: Option<&str>,
        filter: Option<&str>,
        cursor: Option<&AuditExportCursor>,
        limit: u32,
    ) -> io::Result<ExportBrokerAuditResponse> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(
            AuditCommand::ExportPage {
                since: since.map(str::to_owned),
                filter: filter.map(str::to_owned),
                cursor: cursor.cloned(),
                limit,
                reply: reply_tx,
            },
            reply_rx,
        )?
    }

    /// Compatibility projection for the legacy bootstrap probe.
    ///
    /// The typed page contract remains bounded at 1024 records and 768 KiB
    /// per page. This adapter follows every continuation cursor, while
    /// refusing the complete projection if it exceeds 16,384 records or
    /// 512 KiB of serialized response strings. It never silently truncates a legacy
    /// export.
    pub fn export_lines(
        &self,
        since: Option<&str>,
        filter: Option<&str>,
    ) -> io::Result<Vec<String>> {
        let mut lines = Vec::new();
        let mut total_bytes = 0_usize;
        let mut cursor = None;

        loop {
            let page = self.export_page(
                since,
                filter,
                cursor.as_ref(),
                MAX_EXPORTED_AUDIT_PAGE_RECORDS,
            )?;
            for entry in page.entries {
                let line = legacy_export_entry_line(entry)?;
                let encoded_len = serde_json::to_vec(&line)
                    .map_err(|_| io::Error::other("audit-export-encode-failed"))?
                    .len()
                    .saturating_add(1);
                if lines.len() >= MAX_LEGACY_EXPORT_RECORDS
                    || total_bytes.saturating_add(encoded_len) > MAX_LEGACY_EXPORT_BYTES
                {
                    return Err(io::Error::other("audit-export-legacy-limit"));
                }
                total_bytes = total_bytes.saturating_add(encoded_len);
                lines.push(line);
            }
            if page.complete {
                return Ok(lines);
            }
            let next_cursor = page
                .next_cursor
                .ok_or_else(|| io::Error::other("audit-export-pagination-invalid"))?;
            if cursor
                .as_ref()
                .is_some_and(|previous| !cursor_is_after(previous, &next_cursor))
            {
                return Err(io::Error::other("audit-export-pagination-stalled"));
            }
            cursor = Some(next_cursor);
        }
    }

    /// Returns `(uid, gid, mode)` of the current day's daily file.
    pub fn metadata(&self) -> io::Result<(u32, u32, u32)> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.submit(AuditCommand::Metadata { reply: reply_tx }, reply_rx)?
    }
}

impl Drop for AuditLog {
    /// Stop the audit worker deterministically: the Shutdown ack arrives only
    /// after the worker dropped its state (daily appender and the exclusive
    /// directory lock), so a fresh writer can never observe a half-closed
    /// directory and the cross-process mutation boundary holds until the very
    /// last append finished.
    fn drop(&mut self) {
        let (exited_tx, exited_rx) = mpsc::sync_channel(1);
        if self
            .writer
            .commands
            .send(AuditCommand::Shutdown { exited: exited_tx })
            .is_ok()
        {
            #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
            let _ = exited_rx.recv();
        }
    }
}

/// The audit worker's recv loop: every command is one serialized FIFO unit,
/// so appends, rotation, pruning, and exports can never interleave. The
/// blocking recv is the sanctioned bounded-worker channel boundary (plan R4).
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn audit_worker_loop(receiver: mpsc::Receiver<AuditCommand>) {
    let Ok(AuditCommand::Bootstrap {
        audit_dir,
        expected_gid,
        test_mode,
        retention_days,
        #[cfg(test)]
        capture,
        reply,
    }) = receiver.recv()
    else {
        return;
    };
    let mut state = match audit_bootstrap(
        &audit_dir,
        expected_gid,
        test_mode,
        retention_days,
        #[cfg(test)]
        capture,
    ) {
        Ok(state) => state,
        Err(error) => {
            let _ = reply.send(Err(error));
            return;
        }
    };
    let _ = reply.send(Ok(()));
    while let Ok(command) = receiver.recv() {
        match command {
            AuditCommand::Append {
                audit_class,
                operation,
                bytes,
                #[cfg(test)]
                capture_record,
                reply,
            } => {
                #[cfg(test)]
                if state.crash_after_write {
                    // Simulated process death between two records: half the
                    // next line reaches the page cache, then the worker dies
                    // without sync, rollback, or reply. The durable prefix is
                    // all a restart sees.
                    state.crash_after_write = false;
                    let partial_len = (bytes.len() / 2).max(1).min(bytes.len());
                    let _ = state.daily.file.write_all(&bytes[..partial_len]);
                    return;
                }
                let result = append_locked(
                    &mut state,
                    audit_class,
                    &operation,
                    &bytes,
                    #[cfg(test)]
                    capture_record,
                );
                let _ = reply.send(result);
            }
            AuditCommand::ExportPage {
                since,
                filter,
                cursor,
                limit,
                reply,
            } => {
                let result = export_page_locked(
                    &state.audit_dir,
                    since.as_deref(),
                    filter.as_deref(),
                    cursor.as_ref(),
                    limit,
                );
                let _ = reply.send(result);
            }
            AuditCommand::Prune { reply } => {
                let result =
                    prune_expired_daily_files_locked(&state.audit_dir, state.retention_days);
                let _ = reply.send(result);
            }
            AuditCommand::DropSummary { reply } => {
                let _ = reply.send(state.drop_summary);
            }
            #[cfg(test)]
            AuditCommand::DropWarningState { reply } => {
                let _ = reply.send(state.drop_warning_state.clone());
            }
            AuditCommand::CurrentDailyPath { reply } => {
                let _ = reply.send(current_daily_path_locked(&state));
            }
            AuditCommand::Metadata { reply } => {
                let result = metadata_locked(&state);
                let _ = reply.send(result);
            }
            #[cfg(test)]
            AuditCommand::SetWriteLimit {
                writes_per_second,
                freeze_window,
                reply,
            } => {
                let mut limiter = AuditWriteLimiter::new(writes_per_second);
                if freeze_window {
                    // Test-only determinism, opt-in per test: freeze the
                    // window so rate-limit assertions cannot roll over onto a
                    // fresh window mid-loop under host load. Production never
                    // crosses this seam; without the opt-in the real clock
                    // stays in charge.
                    limiter.freeze_windows();
                }
                state.write_limiter = limiter;
                let _ = reply.send(Ok(()));
            }
            #[cfg(test)]
            AuditCommand::InjectIoFailure { failure, reply } => {
                state.daily.io_failure = Some(failure);
                let _ = reply.send(Ok(()));
            }
            #[cfg(test)]
            AuditCommand::InjectCrashAfterWrite { reply } => {
                state.crash_after_write = true;
                let _ = reply.send(Ok(()));
            }
            AuditCommand::Shutdown { exited } => {
                drop(state);
                let _ = exited.send(());
                return;
            }
            AuditCommand::Bootstrap { .. } => {}
        }
    }
}

/// The whole `open()` barrier on the worker: symlink refusal, directory
/// lock, reconciliation of every owned daily file, the current day's
/// appender, and prune-on-open.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn audit_bootstrap(
    audit_dir: &Path,
    expected_gid: u32,
    test_mode: bool,
    retention_days: u32,
    #[cfg(test)]
    capture: Option<Arc<Mutex<Vec<OwnedOpAuditRecord>>>>,
) -> io::Result<AuditWorkerState> {
    // Refuse symlink on the audit dir.
    if let Ok(metadata) = fs::symlink_metadata(audit_dir)
        && metadata.file_type().is_symlink()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "audit directory rejected",
        ));
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

    let directory_lock = open_audit_directory_lock(audit_dir, expected_gid, test_mode)?;
    let owned_daily_files = scan_owned_daily_files(audit_dir)?;
    for (_, path) in &owned_daily_files {
        reconcile_truncated_final_line(path, audit_dir, expected_gid, test_mode)?;
    }

    let today = utc_date_string();
    let daily_path = audit_dir.join(format!("broker-{today}.jsonl"));
    let daily_file = open_append_cloexec(&daily_path, expected_gid, test_mode)?;
    sync_directory(audit_dir)?;

    let state = AuditWorkerState {
        audit_dir: audit_dir.to_path_buf(),
        expected_gid,
        test_mode,
        retention_days,
        directory_lock,
        daily: DailyAppender {
            file: daily_file,
            date_utc: today,
            poisoned: false,
            #[cfg(test)]
            io_failure: None,
        },
        write_limiter: AuditWriteLimiter::new(DEFAULT_AUDIT_WRITES_PER_SECOND),
        drop_summary: AuditDropSummary::default(),
        drop_warning_state: AuditDropWarningState::default(),
        #[cfg(test)]
        capture,
        #[cfg(test)]
        crash_after_write: false,
    };

    // Prune on open so a long-stopped daemon catches up. Best-effort:
    // log + ignore errors (caller should not fail to start the daemon
    // because of a stale-file cleanup hiccup).
    if let Err(err) = prune_expired_daily_files_locked(&state.audit_dir, state.retention_days) {
        if err.to_string() == "audit-owned-file-identity-invalid" {
            return Err(err);
        }
        // We don't have tracing in scope here; rely on the broker
        // runtime to surface this via its own log if it cares.
        // The append path is unaffected.
        let _ = err;
    }

    Ok(state)
}

/// One append transaction, owned by the worker: limiter-check, rotation,
/// write, flush, fsync, directory sync, rollback/poison on failure, then the
/// bounded retention scan - all one serialized unit.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn append_locked(
    state: &mut AuditWorkerState,
    audit_class: AuditWriteClass,
    operation: &str,
    bytes: &[u8],
    #[cfg(test)]
    capture_record: Option<OwnedOpAuditRecord>,
) -> io::Result<()> {
    if state.daily.poisoned {
        return Err(io::Error::other("audit-writer-poisoned"));
    }
    if let Err(err) = state.write_limiter.check(audit_class) {
        record_rate_limited_drop(state, audit_class, operation);
        return Err(err);
    }
    let today = utc_date_string();
    let rotated = today != state.daily.date_utc;
    if rotated {
        // Rotations swap the fd via reopen + atomic rename. We
        // reopen the new day's file in O_APPEND; the old file is
        // closed by replacing it (drop runs).
        state.daily.sync_all()?;
        let new_path = state.audit_dir.join(format!("broker-{today}.jsonl"));
        let new_file = open_append_cloexec(&new_path, state.expected_gid, state.test_mode)?;
        state.daily.file = new_file;
        state.daily.date_utc = today;
    }

    let pre_append_offset = state.daily.file.metadata()?.len();
    let append_result = (|| {
        state.daily.write_all(bytes)?;
        state.daily.flush()?;
        state.daily.sync_all()?;
        sync_directory(&state.audit_dir)?;
        Ok(())
    })();
    if let Err(err) = append_result {
        if state.daily.rollback_to(pre_append_offset, &state.audit_dir).is_err() {
            state.daily.poisoned = true;
            return Err(io::Error::other("audit-writer-poisoned"));
        }
        return Err(err);
    }

    // The typed capture push lands after the durable append, before the
    // reply - the ordering the daily lock gave it before the U6 conversion.
    #[cfg(test)]
    if let Some(capture) = &state.capture
        && let Some(record) = capture_record
        && let Ok(mut records) = capture.lock()
    {
        records.push(record);
    }

    // Keep the worker's serial position through the bounded retention scan
    // so export cannot observe a file set while rotation or pruning is in
    // flight.
    if let Err(err) = prune_expired_daily_files_locked(&state.audit_dir, state.retention_days) {
        // Same swallow as open(): pruning failures must not
        // break the write path. The next rotation retries.
        let _ = err;
    }
    Ok(())
}

/// Worker-side drop accounting: the limiter refused one append.
fn record_rate_limited_drop(
    state: &mut AuditWorkerState,
    audit_class: AuditWriteClass,
    operation: &str,
) {
    let counter = match audit_class {
        AuditWriteClass::Privileged => &mut state.drop_summary.privileged_rate_limited,
        AuditWriteClass::Unprivileged => &mut state.drop_summary.unprivileged_rate_limited,
    };
    *counter = counter.saturating_add(1);
    let dropped_total = *counter;

    if let Some(warning) = state.drop_warning_state.observe(audit_class, dropped_total) {
        tracing::warn!(
            audit_drop_reason = "rate_limited",
            audit_class = audit_class.as_str(),
            operation = %operation,
            dropped_total = warning.dropped_total,
            dropped_since_previous_warning = warning.dropped_since_previous_warning,
            "broker audit records dropped by write limiter"
        );
    }
}

/// The current day's file path, resolved on the worker (the date is the
/// worker's authoritative rotation state).
fn current_daily_path_locked(state: &AuditWorkerState) -> PathBuf {
    state
        .audit_dir
        .join(format!("broker-{}.jsonl", state.daily.date_utc))
}

/// `(uid, gid, mode)` of the current day's daily file, on the worker.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn metadata_locked(state: &AuditWorkerState) -> io::Result<(u32, u32, u32)> {
    let metadata = fs::metadata(current_daily_path_locked(state))?;
    Ok((
        metadata.uid(),
        metadata.gid(),
        metadata.permissions().mode() & 0o777,
    ))
}

/// Delete any `broker-YYYY-MM-DD.jsonl` files whose date stamp is older than
/// `retention_days` days ago in UTC, on the worker. Returns the number of
/// files removed.
///
/// Filename is the source of truth - we never parse JSON to inspect record
/// timestamps. Operators who manually drop in `broker-<utc-date>.jsonl`
/// files retain the same semantics. Foreign files that don't enter the owned
/// namespace are left alone so out-of-band artifacts (export tarballs,
/// operator notes, etc.) survive. An invalid name in the owned namespace
/// fails closed. Reconciled truncated tails use the dated quarantine form
/// and follow the same retention window.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn prune_expired_daily_files_locked(audit_dir: &Path, retention_days: u32) -> io::Result<usize> {
    if retention_days == 0 {
        return Ok(0);
    }
    let cutoff_days = retention_days as i64;
    let today_unix_days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        / 86_400;

    let mut pruned = 0usize;
    let entries = match fs::read_dir(audit_dir) {
        Ok(it) => it,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    for (index, entry) in entries.enumerate() {
        if index >= MAX_AUDIT_DIRECTORY_ENTRIES {
            return Err(io::Error::other("audit-directory-scan-limit"));
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(stem) = dated_audit_artifact_date(&name)? else {
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
            if path_safe::remove_nofollow(&entry.path()).is_ok() {
                pruned += 1;
            }
        }
    }
    if pruned > 0 {
        sync_directory(audit_dir)?;
    }
    Ok(pruned)
}

/// Reads one bounded, typed page from the broker audit chain, on the worker.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn export_page_locked(
    audit_dir: &Path,
    since: Option<&str>,
    filter: Option<&str>,
    cursor: Option<&AuditExportCursor>,
    limit: u32,
) -> io::Result<ExportBrokerAuditResponse> {
    let limit = usize::try_from(limit)
        .ok()
        .filter(|limit| {
            (1..=usize::try_from(MAX_EXPORTED_AUDIT_PAGE_RECORDS).unwrap_or(usize::MAX))
                .contains(limit)
        })
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "audit-export-limit-invalid")
        })?;
    let typed_filter = filter
        .map(serde_json::from_str::<BrokerAuditFilter>)
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "audit-filter-invalid"))?;
    if cursor.is_some_and(|cursor| !is_valid_audit_day(&cursor.day)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "audit-export-cursor-invalid",
        ));
    }
    let daily_paths = scan_owned_daily_files(audit_dir)?;

    let mut output = Vec::new();
    let mut bytes = 0_usize;
    let mut sequence = cursor
        .map(|cursor| cursor.sequence.saturating_add(1))
        .unwrap_or(0);
    let mut next_cursor = None;
    let mut complete = true;
    'files: for (day, path) in daily_paths {
        if cursor.is_some_and(|cursor| {
            day < cursor.day || (day == cursor.day && cursor.line == u64::MAX)
        }) {
            continue;
        }
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(file) => file,
            Err(_) => {
                let entry = AuditExportEntry {
                    sequence,
                    record: None,
                    error: Some(AuditExportErrorCode::ReadFailed),
                };
                if !append_export_entry(
                    &mut output,
                    &mut bytes,
                    &mut sequence,
                    &mut next_cursor,
                    &day,
                    u64::MAX,
                    limit,
                    entry,
                )? {
                    complete = false;
                    break 'files;
                }
                continue;
            }
        };
        let mut reader = BufReader::new(file);
        let mut line_number = 0_u64;
        loop {
            let current_line = line_number;
            let line_bytes = match read_bounded_line(&mut reader) {
                Ok(BoundedLine::EndOfFile) => break,
                Ok(BoundedLine::Record(line)) => {
                    line_number = line_number.saturating_add(1);
                    line
                }
                Ok(BoundedLine::ReadFailed {
                    consumed,
                    end_of_file,
                }) => {
                    if !consumed {
                        // The physical line is still pending. Do not
                        // manufacture a cursor that would skip it.
                        return Err(io::Error::other("audit-export-line-discard-limit"));
                    }
                    line_number = line_number.saturating_add(1);
                    if cursor.is_some_and(|cursor| {
                        day < cursor.day || (day == cursor.day && current_line <= cursor.line)
                    }) {
                        continue;
                    }
                    let entry = AuditExportEntry {
                        sequence,
                        record: None,
                        error: Some(AuditExportErrorCode::ReadFailed),
                    };
                    if !append_export_entry(
                        &mut output,
                        &mut bytes,
                        &mut sequence,
                        &mut next_cursor,
                        &day,
                        current_line,
                        limit,
                        entry,
                    )? {
                        complete = false;
                        break 'files;
                    }
                    if end_of_file {
                        break;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            if cursor.is_some_and(|cursor| {
                day < cursor.day || (day == cursor.day && current_line <= cursor.line)
            }) {
                continue;
            }
            let line = match String::from_utf8(line_bytes) {
                Ok(line) => line,
                Err(_) => {
                    let entry = AuditExportEntry {
                        sequence,
                        record: None,
                        error: Some(AuditExportErrorCode::ReadFailed),
                    };
                    if !append_export_entry(
                        &mut output,
                        &mut bytes,
                        &mut sequence,
                        &mut next_cursor,
                        &day,
                        current_line,
                        limit,
                        entry,
                    )? {
                        complete = false;
                        break 'files;
                    }
                    continue;
                }
            };
            let raw_record = serde_json::from_str::<Value>(&line).ok();
            let is_corrupt = raw_record.is_none();
            if !is_corrupt
                && since.is_some_and(|since| {
                    !raw_record
                        .as_ref()
                        .is_some_and(|record| ts_at_least(record, since))
                })
            {
                continue;
            }
            if !is_corrupt
                && typed_filter.as_ref().is_some_and(|filter| {
                    !raw_record
                        .as_ref()
                        .is_some_and(|record| record_matches_filter(record, filter))
                })
            {
                continue;
            }
            let entry = match raw_record.map(sanitize_audit_value) {
                Some(Value::Object(record)) => AuditExportEntry {
                    sequence,
                    record: Some(Value::Object(record)),
                    error: None,
                },
                _ => AuditExportEntry {
                    sequence,
                    record: None,
                    error: Some(AuditExportErrorCode::RecordInvalid),
                },
            };
            if !append_export_entry(
                &mut output,
                &mut bytes,
                &mut sequence,
                &mut next_cursor,
                &day,
                current_line,
                limit,
                entry,
            )? {
                complete = false;
                break 'files;
            }
            if output.len() >= limit {
                complete = false;
                break 'files;
            }
        }
    }
    if complete {
        next_cursor = None;
    }
    Ok(ExportBrokerAuditResponse {
        entries: output,
        next_cursor,
        complete,
    })
}

#[derive(Debug)]
struct AuditWriteLimiter {
    privileged: AuditWriteBucket,
    unprivileged: AuditWriteBucket,
}

#[derive(Debug)]
struct AuditWriteBucket {
    window_start: Instant,
    writes_this_window: u32,
    max_writes_per_window: u32,
    /// Test-only clock seam: when frozen, the write window never
    /// expires, so rate-limit assertions cannot roll over onto a fresh
    /// window mid-loop under host load. Production never freezes.
    #[cfg(test)]
    window_frozen: bool,
}

impl AuditWriteLimiter {
    fn new(max_writes_per_window: u32) -> Self {
        let unprivileged_max = if max_writes_per_window <= 1 {
            0
        } else {
            (max_writes_per_window / 4).max(1)
        };
        let privileged_max = max_writes_per_window.saturating_sub(unprivileged_max);
        Self {
            privileged: AuditWriteBucket::new(privileged_max),
            unprivileged: AuditWriteBucket::new(unprivileged_max),
        }
    }

    fn check(&mut self, audit_class: AuditWriteClass) -> io::Result<()> {
        match audit_class {
            // Privileged audit is part of the mutation success boundary. It
            // may fail on I/O, but it is never rejected by a quota.
            AuditWriteClass::Privileged => Ok(()),
            AuditWriteClass::Unprivileged => self.unprivileged.check(),
        }
    }

    /// Test-only clock seam: freeze every write window so rate-limit
    /// tests are deterministic (the window cannot roll over mid-loop).
    /// `AuditWriteLimiter::new` keeps the production default: real time.
    #[cfg(test)]
    fn freeze_windows(&mut self) {
        self.privileged.window_frozen = true;
        self.unprivileged.window_frozen = true;
    }
}

impl AuditWriteBucket {
    fn new(max_writes_per_window: u32) -> Self {
        Self {
            window_start: Instant::now(),
            writes_this_window: 0,
            max_writes_per_window,
            #[cfg(test)]
            window_frozen: false,
        }
    }

    fn window_elapsed(&self) -> bool {
        #[cfg(test)]
        if self.window_frozen {
            return false;
        }
        self.window_start.elapsed() >= AUDIT_WRITE_WINDOW
    }

    fn check(&mut self) -> io::Result<()> {
        if self.max_writes_per_window == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "audit write rate limit exceeded",
            ));
        }
        if self.window_elapsed() {
            self.window_start = Instant::now();
            self.writes_this_window = 0;
        }
        if self.writes_this_window >= self.max_writes_per_window {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "audit write rate limit exceeded",
            ));
        }
        self.writes_this_window += 1;
        Ok(())
    }
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn open_append_cloexec(path: &Path, expected_gid: u32, test_mode: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o640)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;
    path_safe::fchmod(file.as_fd(), 0o640)?;
    set_root_d2bd_acl(&file, expected_gid, test_mode)?;
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

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn open_audit_directory_lock(
    audit_dir: &Path,
    expected_gid: u32,
    test_mode: bool,
) -> io::Result<File> {
    let path = audit_dir.join("audit.lock");
    let was_present = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "audit lock rejected",
                ));
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)?;
    let owner_uid = if test_mode {
        Uid::current().as_raw()
    } else {
        0
    };
    let group_gid = if test_mode {
        Gid::current().as_raw()
    } else {
        expected_gid
    };
    if !was_present {
        path_safe::fchmod(file.as_fd(), 0o600)?;
        if let Err(error) = path_safe::fchown(file.as_fd(), Some(owner_uid), Some(group_gid))
            && !test_mode
        {
            return Err(error);
        }
    }
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner_uid
        || metadata.gid() != group_gid
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "audit-lock-ownership-invalid",
        ));
    }
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "audit-lock-held"))?;
    sync_directory(audit_dir)?;
    Ok(file)
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn scan_owned_daily_files(audit_dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut daily_paths = Vec::new();
    for (index, entry) in fs::read_dir(audit_dir)?.enumerate() {
        if index >= MAX_AUDIT_DIRECTORY_ENTRIES {
            return Err(io::Error::other("audit-directory-scan-limit"));
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(day) = dated_audit_artifact_date(&name)? else {
            continue;
        };
        if name.as_bytes().ends_with(b".jsonl") {
            daily_paths.push((day, entry.path()));
        }
    }
    daily_paths.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(daily_paths)
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn reconcile_truncated_final_line(
    path: &Path,
    audit_dir: &Path,
    expected_gid: u32,
    test_mode: bool,
) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "audit daily file rejected",
        ));
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "audit daily path is not a regular file",
        ));
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(());
    }

    file.seek(SeekFrom::Start(file_len - 1))?;
    let mut final_byte = [0_u8; 1];
    file.read_exact(&mut final_byte)?;
    if final_byte[0] == b'\n' {
        return Ok(());
    }

    let truncate_at = last_newline_offset(&mut file, file_len)?
        .map(|offset| offset.saturating_add(1))
        .unwrap_or(0);
    quarantine_truncated_tail(
        &mut file,
        path,
        audit_dir,
        truncate_at,
        file_len.saturating_sub(truncate_at),
        expected_gid,
        test_mode,
    )?;
    file.set_len(truncate_at)?;
    file.sync_all()?;
    sync_directory(audit_dir)
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn last_newline_offset(file: &mut File, file_len: u64) -> io::Result<Option<u64>> {
    let mut cursor = file_len;
    let mut buffer = [0_u8; AUDIT_RECONCILE_CHUNK_BYTES];
    while cursor > 0 {
        let chunk_start = cursor.saturating_sub(buffer.len() as u64);
        let chunk_len = usize::try_from(cursor - chunk_start)
            .map_err(|_| io::Error::other("audit-reconcile-chunk-too-large"))?;
        file.seek(SeekFrom::Start(chunk_start))?;
        file.read_exact(&mut buffer[..chunk_len])?;
        if let Some(index) = buffer[..chunk_len].iter().rposition(|byte| *byte == b'\n') {
            return Ok(Some(chunk_start + index as u64));
        }
        cursor = chunk_start;
    }
    Ok(None)
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn quarantine_truncated_tail(
    source: &mut File,
    daily_path: &Path,
    audit_dir: &Path,
    tail_start: u64,
    tail_len: u64,
    expected_gid: u32,
    test_mode: bool,
) -> io::Result<()> {
    let basename = daily_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "audit daily basename invalid")
        })?;
    let directory_fd = path_safe::open_dir_path_safe(audit_dir)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..MAX_QUARANTINE_NAME_ATTEMPTS {
        let name = format!(
            "{basename}.truncated-{}-{nonce}-{attempt}.quarantine",
            std::process::id()
        );
        let fd = match path_safe::create_file_at_safe(
            &directory_fd,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o640,
        ) {
            Ok(fd) => fd,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let quarantine_path = audit_dir.join(&name);
        let mut quarantine = File::from(fd);
        let result = (|| {
            path_safe::fchmod(quarantine.as_fd(), 0o640)?;
            set_root_d2bd_acl(&quarantine, expected_gid, test_mode)?;
            source.seek(SeekFrom::Start(tail_start))?;
            let mut remaining = tail_len;
            let mut buffer = [0_u8; AUDIT_RECONCILE_CHUNK_BYTES];
            while remaining > 0 {
                let chunk_len = usize::try_from(remaining.min(buffer.len() as u64))
                    .map_err(|_| io::Error::other("audit-reconcile-chunk-too-large"))?;
                source.read_exact(&mut buffer[..chunk_len])?;
                quarantine.write_all(&buffer[..chunk_len])?;
                remaining -= chunk_len as u64;
            }
            quarantine.flush()?;
            quarantine.sync_all()?;
            sync_directory(audit_dir)
        })();
        if let Err(error) = result {
            drop(quarantine);
            let _ = path_safe::remove_nofollow(&quarantine_path);
            return Err(error);
        }
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "audit quarantine name allocation exhausted",
    ))
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn sync_directory(directory: &Path) -> io::Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?;
    file.sync_all()
}

pub(crate) fn sanitize_audit_value(value: Value) -> Value {
    fn walk(value: &mut Value, key: Option<&str>) {
        match value {
            Value::Object(object) => {
                object.retain(|name, child| match name.as_str() {
                    "caller_uid" | "caller_gid" => {
                        key.is_none()
                            && child
                                .as_u64()
                                .is_some_and(|value| value <= u64::from(u32::MAX))
                    }
                    "peer_pid"
                    | "pid"
                    | "pidfd"
                    | "handle"
                    | "start_time_ticks"
                    | "expected_start_time_ticks"
                    | "startTimeTicks"
                    | "expectedStartTimeTicks"
                    | "uid"
                    | "gid"
                    | "owner_uid"
                    | "owner_gid"
                    | "ownerUid"
                    | "ownerGid"
                    | "opaque_ref"
                    | "opaqueRef"
                    | "cgroup"
                    | "cgroup_subtree"
                    | "cgroupSubtree"
                    | "cgroup_path"
                    | "cgroupPath"
                    | "runtime_scope"
                    | "runtime_allocations"
                    | "runtimeScope"
                    | "runtimeAllocations"
                    | "private_runtime_scope"
                    | "privateRuntimeScope"
                    | "runtime_identity"
                    | "runtimeIdentity"
                    | "network_tap_context"
                    | "networkTapContext"
                    | "provider_identity"
                    | "template_identity"
                    | "providerIdentity"
                    | "templateIdentity"
                    | "cid"
                    | "vsock_cid"
                    | "vsockCid"
                    | "api_socket_path"
                    | "apiSocketPath"
                    | "credential"
                    | "credentials" => false,
                    _ => true,
                });
                for (name, child) in object {
                    let propagated = key
                        .filter(|parent| is_sensitive_key(parent))
                        .unwrap_or(name);
                    walk(child, Some(propagated));
                }
            }
            Value::Array(values) => {
                for child in values {
                    walk(child, key);
                }
            }
            Value::String(text)
                if (key.is_some_and(is_sensitive_key)
                    || text.contains('/')
                    || text.chars().any(char::is_whitespace))
                    && !is_canonical_digest(text) =>
            {
                *text = if key
                    .is_some_and(|key| matches!(key, "public_operation_id" | "operation_id"))
                {
                    d2b_audit::OperationIdentity::derive(text)
                        .map(|identity| identity.as_str().to_owned())
                        .unwrap_or_else(|_| opaque_digest(text))
                } else {
                    opaque_digest(text)
                };
            }
            _ => {}
        }
    }

    let mut value = value;
    walk(&mut value, None);
    value
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key,
        "path"
            | "argv"
            | "env"
            | "socket"
            | "peer_role"
            | "subject_id"
            | "scope_id"
            | "public_operation_id"
            | "event_id"
            | "tracing_span_id"
            | "runner_id"
            | "bundle_runner_intent_ref"
            | "bundleRunnerIntentRef"
            | "vm"
            | "vm_id"
            | "vmId"
            | "role_id"
            | "roleId"
            | "zone"
            | "zone_id"
            | "zoneId"
            | "zoneUid"
            | "resourceRef"
            | "resourceUid"
            | "ownerRef"
            | "providerRef"
            | "executionRef"
            | "userRef"
            | "targetUid"
            | "credential"
            | "secret"
            | "message"
            | "error_message"
    ) || key.ends_with("_path")
        || key.ends_with("_uid")
        || key.ends_with("_name")
}

fn opaque_digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut bytes = b"d2b:broker-audit-redaction:v1:".to_vec();
    bytes.extend_from_slice(value.as_bytes());
    let digest = Sha256::digest(bytes);
    let mut output = String::from("sha256:");
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }

    output
}

fn is_canonical_digest(value: &str) -> bool {
    d2b_audit::is_canonical_digest(value)
}

fn set_root_d2bd_acl(file: &File, expected_gid: u32, test_mode: bool) -> io::Result<()> {
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
    ) && !test_mode
    {
        return Err(err);
    }
    Ok(())
}

pub(crate) fn utc_date_string() -> String {
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

fn dated_audit_artifact_date(name: &OsStr) -> io::Result<Option<String>> {
    let bytes = name.as_bytes();
    let looks_owned = bytes.starts_with(b"broker-")
        && (bytes.ends_with(b".jsonl") || bytes.ends_with(b".quarantine"));
    let Some(name) = name.to_str() else {
        return if looks_owned {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit-owned-file-identity-invalid",
            ))
        } else {
            Ok(None)
        };
    };
    let Some(stem) = name.strip_prefix("broker-") else {
        return Ok(None);
    };
    let day = if let Some(day) = stem.strip_suffix(".jsonl") {
        Some(day)
    } else {
        stem.strip_suffix(".quarantine")
            .and_then(|value| value.split_once(".jsonl.truncated-"))
            .map(|(day, _)| day)
    };
    let Some(day) = day else {
        return Ok(None);
    };
    if !is_valid_audit_day(day) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "audit-owned-file-identity-invalid",
        ));
    }
    Ok(Some(day.to_owned()))
}

fn is_valid_audit_day(day: &str) -> bool {
    if day.len() != 10
        || !day.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return false;
    }
    let parts = day.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0].parse::<i32>().is_ok()
        && parts[1].parse::<u32>().is_ok()
        && parts[2].parse::<u32>().is_ok()
        && unix_days_from_ymd(
            parts[0].parse().unwrap_or_default(),
            parts[1].parse().unwrap_or_default(),
            parts[2].parse().unwrap_or_default(),
        )
        .is_some()
}

fn ts_at_least(record: &Value, since: &str) -> bool {
    let wanted = since.parse::<u128>().ok();
    let current = record
        .get("ts_ms")
        .or_else(|| record.get("ts"))
        .and_then(Value::as_u64)
        .map(u128::from);
    match (current, wanted) {
        (Some(current), Some(wanted)) => current >= wanted,
        (None, Some(_)) => false,
        (_, None) => true,
    }
}

fn append_export_entry(
    output: &mut Vec<AuditExportEntry>,
    bytes: &mut usize,
    sequence: &mut u64,
    next_cursor: &mut Option<AuditExportCursor>,
    day: &str,
    physical_line: u64,
    limit: usize,
    entry: AuditExportEntry,
) -> io::Result<bool> {
    let encoded =
        serde_json::to_vec(&entry).map_err(|_| io::Error::other("audit-export-encode-failed"))?;
    if output.len() >= limit || bytes.saturating_add(encoded.len()) > MAX_EXPORTED_AUDIT_BYTES {
        return Ok(false);
    }
    *bytes = bytes.saturating_add(encoded.len());
    let emitted_sequence = *sequence;
    output.push(entry);
    *sequence = sequence.saturating_add(1);
    *next_cursor = Some(AuditExportCursor {
        day: day.to_owned(),
        line: physical_line,
        sequence: emitted_sequence,
    });
    Ok(true)
}

fn legacy_export_entry_line(entry: AuditExportEntry) -> io::Result<String> {
    serde_json::to_string(&entry.record.or_else(|| {
        Some(serde_json::json!({
            "export_error": entry.error,
            "sequence": entry.sequence,
        }))
    }))
    .map_err(|error| io::Error::other(error.to_string()))
}

fn cursor_is_after(previous: &AuditExportCursor, next: &AuditExportCursor) -> bool {
    next.day > previous.day || (next.day == previous.day && next.line > previous.line)
}

fn record_matches_filter(record: &Value, filter: &BrokerAuditFilter) -> bool {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| record.get(*key).and_then(Value::as_str))
    };
    filter.env.as_deref().is_none_or(|expected| {
        text(&["env", "scope_id", "zone_id"])
            .is_some_and(|value| identity_filter_matches(expected, value))
    }) && filter.vm.as_deref().is_none_or(|expected| {
        text(&["vm", "vm_id"]).is_some_and(|value| identity_filter_matches(expected, value))
    }) && filter.role.as_deref().is_none_or(|expected| {
        text(&["peer_role", "role"]).is_some_and(|value| identity_filter_matches(expected, value))
    }) && filter.operation.as_deref().is_none_or(|expected| {
        text(&["operation", "op"]).is_some_and(|value| {
            value == expected
                || d2b_audit::OperationIdentity::derive(expected)
                    .is_ok_and(|identity| identity.as_str() == value)
        }) || text(&["public_operation_id", "operation_id"]).is_some_and(|value| {
            d2b_audit::OperationIdentity::derive(expected)
                .is_ok_and(|identity| identity.as_str() == value)
        })
    }) && filter.outcome.as_deref().is_none_or(|expected| {
        text(&["outcome", "result", "decision"]).is_some_and(|value| value == expected)
    }) && filter
        .severity
        .is_none_or(|severity| severity_matches(record, severity))
}

fn identity_filter_matches(expected: &str, actual: &str) -> bool {
    actual == expected
        || d2b_audit::is_canonical_digest(expected) && actual == expected
        || actual == opaque_digest(expected)
        || d2b_audit::ZoneId::derive(expected).is_ok_and(|zone| zone.as_str() == actual)
}

fn severity_matches(record: &Value, severity: BrokerAuditSeverity) -> bool {
    let decision = record
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = record
        .get("result")
        .or_else(|| record.get("outcome"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let error = record
        .get("error_kind")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    match severity {
        BrokerAuditSeverity::Denied => decision.starts_with("denied") || result == "denied",
        BrokerAuditSeverity::Error => {
            error
                || matches!(decision, "error" | "errored")
                || matches!(result, "error" | "errored")
        }
        BrokerAuditSeverity::Warning => {
            !severity_matches(record, BrokerAuditSeverity::Denied)
                && !severity_matches(record, BrokerAuditSeverity::Error)
                && matches!(result, "warning" | "degraded" | "rejected")
        }
        BrokerAuditSeverity::Info => {
            !severity_matches(record, BrokerAuditSeverity::Denied)
                && !severity_matches(record, BrokerAuditSeverity::Error)
                && !severity_matches(record, BrokerAuditSeverity::Warning)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BoundedLine {
    EndOfFile,
    Record(Vec<u8>),
    ReadFailed { consumed: bool, end_of_file: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscardedLine {
    Newline,
    EndOfFile,
    BudgetExhausted,
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<BoundedLine> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(BoundedLine::EndOfFile)
            } else {
                Ok(BoundedLine::ReadFailed {
                    consumed: true,
                    end_of_file: true,
                })
            };
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > MAX_EXPORTED_AUDIT_LINE_BYTES {
            return Ok(match discard_oversized_line(reader)? {
                DiscardedLine::Newline => BoundedLine::ReadFailed {
                    consumed: true,
                    end_of_file: false,
                },
                DiscardedLine::EndOfFile => BoundedLine::ReadFailed {
                    consumed: true,
                    end_of_file: true,
                },
                DiscardedLine::BudgetExhausted => BoundedLine::ReadFailed {
                    consumed: false,
                    end_of_file: false,
                },
            });
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            bytes.pop();
            return Ok(BoundedLine::Record(bytes));
        }
    }
}

fn discard_oversized_line<R: BufRead>(reader: &mut R) -> io::Result<DiscardedLine> {
    let mut remaining = MAX_EXPORTED_AUDIT_DISCARD_BYTES;
    while remaining > 0 {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(DiscardedLine::EndOfFile);
        }
        if let Some(index) = chunk.iter().position(|byte| *byte == b'\n') {
            let take = index.saturating_add(1);
            if take <= remaining {
                reader.consume(take);
                return Ok(DiscardedLine::Newline);
            }
        }
        let take = chunk.len().min(remaining);
        reader.consume(take);
        remaining -= take;
    }
    if reader.fill_buf()?.is_empty() {
        Ok(DiscardedLine::EndOfFile)
    } else {
        Ok(DiscardedLine::BudgetExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::evidence_chain::{ChainLeg, ChainOutcome, ChainRecordClass};

    fn target_scratch_root(prefix: &str) -> PathBuf {
        let base = crate::test_scratch_root();
        base.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ))
    }

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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn make_audit_with_files(retention_days: u32, file_dates: &[(i32, u32, u32)]) -> AuditLog {
        let dir = target_scratch_root("d2bd-broker-audit-prune");
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn prune_ignores_non_matching_filenames() {
        let today_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let old_50d = ymd_from_unix(today_unix - 86_400 * 50);

        let log = make_audit_with_files(14, &[old_50d]);
        // Seed an operator note + an export tarball - both should
        // survive pruning.
        let note = log.audit_dir.join("NOTES-operator.txt");
        let tar = log.audit_dir.join("export-2024-01-01.tar.gz");
        let stray = log.audit_dir.join("foreign-audit.jsonl");
        fs::write(&note, b"todo").unwrap();
        fs::write(&tar, b"\0").unwrap();
        fs::write(&stray, b"{}\n").unwrap();

        let pruned = log.prune_expired_daily_files().expect("prune ok");
        assert_eq!(pruned, 1, "only the dated daily file should be pruned");
        assert!(note.exists(), "operator notes must survive prune");
        assert!(tar.exists(), "export tarballs must survive prune");
        assert!(stray.exists(), "foreign jsonl must survive prune");

        let _ = fs::remove_dir_all(log.audit_dir.parent().unwrap());
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn prune_removes_expired_truncated_quarantines() {
        let today_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let old = ymd_from_unix(today_unix - 86_400 * 50);
        let log = make_audit_with_files(14, &[]);
        let quarantine = log.audit_dir.join(format!(
            "broker-{y:04}-{m:02}-{d:02}.jsonl.truncated-test.quarantine",
            y = old.0,
            m = old.1,
            d = old.2
        ));
        fs::write(&quarantine, b"truncated").expect("seed old quarantine");

        let pruned = log.prune_expired_daily_files().expect("prune quarantine");
        assert_eq!(pruned, 1);
        assert!(!quarantine.exists(), "expired quarantine should be pruned");

        let _ = fs::remove_dir_all(log.audit_dir.parent().unwrap());
    }

    #[test]
    fn audit_write_bucket_reseats_the_window_after_it_expires() {
        // Deliberately NOT frozen: this test owns the wall-clock seam and
        // drives the rollover with an aged window instead of sleeping.
        let mut bucket = AuditWriteBucket::new(2);
        assert!(bucket.check().is_ok(), "fresh window admits the first write");
        assert!(
            bucket.check().is_ok(),
            "fresh window admits the second write"
        );
        assert!(
            bucket.check().is_err(),
            "budget exhausted inside the fresh window"
        );

        // Age the window past AUDIT_WRITE_WINDOW without waiting on the
        // clock: the next check must reseat it and reset the counter
        // before admitting. The invariant: an expired window restores the
        // full budget - an excess write can never stand after a rollover.
        let expired = Instant::now() - AUDIT_WRITE_WINDOW - Duration::from_secs(1);
        bucket.window_start = expired;

        assert!(
            bucket.check().is_ok(),
            "expired window reseats and admits the next write"
        );
        assert_eq!(
            bucket.writes_this_window, 1,
            "counter reset to zero before the write was admitted"
        );
        assert!(
            bucket.window_start > expired,
            "reseat moved the window start forward"
        );

        // The reseated window is a fresh window again: it fills to budget
        // and then refuses an excess write, exactly like the original one.
        assert!(
            bucket.check().is_ok(),
            "reseated window admits within its budget"
        );
        assert!(
            bucket.check().is_err(),
            "reseated window refuses an excess write again"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn privileged_audit_is_not_rate_limited() {
        let root = target_scratch_root("audit-rate-limit");
        let log = AuditLog::open_with_write_limit(&root, Gid::current().as_raw(), true, 14, 4, true)
            .expect("open audit log with low write limit");
        log.write_entry("UsbipBind", 1000, "allowed", "operation", "ok")
            .expect("first write allowed");
        for _ in 0..8 {
            log.write_entry("UsbipBind", 1000, "allowed", "operation", "ok")
                .expect("privileged audit remains writable under pressure");
        }

        assert_eq!(log.audit_drop_summary().unwrap().privileged_rate_limited, 0);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn typed_export_is_bounded_and_cursor_paginated() {
        let root = target_scratch_root("audit-typed-export");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open typed export audit log");
        log.write_entry("Hello", 1000, "allowed", "operation", "success")
            .expect("write first record");
        log.write_entry("ExportBrokerAudit", 1000, "allowed", "audit-log", "success")
            .expect("write second record");
        let first = log
            .export_page(None, None, None, 1)
            .expect("export first page");
        assert_eq!(first.entries.len(), 1);
        assert_eq!(first.entries[0].sequence, 0);
        assert_eq!(first.next_cursor.as_ref().unwrap().sequence, 0);
        assert!(!first.complete);
        let second = log
            .export_page(None, None, first.next_cursor.as_ref(), 1)
            .expect("export second page");
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.entries[0].sequence, 1);
        assert_eq!(second.next_cursor.as_ref().unwrap().sequence, 1);
        assert!(second.entries.iter().all(|entry| entry.record.is_some()));
        let third = log
            .export_page(None, None, second.next_cursor.as_ref(), 1)
            .expect("export completion page");
        assert!(third.entries.is_empty());
        assert!(third.complete);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn append_failures_roll_back_and_keep_writer_usable() {
        let root = target_scratch_root("audit-append-rollback");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open rollback audit log");
        log.write_entry(
            "BaselineOperation",
            1000,
            "allowed",
            "baseline-target",
            "success",
        )
        .expect("write baseline record");

        for failure in [
            InjectedAuditIoFailure::PartialWrite,
            InjectedAuditIoFailure::Flush,
            InjectedAuditIoFailure::Sync { remaining: 1 },
        ] {
            let before = fs::read(log.current_daily_path()).expect("read baseline audit");
            log.inject_io_failure(failure)
                .expect("install audit failure injection");
            let error = log
                .write_entry(
                    "FailedOperationWithEnoughBytesForPartialWrite",
                    1000,
                    "allowed",
                    "failed-target",
                    "success",
                )
                .expect_err("injected append failure must propagate");
            assert_ne!(
                error.to_string(),
                "audit-writer-poisoned",
                "durable rollback should keep the writer usable"
            );
            assert_eq!(
                fs::read(log.current_daily_path()).expect("read rolled-back audit"),
                before,
                "failed append must not leave partial authoritative bytes"
            );
            log.write_entry(
                "RecoveryOperation",
                1000,
                "allowed",
                "recovery-target",
                "success",
            )
            .expect("writer should remain usable after durable rollback");
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn append_failure_poisons_writer_when_rollback_sync_fails() {
        let root = target_scratch_root("audit-append-poison");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open poison audit log");
        log.write_entry(
            "BaselineOperation",
            1000,
            "allowed",
            "baseline-target",
            "success",
        )
        .expect("write baseline record");
        let before = fs::read(log.current_daily_path()).expect("read baseline audit");

        log.inject_io_failure(InjectedAuditIoFailure::Sync { remaining: 2 })
            .expect("install sync failure injection");
        let error = log
            .write_entry(
                "PoisonedOperation",
                1000,
                "allowed",
                "poisoned-target",
                "success",
            )
            .expect_err("sync failure with uncertain rollback must fail closed");
        assert_eq!(error.to_string(), "audit-writer-poisoned");
        assert_eq!(
            fs::read(log.current_daily_path()).expect("read poisoned audit"),
            before,
            "poisoning must not expose the failed record as authoritative"
        );
        let second_error = log
            .write_entry(
                "AfterPoisonOperation",
                1000,
                "allowed",
                "after-poison-target",
                "success",
            )
            .expect_err("poisoned writer must reject subsequent appends");
        assert_eq!(second_error.to_string(), "audit-writer-poisoned");

        drop(log);
        let reopened = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("reopen after poisoned writer");
        reopened
            .write_entry(
                "ReopenedOperation",
                1000,
                "allowed",
                "reopened-target",
                "success",
            )
            .expect("a fresh writer should recover after restart");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn reopen_quarantines_truncated_final_line_before_appending() {
        let root = target_scratch_root("audit-reopen-truncated");
        fs::create_dir_all(&root).expect("create audit root");
        let path = root.join(format!("broker-{}.jsonl", utc_date_string()));
        let prefix = b"{\"ts\":1,\"op\":\"complete\"}\n";
        let truncated = br#"{"ts":2,"op":"truncated"#;
        let mut seed = prefix.to_vec();
        seed.extend_from_slice(truncated);
        fs::write(&path, seed).expect("seed truncated audit");

        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("reopen and reconcile audit log");
        assert_eq!(
            fs::read(&path).expect("read reconciled audit"),
            prefix,
            "reopen must remove the incomplete final JSONL record"
        );
        let quarantines: Vec<_> = fs::read_dir(&root)
            .expect("read audit directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".quarantine"))
            .collect();
        assert_eq!(quarantines.len(), 1, "truncated bytes must be quarantined");
        assert_eq!(
            fs::read(quarantines[0].path()).expect("read quarantined bytes"),
            truncated,
            "quarantine must preserve the exact incomplete tail"
        );

        log.write_entry(
            "AfterReopenOperation",
            1000,
            "allowed",
            "reopen-target",
            "success",
        )
        .expect("append after reconciliation");
        let contents = fs::read_to_string(&path).expect("read post-reopen audit");
        assert_eq!(contents.lines().count(), 2);
        assert!(
            contents
                .lines()
                .all(|line| serde_json::from_str::<Value>(line).is_ok())
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn open_reconciles_prior_day_tail_and_ignores_foreign_files() {
        let root = target_scratch_root("audit-reopen-prior-day");
        fs::create_dir_all(&root).expect("create audit root");
        let today = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let prior = ymd_from_unix(today - 86_400);
        let prior_path = root.join(format!(
            "broker-{y:04}-{m:02}-{d:02}.jsonl",
            y = prior.0,
            m = prior.1,
            d = prior.2
        ));
        let complete = b"{\"ts\":1,\"op\":\"prior-complete\"}\n";
        let truncated = b"{\"ts\":2,\"op\":\"prior-truncated\"";
        let mut seeded = complete.to_vec();
        seeded.extend_from_slice(truncated);
        fs::write(&prior_path, seeded).expect("seed prior-day tail");
        let foreign = root.join("operator-export.jsonl");
        fs::write(&foreign, b"foreign").expect("seed foreign file");

        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open should reconcile every owned daily file");
        assert_eq!(
            fs::read(&prior_path).expect("read repaired prior-day file"),
            complete,
            "a prior-day tail must be repaired even after the UTC day changes"
        );
        let quarantines: Vec<_> = fs::read_dir(&root)
            .expect("read audit directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_name().to_string_lossy().starts_with("broker-")
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .contains(".jsonl.truncated-")
            })
            .collect();
        assert_eq!(quarantines.len(), 1);
        assert_eq!(
            fs::read(quarantines[0].path()).expect("read prior-day quarantine"),
            truncated
        );
        assert!(
            foreign.exists(),
            "foreign files must survive reconciliation"
        );
        drop(log);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn open_fails_closed_for_invalid_or_unsafe_owned_files() {
        let root = target_scratch_root("audit-invalid-owned-file");
        fs::create_dir_all(&root).expect("create audit root");
        fs::write(root.join("broker-2024-02-30.jsonl"), b"{}\n")
            .expect("seed invalid owned identity");
        let error = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect_err("invalid owned date must fail closed");
        assert_eq!(
            error.to_string(),
            "audit-owned-file-identity-invalid",
            "invalid owned identity must not be treated as foreign"
        );
        let _ = fs::remove_dir_all(&root);

        let root = target_scratch_root("audit-unsafe-owned-file");
        fs::create_dir_all(&root).expect("create audit root");
        let day = utc_date_string();
        let target = root.join("outside");
        let unsafe_daily = root.join(format!("broker-{day}.jsonl"));
        fs::write(&target, b"{}\n").expect("seed symlink target");
        std::os::unix::fs::symlink(&target, &unsafe_daily).expect("seed unsafe daily symlink");
        let error = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect_err("owned symlink must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn audit_directory_lock_is_held_until_log_drop() {
        let root = target_scratch_root("audit-directory-lock");
        let log =
            AuditLog::open(&root, Gid::current().as_raw(), true, 30).expect("open first audit log");
        let error = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect_err("second audit owner must not enter the directory");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        drop(log);
        let reopened = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("directory lock should release with the writer");
        drop(reopened);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn full_page_before_unreadable_file_keeps_last_emitted_cursor() {
        let root = target_scratch_root("audit-full-page-unreadable");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30).expect("open audit log");
        log.write_entry("First", 1000, "allowed", "target", "success")
            .expect("write first record");
        let today = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let next_day = ymd_from_unix(today + 86_400);
        let next_day_path = root.join(format!(
            "broker-{y:04}-{m:02}-{d:02}.jsonl",
            y = next_day.0,
            m = next_day.1,
            d = next_day.2
        ));
        let target = root.join("unreadable-target");
        fs::write(&target, b"{}\n").expect("seed unreadable target");
        std::os::unix::fs::symlink(&target, &next_day_path).expect("seed unreadable owned symlink");

        let first = log
            .export_page(None, None, None, 1)
            .expect("export first full page");
        assert_eq!(first.entries.len(), 1);
        let cursor = first.next_cursor.as_ref().expect("cursor after full page");
        assert_eq!(cursor.day, utc_date_string());
        assert_eq!(cursor.line, 0, "cursor must remain on the emitted record");
        let second = log
            .export_page(None, None, Some(cursor), 10)
            .expect("retry unreadable file");
        assert_eq!(second.entries.len(), 1);
        assert_eq!(
            second.entries[0].error,
            Some(AuditExportErrorCode::ReadFailed)
        );
        assert!(second.complete);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn oversized_line_is_reported_once_and_valid_continuation_is_preserved() {
        let root = target_scratch_root("audit-oversized-line");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open oversized-line audit log");
        let path = log.current_daily_path();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open daily file");
        let oversized = vec![b'x'; MAX_EXPORTED_AUDIT_LINE_BYTES + 1024];
        file.write_all(&oversized).expect("write oversized line");
        file.write_all(b"\n{\"ts\":1,\"op\":\"after-oversized\"}\n")
            .expect("write valid continuation");
        file.sync_all().expect("sync oversized fixture");
        drop(file);

        let first = log
            .export_page(None, None, None, 1)
            .expect("export oversized line");
        assert_eq!(first.entries.len(), 1);
        assert_eq!(
            first.entries[0].error,
            Some(AuditExportErrorCode::ReadFailed)
        );
        let cursor = first
            .next_cursor
            .as_ref()
            .expect("cursor after oversized line");
        assert_eq!(cursor.line, 0);
        let second = log
            .export_page(None, None, Some(cursor), 10)
            .expect("continue after oversized line");
        assert_eq!(second.entries.len(), 1);
        assert_eq!(
            second.entries[0]
                .record
                .as_ref()
                .and_then(|record| record.get("op"))
                .and_then(Value::as_str),
            Some("after-oversized")
        );
        assert!(second.complete);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn oversized_line_spanning_small_reads_drains_exactly_one_physical_line() {
        let mut bytes = vec![b'x'; MAX_EXPORTED_AUDIT_LINE_BYTES + 17];
        bytes.extend_from_slice(b"\n{\"ts\":1,\"op\":\"valid\"}\n");
        let mut reader = BufReader::with_capacity(7, io::Cursor::new(bytes));
        assert_eq!(
            read_bounded_line(&mut reader).expect("read oversized line"),
            BoundedLine::ReadFailed {
                consumed: true,
                end_of_file: false,
            }
        );
        assert_eq!(
            read_bounded_line(&mut reader).expect("read valid continuation"),
            BoundedLine::Record(br#"{"ts":1,"op":"valid"}"#.to_vec())
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn truncated_export_line_is_typed_once_after_consuming_eof() {
        let root = target_scratch_root("audit-export-truncated-line");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open truncated export audit log");
        let path = log.current_daily_path();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open daily file");
        file.write_all(br#"{"ts":1,"op":"truncated-at-eof""#)
            .expect("write truncated line");
        file.sync_all().expect("sync truncated fixture");
        drop(file);

        let page = log
            .export_page(None, None, None, 10)
            .expect("export truncated line");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.entries[0].error,
            Some(AuditExportErrorCode::ReadFailed)
        );
        assert!(page.complete);
        assert!(page.next_cursor.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn oversized_line_without_bounded_newline_fails_closed() {
        let root = target_scratch_root("audit-oversized-no-newline");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open oversized no-newline audit log");
        let path = log.current_daily_path();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open daily file");
        file.write_all(&vec![
            b'x';
            MAX_EXPORTED_AUDIT_LINE_BYTES
                + MAX_EXPORTED_AUDIT_DISCARD_BYTES
                + 1
        ])
        .expect("write unbounded oversized line");
        file.sync_all().expect("sync oversized fixture");
        drop(file);

        let error = log
            .export_page(None, None, None, 10)
            .expect_err("line without a bounded newline must fail closed");
        assert_eq!(error.to_string(), "audit-export-line-discard-limit");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn legacy_export_follows_all_pages_above_1024_in_order() {
        let root = target_scratch_root("audit-legacy-export-pages");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open legacy export audit log");
        let mut records = String::new();
        for index in 0..1025 {
            records.push_str(&format!(r#"{{"ts":{index},"op":"legacy-{index}"}}"#));
            records.push('\n');
        }
        let path = log.current_daily_path();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open audit file for page fixture");
        file.write_all(records.as_bytes())
            .expect("append page fixture");
        file.sync_all().expect("sync page fixture");
        drop(file);

        let exported = log
            .export_lines(None, None)
            .expect("legacy export should follow continuation pages");
        assert_eq!(exported.len(), 1025);
        for (index, line) in exported.iter().enumerate() {
            let value: Value = serde_json::from_str(line).expect("parse exported line");
            assert_eq!(value["ts"], index as u64);
            assert_eq!(value["op"], format!("legacy-{index}"));
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn legacy_export_refuses_records_beyond_compatibility_cap() {
        let root = target_scratch_root("audit-legacy-export-cap");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open capped legacy export audit log");
        let mut records = String::new();
        for index in 0..=MAX_LEGACY_EXPORT_RECORDS {
            records.push_str(&format!(r#"{{"ts":{index},"op":"legacy"}}"#));
            records.push('\n');
        }
        let path = log.current_daily_path();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open audit file for cap fixture");
        file.write_all(records.as_bytes())
            .expect("append cap fixture");
        file.sync_all().expect("sync cap fixture");
        drop(file);

        let error = log
            .export_lines(None, None)
            .expect_err("legacy export must refuse an over-cap whole-log projection");
        assert_eq!(error.to_string(), "audit-export-legacy-limit");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn typed_export_applies_record_predicates_and_numeric_since() {
        let root = target_scratch_root("audit-typed-filter");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open typed filter audit log");
        log.write_entry("Hello", 1000, "allowed", "operation", "success")
            .expect("write info record");
        log.write_error_entry(
            "ExportBrokerAudit",
            1000,
            "denied-policy",
            "audit-log",
            "policy",
            "redacted",
        )
        .expect("write denied record");
        let filter = serde_json::json!({
            "env": null,
            "operation": "Hello",
            "vm": null,
            "role": null,
            "outcome": "success",
            "severity": "info",
        })
        .to_string();
        let page = log
            .export_page(Some("0"), Some(&filter), None, 10)
            .expect("filter export");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.entries[0]
                .record
                .as_ref()
                .and_then(|record| record.get("op"))
                .and_then(Value::as_str),
            Some("Hello")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn typed_export_surfaces_corruption_and_advances_past_failed_physical_records() {
        let root = target_scratch_root("audit-corrupt-export");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open corrupt export audit log");
        log.write_entry("Hello", 1000, "allowed", "operation", "success")
            .expect("write valid record");
        let path = log.current_daily_path();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open daily file");
        file.write_all(&[0xff, b'\n'])
            .expect("append invalid UTF-8");
        file.write_all(
            b"{\"ts\":1,\"op\":\"AfterCorruption\",\"outcome\":\"denied\",\"decision\":\"denied-policy\"}\n",
        )
        .expect("append valid record after corruption");
        file.sync_all().expect("sync corrupt record");
        drop(file);

        let filter = serde_json::json!({
            "severity": "denied"
        })
        .to_string();
        let first = log
            .export_page(None, Some(&filter), None, 1)
            .expect("export corruption");
        assert_eq!(first.entries.len(), 1);
        assert_eq!(
            first.entries[0].error,
            Some(AuditExportErrorCode::ReadFailed)
        );
        let cursor = first.next_cursor.as_ref().expect("cursor after failure");
        assert_eq!(cursor.line, 1);

        let second = log
            .export_page(None, Some(&filter), Some(cursor), 10)
            .expect("resume after corruption");
        assert_eq!(second.entries.len(), 1);
        assert_eq!(
            second.entries[0]
                .record
                .as_ref()
                .and_then(|record| record.get("op"))
                .and_then(Value::as_str),
            Some("AfterCorruption")
        );
        assert!(second.complete);
        let legacy = log
            .export_lines(None, Some(&filter))
            .expect("legacy export should preserve typed corruption");
        assert_eq!(legacy.len(), 2);
        let legacy_error: Value =
            serde_json::from_str(&legacy[0]).expect("parse legacy corruption entry");
        assert_eq!(legacy_error["export_error"], "read-failed");
        let legacy_record: Value =
            serde_json::from_str(&legacy[1]).expect("parse legacy recovered entry");
        assert_eq!(legacy_record["op"], "AfterCorruption");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn typed_export_filters_redacted_identity_fields_and_severity() {
        let root = target_scratch_root("audit-redacted-filter");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open redacted filter audit log");
        let path = log.current_daily_path();
        let records = [
            serde_json::json!({
                "ts": 1000,
                "env": opaque_digest("work"),
                "vm": opaque_digest("vm-a"),
                "peer_role": opaque_digest("launcher"),
                "operation": "RunHostInstall",
                "outcome": "success",
                "decision": "allowed"
            }),
            serde_json::json!({
                "ts": 2000,
                "env": opaque_digest("work"),
                "vm": opaque_digest("vm-b"),
                "peer_role": opaque_digest("admin"),
                "operation": "RunHostInstall",
                "outcome": "denied",
                "decision": "denied-policy"
            }),
        ];
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open daily file");
        for record in records {
            writeln!(file, "{record}").expect("append filter record");
        }
        file.sync_all().expect("sync filter records");

        let filter = serde_json::json!({
            "env": "work",
            "vm": "vm-a",
            "role": "launcher",
            "operation": "RunHostInstall",
            "outcome": "success",
            "severity": "info"
        })
        .to_string();
        let page = log
            .export_page(Some("1000"), Some(&filter), None, 10)
            .expect("filter redacted records");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.entries[0]
                .record
                .as_ref()
                .and_then(|record| record.get("vm"))
                .and_then(Value::as_str),
            Some(opaque_digest("vm-a").as_str())
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn record_with_join_uses_one_authoritative_operation_key() {
        let root = target_scratch_root("audit-record-join");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open joined audit log");
        let zone = d2b_audit::ZoneId::derive("work").expect("zone");
        let operation =
            d2b_audit::OperationIdentity::derive("authoritative-operation").expect("operation");
        log.record_with_join(
            "RunHostInstall",
            "display-operation",
            1000,
            1000,
            42,
            "d2b-admin",
            "admin",
            "subject",
            "display-scope",
            "run",
            serde_json::json!({}),
            "allowed",
            None,
            None,
            "v3",
            "sha256:bundle",
            1,
            None,
            Some((zone.as_str(), operation.as_str())),
        )
        .expect("joined audit record");
        let line = fs::read_to_string(log.current_daily_path()).expect("read joined record");
        let value: Value = serde_json::from_str(line.lines().next().expect("record line"))
            .expect("parse joined record");
        assert_eq!(
            value.get("public_operation_id").and_then(Value::as_str),
            Some(operation.as_str())
        );
        assert_eq!(
            value.get("operation_identity").and_then(Value::as_str),
            Some(operation.as_str())
        );
        assert_eq!(
            value.get("zone_id").and_then(Value::as_str),
            Some(zone.as_str())
        );
        assert_eq!(
            value
                .get("zone_operation_key")
                .and_then(|key| key.get("zone"))
                .and_then(Value::as_str),
            Some(zone.as_str())
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn audit_drop_warning_state_geometrically_summarizes_drops() {
        let mut state = AuditDropWarningState::default();
        let warnings: Vec<_> = (1..=16)
            .filter_map(|dropped_total| state.observe(AuditWriteClass::Privileged, dropped_total))
            .collect();
        assert_eq!(
            warnings,
            vec![
                AuditDropWarning {
                    dropped_total: 1,
                    dropped_since_previous_warning: 1,
                },
                AuditDropWarning {
                    dropped_total: 2,
                    dropped_since_previous_warning: 1,
                },
                AuditDropWarning {
                    dropped_total: 4,
                    dropped_since_previous_warning: 2,
                },
                AuditDropWarning {
                    dropped_total: 8,
                    dropped_since_previous_warning: 4,
                },
                AuditDropWarning {
                    dropped_total: 16,
                    dropped_since_previous_warning: 8,
                },
            ],
            "warnings should be emitted only at power-of-two totals"
        );

        assert_eq!(
            state.observe(AuditWriteClass::Unprivileged, 1),
            Some(AuditDropWarning {
                dropped_total: 1,
                dropped_since_previous_warning: 1,
            }),
            "each audit class keeps an independent warning cursor"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn unprivileged_drop_counters_remain_exact_when_warnings_are_suppressed() {
        let root = target_scratch_root("audit-drop-summary-aggregate");
        let log = AuditLog::open_with_write_limit(&root, Gid::current().as_raw(), true, 14, 4, true)
            .expect("open audit log with low write limit");
        log.write_entry_with_class(
            AuditWriteClass::Unprivileged,
            "UsbipBind",
            1000,
            "allowed",
            "operation",
            "ok",
        )
        .expect("first write allowed");

        for _ in 0..8 {
            let err = log
                .write_entry_with_class(
                    AuditWriteClass::Unprivileged,
                    "UsbipBind",
                    1000,
                    "allowed",
                    "operation",
                    "ok",
                )
                .expect_err("excess write in same window must be rate-limited");
            assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        }

        let summary = log.audit_drop_summary().expect("drop summary");
        assert_eq!(summary.privileged_rate_limited, 0);
        assert_eq!(summary.unprivileged_rate_limited, 8);
        let warning_state = log.drop_warning_state_snapshot();
        assert_eq!(warning_state.privileged_reported, 0);
        assert_eq!(warning_state.unprivileged_reported, 8);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn privileged_usb_op_records_are_not_rate_limited() {
        let root = target_scratch_root("audit-usb-op-rate-limit");
        let log = AuditLog::open_with_write_limit(&root, Gid::current().as_raw(), true, 14, 1, true)
            .expect("open audit log with low write limit");
        log.record(
            "UsbipBind",
            "usbip-bind",
            1000,
            1000,
            42,
            "d2b-admin",
            "admin",
            "vm:work",
            "usbip",
            "bind",
            serde_json::json!({"bus_id": "redacted"}),
            "allowed",
            None,
            None,
            "v2",
            "fnv1a64:test",
            10,
            Some(serde_json::json!({
                "bus_id": "1-2",
                "vm": "work",
                "device_identity": {
                    "vendorId": "1050",
                    "productId": "0407",
                    "serialObserved": false
                }
            })),
        )
        .expect("first USB op record allowed");
        log.record(
            "UsbipBind",
            "usbip-bind",
            1000,
            1000,
            42,
            "d2b-admin",
            "admin",
            "vm:work",
            "usbip",
            "bind",
            serde_json::json!({"bus_id": "redacted"}),
            "allowed",
            None,
            None,
            "v2",
            "fnv1a64:test",
            10,
            Some(serde_json::json!({
                "bus_id": "1-2",
                "vm": "work",
                "device_identity": {
                    "vendorId": "1050",
                    "productId": "0407",
                    "serialObserved": false
                }
            })),
        )
        .expect("privileged USB audit remains durable under pressure");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn unprivileged_audit_drops_do_not_starve_privileged_usb_records() {
        let root = target_scratch_root("audit-unprivileged-drop-reserve");
        let log = AuditLog::open_with_write_limit(&root, Gid::current().as_raw(), true, 14, 4, true)
            .expect("open audit log with low write limit");

        log.write_entry_with_class(
            AuditWriteClass::Unprivileged,
            "UsbipBind",
            2000,
            "peer-refused",
            "operation",
            "closed",
        )
        .expect("first unprivileged refusal allowed");
        let err = log
            .write_entry_with_class(
                AuditWriteClass::Unprivileged,
                "UsbipBind",
                2000,
                "peer-refused",
                "operation",
                "closed",
            )
            .expect_err("second unprivileged refusal must be dropped");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

        log.record(
            "UsbipBind",
            "usbip-bind",
            0,
            0,
            42,
            "d2b-admin",
            "admin",
            "vm:work",
            "usbip",
            "bind",
            serde_json::json!({"bus_id": "redacted"}),
            "allowed",
            None,
            None,
            "v2",
            "fnv1a64:test",
            10,
            Some(serde_json::json!({
                "bus_id": "1-2",
                "vm": "work",
                "device_identity": {
                    "vendorId": "1050",
                    "productId": "0407",
                    "serialObserved": false
                }
            })),
        )
        .expect("privileged USB op record must retain reserved capacity");

        let summary = log.audit_drop_summary().expect("drop summary");
        assert_eq!(summary.unprivileged_rate_limited, 1);
        assert_eq!(summary.privileged_rate_limited, 0);

        let audit = fs::read_to_string(log.current_daily_path()).expect("read audit log");
        assert_eq!(audit.matches(r#""disposition":"peer-refused""#).count(), 1);
        assert!(audit.contains(r#""operation":"UsbipBind""#), "{audit}");
        assert!(audit.contains(r#""decision":"allowed""#), "{audit}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sanitizer_preserves_numeric_legacy_caller_ids_and_redacts_other_identity() {
        let sanitized = sanitize_audit_value(serde_json::json!({
            "ts": 1,
            "op": "Hello",
            "caller_uid": 4242_u32,
            "caller_gid": 1000_u32,
            "disposition": "peer-refused",
            "opaque_target_id": "daemon-handshake",
            "outcome": "closed",
            "peer_pid": 4242,
            "pid": 4243,
            "pidfd": 9,
            "handle": "opaque-handle",
            "start_time_ticks": 99,
            "expected_start_time_ticks": 100,
            "owner_uid": 1100,
            "owner_gid": 1100,
            "opaque_ref": "cid:42",
            "cgroup_subtree": "d2b.slice/private/role",
            "expectedStartTimeTicks": 101,
            "runtimeAllocations": [{"kind": "vsock-cid", "opaqueRef": "cid:43"}],
            "vmId": "secret-vm-camel",
            "roleId": "secret-role-camel",
            "resourceUid": "123e4567-e89b-42d3-a456-426614174000",
            "path": "/private/host/path",
            "peer_role": "AdminUid(4242)",
            "nested": {
                "caller_uid": 7,
                "caller_gid": 8,
                "role_name": "attacker text"
            }
        }));
        let object = sanitized.as_object().expect("sanitized audit object");

        assert_eq!(object.get("caller_uid").and_then(Value::as_u64), Some(4242));
        assert_eq!(object.get("caller_gid").and_then(Value::as_u64), Some(1000));
        for forbidden in [
            "peer_pid",
            "pid",
            "pidfd",
            "handle",
            "start_time_ticks",
            "expected_start_time_ticks",
            "owner_uid",
            "owner_gid",
            "opaque_ref",
            "cgroup_subtree",
        ] {
            assert!(!object.contains_key(forbidden), "{forbidden}: {object:?}");
        }
        assert!(
            object
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        assert!(
            object
                .get("peer_role")
                .and_then(Value::as_str)
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        let nested = object
            .get("nested")
            .and_then(Value::as_object)
            .expect("nested audit object");
        assert!(!nested.contains_key("caller_uid"));
        assert!(!nested.contains_key("caller_gid"));
        assert!(
            nested
                .get("role_name")
                .and_then(Value::as_str)
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        let string_ids = sanitize_audit_value(serde_json::json!({
            "caller_uid": "uid-name",
            "caller_gid": "gid-name"
        }));
        assert!(
            string_ids
                .as_object()
                .expect("string identity object")
                .is_empty()
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn audit_output_redacts_peer_identity_paths_and_attacker_text() {
        let root = target_scratch_root("audit-redaction-canary");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 14).expect("open audit log");
        log.record(
            "SpawnRunner",
            "operation-canary",
            1000,
            1000,
            4242,
            "d2b-admin",
            "admin",
            "User/secret-user",
            "Zone/secret-zone",
            "spawn",
            serde_json::json!({
                "path": "/private/host/path",
                "argv": ["attacker-text-canary"],
                "env": {"TOKEN": "secret-token-canary"},
                "opaque_ref": "cid:42",
                "expected_start_time_ticks": 99,
                "owner_uid": 1100,
                "cgroup_subtree": "d2b.slice/private/role",
            }),
            "allowed",
            Some("attacker error text"),
            Some("trace-canary"),
            "v3",
            "sha256:bundle",
            1,
            Some(serde_json::json!({
                "vm_id": "secret-vm",
                "socket": "/run/private.sock",
                "pid": 4242,
            })),
        )
        .unwrap();
        let rendered = fs::read_to_string(log.current_daily_path()).unwrap();
        for forbidden in [
            "operation-canary",
            "User/secret-user",
            "Zone/secret-zone",
            "/private/host/path",
            "attacker-text-canary",
            "secret-token-canary",
            "cid:42",
            "cid:43",
            "d2b.slice/private/role",
            "secret-vm-camel",
            "secret-role-camel",
            "secret-vm",
            "/run/private.sock",
        ] {
            assert!(!rendered.contains(forbidden), "{forbidden}: {rendered}");
        }
        assert!(
            rendered.contains(
                d2b_audit::OperationIdentity::derive("operation-canary")
                    .unwrap()
                    .as_str()
            )
        );
        assert!(rendered.contains("\"peer_uid\":1000"));
        assert!(rendered.contains("\"peer_gid\":1000"));
        assert!(!rendered.contains("\"peer_pid\""));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_chain_record_round_trips_through_the_daily_file_with_its_correlation_key() {
        // KTD6 writer contract: a chain record lands as one JSON line in
        // the day's file under the broker's typed write path, and reading
        // it back yields the same record - correlation key included - so
        // audit consumers can count root records per invocation id and
        // correlate nested legs by (invocation id, depth).
        let root = target_scratch_root("audit-chain-record");
        let log =
            AuditLog::open(&root, Gid::current().as_raw(), true, 30).expect("open chain audit log");
        log.write_chain_record(&d2b_audit::evidence_chain::ChainRecord {
            ts_ms: 1234,
            record_class: d2b_audit::evidence_chain::ChainRecordClass::Correlation,
            leg: d2b_audit::evidence_chain::ChainLeg::Broker,
            invocation_id: "invocation-chain-1".to_owned(),
            depth: 2,
            initiating_identity: "provider-alpha".to_owned(),
            invoking_identity: "provider-beta".to_owned(),
            operation: "BetaService".to_owned(),
            zone: "zone-a".to_owned(),
            outcome: d2b_audit::evidence_chain::ChainOutcome::Refused,
            code: Some(d2b_audit::evidence_chain::NESTED_DEPTH_EXCEEDED.to_owned()),
        })
        .expect("write chain record");
        let line = fs::read_to_string(log.current_daily_path()).expect("read chain record");
        let record: d2b_audit::evidence_chain::ChainRecord =
            serde_json::from_str(line.lines().next().expect("record line"))
                .expect("parse chain record");
        assert_eq!(record.ts_ms, 1234);
        assert_eq!(record.record_class, ChainRecordClass::Correlation);
        assert_eq!(record.leg, ChainLeg::Broker);
        assert_eq!(record.invocation_id, "invocation-chain-1");
        assert_eq!(record.depth, 2);
        assert_eq!(record.initiating_identity, "provider-alpha");
        assert_eq!(record.invoking_identity, "provider-beta");
        assert_eq!(record.operation, "BetaService");
        // The typed write path applies the broker-wide redaction rule to
        // zone strings that are not canonical digests - the same rule
        // every other typed record lands under - so the zone is opaque
        // here but the record still round-trips.
        assert!(is_canonical_digest(&record.zone), "{}", record.zone);
        assert_ne!(record.zone, "zone-a");
        assert_eq!(record.outcome, ChainOutcome::Refused);
        assert_eq!(
            record.code.as_deref(),
            Some(d2b_audit::evidence_chain::NESTED_DEPTH_EXCEEDED)
        );
        assert_eq!(record.correlation_key(), ("invocation-chain-1", 2));
        assert!(!record.is_root());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cancelled_append_completes_without_partial_line() {
        // U6 invariant: the audit worker owns the append transaction to
        // completion. A caller that cancels mid-append (drops its reply
        // before the worker finishes) must not leak a partial JSONL line:
        // the worker finishes the write+fsync unit and the next append
        // proceeds normally.
        let root = target_scratch_root("audit-cancel-append");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open audit log");
        log.write_entry("Baseline", 1000, "allowed", "t", "success")
            .expect("baseline record");

        // Stall the worker inside the next append transaction.
        let (stalled_tx, stalled_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        log.inject_io_failure(InjectedAuditIoFailure::Stall {
            stalled: stalled_tx,
            release: release_rx,
        })
        .expect("install stall");
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        log.writer
            .commands
            .send(AuditCommand::Append {
                audit_class: AuditWriteClass::Privileged,
                operation: "CancelledOperation".to_owned(),
                bytes: b"{\"ts\":2,\"op\":\"cancelled\"}\n".to_vec(),
                #[cfg(test)]
                capture_record: None,
                reply: reply_tx,
            })
            .expect("send append");
        // The worker is now mid-append, parked inside the transaction.
        stalled_rx.blocking_recv().expect("append started");
        // The caller cancels: drop the reply receiver before the append
        // finishes. The worker still owns the append to completion.
        drop(reply_rx);
        release_tx.send(()).expect("release the append");

        // A barrier append proves the worker continued and drained.
        log.write_entry("AfterCancellation", 1000, "allowed", "t", "success")
            .expect("worker continues after the cancelled caller");
        let contents = fs::read_to_string(log.current_daily_path()).expect("read audit");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3, "baseline + cancelled + barrier, all complete");
        assert!(
            lines
                .iter()
                .all(|line| serde_json::from_str::<Value>(line).is_ok()),
            "no partial line may survive a cancelled caller: {contents}"
        );
        assert!(contents.contains("\"op\":\"cancelled\""), "{contents}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn crash_between_records_leaves_durable_prefix_only() {
        // U6 invariant: per-record durability barrier. A crash between two
        // records (the next append dies mid-write, before sync) leaves only
        // the fsynced prefix as authoritative; the partial tail is
        // quarantined on reopen, never mistaken for a record.
        let root = target_scratch_root("audit-crash-prefix");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open audit log");
        log.write_entry("First", 1000, "allowed", "t", "success")
            .expect("first record");
        let before = fs::read(log.current_daily_path()).expect("read durable prefix");

        log.inject_crash_after_write().expect("arm crash");
        let (reply_tx, _reply_rx) = mpsc::sync_channel(1);
        log.writer
            .commands
            .send(AuditCommand::Append {
                audit_class: AuditWriteClass::Privileged,
                operation: "CrashingOperation".to_owned(),
                bytes: b"{\"ts\":2,\"op\":\"crashing\"}\n".to_vec(),
                #[cfg(test)]
                capture_record: None,
                reply: reply_tx,
            })
            .expect("send crashing append");
        // The worker died mid-append: half the line is in the page cache,
        // no sync, no rollback, no reply - the channel is closed.
        drop(log);

        // Restart: the durable prefix is all that survives, and the partial
        // tail is quarantined.
        let reopened = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("reopen after crash");
        assert_eq!(
            fs::read(reopened.current_daily_path()).expect("read reconciled audit"),
            before,
            "only the fsynced prefix survives the crash"
        );
        let quarantines: Vec<_> = fs::read_dir(&root)
            .expect("read audit directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".quarantine"))
            .collect();
        assert_eq!(
            quarantines.len(),
            1,
            "the partial tail must be quarantined, not appended to"
        );
        reopened
            .write_entry("AfterCrash", 1000, "allowed", "t", "success")
            .expect("append after restart");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn queue_full_unprivileged_appends_are_dropped_with_accounting() {
        // U6 invariant: the append queue is bounded. A full queue drops
        // unprivileged appends with accounting (matching AuditDropSummary)
        // instead of growing or silently losing the count; privileged
        // appends (the mutation-success boundary) are never dropped.
        let root = target_scratch_root("audit-queue-full-accounting");
        let log = AuditLog::open(&root, Gid::current().as_raw(), true, 30)
            .expect("open audit log");

        // Stall the worker inside one append so the bounded queue fills.
        let (stalled_tx, stalled_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        log.inject_io_failure(InjectedAuditIoFailure::Stall {
            stalled: stalled_tx,
            release: release_rx,
        })
        .expect("install stall");
        let (reply_tx, _reply_rx) = mpsc::sync_channel(1);
        log.writer
            .commands
            .send(AuditCommand::Append {
                audit_class: AuditWriteClass::Privileged,
                operation: "StalledOperation".to_owned(),
                bytes: b"{\"ts\":1,\"op\":\"stalled\"}\n".to_vec(),
                #[cfg(test)]
                capture_record: None,
                reply: reply_tx,
            })
            .expect("send stalled append");
        stalled_rx.blocking_recv().expect("append stalled");

        // Fill the bounded queue with raw append commands (their replies are
        // dropped: the worker is stalled, so nobody waits on them). The
        // overflow is refused by the queue, never enqueued.
        let mut admitted = 0usize;
        for _ in 0..(AUDIT_WORKER_QUEUE_DEPTH + 1) {
            let (reply_tx, _reply_rx) = mpsc::sync_channel(1);
            match log.writer.commands.try_send(AuditCommand::Append {
                audit_class: AuditWriteClass::Unprivileged,
                operation: "UsbipBind".to_owned(),
                bytes: b"{\"ts\":1,\"op\":\"queued\"}\n".to_vec(),
                #[cfg(test)]
                capture_record: None,
                reply: reply_tx,
            }) {
                Ok(()) => admitted += 1,
                Err(mpsc::TrySendError::Full(_)) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    panic!("audit worker died while the queue filled")
                }
            }
        }
        assert_eq!(
            admitted, AUDIT_WORKER_QUEUE_DEPTH,
            "the bounded queue admits exactly its depth"
        );
        // One more append through the public path: the full queue drops it
        // with accounting (drop-with-accounting matching AuditDropSummary)
        // and the caller sees the WouldBlock refusal immediately.
        let err = log
            .write_entry_with_class(
                AuditWriteClass::Unprivileged,
                "UsbipBind",
                2000,
                "peer-refused",
                "operation",
                "closed",
            )
            .expect_err("a full queue refuses the next unprivileged append");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        release_tx.send(()).expect("release the stall");

        // Drain barrier: every admitted append lands as a complete line.
        log.write_entry("DrainBarrier", 1000, "allowed", "t", "success")
            .expect("barrier after drain");
        let summary = log.audit_drop_summary().expect("drop summary");
        assert_eq!(
            summary.unprivileged_rate_limited, 1,
            "exactly one queue-full drop is accounted"
        );
        assert_eq!(summary.privileged_rate_limited, 0);
        let contents = fs::read_to_string(log.current_daily_path()).expect("read audit");
        assert_eq!(
            contents.lines().count(),
            admitted + 2,
            "stalled + admitted + barrier, all complete lines"
        );
        assert!(
            contents
                .lines()
                .all(|line| serde_json::from_str::<Value>(line).is_ok()),
            "no partial line may survive the queue-full drain: {contents}"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
