//! A publication that failed leaves the Zone able to publish again.
//!
//! One Zone has at most one outstanding publication transaction, so a
//! publication that fails at any step after staging holds that Zone's single
//! slot. Releasing it was the restart path's job alone, so one failed
//! publication left the Zone refusing every later mutation by the leaked
//! transaction's identity - the store's one-outstanding rule working exactly as
//! declared, with no protocol path back.
//!
//! Both scenarios go through [`SpecStore::open`] and the production
//! [`d2b_resource_runtime::publish`] - the same write path the manager drives -
//! rather than a fixture that stages a store some other way.
//!
//! 1. The candidate a refused fence left staged is released by the next
//!    publication, and that publication commits its own row. The refusal the
//!    first attempt reported is unchanged and still names its own cause.
//! 2. A transaction staged by some other writer is released the same way: the
//!    recovery table answers what it owes before the next candidate stages, so
//!    nothing queues behind a transaction no other step will release.

use d2b_resource_runtime::authority_journal::DesiredMutation;
use d2b_resource_runtime::spec_store::{
    ResourceKey, ResourceProvenance, SpecStore, StoredDesiredResource,
};
use d2b_resource_runtime::test_support::{RecordingPublisher, RefusingPublisher};
use tempfile::TempDir;

const ZONE: &str = "work";

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

/// The refusal is still a refusal: a broker that will not freeze the Zone
/// reports its own refusal and commits nothing. What changes is that the
/// refused attempt no longer takes the Zone's next mutation down with it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_refused_fence_does_not_wedge_the_zone() {
    let dir = TempDir::new().unwrap();
    let store = open_production(&dir);

    let refused = store
        .publish(
            DesiredMutation::Ensure(row("first", b"spec-v1")),
            RefusingPublisher::new("broker refused the fence").as_ref(),
        )
        .await;
    assert!(
        refused.is_err(),
        "a broker that will not freeze the Zone refuses this publication"
    );
    assert!(
        store.get(key("first")).await.is_err(),
        "the refused candidate committed no desired row"
    );

    // The same Zone, the same store, a healthy authority: this is the mutation
    // a refused attempt used to refuse for the rest of the daemon's life.
    let publisher = RecordingPublisher::new();
    let committed = store
        .publish(
            DesiredMutation::Ensure(row("second", b"spec-v1")),
            publisher.as_ref(),
        )
        .await
        .expect("the next publication resolves what the refused one left and commits");
    assert!(committed.ensure().is_some(), "the second row committed");

    let recovery = store.zone_recovery(ZONE).await.expect("the Zone answers what it owes");
    assert!(
        !recovery.has_outstanding(),
        "a settled publication leaves the Zone owing nothing: {recovery:?}"
    );
}

/// A transaction no publication of its own left staged is released by name
/// before the next candidate stages, so nothing waits behind it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_outstanding_candidate_is_released_before_the_next_publication() {
    let dir = TempDir::new().unwrap();
    let store = open_production(&dir);
    let leaked = store
        .stage_mutation(DesiredMutation::Ensure(row("leaked", b"spec-v1")))
        .await
        .expect("an earlier attempt stages its candidate and stops there")
        .transaction;
    assert!(
        store.get(key("leaked")).await.is_err(),
        "the staged candidate committed no desired row"
    );

    let publisher = RecordingPublisher::new();
    store
        .publish(
            DesiredMutation::Ensure(row("next", b"spec-v1")),
            publisher.as_ref(),
        )
        .await
        .unwrap_or_else(|error| {
            panic!("the publication released transaction {leaked} instead of refusing: {error}")
        });
    assert!(
        store.get(key("leaked")).await.is_err(),
        "the released candidate committed nothing: it was cancelled, not adopted"
    );
    assert!(
        store.get(key("next")).await.is_ok(),
        "this publication committed its own row"
    );
}
