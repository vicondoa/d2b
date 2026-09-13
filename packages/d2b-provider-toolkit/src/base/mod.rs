//! The Provider lifecycle base.
//!
//! A provider implements [`ProviderBase`] and nothing else runs in its
//! `main`: the toolkit owns the sequence below it, so two providers cannot
//! diverge on bootstrap order, readiness, attach, startup order, the service
//! loop, or drain ordering.
//!
//! ```text
//! fd10 bootstrap -> readiness marker -> admission -> plane attach ->
//! declared startup steps -> service loop -> drain
//! ```
//!
//! What stays with the provider is the type-specific surface: the effect
//! ports, the operation handlers, the decoder and factory, the `attach` and
//! `drain` bodies, and readiness logic inside its reconcile. What stays with
//! the composition root is the zone-plane port - a provider can only ask for
//! the storage roots, adapters, and services it declared - and the generated
//! service surface the provider's own crate supplies.

mod bootstrap;
pub mod error;
#[cfg(feature = "unix-transport")]
pub mod fd10;
pub mod guest;
pub mod runtime;
pub mod startup;

pub use bootstrap::{
    AllocatorSessionBinding, PROVIDER_RESOURCE_TYPE, ProviderAgentBootstrap, ProviderAgentIdentity,
};
pub use error::ProviderToolkitError;
pub use guest::{
    AllocatorEnrollment, EnrolledRoute, EnrollmentRequest, GUEST_RECONNECT_ATTEMPTS,
    GUEST_RECONNECT_INITIAL_MS, GUEST_RECONNECT_MAX_MS, GUEST_SESSION_MAX_FRAME_BYTES, GuestAgent,
    GuestEnrollment, GuestError, GuestFrame, GuestLink, GuestLinkFuture, GuestPlacement,
    run_guest,
};
pub use runtime::{
    AuthenticatedRoute, ProviderAdmission, ProviderEntrypoint, ProviderLifecycle,
    ProviderRuntimeError, ProviderSessionAdmission,
};
pub use startup::{
    PlannedStep, StartupPlan, StartupPlanRefusal, StartupStepError, StartupStepExecutor,
};

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::identity::SessionPurpose;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_resource_types::{DriverDescriptor, ProviderDeclaration};

use crate::plane::{DrainDeadline, PlaneError, SharedClock, SystemClock, ZonePlaneHandle};

/// The one trait a provider implements.
///
/// The declaration and the drivers are the provider's whole static surface;
/// `attach` and `drain` are the two moments it acts on its own zone-plane
/// integration facts.
#[async_trait]
pub trait ProviderBase: Send + Sync + 'static {
    /// The identity and zone-level facts this provider declares.
    fn declaration(&self) -> &ProviderDeclaration;

    /// What this provider serves, one descriptor per resource type.
    fn drivers(&self) -> &'static [DriverDescriptor];

    /// Attach to a zone plane: claim storage roots, deploy adapters, publish
    /// services - each backed by the declared facts.
    async fn attach(&self, zone: &ZonePlaneHandle) -> Result<(), AttachError>;

    /// Tear down in the order this provider requires, within the deadline.
    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError>;
}

/// Why a provider could not attach to its zone plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachError {
    /// The plane refused one declared action.
    Plane(PlaneError),
    /// The provider's own attach body refused.
    Refused,
}

impl AttachError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Plane(error) => error.code(),
            Self::Refused => "attach-refused",
        }
    }
}

impl fmt::Display for AttachError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AttachError {}

/// Why a provider could not finish draining.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainError {
    /// The bounded drain budget ran out before the provider finished.
    DeadlineExpired,
    /// The provider's own drain body refused.
    Refused,
}

impl DrainError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(self) -> &'static str {
        match self {
            Self::DeadlineExpired => "drain-deadline-expired",
            Self::Refused => "drain-refused",
        }
    }
}

impl fmt::Display for DrainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DrainError {}

/// Why the declared startup plan could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupError {
    /// The declared plan itself is not derivable.
    Plan(StartupPlanRefusal),
    /// Steps were declared but no executor was supplied.
    ExecutorMissing,
    /// One step's body refused.
    Step(StartupStepError),
}

impl StartupError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Plan(refusal) => refusal.code(),
            Self::ExecutorMissing => "startup-executor-missing",
            Self::Step(error) => error.code,
        }
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for StartupError {}

/// The toolkit-owned lifecycle driver.
///
/// Every step below is the same for every provider; only the bodies the
/// provider supplies differ.
pub struct Lifecycle<P: ProviderBase> {
    provider: P,
    clock: SharedClock,
}

impl<P: ProviderBase> Lifecycle<P> {
    /// Build the lifecycle driver over one provider.
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            clock: Arc::new(SystemClock),
        }
    }

    /// Build the lifecycle driver over one provider and one clock.
    ///
    /// A test supplies a clock it controls so drain and requeue behavior is
    /// exercised without waiting on wall time.
    pub fn with_clock(provider: P, clock: SharedClock) -> Self {
        Self { provider, clock }
    }

    /// Borrow the provider.
    pub const fn provider(&self) -> &P {
        &self.provider
    }

    /// Borrow the clock the lifecycle measures deadlines against.
    pub fn clock(&self) -> &SharedClock {
        &self.clock
    }

    /// Borrow the provider's declaration.
    pub fn declaration(&self) -> &ProviderDeclaration {
        self.provider.declaration()
    }

    /// Borrow the provider's drivers.
    pub fn drivers(&self) -> &'static [DriverDescriptor] {
        self.provider.drivers()
    }

    /// Build the zone-plane handle this provider attaches through.
    pub fn plane_handle(
        &self,
        zone: ZoneId,
        port: Arc<dyn crate::plane::ZonePlanePort>,
    ) -> ZonePlaneHandle {
        ZonePlaneHandle::new(zone, self.declaration(), self.drivers(), port)
    }

    /// Attach to the zone plane.
    ///
    /// The toolkit owns the order: the declared storage roots are claimed,
    /// the declared adapters are deployed in declared dependency order, the
    /// declared services are published, and only then does the provider's own
    /// [`ProviderBase::attach`] body run.
    pub async fn attach(&self, zone: &ZonePlaneHandle) -> Result<(), AttachError> {
        zone.claim_declared_storage_roots()
            .await
            .map_err(AttachError::Plane)?;
        zone.deploy_declared_adapters()
            .await
            .map_err(AttachError::Plane)?;
        zone.publish_declared_services()
            .await
            .map_err(AttachError::Plane)?;
        self.provider.attach(zone).await
    }

    /// Derive the startup plan from the declared drivers.
    pub fn startup_plan(&self) -> Result<StartupPlan, StartupPlanRefusal> {
        StartupPlan::derive(self.drivers())
    }

    /// Execute the derived startup plan in order.
    pub async fn run_startup(
        &self,
        executor: Option<&dyn StartupStepExecutor>,
    ) -> Result<(), StartupError> {
        let plan = self.startup_plan().map_err(StartupError::Plan)?;
        if plan.is_empty() {
            return Ok(());
        }
        let Some(executor) = executor else {
            return Err(StartupError::ExecutorMissing);
        };
        for step in plan.steps() {
            executor
                .execute(step.declaration)
                .await
                .map_err(StartupError::Step)?;
        }
        Ok(())
    }

    /// Open a bounded drain against the lifecycle clock.
    pub fn drain_deadline(&self, budget_ms: u64) -> DrainDeadline {
        DrainDeadline::new(Arc::clone(&self.clock), budget_ms)
    }

    /// Drain the provider within the deadline, refusing when it expires.
    pub async fn drain(&self, budget_ms: u64) -> Result<(), DrainError> {
        let deadline = self.drain_deadline(budget_ms);
        let result = self.provider.drain(deadline.clone()).await;
        if deadline.expired() {
            return Err(DrainError::DeadlineExpired);
        }
        result
    }
}

/// The generated service surface a provider's own crate supplies.
///
/// The surface builds the ttrpc service map for one authenticated route. The
/// base owns when the loop runs and how readiness is published; the
/// provider's crate owns the generated methods behind it.
pub struct ServiceSurface<'a> {
    /// The generated service map for one authenticated route.
    pub services: &'a (dyn Fn() -> ServiceMethods + Send + Sync),
}

/// One provider's generated ttrpc service map.
pub type ServiceMethods = std::collections::HashMap<String, ttrpc::r#async::Service>;

/// The supervised provider entrypoint surface.
///
/// The declaration vocabulary states a provider's identity, drivers, and
/// services, but not the session purpose its binary accepts - that is the
/// entrypoint's own fact, and so are the generated services and the zone
/// plane port. A provider crate's `main` states them here and calls
/// [`run`].
pub trait SupervisedProvider: ProviderBase {
    /// The exact session purpose this binary accepts.
    fn accepted_purpose(&self) -> SessionPurpose;

    /// The generated service surface, or `None` when the provider serves no
    /// ComponentSession service.
    fn service_surface(&self) -> Option<ServiceSurface<'_>>;

    /// The zone-plane port this process attaches through.
    ///
    /// The default refuses every declared action, so an attach path that
    /// needs the plane fails loudly instead of silently succeeding. A
    /// provider whose attach body acts through the plane overrides this with
    /// the port the composition root supplied.
    fn plane_port(&self) -> Arc<dyn crate::plane::ZonePlanePort> {
        Arc::new(crate::plane::UnavailablePlanePort)
    }
}

/// Why a supervised provider process could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRunError {
    /// The async runtime could not be built.
    RuntimeUnavailable,
    /// The inherited supervised bootstrap is unavailable in this build.
    BootstrapUnavailable,
    /// Exactly one declared service is required to open a route.
    ServiceDeclarationMissing,
    /// More than one service is declared; the entrypoint binds one route.
    ServiceDeclarationAmbiguous,
    /// The supervised bootstrap or its route was refused.
    BootstrapRefused,
    /// Session admission refused the route.
    AdmissionRefused,
    /// Plane attach refused.
    Attach(AttachError),
    /// The declared startup plan refused.
    Startup(StartupError),
    /// The authenticated service loop failed.
    ServiceLoopFailed,
    /// Drain refused.
    Drain(DrainError),
}

impl ProviderRunError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::RuntimeUnavailable => "runtime-unavailable",
            Self::BootstrapUnavailable => "supervised-bootstrap-unavailable",
            Self::ServiceDeclarationMissing => "service-declaration-missing",
            Self::ServiceDeclarationAmbiguous => "service-declaration-ambiguous",
            Self::BootstrapRefused => "bootstrap-refused",
            Self::AdmissionRefused => "admission-refused",
            Self::Attach(_) => "attach-refused",
            Self::Startup(_) => "startup-refused",
            Self::ServiceLoopFailed => "service-loop-failed",
            Self::Drain(_) => "drain-refused",
        }
    }
}

impl fmt::Display for ProviderRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderRunError {}

/// Run one supervised provider process.
///
/// The process runs the lifecycle above in order and exits with the
/// entrypoint's status code. A provider that declares startup steps must use
/// [`run_with_startup`]: the base refuses to start rather than skip an
/// obligation it cannot execute.
pub fn run<P: SupervisedProvider>(provider: P) -> i32 {
    run_with_startup(provider, None)
}

/// Run one supervised provider process with an executor for its declared
/// startup steps.
pub fn run_with_startup<P: SupervisedProvider>(
    provider: P,
    startup: Option<Arc<dyn StartupStepExecutor>>,
) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return 1,
    };
    match runtime.block_on(serve_supervised(provider, startup)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// The one declared service a supervised route binds.
#[cfg(feature = "unix-transport")]
fn declared_service<P: SupervisedProvider>(provider: &P) -> Result<&'static str, ProviderRunError> {
    let mut declared = provider
        .drivers()
        .iter()
        .flat_map(|driver| driver.services.iter());
    let Some(first) = declared.next() else {
        return Err(ProviderRunError::ServiceDeclarationMissing);
    };
    if declared.next().is_some() {
        return Err(ProviderRunError::ServiceDeclarationAmbiguous);
    }
    Ok(first.id)
}

/// The bounded drain budget the base hands a provider it starts.
pub const DEFAULT_DRAIN_BUDGET_MS: u64 = 5_000;

async fn serve_supervised<P: SupervisedProvider>(
    provider: P,
    startup: Option<Arc<dyn StartupStepExecutor>>,
) -> Result<(), ProviderRunError> {
    let clock: SharedClock = Arc::new(SystemClock);
    let lifecycle = Lifecycle::with_clock(provider, clock);
    #[cfg(feature = "unix-transport")]
    {
        serve_supervised_route(lifecycle, startup).await
    }
    #[cfg(not(feature = "unix-transport"))]
    {
        let _ = (lifecycle, startup);
        Err(ProviderRunError::BootstrapUnavailable)
    }
}

#[cfg(feature = "unix-transport")]
async fn serve_supervised_route<P: SupervisedProvider>(
    lifecycle: Lifecycle<P>,
    startup: Option<Arc<dyn StartupStepExecutor>>,
) -> Result<(), ProviderRunError> {
    let declaration = lifecycle.declaration();
    let service = declared_service(lifecycle.provider())?;
    let provider_ref = ResourceRef::parse(&format!("Provider/{}", declaration.provider_ref))
        .map_err(|_| ProviderRunError::BootstrapRefused)?;
    let spec = fd10::ProviderFd10Spec::new(
        declaration.provider_ref,
        provider_ref.clone(),
        service,
        lifecycle.provider().accepted_purpose(),
    );
    let supervised = fd10::establish_supervised_route(
        spec,
        d2b_session_unix::controller_resource_endpoint_policy(),
        &[],
    )
    .await
    .map_err(|_| ProviderRunError::BootstrapRefused)?;
    let route = supervised.route.clone();
    let mut entrypoint =
        ProviderEntrypoint::with_provider(declaration.provider_ref, provider_ref, service)
            .map_err(|_| ProviderRunError::AdmissionRefused)?;
    if let Some(execution_ref) = route.context().execution_ref().cloned() {
        entrypoint = entrypoint
            .with_execution_target(execution_ref)
            .map_err(|_| ProviderRunError::AdmissionRefused)?;
    }
    if let Some(process_ref) = route.context().process_ref().cloned() {
        entrypoint = entrypoint
            .with_controller_process(process_ref)
            .map_err(|_| ProviderRunError::AdmissionRefused)?;
    }
    if let (Some(provider_generation), Some(controller_generation)) =
        (route.provider_generation(), route.controller_generation())
    {
        entrypoint = entrypoint
            .with_generations(provider_generation, controller_generation)
            .map_err(|_| ProviderRunError::AdmissionRefused)?;
    }
    let registration = entrypoint
        .admit()
        .map_err(|_| ProviderRunError::AdmissionRefused)?;
    let session_admission = entrypoint
        .admit_authenticated(&route)
        .map_err(|_| ProviderRunError::AdmissionRefused)?;
    let plane = lifecycle.plane_handle(route.zone().clone(), lifecycle.provider().plane_port());
    lifecycle
        .attach(&plane)
        .await
        .map_err(ProviderRunError::Attach)?;
    lifecycle
        .run_startup(startup.as_deref())
        .await
        .map_err(ProviderRunError::Startup)?;
    let Some(surface) = lifecycle.provider().service_surface() else {
        lifecycle
            .drain(DEFAULT_DRAIN_BUDGET_MS)
            .await
            .map_err(ProviderRunError::Drain)?;
        return Ok(());
    };
    let services = (surface.services)();
    let driver: Arc<dyn crate::ComponentSessionDriver> = Arc::new(supervised.driver);
    let served = crate::server::serve_authenticated_route(
        entrypoint,
        registration,
        session_admission,
        driver,
        route,
        services,
    )
    .await
    .map_err(|_| ProviderRunError::ServiceLoopFailed);
    let drained = lifecycle.drain(DEFAULT_DRAIN_BUDGET_MS).await;
    served?;
    drained.map_err(ProviderRunError::Drain)
}
