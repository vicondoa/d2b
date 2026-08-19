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

use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    broker_wire::{OpenZoneStoreResponse, ZoneStoreDisposition},
    resource_proto as wire,
    v3::{
        AuthenticatedSubjectContext, BindingDigest, CanonicalJsonValue, ConfigurationGeneration,
        ControllerGeneration, EvidenceClass, Locality, RESOURCE_ENVELOPE_DOMAIN_TAG,
        ReconnectGeneration, ResourceName, ResourceRef, ResourceUid, SchemaFingerprint,
        ServiceName, SessionBinding, SessionPurpose, TranscriptHash, TransportBinding, ZoneId,
        ZoneRevision, canonical_digest, device::DeviceSpec, guest::GuestSpec,
        identity::STANDARD_RESOURCE_TYPES, storage::ZoneStoreId,
    },
};
use d2b_resource_api::{
    RedbBackend, ResourceBusAdapter, ResourceService,
    authz::{
        ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb, SessionVerb,
    },
};
use d2b_resource_store::{PolicySnapshot, StoreSlot};
use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity, write_provisioning_marker};
use d2bd::provider_effects::{
    EffectDispatch, GuestLifecycleOperation, GuestLifecycleRequest, GuestLifecycleState,
    ProviderEffectError, ProviderLifecycleDispatch, ProviderLifecycleEffectPort,
};
use d2bd::provider_registry::{ProviderBinding, ProviderRuntime, ProviderRuntimeDispatch};
use d2bd::resource_runtime::{OpenedZoneStore, ZoneResourceRuntime};
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
        .map(|name| d2b_contracts::v3::ResourceTypeName::parse(*name).unwrap())
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
        d2b_contracts::v3::Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
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
        d2b_contracts::v3::Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
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
            &client,
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
