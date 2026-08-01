//! Production redb backend for one Zone resource store.

pub mod actor;
pub mod keys;
pub mod ownership;
pub mod schema;
mod transaction;
pub mod values;

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::marker::PhantomData;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;

use actor::{ReadPool, SignalCounters, WriterHandle};
use d2b_contracts::v3::{ResourceUid, Timestamp, ZoneId};
use d2b_resource_store::{
    AdmittedAuthorization, PolicySnapshot, StoreCommitResult, StoreError, StoreGetRequest,
    StoreInspectSchemaRequest, StoreListRequest, StoreListResult, StoreMutation,
    StoreOperationContext, StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt,
    StoreWatchRequest, StoredResource, StoredSchema,
};
use redb::Database;
use redb::backends::FileBackend;
use rustix::io::{FdFlags, fcntl_getfd};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};

pub use actor::{
    BackendSignals, GROUP_COMMIT_MAX, MAX_CONCURRENT_READS, READ_LIFETIME, READ_POOL_THREADS,
    SharedChangeBatch, WRITE_QUEUE_CAPACITY,
};
pub use keys::{
    DecodedKey, DecodedKeyComponent, EncodedKey, KeyCodecError, KeyComponent, KeySpace,
    MAX_ENCODED_KEY_BYTES, MAX_KEY_COMPONENTS, MAX_TEXT_COMPONENT_BYTES, encode_key,
};
pub use ownership::{
    MAX_OWNER_CHAIN_DEPTH, OwnerBinding, OwnerIndex, OwnerIndexMutation, OwnershipError,
    ReverseOwnerEntry,
};
pub use schema::{TABLE_SCHEMAS, TableSchema};
pub use transaction::{ChangeBatch, ChangeEntry, ChangeEvent};
pub use values::{
    DecodedValue, EncodedValue, MAX_ENCODED_VALUE_BYTES, MAX_VALUE_PAYLOAD_BYTES, ValueCodecError,
    ValueKind, encode_value,
};

/// Immutable identity and generation binding for one already-provisioned store.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreIdentity {
    store_uuid: String,
    zone: ZoneId,
    zone_uid: ResourceUid,
    created_at: String,
    revisions: PolicySnapshot,
}

impl StoreIdentity {
    pub fn new(
        store_uuid: ResourceUid,
        zone: ZoneId,
        zone_uid: ResourceUid,
        created_at: Timestamp,
        revisions: PolicySnapshot,
    ) -> Self {
        Self {
            store_uuid: store_uuid.as_str().to_owned(),
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
}

impl core::fmt::Debug for StoreIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StoreIdentity(<redacted>)")
    }
}

/// One concrete backend whose mutation authority is instance-bound.
pub struct RedbResourceStore<V> {
    identity: StoreIdentity,
    recovered_after_crash: bool,
    writer: WriterHandle,
    reads: ReadPool,
    signals: Arc<SignalCounters>,
    verified_mutation: PhantomData<fn(V)>,
}

/// Read-only view of one API-owned prepared mutation.
pub trait VerifiedPreparedMutationView {
    fn mutation(&self) -> &StoreMutation;
    fn resource_uid(&self) -> Option<&ResourceUid>;
}

/// API-owned verified mutation required by this backend's only write method.
///
/// The API crate owns the concrete evidence type, so another crate can neither
/// construct that type nor implement this trait for it.
pub trait VerifiedMutationView: Send {
    type Prepared: VerifiedPreparedMutationView;

    fn authorization(&self) -> &AdmittedAuthorization;
    fn policy_snapshot(&self) -> PolicySnapshot;
    fn operation(&self) -> &StoreOperationContext;
    fn mutations(&self) -> &[Self::Prepared];
}

impl<V> core::fmt::Debug for RedbResourceStore<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RedbResourceStore(<redacted>)")
    }
}

impl<V> RedbResourceStore<V> {
    /// Initialize one unpublished empty database after validating its durable marker.
    pub async fn provision_owned(
        file: File,
        mut marker: File,
        identity: StoreIdentity,
    ) -> Result<Self, StoreError> {
        validate_owned_file(&file)?;
        validate_owned_file(&marker)?;
        if file.metadata().map_err(transaction::integrity)?.len() != 0 {
            return Err(transaction::integrity("provision-database-not-empty"));
        }
        validate_provisioning_marker(&mut marker, &identity)?;
        let open_identity = identity.clone();
        let database = tokio::task::spawn_blocking(move || {
            let backend = FileBackend::new(file).map_err(transaction::integrity)?;
            let database = Database::builder()
                .create_with_backend(backend)
                .map_err(transaction::integrity)?;
            transaction::initialize(&database, &open_identity)?;
            Ok::<_, StoreError>(database)
        })
        .await
        .map_err(|_| transaction::integrity("database-provision-task-failed"))??;
        Self::start(database, identity, false)
    }

    /// Consume an already-provisioned nonempty database file.
    ///
    /// Empty existing files are quarantined rather than initialized.
    pub async fn open_owned(file: File, identity: StoreIdentity) -> Result<Self, StoreError> {
        validate_owned_file(&file)?;
        if file.metadata().map_err(transaction::integrity)?.len() == 0 {
            return Err(transaction::quarantined_reason("provisioned-store-empty"));
        }
        let open_identity = identity.clone();
        let database = tokio::task::spawn_blocking(move || {
            let backend = FileBackend::new(file).map_err(transaction::integrity)?;
            let database = Database::builder()
                .create_with_backend(backend)
                .map_err(transaction::integrity)?;
            let meta = transaction::validate_identity(&database, &open_identity)?;
            transaction::validate_consistency(&database)?;
            let recovered_after_crash = !meta.clean_shutdown;
            Ok::<_, StoreError>((database, recovered_after_crash))
        })
        .await
        .map_err(|_| transaction::integrity("database-open-task-failed"))??;
        let (database, recovered_after_crash) = database;
        Self::start(database, identity, recovered_after_crash)
    }

    fn start(
        database: Database,
        identity: StoreIdentity,
        recovered_after_crash: bool,
    ) -> Result<Self, StoreError> {
        let database = Arc::new(database);
        let signals = Arc::new(SignalCounters::default());
        let reads = ReadPool::start(Arc::clone(&database), identity.zone.clone())?;
        let writer = WriterHandle::start(database, Arc::clone(&signals))?;
        Ok(Self {
            identity,
            recovered_after_crash,
            writer,
            reads,
            signals,
            verified_mutation: PhantomData,
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
        self.reads.shutdown()?;
        self.writer.shutdown().await
    }
}

impl<V> RedbResourceStore<V> {
    pub async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        self.reads.get(request).await
    }

    pub async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        self.reads.list(request).await
    }

    pub async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        if request.zone != self.identity.zone {
            return Err(transaction::integrity("request-zone-mismatch"));
        }
        Err(transaction::unavailable("watch-coordinator-unavailable"))
    }

    pub async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        self.reads.resolve(request).await
    }

    pub async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        self.reads.inspect_schema(request).await
    }
}

impl<V> RedbResourceStore<V>
where
    V: VerifiedMutationView,
{
    /// Commit only the exact API-owned verified mutation type bound at open.
    pub async fn commit_verified(&self, mutation: V) -> Result<StoreCommitResult, StoreError> {
        self.writer.commit(mutation).await
    }
}

/// Publish marker bytes for a storage owner before initial database creation.
pub fn write_provisioning_marker(
    marker: &mut File,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    validate_owned_file(marker)?;
    if marker.metadata().map_err(transaction::integrity)?.len() != 0 {
        return Err(transaction::integrity("provision-marker-not-empty"));
    }
    marker
        .write_all(provisioning_marker_bytes(identity).as_bytes())
        .and_then(|()| marker.sync_all())
        .map_err(transaction::durability_failure)
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
        identity.store_uuid,
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
