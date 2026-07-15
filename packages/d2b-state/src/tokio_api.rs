use std::{
    os::fd::OwnedFd,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use d2b_contracts::v2_state::{
    AtomicWriteReceipt, AuditRecord, AuditStream, Digest, LockSpec, OwnershipEpoch, ResourceId,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    AnchoredDir, AnchoredResource, AtomicFilesystem, AtomicWrite, AuditAppender, AuditRecordInput,
    Cancellation, Clock, DurableState, Error, ErrorCode, LockSet, MetadataExpectation,
    QuarantineRecord, ReadPolicy, Result, WritePolicy,
};

#[derive(Debug)]
struct AsyncLockSetInner {
    locks: Mutex<LockSet>,
    acquisition_pending: AtomicBool,
}

#[derive(Clone, Debug)]
pub struct AsyncLockSet {
    inner: Arc<AsyncLockSetInner>,
}

impl Default for AsyncLockSet {
    fn default() -> Self {
        Self::from_lock_set(LockSet::new())
    }
}

impl AsyncLockSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_lock_set(locks: LockSet) -> Self {
        Self {
            inner: Arc::new(AsyncLockSetInner {
                locks: Mutex::new(locks),
                acquisition_pending: AtomicBool::new(false),
            }),
        }
    }

    pub async fn held(&self, lock_id: &ResourceId) -> Result<bool> {
        let locks = Arc::clone(&self.inner);
        let lock_id = lock_id.clone();
        blocking(move || Ok(lock_ready(&locks)?.held(&lock_id))).await
    }
}

fn lock_shared(inner: &AsyncLockSetInner) -> Result<MutexGuard<'_, LockSet>> {
    inner
        .locks
        .lock()
        .map_err(|_| Error::Code(ErrorCode::LockMismatch))
}

fn lock_ready(inner: &AsyncLockSetInner) -> Result<MutexGuard<'_, LockSet>> {
    if inner.acquisition_pending.load(Ordering::Acquire) {
        return Err(Error::Code(ErrorCode::LockContended));
    }
    let locks = lock_shared(inner)?;
    if inner.acquisition_pending.load(Ordering::Acquire) {
        return Err(Error::Code(ErrorCode::LockContended));
    }
    Ok(locks)
}

async fn blocking<T, F>(operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(Error::task_join)?
}

struct CancelOnDrop {
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl CancelOnDrop {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

struct CombinedCancellation {
    caller: Arc<dyn Cancellation + Send + Sync>,
    dropped: Arc<AtomicBool>,
}

struct PendingLockAcquisition {
    slot: AcquisitionSlot,
    lock_id: ResourceId,
    claimed: bool,
}

impl PendingLockAcquisition {
    fn claim(mut self) {
        self.claimed = true;
        self.slot.release();
    }
}

impl Drop for PendingLockAcquisition {
    fn drop(&mut self) {
        if self.claimed {
            return;
        }
        let mut locks = self
            .slot
            .inner
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if locks
            .last()
            .is_some_and(|guard| guard.lock_id() == &self.lock_id)
        {
            let _ = locks.release_last();
        }
    }
}

struct AcquisitionSlot {
    inner: Arc<AsyncLockSetInner>,
    armed: bool,
}

impl AcquisitionSlot {
    fn reserve(inner: Arc<AsyncLockSetInner>) -> Result<Self> {
        inner
            .acquisition_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Code(ErrorCode::LockContended))?;
        Ok(Self { inner, armed: true })
    }

    fn release(&mut self) {
        if self.armed {
            self.inner
                .acquisition_pending
                .store(false, Ordering::Release);
            self.armed = false;
        }
    }
}

impl Drop for AcquisitionSlot {
    fn drop(&mut self) {
        self.release();
    }
}

impl Cancellation for CombinedCancellation {
    fn is_cancelled(&self) -> bool {
        self.dropped.load(Ordering::Acquire) || self.caller.is_cancelled()
    }

    fn acquisition_abandoned(&self) -> bool {
        self.dropped.load(Ordering::Acquire)
    }
}

pub async fn async_open_anchored_dir(path: PathBuf) -> Result<AnchoredDir> {
    blocking(move || AnchoredDir::open_trusted(&path)).await
}

pub async fn async_anchored_dir_from_fd(fd: OwnedFd) -> Result<AnchoredDir> {
    blocking(move || AnchoredDir::from_owned_fd(fd)).await
}

pub async fn async_atomic_read<F, T>(
    mut writer: AtomicWrite<F>,
    policy: ReadPolicy,
) -> Result<(AtomicWrite<F>, DurableState<T>)>
where
    F: AtomicFilesystem + Send + 'static,
    T: DeserializeOwned + Serialize + PartialEq + Send + 'static,
{
    blocking(move || {
        let state = writer.read::<T>(&policy)?;
        Ok((writer, state))
    })
    .await
}

pub async fn async_atomic_write<F, T>(
    mut writer: AtomicWrite<F>,
    payload: T,
    policy: WritePolicy,
    locks: &AsyncLockSet,
) -> Result<(AtomicWrite<F>, AtomicWriteReceipt)>
where
    F: AtomicFilesystem + Send + 'static,
    T: DeserializeOwned + Serialize + PartialEq + Send + 'static,
{
    let locks = Arc::clone(&locks.inner);
    blocking(move || {
        let locks = lock_ready(&locks)?;
        let receipt = writer.write(&payload, &policy, locks.last())?;
        Ok((writer, receipt))
    })
    .await
}

pub async fn async_atomic_quarantine<F>(
    mut writer: AtomicWrite<F>,
    record: QuarantineRecord,
    locks: &AsyncLockSet,
) -> Result<AtomicWrite<F>>
where
    F: AtomicFilesystem + Send + 'static,
{
    let locks = Arc::clone(&locks.inner);
    blocking(move || {
        let locks = lock_ready(&locks)?;
        writer.quarantine(&record, locks.last())?;
        Ok(writer)
    })
    .await
}

pub async fn async_ofd_lock_acquire(
    locks: &AsyncLockSet,
    spec: LockSpec,
    resource: AnchoredResource,
    metadata: MetadataExpectation,
    ownership_epoch: OwnershipEpoch,
    cancellation: Arc<dyn Cancellation + Send + Sync>,
) -> Result<()> {
    async_ofd_lock_acquire_with_clock(
        locks,
        spec,
        resource,
        metadata,
        ownership_epoch,
        cancellation,
        Arc::new(crate::SystemClock),
    )
    .await
}

pub async fn async_ofd_lock_acquire_with_clock(
    locks: &AsyncLockSet,
    spec: LockSpec,
    resource: AnchoredResource,
    metadata: MetadataExpectation,
    ownership_epoch: OwnershipEpoch,
    cancellation: Arc<dyn Cancellation + Send + Sync>,
    clock: Arc<dyn Clock + Send + Sync>,
) -> Result<()> {
    let dropped = Arc::new(AtomicBool::new(false));
    let mut cancel_on_drop = CancelOnDrop::new(Arc::clone(&dropped));
    let cancellation = CombinedCancellation {
        caller: cancellation,
        dropped,
    };
    let slot = AcquisitionSlot::reserve(Arc::clone(&locks.inner))?;
    let lock_id = spec.lock_id.clone();
    let result = blocking(move || {
        let mut lock_set = lock_shared(&slot.inner)?;
        lock_set.acquire_with_clock(
            &spec,
            &resource,
            metadata,
            ownership_epoch,
            &cancellation,
            clock.as_ref(),
        )?;
        drop(lock_set);
        Ok(PendingLockAcquisition {
            slot,
            lock_id,
            claimed: false,
        })
    })
    .await;
    match result {
        Ok(pending) => {
            pending.claim();
            cancel_on_drop.disarm();
            Ok(())
        }
        Err(error) => {
            cancel_on_drop.disarm();
            Err(error)
        }
    }
}

pub async fn async_ofd_lock_release(locks: &AsyncLockSet) -> Result<()> {
    let locks = Arc::clone(&locks.inner);
    blocking(move || {
        let mut locks = lock_ready(&locks)?;
        locks.release_last()?;
        Ok(())
    })
    .await
}

pub async fn async_audit_create(
    resource: AnchoredResource,
    metadata: MetadataExpectation,
    stream: AuditStream,
    previous_segment_digest: Digest,
    first_sequence: u64,
) -> Result<AuditAppender> {
    blocking(move || {
        AuditAppender::create(
            resource,
            metadata,
            stream,
            previous_segment_digest,
            first_sequence,
        )
    })
    .await
}

pub async fn async_audit_resume(
    resource: AnchoredResource,
    metadata: MetadataExpectation,
    stream: AuditStream,
    previous_segment_digest: Digest,
    first_sequence: u64,
) -> Result<AuditAppender> {
    blocking(move || {
        AuditAppender::resume(
            resource,
            metadata,
            stream,
            previous_segment_digest,
            first_sequence,
        )
    })
    .await
}

pub async fn async_audit_append(
    mut appender: AuditAppender,
    input: AuditRecordInput,
) -> Result<(AuditAppender, AuditRecord)> {
    blocking(move || {
        let record = appender.append(input)?;
        Ok((appender, record))
    })
    .await
}
