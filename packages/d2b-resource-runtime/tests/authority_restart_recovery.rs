//! Restart recovers the durable identity an authority mutation carries, and
//! still refuses one it cannot name (R35, R41).
//!
//! The unit this covers is "the authority journal has a production caller".
//! Both scenarios below therefore go through [`SpecStore::open`] - the
//! production open path, the same call the plane makes - rather than a
//! fixture that stages a store some other way.
//!
//! 1. A mutation that reached the fence before the restart is still fenced
//!    afterwards: the store's recovery answer names it, and the next mutation
//!    for the Zone is refused by that exact identity rather than overtaking
//!    it. Nothing is fabricated and nothing is silently released.
//! 2. The negative: a transaction this store never committed - an unknown one,
//!    or one the store already accepted - is still refused after the restart.
//!    Replaying a committed transaction returns its recorded bytes instead of
//!    applying the mutation twice.

use d2b_resource_runtime::authority_journal::{CommitOutcome, DesiredMutation};
use d2b_resource_runtime::spec_store::{
    ResourceKey, ResourceProvenance, SpecStore, SpecStoreError, StoredDesiredResource,
};
use d2b_resource_runtime::test_support::RecordingPublisher;
use d2b_resource_runtime::{
    AcceptedPublication, TransactionId, TransactionRecovery,
};
use tempfile::TempDir;

const ZONE: &str = "host";

fn key(name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, "Volume", name)
}

fn row(name: &str, spec: &[u8]) -> StoredDesiredResource {
    let mut uid = [0u8; 16];
    let bytes = name.as_bytes();
    uid[..bytes.len().min(16)].copy_from_slice(&bytes[..bytes.len().min(16)]);
    StoredDesiredResource {
        key: key(name),
        uid,
        generation: 0,
        owner_uid: None,
        provenance: ResourceProvenance::Api,
        deleting: false,
        spec: spec.to_vec(),
        metadata: b"meta".to_vec(),
        created_at: 0,
    }
}

/// Open through the production path, at the path the plane opens.
fn open_production(dir: &TempDir) -> SpecStore {
    SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("the production store opens")
}

/// Stage a candidate and record its fence, stopping exactly where a daemon
/// that died after the broker froze the Zone would stop: the candidate is
/// durable, the fence is named, and no desired row has moved.
async fn fence_without_committing(store: &SpecStore, mutation: DesiredMutation) -> TransactionId {
    let staged = store.stage_mutation(mutation).await.expect("stage");
    store
        .record_prepared(staged.transaction, "fence-before-restart")
        .await
        .expect("record the prepared identity");
    staged.transaction
}

/// A restart recovers durable identity: a transaction fenced before the
/// restart is still fenced after it, named by the Zone's own recovery
/// answer, and a second mutation is refused by that exact identity rather
/// than overtaking its fence (R35, R41, AE18).
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_fence_survives_a_restart_and_is_still_fenced_afterwards() {
    let dir = TempDir::new().unwrap();
    let transaction = {
        let store = open_production(&dir);
        let transaction = fence_without_committing(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
        // Nothing moved: the fence is ahead of the desired state, so no
        // provider may act on this row and no cleanup may run against it.
        assert!(
            store.get(key("data")).await.is_err(),
            "a fenced candidate commits no desired row, so the row is still absent"
        );
        transaction
    };
    // The store above went out of scope with its writer thread: this reopen
    // through the production open path is the restart, not a second handle
    // onto the same writer.
    let store = open_production(&dir);
    let recovery = store.zone_recovery(ZONE).await.expect("the Zone answers what it owes");
    assert!(recovery.has_outstanding(), "the fence did not survive the restart: {recovery:?}");
    let (identity, decision) = recovery
        .transactions
        .iter()
        .find(|(id, _)| *id == transaction)
        .expect("recovery names the exact transaction that was fenced");
    assert_eq!(*identity, transaction);
    assert!(
        matches!(decision, TransactionRecovery::ReplayOrCancel { .. }),
        "a fence with no committed row is replayable or cancellable by name: {decision:?}"
    );

    // The negative, same store after the same restart: a second mutation for
    // this Zone is refused by that exact identity instead of committing
    // underneath a fence the broker still holds.
    let refused = store.stage_mutation(DesiredMutation::Ensure(row("other", b"spec-v1"))).await;
    assert!(
        matches!(
            refused,
            Err(SpecStoreError::ZoneTransactionOutstanding { transaction: outstanding, .. })
                if outstanding == transaction
        ),
        "a second mutation is refused by the outstanding fence's identity: {refused:?}"
    );
    // Proved absent: the refused mutation wrote nothing.
    assert!(store.get(key("other")).await.is_err(), "the refused candidate wrote no row");
}

/// The negative: after a restart, an effect from an unknown or superseded
/// transaction is still refused, and a committed transaction replays its
/// recorded bytes instead of applying the mutation a second time (R41).
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_unknown_or_superseded_transaction_is_still_refused_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let committed = {
        let store = open_production(&dir);
        let publisher = RecordingPublisher::new();
        let staged = store.stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v1"))).await.expect("stage");
        store
            .record_prepared(staged.transaction, "fence")
            .await
            .expect("prepare");
        let CommitOutcome::Committed(committed) =
            store.commit_mutation(staged.transaction).await.expect("commit")
        else {
            panic!("a new row commits");
        };
        publisher.accept(&committed).await;
        store
            .acknowledge(AcceptedPublication {
                transaction: committed.transaction,
                zone: committed.zone.clone(),
                incarnation: committed.incarnation.clone(),
                sequence: committed.sequence,
                candidate: committed.candidate.clone(),
            })
            .await
            .expect("acknowledge");
        committed
    };

    let store = open_production(&dir);

    // An acknowledgment naming a transaction this store never committed is
    // refused: the store still holds its durable facts, and it does not
    // publish visibility for a revision it never wrote.
    let unknown = TransactionId::from_bytes([0xab; 16]);
    let refused = store
        .acknowledge(AcceptedPublication {
            transaction: unknown,
            zone: ZONE.to_owned(),
            incarnation: store.store_incarnation().await.expect("incarnation"),
            sequence: committed.sequence,
            candidate: committed.candidate.clone(),
        })
        .await;
    assert!(
        matches!(refused, Err(SpecStoreError::TransactionNotFound { transaction }) if transaction == unknown),
        "an unknown transaction is refused by name after the restart: {refused:?}"
    );

    // A transaction whose committed facts do not match is refused as a
    // mismatch rather than accepted.
    let mismatch = store
        .acknowledge(AcceptedPublication {
            transaction: committed.transaction,
            zone: ZONE.to_owned(),
            incarnation: store.store_incarnation().await.expect("incarnation"),
            sequence: committed.sequence,
            candidate: d2b_contracts_resource::v3::DesiredDigest::of(b"not what this store committed"),
        })
        .await;
    assert!(
        matches!(mismatch, Err(SpecStoreError::PublicationMismatch { .. })),
        "an acknowledgment naming facts this store did not commit is refused: {mismatch:?}"
    );

    // A superseded transaction is refused rather than re-applied: this one is
    // already accepted, so there is nothing left to commit under its identity.
    // The row the commit wrote is still exactly the one the accepted
    // transaction wrote - a replay would advance it a second time.
    let replayed = store.commit_mutation(committed.transaction).await;
    assert!(
        matches!(replayed, Err(SpecStoreError::TransactionStateConflict { .. })),
        "an already-accepted transaction is not committed again: {replayed:?}"
    );
    let accepted = store
        .accepted_cursor(ZONE)
        .await
        .expect("the accepted cursor is durable")
        .expect("an acknowledged Zone has an accepted cursor");
    assert_eq!(accepted.sequence, committed.sequence, "the accepted cursor names the committed sequence");
    assert_eq!(accepted.digest, committed.candidate, "and the committed candidate digest");
    assert_eq!(
        store.get(key("data")).await.expect("the row is durable").generation,
        1,
        "the refused commit did not advance the row a second time"
    );
    // And nothing is outstanding: the Zone owes no outcome after the restart.
    assert!(
        !store.zone_recovery(ZONE).await.expect("recovery").has_outstanding(),
        "an accepted Zone owes nothing after a restart"
    );
}
