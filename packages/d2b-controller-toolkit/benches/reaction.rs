//! Synthetic fast-path reaction benchmark for the controller toolkit.
//!
//! This target exercises the implemented in-memory runner and a deliberately
//! slow fake launch effect. It reports commit-to-launch-attempt samples for
//! the 1/10/100 ready-resource profiles and checks that independent launches
//! overlap. The store watch dispatcher and production Process Provider are not
//! wired into this crate, so the timing output is diagnostic rather than
//! evidence for the production latency contract.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use d2b_contracts::v3::{
    ConfigurationGeneration, ControllerGeneration, ResourceGeneration, ResourceRef,
    ResourceTypeName, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_controller_toolkit::{
    CommitDecision, CommitOutcome, ControllerDescriptor, ControllerExecutionPolicy,
    ControllerHealth, ControllerIdentity, ControllerSelector, ControllerSource, ControllerVerb,
    DependencySnapshot, DisruptionClass, DrainResult, FinalizeResult, FreshSnapshot, InitialList,
    ObservationResult, OperationContext, PriorityLane, ReconcileContext, ReconcilePlan,
    ReconcileResult, ResourceKey, ResourceReconciler, ResourceRegistration, ResourceSnapshot,
    ResyncPolicy, Runner, RunnerConfig, SourceError, StatusPersistence, TriggerReason, TriggerSet,
    UpdateAssessment, UpdateAssessmentState, UpgradePlan, UpgradeStage, ValidationResult,
    WatchEvent, WatchFailure, WatchHint,
};
use tokio::sync::{Notify, mpsc};

const PROFILES: [usize; 3] = [1, 10, 100];
const FAKE_LAUNCH_DURATION: Duration = Duration::from_millis(200);

#[derive(Debug)]
struct LaunchRecord {
    key: ResourceKey,
    started_at: Instant,
}

struct FakeLaunchEffect {
    delay: Duration,
    starts: Mutex<Vec<LaunchRecord>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    started: Notify,
}

impl FakeLaunchEffect {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            starts: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            started: Notify::new(),
        }
    }

    async fn launch(&self, key: ResourceKey) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(LaunchRecord {
                key,
                started_at: Instant::now(),
            });
        self.started.notify_waiters();
        tokio::time::sleep(self.delay).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
    }

    async fn wait_for_starts(&self, expected: usize) {
        loop {
            let notified = self.started.notified();
            let reached = self
                .starts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len()
                >= expected;
            if reached {
                return;
            }
            notified.await;
        }
    }

    fn starts(&self) -> Vec<(ResourceKey, Instant)> {
        self.starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|record| (record.key.clone(), record.started_at))
            .collect()
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
}

struct SyntheticSource {
    fresh: BTreeMap<ResourceKey, FreshSnapshot>,
    watch_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Result<WatchEvent, WatchFailure>>>,
    watch_opened: Notify,
}

impl SyntheticSource {
    fn new(
        fresh: BTreeMap<ResourceKey, FreshSnapshot>,
        watch_rx: mpsc::UnboundedReceiver<Result<WatchEvent, WatchFailure>>,
    ) -> Self {
        Self {
            fresh,
            watch_rx: tokio::sync::Mutex::new(watch_rx),
            watch_opened: Notify::new(),
        }
    }
}

impl ControllerSource for SyntheticSource {
    fn register(
        &self,
        _descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn list_initial(
        &self,
        _descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<InitialList, SourceError>> + Send {
        std::future::ready(Ok(InitialList {
            resources: Vec::new(),
            snapshot_revision: ZoneRevision::new(1),
        }))
    }

    fn open_watch(
        &self,
        _descriptor: &ControllerDescriptor,
        _after_revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.watch_opened.notify_waiters();
        std::future::ready(Ok(()))
    }

    async fn receive_watch(&self) -> Result<WatchEvent, WatchFailure> {
        self.watch_rx
            .lock()
            .await
            .recv()
            .await
            .unwrap_or(Ok(WatchEvent::Closed))
    }

    fn read_fresh(
        &self,
        key: &ResourceKey,
    ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send {
        std::future::ready(self.fresh.get(key).cloned().ok_or(SourceError::Unavailable))
    }

    fn write_starting(
        &self,
        _context: &ReconcileContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn await_expedited_commit(
        &self,
        _context: &ReconcileContext,
    ) -> impl Future<Output = Result<CommitDecision, SourceError>> + Send {
        std::future::ready(Ok(CommitDecision::Abort))
    }

    fn commit_result(
        &self,
        _context: &ReconcileContext,
        _result: &ReconcileResult,
    ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
        std::future::ready(Ok(CommitOutcome::Committed(ZoneRevision::new(1))))
    }

    fn complete_expedited(
        &self,
        _context: &ReconcileContext,
        _projection: &d2b_controller_toolkit::ReconcileProjection,
        _status_persistence: StatusPersistence,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn persist_outcome(
        &self,
        _projection: &d2b_controller_toolkit::ReconcileProjection,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn checkpoint(
        &self,
        _context: &ReconcileContext,
        _revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn schedule_requeue(
        &self,
        _key: &ResourceKey,
        _at_tick: u64,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }
}

struct SyntheticReconciler {
    descriptor: ControllerDescriptor,
    effect: Arc<FakeLaunchEffect>,
}

impl ResourceReconciler for SyntheticReconciler {
    type Error = Infallible;

    fn describe(&self) -> impl Future<Output = Result<ControllerDescriptor, Self::Error>> + Send {
        std::future::ready(Ok(self.descriptor.clone()))
    }

    fn validate_spec(
        &self,
        _context: &ReconcileContext,
        _resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<ValidationResult, Self::Error>> + Send {
        std::future::ready(Ok(ValidationResult::Valid))
    }

    fn plan(
        &self,
        _context: &ReconcileContext,
        _resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<ReconcilePlan, Self::Error>> + Send {
        std::future::ready(Ok(ReconcilePlan::new(
            vec!["process-launch".to_owned()],
            false,
        )
        .expect("synthetic launch plan is bounded")))
    }

    async fn reconcile(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &ReconcilePlan,
    ) -> Result<ReconcileResult, Self::Error> {
        context
            .authorize_effect()
            .expect("ordinary synthetic passes authorize effects");
        self.effect.launch(resource.key().clone()).await;
        Ok(ReconcileResult::converged(
            resource.revision(),
            resource.generation(),
        ))
    }

    fn observe(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<ObservationResult, Self::Error>> + Send {
        std::future::ready(Ok(ObservationResult::new(ReconcileResult::converged(
            resource.revision(),
            resource.generation(),
        ))))
    }

    fn finalize(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<FinalizeResult, Self::Error>> + Send {
        std::future::ready(Ok(FinalizeResult::new(ReconcileResult::converged(
            resource.revision(),
            resource.generation(),
        ))))
    }

    fn health(&self) -> impl Future<Output = Result<ControllerHealth, Self::Error>> + Send {
        std::future::ready(Ok(ControllerHealth::Healthy))
    }

    fn drain(
        &self,
        _deadline_tick: u64,
    ) -> impl Future<Output = Result<DrainResult, Self::Error>> + Send {
        std::future::ready(Ok(DrainResult::Drained))
    }

    fn assess_update(
        &self,
        _context: &ReconcileContext,
        _resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<UpdateAssessment, Self::Error>> + Send {
        std::future::ready(Ok(UpdateAssessment::new(
            UpdateAssessmentState::Current,
            Vec::new(),
            true,
        )
        .expect("synthetic assessment is bounded")))
    }

    fn plan_upgrade(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<UpgradePlan, Self::Error>> + Send {
        std::future::ready(Ok(UpgradePlan::new(
            DisruptionClass::Restart,
            true,
            vec![UpgradeStage::Restart(resource.key().resource_ref().clone())],
        )
        .expect("synthetic upgrade plan is bounded")))
    }

    fn execute_upgrade(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &UpgradePlan,
    ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        std::future::ready(Ok(ReconcileResult::converged(
            resource.revision(),
            resource.generation(),
        )))
    }
}

fn key(index: usize) -> ResourceKey {
    ResourceKey::new(
        ZoneId::parse("reaction").expect("valid Zone"),
        ResourceRef::parse(&format!("Process/ready-{index}")).expect("valid Process ref"),
        ResourceUid::parse(format!("123e4567-e89b-42d3-a456-42661417{index:04x}"))
            .expect("valid Process UID"),
    )
}

fn descriptor(concurrency: usize) -> ControllerDescriptor {
    let process = ResourceTypeName::parse("Process").expect("valid ResourceType");
    let identity = ControllerIdentity::new(
        ZoneId::parse("reaction").expect("valid Zone"),
        ResourceRef::parse("Process/controller").expect("valid controller ref"),
        ControllerGeneration::new(1).expect("nonzero controller generation"),
        ResourceRef::parse("Provider/system-minijail").expect("valid Provider ref"),
        ResourceGeneration::new(1).expect("nonzero Provider generation"),
        ResourceRef::parse("Process/controller").expect("valid Process ref"),
        ResourceRef::parse("Host/system").expect("valid Host ref"),
        None,
    )
    .expect("synthetic identity is valid");
    ControllerDescriptor::new(
        identity,
        vec![
            ResourceRegistration::new(process.clone(), vec![1], 30_000, 1)
                .expect("synthetic registration is valid"),
        ],
        vec!["resource-api".to_owned()],
        vec!["host".to_owned()],
        vec![ControllerVerb::ReadSpec, ControllerVerb::WriteStatus],
        vec![
            ControllerSelector::new(process, d2b_controller_toolkit::SelectorField::Spec, None)
                .expect("synthetic selector is valid"),
        ],
        Vec::new(),
        true,
        Vec::new(),
        vec!["reaction.service.v1".to_owned()],
        vec!["reaction.schema.v1".to_owned()],
        ControllerExecutionPolicy::new(
            concurrency,
            concurrency,
            concurrency,
            1,
            u32::try_from(concurrency).expect("profile fits watch credit"),
            ResyncPolicy::new(Some(10_000), 30_000).expect("synthetic resync is valid"),
        )
        .expect("synthetic execution policy is valid"),
    )
    .expect("synthetic descriptor is valid")
}

fn fresh(key: ResourceKey) -> FreshSnapshot {
    FreshSnapshot::Present {
        target: ResourceSnapshot::new(
            key,
            ZoneRevision::new(1),
            ResourceGeneration::new(1).expect("nonzero generation"),
            Vec::new(),
            false,
        ),
        dependencies: Vec::new(),
    }
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    assert!(!samples.is_empty(), "benchmark needs samples");
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = (percentile * sorted.len())
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

async fn run_profile(count: usize) {
    let keys = (0..count).map(key).collect::<Vec<_>>();
    let fresh = keys
        .iter()
        .cloned()
        .map(|key| (key.clone(), fresh(key)))
        .collect::<BTreeMap<_, _>>();
    let (watch_tx, watch_rx) = mpsc::unbounded_channel();
    let source = Arc::new(SyntheticSource::new(fresh, watch_rx));
    let effect = Arc::new(FakeLaunchEffect::new(FAKE_LAUNCH_DURATION));
    let reconciler = Arc::new(SyntheticReconciler {
        descriptor: descriptor(count),
        effect: Arc::clone(&effect),
    });
    let runner = Runner::new(
        Arc::clone(&reconciler),
        Arc::clone(&source),
        RunnerConfig {
            policy_revision: 1,
            api_revision: 1,
            configuration_revision: ConfigurationGeneration::new(1)
                .expect("nonzero configuration generation"),
            deadline_tick: 30_000,
            max_attempts: 1,
        },
    );
    let watch_opened = source.watch_opened.notified();
    let runner_task = tokio::spawn(runner.run());
    watch_opened.await;

    let mut committed_at = BTreeMap::new();
    for (index, key) in keys.iter().enumerate() {
        let operation = OperationContext::new(
            format!("reaction-operation-{index}"),
            format!("reaction-idempotency-{index}"),
            format!("reaction-correlation-{index}"),
            None,
        )
        .expect("synthetic operation is valid");
        committed_at.insert(key.clone(), Instant::now());
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                key.clone(),
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::SpecGenerationChanged]),
                PriorityLane::Ordinary,
                operation,
            )))))
            .expect("runner watch is alive");
    }

    effect.wait_for_starts(count).await;
    drop(watch_tx);
    let report = runner_task
        .await
        .expect("runner task joined")
        .expect("synthetic runner completed");
    assert_eq!(report.dispatched, count);
    assert_eq!(report.checkpointed, count);

    let starts = effect.starts();
    assert_eq!(starts.len(), count);
    let started_at = starts.into_iter().collect::<BTreeMap<_, _>>();
    let samples = keys
        .iter()
        .map(|key| started_at[key].saturating_duration_since(committed_at[key]))
        .collect::<Vec<_>>();
    let next_dispatch = keys
        .windows(2)
        .map(|pair| started_at[&pair[1]].saturating_duration_since(committed_at[&pair[0]]))
        .collect::<Vec<_>>();

    if count > 1 {
        assert!(
            effect.max_active() >= 2,
            "independent Process launches were serialized"
        );
        let first_start = started_at[&keys[0]];
        assert!(
            started_at.values().all(
                |started| started.saturating_duration_since(first_start) < FAKE_LAUNCH_DURATION
            ),
            "a ready Process waited for an earlier fake launch to finish"
        );
        assert!(
            next_dispatch
                .iter()
                .all(|latency| *latency < FAKE_LAUNCH_DURATION),
            "next independent dispatch waited for the prior fake effect"
        );
    }

    println!(
        "reaction profile={count} samples={} p50_us={:.3} p95_us={:.3} p99_us={:.3} max_active={} next_dispatch_max_us={:.3}",
        samples.len(),
        percentile(&samples, 50).as_secs_f64() * 1_000_000.0,
        percentile(&samples, 95).as_secs_f64() * 1_000_000.0,
        percentile(&samples, 99).as_secs_f64() * 1_000_000.0,
        effect.max_active(),
        next_dispatch
            .iter()
            .copied()
            .max()
            .unwrap_or_default()
            .as_secs_f64()
            * 1_000_000.0,
    );
}

#[test]
fn launch_attempt_start() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("create benchmark runtime")
        .block_on(async {
            for count in PROFILES {
                run_profile(count).await;
            }
        });
}
