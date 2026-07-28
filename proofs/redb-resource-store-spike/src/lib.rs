#![forbid(unsafe_code)]

mod actor;
mod codec;
mod disk;
mod model;
pub mod schema;

pub use actor::{ActorStats, Hint, HintKind, Store, Watch};
pub use disk::{
    CrashRecovery, crash_database_path, prepare_crash_database, run_crash_transaction,
    verify_crash_database,
};
pub use model::{
    ChangeBatch, ChangeEntry, Mutation, Resource, ResourceKey, StoreError, StoreResult,
    WriteReceipt, synthetic_resource,
};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

pub fn fixture_path(label: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("spike-data");
    std::fs::create_dir_all(&directory).expect("create spike data directory");
    directory.join(format!(
        "{label}-{}-{}.redb",
        std::process::id(),
        NEXT_PATH.fetch_add(1, Ordering::Relaxed)
    ))
}

pub async fn put_with_backpressure_retry(
    store: &Store,
    principal: &str,
    mutation: Mutation,
) -> StoreResult<WriteReceipt> {
    loop {
        match store.put(principal, mutation.clone()).await {
            Err(StoreError::Backpressure) => tokio::task::yield_now().await,
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actor_commits_and_verifies_all_indexes() {
        let path = fixture_path("actor-indexes");
        let store = Store::open(&path).await.unwrap();
        let mut oracle = BTreeMap::new();

        for index in 0..24 {
            let resource = synthetic_resource(index);
            let receipt = put_with_backpressure_retry(
                &store,
                &format!("principal-{index}"),
                Mutation::create(resource),
            )
            .await
            .unwrap();
            oracle.insert(receipt.resource.key.clone(), receipt.resource);
        }

        store.verify(&oracle).await.unwrap();
        assert_eq!(store.current_revision().await.unwrap(), 24);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watch_registration_replays_then_delivers_live_without_gaps() {
        let path = fixture_path("watch");
        let store = Store::open(&path).await.unwrap();
        for index in 0..4 {
            put_with_backpressure_retry(
                &store,
                "seed",
                Mutation::create(synthetic_resource(index)),
            )
            .await
            .unwrap();
        }

        let mut watch = store
            .watch(1, BTreeSet::from(["Process".to_owned()]))
            .await
            .unwrap();
        put_with_backpressure_retry(&store, "live", Mutation::create(synthetic_resource(100)))
            .await
            .unwrap();

        let mut revisions = Vec::new();
        for _ in 0..4 {
            revisions.push(watch.recv().await.unwrap().revision);
        }
        assert_eq!(revisions, vec![2, 3, 4, 5]);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
