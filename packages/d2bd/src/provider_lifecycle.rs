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
            | Self::OperationSurface { provider_ref, .. } => provider_ref,
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

    /// Start every provider through the base.
    pub(crate) async fn start(self) -> Result<ProviderRuntime, ProviderStartupError> {
        let ProviderSet {
            zone,
            state_root,
            providers,
            trusted_context_publication,
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
        Ok(ProviderRuntime {
            zone,
            port,
            providers,
            startup_order,
            drain_order: Mutex::new(Vec::new()),
            directory: registrations.take_directory(),
            operations,
            trusted_context_publication,
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
    use d2b_provider_toolkit::{Cardinality, IsolationPosture, StorageRoot};

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
}
