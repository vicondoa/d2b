//! The broker's generic stateful mechanism: declared state cells.
//!
//! Plan U3 / KTD3 / KD4. One generic mechanism hosts the broker-owned state
//! the per-family typed registries used to own. Cells are keyed
//! `(cell, invocation_id, initiating_principal)`; compare-and-consume runs
//! under the broker's single-process lock (the broker is the cell owner, so
//! the lock is single-process by construction). A repeated invocation id
//! replays the recorded outcome for the same initiating principal only -
//! invocation ids appear in audit records and are not secrets, so they never
//! gate one-time grants alone.
//!
//! Durability facets ride the committed Operation rows
//! ([`crate::catalog::CellDurability`], regenerated from
//! `docs/reference/policy/broker-operations.json`):
//!
//! - One-time cells commit durably *before* the effect runs: the durable
//!   record is written as `unknown` at consume time, and `complete` rewrites
//!   it as `completed`. A crash between the durable commit and the effect
//!   leaves `outcome = unknown`, and the retried invocation is reconciled
//!   (re-granted under the same invocation id, which the effect pairs with
//!   for idempotency - never a blind retry). A completed record refuses
//!   re-consume, across broker restarts (AE2's restart-replay resistance).
//! - Ephemeral cells keep in-process reset-on-restart semantics; their
//!   records replay in-process and never touch the durable file.
//!
//! Retention (TTL + per-cell size bound) applies only to replayable
//! non-one-time outcome records. Consumed one-time markers are exempt: their
//! count is bounded by the declared one-time cell set, so compaction can
//! never re-enable a double grant.
//!
//! Daemon-side effect actors reach cells only through carrier-mediated cell
//! operations (the U10/U11 seam) - never direct file access; the file here
//! is broker-internal state under the daemon state root, written with the
//! same discipline the broker's other durable rows use
//! (`ops/network.rs::persist_persistent_tap_realization`).
//!
//! ## U6 conversion (async purity)
//!
//! The record table and its durable persistence are owned by one dedicated
//! worker thread (the plan U6 "single-owner task" option) behind a bounded
//! `tokio::sync::mpsc` channel with `tokio::sync::oneshot` replies - the
//! sanctioned bounded-worker channel boundary (plan R4). Every compare-and-
//! consume and its durable persist run as one serialized FIFO unit on the
//! owner, so ordering and the "durable before Granted returns" one-time
//! claim cannot tear. Poison is a latched flag: a panic inside a mutation
//! critical section sets it and every later `consume`/`complete` refuses
//! with [`CellStoreError::Poisoned`] - a corrupted store can never grant a
//! one-time claim twice. Read-only ops stay fail-open.

use std::any::Any;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use std::sync::mpsc::{self, SyncSender};

use crate::catalog::CellDurability;

/// The sub-directory under the daemon state root the durable cell records
/// land in. Mirrors the broker's other state-root rows (`network-attachments`,
/// `secrets/...`).
const STATE_CELLS_DIR: &str = "state-cells";
/// The durable record file inside [`STATE_CELLS_DIR`].
const STATE_CELLS_FILE: &str = "cells.json";
/// The durable file format version.
const DURABLE_VERSION: u32 = 1;

/// The bound on admitted-but-unstarted cell commands, per store.
const CELL_WORKER_QUEUE_DEPTH: usize = 256;

/// The initiating principal broker-internal spawn/registry machinery records
/// for entries it creates on behalf of the daemon's spawn flow. The typed
/// arms the seam retires later will carry the attested caller instead.
pub const BROKER_PRINCIPAL: &str = "broker";

/// One keyed cell record identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellKey {
    /// The declared cell name (the committed row's `stateCell.cell`).
    pub cell: String,
    /// The caller-supplied per-invocation identity (the operation's
    /// invocation id; appears in audit records, never a secret).
    pub invocation_id: String,
    /// The initiating principal, as attested at the envelope boundary.
    pub principal: String,
}

impl CellKey {
    fn new(cell: &str, invocation_id: &str, principal: &str) -> Self {
        Self {
            cell: cell.to_owned(),
            invocation_id: invocation_id.to_owned(),
            principal: principal.to_owned(),
        }
    }
}

/// The recorded outcome of one cell record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellOutcome {
    /// The record is durably pre-committed but no completion has been
    /// recorded: either the effect is in flight or the owner crashed between
    /// the durable commit and the effect. A retried invocation reconciles
    /// under its invocation id.
    Unknown,
    /// The effect recorded completion; a one-time cell refuses re-consume.
    Completed,
}

/// What a compare-and-consume call decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeDecision {
    /// The claim was acquired: the caller may run the effect. This is the
    /// first consume of the key.
    Granted,
    /// The key holds a durable `unknown` record left by a crashed owner: the
    /// caller re-runs the effect idempotently under the same invocation id.
    /// Neither a double grant (same invocation) nor a silent leak (the retry
    /// is admitted).
    Reconciled,
    /// The same key is claimed by an in-flight invocation in this process.
    InProgress,
    /// The same key holds a completed record: replay refused (AE2).
    Replayed,
    /// The same cell + invocation id carries another principal's record;
    /// replay is refused for a different principal (KTD3).
    ForeignPrincipal,
}

/// Why a cell operation failed.
#[derive(Debug)]
pub enum CellStoreError {
    /// The underlying durable store could not be read or written.
    Io(std::io::Error),
    /// The broker's single-process cell lock is poisoned.
    Poisoned,
    /// A completion addressed a key no record exists for.
    MissingRecord,
    /// A completion addressed a record another principal owns.
    ForeignPrincipal,
    /// The durable file exists but is not a valid cell record file.
    CorruptDurable(String),
}

impl PartialEq for CellStoreError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            (Self::Poisoned, Self::Poisoned)
            | (Self::MissingRecord, Self::MissingRecord)
            | (Self::ForeignPrincipal, Self::ForeignPrincipal) => true,
            (Self::CorruptDurable(left), Self::CorruptDurable(right)) => left == right,
            _ => false,
        }
    }
}

impl std::fmt::Display for CellStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "state cell store i/o: {error}"),
            Self::Poisoned => write!(formatter, "state cell store lock poisoned"),
            Self::MissingRecord => write!(formatter, "state cell record missing"),
            Self::ForeignPrincipal => {
                write!(formatter, "state cell record owned by another principal")
            }
            Self::CorruptDurable(reason) => {
                write!(formatter, "state cell durable file corrupt: {reason}")
            }
        }
    }
}

/// Retention bounds for replayable non-one-time outcome records.
///
/// One-time consumed markers are always exempt; records carrying live
/// in-process payloads (e.g. the runner pidfd registry) are live state, not
/// outcome history, and are exempt too.
#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    /// Ephemeral outcome records older than this are evicted on mutation.
    pub outcome_ttl_ms: u64,
    /// Per-cell cap of ephemeral outcome records; the oldest are evicted.
    pub max_ephemeral_outcome_records: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            outcome_ttl_ms: 86_400_000, // 24h
            max_ephemeral_outcome_records: 4096,
        }
    }
}

/// One in-process cell record.
struct CellRecord {
    outcome: CellOutcome,
    /// A live in-process claim. Never persisted: a restarted owner reloads
    /// durable records unclaimed, which is what turns a crash-stale record
    /// into a reconciliation rather than an in-progress refusal.
    claimed: bool,
    durability: CellDurability,
    /// Non-durable cell value (e.g. a runner's pidfd). Live state, exempt
    /// from retention and never serialized.
    payload: Option<Arc<dyn Any + Send + Sync>>,
    consumed_ms: u64,
    completed_ms: Option<u64>,
}

/// The broker's single generic stateful mechanism.
///
/// One owning process - the broker - serializes every compare-and-consume
/// through a single dedicated worker thread (the U6 "single-owner task"
/// shape); the durable file holds only one-time records. The handle is
/// cheap and `Send + Sync`: it is the bounded channel to the owner.
pub struct CellStore {
    owner: CellStoreOwner,
}

/// The caller-side handle of the single-owner worker.
struct CellStoreOwner {
    commands: SyncSender<CellCommand>,
}

/// One serialized command the owner executes in FIFO order.
enum CellCommand {
    /// Load the durable records (or start empty) into the owner.
    Bootstrap {
        root: Option<PathBuf>,
        retention: RetentionPolicy,
        reply: SyncSender<Result<(), CellStoreError>>,
    },
    /// Compare-and-consume one key, persisting the one-time claim before the
    /// reply - the "durable before Granted returns" unit.
    Consume {
        cell: String,
        invocation_id: String,
        principal: String,
        durability: CellDurability,
        reply: SyncSender<Result<ConsumeDecision, CellStoreError>>,
    },
    /// Record completion of one claimed key, durably for one-time cells.
    Complete {
        cell: String,
        invocation_id: String,
        principal: String,
        reply: SyncSender<Result<(), CellStoreError>>,
    },
    /// Insert one payload record into an ephemeral cell.
    InsertPayload {
        cell: String,
        invocation_id: String,
        principal: String,
        payload: Arc<dyn Any + Send + Sync>,
        reply: SyncSender<Result<(), CellStoreError>>,
    },
    Contains {
        cell: String,
        invocation_id: String,
        reply: SyncSender<bool>,
    },
    Payload {
        cell: String,
        invocation_id: String,
        reply: SyncSender<Option<Arc<dyn Any + Send + Sync>>>,
    },
    Remove {
        cell: String,
        invocation_id: String,
        reply: SyncSender<bool>,
    },
    Keys {
        cell: String,
        reply: SyncSender<Vec<String>>,
    },
    Clear {
        cell: String,
        reply: SyncSender<usize>,
    },
    RecordCount {
        reply: SyncSender<usize>,
    },
    /// An injected panic inside a mutation critical section (U6 invariant
    /// test): the owner latches the poison flag before serving the next
    /// command.
    #[cfg(test)]
    InjectPanic,
    /// An injected crash between the durable persist and the Granted reply
    /// (U6 invariant test): the owner commits the one-time pre-commit, signals
    /// `persisted`, then exits without answering - no double grant survives
    /// the restart.
    #[cfg(test)]
    ConsumeCrashAfterPersist {
        cell: String,
        invocation_id: String,
        principal: String,
        durability: CellDurability,
        persisted: SyncSender<Result<(), CellStoreError>>,
    },
    Shutdown {
        exited: SyncSender<()>,
    },
}

/// The owner thread's whole state: every field below is touched only on the
/// owner thread, so mutations never race and a panic inside one is caught and
/// converted into the latched poison flag.
struct CellWorkerState {
    /// The durable root directory, when the store is file-backed.
    root: Option<PathBuf>,
    records: BTreeMap<CellKey, CellRecord>,
    retention: RetentionPolicy,
    /// Latched by a panic inside a mutation critical section; `consume` and
    /// `complete` refuse with [`CellStoreError::Poisoned`] once set.
    poisoned: bool,
}

/// The durable file layout: cell name → invocation id → one record.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DurableFile {
    version: u32,
    #[serde(default)]
    records: BTreeMap<String, BTreeMap<String, DurableRecord>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DurableRecord {
    principal: String,
    outcome: String,
    consumed_at_ms: u64,
    #[serde(default)]
    completed_at_ms: Option<u64>,
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
impl CellStore {
    /// An in-memory store with no durable file.
    pub fn in_memory() -> Self {
        Self::spawn_owner(None, RetentionPolicy::default())
            .expect("spawn in-memory cell store owner")
    }

    /// Open the store for one state root, recovering every durable record.
    /// A missing file is an empty store; a malformed file fails closed.
    pub fn open(root: &Path) -> Result<Self, CellStoreError> {
        Self::spawn_owner(Some(root.to_path_buf()), RetentionPolicy::default())
    }

    /// Test/embedding knob: the root plus an explicit retention policy.
    pub(crate) fn with_retention(root: Option<PathBuf>, retention: RetentionPolicy) -> Self {
        Self::spawn_owner(root, retention)
            .expect("spawn cell store owner with retention")
    }

    /// Spawn the single owner thread and hand it the bootstrap command.
    ///
    /// The owner loads the durable records (or starts empty) before the
    /// handle is returned, so a corrupt durable file fails `open` closed
    /// rather than surfacing on the first mutation.
    fn spawn_owner(
        root: Option<PathBuf>,
        retention: RetentionPolicy,
    ) -> Result<Self, CellStoreError> {
        let (commands, receiver) = mpsc::sync_channel::<CellCommand>(CELL_WORKER_QUEUE_DEPTH);
        std::thread::Builder::new()
            .name("d2b-broker-cell-store".to_owned())
            .spawn(move || cell_worker_loop(receiver))
            .map_err(|error| CellStoreError::Io(error))?;
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        commands
            .send(CellCommand::Bootstrap {
                root,
                retention,
                reply: reply_tx,
            })
            .map_err(|_| CellStoreError::Poisoned)?;
        reply_rx.recv().map_err(|_| CellStoreError::Poisoned)??;
        Ok(Self {
            owner: CellStoreOwner { commands },
        })
    }

    /// Compare-and-consume one cell key.
    ///
    /// For a one-time cell the claim is committed durably (outcome
    /// `unknown`) before this returns `Granted`, so a crash between the
    /// commit and the effect reconciles on retry instead of granting twice.
    /// The atomic unit (check + commit + durable persist) runs on the
    /// single owner, so it cannot tear under concurrency.
    pub fn consume(
        &self,
        cell: &str,
        invocation_id: &str,
        principal: &str,
        durability: CellDurability,
    ) -> Result<ConsumeDecision, CellStoreError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.owner
            .commands
            .send(CellCommand::Consume {
                cell: cell.to_owned(),
                invocation_id: invocation_id.to_owned(),
                principal: principal.to_owned(),
                durability,
                reply: reply_tx,
            })
            .map_err(|_| CellStoreError::Poisoned)?;
        reply_rx.recv().map_err(|_| CellStoreError::Poisoned)?
    }

    /// Record completion for one claimed key.
    ///
    /// The completion is durable for a one-time cell, making the marker
    /// restart-replay resistant; completing an already-completed record is
    /// idempotent.
    pub fn complete(
        &self,
        cell: &str,
        invocation_id: &str,
        principal: &str,
    ) -> Result<(), CellStoreError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.owner
            .commands
            .send(CellCommand::Complete {
                cell: cell.to_owned(),
                invocation_id: invocation_id.to_owned(),
                principal: principal.to_owned(),
                reply: reply_tx,
            })
            .map_err(|_| CellStoreError::Poisoned)?;
        reply_rx.recv().map_err(|_| CellStoreError::Poisoned)?
    }

    /// Insert one payload record into an ephemeral cell (e.g. a registered
    /// runner's pidfd). The cell record is in-process state, never durable;
    /// the recorded principal is the initiating principal of the write.
    pub fn insert_payload(
        &self,
        cell: &str,
        invocation_id: &str,
        principal: &str,
        payload: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), CellStoreError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.owner
            .commands
            .send(CellCommand::InsertPayload {
                cell: cell.to_owned(),
                invocation_id: invocation_id.to_owned(),
                principal: principal.to_owned(),
                payload,
                reply: reply_tx,
            })
            .map_err(|_| CellStoreError::Poisoned)?;
        reply_rx.recv().map_err(|_| CellStoreError::Poisoned)?
    }

    /// Whether one ephemeral cell key holds a record. Reconciler-level read:
    /// principal-agnostic, matching the typed registry's `contains_key`.
    /// Fail-open read: after a poisoned mutation it still serves the map.
    pub fn contains(&self, cell: &str, invocation_id: &str) -> bool {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::Contains {
                cell: cell.to_owned(),
                invocation_id: invocation_id.to_owned(),
                reply: reply_tx,
            })
            .is_err()
        {
            return false;
        }
        reply_rx.recv().unwrap_or(false)
    }

    /// The payload of one ephemeral cell record, when present.
    pub fn payload(&self, cell: &str, invocation_id: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::Payload {
                cell: cell.to_owned(),
                invocation_id: invocation_id.to_owned(),
                reply: reply_tx,
            })
            .is_err()
        {
            return None;
        }
        reply_rx.recv().unwrap_or(None)
    }

    /// Remove one ephemeral cell record (reconciler-level cleanup).
    pub fn remove(&self, cell: &str, invocation_id: &str) -> bool {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::Remove {
                cell: cell.to_owned(),
                invocation_id: invocation_id.to_owned(),
                reply: reply_tx,
            })
            .is_err()
        {
            return false;
        }
        reply_rx.recv().unwrap_or(false)
    }

    /// Every invocation id currently held by one cell.
    pub fn keys(&self, cell: &str) -> Vec<String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::Keys {
                cell: cell.to_owned(),
                reply: reply_tx,
            })
            .is_err()
        {
            return Vec::new();
        }
        reply_rx.recv().unwrap_or_default()
    }

    /// Remove every record of one cell. Returns the number removed.
    pub fn clear(&self, cell: &str) -> usize {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::Clear {
                cell: cell.to_owned(),
                reply: reply_tx,
            })
            .is_err()
        {
            return 0;
        }
        reply_rx.recv().unwrap_or(0)
    }

    /// The number of records (broker-internal introspection/test hook).
    pub fn record_count(&self) -> usize {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::RecordCount { reply: reply_tx })
            .is_err()
        {
            return 0;
        }
        reply_rx.recv().unwrap_or(0)
    }

    /// Inject a panic inside a mutation critical section (U6 invariant test).
    ///
    /// The owner catches the panic and latches the poison flag; every later
    /// `consume`/`complete`/`insert_payload` refuses with
    /// [`CellStoreError::Poisoned`], so the one-time claim can never grant
    /// twice after the corrupted mutation.
    #[cfg(test)]
    pub fn inject_mutation_panic(&self) {
        let _ = self.owner.commands.send(CellCommand::InjectPanic);
    }
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
impl Drop for CellStore {
    /// Stop the owner deterministically: the Shutdown ack arrives only after
    /// the owner's state (including any in-flight durable persist) is gone,
    /// so a caller that reopens the same root cannot race a straggler write.
    fn drop(&mut self) {
        let (exited_tx, exited_rx) = mpsc::sync_channel(1);
        if self
            .owner
            .commands
            .send(CellCommand::Shutdown { exited: exited_tx })
            .is_ok()
        {
            let _ = exited_rx.recv();
        }
    }
}

/// The durable record file path under one state root.
fn cell_durable_path(root: &Path) -> PathBuf {
    root.join(STATE_CELLS_FILE)
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn cell_worker_loop(receiver: mpsc::Receiver<CellCommand>) {
    let Ok(CellCommand::Bootstrap {
        root,
        retention,
        reply,
    }) = receiver.recv()
    else {
        return;
    };
    let mut state = match cell_bootstrap(root, retention) {
        Ok(state) => state,
        Err(error) => {
            let _ = reply.send(Err(error));
            return;
        }
    };
    let _ = reply.send(Ok(()));
    while let Ok(command) = receiver.recv() {
        match command {
            // Shutdown runs outside the mutation dispatch so the ack can
            // follow the actual state drop: a caller that reopens the same
            // root after the ack can never race an in-flight persist.
            CellCommand::Shutdown { exited } => {
                drop(state);
                let _ = exited.send(());
                return;
            }
            command => {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    cell_handle(&mut state, command)
                }));
                match outcome {
                    Ok(LoopControl::Continue) => {}
                    Ok(LoopControl::Exit) => return,
                    Err(_) => {
                        // A panic inside a mutation critical section latches
                        // the poison flag fail-closed: the one-time claim
                        // must never grant twice after the store was
                        // corrupted mid-mutation.
                        state.poisoned = true;
                    }
                }
            }
        }
    }
}

fn cell_bootstrap(
    root: Option<PathBuf>,
    retention: RetentionPolicy,
) -> Result<CellWorkerState, CellStoreError> {
    let records = match &root {
        Some(root) => load(root)?,
        None => BTreeMap::new(),
    };
    Ok(CellWorkerState {
        root,
        records,
        retention,
        poisoned: false,
    })
}

/// What one command leaves the owner loop with.
enum LoopControl {
    Continue,
    Exit,
}

fn cell_handle(state: &mut CellWorkerState, command: CellCommand) -> LoopControl {
    match command {
        // Intercepted by the loop before dispatch (the ack must follow the
        // real state drop); unreachable here.
        CellCommand::Shutdown { .. } => LoopControl::Exit,
        CellCommand::Bootstrap { .. } => LoopControl::Continue,
        CellCommand::Consume {
            cell,
            invocation_id,
            principal,
            durability,
            reply,
        } => {
            let decision = if state.poisoned {
                Err(CellStoreError::Poisoned)
            } else {
                consume_locked(
                    &mut state.records,
                    state.root.as_deref(),
                    state.retention,
                    &cell,
                    &invocation_id,
                    &principal,
                    durability,
                )
            };
            let _ = reply.send(decision);
            LoopControl::Continue
        }
        CellCommand::Complete {
            cell,
            invocation_id,
            principal,
            reply,
        } => {
            let result = if state.poisoned {
                Err(CellStoreError::Poisoned)
            } else {
                complete_locked(
                    &mut state.records,
                    state.root.as_deref(),
                    state.retention,
                    &cell,
                    &invocation_id,
                    &principal,
                )
            };
            let _ = reply.send(result);
            LoopControl::Continue
        }
        CellCommand::InsertPayload {
            cell,
            invocation_id,
            principal,
            payload,
            reply,
        } => {
            let result = if state.poisoned {
                Err(CellStoreError::Poisoned)
            } else {
                insert_payload_locked(
                    &mut state.records,
                    state.retention,
                    &cell,
                    &invocation_id,
                    &principal,
                    payload,
                )
            };
            let _ = reply.send(result);
            LoopControl::Continue
        }
        CellCommand::Contains {
            cell,
            invocation_id,
            reply,
        } => {
            let found = state
                .records
                .keys()
                .any(|key| key.cell == cell && key.invocation_id == invocation_id);
            let _ = reply.send(found);
            LoopControl::Continue
        }
        CellCommand::Payload {
            cell,
            invocation_id,
            reply,
        } => {
            let payload = state
                .records
                .range(..)
                .find(|(key, _)| key.cell == cell && key.invocation_id == invocation_id)
                .and_then(|(_, record)| record.payload.clone());
            let _ = reply.send(payload);
            LoopControl::Continue
        }
        CellCommand::Remove {
            cell,
            invocation_id,
            reply,
        } => {
            let keys: Vec<CellKey> = state
                .records
                .keys()
                .filter(|key| key.cell == cell && key.invocation_id == invocation_id)
                .cloned()
                .collect();
            let removed = !keys.is_empty();
            for key in keys {
                state.records.remove(&key);
            }
            let _ = reply.send(removed);
            LoopControl::Continue
        }
        CellCommand::Keys { cell, reply } => {
            let mut ids: Vec<String> = state
                .records
                .keys()
                .filter(|key| key.cell == cell)
                .map(|key| key.invocation_id.clone())
                .collect();
            ids.sort();
            ids.dedup();
            let _ = reply.send(ids);
            LoopControl::Continue
        }
        CellCommand::Clear { cell, reply } => {
            let keys: Vec<CellKey> = state
                .records
                .keys()
                .filter(|key| key.cell == cell)
                .cloned()
                .collect();
            let count = keys.len();
            for key in keys {
                state.records.remove(&key);
            }
            let _ = reply.send(count);
            LoopControl::Continue
        }
        CellCommand::RecordCount { reply } => {
            let _ = reply.send(state.records.len());
            LoopControl::Continue
        }
        #[cfg(test)]
        CellCommand::InjectPanic => {
            // A mutation critical section that panics mid-way: the record is
            // inserted (the corrupted partial mutation) before the panic, so
            // the owner's in-memory state is dirty when the latch trips.
            let key = CellKey::new("injected-panic", "injected", BROKER_PRINCIPAL);
            state.records.insert(
                key,
                CellRecord {
                    outcome: CellOutcome::Unknown,
                    claimed: true,
                    durability: CellDurability::OneTime,
                    payload: None,
                    consumed_ms: 0,
                    completed_ms: None,
                },
            );
            panic!("injected cell store mutation panic");
        }
        #[cfg(test)]
        CellCommand::ConsumeCrashAfterPersist {
            cell,
            invocation_id,
            principal,
            durability,
            persisted,
        } => {
            // The durable pre-commit runs exactly as consume would; then the
            // "process" dies before the Granted reply can cross.
            let now = now_ms();
            enforce_retention_locked(&mut state.records, state.retention, now);
            let key = CellKey::new(&cell, &invocation_id, &principal);
            state.records.insert(
                key,
                CellRecord {
                    outcome: CellOutcome::Unknown,
                    claimed: true,
                    durability,
                    payload: None,
                    consumed_ms: now,
                    completed_ms: None,
                },
            );
            let persist_result = if durability == CellDurability::OneTime {
                persist_locked(state.root.as_deref(), &state.records)
            } else {
                Ok(())
            };
            let _ = persisted.send(persist_result);
            LoopControl::Exit
        }
    }
}

/// Durable record snapshot (one-time cells only) as the file payload.
fn durable_snapshot(records: &BTreeMap<CellKey, CellRecord>) -> DurableFile {
    let mut by_cell: BTreeMap<String, BTreeMap<String, DurableRecord>> = BTreeMap::new();
    for (key, record) in records {
        if record.durability != CellDurability::OneTime || record.payload.is_some() {
            continue;
        }
        by_cell.entry(key.cell.clone()).or_default().insert(
            key.invocation_id.clone(),
            DurableRecord {
                principal: key.principal.clone(),
                outcome: match record.outcome {
                    CellOutcome::Unknown => "unknown".to_owned(),
                    CellOutcome::Completed => "completed".to_owned(),
                },
                consumed_at_ms: record.consumed_ms,
                completed_at_ms: record.completed_ms,
            },
        );
    }
    DurableFile {
        version: DURABLE_VERSION,
        records: by_cell,
    }
}

#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn load(root: &Path) -> Result<BTreeMap<CellKey, CellRecord>, CellStoreError> {
    let path = cell_durable_path(root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(CellStoreError::Io(error)),
    };
    let file: DurableFile = serde_json::from_slice(&bytes).map_err(|error| {
        CellStoreError::CorruptDurable(format!("{}: {error}", path.display()))
    })?;
    if file.version != DURABLE_VERSION {
        return Err(CellStoreError::CorruptDurable(format!(
            "{}: unsupported version {}",
            path.display(),
            file.version
        )));
    }
    let mut records = BTreeMap::new();
    for (cell, invocations) in file.records {
        for (invocation_id, record) in invocations {
            let outcome = match record.outcome.as_str() {
                "unknown" => CellOutcome::Unknown,
                "completed" => CellOutcome::Completed,
                other => {
                    return Err(CellStoreError::CorruptDurable(format!(
                        "{}: record {cell}/{invocation_id} has unknown outcome {other}",
                        path.display()
                    )));
                }
            };
            records.insert(
                CellKey::new(&cell, &invocation_id, &record.principal),
                CellRecord {
                    outcome,
                    claimed: false,
                    durability: CellDurability::OneTime,
                    payload: None,
                    consumed_ms: record.consumed_at_ms,
                    completed_ms: record.completed_at_ms,
                },
            );
        }
    }
    Ok(records)
}

/// Persist the current one-time records atomically under the state root.
///
/// Follows the broker's durable-row discipline
/// (`ops/network.rs::persist_persistent_tap_realization`): strict
/// directory posture, temp file with `create_new`, `sync_data`, rename,
/// and a directory fsync - so a crash never leaves a half-written file.
/// Runs on the single owner thread (U6 "single-owner task" shape).
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn persist_locked(
    root: Option<&Path>,
    records: &BTreeMap<CellKey, CellRecord>,
) -> Result<(), CellStoreError> {
    let Some(root) = root else {
        return Ok(());
    };
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .mode(0o750)
                .create(root)
                .map_err(CellStoreError::Io)?;
            fs::symlink_metadata(root).map_err(CellStoreError::Io)?
        }
        Err(error) => return Err(CellStoreError::Io(error)),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 {
        return Err(CellStoreError::CorruptDurable(format!(
            "{}: state-cell root has wrong posture",
            root.display()
        )));
    }
    let row_path = cell_durable_path(root);
    let temp_path = root.join(format!(".{STATE_CELLS_FILE}.tmp"));
    match fs::symlink_metadata(&temp_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 => {
            return Err(CellStoreError::CorruptDurable(format!(
                "{}: temp file has wrong posture",
                temp_path.display()
            )));
        }
        Ok(_) => {
            fs::remove_file(&temp_path).map_err(CellStoreError::Io)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(CellStoreError::Io(error)),
    }
    let bytes = serde_json::to_vec(&durable_snapshot(records))
        .map_err(|error| CellStoreError::CorruptDurable(error.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .mode(0o640)
        .open(&temp_path)
        .map_err(CellStoreError::Io)?;
    if file.write_all(&bytes).is_err() || file.sync_data().is_err() {
        drop(file);
        let _ = fs::remove_file(&temp_path);
        return Err(CellStoreError::Io(std::io::Error::other(
            "state cell durable write failed",
        )));
    }
    drop(file);
    fs::rename(&temp_path, &row_path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        CellStoreError::Io(error)
    })?;
    std::fs::File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(CellStoreError::Io)
}

/// Enforce retention over replayable non-one-time outcome records.
///
/// One-time consumed markers and payload-bearing live state are exempt.
fn enforce_retention_locked(
    records: &mut BTreeMap<CellKey, CellRecord>,
    retention: RetentionPolicy,
    now: u64,
) {
    let stale_before = now.saturating_sub(retention.outcome_ttl_ms);
    records.retain(|_, record| {
        if record.durability != CellDurability::Ephemeral
            || record.payload.is_some()
            || record.claimed
        {
            return true;
        }
        record.consumed_ms >= stale_before
    });
    let mut per_cell: BTreeMap<String, Vec<CellKey>> = BTreeMap::new();
    for (key, record) in records.iter() {
        if record.durability == CellDurability::Ephemeral
            && record.payload.is_none()
            && !record.claimed
        {
            per_cell
                .entry(key.cell.clone())
                .or_default()
                .push(key.clone());
        }
    }
    for keys in per_cell.into_values() {
        let evict = keys
            .len()
            .saturating_sub(retention.max_ephemeral_outcome_records);
        if evict == 0 {
            continue;
        }
        let mut ordered = keys;
        ordered.sort_by_key(|key| {
            records
                .get(key)
                .map(|record| record.consumed_ms)
                .unwrap_or(0)
        });
        for key in ordered.into_iter().take(evict) {
            records.remove(&key);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Compare-and-consume one cell key, on the owner.
///
/// For a one-time cell the claim is committed durably (outcome `unknown`)
/// before `Granted` is returned, so a crash between the commit and the
/// effect reconciles on retry instead of granting twice.
fn consume_locked(
    records: &mut BTreeMap<CellKey, CellRecord>,
    root: Option<&Path>,
    retention: RetentionPolicy,
    cell: &str,
    invocation_id: &str,
    principal: &str,
    durability: CellDurability,
) -> Result<ConsumeDecision, CellStoreError> {
    // Retention applies to replayable non-one-time outcome records only;
    // the consume below may create one, so stale records go first.
    let now = now_ms();
    enforce_retention_locked(records, retention, now);
    let key = CellKey::new(cell, invocation_id, principal);
    // The principal is part of the key: a record for the same cell +
    // invocation id under a different principal must refuse the replay,
    // never fall through to a fresh grant (invocation ids alone never
    // gate one-time grants, KTD3).
    let Some(owner) = records
        .keys()
        .find(|candidate| candidate.cell == cell && candidate.invocation_id == invocation_id)
    else {
        records.insert(
            key,
            CellRecord {
                outcome: CellOutcome::Unknown,
                claimed: true,
                durability,
                payload: None,
                consumed_ms: now,
                completed_ms: None,
            },
        );
        if durability == CellDurability::OneTime {
            persist_locked(root, records)?;
        }
        return Ok(ConsumeDecision::Granted);
    };
    if owner.principal != key.principal {
        return Ok(ConsumeDecision::ForeignPrincipal);
    }
    let existing = records.get(&key).expect("owner is the requested key");
    if existing.claimed {
        return Ok(ConsumeDecision::InProgress);
    }
    match existing.outcome {
        CellOutcome::Completed => Ok(ConsumeDecision::Replayed),
        // A durable unknown left by a crashed owner: the retried
        // invocation reconciles (idempotently) under its invocation id.
        CellOutcome::Unknown => {
            let record = records.get_mut(&key).expect("key present");
            record.claimed = true;
            if durability == CellDurability::OneTime {
                persist_locked(root, records)?;
            }
            Ok(ConsumeDecision::Reconciled)
        }
    }
}

/// Record completion for one claimed key, on the owner.
fn complete_locked(
    records: &mut BTreeMap<CellKey, CellRecord>,
    root: Option<&Path>,
    retention: RetentionPolicy,
    cell: &str,
    invocation_id: &str,
    principal: &str,
) -> Result<(), CellStoreError> {
    let now = now_ms();
    enforce_retention_locked(records, retention, now);
    let key = CellKey::new(cell, invocation_id, principal);
    let Some(owner) = records
        .keys()
        .find(|candidate| candidate.cell == cell && candidate.invocation_id == invocation_id)
    else {
        return Err(CellStoreError::MissingRecord);
    };
    if owner.principal != key.principal {
        return Err(CellStoreError::ForeignPrincipal);
    }
    let record = records.get_mut(&key).expect("owner is the requested key");
    if record.outcome == CellOutcome::Completed && record.completed_ms.is_some() {
        return Ok(());
    }
    record.outcome = CellOutcome::Completed;
    record.claimed = false;
    record.completed_ms = Some(now);
    if record.durability == CellDurability::OneTime {
        persist_locked(root, records)?;
    }
    Ok(())
}

/// Insert one payload record into an ephemeral cell, on the owner.
fn insert_payload_locked(
    records: &mut BTreeMap<CellKey, CellRecord>,
    retention: RetentionPolicy,
    cell: &str,
    invocation_id: &str,
    principal: &str,
    payload: Arc<dyn Any + Send + Sync>,
) -> Result<(), CellStoreError> {
    let key = CellKey::new(cell, invocation_id, principal);
    enforce_retention_locked(records, retention, now_ms());
    records.insert(
        key,
        CellRecord {
            outcome: CellOutcome::Unknown,
            claimed: false,
            durability: CellDurability::Ephemeral,
            payload: Some(payload),
            consumed_ms: now_ms(),
            completed_ms: None,
        },
    );
    Ok(())
}

/// The broker process's cell store: the single owner of every declared cell.
///
/// Initialized at server start from the daemon state root (`run_server` →
/// [`crate::runtime`] `ServerConfig::state_dir`, the same root the daemon's
/// ProviderSet state lives under); uninitialized use stays in-memory so the
/// typed arms remain testable without a state root. First initialization
/// wins; `run_server` is the only production writer.
pub fn init_broker_store(state_dir: &Path) -> Result<(), CellStoreError> {
    let store = CellStore::open(&state_dir.join(STATE_CELLS_DIR))?;
    let _ = BROKER_STORE.set(store);
    Ok(())
}

static BROKER_STORE: OnceLock<CellStore> = OnceLock::new();

/// The broker process's cell store (in-memory until
/// [`init_broker_store`] runs).
pub fn broker_store() -> &'static CellStore {
    BROKER_STORE.get_or_init(CellStore::in_memory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn scratch(name: &str) -> tempfile::TempDir {
        let _ = crate::test_scratch_root();
        tempfile::tempdir().unwrap_or_else(|_| panic!("{name}: create scratch root"))
    }

    const LEASES: &str = "lifecycle-leases";

    fn consume_ok(store: &CellStore, id: &str, principal: &str) -> ConsumeDecision {
        store
            .consume(LEASES, id, principal, CellDurability::OneTime)
            .expect("consume succeeds")
    }

    #[test]
    fn grants_once_replays_refusal_and_completes_were_recorded() {
        let store = CellStore::in_memory();
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Granted
        );
        // In-process duplicate: the live claim refuses.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::InProgress
        );
        store.complete(LEASES, "inv-1", "alice").expect("complete");
        // Same invocation id replays the recorded outcome: refusal.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
        // A new consume attempt on the consumed one-time cell refuses: it is
        // a re-invocation of the same one-time grant, not a fresh grant.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
        // Replay under a different principal refuses (KTD3).
        assert_eq!(
            consume_ok(&store, "inv-1", "bob"),
            ConsumeDecision::ForeignPrincipal
        );
        // Completion is idempotent.
        store
            .complete(LEASES, "inv-1", "alice")
            .expect("recomplete");
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
    }

    #[test]
    fn cross_principal_consume_never_grants_a_second_time() {
        let store = CellStore::in_memory();
        // Alice claims the key.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Granted
        );
        // Bob consumes the same cell + invocation id: the principal is part
        // of the key, so this is a replay attempt, not a fresh grant - it
        // must refuse whether or not Alice completed.
        assert_eq!(
            consume_ok(&store, "inv-1", "bob"),
            ConsumeDecision::ForeignPrincipal
        );
        store
            .complete(LEASES, "inv-1", "alice")
            .expect("alice completes");
        assert_eq!(
            consume_ok(&store, "inv-1", "bob"),
            ConsumeDecision::ForeignPrincipal
        );
        // Grace completes her own grant exactly once.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
    }

    #[test]
    fn completion_for_missing_or_foreign_record_fails() {
        let store = CellStore::in_memory();
        assert!(matches!(
            store.complete(LEASES, "never-consumed", "alice"),
            Err(CellStoreError::MissingRecord)
        ));
        consume_ok(&store, "inv-1", "alice");
        assert!(matches!(
            store.complete(LEASES, "inv-1", "bob"),
            Err(CellStoreError::ForeignPrincipal)
        ));
    }

    #[test]
    fn one_time_cells_persist_completed_state_and_refuse_reconsume_after_restart() {
        let root = scratch("one-time-restart");
        let root_path = root.path().to_path_buf();
        {
            let store = CellStore::open(&root_path).expect("open first owner");
            assert_eq!(
                consume_ok(&store, "inv-1", "alice"),
                ConsumeDecision::Granted
            );
            store.complete(LEASES, "inv-1", "alice").expect("complete");
            // The completed marker is durable before the store is dropped.
            assert!(root_path.join(STATE_CELLS_FILE).exists());
        }
        // Restart: a fresh owner recovers the durable file.
        let store = CellStore::open(&root_path).expect("reopen after restart");
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
        // Restart-replay resistance holds per principal: a different
        // principal still refuses (the invocation is not replayable to it).
        assert_eq!(
            consume_ok(&store, "inv-1", "bob"),
            ConsumeDecision::ForeignPrincipal
        );
    }

    #[test]
    fn crash_between_durable_commit_and_effect_reconciles_without_double_grant_or_leak() {
        let root = scratch("crash-window");
        let root_path = root.path().to_path_buf();
        // Owner A consumes (durable pre-commit, outcome unknown) and crashes
        // before the effect records completion.
        {
            let store = CellStore::open(&root_path).expect("owner A");
            assert_eq!(
                consume_ok(&store, "inv-1", "alice"),
                ConsumeDecision::Granted
            );
            // No complete: the crash lands between commit and effect.
        }
        // Owner B restarts with the durable file.
        let store = CellStore::open(&root_path).expect("owner B");
        // The retried invocation reconciles under the same invocation id: the
        // idempotent effect re-runs - no silent leak.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Reconciled
        );
        store.complete(LEASES, "inv-1", "alice").expect("complete");
        // The grant is exercised exactly once: later consumes replay the
        // refusal - no double grant.
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
        assert_eq!(
            consume_ok(&store, "inv-1", "bob"),
            ConsumeDecision::ForeignPrincipal
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn crash_between_durable_persist_and_granted_response_regrants_exactly_once() {
        // U6 invariant (AE): the one-time claim is durable before Granted
        // returns; a crash in the window between the durable persist and the
        // Granted response must leave exactly one grant lineage across the
        // restart - never a double grant, never a silent leak.
        let root = scratch("crash-persist-window");
        let root_path = root.path().to_path_buf();
        let store = CellStore::open(&root_path).expect("open");
        let (persisted_tx, persisted_rx) = mpsc::sync_channel(1);
        store
            .owner
            .commands
            .send(CellCommand::ConsumeCrashAfterPersist {
                cell: LEASES.to_owned(),
                invocation_id: "inv-1".to_owned(),
                principal: "alice".to_owned(),
                durability: CellDurability::OneTime,
                persisted: persisted_tx,
            })
            .expect("send crash consume");
        // The durable pre-commit is on disk before the crash.
        persisted_rx
            .recv()
            .expect("durable persist completed")
            .expect("persist ok");
        assert!(root_path.join(STATE_CELLS_FILE).exists());
        // The owner died before the Granted response crossed: this handle is
        // dead, and every op on it fails closed.
        drop(store);
        // Restart: the durable unknown record reconciles - the grant line is
        // exercised exactly once.
        let restarted = CellStore::open(&root_path).expect("restart");
        assert_eq!(
            consume_ok(&restarted, "inv-1", "alice"),
            ConsumeDecision::Reconciled,
            "the crash-stale record reconciles under its invocation id"
        );
        restarted
            .complete(LEASES, "inv-1", "alice")
            .expect("complete");
        assert_eq!(
            consume_ok(&restarted, "inv-1", "alice"),
            ConsumeDecision::Replayed,
            "no double grant after the restart"
        );
        assert_eq!(
            consume_ok(&restarted, "inv-1", "bob"),
            ConsumeDecision::ForeignPrincipal
        );
    }

    #[test]
    fn panic_injected_mutation_poisons_consume_and_complete() {
        // U6 invariant: a panic inside a mutation critical section latches
        // the poison flag; subsequent consume/complete refuse with Poisoned,
        // never Granted/Reconciled - the one-time claim can never grant twice
        // after a corrupted store.
        let store = CellStore::in_memory();
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Granted
        );
        store.complete(LEASES, "inv-1", "alice").expect("complete");
        assert_eq!(store.record_count(), 1);

        // Panic inside a mutation critical section (worker-owned).
        store.inject_mutation_panic();

        // consume refuses with Poisoned for a fresh key and a replayed one -
        // never Granted/Reconciled.
        assert!(matches!(
            store.consume(LEASES, "inv-2", "alice", CellDurability::OneTime),
            Err(CellStoreError::Poisoned)
        ));
        assert!(matches!(
            store.consume(LEASES, "inv-1", "alice", CellDurability::OneTime),
            Err(CellStoreError::Poisoned)
        ));
        // complete refuses too; insert_payload is a mutation and refuses.
        assert!(matches!(
            store.complete(LEASES, "inv-1", "alice"),
            Err(CellStoreError::Poisoned)
        ));
        assert!(matches!(
            store.insert_payload(
                "runner-pidfd-registry",
                "runner-1",
                BROKER_PRINCIPAL,
                Arc::new(1u32),
            ),
            Err(CellStoreError::Poisoned)
        ));
        // Read-only ops stay fail-open (they cannot regrant a claim).
        assert_eq!(
            store.record_count(),
            2,
            "the recorded records plus the corrupted injected partial record"
        );
        assert!(store.contains(LEASES, "inv-1"));
        assert!(
            store.remove(LEASES, "inv-1"),
            "read-mostly cleanup still serves the map"
        );
    }

    #[test]
    fn concurrent_one_time_consumers_have_exactly_one_winner() {
        let root = scratch("concurrent-winner");
        let root_path = root.path().to_path_buf();
        let store = std::sync::Arc::new(CellStore::open(&root_path).expect("open"));
        const CONSUMERS: usize = 8;
        let barrier = Arc::new(Barrier::new(CONSUMERS));
        let mut winners = 0usize;
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..CONSUMERS {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    match store
                        .consume(LEASES, "inv-1", "alice", CellDurability::OneTime)
                        .expect("consume")
                    {
                        ConsumeDecision::Granted | ConsumeDecision::Reconciled => 1usize,
                        ConsumeDecision::InProgress
                        | ConsumeDecision::Replayed
                        | ConsumeDecision::ForeignPrincipal => 0usize,
                    }
                }));
            }
            for handle in handles {
                winners += handle.join().expect("consumer thread");
            }
        });
        // The single owner serializes the claims FIFO: exactly one winner.
        assert_eq!(winners, 1, "exactly one concurrent consumer wins");
        // The durable file survived the interleaved claims.
        let reopened = CellStore::open(&root_path).expect("reopen");
        // The survivors' claims died with the process: the record is an
        // unclaimed durable unknown, reconciled on retry - and the winner of
        // the whole population is still exactly one grant lineage.
        assert_eq!(
            consume_ok(&reopened, "inv-1", "alice"),
            ConsumeDecision::Reconciled
        );
        reopened
            .complete(LEASES, "inv-1", "alice")
            .expect("complete");
        assert_eq!(
            consume_ok(&reopened, "inv-1", "alice"),
            ConsumeDecision::Replayed
        );
    }

    #[test]
    fn ephemeral_cells_reset_on_restart_and_replay_in_process() {
        let root = scratch("ephemeral-restart");
        let root_path = root.path().to_path_buf();
        let store = CellStore::open(&root_path).expect("open");
        assert_eq!(
            store.consume(
                "runner-registry",
                "runner-1",
                "broker",
                CellDurability::Ephemeral
            ),
            Ok(ConsumeDecision::Granted)
        );
        // Ephemeral in-process replay: same rules until completed.
        assert_eq!(
            store.consume(
                "runner-registry",
                "runner-1",
                "broker",
                CellDurability::Ephemeral
            ),
            Ok(ConsumeDecision::InProgress)
        );
        store
            .complete("runner-registry", "runner-1", "broker")
            .expect("complete");
        assert_eq!(
            store.consume(
                "runner-registry",
                "runner-1",
                "broker",
                CellDurability::Ephemeral
            ),
            Ok(ConsumeDecision::Replayed)
        );
        // Ephemeral records never reach the durable file: a restarted owner
        // starts empty even though the completed record existed in-process.
        let restarted = CellStore::open(&root_path).expect("restart");
        assert_eq!(restarted.record_count(), 0);
    }

    #[test]
    fn payload_cells_round_trip_and_scope_by_cell() {
        let store = CellStore::in_memory();
        store
            .insert_payload(
                "runner-pidfd-registry",
                "runner-1",
                BROKER_PRINCIPAL,
                Arc::new(42u32),
            )
            .expect("insert");
        store
            .insert_payload(
                "runner-pidfd-registry",
                "runner-2",
                BROKER_PRINCIPAL,
                Arc::new(7u32),
            )
            .expect("insert");
        assert!(store.contains("runner-pidfd-registry", "runner-1"));
        assert!(!store.contains("runner-pidfd-registry", "missing"));
        assert!(!store.contains("other-cell", "runner-1"));
        let payload = store
            .payload("runner-pidfd-registry", "runner-1")
            .expect("payload");
        assert_eq!(*payload.downcast::<u32>().expect("downcast"), 42u32);
        assert_eq!(
            store.keys("runner-pidfd-registry"),
            vec!["runner-1".to_owned(), "runner-2".to_owned()]
        );
        assert!(store.remove("runner-pidfd-registry", "runner-2"));
        assert!(!store.remove("runner-pidfd-registry", "runner-2"));
        assert_eq!(store.clear("runner-pidfd-registry"), 1);
        assert_eq!(store.record_count(), 0);
    }

    #[test]
    fn retention_never_re_enables_a_consumed_one_time_marker() {
        let root = scratch("retention-exemption");
        let root_path = root.path().to_path_buf();
        let store = CellStore::with_retention(
            Some(root_path.clone()),
            RetentionPolicy {
                outcome_ttl_ms: 0, // every ephemeral outcome record is stale
                max_ephemeral_outcome_records: 1,
            },
        );
        // The one-time marker: consumed and completed.
        assert_eq!(
            store.consume("grant-g", "grant-1", "alice", CellDurability::OneTime),
            Ok(ConsumeDecision::Granted)
        );
        store
            .complete("grant-g", "grant-1", "alice")
            .expect("complete");
        // Comparable non-one-time outcome records: three consumable cells.
        for id in ["r-1", "r-2", "r-3"] {
            assert_eq!(
                store.consume("replayable-r", id, "bob", CellDurability::Ephemeral),
                Ok(ConsumeDecision::Granted)
            );
            store
                .complete("replayable-r", id, "bob")
                .expect("complete replayable record");
        }
        // The next mutation enforces retention: the ephemeral outcome records
        // shrink to the cap; the one-time marker stays.
        assert_eq!(
            store.consume("replayable-r", "r-4", "bob", CellDurability::Ephemeral),
            Ok(ConsumeDecision::Granted)
        );
        store
            .complete("replayable-r", "r-4", "bob")
            .expect("complete r-4");
        // One more completed replayable record pushes the capped cell past
        // its bound; the next mutation enforces the cap.
        assert_eq!(
            store.consume("replayable-r", "r-5", "bob", CellDurability::Ephemeral),
            Ok(ConsumeDecision::Granted)
        );
        store
            .complete("replayable-r", "r-5", "bob")
            .expect("complete r-5");
        // The replayable cell stays bounded at cap+1 between mutations (the
        // completion that pushes it past the cap is the next mutation's
        // evictable tail), while the one-time marker is immune in both
        // dimensions: eviction and count.
        assert_eq!(
            store.record_count(),
            3,
            "one-time marker + capped replayable cell (cap+1 tail)"
        );
        // The consumed one-time marker still refuses after the cap would
        // have evicted a comparable non-one-time record.
        assert_eq!(
            store.consume("grant-g", "grant-1", "alice", CellDurability::OneTime),
            Ok(ConsumeDecision::Replayed)
        );
        assert_eq!(
            store.consume("grant-g", "grant-1", "bob", CellDurability::OneTime),
            Ok(ConsumeDecision::ForeignPrincipal)
        );
        // And it remained durable across a restart.
        let reopened = CellStore::open(&root_path).expect("reopen");
        assert_eq!(
            reopened.consume("grant-g", "grant-1", "alice", CellDurability::OneTime),
            Ok(ConsumeDecision::Replayed)
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn corrupt_durable_file_fails_closed() {
        let root = scratch("corrupt");
        let store = CellStore::open(root.path()).expect("empty open");
        assert_eq!(
            consume_ok(&store, "inv-1", "alice"),
            ConsumeDecision::Granted
        );
        store.complete(LEASES, "inv-1", "alice").expect("complete");
        let path = root.path().join(STATE_CELLS_FILE);
        std::fs::write(&path, b"not json").expect("corrupt the file");
        assert!(matches!(
            CellStore::open(root.path()),
            Err(CellStoreError::CorruptDurable(_))
        ));
    }
}
