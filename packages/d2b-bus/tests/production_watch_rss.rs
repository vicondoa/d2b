use std::fs::OpenOptions;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use d2b_bus::{
    BusConfig, OperationId, OperationSpec, ReceivedFrame, ResourceCall, ResourceQuery,
    StreamLimits, StreamName, router::production_rss::ProductionWatchHarness,
};
use d2b_contracts::v3::{
    CanonicalJsonValue, ConfigurationGeneration, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceName,
    ResourceRef, ResourceTypeName, ResourceUid, Timestamp, ZoneId, ZoneRevision, canonical_digest,
};
use d2b_controller_toolkit::{
    OperationContext, PendingQueue, PriorityLane, QueueHint, ResourceKey, TriggerReason, TriggerSet,
};
use d2b_resource_api::watch::{WatchPumpError, WatchService};
use d2b_resource_store::mutation_seal::{MutationSealBody, MutationSealIssuer, mutation_seal_pair};
use d2b_resource_store::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, ExpectedRevision,
    PolicySnapshot, PreparedStoreMutation, ResourceMutationKind, StoreMutation,
    StoreOperationContext, StoreProjection, StoreWatchRequest,
};
use d2b_resource_store_redb::{
    GROUP_COMMIT_MAX, MAX_CONCURRENT_READS, READ_LIFETIME, READ_POOL_THREADS, RedbResourceStore,
    StoreIdentity, WATCH_ADMISSION_CAPACITY, WRITE_QUEUE_CAPACITY, write_provisioning_marker,
};
use serde_json::Value;

const RESOURCE_COUNT: usize = 10_000;
const WATCH_COUNT: usize = 100;
const MAX_BATCH_MUTATIONS: usize = d2b_contracts::v3::MAX_BATCH_MUTATIONS;
const REVISION_COUNT: u64 = RESOURCE_COUNT.div_ceil(MAX_BATCH_MUTATIONS) as u64;
const RSS_THRESHOLD_KIB: u64 = 24_576;
const RSS_CHILD_ENV: &str = "D2B_REDB_PRODUCTION_WATCH_RSS_CHILD";
const RSS_FIXTURE_ENV: &str = "D2B_REDB_PRODUCTION_WATCH_RSS_FIXTURE";
const RSS_CHILD_MARKER: &str = "PRODUCTION_REDB_WATCH_FIXTURE";
const CACHE_BYTES: usize = 4 * 1024 * 1024;

struct WatchConnection {
    incoming: d2b_bus::IncomingStream,
    task: tokio::task::JoinHandle<Result<(), WatchPumpError>>,
}

#[test]
#[ignore = "run the whole-process production watch RSS fixture through the public heavy gate"]
fn production_backend_hard_fixture_rss() {
    if std::env::var_os(RSS_CHILD_ENV).is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("production watch RSS child runtime");
        runtime.block_on(production_watch_rss_child());
        return;
    }

    let executable = std::env::current_exe().expect("production watch RSS executable");
    let mut raw_runs = Vec::with_capacity(3);
    for run in 1..=3 {
        let fixture = prepare_fixture();
        let output = Command::new(gnu_time_program())
            .args([
                "-v",
                executable
                    .to_str()
                    .expect("production watch RSS executable UTF-8"),
                "--exact",
                "production_backend_hard_fixture_rss",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(RSS_CHILD_ENV, "1")
            .env(
                RSS_FIXTURE_ENV,
                fixture
                    .path()
                    .to_str()
                    .expect("production watch RSS fixture path UTF-8"),
            )
            .output()
            .expect("GNU time is required for the production watch RSS fixture");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "production watch RSS child failed (run {run}):\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains(&format!(
                "{RSS_CHILD_MARKER} resources={RESOURCE_COUNT} watches={WATCH_COUNT}"
            )),
            "production watch RSS child did not report the hard fixture (run {run}):\n{stdout}"
        );
        for line in stdout
            .lines()
            .filter(|line| line.contains(RSS_CHILD_MARKER))
        {
            println!("production watch fixture signals run {run}: {line}");
        }
        let rss = parse_maximum_rss_kib(&stderr);
        assert!(
            rss <= RSS_THRESHOLD_KIB,
            "production watch whole-process RSS run {run} was {rss} KiB, above the unchanged {RSS_THRESHOLD_KIB} KiB threshold"
        );
        println!("production watch whole-process RSS run {run}: {rss} KiB");
        raw_runs.push(rss);
    }

    raw_runs.sort_unstable();
    let median = raw_runs[1];
    assert!(
        median <= RSS_THRESHOLD_KIB,
        "production watch whole-process RSS median was {median} KiB, above the unchanged {RSS_THRESHOLD_KIB} KiB threshold"
    );
    println!(
        "production watch whole-process RSS raw runs: {raw_runs:?}; median: {median} KiB; threshold: {RSS_THRESHOLD_KIB} KiB; baseline subtraction: none"
    );
}

fn parse_maximum_rss_kib(stderr: &str) -> u64 {
    const FIELD: &str = "Maximum resident set size (kbytes):";
    stderr
        .lines()
        .find_map(|line| {
            let value = line
                .find(FIELD)
                .map(|offset| &line[offset + FIELD.len()..])?;
            value.trim().parse::<u64>().ok()
        })
        .expect("GNU time did not report whole-process maximum RSS")
}

fn gnu_time_program() -> String {
    if let Some(program) = std::env::var_os("D2B_GNU_TIME") {
        return program.to_string_lossy().into_owned();
    }
    for candidate in [
        "/usr/bin/time",
        "/bin/time",
        "/run/current-system/sw/bin/time",
    ] {
        if std::path::Path::new(candidate).is_file() {
            return candidate.to_owned();
        }
    }
    "time".to_owned()
}

fn identity() -> StoreIdentity {
    StoreIdentity::new(
        d2b_resource_store::StoreSlot::new(0).expect("fixed store slot"),
        ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("fixed store UID"),
        ZoneId::parse("work").expect("fixed Zone"),
        ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("fixed Zone UID"),
        Timestamp::parse("2026-07-31T00:00:00.000Z").expect("fixed timestamp"),
        PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9)
                .expect("fixed configuration generation"),
            controller_generation: None,
        },
    )
}

fn operation(id: &str) -> StoreOperationContext {
    StoreOperationContext {
        operation_id: id.to_owned(),
        idempotency_key: Some(format!("key-{id}")),
        correlation_id: format!("correlation-{id}"),
        trace_id: None,
        deadline_ms: 30_000,
    }
}

fn host_body(name: &str) -> Vec<u8> {
    let raw = format!(
        r#"{{"apiVersion":"resources.d2bus.org/v3","metadata":{{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"{name}","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"work"}},"spec":{{"providerRef":"Provider/system-core","updatePolicy":{{"disruptive":"manual","nonDisruptive":"automatic"}}}},"status":{{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{{}},"startedAt":null,"update":{{"dependencies":{{"count":0,"refs":[]}},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{{"count":0,"refs":[]}},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}}}},"type":"Host"}}"#
    );
    let mut value = CanonicalJsonValue::parse(raw.as_bytes()).expect("valid Host envelope");
    let CanonicalJsonValue::Object(root) = &mut value else {
        unreachable!()
    };
    let CanonicalJsonValue::Object(metadata) = root.get_mut("metadata").unwrap() else {
        unreachable!()
    };
    metadata.remove("uid");
    value.to_canonical_bytes()
}

fn mutation(
    name: &str,
    canonical: Vec<u8>,
) -> (PreparedStoreMutation, AdmittedAuthorizationTarget) {
    let target = ResourceRef::parse(&format!("Host/{name}")).expect("valid Host ref");
    let digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    (
        PreparedStoreMutation::new(
            StoreMutation {
                kind: ResourceMutationKind::Create,
                zone: ZoneId::parse("work").expect("fixed Zone"),
                target: target.clone(),
                expected: ExpectedRevision::CreateAbsent,
                expected_uid: None,
                owner: None,
                canonical_resource: Some(canonical),
                add_finalizers: Vec::new(),
                remove_finalizers: Vec::new(),
                wait_for_reconcile: false,
                reconcile_deadline_ms: None,
            },
            None,
            Some(digest),
        ),
        AdmittedAuthorizationTarget {
            resource_type: ResourceTypeName::parse("Host").expect("fixed ResourceType"),
            resource_name: Some(target.name().clone()),
            verb: AdmittedVerb::Create,
            subresource: None,
            execution_ref: None,
        },
    )
}

fn mutation_body(
    operation_id: &str,
    mutations: Vec<PreparedStoreMutation>,
    targets: Vec<AdmittedAuthorizationTarget>,
) -> MutationSealBody {
    MutationSealBody {
        mutations,
        authorization: AdmittedAuthorization {
            zone: ZoneId::parse("work").expect("fixed Zone"),
            subject_ref: ResourceRef::parse("Provider/system-core").expect("fixed Provider"),
            subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333")
                .expect("fixed subject UID"),
            targets,
        },
        policy_snapshot: PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9)
                .expect("fixed configuration generation"),
            controller_generation: None,
        },
        operation: operation(operation_id),
    }
}

async fn seed_store(store: &RedbResourceStore, issuer: &MutationSealIssuer) {
    for batch_index in 0..REVISION_COUNT as usize {
        let start = batch_index * MAX_BATCH_MUTATIONS;
        let end = (start + MAX_BATCH_MUTATIONS).min(RESOURCE_COUNT);
        let mut mutations = Vec::with_capacity(end - start);
        let mut targets = Vec::with_capacity(end - start);
        for index in start..end {
            let name = format!("hard-host-{index:05}");
            let (prepared, target) = mutation(&name, host_body(&name));
            mutations.push(prepared);
            targets.push(target);
        }
        let body = mutation_body(
            &format!("production-watch-seed-{batch_index:04}"),
            mutations,
            targets,
        );
        store
            .commit_verified(issuer.seal(body))
            .await
            .expect("production watch RSS seed batch");
    }
}

fn prepare_fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("production watch RSS fixture directory");
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .expect("production watch RSS fixture database");
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.marker"))
        .expect("production watch RSS fixture marker");
    let store_identity = identity();
    write_provisioning_marker(&mut marker, &store_identity)
        .expect("production watch RSS fixture marker write");
    let (issuer, acceptor) = mutation_seal_pair(store_identity.seal_identity());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("production watch RSS fixture runtime");
    runtime.block_on(async {
        let store = RedbResourceStore::provision_owned(file, marker, store_identity, acceptor)
            .await
            .expect("production watch RSS fixture provision");
        seed_store(&store, &issuer).await;
        store
            .shutdown()
            .await
            .expect("production watch RSS fixture shutdown");
    });
    directory
}

fn watch_request(
    id: &str,
    after_revision: ZoneRevision,
    initial_credits: u32,
) -> StoreWatchRequest {
    StoreWatchRequest {
        operation: operation(id),
        zone: ZoneId::parse("work").expect("fixed Zone"),
        resource_types: vec![ResourceTypeName::parse("Host").expect("fixed ResourceType")],
        resource_names: Vec::new(),
        filters: Vec::new(),
        after_revision,
        initial_credits,
        projection: StoreProjection::Full,
    }
}

fn bus_config() -> BusConfig {
    BusConfig {
        stream_limits: StreamLimits {
            max_stream_credit: 64 * 1024,
            max_aggregate_bytes: 8 * 1024 * 1024,
            max_streams: 128,
            max_frame_bytes: 64 * 1024,
            max_streams_per_principal: 128,
            max_credit_per_principal: 8 * 1024 * 1024,
            max_queued_bytes_per_principal: 8 * 1024 * 1024,
        },
        ..BusConfig::default()
    }
}

async fn open_bus_watch(
    service: &WatchService,
    harness: &ProductionWatchHarness,
    query: &ResourceQuery,
    id: &str,
    after_revision: ZoneRevision,
    initial_credits: u32,
) -> WatchConnection {
    let watch = service
        .open(watch_request(id, after_revision, initial_credits))
        .await
        .expect("production watch API open");
    let bus_stream = harness
        .caller()
        .open_resource_stream(
            harness.route().clone(),
            OperationSpec::new(
                OperationId::parse(format!("{id}-bus")).expect("production bus operation"),
                30_000,
            )
            .expect("production bus operation"),
            ResourceCall::Watch(query.clone()),
            StreamName::parse(format!("watch:{id}")).expect("production bus stream name"),
            64 * 1024,
        )
        .await
        .expect("production bus named stream open");
    let incoming = harness
        .take_incoming()
        .expect("production controller incoming stream");
    let task = tokio::spawn(async move {
        let mut watch = watch;
        watch.pump_to(&bus_stream).await
    });
    WatchConnection { incoming, task }
}

fn enqueue_controller_frame(queue: &PendingQueue, frame: &ReceivedFrame, revision: ZoneRevision) {
    let payload: Value =
        serde_json::from_slice(frame.payload()).expect("production controller frame JSON");
    for entry in payload["entries"]
        .as_array()
        .expect("production controller entries")
    {
        let resource_ref = ResourceRef::new(
            ResourceTypeName::parse(entry["resource_type"].as_str().unwrap()).unwrap(),
            ResourceName::parse(entry["resource_name"].as_str().unwrap()).unwrap(),
        );
        let key = ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            resource_ref,
            ResourceUid::parse(entry["resource_uid"].as_str().unwrap()).unwrap(),
        );
        let operation_id = format!("production-watch-controller-{}", revision.get());
        queue
            .push(
                QueueHint::new(
                    key,
                    revision,
                    TriggerSet::new([TriggerReason::SpecGenerationChanged]),
                    PriorityLane::Ordinary,
                    OperationContext::new(
                        operation_id.clone(),
                        format!("{operation_id}-key"),
                        format!("{operation_id}-correlation"),
                        None,
                    )
                    .unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
    }
}

async fn commit_resource(
    store: &RedbResourceStore,
    issuer: &MutationSealIssuer,
    operation_id: &str,
    name: &str,
) -> d2b_resource_store::StoreCommitResult {
    let (prepared, target) = mutation(name, host_body(name));
    store
        .commit_verified(issuer.seal(mutation_body(operation_id, vec![prepared], vec![target])))
        .await
        .expect("production watch RSS mutation")
}

async fn production_watch_rss_child() {
    let fixture = std::env::var(RSS_FIXTURE_ENV).expect("production watch RSS fixture path");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(std::path::Path::new(&fixture).join("store.redb"))
        .expect("production watch RSS database open");
    let store_identity = identity();
    let (issuer, acceptor) = mutation_seal_pair(store_identity.seal_identity());
    let store = Arc::new(
        RedbResourceStore::open_owned(file, store_identity, acceptor)
            .await
            .expect("production watch RSS store open"),
    );
    let current_revision = REVISION_COUNT;
    assert_eq!(WRITE_QUEUE_CAPACITY, 256);
    assert_eq!(GROUP_COMMIT_MAX, 16);
    assert_eq!(READ_POOL_THREADS, 4);
    assert_eq!(MAX_CONCURRENT_READS, 16);
    assert_eq!(READ_LIFETIME, Duration::from_millis(250));

    let listed = store
        .list(d2b_resource_store::StoreListRequest {
            operation: operation("production-watch-list"),
            zone: ZoneId::parse("work").unwrap(),
            resource_types: vec![ResourceTypeName::parse("Host").unwrap()],
            resource_names: Vec::new(),
            filters: Vec::new(),
            page_size: 1,
            cursor: None,
            projection: StoreProjection::MetadataOnly,
        })
        .await
        .expect("production watch RSS list");
    assert_eq!(listed.resources.len(), 1);

    let backend_before_replay = store.signals();
    let replay_batches = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let replay_entries = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let batches = Arc::clone(&replay_batches);
    let entries = Arc::clone(&replay_entries);
    store
        .replay_backend(
            ZoneRevision::new(current_revision - 1).get(),
            [ResourceTypeName::parse("Host").unwrap()],
            move |batch| {
                batches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                entries.fetch_add(
                    batch.entries().len() as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
                Ok(())
            },
        )
        .await
        .expect("production watch RSS backend replay");
    let backend_after_replay = store.signals();
    assert_eq!(
        backend_after_replay.revision_range_seeks - backend_before_replay.revision_range_seeks,
        1
    );
    assert_eq!(
        backend_after_replay.replay_rows_scanned - backend_before_replay.replay_rows_scanned,
        1
    );
    assert_eq!(
        backend_after_replay.replay_rows_decoded - backend_before_replay.replay_rows_decoded,
        1
    );
    assert_eq!(replay_batches.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(replay_entries.load(std::sync::atomic::Ordering::Relaxed) > 0);

    let service = WatchService::new(Arc::clone(&store));
    let harness = ProductionWatchHarness::new(bus_config()).expect("production watch RSS bus");
    let query = ResourceQuery::new(
        vec![ResourceTypeName::parse("Host").unwrap()],
        Vec::new(),
        Vec::new(),
    )
    .expect("production watch RSS query");
    let replay = open_bus_watch(
        &service,
        &harness,
        &query,
        "production-watch-replay",
        ZoneRevision::new(current_revision - 1),
        2,
    )
    .await;
    let replay_frame =
        tokio::time::timeout(Duration::from_secs(10), replay.incoming.receive_next())
            .await
            .expect("production watch replay frame timeout")
            .expect("production watch replay frame");
    let replay_payload: Value =
        serde_json::from_slice(replay_frame.payload()).expect("production watch replay JSON");
    assert_eq!(replay_payload["revision"].as_u64(), Some(current_revision));
    replay
        .incoming
        .grant_frame(&replay_frame, replay_frame.payload().len())
        .await
        .expect("production watch replay credit");
    replay.task.abort();
    let _ = replay.task.await;
    assert_eq!(
        store.watch_signals().unwrap().replay_work,
        1,
        "API replay remains one bounded replay unit"
    );

    let mut watchers = Vec::with_capacity(WATCH_COUNT);
    for index in 0..WATCH_COUNT {
        watchers.push(
            open_bus_watch(
                &service,
                &harness,
                &query,
                &format!("production-watch-{index:03}"),
                ZoneRevision::new(current_revision),
                2,
            )
            .await,
        );
    }
    let registered = store
        .watch_signals()
        .expect("production watch registrations");
    assert_eq!(registered.current_registrations, WATCH_COUNT as u64);
    assert_eq!(registered.budget_used, 0);
    assert_eq!(registered.budget_capacity, WATCH_ADMISSION_CAPACITY as u64);

    let backend_before_fanout = store.signals();
    let fanout_commit =
        commit_resource(&store, &issuer, "production-watch-fanout", "hard-fanout").await;
    let mut deliveries = Vec::with_capacity(WATCH_COUNT);
    for watcher in &mut watchers {
        let frame = tokio::time::timeout(Duration::from_secs(10), watcher.incoming.receive_next())
            .await
            .expect("production watch fan-out frame timeout")
            .expect("production watch fan-out frame");
        let payload: Value =
            serde_json::from_slice(frame.payload()).expect("production watch fan-out JSON");
        assert_eq!(
            payload["revision"].as_u64(),
            Some(fanout_commit.revision.get())
        );
        assert_eq!(payload["entries"].as_array().map(Vec::len), Some(1));
        deliveries.push(frame);
    }
    let after_fanout = store
        .watch_signals()
        .expect("production watch fan-out signals");
    assert_eq!(after_fanout.budget_used, WATCH_COUNT as u64);
    assert_eq!(after_fanout.current_registrations, WATCH_COUNT as u64);
    let backend_after_fanout = store.signals();
    assert_eq!(
        backend_after_fanout.shared_immutable_batches
            - backend_before_fanout.shared_immutable_batches,
        1
    );
    assert_eq!(
        backend_after_fanout.fanout_references - backend_before_fanout.fanout_references,
        WATCH_COUNT as u64
    );
    assert_eq!(backend_after_fanout.writer_queue_depth, 0);

    let controller_queue = PendingQueue::new(WATCH_COUNT + 1, 1);
    for (watcher, frame) in watchers.iter().zip(deliveries.iter()) {
        enqueue_controller_frame(&controller_queue, frame, fanout_commit.revision);
        watcher
            .incoming
            .grant_frame(frame, frame.payload().len())
            .await
            .expect("production controller credit");
    }
    let mut controller_items = 0;
    while let Some(work) = controller_queue.pop_ready() {
        assert_eq!(work.high_water_revision(), fanout_commit.revision);
        controller_queue
            .finish(work.key())
            .expect("controller queue finish");
        controller_items += 1;
    }
    assert_eq!(
        controller_items, 1,
        "controller fan-in coalesces the same resource across all streams"
    );
    for watcher in watchers {
        watcher.task.abort();
        let _ = watcher.task.await;
        drop(watcher.incoming);
    }
    drop(service);
    drop(harness);

    let rejected = WatchService::new(Arc::clone(&store))
        .open(watch_request(
            "production-watch-rejected",
            fanout_commit.revision,
            0,
        ))
        .await
        .expect_err("zero-credit watch admission");
    assert_eq!(
        rejected.kind(),
        d2b_resource_store::StoreErrorKind::StoreBackpressure
    );

    let slow_start = fanout_commit.revision;
    let slow = store
        .watch_stream(watch_request("production-watch-slow", slow_start, 1))
        .await
        .expect("production watch slow registration");
    let _slow_first = commit_resource(
        &store,
        &issuer,
        "production-watch-slow-first",
        "hard-slow-first",
    )
    .await;
    let _slow_second = commit_resource(
        &store,
        &issuer,
        "production-watch-slow-second",
        "hard-slow-second",
    )
    .await;
    let watch_signals = store
        .watch_signals()
        .expect("production watch saturation signals");
    assert!(watch_signals.admission_rejections >= 1);
    assert!(watch_signals.slow_watcher_evictions >= 1);
    assert_eq!(watch_signals.current_registrations, 0);
    assert_eq!(watch_signals.budget_used, 0);
    drop(slow.1);

    let backend_signals = store.signals();
    println!(
        "{RSS_CHILD_MARKER} resources={RESOURCE_COUNT} watches={WATCH_COUNT} range_seeks={} scanned_rows={} decoded_rows={} shared_batches={} fanout_references={} queue_depth={} queue_capacity={} read_pool_threads={} max_concurrent_reads={} cache_bytes={} watch_registrations={} watch_budget_used={} watch_budget_capacity={} slow_watcher_evictions={} admission_rejections={} replay_work={}",
        backend_signals.revision_range_seeks,
        backend_signals.replay_rows_scanned,
        backend_signals.replay_rows_decoded,
        backend_signals.shared_immutable_batches,
        backend_signals.fanout_references,
        backend_signals.writer_queue_depth,
        backend_signals.writer_queue_capacity,
        READ_POOL_THREADS,
        MAX_CONCURRENT_READS,
        CACHE_BYTES,
        watch_signals.current_registrations,
        watch_signals.budget_used,
        watch_signals.budget_capacity,
        watch_signals.slow_watcher_evictions,
        watch_signals.admission_rejections,
        watch_signals.replay_work,
    );
    drop(issuer);
    let store = Arc::try_unwrap(store).expect("production watch RSS store references");
    store
        .shutdown()
        .await
        .expect("production watch RSS store shutdown");
}
