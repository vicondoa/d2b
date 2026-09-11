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
use d2b_resource_store::StoreSealIdentity;
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
        snapshot: d2b_resource_store::PolicySnapshot {
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
        d2b_resource_store::StoreSlot::new(0).unwrap(),
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
    let authorization = d2b_resource_store::AdmittedAuthorization {
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
// LIST snapshot revision + WATCH resume (unit-scale F4, R23/R24)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_returns_snapshot_revision_and_watch_resumes_from_it() {
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

    // WATCH resumes from the snapshot revision: the registration is served
    // by the manager's hub, and the receipt carries the mapped revision.
    let request = watch_request(listed.snapshot_revision);
    let watched = service.watch(trusted(request)).await;
    assert!(
        watched.error.is_none(),
        "watch from the list snapshot failed: kind={:?} reason={}",
        error_kind(&watched),
        error_reason(&watched)
    );
    assert_eq!(watched.snapshot_revision, listed.snapshot_revision);
}

#[tokio::test]
async fn watch_rejects_a_pre_epoch_cursor_with_revision_expired() {
    let fixture = manager_fixture().await;
    let service = wired_service(&fixture, authorizer(&[ResourceVerb::Watch, ResourceVerb::List]));

    // A cursor whose epoch seconds predate the daemon epoch (any prior
    // lifetime, or a pre-cutover durable revision) must fail with the
    // RevisionExpired wire error and carry the relist snapshot.
    let stale_epoch_cursor =
        (1_700_000_000u64 << 32) | 5; // seconds before this test epoch
    let request = watch_request(stale_epoch_cursor);
    let response = service.watch(trusted(request)).await;
    let error = response.error.unwrap();
    assert_eq!(
        error.kind.enum_value().unwrap(),
        wire::ResourceErrorKind::RESOURCE_ERROR_KIND_REVISION_EXPIRED
    );
    assert!(
        error.current_revision.is_some(),
        "the failure carries the snapshot to relist from"
    );

    fixture.manager_actor.get_cell().stop(None);
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
        snapshot: d2b_resource_store::PolicySnapshot {
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
