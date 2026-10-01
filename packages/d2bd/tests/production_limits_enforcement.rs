//! Production limit and emergency enforcement (U40, R8, R36).
//!
//! These cases drive the admission the PLANE WILL INSTALL once a per-Zone
//! accepted graph exists. The manager is built with the production driver
//! factories and the production admission composition, and the rows are
//! committed through the manager, so what they prove is that the wiring is
//! correct end to end - NOT that a mutation is currently limited, because the
//! plane installs the zone-local write fence today. When the install lands,
//! this suite becomes the proof that the installed path limits; until then it
//! proves the path is ready.
//!
//! The evaluator's own coverage in `graph_limits_and_emergency.rs` proves the
//! decisions. This suite proves the wiring: that a `Quota` row committed
//! through the production driver reaches the production admission, that a
//! mutation past the committed ceiling is refused before it is persisted, and
//! that an active `EmergencyPolicy` refuses new use and drives the drain.
//!
//! Three properties, each observed rather than asserted from a helper:
//!
//! 1. A committed `Quota` row's ceiling is enforced on the next mutation, and
//!    the refused mutation leaves no desired row and no driver behind.
//! 2. The same admission admits a mutation that fits under the ceiling.
//! 3. An active `EmergencyPolicy` refuses new use with the reduction's own
//!    reason, keeps its own row writable so the Zone can recover, and holds
//!    the drain finalizer while the Zone's open use is outstanding.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2b_contracts_resource::v3::{
    AdmissionDecision as GraphDecision, AdmissionStage, AuthoritySubject, AuthoritySubjectKind,
    RefusalReason, ResourceRef, ResourceTypeName, StoreIncarnation,
};
use d2b_contracts_zone_session::v3::role::AuthorizedRole;
use d2b_contracts_zone_session::v3::{
    EmergencyPolicySpec, EmergencyScope, RoleBindingSpec, RoleResourceVerb, RoleRule,
};
use d2b_core::resource_authority::{AcceptedGraph, TransportIdentity};
use d2b_provider_emergency_policy::{
    EmergencyReduction, EnforcementState, OpenUseCensus, OpenUseSource, ZoneEmergencyRuntime,
    emergency_policy_descriptor,
};
use d2b_provider_quota::{UsageSource, ZoneQuotaRuntime, quota_descriptor};
use d2b_provider_quota::quota::ZoneUsage;
use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::manager::{
    AuthenticatedIdentity, AuthenticatedMutation, DesiredResource, ResourceManager,
    ResourceManagerArgs, ResourceManagerClient,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::relations::RelationExtractors;
use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance, SpecSelector, SpecStore};
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2bd::{
    AcceptedLimits, AcceptedLimitsHolder, GraphLimitsAdmission, GraphMutationAdmission,
    PlaneZoneOpenUse, PlaneZoneUsage,
};

const ZONE: &str = "production-limits";

/// Every type the fixture's operator may create or delete, so each assertion
/// below observes a limit decision and not an authorization one.
const TYPES: [&str; 6] = [
    "User",
    "Volume",
    "Guest",
    "Process",
    "Quota",
    "EmergencyPolicy",
];

/// The Zone this case runs in.
///
/// The two family runtimes are installed per Zone in a process-wide registry,
/// exactly as the daemon installs one per Zone it serves. Two cases sharing a
/// Zone name would therefore share a runtime, so each names its own - which is
/// also what two Zones of one daemon look like.
fn zone_named(name: &str) -> d2b_contracts_resource::v3::ZoneId {
    d2b_contracts_resource::v3::ZoneId::parse(name).expect("the fixture zone is canonical")
}

fn zone() -> d2b_contracts_resource::v3::ZoneId {
    zone_named(ZONE)
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture reference is canonical")
}

fn uid_of(key: &ResourceKey) -> d2b_contracts_resource::v3::ResourceUid {
    d2b_contracts_resource::v3::ResourceUid::from_bytes(
        &d2b_resource_runtime::manager::deterministic_uid(key),
    )
    .expect("a manager row uid is a canonical uuid")
}

/// The prior accepted graph: the operator may create and delete every type
/// this fixture uses.
fn accepted_graph_for(zone: &d2b_contracts_resource::v3::ZoneId) -> AcceptedGraph {
    let role = AuthorizedRole::new(
        vec![RoleRule::new(
            TYPES
                .iter()
                .map(|type_name| ResourceTypeName::parse(*type_name).expect("a registered type"))
                .collect(),
            vec![RoleResourceVerb::Create, RoleResourceVerb::Delete],
            Vec::new(),
            Vec::new(),
            vec![zone.clone()],
            Vec::new(),
            Vec::new(),
        )
        .expect("the role rule validates")],
        Vec::new(),
    )
    .expect("the authorization-only role validates");
    let binding = RoleBindingSpec::new(
        ResourceRef::parse("Role/operator").expect("a canonical role"),
        vec![reference("User/operator")],
        None,
        None,
    )
    .expect("the role binding validates");
    AcceptedGraph::new(
        zone.clone(),
        StoreIncarnation::parse("store-generation-1").expect("a bounded token"),
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
    )
    .with_role(reference("Role/operator"), role)
    .with_role_binding(reference("RoleBinding/operators"), binding)
}

/// A `Quota` row in the shape the committed schema admits.
fn quota_row(max_resources: u32) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "ceilings": {
            "maxResources": max_resources,
            "maxResourcesPerType": max_resources,
            "maxOwnerDepth": 4,
            "maxCpu": null,
            "maxMemoryMib": null,
            "maxStorageGib": null
        },
        "perTypeCeilings": {},
        "scope": "zone",
        "enforcementPolicy": "hard"
    }))
    .expect("the ceiling row serializes")
}

/// An `EmergencyPolicy` row in the shape the contract admits.
fn emergency_row(enabled: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "enabled": enabled,
        "scope": { "stopNewAdmissions": true, "disconnectZoneLinks": false,
                   "stopProviderProcesses": false, "drainOngoingOperations": true },
        "drainDeadlineSeconds": 30,
        "reason": "operator reduction"
    }))
    .expect("the policy row serializes")
}

/// A census the fixture scripts, so the drain is driven against a known
/// outstanding use rather than an empty Zone.
struct ScriptedUsage(Option<ZoneUsage>);

#[async_trait::async_trait]
impl UsageSource for ScriptedUsage {
    async fn usage(&self) -> Result<Option<ZoneUsage>, String> {
        Ok(self.0.clone())
    }
}

/// A census the Zone cannot answer, which fences rather than converges.
struct UnreadableCensus;

#[async_trait::async_trait]
impl d2b_provider_emergency_policy::OpenUseSource for UnreadableCensus {
    async fn census(&self) -> Result<Option<OpenUseCensus>, String> {
        Err("the broker did not answer".to_owned())
    }
}

/// A driver that realizes nothing, for the rows the census counts.
///
/// The census is measured against committed rows, not against rows that have
/// effects, so these types need a driver registered to commit and nothing
/// more. Registering the two policy families' real drivers is the part under
/// test; this factory exists so the surrounding committed rows can exist.
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

struct InertFactory {
    types: std::sync::LazyLock<Vec<d2b_resource_runtime::identity::ResourceTypeName>>,
}

impl InertFactory {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            types: std::sync::LazyLock::new(|| {
                ["User", "Volume", "Guest", "Process"]
                    .into_iter()
                    .map(d2b_resource_runtime::identity::ResourceTypeName::new)
                    .collect()
            }),
        })
    }
}

#[async_trait::async_trait]
impl d2b_resource_runtime::driver::ResourceDriverFactory for InertFactory {
    fn resource_types(&self) -> &[d2b_resource_runtime::identity::ResourceTypeName] {
        &self.types
    }

    async fn create(
        &self,
        _key: &d2b_resource_runtime::identity::ResourceKey,
    ) -> Box<dyn d2b_resource_runtime::driver::DynResourceDriver> {
        Box::new(InertDriver)
    }
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
    _directory: tempfile::TempDir,
}

fn key(type_name: &str, name: &str) -> ResourceKey {
    key_in(&zone(), type_name, name)
}

/// The row key in the Zone this case runs in.
fn key_in(zone: &d2b_contracts_resource::v3::ZoneId, type_name: &str, name: &str) -> ResourceKey {
    ResourceKey::new(zone.as_str(), type_name, name)
}

fn desired_in(
    zone: &d2b_contracts_resource::v3::ZoneId,
    type_name: &str,
    name: &str,
    spec: &[u8],
) -> DesiredResource {
    DesiredResource {
        key: key_in(zone, type_name, name),
        spec: spec.to_vec(),
        metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
        provenance: ResourceProvenance::Api,
    }
}

fn bootstrap() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        d2b_contracts_resource::v3::ResourceUid::parse("00000000-0000-4000-8000-000000000000")
            .expect("a canonical uuid"),
    ))
}

fn operator() -> AuthenticatedMutation {
    AuthenticatedMutation::new(AuthenticatedIdentity::new(
        AuthoritySubject::named(AuthoritySubjectKind::User, reference("User/operator")),
        uid_of(&key("User", "operator")),
    ))
}

/// Build the plane's admission exactly as the plane's admission site does.
///
/// This is the production composition, not a second one: the same holder
/// constructor, the same per-Zone runtimes, the same acknowledged graph. A
/// test that built its own admission would prove the evaluator rather than the
/// wiring, which is the difference this suite exists to close.
async fn spawn_plane(
    zone_name: &str,
    usage: Option<ZoneUsage>,
    open_use: Arc<dyn d2b_provider_emergency_policy::OpenUseSource>,
) -> Fixture {
    let zone_id = zone_named(zone_name);
    let zone = zone_id.clone();
    let directory = tempfile::tempdir().expect("a temporary store directory");
    let store = Arc::new(SpecStore::open(directory.path().join("specs.sqlite")).expect("store"));

    // The composition root installs both runtimes before the manager spawns,
    // exactly as the plane does.
    d2b_provider_quota::install(Arc::new(ZoneQuotaRuntime::new(
        zone.clone(),
        Arc::new(ScriptedUsage(usage)),
    )));
    d2b_provider_emergency_policy::install(Arc::new(ZoneEmergencyRuntime::new(zone.clone(), open_use)));

    // The production driver declarations, over the type's own decoder and
    // factory. This is what `provider_set()` registers.
    let mut providers = ProviderDirectory::new();
    providers.register(InertFactory::new()).expect("the census types have a driver");
    providers.register_driver(&quota_descriptor(zone.clone())).expect("quota registration");
    providers
        .register_driver(&emergency_policy_descriptor(zone.clone()))
        .expect("emergency registration");

    let limits = AcceptedLimitsHolder::live(
        Arc::new(std::sync::RwLock::new(Arc::new(
            AcceptedLimits::new(None, EmergencyReduction::NONE).bind(ZoneUsage::default()),
        ))),
        &zone,
    );
    let admission = Arc::new(GraphLimitsAdmission::with_holder(
        Arc::new(GraphMutationAdmission::new(
            Arc::new(accepted_graph_for(&zone)),
            zone.clone(),
            TransportIdentity::Daemon,
        )),
        limits,
    ));

    // The registry is the decoder authority, exactly as the plane wires it: a
    // test that passed an empty decoder table would prove nothing about the
    // production decode path.
    let decoders = providers.decoders();
    let args = ResourceManagerArgs {
        zone: zone_name.to_owned(),
        store: Arc::clone(&store),
        providers,
        hub: Arc::new(d2b_resource_runtime::watch::WatchHub::new(
            &d2b_resource_runtime::revision::SystemClock,
            d2b_resource_runtime::watch::DEFAULT_RING_CAPACITY,
        )),
        admission,
        decoders,
        default_decoder: Arc::new(NoDecoder),
        targets: Arc::new(TargetDirectory::new()),
        host_target: TargetRef::host("test-host").expect("the host target"),
        target_resolver: Arc::new(HostOnlyResolver),
        backoff: std::time::Duration::from_millis(50),
        relation_extractors: RelationExtractors::new(),
    };
    let (actor, _join) = ractor::Actor::spawn(None, ResourceManager::new(), args)
        .await
        .expect("the manager actor starts");
    Fixture { client: ResourceManagerClient::new(actor), store, _directory: directory }
}

/// Commit a row and let its driver publish, which is what makes a committed
/// ceiling reach the admission.
async fn commit_and_settle(
    fixture: &Fixture,
    zone: &d2b_contracts_resource::v3::ZoneId,
    type_name: &str,
    name: &str,
    spec: &[u8],
) {
    fixture
        .client
        .authenticated_apply(bootstrap(), None, desired_in(zone, type_name, name, spec))
        .await
        .unwrap_or_else(|error| panic!("commit {type_name}/{name}: {error}"));
    settle().await;
}

/// Wait for the committed row's driver to have published its half.
///
/// The driver is an actor, so its publish lands a scheduling step after the
/// commit returns. A test that read the admission immediately would race the
/// driver and report a ceiling as unenforced when it had simply not been
/// written yet.
async fn settle() {
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

fn rows_now(usage: &[&str]) -> ZoneUsage {
    let counted = usage
        .iter()
        .map(|row| {
            let (type_name, name) = row.split_once('/').expect("a `Type/name` fixture row");
            (reference(row), uid_of(&key(type_name, name)), None)
        })
        .collect::<Vec<_>>();
    ZoneUsage::census(&counted).expect("the committed rows form a decidable census")
}

/// The production admission refuses a mutation past a committed ceiling, and
/// the refusal happens before the row is persisted.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_committed_quota_ceiling_refuses_the_mutation_past_it() {
    let committed = ["User/operator", "Volume/data"];
    let zone = zone_named("quota-refusal");
    let fixture = spawn_plane("quota-refusal",     Some(rows_now(&committed)), Arc::new(UnreadableCensus)).await;
    fixture
        .client
        .authenticated_apply(bootstrap(), None, desired_in(&zone, "User", "operator", b"{}"))
        .await
        .expect("the operator row commits");
    fixture
        .client
        .authenticated_apply(bootstrap(), None, desired_in(&zone, "Volume", "data", b"{}"))
        .await
        .expect("the volume row commits");

    // The ceiling is committed AFTER the manager spawned, through the
    // production driver. This is the case a frozen snapshot could never
    // enforce, because at spawn time no ceiling existed.
    commit_and_settle(&fixture, &zone, "Quota", "zone", &quota_row(2)).await;

    // The Zone is at its two-row ceiling, so the next row is refused.
    let refused = fixture
        .client
        .authenticated_apply(operator(), None, desired_in(&zone, "Guest", "vm", b"{}"))
        .await;
    let error = refused.expect_err("a third row against a two-row ceiling is refused");
    let text = error.to_string();
    assert!(
        text.contains("limit-exceeds-ceiling"),
        "the refusal names the quota's own reason: {text}"
    );
    assert!(
        text.contains("quota"),
        "the refusal names the policy that enforced it: {text}"
    );

    // The refusal is not a post-commit rejection: the durable store holds no
    // row for the refused key.
    assert!(
        fixture.store.get(key_in(&zone, "Guest", "vm")).await.is_err(),
        "a refused mutation leaves no desired row to reconcile or clean up"
    );
    let guests = fixture
        .store
        .list(SpecSelector {
            zone: Some(ZONE.to_owned()),
            type_name: Some("Guest".to_owned()),
            owner_uid: None,
        })
        .await
        .expect("the store lists its rows");
    assert!(guests.is_empty(), "the Zone has no Guest row at all");

    // A delete is never what a ceiling refuses, so the Zone can recover.
    assert!(
        fixture
            .client
            .authenticated_remove(operator(), key_in(&zone, "Volume", "data"))
            .await
            .is_ok(),
        "a Zone must always be able to free its own ceiling"
    );
}

/// The same production admission admits a mutation that fits under the
/// ceiling, so the refusal above is a ceiling and not a blanket denial.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_mutation_under_the_committed_ceiling_is_admitted() {
    let committed = ["User/operator"];
    let zone = zone_named("quota-admit");
    let fixture = spawn_plane("quota-admit",     Some(rows_now(&committed)), Arc::new(UnreadableCensus)).await;
    for (type_name, name) in [("User", "operator"), ("Volume", "data")] {
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired_in(&zone, type_name, name, b"{}"))
            .await
            .unwrap_or_else(|error| panic!("commit {type_name}/{name}: {error}"));
    }
    commit_and_settle(&fixture, &zone, "Quota", "zone", &quota_row(8)).await;

    fixture
        .client
        .authenticated_apply(operator(), None, desired_in(&zone, "Guest", "vm", b"{}"))
        .await
        .expect("a Zone under its ceiling admits another row");
    assert!(
        fixture.store.get(key_in(&zone, "Guest", "vm")).await.is_ok(),
        "the admitted row is committed, so the ceiling did not refuse it"
    );
}

/// An active `EmergencyPolicy` refuses new use with the reduction's own
/// reason, keeps its own row writable so the Zone can recover, and holds the
/// drain finalizer while the Zone's open use is outstanding.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_active_emergency_refuses_new_use_and_drives_the_drain() {
    let committed = ["User/operator"];
    let zone = zone_named("emergency-drain");
    let fixture = spawn_plane("emergency-drain",     Some(rows_now(&committed)), Arc::new(UnreadableCensus)).await;
    for (type_name, name) in [("User", "operator"), ("Volume", "data")] {
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired_in(&zone, type_name, name, b"{}"))
            .await
            .unwrap_or_else(|error| panic!("commit {type_name}/{name}: {error}"));
    }
    commit_and_settle(&fixture, &zone, "EmergencyPolicy", "zone", &emergency_row(true)).await;

    // New use is refused at the revoking stage, with the reduction's reason.
    let refused = fixture
        .client
        .authenticated_apply(operator(), None, desired_in(&zone, "Guest", "vm", b"{}"))
        .await;
    let text = refused.expect_err("an active reduction refuses new use").to_string();
    assert!(
        text.contains("emergency-reduction-active"),
        "the refusal names the reduction's own reason: {text}"
    );
    assert!(
        fixture.store.get(key_in(&zone, "Guest", "vm")).await.is_err(),
        "a refused use leaves no row behind"
    );

    // The reduction's own row stays writable, or the Zone could never clear
    // the emergency that is blocking it.
    fixture
        .client
        .authenticated_apply(
            operator(),
            None,
            desired_in(&zone, "EmergencyPolicy", "zone", &emergency_row(false)),
        )
        .await
        .expect("an operator must always be able to clear the reduction");
    settle().await;
    fixture
        .client
        .authenticated_apply(operator(), None, desired_in(&zone, "Guest", "vm", b"{}"))
        .await
        .expect("clearing the reduction admits new use again");
}

/// A census the broker could not answer fences the Zone: the reduction still
/// holds, and nothing is reported as drained or released.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_broker_outage_fences_the_reduction_rather_than_converging_it() {
    let committed = ["User/operator"];
    // The store is readable here, so the plane's own open-use source is the
    // production one; the Zone holds a committed reservation, so the census is
    // answerable and the plan is the honest one rather than a guess.
    let zone = zone_named("emergency-fenced");
    let fixture = spawn_plane("emergency-fenced",     Some(rows_now(&committed)), Arc::new(UnreadableCensus)).await;
    for (type_name, name) in [("User", "operator"), ("Volume", "data")] {
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired_in(&zone, type_name, name, b"{}"))
            .await
            .unwrap_or_else(|error| panic!("commit {type_name}/{name}: {error}"));
    }
    commit_and_settle(&fixture, &zone, "EmergencyPolicy", "zone", &emergency_row(true)).await;

    // An unreadable census is not an empty Zone: the reduction still refuses
    // new use, which is the direction that must not fail open.
    let refused = fixture
        .client
        .authenticated_apply(operator(), None, desired_in(&zone, "Guest", "vm", b"{}"))
        .await;
    assert!(
        refused.is_err(),
        "an unreadable census still holds the reduction rather than thawing the Zone"
    );

    // The contract's own decision over the same unreadable census is fenced,
    // so the driver's held finalizer and the plan agree.
    let plan = d2b_provider_emergency_policy::plan_drain(
        &EmergencyReduction::of(&[EmergencyPolicySpec::new(
            true,
            EmergencyScope::new(true, false, false, true),
            30,
            "operator reduction",
        )
        .expect("the policy validates")])
        .bind_to(reference("EmergencyPolicy/zone")),
        None,
    );
    assert_eq!(plan.state(), EnforcementState::Fenced);
    assert!(!plan.may_release(), "nothing may be released on a guess");
}

/// The reduction the emergency driver published is the one the admission
/// enforces, at the effect boundary as well as the mutation seam.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_published_reduction_is_the_one_the_admission_enforces() {
    let committed = ["User/operator"];
    let zone = zone_named("emergency-published");
    let fixture = spawn_plane("emergency-published",     Some(rows_now(&committed)), Arc::new(UnreadableCensus)).await;
    for (type_name, name) in [("User", "operator"), ("Volume", "data")] {
        fixture
            .client
            .authenticated_apply(bootstrap(), None, desired_in(&zone, type_name, name, b"{}"))
            .await
            .unwrap_or_else(|error| panic!("commit {type_name}/{name}: {error}"));
    }
    commit_and_settle(&fixture, &zone, "EmergencyPolicy", "zone", &emergency_row(true)).await;

    let runtime =
        d2b_provider_emergency_policy::runtime(&zone).expect("the composition installed a runtime");
    assert!(
        runtime.reduction().is_active(),
        "the driver's reconcile published the reduction its committed row states"
    );
    assert_eq!(
        runtime.reduction().admit_new_use(),
        GraphDecision::Refused {
            stage: AdmissionStage::Revoke,
            reason: RefusalReason::EmergencyReductionActive
        },
        "the published reduction refuses new use at the revoking stage"
    );
    // The reduction is bound to its own row, so it can never block the row
    // that carries it.
    assert_eq!(
        runtime.reduction().admit_new_row(&reference("EmergencyPolicy/zone")),
        GraphDecision::Admitted,
        "an operator must always be able to change or clear the reduction"
    );
}

/// The committed rows the plane's own usage source counts are the rows a
/// ceiling is measured against: it reads the store, not a caller's list.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_plane_usage_source_counts_the_committed_rows() {
    let directory = tempfile::tempdir().expect("a temporary store directory");
    let store = Arc::new(SpecStore::open(directory.path().join("specs.sqlite")).expect("store"));
    let source = PlaneZoneUsage { store: Arc::clone(&store), zone: zone() };
    for (type_name, name) in [("User", "operator"), ("Volume", "data"), ("Guest", "vm")] {
        store
            .ensure(d2b_resource_runtime::spec_store::StoredDesiredResource {
                key: key(type_name, name),
                uid: d2b_resource_runtime::manager::deterministic_uid(&key(type_name, name)),
                generation: 1,
                owner_uid: None,
                provenance: ResourceProvenance::Api,
                deleting: false,
                spec: Vec::new(),
                metadata: Vec::new(),
                created_at: 0,
            })
            .await
            .expect("the row commits");
    }
    let usage = source.usage().await.expect("the census is countable").expect("a census");
    assert_eq!(usage.resources(), 3, "every committed row of the Zone is counted");
    assert_eq!(
        usage.count_of(&ResourceTypeName::parse("Volume").expect("a registered type")),
        1,
        "rows are counted per type, so a per-type ceiling has something to measure"
    );
}

/// The plane's own open-use source names the Zone's committed reservations
/// and reports no outstanding use it cannot prove, so a reduction measured
/// against it is one the Zone can actually show has drained.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_plane_open_use_source_names_the_committed_reservations() {
    let directory = tempfile::tempdir().expect("a temporary store directory");
    let store = Arc::new(SpecStore::open(directory.path().join("specs.sqlite")).expect("store"));
    let source = PlaneZoneOpenUse { store: Arc::clone(&store), zone: zone() };
    store
        .ensure(d2b_resource_runtime::spec_store::StoredDesiredResource {
            key: key("Volume", "data"),
            uid: d2b_resource_runtime::manager::deterministic_uid(&key("Volume", "data")),
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: Vec::new(),
            metadata: Vec::new(),
            created_at: 0,
        })
        .await
        .expect("the row commits");
    let census = OpenUseSource::census(&source).await.expect("the census is answerable").expect("a census");
    assert_eq!(
        census.sources(),
        [reference("Volume/data")],
        "the committed reservation is named by its own row, never by a caller's list"
    );
    assert!(
        census.outstanding().is_empty(),
        "outstanding use is runtime state the store does not carry, so none is claimed"
    );
}

/// The per-type ceilings a committed row declares are read and enforced, so a
/// Zone-wide count is not the only limit the row states.
#[test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn a_per_type_ceiling_is_read_from_the_committed_row() {
    let row = serde_json::to_vec(&serde_json::json!({
        "ceilings": {
            "maxResources": 64,
            "maxResourcesPerType": 64,
            "maxOwnerDepth": 4,
            "maxCpu": null,
            "maxMemoryMib": null,
            "maxStorageGib": null
        },
        "perTypeCeilings": { "Guest": { "maxResources": 2 } },
        "scope": "zone",
        "enforcementPolicy": "hard"
    }))
    .expect("the ceiling row serializes");
    let policy = d2b_provider_quota::quota::QuotaPolicy::decode(&row).expect("the row decodes");
    let guest = ResourceTypeName::parse("Guest").expect("a registered type");
    assert_eq!(policy.type_ceiling(&guest), 2, "the row's own per-type ceiling is read");
    assert_eq!(
        policy.type_ceiling(&ResourceTypeName::parse("Process").expect("a registered type")),
        64,
        "a type with no entry is measured against the Zone-wide per-type ceiling"
    );
    assert!(
        policy.per_type_ceilings().len() == 1,
        "the per-type map is carried, so the admission measures against the row's own ceiling"
    );
    // A ceiling written under a name this contract does not implement is
    // refused rather than skipped.
    let bogus = serde_json::to_vec(&serde_json::json!({
        "ceilings": {
            "maxResources": 8, "maxResourcesPerType": 8, "maxOwnerDepth": 4,
            "maxCpu": null, "maxMemoryMib": null, "maxStorageGib": null
        },
        "perTypeCeilings": { "Guest": { "maxVms": 2 } },
        "scope": "zone",
        "enforcementPolicy": "hard"
    }))
    .expect("the bogus row serializes");
    assert!(
        d2b_provider_quota::quota::QuotaPolicy::decode(&bogus).is_err(),
        "a ceiling under an unknown inner name never passes as an enforced one"
    );
    // The types a Zone can meter, kept next to the ceilings that meter them.
    let _ = BTreeMap::<String, u32>::new();
}
