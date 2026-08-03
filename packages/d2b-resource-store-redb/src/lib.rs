//! Production redb backend for one Zone resource store.

pub mod actor;
pub mod audit;
pub mod backup;
pub mod keys;
pub mod metrics;
pub mod ownership;
pub mod revision_log;
pub mod schema;
pub mod tracing;
mod transaction;
pub mod values;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Mutex};

use actor::{ReadPool, SignalCounters, WriterHandle};
use d2b_contracts::v3::{ResourceUid, Timestamp, ZoneId};
use d2b_resource_store::mutation_seal::{MutationSealAcceptor, SealedMutation};
use d2b_resource_store::{
    PolicySnapshot, StoreCommitResult, StoreError, StoreGetRequest, StoreInspectSchemaRequest,
    StoreListRequest, StoreListResult, StoreResolveRequest, StoreResolvedIdentity,
    StoreSealIdentity, StoreSlot, StoreWatchReceipt, StoreWatchRequest, StoredResource,
    StoredSchema,
};
use redb::Database;
use redb::backends::FileBackend;
use rustix::io::{FdFlags, fcntl_getfd};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};

pub use actor::{
    BackendSignals, GROUP_COMMIT_MAX, MAX_CONCURRENT_READS, READ_LIFETIME, READ_POOL_THREADS,
    SharedChangeBatch, WRITE_QUEUE_CAPACITY,
};
pub use backup::{
    BackupRow, BackupTable, LOGICAL_BACKUP_FORMAT_VERSION, LogicalBackup, MAX_LOGICAL_BACKUP_BYTES,
    MAX_LOGICAL_BACKUP_ROWS, MAX_PUBLICATION_NAME_BYTES, PublicationState, publication_state,
    publish_staged, sync_staged_file,
};
pub use keys::{
    DecodedKey, DecodedKeyComponent, EncodedKey, KeyCodecError, KeyComponent, KeySpace,
    MAX_ENCODED_KEY_BYTES, MAX_KEY_COMPONENTS, MAX_TEXT_COMPONENT_BYTES, encode_key,
};
pub use ownership::{
    MAX_OWNER_CHAIN_DEPTH, OwnerBinding, OwnerIndex, OwnerIndexMutation, OwnershipError,
    ReverseOwnerEntry,
};
pub use revision_log::{
    MAX_COMPACTION_BYTES_PER_TRANSACTION, MAX_COMPACTION_ROWS_PER_TRANSACTION,
    MAX_INITIAL_WATCH_CREDITS, MAX_RETAINED_RESUME_CURSORS, MAX_WATCH_REGISTRATIONS,
    WATCH_ADMISSION_CAPACITY, WatchCoordinator, WatchRegistrationId, WatchSelector, WatchSignals,
    WatchStream, compact,
};
pub use schema::{TABLE_SCHEMAS, TableSchema};
pub use transaction::{ChangeBatch, ChangeEntry, ChangeEvent};
pub use values::{
    DecodedValue, EncodedValue, MAX_ENCODED_VALUE_BYTES, MAX_VALUE_PAYLOAD_BYTES, ValueCodecError,
    ValueKind, encode_value,
};

/// Bound redb's page cache so database scale cannot turn into process RSS.
pub const REDB_CACHE_SIZE: usize = 4 * 1024 * 1024;

/// Immutable identity and generation binding for one already-provisioned store.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreIdentity {
    slot: StoreSlot,
    store_uuid: ResourceUid,
    zone: ZoneId,
    zone_uid: ResourceUid,
    created_at: String,
    revisions: PolicySnapshot,
}

impl StoreIdentity {
    pub fn new(
        slot: StoreSlot,
        store_uuid: ResourceUid,
        zone: ZoneId,
        zone_uid: ResourceUid,
        created_at: Timestamp,
        revisions: PolicySnapshot,
    ) -> Self {
        Self {
            slot,
            store_uuid,
            zone,
            zone_uid,
            created_at: created_at.as_str().to_owned(),
            revisions,
        }
    }

    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    pub const fn slot(&self) -> StoreSlot {
        self.slot
    }

    pub fn seal_identity(&self) -> StoreSealIdentity {
        StoreSealIdentity::new(self.slot, self.zone.clone(), self.store_uuid.clone())
    }
}

impl core::fmt::Debug for StoreIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StoreIdentity(<redacted>)")
    }
}

/// One concrete backend whose mutation authority is instance-bound.
pub struct RedbResourceStore {
    identity: StoreIdentity,
    recovered_after_crash: bool,
    writer: WriterHandle,
    reads: ReadPool,
    signals: Arc<SignalCounters>,
    seal: MutationSealAcceptor,
    watch_coordinator: Arc<Mutex<WatchCoordinator>>,
    retained_watch_streams: Mutex<BTreeMap<WatchRegistrationId, WatchStream>>,
}

impl core::fmt::Debug for RedbResourceStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RedbResourceStore(<redacted>)")
    }
}

impl RedbResourceStore {
    /// Initialize one unpublished empty database after validating its durable marker.
    pub async fn provision_owned(
        file: File,
        mut marker: File,
        identity: StoreIdentity,
        acceptor: MutationSealAcceptor,
    ) -> Result<Self, StoreError> {
        let slot = identity.slot();
        validate_acceptor(&identity, &acceptor)?;
        validate_owned_file(&file).map_err(|error| error.with_store_slot(slot))?;
        validate_owned_file(&marker).map_err(|error| error.with_store_slot(slot))?;
        if file
            .metadata()
            .map_err(transaction::integrity)
            .map_err(|error| error.with_store_slot(slot))?
            .len()
            != 0
        {
            return Err(
                transaction::integrity("provision-database-not-empty").with_store_slot(slot)
            );
        }
        validate_provisioning_marker(&mut marker, &identity)
            .map_err(|error| error.with_store_slot(slot))?;
        let open_identity = identity.clone();
        let database = tokio::task::spawn_blocking(move || {
            let backend = FileBackend::new(file).map_err(transaction::integrity)?;
            let database = Database::builder()
                .set_cache_size(REDB_CACHE_SIZE)
                .create_with_backend(backend)
                .map_err(transaction::integrity)?;
            transaction::initialize(&database, &open_identity)?;
            Ok::<_, StoreError>(database)
        })
        .await
        .map_err(|_| {
            transaction::integrity("database-provision-task-failed").with_store_slot(slot)
        })?
        .map_err(|error| error.with_store_slot(slot))?;
        Self::start(database, identity, false, acceptor)
            .map_err(|error| error.with_store_slot(slot))
    }

    /// Consume an already-provisioned nonempty database file.
    ///
    /// Empty existing files are quarantined rather than initialized.
    pub async fn open_owned(
        file: File,
        identity: StoreIdentity,
        acceptor: MutationSealAcceptor,
    ) -> Result<Self, StoreError> {
        let slot = identity.slot();
        validate_acceptor(&identity, &acceptor)?;
        validate_owned_file(&file).map_err(|error| error.with_store_slot(slot))?;
        if file
            .metadata()
            .map_err(transaction::integrity)
            .map_err(|error| error.with_store_slot(slot))?
            .len()
            == 0
        {
            return Err(
                transaction::quarantined_reason("provisioned-store-empty").with_store_slot(slot)
            );
        }
        let open_identity = identity.clone();
        let database = tokio::task::spawn_blocking(move || {
            let backend = FileBackend::new(file).map_err(transaction::integrity)?;
            let database = Database::builder()
                .set_cache_size(REDB_CACHE_SIZE)
                .create_with_backend(backend)
                .map_err(transaction::integrity)?;
            let meta = transaction::validate_identity(&database, &open_identity)?;
            transaction::validate_consistency(&database)?;
            let recovered_after_crash = !meta.clean_shutdown;
            Ok::<_, StoreError>((database, recovered_after_crash))
        })
        .await
        .map_err(|_| transaction::integrity("database-open-task-failed").with_store_slot(slot))?
        .map_err(|error| error.with_store_slot(slot))?;
        let (database, recovered_after_crash) = database;
        Self::start(database, identity, recovered_after_crash, acceptor)
            .map_err(|error| error.with_store_slot(slot))
    }

    fn start(
        database: Database,
        identity: StoreIdentity,
        recovered_after_crash: bool,
        seal: MutationSealAcceptor,
    ) -> Result<Self, StoreError> {
        let database = Arc::new(database);
        let signals = Arc::new(SignalCounters::default());
        let reads = ReadPool::start(Arc::clone(&database), identity.zone.clone())?;
        let watch_coordinator = Arc::new(Mutex::new(WatchCoordinator::default()));
        let writer = WriterHandle::start(
            database,
            Arc::clone(&signals),
            Arc::clone(&watch_coordinator),
        )?;
        Ok(Self {
            identity,
            recovered_after_crash,
            writer,
            reads,
            signals,
            seal,
            watch_coordinator,
            retained_watch_streams: Mutex::new(BTreeMap::new()),
        })
    }

    /// Policy-neutral replay/live primitive for a future watch coordinator.
    pub async fn replay_backend(
        &self,
        after_revision: u64,
        resource_types: impl IntoIterator<Item = d2b_contracts::v3::ResourceTypeName>,
        visit: impl FnMut(SharedChangeBatch) -> Result<(), StoreError> + Send + 'static,
    ) -> Result<d2b_contracts::v3::ZoneRevision, StoreError> {
        let meta = self.reads.meta().await?;
        if after_revision < meta.compaction_floor {
            return Err(transaction::revision_expired(meta.current_revision));
        }
        self.writer
            .replay(after_revision, resource_types.into_iter().collect(), visit)
            .await
    }

    /// Capture a consistent logical snapshot while the writer owns ordering.
    pub async fn logical_backup(&self) -> Result<LogicalBackup, StoreError> {
        self.writer.backup(self.identity.clone()).await
    }

    /// Alias used by storage owners when exporting the logical image.
    pub async fn backup(&self) -> Result<LogicalBackup, StoreError> {
        self.logical_backup().await
    }

    pub fn signals(&self) -> BackendSignals {
        self.signals.snapshot()
    }

    pub const fn identity(&self) -> &StoreIdentity {
        &self.identity
    }

    /// Whether the existing store lacked a clean-shutdown marker when opened.
    pub const fn recovered_after_crash(&self) -> bool {
        self.recovered_after_crash
    }

    /// Persist a clean-shutdown marker and join the owned worker threads.
    pub async fn shutdown(mut self) -> Result<(), StoreError> {
        if let Ok(mut streams) = self.retained_watch_streams.lock() {
            streams.clear();
        }
        self.reads.shutdown()?;
        self.writer.shutdown().await
    }

    /// Restore a validated logical image into a new owned descriptor.
    ///
    /// The target descriptor must be empty and the marker must already have
    /// been provisioned for the same store identity.  Publication of the
    /// staged descriptor remains an fd-relative storage-owner operation.
    pub async fn restore_owned(
        file: File,
        mut marker: File,
        backup: LogicalBackup,
        identity: StoreIdentity,
        acceptor: MutationSealAcceptor,
    ) -> Result<Self, StoreError> {
        let slot = identity.slot();
        validate_acceptor(&identity, &acceptor)?;
        validate_owned_file(&file).map_err(|error| error.with_store_slot(slot))?;
        validate_owned_file(&marker).map_err(|error| error.with_store_slot(slot))?;
        validate_provisioning_marker(&mut marker, &identity)
            .map_err(|error| error.with_store_slot(slot))?;
        if file
            .metadata()
            .map_err(transaction::integrity)
            .map_err(|error| error.with_store_slot(slot))?
            .len()
            != 0
        {
            return Err(
                transaction::quarantined_reason("restore-target-not-empty").with_store_slot(slot)
            );
        }
        let open_identity = identity.clone();
        let database =
            tokio::task::spawn_blocking(move || backup.restore_file(file, &open_identity))
                .await
                .map_err(|_| {
                    transaction::integrity("database-restore-task-failed").with_store_slot(slot)
                })?
                .map_err(|error| error.with_store_slot(slot))?;
        Self::start(database, identity, false, acceptor)
            .map_err(|error| error.with_store_slot(slot))
    }
}

impl RedbResourceStore {
    pub async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        self.reads.get(request).await
    }

    pub async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        self.reads.list(request).await
    }

    /// Open a watch and return its stream to the caller that owns delivery.
    ///
    /// Registration, replay, and the writer's live-delivery boundary execute
    /// in the same actor, so no commit can fall between registration and the
    /// replay high-water mark.
    pub async fn watch_stream(
        &self,
        request: StoreWatchRequest,
    ) -> Result<(StoreWatchReceipt, WatchStream), StoreError> {
        if request.zone != self.identity.zone {
            return Err(transaction::integrity("request-zone-mismatch"));
        }
        let selector = WatchSelector::new(
            request.resource_types,
            request.resource_names,
            request.filters,
        );
        let (stream, snapshot_revision) = self
            .writer
            .watch(request.after_revision, selector, request.initial_credits)
            .await?;
        let receipt = StoreWatchReceipt {
            stream_name: Self::stream_name(stream.id()),
            snapshot_revision,
        };
        Ok((receipt, stream))
    }

    /// Register a watch for the resource API and retain its stream by id.
    ///
    /// The API's current receipt-only contract hands the stream to a named
    /// bus layer later.  [`Self::take_watch_stream`] is the single transfer
    /// point for that handoff.
    pub async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        let (receipt, stream) = self.watch_stream(request).await?;
        let id = stream.id();
        let mut retained = match self.retained_watch_streams.lock() {
            Ok(retained) => retained,
            Err(_) => {
                self.unregister_watch_now(id);
                return Err(transaction::integrity("watch-stream-registry-poisoned"));
            }
        };
        if retained.insert(id, stream).is_some() {
            drop(retained);
            self.unregister_watch_now(id);
            return Err(transaction::integrity("watch-registration-duplicate"));
        }
        Ok(receipt)
    }

    /// Transfer a receipt-created stream to the bus/session owner.
    pub fn take_watch_stream(
        &self,
        id: WatchRegistrationId,
    ) -> Result<Option<WatchStream>, StoreError> {
        self.retained_watch_streams
            .lock()
            .map_err(|_| transaction::integrity("watch-stream-registry-poisoned"))
            .map(|mut streams| streams.remove(&id))
    }

    /// Transfer a receipt-created stream by its opaque receipt name.
    pub fn take_watch_stream_named(&self, name: &str) -> Result<Option<WatchStream>, StoreError> {
        let id = name
            .strip_prefix("watch-")
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u64>().ok())
            .map(WatchRegistrationId::from_raw)
            .ok_or_else(|| transaction::integrity("watch-stream-name-invalid"))?;
        if Self::stream_name(id) != name {
            return Err(transaction::integrity("watch-stream-name-invalid"));
        }
        self.take_watch_stream(id)
    }

    /// Acknowledge all queued deliveries through `revision`.
    pub async fn acknowledge_watch(
        &self,
        id: WatchRegistrationId,
        revision: d2b_contracts::v3::ZoneRevision,
    ) -> Result<(), StoreError> {
        self.writer.acknowledge_watch(id, revision).await
    }

    /// Unregister a watch and release all of its global budget.
    pub async fn unregister_watch(
        &self,
        id: WatchRegistrationId,
    ) -> Result<Option<d2b_contracts::v3::ZoneRevision>, StoreError> {
        self.retained_watch_streams
            .lock()
            .map_err(|_| transaction::integrity("watch-stream-registry-poisoned"))?
            .remove(&id);
        self.writer.unregister_watch(id).await
    }

    /// Unregister a watch from a synchronous owner-drop path.
    pub fn unregister_watch_now(&self, id: WatchRegistrationId) {
        if let Ok(mut streams) = self.retained_watch_streams.lock() {
            streams.remove(&id);
        }
        if let Ok(mut coordinator) = self.watch_coordinator.lock() {
            let _ = coordinator.unregister(id);
        }
    }

    /// Return the fixed-cardinality watch saturation snapshot.
    pub fn watch_signals(&self) -> Result<WatchSignals, StoreError> {
        self.watch_coordinator
            .lock()
            .map_err(|_| transaction::integrity("watch-coordinator-poisoned"))
            .map(|coordinator| coordinator.signals())
    }

    pub async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        self.reads.resolve(request).await
    }

    fn stream_name(id: WatchRegistrationId) -> String {
        format!("watch-{}", id.get())
    }

    pub async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        self.reads.inspect_schema(request).await
    }
}

impl RedbResourceStore {
    /// Commit only evidence opened by this store's paired acceptor.
    pub async fn commit_verified(
        &self,
        sealed: SealedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        let opened = self.seal.open(sealed)?;
        self.writer.commit(opened).await
    }
}

fn validate_acceptor(
    identity: &StoreIdentity,
    acceptor: &MutationSealAcceptor,
) -> Result<(), StoreError> {
    if let Err(mismatch) = acceptor.diagnose(&identity.seal_identity()) {
        return Err(transaction::integrity(mismatch.reason_code()).with_store_slot(identity.slot()));
    }
    if acceptor.declared_slot() != identity.slot() {
        return Err(
            transaction::integrity("mutation-seal-acceptor-slot-mismatch")
                .with_store_slot(identity.slot()),
        );
    }
    Ok(())
}

/// Publish marker bytes for a storage owner before initial database creation.
pub fn write_provisioning_marker(
    marker: &mut File,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let slot = identity.slot();
    validate_owned_file(marker).map_err(|error| error.with_store_slot(slot))?;
    if marker
        .metadata()
        .map_err(transaction::integrity)
        .map_err(|error| error.with_store_slot(slot))?
        .len()
        != 0
    {
        return Err(transaction::integrity("provision-marker-not-empty").with_store_slot(slot));
    }
    marker
        .write_all(provisioning_marker_bytes(identity).as_bytes())
        .and_then(|()| marker.sync_all())
        .map_err(transaction::durability_failure)
        .map_err(|error| error.with_store_slot(slot))
}

fn validate_provisioning_marker(
    marker: &mut File,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    marker
        .seek(SeekFrom::Start(0))
        .map_err(transaction::integrity)?;
    let mut bytes = Vec::new();
    marker
        .take(4096)
        .read_to_end(&mut bytes)
        .map_err(transaction::integrity)?;
    if bytes != provisioning_marker_bytes(identity).as_bytes() {
        return Err(transaction::quarantined_reason(
            "provision-marker-identity-mismatch",
        ));
    }
    Ok(())
}

fn provisioning_marker_bytes(identity: &StoreIdentity) -> String {
    format!(
        "d2b-redb-store/v1\n{}\n{}\n{}\n{}\n",
        identity.store_uuid.as_str(),
        identity.zone.as_str(),
        identity.zone_uid.as_str(),
        identity.created_at
    )
}

fn validate_owned_file(file: &File) -> Result<(), StoreError> {
    let metadata = file.metadata().map_err(transaction::integrity)?;
    if !metadata.file_type().is_file() {
        return Err(transaction::integrity("database-fd-is-not-regular"));
    }
    if !fcntl_getfd(file)
        .map_err(transaction::integrity)?
        .contains(FdFlags::CLOEXEC)
    {
        return Err(transaction::integrity("database-fd-missing-cloexec"));
    }
    Ok(())
}

/// Atomically receive exactly one database fd with `MSG_CMSG_CLOEXEC`.
pub fn receive_database_file(socket: impl AsFd) -> Result<File, StoreError> {
    let mut payload = [0_u8; 1];
    let mut iov = [rustix::io::IoSliceMut::new(&mut payload)];
    let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(2))];
    let mut control = RecvAncillaryBuffer::new(&mut control_bytes);
    let result = recvmsg(socket, &mut iov, &mut control, RecvFlags::CMSG_CLOEXEC)
        .map_err(transaction::integrity)?;
    const MSG_CTRUNC: RecvFlags = RecvFlags::from_bits_retain(0x08);
    if result.bytes != 1 || result.flags.contains(RecvFlags::TRUNC | MSG_CTRUNC) {
        return Err(transaction::integrity("database-fd-frame-invalid"));
    }
    let mut received = Vec::<OwnedFd>::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(files) = message {
            received.extend(files);
        } else {
            return Err(transaction::integrity("database-fd-control-invalid"));
        }
    }
    if received.len() != 1 {
        return Err(transaction::integrity("database-fd-count-invalid"));
    }
    let file = File::from(received.pop().expect("one fd was checked"));
    validate_owned_file(&file)?;
    Ok(file)
}

#[cfg(test)]
mod tests;
