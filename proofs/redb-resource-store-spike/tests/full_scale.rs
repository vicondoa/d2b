use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use redb_resource_store_spike::{
    ChangeBatch, ChangeEntry, HintKind, Mutation, OracleCheckpoint, Resource, Store, StoreError,
    fixture_path, put_with_backpressure_retry, synthetic_resource,
};
use tokio::sync::Barrier;

async fn insert_group(
    store: &Store,
    resources: impl IntoIterator<Item = Resource>,
) -> Vec<redb_resource_store_spike::WriteReceipt> {
    let mut tasks = tokio::task::JoinSet::new();
    for resource in resources {
        let store = store.clone();
        tasks.spawn(async move {
            let principal = format!("principal-{}", resource.uid);
            put_with_backpressure_retry(&store, &principal, Mutation::create(resource)).await
        });
    }
    let mut receipts = Vec::new();
    while let Some(result) = tasks.join_next().await {
        receipts.push(result.unwrap().unwrap());
    }
    receipts
}

fn expected_watch_batch(revision: u64, filter: &str) -> ChangeBatch {
    let mut resource = synthetic_resource(usize::try_from(revision - 1).unwrap());
    resource.revision = revision;
    let entries = if resource.key.resource_type == filter {
        vec![ChangeEntry {
            ordinal: 0,
            operation_id: format!("create-{}", resource.uid),
            resource,
            event: "Created".to_owned(),
        }]
    } else {
        Vec::new()
    };
    ChangeBatch { revision, entries }
}

fn check_watch_batch(actual: &ChangeBatch, revision: u64, filter: &str) -> Result<(), String> {
    let expected = expected_watch_batch(revision, filter);
    if *actual == expected {
        Ok(())
    } else {
        Err(format!(
            "watch oracle divergence at revision {revision} for {filter}: actual_entries={} expected_entries={}",
            actual.entries.len(),
            expected.entries.len()
        ))
    }
}

#[test]
fn watch_oracle_rejects_removed_matching_delivery() {
    let expected = expected_watch_batch(1, "Process");
    assert_eq!(expected.entries.len(), 1);
    let removed = ChangeBatch {
        revision: 1,
        entries: Vec::new(),
    };
    assert!(check_watch_batch(&removed, 1, "Process").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "full-scale 10k x 5 oracle experiment"]
async fn correctness_10k_five_runs_zero_divergence() {
    for run in 0..5 {
        let path = fixture_path(&format!("correctness-run-{run}"));
        let store = Store::open(&path).await.unwrap();
        let mut oracle = BTreeMap::new();
        for index in 0..10_000 {
            let receipt = put_with_backpressure_retry(
                &store,
                &format!("oracle-{index}"),
                Mutation::create(synthetic_resource(index)),
            )
            .await
            .unwrap();
            oracle.insert(receipt.resource.key.clone(), receipt.resource.clone());
            let owner_count = u64::try_from(
                oracle
                    .values()
                    .filter(|item| item.owner_uid.is_some())
                    .count(),
            )
            .unwrap();
            let producer_count = u64::try_from(
                oracle
                    .values()
                    .filter(|item| item.producer_uid.is_some())
                    .count(),
            )
            .unwrap();
            store
                .verify_transition(OracleCheckpoint {
                    changed_resource: receipt.resource,
                    resource_count: u64::try_from(oracle.len()).unwrap(),
                    owner_count,
                    producer_count,
                    operation_count: u64::try_from(index + 1).unwrap(),
                    revision: receipt.revision,
                })
                .await
                .unwrap();
        }
        store.verify(&oracle).await.unwrap();
        assert_eq!(oracle.len(), 10_000);
        println!(
            "correctness_run={} resources={} mutations={} divergences=0",
            run + 1,
            oracle.len(),
            oracle.len()
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "full-scale 100-watch replay and live experiment"]
async fn watches_100_have_no_misses_duplicates_or_gaps() {
    let path = fixture_path("watch-100");
    let store = Store::open(&path).await.unwrap();
    for index in 0..24 {
        put_with_backpressure_retry(
            &store,
            "watch-seed",
            Mutation::create(synthetic_resource(index)),
        )
        .await
        .unwrap();
    }

    let mut registrations = tokio::task::JoinSet::new();
    for index in 0..100 {
        let store = store.clone();
        registrations.spawn(async move {
            let resource_type =
                ["Process", "Endpoint", "Volume", "Device", "Guest", "Policy"][index % 6];
            let after_revision = u64::try_from(index % 12).unwrap();
            let watch = store
                .watch(after_revision, BTreeSet::from([resource_type.to_owned()]))
                .await
                .unwrap();
            (after_revision, resource_type.to_owned(), watch)
        });
    }

    let mut watches = Vec::new();
    while let Some(result) = registrations.join_next().await {
        watches.push(result.unwrap());
    }

    let writer_store = store.clone();
    let writer = tokio::spawn(async move {
        for index in 24..224 {
            put_with_backpressure_retry(
                &writer_store,
                &format!("watch-writer-{}", index % 8),
                Mutation::create(synthetic_resource(index)),
            )
            .await
            .unwrap();
        }
    });
    writer.await.unwrap();
    let final_revision = store.current_revision().await.unwrap();

    let mut delivered_batches = 0_u64;
    let mut delivered_entries = 0_u64;
    for (after_revision, filter, mut watch) in watches {
        let mut observed = BTreeSet::new();
        let mut replay_matches = 0_u64;
        let mut live_matches = 0_u64;
        for expected_revision in after_revision + 1..=final_revision {
            let batch = tokio::time::timeout(Duration::from_secs(1), watch.recv())
                .await
                .expect("every revision is delivered without timeout")
                .expect("watch remains open");
            check_watch_batch(&batch, expected_revision, &filter).unwrap();
            assert!(observed.insert(batch.revision), "duplicate revision");
            if !batch.entries.is_empty() && expected_revision <= 24 {
                replay_matches += 1;
            }
            if !batch.entries.is_empty() && expected_revision > 24 {
                live_matches += 1;
            }
            delivered_batches += 1;
            delivered_entries += u64::try_from(batch.entries.len()).unwrap();
        }
        assert_eq!(
            observed.len(),
            usize::try_from(final_revision - after_revision).unwrap()
        );
        assert!(replay_matches > 0, "filter {filter} has replay matches");
        assert!(live_matches > 0, "filter {filter} has live matches");
    }
    let stats = store.stats().await.unwrap();
    assert_eq!(stats.watch_delivery_failures, 0);
    println!(
        "watchers=100 final_revision={final_revision} batches={delivered_batches} entries={delivered_entries} missed=0 duplicated=0 gaps=0"
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "full-scale 500-writer conflict storm"]
async fn conflict_storm_groups_at_least_half_of_non_conflicting_writes() {
    let path = fixture_path("conflict-storm");
    let store = Store::open(&path).await.unwrap();
    let mut seeded = Vec::new();
    for index in 0..50 {
        seeded.push(
            put_with_backpressure_retry(
                &store,
                "storm-seed",
                Mutation::create(synthetic_resource(index)),
            )
            .await
            .unwrap(),
        );
    }

    let barrier = Arc::new(Barrier::new(501));
    let mut tasks = tokio::task::JoinSet::new();
    for writer in 0..500 {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        let target = writer % 50;
        let mut resource = seeded[target].resource.clone();
        resource.generation += 1;
        resource.spec_json.push_str(&format!("-writer-{writer}"));
        let expected_revision = seeded[target].revision;
        tasks.spawn(async move {
            barrier.wait().await;
            put_with_backpressure_retry(
                &store,
                &format!("storm-writer-{writer}"),
                Mutation::update(resource, expected_revision, format!("storm-{writer}")),
            )
            .await
        });
    }
    barrier.wait().await;

    let mut success = Vec::new();
    let mut conflicts = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(receipt) => success.push(receipt),
            Err(StoreError::Conflict { .. }) => conflicts += 1,
            Err(error) => panic!("unexpected storm result: {error}"),
        }
    }
    assert_eq!(success.len(), 50);
    assert_eq!(conflicts, 450);
    let grouped = success
        .iter()
        .filter(|receipt| receipt.batch_size > 1)
        .count();
    let grouped_percent = grouped * 100 / success.len();
    assert!(grouped_percent >= 50);
    println!(
        "writers=500 targets=50 successful_non_conflicting={} conflicts={conflicts} grouped_batch_gt_1={grouped} grouped_percent={grouped_percent}",
        success.len()
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "full-scale 4-level 8-way owner fan-in"]
async fn owner_fan_in_emits_one_direct_hint_per_child_mutation() {
    let path = fixture_path("owner-fan-in");
    let store = Store::open(&path).await.unwrap();
    let mut hints = store
        .hint_consumer(BTreeSet::from([
            "Process".to_owned(),
            "Endpoint".to_owned(),
            "Volume".to_owned(),
            "Device".to_owned(),
            "Guest".to_owned(),
            "Policy".to_owned(),
        ]))
        .await
        .unwrap();
    let collector = tokio::spawn(async move {
        let mut owner_hints = 0_usize;
        let mut resource_hints = 0_usize;
        while owner_hints < 4_680 || resource_hints < 4_681 {
            let hint = hints.recv().await.expect("hint bus remains open");
            match hint.kind {
                HintKind::ResourceChanged => resource_hints += 1,
                HintKind::OwnedResourceChanged => owner_hints += 1,
            }
        }
        (resource_hints, owner_hints)
    });

    for start in (0..4_681).step_by(16) {
        insert_group(
            &store,
            (start..(start + 16).min(4_681)).map(|index| {
                let mut resource = synthetic_resource(index);
                resource.owner_uid = (index > 0).then(|| format!("uid-{:08}", (index - 1) / 8));
                resource
            }),
        )
        .await;
    }
    let (resource_hints, owner_hints) = collector.await.unwrap();
    assert_eq!(resource_hints, 4_681);
    assert_eq!(owner_hints, 4_680);
    let stats = store.stats().await.unwrap();
    assert_eq!(stats.hint_delivery_failures, 0);
    println!(
        "owner_tree_levels=4 fanout=8 resources=4681 resource_hints={resource_hints} owner_hints={owner_hints} delivery_failures=0"
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}
