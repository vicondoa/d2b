//! The detached-exec registry: slot allocation, exact log-quota accounting,
//! the transactional creation state machine, re-adoption, TTL/GC, live
//! reconciliation of vanished units, and the two-phase cancel.
//!
//! The registry is the guestd-side owner of every detached exec. It is generic
//! over a fakeable [`SlotStore`] (the on-disk slot protocol), a
//! [`TransientUnitManager`](crate::detached::TransientUnitManager) (systemd-run
//! units), a [`WallClock`], and a [`Sleeper`] so the entire lifecycle matrix is
//! unit-tested deterministically without spawning real processes or units.
//!
//! Detached records are visible to ANY same-VM connection (cross-connection
//! access is allowed, unlike attached execs) bounded to the current boot id; a
//! boot mismatch is [`ExecError::StaleSession`]. The opaque exec id never
//! appears in a unit name, argv, or journald metadata — units are
//! `d2b-exec-<NN>.service` keyed only by slot.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::detached::{
    ManagedUnit, ManagedUnitKind, RunnerUnitPaths, TransientUnitManager, UnitIdentity,
    exec_start_raw_fields, parse_exec_start, unit_name, workload_unit_name,
};
use crate::exec::{
    ExecError, ExecIdSource, ExecSnapshot, ExecState, ExitOutcome, Stream as RtStream,
    TtyStdinSnapshot, ValidatedCommand,
};

use d2b_exec_runner::filering::{FileRingError, RingChunk, StreamMeta};
use d2b_exec_runner::paths::{RunnerPaths, Stream as RunnerStream};
use d2b_exec_runner::record::{DurableRecord, RecordState, StatusPhase};
use d2b_exec_runner::spec::ExecSpec;
use d2b_exec_runner::{
    DETACHED_ACTIVE_PER_VM, DETACHED_LOG_QUOTA_BYTES, DETACHED_RETAINED_PER_VM,
    DETACHED_STREAM_LOG_BYTES, RunnerEnv,
};

/// Max wait for the runner's first phase marker before create resolves via a
/// unit re-query (see the creation state machine).
pub const CREATE_TIMEOUT_MS: u64 = 10_000;
/// A crash-recovered no-unit `Dispatching` record is held in-flight until this
/// deadline elapses with a negative re-query (covers a `systemd-run` helper
/// that registers the unit after guestd died).
pub const DISPATCH_DEADLINE_MS: u64 = 30_000;
/// Terminal-record retention before GC. NEVER applies to a Running record.
pub const RETENTION_TTL_MS: u64 = 30 * 60 * 1_000;
/// Control-watcher / status-file poll cadence.
pub const STATUS_POLL_INTERVAL_MS: u64 = 100;
/// Bounded wait for a terminal status after writing the cancel sentinel, before
/// the `stop_unit` backstop. Strictly larger than the unit `TimeoutStopSec`.
pub const CANCEL_DEADLINE_MS: u64 = 15_000;
/// systemd unit `TimeoutStopSec` (covers control-poll + child grace + reap +
/// status fsync + margin). The guestd cancel deadline is strictly larger.
pub const TIMEOUT_STOP_SEC: u64 = 10;
/// Extra seconds added on top of `ceiling_sec + TIMEOUT_STOP_SEC` when emitting
/// the optional systemd `RuntimeMaxSec` backstop, so the unit-level
/// `RuntimeMaxSec` SIGTERM only fires well AFTER the runner's own control
/// watcher has run its TERM->grace->KILL->reap->`cancelled`-status path. The
/// runner has no signal handler, so a too-early `RuntimeMaxSec` SIGTERM would
/// kill it before it could publish `cancelled`.
pub const RUNTIME_MAX_MARGIN_SEC: u64 = 5;

/// The argv hash plus the resolved per-exec log/runtime caps for one create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedCaps {
    pub stdout_log_cap: u64,
    pub stderr_log_cap: u64,
    pub max_runtime_sec: u64,
}

impl DetachedCaps {
    /// The standard per-VM caps (both streams `DETACHED_STREAM_LOG_BYTES`).
    pub fn standard(max_runtime_sec: u64) -> Self {
        Self {
            stdout_log_cap: DETACHED_STREAM_LOG_BYTES,
            stderr_log_cap: DETACHED_STREAM_LOG_BYTES,
            max_runtime_sec,
        }
    }

    fn reserved_bytes(&self) -> u64 {
        self.stdout_log_cap.saturating_add(self.stderr_log_cap)
    }
}

/// One detached record exposed by `ExecList` (never raw argv/env/cwd/output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecListEntryData {
    pub exec_id: String,
    pub slot: u32,
    pub state: ExecState,
    pub create_time_unix: u64,
    pub argv_sha256: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub dropped_bytes: u64,
}

/// The on-disk slot protocol, abstracted for deterministic testing. Production
/// uses dir-fd `openat`/`O_NOFOLLOW` rooted at `/run/d2b-exec`.
pub trait SlotStore: Send + Sync + 'static {
    /// Create `/run/d2b-exec/slot-<NN>` (root-owned 0700) after validating
    /// the root-owned 0700 parent. Idempotent.
    fn prepare_slot_dir(&self, slot: u32) -> Result<(), ExecError>;
    /// Atomically write+fsync the durable record.
    fn write_record(&self, slot: u32, record: &DurableRecord) -> Result<(), ExecError>;
    /// Read the durable record (authenticity-validated).
    fn read_record(&self, slot: u32) -> Result<DurableRecord, ExecError>;
    /// Atomically write+fsync the runner spec.
    fn write_spec(&self, slot: u32, spec: &ExecSpec) -> Result<(), ExecError>;
    /// Read the runner spec back for re-adoption identity checks.
    fn read_spec(&self, slot: u32) -> Result<ExecSpec, ExecError>;
    /// Atomically write+fsync the cancel sentinel.
    fn write_cancel(&self, slot: u32) -> Result<(), ExecError>;
    /// Read the runner status phase (`Ok(None)` if no status yet).
    fn read_status(&self, slot: u32) -> Result<Option<StatusPhase>, ExecError>;
    /// Read a stream's sidecar metadata (`Ok(None)` if the file is absent).
    fn read_log_meta(
        &self,
        slot: u32,
        stream: RunnerStream,
    ) -> Result<Option<StreamMeta>, ExecError>;
    /// Read a bounded log chunk.
    fn read_log(
        &self,
        slot: u32,
        stream: RunnerStream,
        offset: u64,
        max_len: u64,
    ) -> Result<RingChunk, FileRingError>;
    /// Mark both streams lost (vanished unit, no clean EOF). Idempotent.
    fn mark_lost(&self, slot: u32) -> Result<(), ExecError>;
    /// Remove the slot directory (respecting that no read is in flight is the
    /// caller's responsibility).
    fn delete_slot_dir(&self, slot: u32) -> Result<(), ExecError>;
    /// Remove any stale per-slot runner files (status/cancel/log data+sidecars)
    /// left by a prior occupant before a slot is reused, so a reused slot never
    /// inherits another exec's status or captured output. Idempotent.
    fn scrub_slot_files(&self, slot: u32) -> Result<(), ExecError>;
    /// Re-adoption authenticity gate: every present slot dir + file must be
    /// reached via `openat`/`O_NOFOLLOW` and be root-owned with the expected
    /// type/mode (dirs 0700; files regular 0600 with link-count 1). Returns
    /// `Err` (→ quarantine) on any deviation. Absent files are permitted.
    fn validate_authenticity(&self, slot: u32) -> Result<(), ExecError>;
    /// Enumerate present slot directories (re-adoption input).
    fn list_slot_dirs(&self) -> Result<Vec<u32>, ExecError>;
}

/// Monotonic-ish wall clock in milliseconds (fakeable).
pub trait WallClock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

/// Async sleep (fakeable; the fake advances a paired clock for determinism).
#[async_trait]
pub trait Sleeper: Send + Sync + 'static {
    async fn sleep_ms(&self, ms: u64);
}

/// Production wall clock.
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Production sleeper backed by the tokio timer.
pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep_ms(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

/// In-memory per-slot bookkeeping. The durable copy lives in the `record` file.
#[derive(Debug, Clone)]
struct SlotEntry {
    record: DurableRecord,
    caps: DetachedCaps,
    /// In-flight live-create guard: invisible to ExecList/reaper until resolved.
    creating: bool,
    /// Crash-recovered `Dispatching` hold within the dispatch deadline. Like
    /// `creating` it hides the entry from ExecList/find_by_id, but UNLIKE
    /// `creating` the periodic reaper actively resolves it after the deadline
    /// (re-query → adopt a late unit, or delete + release). Tracked separately
    /// so a held dispatch is never skipped forever the way a `creating` guard
    /// is.
    dispatch_hold: bool,
    /// Bumped on every observable state transition.
    generation: u64,
    /// Active-concurrency counter still held for this record.
    active_counted: bool,
    /// In-flight `ExecLogs` reads (GC defers unlink while > 0).
    read_guards: u32,
}

impl SlotEntry {
    fn is_terminal(&self) -> bool {
        self.record.state.is_terminal()
    }

    /// Hidden from `ExecList`/`find_by_id`: an in-flight live create OR a
    /// crash-recovered dispatch hold. The reaper still resolves dispatch holds.
    fn hidden(&self) -> bool {
        self.creating || self.dispatch_hold
    }
}

/// Per-slot unit liveness resolved against systemd. A query error is its
/// own variant — it is NEVER collapsed into `Absent`, so a transient
/// `systemctl` failure cannot trigger destructive reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotLiveness {
    /// Present, active/activating, AND identity-verified (`Slice` +
    /// `ExecStart` match the expected runner for this slot, and the workload
    /// unit is also active in the dedicated slice).
    Live,
    /// The workload unit is live but the root runner unit is gone. It must be
    /// cleaned up before the record is marked lost.
    OrphanWorkload,
    /// Present but not adoptable as our live runner: inactive/failed, or the
    /// `Slice`/`ExecStart` identity does not match (an impostor at our slot).
    Foreign,
    /// No unit present for this slot.
    Absent,
    /// The liveness query itself failed; liveness is unknown. Callers MUST skip
    /// destructive reconciliation and retry later.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitClass {
    Live,
    Foreign,
    Unknown,
}

struct WorkloadIdentity<'a> {
    slice: Option<&'a str>,
    exec_start: Option<&'a str>,
    binds_to: Option<&'a str>,
    part_of: Option<&'a str>,
    after: Option<&'a str>,
}

/// How a durable record is re-adopted on startup.
enum AdoptKind {
    /// A terminal status was present: adopt as the terminal state.
    Terminal(RecordState, Option<i32>, Option<u32>),
    /// A live (or conservatively-assumed-live) record: adopt as `Running`.
    Running,
    /// A crash-recovered `Dispatching` record still within its dispatch
    /// deadline: hold (reserved, non-listable) until the reaper resolves it.
    DispatchHold,
}

#[derive(Default)]
struct RegistryState {
    /// slot -> entry.
    slots: BTreeMap<u32, SlotEntry>,
    /// Active (Dispatching/Running) detached execs.
    active: u32,
    /// Sum of reserved log bytes across live records.
    reserved_log_bytes: u64,
    /// Bounded slot+id tombstones so a GC-evicted lookup returns `ExecExpired`.
    tombstones: VecDeque<String>,
}

impl RegistryState {
    fn free_slot(&self) -> Option<u32> {
        (0..DETACHED_RETAINED_PER_VM as u32).find(|slot| !self.slots.contains_key(slot))
    }

    fn find_by_id(&self, exec_id: &str) -> Option<u32> {
        self.slots
            .iter()
            .find(|(_, entry)| !entry.hidden() && entry.record.exec_id == exec_id)
            .map(|(slot, _)| *slot)
    }

    fn find_any_by_id(&self, exec_id: &str) -> Option<u32> {
        self.slots
            .iter()
            .find(|(_, entry)| entry.record.exec_id == exec_id)
            .map(|(slot, _)| *slot)
    }

    fn push_tombstone(&mut self, exec_id: String) {
        if self.tombstones.len() >= DETACHED_RETAINED_PER_VM {
            self.tombstones.pop_front();
        }
        self.tombstones.push_back(exec_id);
    }

    fn is_tombstoned(&self, exec_id: &str) -> bool {
        self.tombstones.iter().any(|id| id == exec_id)
    }

    /// Release the active counter exactly once for an entry that just became
    /// terminal.
    fn release_active(&mut self, slot: u32) {
        if let Some(entry) = self.slots.get_mut(&slot)
            && entry.active_counted
        {
            entry.active_counted = false;
            self.active = self.active.saturating_sub(1);
        }
    }

    /// Drop an entry entirely (frees slot + quota + active).
    fn remove_entry(&mut self, slot: u32) {
        if let Some(entry) = self.slots.remove(&slot) {
            if entry.active_counted {
                self.active = self.active.saturating_sub(1);
            }
            self.reserved_log_bytes = self
                .reserved_log_bytes
                .saturating_sub(entry.caps.reserved_bytes());
        }
    }
}

/// Configuration for the detached registry.
#[derive(Clone)]
pub struct RegistryConfig {
    pub paths: RunnerUnitPaths,
    pub boot_id: String,
    /// Default per-exec runtime ceiling in seconds (0 = unlimited).
    pub max_runtime_sec: u64,
    /// Host-fixed workload user, resolved non-root before the registry is built.
    pub exec_user: String,
    /// Resolved non-root UID for `exec_user`; stored so the root runner can
    /// re-resolve and compare immediately before spawning the workload unit.
    pub exec_uid: u32,
    /// Absolute path to `systemd-run`, copied into each runner spec.
    pub systemd_run_path: String,
    /// Absolute path to the login shell used by the workload unit wrapper.
    pub login_shell_path: String,
}

/// The detached-exec registry.
pub struct DetachedRegistry {
    state: Mutex<RegistryState>,
    units: Arc<dyn TransientUnitManager>,
    store: Arc<dyn SlotStore>,
    clock: Arc<dyn WallClock>,
    sleeper: Arc<dyn Sleeper>,
    ids: Arc<dyn ExecIdSource>,
    config: RegistryConfig,
}

impl DetachedRegistry {
    pub fn new(
        units: Arc<dyn TransientUnitManager>,
        store: Arc<dyn SlotStore>,
        clock: Arc<dyn WallClock>,
        sleeper: Arc<dyn Sleeper>,
        ids: Arc<dyn ExecIdSource>,
        config: RegistryConfig,
    ) -> Self {
        Self {
            state: Mutex::new(RegistryState::default()),
            units,
            store,
            clock,
            sleeper,
            ids,
            config,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        self.state.lock().expect("detached registry poisoned")
    }

    /// The default per-exec caps (both streams `DETACHED_STREAM_LOG_BYTES`,
    /// runtime ceiling from the host config; 0 = unlimited).
    pub fn default_caps(&self) -> DetachedCaps {
        DetachedCaps::standard(self.config.max_runtime_sec)
    }

    /// Quota invariant: `quota == slots * 2 * stream_cap`. Asserted at runtime
    /// so a config drift can never make over-budget reservation possible.
    fn assert_quota_invariant() {
        debug_assert_eq!(
            DETACHED_LOG_QUOTA_BYTES,
            DETACHED_RETAINED_PER_VM as u64 * 2 * DETACHED_STREAM_LOG_BYTES
        );
    }

    // ---- creation state machine -----------------------------------------

    /// Create a detached exec. Returns the opaque id + the initial snapshot.
    pub async fn create(
        &self,
        boot_id: &str,
        command: ValidatedCommand,
        caps: DetachedCaps,
    ) -> Result<(String, ExecSnapshot), ExecError> {
        self.create_inner(boot_id, command, caps, None).await
    }

    pub async fn create_with_exec_id(
        &self,
        boot_id: &str,
        command: ValidatedCommand,
        caps: DetachedCaps,
        exec_id: String,
    ) -> Result<(String, ExecSnapshot), ExecError> {
        self.create_inner(boot_id, command, caps, Some(exec_id))
            .await
    }

    async fn create_inner(
        &self,
        boot_id: &str,
        command: ValidatedCommand,
        caps: DetachedCaps,
        requested_exec_id: Option<String>,
    ) -> Result<(String, ExecSnapshot), ExecError> {
        Self::assert_quota_invariant();
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }

        let argv_sha256 = argv_hash(&command);
        let has_requested_exec_id = requested_exec_id.is_some();
        let exec_id = if let Some(exec_id) = requested_exec_id {
            if !is_valid_exec_id(&exec_id) {
                return Err(ExecError::InvalidArgv);
            }
            let existing = {
                let state = self.lock();
                if state.is_tombstoned(&exec_id) {
                    return Err(ExecError::ExecExpired);
                }
                state.find_any_by_id(&exec_id).map(|slot| {
                    let entry = state.slots.get(&slot).expect("slot found by id");
                    (slot, entry.record.argv_sha256.clone(), entry.hidden())
                })
            };
            if let Some((_slot, existing_hash, hidden)) = existing {
                if existing_hash != argv_sha256 {
                    return Err(ExecError::InvalidArgv);
                }
                if hidden {
                    return Err(ExecError::ExecClosing);
                }
                let snapshot = self.inspect(&exec_id, boot_id).await?;
                return Ok((exec_id, snapshot));
            }
            exec_id
        } else {
            self.ids.next_exec_id()?
        };
        let now = self.clock.now_ms();

        // Step 1: reserve slot + active + quota under the Creating guard.
        let slot = {
            let mut state = self.lock();
            if has_requested_exec_id {
                if state.is_tombstoned(&exec_id) {
                    return Err(ExecError::ExecExpired);
                }
                if let Some(existing_slot) = state.find_any_by_id(&exec_id) {
                    let existing = state
                        .slots
                        .get(&existing_slot)
                        .expect("slot found by requested exec id");
                    return if existing.record.argv_sha256 == argv_sha256 {
                        Err(ExecError::ExecClosing)
                    } else {
                        Err(ExecError::InvalidArgv)
                    };
                }
            }
            if state.active >= DETACHED_ACTIVE_PER_VM as u32 {
                return Err(ExecError::ExecCapacityExceeded);
            }
            let Some(slot) = state.free_slot() else {
                return Err(ExecError::ExecCapacityExceeded);
            };
            let reserve = caps.reserved_bytes();
            if state.reserved_log_bytes.saturating_add(reserve) > DETACHED_LOG_QUOTA_BYTES {
                return Err(ExecError::RetainedLogQuotaExceeded);
            }
            let record = DurableRecord {
                exec_id: exec_id.clone(),
                slot,
                boot_id: self.config.boot_id.clone(),
                create_time_unix: now,
                dispatch_deadline_unix: now.saturating_add(DISPATCH_DEADLINE_MS),
                argv_sha256: argv_sha256.clone(),
                state: RecordState::Dispatching,
                exit_code: None,
                term_signal: None,
                lost: false,
                terminal_time_unix: None,
            };
            state.reserved_log_bytes = state.reserved_log_bytes.saturating_add(reserve);
            state.active = state.active.saturating_add(1);
            state.slots.insert(
                slot,
                SlotEntry {
                    record,
                    caps: caps.clone(),
                    creating: true,
                    dispatch_hold: false,
                    generation: 0,
                    active_counted: true,
                    read_guards: 0,
                },
            );
            slot
        };

        let spec = match build_spec(&command, &caps, &self.config, slot) {
            Ok(spec) => spec,
            Err(error) => {
                self.abort_create(slot).await;
                return Err(error);
            }
        };

        // Step 2: write+fsync record (Dispatching) + spec BEFORE systemd-run.
        if let Err(error) = self.persist_dispatch(slot, &spec) {
            self.abort_create(slot).await;
            return Err(error);
        }

        // Step 4: start the transient unit (blocks until the job is registered).
        if self
            .units
            .start_transient_unit(slot, caps.max_runtime_sec, &self.config.paths)
            .await
            .is_err()
        {
            self.abort_create(slot).await;
            return Err(ExecError::Internal);
        }

        // Step 5: await the runner's first phase marker, bounded by CREATE_TIMEOUT.
        self.await_create_resolution(slot, &exec_id).await
    }

    fn persist_dispatch(&self, slot: u32, spec: &ExecSpec) -> Result<(), ExecError> {
        self.store.prepare_slot_dir(slot)?;
        // A reused slot must never inherit a prior occupant's status,
        // cancel sentinel, or captured-output files (e.g. after a partial
        // delete_slot_dir). Scrub before writing the new record/spec.
        self.store.scrub_slot_files(slot)?;
        let record = {
            let state = self.lock();
            state
                .slots
                .get(&slot)
                .map(|entry| entry.record.clone())
                .ok_or(ExecError::Internal)?
        };
        self.store.write_record(slot, &record)?;
        self.store.write_spec(slot, spec)?;
        Ok(())
    }

    /// Tear down a failed in-flight create: stop the unit (best-effort), delete
    /// the slot dir, release the reservation, and drop the guard so no id was
    /// ever externally visible.
    async fn abort_create(&self, slot: u32) {
        let _ = self.units.stop_unit(slot).await;
        let _ = self.units.reset_failed(slot).await;
        let _ = self.store.delete_slot_dir(slot);
        let mut state = self.lock();
        state.remove_entry(slot);
    }

    async fn await_create_resolution(
        &self,
        slot: u32,
        exec_id: &str,
    ) -> Result<(String, ExecSnapshot), ExecError> {
        let start = self.clock.now_ms();
        loop {
            // A read_status IO/decode error must NOT propagate while the
            // slot is still reserved under the Creating guard — that would leak
            // the active/quota reservation and the reaper would skip it forever
            // (creating). Tear the create down first, then surface the error.
            let status = match self.store.read_status(slot) {
                Ok(status) => status,
                Err(err) => {
                    self.abort_create(slot).await;
                    return Err(err);
                }
            };
            match status {
                Some(StatusPhase::Started) => {
                    self.commit_running(slot);
                    return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                }
                Some(StatusPhase::Exited(code)) => {
                    self.commit_terminal(slot, RecordState::Exited, Some(code), None);
                    return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                }
                Some(StatusPhase::Signaled(signal)) => {
                    self.commit_terminal(slot, RecordState::Signaled, None, Some(signal as u32));
                    return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                }
                Some(StatusPhase::Cancelled) => {
                    self.commit_terminal(slot, RecordState::Cancelled, None, None);
                    return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                }
                Some(StatusPhase::SpawnFailed) => {
                    self.commit_terminal(slot, RecordState::SpawnFailed, None, None);
                    return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                }
                Some(StatusPhase::InfraFailed) => {
                    // The runner spawned the workload but its post-spawn safety
                    // verification failed (slice placement, the never-root
                    // re-guard, or a unit-setup fault) and refused to publish
                    // `Started`. The operator-facing wire error is intentionally
                    // coarse and carries no argv, so emit a guest-journal
                    // diagnostic naming the failure class and what to inspect.
                    eprintln!(
                        "d2b-guestd: detached exec create aborted (slot {slot:02}): the exec \
                         runner reported an infrastructure failure — post-spawn verification \
                         failed. Inspect the d2b-exec.slice placement and status of \
                         d2b-exec-{slot:02}.service and d2b-exec-{slot:02}-w.service in \
                         the guest journal."
                    );
                    self.abort_create(slot).await;
                    return Err(ExecError::RetainedLogPathUnsafe);
                }
                None => {
                    if self.clock.now_ms().saturating_sub(start) >= CREATE_TIMEOUT_MS {
                        // Re-query the unit. A verified-live unit — or an
                        // UNKNOWN query result — commits Running: we must never
                        // kill a job whose unit might be live just because the
                        // status marker has not landed yet. Only a definitive
                        // Absent/Foreign result fails the create.
                        match self.unit_liveness(slot).await {
                            SlotLiveness::Live | SlotLiveness::Unknown => {
                                self.commit_running(slot);
                                return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                            }
                            SlotLiveness::Absent
                            | SlotLiveness::Foreign
                            | SlotLiveness::OrphanWorkload => {
                                self.abort_create(slot).await;
                                return Err(ExecError::Internal);
                            }
                        }
                    }
                    self.sleeper.sleep_ms(STATUS_POLL_INTERVAL_MS).await;
                }
            }
        }
    }

    fn commit_running(&self, slot: u32) {
        let mut state = self.lock();
        if let Some(entry) = state.slots.get_mut(&slot) {
            if entry.record.state == RecordState::Dispatching {
                entry.record.state = RecordState::Running;
                entry.generation += 1;
            }
            entry.creating = false;
            entry.dispatch_hold = false;
        }
        let record = state.slots.get(&slot).map(|e| e.record.clone());
        drop(state);
        if let Some(record) = record {
            let _ = self.store.write_record(slot, &record);
        }
    }

    fn commit_terminal(
        &self,
        slot: u32,
        terminal: RecordState,
        exit_code: Option<i32>,
        term_signal: Option<u32>,
    ) {
        let now = self.clock.now_ms();
        let record = {
            let mut state = self.lock();
            let Some(entry) = state.slots.get_mut(&slot) else {
                return;
            };
            if !entry.is_terminal() {
                entry.record.state = terminal;
                entry.record.exit_code = exit_code;
                entry.record.term_signal = term_signal;
                entry.record.terminal_time_unix = Some(now);
                entry.generation += 1;
            }
            entry.creating = false;
            entry.dispatch_hold = false;
            let record = entry.record.clone();
            state.release_active(slot);
            record
        };
        let _ = self.store.write_record(slot, &record);
    }

    /// Resolve one slot's unit liveness against systemd. A query error
    /// becomes [`SlotLiveness::Unknown`] — never `Absent` — so a transient
    /// `systemctl` failure cannot drive destructive reconciliation.
    async fn unit_liveness(&self, slot: u32) -> SlotLiveness {
        match self.units.list_managed_units().await {
            Ok(units) => self.classify_unit(&units, slot),
            Err(_) => SlotLiveness::Unknown,
        }
    }

    /// Classify a slot against an already-fetched unit list.
    fn classify_unit(&self, units: &[ManagedUnit], slot: u32) -> SlotLiveness {
        let runner = units
            .iter()
            .find(|u| u.slot == slot && u.kind == ManagedUnitKind::Runner);
        let workload = units
            .iter()
            .find(|u| u.slot == slot && u.kind == ManagedUnitKind::Workload);

        let runner_state = runner.map(|unit| self.classify_runner_unit(unit, slot));
        let workload_state = workload.map(|unit| self.classify_workload_unit(unit, slot));

        if matches!(runner_state, Some(UnitClass::Foreign))
            || matches!(workload_state, Some(UnitClass::Foreign))
        {
            return SlotLiveness::Foreign;
        }
        if matches!(runner_state, Some(UnitClass::Unknown))
            || matches!(workload_state, Some(UnitClass::Unknown))
        {
            return SlotLiveness::Unknown;
        }
        match (runner_state, workload_state) {
            (Some(UnitClass::Live), Some(UnitClass::Live)) => SlotLiveness::Live,
            (Some(UnitClass::Live), None) => SlotLiveness::Unknown,
            (None, Some(UnitClass::Live)) => SlotLiveness::OrphanWorkload,
            (None, None) => SlotLiveness::Absent,
            _ => SlotLiveness::Foreign,
        }
    }

    fn classify_runner_unit(&self, unit: &ManagedUnit, slot: u32) -> UnitClass {
        if !unit.active {
            return UnitClass::Foreign;
        }
        match &unit.identity {
            UnitIdentity::Unqueried => UnitClass::Unknown,
            UnitIdentity::Queried {
                slice, exec_start, ..
            } => {
                if self.identity_matches(slice.as_deref(), exec_start.as_deref(), slot) {
                    UnitClass::Live
                } else {
                    UnitClass::Foreign
                }
            }
        }
    }

    fn classify_workload_unit(&self, unit: &ManagedUnit, slot: u32) -> UnitClass {
        if !unit.active {
            return UnitClass::Foreign;
        }
        match &unit.identity {
            UnitIdentity::Unqueried => UnitClass::Unknown,
            UnitIdentity::Queried {
                slice,
                exec_start,
                binds_to,
                part_of,
                after,
            } => {
                let Ok(spec) = self.store.read_spec(slot) else {
                    return UnitClass::Foreign;
                };
                let identity = WorkloadIdentity {
                    slice: slice.as_deref(),
                    exec_start: exec_start.as_deref(),
                    binds_to: binds_to.as_deref(),
                    part_of: part_of.as_deref(),
                    after: after.as_deref(),
                };
                if self.workload_identity_matches(identity, slot, &spec) {
                    UnitClass::Live
                } else {
                    UnitClass::Foreign
                }
            }
        }
    }

    /// Verify a QUERIED unit identity really is THIS slot's runner. The check
    /// is STRUCTURAL, never substring-based: the unit must live in the
    /// dedicated `d2b-exec.slice`, the resolved `ExecStart` executable path
    /// must EQUAL the configured runner abs path, and the argv token sequence
    /// must contain `--serve-exec` and an adjacent `--slot <NN>` (this slot's
    /// zero-padded NN) as DISTINCT argv tokens. An impostor that merely embeds
    /// those strings inside an unrelated argument, or runs a different
    /// executable / a different slot, is rejected.
    fn identity_matches(&self, slice: Option<&str>, exec_start: Option<&str>, slot: u32) -> bool {
        if slice != Some("d2b-exec.slice") {
            return false;
        }
        let Some(exec_start) = exec_start else {
            return false;
        };
        let Some(parsed) = parse_exec_start(exec_start) else {
            return false;
        };
        let runner = self.config.paths.exec_runner_path.to_string_lossy();
        // Exact executable-path equality, not a substring containment.
        if parsed.exe != runner.as_ref() {
            return false;
        }
        // `--serve-exec` must be a standalone argv token.
        let has_serve_exec = parsed.argv.iter().any(|t| t == "--serve-exec");
        // `--slot` must be immediately followed by THIS slot's zero-padded NN
        // as a distinct token (not a substring of an unrelated argument).
        let slot_token = format!("{slot:02}");
        let has_slot = parsed
            .argv
            .windows(2)
            .any(|w| w[0] == "--slot" && w[1] == slot_token);
        has_serve_exec && has_slot
    }

    fn workload_identity_matches(
        &self,
        identity: WorkloadIdentity<'_>,
        slot: u32,
        spec: &ExecSpec,
    ) -> bool {
        // The workload's TRUST identity is structural and systemd-/root-enforced:
        // membership in `d2b-exec.slice` plus `BindsTo`/`PartOf`/`After`
        // pinned to THIS slot's runner unit, both checked below. Only the
        // framework (running as root in the guest) creates `d2b-exec-*` units
        // with those properties, so a non-root caller cannot place an impostor at
        // our slot. The `argv[]` comparison that follows is a best-effort
        // CONSISTENCY check (does the live command match the persisted spec?)
        // recovered from systemd's lossy single-line `systemctl show` rendering;
        // it is hardened against the realistic argv bytes our own well-formed
        // units carry, but it is not — and need not be — a defense against a
        // guest-root attacker crafting a multi-line/forged ExecStart, which is a
        // total compromise the structural boundary cannot help with either.
        if identity.slice != Some("d2b-exec.slice") {
            return false;
        }
        let runner = unit_name(slot);
        if !property_contains_unit(identity.binds_to, &runner)
            || !property_contains_unit(identity.part_of, &runner)
            || !property_contains_unit(identity.after, &runner)
        {
            return false;
        }
        let Some(exec_start) = identity.exec_start else {
            return false;
        };
        // Match against the RAW `path=`/`argv[]=` fields, never the quote-aware
        // token parser: `systemctl show` renders argv[] losslessly space-joined
        // and UNescaped, and `validate_command` permits argv bytes the parser
        // would reject (literal `"`, trailing `\`) or fail-OPEN truncate at (a
        // user `;`). Render the expected argv the same lossy way and compare the
        // raw strings byte-for-byte.
        let Some((raw_path, raw_argv)) = exec_start_raw_fields(exec_start) else {
            return false;
        };
        if raw_path != spec.login_shell_path {
            return false;
        }
        let mut expected = Vec::with_capacity(5 + spec.argv.len());
        expected.push(spec.login_shell_path.clone());
        expected.push("-l".to_owned());
        expected.push("-c".to_owned());
        expected.push(r#"exec "$@""#.to_owned());
        expected.push("d2b-exec".to_owned());
        expected.extend(spec.argv.iter().cloned());
        raw_argv == expected.join(" ")
    }

    // ---- read-side ops ---------------------------------------------------

    /// Test-only accessor for the active-concurrency counter
    /// (`DETACHED_ACTIVE_PER_VM` reservation). Lets reconciliation tests assert
    /// the counter is released precisely (not merely that a record is retained).
    #[cfg(test)]
    pub(crate) fn active_count(&self) -> u32 {
        self.lock().active
    }

    pub async fn inspect(&self, exec_id: &str, boot_id: &str) -> Result<ExecSnapshot, ExecError> {
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }
        let slot = self.resolve_slot(exec_id)?;
        self.reconcile_slot(slot).await;
        self.snapshot_for(slot)
    }

    pub async fn wait(
        &self,
        exec_id: &str,
        boot_id: &str,
        known_generation: Option<u64>,
        timeout_ms: u64,
    ) -> Result<(ExecSnapshot, bool), ExecError> {
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }
        let slot = self.resolve_slot(exec_id)?;
        let start = self.clock.now_ms();
        loop {
            self.reconcile_slot(slot).await;
            let snapshot = self.snapshot_for(slot)?;
            let terminal = !matches!(snapshot.state, ExecState::Running);
            let generation_changed = known_generation
                .map(|known| snapshot.state_generation != known)
                .unwrap_or(false);
            if terminal || generation_changed {
                return Ok((snapshot, false));
            }
            if self.clock.now_ms().saturating_sub(start) >= timeout_ms {
                return Ok((snapshot, true));
            }
            self.sleeper.sleep_ms(STATUS_POLL_INTERVAL_MS).await;
        }
    }

    pub async fn read_logs(
        &self,
        exec_id: &str,
        boot_id: &str,
        stream: RtStream,
        offset: u64,
        max_len: u64,
    ) -> Result<RingChunk, ExecError> {
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }
        let slot = self.resolve_slot(exec_id)?;
        // Take a read guard so GC defers unlink for the read's duration.
        {
            let mut state = self.lock();
            let Some(entry) = state.slots.get_mut(&slot) else {
                // The entry was GC'd between resolve and lock. Compute the kind
                // from the already-held state — never re-lock the mutex here
                // (missing_kind() would deadlock on the non-reentrant std
                // Mutex).
                return Err(if state.is_tombstoned(exec_id) {
                    ExecError::ExecExpired
                } else {
                    ExecError::ExecNotFound
                });
            };
            entry.read_guards += 1;
        }
        let result = self
            .store
            .read_log(slot, runner_stream(stream), offset, max_len)
            .map_err(map_ring_error);
        {
            let mut state = self.lock();
            if let Some(entry) = state.slots.get_mut(&slot) {
                entry.read_guards = entry.read_guards.saturating_sub(1);
            }
        }
        result
    }

    pub async fn list(&self, boot_id: &str) -> Result<Vec<ExecListEntryData>, ExecError> {
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }
        // Reconcile every live record so a vanished unit lists as Cancelled/lost.
        let slots: Vec<u32> = {
            let state = self.lock();
            state
                .slots
                .iter()
                .filter(|(_, entry)| !entry.hidden())
                .map(|(slot, _)| *slot)
                .collect()
        };
        for slot in &slots {
            self.reconcile_slot(*slot).await;
        }

        let entries: Vec<(u32, DurableRecord, u64)> = {
            let state = self.lock();
            state
                .slots
                .iter()
                .filter(|(_, entry)| !entry.hidden())
                .map(|(slot, entry)| (*slot, entry.record.clone(), entry.generation))
                .collect()
        };
        let mut out = Vec::with_capacity(entries.len());
        for (slot, record, _generation) in entries {
            let (stdout_meta, stderr_meta) = self.stream_metas(slot);
            out.push(ExecListEntryData {
                exec_id: record.exec_id.clone(),
                slot,
                state: public_state(&record),
                create_time_unix: record.create_time_unix,
                argv_sha256: record.argv_sha256.clone(),
                stdout_truncated: stdout_meta.map(|m| m.truncated || m.lost).unwrap_or(false),
                stderr_truncated: stderr_meta.map(|m| m.truncated || m.lost).unwrap_or(false),
                dropped_bytes: stdout_meta.map(|m| m.dropped_bytes).unwrap_or(0)
                    + stderr_meta.map(|m| m.dropped_bytes).unwrap_or(0),
            });
        }
        Ok(out)
    }

    // ---- two-phase cancel ------------------------------------------------

    /// Returns `true` when the exec was already terminal (idempotent duplicate).
    pub async fn cancel(&self, exec_id: &str, boot_id: &str) -> Result<bool, ExecError> {
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }
        let slot = self.resolve_slot(exec_id)?;
        self.reconcile_slot(slot).await;
        if self.is_terminal(slot) {
            return Ok(true);
        }

        // Phase 1: write+fsync the cancel sentinel (NOT stop_unit first).
        self.store.write_cancel(slot)?;

        // Phase 2: wait (bounded) for the runner to publish a terminal status.
        let start = self.clock.now_ms();
        loop {
            self.reconcile_slot(slot).await;
            if self.is_terminal(slot) {
                return Ok(false);
            }
            if self.clock.now_ms().saturating_sub(start) >= CANCEL_DEADLINE_MS {
                break;
            }
            self.sleeper.sleep_ms(STATUS_POLL_INTERVAL_MS).await;
        }

        // Phase 3: last-resort backstop — only now stop the unit. Never
        // terminalize merely because the deadline elapsed: a stop failure with
        // fresh live/unknown liveness means the workload may still be running,
        // so leave it Running for a later cancel/reaper retry.
        let stopped = self.units.stop_unit(slot).await.is_ok();
        if stopped {
            let _ = self.units.reset_failed(slot).await;
        }
        self.reconcile_slot(slot).await;
        if !self.is_terminal(slot)
            && (stopped || matches!(self.unit_liveness(slot).await, SlotLiveness::Absent))
        {
            self.mark_lost(slot);
        }
        Ok(false)
    }

    // ---- live reconciliation + TTL/GC ------------------------------------

    /// Reconcile one live (Dispatching/Running) record against its unit/status:
    /// adopt a terminal status, or — if the unit vanished with no terminal
    /// status — mark the record `Cancelled`/lost (release only the active
    /// counter, retain slot+logs+quota until TTL/GC).
    async fn reconcile_slot(&self, slot: u32) {
        let (hidden, terminal) = {
            let state = self.lock();
            match state.slots.get(&slot) {
                Some(entry) => (entry.hidden(), entry.is_terminal()),
                None => return,
            }
        };
        if hidden || terminal {
            return;
        }
        // A published terminal status wins regardless of unit liveness.
        match self.store.read_status(slot) {
            Ok(Some(StatusPhase::Exited(code))) => {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
                self.commit_terminal(slot, RecordState::Exited, Some(code), None);
                return;
            }
            Ok(Some(StatusPhase::Signaled(signal))) => {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
                self.commit_terminal(slot, RecordState::Signaled, None, Some(signal as u32));
                return;
            }
            Ok(Some(StatusPhase::Cancelled)) => {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
                self.commit_terminal(slot, RecordState::Cancelled, None, None);
                return;
            }
            Ok(Some(StatusPhase::SpawnFailed)) => {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
                self.commit_terminal(slot, RecordState::SpawnFailed, None, None);
                return;
            }
            Ok(Some(StatusPhase::Started)) => {
                self.commit_running(slot);
            }
            Ok(Some(StatusPhase::InfraFailed)) | Ok(None) | Err(_) => {}
        }

        // No terminal status: resolve against the unit's verified liveness.
        match self.unit_liveness(slot).await {
            // Live + identity-verified: healthy, leave it running.
            SlotLiveness::Live => return,
            // Query error: liveness is UNKNOWN — never mark a maybe-live exec
            // lost on a transient systemctl failure. Retry on the next pass.
            SlotLiveness::Unknown => return,
            // The root runner disappeared while the workload unit is still
            // active. Kill the workload first; only then mark the slot lost so
            // guestd never reports a terminal state while untracked work runs.
            SlotLiveness::OrphanWorkload => {
                if self.units.stop_unit(slot).await.is_err() {
                    return;
                }
            }
            // An impostor unit sits at our slot: clean it up, then treat the
            // record as having no live unit (fall through to lost handling).
            SlotLiveness::Foreign => {
                if self.units.stop_unit(slot).await.is_err() {
                    return;
                }
                let _ = self.units.reset_failed(slot).await;
            }
            SlotLiveness::Absent => {}
        }
        // Within the dispatch deadline a not-yet-registered unit is normal.
        let within_deadline = {
            let state = self.lock();
            state
                .slots
                .get(&slot)
                .map(|entry| {
                    entry.record.state == RecordState::Dispatching
                        && self.clock.now_ms() < entry.record.dispatch_deadline_unix
                })
                .unwrap_or(false)
        };
        if within_deadline {
            return;
        }
        // Unit gone + no terminal status + past any dispatch deadline => lost.
        self.mark_lost(slot);
    }

    fn mark_lost(&self, slot: u32) {
        let now = self.clock.now_ms();
        let record = {
            let mut state = self.lock();
            let Some(entry) = state.slots.get_mut(&slot) else {
                return;
            };
            if entry.is_terminal() {
                return;
            }
            entry.record.state = RecordState::Cancelled;
            entry.record.lost = true;
            entry.record.terminal_time_unix = Some(now);
            entry.generation += 1;
            let record = entry.record.clone();
            state.release_active(slot);
            record
        };
        let _ = self.store.mark_lost(slot);
        let _ = self.store.write_record(slot, &record);
    }

    /// Periodic reaper: reconcile live records and GC terminal records past TTL.
    pub async fn reap_once(&self) {
        self.sweep_orphan_units().await;
        let slots: Vec<u32> = {
            let state = self.lock();
            state.slots.keys().copied().collect()
        };
        for slot in slots {
            let (creating, dispatch_hold, terminal, terminal_time) = {
                let state = self.lock();
                match state.slots.get(&slot) {
                    Some(entry) => (
                        entry.creating,
                        entry.dispatch_hold,
                        entry.is_terminal(),
                        entry.record.terminal_time_unix,
                    ),
                    None => continue,
                }
            };
            if creating {
                continue;
            }
            if dispatch_hold {
                // Unlike a `creating` guard, a crash-recovered dispatch hold
                // is actively resolved by the reaper after its deadline.
                self.resolve_dispatch_hold(slot).await;
                continue;
            }
            if !terminal {
                self.reconcile_slot(slot).await;
                continue;
            }
            // Terminal: GC past TTL. The read-guard recheck happens INSIDE
            // gc_slot under the mutex so a reader that took a guard after this
            // snapshot cannot race the unlink.
            let expired = terminal_time
                .map(|t| self.clock.now_ms().saturating_sub(t) >= RETENTION_TTL_MS)
                .unwrap_or(false);
            if expired {
                self.gc_slot(slot).await;
            }
        }
    }

    async fn sweep_orphan_units(&self) {
        let Ok(present_units) = self.units.list_managed_units().await else {
            return;
        };
        let adopted: BTreeSet<u32> = {
            let state = self.lock();
            state.slots.keys().copied().collect()
        };
        let mut stopped = BTreeSet::new();
        for unit in present_units {
            if adopted.contains(&unit.slot) || !stopped.insert(unit.slot) {
                continue;
            }
            let _ = self.units.stop_unit(unit.slot).await;
            let _ = self.units.reset_failed(unit.slot).await;
        }
    }

    async fn gc_slot(&self, slot: u32) {
        // Best-effort unit teardown happens outside the registry mutex (async).
        let _ = self.units.stop_unit(slot).await;
        let _ = self.units.reset_failed(slot).await;
        // Recheck read guards and unlink UNDER the mutex: a reader that took a
        // guard after the reaper's snapshot must defer the unlink.
        // delete_slot_dir is synchronous, so holding the std mutex across it
        // introduces no await and cannot deadlock.
        let mut state = self.lock();
        let Some(entry) = state.slots.get(&slot) else {
            return;
        };
        if entry.read_guards > 0 {
            // An ExecLogs read is in flight; defer GC to a later reaper pass.
            return;
        }
        match self.store.delete_slot_dir(slot) {
            Ok(()) => {
                let exec_id = state.slots.get(&slot).map(|e| e.record.exec_id.clone());
                state.remove_entry(slot);
                if let Some(exec_id) = exec_id {
                    state.push_tombstone(exec_id);
                }
            }
            Err(_) => {
                // Deletion failed — retain the entry so the slot is NOT
                // freed for reuse with stale files still on disk. A later reaper
                // pass retries the unlink.
            }
        }
    }

    /// Resolve a crash-recovered dispatch hold: a late-registered unit is
    /// promoted to Running; a definitive absence past the dispatch deadline is
    /// deleted + released; anything else (still within the deadline, or an
    /// unknown query) stays held for the next pass.
    async fn resolve_dispatch_hold(&self, slot: u32) {
        let past_deadline = {
            let state = self.lock();
            match state.slots.get(&slot) {
                Some(entry) => self.clock.now_ms() >= entry.record.dispatch_deadline_unix,
                None => return,
            }
        };
        match self.store.read_status(slot) {
            Ok(Some(StatusPhase::Started)) => {
                self.promote_dispatch_hold(slot);
                return;
            }
            Ok(Some(StatusPhase::InfraFailed)) => {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
                self.delete_dispatch_hold(slot).await;
                return;
            }
            Ok(status @ Some(_)) => {
                if let Some((terminal, code, signal)) = status_to_terminal(status) {
                    let _ = self.units.stop_unit(slot).await;
                    let _ = self.units.reset_failed(slot).await;
                    self.commit_terminal(slot, terminal, code, signal);
                    return;
                }
            }
            Ok(None) | Err(_) => {}
        }
        match self.unit_liveness(slot).await {
            SlotLiveness::Live => self.promote_dispatch_hold(slot),
            // Unknown: never resolve destructively on a query error; keep held.
            SlotLiveness::Unknown => {}
            SlotLiveness::OrphanWorkload | SlotLiveness::Foreign => {
                // Impostor unit at our slot; clean it up. Delete the hold once
                // past the deadline (the real runner never registered).
                if self.units.stop_unit(slot).await.is_err() {
                    return;
                }
                let _ = self.units.reset_failed(slot).await;
                if past_deadline {
                    self.delete_dispatch_hold(slot).await;
                }
            }
            SlotLiveness::Absent => {
                if past_deadline {
                    self.delete_dispatch_hold(slot).await;
                }
            }
        }
    }

    /// A late unit appeared for a held dispatch: clear the hold and promote it
    /// to Running (it keeps its already-counted active reservation).
    fn promote_dispatch_hold(&self, slot: u32) {
        let record = {
            let mut state = self.lock();
            let Some(entry) = state.slots.get_mut(&slot) else {
                return;
            };
            if !entry.dispatch_hold {
                return;
            }
            entry.dispatch_hold = false;
            if entry.record.state == RecordState::Dispatching {
                entry.record.state = RecordState::Running;
            }
            entry.generation += 1;
            entry.record.clone()
        };
        let _ = self.store.write_record(slot, &record);
    }

    /// A held dispatch never registered a unit within its deadline: delete the
    /// slot dir + release the reservation — but ONLY once the on-disk dir is
    /// actually gone. If the unlink fails, keep the (hidden) dispatch-hold
    /// entry so the slot is never freed for reuse with stale files on disk; a
    /// later reaper pass retries the unlink (consistent with the GC
    /// retain-on-failure path). No id was ever externally visible.
    async fn delete_dispatch_hold(&self, slot: u32) {
        if self.store.delete_slot_dir(slot).is_ok() {
            let mut state = self.lock();
            state.remove_entry(slot);
        }
    }

    // ---- startup re-adoption --------------------------------------------

    /// Re-adopt durable records on startup. The `record` files are canonical
    /// (NOT `systemctl list-units`). Applies the reconciliation matrix, then
    /// runs the defense-in-depth over-budget eviction.
    pub async fn reconcile_on_startup(&self) {
        Self::assert_quota_invariant();
        let slots = match self.store.list_slot_dirs() {
            Ok(slots) => slots,
            Err(_) => return,
        };
        // A query error must NOT be treated as "no units present" — that
        // would make every no-status record look unit-less and trigger
        // destructive reconciliation. On error, classify every slot as Unknown
        // and adopt non-destructively; the periodic reaper resolves once
        // systemd is queryable again.
        let present_units = self.units.list_managed_units().await.ok();

        for slot in &slots {
            let liveness = match &present_units {
                Some(units) => self.classify_unit(units, *slot),
                None => SlotLiveness::Unknown,
            };
            self.adopt_slot(*slot, liveness).await;
        }

        // Orphan units with no record → stop + reset-failed. Only safe when the
        // unit list was actually obtained (skip on a query error).
        if let Some(present_units) = &present_units {
            let adopted: Vec<u32> = {
                let state = self.lock();
                state.slots.keys().copied().collect()
            };
            for unit in present_units {
                if !adopted.contains(&unit.slot) {
                    let _ = self.units.stop_unit(unit.slot).await;
                    let _ = self.units.reset_failed(unit.slot).await;
                }
            }
        }

        self.evict_over_budget().await;
    }

    async fn adopt_slot(&self, slot: u32, liveness: SlotLiveness) {
        // Authenticity gate BEFORE trusting any on-disk bytes. A slot whose
        // dir/files fail the root-owned/mode/link-count/no-symlink checks is
        // quarantined (stop+reset any unit, delete the dir) and never adopted.
        if self.store.validate_authenticity(slot).is_err() {
            let _ = self.units.stop_unit(slot).await;
            let _ = self.units.reset_failed(slot).await;
            let _ = self.store.delete_slot_dir(slot);
            return;
        }
        let Ok(record) = self.store.read_record(slot) else {
            // Unreadable/corrupt → quarantine (delete).
            let _ = self.store.delete_slot_dir(slot);
            return;
        };
        // Authenticity: opaque id shape + boot id.
        if !is_valid_exec_id(&record.exec_id) {
            let _ = self.store.delete_slot_dir(slot);
            return;
        }
        if record.boot_id != self.config.boot_id {
            // Reboot: stale slot from a prior boot. Clean it up.
            let _ = self.units.stop_unit(slot).await;
            let _ = self.units.reset_failed(slot).await;
            let _ = self.store.delete_slot_dir(slot);
            return;
        }

        let status = self.store.read_status(slot).unwrap_or(None);
        let terminal_status = status_to_terminal(status);

        // Resolve the action per the reconciliation matrix.
        if let Some((terminal, code, signal)) = terminal_status {
            // Terminal status present (unit live or gone) — adopt terminal.
            if matches!(status, Some(StatusPhase::InfraFailed)) {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
                let _ = self.store.delete_slot_dir(slot);
                return;
            }
            if matches!(
                liveness,
                SlotLiveness::Live | SlotLiveness::OrphanWorkload | SlotLiveness::Foreign
            ) {
                let _ = self.units.stop_unit(slot).await;
                let _ = self.units.reset_failed(slot).await;
            }
            self.insert_adopted(slot, record, AdoptKind::Terminal(terminal, code, signal));
            return;
        }

        // No terminal status: branch on the unit's verified liveness.
        match liveness {
            SlotLiveness::Live => {
                // Authentic live unit + started/none → adopt Running. Never kill.
                self.insert_adopted(slot, record, AdoptKind::Running);
            }
            SlotLiveness::Foreign => {
                // Present but not our active runner (inactive/failed/mismatch):
                // clean up the impostor, then treat as having no live unit.
                if self.units.stop_unit(slot).await.is_err() {
                    self.insert_adopted(slot, record, AdoptKind::Running);
                } else {
                    let _ = self.units.reset_failed(slot).await;
                    self.adopt_no_unit(slot, record);
                }
            }
            SlotLiveness::OrphanWorkload => {
                if self.units.stop_unit(slot).await.is_err() {
                    self.insert_adopted(slot, record, AdoptKind::Running);
                } else {
                    self.adopt_no_unit(slot, record);
                }
            }
            SlotLiveness::Absent => {
                self.adopt_no_unit(slot, record);
            }
            SlotLiveness::Unknown => {
                // A query error is NOT absence. Adopt non-destructively and
                // let the reaper resolve once systemd is queryable: a
                // Dispatching record holds; anything else keeps Running.
                if record.state == RecordState::Dispatching {
                    self.insert_adopted(slot, record, AdoptKind::DispatchHold);
                } else {
                    self.insert_adopted(slot, record, AdoptKind::Running);
                }
            }
        }
    }

    /// Adopt a record with NO terminal status and NO live unit (definitive).
    /// Distinguishes "never ran" from "was running, runner vanished":
    /// - Dispatching within deadline → dispatch hold (reaper resolves).
    /// - Dispatching past deadline → delete + release (never ran); on an unlink
    ///   failure, keep a hidden retryable hold so the slot is not reused.
    /// - any other live state (e.g. Running) → mark lost, RETAIN slot+logs+
    ///   quota: the runner was up while guestd was down.
    fn adopt_no_unit(&self, slot: u32, record: DurableRecord) {
        if record.state == RecordState::Dispatching {
            if self.clock.now_ms() < record.dispatch_deadline_unix {
                self.insert_adopted(slot, record, AdoptKind::DispatchHold);
            } else {
                // Past the dispatch deadline, never ran → delete + release. If
                // the unlink fails, keep a hidden retryable dispatch-hold entry
                // so the slot is NOT reused with stale files on disk; a later
                // reaper pass retries the unlink (consistent with the GC and
                // dispatch-hold retain-on-failure paths).
                if self.store.delete_slot_dir(slot).is_err() {
                    self.insert_adopted(slot, record, AdoptKind::DispatchHold);
                }
            }
        } else {
            // Persisted Running (or other non-Dispatching live) with no unit
            // and no terminal status. Route through the SAME lost path as live
            // reconciliation: adopt as Running, then mark lost — releases only
            // the active counter, retaining slot + logs + quota until TTL/GC.
            self.insert_adopted(slot, record, AdoptKind::Running);
            self.mark_lost(slot);
        }
    }

    fn insert_adopted(&self, slot: u32, mut record: DurableRecord, kind: AdoptKind) {
        let caps = DetachedCaps::standard(self.config.max_runtime_sec);
        let now = self.clock.now_ms();
        let (active_counted, dispatch_hold) = match &kind {
            AdoptKind::Terminal(terminal_state, code, signal) => {
                record.state = *terminal_state;
                record.exit_code = *code;
                record.term_signal = *signal;
                if record.terminal_time_unix.is_none() {
                    record.terminal_time_unix = Some(now);
                }
                (false, false)
            }
            AdoptKind::Running => {
                record.state = RecordState::Running;
                (true, false)
            }
            AdoptKind::DispatchHold => {
                // Keep the persisted Dispatching state; reserved + non-listable
                // until the reaper resolves the hold.
                (true, true)
            }
        };
        let mut state = self.lock();
        state.reserved_log_bytes = state
            .reserved_log_bytes
            .saturating_add(caps.reserved_bytes());
        if active_counted {
            state.active = state.active.saturating_add(1);
        }
        state.slots.insert(
            slot,
            SlotEntry {
                record,
                caps,
                creating: false,
                dispatch_hold,
                generation: 0,
                active_counted,
                read_guards: 0,
            },
        );
        let _ = self
            .store
            .write_record(slot, &state.slots[&slot].record.clone());
    }

    /// Defense-in-depth: if the adopted reserved sum somehow exceeds the quota
    /// (e.g. the cap shrank between boots), evict the oldest TERMINAL records
    /// (never a Running job) until within budget.
    async fn evict_over_budget(&self) {
        loop {
            let victim = {
                let state = self.lock();
                if state.reserved_log_bytes <= DETACHED_LOG_QUOTA_BYTES {
                    None
                } else {
                    state
                        .slots
                        .iter()
                        .filter(|(_, entry)| entry.is_terminal())
                        .min_by_key(|(_, entry)| {
                            entry.record.terminal_time_unix.unwrap_or(u64::MAX)
                        })
                        .map(|(slot, _)| *slot)
                }
            };
            match victim {
                Some(slot) => self.gc_slot(slot).await,
                None => break,
            }
        }
    }

    // ---- helpers ---------------------------------------------------------

    fn resolve_slot(&self, exec_id: &str) -> Result<u32, ExecError> {
        let state = self.lock();
        match state.find_by_id(exec_id) {
            Some(slot) => Ok(slot),
            None => Err(if state.is_tombstoned(exec_id) {
                ExecError::ExecExpired
            } else {
                ExecError::ExecNotFound
            }),
        }
    }

    fn is_terminal(&self, slot: u32) -> bool {
        self.lock()
            .slots
            .get(&slot)
            .map(|entry| entry.is_terminal())
            .unwrap_or(false)
    }

    fn stream_metas(&self, slot: u32) -> (Option<StreamMeta>, Option<StreamMeta>) {
        let stdout = self
            .store
            .read_log_meta(slot, RunnerStream::Stdout)
            .unwrap_or(None);
        let stderr = self
            .store
            .read_log_meta(slot, RunnerStream::Stderr)
            .unwrap_or(None);
        (stdout, stderr)
    }

    fn snapshot_for(&self, slot: u32) -> Result<ExecSnapshot, ExecError> {
        let (record, generation) = {
            let state = self.lock();
            let entry = state.slots.get(&slot).ok_or(ExecError::Internal)?;
            (entry.record.clone(), entry.generation)
        };
        let (stdout_meta, stderr_meta) = self.stream_metas(slot);
        let stdout = stdout_meta.unwrap_or_else(zero_meta);
        let stderr = stderr_meta.unwrap_or_else(zero_meta);
        Ok(ExecSnapshot {
            state: public_state(&record),
            outcome: public_outcome(&record),
            state_generation: generation,
            stdout_start_offset: stdout.start_offset,
            stdout_end_offset: stdout.end_offset,
            stderr_start_offset: stderr.start_offset,
            stderr_end_offset: stderr.end_offset,
            stdout_dropped_bytes: stdout.dropped_bytes,
            stderr_dropped_bytes: stderr.dropped_bytes,
            stdout_truncated: stdout.truncated || stdout.lost,
            stderr_truncated: stderr.truncated || stderr.lost,
            // Detached execs are never interactive (TTY requires non-detached).
            stdin: TtyStdinSnapshot::NotInteractive,
            last_control_seq: 0,
        })
    }
}

fn zero_meta() -> StreamMeta {
    StreamMeta {
        cap: 0,
        start_offset: 0,
        end_offset: 0,
        dropped_bytes: 0,
        truncated: false,
        eof: false,
        lost: false,
    }
}

fn runner_stream(stream: RtStream) -> RunnerStream {
    match stream {
        RtStream::Stdout => RunnerStream::Stdout,
        RtStream::Stderr => RunnerStream::Stderr,
    }
}

fn map_ring_error(error: FileRingError) -> ExecError {
    match error {
        FileRingError::OffsetExpired | FileRingError::Busy => ExecError::OffsetExpired,
        FileRingError::OffsetInFuture => ExecError::OffsetInFuture,
        _ => ExecError::Internal,
    }
}

fn public_state(record: &DurableRecord) -> ExecState {
    match record.state {
        RecordState::Dispatching | RecordState::Running => ExecState::Running,
        RecordState::Exited => ExecState::Exited,
        RecordState::Signaled => ExecState::Signaled,
        // A spawn failure is a legitimate terminal exec; surface it as an exit.
        RecordState::SpawnFailed => ExecState::Exited,
        RecordState::Cancelled => {
            if record.lost {
                ExecState::LostGuestd
            } else {
                ExecState::Cancelled
            }
        }
    }
}

fn public_outcome(record: &DurableRecord) -> Option<ExitOutcome> {
    match record.state {
        RecordState::Exited => Some(ExitOutcome::Exited(record.exit_code.unwrap_or(-1))),
        RecordState::Signaled => Some(ExitOutcome::Signaled(record.term_signal.unwrap_or(0))),
        // Spawn failure maps to the shell "command could not execute" code.
        RecordState::SpawnFailed => Some(ExitOutcome::Exited(127)),
        _ => None,
    }
}

fn status_to_terminal(
    status: Option<StatusPhase>,
) -> Option<(RecordState, Option<i32>, Option<u32>)> {
    match status {
        Some(StatusPhase::Exited(code)) => Some((RecordState::Exited, Some(code), None)),
        Some(StatusPhase::Signaled(signal)) => {
            Some((RecordState::Signaled, None, Some(signal as u32)))
        }
        Some(StatusPhase::Cancelled) => Some((RecordState::Cancelled, None, None)),
        Some(StatusPhase::SpawnFailed) => Some((RecordState::SpawnFailed, None, None)),
        // InfraFailed is handled separately (cleanup, not adoption).
        Some(StatusPhase::InfraFailed) => Some((RecordState::SpawnFailed, None, None)),
        Some(StatusPhase::Started) | None => None,
    }
}

fn property_contains_unit(value: Option<&str>, unit: &str) -> bool {
    value
        .map(|v| v.split_whitespace().any(|token| token == unit))
        .unwrap_or(false)
}

fn build_spec(
    command: &ValidatedCommand,
    caps: &DetachedCaps,
    config: &RegistryConfig,
    slot: u32,
) -> Result<ExecSpec, ExecError> {
    let program = command.program.to_str().ok_or(ExecError::InvalidArgv)?;
    let mut argv = Vec::with_capacity(command.args.len() + 1);
    argv.push(program.to_owned());
    argv.extend(command.args.iter().cloned());
    let cwd = command
        .cwd
        .to_str()
        .map(|s| s.to_owned())
        .ok_or(ExecError::CwdInvalid)?;
    let env = command
        .env
        .iter()
        .map(|(key, value)| RunnerEnv {
            key: key.clone(),
            value: value.clone(),
        })
        .collect();
    let workload_unit = workload_unit_name(slot);
    let systemd_run_args = workload_systemd_run_args(command, config, slot, &workload_unit)?;
    Ok(ExecSpec {
        argv,
        cwd: Some(cwd),
        env,
        stdout_log_cap: caps.stdout_log_cap,
        stderr_log_cap: caps.stderr_log_cap,
        max_runtime_sec: caps.max_runtime_sec,
        exec_user: config.exec_user.clone(),
        exec_uid: config.exec_uid,
        systemd_run_path: config.systemd_run_path.clone(),
        login_shell_path: config.login_shell_path.clone(),
        workload_unit_name: workload_unit,
        systemd_run_args,
    })
}

fn workload_systemd_run_args(
    command: &ValidatedCommand,
    config: &RegistryConfig,
    slot: u32,
    workload_unit: &str,
) -> Result<Vec<String>, ExecError> {
    let cwd = command
        .cwd
        .to_str()
        .map(|s| s.to_owned())
        .ok_or(ExecError::CwdInvalid)?;
    let program = command.program.to_str().ok_or(ExecError::InvalidArgv)?;
    let runner_unit = unit_name(slot);
    let mut argv = vec![
        format!("--uid={}", config.exec_user),
        format!("--unit={workload_unit}"),
        "--quiet".to_owned(),
        "--collect".to_owned(),
        "--expand-environment=no".to_owned(),
        "--slice=d2b-exec.slice".to_owned(),
        "--pipe".to_owned(),
        "--wait".to_owned(),
        "--property=PAMName=login".to_owned(),
        format!("--property=BindsTo={runner_unit}"),
        format!("--property=PartOf={runner_unit}"),
        format!("--property=After={runner_unit}"),
        format!("--working-directory={cwd}"),
    ];
    for (key, value) in &command.env {
        argv.push("--setenv".to_owned());
        argv.push(format!("{key}={value}"));
    }
    argv.push("--".to_owned());
    argv.push(config.login_shell_path.clone());
    argv.push("-l".to_owned());
    argv.push("-c".to_owned());
    argv.push(r#"exec "$@""#.to_owned());
    argv.push("d2b-exec".to_owned());
    argv.push(program.to_owned());
    argv.extend(command.args.iter().cloned());
    Ok(argv)
}

fn argv_hash(command: &ValidatedCommand) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.program.as_os_str().as_encoded_bytes());
    for arg in &command.args {
        hasher.update([0u8]);
        hasher.update(arg.as_bytes());
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The opaque exec id is 16 random bytes hex-encoded (32 lowercase hex chars).
fn is_valid_exec_id(exec_id: &str) -> bool {
    exec_id.len() == 32
        && exec_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Stable unit name for a slot (re-exported for the service layer).
pub fn slot_unit_name(slot: u32) -> String {
    unit_name(slot)
}

/// Production [`SlotStore`] rooted at `/run/d2b-exec` (root-owned 0700,
/// boot-scoped). All on-disk access is `O_NOFOLLOW` and the parent/slot dirs
/// are validated root-owned before any open, mirroring the runner's
/// `validate_slot_dir`. Reuses the dependency-pure exec-runner primitives
/// (`atomicio`, `FileRing`, the record/spec/status codecs) so the wire layout
/// is identical to what the runner reads and writes.
pub struct RunSlotStore {
    base: std::path::PathBuf,
}

impl RunSlotStore {
    /// Production base (`/run/d2b-exec`).
    pub fn new() -> Self {
        Self {
            base: std::path::PathBuf::from(d2b_exec_runner::paths::RUN_DIR),
        }
    }

    /// Construct rooted at an arbitrary base (Layer-2 / integration harnesses).
    pub fn with_base(base: impl Into<std::path::PathBuf>) -> Self {
        Self { base: base.into() }
    }

    fn paths(&self, slot: u32) -> RunnerPaths {
        RunnerPaths::new(self.base.clone(), slot)
    }

    /// Validate the parent and slot directories are root-owned 0700 dirs via
    /// dir-fd `openat`/`O_NOFOLLOW` (mirrors the runner's `validate_slot_dir`).
    fn validate_slot_dir(&self, paths: &RunnerPaths) -> Result<(), ExecError> {
        use rustix::fs::{Mode, OFlags, fstat, open, openat};
        let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let base = open(paths.base(), dir_flags, Mode::empty()).map_err(|_| ExecError::Internal)?;
        let base_stat = fstat(&base).map_err(|_| ExecError::Internal)?;
        check_dir_stat(&base_stat)?;
        let slot = openat(&base, paths.slot_dir_name(), dir_flags, Mode::empty())
            .map_err(|_| ExecError::Internal)?;
        let slot_stat = fstat(&slot).map_err(|_| ExecError::Internal)?;
        check_dir_stat(&slot_stat)?;
        Ok(())
    }
}

/// A re-adoption authenticity check failed for a slot dir: it must be a
/// root-owned 0700 directory.
fn check_dir_stat(st: &rustix::fs::Stat) -> Result<(), ExecError> {
    use rustix::fs::FileType;
    if FileType::from_raw_mode(st.st_mode) != FileType::Directory
        || st.st_uid != 0
        || (st.st_mode & 0o777) != 0o700
    {
        return Err(ExecError::RetainedLogPathUnsafe);
    }
    Ok(())
}

/// A re-adoption authenticity check failed for a slot file: it must be a
/// root-owned, regular 0600 file with exactly one hard link.
fn check_file_stat(st: &rustix::fs::Stat) -> Result<(), ExecError> {
    use rustix::fs::FileType;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile
        || st.st_uid != 0
        || (st.st_mode & 0o777) != 0o600
        || st.st_nlink != 1
    {
        return Err(ExecError::RetainedLogPathUnsafe);
    }
    Ok(())
}

impl Default for RunSlotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotStore for RunSlotStore {
    fn prepare_slot_dir(&self, slot: u32) -> Result<(), ExecError> {
        use rustix::fs::{Mode, OFlags, fstat, open};
        use std::os::unix::fs::DirBuilderExt;
        let paths = self.paths(slot);
        // The parent must exist and be root-owned 0700 (created by the nixos
        // tmpfiles rule); fail closed otherwise.
        let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let base = open(paths.base(), dir_flags, Mode::empty()).map_err(|_| ExecError::Internal)?;
        let base_stat = fstat(&base).map_err(|_| ExecError::Internal)?;
        if base_stat.st_uid != 0 {
            return Err(ExecError::RetainedLogPathUnsafe);
        }
        let slot_dir = paths.slot_dir();
        match std::fs::DirBuilder::new().mode(0o700).create(&slot_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ExecError::Internal),
        }
        d2b_exec_runner::atomicio::fsync_parent_dir(&slot_dir).map_err(|_| ExecError::Internal)?;
        // Re-validate ownership after creation (defends against a pre-seeded
        // foreign slot dir).
        self.validate_slot_dir(&paths)?;
        Ok(())
    }

    fn write_record(&self, slot: u32, record: &DurableRecord) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        d2b_exec_runner::atomicio::atomic_write(&paths.record(), &record.encode())
            .map_err(|_| ExecError::Internal)
    }

    fn read_record(&self, slot: u32) -> Result<DurableRecord, ExecError> {
        let paths = self.paths(slot);
        let bytes = d2b_exec_runner::atomicio::read_file_nofollow(&paths.record())
            .map_err(|_| ExecError::ExecNotFound)?;
        DurableRecord::decode(&bytes).map_err(|_| ExecError::Internal)
    }

    fn write_spec(&self, slot: u32, spec: &ExecSpec) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        let bytes =
            d2b_exec_runner::spec::SpecCodec::encode(spec).map_err(|_| ExecError::Internal)?;
        d2b_exec_runner::atomicio::atomic_write(&paths.spec(), &bytes)
            .map_err(|_| ExecError::Internal)
    }

    fn read_spec(&self, slot: u32) -> Result<ExecSpec, ExecError> {
        let paths = self.paths(slot);
        let bytes = d2b_exec_runner::atomicio::read_file_nofollow(&paths.spec())
            .map_err(|_| ExecError::Internal)?;
        d2b_exec_runner::spec::SpecCodec::decode(&bytes).map_err(|_| ExecError::Internal)
    }

    fn write_cancel(&self, slot: u32) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        // The sentinel's presence is the cancel signal; content is irrelevant.
        d2b_exec_runner::atomicio::atomic_write(&paths.cancel(), b"1")
            .map_err(|_| ExecError::Internal)
    }

    fn read_status(&self, slot: u32) -> Result<Option<StatusPhase>, ExecError> {
        let paths = self.paths(slot);
        match d2b_exec_runner::atomicio::read_file_nofollow(&paths.status()) {
            Ok(bytes) => {
                let rec = d2b_exec_runner::record::StatusRecord::decode(&bytes)
                    .map_err(|_| ExecError::Internal)?;
                Ok(Some(rec.phase))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(ExecError::Internal),
        }
    }

    fn read_log_meta(
        &self,
        slot: u32,
        stream: RunnerStream,
    ) -> Result<Option<StreamMeta>, ExecError> {
        let paths = self.paths(slot);
        match d2b_exec_runner::filering::FileRingReader::open(
            &paths.data(stream),
            &paths.sidecar(stream),
        ) {
            Ok(reader) => reader.meta().map(Some).map_err(|_| ExecError::Internal),
            Err(FileRingError::Io(std::io::ErrorKind::NotFound)) => Ok(None),
            Err(_) => Err(ExecError::Internal),
        }
    }

    fn read_log(
        &self,
        slot: u32,
        stream: RunnerStream,
        offset: u64,
        max_len: u64,
    ) -> Result<RingChunk, FileRingError> {
        let paths = self.paths(slot);
        let reader = d2b_exec_runner::filering::FileRingReader::open(
            &paths.data(stream),
            &paths.sidecar(stream),
        )?;
        reader.read(offset, max_len)
    }

    fn mark_lost(&self, slot: u32) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        for stream in [RunnerStream::Stdout, RunnerStream::Stderr] {
            match d2b_exec_runner::filering::mark_stream_lost(&paths.sidecar(stream)) {
                Ok(()) => {}
                Err(FileRingError::Io(std::io::ErrorKind::NotFound)) => {}
                Err(_) => return Err(ExecError::Internal),
            }
        }
        Ok(())
    }

    fn delete_slot_dir(&self, slot: u32) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        match std::fs::remove_dir_all(paths.slot_dir()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ExecError::Internal),
        }
        d2b_exec_runner::atomicio::fsync_parent_dir(&paths.slot_dir())
            .map_err(|_| ExecError::Internal)
    }

    fn scrub_slot_files(&self, slot: u32) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        // The runner-written files a reused slot must never inherit. The
        // record/spec are immediately rewritten by persist_dispatch, so they are
        // left to be replaced atomically.
        let stale = [
            paths.status(),
            paths.cancel(),
            paths.data(RunnerStream::Stdout),
            paths.data(RunnerStream::Stderr),
            paths.sidecar(RunnerStream::Stdout),
            paths.sidecar(RunnerStream::Stderr),
        ];
        for path in stale {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ExecError::Internal),
            }
        }
        d2b_exec_runner::atomicio::fsync_parent_dir(&paths.slot_dir())
            .map_err(|_| ExecError::Internal)
    }

    fn validate_authenticity(&self, slot: u32) -> Result<(), ExecError> {
        use rustix::fs::{Mode, OFlags, fstat, open, openat};
        let paths = self.paths(slot);
        let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        // Base dir: root-owned 0700 directory.
        let base = open(paths.base(), dir_flags, Mode::empty()).map_err(|_| ExecError::Internal)?;
        check_dir_stat(&fstat(&base).map_err(|_| ExecError::Internal)?)?;
        // Slot dir: root-owned 0700 directory reached via openat O_NOFOLLOW.
        let slot_fd = openat(&base, paths.slot_dir_name(), dir_flags, Mode::empty())
            .map_err(|_| ExecError::RetainedLogPathUnsafe)?;
        check_dir_stat(&fstat(&slot_fd).map_err(|_| ExecError::RetainedLogPathUnsafe)?)?;
        // Each present per-slot file: root-owned regular 0600, link-count 1,
        // reached without traversing a symlink (O_NOFOLLOW). Absent files are
        // permitted (the runner may not have created every stream yet).
        let file_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        for name in RunnerPaths::slot_file_names() {
            match openat(&slot_fd, name, file_flags, Mode::empty()) {
                Ok(fd) => {
                    check_file_stat(&fstat(&fd).map_err(|_| ExecError::RetainedLogPathUnsafe)?)?;
                }
                Err(rustix::io::Errno::NOENT) => {}
                // ELOOP (symlink under O_NOFOLLOW) or any other failure → unsafe.
                Err(_) => return Err(ExecError::RetainedLogPathUnsafe),
            }
        }
        Ok(())
    }

    fn list_slot_dirs(&self) -> Result<Vec<u32>, ExecError> {
        let mut slots = Vec::new();
        let entries = match std::fs::read_dir(&self.base) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(slots),
            Err(_) => return Err(ExecError::Internal),
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(rest) = name.strip_prefix("slot-") else {
                continue;
            };
            if let Ok(slot) = rest.parse::<u32>()
                && (slot as usize) < DETACHED_RETAINED_PER_VM
            {
                slots.push(slot);
            }
        }
        slots.sort_unstable();
        Ok(slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detached::UnitError;
    use std::collections::HashMap;
    use std::sync::Condvar;
    use std::sync::atomic::{AtomicU64, Ordering};

    const RUNNER_PATH: &str = "/run/current-system/sw/bin/d2b-exec-runner";

    // ---- fakes -----------------------------------------------------------

    /// Cross-fake event log: lets order-sensitive tests assert the relative
    /// order of store writes and unit-manager calls (e.g. the cancel sentinel
    /// strictly precedes `stop_unit`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Event {
        WriteCancel(u32),
        StopUnit(u32),
        ScrubSlotFiles(u32),
        DeleteSlotDir(u32),
    }

    type EventLog = Arc<Mutex<Vec<Event>>>;

    /// A barrier that blocks `FakeStore::read_log` until a test releases it,
    /// while signalling once the read-guard has actually been taken. Used to
    /// deterministically exercise the GC-vs-in-flight-read race.
    #[derive(Default)]
    struct ReadGate {
        state: Mutex<ReadGateState>,
        entered_cv: Condvar,
        release_cv: Condvar,
    }
    #[derive(Default)]
    struct ReadGateState {
        entered: bool,
        released: bool,
    }
    impl ReadGate {
        fn wait_in_read(&self) {
            let mut g = self.state.lock().unwrap();
            g.entered = true;
            self.entered_cv.notify_all();
            while !g.released {
                g = self.release_cv.wait(g).unwrap();
            }
        }
        fn wait_until_entered(&self) {
            let mut g = self.state.lock().unwrap();
            while !g.entered {
                g = self.entered_cv.wait(g).unwrap();
            }
        }
        fn release(&self) {
            let mut g = self.state.lock().unwrap();
            g.released = true;
            self.release_cv.notify_all();
        }
    }

    #[derive(Default)]
    struct FakeStoreState {
        records: HashMap<u32, Vec<u8>>,
        specs: HashMap<u32, Vec<u8>>,
        status: HashMap<u32, StatusPhase>,
        cancels: HashMap<u32, bool>,
        stdout_meta: HashMap<u32, StreamMeta>,
        stderr_meta: HashMap<u32, StreamMeta>,
        stdout_data: HashMap<u32, Vec<u8>>,
        prepared: Vec<u32>,
        scrubbed: Vec<u32>,
        fail_prepare: bool,
        /// Slots whose authenticity gate must fail (re-adoption quarantine).
        fail_authenticity: std::collections::HashSet<u32>,
        /// When set, writing the cancel sentinel also publishes this terminal
        /// status (simulating a promptly-reacting runner).
        cancel_terminal: Option<StatusPhase>,
        /// When set, `read_log` blocks on this gate (GC-vs-read race test).
        read_gate: Option<Arc<ReadGate>>,
        /// When set, `read_status` returns this error (create-resolution leak).
        fail_status: bool,
        /// When set, `delete_slot_dir` returns an error (retain-on-failure).
        fail_delete: bool,
        /// When set, `read_status` blocks on this gate (Creating-guard race).
        status_gate: Option<Arc<ReadGate>>,
    }

    #[derive(Clone, Default)]
    struct FakeStore {
        inner: Arc<Mutex<FakeStoreState>>,
        events: EventLog,
    }

    impl FakeStore {
        fn set_status(&self, slot: u32, phase: StatusPhase) {
            self.inner.lock().unwrap().status.insert(slot, phase);
        }
        fn set_stdout(&self, slot: u32, data: &[u8], meta: StreamMeta) {
            let mut s = self.inner.lock().unwrap();
            s.stdout_data.insert(slot, data.to_vec());
            s.stdout_meta.insert(slot, meta);
        }
        fn set_stderr_meta(&self, slot: u32, meta: StreamMeta) {
            self.inner.lock().unwrap().stderr_meta.insert(slot, meta);
        }
        fn cancel_written(&self, slot: u32) -> bool {
            *self
                .inner
                .lock()
                .unwrap()
                .cancels
                .get(&slot)
                .unwrap_or(&false)
        }
        fn set_cancel_terminal(&self, phase: StatusPhase) {
            self.inner.lock().unwrap().cancel_terminal = Some(phase);
        }
        fn slot_exists(&self, slot: u32) -> bool {
            self.inner.lock().unwrap().records.contains_key(&slot)
        }
        fn seed_record(&self, slot: u32, record: &DurableRecord) {
            let mut s = self.inner.lock().unwrap();
            s.records.insert(slot, record.encode());
            s.specs
                .entry(slot)
                .or_insert_with(|| encode_spec_for_slot(slot));
        }
        /// Seed an undecodable record so `read_record` fails (re-adoption must
        /// quarantine the slot rather than trusting corrupt bytes).
        fn seed_corrupt_record(&self, slot: u32) {
            self.inner
                .lock()
                .unwrap()
                .records
                .insert(slot, b"not-a-valid-durable-record".to_vec());
        }
        fn scrubbed(&self) -> Vec<u32> {
            self.inner.lock().unwrap().scrubbed.clone()
        }
        fn set_fail_authenticity(&self, slot: u32) {
            self.inner.lock().unwrap().fail_authenticity.insert(slot);
        }
        fn install_read_gate(&self, gate: Arc<ReadGate>) {
            self.inner.lock().unwrap().read_gate = Some(gate);
        }
        fn install_status_gate(&self, gate: Arc<ReadGate>) {
            self.inner.lock().unwrap().status_gate = Some(gate);
        }
        fn set_fail_status(&self, fail: bool) {
            self.inner.lock().unwrap().fail_status = fail;
        }
        fn set_fail_delete(&self, fail: bool) {
            self.inner.lock().unwrap().fail_delete = fail;
        }
    }

    impl SlotStore for FakeStore {
        fn prepare_slot_dir(&self, slot: u32) -> Result<(), ExecError> {
            let mut s = self.inner.lock().unwrap();
            if s.fail_prepare {
                return Err(ExecError::RetainedLogPathUnsafe);
            }
            s.prepared.push(slot);
            Ok(())
        }
        fn write_record(&self, slot: u32, record: &DurableRecord) -> Result<(), ExecError> {
            self.inner
                .lock()
                .unwrap()
                .records
                .insert(slot, record.encode());
            Ok(())
        }
        fn read_record(&self, slot: u32) -> Result<DurableRecord, ExecError> {
            let s = self.inner.lock().unwrap();
            let bytes = s.records.get(&slot).ok_or(ExecError::ExecNotFound)?;
            DurableRecord::decode(bytes).map_err(|_| ExecError::Internal)
        }
        fn write_spec(&self, slot: u32, spec: &ExecSpec) -> Result<(), ExecError> {
            let bytes =
                d2b_exec_runner::spec::SpecCodec::encode(spec).map_err(|_| ExecError::Internal)?;
            self.inner.lock().unwrap().specs.insert(slot, bytes);
            Ok(())
        }
        fn read_spec(&self, slot: u32) -> Result<ExecSpec, ExecError> {
            let s = self.inner.lock().unwrap();
            let bytes = s.specs.get(&slot).ok_or(ExecError::Internal)?;
            d2b_exec_runner::spec::SpecCodec::decode(bytes).map_err(|_| ExecError::Internal)
        }
        fn write_cancel(&self, slot: u32) -> Result<(), ExecError> {
            self.events.lock().unwrap().push(Event::WriteCancel(slot));
            let mut s = self.inner.lock().unwrap();
            s.cancels.insert(slot, true);
            if let Some(phase) = s.cancel_terminal {
                s.status.insert(slot, phase);
            }
            Ok(())
        }
        fn read_status(&self, slot: u32) -> Result<Option<StatusPhase>, ExecError> {
            // Optionally park here (with the Creating guard held by the caller)
            // so a concurrent ExecList/reaper must observe the hidden entry.
            let gate = self.inner.lock().unwrap().status_gate.clone();
            if let Some(gate) = gate {
                gate.wait_in_read();
            }
            let s = self.inner.lock().unwrap();
            if s.fail_status {
                return Err(ExecError::Internal);
            }
            Ok(s.status.get(&slot).copied())
        }
        fn read_log_meta(
            &self,
            slot: u32,
            stream: RunnerStream,
        ) -> Result<Option<StreamMeta>, ExecError> {
            let s = self.inner.lock().unwrap();
            Ok(match stream {
                RunnerStream::Stdout => s.stdout_meta.get(&slot).copied(),
                RunnerStream::Stderr => s.stderr_meta.get(&slot).copied(),
            })
        }
        fn read_log(
            &self,
            slot: u32,
            stream: RunnerStream,
            offset: u64,
            max_len: u64,
        ) -> Result<RingChunk, FileRingError> {
            // Optionally block here (with the read-guard already held by the
            // caller) so a concurrent GC observes guard>0 and defers.
            let gate = self.inner.lock().unwrap().read_gate.clone();
            if let Some(gate) = gate {
                gate.wait_in_read();
            }
            let s = self.inner.lock().unwrap();
            let (data, meta) = match stream {
                RunnerStream::Stdout => (s.stdout_data.get(&slot), s.stdout_meta.get(&slot)),
                RunnerStream::Stderr => (None, s.stderr_meta.get(&slot)),
            };
            let meta = meta.copied().ok_or(FileRingError::OffsetInFuture)?;
            if offset < meta.start_offset {
                return Err(FileRingError::OffsetExpired);
            }
            if offset > meta.end_offset {
                return Err(FileRingError::OffsetInFuture);
            }
            let data = data.cloned().unwrap_or_default();
            let begin = (offset - meta.start_offset) as usize;
            let take = ((meta.end_offset - offset).min(max_len)) as usize;
            let slice = data
                .get(begin..(begin + take).min(data.len()))
                .unwrap_or(&[])
                .to_vec();
            let next = offset + slice.len() as u64;
            Ok(RingChunk {
                data: slice,
                start_offset: meta.start_offset,
                end_offset: meta.end_offset,
                next_offset: next,
                dropped_bytes: meta.dropped_bytes,
                truncated: meta.truncated || meta.lost,
                eof: meta.eof && next >= meta.end_offset,
            })
        }
        fn mark_lost(&self, slot: u32) -> Result<(), ExecError> {
            let mut s = self.inner.lock().unwrap();
            if let Some(meta) = s.stdout_meta.get_mut(&slot) {
                meta.lost = true;
            }
            if let Some(meta) = s.stderr_meta.get_mut(&slot) {
                meta.lost = true;
            }
            Ok(())
        }
        fn delete_slot_dir(&self, slot: u32) -> Result<(), ExecError> {
            self.events.lock().unwrap().push(Event::DeleteSlotDir(slot));
            let mut s = self.inner.lock().unwrap();
            if s.fail_delete {
                return Err(ExecError::Internal);
            }
            s.records.remove(&slot);
            s.specs.remove(&slot);
            s.status.remove(&slot);
            s.cancels.remove(&slot);
            s.stdout_meta.remove(&slot);
            s.stderr_meta.remove(&slot);
            s.stdout_data.remove(&slot);
            Ok(())
        }
        fn scrub_slot_files(&self, slot: u32) -> Result<(), ExecError> {
            self.events
                .lock()
                .unwrap()
                .push(Event::ScrubSlotFiles(slot));
            // Record-only: the real removal is covered by a dedicated on-disk
            // test. Clearing status here would break create-flow tests that
            // pre-seed a status before calling `create`.
            self.inner.lock().unwrap().scrubbed.push(slot);
            Ok(())
        }
        fn validate_authenticity(&self, slot: u32) -> Result<(), ExecError> {
            if self.inner.lock().unwrap().fail_authenticity.contains(&slot) {
                return Err(ExecError::RetainedLogPathUnsafe);
            }
            Ok(())
        }
        fn list_slot_dirs(&self) -> Result<Vec<u32>, ExecError> {
            Ok(self.inner.lock().unwrap().records.keys().copied().collect())
        }
    }

    struct FakeUnitsState {
        runner_live: Vec<u32>,
        workload_live: Vec<u32>,
        started: Vec<u32>,
        stopped: Vec<u32>,
        fail_start: bool,
        fail_stop: bool,
        /// `list_managed_units` returns an error (liveness must be Unknown).
        fail_list: bool,
        /// Live slots forced to report `active = false` (→ Foreign).
        inactive: std::collections::HashSet<u32>,
        /// Live slots forced to report a non-matching identity (→ Foreign).
        mismatch: std::collections::HashSet<u32>,
        workload_mismatch: std::collections::HashSet<u32>,
        /// Live slots whose `systemctl show` identity enrichment FAILED: the
        /// unit is reported active but its identity is `Unqueried` (→ Unknown,
        /// never Foreign).
        show_fail: std::collections::HashSet<u32>,
        workload_show_fail: std::collections::HashSet<u32>,
        /// Explicit per-slot identity overrides (active unit). Used by the
        /// structural-identity tests to inject impostor argv shapes.
        identity_override: HashMap<u32, UnitIdentity>,
        workload_identity_override: HashMap<u32, UnitIdentity>,
        /// Runner binary path used to synthesize a matching `ExecStart`.
        runner_path: String,
    }

    impl Default for FakeUnitsState {
        fn default() -> Self {
            Self {
                runner_live: Vec::new(),
                workload_live: Vec::new(),
                started: Vec::new(),
                stopped: Vec::new(),
                fail_start: false,
                fail_stop: false,
                fail_list: false,
                inactive: std::collections::HashSet::new(),
                mismatch: std::collections::HashSet::new(),
                workload_mismatch: std::collections::HashSet::new(),
                show_fail: std::collections::HashSet::new(),
                workload_show_fail: std::collections::HashSet::new(),
                identity_override: HashMap::new(),
                workload_identity_override: HashMap::new(),
                runner_path: RUNNER_PATH.to_owned(),
            }
        }
    }

    #[derive(Clone, Default)]
    struct FakeUnits {
        inner: Arc<Mutex<FakeUnitsState>>,
        events: EventLog,
    }

    impl FakeUnits {
        fn set_live(&self, slot: u32, live: bool) {
            self.set_runner_live(slot, live);
            self.set_workload_live(slot, live);
        }
        fn set_runner_live(&self, slot: u32, live: bool) {
            let mut s = self.inner.lock().unwrap();
            s.runner_live.retain(|x| *x != slot);
            if live {
                s.runner_live.push(slot);
            }
        }
        fn set_workload_live(&self, slot: u32, live: bool) {
            let mut s = self.inner.lock().unwrap();
            s.workload_live.retain(|x| *x != slot);
            if live {
                s.workload_live.push(slot);
            }
        }
        fn stopped(&self, slot: u32) -> bool {
            self.inner.lock().unwrap().stopped.contains(&slot)
        }
        fn set_fail_list(&self, fail: bool) {
            self.inner.lock().unwrap().fail_list = fail;
        }
        fn set_inactive(&self, slot: u32) {
            self.inner.lock().unwrap().inactive.insert(slot);
        }
        fn set_mismatch(&self, slot: u32) {
            self.inner.lock().unwrap().mismatch.insert(slot);
        }
        fn set_workload_mismatch(&self, slot: u32) {
            self.inner.lock().unwrap().workload_mismatch.insert(slot);
        }
        /// Simulate a `systemctl show` identity-enrichment failure for an
        /// otherwise-active unit (identity `Unqueried`).
        fn set_show_fail(&self, slot: u32) {
            self.inner.lock().unwrap().show_fail.insert(slot);
        }
        /// Inject an explicit identity for an active unit (structural tests).
        fn set_identity(&self, slot: u32, identity: UnitIdentity) {
            self.inner
                .lock()
                .unwrap()
                .identity_override
                .insert(slot, identity);
        }
        fn set_workload_identity(&self, slot: u32, identity: UnitIdentity) {
            self.inner
                .lock()
                .unwrap()
                .workload_identity_override
                .insert(slot, identity);
        }
        fn set_fail_stop(&self, fail: bool) {
            self.inner.lock().unwrap().fail_stop = fail;
        }
        /// Build the authentic systemd-rendered `ExecStart` for a slot.
        fn authentic_exec_start(runner_path: &str, slot: u32) -> String {
            format!(
                "{{ path={runner_path} ; argv[]={runner_path} --serve-exec --slot {slot:02} ; \
                 ignore_errors=no }}"
            )
        }
    }

    #[async_trait]
    impl TransientUnitManager for FakeUnits {
        async fn start_transient_unit(
            &self,
            slot: u32,
            _ceiling_sec: u64,
            _paths: &RunnerUnitPaths,
        ) -> Result<(), UnitError> {
            let mut s = self.inner.lock().unwrap();
            if s.fail_start {
                return Err(UnitError::SpawnFailed);
            }
            s.started.push(slot);
            Ok(())
        }
        async fn stop_unit(&self, slot: u32) -> Result<(), UnitError> {
            self.events.lock().unwrap().push(Event::StopUnit(slot));
            let mut s = self.inner.lock().unwrap();
            s.stopped.push(slot);
            if s.fail_stop {
                return Err(UnitError::Internal);
            }
            s.runner_live.retain(|x| *x != slot);
            s.workload_live.retain(|x| *x != slot);
            Ok(())
        }
        async fn reset_failed(&self, _slot: u32) -> Result<(), UnitError> {
            Ok(())
        }
        async fn list_managed_units(&self) -> Result<Vec<crate::detached::ManagedUnit>, UnitError> {
            let s = self.inner.lock().unwrap();
            if s.fail_list {
                return Err(UnitError::Internal);
            }
            let mut out = Vec::new();
            for slot in &s.runner_live {
                let identity = if let Some(identity) = s.identity_override.get(slot) {
                    identity.clone()
                } else if s.show_fail.contains(slot) {
                    // `systemctl show` enrichment failed: identity unknown.
                    UnitIdentity::Unqueried
                } else if s.mismatch.contains(slot) {
                    // Plausible-but-foreign command (different exe) at our slot.
                    UnitIdentity::Queried {
                        slice: Some("d2b-exec.slice".to_owned()),
                        exec_start: Some(format!(
                            "{{ path=/usr/bin/evil ; argv[]=/usr/bin/evil --serve-exec \
                                 --slot {slot:02} ; ignore_errors=no }}"
                        )),
                        binds_to: Some(unit_name(*slot)),
                        part_of: Some(unit_name(*slot)),
                        after: Some(unit_name(*slot)),
                    }
                } else {
                    UnitIdentity::Queried {
                        slice: Some("d2b-exec.slice".to_owned()),
                        exec_start: Some(Self::authentic_exec_start(&s.runner_path, *slot)),
                        binds_to: Some(unit_name(*slot)),
                        part_of: Some(unit_name(*slot)),
                        after: Some(unit_name(*slot)),
                    }
                };
                out.push(crate::detached::ManagedUnit {
                    slot: *slot,
                    kind: ManagedUnitKind::Runner,
                    active: !s.inactive.contains(slot),
                    identity,
                });
            }
            for slot in &s.workload_live {
                let identity = if let Some(identity) = s.workload_identity_override.get(slot) {
                    identity.clone()
                } else if s.workload_show_fail.contains(slot) {
                    UnitIdentity::Unqueried
                } else if s.workload_mismatch.contains(slot) {
                    UnitIdentity::Queried {
                        slice: Some("user-1000.slice".to_owned()),
                        exec_start: None,
                        binds_to: None,
                        part_of: None,
                        after: None,
                    }
                } else {
                    UnitIdentity::Queried {
                        slice: Some("d2b-exec.slice".to_owned()),
                        exec_start: Some(workload_exec_start(*slot)),
                        binds_to: Some(unit_name(*slot)),
                        part_of: Some(unit_name(*slot)),
                        after: Some(unit_name(*slot)),
                    }
                };
                out.push(crate::detached::ManagedUnit {
                    slot: *slot,
                    kind: ManagedUnitKind::Workload,
                    active: !s.inactive.contains(slot),
                    identity,
                });
            }
            Ok(out)
        }
    }

    struct FakeClock {
        now: Arc<AtomicU64>,
    }
    impl WallClock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    struct FakeSleeper {
        now: Arc<AtomicU64>,
        step: u64,
    }
    #[async_trait]
    impl Sleeper for FakeSleeper {
        async fn sleep_ms(&self, _ms: u64) {
            // Advance the paired clock so bounded loops terminate deterministically.
            self.now.fetch_add(self.step, Ordering::SeqCst);
            tokio::task::yield_now().await;
        }
    }

    struct SeqIds {
        next: AtomicU64,
    }
    impl ExecIdSource for SeqIds {
        fn next_exec_id(&self) -> Result<String, ExecError> {
            let n = self.next.fetch_add(1, Ordering::SeqCst);
            // 32 lowercase hex chars (valid opaque-id shape).
            Ok(format!("{n:032x}"))
        }
    }

    struct Harness {
        registry: DetachedRegistry,
        store: FakeStore,
        units: FakeUnits,
        now: Arc<AtomicU64>,
        events: EventLog,
    }

    fn harness() -> Harness {
        harness_with_clock_step(STATUS_POLL_INTERVAL_MS)
    }

    fn harness_with_clock_step(step: u64) -> Harness {
        let events: EventLog = Arc::new(Mutex::new(Vec::new()));
        let store = FakeStore {
            inner: Arc::default(),
            events: Arc::clone(&events),
        };
        let units = FakeUnits {
            inner: Arc::default(),
            events: Arc::clone(&events),
        };
        let now = Arc::new(AtomicU64::new(1_000));
        let registry = DetachedRegistry::new(
            Arc::new(units.clone()),
            Arc::new(store.clone()),
            Arc::new(FakeClock {
                now: Arc::clone(&now),
            }),
            Arc::new(FakeSleeper {
                now: Arc::clone(&now),
                step,
            }),
            Arc::new(SeqIds {
                next: AtomicU64::new(1),
            }),
            RegistryConfig {
                paths: RunnerUnitPaths::new(RUNNER_PATH),
                boot_id: "boot-A".to_owned(),
                max_runtime_sec: 0,
                exec_user: "alice".to_owned(),
                exec_uid: 1000,
                systemd_run_path: "/run/current-system/sw/bin/systemd-run".to_owned(),
                login_shell_path: "/run/current-system/sw/bin/bash".to_owned(),
            },
        );
        Harness {
            registry,
            store,
            units,
            now,
            events,
        }
    }

    fn command() -> ValidatedCommand {
        command_with_program("/bin/sleep")
    }

    fn command_with_program(program: &str) -> ValidatedCommand {
        ValidatedCommand {
            program: program.into(),
            args: vec!["3600".to_owned()],
            cwd: "/".into(),
            env: Vec::new(),
            direct_workload_tty: false,
        }
    }

    fn encode_spec_for_slot(slot: u32) -> Vec<u8> {
        let spec = build_spec(
            &command(),
            &DetachedCaps::standard(0),
            &test_registry_config(),
            slot,
        )
        .expect("default seeded spec");
        d2b_exec_runner::spec::SpecCodec::encode(&spec).expect("encode seeded spec")
    }

    fn workload_exec_start(slot: u32) -> String {
        let spec = build_spec(
            &command(),
            &DetachedCaps::standard(0),
            &test_registry_config(),
            slot,
        )
        .expect("default workload spec");
        let mut rendered = Vec::with_capacity(5 + spec.argv.len());
        rendered.push(spec.login_shell_path.clone());
        rendered.push("-l".to_owned());
        rendered.push("-c".to_owned());
        rendered.push(r#"exec "$@""#.to_owned());
        rendered.push("d2b-exec".to_owned());
        rendered.extend(spec.argv);
        // Render argv[] the way real systemd does: a literal single-space join
        // with NO escaping of embedded spaces/quotes (so the `-c` value
        // `exec "$@"` appears verbatim as `... -c exec "$@" d2b-exec ...`). This
        // mirrors `systemctl show -p ExecStart` output so the identity tests
        // exercise the real, lossy rendering rather than an idealized escaped one.
        let argv = rendered.join(" ");
        format!(
            "{{ path={} ; argv[]={argv} ; ignore_errors=no }}",
            spec.login_shell_path
        )
    }

    fn test_registry_config() -> RegistryConfig {
        RegistryConfig {
            paths: RunnerUnitPaths::new(RUNNER_PATH),
            boot_id: "boot-A".to_owned(),
            max_runtime_sec: 0,
            exec_user: "alice".to_owned(),
            exec_uid: 1000,
            systemd_run_path: "/run/current-system/sw/bin/systemd-run".to_owned(),
            login_shell_path: "/run/current-system/sw/bin/bash".to_owned(),
        }
    }

    fn meta(end: u64, dropped: u64, truncated: bool, eof: bool) -> StreamMeta {
        StreamMeta {
            cap: DETACHED_STREAM_LOG_BYTES,
            start_offset: dropped,
            end_offset: end,
            dropped_bytes: dropped,
            truncated,
            eof,
            lost: false,
        }
    }

    #[tokio::test]
    async fn create_success_started_resolves_running() {
        let h = harness();
        // The runner publishes `started` before guestd's first poll.
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, snapshot) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .expect("create");
        assert_eq!(id, format!("{:032x}", 1));
        assert_eq!(snapshot.state, ExecState::Running);
        let list = h.registry.list("boot-A").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].exec_id, id);
        assert_eq!(list[0].slot, 0);
    }

    #[tokio::test]
    async fn requested_exec_id_replays_without_duplicate_spawn() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let requested = "0123456789abcdef0123456789abcdef".to_owned();
        let (first_id, first) = h
            .registry
            .create_with_exec_id(
                "boot-A",
                command(),
                DetachedCaps::standard(0),
                requested.clone(),
            )
            .await
            .unwrap();
        let (second_id, second) = h
            .registry
            .create_with_exec_id(
                "boot-A",
                command(),
                DetachedCaps::standard(0),
                requested.clone(),
            )
            .await
            .unwrap();

        assert_eq!(first_id, requested);
        assert_eq!(second_id, first_id);
        assert_eq!(second.state, first.state);
        assert_eq!(h.registry.list("boot-A").await.unwrap().len(), 1);
        assert_eq!(
            h.registry
                .create_with_exec_id(
                    "boot-A",
                    command_with_program("/bin/false"),
                    DetachedCaps::standard(0),
                    requested,
                )
                .await,
            Err(ExecError::InvalidArgv)
        );
    }

    #[tokio::test]
    async fn concurrent_requested_exec_id_reserves_only_one_slot() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let requested = "fedcba9876543210fedcba9876543210".to_owned();
        let first = h.registry.create_with_exec_id(
            "boot-A",
            command(),
            DetachedCaps::standard(0),
            requested.clone(),
        );
        let second = h.registry.create_with_exec_id(
            "boot-A",
            command(),
            DetachedCaps::standard(0),
            requested,
        );
        let (first, second) = tokio::join!(first, second);

        assert!(first.is_ok() || second.is_ok());
        for result in [&first, &second] {
            if let Err(error) = result {
                assert_eq!(*error, ExecError::ExecClosing);
            }
        }
        assert_eq!(h.registry.list("boot-A").await.unwrap().len(), 1);
    }

    #[test]
    fn workload_systemd_run_args_mirror_login_session_and_never_root() {
        let config = test_registry_config();
        let spec = build_spec(
            &command_with_program("id"),
            &DetachedCaps::standard(0),
            &config,
            7,
        )
        .unwrap();
        assert_eq!(spec.workload_unit_name, "d2b-exec-07-w.service");
        assert_eq!(spec.exec_user, "alice");
        assert_eq!(spec.exec_uid, 1000);
        assert!(spec.systemd_run_args.contains(&"--uid=alice".to_owned()));
        assert!(
            spec.systemd_run_args
                .contains(&"--unit=d2b-exec-07-w.service".to_owned())
        );
        assert!(
            spec.systemd_run_args
                .contains(&"--slice=d2b-exec.slice".to_owned())
        );
        assert!(
            spec.systemd_run_args
                .contains(&"--property=PAMName=login".to_owned())
        );
        assert!(
            spec.systemd_run_args
                .contains(&"--property=BindsTo=d2b-exec-07.service".to_owned())
        );
        assert!(
            spec.systemd_run_args
                .contains(&"--property=PartOf=d2b-exec-07.service".to_owned())
        );
        assert!(
            spec.systemd_run_args
                .contains(&"--property=After=d2b-exec-07.service".to_owned())
        );
        assert!(
            spec.systemd_run_args
                .contains(&"--expand-environment=no".to_owned())
        );
        assert!(!spec.systemd_run_args.iter().any(|arg| arg == "User=root"));
        let sentinel = spec
            .systemd_run_args
            .iter()
            .position(|arg| arg == "d2b-exec")
            .expect("positional shell sentinel");
        assert_eq!(spec.systemd_run_args[sentinel + 1], "id");
    }

    #[test]
    fn detached_bare_absolute_and_relative_programs_flow_to_wrapper() {
        let config = test_registry_config();
        for program in ["id", "/bin/true", "./script"] {
            let spec = build_spec(
                &command_with_program(program),
                &DetachedCaps::standard(0),
                &config,
                3,
            )
            .unwrap();
            let sentinel = spec
                .systemd_run_args
                .iter()
                .position(|arg| arg == "d2b-exec")
                .expect("positional shell sentinel");
            assert_eq!(spec.systemd_run_args[sentinel + 1], program);
            assert_eq!(spec.argv[0], program);
        }
    }

    #[tokio::test]
    async fn create_spawn_failed_is_terminal_retained_with_id() {
        let h = harness();
        h.store.set_status(0, StatusPhase::SpawnFailed);
        let (id, snapshot) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .expect("create");
        assert_eq!(snapshot.state, ExecState::Exited);
        assert_eq!(snapshot.outcome, Some(ExitOutcome::Exited(127)));
        // Retained + discoverable.
        let inspected = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(inspected.state, ExecState::Exited);
    }

    #[tokio::test]
    async fn create_infra_failed_fails_create_with_no_visible_id() {
        let h = harness();
        h.store.set_status(0, StatusPhase::InfraFailed);
        let err = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::RetainedLogPathUnsafe);
        // Nothing visible, slot freed.
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        assert!(!h.store.slot_exists(0));
    }

    #[tokio::test]
    async fn create_unit_start_failure_releases_everything() {
        let h = harness();
        h.units.inner.lock().unwrap().fail_start = true;
        let err = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::Internal);
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_timeout_with_live_unit_resolves_running_not_killed() {
        let h = harness();
        // No status ever; unit is live → create resolves Running after timeout.
        h.units.set_live(0, true);
        let (id, snapshot) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .expect("create");
        assert_eq!(snapshot.state, ExecState::Running);
        assert!(
            !h.units.stopped(0),
            "a live unit is never killed on timeout"
        );
        assert!(h.registry.inspect(&id, "boot-A").await.is_ok());
    }

    #[tokio::test]
    async fn create_timeout_with_no_unit_fails() {
        let h = harness();
        // No status, no live unit → create fails after timeout, slot released.
        let err = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::Internal);
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn boot_mismatch_is_stale_session() {
        let h = harness();
        let err = h
            .registry
            .create("boot-OTHER", command(), DetachedCaps::standard(0))
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::StaleSession);
    }

    #[tokio::test]
    async fn capacity_exhaustion_fails_closed() {
        let h = harness();
        // Fill all active slots with live running execs.
        for slot in 0..DETACHED_ACTIVE_PER_VM as u32 {
            h.units.set_live(slot, true);
            h.store.set_status(slot, StatusPhase::Started);
            h.registry
                .create("boot-A", command(), DetachedCaps::standard(0))
                .await
                .expect("create");
        }
        let err = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::ExecCapacityExceeded);
    }

    #[tokio::test]
    async fn cancel_two_phase_writes_sentinel_before_stop_unit() {
        let h = harness_with_clock_step(CANCEL_DEADLINE_MS + 1);
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        // The runner never publishes terminal status; cancel must write the
        // sentinel first, then fall back to stop_unit after the deadline.
        let duplicate = h.registry.cancel(&id, "boot-A").await.unwrap();
        assert!(!duplicate);
        assert!(h.store.cancel_written(0), "sentinel written");
        assert!(h.units.stopped(0), "stop_unit backstop after deadline");
        // Still no status → marked lost.
        let snapshot = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snapshot.state, ExecState::LostGuestd);
    }

    #[tokio::test]
    async fn cancel_deadline_stop_failure_keeps_live_exec_running() {
        let h = harness_with_clock_step(CANCEL_DEADLINE_MS + 1);
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        h.store.inner.lock().unwrap().status.remove(&0);
        h.units.set_fail_stop(true);

        let duplicate = h.registry.cancel(&id, "boot-A").await.unwrap();

        assert!(!duplicate);
        assert!(h.store.cancel_written(0), "cancel sentinel still written");
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::Running,
            "stop failure + fresh live liveness must not advertise LostGuestd"
        );
        assert_eq!(h.registry.active_count(), 1, "active reservation retained");
        assert!(
            h.units.inner.lock().unwrap().workload_live.contains(&0),
            "workload unit remains live for a later retry"
        );
    }

    #[tokio::test]
    async fn cancel_resolves_on_terminal_status_and_cleans_units() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        // Runner promptly reacts to the cancel sentinel by publishing a
        // terminal status during the phase-2 wait.
        h.store.set_cancel_terminal(StatusPhase::Cancelled);
        let duplicate = h.registry.cancel(&id, "boot-A").await.unwrap();
        assert!(!duplicate);
        assert!(h.store.cancel_written(0));
        assert!(h.units.stopped(0), "terminal status still tears down units");
        // Idempotent: second cancel is a duplicate.
        assert!(h.registry.cancel(&id, "boot-A").await.unwrap());
    }

    #[tokio::test]
    async fn live_reconciliation_marks_vanished_unit_lost() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        // Unit disappears with no terminal status.
        h.units.set_live(0, false);
        let snapshot = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snapshot.state, ExecState::LostGuestd);
        // Active counter released but slot+record retained (still listable).
        let list = h.registry.list("boot-A").await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn live_reconciliation_adopts_exited_status_and_stops_units() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.store.set_status(0, StatusPhase::Exited(42));

        let snapshot = h.registry.inspect(&id, "boot-A").await.unwrap();

        assert_eq!(snapshot.state, ExecState::Exited);
        assert_eq!(snapshot.outcome, Some(ExitOutcome::Exited(42)));
        assert!(h.units.stopped(0), "terminal transition tears down units");
        assert_eq!(h.registry.active_count(), 0);
    }

    #[tokio::test]
    async fn wait_adopts_signaled_status_and_stops_units() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        let initial = h.registry.inspect(&id, "boot-A").await.unwrap();
        h.store.set_status(0, StatusPhase::Signaled(9));

        let (snapshot, timed_out) = h
            .registry
            .wait(&id, "boot-A", Some(initial.state_generation), 60_000)
            .await
            .unwrap();

        assert!(!timed_out);
        assert_eq!(snapshot.state, ExecState::Signaled);
        assert_eq!(snapshot.outcome, Some(ExitOutcome::Signaled(9)));
        assert!(h.units.stopped(0), "terminal transition tears down units");
        assert_eq!(h.registry.active_count(), 0);
    }

    #[tokio::test]
    async fn runner_crash_with_workload_unit_live_is_cleaned_before_lost() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        h.units.set_runner_live(0, false);
        h.units.set_workload_live(0, true);
        h.store.inner.lock().unwrap().status.remove(&0);

        h.registry.reap_once().await;

        assert!(h.units.stopped(0), "orphan workload unit is stopped");
        assert!(!h.units.inner.lock().unwrap().workload_live.contains(&0));
        let snapshot = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snapshot.state, ExecState::LostGuestd);
    }

    #[tokio::test]
    async fn wait_terminates_when_unit_vanishes() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, snapshot) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        h.units.set_live(0, false);
        let (final_snapshot, timed_out) = h
            .registry
            .wait(&id, "boot-A", Some(snapshot.state_generation), 60_000)
            .await
            .unwrap();
        assert!(!timed_out);
        assert_eq!(final_snapshot.state, ExecState::LostGuestd);
    }

    #[tokio::test]
    async fn indefinite_running_record_is_not_reaped_by_ttl() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        // Advance the clock far past the TTL horizon; the unit stays live.
        h.now
            .store(1_000 + RETENTION_TTL_MS * 100, Ordering::SeqCst);
        h.registry.reap_once().await;
        let snapshot = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snapshot.state, ExecState::Running, "indefinite runtime");
    }

    #[tokio::test]
    async fn terminal_record_is_gc_expired_to_tombstone() {
        let h = harness();
        h.store.set_status(0, StatusPhase::Exited(0));
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        // Past TTL → GC removes it and a future lookup is ExecExpired.
        h.now.store(1_000 + RETENTION_TTL_MS + 1, Ordering::SeqCst);
        h.registry.reap_once().await;
        let err = h.registry.inspect(&id, "boot-A").await.unwrap_err();
        assert_eq!(err, ExecError::ExecExpired);
    }

    #[tokio::test]
    async fn logs_serve_bytes_and_report_truncation() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        h.store
            .set_stdout(0, b"hello world", meta(11, 0, false, true));
        let chunk = h
            .registry
            .read_logs(&id, "boot-A", RtStream::Stdout, 0, 1024)
            .await
            .unwrap();
        assert_eq!(chunk.data, b"hello world");
        assert!(chunk.eof);
    }

    #[tokio::test]
    async fn logs_list_and_status_preserve_dropped_and_truncated_metadata() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.store.set_stdout(0, b"hi", meta(7, 5, true, false));
        h.store.set_stderr_meta(0, meta(4, 2, true, false));

        let chunk = h
            .registry
            .read_logs(&id, "boot-A", RtStream::Stdout, 5, 1024)
            .await
            .unwrap();
        assert_eq!(chunk.data, b"hi");
        assert_eq!(chunk.start_offset, 5);
        assert_eq!(chunk.end_offset, 7);
        assert_eq!(chunk.dropped_bytes, 5);
        assert!(chunk.truncated);

        let snapshot = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snapshot.stdout_dropped_bytes, 5);
        assert_eq!(snapshot.stderr_dropped_bytes, 2);
        assert!(snapshot.stdout_truncated);
        assert!(snapshot.stderr_truncated);

        let list = h.registry.list("boot-A").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].dropped_bytes, 7);
        assert!(list[0].stdout_truncated);
        assert!(list[0].stderr_truncated);
    }

    #[tokio::test]
    async fn logs_offset_in_future_is_typed() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_status(0, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        h.store.set_stdout(0, b"abc", meta(3, 0, false, false));
        let err = h
            .registry
            .read_logs(&id, "boot-A", RtStream::Stdout, 10, 16)
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::OffsetInFuture);
    }

    #[tokio::test]
    async fn unknown_id_is_not_found_evicted_is_expired() {
        let h = harness();
        assert_eq!(
            h.registry.inspect("ff", "boot-A").await.unwrap_err(),
            ExecError::ExecNotFound
        );
    }

    #[tokio::test]
    async fn readoption_adopts_live_dispatching_unit_as_running() {
        let h = harness();
        // Seed a Dispatching record + a live unit (crash-after-registration).
        let record = DurableRecord {
            exec_id: format!("{:032x}", 7),
            slot: 5,
            boot_id: "boot-A".to_owned(),
            create_time_unix: 1_000,
            dispatch_deadline_unix: 1_000 + DISPATCH_DEADLINE_MS,
            argv_sha256: "x".repeat(64),
            state: RecordState::Dispatching,
            exit_code: None,
            term_signal: None,
            lost: false,
            terminal_time_unix: None,
        };
        h.store.seed_record(5, &record);
        h.units.set_live(5, true);
        h.registry.reconcile_on_startup().await;
        let snapshot = h
            .registry
            .inspect(&format!("{:032x}", 7), "boot-A")
            .await
            .unwrap();
        assert_eq!(snapshot.state, ExecState::Running);
        assert!(!h.units.stopped(5), "adopted unit not killed");
    }

    #[tokio::test]
    async fn readoption_requires_workload_unit_in_exec_slice() {
        let h = harness();
        h.store.seed_record(4, &rec(4, 70, RecordState::Running, 0));
        h.units.set_runner_live(4, true);
        h.units.set_workload_live(4, true);
        h.units.set_workload_mismatch(4);
        h.registry.reconcile_on_startup().await;
        assert!(h.units.stopped(4), "bad workload unit is torn down");
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 70), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
    }

    #[tokio::test]
    async fn readoption_requires_workload_exec_start_to_match_persisted_spec() {
        let h = harness();
        h.store.seed_record(4, &rec(4, 71, RecordState::Running, 0));
        h.units.set_runner_live(4, true);
        h.units.set_workload_live(4, true);
        h.units.set_workload_identity(
            4,
            UnitIdentity::Queried {
                slice: Some("d2b-exec.slice".to_owned()),
                exec_start: Some(
                    "{ path=/bin/sh ; argv[]=/bin/sh -c /bin/evil ; ignore_errors=no }".to_owned(),
                ),
                binds_to: Some(unit_name(4)),
                part_of: Some(unit_name(4)),
                after: Some(unit_name(4)),
            },
        );

        h.registry.reconcile_on_startup().await;

        assert!(
            h.units.stopped(4),
            "same-slice workload with wrong ExecStart is torn down"
        );
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 71), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
    }

    #[test]
    fn exec_start_raw_fields_handles_lossy_systemd_argv_bytes() {
        use crate::detached::exec_start_raw_fields;

        // Helper: build the expected raw argv[] string the same lossy way systemd
        // renders it (literal single-space join, no escaping).
        fn expected_raw(exe: &str, user_argv: &[&str]) -> String {
            let mut v = vec![
                exe.to_owned(),
                "-l".to_owned(),
                "-c".to_owned(),
                r#"exec "$@""#.to_owned(),
                "d2b-exec".to_owned(),
            ];
            v.extend(user_argv.iter().map(|s| (*s).to_owned()));
            v.join(" ")
        }

        let exe = "/run/current-system/sw/bin/bash";

        // Real systemd renders the `-c` value `exec "$@"` UNescaped and
        // space-joined: `... -c exec "$@" d2b-exec ...`. Raw extraction must
        // recover it verbatim (this is what was reaping our own live workloads).
        let (path, argv) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec /bin/id ; ignore_errors=no ; start_time=[n/a] }"#,
        )
        .expect("extract raw fields");
        assert_eq!(path, exe);
        assert_eq!(argv, expected_raw(exe, &["/bin/id"]));

        // A user argument containing a literal double quote (which the token
        // parser would reject as an unmatched quote, reaping a valid job) is
        // preserved byte-for-byte by raw extraction.
        let (_, argv) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec grep " /etc/hosts ; ignore_errors=no }"#,
        )
        .expect("extract raw fields");
        assert_eq!(argv, expected_raw(exe, &["grep", "\"", "/etc/hosts"]));

        // A user argument ending in a backslash (rejected by the token parser as
        // a dangling escape) is preserved.
        let (_, argv) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec echo back\ ; ignore_errors=no }"#,
        )
        .expect("extract raw fields");
        assert_eq!(argv, expected_raw(exe, &["echo", "back\\"]));

        // A user argument containing a semicolon (the common `sh -c 'a ; b'`
        // pattern) must NOT truncate at the `;`: extraction is bounded by the
        // fixed ` ; ignore_errors=` field, so the whole `a ; b` is preserved.
        // (A bare `;` split would fail-OPEN by comparing only the prefix `a`.)
        let (_, argv) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec sh -c echo a ; echo b ; ignore_errors=no }"#,
        )
        .expect("extract raw fields");
        assert_eq!(argv, expected_raw(exe, &["sh", "-c", "echo a ; echo b"]));

        // A user argument containing the EXACT field delimiter ` ; ignore_errors=`
        // is preserved in full: we bound on the LAST occurrence (systemd's real
        // field), so the user's copy stays in the recovered argv and matches a
        // persisted spec that carries it.
        let (_, argv) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec sh -c echo a ; ignore_errors=evil ; ignore_errors=no ; start_time=[n/a] }"#,
        )
        .expect("extract raw fields");
        assert_eq!(
            argv,
            expected_raw(exe, &["sh", "-c", "echo a ; ignore_errors=evil"])
        );
        // ...and a DIFFERENT persisted command must NOT match it (fail closed):
        // a spec for just `echo a` does not equal the recovered, full argv.
        assert_ne!(argv, expected_raw(exe, &["sh", "-c", "echo a"]));

        // A final argument's trailing whitespace is significant and preserved
        // (the recovered argv is not trimmed).
        let (_, argv) = exec_start_raw_fields(
            "{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec \"$@\" d2b-exec echo hi  ; ignore_errors=no }",
        )
        .expect("extract raw fields");
        assert_eq!(argv, expected_raw(exe, &["echo", "hi "]));

        // Distinct semicolon suffixes are NOT conflated, both at the rendering
        // layer and when driven through real extraction.
        assert_ne!(
            expected_raw(exe, &["sh", "-c", "echo a ; echo b"]),
            expected_raw(exe, &["sh", "-c", "echo a ; echo c"]),
        );
        let (_, argv_b) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec sh -c echo a ; echo b ; ignore_errors=no }"#,
        )
        .expect("extract raw fields");
        let (_, argv_c) = exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec sh -c echo a ; echo c ; ignore_errors=no }"#,
        )
        .expect("extract raw fields");
        assert_ne!(argv_b, argv_c);

        // Missing fields => None (treated as a mismatch, never a match):
        // no path, no argv[], and path-present-but-argv-absent.
        assert!(exec_start_raw_fields("{ argv[]=x ; ignore_errors=no }").is_none());
        assert!(exec_start_raw_fields("{ path=/x ; argv[]=/x ; }").is_none());
        assert!(exec_start_raw_fields("{ path=/x ; ignore_errors=no }").is_none());

        // A truncated / line-split ExecStart (no closing `}`, e.g. a foreign
        // unit whose argv newline split the `systemctl show` property across
        // lines) MUST fail closed — even though it carries a `; ignore_errors=`
        // tail that would otherwise let a recovered prefix match a shorter
        // persisted argv.
        assert!(exec_start_raw_fields(
            r#"{ path=/run/current-system/sw/bin/bash ; argv[]=/run/current-system/sw/bin/bash -l -c exec "$@" d2b-exec sh -c echo a ; ignore_errors=evil"#,
        )
        .is_none());
        // No leading brace (e.g. a captured continuation line) also fails closed.
        assert!(
            exec_start_raw_fields(
                "path=/x ; argv[]=/x -l -c exec \"$@\" d2b-exec id ; ignore_errors=no }"
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn readoption_requires_workload_dependencies_to_bind_runner_unit() {
        let h = harness();
        h.store.seed_record(4, &rec(4, 72, RecordState::Running, 0));
        h.units.set_runner_live(4, true);
        h.units.set_workload_live(4, true);
        h.units.set_workload_identity(
            4,
            UnitIdentity::Queried {
                slice: Some("d2b-exec.slice".to_owned()),
                exec_start: Some(workload_exec_start(4)),
                binds_to: Some("d2b-exec-99.service".to_owned()),
                part_of: Some(unit_name(4)),
                after: Some(unit_name(4)),
            },
        );

        h.registry.reconcile_on_startup().await;

        assert!(
            h.units.stopped(4),
            "workload missing BindsTo/PartOf/After to the runner is torn down"
        );
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 72), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
    }

    #[tokio::test]
    async fn readoption_deletes_past_deadline_no_unit() {
        let h = harness();
        let record = DurableRecord {
            exec_id: format!("{:032x}", 8),
            slot: 6,
            boot_id: "boot-A".to_owned(),
            create_time_unix: 0,
            dispatch_deadline_unix: 10,
            argv_sha256: "x".repeat(64),
            state: RecordState::Dispatching,
            exit_code: None,
            term_signal: None,
            lost: false,
            terminal_time_unix: None,
        };
        h.store.seed_record(6, &record);
        h.now.store(1_000_000, Ordering::SeqCst);
        h.registry.reconcile_on_startup().await;
        // Deleted + released, no visible id.
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        assert!(!h.store.slot_exists(6));
    }

    #[tokio::test]
    async fn readoption_quarantines_wrong_boot() {
        let h = harness();
        let record = DurableRecord {
            exec_id: format!("{:032x}", 9),
            slot: 7,
            boot_id: "boot-OLD".to_owned(),
            create_time_unix: 0,
            dispatch_deadline_unix: 0,
            argv_sha256: "x".repeat(64),
            state: RecordState::Running,
            exit_code: None,
            term_signal: None,
            lost: false,
            terminal_time_unix: None,
        };
        h.store.seed_record(7, &record);
        h.registry.reconcile_on_startup().await;
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        assert!(!h.store.slot_exists(7));
    }

    #[tokio::test]
    async fn quota_invariant_holds() {
        assert_eq!(
            DETACHED_LOG_QUOTA_BYTES,
            DETACHED_RETAINED_PER_VM as u64 * 2 * DETACHED_STREAM_LOG_BYTES
        );
    }

    // ---- regression tests --------------------------------

    /// A record seeded for re-adoption with arbitrary state/deadline.
    fn rec(slot: u32, id: u64, state: RecordState, dispatch_deadline_unix: u64) -> DurableRecord {
        DurableRecord {
            exec_id: format!("{id:032x}"),
            slot,
            boot_id: "boot-A".to_owned(),
            create_time_unix: 1_000,
            dispatch_deadline_unix,
            argv_sha256: "x".repeat(64),
            state,
            exit_code: None,
            term_signal: None,
            lost: false,
            terminal_time_unix: None,
        }
    }

    async fn create_live_running(h: &Harness, slot: u32) -> String {
        h.units.set_live(slot, true);
        h.store.set_status(slot, StatusPhase::Started);
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .expect("create");
        id
    }

    // A transient liveness QUERY error must be Unknown, never Absent — a
    // live exec must not be marked lost on a flaky `systemctl`.
    #[tokio::test]
    async fn f1_query_error_does_not_mark_running_lost() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.units.set_fail_list(true);
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::Running,
            "query error must be Unknown (retain), not Absent (lost)"
        );
        // The periodic reaper path must behave identically.
        h.registry.reap_once().await;
        assert_eq!(
            h.registry.inspect(&id, "boot-A").await.unwrap().state,
            ExecState::Running
        );
    }

    // A merely-loaded (inactive/failed) unit is NOT live.
    #[tokio::test]
    async fn f1_inactive_unit_is_not_live() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.units.set_inactive(0);
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::LostGuestd,
            "inactive unit ⇒ not live"
        );
    }

    // An identity mismatch (foreign command at our slot) is not adoptable.
    #[tokio::test]
    async fn f1_identity_mismatch_unit_is_foreign() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.units.set_mismatch(0);
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
        assert!(h.units.stopped(0), "impostor unit at our slot is stopped");
    }

    // An ACTIVE unit whose `systemctl show` identity enrichment FAILED
    // is UNKNOWN, never Foreign — a transient identity-query failure must NOT
    // stop the unit or mark a possibly-live exec lost. Covers both the
    // on-access and the periodic-reaper reconciliation paths.
    #[tokio::test]
    async fn g1_active_unit_with_show_failure_is_unknown_not_lost() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        // The unit stays active/live, but its identity can no longer be read.
        h.units.set_show_fail(0);
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::Running,
            "active unit + show-failure ⇒ Unknown (retain), never Foreign (lost)"
        );
        assert!(
            !h.units.stopped(0),
            "an unqueried-identity unit must NOT be torn down"
        );
        assert_eq!(h.registry.active_count(), 1, "active reservation retained");
        // The periodic reaper path must behave identically (no destructive
        // reconciliation on a query failure).
        h.registry.reap_once().await;
        assert_eq!(
            h.registry.inspect(&id, "boot-A").await.unwrap().state,
            ExecState::Running
        );
        assert!(!h.units.stopped(0));
        assert_eq!(h.registry.active_count(), 1);
    }

    // Identity verification is STRUCTURAL, not substring-based. An
    // impostor that merely embeds the runner path / `--serve-exec` / `--slot
    // NN` as substrings of unrelated args — while running a DIFFERENT exe or a
    // DIFFERENT slot — is rejected (Foreign), where a naive `contains` check
    // would wrongly accept it. An authentic argv is accepted (Live).
    #[tokio::test]
    async fn g2_structural_identity_rejects_substring_impostor() {
        // Case 1: wrong executable, but the runner path is embedded as a decoy
        // argument so a substring check would falsely match. (create allocates
        // the lowest free slot — 0 — on a fresh harness.)
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.units.set_identity(
            0,
            UnitIdentity::Queried {
                slice: Some("d2b-exec.slice".to_owned()),
                exec_start: Some(format!(
                    "{{ path=/usr/bin/evil ; argv[]=/usr/bin/evil --decoy={RUNNER_PATH} \
                     --serve-exec --slot 00 ; ignore_errors=no }}"
                )),
                binds_to: Some(unit_name(0)),
                part_of: Some(unit_name(0)),
                after: Some(unit_name(0)),
            },
        );
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::LostGuestd,
            "wrong-exe impostor rejected"
        );
        assert!(h.units.stopped(0), "impostor unit is stopped");

        // Case 2: correct executable, but the slot token only appears as a
        // substring of an unrelated arg; the real `--slot` is a DIFFERENT slot.
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.units.set_identity(
            0,
            UnitIdentity::Queried {
                slice: Some("d2b-exec.slice".to_owned()),
                exec_start: Some(format!(
                    "{{ path={RUNNER_PATH} ; argv[]={RUNNER_PATH} --serve-exec \
                     --decoy=--slot 00 --slot 99 ; ignore_errors=no }}"
                )),
                binds_to: Some(unit_name(0)),
                part_of: Some(unit_name(0)),
                after: Some(unit_name(0)),
            },
        );
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::LostGuestd,
            "wrong-slot impostor rejected"
        );
        assert!(h.units.stopped(0));

        // Authentic argv (exe == runner, distinct --serve-exec + --slot NN
        // tokens) is accepted as Live.
        let h = harness();
        let id = create_live_running(&h, 0).await;
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(
            snap.state,
            ExecState::Running,
            "authentic structural identity is accepted"
        );
        assert!(!h.units.stopped(0));
    }

    // Live reconciliation must RELEASE the active-concurrency counter (not just
    // retain the record/logs) on access — otherwise active capacity leaks until
    // a guestd restart. After the vanish, a full fresh batch of active execs
    // must fit.
    #[tokio::test]
    async fn live_reconciliation_on_access_releases_active_counter() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        assert_eq!(h.registry.active_count(), 1);
        h.units.set_live(0, false);
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
        assert_eq!(
            h.registry.active_count(),
            0,
            "active counter released on access (slot+logs retained)"
        );
        // A full active batch fits because the lost exec freed its active slot.
        for slot in 1..=DETACHED_ACTIVE_PER_VM as u32 {
            create_live_running(&h, slot).await;
        }
        assert_eq!(h.registry.active_count(), DETACHED_ACTIVE_PER_VM as u32);
    }

    // Same invariant via the PERIODIC reaper path.
    #[tokio::test]
    async fn periodic_reaper_releases_active_counter() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        assert_eq!(h.registry.active_count(), 1);
        h.units.set_live(0, false);
        h.registry.reap_once().await;
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
        assert_eq!(
            h.registry.active_count(),
            0,
            "active counter released by the reaper (slot+logs retained)"
        );
    }

    // A crash-recovered Dispatching record within its deadline is held —
    // non-listable, non-inspectable — but the slot dir is retained.
    #[tokio::test]
    async fn f2_crash_dispatching_within_deadline_is_held_nonlistable() {
        let h = harness();
        h.store.seed_record(
            3,
            &rec(
                3,
                21,
                RecordState::Dispatching,
                1_000 + DISPATCH_DEADLINE_MS,
            ),
        );
        h.registry.reconcile_on_startup().await;
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        assert_eq!(
            h.registry
                .inspect(&format!("{:032x}", 21), "boot-A")
                .await
                .unwrap_err(),
            ExecError::ExecNotFound
        );
        assert!(h.store.slot_exists(3), "the hold retains the slot dir");
    }

    // A late-registering unit for a held dispatch is promoted to Running by
    // the reaper (the slot does not leak as a forever-hidden hold).
    #[tokio::test]
    async fn f2_late_unit_promotes_hold_to_running() {
        let h = harness();
        h.store.seed_record(
            3,
            &rec(
                3,
                22,
                RecordState::Dispatching,
                1_000 + DISPATCH_DEADLINE_MS,
            ),
        );
        h.registry.reconcile_on_startup().await;
        // The forked systemd-run finally registers the unit.
        h.units.set_live(3, true);
        h.registry.reap_once().await;
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 22), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::Running);
        assert_eq!(
            h.registry.list("boot-A").await.unwrap().len(),
            1,
            "promoted hold is now listable"
        );
    }

    // A held dispatch whose unit never registers is deleted + released once
    // the dispatch deadline passes (NOT skipped forever like a Creating guard).
    #[tokio::test]
    async fn f2_held_dispatch_deleted_after_deadline_passes() {
        let h = harness();
        h.store.seed_record(
            3,
            &rec(
                3,
                24,
                RecordState::Dispatching,
                1_000 + DISPATCH_DEADLINE_MS,
            ),
        );
        h.registry.reconcile_on_startup().await;
        assert!(h.store.slot_exists(3));
        h.now
            .store(1_000 + DISPATCH_DEADLINE_MS + 1, Ordering::SeqCst);
        h.registry.reap_once().await;
        assert!(!h.store.slot_exists(3), "past-deadline hold deleted");
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        // Slot + active fully released: a full batch of fresh creates succeeds.
        for slot in 0..DETACHED_ACTIVE_PER_VM as u32 {
            create_live_running(&h, slot).await;
        }
    }

    // A past-deadline dispatch hold whose slot-dir unlink FAILS must NOT
    // free the slot for reuse with stale files on disk. It stays a hidden
    // retryable hold; once the unlink succeeds a later reaper pass frees it.
    #[tokio::test]
    async fn g3_held_dispatch_delete_failure_retains_hidden_entry_for_retry() {
        let h = harness();
        h.store.seed_record(
            3,
            &rec(
                3,
                24,
                RecordState::Dispatching,
                1_000 + DISPATCH_DEADLINE_MS,
            ),
        );
        h.registry.reconcile_on_startup().await;
        h.now
            .store(1_000 + DISPATCH_DEADLINE_MS + 1, Ordering::SeqCst);
        // The unlink fails: the slot must remain reserved (hidden), NOT freed.
        h.store.set_fail_delete(true);
        h.registry.reap_once().await;
        assert!(
            h.store.slot_exists(3),
            "stale files retained on delete failure"
        );
        // Still hidden (never externally visible).
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        // A retry frees the slot once the unlink succeeds.
        h.store.set_fail_delete(false);
        h.registry.reap_once().await;
        assert!(!h.store.slot_exists(3), "slot freed once unlink succeeds");
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
    }

    // A startup re-adoption of a past-deadline Dispatching record whose
    // unlink FAILS keeps a hidden retryable entry (slot not freed with stale
    // files); when the unlink later succeeds, the reaper frees the slot.
    #[tokio::test]
    async fn g3_readoption_past_deadline_delete_failure_retains_then_frees() {
        let h = harness();
        // Past the dispatch deadline at adoption time, no unit, no status.
        h.store
            .seed_record(3, &rec(3, 24, RecordState::Dispatching, 500));
        h.now.store(1_000, Ordering::SeqCst);
        h.store.set_fail_delete(true);
        h.registry.reconcile_on_startup().await;
        assert!(
            h.store.slot_exists(3),
            "stale slot retained when re-adoption unlink fails"
        );
        assert!(
            h.registry.list("boot-A").await.unwrap().is_empty(),
            "hidden hold"
        );
        // Once the unlink works, the reaper frees the slot.
        h.store.set_fail_delete(false);
        h.now
            .store(1_000 + DISPATCH_DEADLINE_MS + 1, Ordering::SeqCst);
        h.registry.reap_once().await;
        assert!(!h.store.slot_exists(3), "slot freed once unlink succeeds");
    }

    // A persisted Running record with no unit + no terminal status is marked
    // lost and RETAINED (id + logs survive a guestd restart), never deleted.
    #[tokio::test]
    async fn f3_persisted_running_no_unit_is_lost_and_retained() {
        let h = harness();
        h.store.seed_record(
            2,
            &rec(2, 30, RecordState::Running, 1_000 + DISPATCH_DEADLINE_MS),
        );
        h.store.set_stdout(2, b"partial", meta(7, 0, false, false));
        h.registry.reconcile_on_startup().await;
        let id = format!("{:032x}", 30);
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
        // Logs retained + readable; slot retained.
        let chunk = h
            .registry
            .read_logs(&id, "boot-A", RtStream::Stdout, 0, 1024)
            .await
            .unwrap();
        assert_eq!(chunk.data, b"partial");
        assert!(h.store.slot_exists(2));
    }

    // Re-adoption matrix: terminal status with no unit ⇒ adopt terminal.
    #[tokio::test]
    async fn readoption_terminal_status_with_no_unit_adopts_terminal() {
        let h = harness();
        h.store.seed_record(1, &rec(1, 31, RecordState::Running, 0));
        h.store.set_status(1, StatusPhase::Exited(3));
        h.registry.reconcile_on_startup().await;
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 31), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::Exited);
        assert_eq!(snap.outcome, Some(ExitOutcome::Exited(3)));
    }

    // Re-adoption matrix: terminal status with a still-live unit ⇒ stop the
    // unit pair, then adopt terminal so no orphaned workload survives startup.
    #[tokio::test]
    async fn readoption_terminal_status_with_live_unit_adopts_terminal() {
        let h = harness();
        h.store.seed_record(1, &rec(1, 32, RecordState::Running, 0));
        h.store.set_status(1, StatusPhase::Signaled(9));
        h.units.set_live(1, true);
        h.registry.reconcile_on_startup().await;
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 32), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::Signaled);
        assert!(h.units.stopped(1));
    }

    #[tokio::test]
    async fn readoption_terminal_status_with_orphan_workload_stops_workload() {
        let h = harness();
        h.store
            .seed_record(1, &rec(1, 132, RecordState::Running, 0));
        h.store.set_status(1, StatusPhase::Exited(0));
        h.units.set_runner_live(1, false);
        h.units.set_workload_live(1, true);
        h.registry.reconcile_on_startup().await;
        let snap = h
            .registry
            .inspect(&format!("{:032x}", 132), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::Exited);
        assert!(h.units.stopped(1));
        assert!(!h.units.inner.lock().unwrap().workload_live.contains(&1));
    }

    // Re-adoption matrix: an infra-failed status at startup is quarantined.
    #[tokio::test]
    async fn readoption_infra_failed_is_quarantined() {
        let h = harness();
        h.store.seed_record(1, &rec(1, 33, RecordState::Running, 0));
        h.store.set_status(1, StatusPhase::InfraFailed);
        h.units.set_live(1, true);
        h.registry.reconcile_on_startup().await;
        assert!(!h.store.slot_exists(1));
        assert!(h.units.stopped(1));
    }

    // Re-adoption matrix: a live unit with no seeded record is an orphan ⇒ cleaned.
    #[tokio::test]
    async fn readoption_orphan_unit_without_record_is_cleaned() {
        let h = harness();
        h.units.set_live(4, true);
        h.registry.reconcile_on_startup().await;
        assert!(h.units.stopped(4), "orphan unit with no record is stopped");
    }

    #[tokio::test]
    async fn reaper_retries_global_orphan_sweep_after_startup_list_error() {
        let h = harness();
        h.units.set_live(4, true);
        h.units.set_fail_list(true);
        h.registry.reconcile_on_startup().await;
        assert!(
            !h.units.stopped(4),
            "startup query error skips destructive orphan cleanup"
        );

        h.units.set_fail_list(false);
        h.registry.reap_once().await;

        assert!(
            h.units.stopped(4),
            "periodic global sweep retries no-record orphan cleanup"
        );
    }

    // A slot that fails the authenticity gate is quarantined, never adopted.
    #[tokio::test]
    async fn f7_unsafe_slot_is_quarantined_on_readoption() {
        let h = harness();
        h.store.seed_record(2, &rec(2, 40, RecordState::Running, 0));
        h.units.set_live(2, true);
        h.store.set_fail_authenticity(2);
        h.registry.reconcile_on_startup().await;
        assert!(!h.store.slot_exists(2), "unsafe slot deleted");
        assert!(h.units.stopped(2), "unit for unsafe slot stopped");
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
    }

    // Re-adoption matrix: a durable `record` that passes the authenticity gate
    // but cannot be DECODED (corrupt/unreadable bytes) is quarantined — the
    // slot dir is deleted and nothing is adopted, never trusting corrupt bytes.
    #[tokio::test]
    async fn readoption_corrupt_record_is_quarantined() {
        let h = harness();
        // Authenticity passes (not in the fail set), but read_record → Err.
        h.store.seed_corrupt_record(1);
        h.registry.reconcile_on_startup().await;
        assert!(!h.store.slot_exists(1), "corrupt record slot deleted");
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        assert_eq!(
            h.registry.active_count(),
            0,
            "nothing reserved for a corrupt slot"
        );
    }

    #[tokio::test]
    async fn dispatch_hold_reads_terminal_status_before_liveness_delete() {
        let h = harness();
        h.store.seed_record(
            3,
            &rec(
                3,
                23,
                RecordState::Dispatching,
                1_000 + DISPATCH_DEADLINE_MS,
            ),
        );
        h.units.set_fail_list(true);
        h.registry.reconcile_on_startup().await;
        h.units.set_fail_list(false);
        h.store.set_status(3, StatusPhase::Exited(5));
        h.now
            .store(1_000 + DISPATCH_DEADLINE_MS + 1, Ordering::SeqCst);

        h.registry.reap_once().await;

        let snap = h
            .registry
            .inspect(&format!("{:032x}", 23), "boot-A")
            .await
            .unwrap();
        assert_eq!(snap.state, ExecState::Exited);
        assert_eq!(snap.outcome, Some(ExitOutcome::Exited(5)));
        assert!(
            h.store.slot_exists(3),
            "terminal record retained, not deleted"
        );
    }

    // A `read_status` error during create resolution must tear the create
    // down (stop unit + delete dir + release) rather than leak the reservation.
    #[tokio::test]
    async fn f6_create_read_status_error_releases_reservation() {
        let h = harness();
        h.units.set_live(0, true);
        h.store.set_fail_status(true);
        let err = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap_err();
        assert_eq!(err, ExecError::Internal);
        assert!(h.registry.list("boot-A").await.unwrap().is_empty());
        assert!(!h.store.slot_exists(0));
        assert!(h.units.stopped(0), "abort_create stops the unit");
        // Reservation released: a full batch of fresh creates succeeds.
        h.store.set_fail_status(false);
        for slot in 0..DETACHED_ACTIVE_PER_VM as u32 {
            create_live_running(&h, slot).await;
        }
    }

    // A failed slot-dir deletion during GC retains the entry (the slot is
    // never freed for reuse with stale files), and a later pass retries.
    #[tokio::test]
    async fn f12_gc_delete_failure_retains_entry_for_retry() {
        let h = harness();
        h.store.set_status(0, StatusPhase::Exited(0));
        let (id, _) = h
            .registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        h.now.store(1_000 + RETENTION_TTL_MS + 1, Ordering::SeqCst);
        h.store.set_fail_delete(true);
        h.registry.reap_once().await;
        assert!(
            h.registry.inspect(&id, "boot-A").await.is_ok(),
            "entry retained when deletion fails"
        );
        assert!(h.store.slot_exists(0));
        // Retry succeeds once deletion works.
        h.store.set_fail_delete(false);
        h.registry.reap_once().await;
        assert_eq!(
            h.registry.inspect(&id, "boot-A").await.unwrap_err(),
            ExecError::ExecExpired
        );
    }

    // A reused slot is scrubbed before the new record/spec are persisted.
    #[tokio::test]
    async fn f12_create_scrubs_slot_before_persist() {
        let h = harness();
        create_live_running(&h, 0).await;
        assert!(
            h.store.scrubbed().contains(&0),
            "persist_dispatch scrubs the slot before writing"
        );
    }

    // Two-phase cancel ORDER: the cancel sentinel write strictly precedes the
    // last-resort stop_unit backstop (recorded via the shared event log).
    #[tokio::test]
    async fn cancel_event_order_sentinel_strictly_before_stop_unit() {
        let h = harness_with_clock_step(CANCEL_DEADLINE_MS + 1);
        let id = create_live_running(&h, 0).await;
        h.registry.cancel(&id, "boot-A").await.unwrap();
        let events = h.events.lock().unwrap().clone();
        let sentinel = events
            .iter()
            .position(|e| *e == Event::WriteCancel(0))
            .expect("sentinel written");
        let stop = events
            .iter()
            .position(|e| *e == Event::StopUnit(0))
            .expect("stop_unit backstop");
        assert!(
            sentinel < stop,
            "cancel sentinel must precede stop_unit (events: {events:?})"
        );
    }

    // Live reconciliation via the PERIODIC reaper: a vanished unit is marked
    // lost (active released) while retained logs remain readable.
    #[tokio::test]
    async fn periodic_reaper_marks_vanished_unit_lost_and_retains_logs() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        h.store.set_stdout(0, b"out", meta(3, 0, false, false));
        h.units.set_live(0, false);
        h.registry.reap_once().await;
        let snap = h.registry.inspect(&id, "boot-A").await.unwrap();
        assert_eq!(snap.state, ExecState::LostGuestd);
        let chunk = h
            .registry
            .read_logs(&id, "boot-A", RtStream::Stdout, 0, 64)
            .await
            .unwrap();
        assert_eq!(chunk.data, b"out");
    }

    // Creating-guard: an in-flight create is invisible to a concurrent
    // ExecList AND a concurrent reaper — neither may reveal, mark, or delete it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn creating_guard_hides_inflight_create_from_list_and_reaper() {
        let Harness {
            registry,
            store,
            units,
            now: _now,
            events: _events,
        } = harness();
        units.set_live(0, true);
        let gate = Arc::new(ReadGate::default());
        store.install_status_gate(Arc::clone(&gate));
        let registry = Arc::new(registry);

        let create_reg = Arc::clone(&registry);
        let create = tokio::spawn(async move {
            create_reg
                .create("boot-A", command(), DetachedCaps::standard(0))
                .await
        });
        // Wait until create is parked in read_status with the Creating guard held.
        gate.wait_until_entered();

        // Concurrent ExecList must not reveal the in-flight create.
        assert!(
            registry.list("boot-A").await.unwrap().is_empty(),
            "ExecList must not reveal a Creating entry"
        );
        // Concurrent reaper must not mark/delete the in-flight create.
        registry.reap_once().await;
        assert!(
            store.slot_exists(0),
            "reaper must not delete a Creating entry"
        );
        assert!(!units.stopped(0), "reaper must not stop a Creating unit");

        // Release: the create resolves Running and becomes listable.
        store.set_status(0, StatusPhase::Started);
        gate.release();
        let (_id, snapshot) = create.await.unwrap().unwrap();
        assert_eq!(snapshot.state, ExecState::Running);
        assert_eq!(registry.list("boot-A").await.unwrap().len(), 1);
    }

    // GC must recheck read-guards under the mutex — a read that took a guard
    // after the reaper's snapshot keeps serving stable bytes, and only once it
    // completes does a later pass GC the slot to a tombstone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn f5_gc_defers_while_read_guard_held_then_expires() {
        let Harness {
            registry,
            store,
            units: _units,
            now,
            events: _events,
        } = harness();
        store.set_status(0, StatusPhase::Exited(0));
        let (id, _) = registry
            .create("boot-A", command(), DetachedCaps::standard(0))
            .await
            .unwrap();
        store.set_stdout(0, b"hello", meta(5, 0, false, true));
        // Make the terminal record GC-eligible.
        now.store(1_000 + RETENTION_TTL_MS + 1, Ordering::SeqCst);
        let gate = Arc::new(ReadGate::default());
        store.install_read_gate(Arc::clone(&gate));
        let registry = Arc::new(registry);

        let read_reg = Arc::clone(&registry);
        let read_id = id.clone();
        let read = tokio::spawn(async move {
            read_reg
                .read_logs(&read_id, "boot-A", RtStream::Stdout, 0, 1024)
                .await
        });
        // The read guard is now held; the read is parked in read_log.
        gate.wait_until_entered();

        // GC must DEFER while the guard is held.
        registry.reap_once().await;
        assert!(
            registry.inspect(&id, "boot-A").await.is_ok(),
            "GC must defer while a read guard is held"
        );

        // Release the read: it returns stable bytes.
        gate.release();
        let chunk = read.await.unwrap().unwrap();
        assert_eq!(chunk.data, b"hello", "stable bytes before release");

        // Now the guard is dropped: a later pass GCs to a tombstone.
        registry.reap_once().await;
        assert_eq!(
            registry.inspect(&id, "boot-A").await.unwrap_err(),
            ExecError::ExecExpired,
            "ExecExpired after GC"
        );
    }

    // ExecList enforces same-boot: a mismatched boot id is StaleSession.
    #[tokio::test]
    async fn list_boot_mismatch_is_stale_session() {
        let h = harness();
        create_live_running(&h, 0).await;
        assert_eq!(
            h.registry.list("boot-OTHER").await.unwrap_err(),
            ExecError::StaleSession
        );
    }

    // ExecList exposes only the argv hash — never the raw program/args/cwd/env.
    #[tokio::test]
    async fn list_entries_redact_raw_argv() {
        let h = harness();
        let id = create_live_running(&h, 0).await;
        let list = h.registry.list("boot-A").await.unwrap();
        assert_eq!(list.len(), 1);
        let entry = &list[0];
        assert_eq!(entry.exec_id, id);
        assert_eq!(entry.argv_sha256.len(), 64);
        assert!(
            entry
                .argv_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        let rendered = format!("{entry:?}");
        assert!(
            !rendered.contains("/bin/sleep"),
            "raw program must never appear in a list entry"
        );
        assert!(
            !rendered.contains("3600"),
            "raw args must never appear in a list entry"
        );
    }
}
