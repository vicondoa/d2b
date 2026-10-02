//! A publication transaction outstanding across a restart is recovered
//! before the foundation seed publishes.
//!
//! The unit this covers is "the foundation seed is not the first writer to
//! the Zone". The scenario goes through [`SpecStore::open`] - the production
//! open path, the same call the plane makes - rather than a fixture that
//! stages a store some other way.
//!
//! A staged candidate left outstanding by a previous boot is named by the
//! Zone's own recovery answer, so the adoption this file exercises settles it
//! and the seed's publish then commits. The publication write path resolves
//! the same table on its own before it stages anything; that is covered where
//! it lives, in `authority_runtime_recovery.rs`.

use d2b_resource_runtime::authority_journal::{DesiredMutation, TransactionRecovery};
use d2b_resource_runtime::spec_store::{
    ResourceKey, ResourceProvenance, SpecStore, StoredDesiredResource,
};
use d2b_resource_runtime::test_support::RecordingPublisher;
use d2b_resource_runtime::TransactionId;
use tempfile::TempDir;

const ZONE: &str = "system";

fn key(name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, "SeccompProfile", name)
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
        provenance: ResourceProvenance::Nix,
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

/// A restart adopts the transaction the previous boot left outstanding, and
/// the seed's publish then succeeds.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_outstanding_transaction_is_adopted_before_the_seed_publishes() {
    let dir = TempDir::new().unwrap();
    let outstanding: TransactionId = {
        let store = open_production(&dir);
        let staged = store
            .stage_mutation(DesiredMutation::Ensure(row("seeded", b"spec-v1")))
            .await
            .expect("the previous boot stages its candidate");
        staged.transaction
        // The boot dies here: the candidate is durable and nothing settled it.
    };

    // The reopen through the production open path IS the restart: the store
    // above went out of scope with its writer thread.
    let store = open_production(&dir);
    let recovery = store.zone_recovery(ZONE).await.expect("the Zone answers what it owes");
    let decision = recovery
        .transactions
        .iter()
        .find(|(id, _)| *id == outstanding)
        .map(|(_, decision)| decision)
        .expect("recovery names the transaction the previous boot left outstanding");
    assert!(
        matches!(decision, TransactionRecovery::ResumeOrDiscard { .. }),
        "a staged candidate that committed nothing is released by name: {decision:?}"
    );

    // Adoption: the outstanding transaction is explicitly released, never
    // abandoned. This is the production Zone recovery the plane runs before
    // the seed publishes, not a local re-implementation of it.
    let publisher = RecordingPublisher::new();
    d2b_resource_runtime::adopt_outstanding(&store, ZONE, publisher.as_ref())
        .await
        .expect("adoption resolves the transaction the previous boot left");

    let outcome = store
        .publish(
            DesiredMutation::Ensure(row("seeded", b"spec-v1")),
            publisher.as_ref(),
        )
        .await
        .expect("the seed publishes once the Zone owes nothing");
    assert!(outcome.ensure().is_some(), "the seed committed its desired row");

    // And the Zone owes nothing afterwards: the next boot adopts nothing.
    let recovery = store.zone_recovery(ZONE).await.expect("the Zone answers what it owes");
    assert!(
        !recovery.has_outstanding(),
        "a settled publication leaves the Zone owing nothing: {recovery:?}"
    );
}