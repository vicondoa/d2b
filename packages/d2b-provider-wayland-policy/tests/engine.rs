//! The interaction family engine over scripted effects and a recording
//! manager.
//!
//! These tests drive the verbs every interaction type shares: child ensure
//! before the effect, the preserved requeue cadence, one watch per target,
//! endpoint-first retirement, adoption on recover, the drain-before-Provider
//! teardown order, and the closed validate failure surface. The per-type
//! crates test their own row vocabulary on top of this.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef};
use d2b_provider_wayland_policy::{
    InteractionChildContext, InteractionDriver, InteractionDriverArgs, InteractionDriverEffects,
    InteractionDriverStatus, InteractionEffectError, InteractionKind, InteractionSpecEnvelope,
    InteractionType, key_ref, spec_decoder,
};
use d2b_provider_wayland_policy::test_support::{Log, ScriptedEffects};
use d2b_resource_runtime::context::{
    ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, SpecDecoder,
    WatchId, WatchRegistration,
};
use d2b_resource_runtime::driver::{
    ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{FailureClass, ResourceError};
use d2b_resource_runtime::identity::{
    ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource,
};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use serde_json::json;

// -- the test type ----------------------------------------------------------

/// One declared type for the engine: one dependency and two children.
#[derive(Clone)]
struct TestType {
    valid: bool,
}

impl InteractionType for TestType {
    const KIND: InteractionKind = InteractionKind::DisplayWaylandPolicy;
    const RESOURCE_TYPE: &'static str = "test.d2bus.org.Row";
    const PROVIDER_REF: &'static str = "Provider/test";
    const SPEC_PROVIDER_SELECTOR: bool = false;

    fn resync(&self) -> Duration {
        Duration::from_millis(300_000)
    }

    fn validate(&self, _envelope: &InteractionSpecEnvelope) -> Result<(), InteractionEffectError> {
        if self.valid {
            Ok(())
        } else {
            Err(InteractionEffectError::InvalidResource)
        }
    }

    fn dependencies(
        &self,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
        Ok(vec![ResourceRef::parse("Guest/work").expect("guest ref")])
    }

    fn desired_children(
        &self,
        children: &InteractionChildContext<'_>,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        Ok(vec![
            ChildEnsure {
                type_name: ResourceTypeName::new("Process"),
                name: format!("worker-{}", children.generation),
                spec: b"{}".to_vec(),
                metadata: b"{}".to_vec(),
            },
            ChildEnsure {
                type_name: ResourceTypeName::new("Endpoint"),
                name: format!("endpoint-{}", children.generation),
                spec: b"{}".to_vec(),
                metadata: b"{}".to_vec(),
            },
        ])
    }
}

// -- recording manager ------------------------------------------------------

/// Recording manager endpoint over one shared ordered log.
#[derive(Clone)]
struct RecordingManager {
    log: Log,
    rows: Arc<tokio::sync::Mutex<Vec<StoredDesiredResource>>>,
    parent_uid: [u8; 16],
    watches: Arc<tokio::sync::Mutex<Vec<ResourceKey>>>,
}

impl RecordingManager {
    fn new(log: Log, parent_uid: [u8; 16]) -> Self {
        Self {
            log,
            rows: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            parent_uid,
            watches: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    fn seed_owned(&self, key: ResourceKey, deleting: bool) {
        self.rows.try_lock().expect("fixture rows uncontended").push(StoredDesiredResource {
            key,
            uid: [0x77; 16],
            generation: 1,
            owner_uid: Some(self.parent_uid),
            provenance: ResourceProvenance::Resource,
            deleting,
            spec: Vec::new(),
            metadata: Vec::new(),
            created_at: 0,
        });
    }
}

#[async_trait::async_trait]
impl ManagerEndpoint for RecordingManager {
    async fn ensure_child(
        &self,
        _parent: &ResourceKey,
        child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        let key = ResourceKey::new("work", child.type_name.as_str(), child.name.clone());
        self.log
            .lock().await
            .push(format!("ensure:{}/{}", key.type_name, key.name));
        let mut rows = self.rows.lock().await;
        match rows.iter_mut().find(|row| row.key == key) {
            Some(row) => {
                row.generation += 1;
                row.spec = child.spec;
                row.metadata = child.metadata;
                Ok(EnsureOutcome::Updated(row.clone()))
            }
            None => {
                rows.push(StoredDesiredResource {
                    key: key.clone(),
                    uid: [0x88; 16],
                    generation: 1,
                    owner_uid: Some(self.parent_uid),
                    provenance: ResourceProvenance::Resource,
                    deleting: false,
                    spec: child.spec,
                    metadata: child.metadata,
                    created_at: 0,
                });
                Ok(EnsureOutcome::Created(rows.last().expect("pushed").clone()))
            }
        }
    }

    async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        self.log
            .lock().await
            .push(format!("get:{}/{}", key.type_name, key.name));
        Ok(self.rows.lock().await.iter().find(|row| row.key == *key).cloned())
    }

    async fn view(
        &self,
        _key: &ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
        // Desired rows only: this fixture publishes no runtime status.
        Ok(None)
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
        self.log
            .lock().await
            .push(format!("delete:{}/{}", key.type_name, key.name));
        if let Some(row) = self.rows.lock().await.iter_mut().find(|row| row.key == *key) {
            row.deleting = true;
        }
        Ok(())
    }

    async fn list_owned(
        &self,
        owner_uid: [u8; 16],
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        self.log.lock().await.push("list-owned".to_owned());
        Ok(self
            .rows
            .lock().await
            .iter()
            .filter(|row| row.owner_uid == Some(owner_uid))
            .cloned()
            .collect())
    }

    async fn register_watch(
        &self,
        _subscriber: &ResourceKey,
        registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        self.log.lock().await.push(format!(
            "watch:{}/{}",
            registration.target.type_name, registration.target.name
        ));
        self.watches.lock().await.push(registration.target);
        Ok(WatchId(self.watches.lock().await.len() as u64))
    }

    async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// Requeue scheduler recording schedules over the shared log.
struct RecordingRequeue {
    log: Log,
    schedules: AtomicUsize,
}

impl RequeueScheduler for RecordingRequeue {
    fn schedule(&self, _key: ResourceKey, after: Duration) -> RequeueId {
        self.log
            .try_lock()
            .expect("fixture log uncontended")
            .push(format!("requeue:{}ms", after.as_millis()));
        RequeueId(self.schedules.fetch_add(1, Ordering::SeqCst) as u64)
    }

    fn cancel(&self, _id: RequeueId) {}
}

struct Fixture {
    ctx: ResourceContext,
    log: Log,
    effects: Arc<ScriptedEffects>,
    manager: RecordingManager,
}

fn build_fixture(
    row: StoredDesiredResource,
    valid: bool,
) -> (Fixture, InteractionDriver<TestType>) {
    let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let effects = ScriptedEffects::shared(Arc::clone(&log));
    let manager = RecordingManager::new(Arc::clone(&log), row.uid);
    let requeue = RecordingRequeue {
        log: Arc::clone(&log),
        schedules: AtomicUsize::new(0),
    };
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = ResourceContext::new(
        row,
        spec_decoder(),
        Arc::new(manager.clone()),
        Arc::new(requeue),
        effects_tx,
        notify_tx,
    );
    let driver = InteractionDriver::new(InteractionDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(3).unwrap(),
        effects: Arc::clone(&effects) as Arc<dyn InteractionDriverEffects>,
        behavior: TestType { valid },
    })
    .expect("driver");
    (
        Fixture {
            ctx,
            log,
            effects,
            manager,
        },
        driver,
    )
}

fn row() -> StoredDesiredResource {
    StoredDesiredResource {
        key: ResourceKey::new("work", "test.d2bus.org.Row", "row"),
        uid: [0x42; 16],
        generation: 4,
        owner_uid: None,
        provenance: ResourceProvenance::Nix,
        deleting: false,
        spec: br#"{"providerRef":"Provider/test"}"#.to_vec(),
        metadata: b"{}".to_vec(),
        created_at: 0,
    }
}

// -- decoder ----------------------------------------------------------------

/// The engine's decoder accepts both persisted representations: the compiled
/// spec document and the full envelope minus status.
#[test]
fn the_decoder_accepts_both_persisted_representations() {
    let decoder: Arc<dyn SpecDecoder> = spec_decoder();
    for bytes in [
        br#"{"providerRef":"Provider/test","loginShellRef":"artifact://shell"}"#.as_slice(),
        br#"{"apiVersion":"d2b.v3","spec":{"providerRef":"Provider/test"},"metadata":{}}"#.as_slice(),
    ] {
        let decoded = decoder.decode(bytes).expect("decodes");
        let envelope = decoded
            .downcast::<InteractionSpecEnvelope>()
            .expect("the decoder yields the family envelope");
        assert_eq!(envelope.provider_ref(), Some("Provider/test"));
        assert!(envelope.base().get("providerRef").is_none());
    }
    assert!(decoder.decode(b"[]").is_err());
    assert!(decoder.decode(b"not json").is_err());
}

/// The single-type factory serves exactly its declared type.
#[test]
fn the_factory_serves_only_its_declared_type() {
    let effects = ScriptedEffects::shared(Arc::new(tokio::sync::Mutex::new(Vec::new())));
    let factory = d2b_provider_wayland_policy::InteractionDriverFactory::new(
        InteractionDriverArgs {
            zone: "work".to_owned(),
            controller_generation: ControllerGeneration::new(3).unwrap(),
            effects,
            behavior: TestType { valid: true },
        },
    );
    let served = factory
        .resource_types()
        .iter()
        .map(|resource_type| resource_type.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(served, vec!["test.d2bus.org.Row".to_owned()]);
}

/// A malformed zone token is refused at the driver boundary instead of
/// panicking.
#[test]
fn the_driver_refuses_a_malformed_zone_token() {
    let effects = ScriptedEffects::shared(Arc::new(tokio::sync::Mutex::new(Vec::new())));
    let refusal = InteractionDriver::new(InteractionDriverArgs {
        zone: "not a zone token".to_owned(),
        controller_generation: ControllerGeneration::new(3).unwrap(),
        effects: Arc::clone(&effects) as Arc<dyn InteractionDriverEffects>,
        behavior: TestType { valid: true },
    })
    .expect_err("a malformed zone token is a typed refusal, not a panic");
    assert_eq!(refusal.to_string(), "interaction-spec-invalid");
}

// -- validate ---------------------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn validate_accepts_the_declared_row_and_refuses_another_zone() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    ResourceDriver::validate(&mut driver, &mut fixture.ctx)
        .await
        .expect("the declared row validates");

    let mut foreign = row();
    foreign.key = ResourceKey::new("other", "test.d2bus.org.Row", "row");
    let (mut foreign_fixture, mut foreign_driver) = build_fixture(foreign, true);
    let failure = ResourceDriver::validate(&mut foreign_driver, &mut foreign_fixture.ctx)
        .await
        .expect_err("another zone is terminal");
    assert_eq!(
        foreign_driver.classify_error(&failure).class(),
        FailureClass::Terminal
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn validate_refuses_another_resource_type() {
    let mut foreign = row();
    foreign.key = ResourceKey::new("work", "test.d2bus.org.Other", "row");
    let (mut fixture, mut driver) = build_fixture(foreign, true);
    let failure = ResourceDriver::validate(&mut driver, &mut fixture.ctx)
        .await
        .expect_err("another type is terminal");
    assert_eq!(
        driver.classify_error(&failure).class(),
        FailureClass::Terminal
    );
}

// -- reconcile --------------------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn reconcile_ensures_children_then_runs_the_effect_and_requeues() {
    let (mut fixture, mut driver) = build_fixture(row(), true);

    let outcome = ResourceDriver::reconcile(&mut driver, &mut fixture.ctx)
        .await
        .expect("reconcile");
    assert_eq!(outcome, ReconcileOutcome::Satisfied);

    // Every child rides the manager child API before the typed effect, and the
    // not-ready phase requeues on the type's preserved cadence.
    let log = fixture.log.lock().await.clone();
    let effect_at = log
        .iter()
        .position(|entry| entry == "effect:display-wayland-policy")
        .expect("typed effect ran");
    assert_eq!(log.iter().filter(|entry| entry.starts_with("ensure:")).count(), 2);
    assert!(
        log[..effect_at].iter().all(|entry| !entry.starts_with("effect:")),
        "every ensure is committed before the effect: {log:?}"
    );
    assert_eq!(log.last().map(String::as_str), Some("requeue:300000ms"));
    let status = fixture.ctx.status::<InteractionDriverStatus>().unwrap();
    assert!(!status.ready);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn reconcile_projects_ready_status_and_registers_each_watch_once() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    fixture.effects.make_ready();

    ResourceDriver::reconcile(&mut driver, &mut fixture.ctx)
        .await
        .expect("reconcile");
    ResourceDriver::reconcile(&mut driver, &mut fixture.ctx)
        .await
        .expect("reconcile");

    let status = fixture.ctx.status::<InteractionDriverStatus>().unwrap();
    assert!(status.ready);
    assert_eq!(status.resource, Some(json!({"phase": "Ready"})));
    let watches = fixture.manager.watches.lock().await;
    let mut unique = watches.clone();
    unique.sort_by(|left, right| {
        (left.type_name.as_str(), left.name.as_str())
            .cmp(&(right.type_name.as_str(), right.name.as_str()))
    });
    unique.dedup();
    assert_eq!(unique.len(), watches.len(), "one watch per target: {watches:?}");
    // The dependency plus the two children.
    assert_eq!(watches.len(), 3);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn reconcile_retires_obsolete_children_endpoint_first() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Process", "stale-worker"), false);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Endpoint", "stale-endpoint"), false);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Process", "already-deleting"), true);

    ResourceDriver::reconcile(&mut driver, &mut fixture.ctx)
        .await
        .expect("reconcile");

    let log = fixture.log.lock().await.clone();
    let deletes = log
        .iter()
        .filter(|entry| entry.starts_with("delete:"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        deletes,
        vec![
            "delete:Endpoint/stale-endpoint".to_owned(),
            "delete:Process/stale-worker".to_owned(),
        ],
        "endpoint-first / process-last, already-deleting rows left alone: {log:?}"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn reconcile_has_no_spawn_surface_beyond_the_manager_child_api() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    ResourceDriver::reconcile(&mut driver, &mut fixture.ctx)
        .await
        .expect("reconcile");
    let log = fixture.log.lock().await.clone();
    assert!(
        log.iter().all(|entry| entry.starts_with("ensure:")
            || entry.starts_with("delete:")
            || entry.starts_with("list-owned")
            || entry.starts_with("watch:")
            || entry.starts_with("requeue:")
            || entry.starts_with("effect:")),
        "no spawn-shaped call may appear: {log:?}"
    );
}

// -- recover ----------------------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn recover_adopts_when_every_desired_child_is_owned_and_missing_otherwise() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    assert_eq!(
        ResourceDriver::recover(&mut driver, &mut fixture.ctx)
            .await
            .expect("recover"),
        RecoveryOutcome::Missing
    );
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Process", "worker-4"), false);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Endpoint", "endpoint-4"), false);
    assert_eq!(
        ResourceDriver::recover(&mut driver, &mut fixture.ctx)
            .await
            .expect("recover"),
        RecoveryOutcome::Adopted
    );
}

// -- finalize and delete ----------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn finalize_finalizes_owned_children_before_the_provider_stage() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Process", "worker"), false);

    // A live owned child: the pass requeues and the Provider teardown stage
    // does not run.
    let failure = ResourceDriver::finalize(&mut driver, &mut fixture.ctx)
        .await
        .expect_err("owned child still live");
    assert_eq!(
        driver.classify_error(&failure).class(),
        FailureClass::Retryable
    );
    let log = fixture.log.lock().await.clone();
    assert!(
        log.iter().any(|entry| entry == "delete:Process/worker"),
        "the owned child is nudged through its own finalize-before-delete pass: {log:?}"
    );
    assert!(
        !log.iter().any(|entry| entry.starts_with("finalize:")),
        "the Provider teardown stage has not run: {log:?}"
    );

    // The child row retires: the same pass converges with no Provider stage.
    fixture.manager.rows.lock().await.clear();
    ResourceDriver::finalize(&mut driver, &mut fixture.ctx)
        .await
        .expect("converged once the child retired");
    assert!(
        !fixture
            .log
            .lock().await
            .iter()
            .any(|entry| entry.starts_with("finalize:")),
        "finalize runs no Provider effect"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn delete_runs_the_provider_stage_then_retires_every_owned_child() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Endpoint", "endpoint"), false);
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Process", "worker"), false);

    ResourceDriver::delete(&mut driver, &mut fixture.ctx)
        .await
        .expect("delete");

    let log = fixture.log.lock().await.clone();
    assert_eq!(log[0], "finalize:display-wayland-policy");
    assert_eq!(
        log.iter()
            .filter(|entry| entry.starts_with("delete:"))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "delete:Endpoint/endpoint".to_owned(),
            "delete:Process/worker".to_owned()
        ]
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn delete_is_retryable_while_the_provider_stage_is_pending() {
    let (mut fixture, mut driver) = build_fixture(row(), true);
    fixture.effects.hold_finalize();
    fixture
        .manager
        .seed_owned(ResourceKey::new("work", "Process", "worker"), false);

    let failure = ResourceDriver::delete(&mut driver, &mut fixture.ctx)
        .await
        .expect_err("provider stage pending");
    assert_eq!(
        driver.classify_error(&failure).class(),
        FailureClass::Retryable
    );
    assert!(
        !fixture
            .log
            .lock().await
            .iter()
            .any(|entry| entry.starts_with("delete:"))
    );
}

/// The manager child rows the type derives carry the row's generation and the
/// manager's identity, never a spawn payload.
#[test]
fn the_driver_registers_the_declared_type_with_the_registry() {
    let effects = ScriptedEffects::shared(Arc::new(tokio::sync::Mutex::new(Vec::new())));
    let descriptor =
        d2b_provider_wayland_policy::wayland_policy_descriptor(InteractionDriverArgs {
            zone: "work".to_owned(),
            controller_generation: ControllerGeneration::new(3).unwrap(),
            effects,
            behavior: d2b_provider_wayland_policy::WaylandPolicy,
        });
    let mut providers = d2b_resource_runtime::provider::ProviderDirectory::new();
    providers
        .register_driver(&descriptor)
        .expect("the declaration registers");
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new("display-wayland.d2bus.org.WaylandPolicy"))
    );
    assert_eq!(key_ref(&row().key).to_canonical_string(), "test.d2bus.org.Row/row");
}
