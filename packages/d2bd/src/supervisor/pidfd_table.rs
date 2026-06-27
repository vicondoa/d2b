use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tracing;

static SNAPSHOT_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static SIGNAL_EPERM_TEST_ROLES: OnceLock<Mutex<std::collections::HashSet<(String, String)>>> =
    OnceLock::new();

use crate::supervisor::state::parse_proc_stat_starttime;

#[derive(Debug)]
pub struct PidfdTable {
    pub(crate) entries: RwLock<BTreeMap<(String, String), PidfdEntry>>,
    pub(crate) state_path: PathBuf,
    broker_reap_log: OnceLock<Arc<BrokerReapLog>>,
    /// Serializes register/deregister + snapshot sequences so concurrent
    /// different-VM lifecycle ops cannot interleave a register against a
    /// snapshot and lose an entry on disk. Callers that perform a
    /// "mutate the map, then persist" sequence hold this across BOTH
    /// steps via [`PidfdTable::mutation_guard`].
    mutation_lock: Mutex<()>,
    generation: AtomicU64,
}

#[derive(Debug)]
pub struct PidfdEntry {
    pub pidfd: OwnedFd,
    pub pid: i32,
    pub start_time_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidfdRegistration {
    pub vm: String,
    pub role: String,
    pub pid: i32,
    pub start_time_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitTermination {
    /// Child exited and was reaped by d2bd (normal path).
    Terminated,
    /// Child was already reaped by the broker's SIGCHLD handler;
    /// `wait_terminated` returned immediately on `ECHILD` using the
    /// buffered exit status rather than re-entering `/proc` polling.
    TerminatedByBroker {
        exit_status: d2b_contracts::broker_wire::ChildExitStatus,
    },
    TimedOut,
}

/// Shared log of broker-reaped child events.
///
/// Populated by d2bd's broker-interaction layer when it calls
/// `BrokerRequest::PollChildReaped`. Consulted by
/// [`PidfdTable::wait_terminated`] on `ECHILD` so it can return
/// [`WaitTermination::TerminatedByBroker`] immediately instead of
/// spinning on `/proc` polling.
#[derive(Debug, Default)]
pub struct BrokerReapLog {
    inner: Mutex<HashMap<i32, d2b_contracts::broker_wire::ChildReapedNotification>>,
}

impl BrokerReapLog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Insert (or overwrite) a ChildReaped event keyed by PID.
    pub fn insert(&self, notif: d2b_contracts::broker_wire::ChildReapedNotification) {
        self.inner.lock().insert(notif.pid, notif);
    }

    /// Remove and return the event for `pid`, if any.
    pub fn take(&self, pid: i32) -> Option<d2b_contracts::broker_wire::ChildReapedNotification> {
        self.inner.lock().remove(&pid)
    }

    /// Remove and return the event for a `(vm, role)` runner id, if any.
    pub fn take_for(
        &self,
        vm: &str,
        role: &str,
    ) -> Option<d2b_contracts::broker_wire::ChildReapedNotification> {
        let runner_id = format!("{vm}:{role}");
        let mut inner = self.inner.lock();
        let pid = inner
            .iter()
            .find_map(|(pid, notif)| (notif.runner_id == runner_id).then_some(*pid))?;
        inner.remove(&pid)
    }

    /// PEEK (non-consuming) the event for a `(vm, role)` runner id, if any.
    ///
    /// Unlike [`Self::take_for`] this does NOT remove the entry — the
    /// readiness liveness probe must only observe so the buffered exit
    /// status remains available to the mutating teardown path
    /// (`wait_terminated` / rollback) that owns deregistration.
    pub fn peek_for(
        &self,
        vm: &str,
        role: &str,
    ) -> Option<d2b_contracts::broker_wire::ChildReapedNotification> {
        let runner_id = format!("{vm}:{role}");
        self.inner
            .lock()
            .values()
            .find(|notif| notif.runner_id == runner_id)
            .cloned()
    }
}

#[derive(Debug)]
pub enum PidfdTableError {
    DuplicateRegistration {
        vm: String,
        role: String,
    },
    NotFound {
        vm: String,
        role: String,
    },
    InvalidSignal {
        signal: libc::c_int,
    },
    SignalFailed {
        vm: String,
        role: String,
        pid: i32,
        signal: libc::c_int,
        errno: Option<i32>,
        detail: String,
    },
    WaitFailed {
        vm: String,
        role: String,
        pid: i32,
        errno: Option<i32>,
        detail: String,
    },
    SnapshotFailed {
        path: PathBuf,
        detail: String,
    },
    RestoreFailed {
        path: PathBuf,
        detail: String,
    },
}

impl std::fmt::Display for PidfdTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateRegistration { vm, role } => {
                write!(f, "duplicate pidfd registration for {vm}:{role}")
            }
            Self::NotFound { vm, role } => write!(f, "pidfd table miss for {vm}:{role}"),
            Self::InvalidSignal { signal } => write!(f, "invalid pidfd signal {signal}"),
            Self::SignalFailed {
                vm,
                role,
                pid,
                signal,
                detail,
                ..
            } => write!(
                f,
                "pidfd_send_signal(vm={vm}, role={role}, pid={pid}, signal={signal}) failed: {detail}"
            ),
            Self::WaitFailed {
                vm,
                role,
                pid,
                detail,
                ..
            } => write!(
                f,
                "pidfd wait(vm={vm}, role={role}, pid={pid}) failed: {detail}"
            ),
            Self::SnapshotFailed { path, detail } => {
                write!(f, "snapshot {} failed: {detail}", path.display())
            }
            Self::RestoreFailed { path, detail } => {
                write!(f, "restore {} failed: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for PidfdTableError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedPidfdTable {
    entries: Vec<PersistedPidfdEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedPidfdEntry {
    vm: String,
    role: String,
    pid: i32,
    start_time_ticks: u64,
}

impl PidfdTable {
    pub fn new(state_path: PathBuf) -> Self {
        Self {
            entries: RwLock::new(BTreeMap::new()),
            state_path,
            broker_reap_log: OnceLock::new(),
            mutation_lock: Mutex::new(()),
            generation: AtomicU64::new(0),
        }
    }

    /// Attach a `BrokerReapLog` for ECHILD fast-path handling.
    pub fn with_broker_reap_log(self, log: Arc<BrokerReapLog>) -> Self {
        let _ = self.broker_reap_log.set(log);
        self
    }

    /// Set the `BrokerReapLog` on an already-constructed table (e.g.
    /// after `restore_from_disk`).
    pub fn set_broker_reap_log(&self, log: Arc<BrokerReapLog>) {
        let _ = self.broker_reap_log.set(log);
    }

    pub fn register(
        &self,
        vm: String,
        role: String,
        entry: PidfdEntry,
    ) -> Result<(), PidfdTableError> {
        let mut entries = self.entries.write();
        let key = (vm.clone(), role.clone());
        if entries.contains_key(&key) {
            return Err(PidfdTableError::DuplicateRegistration { vm, role });
        }
        entries.insert(key, entry);
        self.bump_generation();
        Ok(())
    }

    pub fn deregister(&self, vm: &str, role: &str) -> Option<PidfdEntry> {
        let mut entries = self.entries.write();
        let removed = entries.remove(&(vm.to_owned(), role.to_owned()));
        if removed.is_some() {
            self.bump_generation();
        }
        removed
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn contains(&self, vm: &str, role: &str) -> bool {
        self.entries
            .read()
            .contains_key(&(vm.to_owned(), role.to_owned()))
    }

    /// v1.1.1fu14 B3 + B7: validate every registered entry's
    /// `(pid, start_time_ticks)` against `/proc/<pid>/stat`,
    /// dropping any whose process has died or been replaced
    /// (start_time mismatch indicates PID reuse).
    ///
    /// Called from the `vm start` request handler before the
    /// "already has a registered supervisor pidfd" check, so
    /// orphaned entries left behind by a daemon crash + restart
    /// don't permanently block a fresh start.
    ///
    /// Returns the number of entries dropped. Snapshot is
    /// re-persisted to disk if any entries were dropped.
    pub fn prune_dead_entries(&self) -> Result<usize, PidfdTableError> {
        // Serialize the mutate + snapshot sequence against concurrent
        // register/deregister+snapshot from other VMs (same invariant as
        // `register_node_pidfd`): without this guard a prune snapshot can
        // land between another thread's register and snapshot and drop the
        // newer entry on disk. Neither caller (`is_running`, vm-start
        // dispatch) holds this guard, so acquiring it here is deadlock-free.
        let _mguard = self.mutation_guard();
        let mut to_drop: Vec<(String, String)> = Vec::new();
        {
            let entries = self.entries.read();
            for ((vm, role), entry) in entries.iter() {
                let alive = match read_proc_start_time(entry.pid) {
                    Ok(Some(observed)) => observed == entry.start_time_ticks,
                    Ok(None) => false,
                    Err(_) => false,
                };
                if !alive {
                    to_drop.push((vm.clone(), role.clone()));
                }
            }
        }
        let dropped = to_drop.len();
        if dropped > 0 {
            let mut entries = self.entries.write();
            for key in &to_drop {
                entries.remove(key);
            }
            self.bump_generation();
            drop(entries);
            self.snapshot()?;
            for (vm, role) in &to_drop {
                tracing::warn!(
                    vm = %vm,
                    role = %role,
                    "pidfd-table: dropped stale entry (process died or PID reused)",
                );
            }
        }
        Ok(dropped)
    }

    pub fn list_for_vm(&self, vm: &str) -> Vec<PidfdRegistration> {
        self.entries
            .read()
            .iter()
            .filter(|((entry_vm, _), _)| entry_vm == vm)
            .map(|((entry_vm, role), entry)| PidfdRegistration {
                vm: entry_vm.clone(),
                role: role.clone(),
                pid: entry.pid,
                start_time_ticks: entry.start_time_ticks,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn signal(&self, vm: &str, role: &str, sig: libc::c_int) -> Result<(), PidfdTableError> {
        let signal = rustix::process::Signal::from_raw(sig)
            .ok_or(PidfdTableError::InvalidSignal { signal: sig })?;
        let entries = self.entries.read();
        let entry = entries
            .get(&(vm.to_owned(), role.to_owned()))
            .ok_or_else(|| PidfdTableError::NotFound {
                vm: vm.to_owned(),
                role: role.to_owned(),
            })?;
        #[cfg(test)]
        if SIGNAL_EPERM_TEST_ROLES
            .get_or_init(|| Mutex::new(Default::default()))
            .lock()
            .contains(&(vm.to_owned(), role.to_owned()))
        {
            return Err(PidfdTableError::SignalFailed {
                vm: vm.to_owned(),
                role: role.to_owned(),
                pid: entry.pid,
                signal: sig,
                errno: Some(libc::EPERM),
                detail: "forced EPERM for test".to_owned(),
            });
        }
        rustix::process::pidfd_send_signal(entry.pidfd.as_fd(), signal).map_err(|err| {
            PidfdTableError::SignalFailed {
                vm: vm.to_owned(),
                role: role.to_owned(),
                pid: entry.pid,
                signal: sig,
                errno: Some(err.raw_os_error()),
                detail: err.to_string(),
            }
        })
    }

    pub fn wait_terminated(
        &self,
        vm: &str,
        role: &str,
        timeout: Duration,
    ) -> Result<WaitTermination, PidfdTableError> {
        use nix::errno::Errno;
        use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};

        let key = (vm.to_owned(), role.to_owned());
        let (pid, start_time_ticks, pidfd) = {
            let entries = self.entries.read();
            let entry = entries.get(&key).ok_or_else(|| PidfdTableError::NotFound {
                vm: vm.to_owned(),
                role: role.to_owned(),
            })?;
            let pidfd = rustix::io::dup(entry.pidfd.as_fd()).map_err(|err| {
                PidfdTableError::WaitFailed {
                    vm: vm.to_owned(),
                    role: role.to_owned(),
                    pid: entry.pid,
                    errno: Some(err.raw_os_error()),
                    detail: format!("dup pidfd: {err}"),
                }
            })?;
            (entry.pid, entry.start_time_ticks, pidfd)
        };
        let deadline = Instant::now() + timeout;

        loop {
            let mut waitid_error: Option<(Option<i32>, String)> = None;
            match waitid(
                Id::PIDFd(pidfd.as_fd()),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
            ) {
                Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => {
                    let mut entries = self.entries.write();
                    if matches!(
                        entries.get(&key),
                        Some(current)
                            if current.pid == pid && current.start_time_ticks == start_time_ticks
                    ) {
                        entries.remove(&key);
                        self.bump_generation();
                    }
                    return Ok(WaitTermination::Terminated);
                }
                Ok(WaitStatus::StillAlive) | Ok(_) => {}
                Err(Errno::ECHILD) => {
                    if let Some(log) = self.broker_reap_log.get()
                        && let Some(notif) = log.take(pid)
                    {
                        let mut entries = self.entries.write();
                        if matches!(
                            entries.get(&key),
                            Some(current)
                                if current.pid == pid
                                    && current.start_time_ticks == start_time_ticks
                        ) {
                            entries.remove(&key);
                            self.bump_generation();
                        }
                        return Ok(WaitTermination::TerminatedByBroker {
                            exit_status: notif.exit_status,
                        });
                    }
                    waitid_error = Some((Some(libc::ECHILD), "waitid(P_PIDFD): ECHILD".to_owned()));
                }
                Err(err) => waitid_error = Some((Some(err as i32), err.to_string())),
            }

            let proc_state =
                read_proc_start_time(pid).map_err(|detail| PidfdTableError::WaitFailed {
                    vm: vm.to_owned(),
                    role: role.to_owned(),
                    pid,
                    errno: None,
                    detail,
                })?;
            match proc_state {
                Some(observed) if observed == start_time_ticks => {
                    if let Some((errno, detail)) = waitid_error {
                        return Err(PidfdTableError::WaitFailed {
                            vm: vm.to_owned(),
                            role: role.to_owned(),
                            pid,
                            errno,
                            detail: format!("waitid(P_PIDFD): {detail}"),
                        });
                    }
                    if Instant::now() >= deadline {
                        return Ok(WaitTermination::TimedOut);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Some(_) | None => {
                    let mut entries = self.entries.write();
                    if matches!(
                        entries.get(&key),
                        Some(current)
                            if current.pid == pid && current.start_time_ticks == start_time_ticks
                    ) {
                        entries.remove(&key);
                        self.bump_generation();
                    }
                    return Ok(WaitTermination::Terminated);
                }
            }
        }
    }

    pub fn still_alive_same_start_time(&self, vm: &str, role: &str) -> bool {
        let (pid, start_time_ticks) = {
            let entries = self.entries.read();
            match entries.get(&(vm.to_owned(), role.to_owned())) {
                Some(entry) => (entry.pid, entry.start_time_ticks),
                None => return false,
            }
        };
        matches!(read_proc_start_time(pid), Ok(Some(observed)) if observed == start_time_ticks)
    }

    /// Acquire the register/deregister + snapshot serialization guard.
    ///
    /// Callers that perform a "mutate the in-memory map, then persist a
    /// snapshot" sequence hold this guard across BOTH steps so concurrent
    /// different-VM ops cannot lose an entry on disk (one thread's
    /// snapshot landing between another thread's register and snapshot).
    pub fn mutation_guard(&self) -> parking_lot::MutexGuard<'_, ()> {
        self.mutation_lock.lock()
    }

    /// Duplicate the daemon-held pidfd for `(vm, role)` for a read-only
    /// liveness poll. Returns the dup'd fd plus the registered
    /// `(pid, start_time_ticks)`, or `None` when no entry is registered
    /// (e.g. rollback already removed it) or the dup fails.
    ///
    /// This OBSERVES only — it never removes the entry. All
    /// deregistration stays in the teardown / rollback path.
    pub fn dup_pidfd_for(&self, vm: &str, role: &str) -> Option<(OwnedFd, i32, u64)> {
        let entries = self.entries.read();
        let entry = entries.get(&(vm.to_owned(), role.to_owned()))?;
        let dup = rustix::io::dup(entry.pidfd.as_fd()).ok()?;
        Some((dup, entry.pid, entry.start_time_ticks))
    }

    /// Whether `(vm, role)` is currently registered. Distinguishes a
    /// rollback-removed entry (`false`) from a live one without mutating.
    pub fn has_entry(&self, vm: &str, role: &str) -> bool {
        self.contains(vm, role)
    }

    pub fn snapshot(&self) -> Result<(), PidfdTableError> {
        let persisted = {
            let entries = self.entries.read();
            PersistedPidfdTable {
                entries: entries
                    .iter()
                    .map(|((vm, role), entry)| PersistedPidfdEntry {
                        vm: vm.clone(),
                        role: role.clone(),
                        pid: entry.pid,
                        start_time_ticks: entry.start_time_ticks,
                    })
                    .collect(),
            }
        };
        write_snapshot(&self.state_path, &persisted)
    }

    pub fn restore_from_disk(state_path: &Path) -> Result<Self, PidfdTableError> {
        let table = Self::new(state_path.to_path_buf());
        if !state_path.exists() {
            return Ok(table);
        }

        let bytes = fs::read(state_path).map_err(|err| PidfdTableError::RestoreFailed {
            path: state_path.to_path_buf(),
            detail: err.to_string(),
        })?;
        let persisted: PersistedPidfdTable =
            serde_json::from_slice(&bytes).map_err(|err| PidfdTableError::RestoreFailed {
                path: state_path.to_path_buf(),
                detail: err.to_string(),
            })?;

        let mut dropped_stale = false;
        {
            let mut entries = table.entries.write();
            for persisted_entry in persisted.entries {
                let key = (persisted_entry.vm.clone(), persisted_entry.role.clone());
                if entries.contains_key(&key) {
                    return Err(PidfdTableError::RestoreFailed {
                        path: state_path.to_path_buf(),
                        detail: format!(
                            "duplicate pidfd-table key for {}:{}",
                            persisted_entry.vm, persisted_entry.role
                        ),
                    });
                }
                match reopen_persisted_entry(&persisted_entry).map_err(|detail| {
                    PidfdTableError::RestoreFailed {
                        path: state_path.to_path_buf(),
                        detail,
                    }
                })? {
                    Some(entry) => {
                        entries.insert(key, entry);
                        table.generation.fetch_add(1, Ordering::AcqRel);
                    }
                    None => dropped_stale = true,
                }
            }
        }

        if dropped_stale {
            table.snapshot()?;
        }

        Ok(table)
    }
}

#[cfg(test)]
pub fn force_signal_eperm_for_tests(vm: &str, role: &str, enabled: bool) {
    let mut roles = SIGNAL_EPERM_TEST_ROLES
        .get_or_init(|| Mutex::new(Default::default()))
        .lock();
    let key = (vm.to_owned(), role.to_owned());
    if enabled {
        roles.insert(key);
    } else {
        roles.remove(&key);
    }
}

fn write_snapshot(path: &Path, snapshot: &PersistedPidfdTable) -> Result<(), PidfdTableError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;
    }

    let bytes =
        serde_json::to_vec_pretty(snapshot).map_err(|err| PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;
    let tmp = snapshot_tmp_path(path);
    let mut file = File::create(&tmp).map_err(|err| PidfdTableError::SnapshotFailed {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    if let Err(err) = file.set_permissions(fs::Permissions::from_mode(0o644)) {
        if let Err(rm_err) = fs::remove_file(&tmp) {
            tracing::debug!(?tmp, %rm_err, "pidfd-table: tmpfile cleanup failed (chmod path)");
        }
        return Err(PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        });
    }
    if let Err(err) = file.write_all(&bytes) {
        // panel-rust v1.1.2-final-R1 should-fix: log cleanup
        // failures so disk-full / permission regressions surface.
        if let Err(rm_err) = fs::remove_file(&tmp) {
            tracing::debug!(?tmp, %rm_err, "pidfd-table: tmpfile cleanup failed (write path)");
        }
        return Err(PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        });
    }
    if let Err(err) = file.sync_all() {
        if let Err(rm_err) = fs::remove_file(&tmp) {
            tracing::debug!(?tmp, %rm_err, "pidfd-table: tmpfile cleanup failed (sync path)");
        }
        return Err(PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        });
    }
    if let Err(err) = fs::rename(&tmp, path) {
        if let Err(rm_err) = fs::remove_file(&tmp) {
            tracing::debug!(?tmp, %rm_err, "pidfd-table: tmpfile cleanup failed (rename path)");
        }
        return Err(PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        });
    }
    Ok(())
}

fn snapshot_tmp_path(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let seq = SNAPSHOT_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = OsString::from(path.as_os_str());
    tmp.push(format!(".tmp.{}.{}", pid, seq));
    PathBuf::from(tmp)
}

fn reopen_persisted_entry(record: &PersistedPidfdEntry) -> Result<Option<PidfdEntry>, String> {
    let Some(pid) = rustix::process::Pid::from_raw(record.pid) else {
        return Ok(None);
    };
    let pidfd = match rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()) {
        Ok(pidfd) => pidfd,
        // A non-root daemon cannot always reopen pidfds for runner
        // principals directly. Treat that like a stale direct restore:
        // startup's broker-backed orphan adoption can reacquire live
        // runners through the privileged OpenPidfd op.
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            ) || matches!(err.raw_os_error(), libc::ESRCH | libc::EPERM | libc::EACCES) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(format!("pidfd_open({}): {err}", record.pid)),
    };
    match read_proc_start_time(record.pid)? {
        Some(observed) if observed == record.start_time_ticks => Ok(Some(PidfdEntry {
            pidfd,
            pid: record.pid,
            start_time_ticks: record.start_time_ticks,
        })),
        Some(_) | None => Ok(None),
    }
}

fn read_proc_start_time(pid: i32) -> Result<Option<u64>, String> {
    let path = format!("/proc/{pid}/stat");
    match fs::read_to_string(&path) {
        Ok(content) => proc_stat_live_start_time(&content),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

fn proc_stat_live_start_time(content: &str) -> Result<Option<u64>, String> {
    if proc_stat_state(content).is_some_and(|state| matches!(state, 'Z' | 'X')) {
        return Ok(None);
    }
    parse_proc_stat_starttime(content)
        .map(Some)
        .map_err(|err| err.to_string())
}

fn proc_stat_state(content: &str) -> Option<char> {
    let close = content.trim_end_matches('\n').rfind(')')?;
    content[close + 1..]
        .split_whitespace()
        .next()
        .and_then(|state| state.chars().next())
}

/// Public read-only `/proc/<pid>/stat` start-time read used by the
/// readiness liveness probe to distinguish a still-our-process from a
/// PID-reuse (start-time drift) or a gone process. Never mutates.
pub fn read_proc_start_time_pub(pid: i32) -> Result<Option<u64>, String> {
    read_proc_start_time(pid)
}

pub fn set_child_subreaper_with_self_test() -> Result<(), PidfdTableError> {
    let enable = rustix::process::Pid::from_raw(1);
    rustix::process::set_child_subreaper(enable).map_err(|err| PidfdTableError::RestoreFailed {
        path: PathBuf::from("PR_SET_CHILD_SUBREAPER"),
        detail: format!("PR_SET_CHILD_SUBREAPER=1: {err}"),
    })?;
    let observed =
        rustix::process::child_subreaper().map_err(|err| PidfdTableError::RestoreFailed {
            path: PathBuf::from("PR_GET_CHILD_SUBREAPER"),
            detail: format!("PR_GET_CHILD_SUBREAPER readback: {err}"),
        })?;
    if observed.is_none() {
        return Err(PidfdTableError::RestoreFailed {
            path: PathBuf::from("PR_GET_CHILD_SUBREAPER"),
            detail: "PR_GET_CHILD_SUBREAPER returned 0 after PR_SET_CHILD_SUBREAPER=1".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    use nix::fcntl::OFlag;
    use nix::unistd::pipe2;

    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct ChildGuard {
        child: Child,
    }

    impl ChildGuard {
        fn new(child: Child) -> Self {
            Self { child }
        }

        fn child(&self) -> &Child {
            &self.child
        }

        fn wait(mut self) -> std::process::ExitStatus {
            self.child.wait().expect("wait child")
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Ok(None) = self.child.try_wait() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    fn fresh_state_path(test_name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/pidfd-table-tests");
        fs::create_dir_all(&root).expect("create pidfd-table-tests dir");
        let path = root.join(format!(
            "{test_name}-{}-{}.json",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::remove_file(&path).ok();
        path
    }

    fn pipe_owned_fd() -> OwnedFd {
        let (read_end, write_end) = pipe2(OFlag::O_CLOEXEC).expect("pipe2");
        std::mem::forget(write_end);
        read_end
    }

    fn read_child_start_time(child: &Child) -> u64 {
        read_proc_start_time(child.id() as i32)
            .expect("read proc start time")
            .expect("child still present")
    }

    #[test]
    fn proc_stat_live_start_time_treats_zombie_and_dead_as_gone() {
        let zombie =
            "1234 (cloud-hyperv) Z 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 987654321 0";
        let dead =
            "1234 (cloud-hyperv) X 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 987654321 0";
        let sleeping =
            "1234 (cloud-hyperv) S 1 1234 1234 0 -1 0 0 0 0 0 10 20 0 0 20 0 1 0 987654321 0";

        assert_eq!(proc_stat_live_start_time(zombie).expect("zombie"), None);
        assert_eq!(proc_stat_live_start_time(dead).expect("dead"), None);
        assert_eq!(
            proc_stat_live_start_time(sleeping).expect("sleeping"),
            Some(987_654_321)
        );
    }

    fn open_child_pidfd(child: &Child) -> OwnedFd {
        rustix::process::pidfd_open(
            rustix::process::Pid::from_child(child),
            rustix::process::PidfdFlags::empty(),
        )
        .expect("pidfd_open child")
    }

    #[test]
    fn registers_and_deregisters() {
        let table = PidfdTable::new(fresh_state_path("registers-and-deregisters"));
        table
            .register(
                "alpha".into(),
                "ch".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 4242,
                    start_time_ticks: 1234,
                },
            )
            .unwrap();
        assert_eq!(table.len(), 1);
        assert!(table.contains("alpha", "ch"));
        let dropped = table.deregister("alpha", "ch").expect("present");
        assert_eq!(dropped.pid, 4242);
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn observe_only_helpers_do_not_deregister() {
        // The readiness liveness probe ONLY observes — `dup_pidfd_for`
        // and the reap-log `peek_for` must never remove an entry, so
        // the mutating rollback path retains ownership of deregistration.
        let table = PidfdTable::new(fresh_state_path("observe-only"));
        table
            .register(
                "work".into(),
                "swtpm".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 7777,
                    start_time_ticks: 99,
                },
            )
            .unwrap();
        assert_eq!(table.len(), 1);

        // Repeated dups never consume the entry.
        for _ in 0..3 {
            let (dup, pid, start) = table
                .dup_pidfd_for("work", "swtpm")
                .expect("entry present for dup");
            assert_eq!(pid, 7777);
            assert_eq!(start, 99);
            drop(dup);
        }
        assert!(table.has_entry("work", "swtpm"));
        assert_eq!(table.len(), 1, "dup_pidfd_for must not deregister");

        let reap_log = BrokerReapLog::new();
        reap_log.insert(d2b_contracts::broker_wire::ChildReapedNotification {
            runner_id: "work:swtpm".to_owned(),
            pid: 7777,
            exit_status: d2b_contracts::broker_wire::ChildExitStatus {
                kind: d2b_contracts::broker_wire::ChildExitKind::Exited,
                code: Some(1),
                signal: None,
            },
            reaped_at_ms: 0,
        });
        // Repeated peeks never consume the reap entry.
        for _ in 0..3 {
            let peeked = reap_log
                .peek_for("work", "swtpm")
                .expect("reap entry present");
            assert_eq!(peeked.pid, 7777);
        }
        // After observe-only peeks the consuming take_for still finds it.
        assert!(
            reap_log.take_for("work", "swtpm").is_some(),
            "peek_for must not consume the buffered reap status"
        );

        // Deregistration remains the rollback path's responsibility.
        let dropped = table.deregister("work", "swtpm").expect("present");
        assert_eq!(dropped.pid, 7777);
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn lists_entries_for_specific_vm() {
        let table = PidfdTable::new(fresh_state_path("list-for-vm"));
        table
            .register(
                "alpha".into(),
                "ch".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 4242,
                    start_time_ticks: 1,
                },
            )
            .unwrap();
        table
            .register(
                "alpha".into(),
                "virtiofsd".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 4343,
                    start_time_ticks: 2,
                },
            )
            .unwrap();
        table
            .register(
                "beta".into(),
                "ch".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 4444,
                    start_time_ticks: 3,
                },
            )
            .unwrap();

        assert_eq!(
            table.list_for_vm("alpha"),
            vec![
                PidfdRegistration {
                    vm: "alpha".into(),
                    role: "ch".into(),
                    pid: 4242,
                    start_time_ticks: 1,
                },
                PidfdRegistration {
                    vm: "alpha".into(),
                    role: "virtiofsd".into(),
                    pid: 4343,
                    start_time_ticks: 2,
                },
            ]
        );
        assert_eq!(
            table.list_for_vm("beta"),
            vec![PidfdRegistration {
                vm: "beta".into(),
                role: "ch".into(),
                pid: 4444,
                start_time_ticks: 3,
            }]
        );
        assert!(table.list_for_vm("missing").is_empty());
    }

    #[test]
    fn refuses_duplicate_registration() {
        let table = PidfdTable::new(fresh_state_path("duplicate-registration"));
        table
            .register(
                "alpha".into(),
                "ch".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 4242,
                    start_time_ticks: 1,
                },
            )
            .unwrap();
        let err = table
            .register(
                "alpha".into(),
                "ch".into(),
                PidfdEntry {
                    pidfd: pipe_owned_fd(),
                    pid: 4242,
                    start_time_ticks: 1,
                },
            )
            .unwrap_err();
        match err {
            PidfdTableError::DuplicateRegistration { vm, role } => {
                assert_eq!(vm, "alpha");
                assert_eq!(role, "ch");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn child_subreaper_self_test_takes_effect() {
        set_child_subreaper_with_self_test().expect("subreaper self test");
        set_child_subreaper_with_self_test().expect("idempotent");
    }

    #[test]
    fn register_signal_snapshot_roundtrip() {
        let state_path = fresh_state_path("signal-roundtrip");
        let table = PidfdTable::new(state_path.clone());
        let child = ChildGuard::new(
            Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child"),
        );
        let pid = child.child().id() as i32;
        table
            .register(
                "alpha".into(),
                "ch-runner".into(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks: read_child_start_time(child.child()),
                },
            )
            .unwrap();
        table.snapshot().expect("snapshot pidfd table");
        let snapshot = fs::read_to_string(&state_path).expect("read snapshot");
        assert!(snapshot.contains("alpha"));
        assert!(snapshot.contains("ch-runner"));

        let restored = PidfdTable::restore_from_disk(&state_path).expect("restore pidfd table");
        assert!(restored.contains("alpha", "ch-runner"));
        restored
            .signal("alpha", "ch-runner", libc::SIGTERM)
            .expect("signal child by pidfd");
        assert!(matches!(
            restored
                .wait_terminated("alpha", "ch-runner", Duration::from_secs(5))
                .expect("wait terminated"),
            WaitTermination::Terminated | WaitTermination::TerminatedByBroker { .. }
        ));
        restored.snapshot().expect("snapshot cleared table");
        assert!(!restored.contains("alpha", "ch-runner"));

        let status = child.wait();
        assert!(!status.success());
    }

    #[test]
    fn restore_drops_stale_esrch_entries() {
        let state_path = fresh_state_path("restore-drops-stale-esrch");
        write_snapshot(
            &state_path,
            &PersistedPidfdTable {
                entries: vec![PersistedPidfdEntry {
                    vm: "alpha".to_owned(),
                    role: "ch-runner".to_owned(),
                    pid: i32::MAX,
                    start_time_ticks: 1,
                }],
            },
        )
        .expect("write stale snapshot");

        let restored = PidfdTable::restore_from_disk(&state_path).expect("restore pidfd table");
        assert!(!restored.contains("alpha", "ch-runner"));
        assert_eq!(restored.len(), 0);

        let bytes = fs::read(&state_path).expect("read pruned snapshot");
        let persisted: PersistedPidfdTable =
            serde_json::from_slice(&bytes).expect("parse pruned snapshot");
        assert!(persisted.entries.is_empty());
    }

    #[test]
    fn wait_terminated_times_out_for_running_child() {
        let table = PidfdTable::new(fresh_state_path("wait-terminated-timeout"));
        let child = ChildGuard::new(
            Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child"),
        );
        let pid = child.child().id() as i32;
        table
            .register(
                "alpha".into(),
                "ch-runner".into(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks: read_child_start_time(child.child()),
                },
            )
            .unwrap();

        assert_eq!(
            table
                .wait_terminated("alpha", "ch-runner", Duration::from_millis(100))
                .expect("wait timed out"),
            WaitTermination::TimedOut
        );
        assert!(table.contains("alpha", "ch-runner"));
        table
            .signal("alpha", "ch-runner", libc::SIGKILL)
            .expect("kill child");
        assert!(matches!(
            table
                .wait_terminated("alpha", "ch-runner", Duration::from_secs(5))
                .expect("wait after kill"),
            WaitTermination::Terminated | WaitTermination::TerminatedByBroker { .. }
        ));
        let status = child.wait();
        assert!(!status.success());
    }

    /// Broker-reaped child fast-path.
    ///
    /// Simulates the broker having already consumed the child's exit
    /// status via `waitid(P_PIDFD, WEXITED)` and the daemon having
    /// recorded the corresponding `ChildReaped` notification.
    #[test]
    fn wait_terminated_echild_uses_broker_reap_log() {
        use d2b_contracts::broker_wire::{ChildExitKind, ChildExitStatus, ChildReapedNotification};
        use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};

        let child = Command::new("sleep")
            .arg("1")
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id() as i32;
        let pidfd = open_child_pidfd(&child);
        let reaper_pidfd = rustix::io::dup(pidfd.as_fd()).expect("dup pidfd for reaper");
        let start_time = read_child_start_time(&child);

        let log = BrokerReapLog::new();
        let table = PidfdTable::new(fresh_state_path("echild-broker-reap"))
            .with_broker_reap_log(log.clone());
        table
            .register(
                "test-vm".into(),
                "ch-runner".into(),
                PidfdEntry {
                    pidfd,
                    pid,
                    start_time_ticks: start_time,
                },
            )
            .expect("register child");

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match waitid(
                Id::PIDFd(reaper_pidfd.as_fd()),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG,
            ) {
                Ok(WaitStatus::StillAlive) => {
                    assert!(Instant::now() < deadline, "child did not exit in time");
                    std::thread::sleep(Duration::from_millis(25));
                }
                Ok(WaitStatus::Exited(_, code)) => {
                    assert_eq!(code, 0);
                    break;
                }
                Ok(other) => panic!("unexpected wait status: {other:?}"),
                Err(err) => panic!("waitid reaper pidfd failed: {err}"),
            }
        }

        log.insert(ChildReapedNotification {
            runner_id: "test-vm:ch-runner".to_owned(),
            pid,
            exit_status: ChildExitStatus {
                kind: ChildExitKind::Exited,
                code: Some(0),
                signal: None,
            },
            reaped_at_ms: 0,
        });
        std::mem::forget(child);

        let result = table
            .wait_terminated("test-vm", "ch-runner", Duration::from_secs(5))
            .expect("wait_terminated should succeed");
        match result {
            WaitTermination::TerminatedByBroker { exit_status } => {
                assert_eq!(exit_status.kind, ChildExitKind::Exited);
                assert_eq!(exit_status.code, Some(0));
            }
            other => panic!("expected TerminatedByBroker, got {other:?}"),
        }
    }

    #[test]
    fn child_reap_buffer_survives_disconnect_reconnect() {
        use d2b_contracts::broker_wire::{ChildExitKind, ChildExitStatus, ChildReapedNotification};

        let log = BrokerReapLog::new();
        log.insert(ChildReapedNotification {
            runner_id: "vm-a:ch".to_owned(),
            pid: 1001,
            exit_status: ChildExitStatus {
                kind: ChildExitKind::Exited,
                code: Some(0),
                signal: None,
            },
            reaped_at_ms: 1000,
        });
        log.insert(ChildReapedNotification {
            runner_id: "vm-b:ch".to_owned(),
            pid: 1002,
            exit_status: ChildExitStatus {
                kind: ChildExitKind::Killed,
                code: None,
                signal: Some(9),
            },
            reaped_at_ms: 2000,
        });

        let retrieved1 = log.take(1001).expect("should have notif for pid 1001");
        assert_eq!(retrieved1.runner_id, "vm-a:ch");
        let retrieved2 = log.take(1002).expect("should have notif for pid 1002");
        assert_eq!(retrieved2.exit_status.kind, ChildExitKind::Killed);
        assert!(log.take(1001).is_none());
    }

    /// v1.1.2-final-R1 (panel-test CRITICAL): concurrent snapshot
    /// stress test that would have caught the fu32 tmpfile race.
    /// 8 threads each register a pidfd then call snapshot(). The
    /// snapshot_tmp_path uses pid + atomic counter so concurrent
    /// calls must NOT collide on the .tmp filename.
    ///
    /// Pre-fix behavior: ~50% snapshot() calls fail with ENOENT
    /// because thread A's rename(.tmp → path) wins, then thread
    /// B's rename(.tmp → path) fails because thread A's File::create
    /// truncated thread B's tmp file before A's rename.
    ///
    /// Post-fix behavior: each thread writes to a unique
    /// .tmp.<pid>.<seq> path; all 8 snapshot() calls succeed.
    #[test]
    fn snapshot_under_concurrent_load_succeeds() {
        let tmpdir = mktemp_dir();
        let state_path = tmpdir.join("pidfd-table.json");
        let table = std::sync::Arc::new(PidfdTable::new(state_path.clone()));

        // Seed with a dummy entry so snapshot() has something to
        // serialise (an empty entries map is also fine but real
        // workloads always have entries during contention).
        let pid_self = std::process::id() as i32;
        let starttime = read_proc_start_time(pid_self).ok().flatten().unwrap_or(1);
        let pidfd_self = rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(pid_self).unwrap(),
            rustix::process::PidfdFlags::empty(),
        )
        .expect("pidfd_open self");
        table
            .register(
                "seed-vm".to_owned(),
                "seed-role".to_owned(),
                PidfdEntry {
                    pidfd: pidfd_self,
                    pid: pid_self,
                    start_time_ticks: starttime,
                },
            )
            .expect("seed register");

        let mut handles = Vec::new();
        for _ in 0..8 {
            let t = std::sync::Arc::clone(&table);
            handles.push(std::thread::spawn(move || {
                for _ in 0..20 {
                    t.snapshot().expect("concurrent snapshot must succeed");
                }
            }));
        }
        for h in handles {
            h.join().expect("thread join");
        }

        // Final state-file must exist and be valid JSON
        let bytes = fs::read(&state_path).expect("read final snapshot");
        let _: PersistedPidfdTable =
            serde_json::from_slice(&bytes).expect("final snapshot is valid JSON");

        // No leaked tmp files should remain (each was either
        // renamed onto state_path or cleaned up on error path).
        let parent = state_path.parent().expect("has parent");
        for entry in fs::read_dir(parent).expect("read tmpdir") {
            let entry = entry.expect("dir entry");
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            assert!(!name_str.contains(".tmp."), "leaked tmp file: {}", name_str);
        }
    }

    /// v1.1.2-final-R1 (panel-kernel + panel-test): verify
    /// snapshot_tmp_path generates distinct names for concurrent
    /// callers. Direct unit test of the file-naming function.
    #[test]
    fn snapshot_tmp_path_is_unique_per_call() {
        let path = PathBuf::from("/var/lib/d2b/daemon-state/pidfd-table.json");
        let a = snapshot_tmp_path(&path);
        let b = snapshot_tmp_path(&path);
        let c = snapshot_tmp_path(&path);
        assert_ne!(a, b, "consecutive tmp paths must differ");
        assert_ne!(b, c, "consecutive tmp paths must differ");
        assert_ne!(a, c, "consecutive tmp paths must differ");
        // All three must START with the canonical path + ".tmp."
        let prefix = format!("{}.tmp.", path.display());
        for p in [&a, &b, &c] {
            let s = p.display().to_string();
            assert!(
                s.starts_with(&prefix),
                "{} does not start with {}",
                s,
                prefix
            );
        }
    }

    /// fix2b: register + snapshot under the table's `mutation_guard` for two
    /// distinct VMs concurrently must persist BOTH entries. Without
    /// serialising the read-modify-persist sequence, a delayed snapshot from
    /// thread A (taken before B registered) could overwrite the file and drop
    /// B's entry. The guard makes register+snapshot atomic per op, so the
    /// final persisted snapshot is the union of every VM's entries.
    #[test]
    fn concurrent_different_vm_register_and_snapshot_under_guard_loses_no_entries() {
        let tmpdir = mktemp_dir();
        let state_path = tmpdir.join("pidfd-table.json");
        let table = std::sync::Arc::new(PidfdTable::new(state_path.clone()));

        let mut handles = Vec::new();
        for vm in ["vm-a", "vm-b", "vm-c", "vm-d"] {
            let t = std::sync::Arc::clone(&table);
            handles.push(std::thread::spawn(move || {
                // Mirror register_node_pidfd's critical section: hold the
                // mutation guard across register + snapshot.
                let _g = t.mutation_guard();
                t.register(
                    vm.to_owned(),
                    "swtpm".to_owned(),
                    PidfdEntry {
                        pidfd: pipe_owned_fd(),
                        pid: 4242,
                        start_time_ticks: 7,
                    },
                )
                .expect("register");
                t.snapshot().expect("snapshot under guard");
            }));
        }
        for h in handles {
            h.join().expect("thread join");
        }

        let bytes = fs::read(&state_path).expect("read final snapshot");
        let persisted: PersistedPidfdTable =
            serde_json::from_slice(&bytes).expect("final snapshot is valid JSON");
        let vms: std::collections::BTreeSet<&str> =
            persisted.entries.iter().map(|e| e.vm.as_str()).collect();
        for vm in ["vm-a", "vm-b", "vm-c", "vm-d"] {
            assert!(vms.contains(vm), "persisted snapshot dropped {vm}: {vms:?}");
        }
        assert_eq!(persisted.entries.len(), 4, "no entries lost or duplicated");
    }

    fn mktemp_dir() -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/pidfd-table-tests");
        std::fs::create_dir_all(&root).expect("mkdir test root");
        let path = root.join(format!("d2b-pidfd-test-{pid}-{id}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("mkdir test tmpdir");
        path
    }
}
