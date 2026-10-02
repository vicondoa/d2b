//! New-graph limit and emergency enforcement (U40, KTD6-KTD10; R8, R36, R49).
//!
//! These cases drive the construction U34 installs in production: the manager
//! calls the plane's injected [`GraphLimitsAdmission`], which decides who is
//! asking through the one shared evaluator, then asks the two owning
//! providers whether the candidate fits the Zone's accepted ceilings and
//! whether an accepted emergency reduction admits new use. The unchanged
//! production entry points (the graph admission plus the string-subject
//! messages) are deliberately untouched and are not exercised here.
//!
//! Three properties are proven, each against observed state rather than a
//! helper's internals:
//!
//! 1. An over-quota mutation leaves *nothing* behind. The manager's durable
//!    store holds no row for the refused key and no driver was ever asked for
//!    it, which is the difference between a limit and a report: a quota
//!    checked after the commit would leave a desired row to reconcile.
//! 2. An emergency reduction blocks new use and drives the use that already
//!    exists to its safe state in the order KTD10 requires - the consumer
//!    detaches while its helper legs still exist, and the source reservation
//!    is released last.
//! 3. A broker that cannot answer leaves the Zone fenced with conservative
//!    ownership: the reduction still blocks new use, nothing is reported as
//!    drained, and no reservation may be released.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2b_contracts_resource::v3::{
    AdmissionDecision as GraphDecision, AdmissionStage, AuthoritySubject, AuthoritySubjectKind,
    RefusalReason, ResourceRef, ResourceUid, ResourceTypeName, StoreIncarnation,
};
use d2b_contracts_zone_session::v3::role::AuthorizedRole;
use d2b_contracts_zone_session::v3::{
    EmergencyPolicySpec, EmergencyScope, RoleBindingSpec, RoleResourceVerb, RoleRule,
};
use d2b_core::resource_authority::{AcceptedGraph, GraphAuthority, TransportIdentity};
use d2b_provider_emergency_policy::{
    DrainStep, EmergencyReduction, EnforcementState, OpenUse, OpenUseCensus, plan_drain,
};
use d2b_provider_quota::quota::{
    QuotaCeilings, QuotaEnforcementPolicy, QuotaPolicy, ZoneBudget, ZoneUsage,
};
use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::manager::{
    AdmissionOp, AuthenticatedIdentity, AuthenticatedMutation, DesiredResource, MutationAdmission,
    MutationRequest, MutationSubject, ResourceManager, ResourceManagerArgs, ResourceManagerClient,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::relations::RelationExtractors;
use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance, SpecSelector, SpecStore};
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2bd::{AcceptedLimits, GraphLimitsAdmission, GraphMutationAdmission};

const ZONE: &str = "limits-and-emergency";
const STORE: &str = "store-generation-1";
const ROLE: &str = "Role/operator";
const BINDING: &str = "RoleBinding/operators";
const QUOTA: &str = "Quota/zone";
const EMERGENCY: &str = "EmergencyPolicy/zone";

/// Every type this fixture's operator may create or delete.
const TYPES: [&str; 4] = ["User", "Volume", "Guest", "Process"];

fn zone() -> d2b_contracts_resource::v3::ZoneId {
    d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("the fixture zone is canonical")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture reference is canonical")
}

fn type_of(value: &str) -> ResourceTypeName {
    ResourceTypeName::parse(value).expect("a registered resource type")
}

fn uid_of(key: &ResourceKey) -> ResourceUid {
    ResourceUid::from_bytes(&d2b_resource_runtime::manager::deterministic_uid(key))
        .expect("a manager row uid is a canonical uuid")
}

/// The prior accepted graph: the operator may create and delete every type
/// this fixture uses, so each assertion below observes a limit decision and
/// not an authorization one.
fn accepted_graph() -> AcceptedGraph {
    let mut types: Vec<ResourceTypeName> = TYPES
        .iter()
        .map(|type_name| type_of(type_name))
        .collect();
    types.push(type_of("Quota"));
    types.push(type_of("EmergencyPolicy"));
    types.push(type_of("Role"));
    types.push(type_of("RoleBinding"));
    let role = AuthorizedRole::new(
        vec![RoleRule::new(
            types,
            vec![RoleResourceVerb::Create, RoleResourceVerb::Delete],
            Vec::new(),
            Vec::new(),
            vec![zone()],
            Vec::new(),
            Vec::new(),
        )
        .expect("the role rule validates")],
        Vec::new(),
    )
    .expect("the authorization-only role validates");
    let binding =
        RoleBindingSpec::new(reference(ROLE), vec![reference("User/operator")], None, None)
            .expect("the role binding validates");
    AcceptedGraph::new(
        zone(),
        StoreIncarnation::parse(STORE).expect("a bounded token"),
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
    )
    .with_role(reference(ROLE), role)
    .with_role_binding(reference(BINDING), binding)
}

/// A hard ceiling of `max_resources` rows, read from the Zone's own `Quota`
/// row.
fn quota(max_resources: u32) -> QuotaPolicy {
    QuotaPolicy::new(
        QuotaCeilings::new(max_resources, max_resources, 4, None, None, None)
            .expect("ceilings are in range"),
        BTreeMap::new(),
        QuotaEnforcementPolicy::Hard,
    )
    .expect("the policy validates")
    .bind_to(reference(QUOTA))
}

/// A reduction that blocks new use and drains what is already running.
fn reduction() -> EmergencyReduction {
    EmergencyReduction::of(&[EmergencyPolicySpec::new(
        true,
        EmergencyScope::new(true, false, false, true),
        30,
        "operator reduction",
    )
    .expect("the policy validates")])
    .bind_to(reference(EMERGENCY))
}

fn admitted_graph() -> GraphMutationAdmission {
    GraphMutationAdmission::new(
        Arc::new(accepted_graph()),
        zone(),
        TransportIdentity::ComponentSession,
    )
}

/// A driver that realizes nothing.
///
/// The limits under test are decided before any row exists, so a driver that
/// holds no privilege is exactly what keeps the fixture honest: a row that
/// spawned with authority would prove nothing about the refusal that stopped
/// it.
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

/// A factory that records every key it was asked to realize.
///
/// The record is the observable half of "no effect grant": a refused mutation
/// that had already spawned an actor would show up here even if the row itself
/// were cleaned up afterwards.
struct RecordingFactory {
    created: Arc<tokio::sync::Mutex<Vec<ResourceKey>>>,
}

#[async_trait::async_trait]
impl d2b_resource_runtime::driver::ResourceDriverFactory for RecordingFactory {
    fn resource_types(&self) -> &[d2b_resource_runtime::identity::ResourceTypeName] {
        fixture_types()
    }

    async fn create(
        &self,
        key: &ResourceKey,
    ) -> Box<dyn d2b_resource_runtime::driver::DynResourceDriver> {
        self.created.lock().await.push(key.clone());
        Box::new(InertDriver)
    }
}

/// The types this fixture's factory serves, built once per process.
fn fixture_types() -> &'static [d2b_resource_runtime::identity::ResourceTypeName] {
    static TYPES: std::sync::LazyLock<Vec<d2b_resource_runtime::identity::ResourceTypeName>> =
        std::sync::LazyLock::new(|| {
            ["User", "Volume", "Guest", "Process", "Quota", "EmergencyPolicy"]
                .into_iter()
                .map(d2b_resource_runtime::identity::ResourceTypeName::new)
                .collect()
        });
    &TYPES
}

fn providers(created: Arc<tokio::sync::Mutex<Vec<ResourceKey>>>) -> ProviderDirectory {
    let mut directory = ProviderDirectory::new();
    directory
        .register(
            Arc::new(RecordingFactory { created })
                as Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory>,
        )
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
    store: Arc<SpecStore>,
    created: Arc<tokio::sync::Mutex<Vec<ResourceKey>>>,
    _directory: tempfile::TempDir,
}

async fn spawn_fixture(limits: AcceptedLimits) -> Fixture {
    let directory = tempfile::tempdir().expect("a temporary store directory");
    let store = Arc::new(SpecStore::open(directory.path().join("specs.sqlite")).expect("store"));
    let created = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let args = ResourceManagerArgs {
        zone: ZONE.to_owned(),
        store: store.clone(),
        // No broker in this fixture: the recording publisher fences and
        // accepts what the manager publishes.
        authority: d2b_resource_runtime::test_support::RecordingPublisher::new(),
        providers: providers(Arc::clone(&created)),
        hub: Arc::new(d2b_resource_runtime::watch::WatchHub::new(
            &d2b_resource_runtime::revision::SystemClock,
            d2b_resource_runtime::watch::DEFAULT_RING_CAPACITY,
        )),
        admission: Arc::new(GraphLimitsAdmission::new(
            Arc::new(admitted_graph()),
            Arc::new(limits),
        )),
        decoders: Default::default(),
        default_decoder: Arc::new(NoDecoder),
        targets: Arc::new(TargetDirectory::new()),
        host_target: TargetRef::host("test-host").expect("the host target"),
        target_resolver: Arc::new(HostOnlyResolver),
        backoff: std::time::Duration::from_millis(200),
        relation_extractors: RelationExtractors::new(),
    };
    let (actor, _join) = ractor::Actor::spawn(None, ResourceManager::new(), args)
        .await
        .expect("the manager actor starts");
    Fixture {
        client: ResourceManagerClient::new(actor),
        store,
        created,
        _directory: directory,
    }
}

fn key(type_name: &str, name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, type_name, name)
}

fn desired(type_name: &str, name: &str) -> DesiredResource {
    DesiredResource {
        key: key(type_name, name),
        spec: Vec::new(),
        metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
        provenance: ResourceProvenance::Api,
    }
}

/// The verified deployment graph's own authority: it seeds the initial rows.
fn bootstrap() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        ResourceUid::parse("00000000-0000-4000-8000-000000000000")
            .expect("a canonical uuid"),
    ))
}

fn operator() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::named(AuthoritySubjectKind::User, reference("User/operator")),
        uid_of(&key("User", "operator")),
    ))
}

/// The census the committed rows yield.
fn census(rows: &[&str]) -> ZoneUsage {
    let counted: Vec<(ResourceRef, ResourceUid, Option<ResourceUid>)> = rows
        .iter()
        .map(|row| {
            let (type_name, name) = row.split_once('/').expect("a `Type/name` fixture row");
            (reference(row), uid_of(&key(type_name, name)), None)
        })
        .collect();
    ZoneUsage::census(&counted).expect("the committed rows form a decidable census")
}

async fn seed(fixture: &Fixture, rows: &[(&str, &str)]) {
    for (type_name, name) in rows {
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired(type_name, name))
            .await
            .unwrap_or_else(|error| panic!("seed {type_name}/{name}: {error}"));
    }
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_over_quota_mutation_leaves_no_desired_row_and_no_effect_grant() {
    let rows = [("User", "operator"), ("Volume", "data")];
    let limits =
        AcceptedLimits::new(Some(quota(2)), EmergencyReduction::NONE).bind(census(&["User/operator", "Volume/data"]));
    let fixture = spawn_fixture(limits).await;
    seed(&fixture, &rows).await;

    // The Zone is at its ceiling, so the next row is refused at admission.
    let refused = fixture
        .client
        .authenticated_apply(operator(), None, desired("Guest", "vm"))
        .await;
    assert!(
        refused.is_err(),
        "a third row against a two-row ceiling is refused"
    );

    // The refusal is not a post-commit rejection: the durable store holds no
    // row for the refused key.
    assert!(
        fixture.store.get(key("Guest", "vm")).await.is_err(),
        "a refused mutation leaves no desired row to reconcile or clean up"
    );
    let remaining = fixture
        .store
        .list(SpecSelector {
            zone: Some(ZONE.to_owned()),
            type_name: Some("Guest".to_owned()),
            owner_uid: None,
        })
        .await
        .expect("the store lists its rows");
    assert!(remaining.is_empty(), "the Zone has no Guest row at all");

    // And no effect was ever granted: no actor was spawned for it. The two
    // seeded rows are the control - they were admitted, so the factory was
    // reached for them, and the refused key is absent from the same record.
    let created = fixture.created.lock().await.clone();
    assert!(
        created.contains(&key("Volume", "data")),
        "an admitted row reaches its driver, so the record is live: {created:?}"
    );
    assert!(
        !created.contains(&key("Guest", "vm")),
        "a refused mutation grants no effect: {created:?}"
    );

    // A delete is never what a ceiling refuses, so the Zone can recover.
    fixture
        .client
        .authenticated_apply(operator(), None, desired("Volume", "data"))
        .await
        .ok();
    let remove = fixture
        .client
        .authenticated_remove(operator(), key("Volume", "data"))
        .await;
    assert!(remove.is_ok(), "a Zone must always be able to free its own ceiling");
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_emergency_reduction_blocks_new_use_and_drains_existing_use_in_order() {
    let rows = [("User", "operator"), ("Volume", "data")];
    let limits = AcceptedLimits::new(Some(quota(8)), reduction())
        .bind(census(&["User/operator", "Volume/data"]));
    let fixture = spawn_fixture(limits).await;
    seed(&fixture, &rows).await;
    let admission = GraphLimitsAdmission::new(
        Arc::new(admitted_graph()),
        Arc::new(AcceptedLimits::new(Some(quota(8)), reduction())),
    );

    // New use is refused at the revoking stage, with the reduction's own
    // reason, at the mutation seam and at the effect boundary alike.
    assert!(fixture
        .client
        .authenticated_apply(operator(), None, desired("Guest", "vm"))
        .await
        .is_err());
    assert!(fixture.store.get(key("Guest", "vm")).await.is_err());
    assert_eq!(
        admission.admit_new_use(),
        GraphDecision::Refused {
            stage: AdmissionStage::Revoke,
            reason: RefusalReason::EmergencyReductionActive,
        }
    );

    // The reduction's own row stays writable, or the Zone could never clear
    // the emergency that is blocking it.
    assert!(
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired("EmergencyPolicy", "zone"))
            .await
            .is_ok(),
        "an operator must always be able to change or clear the reduction"
    );

    // The use that already exists is driven to its safe state in KTD10's
    // order: fence, detach the consumer while its helper legs still exist,
    // then finalize the helpers, then release the source.
    let census = OpenUseCensus::new(
        [reference("Volume/data")],
        0,
        0,
        [OpenUse::new(
            reference("Volume/data"),
            reference("Guest/vm"),
            [reference("Endpoint/virtiofsd")],
        )],
    );
    let plan = admission.drain(Some(&census));
    assert_eq!(plan.state(), EnforcementState::Pending);
    assert!(!plan.may_release(), "outstanding use is still held");
    let drain = &plan.drains()[0];
    assert_eq!(
        drain.steps(),
        [
            DrainStep::FenceNewUse,
            DrainStep::DetachConsumer,
            DrainStep::FinalizeHelper(reference("Endpoint/virtiofsd")),
            DrainStep::ReleaseSource,
        ],
        "the consumer detaches while its helper leg still exists, and the source is released last"
    );
    assert_eq!(
        plan.deadline_seconds(),
        30,
        "the tightest admitted deadline drives the drain"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_broker_outage_leaves_enforcement_fenced_rather_than_converged() {
    let reduction = reduction();
    let settled = OpenUseCensus::new([reference("Volume/data")], 0, 0, []);

    // With an answerable census and nothing outstanding, the reduction has
    // converged and a reservation may be released.
    let converged = plan_drain(&reduction, Some(&settled));
    assert_eq!(converged.state(), EnforcementState::Converged);
    assert!(converged.may_release());

    // The same reduction with no answer at all is not a reduction with
    // nothing left to do: it is a Zone whose open use is unknown.
    let fenced = plan_drain(&reduction, None);
    assert_eq!(fenced.state(), EnforcementState::Fenced);
    assert!(!fenced.is_converged(), "an unreachable broker is not an empty Zone");
    assert!(!fenced.may_release(), "nothing may be released on a guess");
    assert_eq!(
        fenced.new_use(),
        d2b_provider_emergency_policy::NewUseState::Blocked,
        "the reduction still holds while the Zone is fenced"
    );
    assert!(fenced.drains().is_empty(), "no drain is reported as done");

    // An open use whose source the census does not name is equally undecidable.
    let dangling = OpenUseCensus::new(
        [reference("Volume/data")],
        0,
        0,
        [OpenUse::new(
            reference("Device/gpu"),
            reference("Guest/vm"),
            [reference("Endpoint/virtiofsd")],
        )],
    );
    let undecidable = plan_drain(&reduction, Some(&dangling));
    assert_eq!(undecidable.state(), EnforcementState::Fenced);
    assert!(!undecidable.may_release());
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_typed_budget_is_refused_where_it_is_declared_and_a_silent_one_is_not() {
    let policy = QuotaPolicy::new(
        QuotaCeilings::new(8, 8, 4, Some(1), None, None).expect("ceilings are in range"),
        BTreeMap::new(),
        QuotaEnforcementPolicy::Hard,
    )
    .expect("the policy validates")
    .bind_to(reference(QUOTA));
    let usage = ZoneUsage::default()
        .with_budget(ZoneBudget { cpu: 1, memory_mib: 0, storage_gib: 0 })
        .expect("the budget is representable");
    let admission = GraphLimitsAdmission::new(
        Arc::new(admitted_graph()),
        Arc::new(AcceptedLimits::new(Some(policy), EmergencyReduction::NONE).bind(usage)),
    );
    assert_eq!(
        admission.admit_budget(
            &reference("Process/job"),
            ZoneBudget { cpu: 1, memory_mib: 0, storage_gib: 0 },
            1
        ),
        GraphDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling),
        "a Zone at its CPU ceiling admits no further CPU"
    );
    assert_eq!(
        admission.admit_budget(&reference("Process/job"), ZoneBudget::ZERO, 1),
        GraphDecision::Admitted
    );
}

#[test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn a_limit_is_not_a_way_to_see_an_authorization_failure() {
    // Both limits would refuse this candidate: the Zone is at its ceiling and
    // its emergency reduction is in force. The refusal that surfaces is the
    // shared evaluator's, so a caller learns that the subject is not
    // authorized and nothing about which ceiling or flag stopped it.
    let admission = GraphLimitsAdmission::new(
        Arc::new(admitted_graph()),
        Arc::new(AcceptedLimits::new(Some(quota(2)), reduction()).bind(census(&["Guest/vm"]))),
    );
    let stranger = MutationSubject {
        principal: reference("User/stranger").to_canonical_string(),
        origin: ResourceProvenance::Api,
    };
    let denied = admission.admit(
        &stranger,
        &MutationRequest {
            key: key("Guest", "second"),
            op: AdmissionOp::Ensure,
            spec: Vec::new(),
            metadata: br#"{"ownerRef":null}"#.to_vec(),
        },
    );
    let d2b_resource_runtime::manager::AdmissionDecision::Deny(reason) = denied else {
        panic!("an unauthorized subject is refused whatever the limits say");
    };
    assert!(reason.contains("identity-not-authorized"), "{reason}");
    assert!(!reason.contains("emergency"), "{reason}");
    assert!(!reason.contains("limit-exceeds"), "{reason}");

    // The shared evaluator's own answer for the same two subjects is the one
    // the composition deferred to, with no limit vocabulary anywhere in it.
    let accepted = accepted_graph();
    let mutation = |principal: &str| {
        d2b_core::resource_authority::GraphMutation::new(
            zone(),
            d2b_core::resource_authority::MutationSubjectEvidence::new(
                AuthoritySubject::named(AuthoritySubjectKind::User, reference(principal)),
                TransportIdentity::ComponentSession,
            ),
            d2b_core::resource_authority::MutationKind::Create,
            reference("Guest/second"),
        )
    };
    assert_eq!(
        GraphAuthority::admit_mutation(&mutation("User/operator"), &accepted),
        GraphDecision::Admitted
    );
    assert_eq!(
        GraphAuthority::admit_mutation(&mutation("User/stranger"), &accepted),
        GraphDecision::Refused {
            stage: AdmissionStage::Authorize,
            reason: RefusalReason::IdentityNotAuthorized,
        }
    );
}
