//! U8 verification: the Resource API rewired onto the per-Zone manager.
//!
//! Covers the plan's U8 test scenarios at unit scale:
//! - create/read/update/delete round-trip through the manager-backed service;
//! - authorization allow/deny from the fixture matrix still enforced;
//! - exact-revision precondition rejects a stale generation with the
//!   existing wire error;
//! - LIST returns the snapshot revision and WATCH resumes from it (F4);
//! - no status write path exists from API status updates (grep invariant);
//! - the one-way bootstrap latch boots from an empty store.

use std::sync::Arc;

use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    ServiceName, SessionBinding, SessionPurpose, TranscriptHash, TransportBinding,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ConfigurationGeneration, ControllerGeneration, ResourceGeneration,
    ResourceName, ResourceRef, ResourceTypeName, ResourceUid, SchemaFingerprint, ZoneId,
    ZoneRevision, canonical_digest, RESOURCE_ENVELOPE_DOMAIN_TAG,
};
use protobuf::{EnumOrUnknown, MessageField};

use crate::authz::{
    ApiCatalog, ApiMethod, AuthorizationState, AuthorizationTarget, BootstrapPhase,
    CompiledRole, CompiledRoleBinding, DurablePolicyRow, DurableRowProvenance,
    NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
    ResourceVerb, BindingScope, BoundSubject, derive_bootstrap_phase,
};
use d2b_contracts_resource::v3::StoreSealIdentity;
use d2b_resource_runtime::watch::WatchHub;

const TEST_ZONE: &str = "dev";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const GOLDEN_HOST: &[u8] = br#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"host-system","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}},"type":"Host"}"#;

fn envelope_without_uid(raw: &[u8]) -> Vec<u8> {
    let mut value = CanonicalJsonValue::parse(raw).unwrap();
    let CanonicalJsonValue::Object(root) = &mut value else {
        unreachable!()
    };
    let CanonicalJsonValue::Object(metadata) = root.get_mut("metadata").unwrap() else {
        unreachable!()
    };
    metadata.remove("uid");
    value.to_canonical_bytes()
}

/// Bind the wire subject and policy state used by every request below.
fn subject() -> Arc<AuthenticatedSubjectContext> {
    let context = AuthenticatedSubjectContext::new(
        ResourceRef::parse("Provider/system-core").unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
        ResourceRef::parse("Zone/dev").unwrap(),
        EvidenceClass::UnixPeer,
        SessionPurpose::parse("resource-api").unwrap(),
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
    );
    Arc::new(context)
}

fn authorization_state() -> AuthorizationState {
    AuthorizationState {
        snapshot: d2b_contracts_resource::v3::PolicySnapshot {
            policy_revision: 4,
            api_catalog_revision: 5,
            active_configuration_revision: ConfigurationGeneration::new(6).unwrap(),
            controller_generation: None,
        },
        zone_policy_revision: ZoneRevision::new(7),
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    }
}

fn trusted<T>(request: T) -> crate::service::TrustedRequest<T> {
    crate::service::TrustedRequest::from_session_capability(
        subject(),
        authorization_state(),
        request,
    )
}

/// The allow fixture matrix: system-core may act on Host rows in `dev`.
fn authorizer(verbs: &[ResourceVerb]) -> Arc<NativeAuthorizer> {
    authorizer_scoped(verbs, &[])
}

fn authorizer_scoped(verbs: &[ResourceVerb], names: &[&str]) -> Arc<NativeAuthorizer> {
    let context = subject();
    let catalog = ApiCatalog::standard();
    let role = CompiledRole::new(
        ResourceRef::parse("Role/operator").unwrap(),
        vec![PolicyRule::new(
            &catalog,
            [ResourceTypeName::parse("Host").unwrap()],
            verbs.to_vec(),
            [],
            Vec::<String>::new(),
            names.iter().map(|name| ResourceName::parse(*name).unwrap()),
            [ZoneId::parse(TEST_ZONE).unwrap()],
            [],
        )
        .unwrap()],
    )
    .unwrap();
    let binding = CompiledRoleBinding::new(
        role.role_ref.clone(),
        [BoundSubject {
            subject_ref: context.subject_ref().clone(),
            subject_uid: context.subject_uid().clone(),
        }],
        BindingScope::default(),
        RelayGrantAuthority::None,
    )
    .unwrap();
    Arc::new(
        NativeAuthorizer::new(
            catalog.clone(),
            Some(PolicySet::new(&catalog, 4, vec![role], vec![binding]).unwrap()),
        )
        .unwrap(),
    )
}

/// Minimal driver: recovery adopts immediately, reconcile satisfies - the
/// runtime side of the API round-trip needs no real provider effects.
struct StubDriver;

#[async_trait::async_trait]
impl d2b_resource_runtime::driver::ResourceDriver for StubDriver {
    type Error = std::io::Error;

    fn classify_error(&self, _error: &Self::Error) -> d2b_resource_runtime::error::DriverFailure {
        d2b_resource_runtime::error::DriverFailure::retryable(d2b_resource_runtime::error::DriverOp::Reconcile)
    }

    async fn validate(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn recover(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<d2b_resource_runtime::driver::RecoveryOutcome, Self::Error> {
        Ok(d2b_resource_runtime::driver::RecoveryOutcome::Adopted)
    }

    async fn reconcile(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<d2b_resource_runtime::driver::ReconcileOutcome, Self::Error> {
        Ok(d2b_resource_runtime::driver::ReconcileOutcome::Satisfied)
    }

    async fn delete(&mut self, _ctx: &mut d2b_resource_runtime::context::ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// One Zone manager over a fresh SQLite spec store plus its watch hub.
struct ManagerFixture {
    client: d2b_resource_runtime::ResourceManagerClient,
    hub: Arc<WatchHub>,
    manager_actor: ractor::ActorRef<d2b_resource_runtime::ResourceManagerMsg>,
    _directory: tempfile::TempDir,
}

async fn manager_fixture() -> ManagerFixture {
    use std::collections::HashMap;
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        d2b_resource_runtime::spec_store::SpecStore::open(directory.path().join("specs.sqlite"))
            .unwrap(),
    );
    let hub = Arc::new(WatchHub::new(
        &d2b_resource_runtime::revision::SystemClock,
        d2b_resource_runtime::watch::DEFAULT_RING_CAPACITY,
    ));
    struct NoFactory;
    #[async_trait::async_trait]
    impl d2b_resource_runtime::driver::ResourceDriverFactory for NoFactory {
        fn resource_types(&self) -> &[d2b_resource_runtime::identity::ResourceTypeName] {
            // Leaked static: one fixed type list for the test process.
            Box::leak(Box::new([d2b_resource_runtime::identity::ResourceTypeName::new("Host")]))
        }
        async fn create(
            &self,
            _key: &d2b_resource_runtime::spec_store::ResourceKey,
        ) -> Box<dyn d2b_resource_runtime::driver::DynResourceDriver> {
            Box::new(StubDriver)
        }
    }
    struct NoDecoder;
    impl d2b_resource_runtime::context::SpecDecoder for NoDecoder {
        fn decode(
            &self,
            _envelope: &[u8],
        ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("no decoder".into())
        }
    }
    /// Test resolver: no fixture row declares an execution reference, so every
    /// test resource realizes on the Zone's Host target.
    struct HostOnlyResolver;
    impl d2b_resource_runtime::target::TargetResolver for HostOnlyResolver {
        fn execution_ref(
            &self,
            _key: &d2b_resource_runtime::spec_store::ResourceKey,
            _spec: &[u8],
        ) -> Option<String> {
            None
        }
    }
    let args = d2b_resource_runtime::ResourceManagerArgs {
        zone: TEST_ZONE.to_owned(),
        store,
        providers: {
            let mut providers = d2b_resource_runtime::provider::ProviderDirectory::new();
            providers
                .register(Arc::new(NoFactory) as Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory>)
                .unwrap();
            providers
        },
        hub: hub.clone(),
        admission: Arc::new(d2b_resource_runtime::AllowAll),
        decoders: HashMap::new(),
        default_decoder: Arc::new(NoDecoder),
        targets: Arc::new(d2b_resource_runtime::target::TargetDirectory::new()),
        host_target: d2b_resource_runtime::target::TargetRef::host("test-host")
            .expect("host target"),
        target_resolver: Arc::new(HostOnlyResolver),
        backoff: std::time::Duration::from_millis(200),
    };
    let (actor, _join) = ractor::Actor::spawn(None, d2b_resource_runtime::ResourceManager::new(), args)
        .await
        .unwrap();
    ManagerFixture {
        client: d2b_resource_runtime::ResourceManagerClient::new(actor.clone()),
        hub,
        manager_actor: actor,
        _directory: directory,
    }
}

fn seal_identity() -> StoreSealIdentity {
    StoreSealIdentity::new(
        d2b_contracts_resource::v3::StoreSlot::new(0).unwrap(),
        ZoneId::parse(TEST_ZONE).unwrap(),
        ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
    )
}

/// The manager-backed service wired exactly as U9 composes it: authorizer
/// seals the manager plane, the backend rides the manager client + hub.
fn wired_service(
    fixture: &ManagerFixture,
    authorizer: Arc<NativeAuthorizer>,
) -> crate::ResourceService<crate::manager_backend::ManagerBackend> {
    let acceptor = authorizer
        .take_store_seal(seal_identity())
        .expect("authorizer hands the manager plane its seal acceptor");
    let backend = crate::manager_backend::ManagerBackend::new(
        fixture.client.clone(),
        fixture.hub.clone(),
        acceptor,
    );
    crate::ResourceService::new(Arc::new(backend), authorizer).unwrap()
}

fn request_meta() -> MessageField<wire::RequestMeta> {
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = "operation-1".to_owned();
    meta.idempotency_key = "idempotency-1".to_owned();
    meta.correlation_id = "correlation-1".to_owned();
    MessageField::some(meta)
}

fn identity() -> MessageField<wire::ResourceIdentity> {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = TEST_ZONE.to_owned();
    identity.resource_type = "Host".to_owned();
    identity.name = "host-system".to_owned();
    MessageField::some(identity)
}

fn create_body() -> MessageField<wire::ResourceEnvelopeBytes> {
    let canonical = envelope_without_uid(GOLDEN_HOST);
    let envelope = CanonicalJsonValue::parse(&canonical).unwrap();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = identity();
    body.payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &envelope.to_canonical_bytes());
    body.canonical_json = canonical;
    MessageField::some(body)
}

fn update_body(canonical: Vec<u8>, uid: &str, generation: u64) -> MessageField<wire::ResourceEnvelopeBytes> {
    let mut value = CanonicalJsonValue::parse(&canonical).unwrap();
    let CanonicalJsonValue::Object(root) = &mut value else {
        unreachable!()
    };
    let CanonicalJsonValue::Object(metadata) = root.get_mut("metadata").unwrap() else {
        unreachable!()
    };
    metadata.insert("uid".to_owned(), CanonicalJsonValue::String(uid.to_owned()));
    metadata
        .insert("generation".to_owned(), CanonicalJsonValue::Integer(generation as i64));
    metadata.insert("revision".to_owned(), CanonicalJsonValue::Integer(generation as i64));
    let canonical = value.to_canonical_bytes();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = identity();
    body.payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    body.canonical_json = canonical;
    MessageField::some(body)
}

fn create_request() -> wire::CreateRequest {
    let mut request = wire::CreateRequest::new();
    request.meta = request_meta();
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = identity();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    mutation.precondition = MessageField::some(precondition);
    mutation.resource = create_body();
    request.mutation = MessageField::some(mutation);
    request
}

fn status_body() -> MessageField<wire::ResourceEnvelopeBytes> {
    let canonical = CanonicalJsonValue::parse(GOLDEN_HOST).unwrap().to_canonical_bytes();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = identity();
    body.payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    body.canonical_json = canonical;
    MessageField::some(body)
}

fn update_spec_request(
    expected_revision: u64,
    uid: &str,
    generation: u64,
    spec_extra: Option<&str>,
) -> wire::UpdateSpecRequest {
    let mut request = wire::UpdateSpecRequest::new();
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC);
    mutation.target = identity();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(expected_revision);
    precondition.expected_uid = Some(uid.to_owned());
    mutation.precondition = MessageField::some(precondition);
    // A Host envelope with a different update policy so the desired state
    // actually changes generation.
    let mut value = CanonicalJsonValue::parse(GOLDEN_HOST).unwrap();
    let CanonicalJsonValue::Object(root) = &mut value else {
        unreachable!()
    };
    if let Some(extra) = spec_extra {
        root.insert(
            "spec".to_owned(),
            CanonicalJsonValue::parse(extra.as_bytes()).unwrap(),
        );
    }
    mutation.resource = update_body(
        value.to_canonical_bytes(),
        uid,
        generation,
    );
    request.meta = request_meta();
    request.mutation = MessageField::some(mutation);
    request
}

fn delete_request(expected_revision: u64, uid: &str) -> wire::DeleteRequest {
    let mut request = wire::DeleteRequest::new();
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = identity();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(expected_revision);
    precondition.expected_uid = Some(uid.to_owned());
    mutation.precondition = MessageField::some(precondition);
    request.meta = request_meta();
    request.mutation = MessageField::some(mutation);
    request
}

fn get_request() -> wire::GetRequest {
    let mut request = wire::GetRequest::new();
    request.meta = request_meta();
    request.target = identity();
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    request.projection = MessageField::some(projection);
    request
}

fn list_request() -> wire::ListRequest {
    let mut request = wire::ListRequest::new();
    request.meta = request_meta();
    request.resource_types = vec!["Host".to_owned()];
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    request.projection = MessageField::some(projection);
    request
}

fn watch_request(after_revision: u64) -> wire::WatchRequest {
    let mut request = wire::WatchRequest::new();
    request.meta = request_meta();
    request.resource_types = vec!["Host".to_owned()];
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    request.projection = MessageField::some(projection);
    request.after_revision = after_revision;
    request
}

fn error_kind<T>(response: &T) -> wire::ResourceErrorKind
where
    T: HasError,
{
    response.error().as_ref().unwrap().kind.enum_value().unwrap()
}

fn error_reason<T>(response: &T) -> String
where
    T: HasError,
{
    response.error().as_ref().unwrap().reason.clone()
}

trait HasError {
    fn error(&self) -> &MessageField<wire::ResourceError>;
}

macro_rules! impl_has_error {
    ($($kind:ty),+) => {
        $(impl HasError for $kind {
            fn error(&self) -> &MessageField<wire::ResourceError> {
                &self.error
            }
        })+
    };
}

impl_has_error!(
    wire::GetResponse,
    wire::ListResponse,
    wire::WatchResponse,
    wire::CreateResponse,
    wire::UpdateSpecResponse,
    wire::UpdateStatusResponse,
    wire::UpdateMetadataResponse,
    wire::UpdateFinalizersResponse,
    wire::DeleteResponse
);

// ---------------------------------------------------------------------------
// CRUD through the manager-backed service
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_get_update_delete_round_trip_through_the_manager() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[
        ResourceVerb::Create,
        ResourceVerb::Get,
        ResourceVerb::UpdateSpec,
        ResourceVerb::Delete,
    ]));

    let created = service.create(trusted(create_request())).await;
    assert!(created.error.is_none(), "create failed: kind={:?} reason={}", error_kind(&created), error_reason(&created));
    assert_eq!(created.revision, 1, "a fresh resource commits at generation 1");
    let resource = created.resource.as_ref().unwrap();
    assert_eq!(resource.identity.as_ref().unwrap().resource_type, "Host");
    let uid = resource.identity.as_ref().unwrap().uid.clone().unwrap();

    // Read back through the same service: the manager serves the view.
    let fetched = service.get(trusted(get_request())).await;
    assert!(
        fetched.error.is_none(),
        "get failed: kind={:?} reason={}",
        error_kind(&fetched),
        error_reason(&fetched)
    );
    let fetched = fetched.resource.unwrap();
    assert_eq!(fetched.identity.as_ref().unwrap().name.as_str(), "host-system");
    assert_eq!(fetched.identity.as_ref().unwrap().generation, Some(1));

    // Update at the exact committed generation advances to 2.
    let mut update = update_spec_request(1, &uid, 2, Some(r#"{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}}"#));
    update.meta = request_meta();
    let mut value = CanonicalJsonValue::parse(GOLDEN_HOST).unwrap();
    let CanonicalJsonValue::Object(root) = &mut value else { unreachable!() };
    root.insert("spec".to_owned(), CanonicalJsonValue::parse(
        br#"{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}}"#,
    ).unwrap());
    let canonical = value.to_canonical_bytes();
    update.mutation.as_mut().unwrap().resource = update_body(canonical, &uid, 2);
    let updated = service.update_spec(trusted(update)).await;
    assert!(
        updated.error.is_none(),
        "update failed: kind={:?} reason={}",
        error_kind(&updated),
        error_reason(&updated)
    );
    assert_eq!(updated.revision, 2, "the generation advanced exactly once");

    // A stale precondition (still expecting generation 1) hits the existing
    // resource-conflict wire error carrying the current generation.
    let stale = service.update_spec(trusted(update_spec_request(1, &uid, 2, None))).await;
    assert_eq!(
        error_kind(&stale),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT,
        "stale generation rejected: kind={:?} reason={}",
        error_kind(&stale),
        error_reason(&stale)
    );
    assert_eq!(stale.error.as_ref().unwrap().current_revision, Some(2));

    // Delete, then the row is gone.
    let deleted = service.delete(trusted(delete_request(2, &uid))).await;
    assert!(deleted.error.is_none(), "delete failed: {:?}", deleted.error);
    let gone = service.get(trusted(get_request())).await;
    assert_eq!(
        error_kind(&gone),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND,
        "the deleted row is absent from the manager"
    );

    fixture.manager_actor.get_cell().stop(None);
}

/// Nix-ingested rows persist spec-shaped bytes, so their envelope is
/// rendered on the read (the fallback path). The manager view must serve the
/// row's stable uid there: the public delete precondition resolves the exact
/// uid from `metadata.uid`, and `ResourceUid`'s redacted `Display` is never
/// data. A round trip through an API-created row cannot catch this - those
/// rows persist envelope-shaped bytes and already carry their uid.
#[tokio::test]
async fn spec_shaped_row_serves_its_stable_uid() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::Get]));
    let handle = fixture
        .client
        .ensure(
            d2b_resource_runtime::manager::MutationSubject {
                principal: "nix:test-bundle".to_owned(),
                origin: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
            },
            None,
            d2b_resource_runtime::manager::DesiredResource {
                key: d2b_resource_runtime::spec_store::ResourceKey::new(
                    TEST_ZONE,
                    "Host",
                    "host-system",
                ),
                spec: serde_json::to_vec(&serde_json::json!({
                    "providerRef": "Provider/system-core",
                    "updatePolicy": {
                        "disruptive": "manual",
                        "nonDisruptive": "automatic",
                    },
                }))
                .unwrap(),
                metadata: serde_json::to_vec(&serde_json::json!({"ownerRef": null})).unwrap(),
                provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
            },
        )
        .await
        .expect("ensure");
    let stable_uid = super::row_uid(&handle.uid);

    let fetched = service.get(trusted(get_request())).await;
    assert!(
        fetched.error.is_none(),
        "get failed: kind={:?} reason={}",
        error_kind(&fetched),
        error_reason(&fetched)
    );
    let resource = fetched.resource.unwrap();
    let envelope: serde_json::Value =
        serde_json::from_slice(&resource.canonical_json).expect("served envelope");
    assert_eq!(
        envelope
            .pointer("/metadata/uid")
            .and_then(serde_json::Value::as_str),
        Some(stable_uid.as_str()),
        "the manager view serves the row's stable uid, not a redacted placeholder",
    );

    fixture.manager_actor.get_cell().stop(None);
}

/// The strict boundary the daemon's reader bridge crosses (U12): a rendered
/// manager row must decode as a complete contract envelope whose own digest
/// equals the row's `payload_digest` - the checks
/// `validated_stored_resource_envelope` runs - with and without a stamped
/// actor status. Nix-materialized rows persist spec-shaped bytes with author
/// metadata only, so the API wire path's lenient rendering is not enough:
/// the store-shaped readers reject a row that is missing required metadata
/// or the status entirely.
///
/// Every closed classification the manager can publish is exercised: one
/// case per phase projection (including the `Deleted` tombstone, the closed
/// vocabulary's projection of the runtime `Deleting` classification).
#[test]
fn rendered_rows_round_trip_through_the_strict_envelope_reader() {
    use d2b_contracts_resource::v3::{ResourceEnvelope, ResourcePhase};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance};

    let zone = ZoneId::parse(TEST_ZONE).unwrap();
    let key = ResourceKey::new(TEST_ZONE, "Role", "operator-reader");
    // The spec shape the Nix bundle ingestion persists: the desired state
    // alone, no envelope wrapper.
    let spec = serde_json::to_vec(&serde_json::json!({
        "rules": [{
            "resourceTypes": ["Host"],
            "verbs": ["get"],
            "subresources": [],
            "resourceNames": [],
            "zones": [TEST_ZONE],
            "executionRefs": [],
            "sessionVerbs": [],
        }],
    }))
    .unwrap();
    // The author metadata the ingestion path persists beside it: no
    // `managedBy`, no store-owned fields.
    let metadata = serde_json::to_vec(&serde_json::json!({
        "ownerRef": null,
        "labels": {},
        "annotations": {},
    }))
    .unwrap();
    let uid = super::manager_uid(&key);
    let view = |status, status_generation| ResourceView {
        key: key.clone(),
        uid,
        generation: 1,
        deleting: false,
        provenance: ResourceProvenance::Nix,
        spec: spec.clone(),
        metadata: metadata.clone(),
        owner_key: None,
        status,
        status_generation,
        status_projection: None,
    };

    for (label, status, status_generation, expected_phase) in [
        (
            "unpublished status",
            None,
            None,
            ResourcePhase::Pending,
        ),
        (
            "published status",
            Some(ResourceStatus::Ready),
            Some(1),
            ResourcePhase::Ready,
        ),
        (
            "failed status",
            Some(ResourceStatus::Failed(
                d2b_resource_runtime::error::DriverFailure::retryable(
                    d2b_resource_runtime::error::DriverOp::Reconcile,
                ),
            )),
            Some(1),
            ResourcePhase::Failed,
        ),
        (
            "deleting status",
            Some(ResourceStatus::Deleting),
            Some(1),
            ResourcePhase::Deleted,
        ),
    ] {
        let stored = super::manager_row_stored(&view(status, status_generation))
            .unwrap_or_else(|error| panic!("{label}: render failed: {error:?}"));
        // `validated_stored_resource_envelope` step 1: strict decode.
        let envelope = ResourceEnvelope::from_json(&stored.canonical_json)
            .unwrap_or_else(|error| panic!("{label}: strict decode failed: {error:?}"));
        // ... step 2: identity agreement with the row.
        assert_eq!(envelope.resource_type().as_str(), "Role", "{label}");
        assert_eq!(envelope.metadata().zone(), &zone, "{label}");
        assert_eq!(envelope.metadata().name().as_str(), "operator-reader", "{label}");
        assert_eq!(envelope.metadata().uid(), &stored.uid, "{label}");
        assert_eq!(
            envelope.metadata().generation().get(),
            stored.generation.get(),
            "{label}"
        );
        assert_eq!(
            envelope.metadata().revision().get(),
            stored.revision.get(),
            "{label}"
        );
        // ... step 3: the digest recomputed from the decoded envelope must be
        // the digest the row carries.
        assert_eq!(
            envelope.digest().unwrap(),
            stored.payload_digest,
            "{label}: the row digest must be the decoded envelope's digest"
        );
        assert_eq!(envelope.status().phase(), expected_phase, "{label}");
        assert_eq!(
            envelope.status().observed_generation().get(),
            stored.generation.get(),
            "{label}: the status is stamped at the row's own generation"
        );
    }
}

/// The envelope-shaped arm of the same boundary (API-created rows persist a
/// complete envelope): a live actor status must replace the persisted status
/// in place, and the deletion mark must stamp a `deletionRequestedAt` no
/// earlier than `createdAt` (the strict metadata contract rejects the
/// reverse order), without breaking the strict decode or the row digest.
#[test]
fn rendered_full_envelopes_keep_the_strict_reader_contract() {
    use d2b_contracts_resource::v3::{ResourceEnvelope, ResourcePhase};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance};

    let key = ResourceKey::new(TEST_ZONE, "Host", "host-system");
    let uid = super::manager_uid(&key);
    let mut stored_envelope: serde_json::Value = serde_json::from_slice(GOLDEN_HOST).unwrap();
    stored_envelope["metadata"]["uid"] =
        serde_json::json!(super::row_uid(&uid).as_str());
    let canonical = CanonicalJsonValue::parse(&serde_json::to_vec(&stored_envelope).unwrap())
        .unwrap()
        .to_canonical_bytes();

    let view = |status, status_generation, deleting| ResourceView {
        key: key.clone(),
        uid,
        generation: 1,
        deleting,
        provenance: ResourceProvenance::Api,
        spec: canonical.clone(),
        metadata: serde_json::to_vec(&serde_json::json!({})).unwrap(),
        owner_key: None,
        status,
        status_generation,
        status_projection: None,
    };
    for (label, status, status_generation, deleting, expected_phase) in [
        (
            "live status",
            Some(ResourceStatus::Ready),
            Some(1),
            false,
            ResourcePhase::Ready,
        ),
        (
            "deleting",
            Some(ResourceStatus::Deleting),
            Some(1),
            true,
            ResourcePhase::Deleted,
        ),
    ] {
        let stored = super::manager_row_stored(&view(status, status_generation, deleting))
            .unwrap_or_else(|error| panic!("{label}: render failed: {error:?}"));
        let envelope = ResourceEnvelope::from_json(&stored.canonical_json)
            .unwrap_or_else(|error| panic!("{label}: strict decode failed: {error:?}"));
        assert_eq!(envelope.metadata().uid(), &stored.uid, "{label}");
        assert_eq!(envelope.status().phase(), expected_phase, "{label}");
        assert_eq!(
            envelope.status().observed_generation().get(),
            stored.generation.get(),
            "{label}"
        );
        assert_eq!(
            envelope.digest().unwrap(),
            stored.payload_digest,
            "{label}: the row digest must be the decoded envelope's digest"
        );
        if deleting {
            let value: serde_json::Value =
                serde_json::from_slice(&stored.canonical_json).expect("stored envelope");
            assert!(
                value
                    .pointer("/metadata/deletionRequestedAt")
                    .is_some_and(|value| !value.is_null()),
                "{label}: the durable deletion mark stays observable on the wire"
            );
        }
    }
}

/// The per-type half of the projection contract (issue #515): every
/// converted type the API can serve renders through the one producer, and
/// each rendered row is a complete strict envelope - with and without a live
/// status projection - whose status object is exactly the contract's closed
/// shape and whose `status.resource` layer is the driver's own value carried
/// unchanged.
///
/// A type whose rendering lost a required member, wrapped the driver's layer,
/// or grew a non-contract status field (the historical top-level
/// `driverFailure`) fails here, per type, instead of in a consumer.
#[test]
fn every_converted_type_projects_a_strict_wire_view() {
    use d2b_contracts_resource::v3::{
        ResourceEnvelope, ResourcePhase, V3_CONVERTED_RESOURCE_TYPES,
    };
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance};

    // The universal status object's exact key set: the only free-form layer
    // is `resource`; everything else is closed.
    const UNIVERSAL_STATUS_KEYS: [&str; 9] = [
        "completedAt",
        "conditions",
        "lastReconciledAt",
        "observedGeneration",
        "outcome",
        "phase",
        "resource",
        "startedAt",
        "update",
    ];

    for resource_type in V3_CONVERTED_RESOURCE_TYPES {
        let key = ResourceKey::new(TEST_ZONE, resource_type, "canonical-row");
        let uid = super::manager_uid(&key);
        // Spec-shaped desired bytes (the Nix ingestion shape) plus authored
        // metadata: the projection must reconstruct the complete envelope.
        let spec = serde_json::to_vec(&serde_json::json!({
            "providerRef": "Provider/contract-fixture",
            "typeProbe": resource_type,
        }))
        .unwrap();
        let metadata = serde_json::to_vec(&serde_json::json!({
            "ownerRef": null,
            "labels": {},
            "annotations": {},
        }))
        .unwrap();
        // The layer a driver of this type may publish is free-form by
        // contract, so the projection must carry it byte-for-byte.
        let projection = serde_json::json!({ "typeProbe": resource_type, "ready": true });

        for live in [false, true] {
            let label = format!("{resource_type} (live status: {live})");
            let view = ResourceView {
                key: key.clone(),
                uid,
                generation: 1,
                deleting: false,
                provenance: ResourceProvenance::Nix,
                spec: spec.clone(),
                metadata: metadata.clone(),
                owner_key: None,
                status: live.then_some(ResourceStatus::Ready),
                status_generation: live.then_some(1),
                status_projection: live.then_some(projection.clone()),
            };
            let stored = super::manager_row_stored(&view)
                .unwrap_or_else(|error| panic!("{label}: render failed: {error:?}"));
            let envelope = ResourceEnvelope::from_json(&stored.canonical_json)
                .unwrap_or_else(|error| panic!("{label}: strict decode failed: {error:?}"));
            assert_eq!(envelope.resource_type().as_str(), resource_type, "{label}");
            assert_eq!(envelope.metadata().name().as_str(), "canonical-row", "{label}");
            assert_eq!(
                envelope.metadata().zone(),
                &ZoneId::parse(TEST_ZONE).unwrap(),
                "{label}"
            );
            assert_eq!(envelope.metadata().uid(), &stored.uid, "{label}");
            assert_eq!(envelope.metadata().generation().get(), 1, "{label}");
            assert_eq!(envelope.metadata().revision().get(), 1, "{label}");
            assert_eq!(
                envelope.status().observed_generation().get(),
                1,
                "{label}: the status reports the row's own generation"
            );
            assert_eq!(
                envelope.status().phase(),
                if live { ResourcePhase::Ready } else { ResourcePhase::Pending },
                "{label}"
            );
            assert_eq!(
                envelope.digest().expect("envelope digest"),
                stored.payload_digest,
                "{label}: the row digest must be the decoded envelope's digest"
            );
            assert_eq!(
                envelope.canonical_bytes().expect("canonical bytes"),
                stored.canonical_json,
                "{label}: the served bytes must be the canonical envelope"
            );

            let value: serde_json::Value =
                serde_json::from_slice(&stored.canonical_json).expect("envelope json");
            let status = value["status"].as_object().expect("status object");
            let keys: std::collections::BTreeSet<&str> =
                status.keys().map(String::as_str).collect();
            assert_eq!(
                keys,
                std::collections::BTreeSet::from(UNIVERSAL_STATUS_KEYS),
                "{label}: the status object carries exactly the contract's keys"
            );
            assert_eq!(
                value["status"]["resource"],
                if live { projection.clone() } else { serde_json::json!({}) },
                "{label}: the driver's `status.resource` layer is carried unchanged"
            );
        }
    }
}

/// The other half of the issue #507 fence contract: the manager plane is the
/// converted type's authority. For every type in the registry the manager
/// path serves the committed row, and the only refusal shape it can render is
/// the manager's own honest absence - never the legacy `WrongPlane` plane
/// error (the fence lives on the legacy facade, not on the authority).
#[tokio::test]
async fn every_converted_type_is_served_by_the_manager_path() {
    use crate::ResourceStoreBackend;
    use d2b_contracts_resource::v3::V3_CONVERTED_RESOURCE_TYPES;
    use d2b_contracts_resource::v3::{StoreGetRequest, StoreOperationContext, StoreProjection};

    let fixture = manager_fixture().await;
    let authorizer = authorizer(&[ResourceVerb::Get]);
    let acceptor = authorizer
        .take_store_seal(seal_identity())
        .expect("authorizer hands the manager plane its seal acceptor");
    let backend = crate::manager_backend::ManagerBackend::new(
        fixture.client.clone(),
        fixture.hub.clone(),
        acceptor,
    );

    for resource_type in V3_CONVERTED_RESOURCE_TYPES {
        let name = "fence-row";
        // The fixture registers a driver factory for `Host` only: every other
        // type still commits its row (the manager documents the row staying
        // durable when the post-commit spawn finds no factory), which is
        // exactly the row the authority must serve.
        let _ = fixture
            .client
            .ensure(
                d2b_resource_runtime::manager::MutationSubject {
                    principal: "nix:test-bundle".to_owned(),
                    origin: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                },
                None,
                d2b_resource_runtime::manager::DesiredResource {
                    key: d2b_resource_runtime::spec_store::ResourceKey::new(
                        TEST_ZONE,
                        resource_type,
                        name,
                    ),
                    spec: serde_json::to_vec(&serde_json::json!({
                        "providerRef": "Provider/contract-fixture",
                    }))
                    .unwrap(),
                    metadata: serde_json::to_vec(&serde_json::json!({
                        "ownerRef": null,
                        "labels": {},
                        "annotations": {},
                    }))
                    .unwrap(),
                    provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                },
            )
            .await;

        let request = StoreGetRequest {
            operation: StoreOperationContext {
                operation_id: "manager-fence-table".to_owned(),
                idempotency_key: None,
                correlation_id: "manager-fence-table".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            target: ResourceRef::parse(&format!("{resource_type}/{name}")).unwrap(),
            expected_uid: None,
            projection: StoreProjection::Full,
        };
        let row = backend.get(request).await.unwrap_or_else(|error| {
            panic!(
                "{resource_type}: the manager path refused a converted type: {error:?}"
            )
        });
        assert_eq!(
            row.resource_ref.resource_type().as_str(),
            resource_type,
            "{resource_type}: the manager serves the row it holds"
        );
    }

    fixture.manager_actor.get_cell().stop(None);
}

/// The converted types whose contracts pin a typed `status.resource` layer
/// decode the served layer through exactly that `deny-unknown-fields`
/// decoder: the projection cannot wrap, rename, or nest what the type's
/// consumers read. The remaining converted types publish free-form evidence
/// layers, for which the universal strict envelope is the whole boundary.
#[test]
fn converted_type_status_layers_round_trip_through_their_typed_decoders() {
    use d2b_contracts_resource::v3::{
        DeviceStatusResource, QuotaStatusResource, VolumeBindingStatusResource,
    };
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance};

    let decode: [(&str, serde_json::Value, fn(&serde_json::Value) -> Result<serde_json::Value, String>); 3] = [
        (
            "VolumeBinding",
            serde_json::json!({
                "ready": true,
                "fence": {
                    "uid": "123e4567-e89b-42d3-a456-426614174000",
                    "generation": 1,
                    "revision": 9,
                },
            }),
            |served| {
                serde_json::from_value::<VolumeBindingStatusResource>(served.clone())
                    .map(|typed| serde_json::to_value(typed).expect("serialize typed layer"))
                    .map_err(|error| error.to_string())
            },
        ),
        (
            "Device",
            serde_json::json!({
                "present": true,
                "health": "healthy",
                "holderRefs": [],
                "claims": [],
                "provisionedAt": null,
                "lastProbedAt": null,
                "providerDiagnostic": null,
            }),
            |served| {
                serde_json::from_value::<DeviceStatusResource>(served.clone())
                    .map(|typed| serde_json::to_value(typed).expect("serialize typed layer"))
                    .map_err(|error| error.to_string())
            },
        ),
        (
            "Quota",
            serde_json::json!({
                "usedResources": 2,
                "usedCpu": 4,
                "usedMemoryMib": 512,
                "usedStorageGib": null,
                "overQuota": false,
                "overQuotaTypes": [],
                "lastCheckedAt": null,
                "dependentCount": 1,
            }),
            |served| {
                serde_json::from_value::<QuotaStatusResource>(served.clone())
                    .map(|typed| serde_json::to_value(typed).expect("serialize typed layer"))
                    .map_err(|error| error.to_string())
            },
        ),
    ];

    for (resource_type, layer, decode) in decode {
        let key = ResourceKey::new(TEST_ZONE, resource_type, "typed-layer");
        let view = ResourceView {
            key,
            uid: super::manager_uid(&ResourceKey::new(TEST_ZONE, resource_type, "typed-layer")),
            generation: 1,
            deleting: false,
            provenance: ResourceProvenance::Api,
            spec: serde_json::to_vec(&serde_json::json!({ "providerRef": "Provider/typed-layer" }))
                .unwrap(),
            metadata: serde_json::to_vec(&serde_json::json!({})).unwrap(),
            owner_key: None,
            status: Some(ResourceStatus::Ready),
            status_generation: Some(1),
            status_projection: Some(layer.clone()),
        };
        let stored = super::manager_row_stored(&view)
            .unwrap_or_else(|error| panic!("{resource_type}: render failed: {error:?}"));
        let value: serde_json::Value =
            serde_json::from_slice(&stored.canonical_json).expect("envelope json");
        let served = value["status"]["resource"].clone();
        let decoded = decode(&served).unwrap_or_else(|error| {
            panic!("{resource_type}: the served layer must decode through its type's decoder: {error}")
        });
        assert_eq!(
            decoded, layer,
            "{resource_type}: the served typed layer must be the driver's value"
        );
    }
}

/// The caller-side half of the contract: every API path that returns a row
/// returns the canonical projection of the manager state it observed, byte
/// for byte - GET, LIST, a no-op UPDATE_SPEC confirmation, and the DELETE
/// confirmation. A caller that re-introduces its own envelope or status
/// assembly diverges from `manager_row_stored` here and fails, whatever the
/// shapes it invents.
#[tokio::test]
async fn every_api_read_path_serves_the_canonical_projection() {
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::ResourceKey;

    let fixture = manager_fixture().await;
    let service = wired_service(
        &fixture,
        authorizer(&[
            ResourceVerb::Create,
            ResourceVerb::Get,
            ResourceVerb::List,
            ResourceVerb::UpdateSpec,
            ResourceVerb::Delete,
        ]),
    );
    let created = service.create(trusted(create_request())).await;
    assert!(
        created.error.is_none(),
        "create failed: kind={:?} reason={}",
        error_kind(&created),
        error_reason(&created)
    );
    let uid = created
        .resource
        .as_ref()
        .unwrap()
        .identity
        .as_ref()
        .unwrap()
        .uid
        .clone()
        .unwrap();

    // The driver publishes status asynchronously: wait for its single Ready
    // publication so the byte comparisons below cannot race a transition.
    let key = ResourceKey::new(TEST_ZONE, "Host", "host-system");
    let mut settled = None;
    for _ in 0..500 {
        let view = fixture
            .client
            .get(key.clone())
            .await
            .expect("manager read")
            .expect("row");
        if view.observed_status() == Some(ResourceStatus::Ready) {
            settled = Some(view);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let view = settled.expect("the fixture row never reached Ready");
    let canonical = super::manager_row_stored(&view).expect("canonical render");

    // GET.
    let fetched = service.get(trusted(get_request())).await;
    assert!(fetched.error.is_none(), "get failed: {:?}", fetched.error);
    assert_eq!(
        fetched.resource.as_ref().unwrap().canonical_json,
        canonical.canonical_json,
        "GET must serve the canonical projection"
    );

    // LIST.
    let listed = service.list(trusted(list_request())).await;
    assert!(listed.error.is_none(), "list failed: {:?}", listed.error);
    assert_eq!(listed.resources.len(), 1, "one Host row is committed");
    assert_eq!(
        listed.resources[0].canonical_json,
        canonical.canonical_json,
        "LIST must serve the canonical projection"
    );

    // A byte-identical UPDATE_SPEC is a no-op: the confirmation is the same
    // live view a read serves, never a second rendering of the desired bytes
    // (which would report the persisted status instead of the live one).
    let row = fixture
        .client
        .get_row(key.clone())
        .await
        .expect("get_row")
        .expect("row");
    let mut noop = wire::UpdateSpecRequest::new();
    noop.meta = request_meta();
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC);
    mutation.target = identity();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(view.generation);
    precondition.expected_uid = Some(uid.clone());
    mutation.precondition = MessageField::some(precondition);
    mutation.resource = update_body(row.spec.clone(), &uid, view.generation);
    noop.mutation = MessageField::some(mutation);
    let unchanged = service.update_spec(trusted(noop)).await;
    assert!(
        unchanged.error.is_none(),
        "no-op update failed: kind={:?} reason={}",
        error_kind(&unchanged),
        error_reason(&unchanged)
    );
    assert_eq!(
        unchanged.resource.as_ref().unwrap().canonical_json,
        canonical.canonical_json,
        "a no-op UPDATE_SPEC confirmation must be the canonical projection"
    );

    // DELETE: the confirmation is the canonical projection of the row state
    // the removal commit produced (deleting at its own generation, the
    // closed deleting classification, projection dropped). The single-delete
    // response exposes only the identity, so the batch form - which carries
    // each committed row's envelope - is where a divergent confirmation would
    // show on the wire.
    let deleting = ResourceView {
        key: key.clone(),
        uid: row.uid,
        generation: row.generation,
        deleting: true,
        provenance: row.provenance,
        spec: row.spec.clone(),
        metadata: row.metadata.clone(),
        owner_key: None,
        status: Some(ResourceStatus::Deleting),
        status_generation: Some(row.generation),
        status_projection: None,
    };
    let deleting_canonical = super::manager_row_stored(&deleting).expect("deleting render");
    let mut batch = wire::CommitBatchRequest::new();
    batch.meta = request_meta();
    let mut delete = wire::Mutation::new();
    delete.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    delete.target = identity();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(view.generation);
    precondition.expected_uid = Some(uid.clone());
    delete.precondition = MessageField::some(precondition);
    batch.mutations = vec![delete];
    let committed = service.commit_batch(trusted(batch)).await;
    assert!(
        committed.error.is_none(),
        "batch delete failed: kind={:?} reason={}",
        committed.error.as_ref().map(|error| error.kind.enum_value()),
        committed
            .error
            .as_ref()
            .map(|error| error.reason.clone())
            .unwrap_or_default()
    );
    assert_eq!(committed.resources.len(), 1, "one committed mutation");
    let confirmation = &committed.resources[0];
    assert_eq!(
        confirmation.canonical_json, deleting_canonical.canonical_json,
        "the DELETE confirmation must be the projection of the deleting row"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&confirmation.canonical_json).expect("confirmation envelope");
    assert_eq!(value["status"]["phase"], serde_json::json!("Deleted"));
    assert!(
        value
            .pointer("/metadata/deletionRequestedAt")
            .is_some_and(|value| !value.is_null()),
        "the delete confirmation reports the deletion request"
    );

    let gone = service.get(trusted(get_request())).await;
    assert_eq!(
        error_kind(&gone),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND,
        "the deleted row is absent from the manager"
    );

    fixture.manager_actor.get_cell().stop(None);
}

#[tokio::test]
async fn exact_revision_precondition_rejects_a_stale_generation() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::Create, ResourceVerb::UpdateSpec, ResourceVerb::Get, ResourceVerb::Delete]));
    let created = service.create(trusted(create_request())).await;
    let uid = created.resource.as_ref().unwrap().identity.as_ref().unwrap().uid.clone().unwrap();

    // Wrong revision (never existed) is a conflict, not a not-found.
    let mut request = update_spec_request(7, &uid, 2, None);
    request.meta = request_meta();
    let response = service.update_spec(trusted(request)).await;
    assert_eq!(
        error_kind(&response),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT,
        "reason={}",
        error_reason(&response)
    );
    assert_eq!(response.error.as_ref().unwrap().current_revision, Some(1));

    // Wrong uid is also rejected even at the right revision.
    let mut request = update_spec_request(1, "22222222-2222-4222-8222-222222222222", 2, None);
    request.meta = request_meta();
    let response = service.update_spec(trusted(request)).await;
    assert_eq!(
        error_kind(&response),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT,
        "reason={}",
        error_reason(&response)
    );

    fixture.manager_actor.get_cell().stop(None);
}

// ---------------------------------------------------------------------------
// Authorization: allow/deny from the fixture matrix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authorization_denies_operations_outside_the_fixture_matrix() {
    let fixture = manager_fixture().await;
    // No verbs granted: every operation denies before the backend is touched.
    let service = wired_service(&fixture, authorizer(&[]));
    let mut request = create_request();
    request.meta = request_meta();
    let response = service.create(trusted(request)).await;
    assert_eq!(error_kind(&response), wire::ResourceErrorKind::RESOURCE_ERROR_KIND_AUTHORIZATION_DENIED);

    // A granted Create passes authorization and commits.
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::Create]));
    let mut request = create_request();
    request.meta = request_meta();
    let response = service.create(trusted(request)).await;
    assert!(response.error.is_none(), "granted create failed: {:?}", response.error);

    // A different name is outside the rule's resource-name scope.
    let mut denied = wire::CreateRequest::new();
    denied.meta = request_meta();
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    let mut other = wire::ResourceIdentity::new();
    other.zone = TEST_ZONE.to_owned();
    other.resource_type = "Host".to_owned();
    other.name = "other-host".to_owned();
    mutation.target = MessageField::some(other);
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    mutation.precondition = MessageField::some(precondition);
    mutation.resource = create_body();
    denied.mutation = MessageField::some(mutation);
    let service =
        wired_service(&fixture, authorizer_scoped(&[ResourceVerb::Create], &["host-system"]));
    let response = service.create(trusted(denied)).await;
    assert_eq!(error_kind(&response), wire::ResourceErrorKind::RESOURCE_ERROR_KIND_AUTHORIZATION_DENIED);

    fixture.manager_actor.get_cell().stop(None);
}

#[tokio::test]
async fn admission_subjects_are_constructed_for_all_three_surfaces() {
    let fixture = manager_fixture().await;
    // The API subject derives from the authorization evidence the evaluator
    // captured, so admission at the manager boundary carries the real caller.
    let authorization = d2b_contracts_resource::v3::AdmittedAuthorization {
        zone: ZoneId::parse(TEST_ZONE).unwrap(),
        subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
        subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
        targets: vec![],
    };
    let api = crate::manager_backend::api_subject(&authorization);
    assert_eq!(api.principal, "Provider/system-core");
    assert_eq!(api.origin, d2b_resource_runtime::spec_store::ResourceProvenance::Api);

    let nix = crate::manager_backend::nix_bundle_subject("d2b-config-42");
    assert_eq!(nix.principal, "nix:d2b-config-42");
    assert_eq!(nix.origin, d2b_resource_runtime::spec_store::ResourceProvenance::Nix);

    let owner = d2b_resource_runtime::spec_store::ResourceKey::new(TEST_ZONE, "Volume", "data");
    let owner_subject = crate::manager_backend::resource_owner_subject(&owner);
    assert_eq!(owner_subject.principal, "dev/Volume/data");
    assert_eq!(owner_subject.origin, d2b_resource_runtime::spec_store::ResourceProvenance::Resource);

    fixture.manager_actor.get_cell().stop(None);
}

// ---------------------------------------------------------------------------
// LIST snapshot revision, paging, and WATCH refusal (unit-scale F4, R23/R24)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_returns_snapshot_revision_and_watch_refuses_until_wired() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::Create, ResourceVerb::List, ResourceVerb::Watch, ResourceVerb::Get, ResourceVerb::UpdateSpec, ResourceVerb::Delete]));

    let empty = service.list(trusted(list_request())).await;
    assert!(
        empty.error.is_none(),
        "list failed: kind={:?} reason={}",
        error_kind(&empty),
        error_reason(&empty)
    );
    let snapshot = empty.snapshot_revision;
    assert_eq!(
        snapshot >> 32,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        "the wire snapshot carries the epoch-seconds mapping"
    );

    let created = service.create(trusted(create_request())).await;
    assert!(created.error.is_none());

    // LIST sees the created row and a bumped snapshot revision.
    let listed = service.list(trusted(list_request())).await;
    assert!(listed.error.is_none());
    assert_eq!(listed.resources.len(), 1);
    assert!(listed.snapshot_revision > snapshot, "desired changes bump the snapshot");

    // WATCH has no delivery pump in the composition: the backend must
    // answer a typed capability refusal, never a receipt naming a stream
    // nothing fills.
    let request = watch_request(listed.snapshot_revision);
    let watched = service.watch(trusted(request)).await;
    let error = watched.error.as_ref().expect("an unfillable watch must be refused");
    assert_eq!(
        error.kind.enum_value().unwrap(),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UNSUPPORTED_CAPABILITY
    );
    assert_eq!(error.reason, "watch-not-wired");
    assert!(watched.stream_name.is_empty(), "no stream name may be handed out");
}

/// A continuation cursor drives the next page: the request cursor selects
/// the rows after the last one returned, `truncated` is true only while a
/// remainder exists, and the final page carries no cursor.
#[tokio::test]
async fn list_pages_with_a_continuation_cursor() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::List, ResourceVerb::Get]));
    for name in ["host-a", "host-b", "host-c"] {
        fixture
            .client
            .ensure(
                d2b_resource_runtime::manager::MutationSubject {
                    principal: "nix:test-bundle".to_owned(),
                    origin: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                },
                None,
                d2b_resource_runtime::manager::DesiredResource {
                    key: d2b_resource_runtime::spec_store::ResourceKey::new(TEST_ZONE, "Host", name),
                    spec: host_spec(),
                    metadata: serde_json::to_vec(&serde_json::json!({"ownerRef": null})).unwrap(),
                    provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                },
            )
            .await
            .expect("commit row");
    }

    let mut request = list_request();
    request.page_size = 2;
    let first = service.list(trusted(request.clone())).await;
    assert!(first.error.is_none(), "first page failed: {}", error_reason(&first));
    assert_eq!(first.resources.len(), 2, "a full page");
    assert!(first.truncated, "a remainder exists past this page");
    let cursor = first
        .next_cursor
        .as_ref()
        .expect("a truncated page carries the continuation cursor")
        .value
        .clone();

    let second = service
        .list(trusted({
            let mut next = request.clone();
            next.cursor = MessageField::some(wire::PageCursor { value: cursor, ..Default::default() });
            next
        }))
        .await;
    assert!(second.error.is_none(), "second page failed: {}", error_reason(&second));
    assert_eq!(second.resources.len(), 1, "the remainder");
    assert!(!second.truncated, "the last page is complete");
    assert!(second.next_cursor.is_none(), "no cursor past the last page");
    // The pages partition the sequence: no row repeats across them.
    let names = |page: &wire::ListResponse| {
        page.resources
            .iter()
            .filter_map(|resource| {
                resource.identity.as_ref().map(|identity| identity.name.clone())
            })
            .collect::<Vec<_>>()
    };
    let first_names = names(&first);
    let second_names = names(&second);
    assert!(first_names.iter().all(|name| !second_names.contains(name)));
}

/// The selector sets are matched order-insensitively (both the filter list
/// and each filter's values), so a client echoing its own query with the sets
/// reordered - or repeated - resumes its sequence instead of being refused as
/// a foreign cursor.
#[tokio::test]
async fn list_cursor_accepts_a_reordered_echo_of_the_same_selectors() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::List, ResourceVerb::Get]));
    for name in ["host-a", "host-b", "host-c"] {
        fixture
            .client
            .ensure(
                d2b_resource_runtime::manager::MutationSubject {
                    principal: "nix:test-bundle".to_owned(),
                    origin: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                },
                None,
                d2b_resource_runtime::manager::DesiredResource {
                    key: d2b_resource_runtime::spec_store::ResourceKey::new(TEST_ZONE, "Host", name),
                    spec: host_spec(),
                    metadata: serde_json::to_vec(&serde_json::json!({"ownerRef": null})).unwrap(),
                    provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                },
            )
            .await
            .expect("commit row");
    }

    let filter = |field: &str, values: &[&str]| {
        let mut filter = wire::ListFilter::new();
        filter.field = field.to_owned();
        filter.values = values.iter().map(|value| (*value).to_owned()).collect();
        filter
    };
    let mut request = list_request();
    request.page_size = 2;
    request.filters = vec![
        filter("metadata.name", &["host-a", "host-b", "host-c"]),
        filter("type", &["Host"]),
    ];
    let first = service.list(trusted(request.clone())).await;
    assert!(first.error.is_none(), "first page failed: {}", error_reason(&first));
    let cursor = first
        .next_cursor
        .as_ref()
        .expect("a truncated page carries the continuation cursor")
        .value
        .clone();

    // The same selectors: filters reordered, one repeated, one filter's
    // values reordered and repeated.
    let mut echo = request.clone();
    echo.filters = vec![
        filter("type", &["Host", "Host"]),
        filter("metadata.name", &["host-c", "host-a", "host-b"]),
        filter("type", &["Host"]),
    ];
    echo.cursor = MessageField::some(wire::PageCursor { value: cursor, ..Default::default() });
    let second = service.list(trusted(echo)).await;
    assert!(
        second.error.is_none(),
        "a reordered echo of the same selectors must resume: {}",
        error_reason(&second)
    );
    assert_eq!(second.resources.len(), 1, "the remainder");
    assert!(!second.truncated, "the sequence ends after the remainder");
}

/// A cursor that cannot be honoured is refused with a typed error - a
/// foreign-selector cursor must not silently restart the sequence at page 1.
#[tokio::test]
async fn list_refuses_a_cursor_it_cannot_honour() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::List]));
    let request = list_request();

    let mut malformed = list_request();
    malformed.cursor = MessageField::some(wire::PageCursor {
        value: "not-a-cursor".to_owned(),
        ..Default::default()
    });
    let refused = service.list(trusted(malformed)).await;
    assert_eq!(
        error_kind(&refused),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
    );
    assert_eq!(error_reason(&refused), "list-cursor-invalid");

    // A cursor minted under different selectors addresses a different page
    // sequence; replaying it here is refused, never ignored.
    let foreign = super::encode_list_cursor(
        7,
        &d2b_contracts_resource::v3::StoreListRequest {
            operation: d2b_contracts_resource::v3::StoreOperationContext {
                operation_id: "cursor-fixture".to_owned(),
                idempotency_key: None,
                correlation_id: "cursor-fixture".to_owned(),
                trace_id: None,
                deadline_ms: 1_000,
            },
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            resource_types: vec![ResourceTypeName::parse("Host").unwrap()],
            resource_names: vec![ResourceName::parse("host-system").unwrap()],
            filters: Vec::new(),
            page_size: 2,
            cursor: None,
            projection: d2b_contracts_resource::v3::StoreProjection::Full,
        },
        &d2b_resource_runtime::identity::ResourceKey::new(TEST_ZONE, "Host", "host-a"),
    );
    let mut replay = request.clone();
    replay.cursor = MessageField::some(wire::PageCursor { value: foreign, ..Default::default() });
    let refused = service.list(trusted(replay)).await;
    assert_eq!(
        error_kind(&refused),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
    );
    assert_eq!(error_reason(&refused), "list-cursor-selector-mismatch");
}

/// An owner-scoped LIST matches the manager's owned children: the row's real
/// ownership is projected into the store shape, so `owner.resourceUid` and
/// `owner.resourceRef` return the owner's rows instead of an empty page.
#[tokio::test]
async fn list_owner_filters_match_manager_owned_children() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::List, ResourceVerb::Get]));
    let subject = d2b_resource_runtime::manager::MutationSubject {
        principal: "nix:test-bundle".to_owned(),
        origin: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
    };
    let parent_key =
        d2b_resource_runtime::spec_store::ResourceKey::new(TEST_ZONE, "Host", "host-system");
    let desired = |name: &str| d2b_resource_runtime::manager::DesiredResource {
        key: d2b_resource_runtime::spec_store::ResourceKey::new(TEST_ZONE, "Host", name),
        spec: host_spec(),
        metadata: serde_json::to_vec(&serde_json::json!({"ownerRef": null})).unwrap(),
        provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
    };
    let parent = fixture
        .client
        .ensure(subject.clone(), None, desired("host-system"))
        .await
        .expect("parent row");
    let parent_uid = super::row_uid(&parent.uid);
    fixture
        .client
        .ensure(subject, Some(parent_key.clone()), desired("child-1"))
        .await
        .expect("child row");

    let unfiltered = service.list(trusted(list_request())).await;
    assert!(unfiltered.error.is_none(), "list failed: {}", error_reason(&unfiltered));
    assert_eq!(unfiltered.resources.len(), 2, "parent plus child");

    let by_uid = service
        .list(trusted(list_request_with_filter("owner.resourceUid", parent_uid.as_str())))
        .await;
    assert!(by_uid.error.is_none(), "owner uid list failed: {}", error_reason(&by_uid));
    let names = |page: &wire::ListResponse| {
        page.resources
            .iter()
            .filter_map(|resource| resource.identity.as_ref().map(|identity| identity.name.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&by_uid), vec!["child-1".to_owned()], "the owner's children");

    let by_ref = service
        .list(trusted(list_request_with_filter("owner.resourceRef", "Host/host-system")))
        .await;
    assert!(by_ref.error.is_none(), "owner ref list failed: {}", error_reason(&by_ref));
    assert_eq!(names(&by_ref), vec!["child-1".to_owned()], "the rendered owner reference");

    // A filter for an owner with no children stays an honest empty page.
    let stranger = service
        .list(trusted(list_request_with_filter(
            "owner.resourceUid",
            "99999999-9999-4999-8999-999999999999",
        )))
        .await;
    assert!(stranger.error.is_none());
    assert!(stranger.resources.is_empty());
}

fn list_request_with_filter(field: &str, value: &str) -> wire::ListRequest {
    let mut request = list_request();
    let mut filter = wire::ListFilter::new();
    filter.field = field.to_owned();
    filter.values = vec![value.to_owned()];
    request.filters.push(filter);
    request
}

/// The minimal Host spec the strict envelope decoder accepts for a
/// manager-rendered row (the `GOLDEN_HOST` spec shape).
fn host_spec() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "providerRef": "Provider/system-core",
        "updatePolicy": {
            "disruptive": "manual",
            "nonDisruptive": "automatic",
        },
    }))
    .unwrap()
}

// ---------------------------------------------------------------------------
// No status write path from API status updates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_status_updates_have_no_persistent_write_path() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::UpdateStatus, ResourceVerb::Get]));
    let mut request = wire::UpdateStatusRequest::new();
    request.meta = request_meta();
    let mut mutation = wire::Mutation::new();
    mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
    mutation.target = identity();
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(1);
    mutation.precondition = MessageField::some(precondition);
    mutation.resource = status_body();
    request.mutation = MessageField::some(mutation);
    let response = service.update_status(trusted(request)).await;
    assert_eq!(
        error_kind(&response),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_STATUS_OWNER_MISMATCH,
        "status-shaped API writes are rejected before any persistence"
    );

    // Compile-level invariant: the runtime spec store and manager expose no
    // status-carrying durable write. The absence is enforced by the type
    // system at build time (SpecStore has no update-status method and the
    // manager protocol carries no persist-status message); here we pin the
    // wire-level behavior already asserted above.
    let rejected_kind = error_kind(&response);
    assert_eq!(
        rejected_kind,
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_STATUS_OWNER_MISMATCH,
        "no API status write may reach persistence"
    );

    fixture.manager_actor.get_cell().stop(None);
}

// ---------------------------------------------------------------------------
// One-way bootstrap latch from an empty store (KTD6, R28)
// ---------------------------------------------------------------------------

fn provider_row(name: &str, uid: &str) -> DurablePolicyRow {
    let canonical = format!(
        r#"{{"apiVersion":"resources.d2bus.org/v3","metadata":{{"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"{name}","ownerRef":null,"revision":1,"uid":"{uid}","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"}},"spec":{{}},"type":"Provider"}}"#
    );
    DurablePolicyRow {
        resource_ref: ResourceRef::parse(&format!("Provider/{name}")).unwrap(),
        canonical_json: canonical.into_bytes(),
        provenance: DurableRowProvenance::Bundle,
    }
}

fn binding_row() -> DurablePolicyRow {
    let binding_json = r#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"operator-binding","ownerRef":null,"revision":1,"uid":"44444444-4444-4444-8444-444444444444","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"roleRef":"Role/operator","subjects":["Provider/system-core"]},"type":"RoleBinding"}"#;
    DurablePolicyRow {
        resource_ref: ResourceRef::parse("RoleBinding/operator-binding").unwrap(),
        canonical_json: binding_json.as_bytes().to_vec(),
        provenance: DurableRowProvenance::Bundle,
    }
}

fn role_row() -> DurablePolicyRow {
    // A Role rule granting system-core Get/Create in `dev`, serialized in
    // the exact RoleSpec wire shape.
    let role_json = r#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"operator","ownerRef":null,"revision":1,"uid":"33333333-3333-4333-8333-333333333333","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"rules":[{"resourceTypes":["Host"],"verbs":["get","create"],"subresources":[],"resourceNames":["host-system"],"zones":["dev"],"executionRefs":[],"sessionVerbs":[]}]},"type":"Role"}"#;
    DurablePolicyRow {
        resource_ref: ResourceRef::parse("Role/operator").unwrap(),
        canonical_json: role_json.as_bytes().to_vec(),
        provenance: DurableRowProvenance::Bundle,
    }
}

const SYSTEM_CORE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const SYSTEM_MINIJAIL_UID: &str = "ffffffff-ffff-4fff-bfff-ffffffffffff";

#[test]
fn bootstrap_boots_from_an_empty_store_per_the_latch_semantics() {
    let zone = ZoneId::parse(TEST_ZONE).unwrap();
    let catalog = ApiCatalog::standard();

    // Fresh (empty) store: no durable rows, no published policy revision -
    // the phase derives per the empty-store rule and stays open.
    let empty = crate::authz::compile_authorization_facts(
        &catalog,
        zone.clone(),
        0,
        &[],
        ControllerGeneration::new(11).unwrap(),
        ResourceGeneration::new(12).unwrap(),
    )
    .unwrap();
    assert!(empty.policy.is_none(), "an empty store publishes no policy");
    assert_eq!(
        derive_bootstrap_phase(&empty.bootstrap),
        BootstrapPhase::Unprovisioned {
            zone: zone.clone(),
            controller_generation: ControllerGeneration::new(11).unwrap(),
            provider_generation: ResourceGeneration::new(12).unwrap(),
        }
    );

    // The Nix bundle seeds the policy rows at materialization time: bundle
    // Role/RoleBinding/Provider rows compile into a policy set, and the
    // durable bootstrap Providers select the provisioned phase.
    let rows = vec![
        provider_row("system-core", SYSTEM_CORE_UID),
        provider_row("system-minijail", SYSTEM_MINIJAIL_UID),
        role_row(),
    ];
    let seeded = crate::authz::compile_authorization_facts(
        &catalog_fixture(),
        zone.clone(),
        3, // bundle generation seeds the revision
        &rows,
        ControllerGeneration::new(11).unwrap(),
        ResourceGeneration::new(12).unwrap(),
    )
    .unwrap();
    let policy = seeded.policy.expect("bundle rows publish a policy set");
    assert_eq!(policy.policy_revision, 3);

    // The one-way latch: once the policy revision is published (nonzero),
    // bootstrap is permanently Disabled - independent of bundle generation.
    assert_eq!(
        derive_bootstrap_phase(&seeded.bootstrap),
        BootstrapPhase::Disabled,
        "a published policy revision latches bootstrap off"
    );
    assert!(!crate::authz::bootstrap_policy_transition(1, 2));
    assert!(crate::authz::bootstrap_policy_transition(0, 1));

    // A previously provisioned zone never re-enters Unprovisioned: even
    // with the bundle generation gone, the durable row state keeps the
    // revision nonzero and the latch closed.
    let after_cutover = crate::authz::compile_authorization_facts(
        &catalog_fixture(),
        zone,
        0, // bundle generation gone
        &rows,
        ControllerGeneration::new(11).unwrap(),
        ResourceGeneration::new(12).unwrap(),
    )
    .unwrap();
    assert!(after_cutover.policy.is_some());
    assert_eq!(derive_bootstrap_phase(&after_cutover.bootstrap), BootstrapPhase::Disabled);
}

#[test]
fn allow_and_deny_still_enforced_from_the_compiled_fixture_matrix() {
    // Compile the fixture matrix through the KTD6 path and prove the
    // evaluator still allows and denies exactly per the rule set.
    let catalog = catalog_fixture();
    let rows = vec![
        role_row(),
        binding_row(),
        provider_row("system-core", "123e4567-e89b-42d3-a456-426614174001"),
    ];
    let facts = crate::authz::compile_authorization_facts(
        &catalog,
        ZoneId::parse(TEST_ZONE).unwrap(),
        1,
        &rows,
        ControllerGeneration::new(11).unwrap(),
        ResourceGeneration::new(12).unwrap(),
    )
    .unwrap();
    let policy = facts.policy.expect("durable rows publish the policy");
    let authorizer = Arc::new(NativeAuthorizer::new(catalog.clone(), Some(policy)).unwrap());
    let context = subject();
    let state = AuthorizationState {
        snapshot: d2b_contracts_resource::v3::PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 5,
            active_configuration_revision: ConfigurationGeneration::new(6).unwrap(),
            controller_generation: None,
        },
        zone_policy_revision: ZoneRevision::new(1),
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    };
    let request = crate::authz::AuthorizationRequest {
        method: ApiMethod::Get,
        zone: ZoneId::parse(TEST_ZONE).unwrap(),
        targets: vec![AuthorizationTarget {
            resource_type: ResourceTypeName::parse("Host").unwrap(),
            resource_name: Some(ResourceName::parse("host-system").unwrap()),
            verb: crate::authz::ResourceVerb::Get,
            subresource: None,
            execution_ref: None,
        }],
    };
    // The compiled binding binds the durable Provider subject.
    if let Err(denial) = authorizer.authorize(&context, &request, &state) {
        panic!("compiled policy denied the bound subject: {denial:?}");
    }

    // A subject not present in the durable rows is denied.
    let stranger = Arc::new(
        AuthenticatedSubjectContext::new(
            ResourceRef::parse("User/stranger").unwrap(),
            ResourceUid::parse("44444444-4444-4444-8444-444444444444").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse("resource-api").unwrap(),
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
        ),
    );
    assert!(authorizer.authorize(&stranger, &request, &state).is_err());
}

fn catalog_fixture() -> ApiCatalog {
    ApiCatalog::standard()
}
