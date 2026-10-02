//! Contract coverage for durable desired revisions and recoverable
//! publication transactions (plan unit U5, KTD5-KTD6).
//!
//! The suite walks the four scenarios the unit names, and each one is
//! written against a failure boundary rather than against a code path:
//!
//! 1. Spec, owner, metadata, and deletion changes advance the desired
//!    revision; an identical ensure and runtime status do not (R35, R41,
//!    AE16).
//! 2. The desired row and its outbox entry commit in one transaction under
//!    injected write failures at three different columns, so a failure
//!    leaves no half-published authority change behind.
//! 3. Staged, prepared, and committed-but-unacknowledged transaction
//!    identities survive a restart and recover with no duplicate mutation
//!    (AE18).
//! 4. An exhausted sequence refuses the mutation instead of wrapping.
//!
//! Every store is a temporary database in this directory. Nothing here
//! touches an operator's store, and the injected failures are ordinary
//! SQLite triggers on that temporary database.

use std::path::{Path, PathBuf};

use d2b_resource_runtime::authority_journal::{CommitOutcome, DesiredMutation, PublicationState};
use d2b_resource_runtime::identity::row_freshness;
use d2b_resource_runtime::spec_store::{
    ResourceKey, ResourceProvenance, SpecSelector, SpecStore, SpecStoreError, StoredDesiredResource,
};
use d2b_resource_runtime::{
    AcceptedPublication, DesiredRow, TransactionId, TransactionRecovery,
};
use tempfile::TempDir;

const ZONE: &str = "host";
const OTHER_ZONE: &str = "guest";

fn key(name: &str) -> ResourceKey {
    keyed(ZONE, name)
}

fn keyed(zone: &str, name: &str) -> ResourceKey {
    ResourceKey::new(zone, "Volume", name)
}

fn uid_for(name: &str) -> [u8; 16] {
    let mut uid = [0u8; 16];
    let bytes = name.as_bytes();
    uid[..bytes.len().min(16)].copy_from_slice(&bytes[..bytes.len().min(16)]);
    uid
}

fn row(name: &str, spec: &[u8]) -> StoredDesiredResource {
    rowed(ZONE, name, spec)
}

fn rowed(zone: &str, name: &str, spec: &[u8]) -> StoredDesiredResource {
    StoredDesiredResource {
        key: keyed(zone, name),
        uid: uid_for(name),
        generation: 0,
        owner_uid: None,
        provenance: ResourceProvenance::Api,
        deleting: false,
        spec: spec.to_vec(),
        metadata: b"meta".to_vec(),
        created_at: 0,
    }
}

fn open(dir: &TempDir) -> SpecStore {
    SpecStore::open(dir.path().join("authority.db")).expect("open store")
}

fn database(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Run one desired mutation through the whole protocol and acknowledge it.
///
/// The four steps mirror the only ordering the protocol permits: stage the
/// candidate, record the broker's fence, commit the desired rows with their
/// outbox entry, then acknowledge the accepted revision.
async fn publish(store: &SpecStore, mutation: DesiredMutation) -> CommitOutcome {
    let staged = store.stage_mutation(mutation).await.expect("stage");
    store.record_prepared(staged.transaction, "prepared-1").await.expect("prepare");
    let outcome = store.commit_mutation(staged.transaction).await.expect("commit");
    if let CommitOutcome::Committed(committed) | CommitOutcome::AlreadyCommitted(committed) =
        &outcome
    {
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
    }
    outcome
}

fn revision_of(row: &DesiredRow) -> u64 {
    row.revision.get()
}

// ---------------------------------------------------------------------------
// Scenario 1: what advances the desired revision
// ---------------------------------------------------------------------------

/// R35/AE16: an ownership change, a metadata change, and a deletion advance
/// the desired revision exactly as a spec change does, because each of them
/// invalidates earlier effect authority; an identical ensure advances
/// nothing, and neither do reads, which is all runtime status can be to this
/// store.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn spec_owner_metadata_and_deletion_advance_the_revision_but_an_identical_ensure_does_not() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);

    // A created row starts at revision 1.
    match publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await {
        CommitOutcome::Committed(committed) => {
            assert_eq!(committed.sequence.get(), 1);
            assert_eq!(revision_of(&committed.publication.rows[0]), 1);
            assert_eq!(committed.publication.rows[0].row.generation, 1);
        }
        other => panic!("a first ensure commits: {other:?}"),
    }

    // An identical ensure settles without a revision, without an outbox
    // entry, and without leaving the Zone waiting for a publication.
    let repeated = publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
    let CommitOutcome::Unchanged { sequence, row: unchanged, .. } = repeated else {
        panic!("an identical ensure must not advance: {repeated:?}");
    };
    assert_eq!(sequence.get(), 2, "the reservation still moves; the revision does not");
    assert_eq!(revision_of(&unchanged), 1, "an identical ensure keeps the revision");
    assert_eq!(unchanged.row.generation, 1);

    // A spec change advances the revision and the generation together.
    match publish(&store, DesiredMutation::Ensure(row("data", b"spec-v2"))).await {
        CommitOutcome::Committed(committed) => {
            let committed_row = &committed.publication.rows[0];
            assert_eq!(revision_of(committed_row), 2);
            assert_eq!(committed_row.row.generation, 2);
        }
        other => panic!("a spec change commits: {other:?}"),
    }

    // A metadata-only change: the rendered spec bytes are unchanged, but the
    // ownership-adjacent authored envelope moved, so the revision advances
    // while the spec's own generation does not.
    let mut annotated = row("data", b"spec-v2");
    annotated.metadata = b"authored-2".to_vec();
    match publish(&store, DesiredMutation::Ensure(annotated)).await {
        CommitOutcome::Committed(committed) => {
            let committed_row = &committed.publication.rows[0];
            assert_eq!(revision_of(committed_row), 3, "a metadata change advances the revision");
            assert_eq!(
                committed_row.row.generation, 2,
                "identity and generation remain the spec's contract"
            );
        }
        other => panic!("a metadata change commits: {other:?}"),
    }

    // An owner change is the case AE16 names: spec bytes identical, revision
    // advanced, so earlier effect authority no longer looks current.
    let mut owned = row("data", b"spec-v2");
    owned.metadata = b"authored-2".to_vec();
    owned.owner_uid = Some(uid_for("owner"));
    match publish(&store, DesiredMutation::Ensure(owned)).await {
        CommitOutcome::Committed(committed) => {
            let committed_row = &committed.publication.rows[0];
            assert_eq!(revision_of(committed_row), 4, "an owner change advances the revision");
            assert_eq!(committed_row.row.generation, 2);
            assert_eq!(committed_row.row.owner_uid, Some(uid_for("owner")));
        }
        other => panic!("an owner change commits: {other:?}"),
    }

    // Marking the row deleting is a desired change and advances the revision.
    match publish(&store, DesiredMutation::MarkDeleting(key("data"))).await {
        CommitOutcome::Committed(committed) => {
            let committed_row = &committed.publication.rows[0];
            assert!(committed_row.row.deleting);
            assert_eq!(revision_of(committed_row), 5, "a deletion mark advances the revision");
        }
        other => panic!("a deletion mark commits: {other:?}"),
    }

    // Re-marking an already-deleting row changes nothing.
    let CommitOutcome::Unchanged { row: unchanged, .. } =
        publish(&store, DesiredMutation::MarkDeleting(key("data"))).await
    else {
        panic!("a repeated deletion mark must not advance");
    };
    assert_eq!(revision_of(&unchanged), 5);

    // Removing the row retires it; the Zone sequence still moves so the
    // broker learns the relationship is gone.
    match publish(&store, DesiredMutation::Remove(key("data"))).await {
        CommitOutcome::Committed(committed) => {
            assert!(committed.publication.rows.is_empty());
            assert_eq!(committed.publication.removed, vec![key("data")]);
        }
        other => panic!("a removal commits: {other:?}"),
    }
    assert!(
        store.desired_row(key("data")).await.is_err(),
        "a removed row is gone, so its revision is not readable"
    );
    assert_eq!(
        store.zone_sequence(ZONE).await.unwrap().get(),
        8,
        "every staged candidate consumes its Zone sequence, including the two that changed nothing"
    );
}

/// Runtime status has no write surface in this store, and the desired
/// revision cannot drift under observation: reads leave the row's revision,
/// its digest, and the Zone's accepted sequence untouched.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn reads_and_status_shaped_traffic_never_move_the_desired_revision() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;

    let before = store.desired_row(key("data")).await.unwrap();
    let accepted_before = store.accepted_cursor(ZONE).await.unwrap();

    for _ in 0..8 {
        store.desired_row(key("data")).await.unwrap();
        store.desired_rows(SpecSelector::default()).await.unwrap();
        store.zone_sequence(ZONE).await.unwrap();
        store.zone_recovery(ZONE).await.unwrap();
        store.history(16).await.unwrap();
    }
    let after = store.desired_row(key("data")).await.unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.digest, before.digest);
    assert_eq!(store.accepted_cursor(ZONE).await.unwrap(), accepted_before);
    assert_eq!(store.zone_sequence(ZONE).await.unwrap().get(), 1);

    // Every audit record is a desired mutation. Nothing status-shaped can
    // appear, because no request carries a status payload.
    let history = store.history(64).await.unwrap();
    assert!(history.iter().all(|record| record.operation.starts_with("authority.")), "{history:?}");
}

// ---------------------------------------------------------------------------
// Scenario 2: one transaction for the row, the revision, and the outbox
// ---------------------------------------------------------------------------

/// Inject a write failure on one column of the commit transaction.
///
/// A SQLite trigger that aborts the statement is the hermetic way to make a
/// durable write fail: the store cannot tell it from a disk failure, and the
/// database is this test's own temporary file.
fn inject_write_failure(path: &Path, table: &str, operation: &str) {
    let conn = rusqlite::Connection::open(database(path)).expect("open for injection");
    conn.execute_batch(&format!(
        "CREATE TRIGGER injected_{table}_{operation} BEFORE {operation} ON {table} \
         BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;"
    ))
    .expect("install failure trigger");
}

fn drop_write_failure(path: &Path, table: &str, operation: &str) {
    let conn = rusqlite::Connection::open(database(path)).expect("open for cleanup");
    conn.execute_batch(&format!("DROP TRIGGER injected_{table}_{operation};"))
        .expect("drop failure trigger");
}

/// KTD6: the desired row, its revision, its audit record, and its outbox
/// entry commit together. A failure at any one of those three columns leaves
/// the desired row exactly as it was and no outbox entry behind, and the
/// same transaction commits cleanly once the failure is gone.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn desired_row_and_outbox_commit_atomically_under_injected_write_failures() {
    for (table, operation) in [("resources", "UPDATE"), ("audit_log", "INSERT"), ("publication_outbox", "INSERT")] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("authority.db");
        let store = SpecStore::open(&path).unwrap();
        publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
        let before = store.desired_row(key("data")).await.unwrap();

        let staged = store
            .stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v2")))
            .await
            .expect("stage the second mutation");
        store.record_prepared(staged.transaction, "prepared-2").await.unwrap();
        // The sequence was consumed by the reservation; the failed commit
        // must not consume another.
        let reserved = store.zone_sequence(ZONE).await.unwrap();
        inject_write_failure(&path, table, operation);
        let failed = store.commit_mutation(staged.transaction).await;
        drop_write_failure(&path, table, operation);

        assert!(failed.is_err(), "the injected {operation} on {table} must fail the commit");

        // Proved absent: the desired row did not move.
        let after = store.desired_row(key("data")).await.unwrap();
        assert_eq!(after.row.spec, before.row.spec, "{table} {operation}: spec must not have moved");
        assert_eq!(after.revision, before.revision, "{table} {operation}: revision must not have moved");
        assert_eq!(
            store.zone_sequence(ZONE).await.unwrap(),
            reserved,
            "{table} {operation}: the failed commit consumed no sequence"
        );

        // The transaction is still replayable and the recovery table still
        // names it, so the failure is a retry, not a lost mutation.
        let recovery = store.zone_recovery(ZONE).await.unwrap();
        assert!(recovery.has_outstanding(), "{table} {operation}: the transaction stays outstanding");

        // The identical candidate commits exactly once when the failure is gone.
        match store.commit_mutation(staged.transaction).await.unwrap() {
            CommitOutcome::Committed(committed) => {
                assert_eq!(revision_of(&committed.publication.rows[0]), 2);
                assert_eq!(committed.publication.rows[0].row.spec, b"spec-v2");
            }
            other => panic!("{table} {operation}: the replay must commit: {other:?}"),
        }
        let replay = store.commit_mutation(staged.transaction).await.unwrap();
        assert!(
            matches!(replay, CommitOutcome::AlreadyCommitted(_)),
            "{table} {operation}: a second replay must not apply the mutation again"
        );
        assert_eq!(store.desired_row(key("data")).await.unwrap().revision.get(), 2);
    }
}

// ---------------------------------------------------------------------------
// Scenario 3: recovery of each durable transaction identity (AE18)
// ---------------------------------------------------------------------------

/// AE18 at the store's boundary: after a restart the staged, prepared, and
/// committed-but-unacknowledged identities are all still readable, each with
/// the recovery decision its durable facts imply, and replaying any of them
/// applies the candidate exactly once.
///
/// Each Zone carries at most one outstanding transaction, so the three
/// failure boundaries are three Zones - which is also how they occur in
/// practice, because a Zone's mutations queue behind its pending fence.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn staged_prepared_and_committed_transactions_recover_after_restart_without_duplicate_mutation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("authority.db");
    let (staged_id, prepared_id, committed_id) = {
        let store = SpecStore::open(&path).unwrap();

        // Failure boundary 1: staged, no fence, no desired row.
        let staged = store
            .stage_mutation(DesiredMutation::Ensure(rowed("staged-zone", "first", b"spec-v1")))
            .await
            .unwrap();
        let staged_id = staged.transaction;

        // Failure boundary 2: the broker fenced the Zone, nothing committed.
        let prepared = store
            .stage_mutation(DesiredMutation::Ensure(rowed("prepared-zone", "second", b"spec-v1")))
            .await
            .unwrap();
        store.record_prepared(prepared.transaction, "prepared-second").await.unwrap();
        let prepared_id = prepared.transaction;

        // Failure boundary 3: the desired rows and the outbox entry are
        // committed, the broker has not acknowledged.
        let committed = store
            .stage_mutation(DesiredMutation::Ensure(rowed("committed-zone", "third", b"spec-v1")))
            .await
            .unwrap();
        store.record_prepared(committed.transaction, "prepared-third").await.unwrap();
        store.commit_mutation(committed.transaction).await.unwrap();
        let committed_id = committed.transaction;

        // Abandoning the store handle mid-flight is the crash: no graceful
        // checkpoint, no chance to clean up.
        std::mem::forget(store);
        (staged_id, prepared_id, committed_id)
    };

    let store = SpecStore::open(&path).unwrap();
    let incarnation = store.store_incarnation().await.unwrap();

    // Boundary 1: the staged candidate may exist and the desired rows are
    // unchanged, so recovery must resume it or discard it.
    let staged_recovery = store.zone_recovery("staged-zone").await.unwrap();
    assert_eq!(staged_recovery.incarnation, incarnation);
    assert_eq!(staged_recovery.accepted, None);
    let [(_, staged_decision)] = &staged_recovery.transactions[..] else {
        panic!("the staged transaction survives the restart: {:?}", staged_recovery.transactions);
    };
    assert_eq!(staged_decision.transaction().transaction, staged_id);
    assert!(matches!(
        staged_decision,
        TransactionRecovery::ResumeOrDiscard { .. }
    ));
    assert!(store.desired_row(keyed("staged-zone", "first")).await.is_err());

    // Boundary 2: the broker holds a prepared identity and this store a
    // staged candidate, so the exact candidate is replayable or cancellable.
    let prepared_recovery = store.zone_recovery("prepared-zone").await.unwrap();
    let [(_, prepared_decision)] = &prepared_recovery.transactions[..] else {
        panic!("the prepared transaction survives: {:?}", prepared_recovery.transactions);
    };
    assert!(matches!(
        prepared_decision,
        TransactionRecovery::ReplayOrCancel { .. }
    ));
    assert!(store.desired_row(keyed("prepared-zone", "second")).await.is_err());

    // Boundary 3: the desired rows and the outbox entry are committed, so
    // the exact CommitChange is replayed and nothing is applied twice.
    let committed_recovery = store.zone_recovery("committed-zone").await.unwrap();
    assert_eq!(committed_recovery.incarnation, incarnation);
    let [(_, committed_decision)] = &committed_recovery.transactions[..] else {
        panic!("the committed transaction survives: {:?}", committed_recovery.transactions);
    };
    let TransactionRecovery::ReplayCommit { publication, transaction } = committed_decision else {
        panic!("a committed transaction replays its exact publication: {committed_decision:?}");
    };
    assert_eq!(transaction.transaction, committed_id);
    assert_eq!(publication.sequence.get(), 1);
    assert_eq!(publication.rows[0].row.key, keyed("committed-zone", "third"));
    assert_eq!(revision_of(&publication.rows[0]), 1);
    assert!(store.accepted_cursor("committed-zone").await.unwrap().is_none());

    match store.commit_mutation(committed_id).await.unwrap() {
        CommitOutcome::AlreadyCommitted(replayed) => {
            assert_eq!(replayed.publication.rows, publication.rows);
            assert_eq!(replayed.candidate, publication.candidate);
            assert_eq!(replayed.sequence, publication.sequence);
        }
        other => panic!("a committed transaction replays, it does not re-apply: {other:?}"),
    }

    // Replaying the prepared candidate commits it exactly once.
    match store.commit_mutation(prepared_id).await.unwrap() {
        CommitOutcome::Committed(committed) => {
            assert_eq!(committed.publication.rows[0].row.key, keyed("prepared-zone", "second"));
        }
        other => panic!("the prepared candidate commits on replay: {other:?}"),
    }
    assert!(
        matches!(
            store.commit_mutation(prepared_id).await.unwrap(),
            CommitOutcome::AlreadyCommitted(_)
        ),
        "the replayed candidate is never applied twice"
    );

    // Discarding the staged candidate instead: it committed no desired row,
    // so cancelling leaves no trace of it.
    assert!(matches!(
        store.cancel_transaction(staged_id).await.unwrap().state,
        PublicationState::Cancelled
    ));
    assert!(store.desired_row(keyed("staged-zone", "first")).await.is_err());
    assert!(matches!(
        store.commit_mutation(staged_id).await.unwrap_err(),
        SpecStoreError::TransactionStateConflict { state: "cancelled", .. }
    ));
    assert!(!store.zone_recovery("staged-zone").await.unwrap().has_outstanding());

    // One row per surviving Zone, and no outstanding transaction anywhere.
    assert_eq!(store.desired_rows(SpecSelector::default()).await.unwrap().len(), 2);
    for zone in ["prepared-zone", "committed-zone"] {
        let recovery = store.zone_recovery(zone).await.unwrap();
        assert!(recovery.has_outstanding(), "{zone} still owes an outcome");
    }
    assert!(!store.zone_recovery("staged-zone").await.unwrap().has_outstanding());
}

/// The Zone sequence and the accepted cursor move only where the plan allows:
/// the cursor follows an acknowledgment, and an acknowledgment that names
/// facts this store never committed is refused instead of published.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_acknowledgment_must_name_the_committed_transaction() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let staged = store
        .stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v1")))
        .await
        .unwrap();
    store.record_prepared(staged.transaction, "prepared-1").await.unwrap();

    // Nothing has committed, so there is nothing to acknowledge.
    let early = store
        .acknowledge(AcceptedPublication {
            transaction: staged.transaction,
            zone: staged.zone.clone(),
            incarnation: staged.incarnation.clone(),
            sequence: staged.sequence,
            candidate: staged.candidate.clone(),
        })
        .await;
    assert!(
        matches!(early, Err(SpecStoreError::TransactionStateConflict { state: "prepared", .. })),
        "an uncommitted candidate cannot be acknowledged"
    );

    let committed = match store.commit_mutation(staged.transaction).await.unwrap() {
        CommitOutcome::Committed(committed) => committed,
        other => panic!("commit: {other:?}"),
    };

    // A different candidate digest for the same transaction is refused.
    let mismatched = store
        .acknowledge(AcceptedPublication {
            transaction: committed.transaction,
            zone: committed.zone.clone(),
            incarnation: committed.incarnation.clone(),
            sequence: committed.sequence,
            candidate: DesiredMutation::Ensure(row("data", b"other")).candidate_digest(),
        })
        .await;
    assert!(
        matches!(mismatched, Err(SpecStoreError::PublicationMismatch { .. })),
        "an acknowledgment for other bytes is refused"
    );

    // An unknown transaction is refused, not treated as accepted.
    let unknown = store
        .acknowledge(AcceptedPublication {
            transaction: TransactionId::from_bytes([7; 16]),
            zone: committed.zone.clone(),
            incarnation: committed.incarnation.clone(),
            sequence: committed.sequence,
            candidate: committed.candidate.clone(),
        })
        .await;
    assert!(matches!(unknown, Err(SpecStoreError::TransactionNotFound { .. })));

    let acknowledgment = AcceptedPublication {
        transaction: committed.transaction,
        zone: committed.zone.clone(),
        incarnation: committed.incarnation.clone(),
        sequence: committed.sequence,
        candidate: committed.candidate.clone(),
    };
    let cursor = store.accepted_cursor(ZONE).await.unwrap();
    assert!(cursor.is_none(), "the cursor does not move before the acknowledgment");
    let accepted = store.acknowledge(acknowledgment.clone()).await.unwrap();
    assert_eq!(accepted.sequence, committed.sequence);
    assert_eq!(accepted.digest, committed.candidate);

    // The acknowledgment is idempotent, so a lost response is a safe retry.
    let repeated = store.acknowledge(acknowledgment).await.unwrap();
    assert_eq!(repeated.sequence, accepted.sequence);
    assert_eq!(store.accepted_cursor(ZONE).await.unwrap(), Some(accepted));
}

/// An authority mutation for a Zone with an outstanding transaction is
/// refused rather than allowed to overtake its fence, and the accepted
/// cursor still resolves as a lower bound afterwards.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_second_mutation_queues_behind_the_outstanding_transaction() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let staged = store
        .stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v1")))
        .await
        .unwrap();

    let overtaking = store.stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v2"))).await;
    assert!(
        matches!(
            overtaking,
            Err(SpecStoreError::ZoneTransactionOutstanding { .. })
        ),
        "a queued mutation must not overtake the pending fence"
    );

    // A different Zone keeps its own sequence, so Zones do not block one
    // another.
    let other = store
        .stage_mutation(DesiredMutation::Ensure(StoredDesiredResource {
            key: ResourceKey::new(OTHER_ZONE, "Volume", "data"),
            ..row("data", b"spec-v1")
        }))
        .await
        .unwrap();
    assert_eq!(other.sequence.get(), 1);

    store.cancel_transaction(staged.transaction).await.unwrap();
    let retried = store
        .stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v2")))
        .await
        .unwrap();
    assert_eq!(
        retried.sequence.get(),
        2,
        "a cancelled candidate keeps its consumed sequence: it is never handed out twice"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4: counters fail closed
// ---------------------------------------------------------------------------

/// The Zone sequence and one row's revision refuse the mutation when they
/// cannot advance, instead of wrapping into a value that looks older than
/// what it replaced.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_exhausted_counter_fails_explicitly_instead_of_wrapping() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("authority.db");
    let store = SpecStore::open(&path).unwrap();
    publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
    let ceiling = i64::MAX;

    {
        let conn = rusqlite::Connection::open(database(&path)).unwrap();
        conn.execute("UPDATE zone_desired_sequence SET sequence = ?1", [ceiling]).unwrap();
        conn.execute("UPDATE resources SET desired_revision = ?1", [ceiling]).unwrap();
    }

    // The Zone sequence fails first, because staging reserves it.
    let exhausted_zone = store.stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v2"))).await;
    assert!(
        matches!(exhausted_zone, Err(SpecStoreError::ZoneSequenceExhausted { .. })),
        "an exhausted Zone sequence refuses the mutation"
    );
    assert_eq!(store.zone_sequence(ZONE).await.unwrap().get(), ceiling as u64);

    // With the Zone sequence usable again, the row's own revision is what
    // refuses the mutation - and it refuses it while the candidate is staged,
    // because that is where the revision it would commit at is decided.
    {
        let conn = rusqlite::Connection::open(database(&path)).unwrap();
        conn.execute("UPDATE zone_desired_sequence SET sequence = 1", []).unwrap();
    }
    let exhausted_row = store.stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v2"))).await;
    assert!(
        matches!(exhausted_row, Err(SpecStoreError::RowRevisionExhausted { .. })),
        "an exhausted row revision refuses the mutation"
    );
    assert!(
        !store.zone_recovery(ZONE).await.unwrap().has_outstanding(),
        "a refused candidate reserves nothing, so the Zone is not left fenced"
    );

    // Proved absent: the refused mutation wrote nothing.
    let row = store.desired_row(key("data")).await.unwrap();
    assert_eq!(row.row.spec, b"spec-v1");
    assert_eq!(revision_of(&row), ceiling as u64);

    // A stored counter that is not a counter at all is refused on read, not
    // reinterpreted as a nearby revision.
    {
        let conn = rusqlite::Connection::open(database(&path)).unwrap();
        conn.execute("UPDATE resources SET desired_revision = -1", []).unwrap();
    }
    assert!(
        matches!(
            store.desired_row(key("data")).await,
            Err(SpecStoreError::CorruptCounter { .. })
        ),
        "a negative counter column is refused"
    );
}

// ---------------------------------------------------------------------------
// Format, freshness, and the store-generation fence
// ---------------------------------------------------------------------------

/// The clean break: a store written by an earlier release is refused rather
/// than converted, and the journal protocol is the only write path a
/// committed store has at all.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_existing_production_store_is_refused_rather_than_converted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    {
        // The shape an earlier release left behind: desired rows with no
        // revision column and no publication journal.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE resources (\
                 zone TEXT NOT NULL, type TEXT NOT NULL, name TEXT NOT NULL, uid BLOB NOT NULL,\
                 generation INTEGER NOT NULL, owner_uid BLOB, provenance TEXT NOT NULL,\
                 deleting INTEGER NOT NULL DEFAULT 0, spec BLOB NOT NULL, metadata BLOB NOT NULL,\
                 created_at INTEGER NOT NULL, PRIMARY KEY (zone, type, name));\
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let refused = SpecStore::open(&path).map(|_| ());
    assert!(
        matches!(
            refused,
            Err(SpecStoreError::Schema(
                d2b_resource_runtime::schema::SchemaError::RefusedSchema { user_version: 1 }
            ))
        ),
        "an old store is refused, not migrated: {refused:?}"
    );
}

/// KTD6's store half: no store transaction is open while the caller's broker
/// I/O runs. A second connection can take the database's write lock
/// immediately after each protocol step returns, which it could not do if the
/// step had left a transaction open.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn no_store_transaction_is_open_across_the_broker_wait() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("authority.db");
    let store = SpecStore::open(&path).unwrap();

    let staged = store
        .stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v1")))
        .await
        .unwrap();
    take_the_write_lock(&path, "staged");

    store.record_prepared(staged.transaction, "prepared-1").await.unwrap();
    take_the_write_lock(&path, "prepared");

    store.commit_mutation(staged.transaction).await.unwrap();
    take_the_write_lock(&path, "committed");

    store
        .acknowledge(AcceptedPublication {
            transaction: staged.transaction,
            zone: staged.zone.clone(),
            incarnation: staged.incarnation.clone(),
            sequence: staged.sequence,
            candidate: staged.candidate.clone(),
        })
        .await
        .unwrap();
    take_the_write_lock(&path, "acknowledged");
}

/// The exact desired state one admitted effect is fenced against names the
/// store generation, the row, its revision, and its digest - and a rejected
/// identity is refused rather than approximated.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_committed_row_names_its_exact_freshness_tuple() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
    let incarnation = store.store_incarnation().await.unwrap();
    let committed = store.desired_row(key("data")).await.unwrap();

    let freshness = row_freshness(&committed, &incarnation).expect("canonical row identity");
    assert_eq!(freshness.store_incarnation(), &incarnation);
    assert_eq!(freshness.desired_revision(), committed.revision);
    assert_eq!(freshness.desired_digest(), &committed.digest);
    assert!(freshness.same_store(&row_freshness(
        &store.desired_row(key("data")).await.unwrap(),
        &incarnation
    )
    .unwrap()));

    let mut unrepresentable = committed.row.clone();
    unrepresentable.key.type_name = "not a type".to_owned();
    let broken = DesiredRow { row: unrepresentable, ..committed };
    assert!(
        row_freshness(&broken, &incarnation).is_err(),
        "a row whose identity is not canonical is refused, not approximated"
    );
}

/// The store generation is minted once and is not a counter: reopening the
/// same database reuses it, while a second database mints its own.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_store_incarnation_is_minted_once_per_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("authority.db");
    let first = {
        let store = SpecStore::open(&path).unwrap();
        let incarnation = store.store_incarnation().await.unwrap();
        assert!(incarnation.as_str().starts_with("store-"));
        drop(store);
        incarnation
    };
    let reopened = SpecStore::open(&path).unwrap();
    assert_eq!(reopened.store_incarnation().await.unwrap(), first);

    let other_dir = TempDir::new().unwrap();
    let other = open(&other_dir);
    assert_ne!(other.store_incarnation().await.unwrap(), first);
}

/// Taking the database's write lock proves the store holds no transaction at
/// this moment. A retained write transaction would time out here instead.
fn take_the_write_lock(path: &Path, boundary: &str) {
    let conn = rusqlite::Connection::open(database(path)).expect("open for lock probe");
    conn.busy_timeout(std::time::Duration::from_millis(250)).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap_or_else(|error| {
        panic!("after {boundary} the store must hold no open transaction: {error}")
    });
    conn.execute_batch("COMMIT").unwrap();
}
