use std::{
    os::fd::OwnedFd,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use d2b_contracts::v2_state::{
    AtomicWriteReceipt, AuditRecord, AuditStream, Digest, LockSpec, OwnershipEpoch,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    AnchoredDir, AnchoredResource, AtomicFilesystem, AtomicWrite, AuditAppender, AuditRecordInput,
    Cancellation, Clock, DurableState, Error, LockSet, MetadataExpectation, QuarantineRecord,
    ReadPolicy, Result, WritePolicy,
};

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
    locks: LockSet,
) -> Result<(AtomicWrite<F>, AtomicWriteReceipt, LockSet)>
where
    F: AtomicFilesystem + Send + 'static,
    T: DeserializeOwned + Serialize + PartialEq + Send + 'static,
{
    blocking(move || {
        let receipt = writer.write(&payload, &policy, locks.last())?;
        Ok((writer, receipt, locks))
    })
    .await
}

pub async fn async_atomic_quarantine<F>(
    mut writer: AtomicWrite<F>,
    record: QuarantineRecord,
    locks: LockSet,
) -> Result<(AtomicWrite<F>, LockSet)>
where
    F: AtomicFilesystem + Send + 'static,
{
    blocking(move || {
        writer.quarantine(&record, locks.last())?;
        Ok((writer, locks))
    })
    .await
}

pub async fn async_ofd_lock_acquire(
    locks: LockSet,
    spec: LockSpec,
    resource: AnchoredResource,
    metadata: MetadataExpectation,
    ownership_epoch: OwnershipEpoch,
    cancellation: Arc<dyn Cancellation + Send + Sync>,
) -> Result<LockSet> {
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
    mut locks: LockSet,
    spec: LockSpec,
    resource: AnchoredResource,
    metadata: MetadataExpectation,
    ownership_epoch: OwnershipEpoch,
    cancellation: Arc<dyn Cancellation + Send + Sync>,
    clock: Arc<dyn Clock + Send + Sync>,
) -> Result<LockSet> {
    let dropped = Arc::new(AtomicBool::new(false));
    let mut cancel_on_drop = CancelOnDrop::new(Arc::clone(&dropped));
    let cancellation = CombinedCancellation {
        caller: cancellation,
        dropped,
    };
    let result = blocking(move || {
        locks.acquire_with_clock(
            &spec,
            &resource,
            metadata,
            ownership_epoch,
            &cancellation,
            clock.as_ref(),
        )?;
        Ok(locks)
    })
    .await;
    cancel_on_drop.disarm();
    result
}

pub async fn async_ofd_lock_release(mut locks: LockSet) -> Result<LockSet> {
    blocking(move || {
        locks.release_last()?;
        Ok(locks)
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
