//! Fair single-writer actor, bounded reads, replay, and shared live delivery.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use d2b_contracts::v3::{ResourceRef, ResourceTypeName, ZoneId, ZoneRevision};
use d2b_resource_store::{
    StoreError, StoreFilter, StoreGetRequest, StoreInspectSchemaRequest, StoreListRequest,
    StoreListResult, StoreProjection, StoreResolveRequest, StoreResolvedIdentity, StoredResource,
    StoredSchema,
};
use redb::{Database, ReadableDatabase, ReadableTable};
use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot};

use crate::backup::LogicalBackup;
use crate::revision_log::{WatchCoordinator, WatchRegistrationId, WatchSelector, WatchStream};
use crate::transaction::{
    API_SCHEMAS, ChangeBatch, CommittedGroup, RESOURCES, ResourceRecord, StoreMeta, VerifiedWrite,
    apply_group, backpressure, current_meta, decode, resource_key, stored_resource, timeout,
};
use crate::{KeySpace, ValueKind};
use d2b_resource_store::mutation_seal::OpenedMutation;

/// Bounded public writer admission queue.
pub const WRITE_QUEUE_CAPACITY: usize = 256;
/// Maximum independent mutation requests in one crash-safe commit.
pub const GROUP_COMMIT_MAX: usize = 16;
/// Dedicated blocking MVCC read workers.
pub const READ_POOL_THREADS: usize = 4;
/// Maximum read transactions admitted at once.
pub const MAX_CONCURRENT_READS: usize = 16;
/// Worker-enforced lifetime ceiling for an admitted read transaction.
pub const READ_LIFETIME: Duration = Duration::from_millis(250);

/// Lightweight filtered view over one immutable decoded batch.
#[derive(Clone)]
pub struct SharedChangeBatch {
    batch: Arc<ChangeBatch>,
    indices: Arc<[usize]>,
}

impl core::fmt::Debug for SharedChangeBatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedChangeBatch")
            .field("revision", &self.revision())
            .field("entry_count", &self.indices.len())
            .finish()
    }
}

impl SharedChangeBatch {
    pub fn revision(&self) -> ZoneRevision {
        self.batch.revision()
    }

    pub(crate) fn batch_arc(&self) -> Arc<ChangeBatch> {
        Arc::clone(&self.batch)
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = &crate::transaction::ChangeEntry> {
        self.indices
            .iter()
            .map(|index| &self.batch.entries()[*index])
    }

    pub fn shares_batch_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.batch, &other.batch)
    }
}

/// Fixed-cardinality backend signal snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendSignals {
    pub revision_range_seeks: u64,
    pub replay_rows_scanned: u64,
    pub replay_rows_decoded: u64,
    pub shared_immutable_batches: u64,
    pub fanout_references: u64,
    pub writer_queue_depth: u64,
    pub writer_queue_capacity: u64,
}

#[derive(Default)]
pub(crate) struct SignalCounters {
    revision_range_seeks: AtomicU64,
    replay_rows_scanned: AtomicU64,
    replay_rows_decoded: AtomicU64,
    shared_immutable_batches: AtomicU64,
    fanout_references: AtomicU64,
    writer_queue_depth: AtomicU64,
}

impl SignalCounters {
    pub(crate) fn snapshot(&self) -> BackendSignals {
        BackendSignals {
            revision_range_seeks: self.revision_range_seeks.load(Ordering::Relaxed),
            replay_rows_scanned: self.replay_rows_scanned.load(Ordering::Relaxed),
            replay_rows_decoded: self.replay_rows_decoded.load(Ordering::Relaxed),
            shared_immutable_batches: self.shared_immutable_batches.load(Ordering::Relaxed),
            fanout_references: self.fanout_references.load(Ordering::Relaxed),
            writer_queue_depth: self.writer_queue_depth.load(Ordering::Relaxed),
            writer_queue_capacity: WRITE_QUEUE_CAPACITY as u64,
        }
    }

    fn record_shared_batch(&self) {
        self.shared_immutable_batches
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_fanout_reference(&self) {
        self.fanout_references.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct WriterHandle {
    sender: Option<mpsc::Sender<WriterCommand>>,
    signals: Arc<SignalCounters>,
    write_permits: Arc<tokio::sync::Semaphore>,
    thread: Option<std::thread::JoinHandle<()>>,
    quarantined: Arc<AtomicBool>,
}

impl WriterHandle {
    pub(crate) fn start(
        database: Arc<Database>,
        signals: Arc<SignalCounters>,
        watch_coordinator: Arc<std::sync::Mutex<WatchCoordinator>>,
    ) -> Result<Self, StoreError> {
        let (sender, receiver) = mpsc::channel(WRITE_QUEUE_CAPACITY);
        crate::transaction::set_clean_shutdown(&database, false)?;
        let actor_signals = Arc::clone(&signals);
        let quarantined = Arc::new(AtomicBool::new(false));
        let actor_quarantined = Arc::clone(&quarantined);
        let actor_watch_coordinator = Arc::clone(&watch_coordinator);
        let thread = std::thread::Builder::new()
            .name("d2b-redb-writer".to_owned())
            .spawn(move || {
                WriterActor::new(
                    database,
                    receiver,
                    actor_signals,
                    actor_quarantined,
                    actor_watch_coordinator,
                )
                .run();
            })
            .map_err(|_| crate::transaction::integrity("writer-actor-start-failed"))?;
        Ok(Self {
            sender: Some(sender),
            signals,
            write_permits: Arc::new(tokio::sync::Semaphore::new(WRITE_QUEUE_CAPACITY)),
            thread: Some(thread),
            quarantined,
        })
    }

    pub(crate) async fn commit(
        &self,
        opened: OpenedMutation,
    ) -> Result<d2b_resource_store::StoreCommitResult, StoreError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(crate::transaction::quarantined());
        }
        if opened.body().mutations.is_empty() {
            return Err(crate::transaction::integrity("empty-verified-mutation"));
        }
        if opened.body().mutations.len() > d2b_contracts::v3::MAX_BATCH_MUTATIONS {
            return Err(crate::transaction::integrity(
                "verified-mutation-over-limit",
            ));
        }
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?;
        let queue_permit = Arc::clone(&self.write_permits)
            .try_acquire_owned()
            .map_err(|_| backpressure())?;
        let (response, receiver) = oneshot::channel();
        let principal = opened.body().authorization.subject_uid.as_str().to_owned();
        let mut resources = opened
            .body()
            .mutations
            .iter()
            .flat_map(|prepared| {
                [
                    Some(prepared.mutation().target.clone()),
                    prepared.mutation().owner.clone(),
                ]
            })
            .flatten()
            .collect::<Vec<_>>();
        resources.sort();
        resources.dedup();
        self.signals
            .writer_queue_depth
            .fetch_add(1, Ordering::Relaxed);
        if let Err(error) = sender.try_send(WriterCommand::Commit(Box::new(WriteRequest {
            sequence: 0,
            principal,
            resources,
            mutation: VerifiedWrite::from_opened(opened),
            queue_permit,
            response,
        }))) {
            self.signals
                .writer_queue_depth
                .fetch_sub(1, Ordering::Relaxed);
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => backpressure(),
                mpsc::error::TrySendError::Closed(_) => {
                    crate::transaction::integrity("writer-closed")
                }
            });
        }
        receiver
            .await
            .map_err(|_| crate::transaction::integrity("writer-response-closed"))?
    }

    pub(crate) async fn replay(
        &self,
        after_revision: u64,
        resource_types: BTreeSet<ResourceTypeName>,
        visit: impl FnMut(SharedChangeBatch) -> Result<(), StoreError> + Send + 'static,
    ) -> Result<ZoneRevision, StoreError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(crate::transaction::quarantined());
        }
        let (response, ready) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?
            .send(WriterCommand::Replay {
                after_revision,
                resource_types,
                visit: Box::new(visit),
                response,
            })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        let high_water = ready
            .await
            .map_err(|_| crate::transaction::integrity("watch-replay-closed"))??;
        Ok(ZoneRevision::new(high_water))
    }

    pub(crate) async fn watch(
        &self,
        after_revision: ZoneRevision,
        selector: WatchSelector,
        initial_credits: u32,
    ) -> Result<(WatchStream, ZoneRevision), StoreError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(crate::transaction::quarantined());
        }
        let (response, receiver) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?
            .send(WriterCommand::Watch {
                after_revision,
                selector,
                initial_credits,
                response,
            })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        receiver
            .await
            .map_err(|_| crate::transaction::integrity("watch-response-closed"))?
    }

    pub(crate) async fn acknowledge_watch(
        &self,
        id: WatchRegistrationId,
        revision: ZoneRevision,
    ) -> Result<(), StoreError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(crate::transaction::quarantined());
        }
        let (response, receiver) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?
            .send(WriterCommand::AcknowledgeWatch {
                id,
                revision,
                response,
            })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        receiver
            .await
            .map_err(|_| crate::transaction::integrity("watch-ack-response-closed"))?
    }

    pub(crate) async fn unregister_watch(
        &self,
        id: WatchRegistrationId,
    ) -> Result<Option<ZoneRevision>, StoreError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(crate::transaction::quarantined());
        }
        let (response, receiver) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?
            .send(WriterCommand::UnregisterWatch { id, response })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        receiver
            .await
            .map_err(|_| crate::transaction::integrity("watch-unregister-response-closed"))?
    }

    pub(crate) async fn backup(
        &self,
        identity: crate::StoreIdentity,
    ) -> Result<LogicalBackup, StoreError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(crate::transaction::quarantined());
        }
        let (response, receiver) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?
            .send(WriterCommand::Backup { identity, response })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        receiver
            .await
            .map_err(|_| crate::transaction::integrity("writer-backup-response-closed"))?
    }

    pub(crate) async fn shutdown(&mut self) -> Result<(), StoreError> {
        let sender = self
            .sender
            .take()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?;
        let (response, receiver) = oneshot::channel();
        sender
            .send(WriterCommand::Shutdown { response })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        receiver
            .await
            .map_err(|_| crate::transaction::integrity("writer-shutdown-response-closed"))??;
        if self
            .thread
            .take()
            .ok_or_else(|| crate::transaction::integrity("writer-thread-missing"))?
            .join()
            .is_err()
        {
            return Err(crate::transaction::integrity("writer-thread-failed"));
        }
        Ok(())
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) struct WriteRequest {
    pub(crate) sequence: u64,
    pub(crate) principal: String,
    pub(crate) resources: Vec<ResourceRef>,
    pub(crate) mutation: VerifiedWrite,
    pub(crate) queue_permit: OwnedSemaphorePermit,
    pub(crate) response: oneshot::Sender<Result<d2b_resource_store::StoreCommitResult, StoreError>>,
}

enum WriterCommand {
    Commit(Box<WriteRequest>),
    Replay {
        after_revision: u64,
        resource_types: BTreeSet<ResourceTypeName>,
        visit: Box<dyn FnMut(SharedChangeBatch) -> Result<(), StoreError> + Send>,
        response: oneshot::Sender<Result<u64, StoreError>>,
    },
    Watch {
        after_revision: ZoneRevision,
        selector: WatchSelector,
        initial_credits: u32,
        response: oneshot::Sender<Result<(WatchStream, ZoneRevision), StoreError>>,
    },
    AcknowledgeWatch {
        id: WatchRegistrationId,
        revision: ZoneRevision,
        response: oneshot::Sender<Result<(), StoreError>>,
    },
    UnregisterWatch {
        id: WatchRegistrationId,
        response: oneshot::Sender<Result<Option<ZoneRevision>, StoreError>>,
    },
    Backup {
        identity: crate::StoreIdentity,
        response: oneshot::Sender<Result<LogicalBackup, StoreError>>,
    },
    Shutdown {
        response: oneshot::Sender<Result<(), StoreError>>,
    },
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

    fn pop_group(&mut self) -> Vec<WriteRequest> {
        let mut group = Vec::with_capacity(GROUP_COMMIT_MAX);
        let mut resources = BTreeSet::new();
        let mut stalled = 0;
        while group.len() < GROUP_COMMIT_MAX && !self.ring.is_empty() {
            if stalled >= self.ring.len() {
                break;
            }
            let principal = self.ring.pop_front().expect("ring is nonempty");
            let request = self
                .queues
                .get_mut(&principal)
                .and_then(VecDeque::pop_front)
                .expect("active principal has a request");
            if request
                .resources
                .iter()
                .any(|resource| resources.contains(resource))
                || self.has_earlier_resource(request.sequence, &request.resources)
            {
                self.queues
                    .get_mut(&principal)
                    .expect("principal queue exists")
                    .push_front(request);
                self.ring.push_back(principal);
                stalled += 1;
                continue;
            }
            resources.extend(request.resources.iter().cloned());
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
            group.push(request);
        }
        group
    }

    fn has_earlier_resource(&self, sequence: u64, resources: &[ResourceRef]) -> bool {
        self.queues.values().any(|queue| {
            queue.iter().any(|request| {
                request.sequence < sequence
                    && request
                        .resources
                        .iter()
                        .any(|candidate| resources.contains(candidate))
            })
        })
    }
}

struct WriterActor {
    database: Arc<Database>,
    receiver: mpsc::Receiver<WriterCommand>,
    scheduler: FairScheduler,
    signals: Arc<SignalCounters>,
    sequence: u64,
    quarantined: Arc<AtomicBool>,
    watch_coordinator: Arc<std::sync::Mutex<WatchCoordinator>>,
}

impl WriterActor {
    fn new(
        database: Arc<Database>,
        receiver: mpsc::Receiver<WriterCommand>,
        signals: Arc<SignalCounters>,
        quarantined: Arc<AtomicBool>,
        watch_coordinator: Arc<std::sync::Mutex<WatchCoordinator>>,
    ) -> Self {
        Self {
            database,
            receiver,
            scheduler: FairScheduler::default(),
            signals,
            sequence: 0,
            quarantined,
            watch_coordinator,
        }
    }

    fn run(mut self) {
        let mut deferred = None;
        loop {
            let command = deferred.take().or_else(|| self.receiver.blocking_recv());
            let Some(command) = command else {
                break;
            };
            match command {
                WriterCommand::Commit(request) => {
                    self.enqueue(*request);
                    while self.scheduler.len < WRITE_QUEUE_CAPACITY {
                        match self.receiver.try_recv() {
                            Ok(WriterCommand::Commit(request)) => self.enqueue(*request),
                            Ok(control) => {
                                deferred = Some(control);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    self.flush();
                }
                WriterCommand::Replay {
                    after_revision,
                    resource_types,
                    mut visit,
                    response,
                } => {
                    let high_water = current_meta(&self.database).map(|meta| meta.current_revision);
                    let replayed = match high_water {
                        Ok(high_water) => {
                            replay_after(&self.database, after_revision, &self.signals, |batch| {
                                let batch = Arc::new(batch);
                                let Some(filtered) = filter_batch(batch, &resource_types) else {
                                    return Ok(());
                                };
                                self.signals.record_shared_batch();
                                self.signals.record_fanout_reference();
                                visit(filtered)
                            })
                            .map(|()| high_water)
                        }
                        Err(error) => Err(error),
                    };
                    if let Err(error) = &replayed
                        && error.kind() == d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
                    {
                        self.quarantine(error.clone());
                    }
                    let _ = response.send(replayed);
                }
                WriterCommand::Watch {
                    after_revision,
                    selector,
                    initial_credits,
                    response,
                } => {
                    let result = self
                        .watch_coordinator
                        .lock()
                        .map_err(|_| crate::transaction::integrity("watch-coordinator-poisoned"))
                        .and_then(|mut coordinator| {
                            coordinator.register_and_replay(
                                &self.database,
                                after_revision,
                                selector,
                                initial_credits,
                            )
                        });
                    if let Err(error) = &result
                        && error.kind() == d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
                    {
                        self.quarantine(error.clone());
                    }
                    let _ = response.send(result);
                }
                WriterCommand::AcknowledgeWatch {
                    id,
                    revision,
                    response,
                } => {
                    let result = self
                        .watch_coordinator
                        .lock()
                        .map_err(|_| crate::transaction::integrity("watch-coordinator-poisoned"))
                        .and_then(|mut coordinator| coordinator.acknowledge(id, revision));
                    let _ = response.send(result);
                }
                WriterCommand::UnregisterWatch { id, response } => {
                    let result = self
                        .watch_coordinator
                        .lock()
                        .map_err(|_| crate::transaction::integrity("watch-coordinator-poisoned"))
                        .map(|mut coordinator| coordinator.unregister(id));
                    let _ = response.send(result);
                }
                WriterCommand::Backup { identity, response } => {
                    let backup = LogicalBackup::from_database(&self.database, &identity);
                    if let Err(error) = &backup
                        && error.kind() == d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
                    {
                        self.quarantine(error.clone());
                    }
                    let _ = response.send(backup);
                }
                WriterCommand::Shutdown { response } => {
                    let result = if self.quarantined.load(Ordering::Acquire) {
                        Err(crate::transaction::quarantined())
                    } else {
                        crate::transaction::set_clean_shutdown(&self.database, true)
                    };
                    let stop = result.is_ok();
                    let _ = response.send(result);
                    if stop {
                        break;
                    }
                }
            }
        }
    }

    fn enqueue(&mut self, mut request: WriteRequest) {
        request.sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        self.scheduler.push(request);
    }

    fn flush(&mut self) {
        while self.scheduler.len > 0 {
            let requests = self.scheduler.pop_group();
            if requests.is_empty() {
                return;
            }
            self.signals.writer_queue_depth.fetch_sub(
                u64::try_from(requests.len()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
            let owned = requests
                .into_iter()
                .map(|request| {
                    drop(request.queue_permit);
                    (request.mutation, request.response)
                })
                .collect::<Vec<_>>();
            let (mutations, responses): (Vec<_>, Vec<_>) = owned.into_iter().unzip();
            match apply_group(&self.database, mutations) {
                Ok(CommittedGroup { results, batch }) => {
                    let integrity_error = results.iter().find_map(|result| match result {
                        Err(error)
                            if error.kind()
                                == d2b_resource_store::StoreErrorKind::StoreIntegrityFailure =>
                        {
                            Some(error.clone())
                        }
                        _ => None,
                    });
                    if let Some(batch) = batch
                        && let Err(error) = self.dispatch_live(batch)
                    {
                        for response in responses {
                            let _ = response.send(Err(error.clone()));
                        }
                        self.quarantine(error);
                        return;
                    }
                    for (response, result) in responses.into_iter().zip(results) {
                        let _ = response.send(result);
                    }
                    if let Some(error) = integrity_error {
                        self.quarantine(error);
                        return;
                    }
                }
                Err(error) => {
                    for response in responses {
                        let _ = response.send(Err(error.clone()));
                    }
                    if error.kind() == d2b_resource_store::StoreErrorKind::StoreIntegrityFailure {
                        self.quarantine(error);
                        return;
                    }
                }
            }
        }
    }

    fn dispatch_live(&self, batch: ChangeBatch) -> Result<(), StoreError> {
        let Some(shared) = shared_batch(batch) else {
            return Ok(());
        };
        let fanout = self
            .watch_coordinator
            .lock()
            .map_err(|_| crate::transaction::integrity("watch-coordinator-poisoned"))?
            .dispatch(shared);
        if fanout != 0 {
            self.signals.record_shared_batch();
            self.signals
                .fanout_references
                .fetch_add(fanout, Ordering::Relaxed);
        }
        Ok(())
    }

    fn quarantine(&mut self, error: StoreError) {
        self.quarantined.store(true, Ordering::Release);
        self.receiver.close();
        while self.scheduler.len > 0 {
            let requests = self.scheduler.pop_group();
            self.signals.writer_queue_depth.fetch_sub(
                u64::try_from(requests.len()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
            for request in requests {
                let _ = request
                    .response
                    .send(Err(crate::transaction::quarantined()));
            }
        }
        while let Ok(command) = self.receiver.try_recv() {
            match command {
                WriterCommand::Commit(request) => {
                    self.signals
                        .writer_queue_depth
                        .fetch_sub(1, Ordering::Relaxed);
                    let _ = request
                        .response
                        .send(Err(crate::transaction::quarantined()));
                }
                WriterCommand::Replay { response, .. } => {
                    let _ = response.send(Err(crate::transaction::quarantined()));
                }
                WriterCommand::Watch { response, .. } => {
                    let _ = response.send(Err(crate::transaction::quarantined()));
                }
                WriterCommand::AcknowledgeWatch { response, .. } => {
                    let _ = response.send(Err(crate::transaction::quarantined()));
                }
                WriterCommand::UnregisterWatch { response, .. } => {
                    let _ = response.send(Err(crate::transaction::quarantined()));
                }
                WriterCommand::Backup { response, .. } => {
                    let _ = response.send(Err(crate::transaction::quarantined()));
                }
                WriterCommand::Shutdown { response } => {
                    let _ = response.send(Err(error.clone()));
                }
            }
        }
    }
}

pub(crate) fn filter_batch(
    batch: Arc<ChangeBatch>,
    resource_types: &BTreeSet<ResourceTypeName>,
) -> Option<SharedChangeBatch> {
    filter_batch_with(batch, |entry| {
        resource_types.contains(entry.resource_type())
    })
}

pub(crate) fn filter_batch_with(
    batch: Arc<ChangeBatch>,
    mut matches: impl FnMut(&crate::transaction::ChangeEntry) -> bool,
) -> Option<SharedChangeBatch> {
    let indices = batch
        .entries()
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| matches(entry).then_some(index))
        .collect::<Vec<_>>();
    (!indices.is_empty()).then(|| SharedChangeBatch {
        batch,
        indices: Arc::from(indices),
    })
}

pub(crate) fn shared_batch(batch: ChangeBatch) -> Option<SharedChangeBatch> {
    filter_batch_with(Arc::new(batch), |_| true)
}

pub(crate) fn replay_after<F>(
    database: &Database,
    after_revision: u64,
    signals: &SignalCounters,
    visit: F,
) -> Result<(), StoreError>
where
    F: FnMut(ChangeBatch) -> Result<(), StoreError>,
{
    let mut replay = crate::revision_log::ReplaySignals::default();
    let result = crate::revision_log::stream_after(database, after_revision, &mut replay, visit);
    signals
        .revision_range_seeks
        .fetch_add(replay.range_seeks(), Ordering::Relaxed);
    signals
        .replay_rows_scanned
        .fetch_add(replay.rows_scanned(), Ordering::Relaxed);
    signals
        .replay_rows_decoded
        .fetch_add(replay.rows_decoded(), Ordering::Relaxed);
    result
}

pub(crate) struct ReadPool {
    senders: Vec<std::sync::mpsc::SyncSender<ReadWork>>,
    next_worker: AtomicU64,
    zone: ZoneId,
    permits: Arc<tokio::sync::Semaphore>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl ReadPool {
    pub(crate) fn start(database: Arc<Database>, zone: ZoneId) -> Result<Self, StoreError> {
        let per_worker_capacity = MAX_CONCURRENT_READS / READ_POOL_THREADS;
        debug_assert_eq!(
            per_worker_capacity * READ_POOL_THREADS,
            MAX_CONCURRENT_READS
        );
        let mut senders = Vec::with_capacity(READ_POOL_THREADS);
        let mut threads: Vec<std::thread::JoinHandle<()>> = Vec::with_capacity(READ_POOL_THREADS);
        for index in 0..READ_POOL_THREADS {
            let database = Arc::clone(&database);
            let (sender, receiver) = std::sync::mpsc::sync_channel(per_worker_capacity);
            let thread = match std::thread::Builder::new()
                .name(format!("d2b-redb-read-{index}"))
                .spawn(move || read_worker(database, receiver))
            {
                Ok(thread) => thread,
                Err(_) => {
                    senders.clear();
                    for thread in threads {
                        let _ = thread.join();
                    }
                    return Err(crate::transaction::integrity("read-pool-start-failed"));
                }
            };
            senders.push(sender);
            threads.push(thread);
        }
        Ok(Self {
            senders,
            next_worker: AtomicU64::new(0),
            zone,
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_READS)),
            threads,
        })
    }

    async fn submit<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, StoreError>>) -> ReadCommand,
    ) -> Result<T, StoreError> {
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| backpressure())?;
        let (response, receiver) = oneshot::channel();
        let deadline = Instant::now() + READ_LIFETIME;
        let worker = usize::try_from(
            self.next_worker.fetch_add(1, Ordering::Relaxed) % READ_POOL_THREADS as u64,
        )
        .expect("read-worker index fits usize");
        self.senders[worker]
            .try_send(ReadWork {
                command: make(response),
                deadline,
                permit,
            })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => backpressure(),
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    crate::transaction::integrity("read-pool-closed")
                }
            })?;
        tokio::time::timeout(READ_LIFETIME + Duration::from_millis(25), receiver)
            .await
            .map_err(|_| timeout())?
            .map_err(|_| crate::transaction::integrity("read-response-closed"))?
    }

    pub(crate) async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        self.validate_zone(&request.zone)?;
        self.submit(|response| ReadCommand::Get { request, response })
            .await
    }

    pub(crate) async fn list(
        &self,
        request: StoreListRequest,
    ) -> Result<StoreListResult, StoreError> {
        self.validate_zone(&request.zone)?;
        self.submit(|response| ReadCommand::List { request, response })
            .await
    }

    pub(crate) async fn resolve(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        self.validate_zone(&request.zone)?;
        self.submit(|response| ReadCommand::Resolve { request, response })
            .await
    }

    pub(crate) async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        self.validate_zone(&request.zone)?;
        self.submit(|response| ReadCommand::InspectSchema { request, response })
            .await
    }

    pub(crate) async fn meta(&self) -> Result<StoreMeta, StoreError> {
        self.submit(|response| ReadCommand::Meta { response }).await
    }

    fn validate_zone(&self, zone: &ZoneId) -> Result<(), StoreError> {
        if zone != &self.zone {
            return Err(crate::transaction::integrity("request-zone-mismatch"));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn expiry_probe(&self) -> Result<(), StoreError> {
        self.submit(|response| ReadCommand::NeverRespond { response })
            .await
    }

    #[cfg(test)]
    pub(crate) fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }
}

impl Drop for ReadPool {
    fn drop(&mut self) {
        self.senders.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl ReadPool {
    pub(crate) fn shutdown(&mut self) -> Result<(), StoreError> {
        self.senders.clear();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                return Err(crate::transaction::integrity("read-worker-failed"));
            }
        }
        Ok(())
    }
}

struct ReadWork {
    command: ReadCommand,
    deadline: Instant,
    permit: OwnedSemaphorePermit,
}

enum ReadCommand {
    Get {
        request: StoreGetRequest,
        response: oneshot::Sender<Result<StoredResource, StoreError>>,
    },
    List {
        request: StoreListRequest,
        response: oneshot::Sender<Result<StoreListResult, StoreError>>,
    },
    Resolve {
        request: StoreResolveRequest,
        response: oneshot::Sender<Result<StoreResolvedIdentity, StoreError>>,
    },
    InspectSchema {
        request: StoreInspectSchemaRequest,
        response: oneshot::Sender<Result<StoredSchema, StoreError>>,
    },
    Meta {
        response: oneshot::Sender<Result<StoreMeta, StoreError>>,
    },
    #[cfg(test)]
    NeverRespond {
        response: oneshot::Sender<Result<(), StoreError>>,
    },
}

fn read_worker(database: Arc<Database>, receiver: std::sync::mpsc::Receiver<ReadWork>) {
    loop {
        let command = receiver.recv();
        let Ok(ReadWork {
            command,
            deadline,
            permit,
        }) = command
        else {
            return;
        };
        if Instant::now() >= deadline {
            send_read_result(command, Err(timeout()));
            drop(permit);
            continue;
        }
        match command {
            ReadCommand::Get { request, response } => {
                let _ = response.send(read_get(&database, request, deadline));
            }
            ReadCommand::List { request, response } => {
                let _ = response.send(read_list(&database, request, deadline));
            }
            ReadCommand::Resolve { request, response } => {
                let result = read_get(
                    &database,
                    StoreGetRequest {
                        operation: request.operation,
                        zone: request.zone,
                        target: request.target,
                        expected_uid: request.expected_uid,
                        projection: StoreProjection::MetadataOnly,
                    },
                    deadline,
                )
                .map(|resource| StoreResolvedIdentity {
                    zone: resource.zone,
                    resource_ref: resource.resource_ref,
                    uid: resource.uid,
                    generation: resource.generation,
                    revision: resource.revision,
                });
                let _ = response.send(result);
            }
            ReadCommand::InspectSchema { request, response } => {
                let _ = response.send(read_schema(&database, request, deadline));
            }
            ReadCommand::Meta { response } => {
                let result = if Instant::now() >= deadline {
                    Err(timeout())
                } else {
                    current_meta(&database)
                };
                let _ = response.send(result);
            }
            #[cfg(test)]
            ReadCommand::NeverRespond { response } => {
                std::thread::sleep(READ_LIFETIME + Duration::from_millis(10));
                let _ = response.send(Err(timeout()));
            }
        }
        drop(permit);
    }
}

fn send_read_result(command: ReadCommand, result: Result<(), StoreError>) {
    match command {
        ReadCommand::Get { response, .. } => {
            let _ = response.send(Err(result.unwrap_err()));
        }
        ReadCommand::List { response, .. } => {
            let _ = response.send(Err(result.unwrap_err()));
        }
        ReadCommand::Resolve { response, .. } => {
            let _ = response.send(Err(result.unwrap_err()));
        }
        ReadCommand::InspectSchema { response, .. } => {
            let _ = response.send(Err(result.unwrap_err()));
        }
        ReadCommand::Meta { response } => {
            let _ = response.send(Err(result.unwrap_err()));
        }
        #[cfg(test)]
        ReadCommand::NeverRespond { response } => {
            let _ = response.send(result);
        }
    }
}

fn read_get(
    database: &Database,
    request: StoreGetRequest,
    deadline: Instant,
) -> Result<StoredResource, StoreError> {
    check_deadline(deadline)?;
    let read = database
        .begin_read()
        .map_err(crate::transaction::integrity)?;
    let table = read
        .open_table(RESOURCES)
        .map_err(crate::transaction::integrity)?;
    let key = resource_key(&request.target)?;
    let bytes = table
        .get(key.as_slice())
        .map_err(crate::transaction::integrity)?
        .ok_or_else(not_found)?;
    check_deadline(deadline)?;
    let record: ResourceRecord = decode(ValueKind::ResourceRecord, bytes.value())?;
    let mut resource = stored_resource(&request.zone, &request.target, &record)?;
    if request
        .expected_uid
        .as_ref()
        .is_some_and(|uid| uid != &resource.uid)
    {
        return Err(not_found());
    }
    project_resource(&mut resource, request.projection)?;
    Ok(resource)
}

fn read_list(
    database: &Database,
    request: StoreListRequest,
    deadline: Instant,
) -> Result<StoreListResult, StoreError> {
    check_deadline(deadline)?;
    let read = database
        .begin_read()
        .map_err(crate::transaction::integrity)?;
    let table = read
        .open_table(RESOURCES)
        .map_err(crate::transaction::integrity)?;
    let meta = crate::transaction::read_meta(&read)?;
    let snapshot_revision = meta.current_revision;
    let selector_digest = list_selector_digest(&request);
    let mut resources = Vec::new();
    let after_key = match request.cursor.as_deref() {
        Some(cursor) => {
            let cursor = decode_list_cursor(cursor)?;
            if cursor.selector_digest != selector_digest {
                return Err(crate::transaction::integrity(
                    "list-cursor-selector-mismatch",
                ));
            }
            if cursor.snapshot_revision != snapshot_revision {
                return Err(crate::transaction::revision_expired(snapshot_revision));
            }
            Some(cursor.after_key)
        }
        None => None,
    };
    let page_size = usize::try_from(request.page_size)
        .map_err(crate::transaction::integrity)?
        .max(1);
    for row in table.iter().map_err(crate::transaction::integrity)? {
        check_deadline(deadline)?;
        let (key, value) = row.map_err(crate::transaction::integrity)?;
        if after_key
            .as_ref()
            .is_some_and(|after_key| key.value() <= after_key.as_slice())
        {
            continue;
        }
        let resource_ref = crate::transaction::resource_ref_from_key(key.value())?;
        let resource_type = resource_ref.resource_type().as_str();
        let name = resource_ref.name().as_str();
        if !request.resource_types.is_empty()
            && !request
                .resource_types
                .iter()
                .any(|candidate| candidate.as_str() == resource_type)
        {
            continue;
        }
        if !request.resource_names.is_empty()
            && !request
                .resource_names
                .iter()
                .any(|candidate| candidate.as_str() == name)
        {
            continue;
        }
        if !filters_match(&request.filters, resource_type, name) {
            continue;
        }
        let record: ResourceRecord = decode(ValueKind::ResourceRecord, value.value())?;
        let mut resource = stored_resource(&request.zone, &resource_ref, &record)?;
        project_resource(&mut resource, request.projection)?;
        resources.push((key.value().to_vec(), resource));
        if resources.len() > page_size {
            break;
        }
    }
    let truncated = resources.len() > page_size;
    resources.truncate(page_size);
    let next_cursor = if truncated {
        let after_key = resources
            .last()
            .ok_or_else(|| crate::transaction::integrity("list-page-state-invalid"))?
            .0
            .clone();
        Some(encode_list_cursor(
            snapshot_revision,
            &selector_digest,
            &after_key,
        ))
    } else {
        None
    };
    let resources = resources
        .into_iter()
        .map(|(_, resource)| resource)
        .collect();
    Ok(StoreListResult {
        resources,
        snapshot_revision: ZoneRevision::new(snapshot_revision),
        next_cursor,
        truncated,
    })
}

struct ListCursor {
    snapshot_revision: u64,
    selector_digest: String,
    after_key: Vec<u8>,
}

fn list_selector_digest(request: &StoreListRequest) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(request.zone.as_str().as_bytes());
    digest.update([request.projection as u8]);
    for resource_type in &request.resource_types {
        digest.update(resource_type.as_str().as_bytes());
        digest.update([0]);
    }
    for name in &request.resource_names {
        digest.update(name.as_str().as_bytes());
        digest.update([0]);
    }
    for filter in &request.filters {
        digest.update(filter.field.as_bytes());
        digest.update([0]);
        for value in &filter.values {
            digest.update(value.as_bytes());
            digest.update([0]);
        }
    }
    format!("{:x}", digest.finalize())
}

fn encode_list_cursor(revision: u64, selector_digest: &str, after_key: &[u8]) -> String {
    format!("v1.{revision}.{selector_digest}.{}", hex_encode(after_key))
}

fn decode_list_cursor(value: &str) -> Result<ListCursor, StoreError> {
    let mut parts = value.split('.');
    if parts.next() != Some("v1") {
        return Err(crate::transaction::integrity("list-cursor-invalid"));
    }
    let snapshot_revision = parts
        .next()
        .ok_or_else(|| crate::transaction::integrity("list-cursor-invalid"))?
        .parse()
        .map_err(|_| crate::transaction::integrity("list-cursor-invalid"))?;
    let selector_digest = parts
        .next()
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| crate::transaction::integrity("list-cursor-invalid"))?
        .to_owned();
    let after_key = hex_decode(
        parts
            .next()
            .ok_or_else(|| crate::transaction::integrity("list-cursor-invalid"))?,
    )?;
    if parts.next().is_some() {
        return Err(crate::transaction::integrity("list-cursor-invalid"));
    }
    crate::transaction::resource_ref_from_key(&after_key)?;
    Ok(ListCursor {
        snapshot_revision,
        selector_digest,
        after_key,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn hex_decode(value: &str) -> Result<Vec<u8>, StoreError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(crate::transaction::integrity("list-cursor-invalid"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair)
                .map_err(|_| crate::transaction::integrity("list-cursor-invalid"))?;
            u8::from_str_radix(text, 16)
                .map_err(|_| crate::transaction::integrity("list-cursor-invalid"))
        })
        .collect()
}

fn filters_match(filters: &[StoreFilter], resource_type: &str, name: &str) -> bool {
    filters.iter().all(|filter| match filter.field.as_str() {
        "metadata.name" => filter.values.iter().any(|value| value == name),
        "type" => filter.values.iter().any(|value| value == resource_type),
        _ => false,
    })
}

fn project_resource(
    resource: &mut StoredResource,
    projection: StoreProjection,
) -> Result<(), StoreError> {
    if projection == StoreProjection::Full {
        return Ok(());
    }
    let mut value = d2b_contracts::v3::CanonicalJsonValue::parse(&resource.canonical_json)
        .map_err(crate::transaction::integrity)?;
    let d2b_contracts::v3::CanonicalJsonValue::Object(root) = &mut value else {
        return Err(crate::transaction::integrity(
            "stored-resource-envelope-invalid",
        ));
    };
    match projection {
        StoreProjection::Full => unreachable!("full projection returned above"),
        StoreProjection::BaseOnly => {
            if let Some(d2b_contracts::v3::CanonicalJsonValue::Object(spec)) = root.get_mut("spec")
            {
                spec.remove("provider");
            }
            if let Some(d2b_contracts::v3::CanonicalJsonValue::Object(status)) =
                root.get_mut("status")
            {
                status.remove("provider");
            }
        }
        StoreProjection::MetadataOnly => {
            root.retain(|key, _| matches!(key.as_str(), "apiVersion" | "metadata" | "type"));
        }
    }
    resource.canonical_json = value.to_canonical_bytes();
    Ok(())
}

fn read_schema(
    database: &Database,
    request: StoreInspectSchemaRequest,
    deadline: Instant,
) -> Result<StoredSchema, StoreError> {
    check_deadline(deadline)?;
    let read = database
        .begin_read()
        .map_err(crate::transaction::integrity)?;
    let table = read
        .open_table(API_SCHEMAS)
        .map_err(crate::transaction::integrity)?;
    let key = crate::encode_key(
        KeySpace::ApiSchemas,
        &[crate::KeyComponent::Text(request.resource_type.as_str())],
    )
    .map_err(crate::transaction::integrity)?;
    let bytes = table
        .get(key.as_bytes())
        .map_err(crate::transaction::integrity)?
        .ok_or_else(not_found)?;
    let decoded =
        crate::DecodedValue::decode(bytes.value()).map_err(crate::transaction::integrity)?;
    if decoded.kind() != ValueKind::ApiSchemaRecord {
        return Err(crate::transaction::integrity("table-value-kind-mismatch"));
    }
    let canonical_json = decoded.canonical_json().to_vec();
    let payload_digest =
        d2b_contracts::v3::canonical_digest("d2b:v3:resource-schema", &canonical_json);
    Ok(StoredSchema {
        resource_type: request.resource_type,
        canonical_json,
        payload_digest,
    })
}

fn check_deadline(deadline: Instant) -> Result<(), StoreError> {
    if Instant::now() >= deadline {
        return Err(timeout());
    }
    Ok(())
}

fn not_found() -> StoreError {
    d2b_resource_store::StoreError::new(
        d2b_resource_store::StoreErrorKind::ResourceNotFound,
        None,
        None,
        d2b_contracts::v3::RetryClass::Never,
        "resource-not-found",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::{ChangeEntry, ChangeEvent, REVISION_LOG, encode, revision_key};
    use d2b_contracts::v3::{ResourceGeneration, ResourceName, ResourceUid};
    use std::fs::OpenOptions;

    fn database(label: &str) -> (tempfile::TempDir, Arc<Database>) {
        let directory = tempfile::tempdir().unwrap();
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join(format!("{label}.redb")))
            .unwrap();
        let backend = redb::backends::FileBackend::new(file).unwrap();
        let database = Database::builder().create_with_backend(backend).unwrap();
        (directory, Arc::new(database))
    }

    fn batch(revision: u64) -> ChangeBatch {
        ChangeBatch::new(
            ZoneRevision::new(revision),
            vec![
                ChangeEntry::new(
                    0,
                    ResourceTypeName::parse("Process").unwrap(),
                    ResourceName::parse("worker").unwrap(),
                    ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                    ChangeEvent::Created,
                    None,
                    Some(ResourceGeneration::new(1).unwrap()),
                    None,
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                    None,
                    "op".to_owned(),
                    "corr".to_owned(),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn range_seek_never_scans_or_decodes_a_corrupt_older_envelope() {
        let (_directory, database) = database("range-seek-corrupt-old");
        let write = database.begin_write().unwrap();
        {
            let mut table = write.open_table(REVISION_LOG).unwrap();
            table
                .insert(
                    revision_key(1).unwrap().as_slice(),
                    b"not-a-value".as_slice(),
                )
                .unwrap();
            let current = encode(ValueKind::ChangeBatch, &batch(2)).unwrap();
            table
                .insert(revision_key(2).unwrap().as_slice(), current.as_slice())
                .unwrap();
        }
        write.commit().unwrap();

        let signals = SignalCounters::default();
        let mut revisions = Vec::new();
        replay_after(&database, 1, &signals, |batch| {
            revisions.push(batch.revision().get());
            Ok(())
        })
        .unwrap();
        assert_eq!(revisions, [2]);
        let signals = signals.snapshot();
        assert_eq!(signals.revision_range_seeks, 1);
        assert_eq!(signals.replay_rows_scanned, 1);
        assert_eq!(signals.replay_rows_decoded, 1);
    }

    #[test]
    fn filtered_views_share_one_batch_and_nonmatches_are_absent() {
        let process = batch(1).entries()[0].clone();
        let device = ChangeEntry::new(
            1,
            ResourceTypeName::parse("Device").unwrap(),
            ResourceName::parse("gpu").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
            ChangeEvent::Created,
            None,
            Some(ResourceGeneration::new(1).unwrap()),
            None,
            "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
            None,
            "op".to_owned(),
            "corr".to_owned(),
        )
        .unwrap();
        let mixed =
            Arc::new(ChangeBatch::new(ZoneRevision::new(1), vec![process, device]).unwrap());
        let all = filter_batch(
            Arc::clone(&mixed),
            &BTreeSet::from([
                ResourceTypeName::parse("Process").unwrap(),
                ResourceTypeName::parse("Device").unwrap(),
            ]),
        )
        .unwrap();
        let process = filter_batch(
            Arc::clone(&mixed),
            &BTreeSet::from([ResourceTypeName::parse("Process").unwrap()]),
        )
        .unwrap();
        let absent = filter_batch(
            mixed,
            &BTreeSet::from([ResourceTypeName::parse("Volume").unwrap()]),
        );

        assert!(all.shares_batch_with(&process));
        assert_eq!(all.entries().len(), 2);
        assert_eq!(process.entries().len(), 1);
        assert!(absent.is_none());
    }

    #[test]
    fn fair_scheduler_round_robins_principals_and_preserves_resource_order() {
        let permits = Arc::new(tokio::sync::Semaphore::new(3));
        let request = |sequence, principal: &str, resource: &str| {
            crate::transaction::empty_write_request_for_test(
                sequence,
                principal,
                ResourceRef::parse(resource).unwrap(),
                Arc::clone(&permits).try_acquire_owned().unwrap(),
            )
        };
        let mut scheduler = FairScheduler::default();
        scheduler.push(request(0, "alice", "Process/shared"));
        scheduler.push(request(1, "alice", "Process/shared"));
        scheduler.push(request(2, "bob", "Process/other"));

        let first = scheduler.pop_group();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].sequence, 0);
        assert_eq!(first[1].principal, "bob");
        let second = scheduler.pop_group();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].sequence, 1);
    }

    #[test]
    fn engine_failure_quarantines_actor_and_rejects_queued_writes() {
        let (_directory, database) = database("quarantine-on-engine-failure");
        let (_command_sender, command_receiver) = mpsc::channel(1);
        let signals = Arc::new(SignalCounters::default());
        let quarantined = Arc::new(AtomicBool::new(false));
        let permits = Arc::new(tokio::sync::Semaphore::new(2));
        let first = crate::transaction::empty_write_request_for_test(
            0,
            "alice",
            ResourceRef::parse("Process/first").unwrap(),
            Arc::clone(&permits).try_acquire_owned().unwrap(),
        );
        let second = crate::transaction::empty_write_request_for_test(
            1,
            "bob",
            ResourceRef::parse("Process/first").unwrap(),
            Arc::clone(&permits).try_acquire_owned().unwrap(),
        );
        let (second_response, second_result) = oneshot::channel();
        let mut second = second;
        second.response = second_response;
        let watch_coordinator = Arc::new(std::sync::Mutex::new(WatchCoordinator::default()));
        let mut actor = WriterActor::new(
            database,
            command_receiver,
            Arc::clone(&signals),
            Arc::clone(&quarantined),
            watch_coordinator,
        );
        actor.scheduler.push(first);
        actor.scheduler.push(second);
        signals.writer_queue_depth.store(2, Ordering::Relaxed);
        crate::transaction::fail_next_apply_group_for_test();
        actor.flush();

        assert!(quarantined.load(Ordering::Acquire));
        assert_eq!(actor.scheduler.len, 0);
        assert_eq!(signals.writer_queue_depth.load(Ordering::Relaxed), 0);
        assert_eq!(
            second_result.blocking_recv().unwrap().unwrap_err().kind(),
            d2b_resource_store::StoreErrorKind::StoreQuarantined
        );
    }
}
