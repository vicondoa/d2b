//! Zone-bound Provider lifecycle acceptance through the daemon boundary.
//!
//! The effect fixture is filesystem-backed rather than a call recorder.  It
//! gives the daemon a durable process state to observe, mutate, and adopt
//! after reconstruction, while the Provider registry and lifecycle admission
//! remain the production implementations under test.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue,
    ConfigurationGeneration,
    ControllerGeneration,
    RESOURCE_ENVELOPE_DOMAIN_TAG,
    ResourceEnvelope,
    ResourceName,
    ResourceRef,
    ResourceUid,
    SchemaFingerprint,
    ZoneId,
    ZoneRevision,
    canonical_digest,
    device::DeviceSpec,
    guest::GuestSpec,
    storage::ZoneStoreId,
};
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext,
    BindingDigest,
    EvidenceClass,
    Locality,
    ReconnectGeneration,
    ServiceName,
    SessionBinding,
    SessionPurpose,
    TranscriptHash,
    TransportBinding,
    STANDARD_RESOURCE_TYPES,
};
use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, OpenZoneStoreResponse, ZoneStoreDisposition,
};
use d2b_core_controller::controller_assignment::ScopedCommitTransport;
use d2b_resource_api::{
    RedbBackend, ResourceBusAdapter, ResourceService,
    authz::{
        ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb, SessionVerb,
    },
};
use d2b_resource_store::{PolicySnapshot, StoreSlot};
use d2b_resource_store_redb::{
    DecodedKey, DecodedKeyComponent, DecodedValue, RedbResourceStore, StoreIdentity, ValueKind,
    write_provisioning_marker,
};
use d2bd::provider_effects::{
    EffectDispatch, GuestLifecycleOperation, GuestLifecycleRequest, GuestLifecycleState,
    ProviderEffectError, ProviderLifecycleDispatch, ProviderLifecycleEffectPort,
};
use d2bd::provider_registry::{ProviderBinding, ProviderRuntime, ProviderRuntimeDispatch};
use d2bd::resource_runtime::ZoneResourceRuntime;
use d2bd_runtime::resource_store_runtime::OpenedZoneStore;
use protobuf::{EnumOrUnknown, MessageField};
use serde_json::json;
use sha2::{Digest, Sha256};

#[path = "zone_provider_acceptance.rs"]
mod zone_provider_acceptance;

struct FilesystemLifecycle {
    root: PathBuf,
    apply_calls: AtomicUsize,
}

impl FilesystemLifecycle {
    fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            apply_calls: AtomicUsize::new(0),
        }
    }

    fn state_path(&self, request: &GuestLifecycleRequest) -> PathBuf {
        self.root
            .join(format!("{}.state", request.guest().name().as_str()))
    }

    fn write_state(&self, request: &GuestLifecycleRequest, state: &str) {
        let path = self.state_path(request);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .expect("open filesystem-backed lifecycle state");
        file.write_all(state.as_bytes())
            .expect("write filesystem-backed lifecycle state");
        file.sync_all().expect("sync lifecycle state");
    }
}

impl ProviderLifecycleEffectPort for FilesystemLifecycle {
    type Output = GuestLifecycleState;

    fn actual_state(
        &self,
        request: &GuestLifecycleRequest,
    ) -> Result<GuestLifecycleState, ProviderEffectError> {
        match fs::read_to_string(self.state_path(request)) {
            Ok(contents) => match contents.as_str() {
                "started" => Ok(GuestLifecycleState::Started),
                "stopped" => Ok(GuestLifecycleState::Stopped),
                _ => Err(ProviderEffectError::StateUnavailable),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(GuestLifecycleState::Stopped)
            }
            Err(_) => Err(ProviderEffectError::StateUnavailable),
        }
    }

    fn apply(&self, request: &GuestLifecycleRequest) -> Result<Self::Output, ProviderEffectError> {
        self.apply_calls.fetch_add(1, Ordering::SeqCst);
        let state = match request.operation() {
            GuestLifecycleOperation::Start => ("started", GuestLifecycleState::Started),
            GuestLifecycleOperation::Stop => ("stopped", GuestLifecycleState::Stopped),
        };
        self.write_state(request, state.0);
        Ok(state.1)
    }
}

fn zone() -> ZoneId {
    ZoneId::parse("work").expect("valid Zone")
}

fn provider_binding(name: &str) -> ProviderBinding {
    ProviderBinding::new(
        zone(),
        ResourceRef::parse(&format!("Provider/{name}")).expect("valid Provider ref"),
        ResourceName::parse(name).expect("valid Provider name"),
        "sha256:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("valid Provider binding")
}

fn request(operation: GuestLifecycleOperation, key: &str) -> GuestLifecycleRequest {
    GuestLifecycleRequest::new(
        zone(),
        ResourceRef::parse("Guest/workstation").expect("valid Guest ref"),
        operation,
        key,
    )
    .expect("valid lifecycle request")
}

#[test]
fn activation_refusal_and_removal_cross_the_provider_boundary() {
    let directory = tempfile::tempdir().expect("temporary lifecycle state");
    let effect = FilesystemLifecycle::new(directory.path());
    let runtime = ProviderRuntime::from_bindings(
        zone(),
        1,
        [provider_binding("runtime")],
        [(
            "workstation".to_owned(),
            ResourceRef::parse("Provider/runtime").expect("Provider route"),
        )],
    )
    .expect("compose Provider runtime");
    let admin = BrokerCallerRole::AdminUid { uid: 1000 };

    assert_eq!(
        runtime
            .dispatch_lifecycle(
                &admin,
                "workstation",
                GuestLifecycleOperation::Start,
                "activate-workstation",
                &effect,
            )
            .expect("activate Guest"),
        ProviderRuntimeDispatch::Active(EffectDispatch::Dispatched(GuestLifecycleState::Started))
    );
    assert_eq!(
        effect.actual_state(&request(GuestLifecycleOperation::Start, "state")),
        Ok(GuestLifecycleState::Started)
    );

    assert_eq!(
        runtime.dispatch_lifecycle(
            &BrokerCallerRole::NotAuthorized,
            "workstation",
            GuestLifecycleOperation::Stop,
            "refused-stop",
            &effect,
        ),
        Err(ProviderEffectError::CallerRoleDenied)
    );
    assert_eq!(
        effect.actual_state(&request(GuestLifecycleOperation::Start, "state")),
        Ok(GuestLifecycleState::Started),
        "authorization refusal must not mutate the process state"
    );

    assert_eq!(
        runtime
            .dispatch_lifecycle(
                &admin,
                "workstation",
                GuestLifecycleOperation::Stop,
                "remove-workstation",
                &effect,
            )
            .expect("remove Guest"),
        ProviderRuntimeDispatch::Active(EffectDispatch::Dispatched(GuestLifecycleState::Stopped))
    );
    assert_eq!(
        effect.actual_state(&request(GuestLifecycleOperation::Stop, "state")),
        Ok(GuestLifecycleState::Stopped)
    );
}

#[test]
fn pending_activation_is_adopted_after_daemon_restart() {
    let directory = tempfile::tempdir().expect("temporary lifecycle state");
    let state_path = directory.path().join("provider-lifecycle.json");
    let effect = FilesystemLifecycle::new(directory.path());
    let admin = BrokerCallerRole::AdminUid { uid: 1000 };
    let activation = request(GuestLifecycleOperation::Start, "restart-adoption");

    let first = ProviderLifecycleDispatch::new_persistent(zone(), &state_path)
        .expect("create first durable dispatcher");
    assert_eq!(
        first.admit(&admin, &activation).expect("admit activation"),
        d2bd::provider_effects::LifecycleDispatch::Dispatch
    );
    effect
        .apply(&activation)
        .expect("external process activation");
    drop(first);

    let restarted = ProviderLifecycleDispatch::new_persistent(zone(), &state_path)
        .expect("recreate durable dispatcher");
    assert_eq!(
        restarted
            .dispatch(&admin, &activation, &effect)
            .expect("adopt already-running process"),
        EffectDispatch::Duplicate
    );
    assert_eq!(
        effect.apply_calls.load(Ordering::SeqCst),
        1,
        "restart adoption must not launch a second process"
    );
}

fn stable_uid(domain: &str, value: &str) -> ResourceUid {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(value.as_bytes());
    let mut bytes: [u8; 16] = digest.finalize()[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ResourceUid::parse(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
    .unwrap()
}

fn operator_context(
    zone: &ZoneId,
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
) -> AuthenticatedSubjectContext {
    AuthenticatedSubjectContext::new(
        subject_ref,
        subject_uid,
        ResourceRef::parse(&format!("Zone/{}", zone.as_str())).unwrap(),
        EvidenceClass::UnixPeer,
        SessionPurpose::parse("zone-bus").unwrap(),
        ServiceName::parse("d2b.resource.v3").unwrap(),
        SessionBinding::new(
            SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            TransportBinding::new(
                Locality::Local,
                BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
            ),
            ReconnectGeneration::new(1).unwrap(),
            TranscriptHash::from_bytes([3; 32]),
        ),
    )
}

fn list_request(resource_type: &str) -> wire::ListRequest {
    let mut request = wire::ListRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = format!("operator-list-{resource_type}");
    meta.correlation_id = meta.operation_id.clone();
    request.meta = MessageField::some(meta);
    request.resource_types.push(resource_type.to_owned());
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    request.projection = MessageField::some(projection);
    request
}

fn resource_policy(zone: &ZoneId, snapshot: PolicySnapshot) -> (PolicySet, AuthorizationState) {
    let catalog = ApiCatalog::standard();
    let resource_types = STANDARD_RESOURCE_TYPES
        .iter()
        .map(|name| d2b_contracts_resource::v3::ResourceTypeName::parse(*name).unwrap())
        .collect::<Vec<_>>();
    let resource_verbs = [
        ResourceVerb::Get,
        ResourceVerb::List,
        ResourceVerb::Watch,
        ResourceVerb::Create,
        ResourceVerb::UpdateSpec,
        ResourceVerb::UpdateStatus,
        ResourceVerb::UpdateMetadata,
        ResourceVerb::UpdateFinalizers,
        ResourceVerb::Delete,
    ];
    let session_verbs = [
        SessionVerb::Connect,
        SessionVerb::Invoke,
        SessionVerb::OpenStream,
        SessionVerb::Cancel,
    ];
    let mut rules = Vec::new();
    for chunk in resource_types.chunks(16) {
        rules.push(
            PolicyRule::new(
                &catalog,
                chunk.iter().cloned(),
                resource_verbs,
                session_verbs,
                [],
                [],
                [zone.clone()],
                [],
            )
            .unwrap(),
        );
    }
    let role_ref = ResourceRef::parse("Role/system-core-runtime").unwrap();
    let role = CompiledRole::new(role_ref.clone(), rules).unwrap();
    let binding = CompiledRoleBinding::new(
        role_ref,
        [BoundSubject {
            subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
            subject_uid: ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
        }],
        BindingScope {
            zones: [zone.clone()].into_iter().collect(),
            ..BindingScope::default()
        },
        RelayGrantAuthority::None,
    )
    .unwrap();
    let policy = PolicySet::new(
        &catalog,
        snapshot.policy_revision,
        vec![role],
        vec![binding],
    )
    .unwrap();
    let state = AuthorizationState {
        snapshot,
        zone_policy_revision: ZoneRevision::new(0),
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    };
    (policy, state)
}

async fn seed_host_resource(
    zone: &ZoneId,
    database_path: &std::path::Path,
    marker_path: &std::path::Path,
    marker_identity: &str,
    store_identity: StoreIdentity,
    snapshot: PolicySnapshot,
) {
    let (policy, state) = resource_policy(zone, snapshot);
    let authorizer =
        std::sync::Arc::new(NativeAuthorizer::new(ApiCatalog::standard(), Some(policy)).unwrap());
    let acceptor = authorizer
        .take_store_seal(store_identity.seal_identity())
        .unwrap();
    let database = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(database_path)
        .expect("create redb file");
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(marker_path)
        .expect("create store marker");
    write_provisioning_marker(&mut marker, &store_identity).expect("write store marker");
    let store = std::sync::Arc::new(
        RedbResourceStore::provision_owned(database, marker, store_identity, acceptor)
            .await
            .expect("provision resource store"),
    );
    let backend = std::sync::Arc::new(RedbBackend::from_arc(std::sync::Arc::clone(&store)));
    let service = std::sync::Arc::new(
        ResourceService::new(backend, std::sync::Arc::clone(&authorizer)).unwrap(),
    );
    let subject = authorizer
        .issue_authenticated_subject(
            operator_context(
                zone,
                ResourceRef::parse("Provider/system-core").unwrap(),
                ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            ),
            state,
        )
        .unwrap();
    let adapter = ResourceBusAdapter::bind_component_session(service, subject).unwrap();
    let client = adapter.client();
    let payload = CanonicalJsonValue::parse(
        br#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"configurationGeneration":1,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"host-system","ownerRef":null,"revision":1,"updatedAt":"2026-07-22T00:00:00.000Z","zone":"work"},"spec":{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}},"type":"Host"}"#,
    )
    .unwrap()
    .to_canonical_bytes();
    let target = ResourceRef::parse("Host/host-system").unwrap();
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = zone.as_str().to_owned();
    identity.resource_type = "Host".to_owned();
    identity.name = "host-system".to_owned();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = MessageField::some(identity.clone());
    body.payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &payload);
    body.canonical_json = payload;
    let mut precondition = wire::Precondition::new();
    precondition.kind = EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = MessageField::some(identity);
    mutation.precondition = MessageField::some(precondition);
    mutation.resource = MessageField::some(body);
    let mut request = wire::CreateRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = "seed-host".to_owned();
    meta.correlation_id = "seed-host".to_owned();
    meta.idempotency_key = "seed-host".to_owned();
    request.meta = MessageField::some(meta);
    request.mutation = MessageField::some(mutation);
    let response = client.create(request).await;
    assert!(
        response.error.is_none(),
        "seed Host failed: kind={:?} reason={}",
        response.error.as_ref().map(|error| error.kind),
        response
            .error
            .as_ref()
            .map_or("<none>", |error| error.reason.as_str())
    );
    assert_eq!(target.to_canonical_string(), "Host/host-system");
    drop(client);
    drop(adapter);
    let store = std::sync::Arc::try_unwrap(store).expect("release seed store");
    store
        .shutdown()
        .await
        .expect("cleanly close resource store");
    assert!(marker_identity.starts_with("sha256:"));
}

async fn create_operator_resource(
    client: &d2b_resource_api::ResourceApiClient<
        RedbBackend,
        d2b_resource_api::service::UnavailableUpgradeDispatcher,
    >,
    resource_type: &str,
    name: &str,
    provider_ref: &str,
    operation_id: &str,
) {
    let mut spec = match resource_type {
        "Volume" => serde_json::to_value(zone_provider_acceptance::volume_spec())
            .expect("serialize Volume spec"),
        "Network" => serde_json::to_value(zone_provider_acceptance::network_spec())
            .expect("serialize Network spec"),
        "Device" => {
            serde_json::to_value(DeviceSpec::emulated_exclusive()).expect("serialize Device spec")
        }
        "Guest" => serde_json::to_value(GuestSpec::system_default()).expect("serialize Guest spec"),
        _ => panic!("unsupported operator acceptance resource type"),
    };
    let spec_object = spec
        .as_object_mut()
        .expect("typed operator spec is an object");
    spec_object.insert("providerRef".to_owned(), json!(provider_ref));
    spec_object.insert(
        "updatePolicy".to_owned(),
        json!({
            "disruptive": "manual",
            "nonDisruptive": "automatic"
        }),
    );
    let payload = CanonicalJsonValue::parse(
        &serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "metadata": {
                "configurationGeneration": 1,
                "createdAt": "2026-07-22T00:00:00.000Z",
                "deletionRequestedAt": null,
                "finalizers": [],
                "generation": 1,
                "managedBy": "configuration",
                "name": name,
                "ownerRef": null,
                "revision": 1,
                "updatedAt": "2026-07-22T00:00:00.000Z",
                "zone": "work"
            },
            "spec": spec,
            "status": {
                "completedAt": null,
                "conditions": [],
                "lastReconciledAt": null,
                "observedGeneration": 0,
                "outcome": null,
                "phase": "Pending",
                "resource": {},
                "startedAt": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                }
            },
            "type": resource_type
        }))
        .expect("serialize operator resource"),
    )
    .expect("canonicalize operator resource")
    .to_canonical_bytes();
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = "work".to_owned();
    identity.resource_type = resource_type.to_owned();
    identity.name = name.to_owned();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = MessageField::some(identity.clone());
    body.payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &payload);
    body.canonical_json = payload;
    let mut precondition = wire::Precondition::new();
    precondition.kind = EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = MessageField::some(identity);
    mutation.precondition = MessageField::some(precondition);
    mutation.resource = MessageField::some(body);
    let mut request = wire::CreateRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.correlation_id = operation_id.to_owned();
    meta.idempotency_key = operation_id.to_owned();
    request.meta = MessageField::some(meta);
    request.mutation = MessageField::some(mutation);
    let response = client.create(request).await;
    assert!(
        response.error.is_none(),
        "seed {resource_type} failed: kind={:?} reason={}",
        response.error.as_ref().map(|error| error.kind),
        response
            .error
            .as_ref()
            .map_or("<none>", |error| error.reason.as_str())
    );
}

#[tokio::test]
async fn authenticated_operator_reaches_ready_resource_plane_and_refuses_other_subjects() {
    let directory = tempfile::tempdir().expect("resource-plane directory");
    let zone = ZoneId::parse("work").unwrap();
    let marker_identity = format!("sha256:{}", "c".repeat(64));
    let database_path = directory.path().join("store.redb");
    let marker_path = directory.path().join("store.marker");
    let store_identity = StoreIdentity::new(
        StoreSlot::new(0).unwrap(),
        stable_uid("store", &marker_identity),
        zone.clone(),
        stable_uid("zone", zone.as_str()),
        d2b_contracts_resource::v3::Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
        PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
            controller_generation: Some(ControllerGeneration::new(1).unwrap()),
        },
    );
    seed_host_resource(
        &zone,
        &database_path,
        &marker_path,
        &marker_identity,
        store_identity,
        PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
            controller_generation: Some(ControllerGeneration::new(1).unwrap()),
        },
    )
    .await;

    let database = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("reopen redb file");
    assert!(
        rustix::io::fcntl_getfd(&database)
            .expect("read database fd flags")
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
    let mut runtime = ZoneResourceRuntime::open(
        zone.clone(),
        OpenedZoneStore {
            response: OpenZoneStoreResponse {
                zone_store_id: ZoneStoreId::parse("zone-store-work").unwrap(),
                store_identity: marker_identity,
                disposition: ZoneStoreDisposition::Opened,
                fd_index: 0,
            },
            database_fd: database.into(),
            external_inventory: None,
        },
    )
    .await
    .expect("open production Zone runtime");
    runtime.set_provider_path_ready(true);
    println!("runtime readiness: {:?}", runtime.readiness());
    assert!(runtime.readiness().is_ready());

    let (operator_ref, operator_uid) = ZoneResourceRuntime::operator_subject_identity();
    let client = runtime
        .bind_operator_resource_client(operator_context(&zone, operator_ref, operator_uid))
        .expect("bind authenticated operator Resource API client");
    let response = client.list(list_request("Volume")).await;
    assert!(
        response.error.is_none(),
        "authenticated operator list failed: kind={:?} reason={}",
        response.error.as_ref().map(|error| error.kind),
        response
            .error
            .as_ref()
            .map_or("<none>", |error| error.reason.as_str())
    );

    let refused = runtime.bind_operator_resource_client(operator_context(
        &zone,
        ResourceRef::parse("User/not-authorized").unwrap(),
        ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
    ));
    assert!(
        refused.is_err(),
        "unbound User must not inherit operator Resource API grants"
    );
    drop(client);
    runtime.shutdown().await.expect("shutdown resource runtime");
}

#[tokio::test]
async fn scoped_status_finalizer_batch_reaches_redb_atomically_and_rebinds_assignment() {
    let directory = tempfile::tempdir().expect("resource-plane directory");
    let zone = ZoneId::parse("work").unwrap();
    let marker_identity = format!("sha256:{}", "e".repeat(64));
    let database_path = directory.path().join("store.redb");
    let marker_path = directory.path().join("store.marker");
    let snapshot = PolicySnapshot {
        policy_revision: 1,
        api_catalog_revision: 1,
        active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
        controller_generation: Some(ControllerGeneration::new(1).unwrap()),
    };
    let store_identity = StoreIdentity::new(
        StoreSlot::new(0).unwrap(),
        stable_uid("store", &marker_identity),
        zone.clone(),
        stable_uid("zone", zone.as_str()),
        d2b_contracts_resource::v3::Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
        snapshot,
    );
    seed_host_resource(
        &zone,
        &database_path,
        &marker_path,
        &marker_identity,
        store_identity,
        snapshot,
    )
    .await;

    let database = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("reopen redb file");
    let mut runtime = ZoneResourceRuntime::open(
        zone.clone(),
        OpenedZoneStore {
            response: OpenZoneStoreResponse {
                zone_store_id: ZoneStoreId::parse("zone-store-work").unwrap(),
                store_identity: marker_identity,
                disposition: ZoneStoreDisposition::Opened,
                fd_index: 0,
            },
            database_fd: database.into(),
            external_inventory: None,
        },
    )
    .await
    .expect("open production Zone runtime");
    runtime.set_provider_path_ready(true);

    let (operator_ref, operator_uid) = ZoneResourceRuntime::operator_subject_identity();
    let controller_generation = runtime
        .committed_policy_snapshot()
        .controller_generation
        .expect("runtime controller generation");
    let client = runtime
        .bind_operator_resource_client(
            operator_context(&zone, operator_ref, operator_uid)
                .with_controller_generation(controller_generation),
        )
        .expect("bind authenticated operator Resource API client");
    let mut target_identity = wire::ResourceIdentity::new();
    target_identity.zone = zone.as_str().to_owned();
    target_identity.resource_type = "Host".to_owned();
    target_identity.name = "host-system".to_owned();
    let mut get = wire::GetRequest::new();
    let mut get_meta = wire::RequestMeta::new();
    get_meta.operation_id = "scoped-assignment-read".to_owned();
    get_meta.correlation_id = get_meta.operation_id.clone();
    get.meta = MessageField::some(get_meta);
    get.target = MessageField::some(target_identity);
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    get.projection = MessageField::some(projection);
    let current = client.get(get).await;
    assert!(
        current.error.is_none(),
        "read seeded Host failed: {:?}",
        current.error
    );
    let current = current.resource.as_ref().expect("seeded Host response");
    let current_identity = current.identity.as_ref().expect("seeded Host identity");
    let uid = ResourceUid::parse(current_identity.uid.as_ref().expect("seeded Host UID")).unwrap();
    let initial_revision = current_identity.revision.expect("seeded Host revision");
    let before_batch = runtime
        .backup_before_live_adoption()
        .await
        .expect("capture pre-batch redb state");
    assert_eq!(before_batch.current_revision, initial_revision);
    let batch_revision = initial_revision
        .checked_add(1)
        .expect("batch revision remains in range");
    let mut status_value = CanonicalJsonValue::parse(&current.canonical_json).unwrap();
    let CanonicalJsonValue::Object(root) = &mut status_value else {
        panic!("seeded Host envelope must be an object");
    };
    let CanonicalJsonValue::Object(status) = root.get_mut("status").expect("seeded Host status")
    else {
        panic!("seeded Host status must be an object");
    };
    status.insert(
        "phase".to_owned(),
        CanonicalJsonValue::String("Ready".to_owned()),
    );
    let status_payload = status_value.to_canonical_bytes();
    let status_digest = ResourceEnvelope::from_json(&status_payload)
        .unwrap()
        .digest()
        .unwrap();
    let make_precondition = |revision| {
        let mut precondition = wire::Precondition::new();
        precondition.kind =
            EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
        precondition.expected_revision = Some(revision);
        precondition.expected_uid = Some(uid.as_str().to_owned());
        precondition
    };
    let make_batch = |operation_id: &str, status_revision: u64, finalizer_revision: u64| {
        let mut status_mutation = wire::Mutation::new();
        status_mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
        status_mutation.target = MessageField::some(current_identity.clone());
        status_mutation.precondition = MessageField::some(make_precondition(status_revision));
        let mut status_body = wire::ResourceEnvelopeBytes::new();
        status_body.identity = MessageField::some(current_identity.clone());
        status_body.canonical_json = status_payload.clone();
        status_body.payload_digest = status_digest.clone();
        status_mutation.resource = MessageField::some(status_body);

        let mut finalizer_mutation = wire::Mutation::new();
        finalizer_mutation.kind =
            EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
        finalizer_mutation.target = MessageField::some(current_identity.clone());
        finalizer_mutation.precondition = MessageField::some(make_precondition(finalizer_revision));
        finalizer_mutation
            .add_finalizers
            .push("core.controller-batch".to_owned());

        let mut request = wire::CommitBatchRequest::new();
        let mut meta = wire::RequestMeta::new();
        meta.operation_id = operation_id.to_owned();
        meta.idempotency_key = operation_id.to_owned();
        meta.correlation_id = operation_id.to_owned();
        request.meta = MessageField::some(meta);
        request.mutations = vec![status_mutation, finalizer_mutation];
        request
    };
    let transport = ScopedCommitTransport::decode(
        format!(
            r#"{{"version":1,"assignment":{{"resourceUid":"{}","resourceRevision":{},"providerGeneration":2,"controllerGeneration":3,"controllerRole":"Process/process-controller","target":{{"kind":"execution","targetKind":"host","reference":"Host/host-system"}},"sessionGeneration":1,"epoch":1}},"mutations":[{{"target":"Host/host-system","verb":"UpdateStatus"}},{{"target":"Host/host-system","verb":"UpdateFinalizers"}}]}}"#,
            uid.as_str(),
            initial_revision,
        )
        .as_bytes(),
    )
    .unwrap();

    let committed = client
        .scoped_commit_batch(
            make_batch("scoped-assignment-batch", initial_revision, batch_revision),
            transport.mutations().to_vec(),
        )
        .await;
    assert!(
        committed.error.is_none(),
        "valid scoped batch failed: kind={:?} reason={}",
        committed.error.as_ref().map(|error| error.kind),
        committed
            .error
            .as_ref()
            .map_or("<none>", |error| error.reason.as_str())
    );
    assert_eq!(committed.revision, batch_revision);
    assert_eq!(committed.resources.len(), 2);

    let backup = runtime
        .backup_before_live_adoption()
        .await
        .expect("capture committed redb backup");
    assert_eq!(backup.current_revision, batch_revision);
    let resources = backup
        .tables
        .iter()
        .find(|table| table.name == "resources")
        .expect("resources table in backup");
    let resource_row = resources
        .rows
        .iter()
        .find(|row| {
            let key = DecodedKey::decode(&row.key).expect("decode resource key");
            matches!(
                key.components(),
                [
                    DecodedKeyComponent::Text(resource_type),
                    DecodedKeyComponent::Text(resource_name)
                ] if resource_type == "Host" && resource_name == "host-system"
            )
        })
        .expect("durable Host resource row");
    let encoded_record = DecodedValue::decode(&resource_row.value).expect("decode resource record");
    assert_eq!(encoded_record.kind(), ValueKind::ResourceRecord);
    let record: serde_json::Value = serde_json::from_slice(encoded_record.canonical_json())
        .expect("decode resource record JSON");
    assert_eq!(record["assignment"]["resourceRevision"], batch_revision);
    let canonical_json: Vec<u8> =
        serde_json::from_value(record["canonical_json"].clone()).expect("decode stored envelope");
    let envelope = ResourceEnvelope::from_json(&canonical_json).expect("stored envelope");
    assert_eq!(
        envelope.status().phase(),
        d2b_contracts_resource::v3::ResourcePhase::Ready
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&canonical_json)
            .expect("decode stored envelope JSON")["metadata"]["finalizers"]
            .as_array()
            .expect("stored finalizer array")
            .iter()
            .any(|finalizer| finalizer == "core.controller-batch")
    );

    let stale = client
        .scoped_commit_batch(
            make_batch(
                "scoped-stale-assignment-batch",
                batch_revision,
                batch_revision
                    .checked_add(1)
                    .expect("stale batch revision remains in range"),
            ),
            transport.mutations().to_vec(),
        )
        .await;
    assert!(
        stale.error.is_some(),
        "stale assignment batch must be rejected"
    );
    assert_eq!(
        stale
            .error
            .as_ref()
            .map(|error| error.kind.enum_value_or_default()),
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT)
    );
    assert_eq!(
        stale.error.as_ref().map(|error| error.reason.as_str()),
        Some("stale-assignment")
    );
    let after_stale = runtime
        .backup_before_live_adoption()
        .await
        .expect("capture redb state after stale batch");
    assert_eq!(after_stale.current_revision, batch_revision);
    drop(client);
    runtime.shutdown().await.expect("shutdown resource runtime");
}

#[tokio::test]
async fn authenticated_operator_drives_wave6_resources_through_production_boundary() {
    let directory = tempfile::tempdir().expect("resource-plane directory");
    let zone = ZoneId::parse("work").unwrap();
    let marker_identity = format!("sha256:{}", "d".repeat(64));
    let database_path = directory.path().join("store.redb");
    let marker_path = directory.path().join("store.marker");
    let store_identity = StoreIdentity::new(
        StoreSlot::new(0).unwrap(),
        stable_uid("store", &marker_identity),
        zone.clone(),
        stable_uid("zone", zone.as_str()),
        d2b_contracts_resource::v3::Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
        PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
            controller_generation: Some(ControllerGeneration::new(1).unwrap()),
        },
    );
    seed_host_resource(
        &zone,
        &database_path,
        &marker_path,
        &marker_identity,
        store_identity,
        PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
            controller_generation: Some(ControllerGeneration::new(1).unwrap()),
        },
    )
    .await;

    let database = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("reopen redb file");
    let mut runtime = ZoneResourceRuntime::open(
        zone.clone(),
        OpenedZoneStore {
            response: OpenZoneStoreResponse {
                zone_store_id: ZoneStoreId::parse("zone-store-work").unwrap(),
                store_identity: marker_identity,
                disposition: ZoneStoreDisposition::Opened,
                fd_index: 0,
            },
            database_fd: database.into(),
            external_inventory: None,
        },
    )
    .await
    .expect("open production Zone runtime");
    runtime.set_provider_path_ready(true);
    assert!(runtime.readiness().is_ready());

    let (operator_ref, operator_uid) = ZoneResourceRuntime::operator_subject_identity();
    let client = runtime
        .bind_operator_resource_client(operator_context(&zone, operator_ref, operator_uid))
        .expect("bind authenticated operator Resource API client");
    for (resource_type, name, operation_id) in [
        ("Volume", "store", "seed-wave6-volume"),
        ("Network", "work", "seed-wave6-network"),
        ("Device", "work-tpm", "seed-wave6-device"),
        ("Guest", "workstation", "seed-wave6-guest"),
    ] {
        create_operator_resource(
            client.as_ref(),
            resource_type,
            name,
            "Provider/system-core",
            operation_id,
        )
        .await;
    }

    let boundary =
        zone_provider_acceptance::Wave6RealBoundary::new(directory.path().join("provider-effects"));
    let report = runtime
        .reconcile_wave6_operator_acceptance(&client, &boundary)
        .await
        .expect("operator acceptance reaches all four Providers");
    assert!(report.ready);
    assert!(report.adopted_after_restart);
    assert!(report.removed);
    assert!(report.device_state_retained);
    assert_eq!(
        report.resources.volume.provider_ref.to_canonical_string(),
        "Provider/system-core"
    );
    assert_eq!(
        report.resources.network.resource_ref.to_canonical_string(),
        "Network/work"
    );
    assert_eq!(
        report
            .resources
            .cloud_hypervisor_guest
            .resource_ref
            .to_canonical_string(),
        "Guest/workstation"
    );

    let refused = runtime.bind_operator_resource_client(operator_context(
        &zone,
        ResourceRef::parse("User/not-authorized").unwrap(),
        ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
    ));
    assert!(
        refused.is_err(),
        "unauthorized subject must not reach the acceptance boundary"
    );
    drop(client);
    runtime.shutdown().await.expect("shutdown resource runtime");
}
