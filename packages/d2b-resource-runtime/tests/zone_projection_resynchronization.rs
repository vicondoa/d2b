//! The Zone's durable projection, and the reconciliation it exists for.
//!
//! A restarted broker refuses its own cached authority and serves nothing
//! until the manager shows it the projection it already accepted. That document
//! has to be rebuildable from the store alone, after a restart, by a different
//! process: the accepted cursor, every committed row at the revision and digest
//! it committed at, and - the part a binding row cannot supply from its own
//! bytes - the committed identity its relationship key folds in.
//!
//! These cases go through [`SpecStore::open`] and the production
//! [`d2b_resource_runtime::publish`], then read the projection back through the
//! production [`d2b_resource_runtime::resynchronize`]. What is pinned is that a
//! reopened store presents the same projection its previous boot published -
//! including a `VolumeBinding` row's resolved source and consumer identity - and
//! that the reconciliation runs against the store, never against a manager's
//! memory.

use d2b_contracts_resource::v3::volume::AttachmentAccess;
use d2b_contracts_resource::v3::{
    BindingArbitration, BindingRealizationFacet, BindingSourceDecision, CanonicalJsonObject,
    DesiredDigest, RequestedRights, ResourceRef, VolumeBindingSpec,
    volume_binding::VolumePresentation,
};
use d2b_resource_runtime::authority_journal::DesiredMutation;
use d2b_resource_runtime::spec_store::{
    ResourceKey, ResourceProvenance, SpecStore, StoredDesiredResource,
};
use d2b_resource_runtime::test_support::{Recorded, RecordingPublisher, RefusingPublisher};
use d2b_resource_runtime::{adopt_outstanding, publish, resynchronize};
use tempfile::TempDir;

const ZONE: &str = "work";

fn key(type_name: &str, name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, type_name, name)
}

fn row(type_name: &str, name: &str, seed: u8, spec: &[u8]) -> StoredDesiredResource {
    StoredDesiredResource {
        key: key(type_name, name),
        uid: [seed; 16],
        generation: 0,
        owner_uid: None,
        provenance: ResourceProvenance::Api,
        deleting: false,
        spec: spec.to_vec(),
        metadata: b"meta".to_vec(),
        created_at: 0,
    }
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture references are canonical")
}

/// The exact canonical bytes a contract value commits as.
fn canonical_bytes(spec: &serde_json::Value) -> Vec<u8> {
    CanonicalJsonObject::parse(&serde_json::to_vec(spec).expect("the fixture row renders"))
        .expect("the fixture row is a canonical JSON object")
        .to_canonical_bytes()
}

fn json(value: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&value).expect("the fixture row renders")
}

/// A `VolumeBinding/data` row naming a source and a consumer by reference.
/// Its own bytes deliberately do NOT repeat either row's identity: a
/// relationship key is over committed identity, so that identity has to come
/// from the store's committed rows.
fn volume_binding_row() -> StoredDesiredResource {
    let decision = BindingSourceDecision::new(
        vec![RequestedRights::Consume],
        BindingArbitration::Shared,
        vec![BindingRealizationFacet::FilesystemPresentation],
    )
    .expect("the source decision is well formed");
    let spec = VolumeBindingSpec::new(
        reference("Volume/data"),
        reference("Guest/work"),
        "root",
        AttachmentAccess::ReadWrite,
        VolumePresentation::filesystem("/mnt/data").expect("a consumer destination"),
        "root",
        decision,
    )
    .expect("the binding row is well formed");
    row(
        "VolumeBinding",
        "data",
        0x40,
        &canonical_bytes(&serde_json::to_value(&spec).expect("the binding renders")),
    )
}

/// The source and consumer rows the binding names. They are committed BEFORE
/// the binding, which is the only order in which a relationship key can be
/// folded at all.
fn source_row() -> StoredDesiredResource {
    row("Volume", "data", 0x41, &json(serde_json::json!({ "kind": "local" })))
}

fn consumer_row() -> StoredDesiredResource {
    row("Guest", "work", 0x42, &json(serde_json::json!({ "session": "work" })))
}

/// The projection a Zone that has published nothing is at: the initial cursor,
/// whose digest is the digest of no committed bytes.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_unpublished_zone_is_projected_at_the_initial_cursor() {
    let dir = TempDir::new().unwrap();
    let store = SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("the store opens");

    let projection = store
        .zone_projection(ZONE)
        .await
        .expect("the store answers a projection");
    assert_eq!(projection.zone, ZONE);
    assert!(projection.rows.is_empty(), "no row has committed");
    assert_eq!(
        projection.accepted.sequence.get(),
        0,
        "an unpublished Zone is at sequence zero"
    );
    assert_eq!(
        projection.accepted.digest,
        DesiredDigest::of(&[]),
        "sequence zero has no committed bytes behind it"
    );
    assert!(projection.outstanding.is_none(), "nothing is owed");
}

/// A committed binding row's resolved relationship identity survives into the
/// projection, which is what lets the broker fold the same key the manager did
/// and lets a reconciliation restate it instead of presenting an unresolved
/// relationship the broker must refuse.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_committed_binding_row_is_projected_with_its_resolved_identity() {
    let dir = TempDir::new().unwrap();
    let store = SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("the store opens");
    let publisher = RecordingPublisher::new();
    for desired in [source_row(), consumer_row(), volume_binding_row()] {
        publish(&store, DesiredMutation::Ensure(desired), publisher.as_ref())
            .await
            .expect("the row publishes");
    }

    let projection = store
        .zone_projection(ZONE)
        .await
        .expect("the store answers a projection");
    let binding = projection
        .rows
        .iter()
        .find(|published| published.row.key.type_name == "VolumeBinding")
        .expect("the projection carries the binding row");
    assert_eq!(
        binding.source_uid,
        Some([0x41; 16]),
        "the binding carries the committed identity of the source it names"
    );
    assert_eq!(
        binding.consumer_uid,
        Some([0x42; 16]),
        "the binding carries the committed identity of the consumer it names"
    );

    // A row that declares no relationship carries no identity: the identity is
    // resolved from the Zone's committed rows, never invented per row kind.
    let volume = projection
        .rows
        .iter()
        .find(|published| published.row.key.type_name == "Volume")
        .expect("the projection carries the source row");
    assert!(
        volume.source_uid.is_none() && volume.consumer_uid.is_none(),
        "a row outside the binding families resolves no relationship"
    );
}

/// A reopened store presents exactly the projection its previous boot
/// published, and the reconciliation the manager drives reads it from the store
/// rather than from anything a manager remembers.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_reopened_store_presents_the_same_projection() {
    let dir = TempDir::new().unwrap();
    let publisher = RecordingPublisher::new();
    let before = {
        let store = SpecStore::open(dir.path().join("spec-store.sqlite3"))
            .expect("the store opens");
        for desired in [
            source_row(),
            consumer_row(),
            volume_binding_row(),
            row("Process", "shell", 0x43, &json(serde_json::json!({ "declared": "shell" }))),
        ] {
            publish(&store, DesiredMutation::Ensure(desired), publisher.as_ref())
                .await
                .expect("the row publishes");
        }
        store
            .zone_projection(ZONE)
            .await
            .expect("the store answers a projection")
    };

    // The store goes away and a new one opens the same database: this is the
    // restart the reconciliation exists for.
    let reopened = SpecStore::open(dir.path().join("spec-store.sqlite3"))
        .expect("the store reopens over its own database");
    let after = reopened
        .zone_projection(ZONE)
        .await
        .expect("the reopened store answers a projection");
    assert_eq!(
        before.rows, after.rows,
        "a reopened store presents the rows its previous boot published"
    );
    assert_eq!(
        before.accepted, after.accepted,
        "a reopened store presents the cursor its previous boot acknowledged"
    );
    assert_eq!(
        before.transaction, after.transaction,
        "the reconciliation identity is derived from that cursor, so a retry \
         re-presents it rather than stacking a second one"
    );

    // And the reconciliation the manager drives reads exactly that projection.
    let recording = RecordingPublisher::new();
    resynchronize(&reopened, ZONE, recording.as_ref())
        .await
        .expect("the reconciliation is served");
    assert_eq!(
        recording.recorded().await,
        vec![Recorded::Resynchronized {
            zone: ZONE.to_owned(),
            sequence: after.accepted.sequence,
            rows: after.rows.len(),
            outstanding: after.outstanding,
        }],
        "the reconciliation carries the store's own projection"
    );
}

/// A reconciliation is the manager's LAST restart step: adoption settles what
/// the previous boot owed first, so the reconciliation restates a cursor the
/// adoption actually moved rather than one it is about to leave behind.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_released_candidate_leaves_the_reconciliation_carrying_nothing() {
    let dir = TempDir::new().unwrap();
    let store = SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("the store opens");
    // A staged candidate that reached no fence is released by the recovery
    // table, so the reconciliation that follows carries no identity forward.
    let refused = store
        .publish(
            DesiredMutation::Ensure(row("Volume", "first", 0x51, b"spec-v1")),
            RefusingPublisher::new("no broker").as_ref(),
        )
        .await;
    assert!(refused.is_err(), "the fence was refused");

    let publisher = RecordingPublisher::new();
    adopt_outstanding(&store, ZONE, publisher.as_ref())
        .await
        .expect("the staged candidate is released");
    resynchronize(&store, ZONE, publisher.as_ref())
        .await
        .expect("the reconciliation is served");
    let projection = store.zone_projection(ZONE).await.expect("a projection");
    assert_eq!(
        publisher.recorded().await.last(),
        Some(&Recorded::Resynchronized {
            zone: ZONE.to_owned(),
            sequence: projection.accepted.sequence,
            rows: 0,
            outstanding: None,
        }),
        "a released candidate leaves the reconciliation carrying nothing"
    );
}
