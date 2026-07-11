//! Daemon-side JSONL audit events emitted by `d2bd` for transitions
//! not covered by the broker's `OpAuditRecord` stream.
//!
//! Events are written to
//! `{daemon_state_dir}/daemon-events-{YYYY-MM-DD}.jsonl` (daemon-owned,
//! separate from the broker's `broker-{date}.jsonl` files). Each line is
//! a self-contained JSON object carrying `ts_ms` + `source` + a
//! per-variant `event` object plus `prev_hash` / `record_hash` hash-chain
//! fields. The record hash is SHA-256 over a stable length-prefixed payload
//! of source, timestamp, previous hash, and canonical event JSON; it never
//! includes `record_hash` itself.
//!
//! # Additive-only contract
//!
//! `DaemonEvent` is `#[non_exhaustive]`. New variants MAY be added in
//! any release; existing variants MUST NOT be renamed or removed. Field
//! names use `snake_case` (matching the `#[serde(rename_all = "snake_case")]`
//! attribute). This mirrors the broker audit's forward-compat posture.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Closed detached exec audit action.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DetachedExecAuditAction {
    Create,
    Cancel,
}

/// Closed detached exec audit result.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DetachedExecAuditResult {
    Created,
    Cancelling,
    AlreadyTerminal,
    Error,
}

/// Closed persistent-shell owner action.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShellAuditAction {
    Create,
    Attach,
    List,
    Detach,
    Kill,
    Close,
    Failure,
}

/// Closed persistent-shell owner/management result.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShellAuditResult {
    Requested,
    Attached,
    Listed,
    Detached,
    Killed,
    Closed,
    Refused,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShellAuditProvider {
    GuestControl,
    UnsafeLocal,
}

/// Closed reason a vm-start runner node fast-failed before readiness.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VmStartRunnerExitReason {
    /// The spawned runner terminated before its readiness signal fired.
    RunnerExited,
    /// The runner's PID was reused by a different process (start-time
    /// drift) — our runner is gone.
    RunnerReused,
}

/// Closed, bounded runner exit kind mirrored from the broker reap
/// notification. Carries no high-cardinality detail.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerExitKind {
    Exited,
    Signaled,
    Killed,
}

/// Closed provider labels for daemon-side VM shutdown audit.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VmShutdownProvider {
    CloudHypervisor,
    QemuMedia,
    Unknown,
}

/// Closed final outcome for provider-aware VM shutdown.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VmShutdownOutcome {
    CleanGuestShutdown,
    CleanVmmCleanup,
    ApiUnavailable,
    TimeoutExceeded,
    ForceRequested,
    Disabled,
    ForcedCleanup,
    CleanupFailed,
}

/// Daemon-side audit event variants.
///
/// Additive-only: new variants may be added; existing ones must not be
/// renamed or removed. `#[non_exhaustive]` enforces this at the type level.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum DaemonEvent {
    /// Bounded configured-launch lifecycle boundary. Target and item identity
    /// are trusted bundle tokens; execution details never enter this record.
    WorkloadLauncher {
        target: String,
        item_id: String,
        operation_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        exec_id: Option<String>,
        peer_uid: u32,
        provider: WorkloadLaunchProvider,
        result: WorkloadLaunchResult,
    },
    /// Emitted when the api-ready phase of a VM start does not converge
    /// within the configured timeout in strict split-readiness mode.
    ApiReadyTimeout {
        /// VM name (matches the `vmStart` request).
        vm: String,
        /// Role id of the runner node whose api-ready probe timed out.
        runner: String,
        /// Configured timeout that elapsed, in whole seconds.
        elapsed_secs: u64,
        /// Split-readiness mode: `"strict"` or `"no-wait-api"`.
        mode: String,
    },
    /// Emitted when an authenticated `vm exec` owner session is established
    /// (after admin authz + capability negotiation, before any op proxy).
    ///
    /// Leak-safe by construction: carries ONLY the VM name, the admin peer
    /// uid, and the negotiated tty shape. The opaque session handle, argv,
    /// env, cwd, and any stdio bytes are NEVER recorded.
    GuestControlExecEstablished {
        /// VM name the exec session targets.
        vm: String,
        /// Admin peer uid (from `SO_PEERCRED`) that opened the session.
        peer_uid: u32,
        /// Whether a PTY was negotiated for the session.
        tty: bool,
    },
    /// Emitted when a previously-established `vm exec` owner session ends
    /// (owner disconnect, command terminal, or teardown).
    ///
    /// Leak-safe: carries ONLY the VM name and the admin peer uid. No
    /// session handle, exit status bytes, argv, env, cwd, or stdio.
    GuestControlExecTerminated {
        /// VM name the exec session targeted.
        vm: String,
        /// Admin peer uid (from `SO_PEERCRED`) that owned the session.
        peer_uid: u32,
    },
    /// Emitted when an authenticated persistent shell owner attachment is
    /// established.
    ///
    /// Leak-safe: carries ONLY the VM name, admin peer uid, and whether a force
    /// takeover was requested. Shell names, session handles, and terminal bytes
    /// are never recorded.
    GuestControlShellAttached {
        /// VM name the shell attachment targets.
        vm: String,
        /// Admin peer uid (from `SO_PEERCRED`) that opened the attachment.
        peer_uid: u32,
        /// Closed shell action.
        action: ShellAuditAction,
        /// Closed shell result.
        result: ShellAuditResult,
        /// Fixed-length non-raw correlation digest for the targeted shell. This
        /// is safe to record; raw shell names/session ids are never written.
        shell_ref_digest: String,
        /// Whether the caller requested force takeover.
        force: bool,
    },
    /// Emitted when a persistent shell owner attachment ends.
    ///
    /// Leak-safe: carries ONLY the VM name, admin peer uid, and a closed result
    /// enum. No shell name, session handle, or terminal bytes.
    GuestControlShellDetached {
        /// VM name the shell attachment targeted.
        vm: String,
        /// Admin peer uid (from `SO_PEERCRED`) that owned the attachment.
        peer_uid: u32,
        /// Closed shell action.
        action: ShellAuditAction,
        /// Closed teardown result.
        result: ShellAuditResult,
        /// Fixed-length non-raw correlation digest for the targeted shell. This
        /// is safe to record; raw shell names/session ids are never written.
        shell_ref_digest: String,
    },
    /// Provider-neutral persistent-shell lifecycle boundary. `target` is either
    /// a configured canonical workload target or a local VM identifier.
    /// Operation/session values are fixed-length digests; raw shell names,
    /// handles, supervisor metadata, terminal bytes, paths, and diagnostics are
    /// never present.
    ShellLifecycle {
        target: String,
        peer_uid: u32,
        provider: ShellAuditProvider,
        action: ShellAuditAction,
        result: ShellAuditResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        force: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        operation_digest: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_digest: Option<String>,
    },
    /// Emitted when a detached `vm exec -d` create succeeds.
    ///
    /// Leak-safe: carries ONLY the VM name, admin peer uid, closed
    /// action/result enums, and the opaque guest exec id. argv, env, cwd, and
    /// retained output bytes are never recorded.
    GuestControlExecDetachedCreate {
        vm: String,
        peer_uid: u32,
        action: DetachedExecAuditAction,
        result: DetachedExecAuditResult,
        exec_id: String,
    },
    /// Emitted when a detached `vm exec <vm> kill <id>` cancel path is
    /// attempted.
    ///
    /// Leak-safe: carries ONLY the VM name, admin peer uid, closed
    /// action/result enums, and the opaque target exec id. No argv, env, cwd,
    /// or log bytes.
    GuestControlExecDetachedKill {
        vm: String,
        peer_uid: u32,
        action: DetachedExecAuditAction,
        result: DetachedExecAuditResult,
        exec_id: String,
    },
    /// Emitted when a `vm start` long-lived runner node fast-fails because
    /// the spawned runner terminated (or its PID was reused) BEFORE its
    /// readiness signal fired — the `tpm.enable` first-run wedge fix.
    ///
    /// Bounded by construction: carries ONLY the VM name, the closed
    /// `role_id` of the failed node, a closed reason kind, the optional
    /// closed broker exit kind/code/signal, and the elapsed wall-clock
    /// milliseconds. No node-reason string, pid, or path label is
    /// recorded.
    VmStartRunnerExited {
        /// VM name (matches the `vmStart` request).
        vm: String,
        /// Role id of the runner node that exited (e.g. `swtpm`,
        /// `ch-runner`).
        role_id: String,
        /// Closed reason kind: exited vs PID-reused.
        reason_kind: VmStartRunnerExitReason,
        /// Bounded broker exit kind, when a reap status was buffered.
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_kind: Option<RunnerExitKind>,
        /// Exit code, when `exit_kind == "exited"`.
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        /// Signal number, when `exit_kind` is `signaled`/`killed`.
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_signal: Option<i32>,
        /// Wall-clock milliseconds from DAG dispatch to fast-fail.
        elapsed_ms: u64,
    },
    /// Emitted before a VM stop sends a provider shutdown request or force
    /// cleanup signal.
    VmShutdownIntent {
        vm: String,
        peer_uid: u32,
        provider: VmShutdownProvider,
        force_requested: bool,
        timeout_secs: u64,
    },
    /// Emitted after a VM stop reaches a terminal graceful/fallback outcome.
    VmShutdownOutcome {
        vm: String,
        peer_uid: u32,
        provider: VmShutdownProvider,
        outcome: VmShutdownOutcome,
        elapsed_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadLaunchProvider {
    LocalVm,
    UnsafeLocal,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadLaunchResult {
    Committed,
    AlreadyCommitted,
    Refused,
    Failed,
}

/// JSONL audit-log writer for daemon-side events.
///
/// - **Production**: use [`DaemonAuditLog::new`]; events are appended to
///   the day's `daemon-events-{YYYY-MM-DD}.jsonl` file inside the
///   daemon-state directory.
/// - **Tests that don't care about audit output**: use
///   [`DaemonAuditLog::no_op`]; all writes are silently discarded.
///
/// Appends are serialized behind a single in-process writer mutex so
/// concurrent connection-handler threads cannot interleave bytes within
/// a JSONL line. Retention is enforced best-effort: stale
/// `daemon-events-*.jsonl` files older than [`AUDIT_RETENTION_DAYS`] are
/// pruned on open and again whenever a write crosses a day boundary
/// (the file name itself provides day-boundary rotation).
#[derive(Debug)]
pub struct DaemonAuditLog {
    state_dir: Option<PathBuf>,
    writer: Arc<Mutex<AuditWriterState>>,
    #[cfg(test)]
    pub(crate) captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

/// Default retention window for daemon audit JSONL files (days).
pub const AUDIT_RETENTION_DAYS: i64 = 14;

/// Minimum daemon audit retention floor used by the default health helper.
pub const AUDIT_RETENTION_FLOOR_DAYS: i64 = 7;

/// First-link marker for a daemon audit hash chain.
pub const DAEMON_AUDIT_GENESIS_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

const DAEMON_AUDIT_SOURCE: &str = "d2bd";
const DAEMON_AUDIT_HASH_DOMAIN: &[u8] = b"d2bd-daemon-audit-v1";
const DAEMON_AUDIT_TAIL_CHUNK_BYTES: u64 = 8192;
const MAX_DAEMON_AUDIT_TAIL_LINE_BYTES: usize = 1024 * 1024;
static HEALTHCHECK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Serialization + day-boundary bookkeeping for the single audit appender.
#[derive(Debug, Default)]
struct AuditWriterState {
    /// UTC date string (`YYYY-MM-DD`) of the most recent append. Used to
    /// detect a day-boundary crossing so retention pruning re-runs.
    last_date: Option<String>,
    /// Hash of the last record this process successfully emitted, or the
    /// newest on-disk record discovered before the first append.
    last_hash: Option<String>,
    /// Whether `last_hash` has been initialized from existing on-disk daemon
    /// audit records for this process.
    initialized_from_disk: bool,
}

/// Overall daemon audit sink health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonAuditSinkStatus {
    Ok,
    Degraded,
    Unavailable,
}

/// Bounded daemon audit sink health problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DaemonAuditSinkProblem {
    /// No state directory is configured (for example, a no-op test sink).
    NoStateDir,
    /// Effective retention is lower than the required floor.
    RetentionBelowFloor {
        configured_days: i64,
        required_floor_days: i64,
    },
    /// Creating the daemon state directory failed.
    CreateStateDirFailed { error_kind: String },
    /// Opening/listing the daemon state directory failed.
    OpenStateDirFailed { error_kind: String },
    /// Opening the bounded write probe failed.
    OpenProbeFailed { error_kind: String },
    /// Writing the bounded probe failed.
    WriteProbeFailed { error_kind: String },
    /// Flushing the bounded probe failed.
    FlushProbeFailed { error_kind: String },
    /// Removing the bounded probe failed after a successful write.
    CleanupProbeFailed { error_kind: String },
}

impl DaemonAuditSinkProblem {
    fn makes_sink_unavailable(&self) -> bool {
        matches!(
            self,
            Self::NoStateDir
                | Self::CreateStateDirFailed { .. }
                | Self::OpenStateDirFailed { .. }
                | Self::OpenProbeFailed { .. }
                | Self::WriteProbeFailed { .. }
                | Self::FlushProbeFailed { .. }
        )
    }
}

/// Explicit daemon audit sink health report. It intentionally carries no
/// filesystem path or raw IO message so state-dir, argv/env, and secret
/// canaries cannot leak through health JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonAuditSinkHealthReport {
    pub source: String,
    pub status: DaemonAuditSinkStatus,
    pub writable: bool,
    pub retention_days: i64,
    pub required_retention_floor_days: i64,
    pub problems: Vec<DaemonAuditSinkProblem>,
}

impl DaemonAuditSinkHealthReport {
    fn unavailable(
        retention_days: i64,
        required_retention_floor_days: i64,
        problem: DaemonAuditSinkProblem,
    ) -> Self {
        Self::from_parts(
            false,
            retention_days,
            required_retention_floor_days,
            vec![problem],
        )
    }

    fn from_parts(
        writable: bool,
        retention_days: i64,
        required_retention_floor_days: i64,
        problems: Vec<DaemonAuditSinkProblem>,
    ) -> Self {
        let status = if problems
            .iter()
            .any(DaemonAuditSinkProblem::makes_sink_unavailable)
        {
            DaemonAuditSinkStatus::Unavailable
        } else if problems.is_empty() {
            DaemonAuditSinkStatus::Ok
        } else {
            DaemonAuditSinkStatus::Degraded
        };
        Self {
            source: DAEMON_AUDIT_SOURCE.to_owned(),
            status,
            writable,
            retention_days,
            required_retention_floor_days,
            problems,
        }
    }
}

/// One daemon audit hash-chain problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DaemonAuditChainProblem {
    ParseError {
        message: String,
    },
    MissingField {
        field: String,
    },
    WrongFieldType {
        field: String,
        expected: String,
        actual: String,
    },
    SourceMismatch {
        expected: String,
        actual: String,
    },
    MalformedHash {
        field: String,
    },
    PreviousHashMismatch {
        expected: String,
        actual: String,
    },
    RecordHashMismatch {
        expected: String,
        actual: String,
    },
}

/// One daemon audit line plus its hash-chain problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonAuditChainDefect {
    pub line_index: usize,
    pub source_file: Option<String>,
    pub problem: DaemonAuditChainProblem,
}

/// Aggregated daemon audit hash-chain verification report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonAuditChainReport {
    pub records_scanned: usize,
    pub records_ok: usize,
    pub defects: Vec<DaemonAuditChainDefect>,
}

impl DaemonAuditChainReport {
    pub fn is_clean(&self) -> bool {
        self.defects.is_empty()
    }
}

impl DaemonAuditLog {
    /// Production constructor. Events are appended to the day's JSONL
    /// file under `state_dir`. Prunes stale logs once on open
    /// (best-effort).
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        prune_old_audit_logs(&state_dir, AUDIT_RETENTION_DAYS);
        Self {
            state_dir: Some(state_dir),
            writer: Arc::new(Mutex::new(AuditWriterState::default())),
            #[cfg(test)]
            captured: Default::default(),
        }
    }

    /// No-op constructor for tests that do not exercise audit output.
    pub fn no_op() -> Self {
        Self {
            state_dir: None,
            writer: Arc::new(Mutex::new(AuditWriterState::default())),
            #[cfg(test)]
            captured: Default::default(),
        }
    }

    /// Serialize and append one `DaemonEvent` JSONL line.
    ///
    /// This method is best-effort: callers MUST NOT abort the surrounding
    /// operation on audit failure. They should log the error (if any) and
    /// continue.
    ///
    /// The actual file append is serialized behind a single writer mutex
    /// so concurrent handler threads produce a valid, line-atomic JSONL
    /// stream. A day-boundary crossing triggers best-effort retention
    /// pruning of stale `daemon-events-*.jsonl` files.
    pub fn write_event(&self, event: &DaemonEvent) -> io::Result<()> {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let event_value = serde_json::to_value(event)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut writer = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("DaemonAuditLog writer mutex poisoned"))?;
        if let Some(ref state_dir) = self.state_dir {
            initialize_chain_from_disk(state_dir, &mut writer);
        }

        let prev_hash = writer
            .last_hash
            .as_deref()
            .unwrap_or(DAEMON_AUDIT_GENESIS_HASH);
        let (record, record_hash) = build_chained_record(ts_ms, &event_value, prev_hash)?;
        let mut line = serde_json::to_string(&record)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        if let Some(ref state_dir) = self.state_dir {
            let today = utc_date_string();
            // First write of the process or a day-boundary crossing:
            // re-run retention pruning (best-effort) before appending.
            if writer.last_date.as_deref() != Some(today.as_str()) {
                prune_old_audit_logs(state_dir, AUDIT_RETENTION_DAYS);
                writer.last_date = Some(today.clone());
            }
            write_jsod2b_line_for_date(state_dir, &today, &line)?;
        }

        writer.last_hash = Some(record_hash);

        #[cfg(test)]
        {
            self.captured
                .lock()
                .map_err(|_| io::Error::other("DaemonAuditLog capture mutex poisoned"))?
                .push(line.trim_end_matches('\n').to_owned());
        }
        Ok(())
    }

    /// Report explicit daemon audit sink health without changing
    /// [`Self::write_event`]'s best-effort caller contract.
    pub fn sink_health_report(&self) -> DaemonAuditSinkHealthReport {
        self.sink_health_report_with_floor(AUDIT_RETENTION_FLOOR_DAYS)
    }

    /// Report explicit daemon audit sink health against a caller-provided
    /// retention floor.
    pub fn sink_health_report_with_floor(
        &self,
        required_retention_floor_days: i64,
    ) -> DaemonAuditSinkHealthReport {
        match self.state_dir.as_deref() {
            Some(state_dir) => daemon_audit_sink_health_report(
                state_dir,
                AUDIT_RETENTION_DAYS,
                required_retention_floor_days,
            ),
            None => DaemonAuditSinkHealthReport::unavailable(
                AUDIT_RETENTION_DAYS,
                required_retention_floor_days,
                DaemonAuditSinkProblem::NoStateDir,
            ),
        }
    }
}

/// Probe daemon audit sink health without writing an audit record.
pub fn daemon_audit_sink_health_report(
    state_dir: &Path,
    configured_retention_days: i64,
    required_retention_floor_days: i64,
) -> DaemonAuditSinkHealthReport {
    let mut problems = Vec::new();
    if configured_retention_days < required_retention_floor_days {
        problems.push(DaemonAuditSinkProblem::RetentionBelowFloor {
            configured_days: configured_retention_days,
            required_floor_days: required_retention_floor_days,
        });
    }

    if let Err(err) = fs::create_dir_all(state_dir) {
        problems.push(DaemonAuditSinkProblem::CreateStateDirFailed {
            error_kind: io_error_kind(err.kind()).to_owned(),
        });
        return DaemonAuditSinkHealthReport::from_parts(
            false,
            configured_retention_days,
            required_retention_floor_days,
            problems,
        );
    }

    if let Err(err) = fs::read_dir(state_dir) {
        problems.push(DaemonAuditSinkProblem::OpenStateDirFailed {
            error_kind: io_error_kind(err.kind()).to_owned(),
        });
        return DaemonAuditSinkHealthReport::from_parts(
            false,
            configured_retention_days,
            required_retention_floor_days,
            problems,
        );
    }

    let probe_path = unique_health_probe_path(state_dir);
    let mut writable = false;
    let mut opened_probe = false;
    match fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe_path)
    {
        Ok(mut file) => {
            opened_probe = true;
            match file.write_all(b"d2bd-daemon-audit-health\n") {
                Ok(()) => match file.flush() {
                    Ok(()) => writable = true,
                    Err(err) => problems.push(DaemonAuditSinkProblem::FlushProbeFailed {
                        error_kind: io_error_kind(err.kind()).to_owned(),
                    }),
                },
                Err(err) => problems.push(DaemonAuditSinkProblem::WriteProbeFailed {
                    error_kind: io_error_kind(err.kind()).to_owned(),
                }),
            }
        }
        Err(err) => problems.push(DaemonAuditSinkProblem::OpenProbeFailed {
            error_kind: io_error_kind(err.kind()).to_owned(),
        }),
    }

    if opened_probe && let Err(err) = fs::remove_file(&probe_path) {
        problems.push(DaemonAuditSinkProblem::CleanupProbeFailed {
            error_kind: io_error_kind(err.kind()).to_owned(),
        });
    }

    DaemonAuditSinkHealthReport::from_parts(
        writable,
        configured_retention_days,
        required_retention_floor_days,
        problems,
    )
}

/// Verify daemon audit JSONL records as one strict hash-chain sequence.
pub fn verify_daemon_audit_lines<'a, I>(lines: I) -> DaemonAuditChainReport
where
    I: IntoIterator<Item = (Option<&'a str>, &'a str)>,
{
    let mut report = DaemonAuditChainReport {
        records_scanned: 0,
        records_ok: 0,
        defects: Vec::new(),
    };
    let mut expected_prev_hash = DAEMON_AUDIT_GENESIS_HASH.to_owned();

    for (idx, (source_file, line)) in lines.into_iter().enumerate() {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        report.records_scanned += 1;
        let line_index = idx + 1;
        let source_owned = source_file.map(str::to_owned);
        let mut problems = match validate_daemon_audit_chain_line(trimmed, &expected_prev_hash) {
            Ok(record_hash) => {
                expected_prev_hash = record_hash;
                report.records_ok += 1;
                Vec::new()
            }
            Err((line_problems, next_prev)) => {
                if let Some(record_hash) = next_prev {
                    expected_prev_hash = record_hash;
                }
                line_problems
            }
        };
        for problem in problems.drain(..) {
            report.defects.push(DaemonAuditChainDefect {
                line_index,
                source_file: source_owned.clone(),
                problem,
            });
        }
    }

    report
}

fn validate_daemon_audit_chain_line(
    line: &str,
    expected_prev_hash: &str,
) -> Result<String, (Vec<DaemonAuditChainProblem>, Option<String>)> {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            return Err((
                vec![DaemonAuditChainProblem::ParseError {
                    message: err.to_string(),
                }],
                None,
            ));
        }
    };
    let Some(obj) = value.as_object() else {
        return Err((
            vec![DaemonAuditChainProblem::ParseError {
                message: format!("expected JSON object, got {}", json_value_kind(&value)),
            }],
            None,
        ));
    };

    let mut problems = Vec::new();
    let ts_ms = match obj.get("ts_ms") {
        Some(Value::Number(number)) => match number.as_u64() {
            Some(ts_ms) => Some(u128::from(ts_ms)),
            None => {
                problems.push(DaemonAuditChainProblem::WrongFieldType {
                    field: "ts_ms".to_owned(),
                    expected: "unsigned-integer".to_owned(),
                    actual: "number".to_owned(),
                });
                None
            }
        },
        Some(value) => {
            problems.push(DaemonAuditChainProblem::WrongFieldType {
                field: "ts_ms".to_owned(),
                expected: "number".to_owned(),
                actual: json_value_kind(value).to_owned(),
            });
            None
        }
        None => {
            problems.push(DaemonAuditChainProblem::MissingField {
                field: "ts_ms".to_owned(),
            });
            None
        }
    };

    let source = match obj.get("source") {
        Some(Value::String(source)) => {
            if source != DAEMON_AUDIT_SOURCE {
                problems.push(DaemonAuditChainProblem::SourceMismatch {
                    expected: DAEMON_AUDIT_SOURCE.to_owned(),
                    actual: source.clone(),
                });
            }
            Some(source.as_str())
        }
        Some(value) => {
            problems.push(DaemonAuditChainProblem::WrongFieldType {
                field: "source".to_owned(),
                expected: "string".to_owned(),
                actual: json_value_kind(value).to_owned(),
            });
            None
        }
        None => {
            problems.push(DaemonAuditChainProblem::MissingField {
                field: "source".to_owned(),
            });
            None
        }
    };

    let event = match obj.get("event") {
        Some(Value::Object(_)) => obj.get("event"),
        Some(value) => {
            problems.push(DaemonAuditChainProblem::WrongFieldType {
                field: "event".to_owned(),
                expected: "object".to_owned(),
                actual: json_value_kind(value).to_owned(),
            });
            None
        }
        None => {
            problems.push(DaemonAuditChainProblem::MissingField {
                field: "event".to_owned(),
            });
            None
        }
    };

    let prev_hash = extract_hash_field(obj, "prev_hash", &mut problems);
    let record_hash = extract_hash_field(obj, "record_hash", &mut problems);

    if let Some(prev_hash) = prev_hash
        && prev_hash != expected_prev_hash
    {
        problems.push(DaemonAuditChainProblem::PreviousHashMismatch {
            expected: expected_prev_hash.to_owned(),
            actual: prev_hash.to_owned(),
        });
    }

    if let (Some(ts_ms), Some(source), Some(event), Some(prev_hash), Some(record_hash)) =
        (ts_ms, source, event, prev_hash, record_hash)
        && source == DAEMON_AUDIT_SOURCE
    {
        match compute_record_hash(ts_ms, source, event, prev_hash) {
            Ok(expected) if expected != record_hash => {
                problems.push(DaemonAuditChainProblem::RecordHashMismatch {
                    expected,
                    actual: record_hash.to_owned(),
                });
            }
            Ok(_) => {}
            Err(err) => problems.push(DaemonAuditChainProblem::ParseError {
                message: err.to_string(),
            }),
        }
    }

    if problems.is_empty() {
        Ok(record_hash
            .expect("record_hash present when problems is empty")
            .to_owned())
    } else {
        Err((problems, record_hash.map(str::to_owned)))
    }
}

fn build_chained_record(
    ts_ms: u128,
    event: &Value,
    prev_hash: &str,
) -> io::Result<(Value, String)> {
    let record_hash = compute_record_hash(ts_ms, DAEMON_AUDIT_SOURCE, event, prev_hash)?;
    Ok((
        serde_json::json!({
            "ts_ms": ts_ms,
            "source": DAEMON_AUDIT_SOURCE,
            "prev_hash": prev_hash,
            "record_hash": record_hash.clone(),
            "event": event,
        }),
        record_hash,
    ))
}

fn compute_record_hash(
    ts_ms: u128,
    source: &str,
    event: &Value,
    prev_hash: &str,
) -> io::Result<String> {
    let event_bytes = canonical_json_bytes(event)?;
    let mut hasher = Sha256::new();
    hash_component(&mut hasher, DAEMON_AUDIT_HASH_DOMAIN);
    hash_component(&mut hasher, source.as_bytes());
    hash_component(&mut hasher, ts_ms.to_string().as_bytes());
    hash_component(&mut hasher, prev_hash.as_bytes());
    hash_component(&mut hasher, &event_bytes);
    Ok(hex_lower(&hasher.finalize()))
}

fn initialize_chain_from_disk(state_dir: &Path, writer: &mut AuditWriterState) {
    if writer.initialized_from_disk {
        return;
    }
    writer.last_hash = last_daemon_record_hash_on_disk(state_dir);
    writer.initialized_from_disk = true;
}

fn last_daemon_record_hash_on_disk(state_dir: &Path) -> Option<String> {
    let mut files = discover_daemon_daily_files(state_dir).ok()?;
    files.sort();
    for path in files.iter().rev() {
        if let Some(hash) = last_daemon_record_hash_in_file(path) {
            return Some(hash);
        }
    }
    None
}

fn last_daemon_record_hash_in_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut position = file.seek(SeekFrom::End(0)).ok()?;
    let mut reversed_line = Vec::new();
    let mut skipping_oversized_line = false;

    while position > 0 {
        let read_len = position.min(DAEMON_AUDIT_TAIL_CHUNK_BYTES) as usize;
        position -= read_len as u64;
        file.seek(SeekFrom::Start(position)).ok()?;
        let mut chunk = vec![0_u8; read_len];
        file.read_exact(&mut chunk).ok()?;

        for byte in chunk.iter().rev().copied() {
            if byte == b'\n' {
                if skipping_oversized_line {
                    reversed_line.clear();
                    skipping_oversized_line = false;
                    continue;
                }
                if reversed_line.is_empty() {
                    continue;
                }
                if let Some(hash) = record_hash_from_reversed_line(&reversed_line) {
                    return Some(hash);
                }
                reversed_line.clear();
                continue;
            }

            if skipping_oversized_line {
                continue;
            }
            if reversed_line.len() >= MAX_DAEMON_AUDIT_TAIL_LINE_BYTES {
                reversed_line.clear();
                skipping_oversized_line = true;
                continue;
            }
            reversed_line.push(byte);
        }
    }

    if !skipping_oversized_line && !reversed_line.is_empty() {
        record_hash_from_reversed_line(&reversed_line)
    } else {
        None
    }
}

fn record_hash_from_reversed_line(reversed_line: &[u8]) -> Option<String> {
    let mut line = reversed_line.to_vec();
    line.reverse();
    while matches!(line.last(), Some(b'\r' | b' ' | b'\t')) {
        line.pop();
    }
    if line.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_slice(&line).ok()?;
    let hash = value.get("record_hash").and_then(Value::as_str)?;
    is_sha256_hex(hash).then(|| hash.to_owned())
}

fn discover_daemon_daily_files(state_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(state_dir) {
        Ok(it) => it,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let date = name
                .strip_prefix("daemon-events-")
                .and_then(|rest| rest.strip_suffix(".jsonl"))?;
            parse_ymd(date)?;
            Some(entry.path())
        })
        .collect();
    paths.sort();
    Ok(paths)
}

fn unique_health_probe_path(state_dir: &Path) -> PathBuf {
    let counter = HEALTHCHECK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    state_dir.join(format!(
        ".daemon-audit-healthcheck-{}-{counter}-{nanos}",
        std::process::id()
    ))
}

fn extract_hash_field<'a>(
    obj: &'a serde_json::Map<String, Value>,
    field: &str,
    problems: &mut Vec<DaemonAuditChainProblem>,
) -> Option<&'a str> {
    match obj.get(field) {
        Some(Value::String(hash)) if is_sha256_hex(hash) => Some(hash.as_str()),
        Some(Value::String(_)) => {
            problems.push(DaemonAuditChainProblem::MalformedHash {
                field: field.to_owned(),
            });
            None
        }
        Some(value) => {
            problems.push(DaemonAuditChainProblem::WrongFieldType {
                field: field.to_owned(),
                expected: "string".to_owned(),
                actual: json_value_kind(value).to_owned(),
            });
            None
        }
        None => {
            problems.push(DaemonAuditChainProblem::MissingField {
                field: field.to_owned(),
            });
            None
        }
    }
}

fn canonical_json_bytes(value: &Value) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    write_canonical_json(value, &mut out)?;
    Ok(out)
}

fn write_canonical_json(value: &Value, out: &mut Vec<u8>) -> io::Result<()> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(string) => {
            let encoded = serde_json::to_string(string)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            out.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(values) => {
            out.push(b'[');
            for (idx, item) in values.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                write_canonical_json(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            out.push(b'{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (idx, key) in keys.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                let encoded_key = serde_json::to_string(*key)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                out.extend_from_slice(encoded_key.as_bytes());
                out.push(b':');
                let value = map.get(*key).expect("key collected from map exists");
                write_canonical_json(value, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn hash_component(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn io_error_kind(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "not-found",
        io::ErrorKind::PermissionDenied => "permission-denied",
        io::ErrorKind::ConnectionRefused => "connection-refused",
        io::ErrorKind::ConnectionReset => "connection-reset",
        io::ErrorKind::ConnectionAborted => "connection-aborted",
        io::ErrorKind::NotConnected => "not-connected",
        io::ErrorKind::AddrInUse => "addr-in-use",
        io::ErrorKind::AddrNotAvailable => "addr-not-available",
        io::ErrorKind::BrokenPipe => "broken-pipe",
        io::ErrorKind::AlreadyExists => "already-exists",
        io::ErrorKind::WouldBlock => "would-block",
        io::ErrorKind::InvalidInput => "invalid-input",
        io::ErrorKind::InvalidData => "invalid-data",
        io::ErrorKind::TimedOut => "timed-out",
        io::ErrorKind::WriteZero => "write-zero",
        io::ErrorKind::Interrupted => "interrupted",
        io::ErrorKind::Unsupported => "unsupported",
        io::ErrorKind::UnexpectedEof => "unexpected-eof",
        io::ErrorKind::OutOfMemory => "out-of-memory",
        _ => "other",
    }
}

fn write_jsod2b_line_for_date(state_dir: &Path, today: &str, line: &str) -> io::Result<()> {
    let path = state_dir.join(format!("daemon-events-{today}.jsonl"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())
}

/// Best-effort retention: delete `daemon-events-YYYY-MM-DD.jsonl` files
/// whose date is older than `retention_days` before today (UTC). All
/// errors are swallowed — retention must never abort an audit write.
fn prune_old_audit_logs(state_dir: &Path, retention_days: i64) {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let cutoff = ymd_from_unix(now_secs - retention_days * 86_400);
    let Ok(entries) = fs::read_dir(state_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(date) = name
            .strip_prefix("daemon-events-")
            .and_then(|rest| rest.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if let Some(parsed) = parse_ymd(date)
            && parsed < cutoff
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Parse a `YYYY-MM-DD` stamp into a comparable `(year, month, day)`
/// tuple. Returns `None` on any malformed component.
fn parse_ymd(date: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date.split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

fn utc_date_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (y, m, d) = ymd_from_unix(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Civil-from-days algorithm (Howard Hinnant, public domain). Avoids
/// pulling in a chrono / time crate just for date stamping.
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

/// Write the api-ready state for a VM to
/// `{daemon_state_dir}/{vm}/api-ready.json`.
///
/// Called from `dispatch_broker_vm_start` after the DAG executor returns
/// a `DagRunReport` with a non-`None` `api_ready` field, so that
/// `d2b vm status <vm>` can report the live state instead of
/// hard-coding `None`.
///
/// File format: `{"apiReady": <value>}` where `<value>` mirrors the
/// [`supervisor::dag::ApiReadyState`] wire encoding:
/// - `"yes"` | `"pending"` | `"timeout"` for simple states
/// - `{"error": "<reason>"}` for error states
///
/// Best-effort: a write failure is logged via `tracing::warn!` but MUST
/// NOT abort the surrounding vm-start response.
pub fn write_vm_api_ready_state(
    daemon_state_dir: &Path,
    vm: &str,
    api_ready_value: serde_json::Value,
) -> io::Result<()> {
    let dir = daemon_state_dir.join(vm);
    fs::create_dir_all(&dir)?;
    let path = dir.join("api-ready.json");
    let content = serde_json::json!({ "apiReady": api_ready_value });
    let bytes =
        serde_json::to_vec(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // Atomic tmp+rename so the reader never sees a partial file.
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_chained_records() -> Vec<String> {
        let log = DaemonAuditLog::no_op();
        log.write_event(&DaemonEvent::ApiReadyTimeout {
            vm: "vm-a".to_owned(),
            runner: "ch-runner".to_owned(),
            elapsed_secs: 60,
            mode: "strict".to_owned(),
        })
        .expect("write first audit event");
        log.write_event(&DaemonEvent::VmStartRunnerExited {
            vm: "vm-a".to_owned(),
            role_id: "swtpm".to_owned(),
            reason_kind: VmStartRunnerExitReason::RunnerExited,
            exit_kind: Some(RunnerExitKind::Exited),
            exit_code: Some(1),
            exit_signal: None,
            elapsed_ms: 12,
        })
        .expect("write second audit event");
        log.captured.lock().expect("lock captured").clone()
    }

    #[test]
    fn api_ready_timeout_event_writes_jsod2b_and_captures() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = DaemonAuditLog::new(dir.path());

        // Trigger a fake api-ready timeout event.
        log.write_event(&DaemonEvent::ApiReadyTimeout {
            vm: "vm-a".to_owned(),
            runner: "ch-runner".to_owned(),
            elapsed_secs: 60,
            mode: "strict".to_owned(),
        })
        .expect("write api-ready-timeout event");

        // Assert the in-memory captured record has the expected fields.
        let records = log.captured.lock().expect("lock captured");
        assert_eq!(
            records.len(),
            1,
            "expected exactly one captured audit record"
        );
        let record: serde_json::Value =
            serde_json::from_str(&records[0]).expect("parse captured audit record as JSON");

        assert_eq!(
            record.get("source").and_then(|v| v.as_str()),
            Some(DAEMON_AUDIT_SOURCE),
            "source field must be 'd2bd'",
        );
        assert_eq!(
            record.get("prev_hash").and_then(|v| v.as_str()),
            Some(DAEMON_AUDIT_GENESIS_HASH),
            "first record must use the genesis previous hash",
        );
        let record_hash = record
            .get("record_hash")
            .and_then(|v| v.as_str())
            .expect("record_hash must be present");
        assert!(is_sha256_hex(record_hash), "malformed record_hash");
        let event = record.get("event").expect("event field must be present");
        assert_eq!(
            event.get("kind").and_then(|v| v.as_str()),
            Some("api_ready_timeout"),
            "event.kind must be 'api_ready_timeout'",
        );
        assert_eq!(
            event.get("vm").and_then(|v| v.as_str()),
            Some("vm-a"),
            "event.vm must match",
        );
        assert_eq!(
            event.get("runner").and_then(|v| v.as_str()),
            Some("ch-runner"),
            "event.runner must match",
        );
        assert_eq!(
            event.get("elapsed_secs").and_then(|v| v.as_u64()),
            Some(60),
            "event.elapsed_secs must match",
        );
        assert_eq!(
            event.get("mode").and_then(|v| v.as_str()),
            Some("strict"),
            "event.mode must be 'strict'",
        );

        // Also confirm the JSONL file was written to disk.
        let day_files: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read temp dir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("daemon-events-")
            })
            .collect();
        assert_eq!(
            day_files.len(),
            1,
            "expected exactly one daily daemon-events JSONL file"
        );
        // Read back and verify the line on disk matches the captured record.
        let path = day_files[0].path();
        let content = std::fs::read_to_string(&path).expect("read daemon-events file");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "expected one JSONL line in the file");
        let disk_record: serde_json::Value =
            serde_json::from_str(lines[0]).expect("parse disk JSONL line");
        assert_eq!(
            disk_record
                .get("event")
                .and_then(|e| e.get("kind"))
                .and_then(|v| v.as_str()),
            Some("api_ready_timeout"),
        );
        assert_eq!(
            disk_record.get("record_hash").and_then(|v| v.as_str()),
            Some(record_hash),
            "disk and captured records must carry the same record hash",
        );
    }

    #[test]
    fn daemon_audit_records_are_hash_chained_and_verifiable() {
        let records = two_chained_records();
        assert_eq!(records.len(), 2);
        let first: Value = serde_json::from_str(&records[0]).expect("first record parses");
        let second: Value = serde_json::from_str(&records[1]).expect("second record parses");
        let first_hash = first["record_hash"].as_str().expect("first record hash");
        let second_prev = second["prev_hash"].as_str().expect("second prev hash");
        assert!(is_sha256_hex(first_hash));
        assert!(is_sha256_hex(
            second["record_hash"].as_str().expect("second record hash")
        ));
        assert_eq!(first["prev_hash"].as_str(), Some(DAEMON_AUDIT_GENESIS_HASH));
        assert_eq!(second_prev, first_hash);

        let report = verify_daemon_audit_lines(
            records
                .iter()
                .map(|line| (Some("daemon-events-2026-06-20.jsonl"), line.as_str())),
        );
        assert_eq!(report.records_scanned, 2);
        assert_eq!(report.records_ok, 2);
        assert!(report.is_clean(), "chain report not clean: {:?}", report);
    }

    #[test]
    fn daemon_audit_verify_fails_on_altered_event() {
        let mut records = two_chained_records();
        let mut second: Value = serde_json::from_str(&records[1]).expect("parse second");
        second["event"]["elapsed_ms"] = Value::from(99_u64);
        records[1] = second.to_string();

        let report =
            verify_daemon_audit_lines(records.iter().map(|line| (None::<&str>, line.as_str())));
        assert!(report.defects.iter().any(|defect| matches!(
            defect.problem,
            DaemonAuditChainProblem::RecordHashMismatch { .. }
        )));
    }

    #[test]
    fn daemon_audit_verify_fails_on_missing_link() {
        let records = two_chained_records();
        let report = verify_daemon_audit_lines([(None, records[1].as_str())]);
        assert!(report.defects.iter().any(|defect| matches!(
            &defect.problem,
            DaemonAuditChainProblem::PreviousHashMismatch { expected, .. }
                if expected == DAEMON_AUDIT_GENESIS_HASH
        )));
    }

    #[test]
    fn daemon_audit_verify_fails_on_altered_previous_hash() {
        let mut records = two_chained_records();
        let mut second: Value = serde_json::from_str(&records[1]).expect("parse second");
        second["prev_hash"] = Value::String(DAEMON_AUDIT_GENESIS_HASH.to_owned());
        records[1] = second.to_string();

        let report =
            verify_daemon_audit_lines(records.iter().map(|line| (None::<&str>, line.as_str())));
        assert!(report.defects.iter().any(|defect| matches!(
            defect.problem,
            DaemonAuditChainProblem::PreviousHashMismatch { .. }
        )));
        assert!(report.defects.iter().any(|defect| matches!(
            defect.problem,
            DaemonAuditChainProblem::RecordHashMismatch { .. }
        )));
    }

    #[test]
    fn daemon_audit_verify_fails_on_malformed_hash_fields() {
        let mut records = two_chained_records();
        let mut first: Value = serde_json::from_str(&records[0]).expect("parse first");
        first["record_hash"] = Value::String("not-a-sha256".to_owned());
        records[0] = first.to_string();

        let report =
            verify_daemon_audit_lines(records.iter().map(|line| (None::<&str>, line.as_str())));
        assert!(report.defects.iter().any(|defect| matches!(
            &defect.problem,
            DaemonAuditChainProblem::MalformedHash { field } if field == "record_hash"
        )));
    }

    #[test]
    fn daemon_audit_verify_fails_on_missing_chain_field() {
        let mut records = two_chained_records();
        let mut first: Value = serde_json::from_str(&records[0]).expect("parse first");
        first.as_object_mut().unwrap().remove("prev_hash");
        records[0] = first.to_string();

        let report =
            verify_daemon_audit_lines(records.iter().map(|line| (None::<&str>, line.as_str())));
        assert!(report.defects.iter().any(|defect| matches!(
            &defect.problem,
            DaemonAuditChainProblem::MissingField { field } if field == "prev_hash"
        )));
    }

    #[test]
    fn exec_lifecycle_events_are_leak_safe() {
        // The exec establish + terminate audit events carry ONLY
        // leak-safe fields (vm, peer_uid, tty). A planted sentinel standing in
        // for a session handle / argv / env / cwd must never appear, and the
        // serialized event must expose no unexpected key.
        const SENTINEL: &str = "SECRET-handle-argv-env-cwd-/nix/store/path-like-token-9b2f";
        let log = DaemonAuditLog::no_op();

        log.write_event(&DaemonEvent::GuestControlExecEstablished {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            tty: true,
        })
        .expect("write established event");
        log.write_event(&DaemonEvent::GuestControlExecTerminated {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
        })
        .expect("write terminated event");

        let records = log.captured.lock().expect("lock captured");
        assert_eq!(records.len(), 2, "expected two captured lifecycle records");

        for line in records.iter() {
            assert!(
                !line.contains(SENTINEL),
                "exec lifecycle audit leaked a sentinel: {line}"
            );
            for forbidden in ["SECRET", "/nix/store", "argv", "env", "cwd"] {
                assert!(
                    !line.contains(forbidden),
                    "exec lifecycle audit leaked forbidden canary {forbidden:?}: {line}",
                );
            }
            let record: serde_json::Value =
                serde_json::from_str(line).expect("parse captured lifecycle record");
            assert_eq!(record.get("source").and_then(|v| v.as_str()), Some("d2bd"));
            let event = record.get("event").expect("event object");
            let obj = event.as_object().expect("event is an object");
            // Closed key set: kind + the leak-safe fields only. No `session`,
            // `handle`, `argv`, `env`, `cwd`, or stdio keys may appear.
            for key in obj.keys() {
                assert!(
                    matches!(key.as_str(), "kind" | "vm" | "peer_uid" | "tty"),
                    "exec lifecycle audit exposed an unexpected key {key:?}: {line}"
                );
            }
            assert_eq!(event.get("vm").and_then(|v| v.as_str()), Some("corp-vm"));
            assert_eq!(event.get("peer_uid").and_then(|v| v.as_u64()), Some(1000));
        }

        let established = serde_json::from_str::<serde_json::Value>(&records[0])
            .expect("parse established record");
        assert_eq!(
            established["event"]["kind"].as_str(),
            Some("guest_control_exec_established")
        );
        assert_eq!(established["event"]["tty"].as_bool(), Some(true));

        let terminated = serde_json::from_str::<serde_json::Value>(&records[1])
            .expect("parse terminated record");
        assert_eq!(
            terminated["event"]["kind"].as_str(),
            Some("guest_control_exec_terminated")
        );
        // The terminate event has no tty field (only vm + peer_uid).
        assert!(terminated["event"].get("tty").is_none());
    }

    #[test]
    fn shell_lifecycle_events_are_leak_safe() {
        const SENTINEL: &str = "SECRET-shell-name-session-terminal-/nix/store/path-like-token";
        let log = DaemonAuditLog::no_op();

        log.write_event(&DaemonEvent::GuestControlShellAttached {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            action: ShellAuditAction::Attach,
            result: ShellAuditResult::Attached,
            shell_ref_digest: "0123456789abcdef".to_owned(),
            force: true,
        })
        .expect("write shell attached event");
        log.write_event(&DaemonEvent::GuestControlShellDetached {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            action: ShellAuditAction::Detach,
            result: ShellAuditResult::Closed,
            shell_ref_digest: "0123456789abcdef".to_owned(),
        })
        .expect("write shell detached event");
        log.write_event(&DaemonEvent::ShellLifecycle {
            target: "tools.host.d2b".to_owned(),
            peer_uid: 1000,
            provider: ShellAuditProvider::UnsafeLocal,
            action: ShellAuditAction::Attach,
            result: ShellAuditResult::Attached,
            force: Some(true),
            operation_digest: Some("1111111111111111".to_owned()),
            session_digest: Some("2222222222222222".to_owned()),
        })
        .expect("write provider-neutral shell event");

        let records = log.captured.lock().expect("lock captured");
        assert_eq!(records.len(), 3, "expected three captured shell records");
        for line in records.iter() {
            assert!(
                !line.contains(SENTINEL),
                "shell lifecycle audit leaked sentinel: {line}"
            );
            for forbidden in [
                "SECRET-shell-name",
                "/nix/store/path-like-token",
                "supervisor_id",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "shell lifecycle audit leaked forbidden canary {forbidden:?}: {line}",
                );
            }
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&records[0]).unwrap()["event"]["result"]
                .as_str(),
            Some("attached")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&records[1]).unwrap()["event"]["result"]
                .as_str(),
            Some("closed")
        );
        let provider =
            serde_json::from_str::<serde_json::Value>(&records[2]).expect("provider shell event");
        assert_eq!(provider["event"]["force"].as_bool(), Some(true));
        let keys = provider["event"]
            .as_object()
            .expect("provider event object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "action",
                "force",
                "kind",
                "operation_digest",
                "peer_uid",
                "provider",
                "result",
                "session_digest",
                "target",
            ])
        );
    }

    #[test]
    fn detached_exec_audit_events_are_leak_safe() {
        const SENTINEL: &str = "SECRET-argv-env-cwd-/nix/store/log-bytes-2d7b";
        let log = DaemonAuditLog::no_op();

        log.write_event(&DaemonEvent::GuestControlExecDetachedCreate {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            action: DetachedExecAuditAction::Create,
            result: DetachedExecAuditResult::Created,
            exec_id: "exec-opaque-1".to_owned(),
        })
        .expect("write detached create event");
        log.write_event(&DaemonEvent::GuestControlExecDetachedKill {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            action: DetachedExecAuditAction::Cancel,
            result: DetachedExecAuditResult::Cancelling,
            exec_id: "exec-opaque-1".to_owned(),
        })
        .expect("write detached kill event");

        let records = log.captured.lock().expect("lock captured");
        assert_eq!(records.len(), 2, "expected two detached audit records");

        for line in records.iter() {
            assert!(
                !line.contains(SENTINEL),
                "detached exec audit leaked a sentinel: {line}"
            );
            for forbidden in ["SECRET", "/nix/store", "argv", "env", "cwd"] {
                assert!(
                    !line.contains(forbidden),
                    "detached exec audit leaked forbidden canary {forbidden:?}: {line}",
                );
            }
            for forbidden in [
                "\"argv\"",
                "\"env\"",
                "\"cwd\"",
                "\"stdout\"",
                "\"stderr\"",
                "\"log_bytes\"",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "detached exec audit exposed forbidden field {forbidden:?}: {line}"
                );
            }
            let record: serde_json::Value =
                serde_json::from_str(line).expect("parse detached audit record");
            let event = record.get("event").expect("event object");
            let obj = event.as_object().expect("event is an object");
            for key in obj.keys() {
                assert!(
                    matches!(
                        key.as_str(),
                        "kind" | "vm" | "peer_uid" | "action" | "result" | "exec_id"
                    ),
                    "detached exec audit exposed unexpected key {key:?}: {line}"
                );
            }
            assert_eq!(event.get("vm").and_then(|v| v.as_str()), Some("corp-vm"));
            assert_eq!(event.get("peer_uid").and_then(|v| v.as_u64()), Some(1000));
            assert_eq!(
                event.get("exec_id").and_then(|v| v.as_str()),
                Some("exec-opaque-1")
            );
        }

        let create = serde_json::from_str::<serde_json::Value>(&records[0])
            .expect("parse detached create record");
        assert_eq!(
            create["event"]["kind"].as_str(),
            Some("guest_control_exec_detached_create")
        );
        assert_eq!(create["event"]["action"].as_str(), Some("create"));
        assert_eq!(create["event"]["result"].as_str(), Some("created"));

        let kill = serde_json::from_str::<serde_json::Value>(&records[1])
            .expect("parse detached kill record");
        assert_eq!(
            kill["event"]["kind"].as_str(),
            Some("guest_control_exec_detached_kill")
        );
        assert_eq!(kill["event"]["action"].as_str(), Some("cancel"));
        assert_eq!(kill["event"]["result"].as_str(), Some("cancelling"));
    }

    #[test]
    fn daemon_audit_health_ok_when_writable_and_retention_floor_met() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let report = daemon_audit_sink_health_report(
            dir.path(),
            AUDIT_RETENTION_DAYS,
            AUDIT_RETENTION_FLOOR_DAYS,
        );
        assert_eq!(report.status, DaemonAuditSinkStatus::Ok);
        assert!(report.writable);
        assert!(report.problems.is_empty());
    }

    #[test]
    fn daemon_audit_health_degraded_when_retention_below_floor() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let report = daemon_audit_sink_health_report(dir.path(), 3, 7);
        assert_eq!(report.status, DaemonAuditSinkStatus::Degraded);
        assert!(report.writable);
        assert!(report.problems.iter().any(|problem| matches!(
            problem,
            DaemonAuditSinkProblem::RetentionBelowFloor {
                configured_days: 3,
                required_floor_days: 7,
            }
        )));
    }

    #[test]
    fn daemon_audit_health_reports_unavailable_without_leaking_state_path() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let secret_component = "SECRET-argv-env-cwd";
        let blocked_parent = dir.path().join(secret_component).join("nix").join("store");
        std::fs::create_dir_all(&blocked_parent).expect("create path-like parent");
        let blocked = blocked_parent.join("path-like-token");
        std::fs::write(&blocked, "not a directory").expect("write blocker file");

        let report = daemon_audit_sink_health_report(
            &blocked,
            AUDIT_RETENTION_DAYS,
            AUDIT_RETENTION_FLOOR_DAYS,
        );
        assert_eq!(report.status, DaemonAuditSinkStatus::Unavailable);
        assert!(!report.writable);
        assert!(
            report.problems.iter().any(|problem| matches!(
                problem,
                DaemonAuditSinkProblem::CreateStateDirFailed { .. }
            ))
        );
        let serialized = serde_json::to_string(&report).expect("serialize health report");
        for forbidden in [
            secret_component,
            "SECRET",
            "argv",
            "env",
            "cwd",
            "/nix/store",
            "path-like-token",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "daemon audit health leaked forbidden canary {forbidden:?}: {serialized}",
            );
        }
    }

    #[test]
    fn daemon_audit_health_probe_uses_unique_scratch_paths() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let first = unique_health_probe_path(dir.path());
        let second = unique_health_probe_path(dir.path());
        assert_ne!(first, second);

        let report = daemon_audit_sink_health_report(
            dir.path(),
            AUDIT_RETENTION_DAYS,
            AUDIT_RETENTION_FLOOR_DAYS,
        );
        assert_eq!(report.status, DaemonAuditSinkStatus::Ok);
        assert!(report.writable);
        assert_eq!(report.problems, Vec::<DaemonAuditSinkProblem>::new());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read temp dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".daemon-audit-healthcheck-")
            })
            .collect();
        assert!(leftovers.is_empty(), "health probe left scratch files");
    }

    #[test]
    fn daemon_audit_write_event_returns_error_for_unwritable_destination() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "blocks directory creation").expect("write blocker");
        let log = DaemonAuditLog::new(blocker.join("child"));
        let error = log
            .write_event(&DaemonEvent::ApiReadyTimeout {
                vm: "vm-a".to_owned(),
                runner: "ch-runner".to_owned(),
                elapsed_secs: 30,
                mode: "strict".to_owned(),
            })
            .expect_err("blocked destination must return an io error");
        assert!(matches!(
            error.kind(),
            io::ErrorKind::AlreadyExists
                | io::ErrorKind::NotADirectory
                | io::ErrorKind::PermissionDenied
        ));
        assert!(log.captured.lock().expect("captured").is_empty());
    }

    #[test]
    fn no_op_sink_health_reports_unavailable() {
        let log = DaemonAuditLog::no_op();
        let report = log.sink_health_report();
        assert_eq!(report.status, DaemonAuditSinkStatus::Unavailable);
        assert!(!report.writable);
        assert!(
            report
                .problems
                .iter()
                .any(|problem| matches!(problem, DaemonAuditSinkProblem::NoStateDir))
        );
    }

    #[test]
    fn last_record_hash_reads_tail_without_loading_entire_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("daemon-events-2026-06-20.jsonl");
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        {
            let mut file = std::fs::File::create(&path).expect("create audit file");
            for idx in 0..2048 {
                writeln!(file, "{{\"noise\":{idx}}}").expect("write noise line");
            }
            writeln!(file, "{{\"source\":\"d2bd\",\"record_hash\":\"{hash}\"}}")
                .expect("write hash line");
        }
        assert_eq!(
            last_daemon_record_hash_in_file(&path),
            Some(hash.to_owned())
        );
    }

    #[test]
    fn no_op_does_not_write_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        // Create a no-op log — but give it the temp dir to make sure the
        // file is NOT created.
        let log = DaemonAuditLog::no_op();
        // Manually set state_dir to the temp dir via a helper.
        // We can't do that here because state_dir is private; instead,
        // create a no_op and verify its captured vec is empty.
        log.write_event(&DaemonEvent::ApiReadyTimeout {
            vm: "vm-a".to_owned(),
            runner: "ch-runner".to_owned(),
            elapsed_secs: 30,
            mode: "strict".to_owned(),
        })
        .expect("no-op write should not error");

        // No file should appear in temp dir (no state_dir set).
        let count = std::fs::read_dir(dir.path())
            .expect("read temp dir")
            .count();
        assert_eq!(count, 0, "no-op log must not write any files");
    }

    #[test]
    fn write_vm_api_ready_state_roundtrip() {
        let dir = tempfile::tempdir().expect("create temp dir");
        write_vm_api_ready_state(
            dir.path(),
            "vm-a",
            serde_json::Value::String("timeout".to_owned()),
        )
        .expect("write api-ready state");

        let path = dir.path().join("vm-a").join("api-ready.json");
        let content = std::fs::read_to_string(&path).expect("read api-ready.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("parse api-ready.json");
        assert_eq!(
            parsed.get("apiReady").and_then(|v| v.as_str()),
            Some("timeout"),
        );
    }

    #[test]
    fn concurrent_writes_produce_valid_jsonl() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = std::sync::Arc::new(DaemonAuditLog::new(dir.path()));

        let mut handles = Vec::new();
        for thread_idx in 0..8 {
            let log = std::sync::Arc::clone(&log);
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    log.write_event(&DaemonEvent::VmStartRunnerExited {
                        vm: format!("vm-{thread_idx}"),
                        role_id: "swtpm".to_owned(),
                        reason_kind: VmStartRunnerExitReason::RunnerExited,
                        exit_kind: Some(RunnerExitKind::Exited),
                        exit_code: Some(1),
                        exit_signal: None,
                        elapsed_ms: 12,
                    })
                    .expect("concurrent write");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("join writer thread");
        }

        let today = utc_date_string();
        let path = dir.path().join(format!("daemon-events-{today}.jsonl"));
        let content = std::fs::read_to_string(&path).expect("read jsonl file");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 8 * 25, "every concurrent append must land");
        for line in &lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each line is valid, line-atomic JSON");
            assert_eq!(parsed.get("source").and_then(|v| v.as_str()), Some("d2bd"),);
        }
        let report = verify_daemon_audit_lines(lines.iter().map(|line| (None::<&str>, *line)));
        assert!(
            report.is_clean(),
            "concurrent writes must preserve hash-chain order: {:?}",
            report
        );
    }

    #[test]
    fn retention_prunes_stale_logs_on_open() {
        let dir = tempfile::tempdir().expect("create temp dir");

        // A file dated well beyond the retention window must be pruned.
        let stale = dir.path().join("daemon-events-2000-01-01.jsonl");
        std::fs::write(&stale, "{}\n").expect("write stale log");
        // A foreign file must be left untouched.
        let foreign = dir.path().join("unrelated.txt");
        std::fs::write(&foreign, "keep me").expect("write foreign file");
        // A current-day file must survive.
        let today = utc_date_string();
        let fresh = dir.path().join(format!("daemon-events-{today}.jsonl"));
        std::fs::write(&fresh, "{}\n").expect("write fresh log");

        // Construction prunes on open.
        let _log = DaemonAuditLog::new(dir.path());

        assert!(!stale.exists(), "stale audit log must be pruned on open");
        assert!(foreign.exists(), "foreign files must not be touched");
        assert!(fresh.exists(), "current-day log must survive retention");
    }

    #[test]
    fn parse_ymd_rejects_malformed_dates() {
        assert_eq!(parse_ymd("2024-03-09"), Some((2024, 3, 9)));
        assert_eq!(parse_ymd("2024-13-01"), None);
        assert_eq!(parse_ymd("2024-03"), None);
        assert_eq!(parse_ymd("2024-03-09-10"), None);
        assert_eq!(parse_ymd("not-a-date"), None);
    }

    #[test]
    fn workload_launch_event_contains_boundary_only() {
        let event = DaemonEvent::WorkloadLauncher {
            target: "browser.host.d2b".to_owned(),
            item_id: "browser".to_owned(),
            operation_id: "launch-1".to_owned(),
            exec_id: None,
            peer_uid: 1000,
            provider: WorkloadLaunchProvider::UnsafeLocal,
            result: WorkloadLaunchResult::Committed,
        };
        let rendered = serde_json::to_string(&event).expect("serialize launch event");
        assert!(rendered.contains("\"kind\":\"workload_launcher\""));
        assert!(rendered.contains("\"peer_uid\":1000"));
        assert!(rendered.contains("\"provider\":\"unsafe-local\""));
        for canary in [
            "private-argv-canary",
            "\"argv\"",
            "\"env\"",
            "\"cwd\"",
            "\"path\"",
            "\"pid\"",
            "\"unit\"",
        ] {
            assert!(!rendered.contains(canary));
        }
    }

    #[test]
    fn local_vm_workload_launch_provider_serializes_canonically() {
        let event = DaemonEvent::WorkloadLauncher {
            target: "browser.work.d2b".to_owned(),
            item_id: "browser".to_owned(),
            operation_id: "launch-2".to_owned(),
            exec_id: Some("0123456789abcdef0123456789abcdef".to_owned()),
            peer_uid: 1000,
            provider: WorkloadLaunchProvider::LocalVm,
            result: WorkloadLaunchResult::Committed,
        };
        let rendered = serde_json::to_string(&event).unwrap();
        assert!(rendered.contains("\"provider\":\"local-vm\""));
        assert!(rendered.contains("\"operation_id\":\"launch-2\""));
        assert!(rendered.contains("\"exec_id\":\"0123456789abcdef0123456789abcdef\""));
        assert!(!rendered.contains("argv"));
        assert!(!rendered.contains("environment"));
        assert!(!rendered.contains("cwd"));
    }
}
