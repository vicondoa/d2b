//! Pre-drain lifecycle coverage for a source-owning binding (plan unit U8,
//! KTD10; R6, R36-R41).
//!
//! ResourceManager commits the durable deleting mark and blocks new authority
//! for the row, then the actor runs the driver's pre-drain hook, and only a
//! successful pre-drain authorizes the generic children-first finalization
//! that follows. The suite drives the real actor, manager, and store, so every
//! assertion is about observed ordering rather than about a helper's internals:
//!
//! 1. The pre-drain stage runs before the generic child finalization, and the
//!    stage that runs first is the one that decides when a source is released.
//! 2. A pre-drain that is not ready defers: the generic child deletion and the
//!    teardown never start, and the reservation is retained across the retry.
//! 3. Cancellation is handled from every state. A request cancelled before it
//!    was ever reserved and a relationship that was prepared but never active
//!    both finish without waiting for consumer activity that cannot exist.
//! 4. A consumer's deletion leaves a shared source and its other consumer
//!    intact.
//! 5. The deleting mark is the fence: a child ensure under a deleting parent
//!    is refused, so no new authority is created while the pre-drain runs.
//!
//! The source-claim half of the same unit - one arbitration decision per
//! source, attenuated helper legs, and the cleanup lane for a missing helper -
//! belongs to the broker's reservation owner and is covered by its own owner
//! tests in `packages/d2b-broker`. This crate cannot name that service without
//! a new production Cargo edge, so it covers the lifecycle the runtime owns.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder};
use d2b_resource_runtime::driver::{ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass, FailureKinds, ResourceError};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::{
    AllowAll, DesiredResource, MutationSubject, ResourceManager, ResourceManagerArgs,
    ResourceManagerClient,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::spec_store::{ResourceProvenance, SpecStore, StoredDesiredResource};
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2b_resource_runtime::watch::{WatchHub, DEFAULT_RING_CAPACITY};

const ZONE: &str = "work";
const BINDING: &str = "Binding";
const HELPER: &str = "Helper";
const SOURCE: &str = "Source";
const CONSUMER_A: &str = "consumer-a";
const CONSUMER_B: &str = "consumer-b";

/// One lifecycle stage the test driver observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    /// `pre_drain` started; the row's deleting mark is already committed.
    PreDrainStarted,
    /// `pre_drain` found nothing to drain and converged without waiting.
    PreDrainNoop,
    /// `pre_drain` is not ready and deferred; nothing downstream may start.
    PreDrainDeferred,
    /// `pre_drain` closed the consumer's prepared handles and helpers.
    HelpersFinalized,
    /// The generic children-first finalization ran.
    Finalize,
    /// Teardown ran.
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    /// Cancelled before anything was reserved.
    Requested,
    /// Prepared but never active: no consumer use exists to detach.
    Prepared,
    /// The consumer is using the relationship.
    Active,
}

#[derive(Debug, Default)]
struct Journal {
    steps: Vec<Step>,
    /// Whether the pre-drain was still holding a reservation.
    reservation_retained: Vec<bool>,
    /// The row the pre-drain read its own fence evidence from.
    fence_row_present: Option<String>,
    /// The consumer-detach observation the pre-drain is waiting for, if any.
    detach_observed: bool,
    defer_until_detached: bool,
}

#[derive(Debug)]
struct DriverError {
    deferred: bool,
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.deferred {
            "binding drain is not ready"
        } else {
            "binding drain failed"
        })
    }
}

impl std::error::Error for DriverError {}

/// A binding-shaped driver whose pre-drain is the unit under test.
struct BindingDriver {
    lifecycle: Lifecycle,
    journal: Arc<Mutex<Journal>>,
}

#[async_trait]
impl ResourceDriver for BindingDriver {
    type Error = DriverError;

    fn classify_error(&self, error: &DriverError) -> DriverFailure {
        if error.deferred {
            DriverFailure::not_yet(DriverOp::Delete, FailureKinds::CHILDREN_DRAINING)
                .at("pre-drain/drain")
                .with_note("the binding owner is still holding its source reservation")
        } else {
            DriverFailure::error(DriverOp::Delete, FailureKinds::DRIVER_ERROR, FailureClass::Retryable)
                .at("pre-drain/drain")
                .with_note("the binding owner could not close its source use")
        }
    }

    async fn validate(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverError> {
        Ok(())
    }

    async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, DriverError> {
        Ok(RecoveryOutcome::Missing)
    }

    async fn reconcile(
        &mut self,
        _ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, DriverError> {
        Ok(ReconcileOutcome::Satisfied)
    }

    /// The KTD10 stage: block new use, then drive the resource's own drain
    /// before anything generic touches its children.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverError> {
        let own = ctx.key().clone();
        // The driver reads its own durable row back so the fence evidence is
        // the committed mark rather than an in-memory flag the driver set.
        let fence_evidence = ctx.get(&own).await.map_err(|_| DriverError { deferred: false })?;
        if let Some(row) = fence_evidence.as_ref() {
            self.journal
                .lock().await
                .fence_row_present = Some(row.key.to_string());
        }
        let helpers = ctx
            .children()
            .await
            .map_err(|_| DriverError { deferred: false })?;
        {
            let mut journal = self.journal.lock().await;
            journal.steps.push(Step::PreDrainStarted);
        }
        match self.lifecycle {
            Lifecycle::Requested => {
                // Nothing was ever reserved or bound, so there is nothing to
                // drain and nothing to wait for.
                self.journal
                    .lock().await
                    .steps
                    .push(Step::PreDrainNoop);
                return Ok(());
            }
            Lifecycle::Prepared => {
                // Prepared but never active: there is no consumer use to
                // detach, so the pass closes the prepared handles and the
                // helpers without waiting for an observation that can never
                // arrive.
            }
            Lifecycle::Active => {
                let waiting = {
                    let journal = self.journal.lock().await;
                    journal.defer_until_detached && !journal.detach_observed
                };
                if waiting {
                    let mut journal = self.journal.lock().await;
                    journal.steps.push(Step::PreDrainDeferred);
                    journal.reservation_retained.push(true);
                    return Err(DriverError { deferred: true });
                }
            }
        }
        // Detach the consumer while the required helpers still exist, then
        // finalize the helpers. Each helper is a child the owner deletes
        // itself, ahead of the generic cascade.
        for helper in &helpers {
            ctx.delete(&helper.key)
                .await
                .map_err(|_| DriverError { deferred: false })?;
        }
        {
            let mut journal = self.journal.lock().await;
            journal.steps.push(Step::HelpersFinalized);
            journal.reservation_retained.push(false);
        }
        Ok(())
    }

    async fn finalize(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverError> {
        self.journal.lock().await.steps.push(Step::Finalize);
        Ok(())
    }

    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverError> {
        self.journal.lock().await.steps.push(Step::Delete);
        Ok(())
    }
}

/// Produces one [`BindingDriver`] per relationship, with the lifecycle the test
/// pins on that key.
struct BindingFactory {
    lifecycles: HashMap<String, Lifecycle>,
    journal: Arc<Mutex<Journal>>,
}

#[async_trait]
impl ResourceDriverFactory for BindingFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        static TYPES: std::sync::LazyLock<Vec<ResourceTypeName>> = std::sync::LazyLock::new(|| {
            vec![
                ResourceTypeName::new(BINDING),
                ResourceTypeName::new(HELPER),
                ResourceTypeName::new(SOURCE),
            ]
        });
        TYPES.as_slice()
    }

    async fn create(
        &self,
        key: &ResourceKey,
    ) -> Box<dyn d2b_resource_runtime::driver::DynResourceDriver> {
        Box::new(BindingDriver {
            lifecycle: self
                .lifecycles
                .get(key.name.as_str())
                .copied()
                .unwrap_or(Lifecycle::Active),
            journal: Arc::clone(&self.journal),
        })
    }
}

struct PassthroughDecoder;

impl SpecDecoder for PassthroughDecoder {
    fn decode(
        &self,
        envelope: &[u8],
    ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(envelope.to_vec()))
    }
}

struct HostOnlyResolver;

impl TargetResolver for HostOnlyResolver {
    fn execution_ref(&self, _key: &ResourceKey, _spec: &[u8]) -> Option<String> {
        None
    }
}

struct Harness {
    client: ResourceManagerClient,
    journal: Arc<Mutex<Journal>>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.client.actor().get_cell().stop(None);
    }
}

async fn harness(lifecycles: HashMap<String, Lifecycle>) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(SpecStore::open(tmp.path().join("specs.sqlite")).expect("store"));
    let journal = Arc::new(Mutex::new(Journal::default()));
    let mut providers = ProviderDirectory::new();
    providers
        .register(Arc::new(BindingFactory {
            lifecycles,
            journal: Arc::clone(&journal),
        }) as Arc<dyn ResourceDriverFactory>)
        .expect("factory registration");
    let hub = Arc::new(WatchHub::new(
        &d2b_resource_runtime::revision::SystemClock,
        DEFAULT_RING_CAPACITY,
    ));
    let args = ResourceManagerArgs {
        zone: ZONE.to_owned(),
        store,
        providers,
        hub,
        admission: Arc::new(AllowAll),
        decoders: HashMap::new(),
        default_decoder: Arc::new(PassthroughDecoder),
        targets: Arc::new(TargetDirectory::new()),
        host_target: TargetRef::host("test-host").expect("host target"),
        target_resolver: Arc::new(HostOnlyResolver),
        backoff: Duration::from_millis(20),
        relation_extractors: d2b_resource_runtime::relations::RelationExtractors::new(),
    };
    let (actor, _join) = ractor::Actor::spawn(None, ResourceManager::new(), args)
        .await
        .expect("manager spawn");
    Harness {
        client: ResourceManagerClient::new(actor),
        journal,
        _tmp: tmp,
    }
}

fn key(type_name: &str, name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, type_name, name)
}

fn subject() -> MutationSubject {
    MutationSubject {
        principal: "test".to_owned(),
        origin: ResourceProvenance::Resource,
    }
}

fn desired(type_name: &str, name: &str) -> DesiredResource {
    DesiredResource {
        key: key(type_name, name),
        spec: format!("{{\"name\":\"{name}\"}}").into_bytes(),
        metadata: Vec::new(),
        provenance: ResourceProvenance::Resource,
    }
}

/// The stored row behind one key, read through the manager mailbox.
async fn row(harness: &Harness, type_name: &str, name: &str) -> Option<StoredDesiredResource> {
    harness
        .client
        .get_row(key(type_name, name))
        .await
        .expect("row read")
}

async fn steps(harness: &Harness) -> Vec<Step> {
    harness.journal.lock().await.steps.clone()
}

/// Wait until one step has been recorded at least `count` times.
async fn until_step(harness: &Harness, step: Step, count: usize) {
    while steps(harness).await.iter().filter(|seen| **seen == step).count() < count {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The whole teardown of one relationship finished.
async fn until_gone(harness: &Harness, type_name: &str, name: &str) {
    while row(harness, type_name, name).await.is_some() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn pre_drain_runs_before_the_generic_child_finalization() {
    let h = harness(HashMap::from([(CONSUMER_A.to_owned(), Lifecycle::Active)])).await;
    h.client
        .ensure(
            subject(),
            None,
            desired(BINDING, CONSUMER_A),
        )
        .await
        .expect("binding");
    h.client
        .ensure_child(
            key(BINDING, CONSUMER_A),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: ResourceTypeName::new(HELPER),
                name: "worker".to_owned(),
                spec: b"{}".to_vec(),
                metadata: Vec::new(),
            },
        )
        .await
        .expect("helper child");
    // The helper is a live child the pre-drain has to finalize itself.
    while row(&h, HELPER, "worker").await.is_none() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    h.client
        .remove(subject(), key(BINDING, CONSUMER_A))
        .await
        .expect("delete requested");

    until_gone(&h, BINDING, CONSUMER_A).await;
    until_gone(&h, HELPER, "worker").await;
    until_step(&h, Step::Delete, 1).await;

    let observed = steps(&h).await;
    let position = |step: Step| observed.iter().position(|seen| *seen == step).expect("step");
    assert!(
        position(Step::PreDrainStarted) < position(Step::HelpersFinalized),
        "the driver closes its helpers inside the pre-drain: {observed:?}"
    );
    assert!(
        position(Step::HelpersFinalized) < position(Step::Finalize),
        "generic child finalization is authorized only after a successful pre-drain: {observed:?}"
    );
    assert!(
        position(Step::Finalize) < position(Step::Delete),
        "teardown is the last stage: {observed:?}"
    );
    assert!(
        !observed.contains(&Step::PreDrainDeferred),
        "an active relationship whose consumer is observed detached converges in one pass"
    );
    assert_eq!(
        h.journal.lock().await.fence_row_present.as_deref(),
        Some("work/Binding/consumer-a"),
        "the pre-drain read its own committed row, so the fence is the durable mark"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_deferred_pre_drain_retains_the_reservation_and_blocks_child_deletion() {
    let h = harness(HashMap::from([(CONSUMER_A.to_owned(), Lifecycle::Active)])).await;
    h.journal.lock().await.defer_until_detached = true;
    h.client
        .ensure(subject(), None, desired(BINDING, CONSUMER_A))
        .await
        .expect("binding");
    h.client
        .ensure_child(
            key(BINDING, CONSUMER_A),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: ResourceTypeName::new(HELPER),
                name: "worker".to_owned(),
                spec: b"{}".to_vec(),
                metadata: Vec::new(),
            },
        )
        .await
        .expect("helper child");
    while row(&h, HELPER, "worker").await.is_none() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    h.client
        .remove(subject(), key(BINDING, CONSUMER_A))
        .await
        .expect("delete requested");
    until_step(&h, Step::PreDrainDeferred, 1).await;

    {
        let journal = h.journal.lock().await;
        assert!(
            !journal.reservation_retained.is_empty()
                && journal.reservation_retained.iter().all(|retained| *retained),
            "every deferred pre-drain retains the reservation: {:?}",
            journal.reservation_retained
        );
    }
    assert!(
        row(&h, BINDING, CONSUMER_A)
            .await
            .is_some_and(|row| row.deleting),
        "the row holds with its durable deleting mark while the drain is pending"
    );
    assert!(
        !steps(&h).await.contains(&Step::HelpersFinalized)
            && !steps(&h).await.contains(&Step::Finalize)
            && !steps(&h).await.contains(&Step::Delete),
        "no ordering downstream of an incomplete pre-drain may start: {:?}",
        steps(&h).await
    );

    // The consumer-detach observation arrives; the retry converges and only
    // then does the ordering continue.
    h.journal.lock().await.detach_observed = true;
    until_gone(&h, BINDING, CONSUMER_A).await;
    until_gone(&h, HELPER, "worker").await;
    let observed = steps(&h).await;
    let first_defer = observed
        .iter()
        .position(|step| *step == Step::PreDrainDeferred)
        .expect("deferral");
    let finalized = observed
        .iter()
        .position(|step| *step == Step::HelpersFinalized)
        .expect("helpers finalized");
    assert!(
        first_defer < finalized,
        "the reservation was retained across the retry: {observed:?}"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn cancellation_finishes_from_a_requested_and_from_a_prepared_relationship() {
    let h = harness(HashMap::from([
        (CONSUMER_A.to_owned(), Lifecycle::Requested),
        (CONSUMER_B.to_owned(), Lifecycle::Prepared),
    ]))
    .await;
    for name in [CONSUMER_A, CONSUMER_B] {
        h.client
            .ensure(subject(), None, desired(BINDING, name))
            .await
            .expect("binding");
        h.client
            .ensure_child(
                key(BINDING, name),
                d2b_resource_runtime::context::ChildEnsure {
                    type_name: ResourceTypeName::new(HELPER),
                    name: format!("{name}-worker"),
                    spec: b"{}".to_vec(),
                    metadata: Vec::new(),
                },
            )
            .await
            .expect("helper child");
    }

    for name in [CONSUMER_A, CONSUMER_B] {
        h.client
            .remove(subject(), key(BINDING, name))
            .await
            .expect("delete requested");
    }
    for name in [CONSUMER_A, CONSUMER_B] {
        until_gone(&h, BINDING, name).await;
    }
    until_step(&h, Step::Delete, 2).await;

    let observed = steps(&h).await;
    assert!(
        !observed.contains(&Step::PreDrainDeferred),
        "neither a cancelled request nor a never-active relationship waits for consumer activity that cannot exist: {observed:?}"
    );
    assert!(
        observed.contains(&Step::PreDrainNoop),
        "a request cancelled before anything was reserved has a no-op pre-drain: {observed:?}"
    );
    assert!(
        observed
            .iter()
            .filter(|step| **step == Step::HelpersFinalized)
            .count()
            >= 2,
        "a prepared relationship closes its prepared handles and helpers without waiting: {observed:?}"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_consumer_deletion_leaves_a_shared_source_and_its_peer_intact() {
    let h = harness(HashMap::from([
        (CONSUMER_A.to_owned(), Lifecycle::Active),
        (CONSUMER_B.to_owned(), Lifecycle::Active),
    ]))
    .await;
    h.client
        .ensure(subject(), None, desired(SOURCE, "shared"))
        .await
        .expect("shared source");
    for name in [CONSUMER_A, CONSUMER_B] {
        h.client
            .ensure_child(
                key(SOURCE, "shared"),
                d2b_resource_runtime::context::ChildEnsure {
                    type_name: ResourceTypeName::new(BINDING),
                    name: name.to_owned(),
                    spec: b"{}".to_vec(),
                    metadata: Vec::new(),
                },
            )
            .await
            .expect("binding");
    }
    h.client
        .ensure_child(
            key(BINDING, CONSUMER_A),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: ResourceTypeName::new(HELPER),
                name: "worker".to_owned(),
                spec: b"{}".to_vec(),
                metadata: Vec::new(),
            },
        )
        .await
        .expect("helper child");

    h.client
        .remove(subject(), key(BINDING, CONSUMER_A))
        .await
        .expect("delete requested");
    until_gone(&h, BINDING, CONSUMER_A).await;
    until_gone(&h, HELPER, "worker").await;

    let source = row(&h, SOURCE, "shared")
        .await
        .expect("the shared source survives a consumer leaving");
    assert!(!source.deleting, "the source is not deleted with its consumer");
    let peer = row(&h, BINDING, CONSUMER_B)
        .await
        .expect("the other valid consumer remains");
    assert!(
        !peer.deleting,
        "the other consumer's relationship is untouched"
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_deleting_mark_blocks_new_authority_under_the_draining_relationship() {
    let h = harness(HashMap::from([(CONSUMER_A.to_owned(), Lifecycle::Active)])).await;
    h.journal.lock().await.defer_until_detached = true;
    h.client
        .ensure(subject(), None, desired(BINDING, CONSUMER_A))
        .await
        .expect("binding");

    h.client
        .remove(subject(), key(BINDING, CONSUMER_A))
        .await
        .expect("delete requested");
    until_step(&h, Step::PreDrainDeferred, 1).await;

    // The durable deleting mark is the fence: while the pre-drain is still to
    // run, no new child authority can be created under the draining row.
    let refused = h
        .client
        .ensure_child(
            key(BINDING, CONSUMER_A),
            d2b_resource_runtime::context::ChildEnsure {
                type_name: ResourceTypeName::new(HELPER),
                name: "late".to_owned(),
                spec: b"{}".to_vec(),
                metadata: Vec::new(),
            },
        )
        .await;
    assert!(
        matches!(
            refused,
            Err(ResourceError::DeletingConflict { .. })
                | Err(ResourceError::ManagerRejected { .. })
        ),
        "a child ensure under a deleting parent is refused: {refused:?}"
    );
    assert!(
        row(&h, HELPER, "late").await.is_none(),
        "the refused ensure created no row"
    );

    h.journal.lock().await.detach_observed = true;
    until_gone(&h, BINDING, CONSUMER_A).await;
}
