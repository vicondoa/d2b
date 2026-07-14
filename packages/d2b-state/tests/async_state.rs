#![cfg(feature = "tokio")]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use d2b_contracts::v2_state::{
    AuditActor, AuditEvent, AuditOutcome, AuditReason, AuditStream, AuthorityRef,
    CancellationPolicy, ContentionPolicy, CorrelationId, Digest, FdTransferPolicy, Generation,
    IdentityScope, LockClass, LockKey, LockKind, LockSpec, OwnershipEpoch, ResourceId,
};
use d2b_state::{
    AnchoredDir, AnchoredResource, AtomicFilesystem, AtomicWrite, AuditRecordInput, Cancellation,
    Clock, Error, ErrorCode, GenerationPolicy, LeafName, LockSet, MetadataExpectation,
    NeverCancelled, QuarantineRecord, ReadPolicy, WritePolicy, async_atomic_quarantine,
    async_atomic_read, async_atomic_write, async_audit_append, async_audit_create,
    async_audit_resume, async_ofd_lock_acquire, async_ofd_lock_acquire_with_clock,
    async_ofd_lock_release, async_open_anchored_dir,
};
use serde::{Deserialize, Serialize};

fn generation(value: u64) -> Generation {
    Generation::new(value).unwrap()
}

fn epoch(value: u64) -> OwnershipEpoch {
    OwnershipEpoch::new(value).unwrap()
}

fn resource(value: &str) -> ResourceId {
    ResourceId::parse(value).unwrap()
}

fn zero_digest() -> Digest {
    Digest::parse("0".repeat(64)).unwrap()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    value: u64,
}

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: AtomicBool,
    dropped: AtomicBool,
    mutex: Mutex<()>,
    cv: Condvar,
}

impl Gate {
    fn block(&self) {
        self.entered.store(true, Ordering::Release);
        let mut guard = self.mutex.lock().unwrap();
        while !self.released.load(Ordering::Acquire) {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.cv.notify_all();
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !self.entered.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_dropped(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !self.dropped.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockAt {
    Read,
    Create,
    Quarantine,
}

struct BlockingFs {
    id: ResourceId,
    metadata: MetadataExpectation,
    target: Option<Vec<u8>>,
    temp: Option<Vec<u8>>,
    quarantined: Option<Vec<u8>>,
    block_at: Option<BlockAt>,
    gate: Arc<Gate>,
}

impl Drop for BlockingFs {
    fn drop(&mut self) {
        self.gate.dropped.store(true, Ordering::Release);
    }
}

impl AtomicFilesystem for BlockingFs {
    type Temp = ();

    fn resource_id(&self) -> &ResourceId {
        &self.id
    }

    fn read_target(&mut self, maximum: u64) -> d2b_state::Result<(Vec<u8>, MetadataExpectation)> {
        if self.block_at == Some(BlockAt::Read) {
            self.gate.block();
            self.block_at = None;
        }
        let bytes = self.target.clone().ok_or(Error::Code(ErrorCode::Missing))?;
        if bytes.len() as u64 >= maximum {
            return Err(Error::Code(ErrorCode::TooLarge));
        }
        Ok((bytes, self.metadata))
    }

    fn inspect_target_metadata(&mut self) -> d2b_state::Result<MetadataExpectation> {
        self.target
            .as_ref()
            .map(|_| self.metadata)
            .ok_or(Error::Code(ErrorCode::Missing))
    }

    fn create_temp(&mut self, _metadata: MetadataExpectation) -> d2b_state::Result<Self::Temp> {
        if self.block_at == Some(BlockAt::Create) {
            self.gate.block();
            self.block_at = None;
        }
        self.temp = Some(Vec::new());
        Ok(())
    }

    fn write_temp(&mut self, _temp: &mut Self::Temp, bytes: &[u8]) -> d2b_state::Result<usize> {
        self.temp.as_mut().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn sync_temp(&mut self, _temp: &mut Self::Temp) -> d2b_state::Result<()> {
        Ok(())
    }

    fn rename_temp(&mut self, _temp: &mut Self::Temp) -> d2b_state::Result<()> {
        self.target = self.temp.take();
        Ok(())
    }

    fn sync_parent(&mut self) -> d2b_state::Result<()> {
        Ok(())
    }

    fn remove_temp(&mut self, _temp: &mut Self::Temp) {
        self.temp = None;
    }

    fn quarantine_target(&mut self, _name: &LeafName) -> d2b_state::Result<()> {
        if self.block_at == Some(BlockAt::Quarantine) {
            self.gate.block();
            self.block_at = None;
        }
        self.quarantined = self.target.take();
        Ok(())
    }
}

struct PanicFs {
    id: ResourceId,
}

impl AtomicFilesystem for PanicFs {
    type Temp = ();

    fn resource_id(&self) -> &ResourceId {
        &self.id
    }

    fn read_target(&mut self, _maximum: u64) -> d2b_state::Result<(Vec<u8>, MetadataExpectation)> {
        panic!("sensitive panic detail")
    }

    fn inspect_target_metadata(&mut self) -> d2b_state::Result<MetadataExpectation> {
        unreachable!()
    }

    fn create_temp(&mut self, _metadata: MetadataExpectation) -> d2b_state::Result<Self::Temp> {
        unreachable!()
    }

    fn write_temp(&mut self, _temp: &mut Self::Temp, _bytes: &[u8]) -> d2b_state::Result<usize> {
        unreachable!()
    }

    fn sync_temp(&mut self, _temp: &mut Self::Temp) -> d2b_state::Result<()> {
        unreachable!()
    }

    fn rename_temp(&mut self, _temp: &mut Self::Temp) -> d2b_state::Result<()> {
        unreachable!()
    }

    fn sync_parent(&mut self) -> d2b_state::Result<()> {
        unreachable!()
    }

    fn remove_temp(&mut self, _temp: &mut Self::Temp) {}

    fn quarantine_target(&mut self, _name: &LeafName) -> d2b_state::Result<()> {
        unreachable!()
    }
}

static SCRATCH_ID: AtomicU64 = AtomicU64::new(1);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("d2b-state-async-tests")
            .join(format!(
                "{name}-{}-{}",
                std::process::id(),
                SCRATCH_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn host_metadata(path: &Path, mode: u32) -> MetadataExpectation {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).unwrap();
    MetadataExpectation {
        uid: metadata.uid(),
        gid: metadata.gid(),
        mode,
    }
}

fn fake_metadata() -> MetadataExpectation {
    MetadataExpectation {
        uid: 1000,
        gid: 100,
        mode: 0o600,
    }
}

fn lock_spec(contention: ContentionPolicy) -> LockSpec {
    LockSpec {
        lock_id: resource("state-lock"),
        key: LockKey {
            class: LockClass::LocalRoot,
            scope: IdentityScope::LocalRoot,
            resource_id: resource("state"),
        },
        kind: LockKind::Ofd,
        owner: AuthorityRef::LocalRootBroker,
        release_authority: AuthorityRef::LocalRootBroker,
        global_order: 1,
        acquire_after: Vec::new(),
        cloexec: true,
        fd_transfer: FdTransferPolicy::Never,
        contention,
        deadline_ms: 2_000,
        cancellation: CancellationPolicy::Cancellable,
    }
}

fn lock_resource(anchor: &AnchoredDir) -> AnchoredResource {
    AnchoredResource::new(
        resource("state"),
        anchor,
        LeafName::parse("state.lock").unwrap(),
    )
}

fn write_policy(state: u64, previous: Option<u64>) -> WritePolicy {
    WritePolicy {
        metadata: fake_metadata(),
        writer: AuthorityRef::LocalRootBroker,
        config_generation: generation(7),
        state_generation: generation(state),
        expected_previous: previous.map(generation),
        lock_id: resource("state-lock"),
        ownership_epoch: epoch(1),
    }
}

fn read_policy(state: u64) -> ReadPolicy {
    ReadPolicy {
        metadata: fake_metadata(),
        writer: AuthorityRef::LocalRootBroker,
        config_generation: generation(7),
        state_generation: GenerationPolicy::Exact(generation(state)),
    }
}

fn heartbeat() -> (
    Arc<AtomicBool>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    let ticks = Arc::new(AtomicUsize::new(0));
    let task_stop = Arc::clone(&stop);
    let task_ticks = Arc::clone(&ticks);
    let task = tokio::spawn(async move {
        while !task_stop.load(Ordering::Acquire) {
            task_ticks.fetch_add(1, Ordering::Relaxed);
            tokio::task::yield_now().await;
        }
    });
    (stop, ticks, task)
}

async fn acquire_state_lock(
    anchor: &AnchoredDir,
    metadata: MetadataExpectation,
    contention: ContentionPolicy,
) -> LockSet {
    async_ofd_lock_acquire(
        LockSet::new(),
        lock_spec(contention),
        lock_resource(anchor),
        metadata,
        epoch(1),
        Arc::new(NeverCancelled),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn atomic_adapters_keep_heartbeat_live_and_preserve_guard() {
    let scratch = Scratch::new("atomic-heartbeat");
    let anchor = async_open_anchored_dir(scratch.0.clone()).await.unwrap();
    let locks = acquire_state_lock(
        &anchor,
        host_metadata(&scratch.0, 0o600),
        ContentionPolicy::FailFast,
    )
    .await;
    let gate = Arc::new(Gate::default());
    let filesystem = BlockingFs {
        id: resource("state"),
        metadata: fake_metadata(),
        target: None,
        temp: None,
        quarantined: None,
        block_at: Some(BlockAt::Create),
        gate: Arc::clone(&gate),
    };
    let (stop, ticks, beat) = heartbeat();
    let task = tokio::spawn(async_atomic_write(
        AtomicWrite::new(filesystem),
        Payload { value: 1 },
        write_policy(1, None),
        locks,
    ));
    gate.wait_entered().await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let write_ticks = ticks.load(Ordering::Relaxed);
    gate.release();
    assert!(write_ticks > 0);
    let (writer, receipt, locks) = task.await.unwrap().unwrap();
    assert!(receipt.success);
    assert!(locks.last().unwrap().is_held());

    let mut filesystem = writer.into_inner();
    let read_gate = Arc::new(Gate::default());
    filesystem.gate = Arc::clone(&read_gate);
    filesystem.block_at = Some(BlockAt::Read);
    let read_task = tokio::spawn(async_atomic_read::<_, Payload>(
        AtomicWrite::new(filesystem),
        read_policy(1),
    ));
    read_gate.wait_entered().await;
    let before_read = ticks.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(10)).await;
    let after_read = ticks.load(Ordering::Relaxed);
    read_gate.release();
    assert!(after_read > before_read);
    let (writer, state) = read_task.await.unwrap().unwrap();
    assert_eq!(state.payload, Payload { value: 1 });
    let record = QuarantineRecord::for_error(
        resource("state"),
        resource("state-lock"),
        AuthorityRef::LocalRootBroker,
        epoch(1),
        &Error::Code(ErrorCode::ChecksumMismatch),
        None,
    );
    let mut filesystem = writer.into_inner();
    let quarantine_gate = Arc::new(Gate::default());
    filesystem.gate = Arc::clone(&quarantine_gate);
    filesystem.block_at = Some(BlockAt::Quarantine);
    let quarantine_task = tokio::spawn(async_atomic_quarantine(
        AtomicWrite::new(filesystem),
        record,
        locks,
    ));
    quarantine_gate.wait_entered().await;
    let before_quarantine = ticks.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(10)).await;
    let after_quarantine = ticks.load(Ordering::Relaxed);
    quarantine_gate.release();
    assert!(after_quarantine > before_quarantine);
    let (_writer, locks) = quarantine_task.await.unwrap().unwrap();
    assert!(locks.last().unwrap().is_held());
    async_ofd_lock_release(locks).await.unwrap();
    stop.store(true, Ordering::Release);
    beat.await.unwrap();
}

struct BlockingClock {
    gate: Arc<Gate>,
    started: Instant,
}

impl Clock for BlockingClock {
    fn now(&self) -> Instant {
        self.started
    }

    fn sleep(&self, _duration: Duration) {
        self.gate.block();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn lock_adapter_keeps_current_thread_runtime_live_with_blocking_clock() {
    let scratch = Scratch::new("lock-heartbeat");
    let anchor = async_open_anchored_dir(scratch.0.clone()).await.unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let holder = acquire_state_lock(&anchor, metadata, ContentionPolicy::FailFast).await;
    let gate = Arc::new(Gate::default());
    let clock: Arc<dyn Clock + Send + Sync> = Arc::new(BlockingClock {
        gate: Arc::clone(&gate),
        started: Instant::now(),
    });
    let (stop, ticks, beat) = heartbeat();
    let waiter = tokio::spawn(async_ofd_lock_acquire_with_clock(
        LockSet::new(),
        lock_spec(ContentionPolicy::BoundedWait),
        lock_resource(&anchor),
        metadata,
        epoch(1),
        Arc::new(NeverCancelled),
        clock,
    ));
    gate.wait_entered().await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let observed_ticks = ticks.load(Ordering::Relaxed);
    async_ofd_lock_release(holder).await.unwrap();
    gate.release();
    assert!(observed_ticks > 0);
    let waiter = waiter.await.unwrap().unwrap();
    async_ofd_lock_release(waiter).await.unwrap();
    stop.store(true, Ordering::Release);
    beat.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborted_atomic_future_finishes_critical_section_then_releases_lock() {
    let scratch = Scratch::new("abort-cleanup");
    let anchor = async_open_anchored_dir(scratch.0.clone()).await.unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let locks = acquire_state_lock(&anchor, metadata, ContentionPolicy::FailFast).await;
    let gate = Arc::new(Gate::default());
    let filesystem = BlockingFs {
        id: resource("state"),
        metadata: fake_metadata(),
        target: None,
        temp: None,
        quarantined: None,
        block_at: Some(BlockAt::Create),
        gate: Arc::clone(&gate),
    };
    let task = tokio::spawn(async_atomic_write(
        AtomicWrite::new(filesystem),
        Payload { value: 1 },
        write_policy(1, None),
        locks,
    ));
    gate.wait_entered().await;
    task.abort();
    let join_error = match task.await {
        Ok(_) => panic!("aborted task unexpectedly completed"),
        Err(error) => error,
    };
    assert!(join_error.is_cancelled());

    let contended = async_ofd_lock_acquire(
        LockSet::new(),
        lock_spec(ContentionPolicy::FailFast),
        lock_resource(&anchor),
        metadata,
        epoch(1),
        Arc::new(NeverCancelled),
    )
    .await;

    gate.release();
    gate.wait_dropped().await;
    let contended = match contended {
        Ok(locks) => {
            async_ofd_lock_release(locks).await.unwrap();
            panic!("competing lock acquired while critical section was blocked")
        }
        Err(error) => error,
    };
    assert_eq!(contended.code(), ErrorCode::LockContended);
    let reacquired = acquire_state_lock(&anchor, metadata, ContentionPolicy::FailFast).await;
    async_ofd_lock_release(reacquired).await.unwrap();
}

struct Cancelled;

impl Cancellation for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_lock_wait_cleans_up_without_acquiring() {
    let scratch = Scratch::new("cancel-lock");
    let anchor = async_open_anchored_dir(scratch.0.clone()).await.unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let holder = acquire_state_lock(&anchor, metadata, ContentionPolicy::FailFast).await;
    let error = async_ofd_lock_acquire(
        LockSet::new(),
        lock_spec(ContentionPolicy::BoundedWait),
        lock_resource(&anchor),
        metadata,
        epoch(1),
        Arc::new(Cancelled),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    async_ofd_lock_release(holder).await.unwrap();
    let reacquired = acquire_state_lock(&anchor, metadata, ContentionPolicy::FailFast).await;
    async_ofd_lock_release(reacquired).await.unwrap();
}

fn audit_input(sequence: u64) -> AuditRecordInput {
    AuditRecordInput {
        stream: AuditStream::LocalRoot,
        sequence,
        occurred_at_unix_ms: 100 + sequence,
        operation_id: CorrelationId::parse(format!("op-{sequence}")).unwrap(),
        session_id: None,
        provider_id: None,
        actor: AuditActor::LocalRootBroker,
        event: AuditEvent::StorageReconcile,
        outcome: AuditOutcome::Succeeded,
        reason: AuditReason::PolicyAllowed,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn audit_create_append_and_resume_are_offloaded() {
    let scratch = Scratch::new("audit");
    let anchor = async_open_anchored_dir(scratch.0.clone()).await.unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let audit_resource = AnchoredResource::new(
        resource("audit"),
        &anchor,
        LeafName::parse("audit.jsonl").unwrap(),
    );
    let appender = async_audit_create(
        audit_resource,
        metadata,
        AuditStream::LocalRoot,
        zero_digest(),
        1,
    )
    .await
    .unwrap();
    let (appender, _) = async_audit_append(appender, audit_input(1)).await.unwrap();
    let (appender, _) = async_audit_append(appender, audit_input(2)).await.unwrap();
    drop(appender);
    let audit_resource = AnchoredResource::new(
        resource("audit"),
        &anchor,
        LeafName::parse("audit.jsonl").unwrap(),
    );
    let appender = async_audit_resume(
        audit_resource,
        metadata,
        AuditStream::LocalRoot,
        zero_digest(),
        1,
    )
    .await
    .unwrap();
    let (appender, record) = async_audit_append(appender, audit_input(3)).await.unwrap();
    assert_eq!(record.sequence, 3);
    assert_eq!(appender.record_count(), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn join_failures_are_typed_and_redacted() {
    let result = async_atomic_read::<_, Payload>(
        AtomicWrite::new(PanicFs {
            id: resource("state"),
        }),
        read_policy(1),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("panicking filesystem unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::TaskJoin);
    assert!(!format!("{error:?}").contains("sensitive"));
}
