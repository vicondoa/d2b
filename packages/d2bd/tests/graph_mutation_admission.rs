//! New-graph mutation admission and relation indexing (U6, KTD2-KTD4; R2-R8,
//! R16-R18, R35; AE15).
//!
//! These cases drive the construction U34 installs in production: the manager
//! calls the plane's injected [`GraphMutationAdmission`], which defers every
//! rule to the one pure evaluator, and the manager's derived relation indexes
//! are rebuilt from committed rows alone. The unchanged production entry point
//! (`SystemZoneWriteFence` plus the string-subject messages) is deliberately
//! untouched and is not exercised here.

use std::sync::Arc;

use d2b_contracts_resource::v3::volume::AttachmentAccess;
use d2b_contracts_resource::v3::{
    AuthoritySubject, AuthoritySubjectKind, BindingAuthorization, BindingKind, BindingSlot,
    DesiredDigest, DesiredRevision, FreshnessTuple, ResourceRef, ResourceUid, StoreIncarnation,
    VolumeBindingRequest, VolumePresentation, ZoneId,
};
use d2b_contracts_zone_session::v3::role::AuthorizedRole;
use d2b_contracts_zone_session::v3::RoleBindingSpec;
use d2b_core::resource_authority::{
    AcceptedGraph, AcceptedSource, BindingAdmissionRequest, GraphAuthority, GraphMutation,
    MutationKind, MutationSubjectEvidence, TransportIdentity,
};
use d2b_resource_runtime::context::{ManagerEndpoint, SpecDecoder};
use d2b_resource_runtime::manager::{
    AuthenticatedIdentity, AuthenticatedMutation, DesiredResource, ManagerActorEndpoint,
    ResourceManager, ResourceManagerArgs, ResourceManagerClient, ResourceManagerMsg,
    source_controller_kind,
};
use d2b_resource_runtime::relations::{
    BindingRequestRelations, OperationImplementationRelations, RelationExtractors, RelationExtractor,
    RelationEdge, RelationResolver, RelationRow,
};
use d2b_resource_runtime::spec_store::{ResourceKey, SpecStore};
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2bd::GraphMutationAdmission;
use ractor::{Actor, ActorRef};

const ZONE: &str = "graph-admission";
const STORE: &str = "store-generation-1";

/// The ResourceTypes whose committed bytes this crate's own projection reads.
///
/// The authorization and observation classes have no contract-layer schema
/// this crate may depend on, so the fixture supplies a projection over two
/// ordinary rows it writes itself. That is the whole point of the seam: the
/// index derives a class from committed bytes the owning projection
/// understands, and the converted families land with their own units.
const FIXTURE_TYPES: [&str; 2] = ["RoleBinding", "Guest"];

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).unwrap()
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).unwrap()
}

fn uid_of(key: &ResourceKey) -> ResourceUid {
    ResourceUid::from_bytes(&d2b_resource_runtime::manager::deterministic_uid(key))
        .expect("a manager row uid is a canonical uuid")
}

/// The fixture's own canonical relationship declaration.
///
/// It is written into a committed row's desired bytes and read back by the
/// injected projection, so the index can only hold what the bytes say.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureDeclaration {
    authorization: Option<FixtureAuthorization>,
    observation: Option<FixtureObservation>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureAuthorization {
    role: String,
    subjects: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureObservation {
    observer: String,
    observed: String,
}

/// The fixture's projection: it reads only the two classes the converted
/// families do not yet contribute, and resolves every declared reference
/// through the manager's committed rows.
struct FixtureRelations;

impl RelationExtractor for FixtureRelations {
    fn resource_types(&self) -> &[&'static str] {
        &FIXTURE_TYPES
    }

    fn extract(
        &self,
        row: &RelationRow<'_>,
        resolve: &RelationResolver<'_>,
    ) -> Result<Vec<RelationEdge>, d2b_resource_runtime::relations::RelationError> {
        use d2b_resource_runtime::relations::{
            AuthorizationRelation, ObservationRelation, RelationError,
        };
        let Ok(declaration) = serde_json::from_slice::<FixtureDeclaration>(row.spec()) else {
            return Ok(Vec::new());
        };
        let mut edges = Vec::new();
        if let Some(authorization) = declaration.authorization {
            let subjects: Vec<ResourceRef> = authorization
                .subjects
                .iter()
                .map(|value| {
                    ResourceRef::parse(value).map_err(|_| RelationError::UnresolvedReference)
                })
                .collect::<Result<Vec<_>, _>>()?;
            edges.push(RelationEdge::Authorization(AuthorizationRelation {
                binding: row.resource_ref().clone(),
                role: reference(&authorization.role),
                subjects,
            }));
        }
        if let Some(observation) = declaration.observation {
            edges.push(RelationEdge::Observation(ObservationRelation {
                observer: resolve.uid_of(&reference(&observation.observer))?,
                observed: resolve.uid_of(&reference(&observation.observed))?,
            }));
        }
        Ok(edges)
    }
}

/// A driver that realizes nothing.
///
/// The admission and relation surfaces under test are decided before and
/// around the row, not by its driver, so a driver that holds no privilege is
/// exactly what keeps the fixture honest: a row that spawned with authority
/// would prove nothing.
struct InertDriver;

#[async_trait::async_trait]
impl d2b_resource_runtime::driver::DynResourceDriver for InertDriver {
    async fn validate(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<(), d2b_resource_runtime::error::DriverFailure> {
        Ok(())
    }
    async fn recover(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<d2b_resource_runtime::driver::RecoveryOutcome, d2b_resource_runtime::error::DriverFailure>
    {
        Ok(d2b_resource_runtime::driver::RecoveryOutcome::Adopted)
    }
    async fn reconcile(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<d2b_resource_runtime::driver::ReconcileOutcome, d2b_resource_runtime::error::DriverFailure>
    {
        Ok(d2b_resource_runtime::driver::ReconcileOutcome::Satisfied)
    }
    async fn finalize(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<(), d2b_resource_runtime::error::DriverFailure> {
        Ok(())
    }
    async fn delete(
        &mut self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
    ) -> Result<(), d2b_resource_runtime::error::DriverFailure> {
        Ok(())
    }
}

struct InertFactory;

#[async_trait::async_trait]
impl d2b_resource_runtime::driver::ResourceDriverFactory for InertFactory {
    fn resource_types(&self) -> &[d2b_resource_runtime::identity::ResourceTypeName] {
        // Leaked statics: one fixed type list for the test process.
        Box::leak(Box::new([
            d2b_resource_runtime::identity::ResourceTypeName::new("User"),
            d2b_resource_runtime::identity::ResourceTypeName::new("Provider"),
            d2b_resource_runtime::identity::ResourceTypeName::new("Role"),
            d2b_resource_runtime::identity::ResourceTypeName::new("RoleBinding"),
            d2b_resource_runtime::identity::ResourceTypeName::new("Volume"),
            d2b_resource_runtime::identity::ResourceTypeName::new("Guest"),
            d2b_resource_runtime::identity::ResourceTypeName::new(
                BindingKind::Volume.resource_type(),
            ),
        ]))
    }
    async fn create(
        &self,
        _key: &ResourceKey,
    ) -> Box<dyn d2b_resource_runtime::driver::DynResourceDriver> {
        Box::new(InertDriver)
    }
}

fn providers() -> d2b_resource_runtime::provider::ProviderDirectory {
    let mut directory = d2b_resource_runtime::provider::ProviderDirectory::new();
    directory
        .register(Arc::new(InertFactory)
            as Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory>)
        .expect("fixture factory registration");
    directory
}

struct NoDecoder;

impl SpecDecoder for NoDecoder {
    fn decode(
        &self,
        _envelope: &[u8],
    ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Err("no decoder".into())
    }
}

struct HostOnlyResolver;

impl TargetResolver for HostOnlyResolver {
    fn execution_ref(&self, _key: &ResourceKey, _spec: &[u8]) -> Option<String> {
        None
    }
}

struct Fixture {
    client: ResourceManagerClient,
    endpoint: ManagerActorEndpoint,
    /// `None` for a restart fixture, whose rows live in the store it inherited.
    store: Option<Arc<SpecStore>>,
    _directory: tempfile::TempDir,
}

fn role() -> AuthorizedRole {
    use d2b_contracts_zone_session::v3::RoleResourceVerb;
    let mut rules = Vec::new();
    for resource_type in [BindingKind::Volume.resource_type(), "Volume"] {
        rules.push(
            d2b_contracts_zone_session::v3::RoleRule::new(
                vec![d2b_contracts_resource::v3::ResourceTypeName::parse(resource_type).unwrap()],
                vec![RoleResourceVerb::Create, RoleResourceVerb::Delete],
                Vec::new(),
                Vec::new(),
                vec![zone()],
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
        );
    }
    AuthorizedRole::new(rules, Vec::new()).unwrap()
}

fn role_binding() -> RoleBindingSpec {
    RoleBindingSpec::new(
        reference("Role/volume-operator"),
        vec![reference("User/operator"), reference("Provider/volume-local")],
        None,
        None,
    )
    .unwrap()
}

fn accepted_graph() -> AcceptedGraph {
    AcceptedGraph::new(
        zone(),
        StoreIncarnation::parse(STORE).unwrap(),
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
    )
    .with_role(reference("Role/volume-operator"), role())
    .with_role_binding(reference("RoleBinding/operators"), role_binding())
}

async fn spawn_fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(SpecStore::open(directory.path().join("specs.sqlite")).unwrap());
    let mut relations = RelationExtractors::new();
    relations
        .register(Arc::new(BindingRequestRelations))
        .unwrap();
    relations
        .register(Arc::new(OperationImplementationRelations))
        .unwrap();
    relations.register(Arc::new(FixtureRelations)).unwrap();
    let hub = Arc::new(d2b_resource_runtime::watch::WatchHub::new(
        &d2b_resource_runtime::revision::SystemClock,
        d2b_resource_runtime::watch::DEFAULT_RING_CAPACITY,
    ));
    let args = ResourceManagerArgs {
        zone: ZONE.to_owned(),
        store: store.clone(),
        providers: providers(),
        hub,
        admission: Arc::new(GraphMutationAdmission::new(
            Arc::new(accepted_graph()),
            zone(),
            TransportIdentity::ComponentSession,
        )),
        decoders: Default::default(),
        default_decoder: Arc::new(NoDecoder),
        targets: Arc::new(TargetDirectory::new()),
        host_target: TargetRef::host("test-host").unwrap(),
        target_resolver: Arc::new(HostOnlyResolver),
        backoff: std::time::Duration::from_millis(200),
        relation_extractors: relations,
    };
    let (actor, _join): (ActorRef<ResourceManagerMsg>, _) =
        Actor::spawn(None, ResourceManager::new(), args).await.unwrap();
    Fixture {
        client: ResourceManagerClient::new(actor.clone()),
        endpoint: ManagerActorEndpoint::new(actor),
        store: Some(store),
        _directory: directory,
    }
}

fn key(type_name: &str, name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, type_name, name)
}

fn desired(type_name: &str, name: &str, spec: Vec<u8>) -> DesiredResource {
    DesiredResource {
        key: key(type_name, name),
        spec,
        metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
        provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Api,
    }
}

/// The verified deployment graph's own authority: it seeds the initial rows.
fn bootstrap() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        ResourceUid::parse("00000000-0000-4000-8000-000000000000").unwrap(),
    ))
}

fn user_principal() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::named(
            AuthoritySubjectKind::User,
            reference("User/operator"),
        ),
        uid_of(&key("User", "operator")),
    ))
}

fn provider_principal() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::named(
            AuthoritySubjectKind::Provider,
            reference("Provider/volume-local"),
        ),
        uid_of(&key("Provider", "volume-local")),
    ))
}

/// The evidence a Volume source controller produces after admitting a
/// consumer's request: its own authenticated identity, naming the source whose
/// relationship it admits.
fn volume_controller() -> AuthenticatedMutation {
    provider_principal().with_source_controller(AuthenticatedIdentity::new(
        AuthoritySubject::named(
            source_controller_kind("Volume").expect("Volume is a primitive source family"),
            reference("Volume/data"),
        ),
        uid_of(&key("Volume", "data")),
    ))
}

fn binding_spec(slot: &str, access: AttachmentAccess) -> Vec<u8> {
    serde_json::to_vec(
        &VolumeBindingRequest::new(
            reference("Volume/data"),
            reference("Guest/vm"),
            BindingSlot::parse(slot).unwrap(),
            d2b_contracts_resource::v3::BoundedToken::parse("root").unwrap(),
            access,
            VolumePresentation::filesystem("/state").unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn observation_spec(observer: &str, observed: &str) -> Vec<u8> {
    serde_json::to_vec(&FixtureDeclaration {
        authorization: None,
        observation: Some(FixtureObservation {
            observer: observer.to_owned(),
            observed: observed.to_owned(),
        }),
    })
    .unwrap()
}

fn authorization_spec(role: &str, subjects: &[&str]) -> Vec<u8> {
    serde_json::to_vec(&FixtureDeclaration {
        authorization: Some(FixtureAuthorization {
            role: role.to_owned(),
            subjects: subjects.iter().map(|value| (*value).to_owned()).collect(),
        }),
        observation: None,
    })
    .unwrap()
}

async fn seed(fixture: &Fixture) {
    for (type_name, name, spec) in [
        ("User", "operator", Vec::new()),
        ("Provider", "volume-local", Vec::new()),
        ("Role", "volume-operator", Vec::new()),
        ("RoleBinding", "operators", authorization_spec("Role/volume-operator", &[
            "User/operator",
            "Provider/volume-local",
        ])),
        ("Volume", "data", Vec::new()),
        ("Guest", "vm", observation_spec("Guest/vm", "Volume/data")),
    ] {
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired(type_name, name, spec))
            .await
            .unwrap_or_else(|error| panic!("seed {type_name}/{name}: {error}"));
    }
}

fn freshness(key: &ResourceKey) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse(STORE).unwrap(),
        reference(&format!("{}/{}", key.type_name, key.name)),
        uid_of(key),
        DesiredRevision::INITIAL,
        DesiredDigest::of(b"committed"),
    )
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_provider_created_child_and_an_api_mutation_are_admitted_under_their_own_subject() {
    let fixture = spawn_fixture().await;
    seed(&fixture).await;

    // The source controller's driver creates the source-owned binding under
    // its own authenticated evidence.
    let handle = fixture
        .endpoint
        .ensure_source_owned_binding(
            &key("Volume", "data"),
            volume_controller(),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: d2b_resource_runtime::identity::ResourceTypeName::new(
                    BindingKind::Volume.resource_type(),
                ),
                name: "vm-state".to_owned(),
                spec: binding_spec("state", AttachmentAccess::ReadWrite),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("source-owned binding: {error}"));
    assert!(matches!(
        handle,
        d2b_resource_runtime::spec_store::EnsureOutcome::Created(_)
    ));

    // An API caller with its own grant creates its own row under its own
    // subject; the two subjects are not interchangeable.
    fixture
        .client
        .authenticated_apply(
            user_principal(),
            None,
            desired("Volume", "other", Vec::new()),
        )
        .await
        .expect("an authorized user mutation is admitted");

    let denied = fixture
        .client
        .authenticated_apply(
            AuthenticatedMutation::new(AuthenticatedIdentity::new(
                AuthoritySubject::named(
                    AuthoritySubjectKind::User,
                    reference("User/stranger"),
                ),
                ResourceUid::parse("00000000-0000-4000-8000-000000000009").unwrap(),
            )),
            None,
            desired("Volume", "third", Vec::new()),
        )
        .await;
    assert!(
        denied.is_err(),
        "a subject no accepted grant binds must be refused"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn transport_or_parent_ownership_cannot_replace_a_missing_caller_permission() {
    let fixture = spawn_fixture().await;
    seed(&fixture).await;

    // AE15: the same ungranted subject is refused identically through every
    // privileged surface, and the Volume that owns the relationship grants
    // nothing to a caller it never bound.
    let accepted = accepted_graph();
    for transport in [
        TransportIdentity::Daemon,
        TransportIdentity::Broker,
        TransportIdentity::ProviderSession,
        TransportIdentity::OperatorConsole,
        TransportIdentity::ComponentSession,
    ] {
        let stranger = AuthenticatedMutation::new(AuthenticatedIdentity::new(
            AuthoritySubject::named(AuthoritySubjectKind::User, reference("User/stranger")),
            ResourceUid::parse("00000000-0000-4000-8000-000000000009").unwrap(),
        ));
        let outcome = GraphAuthority::admit_mutation(
            &GraphMutation::new(
                zone(),
                MutationSubjectEvidence::new(
                    AuthoritySubject::named(
                        AuthoritySubjectKind::User,
                        reference("User/stranger"),
                    ),
                    transport,
                ),
                MutationKind::Create,
                reference("VolumeBinding/vm-state"),
            ),
            &accepted,
        );
        assert!(!outcome.is_admitted(), "transport {transport} widened the decision");

        let result = fixture
            .client
            .authenticated_apply(
                stranger.clone(),
                None,
                desired("VolumeBinding", "vm-state", binding_spec("state", AttachmentAccess::ReadOnly)),
            )
            .await;
        assert!(result.is_err(), "transport {transport} reached the store");
    }

    // Ownership is not a grant: naming the source as the declaring parent
    // without its controller evidence still refuses the relationship.
    let without_controller = fixture
        .client
        .authenticated_apply(
            provider_principal(),
            Some(key("Volume", "data")),
            desired(
                BindingKind::Volume.resource_type(),
                "vm-state",
                binding_spec("state", AttachmentAccess::ReadOnly),
            ),
        )
        .await;
    assert!(
        without_controller.is_err(),
        "an owner reference alone must not create a source-owned binding"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_mismatched_source_owner_a_conflicting_slot_and_a_direct_binding_are_refused() {
    let fixture = spawn_fixture().await;
    seed(&fixture).await;

    // A mismatched source owner: the evidence names a controller that is not
    // the source that would own the relationship.
    let mismatched = fixture
        .client
        .authenticated_apply(
            volume_controller().with_source_controller(AuthenticatedIdentity::new(
                AuthoritySubject::named(
                    source_controller_kind("Volume").unwrap(),
                    reference("Volume/other"),
                ),
                uid_of(&key("Volume", "other")),
            )),
            Some(key("Volume", "data")),
            desired(
                BindingKind::Volume.resource_type(),
                "vm-state",
                binding_spec("state", AttachmentAccess::ReadWrite),
            ),
        )
        .await;
    assert!(
        mismatched.is_err(),
        "a controller that does not own the source must be refused"
    );

    // Direct unauthenticated binding creation: no declaring source at all.
    let direct = fixture
        .client
        .authenticated_apply(
            provider_principal(),
            None,
            desired(
                BindingKind::Volume.resource_type(),
                "vm-state",
                binding_spec("state", AttachmentAccess::ReadWrite),
            ),
        )
        .await;
    assert!(
        direct.is_err(),
        "a source-owned binding may only be created by its source controller"
    );

    // The admitted relationship, then a conflicting second declaration for the
    // same live consumer slot.
    fixture
        .endpoint
        .ensure_source_owned_binding(
            &key("Volume", "data"),
            volume_controller(),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: d2b_resource_runtime::identity::ResourceTypeName::new(
                    BindingKind::Volume.resource_type(),
                ),
                name: "vm-state".to_owned(),
                spec: binding_spec("state", AttachmentAccess::ReadOnly),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
            },
        )
        .await
        .expect("the source controller admits its own request");
    let conflicting = fixture
        .endpoint
        .ensure_source_owned_binding(
            &key("Volume", "data"),
            volume_controller(),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: d2b_resource_runtime::identity::ResourceTypeName::new(
                    BindingKind::Volume.resource_type(),
                ),
                name: "vm-state-2".to_owned(),
                spec: binding_spec("state", AttachmentAccess::ReadWrite),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
            },
        )
        .await;
    assert!(
        conflicting.is_err(),
        "a second, differently-shaped declaration for one live consumer slot must be refused"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_relation_index_rebuilds_identically_from_committed_rows_after_restart() {
    let fixture = spawn_fixture().await;
    seed(&fixture).await;
    fixture
        .endpoint
        .ensure_source_owned_binding(
            &key("Volume", "data"),
            volume_controller(),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: d2b_resource_runtime::identity::ResourceTypeName::new(
                    BindingKind::Volume.resource_type(),
                ),
                name: "vm-state".to_owned(),
                spec: binding_spec("state", AttachmentAccess::ReadWrite),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
            },
        )
        .await
        .expect("the source controller admits its own request");

    let before = fixture.client.relations().await.unwrap();
    // R3: all six classes are derived from committed rows, and each stays in
    // its own index.
    assert!(before.ownership_edges().count() > 0, "ownership is derived from the durable owner column");
    assert_eq!(before.consumption_all().count(), 1, "one consumption relationship");
    assert_eq!(before.implementation_all().count(), 0);
    assert!(before.placement_all().count() > 0, "a Guest consumer declares placement");
    assert_eq!(before.authorization_all().count(), 1);
    assert_eq!(before.observation_all().count(), 1);
    assert_eq!(before.unresolved().count(), 0);

    // Restart: a fresh manager over the same committed rows.
    let store = fixture.store.clone().expect("a seeded fixture owns its store");
    drop(fixture);
    let restarted = restart(store).await;
    let after = restarted.client.relations().await.unwrap();
    assert_eq!(
        before, after,
        "the relation index is a pure function of the committed rows"
    );
}

async fn restart(store: Arc<SpecStore>) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut relations = RelationExtractors::new();
    relations.register(Arc::new(BindingRequestRelations)).unwrap();
    relations.register(Arc::new(OperationImplementationRelations)).unwrap();
    relations.register(Arc::new(FixtureRelations)).unwrap();
    let hub = Arc::new(d2b_resource_runtime::watch::WatchHub::new(
        &d2b_resource_runtime::revision::SystemClock,
        d2b_resource_runtime::watch::DEFAULT_RING_CAPACITY,
    ));
    let args = ResourceManagerArgs {
        zone: ZONE.to_owned(),
        store,
        providers: providers(),
        hub,
        admission: Arc::new(GraphMutationAdmission::new(
            Arc::new(accepted_graph()),
            zone(),
            TransportIdentity::Daemon,
        )),
        decoders: Default::default(),
        default_decoder: Arc::new(NoDecoder),
        targets: Arc::new(TargetDirectory::new()),
        host_target: TargetRef::host("test-host").unwrap(),
        target_resolver: Arc::new(HostOnlyResolver),
        backoff: std::time::Duration::from_millis(200),
        relation_extractors: relations,
    };
    let (actor, _join): (ActorRef<ResourceManagerMsg>, _) =
        Actor::spawn(None, ResourceManager::new(), args).await.unwrap();
    Fixture {
        client: ResourceManagerClient::new(actor.clone()),
        endpoint: ManagerActorEndpoint::new(actor),
        store: None,
        _directory: directory,
    }
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_binding_without_an_accepted_source_decision_is_refused_by_the_evaluator() {
    // R16: a well-formed request is not authority. The same canonical request
    // the manager admits is refused by the one evaluator when no accepted
    // source decision backs it.
    let binding = VolumeBindingRequest::new(
        reference("Volume/data"),
        reference("Guest/vm"),
        BindingSlot::parse("state").unwrap(),
        d2b_contracts_resource::v3::BoundedToken::parse("root").unwrap(),
        AttachmentAccess::ReadWrite,
        VolumePresentation::filesystem("/state").unwrap(),
    )
    .unwrap();
    let binding_key = binding
        .key(
            zone(),
            uid_of(&key("Volume", "data")),
            uid_of(&key("Guest", "vm")),
        )
        .unwrap();
    let request = BindingAdmissionRequest::new(
        binding_key.clone(),
        binding.requested_rights(),
        binding.required_facets().to_vec(),
        BindingAuthorization::granted(),
        vec![freshness(&key("Volume", "data"))],
    );
    assert!(
        GraphAuthority::admit_binding(request.clone(), &accepted_graph()).is_err(),
        "no accepted source decision means no admitted binding"
    );

    let accepted = accepted_graph().with_source(AcceptedSource::new(
        d2b_contracts_resource::v3::SourceAdmission::new(
            binding_key.clone(),
            vec![d2b_contracts_resource::v3::RequestedRights::Mutate],
            d2b_contracts_resource::v3::BindingArbitration::Shared,
        )
        .unwrap(),
        d2b_contracts_resource::v3::BindingRealizationSupport::new(vec![
            d2b_contracts_resource::v3::BindingRealizationFacet::FilesystemPresentation,
        ])
        .unwrap(),
    ));
    let admission = GraphAuthority::admit_binding(request, &accepted)
        .expect("an accepted source decision admits the exact request");
    assert!(admission.is_current(&[freshness(&key("Volume", "data"))]));
}