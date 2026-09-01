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

use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, OpenZoneStoreResponse, ZoneStoreDisposition,
};
use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    STANDARD_RESOURCE_TYPES, ServiceName, SessionBinding, SessionPurpose, TranscriptHash,
    TransportBinding,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ConfigurationGeneration, ControllerGeneration,
    RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceEnvelope, ResourceName, ResourceRef, ResourceTypeName,
    ResourceUid, SchemaFingerprint, ZoneId, ZoneRevision, canonical_digest, device::DeviceSpec,
    guest::GuestSpec, storage::ZoneStoreId,
};
use d2b_core_controller::controller_assignment::ScopedCommitTransport;
use d2b_provider_display_wayland::{DisplayIdentity, WaylandSessionSpec};
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
    LifecycleAuthorization, ProviderEffectError, ProviderLifecycleDispatch,
    ProviderLifecycleEffectPort,
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
            GuestLifecycleOperation::Restart => ("started", GuestLifecycleState::Started),
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
        LifecycleAuthorization::for_test(
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            1,
            1,
            1,
            key,
        ),
    )
    .expect("valid lifecycle request")
}

fn authorization(operation_id: &str) -> LifecycleAuthorization {
    LifecycleAuthorization::for_test(
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        1,
        1,
        1,
        operation_id,
    )
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
                authorization("activate-workstation"),
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
            authorization("refused-stop"),
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
                authorization("remove-workstation"),
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

fn test_operator_subject_identity() -> (ResourceRef, ResourceUid) {
    (
        ResourceRef::parse("User/d2bd-operator").unwrap(),
        ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
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

fn get_request(resource_type: &str, name: &str, operation_id: &str) -> wire::GetRequest {
    let mut request = wire::GetRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.correlation_id = operation_id.to_owned();
    request.meta = MessageField::some(meta);
    let mut target = wire::ResourceIdentity::new();
    target.zone = "work".to_owned();
    target.resource_type = resource_type.to_owned();
    target.name = name.to_owned();
    request.target = MessageField::some(target);
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    request.projection = MessageField::some(projection);
    request
}

fn delete_request(
    resource_type: &str,
    name: &str,
    revision: u64,
    operation_id: &str,
) -> wire::DeleteRequest {
    let mut request = wire::DeleteRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.correlation_id = operation_id.to_owned();
    meta.idempotency_key = operation_id.to_owned();
    request.meta = MessageField::some(meta);
    let mut target = wire::ResourceIdentity::new();
    target.zone = "work".to_owned();
    target.resource_type = resource_type.to_owned();
    target.name = name.to_owned();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(revision);
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = MessageField::some(target);
    mutation.precondition = MessageField::some(precondition);
    request.mutation = MessageField::some(mutation);
    request
}

fn update_finalizers_request(
    resource_type: &str,
    name: &str,
    revision: u64,
    finalizer: &str,
    add: bool,
    operation_id: &str,
) -> wire::UpdateFinalizersRequest {
    let mut request = wire::UpdateFinalizersRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.correlation_id = operation_id.to_owned();
    meta.idempotency_key = operation_id.to_owned();
    request.meta = MessageField::some(meta);
    let mut target = wire::ResourceIdentity::new();
    target.zone = "work".to_owned();
    target.resource_type = resource_type.to_owned();
    target.name = name.to_owned();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(revision);
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
    mutation.target = MessageField::some(target);
    mutation.precondition = MessageField::some(precondition);
    if add {
        mutation.add_finalizers.push(finalizer.to_owned());
    } else {
        mutation.remove_finalizers.push(finalizer.to_owned());
    }
    request.mutation = MessageField::some(mutation);
    request
}

fn resource_policy(
    zone: &ZoneId,
    snapshot: PolicySnapshot,
) -> (ApiCatalog, PolicySet, AuthorizationState) {
    let catalog = ApiCatalog::with_extensions([
        ResourceTypeName::parse("display-wayland.d2bus.org.WaylandPolicy").unwrap(),
        ResourceTypeName::parse("display-wayland.d2bus.org.WaylandSession").unwrap(),
    ])
    .unwrap();
    let resource_types = STANDARD_RESOURCE_TYPES
        .iter()
        .map(|name| d2b_contracts_resource::v3::ResourceTypeName::parse(*name).unwrap())
        .chain([
            ResourceTypeName::parse("display-wayland.d2bus.org.WaylandPolicy").unwrap(),
            ResourceTypeName::parse("display-wayland.d2bus.org.WaylandSession").unwrap(),
        ])
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
    (catalog, policy, state)
}

async fn open_seeded_resource_api(
    zone: &ZoneId,
    database_path: &std::path::Path,
    marker_path: &std::path::Path,
    store_identity: StoreIdentity,
    snapshot: PolicySnapshot,
) -> (
    std::sync::Arc<RedbResourceStore>,
    ResourceBusAdapter<RedbBackend, d2b_resource_api::service::UnavailableUpgradeDispatcher>,
) {
    let (catalog, policy, state) = resource_policy(zone, snapshot);
    let authorizer = std::sync::Arc::new(NativeAuthorizer::new(catalog, Some(policy)).unwrap());
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
    (store, adapter)
}

async fn seed_host_resource(
    zone: &ZoneId,
    database_path: &std::path::Path,
    marker_path: &std::path::Path,
    marker_identity: &str,
    store_identity: StoreIdentity,
    snapshot: PolicySnapshot,
) {
    let (store, adapter) =
        open_seeded_resource_api(zone, database_path, marker_path, store_identity, snapshot).await;
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
    owner_ref: Option<&str>,
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
        "display-wayland.d2bus.org.WaylandSession" => serde_json::to_value(
            WaylandSessionSpec::new(
                ResourceRef::parse("Guest/workstation").expect("valid Guest ref"),
                ResourceRef::parse("Host/host-system").expect("valid Host ref"),
                ResourceRef::parse("User/alice").expect("valid User ref"),
                ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/display-wayland")
                    .expect("valid WaylandPolicy ref"),
                DisplayIdentity::new("display", "#112233", "#223344", "#334455")
                    .expect("valid display identity"),
                true,
            )
            .expect("valid WaylandSession spec")
            .with_virgl_video(false),
        )
        .expect("serialize WaylandSession spec"),
        "Process" => {
            serde_json::to_value(d2b_contracts_resource::v3::process::ProcessSpec::minimal(
                d2b_contracts_resource::v3::process::ExecutionSpec::minimal(
                    ResourceRef::parse(if name.contains("guest") {
                        "Guest/workstation"
                    } else {
                        "Host/host-system"
                    })
                    .expect("valid Process execution ref"),
                    d2b_contracts_resource::v3::process::ProcessClass::Worker,
                    d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
                        if name.contains("guest") {
                            "wayland-frontend-worker"
                        } else {
                            "wayland-proxy-worker"
                        },
                    )
                    .expect("valid Process template"),
                )
                .expect("minimal Process execution"),
            ))
            .expect("serialize Process spec")
        }
        "Endpoint" => serde_json::to_value(
            d2b_contracts_resource::v3::endpoint::EndpointSpec::new(
                ResourceRef::parse(provider_ref).expect("valid Endpoint provider ref"),
                ResourceRef::parse(if name.contains("guest") {
                    "Process/display-guest-frontend"
                } else {
                    "Process/display-host-proxy"
                })
                .expect("valid Endpoint producer ref"),
                d2b_contracts_resource::v3::endpoint::EndpointClass::Data,
                d2b_contracts_resource::v3::endpoint::EndpointTransport::FdAttachment,
                d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
                    "wayland-cross-domain",
                )
                .expect("valid Endpoint purpose"),
                Some(
                    d2b_contracts_resource::v3::execution_policy::BoundedText::parse(
                        "display-wayland-data-v3",
                    )
                    .expect("valid Endpoint fingerprint"),
                ),
                d2b_contracts_resource::v3::endpoint::EndpointLocality::CrossDomain,
                d2b_contracts_resource::v3::endpoint::EndpointVisibility::Zone,
                d2b_contracts_resource::v3::endpoint::EndpointAttachmentPolicy::new(true, 1)
                    .expect("valid Endpoint attachment policy"),
                d2b_contracts_resource::v3::endpoint::EndpointConsumerPolicy::new(
                    Vec::new(),
                    Vec::new(),
                    vec![d2b_contracts_resource::v3::endpoint::EndpointOperation::Resolve],
                )
                .expect("valid Endpoint consumer policy"),
                d2b_contracts_resource::v3::endpoint::EndpointLifecyclePolicy::RecycleWithProducer,
            )
            .expect("valid Endpoint spec"),
        )
        .expect("serialize Endpoint spec"),
        _ => panic!("unsupported operator acceptance resource type"),
    };
    let spec_object = spec
        .as_object_mut()
        .expect("typed operator spec is an object");
    if matches!(
        resource_type,
        "Volume" | "Network" | "Device" | "Guest" | "Process"
    ) {
        spec_object.insert("providerRef".to_owned(), json!(provider_ref));
        spec_object.insert(
            "updatePolicy".to_owned(),
            json!({
                "disruptive": "manual",
                "nonDisruptive": "automatic"
            }),
        );
    }
    let status_resource = if resource_type == "Endpoint" {
        json!({
            "readiness": "Pending",
            "observedProducerGeneration": 0,
            "observedResourceGeneration": 1,
            "endpointGeneration": 0,
            "connectionAvailability": "unavailable",
            "leaseAvailability": "lease-required"
        })
    } else {
        json!({})
    };
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
                "ownerRef": owner_ref,
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
                "resource": status_resource,
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
    if let Some(owner_ref) = owner_ref {
        let owner_ref = ResourceRef::parse(owner_ref).expect("valid resource owner ref");
        let mut owner = wire::ResourceIdentity::new();
        owner.zone = "work".to_owned();
        owner.resource_type = owner_ref.resource_type().as_str().to_owned();
        owner.name = owner_ref.name().as_str().to_owned();
        mutation.owner = MessageField::some(owner);
    }
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

    let (operator_ref, operator_uid) = test_operator_subject_identity();
    let client = runtime
        .bind_operator_resource_client_for_test(operator_context(&zone, operator_ref, operator_uid))
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

    let refused = runtime.bind_operator_resource_client_for_test(operator_context(
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
async fn durable_process_and_endpoint_crud_survives_redb_reopen_and_drain() {
    let directory = tempfile::tempdir().expect("resource-plane directory");
    let zone = ZoneId::parse("work").unwrap();
    let marker_identity = format!("sha256:{}", "f".repeat(64));
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
                store_identity: marker_identity.clone(),
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

    let (operator_ref, operator_uid) = test_operator_subject_identity();
    let client = runtime
        .bind_operator_resource_client_for_test(operator_context(&zone, operator_ref, operator_uid))
        .expect("bind authenticated operator Resource API client");
    let session_owner = None;
    create_operator_resource(
        client.as_ref(),
        "Process",
        "display-host-proxy",
        "Provider/system-minijail",
        "create-display-host-proxy",
        session_owner,
    )
    .await;
    create_operator_resource(
        client.as_ref(),
        "Process",
        "display-guest-frontend",
        "Provider/system-systemd",
        "create-display-guest-frontend",
        session_owner,
    )
    .await;
    create_operator_resource(
        client.as_ref(),
        "Endpoint",
        "display-host-endpoint",
        "Provider/display-wayland",
        "create-display-host-endpoint",
        session_owner,
    )
    .await;
    create_operator_resource(
        client.as_ref(),
        "Endpoint",
        "display-guest-endpoint",
        "Provider/display-wayland",
        "create-display-guest-endpoint",
        session_owner,
    )
    .await;

    let processes = client.list(list_request("Process")).await;
    assert!(
        processes.error.is_none(),
        "list durable Process resources failed: {:?}",
        processes.error
    );
    let process_names = processes
        .resources
        .iter()
        .filter_map(|resource| resource.identity.as_ref())
        .map(|identity| identity.name.as_str())
        .collect::<Vec<_>>();
    assert!(process_names.contains(&"display-host-proxy"));
    assert!(process_names.contains(&"display-guest-frontend"));

    let endpoint = client
        .get(get_request(
            "Endpoint",
            "display-host-endpoint",
            "get-display-host-endpoint",
        ))
        .await;
    assert!(
        endpoint.error.is_none(),
        "get durable Endpoint failed: {:?}",
        endpoint.error
    );
    let endpoint = endpoint
        .resource
        .as_ref()
        .expect("durable Endpoint response");
    assert_eq!(
        endpoint
            .identity
            .as_ref()
            .expect("Endpoint identity")
            .resource_type,
        "Endpoint"
    );
    let endpoint_revision = endpoint
        .identity
        .as_ref()
        .expect("Endpoint identity")
        .revision
        .expect("Endpoint revision");
    ResourceEnvelope::from_json(&endpoint.canonical_json).expect("valid Endpoint envelope");
    let delete_endpoint = client
        .delete(delete_request(
            "Endpoint",
            "display-host-endpoint",
            endpoint_revision,
            "delete-display-host-endpoint",
        ))
        .await;
    assert!(
        delete_endpoint.error.is_none(),
        "delete Endpoint failed: {:?}",
        delete_endpoint.error
    );
    let removed_endpoint = client
        .get(get_request(
            "Endpoint",
            "display-host-endpoint",
            "get-deleted-display-host-endpoint",
        ))
        .await;
    assert!(
        removed_endpoint.resource.is_none() && removed_endpoint.error.is_some(),
        "finalizer-free Endpoint must be removed by the deletion request"
    );

    let process = client
        .get(get_request(
            "Process",
            "display-host-proxy",
            "get-display-host-proxy",
        ))
        .await;
    assert!(
        process.error.is_none(),
        "get durable Process failed: {:?}",
        process.error
    );
    let process = process.resource.as_ref().expect("durable Process response");
    let process_identity = process.identity.as_ref().expect("Process identity");
    let process_revision = process_identity.revision.expect("Process revision");
    ResourceEnvelope::from_json(&process.canonical_json).expect("valid Process envelope");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&process.canonical_json).expect("Process JSON")
            ["spec"]["executionRef"],
        "Host/host-system"
    );

    let finalizer = "display-wayland.d2bus.org/children";
    let add_finalizer = client
        .update_finalizers(update_finalizers_request(
            "Process",
            "display-host-proxy",
            process_revision,
            finalizer,
            true,
            "add-display-finalizer",
        ))
        .await;
    assert!(
        add_finalizer.error.is_none(),
        "add Process finalizer failed: {:?}",
        add_finalizer.error
    );
    let delete_revision = add_finalizer.revision;
    let delete = client
        .delete(delete_request(
            "Process",
            "display-host-proxy",
            delete_revision,
            "delete-display-host-proxy",
        ))
        .await;
    assert!(
        delete.error.is_none(),
        "delete Process failed: {:?}",
        delete.error
    );
    let retained = client
        .get(get_request(
            "Process",
            "display-host-proxy",
            "get-deleting-display-host-proxy",
        ))
        .await;
    assert!(
        retained.error.is_none(),
        "finalized Process must remain readable while draining: {:?}",
        retained.error
    );
    let retained = retained.resource.as_ref().expect("retained Process");
    let retained_json =
        serde_json::from_slice::<serde_json::Value>(&retained.canonical_json).expect("JSON");
    assert!(retained_json["metadata"]["deletionRequestedAt"].is_string());
    assert_eq!(
        retained_json["metadata"]["finalizers"]
            .as_array()
            .expect("Process finalizers"),
        &[json!(finalizer)]
    );
    let remove_finalizer = client
        .update_finalizers(update_finalizers_request(
            "Process",
            "display-host-proxy",
            retained
                .identity
                .as_ref()
                .expect("retained Process identity")
                .revision
                .expect("retained Process revision"),
            finalizer,
            false,
            "drain-display-finalizer",
        ))
        .await;
    assert!(
        remove_finalizer.error.is_none(),
        "remove Process finalizer failed: {:?}",
        remove_finalizer.error
    );
    let removed = client
        .get(get_request(
            "Process",
            "display-host-proxy",
            "get-drained-display-host-proxy",
        ))
        .await;
    assert!(
        removed.resource.is_none(),
        "drained Process must be removed from Redb"
    );
    assert!(
        removed.error.is_some(),
        "drained Process lookup must report not found"
    );
    drop(client);
    runtime.shutdown().await.expect("shutdown resource runtime");

    let database = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("reopen redb file after drain");
    let mut reopened = ZoneResourceRuntime::open(
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
    .expect("reopen production Zone runtime");
    reopened.set_provider_path_ready(true);
    let (operator_ref, operator_uid) = test_operator_subject_identity();
    let client = reopened
        .bind_operator_resource_client_for_test(operator_context(&zone, operator_ref, operator_uid))
        .expect("rebind authenticated operator Resource API client");
    let remaining = client.list(list_request("Process")).await;
    assert!(
        remaining.error.is_none(),
        "list Process after reopen failed: {:?}",
        remaining.error
    );
    assert!(
        remaining
            .resources
            .iter()
            .filter_map(|resource| resource.identity.as_ref())
            .all(|identity| identity.name != "display-host-proxy")
    );
    let endpoint_after_reopen = client
        .get(get_request(
            "Endpoint",
            "display-guest-endpoint",
            "get-display-guest-endpoint-after-reopen",
        ))
        .await;
    assert!(
        endpoint_after_reopen.error.is_none(),
        "Endpoint did not survive Redb reopen: {:?}",
        endpoint_after_reopen.error
    );
    drop(client);
    reopened
        .shutdown()
        .await
        .expect("shutdown reopened resource runtime");
}

#[tokio::test]
async fn wayland_session_owner_deletion_is_child_and_endpoint_first() {
    let directory = tempfile::tempdir().expect("resource-plane directory");
    let zone = ZoneId::parse("work").unwrap();
    let marker_identity = format!("sha256:{}", "d".repeat(64));
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
    let (store, adapter) = open_seeded_resource_api(
        &zone,
        &database_path,
        &marker_path,
        store_identity,
        snapshot,
    )
    .await;
    let client = adapter.client();
    let session_ref = "display-wayland.d2bus.org.WaylandSession/display-wayland";
    create_operator_resource(
        &client,
        "display-wayland.d2bus.org.WaylandSession",
        "display-wayland",
        "Provider/display-wayland",
        "create-wayland-session-owner",
        None,
    )
    .await;
    let session = client
        .get(get_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            "get-wayland-session-owner",
        ))
        .await;
    assert!(
        session.error.is_none(),
        "get WaylandSession failed: {:?}",
        session.error
    );
    let session_revision = session
        .resource
        .as_ref()
        .expect("WaylandSession response")
        .identity
        .as_ref()
        .expect("WaylandSession identity")
        .revision
        .expect("WaylandSession revision");
    let add_finalizer = client
        .update_finalizers(update_finalizers_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            session_revision,
            "display-wayland.d2bus.org/proxy-stopped",
            true,
            "add-wayland-session-owner-finalizer",
        ))
        .await;
    assert!(
        add_finalizer.error.is_none(),
        "add WaylandSession finalizer failed: {:?}",
        add_finalizer.error
    );

    create_operator_resource(
        &client,
        "Process",
        "display-host-proxy",
        "Provider/system-minijail",
        "create-wayland-owner-host-process",
        Some(session_ref),
    )
    .await;
    create_operator_resource(
        &client,
        "Process",
        "display-guest-frontend",
        "Provider/system-systemd",
        "create-wayland-owner-guest-process",
        Some(session_ref),
    )
    .await;
    create_operator_resource(
        &client,
        "Endpoint",
        "display-host-endpoint",
        "Provider/display-wayland",
        "create-wayland-owner-host-endpoint",
        Some(session_ref),
    )
    .await;
    create_operator_resource(
        &client,
        "Endpoint",
        "display-guest-endpoint",
        "Provider/display-wayland",
        "create-wayland-owner-guest-endpoint",
        Some(session_ref),
    )
    .await;

    let session = client
        .get(get_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            "get-wayland-session-before-delete",
        ))
        .await;
    let delete_requested = client
        .delete(delete_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            session
                .resource
                .as_ref()
                .expect("WaylandSession before delete")
                .identity
                .as_ref()
                .expect("WaylandSession identity before delete")
                .revision
                .expect("WaylandSession revision before delete"),
            "request-wayland-session-delete",
        ))
        .await;
    assert!(
        delete_requested.error.is_none(),
        "WaylandSession deletion request failed: {:?}",
        delete_requested.error
    );
    let deleting_session = client
        .get(get_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            "get-deleting-wayland-session",
        ))
        .await;
    let deleting_session = deleting_session
        .resource
        .as_ref()
        .expect("deleting WaylandSession");
    let deleting_json =
        serde_json::from_slice::<serde_json::Value>(&deleting_session.canonical_json)
            .expect("deleting WaylandSession JSON");
    assert!(deleting_json["metadata"]["deletionRequestedAt"].is_string());
    let remove_finalizer = client
        .update_finalizers(update_finalizers_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            deleting_session
                .identity
                .as_ref()
                .expect("deleting WaylandSession identity")
                .revision
                .expect("deleting WaylandSession revision"),
            "display-wayland.d2bus.org/proxy-stopped",
            false,
            "remove-wayland-session-owner-finalizer",
        ))
        .await;
    assert!(
        remove_finalizer.error.is_none(),
        "remove WaylandSession finalizer failed: {:?}",
        remove_finalizer.error
    );
    let blocked = client
        .delete(delete_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            remove_finalizer.revision,
            "delete-wayland-session-with-children",
        ))
        .await;
    assert_eq!(
        blocked.error.as_ref().map(|error| error.reason.as_str()),
        Some("owned-children-remain"),
        "owner deletion must remain blocked until child resources drain"
    );

    for (resource_type, name, operation_id) in [
        (
            "Endpoint",
            "display-host-endpoint",
            "delete-wayland-host-endpoint",
        ),
        (
            "Endpoint",
            "display-guest-endpoint",
            "delete-wayland-guest-endpoint",
        ),
    ] {
        let endpoint = client
            .get(get_request(
                resource_type,
                name,
                &format!("get-{operation_id}"),
            ))
            .await;
        let endpoint = endpoint.resource.as_ref().expect("Endpoint before delete");
        let revision = endpoint
            .identity
            .as_ref()
            .expect("Endpoint identity before delete")
            .revision
            .expect("Endpoint revision before delete");
        let requested = client
            .delete(delete_request(resource_type, name, revision, operation_id))
            .await;
        assert!(
            requested.error.is_none(),
            "Endpoint deletion request failed: {:?}",
            requested.error
        );
        let removed = client
            .get(get_request(
                resource_type,
                name,
                &format!("get-deleted-{operation_id}"),
            ))
            .await;
        assert!(
            removed.resource.is_none() && removed.error.is_some(),
            "finalizer-free Endpoint must be removed by the deletion request"
        );
    }

    for (name, operation_id) in [
        ("display-host-proxy", "delete-wayland-host-process"),
        ("display-guest-frontend", "delete-wayland-guest-process"),
    ] {
        let process = client
            .get(get_request("Process", name, &format!("get-{operation_id}")))
            .await;
        let process = process.resource.as_ref().expect("Process before delete");
        let requested = client
            .delete(delete_request(
                "Process",
                name,
                process
                    .identity
                    .as_ref()
                    .expect("Process identity before delete")
                    .revision
                    .expect("Process revision before delete"),
                operation_id,
            ))
            .await;
        assert!(
            requested.error.is_none(),
            "Process deletion request failed: {:?}",
            requested.error
        );
        let removed = client
            .get(get_request(
                "Process",
                name,
                &format!("get-deleted-{operation_id}"),
            ))
            .await;
        assert!(
            removed.resource.is_none() && removed.error.is_some(),
            "finalizer-free Process must be removed by the deletion request"
        );
    }

    let session = client
        .get(get_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            "get-drained-wayland-session",
        ))
        .await;
    let drained = client
        .delete(delete_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            session
                .resource
                .as_ref()
                .expect("draining WaylandSession")
                .identity
                .as_ref()
                .expect("draining WaylandSession identity")
                .revision
                .expect("draining WaylandSession revision"),
            "drain-wayland-session",
        ))
        .await;
    assert!(
        drained.error.is_none(),
        "WaylandSession physical deletion failed: {:?}",
        drained.error
    );
    let removed = client
        .get(get_request(
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland",
            "get-removed-wayland-session",
        ))
        .await;
    assert!(
        removed.resource.is_none() && removed.error.is_some(),
        "WaylandSession must be removed after all owned children drain"
    );
    drop(client);
    drop(adapter);
    let store = std::sync::Arc::try_unwrap(store).expect("release seeded resource store");
    store
        .shutdown()
        .await
        .expect("shutdown seeded resource store");
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

    let (operator_ref, operator_uid) = test_operator_subject_identity();
    let controller_generation = runtime
        .committed_policy_snapshot()
        .controller_generation
        .expect("runtime controller generation");
    let client = runtime
        .bind_operator_resource_client_for_test(
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
            r#"{{"version":1,"assignment":{{"resourceUid":"{}","resourceRevision":{},"providerRef":"Provider/system-core","providerGeneration":2,"controllerGeneration":3,"controllerRole":"Process/process-controller","target":{{"kind":"execution","targetKind":"host","reference":"Host/host-system"}},"sessionOwner":"Process/process-controller","sessionGeneration":1,"epoch":1}},"mutations":[{{"target":"Host/host-system","verb":"UpdateStatus"}},{{"target":"Host/host-system","verb":"UpdateFinalizers"}}]}}"#,
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

    let (operator_ref, operator_uid) = test_operator_subject_identity();
    let client = runtime
        .bind_operator_resource_client_for_test(operator_context(&zone, operator_ref, operator_uid))
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
            None,
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

    let refused = runtime.bind_operator_resource_client_for_test(operator_context(
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
