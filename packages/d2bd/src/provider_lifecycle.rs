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
use d2b_provider_host::HOST_EFFECTS_SERVICE;
use d2b_provider_network_local::NETWORK_EFFECTS_SERVICE;
use d2b_provider_process::PROCESS_EFFECTS_SERVICE;
use d2b_provider_process_systemd::effects_service::PROCESS_SYSTEMD_EFFECTS_SERVICE;
use d2b_provider_toolkit::{
    AttachError, Cardinality, DEFAULT_DRAIN_BUDGET_MS, DrainDeadline, DrainError,
    DriverDescriptor, IsolationPosture, Lifecycle, OperationEnvelope, OperationFailure,
    OperationResult, ProviderAgentAuditLog, ProviderBase, ProviderDeclaration, ServiceDecl,
    ZonePlaneHandle,
};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use ractor::{Actor, ActorRef};

use crate::effect_service_actors::{
    EffectServiceBinding, EffectServiceRow, EffectServiceSupervisor, EffectServiceSupervisorArgs,
    EffectServiceSupervisorMsg,
};
use d2b_provider_toolkit::{EffectServiceError, EffectServiceFactory};
use crate::forward_rendezvous::ForwardRendezvous;
use crate::plane_port::{PlaneRefusal, ProductionPlanePort};

/// The declared service one registered service id names, when the daemon
/// hosts it (U15). The registration table carries only ids; the hosting
/// pass resolves the crate-owned declaration so the row can be published
/// with its methods and facets.
fn registered_service_decl(service: &str) -> Option<&'static ServiceDecl> {
    if service == PROCESS_EFFECTS_SERVICE.id {
        Some(&PROCESS_EFFECTS_SERVICE)
    } else if service == NETWORK_EFFECTS_SERVICE.id {
        Some(&NETWORK_EFFECTS_SERVICE)
    } else if service == HOST_EFFECTS_SERVICE.id {
        Some(&HOST_EFFECTS_SERVICE)
    } else if service == PROCESS_SYSTEMD_EFFECTS_SERVICE.id {
        Some(&PROCESS_SYSTEMD_EFFECTS_SERVICE)
    } else {
        None
    }
}

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
    /// A declared effect-service method names a contract facet the hosting
    /// side does not enforce (privileges, payload schema, deadline tier);
    /// the zone must not host a method that would silently run unenforced.
    EffectServiceFacetUnenforced {
        provider_ref: &'static str,
        service: &'static str,
        method: &'static str,
        facet: &'static str,
    },
    /// A registered service id the daemon's composition list does not name.
    /// The registry is the authority for what a registered family serves;
    /// a service it carries that the hand-written declaration list cannot
    /// resolve would otherwise be silently neither published nor refused,
    /// so the zone refuses startup instead of composing nothing.
    EffectServiceRegistrationUnknown {
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
            Self::EffectServiceFacetUnenforced { .. } => "effect-service-facet-unenforced",
            Self::EffectServiceRegistrationUnknown { .. } => {
                "effect-service-registration-unknown"
            }
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
            | Self::EffectServiceFactoryMissing { provider_ref, .. }
            | Self::EffectServiceFacetUnenforced { provider_ref, .. }
            | Self::EffectServiceRegistrationUnknown { provider_ref, .. } => provider_ref,
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
            Self::EffectServiceFacetUnenforced {
                provider_ref,
                service,
                method,
                facet,
            } => format!(
                "{}:{}:{}:{}:{}",
                self.code(),
                provider_ref,
                service,
                method,
                facet
            ),
            Self::EffectServiceRegistrationUnknown { provider_ref, service } => {
                format!("{}:{}:{}", self.code(), provider_ref, service)
            }
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
    directory: tokio::sync::Mutex<ProviderDirectory>,
    refusal: tokio::sync::Mutex<Option<ProviderStartupError>>,
}

impl DriverRegistrations {
    /// Register every driver one provider declared.
    async fn register(&self, provider_ref: &'static str, drivers: &[DriverDescriptor]) -> Result<(), ()> {
        let mut directory = self.directory.lock().await;
        for driver in drivers {
            if let Err(error) = directory.register_driver(driver) {
                let refusal = ProviderStartupError::Registration {
                    provider_ref,
                    reason: registration_reason(&error),
                };
                let mut slot = self.refusal.lock().await;
                if slot.is_none() {
                    *slot = Some(refusal);
                }
                return Err(());
            }
        }
        Ok(())
    }

    /// Take the assembled directory, once.
    async fn take_directory(&self) -> ProviderDirectory {
        std::mem::take(&mut *self.directory.lock().await)
    }

    /// The first registration refusal, if any.
    async fn refusal(&self) -> Option<ProviderStartupError> {
        self.refusal.lock().await.clone()
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
            .await
            .map_err(|_| AttachError::Refused)
    }

    /// Release what this provider claimed on the zone plane.
    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError> {
        if deadline.expired() {
            return Err(DrainError::DeadlineExpired);
        }
        self.port.release(self.provider_ref()).await;
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

    /// Run one forwarded invocation under the evidence chain it was
    /// dispatched on and the U10 family seam (the sanctioned seam, U10).
    ///
    /// The chain identities (root first) are the ones the broker minted
    /// for the root call; the family handler presents them - with its own
    /// identity appended - when it invokes a broker-generic kernel as the
    /// nested core of its operation, so the kernel call is authorized
    /// against the chain's initiating principal and the in-broker leg
    /// records the correlation leg (KTD6). The kernel caller carries the
    /// broker socket, the caller role, the Zone's trusted bundle, and the
    /// daemon-side runner lookup.
    pub(crate) async fn invoke_under_chain(
        &self,
        operation: &str,
        invocation_id: &str,
        payload: CanonicalJsonObject,
        fds: &[RawFd],
        chain_identities: &[String],
        kernel: Option<&d2b_resource_types::KernelCaller>,
    ) -> Result<OperationResult, OperationFailure> {
        self.envelope
            .invoke_named_with_fds_under_chain(
                operation,
                invocation_id,
                &self.caller,
                payload,
                fds,
                chain_identities,
                kernel,
            )
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
    pub(crate) fn with_effect_service_factory(
        mut self,
        service: &'static str,
        factory: Arc<dyn EffectServiceFactory>,
    ) -> Self {
        self.effect_service_factories.insert(service, factory);
        self
    }

    /// Supply the hosting factories the composition root registered for the
    /// plane's declared services (U3, R5): one entry per service identity.
    ///
    /// The composition root feeds this seam from its inputs; a declared
    /// service with no entry still refuses startup by name.
    pub(crate) fn with_effect_service_factories(
        mut self,
        factories: &BTreeMap<&'static str, Arc<dyn EffectServiceFactory>>,
    ) -> Self {
        for (service, factory) in factories {
            self = self.with_effect_service_factory(service, Arc::clone(factory));
        }
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
                        // The hosting side enforces the method-operation
                        // lookup, the fd-leg contracts, and the canonical
                        // payload parse - nothing else. A method that
                        // declares a facet the host does not enforce
                        // (privileges, a payload schema, a deadline tier)
                        // would run silently unenforced, so it refuses
                        // startup by name instead of half-hosting.
                        for method in service.methods {
                            let facet = if !method.privileges.is_empty() {
                                Some("privileges")
                            } else if method.payload_schema.is_some() {
                                Some("payload-schema")
                            } else if method.deadline_tier.is_some() {
                                Some("deadline-tier")
                            } else {
                                None
                            };
                            if let Some(facet) = facet {
                                return Err(ProviderStartupError::EffectServiceFacetUnenforced {
                                    provider_ref: declaration.provider_ref,
                                    service: service.id,
                                    method: method.name,
                                    facet,
                                });
                            }
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
            // U15: a registered family whose service no driver declares
            // (the process-systemd family hosts no plane resource type of
            // its own) still publishes its registered service: the
            // registration table carries the row, and the composition
            // point hosts it from the declared factory over the
            // registered service identity - the daemon names no family
            // string, only the crate's declared service id.
            for registration in crate::resource_plane_v3::PROVIDER_REGISTRATIONS {
                for &service in registration.services {
                    if owners.contains_key(service) {
                        continue;
                    }
                    // The registry is the authority: a registered service
                    // the hand-written declaration list cannot resolve is a
                    // drift that would otherwise be silently neither
                    // published nor refused - refuse startup by name.
                    let Some(decl) = registered_service_decl(service) else {
                        return Err(ProviderStartupError::EffectServiceRegistrationUnknown {
                            provider_ref: registration.provider_ref,
                            service,
                        });
                    };
                    owners.insert(service, registration.provider_ref);
                    for method in decl.methods {
                        let facet = if !method.privileges.is_empty() {
                            Some("privileges")
                        } else if method.payload_schema.is_some() {
                            Some("payload-schema")
                        } else if method.deadline_tier.is_some() {
                            Some("deadline-tier")
                        } else {
                            None
                        };
                        if let Some(facet) = facet {
                            return Err(ProviderStartupError::EffectServiceFacetUnenforced {
                                provider_ref: registration.provider_ref,
                                service: decl.id,
                                method: method.name,
                                facet,
                            });
                        }
                    }
                    let Some(factory) = effect_service_factories.get(service) else {
                        return Err(ProviderStartupError::EffectServiceFactoryMissing {
                            provider_ref: registration.provider_ref,
                            service,
                        });
                    };
                    effect_services.push(EffectServiceRow::declared(
                        zone.as_str(),
                        decl,
                        Arc::clone(factory),
                    ));
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
            let plane_refusal = registrations.refusal().await;
            attached.map_err(|error| match error {
                AttachError::Plane(_) => match port.refusal() {
                    Some(refusal) => ProviderStartupError::Plane(refusal),
                    None => ProviderStartupError::Attach {
                        provider_ref: provider.provider_ref(),
                    },
                },
                AttachError::Refused => plane_refusal.unwrap_or(ProviderStartupError::Attach {
                    provider_ref: provider.provider_ref(),
                }),
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
            drain_order: tokio::sync::Mutex::new(Vec::new()),
            directory: registrations.take_directory().await,
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
    drain_order: tokio::sync::Mutex<Vec<&'static str>>,
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
        // Synchronous tracing/test surface over the `tokio::sync` drain
        // ledger (plan U4): a concurrent drain-writer yields an empty view
        // (fail-closed) instead of blocking the tracing path.
        self.drain_order
            .try_lock()
            .ok()
            .map(|order| order.clone())
            .unwrap_or_default()
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
    /// resolution stays the supervision-test harness surface.
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
    /// The republish seam is composition-side; until the dialogue that
    /// re-drives declared rows on a provider-set republish lands, the
    /// hosting-site tests are its only consumers.
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
            self.drain_order.lock().await.push(provider.provider_ref());
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    use crate::effect_service_actors::ServiceCallData;
    use d2b_contracts_resource::v3::CanonicalJsonObject;
    use d2b_provider_toolkit::{
        EffectResponse, EffectService, EffectServiceError, ServiceInvocation,
    };
    use d2b_resource_runtime::context::{ManagerEndpoint, ServiceResourceContext, SpecDecoder};
    use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
    use d2b_resource_types::{AllowedSources, MethodFdContract, ServiceMethod, WellKnownType};

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
            invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            Ok(EffectResponse::new(invocation.payload.clone()))
        }
    }

    /// One call's data for the hosted fixture services: the real envelope
    /// payload and a fail-closed driver context (no manager seam at the
    /// hosting site).
    fn call_data(payload: CanonicalJsonObject) -> ServiceCallData {
        ServiceCallData {
            zone: zone().as_str().to_owned(),
            invocation_id: "invocation-test".to_owned(),
            payload,
            resources: ServiceResourceContext::fail_closed(),
            method: ECHO_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        }
    }

    /// The canonical payload of one fixture call.
    fn payload(value: serde_json::Value) -> CanonicalJsonObject {
        serde_json::from_value(value).expect("canonical payload")
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

    /// Returns one pre-built service (state-reading fixtures).
    struct OnceFactory(Arc<dyn EffectService>);

    impl EffectServiceFactory for OnceFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            self.0.clone()
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
        let response = binding
            .call(call_data(payload(serde_json::json!({ "echo": "ping" }))))
            .await
            .expect("call");
        assert_eq!(
            response.payload,
            payload(serde_json::json!({ "echo": "ping" })),
            "the hosted actor answered"
        );
        assert_eq!(builds.load(Ordering::SeqCst), 1, "built once from the durable row");
    }

    /// U8 error path, `manager.rs:1190-1211` semantics: killing the actor
    /// mid-supervision respawns the service from its durable row, the next
    /// call succeeds, and the generational revision bumped. The pre-crash
    /// binding refuses as stale (KTD5).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
        let response = binding
            .call(call_data(payload(serde_json::json!({ "echo": "one" }))))
            .await
            .expect("call");
        assert_eq!(response.payload, payload(serde_json::json!({ "echo": "one" })));
        let revision_before = binding.revision();
        let id_before = binding.actor_id();

        // Kill the actor mid-supervision (aborts any in-flight work).
        binding.kill();

        // The zone supervisor respawns from the durable row and bumps the
        // generational revision - observable through the shared counter.
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
            .call_expected(
                revision_before,
                call_data(payload(serde_json::json!({ "echo": "stale" }))),
            )
            .await
            .expect_err("stale revision must refuse");
        assert!(matches!(stale, EffectServiceError::StaleRevision { .. }), "got {stale:?}");
        let dead = binding
            .call(call_data(payload(serde_json::json!({ "echo": "dead" }))))
            .await
            .expect_err("dead actor must refuse");
        assert!(
            matches!(dead, EffectServiceError::ServiceUnavailable { .. }),
            "got {dead:?}"
        );

        // The next call succeeds against the respawned generation.
        let response = respawned
            .call(call_data(payload(serde_json::json!({ "echo": "two" }))))
            .await
            .expect("call after respawn");
        assert_eq!(response.payload, payload(serde_json::json!({ "echo": "two" })));
    }

    /// U8 edge: a republish through the hosting API bumps the generational
    /// revision and rebuilds the actor from the new row (provider-set
    /// republish, KTD5).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

        let response = rebound
            .call(call_data(payload(serde_json::json!({ "echo": "again" }))))
            .await
            .expect("call after republish");
        assert_eq!(response.payload, payload(serde_json::json!({ "echo": "again" })));
    }

    /// A declared service identity repeated across providers refuses
    /// startup: the session layer resolves a service to its one declaring
    /// driver, so the zone cannot host two actors for one id.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    /// A declared method naming a facet the host does not enforce
    /// (privileges, a payload schema, a deadline tier) refuses startup by
    /// name: it would otherwise run silently unenforced between envelope
    /// admission and the handler.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_method_declaring_unenforced_facets_refuses_startup_named() {
        const PRIVILEGED_SERVICE: ServiceDecl = ServiceDecl {
            id: "fixture.privileged",
            methods: &[ServiceMethod {
                name: "admin",
                operation: None,
                payload_schema: None,
                request_fds: MethodFdContract::NONE,
                response_fds: MethodFdContract::NONE,
                state_cells: &[],
                privileges: &["zone-admin"],
                deadline_tier: None,
            }],
            attach_kinds: &[],
            streams: &[],
            endpoint_policy: None,
        };
        const SCHEMA_SERVICE: ServiceDecl = ServiceDecl {
            id: "fixture.schema",
            methods: &[ServiceMethod {
                name: "admin",
                operation: None,
                payload_schema: Some("fixture.admin.schema"),
                request_fds: MethodFdContract::NONE,
                response_fds: MethodFdContract::NONE,
                state_cells: &[],
                privileges: &[],
                deadline_tier: None,
            }],
            attach_kinds: &[],
            streams: &[],
            endpoint_policy: None,
        };
        const TIERED_SERVICE: ServiceDecl = ServiceDecl {
            id: "fixture.tiered",
            methods: &[ServiceMethod {
                name: "admin",
                operation: None,
                payload_schema: None,
                request_fds: MethodFdContract::NONE,
                response_fds: MethodFdContract::NONE,
                state_cells: &[],
                privileges: &[],
                deadline_tier: Some("fast"),
            }],
            attach_kinds: &[],
            streams: &[],
            endpoint_policy: None,
        };
        for (service, facet) in [
            (&PRIVILEGED_SERVICE, "privileges"),
            (&SCHEMA_SERVICE, "payload-schema"),
            (&TIERED_SERVICE, "deadline-tier"),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let error = ProviderSet::new(zone(), dir.path().to_path_buf())
                .with(
                    declared("fixture"),
                    vec![serving_descriptor(std::slice::from_ref(service))],
                )
                .with_effect_service_factory(
                    service.id,
                    Arc::new(EchoFactory { builds: Arc::new(AtomicU64::new(0)) }),
                )
                .start()
                .await
                .expect_err("a declared facet cannot run unenforced");
            assert_eq!(error.code(), "effect-service-facet-unenforced");
            assert_eq!(
                error.message(),
                format!(
                    "effect-service-facet-unenforced:fixture:{}:admin:{facet}",
                    service.id
                ),
                "declared {facet}"
            );
        }
    }

    // ---- U3 real payload and capability object (R6, R7, R8) ----

    /// Answers one fixed view for the row the state-reading service asks
    /// for; every other manager call is unreachable at the hosting site.
    struct FixedViewManager;

    #[async_trait]
    impl ManagerEndpoint for FixedViewManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: d2b_resource_runtime::context::ChildEnsure,
        ) -> Result<d2b_resource_runtime::spec_store::EnsureOutcome, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the hosting site ensures no children")
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::identity::StoredDesiredResource>, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the hosting site reads no stored rows")
        }

        async fn view(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, d2b_resource_runtime::error::ResourceError>
        {
            Ok(Some(d2b_resource_runtime::manager::ResourceView {
                key: key.clone(),
                uid: [7; 16],
                generation: 42,
                deleting: false,
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Api,
                spec: Vec::new(),
                metadata: Vec::new(),
                owner_key: None,
                status: None,
                status_generation: None,
                status_projection: None,
            }))
        }

        async fn delete(
            &self,
            _key: &ResourceKey,
        ) -> Result<(), d2b_resource_runtime::error::ResourceError> {
            unreachable!("the hosting site deletes no rows")
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<d2b_resource_runtime::identity::StoredDesiredResource>, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the hosting site lists no owned rows")
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: d2b_resource_runtime::context::WatchRegistration,
        ) -> Result<d2b_resource_runtime::context::WatchId, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the hosting site registers no watches")
        }

        async fn cancel_watch(
            &self,
            _watch: d2b_resource_runtime::context::WatchId,
        ) -> Result<(), d2b_resource_runtime::error::ResourceError> {
            unreachable!("the hosting site cancels no watches")
        }
    }

    /// Reads one row through the driver context and answers with the
    /// observed generation: the service reaches resource state only through
    /// the generic driver context (R7), never a daemon state type.
    struct StateReadingService;

    #[async_trait]
    impl EffectService for StateReadingService {
        async fn handle(
            &self,
            invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            let key = ResourceKey::new(invocation.zone, "Process", "worker-0");
            match invocation.resources.view(&key).await {
                Ok(Some(view)) => Ok(EffectResponse::new(payload(serde_json::json!({
                    "generation": view.generation,
                })))),
                Ok(None) => Err(EffectServiceError::Declined {
                    service: "fixture.state".to_owned(),
                    reason: "row absent".to_owned(),
                }),
                Err(_) => Err(EffectServiceError::Declined {
                    service: "fixture.state".to_owned(),
                    reason: "read refused".to_owned(),
                }),
            }
        }
    }

    /// U3 happy path (AE3): a hosted service answers an invocation carrying
    /// the real envelope payload and reaches resource state through the
    /// generic driver context - the capability object's driver-context
    /// facet, not a daemon state type.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hosted_service_reaches_state_through_the_driver_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(OnceFactory(Arc::new(StateReadingService))),
            )
            .start()
            .await
            .expect("the provider starts through the base");

        let binding = runtime
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");
        let call = ServiceCallData {
            zone: zone().as_str().to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload: payload(serde_json::json!({ "echo": "ping" })),
            resources: ServiceResourceContext::over(Arc::new(FixedViewManager)),
            method: ECHO_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            payload(serde_json::json!({ "generation": 42 })),
            "the service read the row through the driver context"
        );
    }

    /// KTD8 restart adoption: a restarted provider set re-hosts its
    /// declared service from the durable declaration and the fresh
    /// generation answers - the surface this unit moves adopts on restart.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_provider_set_rehosts_its_declared_services() {
        let dir = tempfile::tempdir().expect("tempdir");
        let builds = Arc::new(AtomicU64::new(0));
        let started = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(EchoFactory { builds: Arc::clone(&builds) }),
            )
            .start()
            .await
            .expect("the first generation starts");
        let binding = started
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");
        let response = binding
            .call(call_data(payload(serde_json::json!({ "echo": "before" }))))
            .await
            .expect("call");
        assert_eq!(response.payload, payload(serde_json::json!({ "echo": "before" })));
        drop(started);

        // The daemon restarts: the provider set is rebuilt from the same
        // declaration and factory, and the service is hosted again.
        let restarted = ProviderSet::new(zone(), dir.path().to_path_buf())
            .with(declared("fixture"), vec![serving_descriptor(&[ECHO_SERVICE])])
            .with_effect_service_factory(
                ECHO_SERVICE.id,
                Arc::new(EchoFactory { builds: Arc::clone(&builds) }),
            )
            .start()
            .await
            .expect("the restarted generation starts");
        let adopted = restarted
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the restarted set re-hosts the declared service");
        assert_eq!(adopted.revision(), 1, "the adopted generation starts fresh");
        let response = adopted
            .call(call_data(payload(serde_json::json!({ "echo": "after" }))))
            .await
            .expect("call after restart");
        assert_eq!(response.payload, payload(serde_json::json!({ "echo": "after" })));
    }
}
