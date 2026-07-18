use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use d2b_contracts::v2_state::{
    AuditActor, AuditCheckpointSignatureVerifier, AuditEvent, AuditOutcome, AuditOwner,
    AuditReason, AuditRetentionDecision, AuditRetentionEvidence, AuditRetentionPolicy, AuditStream,
    AuthorityRef, CancellationPolicy, ContentionPolicy, Digest, FdTransferPolicy, Generation,
    IdentityScope, LeaseRevocation, LockClass, LockKey, LockKind, LockSpec,
    MAX_AUDIT_RECORDS_PER_SEGMENT, MAX_AUDIT_SEGMENT_BYTES, MAX_JSON_DOCUMENT_BYTES,
    OwnershipEpoch, PruneStatus, ResourceId, STATE_SCHEMA_GENERATION, STATE_SCHEMA_VERSION,
    StateEnvelope, state_payload_digest,
};
use d2b_state::{
    AnchoredDir, AnchoredResource, AtomicFilesystem, AtomicWrite, AuditAppender, AuditRecordInput,
    CanonicalJson, Error, ErrorCode, GenerationPolicy, LeafName, LockSet, MetadataExpectation,
    NeverCancelled, QuarantineRecord, ReadPolicy, RealAtomicFilesystem, RelativePath,
    SegmentBuilder, WritePolicy, checkpoint, decide_retention, detect_gap, grant_lease,
    read_audit_segment, revoke_lease, validate_lease,
};
use rustix::io::fcntl_dupfd_cloexec;
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

fn metadata() -> MetadataExpectation {
    MetadataExpectation {
        uid: 1000,
        gid: 100,
        mode: 0o640,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LargePayload {
    padding: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Create,
    Write,
    FileSync,
    Rename,
    ParentSync,
    Cleanup,
    Quarantine,
}

#[derive(Debug, Clone)]
struct FakeFs {
    id: ResourceId,
    target: Option<Vec<u8>>,
    target_metadata: MetadataExpectation,
    temp: Option<Vec<u8>>,
    events: Vec<Event>,
    fail: Option<Event>,
    write_limit: usize,
    quarantine: Option<Vec<u8>>,
}

impl FakeFs {
    fn empty() -> Self {
        Self {
            id: resource("state"),
            target: None,
            target_metadata: metadata(),
            temp: None,
            events: Vec::new(),
            fail: None,
            write_limit: usize::MAX,
            quarantine: None,
        }
    }

    fn step(&mut self, event: Event) -> d2b_state::Result<()> {
        self.events.push(event);
        if self.fail == Some(event) {
            Err(Error::Code(ErrorCode::Io))
        } else {
            Ok(())
        }
    }
}

impl AtomicFilesystem for FakeFs {
    type Temp = ();

    fn resource_id(&self) -> &ResourceId {
        &self.id
    }

    fn read_target(&mut self, maximum: u64) -> d2b_state::Result<(Vec<u8>, MetadataExpectation)> {
        let bytes = self.target.clone().ok_or(Error::Code(ErrorCode::Missing))?;
        if bytes.len() as u64 >= maximum {
            return Err(Error::Code(ErrorCode::TooLarge));
        }
        Ok((bytes, self.target_metadata))
    }

    fn inspect_target_metadata(&mut self) -> d2b_state::Result<MetadataExpectation> {
        self.target
            .as_ref()
            .map(|_| self.target_metadata)
            .ok_or(Error::Code(ErrorCode::Missing))
    }

    fn create_temp(&mut self, _metadata: MetadataExpectation) -> d2b_state::Result<Self::Temp> {
        self.step(Event::Create)?;
        self.temp = Some(Vec::new());
        Ok(())
    }

    fn write_temp(&mut self, _temp: &mut Self::Temp, bytes: &[u8]) -> d2b_state::Result<usize> {
        self.step(Event::Write)?;
        let count = bytes.len().min(self.write_limit);
        self.temp
            .as_mut()
            .unwrap()
            .extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn sync_temp(&mut self, _temp: &mut Self::Temp) -> d2b_state::Result<()> {
        self.step(Event::FileSync)
    }

    fn rename_temp(&mut self, _temp: &mut Self::Temp) -> d2b_state::Result<()> {
        self.step(Event::Rename)?;
        self.target = self.temp.take();
        Ok(())
    }

    fn sync_parent(&mut self) -> d2b_state::Result<()> {
        self.step(Event::ParentSync)
    }

    fn remove_temp(&mut self, _temp: &mut Self::Temp) {
        self.events.push(Event::Cleanup);
        self.temp = None;
    }

    fn quarantine_target(&mut self, _name: &LeafName) -> d2b_state::Result<()> {
        self.step(Event::Quarantine)?;
        self.quarantine = self.target.take();
        Ok(())
    }
}

fn write_policy(state: u64, previous: Option<u64>) -> WritePolicy {
    WritePolicy {
        metadata: metadata(),
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
        metadata: metadata(),
        writer: AuthorityRef::LocalRootBroker,
        config_generation: generation(7),
        state_generation: GenerationPolicy::Exact(generation(state)),
    }
}

fn seeded_fake() -> FakeFs {
    let lock = held_state_lock("state", "state-lock");
    let mut writer = AtomicWrite::new(FakeFs::empty());
    writer
        .write(
            &Payload { value: 1 },
            &write_policy(1, None),
            Some(lock.guard()),
        )
        .unwrap();
    let mut fake = writer.into_inner();
    fake.events.clear();
    fake
}

#[test]
fn atomic_write_orders_all_durability_steps_and_completes_partial_writes() {
    let lock = held_state_lock("state", "state-lock");
    let mut fake = FakeFs::empty();
    fake.write_limit = 7;
    let mut writer = AtomicWrite::new(fake);
    let receipt = writer
        .write(
            &Payload { value: 1 },
            &write_policy(1, None),
            Some(lock.guard()),
        )
        .unwrap();
    assert!(receipt.success);
    assert_eq!(receipt.resource_id, resource("state"));
    let fake = writer.into_inner();
    assert_eq!(fake.events.first(), Some(&Event::Create));
    assert!(
        fake.events
            .iter()
            .filter(|event| **event == Event::Write)
            .count()
            > 1
    );
    let tail = &fake.events[fake.events.len() - 3..];
    assert_eq!(tail, [Event::FileSync, Event::Rename, Event::ParentSync]);
}

#[test]
fn every_crash_phase_fails_closed_and_never_reports_success_early() {
    for failure in [
        Event::Create,
        Event::Write,
        Event::FileSync,
        Event::Rename,
        Event::ParentSync,
    ] {
        let baseline = seeded_fake();
        let prior = baseline.target.clone();
        let mut fake = baseline;
        fake.fail = Some(failure);
        let lock = held_state_lock("state", "state-lock");
        let mut writer = AtomicWrite::new(fake);
        let error = writer
            .write(
                &Payload { value: 2 },
                &write_policy(2, Some(1)),
                Some(lock.guard()),
            )
            .unwrap_err();
        if failure == Event::ParentSync {
            assert_eq!(error.code(), ErrorCode::QuarantineRequired);
        }
        let fake = writer.into_inner();
        if failure == Event::ParentSync {
            assert_ne!(fake.target, prior);
        } else {
            assert_eq!(fake.target, prior);
        }
        if failure == Event::Create {
            assert_eq!(fake.events.last(), Some(&Event::Create));
        } else {
            assert_eq!(fake.events.last(), Some(&Event::Cleanup));
        }
    }
}

#[test]
fn bounded_canonical_read_rejects_missing_corrupt_noncanonical_and_unknown_state() {
    let baseline = seeded_fake();
    let mut writer = AtomicWrite::new(baseline.clone());
    assert_eq!(
        writer.read::<Payload>(&read_policy(1)).unwrap().payload,
        Payload { value: 1 }
    );

    let mut missing = AtomicWrite::new(FakeFs::empty());
    assert_eq!(
        missing.read::<Payload>(&read_policy(1)).unwrap_err().code(),
        ErrorCode::Missing
    );

    let mut whitespace = baseline.clone();
    whitespace.target.as_mut().unwrap().push(b' ');
    assert_eq!(
        AtomicWrite::new(whitespace)
            .read::<Payload>(&read_policy(1))
            .unwrap_err()
            .code(),
        ErrorCode::NonCanonical
    );

    let mut checksum = baseline.clone();
    let bytes = checksum.target.as_mut().unwrap();
    let offset = bytes
        .windows(b"\"value\":1".len())
        .position(|window| window == b"\"value\":1")
        .unwrap();
    bytes[offset + b"\"value\":".len()] = b'2';
    assert_eq!(
        AtomicWrite::new(checksum)
            .read::<Payload>(&read_policy(1))
            .unwrap_err()
            .code(),
        ErrorCode::ChecksumMismatch
    );

    let mut unknown = baseline;
    let bytes = unknown.target.take().unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), serde_json::Value::Bool(true));
    unknown.target = Some(serde_json::to_vec(&value).unwrap());
    assert_eq!(
        AtomicWrite::new(unknown)
            .read::<Payload>(&read_policy(1))
            .unwrap_err()
            .code(),
        ErrorCode::NonCanonical
    );
}

#[test]
fn bounded_read_rejects_canonical_state_document_over_maximum() {
    let empty_payload_len = serde_json::to_vec(&LargePayload {
        padding: String::new(),
    })
    .unwrap()
    .len();
    let padding_len = usize::try_from(MAX_JSON_DOCUMENT_BYTES).unwrap() - empty_payload_len - 1;
    let payload = LargePayload {
        padding: "x".repeat(padding_len),
    };
    let payload_bytes = serde_json::to_vec(&payload).unwrap();
    assert_eq!(payload_bytes.len() as u64, MAX_JSON_DOCUMENT_BYTES - 1);

    let envelope = StateEnvelope {
        schema_version: STATE_SCHEMA_VERSION,
        schema_generation: STATE_SCHEMA_GENERATION,
        config_generation: generation(7),
        state_generation: generation(1),
        writer: AuthorityRef::LocalRootBroker,
        encoded_bytes: payload_bytes.len() as u64,
        checksum: state_payload_digest(&payload_bytes).unwrap(),
        payload,
    };
    envelope
        .validate_payload_bytes(&payload_bytes, &CanonicalJson)
        .unwrap();
    let document = serde_json::to_vec(&envelope).unwrap();
    let decoded: StateEnvelope<LargePayload> = serde_json::from_slice(&document).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), document);
    assert!(document.len() as u64 > MAX_JSON_DOCUMENT_BYTES);

    let mut fake = FakeFs::empty();
    fake.target = Some(document);
    assert_eq!(
        AtomicWrite::new(fake)
            .read::<LargePayload>(&read_policy(1))
            .unwrap_err()
            .code(),
        ErrorCode::TooLarge
    );
}

#[test]
fn writer_metadata_and_generation_are_closed_and_monotonic() {
    let baseline = seeded_fake();
    let mut wrong_writer = read_policy(1);
    wrong_writer.writer = AuthorityRef::LocalRootAllocator;
    assert_eq!(
        AtomicWrite::new(baseline.clone())
            .read::<Payload>(&wrong_writer)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidWriter
    );

    let mut wrong_metadata = read_policy(1);
    wrong_metadata.metadata.mode = 0o600;
    assert_eq!(
        AtomicWrite::new(baseline.clone())
            .read::<Payload>(&wrong_metadata)
            .unwrap_err()
            .code(),
        ErrorCode::MetadataMismatch
    );
    let mut wrong_owner = read_policy(1);
    wrong_owner.metadata.uid += 1;
    assert_eq!(
        AtomicWrite::new(baseline.clone())
            .read::<Payload>(&wrong_owner)
            .unwrap_err()
            .code(),
        ErrorCode::MetadataMismatch
    );

    assert_eq!(
        AtomicWrite::new(baseline.clone())
            .write(
                &Payload { value: 2 },
                &write_policy(1, Some(1)),
                Some(held_state_lock("state", "state-lock").guard()),
            )
            .unwrap_err()
            .code(),
        ErrorCode::GenerationGap
    );
    assert_eq!(
        AtomicWrite::new(baseline)
            .write(
                &Payload { value: 3 },
                &write_policy(3, Some(1)),
                Some(held_state_lock("state", "state-lock").guard()),
            )
            .unwrap_err()
            .code(),
        ErrorCode::GenerationGap
    );
}

#[test]
fn corrupt_state_has_typed_quarantine_and_is_moved_narrowly() {
    let baseline = seeded_fake();
    let error = Error::Code(ErrorCode::ChecksumMismatch);
    let record = QuarantineRecord::for_error(
        resource("state"),
        resource("state-lock"),
        AuthorityRef::LocalRootBroker,
        epoch(1),
        &error,
        baseline.target.as_deref(),
    );
    assert_eq!(
        record.reason,
        d2b_contracts::v2_state::QuarantineReason::CorruptState
    );
    assert!(record.observed_document_digest.is_some());

    let mut writer = AtomicWrite::new(baseline);
    assert_eq!(
        writer.quarantine(&record, None).unwrap_err().code(),
        ErrorCode::LockRequired
    );
    let wrong = held_state_lock("other-state", "other-lock");
    assert_eq!(
        writer
            .quarantine(&record, Some(wrong.guard()))
            .unwrap_err()
            .code(),
        ErrorCode::LockMismatch
    );
    let correct = held_state_lock("state", "state-lock");
    let mut stale_record = record.clone();
    stale_record.ownership_epoch = epoch(2);
    assert_eq!(
        writer
            .quarantine(&stale_record, Some(correct.guard()))
            .unwrap_err()
            .code(),
        ErrorCode::LockMismatch
    );
    let mut released = held_state_lock("state", "state-lock");
    released.set.last_mut().unwrap().release_in_place().unwrap();
    assert_eq!(
        writer
            .quarantine(&record, Some(released.guard()))
            .unwrap_err()
            .code(),
        ErrorCode::LockReleased
    );
    let lock = held_state_lock("state", "state-lock");
    writer.quarantine(&record, Some(lock.guard())).unwrap();
    let fake = writer.into_inner();
    assert!(fake.target.is_none());
    assert!(fake.quarantine.is_some());
    assert_eq!(
        &fake.events[fake.events.len() - 2..],
        [Event::Quarantine, Event::ParentSync]
    );
}

#[test]
fn generation_sensitive_writes_reject_missing_wrong_and_released_guards() {
    let baseline = seeded_fake();
    let prior = baseline.target.clone();
    let mut writer = AtomicWrite::new(baseline);
    assert_eq!(
        writer
            .write(&Payload { value: 2 }, &write_policy(2, Some(1)), None)
            .unwrap_err()
            .code(),
        ErrorCode::LockRequired
    );
    let wrong = held_state_lock("other-state", "other-lock");
    assert_eq!(
        writer
            .write(
                &Payload { value: 2 },
                &write_policy(2, Some(1)),
                Some(wrong.guard()),
            )
            .unwrap_err()
            .code(),
        ErrorCode::LockMismatch
    );
    let correct = held_state_lock("state", "state-lock");
    let mut stale_epoch = write_policy(2, Some(1));
    stale_epoch.ownership_epoch = epoch(2);
    assert_eq!(
        writer
            .write(&Payload { value: 2 }, &stale_epoch, Some(correct.guard()),)
            .unwrap_err()
            .code(),
        ErrorCode::LockMismatch
    );
    let mut released = held_state_lock("state", "state-lock");
    released.set.last_mut().unwrap().release_in_place().unwrap();
    assert_eq!(
        writer
            .write(
                &Payload { value: 2 },
                &write_policy(2, Some(1)),
                Some(released.guard()),
            )
            .unwrap_err()
            .code(),
        ErrorCode::LockReleased
    );
    assert_eq!(writer.into_inner().target, prior);
}

static SCRATCH_ID: AtomicU64 = AtomicU64::new(1);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("d2b-state-tests")
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
    let parent = fs::metadata(path).unwrap();
    MetadataExpectation {
        uid: parent.uid(),
        gid: parent.gid(),
        mode,
    }
}

#[test]
fn real_io_is_anchored_nofollow_and_checks_exact_mode() {
    let scratch = Scratch::new("path");
    let outside = scratch.0.join("outside");
    fs::write(&outside, b"secret").unwrap();
    symlink(&outside, scratch.0.join("state.json")).unwrap();
    let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let state_resource = AnchoredResource::new(
        resource("state"),
        &anchor,
        LeafName::parse("state.json").unwrap(),
    );
    let mut writer = AtomicWrite::new(RealAtomicFilesystem::new(state_resource));
    let policy = ReadPolicy {
        metadata: host_metadata(&scratch.0, 0o600),
        ..read_policy(1)
    };
    assert_eq!(
        writer.read::<Payload>(&policy).unwrap_err().code(),
        ErrorCode::PathRejected
    );
    assert!(RelativePath::from_components(["..", "escape"]).is_err());

    fs::remove_file(scratch.0.join("state.json")).unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let resource = AnchoredResource::new(
        resource("state"),
        &anchor,
        LeafName::parse("state.json").unwrap(),
    );
    let mut real = AtomicWrite::new(RealAtomicFilesystem::new(resource));
    let policy = WritePolicy {
        metadata,
        ..write_policy(1, None)
    };
    let lock = held_state_lock("state", "state-lock");
    real.write(&Payload { value: 9 }, &policy, Some(lock.guard()))
        .unwrap();
    fs::set_permissions(
        scratch.0.join("state.json"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let read = ReadPolicy {
        metadata,
        ..read_policy(1)
    };
    assert_eq!(
        real.read::<Payload>(&read).unwrap_err().code(),
        ErrorCode::MetadataMismatch
    );
}

#[test]
fn real_quarantine_moves_only_the_anchored_resource_and_syncs_both_directories() {
    let scratch = Scratch::new("quarantine");
    let quarantine_path = scratch.0.join("quarantine");
    fs::create_dir(&quarantine_path).unwrap();
    let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let quarantine_anchor = AnchoredDir::open_trusted(&quarantine_path).unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let state_resource = AnchoredResource::new(
        resource("state"),
        &anchor,
        LeafName::parse("state.json").unwrap(),
    );
    let mut writer = AtomicWrite::new(RealAtomicFilesystem::with_quarantine(
        state_resource,
        &quarantine_anchor,
    ));
    let lock = held_state_lock("state", "state-lock");
    writer
        .write(
            &Payload { value: 1 },
            &WritePolicy {
                metadata,
                ..write_policy(1, None)
            },
            Some(lock.guard()),
        )
        .unwrap();
    let mut bytes = fs::read(scratch.0.join("state.json")).unwrap();
    let offset = bytes
        .windows(b"\"value\":1".len())
        .position(|window| window == b"\"value\":1")
        .unwrap();
    bytes[offset + b"\"value\":".len()] = b'2';
    fs::write(scratch.0.join("state.json"), &bytes).unwrap();
    let error = writer
        .read::<Payload>(&ReadPolicy {
            metadata,
            ..read_policy(1)
        })
        .unwrap_err();
    let record = QuarantineRecord::for_error(
        resource("state"),
        resource("state-lock"),
        AuthorityRef::LocalRootBroker,
        epoch(1),
        &error,
        Some(&bytes),
    );
    writer.quarantine(&record, Some(lock.guard())).unwrap();
    assert!(!scratch.0.join("state.json").exists());
    assert_eq!(fs::read_dir(&quarantine_path).unwrap().count(), 1);
}

fn lock_spec(
    id: &str,
    resource_id: &str,
    order: u32,
    dependencies: &[&str],
    contention: ContentionPolicy,
    transfer: FdTransferPolicy,
) -> LockSpec {
    LockSpec {
        lock_id: resource(id),
        key: LockKey {
            class: LockClass::LocalRoot,
            scope: IdentityScope::LocalRoot,
            resource_id: resource(resource_id),
        },
        kind: LockKind::Ofd,
        owner: AuthorityRef::LocalRootBroker,
        release_authority: AuthorityRef::LocalRootBroker,
        global_order: order,
        acquire_after: dependencies.iter().map(|id| resource(id)).collect(),
        cloexec: true,
        fd_transfer: transfer,
        contention,
        deadline_ms: 5,
        cancellation: CancellationPolicy::Cancellable,
    }
}

struct HeldLock {
    set: LockSet,
    _scratch: Scratch,
}

impl HeldLock {
    fn guard(&self) -> &d2b_state::LockGuard {
        self.set.last().unwrap()
    }
}

fn held_state_lock(resource_id: &str, lock_id: &str) -> HeldLock {
    let scratch = Scratch::new("state-lock");
    let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let spec = lock_spec(
        lock_id,
        resource_id,
        1,
        &[],
        ContentionPolicy::FailFast,
        FdTransferPolicy::Never,
    );
    let lock_resource = AnchoredResource::new(
        resource(resource_id),
        &anchor,
        LeafName::parse("state.lock").unwrap(),
    );
    let mut set = LockSet::new();
    set.acquire(&spec, &lock_resource, metadata, epoch(1), &NeverCancelled)
        .unwrap();
    HeldLock {
        set,
        _scratch: scratch,
    }
}

#[test]
fn ofd_locks_enforce_order_contention_deadline_cancellation_and_transfer() {
    let scratch = Scratch::new("locks");
    let anchor_a = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let anchor_b = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let first_spec = lock_spec(
        "first-lock",
        "first-resource",
        10,
        &[],
        ContentionPolicy::FailFast,
        FdTransferPolicy::Never,
    );
    let first_a = AnchoredResource::new(
        resource("first-resource"),
        &anchor_a,
        LeafName::parse("first.lock").unwrap(),
    );
    let first_b = AnchoredResource::new(
        resource("first-resource"),
        &anchor_b,
        LeafName::parse("first.lock").unwrap(),
    );
    let mut set_a = LockSet::new();
    let guard = set_a
        .acquire(&first_spec, &first_a, metadata, epoch(1), &NeverCancelled)
        .unwrap();
    assert_eq!(
        guard
            .authorize_transfer(FdTransferPolicy::ComponentSessionAttachment)
            .unwrap_err()
            .code(),
        ErrorCode::TransferDenied
    );
    let mut set_b = LockSet::new();
    assert_eq!(
        set_b
            .acquire(&first_spec, &first_b, metadata, epoch(1), &NeverCancelled,)
            .unwrap_err()
            .code(),
        ErrorCode::LockContended
    );

    struct Cancelled;
    impl d2b_state::Cancellation for Cancelled {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    let mut waiting_spec = first_spec.clone();
    waiting_spec.contention = ContentionPolicy::BoundedWait;
    assert_eq!(
        set_b
            .acquire(&waiting_spec, &first_b, metadata, epoch(1), &Cancelled,)
            .unwrap_err()
            .code(),
        ErrorCode::Cancelled
    );
    assert_eq!(
        set_b
            .acquire(&waiting_spec, &first_b, metadata, epoch(1), &NeverCancelled,)
            .unwrap_err()
            .code(),
        ErrorCode::Deadline
    );

    let second_spec = lock_spec(
        "second-lock",
        "second-resource",
        20,
        &["first-lock"],
        ContentionPolicy::FailFast,
        FdTransferPolicy::ComponentSessionAttachment,
    );
    let second = AnchoredResource::new(
        resource("second-resource"),
        &anchor_a,
        LeafName::parse("second.lock").unwrap(),
    );
    let second_guard = set_a
        .acquire(&second_spec, &second, metadata, epoch(1), &NeverCancelled)
        .unwrap();
    let transfer = second_guard
        .authorize_transfer(FdTransferPolicy::ComponentSessionAttachment)
        .unwrap();
    let transferred_fd = fcntl_dupfd_cloexec(transfer.fd(), 0).unwrap();
    transfer.commit();
    set_a.release_last().unwrap();

    let second_b = AnchoredResource::new(
        resource("second-resource"),
        &anchor_b,
        LeafName::parse("second.lock").unwrap(),
    );
    let mut observer_spec = second_spec.clone();
    observer_spec.acquire_after.clear();
    let mut observer = LockSet::new();
    assert_eq!(
        observer
            .acquire(
                &observer_spec,
                &second_b,
                metadata,
                epoch(1),
                &NeverCancelled,
            )
            .unwrap_err()
            .code(),
        ErrorCode::LockContended
    );
    drop(transferred_fd);
    observer
        .acquire(
            &observer_spec,
            &second_b,
            metadata,
            epoch(1),
            &NeverCancelled,
        )
        .unwrap();
    observer.release_last().unwrap();

    let third_spec = lock_spec(
        "third-lock",
        "third-resource",
        5,
        &[],
        ContentionPolicy::FailFast,
        FdTransferPolicy::Never,
    );
    let third = AnchoredResource::new(
        resource("third-resource"),
        &anchor_a,
        LeafName::parse("third.lock").unwrap(),
    );
    assert_eq!(
        set_a
            .acquire(&third_spec, &third, metadata, epoch(1), &NeverCancelled,)
            .unwrap_err()
            .code(),
        ErrorCode::LockOrder
    );
}

#[test]
fn two_generation_writers_serialize_and_only_one_can_commit_next_generation() {
    let scratch = Scratch::new("writer-race");
    let metadata = host_metadata(&scratch.0, 0o600);
    let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let mut spec = lock_spec(
        "state-lock",
        "state",
        1,
        &[],
        ContentionPolicy::BoundedWait,
        FdTransferPolicy::Never,
    );
    spec.deadline_ms = 10_000;
    let lock_resource = AnchoredResource::new(
        resource("state"),
        &anchor,
        LeafName::parse("state.lock").unwrap(),
    );
    let mut initial_lock = LockSet::new();
    initial_lock
        .acquire(&spec, &lock_resource, metadata, epoch(1), &NeverCancelled)
        .unwrap();
    let state_resource = AnchoredResource::new(
        resource("state"),
        &anchor,
        LeafName::parse("state.json").unwrap(),
    );
    AtomicWrite::new(RealAtomicFilesystem::new(state_resource))
        .write(
            &Payload { value: 1 },
            &WritePolicy {
                metadata,
                ..write_policy(1, None)
            },
            initial_lock.last(),
        )
        .unwrap();
    initial_lock.release_last().unwrap();

    let root = Arc::new(scratch.0.clone());
    let barrier = Arc::new(Barrier::new(2));
    let handles = [10_u64, 20_u64].map(|value| {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let anchor = AnchoredDir::open_trusted(&root).unwrap();
            let metadata = host_metadata(&root, 0o600);
            let mut spec = lock_spec(
                "state-lock",
                "state",
                1,
                &[],
                ContentionPolicy::BoundedWait,
                FdTransferPolicy::Never,
            );
            spec.deadline_ms = 2_000;
            let lock_resource = AnchoredResource::new(
                resource("state"),
                &anchor,
                LeafName::parse("state.lock").unwrap(),
            );
            let mut locks = LockSet::new();
            barrier.wait();
            locks
                .acquire(&spec, &lock_resource, metadata, epoch(1), &NeverCancelled)
                .unwrap();
            let state_resource = AnchoredResource::new(
                resource("state"),
                &anchor,
                LeafName::parse("state.json").unwrap(),
            );
            let result = AtomicWrite::new(RealAtomicFilesystem::new(state_resource)).write(
                &Payload { value },
                &WritePolicy {
                    metadata,
                    ..write_policy(2, Some(1))
                },
                locks.last(),
            );
            locks.release_last().unwrap();
            (value, result.map(|receipt| receipt.generation))
        })
    });
    let results = handles.map(|handle| handle.join().unwrap());
    assert_eq!(
        results.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter_map(|(_, result)| result.as_ref().err())
            .map(Error::code)
            .collect::<Vec<_>>(),
        vec![ErrorCode::GenerationRollback]
    );
    let successful_value = results
        .iter()
        .find_map(|(value, result)| result.is_ok().then_some(*value))
        .unwrap();

    let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let state_resource = AnchoredResource::new(
        resource("state"),
        &anchor,
        LeafName::parse("state.json").unwrap(),
    );
    let mut reader = AtomicWrite::new(RealAtomicFilesystem::new(state_resource));
    let committed = reader
        .read::<Payload>(&ReadPolicy {
            metadata,
            writer: AuthorityRef::LocalRootBroker,
            config_generation: generation(7),
            state_generation: GenerationPolicy::Exact(generation(2)),
        })
        .unwrap();
    assert_eq!(committed.payload.value, successful_value);
}

fn audit_input(sequence: u64) -> AuditRecordInput {
    AuditRecordInput {
        stream: AuditStream::LocalRoot,
        sequence,
        occurred_at_unix_ms: 100 + sequence,
        operation_id: d2b_contracts::v2_state::CorrelationId::parse(format!("op-{sequence}"))
            .unwrap(),
        session_id: None,
        provider_id: None,
        actor: AuditActor::LocalRootBroker,
        event: AuditEvent::StorageReconcile,
        outcome: AuditOutcome::Succeeded,
        reason: AuditReason::PolicyAllowed,
    }
}

struct NoSignatures;

impl AuditCheckpointSignatureVerifier for NoSignatures {
    fn verify_realm_signature(
        &self,
        _realm_id: &d2b_contracts::v2_identity::RealmId,
        _checkpoint_digest: &Digest,
        _signature_digest: &Digest,
    ) -> bool {
        false
    }
}

#[test]
fn audit_records_segments_checkpoints_gaps_and_retention_are_chained() {
    let first = audit_input(1).build(zero_digest()).unwrap();
    let second = audit_input(2).build(first.record_hash.clone()).unwrap();
    let mut builder = SegmentBuilder::new(
        AuditStream::LocalRoot,
        AuditOwner::LocalRootBroker,
        resource("segment-one"),
        zero_digest(),
        generation(1),
        100,
        1,
    )
    .unwrap();
    builder.push(first).unwrap();
    builder.push(second).unwrap();
    let segment = builder
        .seal(200, PruneStatus::EligibleAfterCheckpoint)
        .unwrap();
    segment.validate().unwrap();

    let mut jsonl = Vec::new();
    for record in &segment.records {
        jsonl.extend(CanonicalJson::encode(record).unwrap());
        jsonl.push(b'\n');
    }
    let decoded = read_audit_segment(segment.summary.clone(), &jsonl, None).unwrap();
    assert_eq!(decoded, segment);

    let record = audit_input(3)
        .build(segment.summary.segment_digest.clone())
        .unwrap();
    let mut next = SegmentBuilder::new(
        AuditStream::LocalRoot,
        AuditOwner::LocalRootBroker,
        resource("segment-two"),
        segment.summary.segment_digest.clone(),
        generation(1),
        201,
        3,
    )
    .unwrap();
    next.push(record).unwrap();
    let next = next
        .seal(220, PruneStatus::EligibleAfterCheckpoint)
        .unwrap();
    next.validate_after(&segment).unwrap();

    let checkpoint = checkpoint(&segment, resource("checkpoint-one"), None, 201, None).unwrap();
    checkpoint
        .verify_for_segment(&segment, None, &NoSignatures)
        .unwrap();
    let policy = AuditRetentionPolicy {
        max_age_days: 14,
        max_segment_bytes: MAX_AUDIT_SEGMENT_BYTES,
        max_records_per_segment: MAX_AUDIT_RECORDS_PER_SEGMENT as u32,
        checkpoint_required_before_prune: true,
    };
    assert_eq!(
        decide_retention(
            &policy,
            AuditRetentionEvidence {
                age_days: 14,
                segment_bytes: segment.summary.encoded_bytes,
                record_count: segment.records.len() as u32,
                sealed_segment: Some(&segment),
                checkpoint: Some(&checkpoint),
                previous_checkpoint: None,
                signature_verifier: &NoSignatures,
            }
        )
        .unwrap(),
        AuditRetentionDecision::PruneCheckpointedSegment
    );
    assert!(
        detect_gap(AuditStream::LocalRoot, 3, 5, 300)
            .unwrap()
            .is_some()
    );

    let mut broken = jsonl;
    broken.pop();
    assert_eq!(
        read_audit_segment(segment.summary, &broken, None)
            .unwrap_err()
            .code(),
        ErrorCode::AuditInvalid
    );
}

#[test]
fn append_only_audit_writer_uses_bounded_complete_jsonl_records() {
    let scratch = Scratch::new("audit");
    let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
    let metadata = host_metadata(&scratch.0, 0o600);
    let audit_resource = AnchoredResource::new(
        resource("audit"),
        &anchor,
        LeafName::parse("audit.jsonl").unwrap(),
    );
    let mut appender = AuditAppender::create(
        audit_resource,
        metadata,
        AuditStream::LocalRoot,
        zero_digest(),
        1,
    )
    .unwrap();
    appender.append(audit_input(1)).unwrap();
    appender.append(audit_input(2)).unwrap();
    assert_eq!(appender.record_count(), 2);
    let first_size = appender.encoded_bytes();
    drop(appender);
    let resource = AnchoredResource::new(
        resource("audit"),
        &anchor,
        LeafName::parse("audit.jsonl").unwrap(),
    );
    let mut appender =
        AuditAppender::resume(resource, metadata, AuditStream::LocalRoot, zero_digest(), 1)
            .unwrap();
    assert_eq!(appender.encoded_bytes(), first_size);
    appender.append(audit_input(3)).unwrap();
    assert_eq!(appender.record_count(), 3);
    assert_eq!(
        appender.encoded_bytes(),
        fs::metadata(scratch.0.join("audit.jsonl")).unwrap().len()
    );
    let bytes = fs::read(scratch.0.join("audit.jsonl")).unwrap();
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 3);
}

#[test]
fn leases_bind_generation_expiry_revocation_and_explicit_transfer() {
    let mut lease = grant_lease(
        resource("lease"),
        resource("state"),
        AuthorityRef::LocalRootBroker,
        generation(3),
        500,
        FdTransferPolicy::ScmRightsLeaseHandoff,
    )
    .unwrap();
    assert!(validate_lease(&lease, generation(3), 499).is_ok());
    assert_eq!(
        validate_lease(&lease, generation(2), 499)
            .unwrap_err()
            .code(),
        ErrorCode::GenerationRollback
    );
    revoke_lease(&mut lease, generation(3), LeaseRevocation::RevokedByOwner).unwrap();
    assert!(validate_lease(&lease, generation(3), 499).is_err());
    assert!(
        grant_lease(
            resource("lease"),
            resource("state"),
            AuthorityRef::LocalRootBroker,
            generation(3),
            500,
            FdTransferPolicy::Never,
        )
        .is_err()
    );
}

#[test]
fn errors_and_paths_are_redacted() {
    let error = Error::Code(ErrorCode::PathRejected);
    assert!(!format!("{error:?}").contains("secret"));
    let io = Error::Os {
        code: ErrorCode::Io,
        errno: Some(13),
    };
    assert_eq!(io.to_string(), "d2b state operation failed (Io, errno=13)");
    assert!(format!("{io:?}").contains("errno: Some(13)"));
    let path = RelativePath::from_components(["realm", "private"]).unwrap();
    assert_eq!(format!("{path:?}"), "RelativePath([redacted])");
}
