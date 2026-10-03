//! Store-view delivery as the realization of an admitted export (U15).
//!
//! The four cases are the unit's scenarios, and each is proven where the
//! property is observable rather than through a plan:
//!
//! 1. **AE21 - prepared before the consumer runs.** The export a consumer
//!    is admitted for names the consumer, the view, and one generation, and
//!    nothing about the consumer's liveness, so the pre-boot condition can
//!    never wait on the post-boot mount observation
//!    ([`a_publication_for_an_admitted_export_needs_no_consumer_state`]).
//! 2. **A restart cannot expose another view or take a second writer.** The
//!    farm is shared, so a writable export would be a write through every
//!    reader's inodes: it is refused outright, a generation the consumer was
//!    not admitted for is refused, and a helper's leg over the parent's
//!    reservation is attenuated rather than a second writer
//!    ([`a_restart_cannot_expose_another_view_or_a_second_writer`]).
//! 3. **A read-only closure export never mutates the shared store.** The real
//!    [`build_read_only_store_view`] runs against a temporary content store,
//!    and the source inodes' identity, link count, ownership, mode, size, and
//!    change times are read back afterwards
//!    ([`a_read_only_closure_export_never_mutates_the_shared_store`]).
//! 4. **Detach keeps the helper available until its use closes.** A fenced
//!    relationship whose helper is still live refuses to finalize helpers and
//!    refuses to release, and the helper keeps the control lane throughout
//!    ([`detach_keeps_the_helper_available_until_its_use_closes`]).
//!
//! Filesystem work lives in synchronous helpers: the async bodies only await
//! the farm primitive, so no blocking call runs on a runtime worker.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use d2b_broker::binding_reservations::{
    BindingReservationService, DrainEvidence, DrainImplementation, DrainStage, LegIdentity,
    LegObservation, ReleaseReport, ReservationCapability, ReservationError,
};
use d2b_broker::ops::store_sync::{StoreSyncExportError, run_store_sync_for_export};
use d2b_broker::ops::store_view_farm::{
    ExportRefusal, FarmMutation, StoreViewExportBinding, StoreViewExportError,
    build_read_only_store_view,
};
use d2b_broker::state_cells::CellStore;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingArbitration, BindingAuthorization, BindingKey,
    BindingKind, BindingRealizationFacet,
    BindingRealizationSupport, BindingSlot, BindingSpecFingerprint, BoundedText, BoundedToken,
    BrokerRequirement, CallableOperation, DesiredDigest, DesiredRevision, FreshnessTuple,
    OperationAudit, OperationAuthority, OperationBounds, OperationDomain, OperationFds,
    OperationImplementation, OperationSurface, PayloadProvenance, PayloadSchema, RefusalReason,
    RequestedRights, ResourceRef, ResourceUid, SecretAccess, SourceAdmission, SourceReservation,
    StoreIncarnation, ZoneId, admit_binding_request,
};
use d2b_contracts_resource::v3::{AuditJoin, AuditMode};
use d2b_core::bundle_resolver::{ResolvedStoreViewIntent, intent_id_store_view};
use d2b_host::hardlink_farm::{self, GenerationMarker};

const CONSUMER: &str = "corp-vm";
const VIEW: &str = "ro-store";
const GENERATION: u64 = 7;
const SOURCE_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca401";
const PEER_SOURCE_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca402";
const CLOSURE_BASENAME: &str = "aaaaaaaa-corp-vm-system";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("valid uid")
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn zone() -> ZoneId {
    ZoneId::parse("work").expect("valid zone")
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("valid token")
}

fn admitted_export() -> StoreViewExportBinding {
    StoreViewExportBinding::admit(token(CONSUMER), token(VIEW), GENERATION, true)
        .expect("a read-only export is admitted")
}

/// A throwaway tree under the OS temp dir, removed when the case ends.
struct Fixture {
    root: PathBuf,
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "d2b-store-view-binding-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create the fixture root");
        Self { root }
    }

    /// A stand-in for the shared content store: one closure directory
    /// holding one file, exactly the shape the farm links.
    fn content_store(&self) -> PathBuf {
        let store = self.root.join("nix-store");
        let closure = store.join(CLOSURE_BASENAME);
        fs::create_dir_all(&closure).expect("create the closure");
        fs::write(closure.join("payload"), b"closure bytes").expect("write the closure payload");
        store
    }

    fn farm_root(&self) -> PathBuf {
        let farm = self.root.join("farm");
        fs::create_dir_all(&farm).expect("create the farm root");
        farm
    }

    fn intent(&self, farm_root: &Path, closure: PathBuf) -> ResolvedStoreViewIntent {
        let db_dump = self.root.join("registration");
        fs::write(&db_dump, b"db-dump").expect("write the db dump");
        ResolvedStoreViewIntent {
            intent_id: intent_id_store_view(&zone(), CONSUMER),
            vm: CONSUMER.to_owned(),
            generation: GENERATION,
            hardlink_farm_path: farm_root.to_path_buf(),
            target_view_path: farm_root.join("live").join(CLOSURE_BASENAME),
            closure_paths: vec![closure],
            db_dump_path: db_dump,
        }
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The identity of one source inode, read straight from the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceInode {
    ino: u64,
    nlink: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    size: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn source_inode(path: &Path) -> SourceInode {
    let meta = fs::metadata(path).expect("the inode is readable");
    SourceInode {
        ino: meta.ino(),
        nlink: meta.nlink(),
        uid: meta.uid(),
        gid: meta.gid(),
        mode: meta.mode(),
        size: meta.size(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        ctime: meta.ctime(),
        ctime_nsec: meta.ctime_nsec(),
    }
}

fn marker(generation: u32) -> GenerationMarker {
    GenerationMarker {
        closure_hash: "closure-hash".to_owned(),
        d2b_version: "0.0.0-bootstrap".to_owned(),
        activated_at: "unix-0".to_owned(),
        vm: CONSUMER.to_owned(),
        generation_number: generation,
    }
}

fn generation_token() -> u32 {
    u32::try_from(GENERATION).expect("the fixture generation fits a u32 token")
}

/// Whether `path` holds no entries. Synchronous on purpose: the async
/// bodies below only await the farm primitive.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn is_empty_dir(path: &Path) -> bool {
    fs::read_dir(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .next()
        .is_none()
}

/// The bytes one source file holds. Synchronous for the same reason.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn read_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn generation_id(closure: &Path) -> String {
    let paths = [closure.to_path_buf()];
    hardlink_farm::generation_id(&paths, hardlink_farm::system_store_path(&paths))
}

// ---------------------------------------------------------------------------
// 1. AE21: publication needs no consumer state
// ---------------------------------------------------------------------------

/// The export a consumer is admitted for is the whole authority a
/// publication runs under, and it carries no liveness, no mount
/// observation, and no running-state input. A Guest's storage can
/// therefore be published before the Guest starts, and the mount is
/// observed afterwards as a separate fact: neither condition waits on the
/// other, which is what AE21 requires and what a single "ready" would
/// collapse.
///
/// The generation is pinned, and the refusal is total: a publication for a
/// generation the consumer was not admitted for is refused before the lock
/// is taken and before anything is created, in the publication wrapper and
/// in the build primitive underneath it alike, so the fence is not only in
/// one layer.
///
/// The publication's own ownership/mode posture pass is the broker's
/// privileged one - it `chown`s the farm's declared rows to the `d2bd`
/// principal - so this case does not claim to cover it. It covers what
/// U15 owns: the export authority, the generation fence, and the read-only
/// build those two feed.
// `#[tokio::test]`'s own expansion drives the body through
// `Runtime::block_on`; that generated bridge is the test harness's, and
// the sanctioned inline allow is what keeps it out of the census.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_publication_for_an_admitted_export_needs_no_consumer_state() {
    let fixture = Fixture::new("publication");
    let store = fixture.content_store();
    let closure = store.join(CLOSURE_BASENAME);
    let farm_root = fixture.farm_root();
    let export = admitted_export();
    assert_eq!(export.consumer().as_str(), CONSUMER);
    assert_eq!(export.view().as_str(), VIEW);
    assert_eq!(export.generation(), GENERATION);
    assert!(export.is_read_only());

    let intent = fixture.intent(&farm_root, closure.clone());
    let superseding = StoreViewExportBinding::admit(
        token(CONSUMER),
        token(VIEW),
        GENERATION + 1,
        true,
    )
    .expect("a read-only export is admitted at any generation");

    // Refused, and refused before any effect: the farm is still exactly the
    // empty directory the fixture created.
    assert!(matches!(
        run_store_sync_for_export(&intent, &superseding).await,
        Err(StoreSyncExportError::Refused(
            ExportRefusal::GenerationNotAdmitted
        ))
    ));
    assert!(
        is_empty_dir(&farm_root),
        "a refused publication created nothing"
    );
    assert!(matches!(
        build_read_only_store_view(
            &export,
            &farm_root,
            GENERATION + 1,
            &generation_id(&closure),
            std::slice::from_ref(&closure),
            &marker(generation_token().saturating_add(1)),
        )
        .await,
        Err(StoreViewExportError::Refused(
            ExportRefusal::GenerationNotAdmitted
        ))
    ));
    assert!(
        is_empty_dir(&farm_root),
        "the build primitive refuses before it creates anything either"
    );

    // The admitted generation materialises, with no consumer running and no
    // consumer state in the request at all.
    build_read_only_store_view(
        &export,
        &farm_root,
        GENERATION,
        &generation_id(&closure),
        std::slice::from_ref(&closure),
        &marker(generation_token()),
    )
    .await
    .expect("an admitted export materialises with no consumer running");
    assert!(
        farm_root
            .join("live")
            .join(CLOSURE_BASENAME)
            .join("payload")
            .exists(),
        "the export is on disk once the build returns"
    );
    // A restart re-runs the same export and links nothing new: the
    // generation, not the attempt, is what identifies the materialisation.
    let again = build_read_only_store_view(
        &export,
        &farm_root,
        GENERATION,
        &generation_id(&closure),
        std::slice::from_ref(&closure),
        &marker(generation_token()),
    )
    .await
    .expect("a restart re-runs the same admitted export");
    assert_eq!(again.linked, 0);
    assert_eq!(again.skipped, 1);
}

// ---------------------------------------------------------------------------
// 2. A restart cannot expose another view or take a second writer
// ---------------------------------------------------------------------------

/// The farm is shared by every consumer, so a writable export through it
/// would be a write through every reader's inodes. A restarted helper
/// cannot obtain one, cannot publish a generation it was not admitted for,
/// and cannot reach another view: its leg is bound to the parent's
/// reservation, is attenuated to a right the parent holds, and names the
/// exact source, so a second writer over that source is refused against the
/// parent's own decision and a leg can never introduce a source of its own.
// `#[tokio::test]`'s own expansion drives the body through
// `Runtime::block_on`; that generated bridge is the test harness's, and
// the sanctioned inline allow is what keeps it out of the census.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_restart_cannot_expose_another_view_or_a_second_writer() {
    let fixture = Fixture::new("restart");
    let store = fixture.content_store();
    let closure = store.join(CLOSURE_BASENAME);
    let farm_root = fixture.farm_root();
    let export = admitted_export();

    // A writable export of a shared closure store is refused outright, so
    // the restart has nothing to come back holding.
    assert_eq!(
        StoreViewExportBinding::admit(token(CONSUMER), token(VIEW), GENERATION, false)
            .expect_err("a writable export is refused"),
        ExportRefusal::WritableExportUnsupported
    );
    // Two views of one source are two exports, and neither can be built at
    // the other's admission: the view is part of what the consumer was
    // admitted for, not a hint the build may re-read.
    let other_view = StoreViewExportBinding::admit(token(CONSUMER), token("live"), GENERATION, true)
        .expect("a read-only export is admitted");
    assert_ne!(export.view(), other_view.view());
    assert_eq!(other_view.generation(), export.generation());

    build_read_only_store_view(
        &export,
        &farm_root,
        GENERATION,
        &generation_id(&closure),
        std::slice::from_ref(&closure),
        &marker(generation_token()),
    )
    .await
    .expect("the admitted export builds");
    // Re-running the same export is idempotent: the restart rebinds the
    // same generation rather than materialising a second one.
    let again = build_read_only_store_view(
        &export,
        &farm_root,
        GENERATION,
        &generation_id(&closure),
        &[store.join(CLOSURE_BASENAME)],
        &marker(generation_token()),
    )
    .await
    .expect("a restart re-runs the same admitted export");
    assert_eq!(again.linked, 0, "the restart linked nothing new");

    let mut service = reservation_service();
    let key = guest_key();
    let parent = service
        .admit_binding(
            admission(&key, RequestedRights::Mutate),
            reservation("serving-helper"),
            &fingerprint("parent"),
        )
        .expect("the parent's claim is admitted");
    service
        .confirm_prepared(&parent)
        .expect("the source is prepared");
    let leg = service
        .attach_leg(
            &parent,
            leg_identity("serving-helper"),
            uid(SOURCE_UID),
            // Attenuated: the helper consumes the parent's exact source
            // under the parent's own decision rather than claiming the
            // mutating right a second time.
            RequestedRights::Consume,
            operation("export"),
        )
        .expect("the helper leg is bound to the parent reservation");
    let grant = service
        .use_leg(&leg, ReservationCapability::Observe)
        .expect("the helper may observe the parent's reservation");
    assert_eq!(grant.source(), key.source_uid());
    assert_eq!(grant.rights(), Some(RequestedRights::Consume));
    assert!(
        !grant.rights().expect("a realization leg holds a right").needs_arbitration(),
        "a helper leg is never a second arbitrating writer"
    );

    // A second writer over the same source is refused against the parent's
    // decision. The helper's presence changed nothing about that decision.
    let peer = peer_key();
    assert!(matches!(
        service.admit_binding(
            admission(&peer, RequestedRights::Mutate),
            reservation("second-writer"),
            &fingerprint("second"),
        ),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Reserve,
            reason: RefusalReason::ConflictingDeclaration,
        })
    ));

    // A leg can never introduce a source of its own, so the restart cannot
    // reach another consumer's view that way either.
    assert!(matches!(
        service.attach_leg(
            &parent,
            leg_identity("restarted-helper"),
            uid(PEER_SOURCE_UID),
            RequestedRights::Consume,
            operation("export"),
        ),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Prepare,
            reason: RefusalReason::ConflictingDeclaration,
        })
    ));
    // And a right the parent never held is refused on the same grounds.
    assert!(matches!(
        service.attach_leg(
            &parent,
            leg_identity("upgrading-helper"),
            uid(SOURCE_UID),
            RequestedRights::Share,
            operation("export"),
        ),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Prepare,
            reason: RefusalReason::SourcePolicyRefused,
        })
    ));
    // A helper re-declared at the same identity is the SAME leg, so the
    // restart cannot mint a second grant over it.
    assert!(matches!(
        service.attach_leg(
            &parent,
            leg_identity("serving-helper"),
            uid(SOURCE_UID),
            RequestedRights::Consume,
            operation("export"),
        ),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Prepare,
            reason: RefusalReason::ConflictingDeclaration,
        })
    ));
}

// ---------------------------------------------------------------------------
// 3. A read-only closure export never mutates the shared store
// ---------------------------------------------------------------------------

/// The real build, run against a temporary content store.
///
/// The farm's live pool is made of hardlinks to the content store, so a
/// read-only export is only read-only if the build writes nothing about
/// those inodes. The evidence is the source's own metadata after the build:
/// the farm shares the inode (that is why the comparison matters at all),
/// and the source's ownership, mode, size, modification time, and change
/// time are all exactly what they were. A recursive posture walk, an
/// ownership change, or a permission change would move the change time; a
/// write would move the size and the modification time.
// `#[tokio::test]`'s own expansion drives the body through
// `Runtime::block_on`; that generated bridge is the test harness's, and
// the sanctioned inline allow is what keeps it out of the census.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_read_only_closure_export_never_mutates_the_shared_store() {
    let fixture = Fixture::new("read-only");
    let store = fixture.content_store();
    let closure = store.join(CLOSURE_BASENAME);
    let payload = closure.join("payload");
    let farm_root = fixture.farm_root();
    let export = admitted_export();

    let closure_before = source_inode(&closure);
    let payload_before = source_inode(&payload);
    let store_before = source_inode(&store);

    build_read_only_store_view(
        &export,
        &farm_root,
        GENERATION,
        &generation_id(&closure),
        std::slice::from_ref(&closure),
        &marker(generation_token()),
    )
    .await
    .expect("the read-only export builds");

    // The farm really does share the inode: the entry is a hardlink, not a
    // copy, which is exactly why the metadata comparison below matters.
    let linked = source_inode(&farm_root.join("live").join(CLOSURE_BASENAME).join("payload"));
    assert_eq!(linked.ino, payload_before.ino, "the export hardlinks");
    assert!(linked.nlink > payload_before.nlink);

    // The one field a hardlink legitimately moves is the link count, and
    // `link(2)` advances the change time with it. Everything the export is
    // forbidden to touch is therefore checked on the fields it could not
    // have moved: a `chmod` or a `chown` would change the mode, the owner,
    // or the group, and a write would change the size or the modification
    // time. None of them moved.
    let payload_after = source_inode(&payload);
    assert_eq!(
        payload_after.nlink,
        payload_before.nlink + 1,
        "the export is exactly one new link"
    );
    assert_eq!(payload_after.ino, payload_before.ino);
    assert_eq!(payload_after.mode, payload_before.mode);
    assert_eq!(payload_after.uid, payload_before.uid);
    assert_eq!(payload_after.gid, payload_before.gid);
    assert_eq!(payload_after.size, payload_before.size);
    assert_eq!(payload_after.mtime, payload_before.mtime);
    assert_eq!(payload_after.mtime_nsec, payload_before.mtime_nsec);
    assert_eq!(
        read_bytes(&payload),
        b"closure bytes",
        "the export never wrote the shared bytes"
    );

    // The directories are the discriminator for the no-recursive-mutation
    // rule. The farm mirrors each linked directory's own mode onto its own
    // copy, so the SOURCE directories gain no link and move no timestamp at
    // all: a recursive posture walk would have moved the change time on
    // both of them, and a recursive ownership walk the owner.
    let closure_after = source_inode(&closure);
    assert_eq!(closure_after.ino, closure_before.ino);
    assert_eq!(closure_after.nlink, closure_before.nlink);
    assert_eq!(closure_after.mode, closure_before.mode);
    assert_eq!(closure_after.uid, closure_before.uid);
    assert_eq!(closure_after.gid, closure_before.gid);
    assert_eq!(closure_after.mtime, closure_before.mtime);
    assert_eq!(
        closure_after.ctime, closure_before.ctime,
        "no posture walk reached the source closure directory"
    );
    let store_after = source_inode(&store);
    assert_eq!(store_after.ino, store_before.ino);
    assert_eq!(store_after.nlink, store_before.nlink);
    assert_eq!(store_after.mode, store_before.mode);
    assert_eq!(store_after.uid, store_before.uid);
    assert_eq!(store_after.gid, store_before.gid);
    assert_eq!(store_after.mtime, store_before.mtime);
    assert_eq!(
        store_after.ctime, store_before.ctime,
        "no posture walk reached the shared store directory"
    );

    // And the rule is a fence with a negative side: every operation that
    // would reach a shared inode through the farm is refused, whatever the
    // caller asks for, and the build set is exactly the four additive
    // operations it performs.
    for mutation in FarmMutation::REFUSED_SET {
        assert!(mutation.mutates_shared_inodes());
        assert_eq!(
            export
                .admits(mutation)
                .expect_err("a shared-inode mutation is refused"),
            ExportRefusal::MutationNotAdmitted,
            "{mutation:?} must never be admitted"
        );
    }
    for mutation in FarmMutation::BUILD_SET {
        assert!(!mutation.mutates_shared_inodes());
        assert!(
            export.admits(mutation).is_ok(),
            "{mutation:?} is part of the build"
        );
    }
    // A mutation target is a path inside the farm the export owns: never
    // the farm root itself, never a declared closure path, never the store.
    assert!(export.owns_mutation_target(&farm_root, &farm_root.join("live")));
    assert!(!export.owns_mutation_target(&farm_root, &farm_root));
    assert!(!export.owns_mutation_target(&farm_root, &payload));
    assert!(!export.owns_mutation_target(&farm_root, &store));
}

// ---------------------------------------------------------------------------
// 4. Detach keeps the helper available until its use closes
// ---------------------------------------------------------------------------

/// A detached relationship whose helper still holds the parent's source
/// keeps that reservation.
///
/// While the relationship is live the helper's leg holds the whole control
/// lane over the parent's reservation. Fencing the relationship retires
/// exactly those grants - a revoked parent cannot keep being driven by the
/// leg that was serving it - and hands the control lane to a cleanup leg
/// declared in advance, which holds no right at all and therefore cannot
/// serve the consumer through it.
///
/// What the detach does NOT do is free the source. Finalizing the helpers
/// and releasing are both refused while the helper is still live, because a
/// live helper is admitted use of the source, and the durable claim
/// survives the whole sequence so a crash between stages finds the
/// reservation rather than a free source. Only a proven-finalized helper
/// lets the drain finish, and only then does the claim retire.
#[test]
fn detach_keeps_the_helper_available_until_its_use_closes() {
    let mut service = reservation_service();
    let key = guest_key();
    let parent = service
        .admit_binding(
            admission(&key, RequestedRights::Mutate),
            reservation("serving-helper"),
            &fingerprint("detach"),
        )
        .expect("the parent's claim is admitted");
    service
        .declare_drain_implementation(
            &parent,
            DrainImplementation::new(
                callable_operation("close"),
                ReservationCapability::CLEANUP_LANE,
            )
            .expect("a declared cleanup implementation"),
        )
        .expect("the cleanup implementation is declared before the fence");
    service
        .confirm_prepared(&parent)
        .expect("the source is prepared");
    let leg = service
        .attach_leg(
            &parent,
            leg_identity("serving-helper"),
            uid(SOURCE_UID),
            RequestedRights::Consume,
            operation("export"),
        )
        .expect("the helper leg is bound to the parent reservation");
    service
        .observe_leg(&leg, LegObservation::Live)
        .expect("the helper is observed live");

    // The helper is available: the full control lane runs over the
    // parent's own reservation, and every grant names that exact source.
    for capability in ReservationCapability::CLEANUP_LANE {
        let grant = service
            .use_leg(&leg, capability)
            .unwrap_or_else(|error| panic!("the control lane is available: {error}"));
        assert_eq!(grant.source(), key.source_uid());
    }

    // The consumer detaches. The relationship is fenced and the serving
    // leg's grants are retired with it.
    service.fence(&parent).expect("the relationship is fenced");
    assert!(matches!(
        service.use_leg(&leg, ReservationCapability::Observe),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Revoke,
            reason: RefusalReason::StaleAuthority,
        })
    ));
    // The control lane moves to the declared cleanup leg, which carries no
    // right: it can drive the teardown and it cannot serve the consumer.
    // A cleanup leg is a NEW leg identity under the new fence, never a
    // remint of the serving leg the revocation just retired.
    let cleanup = service
        .open_cleanup_leg(
            &parent,
            leg_identity("serving-helper-cleanup"),
            uid(SOURCE_UID),
        )
        .expect("the declared cleanup leg opens under the new fence");
    for capability in ReservationCapability::CLEANUP_LANE {
        let grant = service
            .use_leg(&cleanup, capability)
            .unwrap_or_else(|error| panic!("the cleanup lane is available: {error}"));
        assert_eq!(grant.source(), key.source_uid());
        assert_eq!(grant.rights(), None, "a cleanup leg holds no right");
    }

    let plan = service.plan_drain(&parent).expect("a drain plan");
    assert_eq!(plan.next(), Some(DrainStage::ConsumerDetached));
    assert_eq!(
        service
            .advance_drain(
                &parent,
                DrainStage::ConsumerDetached,
                DrainEvidence {
                    consumer_detached: true,
                    helpers_finalized: false,
                },
            )
            .expect("the detach is recorded"),
        DrainStage::ConsumerDetached
    );

    // The helper still holds the source, so the drain stops here...
    assert!(matches!(
        service.advance_drain(
            &parent,
            DrainStage::HelpersFinalized,
            DrainEvidence {
                consumer_detached: true,
                helpers_finalized: false,
            },
        ),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Drain,
            reason: RefusalReason::UnprovenEffect,
        })
    ));
    // ...and the reservation is not released under a live helper.
    assert!(matches!(
        service.release(&parent),
        Err(ReservationError::Refused {
            stage: AdmissionStage::Release,
            reason: RefusalReason::UnprovenEffect,
        })
    ));
    // The relationship is fenced, not released: a refused advance leaves it
    // held, and the helper's liveness - not a separate busy flag - is what
    // keeps it that way. Every stage an owner could advance to is refused
    // while the helper is live.
    assert!(
        service.is_fenced(&parent).expect("the relationship is known"),
        "the relationship stays fenced across the whole drain"
    );
    assert!(
        !service.recoverable_claims().is_empty(),
        "the durable claim survives a detach: a restart finds the \
         reservation, not a free source"
    );

    // The helper's use closes, and only then does the drain finish.
    service
        .observe_leg(&leg, LegObservation::Finalized)
        .expect("the helper's use is proven closed");
    service
        .observe_leg(&cleanup, LegObservation::Finalized)
        .expect("the cleanup leg is proven closed too");
    assert_eq!(
        service
            .advance_drain(
                &parent,
                DrainStage::HelpersFinalized,
                DrainEvidence {
                    consumer_detached: true,
                    helpers_finalized: true,
                },
            )
            .expect("a finalized helper lets the drain advance"),
        DrainStage::HelpersFinalized
    );
    // The release is what retires the claim, so it is the call that ends
    // the sequence: it is the only step that takes the source claim itself
    // out of the owner's table.
    assert_eq!(
        service.release(&parent).expect("release"),
        ReleaseReport::ReleasedSourceFreed,
        "the last holder frees the source claim"
    );
    assert!(
        service.recoverable_claims().is_empty(),
        "the durable claim is retired once the reservation is gone"
    );
    // And a retried release finds no claim at all rather than freeing it a
    // second time, so a drain retried after the release cannot hand the
    // same source to somebody else.
    assert!(matches!(
        service.release(&parent),
        Err(ReservationError::UnknownReservation)
    ));
}

// ---------------------------------------------------------------------------
// Reservation fixtures
// ---------------------------------------------------------------------------

// The in-memory cell store drives its owner through a synchronous bridge;
// a hermetic reservation fixture has no other way to own one.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn reservation_service() -> BindingReservationService {
    BindingReservationService::new(Arc::new(CellStore::in_memory()))
}

fn guest_key() -> BindingKey {
    binding_key("Guest/corp-vm", "1b4e28ba-2fa1-41d2-883f-0016d3cca411", "root")
}

fn peer_key() -> BindingKey {
    binding_key("Guest/personal-vm", "1b4e28ba-2fa1-41d2-883f-0016d3cca412", "root")
}

fn binding_key(consumer: &str, consumer_uid: &str, slot: &str) -> BindingKey {
    BindingKey::new(
        zone(),
        BindingKind::Volume,
        ResourceRef::parse("Volume/work-state").expect("valid source"),
        uid(SOURCE_UID),
        ResourceRef::parse(consumer).expect("valid consumer"),
        uid(consumer_uid),
        BindingSlot::parse(slot).expect("valid slot"),
    )
    .expect("a valid binding key")
}

fn dependencies() -> Vec<FreshnessTuple> {
    vec![FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("store-1").expect("valid incarnation"),
        ResourceRef::parse("Volume/work-state").expect("valid source"),
        uid(SOURCE_UID),
        DesiredRevision::INITIAL.try_next().expect("valid revision"),
        DesiredDigest::of(b"{}"),
    )]
}

fn admission(key: &BindingKey, rights: RequestedRights) -> BindingAdmission {
    let source =
        SourceAdmission::new(key.clone(), vec![rights], BindingArbitration::Exclusive)
            .expect("a valid source admission");
    let support =
        BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
            .expect("a valid realization support");
    admit_binding_request(
        key,
        rights,
        &[BindingRealizationFacet::FilesystemPresentation],
        &BindingAuthorization::granted(),
        &source,
        &support,
        &dependencies(),
    )
    .expect("the request is admitted")
}

fn reservation(name: &str) -> SourceReservation {
    SourceReservation::new(zone(), uid(SOURCE_UID), token(name))
}

fn fingerprint(seed: &str) -> BindingSpecFingerprint {
    BindingSpecFingerprint::from_request(&serde_json::json!({ "seed": seed }))
}

fn leg_identity(name: &str) -> LegIdentity {
    LegIdentity::new(
        token(name),
        uid("1b4e28ba-2fa1-41d2-883f-0016d3cca420"),
        DesiredRevision::INITIAL,
    )
}

/// The declared implementation a helper's leg runs under.
fn operation(method: &str) -> OperationImplementation {
    OperationImplementation::provider_method(
        ResourceRef::parse("Provider/work-state").expect("valid provider"),
        token("binding"),
        token(method),
    )
    .expect("a declared implementation")
}

/// The callable operation a cleanup child is declared with.
///
/// It is a DIFFERENT declared method from the one the helper's ordinary use
/// ran: a helper that is gone is recovered under its own predeclared
/// implementation rather than by reminting the leg that used to serve.
fn callable_operation(method: &str) -> CallableOperation {
    let payload = PayloadSchema::parse(serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["target"],
        "properties": { "target": { "type": "string" } }
    }))
    .expect("a valid payload");
    CallableOperation::new(
        operation(method),
        payload,
        None,
        false,
        SecretAccess::None,
        OperationAudit::new(
            true,
            AuditMode::Yes,
            vec![BoundedText::parse("target").expect("valid field")],
            Vec::new(),
            token("target"),
        )
        .expect("a valid audit facet"),
        Some(
            AuditJoin::new(vec![BoundedText::parse("target").expect("valid field")])
                .expect("a valid join"),
        ),
        OperationAuthority::new(
            OperationSurface::Broker,
            OperationDomain::Host,
            BoundedText::parse("host-operator").expect("valid authority"),
            BrokerRequirement::Yes,
        ),
        OperationFds::default(),
        OperationBounds::default(),
        PayloadProvenance::Request,
    )
    .expect("a valid callable operation")
}
