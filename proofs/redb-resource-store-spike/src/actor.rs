use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot};

use crate::OracleCheckpoint;
use crate::disk::{AppliedGroup, DiskReceipt, DiskStore};
use crate::model::{ChangeBatch, Mutation, ResourceKey, StoreError, StoreResult, WriteReceipt};

const WRITE_QUEUE_CAPACITY: usize = 256;
const MAX_GROUP_COMMIT: usize = 16;
const DELIVERY_QUEUE_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintKind {
    ResourceChanged,
    OwnedResourceChanged,
}

#[derive(Debug, Clone)]
pub struct Hint {
    pub kind: HintKind,
    pub resource: ResourceKey,
    pub changed_resource: Option<ResourceKey>,
    pub revision: u64,
    pub operation_id: String,
    pub committed_at: Instant,
}

/// One shared immutable ChangeBatch, fanned out to matching watchers.
///
/// Watchers receive a refcount handle rather than a private deep copy, so N
/// matching watchers cost one batch plus N pointers instead of N batches.
pub type SharedChangeBatch = Arc<ChangeBatch>;

pub struct Watch {
    receiver: mpsc::Receiver<SharedChangeBatch>,
}

impl Watch {
    pub async fn recv(&mut self) -> Option<SharedChangeBatch> {
        self.receiver.recv().await
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ActorStats {
    pub committed_writes: u64,
    pub grouped_writes: u64,
    pub commits: u64,
    pub watch_delivery_failures: u64,
    pub hint_delivery_failures: u64,
    /// Bounded backend signals. Fixed cardinality: every field below is a
    /// single process-wide counter or gauge with no per-watch, per-resource,
    /// or per-revision label, so the signal set cannot grow with the fixture.
    pub replay_range_seeks: u64,
    pub replay_rows_scanned: u64,
    pub replay_rows_decoded: u64,
    pub shared_batches: u64,
    pub shared_batch_fanout_refs: u64,
    pub write_queue_depth: u64,
    pub write_queue_capacity: u64,
}

#[derive(Clone)]
pub struct Store {
    sender: mpsc::Sender<Command>,
}

struct WriteRequest {
    sequence: u64,
    principal: String,
    mutation: Mutation,
    response: oneshot::Sender<StoreResult<WriteReceipt>>,
}

enum Command {
    Write(Box<WriteRequest>),
    Watch {
        after_revision: u64,
        resource_types: BTreeSet<String>,
        sender: mpsc::Sender<SharedChangeBatch>,
        response: oneshot::Sender<StoreResult<()>>,
    },
    HintConsumer {
        resource_types: BTreeSet<String>,
        sender: mpsc::Sender<Hint>,
        response: oneshot::Sender<StoreResult<()>>,
    },
    Verify {
        oracle: BTreeMap<ResourceKey, crate::model::Resource>,
        response: oneshot::Sender<StoreResult<()>>,
    },
    VerifyTransition {
        checkpoint: Box<OracleCheckpoint>,
        response: oneshot::Sender<StoreResult<()>>,
    },
    CurrentRevision {
        response: oneshot::Sender<StoreResult<u64>>,
    },
    Stats {
        response: oneshot::Sender<ActorStats>,
    },
}

struct WatchRegistration {
    resource_types: BTreeSet<String>,
    sender: mpsc::Sender<SharedChangeBatch>,
}

struct HintRegistration {
    resource_types: BTreeSet<String>,
    sender: mpsc::Sender<Hint>,
}

#[derive(Default)]
struct FairScheduler {
    queues: BTreeMap<String, VecDeque<WriteRequest>>,
    ring: VecDeque<String>,
    len: usize,
}

impl FairScheduler {
    fn push(&mut self, request: WriteRequest) {
        let principal = request.principal.clone();
        let queue = self.queues.entry(principal.clone()).or_default();
        if queue.is_empty() {
            self.ring.push_back(principal);
        }
        queue.push_back(request);
        self.len += 1;
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn has_earlier_for_resource(&self, sequence: u64, key: &ResourceKey) -> bool {
        self.queues.values().any(|queue| {
            queue
                .iter()
                .any(|request| request.sequence < sequence && request.mutation.resource.key == *key)
        })
    }

    fn pop_group(&mut self) -> Vec<WriteRequest> {
        let mut group = Vec::with_capacity(MAX_GROUP_COMMIT);
        let mut selected_resources = BTreeSet::new();
        let mut stalled = 0;
        while group.len() < MAX_GROUP_COMMIT && !self.ring.is_empty() {
            if stalled >= self.ring.len() {
                break;
            }
            let principal = self.ring.pop_front().expect("ring is not empty");
            let mut request = self
                .queues
                .get_mut(&principal)
                .and_then(VecDeque::pop_front)
                .expect("active principal has a request");
            let key = request.mutation.resource.key.clone();
            if selected_resources.contains(&key)
                || self.has_earlier_for_resource(request.sequence, &key)
            {
                self.queues
                    .get_mut(&principal)
                    .expect("principal queue exists")
                    .push_front(request);
                self.ring.push_back(principal);
                stalled += 1;
                continue;
            }

            selected_resources.insert(key);
            self.len -= 1;
            stalled = 0;
            if self
                .queues
                .get(&principal)
                .is_some_and(|queue| !queue.is_empty())
            {
                self.ring.push_back(principal);
            } else {
                self.queues.remove(&principal);
            }
            request.sequence = 0;
            group.push(request);
        }
        group
    }
}

struct Actor {
    disk: DiskStore,
    receiver: mpsc::Receiver<Command>,
    scheduler: FairScheduler,
    watches: Vec<WatchRegistration>,
    hints: Vec<HintRegistration>,
    uid_index: BTreeMap<String, ResourceKey>,
    stats: ActorStats,
    sequence: u64,
}

impl Store {
    pub async fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref().to_owned();
        let (sender, receiver) = mpsc::channel(WRITE_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = oneshot::channel();
        tokio::task::spawn_blocking(move || match DiskStore::open_or_create(&path) {
            Ok(disk) => match disk.uid_index() {
                Ok(uid_index) => {
                    let _ = ready_sender.send(Ok(()));
                    Actor {
                        disk,
                        receiver,
                        scheduler: FairScheduler::default(),
                        watches: Vec::new(),
                        hints: Vec::new(),
                        uid_index,
                        stats: ActorStats::default(),
                        sequence: 0,
                    }
                    .run();
                }
                Err(error) => {
                    let _ = ready_sender.send(Err(error));
                }
            },
            Err(error) => {
                let _ = ready_sender.send(Err(error));
            }
        });
        ready_receiver.await.map_err(|_| StoreError::Closed)??;
        Ok(Self { sender })
    }

    pub async fn put(
        &self,
        principal: impl Into<String>,
        mutation: Mutation,
    ) -> StoreResult<WriteReceipt> {
        let (response, receiver) = oneshot::channel();
        let request = WriteRequest {
            sequence: 0,
            principal: principal.into(),
            mutation,
            response,
        };
        self.sender
            .try_send(Command::Write(Box::new(request)))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => StoreError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => StoreError::Closed,
            })?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn watch(
        &self,
        after_revision: u64,
        resource_types: BTreeSet<String>,
    ) -> StoreResult<Watch> {
        let (sender, receiver) = mpsc::channel(DELIVERY_QUEUE_CAPACITY);
        let (response, ready) = oneshot::channel();
        self.sender
            .send(Command::Watch {
                after_revision,
                resource_types,
                sender,
                response,
            })
            .await
            .map_err(|_| StoreError::Closed)?;
        ready.await.map_err(|_| StoreError::Closed)??;
        Ok(Watch { receiver })
    }

    pub async fn hint_consumer(
        &self,
        resource_types: BTreeSet<String>,
    ) -> StoreResult<mpsc::Receiver<Hint>> {
        let (sender, receiver) = mpsc::channel(DELIVERY_QUEUE_CAPACITY);
        let (response, ready) = oneshot::channel();
        self.sender
            .send(Command::HintConsumer {
                resource_types,
                sender,
                response,
            })
            .await
            .map_err(|_| StoreError::Closed)?;
        ready.await.map_err(|_| StoreError::Closed)??;
        Ok(receiver)
    }

    pub async fn verify(
        &self,
        oracle: &BTreeMap<ResourceKey, crate::model::Resource>,
    ) -> StoreResult<()> {
        let (response, receiver) = oneshot::channel();
        self.sender
            .send(Command::Verify {
                oracle: oracle.clone(),
                response,
            })
            .await
            .map_err(|_| StoreError::Closed)?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn current_revision(&self) -> StoreResult<u64> {
        let (response, receiver) = oneshot::channel();
        self.sender
            .send(Command::CurrentRevision { response })
            .await
            .map_err(|_| StoreError::Closed)?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn verify_transition(&self, checkpoint: OracleCheckpoint) -> StoreResult<()> {
        let (response, receiver) = oneshot::channel();
        self.sender
            .send(Command::VerifyTransition {
                checkpoint: Box::new(checkpoint),
                response,
            })
            .await
            .map_err(|_| StoreError::Closed)?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn stats(&self) -> StoreResult<ActorStats> {
        let (response, receiver) = oneshot::channel();
        self.sender
            .send(Command::Stats { response })
            .await
            .map_err(|_| StoreError::Closed)?;
        receiver.await.map_err(|_| StoreError::Closed)
    }
}

impl Actor {
    fn run(mut self) {
        let mut deferred = None;
        loop {
            let command = deferred.take().or_else(|| self.receiver.blocking_recv());
            let Some(command) = command else {
                break;
            };
            match command {
                Command::Write(request) => {
                    self.enqueue(*request);
                    while self.scheduler.len < WRITE_QUEUE_CAPACITY {
                        match self.receiver.try_recv() {
                            Ok(Command::Write(request)) => self.enqueue(*request),
                            Ok(control) => {
                                deferred = Some(control);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    self.flush_writes();
                }
                control => self.handle_control(control),
            }
        }
    }

    fn enqueue(&mut self, mut request: WriteRequest) {
        request.sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        self.scheduler.push(request);
    }

    fn flush_writes(&mut self) {
        while !self.scheduler.is_empty() {
            let group = self.scheduler.pop_group();
            if group.is_empty() {
                break;
            }
            let mutations = group
                .iter()
                .map(|request| request.mutation.clone())
                .collect::<Vec<_>>();
            match self.disk.apply_group(&mutations) {
                Ok(outcome) => self.finish_group(group, outcome),
                Err(error) => {
                    for request in group {
                        let _ = request.response.send(Err(error.clone()));
                    }
                }
            }
        }
    }

    fn finish_group(&mut self, group: Vec<WriteRequest>, outcome: AppliedGroup) {
        let committed_at = Instant::now();
        let batch_size = outcome
            .receipts
            .iter()
            .filter(|result| result.is_ok())
            .count();
        if let Some(batch) = outcome.batch.as_ref() {
            self.stats.commits += 1;
            self.stats.committed_writes += u64::try_from(batch_size).unwrap();
            if batch_size > 1 {
                self.stats.grouped_writes += u64::try_from(batch_size).unwrap();
            }
            for entry in &batch.entries {
                self.uid_index
                    .insert(entry.resource.uid.clone(), entry.resource.key.clone());
            }
            self.dispatch_watch(batch);
            self.dispatch_hints(batch, committed_at);
        }
        debug_assert_eq!(outcome.revision.is_some(), outcome.batch.is_some());
        for (request, result) in group.into_iter().zip(outcome.receipts) {
            let result = result.map(|DiskReceipt { resource, ordinal }| WriteReceipt {
                revision: resource.revision,
                resource,
                ordinal,
                batch_size,
                committed_at,
            });
            let _ = request.response.send(result);
        }
    }

    /// Fan one shared immutable ChangeBatch out to every matching watcher.
    ///
    /// Watchers that share a filter share a single `Arc<ChangeBatch>`, so the
    /// per-commit cost is one materialized batch per distinct filter plus one
    /// refcount bump per watcher, not one deep clone per watcher. When a
    /// filter admits every entry the unfiltered batch is shared directly and
    /// nothing is copied at all.
    fn dispatch_watch(&mut self, batch: &ChangeBatch) {
        if self.watches.is_empty() {
            return;
        }
        let unfiltered: SharedChangeBatch = Arc::new(batch.clone());
        let mut shared: BTreeMap<BTreeSet<String>, SharedChangeBatch> = BTreeMap::new();
        let mut materialized = 1_u64;
        let mut fanout_refs = 0_u64;
        let stats = &mut self.stats;
        self.watches.retain(|watch| {
            let shared_batch = shared
                .entry(watch.resource_types.clone())
                .or_insert_with(|| {
                    if batch.entries.iter().all(|entry| {
                        watch
                            .resource_types
                            .contains(&entry.resource.key.resource_type)
                    }) {
                        Arc::clone(&unfiltered)
                    } else {
                        let mut filtered = batch.clone();
                        filtered.entries.retain(|entry| {
                            watch
                                .resource_types
                                .contains(&entry.resource.key.resource_type)
                        });
                        materialized += 1;
                        Arc::new(filtered)
                    }
                })
                .clone();
            fanout_refs += 1;
            match watch.sender.try_send(shared_batch) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    stats.watch_delivery_failures += 1;
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
        self.stats.shared_batches += materialized;
        self.stats.shared_batch_fanout_refs += fanout_refs;
    }

    fn dispatch_hints(&mut self, batch: &ChangeBatch, committed_at: Instant) {
        let mut hints = Vec::new();
        for entry in &batch.entries {
            hints.push(Hint {
                kind: HintKind::ResourceChanged,
                resource: entry.resource.key.clone(),
                changed_resource: None,
                revision: batch.revision,
                operation_id: entry.operation_id.clone(),
                committed_at,
            });
            if let Some(owner_uid) = &entry.resource.owner_uid
                && let Some(owner) = self.uid_index.get(owner_uid)
            {
                hints.push(Hint {
                    kind: HintKind::OwnedResourceChanged,
                    resource: owner.clone(),
                    changed_resource: Some(entry.resource.key.clone()),
                    revision: batch.revision,
                    operation_id: entry.operation_id.clone(),
                    committed_at,
                });
            }
        }
        self.hints.retain(|consumer| {
            for hint in hints.iter().filter(|hint| {
                consumer
                    .resource_types
                    .contains(&hint.resource.resource_type)
            }) {
                match consumer.sender.try_send(hint.clone()) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.stats.hint_delivery_failures += 1;
                        return false;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return false,
                }
            }
            true
        });
    }

    fn handle_control(&mut self, command: Command) {
        match command {
            Command::Watch {
                after_revision,
                resource_types,
                sender,
                response,
            } => {
                // Replay streams one range-seeked row at a time. Rows at or
                // below `after_revision` are never read and never decoded, and
                // each decoded batch is filtered, shared, sent, and dropped
                // before the next row is read, so no older complete envelope
                // and no whole-log vector is ever materialized.
                let mut scan = crate::disk::ReplayScan::default();
                let result = self
                    .disk
                    .stream_revision_batches_after(after_revision, |mut batch| {
                        batch.entries.retain(|entry| {
                            resource_types.contains(&entry.resource.key.resource_type)
                        });
                        sender
                            .try_send(Arc::new(batch))
                            .map_err(|_| StoreError::Backpressure)
                    })
                    .map(|observed| {
                        scan = observed;
                    });
                self.stats.replay_range_seeks += scan.range_seeks;
                self.stats.replay_rows_scanned += scan.rows_scanned;
                self.stats.replay_rows_decoded += scan.rows_decoded;
                if result.is_ok() {
                    self.watches.push(WatchRegistration {
                        resource_types,
                        sender,
                    });
                }
                let _ = response.send(result);
            }
            Command::HintConsumer {
                resource_types,
                sender,
                response,
            } => {
                self.hints.push(HintRegistration {
                    resource_types,
                    sender,
                });
                let _ = response.send(Ok(()));
            }
            Command::Verify { oracle, response } => {
                let _ = response.send(self.disk.verify(&oracle));
            }
            Command::VerifyTransition {
                checkpoint,
                response,
            } => {
                let _ = response.send(self.disk.verify_transition(&checkpoint));
            }
            Command::CurrentRevision { response } => {
                let _ = response.send(self.disk.current_revision());
            }
            Command::Stats { response } => {
                self.stats.write_queue_depth = u64::try_from(self.scheduler.len).unwrap_or(u64::MAX);
                self.stats.write_queue_capacity = WRITE_QUEUE_CAPACITY as u64;
                let _ = response.send(self.stats);
            }
            Command::Write(_) => unreachable!("writes are handled separately"),
        }
    }
}
