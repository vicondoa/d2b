//! Production redb backend for one Zone resource store.

pub mod actor;
pub mod keys;
pub mod ownership;
pub mod schema;
mod transaction;
pub mod values;

use std::fs::File;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;

use actor::{BackendWatch, ReadPool, SignalCounters, WriterHandle};
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
pub use transaction::{ChangeBatch, ChangeEntry};
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

/// One concrete backend. The API trait is its only mutation surface.
pub struct RedbResourceStore {
    identity: StoreIdentity,
    mutation_authority: Arc<MutationAuthority>,
    recovered_after_crash: bool,
    writer: WriterHandle,
    reads: ReadPool,
    signals: Arc<SignalCounters>,
}

struct MutationAuthority;

/// Single-owner backend commit capability for one concrete store instance.
///
/// This is a wiring capability. It must move directly into the API bridge and
/// must never be exposed through a provider, controller, or public store client.
pub struct MutationPort {
    authority: Arc<MutationAuthority>,
}

impl core::fmt::Debug for MutationPort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MutationPort(<redacted>)")
    }
}

/// Backend-ready checked mutation supplied by the API bridge.
///
/// External callers cannot construct a mutation without a store-issued port.
///
/// ```compile_fail
/// use d2b_resource_store_redb::MutationPort;
///
/// let _forged = MutationPort {};
/// ```
pub struct CheckedMutation {
    authority: Arc<MutationAuthority>,
    authorization: AdmittedAuthorization,
    policy_snapshot: PolicySnapshot,
    operation: StoreOperationContext,
    mutations: Vec<CheckedPreparedMutation>,
}

/// One prepared full-replacement mutation and its finalized identity.
pub struct CheckedPreparedMutation {
    mutation: StoreMutation,
    resource_uid: Option<ResourceUid>,
    payload_digest: Option<String>,
}

impl core::fmt::Debug for CheckedMutation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CheckedMutation(<redacted>)")
    }
}

impl core::fmt::Debug for CheckedPreparedMutation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CheckedPreparedMutation")
            .field("kind", &self.mutation.kind)
            .field("has_resource_uid", &self.resource_uid.is_some())
            .field("has_payload_digest", &self.payload_digest.is_some())
            .finish()
    }
}

impl CheckedMutation {
    #[doc(hidden)]
    pub fn new(
        port: &MutationPort,
        authorization: AdmittedAuthorization,
        policy_snapshot: PolicySnapshot,
        operation: StoreOperationContext,
        mutations: Vec<CheckedPreparedMutation>,
    ) -> Self {
        Self {
            authority: Arc::clone(&port.authority),
            authorization,
            policy_snapshot,
            operation,
            mutations,
        }
    }

    pub const fn authorization(&self) -> &AdmittedAuthorization {
        &self.authorization
    }

    pub fn mutations(&self) -> &[CheckedPreparedMutation] {
        &self.mutations
    }
}

impl CheckedPreparedMutation {
    #[doc(hidden)]
    pub fn new(
        mutation: StoreMutation,
        resource_uid: Option<ResourceUid>,
        payload_digest: Option<String>,
    ) -> Self {
        Self {
            mutation,
            resource_uid,
            payload_digest,
        }
    }

    pub const fn mutation(&self) -> &StoreMutation {
        &self.mutation
    }

    pub const fn resource_uid(&self) -> Option<&ResourceUid> {
        self.resource_uid.as_ref()
    }

    pub fn payload_digest(&self) -> Option<&str> {
        self.payload_digest.as_deref()
    }
}

impl core::fmt::Debug for RedbResourceStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RedbResourceStore(<redacted>)")
    }
}

impl RedbResourceStore {
    /// Consume one owned regular database file with close-on-exec already set.
    ///
    /// An empty file is initialized. A nonempty file must match the supplied
    /// immutable store identity and physical schema or opening fails closed.
    pub async fn open_owned(
        file: File,
        identity: StoreIdentity,
    ) -> Result<(Self, MutationPort), StoreError> {
        validate_owned_file(&file)?;
        let initialize = file.metadata().map_err(transaction::integrity)?.len() == 0;
        let open_identity = identity.clone();
        let database = tokio::task::spawn_blocking(move || {
            let backend = FileBackend::new(file).map_err(transaction::integrity)?;
            let database = Database::builder()
                .create_with_backend(backend)
                .map_err(transaction::integrity)?;
            let recovered_after_crash = if initialize {
                transaction::initialize(&database, &open_identity)?;
                false
            } else {
                let meta = transaction::validate_identity(&database, &open_identity)?;
                transaction::validate_consistency(&database)?;
                !meta.clean_shutdown
            };
            Ok::<_, StoreError>((database, recovered_after_crash))
        })
        .await
        .map_err(|_| transaction::integrity("database-open-task-failed"))??;
        let (database, recovered_after_crash) = database;
        let database = Arc::new(database);
        let signals = Arc::new(SignalCounters::default());
        let reads = ReadPool::start(Arc::clone(&database), identity.zone.clone());
        let writer = WriterHandle::start(database, Arc::clone(&signals));
        let mutation_authority = Arc::new(MutationAuthority);
        let port = MutationPort {
            authority: Arc::clone(&mutation_authority),
        };
        Ok((
            Self {
                identity,
                mutation_authority,
                recovered_after_crash,
                writer,
                reads,
                signals,
            },
            port,
        ))
    }

    /// Backend-owned replay/live primitive consumed by the watch coordinator.
    ///
    /// Global admission and slow-watcher policy intentionally remain outside
    /// this backend.
    pub async fn watch_backend(
        &self,
        after_revision: u64,
        resource_types: impl IntoIterator<Item = d2b_contracts::v3::ResourceTypeName>,
    ) -> Result<BackendWatch, StoreError> {
        let meta = self.reads.meta().await?;
        if after_revision < meta.compaction_floor {
            return Err(transaction::revision_expired(meta.current_revision));
        }
        self.writer
            .register_watch(after_revision, resource_types.into_iter().collect())
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
}

impl RedbResourceStore {
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
        let after_revision = request.after_revision.get();
        let watch = self
            .watch_backend(after_revision, request.resource_types)
            .await?;
        Ok(StoreWatchReceipt {
            stream_name: format!("redb-watch-{}", watch.id()),
            snapshot_revision: watch.high_water(),
        })
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

    pub async fn commit_checked(
        &self,
        mutation: CheckedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        if !Arc::ptr_eq(&self.mutation_authority, &mutation.authority) {
            return Err(transaction::integrity("mutation-store-identity-mismatch"));
        }
        self.writer.commit(mutation).await
    }
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
