use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::supervisor::state::parse_proc_stat_starttime;

#[derive(Debug)]
pub struct PidfdTable {
    pub(crate) entries: RwLock<BTreeMap<(String, String), PidfdEntry>>,
    pub(crate) state_path: PathBuf,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitTermination {
    Terminated,
    TimedOut,
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
        }
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
        Ok(())
    }

    pub fn deregister(&self, vm: &str, role: &str) -> Option<PidfdEntry> {
        self.entries
            .write()
            .remove(&(vm.to_owned(), role.to_owned()))
    }

    pub fn contains(&self, vm: &str, role: &str) -> bool {
        self.entries
            .read()
            .contains_key(&(vm.to_owned(), role.to_owned()))
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
        use nix::sys::wait::{waitid, Id, WaitPidFlag, WaitStatus};

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
                    detail: format!("dup pidfd: {err}"),
                }
            })?;
            (entry.pid, entry.start_time_ticks, pidfd)
        };
        let deadline = Instant::now() + timeout;

        loop {
            let mut waitid_error = None;
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
                    }
                    return Ok(WaitTermination::Terminated);
                }
                Ok(WaitStatus::StillAlive) | Ok(_) => {}
                Err(Errno::ECHILD) => {}
                Err(err) => waitid_error = Some(err.to_string()),
            }

            let proc_state =
                read_proc_start_time(pid).map_err(|detail| PidfdTableError::WaitFailed {
                    vm: vm.to_owned(),
                    role: role.to_owned(),
                    pid,
                    detail,
                })?;
            match proc_state {
                Some(observed) if observed == start_time_ticks => {
                    if let Some(detail) = waitid_error {
                        return Err(PidfdTableError::WaitFailed {
                            vm: vm.to_owned(),
                            role: role.to_owned(),
                            pid,
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
                    }
                    return Ok(WaitTermination::Terminated);
                }
            }
        }
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
    file.write_all(&bytes)
        .map_err(|err| PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;
    file.sync_all()
        .map_err(|err| PidfdTableError::SnapshotFailed {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;
    fs::rename(&tmp, path).map_err(|err| PidfdTableError::SnapshotFailed {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    Ok(())
}

fn snapshot_tmp_path(path: &Path) -> PathBuf {
    let mut tmp = OsString::from(path.as_os_str());
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn reopen_persisted_entry(record: &PersistedPidfdEntry) -> Result<Option<PidfdEntry>, String> {
    let Some(pid) = rustix::process::Pid::from_raw(record.pid) else {
        return Ok(None);
    };
    let pidfd = match rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()) {
        Ok(pidfd) => pidfd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
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
        Ok(content) => parse_proc_stat_starttime(&content)
            .map(Some)
            .map_err(|err| err.to_string()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
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
        assert_eq!(
            restored
                .wait_terminated("alpha", "ch-runner", Duration::from_secs(5))
                .expect("wait terminated"),
            WaitTermination::Terminated
        );
        restored.snapshot().expect("snapshot cleared table");
        assert!(!restored.contains("alpha", "ch-runner"));

        let status = child.wait();
        assert!(!status.success());
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
        assert_eq!(
            table
                .wait_terminated("alpha", "ch-runner", Duration::from_secs(5))
                .expect("wait after kill"),
            WaitTermination::Terminated
        );
        let status = child.wait();
        assert!(!status.success());
    }
}
