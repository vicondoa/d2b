use std::fs::OpenOptions;
use std::sync::Arc;

use d2b_bus::{
    BusConfig, OperationId, OperationSpec, ReceivedFrame, ResourceCall, ResourceQuery, StreamError,
    StreamLimits, StreamName, router::production_rss::ProductionWatchHarness,
};
use d2b_contracts::v3::{
    CanonicalJsonValue, ConfigurationGeneration, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef,
    ResourceTypeName, ResourceUid, Timestamp, ZoneId, ZoneRevision, canonical_digest,
};
use d2b_resource_api::watch::{WatchPumpError, WatchService};
use d2b_resource_store::mutation_seal::{MutationSealBody, MutationSealIssuer, mutation_seal_pair};
use d2b_resource_store::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, ExpectedRevision,
    PolicySnapshot, PreparedStoreMutation, ResourceMutationKind, StoreError, StoreGetRequest,
    StoreMutation, StoreOperationContext, StoreProjection, StoreSlot, StoreWatchRequest,
    StoredResource,
};
use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity, write_provisioning_marker};
use tokio::task::JoinHandle;

pub struct NamedWatchConnection {
    incoming: d2b_bus::IncomingStream,
    pump: JoinHandle<Result<(), WatchPumpError>>,
}

impl NamedWatchConnection {
    pub async fn receive(&self) -> Result<ReceivedFrame, StreamError> {
        let frame = self.incoming.receive_next().await?;
        self.incoming
            .grant_frame(&frame, frame.payload().len())
            .await?;
        Ok(frame)
    }

    pub async fn abort(self) {
        self.pump.abort();
        let _ = self.pump.await;
    }
}

pub struct ProductionStore {
    _directory: tempfile::TempDir,
    store: Arc<RedbResourceStore>,
    issuer: MutationSealIssuer,
}

impl ProductionStore {
    pub async fn provision() -> Arc<Self> {
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
        Arc::new(Self {
            _directory: directory,
            store: Arc::new(store),
            issuer,
        })
    }

    pub fn store(&self) -> Arc<RedbResourceStore> {
        Arc::clone(&self.store)
    }

    pub async fn commit_process_batch(
        &self,
        profile: usize,
        start: usize,
        end: usize,
    ) -> Result<Vec<StoredResource>, StoreError> {
        let mut mutations = Vec::with_capacity(end - start);
        let mut targets = Vec::with_capacity(end - start);
        for index in start..end {
            let target = process_ref(index);
            let canonical_resource = process_body(index);
            let payload_digest =
                canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical_resource);
            mutations.push(PreparedStoreMutation::new(
                StoreMutation {
                    kind: ResourceMutationKind::Create,
                    zone: ZoneId::parse("dev").expect("valid Zone"),
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
            ));
            targets.push(target);
        }
        let body = MutationSealBody {
            mutations,
            authorization: authorization_targets(&targets, AdmittedVerb::Create, None),
            policy_snapshot: policy_snapshot(),
            operation: operation_context(&format!("reaction-create-{profile}-{start}")),
        };
        self.store
            .commit_verified(self.issuer.seal(body))
            .await
            .map(|result| result.resources)
    }

    pub async fn commit_status(
        &self,
        target: &ResourceRef,
        candidate: &[u8],
        operation_id: &str,
    ) -> Result<ZoneRevision, StoreError> {
        let resource = self.store.get(get_request(target)).await?;
        let canonical = status_envelope(&resource, candidate);
        let digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
        let body = MutationSealBody {
            mutations: vec![PreparedStoreMutation::new(
                StoreMutation {
                    kind: ResourceMutationKind::UpdateStatus,
                    zone: ZoneId::parse("dev").expect("valid Zone"),
                    target: target.clone(),
                    expected: ExpectedRevision::Exact(resource.revision),
                    expected_uid: Some(resource.uid.clone()),
                    owner: None,
                    canonical_resource: Some(canonical),
                    add_finalizers: Vec::new(),
                    remove_finalizers: Vec::new(),
                    wait_for_reconcile: false,
                    reconcile_deadline_ms: None,
                },
                Some(resource.uid.clone()),
                Some(digest),
            )],
            authorization: authorization(target, AdmittedVerb::UpdateStatus, Some("status")),
            policy_snapshot: policy_snapshot(),
            operation: operation_context(operation_id),
        };
        self.store
            .commit_verified(self.issuer.seal(body))
            .await
            .map(|result| result.revision)
    }

    pub async fn shutdown(self) -> Result<(), StoreError> {
        let store = Arc::try_unwrap(self.store)
            .unwrap_or_else(|_| panic!("all production store handles released"));
        store.shutdown().await
    }
}

pub async fn open_named_watch(
    store: Arc<RedbResourceStore>,
    harness: &ProductionWatchHarness,
    request: StoreWatchRequest,
    id: &str,
) -> NamedWatchConnection {
    let watch = WatchService::new(store)
        .open(request)
        .await
        .expect("open production Resource-API watch");
    let bus_stream = harness
        .caller()
        .open_resource_stream(
            harness.route().clone(),
            OperationSpec::new(
                OperationId::parse(format!("reaction-bus-{id}"))
                    .expect("valid production bus operation"),
                30_000,
            )
            .expect("valid production bus operation"),
            ResourceCall::Watch(
                ResourceQuery::new(
                    vec![ResourceTypeName::parse("Host").expect("valid route ResourceType")],
                    Vec::new(),
                    Vec::new(),
                )
                .expect("valid production route query"),
            ),
            StreamName::parse(format!("reaction-watch:{id}"))
                .expect("valid production stream name"),
            2 * 1024 * 1024,
        )
        .await
        .expect("open authenticated production named stream");
    let incoming = harness
        .take_incoming()
        .expect("production controller incoming stream");
    let pump = tokio::spawn(async move {
        let mut watch = watch;
        watch.pump_to(&bus_stream).await
    });
    NamedWatchConnection { incoming, pump }
}

pub fn bus_config() -> BusConfig {
    BusConfig {
        stream_limits: StreamLimits {
            max_stream_credit: 2 * 1024 * 1024,
            max_aggregate_bytes: 8 * 1024 * 1024,
            max_streams: 128,
            max_frame_bytes: 1024 * 1024,
            max_streams_per_principal: 128,
            max_credit_per_principal: 8 * 1024 * 1024,
            max_queued_bytes_per_principal: 8 * 1024 * 1024,
        },
        ..BusConfig::default()
    }
}

fn store_identity() -> StoreIdentity {
    StoreIdentity::new(
        StoreSlot::new(0).expect("valid store slot"),
        ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("valid store UID"),
        ZoneId::parse("dev").expect("valid Zone"),
        ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("valid Zone UID"),
        Timestamp::parse("2026-07-31T00:00:00.000Z").expect("valid timestamp"),
        policy_snapshot(),
    )
}

fn process_ref(index: usize) -> ResourceRef {
    ResourceRef::parse(&format!("Process/ready-{index}")).expect("valid Process ref")
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
                "zone":"dev"
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

fn status_envelope(resource: &StoredResource, candidate: &[u8]) -> Vec<u8> {
    let mut value =
        CanonicalJsonValue::parse(&resource.canonical_json).expect("stored envelope is canonical");
    let CanonicalJsonValue::Object(root) = &mut value else {
        panic!("stored envelope is an object");
    };
    let status = CanonicalJsonValue::parse(candidate).expect("status candidate is canonical");
    root.insert("status".to_owned(), status);
    value.to_canonical_bytes()
}

fn authorization(
    target: &ResourceRef,
    verb: AdmittedVerb,
    subresource: Option<&str>,
) -> AdmittedAuthorization {
    authorization_targets(std::slice::from_ref(target), verb, subresource)
}

fn authorization_targets(
    targets: &[ResourceRef],
    verb: AdmittedVerb,
    subresource: Option<&str>,
) -> AdmittedAuthorization {
    AdmittedAuthorization {
        zone: ZoneId::parse("dev").expect("valid Zone"),
        subject_ref: ResourceRef::parse("Provider/system-minijail").expect("valid Provider ref"),
        subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333")
            .expect("valid subject UID"),
        targets: targets
            .iter()
            .map(|target| AdmittedAuthorizationTarget {
                resource_type: target.resource_type().clone(),
                resource_name: Some(target.name().clone()),
                verb,
                subresource: subresource.map(str::to_owned),
                execution_ref: None,
            })
            .collect(),
    }
}

fn policy_snapshot() -> PolicySnapshot {
    PolicySnapshot {
        policy_revision: 1,
        api_catalog_revision: 1,
        active_configuration_revision: ConfigurationGeneration::new(1)
            .expect("nonzero configuration generation"),
        controller_generation: None,
    }
}

fn get_request(target: &ResourceRef) -> StoreGetRequest {
    StoreGetRequest {
        operation: operation_context(&format!("reaction-read-{}", target.name().as_str())),
        zone: ZoneId::parse("dev").expect("valid Zone"),
        target: target.clone(),
        expected_uid: None,
        projection: StoreProjection::Full,
    }
}

fn operation_context(id: &str) -> StoreOperationContext {
    StoreOperationContext {
        operation_id: id.to_owned(),
        idempotency_key: Some(format!("{id}-key")),
        correlation_id: format!("{id}-correlation"),
        trace_id: None,
        deadline_ms: 30_000,
    }
}
