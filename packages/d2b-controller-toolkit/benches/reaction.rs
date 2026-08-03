//! Production reaction-path benchmark for the controller toolkit.
//!
//! Each profile provisions the production redb store, opens the Resource-API
//! watch adapter, commits ready Process resources, and drives the real
//! system-minijail Process Provider through its core supervisor adapter. The
//! effect backend is hermetic and records the supervisor boundary; it does
//! not stand in for the store, watch dispatcher, controller handler, or
//! Provider path being measured.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts::v3::{
    CanonicalJsonValue, ConfigurationGeneration, ControllerGeneration,
    RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef, ResourceTypeName, ResourceUid, Timestamp, ZoneId,
    ZoneRevision, canonical_digest,
};
use d2b_process::{
    BackendLaunch, BackendObservation, CompiledDigests, ConfigurationDigest, IdentityBinding,
    LaunchTicket, ObservedIdentity, OperationBinding, ProcessEffectBackend, ProcessEffectError,
    ProcessIdentityDigest, ProcessRequest, ProcessStopClass, WaitReapOwner,
};
use d2b_process_conformance::ProcessProvider;
use d2b_provider_supervisor::ProviderSupervisor;
use d2b_provider_system_minijail::MinijailProcessProvider;
use d2b_resource_api::watch::WatchService;
use d2b_resource_store::mutation_seal::{MutationSealBody, MutationSealIssuer, mutation_seal_pair};
use d2b_resource_store::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, ExpectedRevision,
    PolicySnapshot, PreparedStoreMutation, ResourceMutationKind, StoreGetRequest, StoreMutation,
    StoreOperationContext, StoreProjection, StoreSlot, StoreWatchRequest,
};
use d2b_resource_store_redb::{
    BackendSignals, MAX_INITIAL_WATCH_CREDITS, RedbResourceStore, StoreIdentity, WatchSignals,
    write_provisioning_marker,
};
use tokio::sync::Semaphore;

const PROFILES: [usize; 3] = [1, 10, 100];
const HANDLER_P95_LIMIT: Duration = Duration::from_millis(5);
const LAUNCH_P95_LIMIT: Duration = Duration::from_millis(20);
const LAUNCH_EFFECT_WORK: Duration = Duration::from_micros(250);
const PROVIDER_CONCURRENCY: usize = 16;
const WATCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
struct HandlerRecord {
    resource_ref: ResourceRef,
    resource_uid: ResourceUid,
    started_at: Instant,
}

struct LaunchMetrics {
    starts: Mutex<Vec<(ResourceUid, Instant)>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    next_identity: AtomicUsize,
}

impl LaunchMetrics {
    fn new() -> Self {
        Self {
            starts: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            next_identity: AtomicUsize::new(1),
        }
    }

    fn starts(&self) -> Vec<(ResourceUid, Instant)> {
        self.starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::Acquire)
    }
}

struct RecordingEffectBackend {
    metrics: Arc<LaunchMetrics>,
}

impl RecordingEffectBackend {
    fn new(metrics: Arc<LaunchMetrics>) -> Self {
        Self { metrics }
    }
}

impl ProcessEffectBackend for RecordingEffectBackend {
    type Handle = ();

    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
        let ticket = request.ticket();
        let active = self.metrics.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.metrics.max_active.fetch_max(active, Ordering::AcqRel);
        self.metrics
            .starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((ticket.process_uid().clone(), Instant::now()));
        thread::sleep(LAUNCH_EFFECT_WORK);
        self.metrics.active.fetch_sub(1, Ordering::AcqRel);

        let identity_number = self.metrics.next_identity.fetch_add(1, Ordering::Relaxed);
        let mut identity_bytes = [0_u8; 32];
        identity_bytes[..std::mem::size_of::<usize>()]
            .copy_from_slice(&identity_number.to_le_bytes());
        let identity = ProcessIdentityDigest::from_bytes(identity_bytes);
        let observed = ObservedIdentity::from_verified([
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
            IdentityBinding::Executable,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]);
        Ok(BackendLaunch::new(
            BackendObservation::new(identity, observed, WaitReapOwner::Local),
            (),
        ))
    }

    fn observe(
        &self,
        _request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        Ok(None)
    }

    fn open_pidfd(
        &self,
        _observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError> {
        Ok(())
    }

    fn stop(
        &self,
        _handle: &Self::Handle,
        _class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        Ok(())
    }
}

fn store_identity() -> StoreIdentity {
    StoreIdentity::new(
        StoreSlot::new(0).expect("valid store slot"),
        ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("valid store UID"),
        ZoneId::parse("reaction").expect("valid Zone"),
        ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("valid Zone UID"),
        Timestamp::parse("2026-07-31T00:00:00.000Z").expect("valid timestamp"),
        PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1)
                .expect("nonzero configuration generation"),
            controller_generation: None,
        },
    )
}

async fn provision_store() -> (tempfile::TempDir, RedbResourceStore, MutationSealIssuer) {
    let directory = tempfile::tempdir().expect("create hermetic store directory");
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .expect("create hermetic redb file");
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.marker"))
        .expect("create hermetic store marker");
    let identity = store_identity();
    write_provisioning_marker(&mut marker, &identity).expect("write store marker");
    let (issuer, acceptor) = mutation_seal_pair(identity.seal_identity());
    let store = RedbResourceStore::provision_owned(file, marker, identity, acceptor)
        .await
        .expect("provision production redb backend");
    (directory, store, issuer)
}

fn process_ref(index: usize) -> ResourceRef {
    ResourceRef::parse(&format!("Process/ready-{index}")).expect("valid Process ref")
}

fn operation_uid(profile: usize, index: usize) -> ResourceUid {
    ResourceUid::parse(format!(
        "123e4567-e89b-42d3-a456-426614{:06x}",
        profile * 1_000 + index
    ))
    .expect("valid operation UID")
}

fn process_body(index: usize) -> Vec<u8> {
    let raw = format!(
        r#"{{
            "apiVersion":"resources.d2bus.org/v3",
            "metadata":{{
                "configurationGeneration":1,
                "createdAt":"2026-07-22T00:00:00.000Z",
                "deletionRequestedAt":null,
                "finalizers":[],
                "generation":1,
                "managedBy":"configuration",
                "name":"ready-{index}",
                "ownerRef":null,
                "revision":1,
                "uid":"123e4567-e89b-42d3-a456-426614174000",
                "updatedAt":"2026-07-22T00:00:00.000Z",
                "zone":"reaction"
            }},
            "spec":{{
                "executionRef":"Host/host-system",
                "processClass":"worker",
                "providerRef":"Provider/system-minijail",
                "template":"reaction",
                "updatePolicy":{{
                    "disruptive":"manual",
                    "nonDisruptive":"automatic"
                }}
            }},
            "status":{{
                "completedAt":null,
                "conditions":[],
                "lastReconciledAt":null,
                "observedGeneration":0,
                "outcome":null,
                "phase":"Pending",
                "resource":{{}},
                "startedAt":null,
                "update":{{
                    "dependencies":{{"count":0,"refs":[]}},
                    "disruption":"None",
                    "lastAssessedAt":null,
                    "observedGeneration":0,
                    "operationId":null,
                    "owned":{{"count":0,"refs":[]}},
                    "preserveState":true,
                    "reasons":[],
                    "state":"Unknown",
                    "targetGeneration":1
                }}
            }},
            "type":"Process"
        }}"#
    );
    let mut value = CanonicalJsonValue::parse(raw.as_bytes()).expect("valid Process envelope");
    let CanonicalJsonValue::Object(root) = &mut value else {
        panic!("Process envelope is an object");
    };
    let CanonicalJsonValue::Object(metadata) = root
        .get_mut("metadata")
        .expect("Process metadata is present")
    else {
        panic!("Process metadata is an object");
    };
    metadata.remove("uid");
    value.to_canonical_bytes()
}

fn mutation_body(profile: usize, index: usize, target: &ResourceRef) -> MutationSealBody {
    let name = target.name().clone();
    let canonical_resource = process_body(index);
    let payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical_resource);
    MutationSealBody {
        mutations: vec![PreparedStoreMutation::new(
            StoreMutation {
                kind: ResourceMutationKind::Create,
                zone: ZoneId::parse("reaction").expect("valid Zone"),
                target: target.clone(),
                expected: ExpectedRevision::CreateAbsent,
                expected_uid: None,
                owner: None,
                canonical_resource: Some(canonical_resource),
                add_finalizers: Vec::new(),
                remove_finalizers: Vec::new(),
                wait_for_reconcile: false,
                reconcile_deadline_ms: None,
            },
            None,
            Some(payload_digest),
        )],
        authorization: AdmittedAuthorization {
            zone: ZoneId::parse("reaction").expect("valid Zone"),
            subject_ref: ResourceRef::parse("Provider/system-minijail")
                .expect("valid Provider ref"),
            subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333")
                .expect("valid subject UID"),
            targets: vec![AdmittedAuthorizationTarget {
                resource_type: ResourceTypeName::parse("Process").expect("valid ResourceType"),
                resource_name: Some(name),
                verb: AdmittedVerb::Create,
                subresource: None,
                execution_ref: None,
            }],
        },
        policy_snapshot: PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1)
                .expect("nonzero configuration generation"),
            controller_generation: None,
        },
        operation: StoreOperationContext {
            operation_id: format!("reaction-{profile}-{index}"),
            idempotency_key: Some(format!("reaction-key-{profile}-{index}")),
            correlation_id: format!("reaction-correlation-{profile}-{index}"),
            trace_id: None,
            deadline_ms: 30_000,
        },
    }
}

fn watch_request() -> StoreWatchRequest {
    StoreWatchRequest {
        operation: StoreOperationContext {
            operation_id: "reaction-watch".to_owned(),
            idempotency_key: Some("reaction-watch-key".to_owned()),
            correlation_id: "reaction-watch-correlation".to_owned(),
            trace_id: None,
            deadline_ms: 30_000,
        },
        zone: ZoneId::parse("reaction").expect("valid Zone"),
        resource_types: vec![ResourceTypeName::parse("Process").expect("valid ResourceType")],
        resource_names: Vec::new(),
        filters: Vec::new(),
        after_revision: ZoneRevision::new(0),
        initial_credits: MAX_INITIAL_WATCH_CREDITS,
        projection: StoreProjection::Full,
    }
}

fn get_request(target: &ResourceRef) -> StoreGetRequest {
    StoreGetRequest {
        operation: StoreOperationContext {
            operation_id: format!("reaction-read-{}", target.name().as_str()),
            idempotency_key: None,
            correlation_id: "reaction-read-correlation".to_owned(),
            trace_id: None,
            deadline_ms: 30_000,
        },
        zone: ZoneId::parse("reaction").expect("valid Zone"),
        target: target.clone(),
        expected_uid: None,
        projection: StoreProjection::Full,
    }
}

fn compiled_digests() -> CompiledDigests {
    fn digest(seed: u8) -> ConfigurationDigest {
        ConfigurationDigest::from_bytes([seed; 32])
    }

    CompiledDigests {
        sandbox: digest(1),
        budget: digest(2),
        mounts: digest(3),
        devices: digest(4),
        network: digest(5),
        endpoints: digest(6),
        fd_table: digest(7),
    }
}

fn launch_ticket(
    resource: &d2b_resource_store::StoredResource,
    operation: ResourceUid,
) -> LaunchTicket {
    LaunchTicket::new(
        resource.resource_ref.clone(),
        resource.uid.clone(),
        resource.generation,
        ControllerGeneration::new(1).expect("nonzero controller generation"),
        BoundedToken::parse("system-core").expect("valid owner Provider"),
        BoundedToken::parse("reaction").expect("valid component"),
        BoundedToken::parse("reaction").expect("valid template"),
        ResourceRef::parse("Host/host-system").expect("valid Host ref"),
        ExecutionDomain::System,
        None,
        BoundedToken::parse("system-minijail").expect("valid Process Provider"),
        compiled_digests(),
        OperationBinding::new(operation, 30_000).expect("valid launch operation"),
        BTreeSet::from([
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
            IdentityBinding::Executable,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]),
    )
    .expect("valid Process launch ticket")
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    assert!(!samples.is_empty(), "latency sample set is nonempty");
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = (percentile * sorted.len())
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

async fn run_profile(profile: usize) {
    let (_directory, store, issuer) = provision_store().await;
    let store = Arc::new(store);
    let watch_service = WatchService::new(Arc::clone(&store));
    let mut watch = watch_service
        .open(watch_request())
        .await
        .expect("open production Resource-API watch");
    assert_eq!(watch.receipt().snapshot_revision, ZoneRevision::new(0));

    let metrics = Arc::new(LaunchMetrics::new());
    let provider = Arc::new(MinijailProcessProvider::new(
        ProviderSupervisor::with_limits(
            RecordingEffectBackend::new(Arc::clone(&metrics)),
            PROVIDER_CONCURRENCY,
            Duration::from_secs(1),
        ),
    ));
    let launch_admission = Arc::new(Semaphore::new(PROVIDER_CONCURRENCY));
    let commit_times = Arc::new(Mutex::new(BTreeMap::<ResourceRef, Instant>::new()));
    let issuer = Arc::new(Mutex::new(issuer));
    let mut commit_tasks = Vec::with_capacity(profile);
    for index in 0..profile {
        let target = process_ref(index);
        let store = Arc::clone(&store);
        let issuer = Arc::clone(&issuer);
        let commit_times = Arc::clone(&commit_times);
        commit_tasks.push(tokio::spawn(async move {
            let sealed = issuer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .seal(mutation_body(profile, index, &target));
            let result = store
                .commit_verified(sealed)
                .await
                .expect("commit ready Process through production redb backend");
            assert_eq!(result.resources.len(), 1);
            // The concrete writer sends the live batch only after durable
            // commit and resolves this future after dispatch.
            commit_times
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(target.clone(), Instant::now());
            (target, result)
        }));
    }

    let handler_records = Arc::new(Mutex::new(Vec::<HandlerRecord>::new()));
    let mut handler_tasks = Vec::with_capacity(profile);
    let mut watch_revisions = Vec::new();
    let mut seen = 0_usize;
    while seen < profile {
        let batch = tokio::time::timeout(WATCH_TIMEOUT, watch.recv())
            .await
            .expect("production watch delivered before timeout")
            .expect("production watch remained connected");
        watch_revisions.push(batch.revision());
        for entry in batch.entries() {
            assert_eq!(
                entry.event(),
                d2b_resource_store_redb::ChangeEvent::Created,
                "ready Process watch entry is a create"
            );
            let target =
                ResourceRef::new(entry.resource_type().clone(), entry.resource_name().clone());
            let store = Arc::clone(&store);
            let provider = Arc::clone(&provider);
            let launch_admission = Arc::clone(&launch_admission);
            let handler_records = Arc::clone(&handler_records);
            let operation = operation_uid(profile, seen);
            seen += 1;
            handler_tasks.push(tokio::spawn(async move {
                let started_at = Instant::now();
                let resource = store
                    .get(get_request(&target))
                    .await
                    .expect("handler read fresh Process through production redb backend");
                handler_records
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(HandlerRecord {
                        resource_ref: resource.resource_ref.clone(),
                        resource_uid: resource.uid.clone(),
                        started_at,
                    });
                let _permit = launch_admission
                    .acquire_owned()
                    .await
                    .expect("Process launch admission remains open");
                provider
                    .launch(&launch_ticket(&resource, operation))
                    .await
                    .expect("ready Process reaches the real Process Provider");
            }));
        }
    }

    for task in commit_tasks {
        task.await.expect("commit task joined");
    }
    for task in handler_tasks {
        task.await.expect("Process handler task joined");
    }

    let commit_times = commit_times
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let handlers = handler_records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let launches = metrics.starts();
    assert_eq!(commit_times.len(), profile);
    assert_eq!(handlers.len(), profile);
    assert_eq!(launches.len(), profile);
    let handler_refs = handlers
        .iter()
        .map(|record| record.resource_ref.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(handler_refs.len(), profile);
    let launch_by_uid = launches.into_iter().collect::<BTreeMap<_, _>>();
    assert_eq!(launch_by_uid.len(), profile);

    let handler_samples = handlers
        .iter()
        .map(|record| {
            record
                .started_at
                .saturating_duration_since(commit_times[&record.resource_ref])
        })
        .collect::<Vec<_>>();
    let launch_samples = handlers
        .iter()
        .map(|record| {
            launch_by_uid[&record.resource_uid]
                .saturating_duration_since(commit_times[&record.resource_ref])
        })
        .collect::<Vec<_>>();
    let handler_p95 = percentile(&handler_samples, 95);
    let launch_p95 = percentile(&launch_samples, 95);
    assert!(
        handler_p95 <= HANDLER_P95_LIMIT,
        "commit-to-handler p95 {:?} exceeded the 5 ms contract",
        handler_p95
    );
    assert!(
        launch_p95 <= LAUNCH_P95_LIMIT,
        "commit-to-launch p95 {:?} exceeded the 20 ms contract",
        launch_p95
    );
    if profile > 1 {
        assert!(
            metrics.max_active() >= 2,
            "independent Process launches were serialized"
        );
    }

    let backend_signals: BackendSignals = store.signals();
    let watch_signals: WatchSignals = watch_service
        .signals()
        .expect("read production watch saturation signals");
    assert!(!watch_revisions.is_empty());
    assert!(backend_signals.shared_immutable_batches > 0);
    assert!(backend_signals.fanout_references > 0);
    assert_eq!(watch_signals.current_registrations, 1);
    assert_eq!(watch_signals.budget_used, watch_revisions.len() as u64);
    watch
        .acknowledge(
            *watch_revisions
                .iter()
                .max()
                .expect("watch has a delivered revision"),
        )
        .await
        .expect("acknowledge production watch deliveries");
    let watch_signals = watch_service
        .signals()
        .expect("read acknowledged watch signals");
    assert_eq!(watch_signals.budget_used, 0);
    watch
        .close()
        .await
        .expect("close production watch registration");

    println!(
        "reaction profile={profile} handler_p95_us={:.3} launch_p95_us={:.3} max_active={} revisions={} shared_batches={} fanout_references={}",
        handler_p95.as_secs_f64() * 1_000_000.0,
        launch_p95.as_secs_f64() * 1_000_000.0,
        metrics.max_active(),
        watch_revisions.len(),
        backend_signals.shared_immutable_batches,
        backend_signals.fanout_references,
    );

    drop(watch_service);
    let store = Arc::try_unwrap(store).expect("all store handles released");
    store
        .shutdown()
        .await
        .expect("shutdown production redb backend");
}

#[test]
fn production_reaction_path() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("create benchmark runtime")
        .block_on(async {
            for profile in PROFILES {
                run_profile(profile).await;
            }
        });
}
