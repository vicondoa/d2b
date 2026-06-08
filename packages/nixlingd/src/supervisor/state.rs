//! W4-H6: daemon state persistence + restart reconciliation.
//!
//! On every supervisor transition the daemon writes a
//! [`RunnerSnapshotRecord`] to
//! `/var/lib/nixling/daemon-state/<vm>/runtime.json` so a crash and
//! restart can re-adopt the live pidfds rather than re-spawning. On
//! startup the supervisor:
//!
//! 1. enumerates every per-VM snapshot;
//! 2. parses `/proc/<pid>/stat` field 22 (`starttime` ticks);
//! 3. compares the observed start-time against the snapshot's
//!    `start_time_ticks`;
//! 4. classifies each record as [`ReconciliationOutcome::Adopt`],
//!    [`ReconciliationOutcome::Quarantine`], or
//!    [`ReconciliationOutcome::Missing`].
//!
//! Adopt — record can be re-opened with `pidfd_open(pid)` and the
//! supervisor resumes ownership. **In W4 main this classifier only
//! returns the `Adopt` outcome; the actual `pidfd_open` call is wired
//! in W4-fu together with the broker-side `SpawnRunner` execution.**
//!
//! Quarantine — `(pid, start_time)` drifted; the slot is parked and
//! the W3 typed-error envelope surfaces `quarantine-pid-drift` so the
//! operator can decide whether to kill (`pidfd_send_signal` after a
//! one-shot ADR carve-out) or wait out the stale process.
//!
//! Missing — `/proc/<pid>/` is gone; the snapshot is removed.
//!
//! This module is the **pure parser + classification surface**; the
//! filesystem snapshot store is behind a trait so tests can drive the
//! reconciler without touching `/var/lib/`. The production daemon
//! will wire the W4-H5 `SpawnRunner` response into the
//! FilesystemSnapshotStore and re-open pidfds via
//! `nix::sys::pidfd::pidfd_open` in **W4-fu** (the broker-side
//! `SpawnRunner` execution wave). W4 main ships the classification
//! surface only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nixling_ipc::broker_wire::RunnerRole;
use serde::{Deserialize, Serialize};

/// One persisted runner slot. Mirrors what the daemon learnt from a
/// successful W4-H5 SpawnRunner response, kept stable on disk so
/// post-restart reconciliation has authoritative
/// `(pid, start_time_ticks, role)` plus the W3-s1 pidfd table key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerSnapshotRecord {
    /// VM name (matches W3 s1 `PidfdKey::vm_id`).
    pub vm: String,
    /// Per-VM role identifier (matches W3 s1 `PidfdKey::role_id`).
    pub role_id: String,
    /// Which runner kind: CH / virtiofsd / swtpm.
    pub role: RunnerRole,
    /// Live process id at snapshot time.
    pub pid: i32,
    /// `/proc/<pid>/stat` field-22 starttime ticks captured by the
    /// broker at clone() time.
    pub start_time_ticks: u64,
    /// Wall-clock at snapshot write. RFC 3339 string for human
    /// readability; the reconciliation logic itself does not parse it.
    pub snapshotted_at: String,
}

/// Per-record outcome from [`reconcile`]. The supervisor uses the
/// variant to choose between re-opening the pidfd
/// ([`ReconciliationOutcome::Adopt`]), parking the snapshot
/// ([`ReconciliationOutcome::Quarantine`]), or removing it
/// ([`ReconciliationOutcome::Missing`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum ReconciliationOutcome {
    /// `(pid, start_time_ticks)` matches the live `/proc/<pid>/stat`.
    /// Supervisor re-opens the pidfd and re-registers the slot.
    Adopt,
    /// PID still exists, but start-time drifted. The slot is parked
    /// with a `quarantine-pid-drift` audit event; the supervisor does
    /// NOT control the process further.
    Quarantine { observed_start_time_ticks: u64 },
    /// `/proc/<pid>/` is gone. The snapshot file is deleted; the
    /// runner is treated as not-running on next supervisor pass.
    Missing,
    /// `/proc/<pid>/stat` exists but was unparseable. Treated as
    /// quarantine because we cannot prove safety of re-adoption.
    UnparseableProcStat { detail: String },
}

/// Per-record entry in the [`ReconciliationReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationEntry {
    pub vm: String,
    pub role_id: String,
    pub outcome: ReconciliationOutcome,
}

/// Aggregate of every snapshot the reconciler considered. Always
/// ordered for stable test snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationReport {
    pub entries: Vec<ReconciliationEntry>,
}

/// Abstraction over /proc reads so tests do not need /proc on disk.
/// Production daemon: [`ProcReader::system`].
pub trait ProcReader: Send + Sync {
    /// Returns `Some(start_time_ticks)` if `/proc/<pid>/stat` could
    /// be read AND field 22 parsed; `Ok(None)` if `/proc/<pid>/` is
    /// gone; `Err(detail)` if the stat file existed but field 22
    /// could not be parsed.
    fn proc_starttime(&self, pid: i32) -> Result<Option<u64>, String>;
}

/// Production [`ProcReader`] backed by `/proc`.
pub struct SystemProcReader;

impl ProcReader for SystemProcReader {
    fn proc_starttime(&self, pid: i32) -> Result<Option<u64>, String> {
        let path = format!("/proc/{pid}/stat");
        match std::fs::read_to_string(&path) {
            Ok(content) => parse_proc_stat_starttime(&content)
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// Parse the `starttime` field (column 22) from `/proc/<pid>/stat`.
///
/// `/proc/<pid>/stat` looks like:
///
///   `<pid> (<comm>) <state> <ppid> <pgrp> <session> <tty_nr> ...`
///
/// `comm` is enclosed in parentheses and CAN CONTAIN SPACES AND
/// PARENS, so we must split on the LAST `)` to find the start of the
/// stable space-delimited tail.
///
/// Column numbering per proc(5) is 1-based starting at `pid`. Field
/// 22 (`starttime`) is therefore index 19 (0-based) in the stable
/// tail (positions 3..N where 3 = `state`).
pub fn parse_proc_stat_starttime(content: &str) -> Result<u64, ProcStatError> {
    let trimmed = content.trim_end_matches('\n');
    // Find the last ')' so a comm like "(swtpm (in jail))" parses
    // correctly. The kernel writes `comm` with its own parens, so the
    // last ')' is unambiguous.
    let close = trimmed.rfind(')').ok_or(ProcStatError::CommNotFound)?;
    let tail = trimmed[close + 1..].trim_start();
    let fields: Vec<&str> = tail.split_whitespace().collect();
    // Position 3 (state) → index 0; ...; position 22 (starttime) →
    // index 19. Need at least 20 fields after `(comm)`.
    if fields.len() < 20 {
        return Err(ProcStatError::NotEnoughFields { seen: fields.len() });
    }
    fields[19]
        .parse::<u64>()
        .map_err(|_| ProcStatError::StarttimeNotInteger {
            raw: fields[19].to_owned(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcStatError {
    /// `/proc/<pid>/stat` had no closing `)` for the `comm` field.
    CommNotFound,
    /// `/proc/<pid>/stat` had fewer fields after `(comm)` than the
    /// proc(5) minimum (20 required to reach field 22).
    NotEnoughFields { seen: usize },
    /// Field 22 was present but not a u64.
    StarttimeNotInteger { raw: String },
}

impl std::fmt::Display for ProcStatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommNotFound => f.write_str("/proc/<pid>/stat missing closing ')'"),
            Self::NotEnoughFields { seen } => write!(
                f,
                "/proc/<pid>/stat has only {seen} stable-tail fields (need >= 20)"
            ),
            Self::StarttimeNotInteger { raw } => write!(
                f,
                "/proc/<pid>/stat starttime field is not an integer: {raw:?}"
            ),
        }
    }
}

impl std::error::Error for ProcStatError {}

/// Classify each snapshot against `/proc`. Deterministic, ordered
/// output (lex on `(vm, role_id)`) so test snapshots stay stable.
pub fn reconcile(
    snapshots: &[RunnerSnapshotRecord],
    proc_reader: &dyn ProcReader,
) -> ReconciliationReport {
    let mut sorted: Vec<&RunnerSnapshotRecord> = snapshots.iter().collect();
    sorted.sort_by(|a, b| {
        (a.vm.as_str(), a.role_id.as_str()).cmp(&(b.vm.as_str(), b.role_id.as_str()))
    });

    let mut entries = Vec::with_capacity(sorted.len());
    for snap in sorted {
        let outcome = match proc_reader.proc_starttime(snap.pid) {
            Ok(Some(observed)) if observed == snap.start_time_ticks => ReconciliationOutcome::Adopt,
            Ok(Some(observed)) => ReconciliationOutcome::Quarantine {
                observed_start_time_ticks: observed,
            },
            Ok(None) => ReconciliationOutcome::Missing,
            Err(detail) => ReconciliationOutcome::UnparseableProcStat { detail },
        };
        entries.push(ReconciliationEntry {
            vm: snap.vm.clone(),
            role_id: snap.role_id.clone(),
            outcome,
        });
    }
    ReconciliationReport { entries }
}

// ---- W4-fu: live reconcile-and-adopt ----

/// W4-fu: wraps the pure [`reconcile`] classifier with the actual
/// `pidfd_open(2)` adoption call. For each snapshot the classifier
/// returns `Adopt` for, this function opens a fresh pidfd (which
/// the kernel guarantees refers to that exact process — pidfds do
/// not survive pid reuse, even though the (pid, start_time_ticks)
/// tuple we cross-checked against `/proc/<pid>/stat` would not have
/// allowed re-adoption past the start-time-drift guard either).
///
/// The opened pidfd is registered in the supervisor's
/// [`crate::supervisor::pidfd::PidfdTable`] under the snapshot's
/// `(vm, role_id)` key. Snapshots classified as
/// `Quarantine` / `Missing` / `UnparseableProcStat` are NOT
/// re-adopted; the caller decides whether to delete those
/// snapshots or hand them to operator forensics.
///
/// Pure-ish: takes an opener trait so tests can run without /proc
/// or the SYS_pidfd_open syscall. Production daemon uses
/// [`SystemPidfdOpener`] which calls
/// `nixling_priv_broker::sys::pidfd_sys::pidfd_open` via a thin
/// shim (the broker crate has the only `unsafe` quarantine).
/// Abstraction over "open pidfd for this pid AND verify the resulting
/// pidfd still refers to the process we expect". Implementations:
///
/// - production (W*-fu): opens the pidfd via `pidfd_open(2)` THEN
///   re-reads `/proc/<pid>/stat` field 22 and compares against the
///   `expected_start_time_ticks` argument. If the start-time
///   drifted between the original `/proc` read in [`reconcile`]
///   and the post-open re-check, the pid was reused in that
///   window and the opened pidfd refers to an unrelated process —
///   the implementation closes it and surfaces
///   `AdoptOutcome::AdoptRaced`.
///
///   The production opener lives outside this module because
///   `nixlingd` is `#![forbid(unsafe_code)]` and `pidfd_open(2)`
///   syscall access lives in `nixling-priv-broker::sys::pidfd_sys`.
///   The W*-fu wiring sends a `BrokerRequest::OpenPidfd { pid,
///   expected_start_time }` shim over the broker socket, and the
///   broker returns the verified-fd over SCM_RIGHTS. That shim is
///   tracked as a follow-up commit — the trait + verification
///   contract land here so the daemon-side caller is correct
///   the moment the shim ships.
///
/// - tests: deterministic fake that returns canned outcomes.
///
/// W*-fu GPT-5.5 panel finding #1 (CRITICAL): the W3 s1 pidfd
/// contract requires `(pid, start_time)` verification AFTER
/// `pidfd_open`, not just before. Without it, a pid reused between
/// the /proc check and the pidfd_open call would yield a pidfd to
/// an unrelated process. This trait's contract bakes the post-open
/// verification into the signature so callers cannot accidentally
/// skip it.
pub trait PidfdOpener: Send + Sync {
    /// Open the pidfd for the `(vm, role_id, pid)` snapshot tuple and
    /// verify that `/proc/<pid>/stat` field 22 still matches
    /// `expected_start_time_ticks`. Returns `Ok(fd)` only when both
    /// succeed; otherwise the pidfd is closed and an error describing
    /// the race is returned.
    fn open_pidfd(
        &self,
        vm: &str,
        role_id: &str,
        pid: i32,
        expected_start_time_ticks: u64,
    ) -> Result<std::os::fd::OwnedFd, String>;
}

/// Per-record adoption outcome from [`reconcile_and_adopt`].
#[derive(Debug)]
pub enum AdoptOutcome {
    /// Snapshot classified as `Adopt` AND `pidfd_open` succeeded.
    /// The fd is held by the caller (typically registered in the
    /// `PidfdTable`).
    Adopted(std::os::fd::OwnedFd),
    /// Snapshot classified as `Adopt` but `pidfd_open` failed
    /// (race between the /proc check and pidfd_open — the process
    /// exited in that window). Treated as `Missing` for the
    /// purposes of removing the stale snapshot.
    AdoptRaced { detail: String },
    /// Classifier returned `Quarantine`. Forwarded verbatim.
    Quarantine { observed_start_time_ticks: u64 },
    /// Classifier returned `Missing`.
    Missing,
    /// Classifier returned `UnparseableProcStat`.
    UnparseableProcStat { detail: String },
}

/// Per-record adoption result, paired with the snapshot key.
#[derive(Debug)]
pub struct AdoptEntry {
    pub vm: String,
    pub role_id: String,
    pub outcome: AdoptOutcome,
}

/// Walk the classifier report and call `pidfd_open` on every
/// `Adopt` outcome. Returns one [`AdoptEntry`] per input snapshot
/// in the same lex order as [`reconcile`].
pub fn reconcile_and_adopt(
    snapshots: &[RunnerSnapshotRecord],
    proc_reader: &dyn ProcReader,
    opener: &dyn PidfdOpener,
) -> Vec<AdoptEntry> {
    let report = reconcile(snapshots, proc_reader);
    let mut by_key: std::collections::HashMap<(String, String), i32> =
        std::collections::HashMap::new();
    for snap in snapshots {
        by_key.insert((snap.vm.clone(), snap.role_id.clone()), snap.pid);
    }
    let mut out = Vec::with_capacity(report.entries.len());
    for entry in report.entries {
        let outcome = match entry.outcome {
            ReconciliationOutcome::Adopt => {
                let pid = by_key
                    .get(&(entry.vm.clone(), entry.role_id.clone()))
                    .copied()
                    .expect("reconcile preserves snapshot keys");
                // GPT-5.5 panel critical fix: pass the expected
                // start-time so the opener can re-verify AFTER
                // pidfd_open, closing the pid-reuse race window.
                let expected_start_time = snapshots
                    .iter()
                    .find(|s| s.vm == entry.vm && s.role_id == entry.role_id)
                    .map(|s| s.start_time_ticks)
                    .expect("snapshots contains the entry we just classified");
                match opener.open_pidfd(&entry.vm, &entry.role_id, pid, expected_start_time) {
                    Ok(fd) => AdoptOutcome::Adopted(fd),
                    Err(detail) => AdoptOutcome::AdoptRaced { detail },
                }
            }
            ReconciliationOutcome::Quarantine {
                observed_start_time_ticks,
            } => AdoptOutcome::Quarantine {
                observed_start_time_ticks,
            },
            ReconciliationOutcome::Missing => AdoptOutcome::Missing,
            ReconciliationOutcome::UnparseableProcStat { detail } => {
                AdoptOutcome::UnparseableProcStat { detail }
            }
        };
        out.push(AdoptEntry {
            vm: entry.vm,
            role_id: entry.role_id,
            outcome,
        });
    }
    out
}

// ---- snapshot store abstraction ----

/// Errors the [`SnapshotStore`] may surface. Kept abstract over the
/// underlying medium (filesystem in production, in-memory in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotStoreError {
    Io { path: String, detail: String },
    BadJson { path: String, detail: String },
}

impl std::fmt::Display for SnapshotStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, detail } => write!(f, "snapshot I/O failed at {path}: {detail}"),
            Self::BadJson { path, detail } => {
                write!(f, "snapshot JSON at {path} invalid: {detail}")
            }
        }
    }
}

impl std::error::Error for SnapshotStoreError {}

pub trait SnapshotStore: Send + Sync {
    /// Persist or overwrite the snapshot for `(vm, role_id)`.
    fn upsert(&self, record: &RunnerSnapshotRecord) -> Result<(), SnapshotStoreError>;
    /// Remove the snapshot for `(vm, role_id)`. Idempotent: removing
    /// a non-existent record is not an error.
    fn remove(&self, vm: &str, role_id: &str) -> Result<(), SnapshotStoreError>;
    /// Enumerate every persisted record.
    fn list(&self) -> Result<Vec<RunnerSnapshotRecord>, SnapshotStoreError>;
}

/// Filesystem-backed store at `<root>/<vm>/runtime.<role_id>.json`.
/// One file per (vm, role_id) so the daemon can update slots without
/// touching unrelated VMs.
pub struct FilesystemSnapshotStore {
    root: PathBuf,
}

impl FilesystemSnapshotStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn file_path(&self, vm: &str, role_id: &str) -> PathBuf {
        self.root.join(vm).join(format!("runtime.{role_id}.json"))
    }
}

impl SnapshotStore for FilesystemSnapshotStore {
    fn upsert(&self, record: &RunnerSnapshotRecord) -> Result<(), SnapshotStoreError> {
        let path = self.file_path(&record.vm, &record.role_id);
        let parent = path.parent().expect("file_path has a parent");
        std::fs::create_dir_all(parent).map_err(|e| SnapshotStoreError::Io {
            path: parent.display().to_string(),
            detail: e.to_string(),
        })?;
        let bytes = serde_json::to_vec_pretty(record).map_err(|e| SnapshotStoreError::BadJson {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        // W4 GPT-5.5 panel notable: full crash-durable write —
        // write_all + sync_all on the tmp file BEFORE rename, then
        // fsync the parent dir AFTER rename so the directory entry
        // itself reaches disk. This protects against host power
        // loss in addition to process crash (the W4-H6 docs claimed
        // "tmp+rename leaves the previous snapshot intact" which
        // is true for process crash but not power loss).
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(|e| SnapshotStoreError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
            f.write_all(&bytes).map_err(|e| SnapshotStoreError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
            f.sync_all().map_err(|e| SnapshotStoreError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        }
        std::fs::rename(&tmp, &path).map_err(|e| SnapshotStoreError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        // Best-effort parent-dir fsync; some filesystems (e.g.
        // tmpfs) treat this as a no-op which is fine for the test
        // suite. Production deployments land snapshots on ext4 /
        // xfs / btrfs where this guarantees the rename's directory
        // entry is durable.
        if let Ok(parent_dir) = std::fs::File::open(parent) {
            let _ = parent_dir.sync_all();
        }
        Ok(())
    }

    fn remove(&self, vm: &str, role_id: &str) -> Result<(), SnapshotStoreError> {
        let path = self.file_path(vm, role_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(SnapshotStoreError::Io {
                path: path.display().to_string(),
                detail: err.to_string(),
            }),
        }
    }

    fn list(&self) -> Result<Vec<RunnerSnapshotRecord>, SnapshotStoreError> {
        let mut records = Vec::new();
        if !self.root.exists() {
            return Ok(records);
        }
        for vm_entry in std::fs::read_dir(&self.root).map_err(|e| SnapshotStoreError::Io {
            path: self.root.display().to_string(),
            detail: e.to_string(),
        })? {
            let vm_entry = vm_entry.map_err(|e| SnapshotStoreError::Io {
                path: self.root.display().to_string(),
                detail: e.to_string(),
            })?;
            let vm_path = vm_entry.path();
            if !vm_path.is_dir() {
                continue;
            }
            for role_entry in std::fs::read_dir(&vm_path).map_err(|e| SnapshotStoreError::Io {
                path: vm_path.display().to_string(),
                detail: e.to_string(),
            })? {
                let role_entry = role_entry.map_err(|e| SnapshotStoreError::Io {
                    path: vm_path.display().to_string(),
                    detail: e.to_string(),
                })?;
                let role_path = role_entry.path();
                let file_name = match role_path.file_name().and_then(|s| s.to_str()) {
                    Some(s) => s.to_owned(),
                    None => continue,
                };
                if !file_name.starts_with("runtime.") || !file_name.ends_with(".json") {
                    continue;
                }
                let bytes = std::fs::read(&role_path).map_err(|e| SnapshotStoreError::Io {
                    path: role_path.display().to_string(),
                    detail: e.to_string(),
                })?;
                let record: RunnerSnapshotRecord =
                    serde_json::from_slice(&bytes).map_err(|e| SnapshotStoreError::BadJson {
                        path: role_path.display().to_string(),
                        detail: e.to_string(),
                    })?;
                records.push(record);
            }
        }
        Ok(records)
    }
}

/// In-memory [`SnapshotStore`] used by tests. Thread-safe (the
/// supervisor accesses snapshots from the daemon's tokio worker
/// threads).
#[derive(Default)]
pub struct InMemorySnapshotStore {
    inner: std::sync::Mutex<BTreeMap<(String, String), RunnerSnapshotRecord>>,
}

impl InMemorySnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SnapshotStore for InMemorySnapshotStore {
    fn upsert(&self, record: &RunnerSnapshotRecord) -> Result<(), SnapshotStoreError> {
        self.inner
            .lock()
            .unwrap()
            .insert((record.vm.clone(), record.role_id.clone()), record.clone());
        Ok(())
    }

    fn remove(&self, vm: &str, role_id: &str) -> Result<(), SnapshotStoreError> {
        self.inner
            .lock()
            .unwrap()
            .remove(&(vm.to_owned(), role_id.to_owned()));
        Ok(())
    }

    fn list(&self) -> Result<Vec<RunnerSnapshotRecord>, SnapshotStoreError> {
        Ok(self.inner.lock().unwrap().values().cloned().collect())
    }
}

/// Helper: read the path that the [`FilesystemSnapshotStore`] would
/// use for `(vm, role_id)`. Exposed for tests + the supervisor's
/// diagnostic logging.
pub fn snapshot_path(root: &Path, vm: &str, role_id: &str) -> PathBuf {
    root.join(vm).join(format!("runtime.{role_id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn sample(vm: &str, role: &str, pid: i32, st: u64) -> RunnerSnapshotRecord {
        RunnerSnapshotRecord {
            vm: vm.to_owned(),
            role_id: role.to_owned(),
            role: RunnerRole::CloudHypervisor,
            pid,
            start_time_ticks: st,
            snapshotted_at: "2026-05-29T03:00:00Z".to_owned(),
        }
    }

    // ---- proc_stat parser tests ----

    #[test]
    fn parses_simple_proc_stat() {
        // pid=1234 comm=(cloud-hyperv) state=R ppid=1 pgrp=1234 session=1234
        // tty_nr=0 tpgid=-1 flags=0 minflt=0 cminflt=0 majflt=0 cmajflt=0
        // utime=10 stime=20 cutime=0 cstime=0 priority=20 nice=0
        // num_threads=1 itrealvalue=0 starttime=987654321
        let stat =
            "1234 (cloud-hyperv) R 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 987654321 0";
        assert_eq!(parse_proc_stat_starttime(stat).unwrap(), 987_654_321);
    }

    #[test]
    fn parses_comm_with_spaces_and_parens() {
        // Linux allows comm to contain spaces and parens, since it
        // wraps the whole thing in parens. The parser must use the
        // LAST ')' as the comm boundary, not the first.
        let stat = "1234 (sw tpm (in jail)) R 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 555 0";
        assert_eq!(parse_proc_stat_starttime(stat).unwrap(), 555);
    }

    #[test]
    fn parser_rejects_missing_comm_close() {
        let stat = "1234 cloud-hyperv R 1 1234 1234 0";
        assert!(matches!(
            parse_proc_stat_starttime(stat),
            Err(ProcStatError::CommNotFound)
        ));
    }

    #[test]
    fn parser_rejects_short_tail() {
        let stat = "1234 (sh) R 1 1234";
        // Tail after ')' is " R 1 1234" → split_whitespace yields
        // ["R", "1", "1234"] = 3 fields. proc(5) requires 20 to
        // reach starttime (field 22 → index 19).
        assert!(matches!(
            parse_proc_stat_starttime(stat),
            Err(ProcStatError::NotEnoughFields { seen: 3 })
        ));
    }

    #[test]
    fn parser_rejects_non_integer_starttime() {
        let stat = "1234 (sh) R 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 NOT_A_NUMBER 0";
        assert!(matches!(
            parse_proc_stat_starttime(stat),
            Err(ProcStatError::StarttimeNotInteger { .. })
        ));
    }

    #[test]
    fn parser_strips_trailing_newline() {
        let stat = "1234 (sh) R 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 999 0\n";
        assert_eq!(parse_proc_stat_starttime(stat).unwrap(), 999);
    }

    // ---- reconciler tests ----

    /// In-memory ProcReader for unit tests.
    #[derive(Default)]
    struct FakeProcReader {
        map: HashMap<i32, Result<Option<u64>, String>>,
    }

    impl FakeProcReader {
        fn add_alive(&mut self, pid: i32, start_time: u64) {
            self.map.insert(pid, Ok(Some(start_time)));
        }
        fn add_missing(&mut self, pid: i32) {
            self.map.insert(pid, Ok(None));
        }
        fn add_unparseable(&mut self, pid: i32, detail: &str) {
            self.map.insert(pid, Err(detail.to_owned()));
        }
    }

    impl ProcReader for FakeProcReader {
        fn proc_starttime(&self, pid: i32) -> Result<Option<u64>, String> {
            self.map.get(&pid).cloned().unwrap_or(Ok(None))
        }
    }

    #[test]
    fn adopts_matching_record() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_alive(4242, 987_654_321);
        let report = reconcile(&snaps, &reader);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].outcome, ReconciliationOutcome::Adopt);
    }

    #[test]
    fn quarantines_drifted_pid() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        // PID still exists, but start_time is different — the pid
        // got reused by a brand-new unrelated process.
        reader.add_alive(4242, 111);
        let report = reconcile(&snaps, &reader);
        assert_eq!(
            report.entries[0].outcome,
            ReconciliationOutcome::Quarantine {
                observed_start_time_ticks: 111
            }
        );
    }

    #[test]
    fn marks_missing_when_proc_pid_gone() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_missing(4242);
        let report = reconcile(&snaps, &reader);
        assert_eq!(report.entries[0].outcome, ReconciliationOutcome::Missing);
    }

    #[test]
    fn marks_unparseable_when_reader_errors() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_unparseable(4242, "broken /proc/<pid>/stat");
        let report = reconcile(&snaps, &reader);
        match &report.entries[0].outcome {
            ReconciliationOutcome::UnparseableProcStat { detail } => {
                assert!(detail.contains("broken"));
            }
            other => panic!("expected UnparseableProcStat, got {other:?}"),
        }
    }

    #[test]
    fn report_ordered_by_vm_then_role() {
        let snaps = vec![
            sample("zeta-vm", "ch", 1, 100),
            sample("corp-vm", "swtpm", 2, 100),
            sample("corp-vm", "ch", 3, 100),
        ];
        let mut reader = FakeProcReader::default();
        for pid in 1..=3 {
            reader.add_alive(pid, 100);
        }
        let report = reconcile(&snaps, &reader);
        let order: Vec<(String, String)> = report
            .entries
            .iter()
            .map(|e| (e.vm.clone(), e.role_id.clone()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("corp-vm".to_owned(), "ch".to_owned()),
                ("corp-vm".to_owned(), "swtpm".to_owned()),
                ("zeta-vm".to_owned(), "ch".to_owned()),
            ]
        );
    }

    #[test]
    fn empty_snapshots_produce_empty_report() {
        let snaps: Vec<RunnerSnapshotRecord> = vec![];
        let reader = FakeProcReader::default();
        let report = reconcile(&snaps, &reader);
        assert!(report.entries.is_empty());
    }

    // ---- SnapshotStore tests ----

    #[test]
    fn in_memory_store_round_trip() {
        let store = InMemorySnapshotStore::new();
        let r = sample("corp-vm", "ch", 100, 200);
        store.upsert(&r).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed, vec![r.clone()]);
        store.remove("corp-vm", "ch").unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn in_memory_remove_missing_is_ok() {
        let store = InMemorySnapshotStore::new();
        // Removing a never-inserted slot must not error.
        store.remove("ghost", "ch").unwrap();
    }

    #[test]
    fn filesystem_store_round_trip() {
        let dir = tempdir().unwrap();
        let store = FilesystemSnapshotStore::new(dir.path());
        let r1 = sample("corp-vm", "ch", 100, 200);
        let r2 = sample("corp-vm", "swtpm", 101, 201);
        let r3 = sample("zeta-vm", "virtiofsd-ro-store", 102, 202);
        store.upsert(&r1).unwrap();
        store.upsert(&r2).unwrap();
        store.upsert(&r3).unwrap();

        let mut listed = store.list().unwrap();
        listed.sort_by(|a, b| {
            (a.vm.as_str(), a.role_id.as_str()).cmp(&(b.vm.as_str(), b.role_id.as_str()))
        });
        let mut expected = vec![r1.clone(), r2.clone(), r3.clone()];
        expected.sort_by(|a, b| {
            (a.vm.as_str(), a.role_id.as_str()).cmp(&(b.vm.as_str(), b.role_id.as_str()))
        });
        assert_eq!(listed, expected);

        // Files land at the documented path.
        let p = snapshot_path(dir.path(), "corp-vm", "ch");
        assert!(p.exists());
        assert!(p.to_string_lossy().ends_with("runtime.ch.json"));
    }

    #[test]
    fn filesystem_store_upsert_replaces_existing() {
        let dir = tempdir().unwrap();
        let store = FilesystemSnapshotStore::new(dir.path());
        let r1 = sample("corp-vm", "ch", 100, 200);
        let r2 = sample("corp-vm", "ch", 100, 999);
        store.upsert(&r1).unwrap();
        store.upsert(&r2).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].start_time_ticks, 999);
    }

    #[test]
    fn filesystem_store_remove_missing_is_ok() {
        let dir = tempdir().unwrap();
        let store = FilesystemSnapshotStore::new(dir.path());
        store.remove("ghost", "ch").unwrap();
    }

    #[test]
    fn filesystem_store_list_skips_non_runtime_files() {
        let dir = tempdir().unwrap();
        let vm_dir = dir.path().join("corp-vm");
        std::fs::create_dir_all(&vm_dir).unwrap();
        std::fs::write(vm_dir.join("README.md"), b"hello").unwrap();
        std::fs::write(vm_dir.join("runtime.ch.json.tmp"), b"junk").unwrap();
        let store = FilesystemSnapshotStore::new(dir.path());
        let listed = store.list().unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn filesystem_store_list_handles_missing_root() {
        let dir = tempdir().unwrap();
        let missing_root = dir.path().join("nonexistent");
        let store = FilesystemSnapshotStore::new(missing_root);
        let listed = store.list().unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn snapshot_record_round_trip_json() {
        let r = sample("corp-vm", "ch", 100, 200);
        let json = serde_json::to_string(&r).unwrap();
        let parsed: RunnerSnapshotRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn snapshot_record_rejects_unknown_fields() {
        // deny_unknown_fields prevents accidental wire/disk drift.
        let json = serde_json::json!({
            "vm": "corp-vm",
            "roleId": "ch",
            "role": "cloud-hypervisor",
            "pid": 100,
            "startTimeTicks": 200,
            "snapshottedAt": "2026-05-29T03:00:00Z",
            "extraField": "should_be_rejected"
        });
        let res: Result<RunnerSnapshotRecord, _> = serde_json::from_value(json);
        assert!(res.is_err());
    }

    // ---- W4-fu reconcile_and_adopt tests ----

    /// Fake PidfdOpener that lets tests configure per-pid success or
    /// failure without invoking SYS_pidfd_open.
    struct FakeOpener {
        // pid -> Ok(()) means succeed (we hand back a dummy fd); Err means race.
        outcomes: std::sync::Mutex<HashMap<i32, Result<(), String>>>,
    }

    impl FakeOpener {
        fn new() -> Self {
            Self {
                outcomes: std::sync::Mutex::new(HashMap::new()),
            }
        }
        fn succeed(&self, pid: i32) {
            self.outcomes.lock().unwrap().insert(pid, Ok(()));
        }
        fn race(&self, pid: i32, detail: &str) {
            self.outcomes
                .lock()
                .unwrap()
                .insert(pid, Err(detail.to_owned()));
        }
    }

    impl PidfdOpener for FakeOpener {
        fn open_pidfd(
            &self,
            _vm: &str,
            _role_id: &str,
            pid: i32,
            expected_start_time_ticks: u64,
        ) -> Result<std::os::fd::OwnedFd, String> {
            // Record the expected start-time so the test can
            // confirm reconcile_and_adopt is passing it through.
            let _ = expected_start_time_ticks;
            let outcome = self
                .outcomes
                .lock()
                .unwrap()
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| Err(format!("FakeOpener: no outcome configured for pid {pid}")));
            outcome.map(|()| {
                let file = std::fs::File::open("/dev/null").expect("open /dev/null");
                std::os::fd::OwnedFd::from(file)
            })
        }
    }

    #[test]
    fn reconcile_and_adopt_opens_pidfd_for_matching_records() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_alive(4242, 987_654_321);
        let opener = FakeOpener::new();
        opener.succeed(4242);

        let results = reconcile_and_adopt(&snaps, &reader, &opener);
        assert_eq!(results.len(), 1);
        match &results[0].outcome {
            AdoptOutcome::Adopted(_) => {}
            other => panic!("expected Adopted, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_and_adopt_treats_pidfd_race_as_distinct_outcome() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_alive(4242, 987_654_321);
        let opener = FakeOpener::new();
        opener.race(4242, "ESRCH between /proc check and pidfd_open");

        let results = reconcile_and_adopt(&snaps, &reader, &opener);
        match &results[0].outcome {
            AdoptOutcome::AdoptRaced { detail } => {
                assert!(detail.contains("ESRCH"));
            }
            other => panic!("expected AdoptRaced, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_and_adopt_skips_pidfd_for_quarantine_records() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_alive(4242, 111); // start-time drift => Quarantine
        let opener = FakeOpener::new();
        // Opener intentionally NOT configured for pid 4242; if the
        // executor called open_pidfd it would Err. Quarantine path
        // must skip open entirely.

        let results = reconcile_and_adopt(&snaps, &reader, &opener);
        match &results[0].outcome {
            AdoptOutcome::Quarantine {
                observed_start_time_ticks,
            } => assert_eq!(*observed_start_time_ticks, 111),
            other => panic!("expected Quarantine, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_and_adopt_propagates_missing_and_unparseable() {
        let snaps = vec![
            sample("corp-vm", "missing-pid", 1000, 200),
            sample("corp-vm", "bad-stat", 2000, 200),
        ];
        let mut reader = FakeProcReader::default();
        reader.add_missing(1000);
        reader.add_unparseable(2000, "stat layout drift");
        let opener = FakeOpener::new();

        let results = reconcile_and_adopt(&snaps, &reader, &opener);
        let kinds: Vec<&str> = results
            .iter()
            .map(|r| match &r.outcome {
                AdoptOutcome::Missing => "missing",
                AdoptOutcome::UnparseableProcStat { .. } => "unparseable",
                _ => "unexpected",
            })
            .collect();
        // Lex order: "bad-stat" before "missing-pid".
        assert_eq!(kinds, vec!["unparseable", "missing"]);
    }

    /// Recording variant: captures the expected start-time the
    /// trait was called with, for regression-testing the
    /// CRITICAL-fix passthrough.
    struct RecordingOpener {
        recorded: std::sync::Mutex<Vec<(i32, u64)>>,
    }

    impl RecordingOpener {
        fn new() -> Self {
            Self {
                recorded: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl PidfdOpener for RecordingOpener {
        fn open_pidfd(
            &self,
            _vm: &str,
            _role_id: &str,
            pid: i32,
            expected_start_time_ticks: u64,
        ) -> Result<std::os::fd::OwnedFd, String> {
            self.recorded
                .lock()
                .unwrap()
                .push((pid, expected_start_time_ticks));
            let file = std::fs::File::open("/dev/null").expect("open /dev/null");
            Ok(std::os::fd::OwnedFd::from(file))
        }
    }

    /// W*-fu GPT-5.5 panel CRITICAL fix regression test:
    /// reconcile_and_adopt MUST pass the snapshot's
    /// start_time_ticks through to the opener so the opener can
    /// perform the post-open re-verification that closes the
    /// pid-reuse race.
    #[test]
    fn reconcile_and_adopt_passes_expected_start_time_to_opener() {
        let snaps = vec![sample("corp-vm", "ch", 4242, 987_654_321)];
        let mut reader = FakeProcReader::default();
        reader.add_alive(4242, 987_654_321);
        let opener = RecordingOpener::new();
        let _ = reconcile_and_adopt(&snaps, &reader, &opener);
        let recorded = opener.recorded.lock().unwrap().clone();
        assert_eq!(recorded, vec![(4242, 987_654_321)]);
    }
}
