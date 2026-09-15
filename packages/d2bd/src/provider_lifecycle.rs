//! The providers one zone starts, run through the toolkit base.
//!
//! A provider is instantiated here rather than composed inline: the plane
//! states each provider's declaration and the drivers it serves, and the
//! toolkit lifecycle owns the sequence - claim the declared storage roots,
//! deploy the declared adapters in declared dependency order, publish the
//! declared services, then run the provider's own attach body, which
//! registers the drivers it declared. Nothing else registers a driver, so a
//! provider cannot be started outside the framework and the plane cannot hold
//! a family branch of its own.
//!
//! The startup order is the order the providers are added, which is the order
//! the registry has always been assembled in; drain walks the same list in
//! reverse, so the plane stops what it started in the mirror image of the
//! sequence that started it.
//!
//! Refusals keep their names. A provider whose declaration names no identity,
//! a declaration repeated for one reference, a driver registration the
//! registry refuses, and a plane action the production port refuses all land
//! as one typed failure carrying the provider reference and the row, instead
//! of an anonymous `attach-refused`.

use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    BrokerRequest, BrokerResponse, PublishTrustedContextValues,
};
use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId};
use d2bd_runtime::broker_transport::ModeBoundBrokerAdapter;
use d2bd_runtime::target_runtime::DaemonMode;
use d2b_provider_toolkit::{
    AttachError, Cardinality, DEFAULT_DRAIN_BUDGET_MS, DrainDeadline, DrainError,
    DriverDescriptor, IsolationPosture, Lifecycle, OperationEnvelope, OperationFailure,
    OperationResult, ProviderAgentAuditLog, ProviderBase, ProviderDeclaration, ZonePlaneHandle,
};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use ractor::{Actor, ActorRef};

use crate::effect_service_actors::{
    EffectServiceBinding, EffectServiceError, EffectServiceFactory, EffectServiceRow,
    EffectServiceSupervisor, EffectServiceSupervisorArgs, EffectServiceSupervisorMsg,
};
use crate::forward_rendezvous::ForwardRendezvous;
use crate::plane_port::{PlaneRefusal, ProductionPlanePort};

/// The declaration one driver family makes about its zone plane.
///
/// A family that declares plane facts of its own states them in its own
/// crate; this is the shape for a family whose plane integration is its
/// registered drivers alone - no plane adapters, no principals, and no
/// storage roots of its own. The reference is the family's name, and the
/// concrete `Provider/<name>` a row selects stays a row fact.
pub(crate) const fn family_declaration(provider_ref: &'static str) -> ProviderDeclaration {
    ProviderDeclaration {
        provider_ref,
        self_bindings: &[],
        required: true,
        cardinality: Cardinality::AtMostOne,
        isolation_posture: IsolationPosture::Standard,
        plane_adapters: &[],
        principals: &[],
        storage_roots: &[],
    }
}

/// Why one provider did not start or drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderStartupError {
    /// The declared identity is absent, so the plane cannot name the
    /// provider it starts.
    IdentityMissing,
    /// Two providers were instantiated for one declared reference.
    Duplicate { provider_ref: &'static str },
    /// A driver registration the registry refused.
    Registration {
        provider_ref: &'static str,
        reason: String,
    },
    /// The zone plane refused one declared action.
    Plane(PlaneRefusal),
    /// The provider's own attach body refused.
    Attach { provider_ref: &'static str },
    /// The provider's own drain body refused, or its budget expired.
    Drain {
        provider_ref: &'static str,
        code: &'static str,
    },
    /// The provider's declared operations could not be assembled into the
    /// envelope that serves them.
    OperationSurface {
        provider_ref: &'static str,
        reason: &'static str,
    },
    /// Two providers declared one effect service identity; the zone can
    /// only host one service per id.
    EffectServiceDuplicate {
        service: &'static str,
        first: &'static str,
        second: &'static str,
    },
    /// A declared effect service has no hosting factory at the composition
    /// point; the zone cannot host what it cannot build (fail-closed: a
    /// declared service that would answer nothing must not half-start).
    EffectServiceFactoryMissing {
        provider_ref: &'static str,
        service: &'static str,
    },
    /// The zone's effect-service supervisor could not start (ractor
    /// runtime failure).
    EffectServiceSupervisorRefused { reason: String },
}

impl ProviderStartupError {
    /// The stable lower-kebab reason.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::IdentityMissing => "provider-identity-missing",
            Self::Duplicate { .. } => "provider-duplicate",
            Self::Registration { .. } => "provider-registration-refused",
            Self::Plane(refusal) => refusal.reason,
            Self::Attach { .. } => "attach-refused",
            Self::Drain { code, .. } => code,
            Self::OperationSurface { .. } => "operation-surface-refused",
            Self::EffectServiceDuplicate { .. } => "effect-service-duplicate",
            Self::EffectServiceFactoryMissing { .. } => "effect-service-factory-missing",
            Self::EffectServiceSupervisorRefused { .. } => {
                "effect-service-supervisor-refused"
            }
        }
    }

    /// The provider the failure names.
    pub(crate) const fn provider_ref(&self) -> &'static str {
        match self {
            Self::IdentityMissing => "",
            Self::Duplicate { provider_ref }
            | Self::Registration { provider_ref, .. }
            | Self::Attach { provider_ref }
            | Self::Drain { provider_ref, .. }
            | Self::OperationSurface { provider_ref, .. }
            | Self::EffectServiceFactoryMissing { provider_ref, .. } => provider_ref,
            Self::EffectServiceDuplicate { .. } | Self::EffectServiceSupervisorRefused { .. } => "",
            Self::Plane(refusal) => refusal.provider_ref,
        }
    }

    /// The failure as one line: reason, provider, and the offending row.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::IdentityMissing => self.code().to_owned(),
            Self::Duplicate { provider_ref } => {
                format!("{}:{}", self.code(), provider_ref)
            }
            Self::Registration {
                provider_ref,
                reason,
            } => format!("{}:{}:{}", self.code(), provider_ref, reason),
            Self::OperationSurface {
                provider_ref,
                reason,
            } => format!("{}:{}:{}", self.code(), provider_ref, reason),
            Self::Plane(refusal) => refusal.message(),
            Self::Attach { provider_ref } => format!("{}:{}", self.code(), provider_ref),
            Self::Drain { provider_ref, code } => format!("{code}:{provider_ref}"),
            Self::EffectServiceDuplicate {
                service,
                first,
                second,
            } => format!(
                "{}:{}:{}:{}",
                self.code(),
                service,
                first,
                second
            ),
            Self::EffectServiceFactoryMissing {
                provider_ref,
                service,
            } => format!("{}:{}:{}", self.code(), provider_ref, service),
            Self::EffectServiceSupervisorRefused { reason } => {
                format!("{}:{reason}", self.code())
            }
        }
    }
}

impl core::fmt::Display for ProviderStartupError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for ProviderStartupError {}

/// The warning words that belong to the refused registry error.
fn registration_reason(error: &ProviderDirectoryError) -> String {
    match error {
        ProviderDirectoryError::DuplicateType(type_name) => {
            format!("duplicate-type:{}", type_name.as_str())
        }
        ProviderDirectoryError::UnknownType(type_name) => {
            format!("unknown-type:{}", type_name.as_str())
        }
        ProviderDirectoryError::ForeignOperation {
            operation_ref,
            owner,
        } => format!(
            "foreign-operation:{}:{}",
            operation_ref,
            owner.as_str()
        ),
        ProviderDirectoryError::RequiredBeforeOpen { type_name } => {
            format!("required-before-open:{}", type_name.as_str())
        }
    }
}

/// The plane's driver registry, written by the providers it starts.
///
/// The sink is shared by every provider, so a refusal names the provider that
/// offered the declaration rather than whichever registration happened to
/// collide with it.
#[derive(Default)]
struct DriverRegistrations {
    directory: Mutex<ProviderDirectory>,
    refusal: Mutex<Option<ProviderStartupError>>,
}

impl DriverRegistrations {
    /// Register every driver one provider declared.
    fn register(&self, provider_ref: &'static str, drivers: &[DriverDescriptor]) -> Result<(), ()> {
        let mut directory = self.directory.lock().unwrap_or_else(|poisoned| {
            poisoned.into_inner()
        });
        for driver in drivers {
            if let Err(error) = directory.register_driver(driver) {
                let refusal = ProviderStartupError::Registration {
                    provider_ref,
                    reason: registration_reason(&error),
                };
                let mut slot = self
                    .refusal
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if slot.is_none() {
                    *slot = Some(refusal);
                }
                return Err(());
            }
        }
        Ok(())
    }

    /// Take the assembled directory, once.
    fn take_directory(&self) -> ProviderDirectory {
        std::mem::take(
            &mut *self
                .directory
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// The first registration refusal, if any.
    fn refusal(&self) -> Option<ProviderStartupError> {
        self.refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// One provider the plane starts, as the base sees it.
#[derive(Clone)]
pub(crate) struct ZoneProvider {
    declaration: ProviderDeclaration,
    drivers: Arc<[DriverDescriptor]>,
    registrations: Arc<DriverRegistrations>,
    port: Arc<ProductionPlanePort>,
}

impl ZoneProvider {
    /// The provider's reference.
    pub(crate) const fn provider_ref(&self) -> &'static str {
        self.declaration.provider_ref
    }
}

#[async_trait]
impl ProviderBase for ZoneProvider {
    fn declaration(&self) -> &ProviderDeclaration {
        &self.declaration
    }

    fn drivers(&self) -> &[DriverDescriptor] {
        &self.drivers
    }

    /// The provider's own attach body: register what it declared.
    ///
    /// The plane has already claimed the provider's declared storage roots,
    /// deployed its declared adapters, and published its declared services;
    /// what is left is the provider's own obligation, and the registry is the
    /// only thing it writes.
    async fn attach(&self, zone: &ZonePlaneHandle<'_>) -> Result<(), AttachError> {
        if zone.provider_ref() != self.provider_ref() {
            return Err(AttachError::Refused);
        }
        self.registrations
            .register(self.provider_ref(), &self.drivers)
            .map_err(|_| AttachError::Refused)
    }

    /// Release what this provider claimed on the zone plane.
    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError> {
        if deadline.expired() {
            return Err(DrainError::DeadlineExpired);
        }
        self.port.release(self.provider_ref());
        Ok(())
    }
}

/// One started provider's declared operations, behind the toolkit envelope.
///
/// The envelope is built from the provider's own descriptor table, and the
/// only grant it carries is the provider's own declared operations: the
/// forwarded channel's authority is the broker's first-hop authorization
/// against the committed rows, so the provider-side envelope states the one
/// local fact this process owns - this provider may run the handlers it
/// declared - and nothing else. A call that names no declared operation is
/// refused by the envelope before any handler runs.
pub(crate) struct ProviderOperations {
    caller: ResourceRef,
    envelope: Arc<OperationEnvelope>,
}

impl ProviderOperations {
    /// Assemble one provider's declared operations, when it declares any.
    fn over(
        zone: &ZoneId,
        provider: &ZoneProvider,
        audit: &Arc<Mutex<ProviderAgentAuditLog>>,
    ) -> Result<Option<Self>, ProviderStartupError> {
        if provider.drivers.iter().all(|driver| driver.operations.is_empty()) {
            return Ok(None);
        }
        let caller = ResourceRef::parse(&format!("Provider/{}", provider.provider_ref())).map_err(
            |_| ProviderStartupError::OperationSurface {
                provider_ref: provider.provider_ref(),
                reason: "provider-ref-invalid",
            },
        )?;
        let envelope = OperationEnvelope::over(
            zone.clone(),
            caller.clone(),
            &provider.drivers,
            Arc::clone(audit),
        )
        .map_err(|_| ProviderStartupError::OperationSurface {
            provider_ref: provider.provider_ref(),
            reason: "envelope-refused",
        })?;
        for driver in provider.drivers.iter() {
            for declaration in driver.operations {
                envelope.commit_grant(&caller, &declaration.operation_ref);
            }
        }
        Ok(Some(Self {
            caller,
            envelope: Arc::new(envelope),
        }))
    }

    /// Whether this provider declared the operation.
    fn declares(&self, operation: &str) -> bool {
        self.envelope.declares(operation)
    }

    /// Run one forwarded invocation under the identifier the broker minted.
    pub(crate) async fn invoke(
        &self,
        operation: &str,
        invocation_id: &str,
        payload: CanonicalJsonObject,
        fds: &[RawFd],
    ) -> Result<OperationResult, OperationFailure> {
        self.envelope
            .invoke_named_with_fds(operation, invocation_id, &self.caller, payload, fds)
            .await
    }
}

/// The round trip budget for one trusted-context publication over the
/// origination leg: the same order as the daemon's other broker clients.
const TRUSTED_CONTEXT_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(10);

/// The requeue cadence for a hosted effect service's timer-driven poll tick
/// (U8, KTD5: `ractor::time` timers, never threads). The composition point
/// sets the cadence; the fixture `poll` is a no-op, so the default cadence
/// costs nothing until a service declares one.
const EFFECT_SERVICE_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// The base guest generation every Zone's fleet is created from.
///
/// The daemon does not yet track zone-level guest generation movement; the
/// publication carries the served fleet base, and zone freshness between
/// planes is enforced by the provider-set revision, not by this base. When
/// generation movement is tracked, the moving value is published here.
const GUEST_FLEET_BASE_GENERATION: u64 = 1;

/// The daemon's publication binding for one Zone's trusted-context values.
///
/// Carries everything the origination leg needs to publish the Zone's
/// attestation values to the broker: the mode-bound broker adapter (the
/// broker socket and the caller role the daemon presents), and the
/// controller and guest generations the Zone currently serves. The Zone and
/// the provider-set revision are supplied at publication time - the
/// revision must be the rendezvous binding's current revision, which only
/// the rendezvous assigns.
#[derive(Clone)]
pub(crate) struct TrustedContextPublication {
    adapter: ModeBoundBrokerAdapter,
    controller_generation: u64,
    guest_generation: u64,
}

impl TrustedContextPublication {
    /// The production binding for one Zone's plane.
    ///
    /// The controller generation is the Zone authority's; the guest
    /// generation is the Zone's served fleet base ([`GUEST_FLEET_BASE_GENERATION`]).
    pub(crate) fn production(
        mode: DaemonMode,
        broker_socket: impl Into<PathBuf>,
        daemon_uid: u32,
        controller_generation: u64,
    ) -> Self {
        Self {
            adapter: ModeBoundBrokerAdapter::for_mode(mode, broker_socket, daemon_uid),
            controller_generation,
            guest_generation: GUEST_FLEET_BASE_GENERATION,
        }
    }
}

/// The providers one plane starts, in the order they start.
pub(crate) struct ProviderSet {
    zone: ZoneId,
    state_root: PathBuf,
    providers: Vec<(ProviderDeclaration, Vec<DriverDescriptor>)>,
    trusted_context_publication: Option<TrustedContextPublication>,
    /// Hosting factories for the declared effect services, keyed by service
    /// identity (U8, KTD5). A declared service without a factory refuses
    /// startup: the zone cannot host what it cannot build.
    effect_service_factories: BTreeMap<&'static str, Arc<dyn EffectServiceFactory>>,
}

impl ProviderSet {
    /// Open a set for one zone, rooted at the zone's daemon-owned state
    /// directory.
    pub(crate) fn new(zone: ZoneId, state_root: PathBuf) -> Self {
        Self {
            zone,
            state_root,
            providers: Vec::new(),
            trusted_context_publication: None,
            effect_service_factories: BTreeMap::new(),
        }
    }

    /// Bind this set's publication over the origination leg.
    ///
    /// A set started without a publication binding (tests, context-free
    /// deployments) publishes nothing: the rendezvous stays fail-closed
    /// until a broker acknowledgement lands.
    pub(crate) fn with_trusted_context_publication(
        mut self,
        publication: Option<TrustedContextPublication>,
    ) -> Self {
        self.trusted_context_publication = publication;
        self
    }

    /// Add the next provider: its declaration and the drivers it serves.
    ///
    /// The order of these calls is the startup order.
    pub(crate) fn with(
        mut self,
        declaration: ProviderDeclaration,
        drivers: Vec<DriverDescriptor>,
    ) -> Self {
        self.providers.push((declaration, drivers));
        self
    }

    /// Supply the hosting factory for one declared effect service (U8).
    ///
    /// The factory builds the service instance from the durable row; a
    /// respawn calls `build` again, exactly like `ResourceManager`
    /// re-creates its drivers from the committed spec row. A provider that
    /// declares a service without a factory refuses startup.
    ///
    /// Composition feeds this seam when a family declares services (U10
    /// pilot and later); until then the hosting-site tests are its only
    /// consumers (U9+ note: the composition seam owns this call).
    #[allow(dead_code)]
    pub(crate) fn with_effect_service_factory(
        mut self,
        service: &'static str,
        factory: Arc<dyn EffectServiceFactory>,
    ) -> Self {
        self.effect_service_factories.insert(service, factory);
        self
    }

    /// Start every provider through the base.
    pub(crate) async fn start(self) -> Result<ProviderRuntime, ProviderStartupError> {
        let ProviderSet {
            zone,
            state_root,
            providers,
            trusted_context_publication,
            effect_service_factories,
        } = self;
        let mut seen: BTreeMap<&'static str, ()> = BTreeMap::new();
        for (declaration, _) in &providers {
            if declaration.provider_ref.is_empty() {
                return Err(ProviderStartupError::IdentityMissing);
            }
            if seen.insert(declaration.provider_ref, ()).is_some() {
                return Err(ProviderStartupError::Duplicate {
                    provider_ref: declaration.provider_ref,
                });
            }
        }
        let declarations: Vec<(ProviderDeclaration, Arc<[DriverDescriptor]>)> = providers
            .into_iter()
            .map(|(declaration, drivers)| (declaration, Arc::<[DriverDescriptor]>::from(drivers)))
            .collect();
        // The effect services this zone hosts (U8, KTD5): one durable row
        // per DECLARED service, in provider/driver/service declaration
        // order. The session layer resolves a service to its declaring
        // driver, so one service identity may be declared once per zone; a
        // repeated id is a composition error, and a declared service
        // without a hosting factory is refused rather than half-hosted.
        let mut effect_services = Vec::new();
        {
            let mut owners: BTreeMap<&'static str, &'static str> = BTreeMap::new();
            for (declaration, drivers) in &declarations {
                for driver in drivers.iter() {
                    for service in driver.services {
                        if let Some(first) = owners.insert(service.id, declaration.provider_ref) {
                            return Err(ProviderStartupError::EffectServiceDuplicate {
                                service: service.id,
                                first,
                                second: declaration.provider_ref,
                            });
                        }
                        let Some(factory) = effect_service_factories.get(&service.id) else {
                            return Err(ProviderStartupError::EffectServiceFactoryMissing {
                                provider_ref: declaration.provider_ref,
                                service: service.id,
                            });
                        };
                        effect_services.push(EffectServiceRow::declared(
                            zone.as_str(),
                            service,
                            Arc::clone(factory),
                        ));
                    }
                }
            }
        }
        let port = Arc::new(ProductionPlanePort::over(
            zone.clone(),
            state_root,
            &declarations,
        ));
        let port_handle: Arc<dyn d2b_provider_toolkit::ZonePlanePort> = port.clone();
        let registrations = Arc::new(DriverRegistrations::default());
        let audit = Arc::new(Mutex::new(ProviderAgentAuditLog::new()));
        let mut providers = Vec::with_capacity(declarations.len());
        let mut startup_order = Vec::with_capacity(declarations.len());
        let mut operations = Vec::new();
        for (declaration, drivers) in declarations {
            let provider = ZoneProvider {
                declaration,
                drivers,
                registrations: Arc::clone(&registrations),
                port: Arc::clone(&port),
            };
            let lifecycle = Lifecycle::new(provider.clone());
            let attached = {
                let handle = lifecycle.plane_handle(zone.clone(), Arc::clone(&port_handle));
                lifecycle.attach(&handle).await
            };
            attached.map_err(|error| match error {
                AttachError::Plane(_) => match port.refusal() {
                    Some(refusal) => ProviderStartupError::Plane(refusal),
                    None => ProviderStartupError::Attach {
                        provider_ref: provider.provider_ref(),
                    },
                },
                AttachError::Refused => registrations.refusal().unwrap_or(
                    ProviderStartupError::Attach {
                        provider_ref: provider.provider_ref(),
                    },
                ),
            })?;
            if let Some(surface) = ProviderOperations::over(&zone, &provider, &audit)? {
                operations.push(surface);
            }
            startup_order.push(provider.provider_ref());
            providers.push(provider);
        }
        // Start the zone's effect-service supervisor at the same composition
        // point the plane published the declared services (U8, KTD5): one
        // linked ractor actor per declared service, respawned from its
        // durable row, requeue timers on `ractor::time`. The supervisor is
        // the hosting anchor for the U8b rendezvous binding, so it starts
        // even when the set declares no services yet.
        let effect_services = Actor::spawn(
            None,
            EffectServiceSupervisor::new(),
            EffectServiceSupervisorArgs {
                zone: zone.as_str().to_owned(),
                rows: effect_services,
                poll_interval: EFFECT_SERVICE_POLL_INTERVAL,
            },
        )
        .await
        .map_err(|error| ProviderStartupError::EffectServiceSupervisorRefused {
            reason: error.to_string(),
        })?
        .0;
        Ok(ProviderRuntime {
            zone,
            port,
            providers,
            startup_order,
            drain_order: Mutex::new(Vec::new()),
            directory: registrations.take_directory(),
            operations,
            trusted_context_publication,
            effect_services,
        })
    }
}

/// The providers one zone is running.
pub(crate) struct ProviderRuntime {
    zone: ZoneId,
    port: Arc<ProductionPlanePort>,
    providers: Vec<ZoneProvider>,
    startup_order: Vec<&'static str>,
    drain_order: Mutex<Vec<&'static str>>,
    directory: ProviderDirectory,
    /// The operation surfaces of the providers that declared operations.
    operations: Vec<ProviderOperations>,
    /// The origination-leg binding this set carries, when one is bound.
    trusted_context_publication: Option<TrustedContextPublication>,
    /// The zone's effect-service supervisor (U8, KTD5): one linked ractor
    /// actor per declared effect service, respawned from its durable row.
    /// The rendezvous binding resolves live bindings through this
    /// ([`Self::resolve_effect_service`]).
    effect_services: ActorRef<EffectServiceSupervisorMsg>,
}

impl core::fmt::Debug for ProviderRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProviderRuntime")
            .field("startup_order", &self.startup_order)
            .finish_non_exhaustive()
    }
}

impl ProviderRuntime {
    /// The providers in the order they started.
    pub(crate) fn startup_order(&self) -> &[&'static str] {
        &self.startup_order
    }

    /// The providers in the order they drained, once drained.
    pub(crate) fn drain_order(&self) -> Vec<&'static str> {
        self.drain_order
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The claimed storage roots of every started provider.
    pub(crate) fn claimed_roots(&self) -> Vec<crate::plane_port::ClaimedRoot> {
        self.port.claimed_roots()
    }

    /// The deployed adapters of every started provider.
    pub(crate) fn deployed_adapters(&self) -> Vec<(&'static str, &'static str)> {
        self.port.deployed_adapters()
    }

    /// The published services of every started provider.
    pub(crate) fn published_services(&self) -> Vec<(&'static str, &'static str)> {
        self.port.published_services()
    }

    /// Take the assembled driver registry.
    pub(crate) fn take_directory(&mut self) -> ProviderDirectory {
        std::mem::take(&mut self.directory)
    }

    /// The provider that declares one operation, when one does.
    ///
    /// The handler table is the started providers' own descriptor tables, so
    /// the lookup can only answer for an operation a provider actually
    /// declared - there is no second registration step to fall out of sync.
    pub(crate) fn declaring_provider(&self, operation: &str) -> Option<&ProviderOperations> {
        self.operations
            .iter()
            .find(|provider| provider.declares(operation))
    }

    /// Resolve the live binding of one declared effect service (U8, KTD5).
    ///
    /// The binding carries the service's generational revision and the live
    /// actor; a respawn or republish bumps the revision, so a caller that
    /// captured `revision()` can refuse stale traffic. The rendezvous
    /// resolves every effect-service dispatch by operation
    /// ([`Self::resolve_effect_service_for_operation`]); name-based
    /// resolution stays the supervision-test harness surface. U9+ note: a
    /// composition/operator dialogue re-drives rows by name and consumes
    /// this.
    #[allow(dead_code)]
    pub(crate) async fn resolve_effect_service(
        &self,
        service: &str,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.effect_services
            .send_message(EffectServiceSupervisorMsg::Resolve {
                service: service.to_owned(),
                reply: reply_tx,
            })
            .map_err(|_| EffectServiceError::ServiceUnavailable {
                service: service.to_owned(),
            })?;
        reply_rx
            .await
            .map_err(|_| EffectServiceError::ServiceUnavailable {
                service: service.to_owned(),
            })?
    }

    /// Resolve the live binding of the effect service that declares one
    /// operation (KD6, U7).
    ///
    /// A forwarded call names the committed operation row; a service
    /// declares that operation as the surface of one of its methods, and
    /// the declaring service's current binding answers it. The rendezvous
    /// uses this resolution for every effect-service dispatch; an
    /// operation no hosted service declares answers
    /// [`EffectServiceError::OperationUnserved`] and falls through to the
    /// provider operation tables.
    pub(crate) async fn resolve_effect_service_for_operation(
        &self,
        operation: &str,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.effect_services
            .send_message(EffectServiceSupervisorMsg::ResolveOperation {
                operation: operation.to_owned(),
                reply: reply_tx,
            })
            .map_err(|_| EffectServiceError::ServiceUnavailable {
                service: operation.to_owned(),
            })?;
        reply_rx
            .await
            .map_err(|_| EffectServiceError::ServiceUnavailable {
                service: operation.to_owned(),
            })?
    }

    /// (Re)publish one effect service row on the zone's supervisor, taking
    /// effect at the composition point the plane publishes declared
    /// services. A republish of a live service bumps the generational
    /// binding revision and rebuilds the actor from the new row (KTD5).
    ///
    /// The republish seam is composition-side; the dialogue that re-drives
    /// declared rows on a provider-set republish lands with the composition
    /// work (U10 pilot and later). Until then the hosting-site tests are
    /// its only consumers (U9+ note: the composition seam owns this call).
    #[allow(dead_code)]
    pub(crate) async fn publish_effect_service(
        &self,
        row: EffectServiceRow,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let service = row.service.clone();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.effect_services
            .send_message(EffectServiceSupervisorMsg::Publish {
                row,
                reply: reply_tx,
            })
            .map_err(|_| EffectServiceError::ServiceUnavailable { service: service.clone() })?;
        reply_rx
            .await
            .map_err(|_| EffectServiceError::ServiceUnavailable { service })?
    }

    /// Publish this Zone's trusted-context values to the broker over the
    /// origination leg, and advance the rendezvous exactly on the
    /// acknowledged epoch.
    ///
    /// Called once per provider-set publication, after the rendezvous
    /// assigned the revision, so the broker caches the SAME revision the
    /// rendezvous validates against. Fail-closed: a transport error, a
    /// broker refusal, or an unexpected reply shape leaves the
    /// rendezvous's epoch and generations untouched - no acknowledgement,
    /// no trust.
    pub(crate) fn publish_trusted_context(
        &self,
        rendezvous: &ForwardRendezvous,
        provider_set_revision: u64,
    ) {
        let Some(publication) = &self.trusted_context_publication else {
            // No binding: nothing is published, and the rendezvous keeps
            // its fail-closed zero epoch.
            return;
        };
        let request = BrokerRequest::PublishTrustedContext(PublishTrustedContextValues {
            zone: self.zone.as_str().to_owned(),
            provider_set_revision,
            controller_generation: publication.controller_generation,
            guest_generation: publication.guest_generation,
        });
        match publication
            .adapter
            .dispatch(request, Some(TRUSTED_CONTEXT_PUBLICATION_TIMEOUT))
        {
            Ok(BrokerResponse::PublishTrustedContext(reply)) => {
                rendezvous.publish_generations(
                    self.zone.as_str(),
                    publication.controller_generation,
                    publication.guest_generation,
                );
                rendezvous.set_broker_epoch(reply.broker_epoch);
            }
            Ok(_) => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    revision = provider_set_revision,
                    operation = "PublishTrustedContext",
                    "trusted-context publication answered with an unexpected response"
                );
            }
            Err(error) => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    revision = provider_set_revision,
                    operation = "PublishTrustedContext",
                    "trusted-context publication refused: {error}"
                );
            }
        }
    }

    /// Drain every provider, in the reverse of the order they started.
    pub(crate) async fn drain(&self) -> Result<(), ProviderStartupError> {
        for provider in self.providers.iter().rev() {
            let lifecycle = Lifecycle::new(provider.clone());
            lifecycle
                .drain(DEFAULT_DRAIN_BUDGET_MS)
                .await
                .map_err(|error| ProviderStartupError::Drain {
                    provider_ref: provider.provider_ref(),
                    code: error.code(),
                })?;
            self.drain_order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(provider.provider_ref());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_provider_toolkit::{Cardinality, IsolationPosture, ServiceDecl, StorageRoot};

    fn zone() -> ZoneId {
        ZoneId::parse("test").expect("a zone label")
    }

    fn declared(provider_ref: &'static str) -> ProviderDeclaration {
        ProviderDeclaration {
            provider_ref,
            self_bindings: &[],
            required: true,
            cardinality: Cardinality::AtMostOne,
            isolation_posture: IsolationPosture::Standard,
            plane_adapters: &[],
            principals: &[],
            storage_roots: &[],
        }
    }

    /// The base starts every provider in the order the set declares and
    /// drains them in the mirror of it.
    #[tokio::test(flavor = "multi_thread")]
    async fn providers_start_in_order_and_drain_in_reverse() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("process"), Vec::new())
            .with(declared("volume"), Vec::new())
            .with(declared("endpoint"), Vec::new())
            .start()
            .await
            .expect("the providers start through the base");
        assert_eq!(runtime.startup_order(), ["process", "volume", "endpoint"]);
        assert!(runtime.drain_order().is_empty());
        runtime.drain().await.expect("the providers drain");
        assert_eq!(runtime.drain_order(), ["endpoint", "volume", "process"]);
    }

    /// A declaration the plane cannot satisfy refuses at startup, naming the
    /// provider and the row: here two providers claim overlapping subtrees.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unsatisfied_declaration_refuses_named() {
        const ROOTS: &[StorageRoot] = &[StorageRoot {
            path: "state",
            provider_owned: true,
        }];
        const NESTED: &[StorageRoot] = &[StorageRoot {
            path: "state/volumes",
            provider_owned: true,
        }];
        let dir = tempfile::tempdir().expect("tempdir");
        let error = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("volume"), Vec::new())
            .with(
                ProviderDeclaration {
                    storage_roots: ROOTS,
                    ..declared("volume-local")
                },
                Vec::new(),
            )
            .with(
                ProviderDeclaration {
                    storage_roots: NESTED,
                    ..declared("volume-binding")
                },
                Vec::new(),
            )
            .start()
            .await
            .expect_err("the subtree belongs to another provider");
        // The overlap is refused against the declared subtrees, so it is the
        // provider that starts first - whichever order the set declares - that
        // is refused, naming its own declared row. The refusal does not depend
        // on which claim happened first.
        assert_eq!(error.code(), "storage-root-overlap");
        assert_eq!(error.provider_ref(), "volume-local");
        assert_eq!(error.message(), "storage-root-overlap:volume-local:state");
    }

    /// A provider whose declaration names no identity does not start.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_declaration_without_an_identity_refuses_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared(""), Vec::new())
            .start()
            .await
            .expect_err("the plane cannot name the provider");
        assert_eq!(error.code(), "provider-identity-missing");
        assert_eq!(error.message(), "provider-identity-missing");
    }

    /// One reference, one provider: a repeated declaration does not start.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_repeated_declaration_refuses_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("volume"), Vec::new())
            .with(declared("volume"), Vec::new())
            .start()
            .await
            .expect_err("one provider per reference");
        assert_eq!(error.code(), "provider-duplicate");
        assert_eq!(error.message(), "provider-duplicate:volume");
    }

    // ---- U8 effect-service hosting (KTD5) ----
    //
    // The plane publishes the declared services; the runtime hosts each one
    // as a linked ractor actor under the zone supervisor, respawned from
    // its durable row. These tests drive the hosting API
    // (`resolve_effect_service`/`publish_effect_service`), mirroring
    // `manager.rs:1190-1211` supervision semantics: build from the durable
    // row, respawn on kill, generational revision bump on respawn and on
    // republish, and a dedicated refusal for stale bindings.

    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::effect_service_actors::{
        EffectRequest, EffectResponse, EffectService,
    };
    use d2b_resource_runtime::context::SpecDecoder;
    use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
    use d2b_resource_types::{AllowedSources, ServiceMethod, WellKnownType};

    /// The declared service the hosting tests host.
    const ECHO_SERVICE: ServiceDecl = ServiceDecl {
        id: "fixture.echo",
        methods: &[ServiceMethod::zone_plane("ping")],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };

    /// A driver that registers cleanly beside its service declaration; its
    /// spec/driver methods are unreachable at the hosting site.
    fn serving_descriptor(services: &'static [ServiceDecl]) -> DriverDescriptor {
        DriverDescriptor {
            resource_type: WellKnownType::PROCESS,
            allowed_sources: AllowedSources::STARTUP,
            verbs: &[],
            execution: &[],
            exportable: false,
            reads: &[],
            operations: &[],
            creations: &[],
            startup: &[],
            services,
            decoder: Arc::new(NoSpecs),
            factory: Arc::new(NoDrivers),
        }
    }

    struct NoSpecs;

    impl SpecDecoder for NoSpecs {
        fn decode(
            &self,
            _envelope: &[u8],
        ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
            unreachable!("the hosting site decodes no specs")
        }
    }

    struct NoDrivers;

    #[async_trait]
    impl ResourceDriverFactory for NoDrivers {
        fn resource_types(&self) -> &[ResourceTypeName] {
            &[]
        }

        async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
            unreachable!("the hosting site creates no resource drivers")
        }
    }

    /// Echo fixture served by the hosted actor (same shape as the harness).
    struct EchoService;

    #[async_trait]
    impl EffectService for EchoService {
        async fn handle(
            &self,
            request: EffectRequest,
        ) -> Result<EffectResponse, EffectServiceError> {
            Ok(request)
        }
    }

    /// Counts rebuilds from the durable row, so a respawn is observable.
    struct EchoFactory {
        builds: Arc<AtomicU64>,
    }

    impl EffectServiceFactory for EchoFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            Arc::new(EchoService)
        }
    }

    async fn until(condition: impl Fn() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never became true within the deadline");
    }

    /// U8 happy path: a declared effect service is hosted as a linked actor
    /// under the zone supervisor at the composition point, answers through
    /// the hosting API, and its durable-row binding starts at revision 1.
    #[tokio::test(flavor = "multi_thread")]
    async fn declared_effect_service_is_hosted_and_answers_through_the_hosting_api() {
        let dir = tempfile::tempdir().expect("tempdir");
        let builds = Arc::new(AtomicU64::new(0));
        let runtime = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(EchoFactory { builds: Arc::clone(&builds) }),
            )
            .start()
            .await
            .expect("the provider starts through the base");
        assert_eq!(
            runtime.published_services(),
            [("fixture", ECHO_SERVICE.id)],
            "the plane published the declared service"
        );

        let binding = runtime
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");
        assert_eq!(binding.revision(), 1, "first generation");
        let response = binding.call(b"ping".to_vec()).await.expect("call");
        assert_eq!(response, b"ping".to_vec(), "the hosted actor answered");
        assert_eq!(builds.load(Ordering::SeqCst), 1, "built once from the durable row");
    }

    /// U8 error path, `manager.rs:1190-1211` semantics: killing the actor
    /// mid-supervision respawns the service from its durable row, the next
    /// call succeeds, and the generational revision bumped. The pre-crash
    /// binding refuses as stale (KTD5).
    #[tokio::test(flavor = "multi_thread")]
    async fn killing_a_hosted_effect_service_respawns_it_from_the_durable_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let builds = Arc::new(AtomicU64::new(0));
        let runtime = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(EchoFactory { builds: Arc::clone(&builds) }),
            )
            .start()
            .await
            .expect("the provider starts through the base");

        let binding = runtime
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");
        let response = binding.call(b"one".to_vec()).await.expect("call");
        assert_eq!(response, b"one".to_vec());
        let revision_before = binding.revision();
        let id_before = binding.actor_id();

        // Kill the actor mid-supervision (aborts any in-flight work).
        binding.kill();

        // The zone supervisor respawns from the durable row and bumps the
        // generational revision — observable through the shared counter.
        until(|| binding.revision() != revision_before).await;
        let respawned = runtime
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("resolve after respawn");
        assert_eq!(
            respawned.revision(),
            revision_before + 1,
            "respawn bumped the revision"
        );
        assert_ne!(respawned.actor_id(), id_before, "respawned actor is a fresh generation");
        assert_eq!(builds.load(Ordering::SeqCst), 2, "rebuilt from the durable row");

        // Stale bindings refuse (KTD5): the captured revision is stale, and
        // the pre-crash handle targets the dead actor.
        let stale = binding
            .call_expected(revision_before, b"stale".to_vec())
            .await
            .expect_err("stale revision must refuse");
        assert!(matches!(stale, EffectServiceError::StaleRevision { .. }), "got {stale:?}");
        let dead = binding.call(b"dead".to_vec()).await.expect_err("dead actor must refuse");
        assert!(
            matches!(dead, EffectServiceError::ServiceUnavailable { .. }),
            "got {dead:?}"
        );

        // The next call succeeds against the respawned generation.
        let response = respawned.call(b"two".to_vec()).await.expect("call after respawn");
        assert_eq!(response, b"two".to_vec());
    }

    /// U8 edge: a republish through the hosting API bumps the generational
    /// revision and rebuilds the actor from the new row (provider-set
    /// republish, KTD5).
    #[tokio::test(flavor = "multi_thread")]
    async fn republishing_a_declared_effect_service_bumps_its_revision() {
        let dir = tempfile::tempdir().expect("tempdir");
        let builds = Arc::new(AtomicU64::new(0));
        let runtime = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(EchoFactory { builds: Arc::clone(&builds) }),
            )
            .start()
            .await
            .expect("the provider starts through the base");

        let first = runtime
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");
        assert_eq!(first.revision(), 1);

        let rebound = runtime
            .publish_effect_service(EffectServiceRow::declared(
                zone().as_str(),
                &ECHO_SERVICE,
                Arc::new(EchoFactory { builds: Arc::clone(&builds) }),
            ))
            .await
            .expect("republish");
        assert_eq!(rebound.revision(), 2, "republish bumped the revision");
        assert_eq!(builds.load(Ordering::SeqCst), 2, "fresh instance from the new row");

        let response = rebound.call(b"again".to_vec()).await.expect("call after republish");
        assert_eq!(response, b"again".to_vec());
    }

    /// A declared service identity repeated across providers refuses
    /// startup: the session layer resolves a service to its one declaring
    /// driver, so the zone cannot host two actors for one id.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_service_declared_twice_refuses_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("first"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with(declared("second"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(EchoFactory {
                    builds: Arc::new(AtomicU64::new(0)),
                }),
            )
            .start()
            .await
            .expect_err("one hosting actor per service id");
        assert_eq!(error.code(), "effect-service-duplicate");
        assert_eq!(
            error.message(),
            "effect-service-duplicate:fixture.echo:first:second"
        );
    }

    /// A provider that declares a service without a hosting factory refuses
    /// startup: the zone cannot host what it cannot build (fail-closed).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_declared_service_without_a_factory_refuses_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .start()
            .await
            .expect_err("a declared service needs a hosting factory");
        assert_eq!(error.code(), "effect-service-factory-missing");
        assert_eq!(
            error.message(),
            "effect-service-factory-missing:fixture:fixture.echo"
        );
    }
}
