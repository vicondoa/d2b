//! Fair single-writer actor, bounded reads, replay, and shared live delivery.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use d2b_contracts::v3::{ResourceRef, ResourceTypeName, ZoneId, ZoneRevision};
use d2b_resource_store::{
    StoreError, StoreFilter, StoreGetRequest, StoreInspectSchemaRequest, StoreListRequest,
    StoreListResult, StoreProjection, StoreResolveRequest, StoreResolvedIdentity, StoredResource,
    StoredSchema,
};
use redb::{Database, ReadableDatabase, ReadableTable};
use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot};

use crate::CheckedMutation;
use crate::transaction::{
    API_SCHEMAS, ChangeBatch, CommittedGroup, RESOURCES, ResourceRecord, StoreMeta, VerifiedWrite,
    apply_group, backpressure, current_meta, decode, resource_key, revision_key, stored_resource,
    timeout,
};
use crate::{DecodedKey, DecodedKeyComponent, KeySpace, ValueKind};

/// Bounded public writer admission queue.
pub const WRITE_QUEUE_CAPACITY: usize = 256;
/// Maximum independent mutation requests in one crash-safe commit.
pub const GROUP_COMMIT_MAX: usize = 16;
/// Dedicated blocking MVCC read workers.
pub const READ_POOL_THREADS: usize = 4;
/// Maximum read transactions admitted at once.
pub const MAX_CONCURRENT_READS: usize = 16;
/// Hard lifetime ceiling for an admitted read transaction.
pub const READ_LIFETIME: Duration = Duration::from_millis(250);

/// One immutable decoded batch shared by matching live consumers.
pub type SharedChangeBatch = Arc<ChangeBatch>;

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
}

pub(crate) struct WriterHandle {
    sender: Option<mpsc::Sender<WriterCommand>>,
    signals: Arc<SignalCounters>,
    write_permits: Arc<tokio::sync::Semaphore>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WriterHandle {
    pub(crate) fn start(database: Arc<Database>, signals: Arc<SignalCounters>) -> Self {
        let (sender, receiver) = mpsc::channel(WRITE_QUEUE_CAPACITY);
        crate::transaction::set_clean_shutdown(&database, false)
            .expect("validated store must accept its startup marker");
        let actor_signals = Arc::clone(&signals);
        let thread = std::thread::Builder::new()
            .name("d2b-redb-writer".to_owned())
            .spawn(move || WriterActor::new(database, receiver, actor_signals).run())
            .expect("writer actor creation must succeed");
        Self {
            sender: Some(sender),
            signals,
            write_permits: Arc::new(tokio::sync::Semaphore::new(WRITE_QUEUE_CAPACITY)),
            thread: Some(thread),
        }
    }

    pub(crate) async fn commit(
        &self,
        mutation: CheckedMutation,
    ) -> Result<d2b_resource_store::StoreCommitResult, StoreError> {
        if mutation.mutations().is_empty() {
            return Err(crate::transaction::integrity("empty-verified-mutation"));
        }
        if mutation.mutations().len() > d2b_contracts::v3::MAX_BATCH_MUTATIONS {
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
        let principal = mutation.authorization().subject_uid.as_str().to_owned();
        let mut resources = mutation
            .mutations()
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
            mutation: mutation.into(),
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

    pub(crate) async fn register_watch(
        &self,
        after_revision: u64,
        resource_types: BTreeSet<ResourceTypeName>,
    ) -> Result<BackendWatch, StoreError> {
        let (delivery, receiver) = mpsc::channel(WRITE_QUEUE_CAPACITY);
        let (response, ready) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("writer-closed"))?
            .send(WriterCommand::RegisterWatch {
                after_revision,
                resource_types,
                delivery,
                response,
            })
            .await
            .map_err(|_| crate::transaction::integrity("writer-closed"))?;
        let (id, high_water) = ready
            .await
            .map_err(|_| crate::transaction::integrity("watch-registration-closed"))??;
        Ok(BackendWatch {
            id,
            high_water: ZoneRevision::new(high_water),
            receiver,
        })
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
    RegisterWatch {
        after_revision: u64,
        resource_types: BTreeSet<ResourceTypeName>,
        delivery: mpsc::Sender<SharedChangeBatch>,
        response: oneshot::Sender<Result<(u64, u64), StoreError>>,
    },
}

struct WatchRegistration {
    resource_types: BTreeSet<ResourceTypeName>,
    delivery: mpsc::Sender<SharedChangeBatch>,
}

pub struct BackendWatch {
    id: u64,
    high_water: ZoneRevision,
    receiver: mpsc::Receiver<SharedChangeBatch>,
}

impl BackendWatch {
    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn high_water(&self) -> ZoneRevision {
        self.high_water
    }

    pub async fn recv(&mut self) -> Option<SharedChangeBatch> {
        self.receiver.recv().await
    }
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
    watches: Vec<WatchRegistration>,
    signals: Arc<SignalCounters>,
    sequence: u64,
    next_watch_id: u64,
}

impl WriterActor {
    fn new(
        database: Arc<Database>,
        receiver: mpsc::Receiver<WriterCommand>,
        signals: Arc<SignalCounters>,
    ) -> Self {
        Self {
            database,
            receiver,
            scheduler: FairScheduler::default(),
            watches: Vec::new(),
            signals,
            sequence: 0,
            next_watch_id: 0,
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
                WriterCommand::RegisterWatch {
                    after_revision,
                    resource_types,
                    delivery,
                    response,
                } => {
                    let high_water = current_meta(&self.database).map(|meta| meta.current_revision);
                    let registered = match high_water {
                        Ok(high_water) => {
                            let id = self.next_watch_id;
                            self.next_watch_id = self.next_watch_id.wrapping_add(1);
                            let _ = response.send(Ok((id, high_water)));
                            replay_after(&self.database, after_revision, &self.signals, |batch| {
                                let filtered = filter_batch(batch, &resource_types);
                                delivery.blocking_send(Arc::new(filtered)).map_err(|_| {
                                    crate::transaction::integrity("watch-replay-closed")
                                })
                            })
                            .is_ok()
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                            false
                        }
                    };
                    if registered {
                        self.watches.push(WatchRegistration {
                            resource_types,
                            delivery,
                        });
                    }
                }
            }
        }
        crate::transaction::set_clean_shutdown(&self.database, true)
            .expect("writer actor must persist its clean-shutdown marker");
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
                    if let Some(batch) = batch {
                        self.dispatch(&batch);
                    }
                    for (response, result) in responses.into_iter().zip(results) {
                        let _ = response.send(result);
                    }
                }
                Err(error) => {
                    for response in responses {
                        let _ = response.send(Err(error.clone()));
                    }
                }
            }
        }
    }

    fn dispatch(&mut self, batch: &ChangeBatch) {
        if self.watches.is_empty() {
            return;
        }
        let all = Arc::new(batch.clone());
        let mut by_filter: BTreeMap<BTreeSet<ResourceTypeName>, SharedChangeBatch> =
            BTreeMap::new();
        let mut materialized = 1_u64;
        let mut fanout = 0_u64;
        self.watches.retain(|watch| {
            let shared = by_filter
                .entry(watch.resource_types.clone())
                .or_insert_with(|| {
                    if batch.entries.iter().all(|entry| {
                        watch
                            .resource_types
                            .iter()
                            .any(|resource_type| resource_type.as_str() == entry.resource_type)
                    }) {
                        Arc::clone(&all)
                    } else {
                        materialized += 1;
                        Arc::new(filter_batch(batch.clone(), &watch.resource_types))
                    }
                });
            fanout += 1;
            watch.delivery.try_send(Arc::clone(shared)).is_ok()
        });
        self.signals
            .shared_immutable_batches
            .fetch_add(materialized, Ordering::Relaxed);
        self.signals
            .fanout_references
            .fetch_add(fanout, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn dispatch_for_test(&mut self, batch: &ChangeBatch) {
        self.dispatch(batch);
    }
}

fn filter_batch(
    mut batch: ChangeBatch,
    resource_types: &BTreeSet<ResourceTypeName>,
) -> ChangeBatch {
    batch.entries.retain(|entry| {
        resource_types
            .iter()
            .any(|resource_type| resource_type.as_str() == entry.resource_type)
    });
    batch
}

pub(crate) fn replay_after<F>(
    database: &Database,
    after_revision: u64,
    signals: &SignalCounters,
    mut visit: F,
) -> Result<(), StoreError>
where
    F: FnMut(ChangeBatch) -> Result<(), StoreError>,
{
    let Some(first) = after_revision.checked_add(1) else {
        return Ok(());
    };
    let read = database
        .begin_read()
        .map_err(crate::transaction::integrity)?;
    let table = read
        .open_table(crate::transaction::REVISION_LOG)
        .map_err(crate::transaction::integrity)?;
    let lower = revision_key(first)?;
    signals.revision_range_seeks.fetch_add(1, Ordering::Relaxed);
    for row in table
        .range(lower.as_slice()..)
        .map_err(crate::transaction::integrity)?
    {
        let (_, value) = row.map_err(crate::transaction::integrity)?;
        signals.replay_rows_scanned.fetch_add(1, Ordering::Relaxed);
        let batch = decode(ValueKind::ChangeBatch, value.value())?;
        signals.replay_rows_decoded.fetch_add(1, Ordering::Relaxed);
        visit(batch)?;
    }
    Ok(())
}

pub(crate) struct ReadPool {
    senders: Vec<Option<std::sync::mpsc::SyncSender<ReadCommand>>>,
    next_worker: AtomicU64,
    zone: ZoneId,
    permits: Arc<tokio::sync::Semaphore>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl ReadPool {
    pub(crate) fn start(database: Arc<Database>, zone: ZoneId) -> Self {
        let per_worker_capacity = MAX_CONCURRENT_READS / READ_POOL_THREADS;
        debug_assert_eq!(
            per_worker_capacity * READ_POOL_THREADS,
            MAX_CONCURRENT_READS
        );
        let mut senders = Vec::with_capacity(READ_POOL_THREADS);
        let mut threads = Vec::with_capacity(READ_POOL_THREADS);
        for index in 0..READ_POOL_THREADS {
            let database = Arc::clone(&database);
            let (sender, receiver) = std::sync::mpsc::sync_channel(per_worker_capacity);
            senders.push(Some(sender));
            threads.push(
                std::thread::Builder::new()
                    .name(format!("d2b-redb-read-{index}"))
                    .spawn(move || read_worker(database, receiver))
                    .expect("read worker creation must succeed"),
            );
        }
        Self {
            senders,
            next_worker: AtomicU64::new(0),
            zone,
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_READS)),
            threads,
        }
    }

    async fn submit<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, StoreError>>) -> ReadCommand,
    ) -> Result<T, StoreError> {
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| backpressure())?;
        let (response, receiver) = oneshot::channel();
        let worker = usize::try_from(
            self.next_worker.fetch_add(1, Ordering::Relaxed) % READ_POOL_THREADS as u64,
        )
        .expect("read-worker index fits usize");
        self.senders[worker]
            .as_ref()
            .ok_or_else(|| crate::transaction::integrity("read-pool-closed"))?
            .try_send(make(response))
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => backpressure(),
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    crate::transaction::integrity("read-pool-closed")
                }
            })?;
        let result = tokio::time::timeout(READ_LIFETIME, receiver)
            .await
            .map_err(|_| timeout())?
            .map_err(|_| crate::transaction::integrity("read-response-closed"))?;
        drop(permit);
        result
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
}

impl Drop for ReadPool {
    fn drop(&mut self) {
        for sender in &mut self.senders {
            sender.take();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
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

fn read_worker(database: Arc<Database>, receiver: std::sync::mpsc::Receiver<ReadCommand>) {
    loop {
        let command = receiver.recv();
        let Ok(command) = command else {
            return;
        };
        match command {
            ReadCommand::Get { request, response } => {
                let _ = response.send(read_get(&database, request));
            }
            ReadCommand::List { request, response } => {
                let _ = response.send(read_list(&database, request));
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
                let _ = response.send(read_schema(&database, request));
            }
            ReadCommand::Meta { response } => {
                let _ = response.send(current_meta(&database));
            }
            #[cfg(test)]
            ReadCommand::NeverRespond { response } => {
                std::mem::forget(response);
            }
        }
    }
}

fn read_get(database: &Database, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
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
) -> Result<StoreListResult, StoreError> {
    let read = database
        .begin_read()
        .map_err(crate::transaction::integrity)?;
    let table = read
        .open_table(RESOURCES)
        .map_err(crate::transaction::integrity)?;
    let snapshot_revision = crate::transaction::read_meta(&read)?.current_revision;
    let mut resources = Vec::new();
    let offset = request
        .cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| crate::transaction::integrity("list-cursor-invalid"))?;
    let page_size = usize::try_from(request.page_size).map_err(crate::transaction::integrity)?;
    for row in table.iter().map_err(crate::transaction::integrity)? {
        let (key, value) = row.map_err(crate::transaction::integrity)?;
        let decoded = DecodedKey::decode(key.value()).map_err(crate::transaction::integrity)?;
        if decoded.key_space() != KeySpace::Resources {
            return Err(crate::transaction::integrity("resource-key-space-invalid"));
        }
        let [
            DecodedKeyComponent::Text(resource_type),
            DecodedKeyComponent::Text(name),
        ] = decoded.components()
        else {
            return Err(crate::transaction::integrity("resource-key-shape-invalid"));
        };
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
        let resource_ref = ResourceRef::parse(&format!("{resource_type}/{name}"))
            .map_err(crate::transaction::integrity)?;
        let record: ResourceRecord = decode(ValueKind::ResourceRecord, value.value())?;
        let mut resource = stored_resource(&request.zone, &resource_ref, &record)?;
        project_resource(&mut resource, request.projection)?;
        resources.push(resource);
    }
    let selected = resources
        .into_iter()
        .skip(offset)
        .take(page_size.saturating_add(1))
        .collect::<Vec<_>>();
    let truncated = selected.len() > page_size;
    let resources = selected.into_iter().take(page_size).collect::<Vec<_>>();
    let next_cursor = truncated.then(|| (offset + resources.len()).to_string());
    Ok(StoreListResult {
        resources,
        snapshot_revision: ZoneRevision::new(snapshot_revision),
        next_cursor,
        truncated,
    })
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
) -> Result<StoredSchema, StoreError> {
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
    let canonical_json: Vec<u8> = decode(ValueKind::ApiSchemaRecord, bytes.value())?;
    let payload_digest =
        d2b_contracts::v3::canonical_digest("d2b:v3:resource-schema", &canonical_json);
    Ok(StoredSchema {
        resource_type: request.resource_type,
        canonical_json,
        payload_digest,
    })
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
    use crate::transaction::{ChangeEntry, REVISION_LOG, encode};
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
        ChangeBatch {
            revision,
            entries: vec![ChangeEntry {
                ordinal: 0,
                resource_type: "Process".to_owned(),
                resource_name: "worker".to_owned(),
                resource_uid: "123e4567-e89b-42d3-a456-426614174000".to_owned(),
                event: "created".to_owned(),
                old_generation: None,
                new_generation: Some(1),
                owner_uid: None,
                payload_digest: "sha256:00".to_owned(),
                canonical_resource: None,
                operation_id: "op".to_owned(),
                correlation_id: "corr".to_owned(),
            }],
        }
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
            revisions.push(batch.revision);
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
    fn live_dispatch_shares_one_arc_across_identical_watch_filters() {
        let (_directory, database) = database("shared-live-dispatch");
        let (_command_sender, command_receiver) = mpsc::channel(1);
        let signals = Arc::new(SignalCounters::default());
        let (first_sender, mut first_receiver) = mpsc::channel(1);
        let (second_sender, mut second_receiver) = mpsc::channel(1);
        let filter = BTreeSet::from([ResourceTypeName::parse("Process").unwrap()]);
        let mut actor = WriterActor::new(database, command_receiver, Arc::clone(&signals));
        actor.watches = vec![
            WatchRegistration {
                resource_types: filter.clone(),
                delivery: first_sender,
            },
            WatchRegistration {
                resource_types: filter,
                delivery: second_sender,
            },
        ];
        actor.dispatch_for_test(&batch(1));

        let first = first_receiver.try_recv().unwrap();
        let second = second_receiver.try_recv().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.entries.len(), 1);
        let signals = signals.snapshot();
        assert_eq!(signals.shared_immutable_batches, 1);
        assert_eq!(signals.fanout_references, 2);
    }

    #[test]
    fn filtered_live_dispatch_shares_one_arc_and_skips_nonmatching_watches() {
        let (_directory, database) = database("shared-filtered-live-dispatch");
        let (_command_sender, command_receiver) = mpsc::channel(1);
        let signals = Arc::new(SignalCounters::default());
        let (first_sender, mut first_receiver) = mpsc::channel(1);
        let (second_sender, mut second_receiver) = mpsc::channel(1);
        let (other_sender, mut other_receiver) = mpsc::channel(1);
        let process = BTreeSet::from([ResourceTypeName::parse("Process").unwrap()]);
        let device = BTreeSet::from([ResourceTypeName::parse("Device").unwrap()]);
        let mut actor = WriterActor::new(database, command_receiver, Arc::clone(&signals));
        actor.watches = vec![
            WatchRegistration {
                resource_types: process.clone(),
                delivery: first_sender,
            },
            WatchRegistration {
                resource_types: process,
                delivery: second_sender,
            },
            WatchRegistration {
                resource_types: device,
                delivery: other_sender,
            },
        ];
        actor.dispatch_for_test(&batch(1));

        let first = first_receiver.try_recv().unwrap();
        let second = second_receiver.try_recv().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let other = other_receiver.try_recv().unwrap();
        assert!(other.entries.is_empty());
        let signals = signals.snapshot();
        assert_eq!(signals.shared_immutable_batches, 2);
        assert_eq!(signals.fanout_references, 3);
    }

    #[test]
    fn full_and_filtered_watchers_share_within_each_distinct_filter() {
        let (_directory, database) = database("shared-distinct-filter-dispatch");
        let (_command_sender, command_receiver) = mpsc::channel(1);
        let signals = Arc::new(SignalCounters::default());
        let mut receivers = Vec::new();
        let mut watches = Vec::new();
        for (id, filter) in [
            BTreeSet::from([
                ResourceTypeName::parse("Process").unwrap(),
                ResourceTypeName::parse("Device").unwrap(),
            ]),
            BTreeSet::from([
                ResourceTypeName::parse("Process").unwrap(),
                ResourceTypeName::parse("Device").unwrap(),
            ]),
            BTreeSet::from([ResourceTypeName::parse("Process").unwrap()]),
            BTreeSet::from([ResourceTypeName::parse("Process").unwrap()]),
        ]
        .into_iter()
        .enumerate()
        {
            let (delivery, receiver) = mpsc::channel(1);
            watches.push(WatchRegistration {
                resource_types: filter,
                delivery,
            });
            receivers.push((id, receiver));
        }
        let mut mixed = batch(1);
        let mut device = mixed.entries[0].clone();
        device.ordinal = 1;
        device.resource_type = "Device".to_owned();
        device.resource_name = "gpu".to_owned();
        mixed.entries.push(device);
        let mut actor = WriterActor::new(database, command_receiver, Arc::clone(&signals));
        actor.watches = watches;
        actor.dispatch_for_test(&mixed);

        let delivered = receivers
            .into_iter()
            .map(|(id, mut receiver)| (id, receiver.try_recv().unwrap()))
            .collect::<Vec<_>>();
        assert!(Arc::ptr_eq(&delivered[0].1, &delivered[1].1));
        assert!(Arc::ptr_eq(&delivered[2].1, &delivered[3].1));
        assert!(!Arc::ptr_eq(&delivered[0].1, &delivered[2].1));
        assert_eq!(delivered[0].1.entries.len(), 2);
        assert_eq!(delivered[2].1.entries.len(), 1);
        let signals = signals.snapshot();
        assert_eq!(signals.shared_immutable_batches, 2);
        assert_eq!(signals.fanout_references, 4);
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
}
