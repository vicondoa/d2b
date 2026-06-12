//! Detached exec: the transient-unit abstraction and its production
//! `systemd-run`/`systemctl` implementation.
//!
//! The full detached registry (slot allocator, quota accounting, creation
//! state machine, re-adoption, TTL/GC, live reconciliation) is built on top of
//! this trait. Only the abstraction + production manager shape live here so the
//! registry can be unit-tested against an in-memory fake.

use std::path::PathBuf;

use async_trait::async_trait;

use nixling_exec_runner::paths::RUN_DIR;

/// Redacted, typed transient-unit failure. Carries no command output or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitError {
    /// The helper subprocess could not be spawned (binary missing, fork
    /// failure, ...).
    SpawnFailed,
    /// The helper subprocess returned a non-zero status.
    NonZeroExit,
    /// The helper did not complete within the bounded window.
    Timeout,
    /// Detached units are not configured for this guest (no runtime config).
    Unsupported,
    /// Anything else, with no payload surfaced to callers.
    Internal,
}

/// A transient unit the manager currently knows about (re-adoption input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedUnit {
    pub slot: u32,
    /// True when the unit is loaded and active/activating.
    pub active: bool,
}

/// Absolute, controlled paths the manager needs to launch a runner unit. All
/// values are host-supplied constants (never caller-derived).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerUnitPaths {
    /// Absolute path to the `nixling-exec-runner` binary.
    pub exec_runner_path: PathBuf,
    /// Base directory for slot dirs (production: `/run/nixling-exec`).
    pub run_dir: PathBuf,
}

impl RunnerUnitPaths {
    pub fn new(exec_runner_path: impl Into<PathBuf>) -> Self {
        Self {
            exec_runner_path: exec_runner_path.into(),
            run_dir: PathBuf::from(RUN_DIR),
        }
    }
}

/// Manages the per-slot transient units that host detached runners. Async and
/// `Send + Sync + 'static` (mirrors `ProcessSpawner`); held as
/// `Arc<dyn TransientUnitManager>`. Every method is idempotent and
/// non-blocking (subprocesses run on the tokio runtime).
#[async_trait]
pub trait TransientUnitManager: Send + Sync + 'static {
    /// Start `nixling-exec-<slot>.service`. Blocks (on the runtime) until the
    /// unit job is registered, so a successful return proves the unit exists.
    /// `ceiling_sec == 0` means no `RuntimeMaxSec` (indefinite runtime).
    async fn start_transient_unit(
        &self,
        slot: u32,
        ceiling_sec: u64,
        paths: &RunnerUnitPaths,
    ) -> Result<(), UnitError>;

    /// Stop the unit for `slot` (best-effort, idempotent).
    async fn stop_unit(&self, slot: u32) -> Result<(), UnitError>;

    /// Clear a failed unit for `slot` (best-effort, idempotent).
    async fn reset_failed(&self, slot: u32) -> Result<(), UnitError>;

    /// Enumerate the managed `nixling-exec-*` units currently known to systemd.
    async fn list_managed_units(&self) -> Result<Vec<ManagedUnit>, UnitError>;
}

/// Production `TransientUnitManager` shelling out to `systemd-run`/`systemctl`
/// as non-blocking subprocesses.
#[derive(Debug, Clone)]
pub struct SystemdRunUnitManager {
    systemd_run_path: PathBuf,
    systemctl_path: PathBuf,
}

impl SystemdRunUnitManager {
    /// `systemctl` is derived from the directory holding `systemd-run` (both
    /// ship in the systemd package's `bin/`).
    pub fn new(systemd_run_path: impl Into<PathBuf>) -> Self {
        let systemd_run_path = systemd_run_path.into();
        let systemctl_path = systemd_run_path
            .parent()
            .map(|dir| dir.join("systemctl"))
            .unwrap_or_else(|| PathBuf::from("systemctl"));
        Self {
            systemd_run_path,
            systemctl_path,
        }
    }

    pub fn systemd_run_path(&self) -> &PathBuf {
        &self.systemd_run_path
    }

    pub fn systemctl_path(&self) -> &PathBuf {
        &self.systemctl_path
    }
}

#[async_trait]
impl TransientUnitManager for SystemdRunUnitManager {
    async fn start_transient_unit(
        &self,
        _slot: u32,
        _ceiling_sec: u64,
        _paths: &RunnerUnitPaths,
    ) -> Result<(), UnitError> {
        todo!("W13 guestd: systemd-run start_transient_unit")
    }

    async fn stop_unit(&self, _slot: u32) -> Result<(), UnitError> {
        todo!("W13 guestd: systemctl stop_unit")
    }

    async fn reset_failed(&self, _slot: u32) -> Result<(), UnitError> {
        todo!("W13 guestd: systemctl reset-failed")
    }

    async fn list_managed_units(&self) -> Result<Vec<ManagedUnit>, UnitError> {
        todo!("W13 guestd: systemctl list-units")
    }
}

/// Stable unit name for a slot: `nixling-exec-<NN>.service` (zero-padded). The
/// opaque exec id never appears in the unit name (journald cardinality bound).
pub fn unit_name(slot: u32) -> String {
    format!("nixling-exec-{slot:02}.service")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_name_is_slot_keyed_and_id_free() {
        assert_eq!(unit_name(0), "nixling-exec-00.service");
        assert_eq!(unit_name(31), "nixling-exec-31.service");
    }

    #[test]
    fn systemctl_is_derived_next_to_systemd_run() {
        let mgr = SystemdRunUnitManager::new("/run/current-system/sw/bin/systemd-run");
        assert_eq!(
            mgr.systemctl_path(),
            &PathBuf::from("/run/current-system/sw/bin/systemctl")
        );
    }
}
