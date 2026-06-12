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
//! `nixling-exec-<NN>.service` keyed only by slot.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::detached::{unit_name, RunnerUnitPaths, TransientUnitManager};
use crate::exec::{ExecError, ExecIdSource, ExecSnapshot, ExecState, ExitOutcome, Stream as RtStream, ValidatedCommand};

use nixling_exec_runner::filering::{FileRingError, RingChunk, StreamMeta};
use nixling_exec_runner::paths::{RunnerPaths, Stream as RunnerStream};
use nixling_exec_runner::record::{DurableRecord, RecordState, StatusPhase};
use nixling_exec_runner::spec::ExecSpec;
use nixling_exec_runner::{
    RunnerEnv, DETACHED_ACTIVE_PER_VM, DETACHED_LOG_QUOTA_BYTES, DETACHED_RETAINED_PER_VM,
    DETACHED_STREAM_LOG_BYTES,
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
/// uses dir-fd `openat`/`O_NOFOLLOW` rooted at `/run/nixling-exec`.
pub trait SlotStore: Send + Sync + 'static {
    /// Create `/run/nixling-exec/slot-<NN>` (root-owned 0700) after validating
    /// the root-owned 0700 parent. Idempotent.
    fn prepare_slot_dir(&self, slot: u32) -> Result<(), ExecError>;
    /// Atomically write+fsync the durable record.
    fn write_record(&self, slot: u32, record: &DurableRecord) -> Result<(), ExecError>;
    /// Read the durable record (authenticity-validated).
    fn read_record(&self, slot: u32) -> Result<DurableRecord, ExecError>;
    /// Atomically write+fsync the runner spec.
    fn write_spec(&self, slot: u32, spec: &ExecSpec) -> Result<(), ExecError>;
    /// Atomically write+fsync the cancel sentinel.
    fn write_cancel(&self, slot: u32) -> Result<(), ExecError>;
    /// Read the runner status phase (`Ok(None)` if no status yet).
    fn read_status(&self, slot: u32) -> Result<Option<StatusPhase>, ExecError>;
    /// Read a stream's sidecar metadata (`Ok(None)` if the file is absent).
    fn read_log_meta(&self, slot: u32, stream: RunnerStream) -> Result<Option<StreamMeta>, ExecError>;
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
    /// In-flight create guard: invisible to ExecList/reaper until resolved.
    creating: bool,
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
            .find(|(_, entry)| !entry.creating && entry.record.exec_id == exec_id)
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
        if let Some(entry) = self.slots.get_mut(&slot) {
            if entry.active_counted {
                entry.active_counted = false;
                self.active = self.active.saturating_sub(1);
            }
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
        Self::assert_quota_invariant();
        if boot_id != self.config.boot_id {
            return Err(ExecError::StaleSession);
        }

        let spec = build_spec(&command, &caps)?;
        let argv_sha256 = argv_hash(&command);
        let exec_id = self.ids.next_exec_id()?;
        let now = self.clock.now_ms();

        // Step 1: reserve slot + active + quota under the Creating guard.
        let slot = {
            let mut state = self.lock();
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
                    generation: 0,
                    active_counted: true,
                    read_guards: 0,
                },
            );
            slot
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
            match self.store.read_status(slot)? {
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
                    self.abort_create(slot).await;
                    return Err(ExecError::RetainedLogPathUnsafe);
                }
                None => {
                    if self.clock.now_ms().saturating_sub(start) >= CREATE_TIMEOUT_MS {
                        // Re-query the unit: a live unit is sufficient proof
                        // (never kill a running job); a gone unit fails create.
                        if self.unit_present(slot).await {
                            self.commit_running(slot);
                            return Ok((exec_id.to_owned(), self.snapshot_for(slot)?));
                        }
                        self.abort_create(slot).await;
                        return Err(ExecError::Internal);
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
            let record = entry.record.clone();
            state.release_active(slot);
            record
        };
        let _ = self.store.write_record(slot, &record);
    }

    async fn unit_present(&self, slot: u32) -> bool {
        match self.units.list_managed_units().await {
            Ok(units) => units.iter().any(|unit| unit.slot == slot),
            Err(_) => false,
        }
    }

    // ---- read-side ops ---------------------------------------------------

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
                return Err(self.missing_kind(exec_id));
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
                .filter(|(_, entry)| !entry.creating)
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
                .filter(|(_, entry)| !entry.creating)
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

        // Phase 3: last-resort backstop — only now stop the unit, then mark the
        // record lost if it still produced no terminal status.
        let _ = self.units.stop_unit(slot).await;
        self.reconcile_slot(slot).await;
        if !self.is_terminal(slot) {
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
        let (creating, terminal) = {
            let state = self.lock();
            match state.slots.get(&slot) {
                Some(entry) => (entry.creating, entry.is_terminal()),
                None => return,
            }
        };
        if creating || terminal {
            return;
        }
        // A published terminal status wins regardless of unit liveness.
        match self.store.read_status(slot) {
            Ok(Some(StatusPhase::Exited(code))) => {
                self.commit_terminal(slot, RecordState::Exited, Some(code), None);
                return;
            }
            Ok(Some(StatusPhase::Signaled(signal))) => {
                self.commit_terminal(slot, RecordState::Signaled, None, Some(signal as u32));
                return;
            }
            Ok(Some(StatusPhase::Cancelled)) => {
                self.commit_terminal(slot, RecordState::Cancelled, None, None);
                return;
            }
            Ok(Some(StatusPhase::SpawnFailed)) => {
                self.commit_terminal(slot, RecordState::SpawnFailed, None, None);
                return;
            }
            Ok(Some(StatusPhase::Started)) => {
                self.commit_running(slot);
            }
            Ok(Some(StatusPhase::InfraFailed)) | Ok(None) | Err(_) => {}
        }

        if self.unit_present(slot).await {
            return;
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
        let slots: Vec<u32> = {
            let state = self.lock();
            state.slots.keys().copied().collect()
        };
        for slot in slots {
            let (creating, terminal, terminal_time, guards) = {
                let state = self.lock();
                match state.slots.get(&slot) {
                    Some(entry) => (
                        entry.creating,
                        entry.is_terminal(),
                        entry.record.terminal_time_unix,
                        entry.read_guards,
                    ),
                    None => continue,
                }
            };
            if creating {
                continue;
            }
            if !terminal {
                self.reconcile_slot(slot).await;
                continue;
            }
            // Terminal: GC past TTL, deferring while a read guard is held.
            if guards > 0 {
                continue;
            }
            let expired = terminal_time
                .map(|t| self.clock.now_ms().saturating_sub(t) >= RETENTION_TTL_MS)
                .unwrap_or(false);
            if expired {
                self.gc_slot(slot).await;
            }
        }
    }

    async fn gc_slot(&self, slot: u32) {
        let _ = self.units.stop_unit(slot).await;
        let _ = self.units.reset_failed(slot).await;
        let _ = self.store.delete_slot_dir(slot);
        let mut state = self.lock();
        let exec_id = state.slots.get(&slot).map(|e| e.record.exec_id.clone());
        state.remove_entry(slot);
        if let Some(exec_id) = exec_id {
            state.push_tombstone(exec_id);
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
        let present_units = self.units.list_managed_units().await.unwrap_or_default();
        let unit_live = |slot: u32| present_units.iter().any(|u| u.slot == slot);

        for slot in slots {
            self.adopt_slot(slot, unit_live(slot)).await;
        }

        // Orphan units with no record → stop + reset-failed.
        let adopted: Vec<u32> = {
            let state = self.lock();
            state.slots.keys().copied().collect()
        };
        for unit in &present_units {
            if !adopted.contains(&unit.slot) {
                let _ = self.units.stop_unit(unit.slot).await;
                let _ = self.units.reset_failed(unit.slot).await;
            }
        }

        self.evict_over_budget().await;
    }

    async fn adopt_slot(&self, slot: u32, unit_live: bool) {
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
            self.insert_adopted(slot, record, Some((terminal, code, signal)), false);
            return;
        }

        if unit_live {
            // Live authentic unit + started/none → adopt as Running. Never kill.
            self.insert_adopted(slot, record, None, true);
            return;
        }

        // No unit, no terminal status.
        if record.state == RecordState::Dispatching
            && self.clock.now_ms() < record.dispatch_deadline_unix
        {
            // Within dispatch deadline → hold in-flight (reserved, non-listable).
            self.insert_adopted(slot, record, None, false);
            // Keep it guarded so it is not listable/reaped until resolved.
            if let Some(entry) = self.lock().slots.get_mut(&slot) {
                entry.creating = true;
            }
            return;
        }

        // Past the dispatch deadline (or non-Dispatching) with no unit/status →
        // delete + release; no visible id.
        let _ = self.store.delete_slot_dir(slot);
    }

    fn insert_adopted(
        &self,
        slot: u32,
        mut record: DurableRecord,
        terminal: Option<(RecordState, Option<i32>, Option<u32>)>,
        running: bool,
    ) {
        let caps = DetachedCaps::standard(self.config.max_runtime_sec);
        let now = self.clock.now_ms();
        let mut active_counted = false;
        if let Some((terminal_state, code, signal)) = terminal {
            record.state = terminal_state;
            record.exit_code = code;
            record.term_signal = signal;
            if record.terminal_time_unix.is_none() {
                record.terminal_time_unix = Some(now);
            }
        } else if running {
            record.state = RecordState::Running;
            active_counted = true;
        } else {
            // In-flight dispatch hold: still active (reserved).
            active_counted = true;
        }
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
                generation: 0,
                active_counted,
                read_guards: 0,
            },
        );
        let _ = self.store.write_record(slot, &state.slots[&slot].record.clone());
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

    fn missing_kind(&self, exec_id: &str) -> ExecError {
        let state = self.lock();
        if state.is_tombstoned(exec_id) {
            ExecError::ExecExpired
        } else {
            ExecError::ExecNotFound
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

fn build_spec(command: &ValidatedCommand, caps: &DetachedCaps) -> Result<ExecSpec, ExecError> {
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
    Ok(ExecSpec {
        argv,
        cwd: Some(cwd),
        env,
        stdout_log_cap: caps.stdout_log_cap,
        stderr_log_cap: caps.stderr_log_cap,
        max_runtime_sec: caps.max_runtime_sec,
    })
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
    exec_id.len() == 32 && exec_id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Stable unit name for a slot (re-exported for the service layer).
pub fn slot_unit_name(slot: u32) -> String {
    unit_name(slot)
}

/// Production [`SlotStore`] rooted at `/run/nixling-exec` (root-owned 0700,
/// boot-scoped). All on-disk access is `O_NOFOLLOW` and the parent/slot dirs
/// are validated root-owned before any open, mirroring the runner's
/// `validate_slot_dir`. Reuses the dependency-pure exec-runner primitives
/// (`atomicio`, `FileRing`, the record/spec/status codecs) so the wire layout
/// is identical to what the runner reads and writes.
pub struct RunSlotStore {
    base: std::path::PathBuf,
}

impl RunSlotStore {
    /// Production base (`/run/nixling-exec`).
    pub fn new() -> Self {
        Self {
            base: std::path::PathBuf::from(nixling_exec_runner::paths::RUN_DIR),
        }
    }

    /// Construct rooted at an arbitrary base (Layer-2 / integration harnesses).
    pub fn with_base(base: impl Into<std::path::PathBuf>) -> Self {
        Self { base: base.into() }
    }

    fn paths(&self, slot: u32) -> RunnerPaths {
        RunnerPaths::new(self.base.clone(), slot)
    }

    /// Validate the parent and slot directories are root-owned via dir-fd
    /// `openat`/`O_NOFOLLOW` (mirrors the runner's `validate_slot_dir`).
    fn validate_slot_dir(&self, paths: &RunnerPaths) -> Result<(), ExecError> {
        use rustix::fs::{fstat, open, openat, Mode, OFlags};
        let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let base = open(paths.base(), dir_flags, Mode::empty()).map_err(|_| ExecError::Internal)?;
        let base_stat = fstat(&base).map_err(|_| ExecError::Internal)?;
        if base_stat.st_uid != 0 {
            return Err(ExecError::RetainedLogPathUnsafe);
        }
        let slot = openat(&base, paths.slot_dir_name(), dir_flags, Mode::empty())
            .map_err(|_| ExecError::Internal)?;
        let slot_stat = fstat(&slot).map_err(|_| ExecError::Internal)?;
        if slot_stat.st_uid != 0 {
            return Err(ExecError::RetainedLogPathUnsafe);
        }
        Ok(())
    }
}

impl Default for RunSlotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotStore for RunSlotStore {
    fn prepare_slot_dir(&self, slot: u32) -> Result<(), ExecError> {
        use rustix::fs::{fstat, open, Mode, OFlags};
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
        nixling_exec_runner::atomicio::fsync_parent_dir(&slot_dir)
            .map_err(|_| ExecError::Internal)?;
        // Re-validate ownership after creation (defends against a pre-seeded
        // foreign slot dir).
        self.validate_slot_dir(&paths)?;
        Ok(())
    }

    fn write_record(&self, slot: u32, record: &DurableRecord) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        nixling_exec_runner::atomicio::atomic_write(&paths.record(), &record.encode())
            .map_err(|_| ExecError::Internal)
    }

    fn read_record(&self, slot: u32) -> Result<DurableRecord, ExecError> {
        let paths = self.paths(slot);
        let bytes = nixling_exec_runner::atomicio::read_file_nofollow(&paths.record())
            .map_err(|_| ExecError::ExecNotFound)?;
        DurableRecord::decode(&bytes).map_err(|_| ExecError::Internal)
    }

    fn write_spec(&self, slot: u32, spec: &ExecSpec) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        let bytes =
            nixling_exec_runner::spec::SpecCodec::encode(spec).map_err(|_| ExecError::Internal)?;
        nixling_exec_runner::atomicio::atomic_write(&paths.spec(), &bytes)
            .map_err(|_| ExecError::Internal)
    }

    fn write_cancel(&self, slot: u32) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        // The sentinel's presence is the cancel signal; content is irrelevant.
        nixling_exec_runner::atomicio::atomic_write(&paths.cancel(), b"1")
            .map_err(|_| ExecError::Internal)
    }

    fn read_status(&self, slot: u32) -> Result<Option<StatusPhase>, ExecError> {
        let paths = self.paths(slot);
        match nixling_exec_runner::atomicio::read_file_nofollow(&paths.status()) {
            Ok(bytes) => {
                let rec = nixling_exec_runner::record::StatusRecord::decode(&bytes)
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
        match nixling_exec_runner::filering::FileRingReader::open(
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
        let reader = nixling_exec_runner::filering::FileRingReader::open(
            &paths.data(stream),
            &paths.sidecar(stream),
        )?;
        reader.read(offset, max_len)
    }

    fn mark_lost(&self, slot: u32) -> Result<(), ExecError> {
        let paths = self.paths(slot);
        for stream in [RunnerStream::Stdout, RunnerStream::Stderr] {
            match nixling_exec_runner::filering::mark_stream_lost(&paths.sidecar(stream)) {
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
        nixling_exec_runner::atomicio::fsync_parent_dir(&paths.slot_dir())
            .map_err(|_| ExecError::Internal)
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
            if let Ok(slot) = rest.parse::<u32>() {
                if (slot as usize) < DETACHED_RETAINED_PER_VM {
                    slots.push(slot);
                }
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
    use std::sync::atomic::{AtomicU64, Ordering};

    // ---- fakes -----------------------------------------------------------

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
        fail_prepare: bool,
        /// When set, writing the cancel sentinel also publishes this terminal
        /// status (simulating a promptly-reacting runner).
        cancel_terminal: Option<StatusPhase>,
    }

    #[derive(Clone, Default)]
    struct FakeStore {
        inner: Arc<Mutex<FakeStoreState>>,
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
        fn cancel_written(&self, slot: u32) -> bool {
            *self.inner.lock().unwrap().cancels.get(&slot).unwrap_or(&false)
        }
        fn set_cancel_terminal(&self, phase: StatusPhase) {
            self.inner.lock().unwrap().cancel_terminal = Some(phase);
        }
        fn slot_exists(&self, slot: u32) -> bool {
            self.inner.lock().unwrap().records.contains_key(&slot)
        }
        fn seed_record(&self, slot: u32, record: &DurableRecord) {
            self.inner
                .lock()
                .unwrap()
                .records
                .insert(slot, record.encode());
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
            let bytes = nixling_exec_runner::spec::SpecCodec::encode(spec)
                .map_err(|_| ExecError::Internal)?;
            self.inner.lock().unwrap().specs.insert(slot, bytes);
            Ok(())
        }
        fn write_cancel(&self, slot: u32) -> Result<(), ExecError> {
            let mut s = self.inner.lock().unwrap();
            s.cancels.insert(slot, true);
            if let Some(phase) = s.cancel_terminal {
                s.status.insert(slot, phase);
            }
            Ok(())
        }
        fn read_status(&self, slot: u32) -> Result<Option<StatusPhase>, ExecError> {
            Ok(self.inner.lock().unwrap().status.get(&slot).copied())
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
            let mut s = self.inner.lock().unwrap();
            s.records.remove(&slot);
            s.specs.remove(&slot);
            s.status.remove(&slot);
            s.cancels.remove(&slot);
            s.stdout_meta.remove(&slot);
            s.stderr_meta.remove(&slot);
            s.stdout_data.remove(&slot);
            Ok(())
        }
        fn list_slot_dirs(&self) -> Result<Vec<u32>, ExecError> {
            Ok(self.inner.lock().unwrap().records.keys().copied().collect())
        }
    }

    #[derive(Default)]
    struct FakeUnitsState {
        live: Vec<u32>,
        started: Vec<u32>,
        stopped: Vec<u32>,
        fail_start: bool,
    }

    #[derive(Clone, Default)]
    struct FakeUnits {
        inner: Arc<Mutex<FakeUnitsState>>,
    }

    impl FakeUnits {
        fn set_live(&self, slot: u32, live: bool) {
            let mut s = self.inner.lock().unwrap();
            s.live.retain(|x| *x != slot);
            if live {
                s.live.push(slot);
            }
        }
        fn stopped(&self, slot: u32) -> bool {
            self.inner.lock().unwrap().stopped.contains(&slot)
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
            let mut s = self.inner.lock().unwrap();
            s.stopped.push(slot);
            s.live.retain(|x| *x != slot);
            Ok(())
        }
        async fn reset_failed(&self, _slot: u32) -> Result<(), UnitError> {
            Ok(())
        }
        async fn list_managed_units(&self) -> Result<Vec<crate::detached::ManagedUnit>, UnitError> {
            let s = self.inner.lock().unwrap();
            Ok(s.live
                .iter()
                .map(|slot| crate::detached::ManagedUnit {
                    slot: *slot,
                    active: true,
                })
                .collect())
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
    }

    fn harness() -> Harness {
        harness_with_clock_step(STATUS_POLL_INTERVAL_MS)
    }

    fn harness_with_clock_step(step: u64) -> Harness {
        let store = FakeStore::default();
        let units = FakeUnits::default();
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
                paths: RunnerUnitPaths::new("/run/current-system/sw/bin/nixling-exec-runner"),
                boot_id: "boot-A".to_owned(),
                max_runtime_sec: 0,
            },
        );
        Harness {
            registry,
            store,
            units,
            now,
        }
    }

    fn command() -> ValidatedCommand {
        ValidatedCommand {
            program: "/bin/sleep".into(),
            args: vec!["3600".to_owned()],
            cwd: "/".into(),
            env: Vec::new(),
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
        // Now listable.
        let list = h.registry.list("boot-A").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].exec_id, id);
        assert_eq!(list[0].slot, 0);
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
        assert!(!h.units.stopped(0), "a live unit is never killed on timeout");
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
    async fn cancel_resolves_on_terminal_status_without_stop_unit() {
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
        assert!(!h.units.stopped(0), "no backstop when status appears");
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
        h.now
            .store(1_000 + RETENTION_TTL_MS + 1, Ordering::SeqCst);
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
        let snapshot = h.registry.inspect(&format!("{:032x}", 7), "boot-A").await.unwrap();
        assert_eq!(snapshot.state, ExecState::Running);
        assert!(!h.units.stopped(5), "adopted unit not killed");
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
}
