//! The zone-plane handle and the bounded drain deadline.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ZoneId;
use d2b_resource_types::{
    PlaneAdapter, PrincipalName, ProviderDeclaration, ServiceDecl, StorageRoot,
};

use super::SharedClock;

/// The fixed drain budget a provider is given when the plane does not declare
/// one.
///
/// The budget is bounded: a provider cannot ask for an unbounded drain, and
/// the plane cannot shrink the deadline below one millisecond.
pub const MAX_DRAIN_BUDGET_MS: u64 = 30_000;

/// Why the plane refused one declared provider action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneError {
    /// No plane port is attached, so the action has no executor.
    Unavailable,
    /// The plane refused the action for a reason it owns.
    Refused,
}

impl PlaneError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "plane-unavailable",
            Self::Refused => "plane-refused",
        }
    }
}

impl fmt::Display for PlaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PlaneError {}

/// The host-side executor of one provider's declared plane actions.
///
/// The composition root implements this against its own state. Every method
/// receives the provider reference and the declaration row being realized, so
/// the implementation never has to guess which action a provider asked for.
#[async_trait]
pub trait ZonePlanePort: Send + Sync + 'static {
    /// Claim one storage root the provider declared.
    async fn claim_storage_root(
        &self,
        provider_ref: &'static str,
        root: &StorageRoot,
    ) -> Result<(), PlaneError>;

    /// Deploy one plane adapter the provider declared.
    async fn deploy_adapter(
        &self,
        provider_ref: &'static str,
        adapter: &PlaneAdapter,
    ) -> Result<(), PlaneError>;

    /// Publish one service the provider declared.
    async fn publish_service(
        &self,
        provider_ref: &'static str,
        service: &ServiceDecl,
    ) -> Result<(), PlaneError>;
}

/// The refusing port.
///
/// A provider process that has no plane attached - a self-check, a guest
/// agent, or a test that only exercises the operation envelope - gets this
/// port, so an attach path that needs the plane fails loudly with
/// [`PlaneError::Unavailable`] instead of silently succeeding.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailablePlanePort;

#[async_trait]
impl ZonePlanePort for UnavailablePlanePort {
    async fn claim_storage_root(
        &self,
        _provider_ref: &'static str,
        _root: &StorageRoot,
    ) -> Result<(), PlaneError> {
        Err(PlaneError::Unavailable)
    }

    async fn deploy_adapter(
        &self,
        _provider_ref: &'static str,
        _adapter: &PlaneAdapter,
    ) -> Result<(), PlaneError> {
        Err(PlaneError::Unavailable)
    }

    async fn publish_service(
        &self,
        _provider_ref: &'static str,
        _service: &ServiceDecl,
    ) -> Result<(), PlaneError> {
        Err(PlaneError::Unavailable)
    }
}

/// The zone-plane handle one provider attaches through.
///
/// It carries only what the provider declared about itself, so an attach body
/// cannot act on a fact the declaration does not state. The declared facts are
/// borrowed from the provider for the handle's lifetime.
pub struct ZonePlaneHandle<'a> {
    zone: ZoneId,
    provider_ref: &'static str,
    adapters: &'static [PlaneAdapter],
    principals: &'static [PrincipalName],
    storage_roots: &'static [StorageRoot],
    services: PlaneServices<'a>,
    port: Arc<dyn ZonePlanePort>,
}

/// The union of every service the provider's drivers declared.
///
/// The handle resolves services from the drivers' declarations rather than
/// from a second table, so a service is published exactly where it is
/// declared.
#[derive(Clone, Copy)]
pub struct PlaneServices<'a> {
    drivers: &'a [d2b_resource_types::DriverDescriptor],
}

impl<'a> PlaneServices<'a> {
    /// Union the service declarations of every driver.
    pub const fn over(drivers: &'a [d2b_resource_types::DriverDescriptor]) -> Self {
        Self { drivers }
    }

    /// Every declared service, in driver declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &'a ServiceDecl> + 'a {
        self.drivers
            .iter()
            .flat_map(|driver| driver.services.iter())
    }

    /// Whether no driver declared a service.
    pub fn is_empty(&self) -> bool {
        self.drivers.iter().all(|driver| driver.services.is_empty())
    }
}

impl fmt::Debug for PlaneServices<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaneServices")
            .field("service_count", &self.iter().count())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for DrainDeadline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DrainDeadline")
            .field("started_unix_ms", &self.started_unix_ms)
            .field("budget_ms", &self.budget_ms)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ZonePlaneHandle<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZonePlaneHandle")
            .field("zone", &self.zone)
            .field("provider_ref", &self.provider_ref)
            .field("adapter_count", &self.adapters.len())
            .field("service_count", &self.services.iter().count())
            .finish_non_exhaustive()
    }
}

impl<'a> ZonePlaneHandle<'a> {
    /// Build the handle for one provider in one zone.
    pub fn new(
        zone: ZoneId,
        declaration: &ProviderDeclaration,
        drivers: &'a [d2b_resource_types::DriverDescriptor],
        port: Arc<dyn ZonePlanePort>,
    ) -> Self {
        Self {
            zone,
            provider_ref: declaration.provider_ref,
            adapters: declaration.plane_adapters,
            principals: declaration.principals,
            storage_roots: declaration.storage_roots,
            services: PlaneServices::over(drivers),
            port,
        }
    }

    /// Build a handle whose plane port refuses every action.
    pub fn unavailable(
        zone: ZoneId,
        declaration: &ProviderDeclaration,
        drivers: &'static [d2b_resource_types::DriverDescriptor],
    ) -> Self {
        Self::new(zone, declaration, drivers, Arc::new(UnavailablePlanePort))
    }

    /// Borrow the zone this provider was placed in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the provider reference the declarations name.
    pub const fn provider_ref(&self) -> &'static str {
        self.provider_ref
    }

    /// Borrow the declared plane adapters.
    pub const fn adapters(&self) -> &'static [PlaneAdapter] {
        self.adapters
    }

    /// Borrow the declared principals, by name only.
    pub const fn principals(&self) -> &'static [PrincipalName] {
        self.principals
    }

    /// Borrow the declared storage roots.
    pub const fn storage_roots(&self) -> &'static [StorageRoot] {
        self.storage_roots
    }

    /// Borrow the services the provider's drivers declared.
    pub const fn services(&self) -> &PlaneServices<'a> {
        &self.services
    }

    /// Claim every declared storage root through the plane port.
    pub async fn claim_declared_storage_roots(&self) -> Result<(), PlaneError> {
        for root in self.storage_roots {
            self.port
                .claim_storage_root(self.provider_ref, root)
                .await?;
        }
        Ok(())
    }

    /// Deploy every declared adapter through the plane port, in declared
    /// dependency order.
    ///
    /// A declared dependency that names an adapter outside this provider is
    /// placed by the composition root, so it counts as already satisfied; a
    /// cycle among this provider's own adapters is a declaration refusal and
    /// nothing is deployed.
    pub async fn deploy_declared_adapters(&self) -> Result<(), PlaneError> {
        let order = self.declared_adapter_order()?;
        for adapter in order {
            self.port.deploy_adapter(self.provider_ref, adapter).await?;
        }
        Ok(())
    }

    /// Publish every declared service through the plane port.
    pub async fn publish_declared_services(&self) -> Result<(), PlaneError> {
        for service in self.services.iter() {
            self.port
                .publish_service(self.provider_ref, service)
                .await?;
        }
        Ok(())
    }

    /// Derive the adapter attach order from the declared dependencies.
    ///
    /// An adapter attaches after every adapter it depends on.
    pub fn declared_adapter_order(&self) -> Result<Vec<&'static PlaneAdapter>, PlaneError> {
        let mut order: Vec<&'static PlaneAdapter> = Vec::with_capacity(self.adapters.len());
        let mut pending: Vec<&'static PlaneAdapter> = self.adapters.iter().collect();
        while !pending.is_empty() {
            let mut progressed = false;
            let mut index = 0;
            while index < pending.len() {
                let adapter = pending[index];
                let ready = adapter.depends_on.iter().all(|dependency| {
                    order.iter().any(|placed| placed.id == *dependency)
                        || !self.adapters.iter().any(|known| known.id == *dependency)
                });
                if ready {
                    order.push(adapter);
                    pending.remove(index);
                    progressed = true;
                } else {
                    index += 1;
                }
            }
            if !progressed {
                return Err(PlaneError::Refused);
            }
        }
        Ok(order)
    }
}

/// A bounded drain deadline handed to [`crate::base::ProviderBase::drain`].
#[derive(Clone)]
pub struct DrainDeadline {
    started_unix_ms: u64,
    budget_ms: u64,
    clock: SharedClock,
}

impl DrainDeadline {
    /// Open a drain with the supplied bounded budget.
    ///
    /// A zero budget is raised to one millisecond and a budget above
    /// [`MAX_DRAIN_BUDGET_MS`] is clamped to it, so a provider always has a
    /// real deadline and never an unbounded one.
    pub fn new(clock: SharedClock, budget_ms: u64) -> Self {
        Self {
            started_unix_ms: clock.now_unix_ms(),
            budget_ms: budget_ms.clamp(1, MAX_DRAIN_BUDGET_MS),
            clock,
        }
    }

    /// The budget this drain was opened with, after clamping.
    pub const fn budget_ms(&self) -> u64 {
        self.budget_ms
    }

    /// The timestamp the drain started at.
    pub const fn started_unix_ms(&self) -> u64 {
        self.started_unix_ms
    }

    /// The remaining budget, saturating at zero.
    pub fn remaining_ms(&self) -> u64 {
        let elapsed = self
            .clock
            .now_unix_ms()
            .saturating_sub(self.started_unix_ms);
        self.budget_ms.saturating_sub(elapsed)
    }

    /// Whether the drain budget is exhausted.
    pub fn expired(&self) -> bool {
        self.remaining_ms() == 0
    }
}
