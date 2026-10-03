//! Generated provider hosting: ONE declared implementation lookup (U9).
//!
//! A provider used to be registered three times in shared code: once as a
//! table row the daemon composed over, once as a handwritten match that
//! turned a registered service id into its declaration, and once as a second
//! handwritten match that turned the same registered service id into its
//! hosting factory. A fourth table, the per-provider operation envelope,
//! resolved the same operation name under a second spelling, so one
//! invocation could reach a handler through one leg and miss through the
//! other. None of those tables is authority; they are four authored views of
//! one provider declaration, and nothing checked that they agreed.
//!
//! This module is the replacement. The authoritative source is the U3
//! [`ProviderDeclaration`] - the serializable
//! [`ProviderDeclarationSpec`](d2b_contracts_provider::v3::ProviderDeclarationSpec)
//! plus the `ProviderImplementationBindings` that realize it - and the
//! derived source of truth is the U4
//! [`PrivatePlanProjection`]. From those, [`generate_hosting_registry`]
//! builds the [`GeneratedHostingRegistry`] the daemon and the broker
//! composition root host from: one entry per declared method, one executable
//! per entry, and one lookup that both a hosted component method and a
//! provider Operation resolve through.
//!
//! # Nothing here is copied metadata
//!
//! The declared facets a method carries are not restated into a hosting
//! table and then ignored. The composition root states what the SELECTED
//! host actually enforces ([`HostSupport`]) and a declaration that asks for a
//! facet the host does not enforce is refused at admission, with the facet
//! named ([`HostingRefusal::FacetUnenforced`]). What reaches a handler is the
//! [`AdmittedCapabilities`] the host admitted for that call, not the raw
//! declaration row.
//!
//! # No second namespace, no fallback route
//!
//! A method either serves the zone plane or names the committed Operation
//! that is its service surface; both spellings are the same entry in the
//! same table, and [`GeneratedHostingRegistry::bind`] resolves either. An
//! identity the table does not carry is [`HostingRefusal::Undeclared`]: there
//! is no default handler, no family switch, and no silent fall-through to a
//! second route.
//!
//! # Generated into the composition root only
//!
//! The registry is a value the composition root constructs from its own
//! provider exports. No generated file is an input to compiling the
//! declaration that produces it, and nothing in this module names a provider
//! crate: the only provider-specific values are the `ServiceDecl` rows and
//! `EffectServiceFactory` implementations the composition root passes in.

// Every refusal names both sides of the disagreement - which method, which
// operation, which service - so an operator reads which declaration is wrong
// rather than that two tables happened to differ. Boxing the fields would
// trade that for a smaller return type.
#![allow(
    clippy::result_large_err,
    reason = "a refusal names both sides of every disagreement"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use d2b_contracts_provider::v3::projection::PrivatePlanProjection;
use d2b_resource_types::{MethodFdContract, ServiceMethod};

use crate::declaration::provider::{ProviderDeclaration, ProviderDeclarationError};
use crate::service::{EffectService, EffectServiceFactory};

/// One declared method facet a hosting site must enforce (R11, AE13).
///
/// The set is closed on purpose: a facet that is not one of these cannot be
/// declared, copied, or silently dropped, because there is no field to spell
/// it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MethodFacet {
    /// Broker-owned state cells the method reaches by name.
    StateCells,
    /// The descriptor carriage the caller may attach on the request leg.
    RequestFds,
    /// The descriptor carriage the implementation may mint on the response
    /// leg.
    ResponseFds,
    /// Privileges an invocation must hold before the method runs.
    Privileges,
    /// The payload schema the invocation is validated against.
    PayloadSchema,
    /// The deadline tier the invocation runs under.
    DeadlineTier,
}

impl MethodFacet {
    /// Every facet, in canonical order.
    pub const ALL: [Self; 6] = [
        Self::StateCells,
        Self::RequestFds,
        Self::ResponseFds,
        Self::Privileges,
        Self::PayloadSchema,
        Self::DeadlineTier,
    ];

    /// The facet's closed code, as a refusal names it.
    pub const fn code(self) -> &'static str {
        match self {
            Self::StateCells => "state-cells",
            Self::RequestFds => "request-fds",
            Self::ResponseFds => "response-fds",
            Self::Privileges => "privileges",
            Self::PayloadSchema => "payload-schema",
            Self::DeadlineTier => "deadline-tier",
        }
    }
}

impl fmt::Display for MethodFacet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// What the SELECTED host enforces for the declarations it hosts (R11).
///
/// A host is not a table of declared facts; it is the enforcement this
/// process actually applies. A declaration asking for anything outside it is
/// refused by name at admission, which is what stops a copied facet from
/// running unenforced (AE13).
#[derive(Clone, Copy)]
pub struct HostSupport {
    /// The facets this host enforces beyond the descriptor legs it always
    /// checks.
    enforced: &'static [MethodFacet],
    /// The state cells this host actually provides. A method declaring a
    /// cell outside this set is refused naming the cell.
    cells: &'static [&'static str],
    /// The deadline tiers this host mints. A method declaring another tier
    /// is refused naming the tier.
    deadline_tiers: &'static [&'static str],
    /// The most descriptors this host admits on a request leg.
    request_fd_ceiling: u8,
    /// The most descriptors this host admits on a response leg.
    response_fd_ceiling: u8,
}

impl HostSupport {
    /// A host that checks both descriptor legs and enforces no other facet.
    ///
    /// This is the enforcement a plain hosting site really has: it admits the
    /// declared fd carriage and refuses anything else, so a declaration that
    /// asks for privileges, a payload schema, a deadline tier, or a state
    /// cell is refused rather than hosted unenforced.
    pub const STANDARD: Self = Self {
        enforced: &[],
        cells: &[],
        deadline_tiers: &[],
        request_fd_ceiling: u8::MAX,
        response_fd_ceiling: u8::MAX,
    };

    /// State what one host enforces.
    pub const fn new(
        enforced: &'static [MethodFacet],
        cells: &'static [&'static str],
        deadline_tiers: &'static [&'static str],
        request_fd_ceiling: u8,
        response_fd_ceiling: u8,
    ) -> Self {
        Self {
            enforced,
            cells,
            deadline_tiers,
            request_fd_ceiling,
            response_fd_ceiling,
        }
    }

    /// Whether this host enforces one facet.
    pub fn enforces(&self, facet: MethodFacet) -> bool {
        match facet {
            // The descriptor legs are always checked: an undeclared leg is
            // refused before any handler runs, so the ceiling is the only
            // thing left to state.
            MethodFacet::RequestFds | MethodFacet::ResponseFds => true,
            MethodFacet::StateCells => !self.cells.is_empty(),
            MethodFacet::Privileges | MethodFacet::PayloadSchema | MethodFacet::DeadlineTier => {
                self.enforced.contains(&facet)
            }
        }
    }

    /// Whether this host provides one declared state cell.
    pub fn provides_cell(&self, cell: &str) -> bool {
        self.cells.contains(&cell)
    }

    /// Whether this host mints one declared deadline tier.
    pub fn mints_tier(&self, tier: &str) -> bool {
        self.deadline_tiers.contains(&tier)
    }

    /// The most descriptors this host admits on a request leg.
    pub const fn request_fd_ceiling(&self) -> u8 {
        self.request_fd_ceiling
    }

    /// The most descriptors this host admits on a response leg.
    pub const fn response_fd_ceiling(&self) -> u8 {
        self.response_fd_ceiling
    }

    /// The state cells this host provides.
    pub const fn cells(&self) -> &'static [&'static str] {
        self.cells
    }

    /// The deadline tiers this host mints.
    pub const fn deadline_tiers(&self) -> &'static [&'static str] {
        self.deadline_tiers
    }
}

impl fmt::Debug for HostSupport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostSupport")
            .field("enforced", &self.enforced)
            .field("cells", &self.cells)
            .field("deadline_tiers", &self.deadline_tiers)
            .field("request_fd_ceiling", &self.request_fd_ceiling)
            .field("response_fd_ceiling", &self.response_fd_ceiling)
            .finish()
    }
}

/// The declared identity of one hosted method (AE1).
///
/// `service` is the service identity the method is hosted under, or `None`
/// when the component serves the method itself rather than through a
/// declared service. Both halves are the same namespace: a method is a
/// method of its provider's declared component, and the service only says
/// which service surface carries it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MethodKey {
    provider: String,
    component: String,
    service: Option<String>,
    method: String,
}

impl MethodKey {
    /// Bind one declared method identity.
    pub fn new(
        provider: impl Into<String>,
        component: impl Into<String>,
        service: Option<String>,
        method: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            component: component.into(),
            service,
            method: method.into(),
        }
    }

    /// The canonical `Provider/...` reference that owns the method.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The declared component identity.
    pub fn component(&self) -> &str {
        &self.component
    }

    /// The declared service identity, when the method is hosted as one.
    pub fn service(&self) -> Option<&str> {
        self.service.as_deref()
    }

    /// The declared method identity.
    pub fn method(&self) -> &str {
        &self.method
    }
}

impl fmt::Display for MethodKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}/", self.provider, self.component)?;
        if let Some(service) = &self.service {
            formatter.write_str(service)?;
            formatter.write_str("/")?;
        }
        formatter.write_str(&self.method)
    }
}

/// What one invocation names.
///
/// Both forms address the same entry in the same table. `Operation` is what
/// a forwarded call carries; `DeclaredMethod` is what the session plane
/// addresses directly. Neither is a second namespace: they are two
/// spellings of one declared method identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationTarget {
    /// The committed Operation an invocation names.
    Operation(String),
    /// The declared method an invocation names.
    DeclaredMethod(MethodKey),
}

impl fmt::Display for InvocationTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(operation) => write!(formatter, "operation {operation}"),
            Self::DeclaredMethod(key) => write!(formatter, "method {key}"),
        }
    }
}

/// Builds the CURRENT implementation of one declared method.
///
/// A hosted component method and a provider Operation both bind one of
/// these. There is no second factory contract and no family-specific
/// construction: the composition root supplies the one adapter its own
/// implementation needs, and the lookup cannot tell the two apart.
pub trait ImplementationFactory: Send + Sync + 'static {
    /// Build a fresh implementation instance.
    fn build(&self) -> Arc<dyn EffectService>;
}

/// Adapts an [`EffectServiceFactory`] to the one declared-implementation
/// factory.
pub struct EffectServiceFactoryAdapter<F>(pub F);

impl<F> EffectServiceFactoryAdapter<F> {
    /// Wrap one provider's own service factory.
    pub const fn new(factory: F) -> Self {
        Self(factory)
    }
}

impl<F> ImplementationFactory for EffectServiceFactoryAdapter<F>
where
    F: EffectServiceFactory,
{
    fn build(&self) -> Arc<dyn EffectService> {
        self.0.build()
    }
}

impl<F> fmt::Debug for EffectServiceFactoryAdapter<F> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EffectServiceFactoryAdapter")
    }
}

/// The executable one composition-root site binds to a declared service.
///
/// `site` is the composition-root location that supplies the factory. It is
/// recorded so several registered identities cannot land on one shared
/// entry: a site binds one declared service, and a second service claiming
/// the same site is refused before anything is hosted (AE1).
pub struct ImplementationBinding {
    site: &'static str,
    factory: &'static dyn ImplementationFactory,
}

impl ImplementationBinding {
    /// Bind one composition-root site to one executable.
    pub const fn new(site: &'static str, factory: &'static dyn ImplementationFactory) -> Self {
        Self { site, factory }
    }

    /// The composition-root site that supplies the executable.
    pub const fn site(&self) -> &'static str {
        self.site
    }

    /// Build the CURRENT implementation.
    pub fn build(&self) -> Arc<dyn EffectService> {
        self.factory.build()
    }
}

impl fmt::Debug for ImplementationBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImplementationBinding")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

/// One composition-root binding: the executable that serves one declared
/// service and every method it answers.
///
/// A service binds ONE executable site, and two declared services cannot
/// share it: a site answering for several registered identities would be the
/// handwritten table this module replaces. Which declared method of that
/// service answers is provider-owned code inside the implementation, reached
/// through the method identity this module carries.
#[derive(Debug)]
pub struct DeclaredBindings<'a> {
    /// The declared service identity this executable serves.
    pub service: &'a str,
    /// The executable itself.
    pub binding: &'static ImplementationBinding,
}

/// One provider's exports as the composition root holds them.
#[derive(Debug)]
pub struct ProviderExport<'a> {
    /// The authoritative declaration.
    pub declaration: &'a ProviderDeclaration,
    /// The executables this composition root bound for it.
    pub bindings: &'a [DeclaredBindings<'a>],
}

/// One generated hosting entry: exactly one declared method.
///
/// The entry is derived, never authored: its identity comes from the
/// declaration, its declared facets from the bound `ServiceDecl`, and its
/// executable from the composition root. There is no hosted row that a
/// provider did not declare and no declared method without an executable.
#[derive(Debug)]
pub struct HostedDeclaration {
    key: MethodKey,
    operation: Option<String>,
    declared: ServiceMethod,
    binding: &'static ImplementationBinding,
}

impl HostedDeclaration {
    /// The declared method identity.
    pub const fn key(&self) -> &MethodKey {
        &self.key
    }

    /// The committed Operation this method serves, when it serves one.
    pub fn operation(&self) -> Option<&str> {
        self.operation.as_deref()
    }

    /// The declared contract facets of this method.
    pub const fn declared(&self) -> &ServiceMethod {
        &self.declared
    }

    /// The composition-root site that binds this method's executable.
    pub const fn site(&self) -> &'static str {
        self.binding.site
    }
}

/// The capabilities a host admitted for one call.
///
/// These are the facets the host enforces, not a restatement of the
/// declaration: a facet the host does not enforce never reaches here,
/// because the method was refused at admission instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedCapabilities {
    state_cells: Vec<String>,
    privileges: Vec<String>,
    request_fds: MethodFdContract,
    response_fds: MethodFdContract,
    payload_schema: Option<String>,
    deadline_tier: Option<String>,
}

impl AdmittedCapabilities {
    /// The state cells this call may reach.
    pub fn state_cells(&self) -> &[String] {
        &self.state_cells
    }

    /// The privileges this call admitted.
    pub fn privileges(&self) -> &[String] {
        &self.privileges
    }

    /// The descriptor carriage admitted on the request leg.
    pub const fn request_fds(&self) -> MethodFdContract {
        self.request_fds
    }

    /// The descriptor carriage admitted on the response leg.
    pub const fn response_fds(&self) -> MethodFdContract {
        self.response_fds
    }

    /// The payload schema this call's payload was validated against.
    pub fn payload_schema(&self) -> Option<&str> {
        self.payload_schema.as_deref()
    }

    /// The deadline tier this call runs under.
    pub fn deadline_tier(&self) -> Option<&str> {
        self.deadline_tier.as_deref()
    }
}

/// One captured invocation binding.
///
/// The binding names the CURRENT implementation of a declared method and the
/// hosting generation it was captured at. A respawn, a republish, or any
/// other rebuild advances the generation, so a binding captured before it
/// refuses instead of dispatching against a superseded implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationBinding {
    key: MethodKey,
    operation: Option<String>,
    generation: u64,
}

impl InvocationBinding {
    /// The declared method this binding addresses.
    pub const fn key(&self) -> &MethodKey {
        &self.key
    }

    /// The committed Operation this binding serves, when it serves one.
    pub fn operation(&self) -> Option<&str> {
        self.operation.as_deref()
    }

    /// The hosting generation this binding was captured at.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// The current implementation one admitted invocation resolves to.
#[derive(Clone)]
pub struct AdmittedImplementation {
    key: MethodKey,
    operation: Option<String>,
    generation: u64,
    binding: &'static ImplementationBinding,
    admitted: AdmittedCapabilities,
}

impl AdmittedImplementation {
    /// The declared method this implementation serves.
    pub const fn key(&self) -> &MethodKey {
        &self.key
    }

    /// The committed Operation this implementation serves, when it serves
    /// one.
    pub fn operation(&self) -> Option<&str> {
        self.operation.as_deref()
    }

    /// The hosting generation this implementation is current at.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The composition-root site that binds the executable.
    pub const fn site(&self) -> &'static str {
        self.binding.site
    }

    /// The capabilities admitted for this call.
    pub const fn capabilities(&self) -> &AdmittedCapabilities {
        &self.admitted
    }

    /// Build the CURRENT implementation instance.
    pub fn build(&self) -> Arc<dyn EffectService> {
        self.binding.build()
    }
}

impl fmt::Debug for AdmittedImplementation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedImplementation")
            .field("key", &self.key)
            .field("generation", &self.generation)
            .field("site", &self.site())
            .field("admitted", &self.admitted)
            .finish()
    }
}

/// Why a hosting table was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostingRefusal {
    /// The declaration's own two halves disagree.
    Declaration(ProviderDeclarationError),
    /// One declared method was mapped by two declared services.
    DuplicateMethodMapping {
        /// The canonical `Provider/...` reference that owns the method.
        provider: String,
        /// The declared component identity.
        component: String,
        /// The declared method identity.
        method: String,
        /// The first service that claims it.
        first: String,
        /// The second service that claims it.
        second: String,
    },
    /// Two entries claim the same committed Operation.
    DuplicateOperation {
        /// The claimed Operation.
        operation: String,
        /// The first declared method that claims it.
        first: MethodKey,
        /// The second declared method that claims it.
        second: MethodKey,
    },
    /// Two declared services are served by one composition-root site.
    DuplicateHandler {
        /// The claimed site.
        site: &'static str,
        /// The first declared service that claims it.
        first: String,
        /// The second declared service that claims it.
        second: String,
    },
    /// A declared service has no bound executable.
    ImplementationMissing {
        /// The declared component identity that declares the service.
        component: String,
        /// The declared service identity nothing is bound to.
        service: String,
    },
    /// A declared facet the selected host does not enforce (AE13).
    FacetUnenforced {
        /// The declared method that asked for it.
        key: MethodKey,
        /// The facet the host cannot enforce.
        facet: MethodFacet,
        /// The declared value the host refused, when the facet carries one.
        declared: Option<String>,
    },
    /// The generated identities and the U4 projection disagree.
    ProjectionMismatch {
        /// The projection row no generated entry realizes.
        provider: String,
        /// The component the row names.
        component: String,
        /// The method the row names.
        method: String,
    },
    /// Nothing in the table serves the invocation target.
    Undeclared {
        /// What the invocation named.
        target: InvocationTarget,
    },
    /// The hosting generation moved past the captured binding.
    GenerationMoved {
        /// The declared method the binding addresses.
        key: MethodKey,
        /// The generation the caller captured.
        expected: u64,
        /// The generation the implementation is current at.
        current: u64,
    },
}

impl HostingRefusal {
    /// The refusal's closed code, as an operator record names it.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Declaration(_) => "provider-declaration-invalid",
            Self::DuplicateMethodMapping { .. } => "provider-hosting-duplicate-method",
            Self::DuplicateOperation { .. } => "provider-hosting-duplicate-operation",
            Self::DuplicateHandler { .. } => "provider-hosting-duplicate-handler",
            Self::ImplementationMissing { .. } => "provider-hosting-implementation-missing",
            Self::FacetUnenforced { .. } => "provider-hosting-facet-unenforced",
            Self::ProjectionMismatch { .. } => "provider-hosting-projection-mismatch",
            Self::Undeclared { .. } => "provider-hosting-undeclared",
            Self::GenerationMoved { .. } => "provider-hosting-generation-moved",
        }
    }
}

impl fmt::Display for HostingRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declaration(error) => write!(formatter, "provider declaration refused: {error}"),
            Self::DuplicateMethodMapping { provider, component, method, first, second } => write!(
                formatter,
                "method `{method}` of `{component}` in `{provider}` is mapped by both service `{first}` and service `{second}`"
            ),
            Self::DuplicateOperation { operation, first, second } => write!(
                formatter,
                "operation `{operation}` is claimed by both `{first}` and `{second}`"
            ),
            Self::DuplicateHandler { site, first, second } => write!(
                formatter,
                "composition site `{site}` is bound to both service `{first}` and service `{second}`"
            ),
            Self::ImplementationMissing { component, service } => write!(
                formatter,
                "declared service `{service}` of `{component}` has no bound implementation"
            ),
            Self::FacetUnenforced { key, facet, declared } => match declared {
                Some(declared) => write!(
                    formatter,
                    "`{key}` declares facet `{facet}` = `{declared}`, which the selected host does not enforce"
                ),
                None => write!(
                    formatter,
                    "`{key}` declares facet `{facet}`, which the selected host does not enforce"
                ),
            },
            Self::ProjectionMismatch { provider, component, method } => write!(
                formatter,
                "the graph projection row `{provider}`/`{component}`/`{method}` has no generated hosting entry"
            ),
            Self::Undeclared { target } => write!(formatter, "nothing in the hosting table serves {target}"),
            Self::GenerationMoved { key, expected, current } => write!(
                formatter,
                "`{key}` moved to hosting generation {current}, past the captured generation {expected}"
            ),
        }
    }
}

impl std::error::Error for HostingRefusal {}

/// The generated hosting table a composition root hosts from.
///
/// The table is sealed: every duplicate method mapping, duplicate Operation,
/// and duplicate handler site was refused while it was built, and the
/// registry that hosts it did not exist yet, so a duplicate fails before
/// hosting rather than at first call.
pub struct GeneratedHostingRegistry {
    entries: Vec<HostedDeclaration>,
    by_operation: BTreeMap<String, usize>,
    by_method: BTreeMap<MethodKey, usize>,
    generations: Vec<Arc<AtomicU64>>,
    admitted: Vec<AdmittedCapabilities>,
}

impl GeneratedHostingRegistry {
    /// Every generated entry, in declaration order.
    pub fn entries(&self) -> &[HostedDeclaration] {
        &self.entries
    }

    /// The capabilities a host admitted for one entry.
    pub fn admitted(&self, key: &MethodKey) -> Option<&AdmittedCapabilities> {
        self.by_method.get(key).and_then(|index| self.admitted.get(*index))
    }

    /// The hosting generation of one entry.
    pub fn generation(&self, key: &MethodKey) -> Option<u64> {
        self.by_method
            .get(key)
            .and_then(|index| self.generations.get(*index))
            .map(|generation| generation.load(Ordering::Acquire))
    }

    /// Capture a binding to the CURRENT implementation of one target.
    ///
    /// This is the one lookup: a committed Operation and a declared method
    /// resolve through it into the same entry, and a target the table does
    /// not carry is refused rather than routed somewhere else.
    ///
    /// # Errors
    ///
    /// Returns [`HostingRefusal::Undeclared`] when no generated entry serves
    /// the target.
    pub fn bind(&self, target: &InvocationTarget) -> Result<InvocationBinding, HostingRefusal> {
        let index = self.index_of(target)?;
        let entry = &self.entries[index];
        Ok(InvocationBinding {
            key: entry.key.clone(),
            operation: entry.operation.clone(),
            generation: self.generations[index].load(Ordering::Acquire),
        })
    }

    /// Rebuild one entry's implementation and capture the new generation.
    ///
    /// This is what a service restart, a respawn, or a republish does. Every
    /// binding captured before it refuses at
    /// [`GeneratedHostingRegistry::admit`], so an old invocation never
    /// reaches a superseded implementation.
    ///
    /// # Errors
    ///
    /// Returns [`HostingRefusal::Undeclared`] when no generated entry serves
    /// the target.
    pub fn restart(
        &self,
        target: &InvocationTarget,
    ) -> Result<InvocationBinding, HostingRefusal> {
        let index = self.index_of(target)?;
        let entry = &self.entries[index];
        Ok(InvocationBinding {
            key: entry.key.clone(),
            operation: entry.operation.clone(),
            generation: self.generations[index].fetch_add(1, Ordering::AcqRel) + 1,
        })
    }

    /// Resolve one captured binding to its CURRENT implementation.
    ///
    /// # Errors
    ///
    /// Returns [`HostingRefusal::GenerationMoved`] when a rebuild moved the
    /// hosting generation past the captured binding.
    pub fn admit(
        &self,
        binding: &InvocationBinding,
    ) -> Result<AdmittedImplementation, HostingRefusal> {
        let index = self.by_method.get(&binding.key).copied().ok_or_else(|| {
            HostingRefusal::Undeclared {
                target: InvocationTarget::DeclaredMethod(binding.key.clone()),
            }
        })?;
        let entry = &self.entries[index];
        let current = self.generations[index].load(Ordering::Acquire);
        if current != binding.generation {
            return Err(HostingRefusal::GenerationMoved {
                key: binding.key.clone(),
                expected: binding.generation,
                current,
            });
        }
        Ok(AdmittedImplementation {
            key: entry.key.clone(),
            operation: entry.operation.clone(),
            generation: current,
            binding: entry.binding,
            admitted: self.admitted[index].clone(),
        })
    }

    fn index_of(&self, target: &InvocationTarget) -> Result<usize, HostingRefusal> {
        let found = match target {
            InvocationTarget::Operation(operation) => self.by_operation.get(operation).copied(),
            InvocationTarget::DeclaredMethod(key) => self.by_method.get(key).copied(),
        };
        found.ok_or_else(|| HostingRefusal::Undeclared { target: target.clone() })
    }
}

impl fmt::Debug for GeneratedHostingRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedHostingRegistry")
            .field("entries", &self.entries)
            .finish_non_exhaustive()
    }
}

/// Generate the hosting table a composition root hosts from.
///
/// The inputs are the authoritative declaration, the executables the
/// composition root bound for it, what the selected host enforces, and the
/// derived U4 graph projection. The output is one entry per declared method
/// and one lookup that resolves both spellings of it: the committed
/// Operation a forwarded invocation names, and the declared method identity
/// the session plane names. A declaration cannot produce two routes for one
/// method, because the second route is a duplicate mapping and is refused
/// here.
///
/// # Errors
///
/// Refuses, by name, before any registry exists:
///
/// - [`HostingRefusal::Declaration`] when the declaration's two halves
///   disagree.
/// - [`HostingRefusal::ImplementationMissing`] when a declared service has
///   no bound executable. This is the missing-factory refusal, kept with its
///   own name.
/// - [`HostingRefusal::DuplicateMethodMapping`] when two declared services
///   of one component claim the same method.
/// - [`HostingRefusal::DuplicateOperation`] when two declared methods claim
///   one committed Operation.
/// - [`HostingRefusal::DuplicateHandler`] when one composition-root site
///   claims two declared services.
/// - [`HostingRefusal::FacetUnenforced`] when a declaration asks for a facet
///   the selected host does not enforce (AE13).
/// - [`HostingRefusal::ProjectionMismatch`] when a projected graph row for
///   an in-process method has no generated entry.
pub fn generate_hosting_registry(
    exports: &[ProviderExport<'_>],
    support: &HostSupport,
    plan: &PrivatePlanProjection,
) -> Result<GeneratedHostingRegistry, HostingRefusal> {
    let mut builder = Builder {
        entries: Vec::new(),
        generations: Vec::new(),
        admitted: Vec::new(),
        by_operation: BTreeMap::new(),
        by_method: BTreeMap::new(),
        site_owner: BTreeMap::new(),
    };
    // A trusted executable template is launched, not hosted in this process
    // (R31), so its projected row expects no hosting entry.
    let mut launched: BTreeSet<(String, String, String)> = BTreeSet::new();

    for export in exports {
        export
            .declaration
            .validate()
            .map_err(HostingRefusal::Declaration)?;
        let provider = export.declaration.provider().to_canonical_string();
        let bound_services = export.declaration.bindings().services().collect::<Vec<_>>();
        for component in export.declaration.spec().components() {
            let component_id = component.component().component_id().as_str();
            for declared_method in component.methods() {
                if declared_method.template().is_some() {
                    launched.insert((
                        provider.clone(),
                        component_id.to_owned(),
                        declared_method.name().as_str().to_owned(),
                    ));
                }
            }
            for declared_service in component.services() {
                let service_id = declared_service.id().as_str();
                let decl = bound_services
                    .iter()
                    .find(|decl| decl.id == service_id)
                    .copied()
                    .ok_or_else(|| HostingRefusal::ImplementationMissing {
                        component: component_id.to_owned(),
                        service: service_id.to_owned(),
                    })?;
                let binding = export
                    .bindings
                    .iter()
                    .find(|bound| bound.service == service_id)
                    .map(|bound| bound.binding)
                    .ok_or_else(|| HostingRefusal::ImplementationMissing {
                        component: component_id.to_owned(),
                        service: service_id.to_owned(),
                    })?;
                for method in decl.methods {
                    builder.push(
                        &provider,
                        component_id,
                        service_id,
                        method,
                        binding,
                        support,
                    )?;
                }
            }
        }
    }

    // Registry identity and executable handler reachability agree by
    // generated construction: every projected in-process method has exactly
    // one generated entry, so a projected method whose handler the table
    // cannot reach is refused here rather than discovered at first call.
    for row in plan.operations() {
        let identity = (
            row.provider_ref().to_owned(),
            row.component_id().to_owned(),
            row.method().to_owned(),
        );
        if launched.contains(&identity) {
            continue;
        }
        let reachable = builder
            .by_method
            .keys()
            .any(|key| {
                key.provider == identity.0
                    && key.component == identity.1
                    && key.method == identity.2
            });
        if !reachable {
            return Err(HostingRefusal::ProjectionMismatch {
                provider: identity.0,
                component: identity.1,
                method: identity.2,
            });
        }
    }

    Ok(GeneratedHostingRegistry {
        entries: builder.entries,
        by_operation: builder.by_operation,
        by_method: builder.by_method,
        generations: builder.generations,
        admitted: builder.admitted,
    })
}

/// The tables one generated entry is appended to.
///
/// The builder is the whole duplication check: an entry is inserted only
/// after its method, its Operation, and its composition-root site are all
/// unclaimed, so nothing duplicated is ever hosted and nothing partial is
/// left behind for a later call to find.
struct Builder {
    entries: Vec<HostedDeclaration>,
    generations: Vec<Arc<AtomicU64>>,
    admitted: Vec<AdmittedCapabilities>,
    by_operation: BTreeMap<String, usize>,
    by_method: BTreeMap<MethodKey, usize>,
    site_owner: BTreeMap<&'static str, String>,
}

impl Builder {
    /// Append one generated entry, refusing every way it can disagree.
    fn push(
        &mut self,
        provider: &str,
        component: &str,
        service: &str,
        method: &ServiceMethod,
        binding: &'static ImplementationBinding,
        support: &HostSupport,
    ) -> Result<(), HostingRefusal> {
        let key = MethodKey::new(provider, component, Some(service.to_owned()), method.name);
        // A declared method the component hosts under two services is one
        // method mapped twice. Refusing while the table is still being built
        // is what makes the duplicate fail before hosting rather than at the
        // first call the second mapping would win.
        if let Some(existing) = self.by_method.keys().find(|existing| {
            existing.provider == key.provider
                && existing.component == key.component
                && existing.method == key.method
        }) {
            return Err(HostingRefusal::DuplicateMethodMapping {
                provider: provider.to_owned(),
                component: component.to_owned(),
                method: method.name.to_owned(),
                first: existing.service.clone().unwrap_or_default(),
                second: service.to_owned(),
            });
        }
        let operation = method.operation.map(str::to_owned);
        if let Some((operation, index)) = operation
            .as_ref()
            .and_then(|operation| Some((operation, self.by_operation.get(operation).copied()?)))
        {
            return Err(HostingRefusal::DuplicateOperation {
                operation: operation.clone(),
                first: self.entries[index].key.clone(),
                second: key.clone(),
            });
        }
        // One composition-root site serves one declared service. A second
        // service claiming the same site would be the shared entry several
        // registered identities land on, so it is refused while the table is
        // still being built. The site's own service's methods are one
        // implementation answering its own declaration, not a shared entry.
        if let Some(first) = self.site_owner.get(binding.site).filter(|first| *first != service) {
            return Err(HostingRefusal::DuplicateHandler {
                site: binding.site,
                first: first.clone(),
                second: service.to_owned(),
            });
        }
        let capabilities = admit_facets(&key, method, support)?;

        let index = self.entries.len();
        self.by_method.insert(key.clone(), index);
        if let Some(operation) = operation.clone() {
            self.by_operation.insert(operation.clone(), index);
        }
        self.site_owner.insert(binding.site, service.to_owned());
        self.entries.push(HostedDeclaration {
            key,
            operation,
            declared: *method,
            binding,
        });
        self.generations.push(Arc::new(AtomicU64::new(1)));
        self.admitted.push(capabilities);
        Ok(())
    }
}

/// Admit one method's declared facets against what the host enforces.
///
/// Absence is not support: a facet the host cannot enforce is refused here,
/// naming the facet and its declared value, rather than copied into a
/// capability object the implementation would then read as satisfied.
fn admit_facets(
    key: &MethodKey,
    method: &ServiceMethod,
    support: &HostSupport,
) -> Result<AdmittedCapabilities, HostingRefusal> {
    for facet in MethodFacet::ALL {
        let Some(unsupported) = unsupported_facet_value(facet, method, support) else {
            continue;
        };
        return Err(HostingRefusal::FacetUnenforced {
            key: key.clone(),
            facet,
            declared: Some(unsupported),
        });
    }
    Ok(AdmittedCapabilities {
        state_cells: method.state_cells.iter().map(|cell| (*cell).to_owned()).collect(),
        privileges: method.privileges.iter().map(|name| (*name).to_owned()).collect(),
        request_fds: method.request_fds,
        response_fds: method.response_fds,
        payload_schema: method.payload_schema.map(str::to_owned),
        deadline_tier: method.deadline_tier.map(str::to_owned),
    })
}

/// The declared value of one facet the selected host cannot admit.
///
/// `None` is the only pass: the declaration does not ask for the facet, or
/// the host enforces it and can satisfy the exact value asked for. A facet
/// the host does not enforce, and a value of an enforced facet the host
/// cannot satisfy - a state cell it does not provide, a deadline tier it
/// does not mint, a descriptor carriage above its ceiling - both come back
/// as the offending value, so the refusal names what could not be enforced.
fn unsupported_facet_value(
    facet: MethodFacet,
    method: &ServiceMethod,
    support: &HostSupport,
) -> Option<String> {
    match facet {
        MethodFacet::StateCells => {
            (!support.enforces(facet) && !method.state_cells.is_empty())
                .then(|| method.state_cells.join(","))
                .or_else(|| {
                    method.state_cells.iter().find_map(|cell| {
                        (!support.provides_cell(cell)).then(|| (*cell).to_owned())
                    })
                })
        }
        MethodFacet::RequestFds => (!support.enforces(facet)
            || method.request_fds.max_fds > support.request_fd_ceiling())
        .then(|| format!("{} descriptors", method.request_fds.max_fds)),
        MethodFacet::ResponseFds => (!support.enforces(facet)
            || method.response_fds.max_fds > support.response_fd_ceiling())
        .then(|| format!("{} descriptors", method.response_fds.max_fds)),
        MethodFacet::Privileges => {
            (!support.enforces(facet) && !method.privileges.is_empty())
                .then(|| method.privileges.join(","))
        }
        MethodFacet::PayloadSchema => (!support.enforces(facet))
            .then(|| method.payload_schema.map(str::to_owned))
            .flatten(),
        MethodFacet::DeadlineTier => method.deadline_tier.and_then(|tier| {
            (!support.enforces(facet) || !support.mints_tier(tier)).then(|| tier.to_owned())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPPORTED: &[MethodFacet] = &[
        MethodFacet::Privileges,
        MethodFacet::PayloadSchema,
        MethodFacet::DeadlineTier,
    ];
    const CELLS: &[&str] = &["lifecycle-leases"];
    const TIERS: &[&str] = &["bulk"];

    /// A host that enforces the facets the fixture below declares.
    fn capable() -> HostSupport {
        HostSupport::new(SUPPORTED, CELLS, TIERS, 4, 4)
    }

    /// A host that enforces nothing but the descriptor legs.
    fn plain() -> HostSupport {
        HostSupport::new(&[], &[], &[], 4, 4)
    }

    /// A method carrying every facet the capable host enforces.
    fn full_method() -> ServiceMethod {
        ServiceMethod::serving_with(
            "export-volume",
            "export",
            Some("volume-export@1"),
            MethodFdContract { max_fds: 2, fd_kind: Some("any") },
            MethodFdContract { max_fds: 1, fd_kind: Some("socket") },
            &["lifecycle-leases"],
            &["volume-export"],
            Some("bulk"),
        )
    }

    #[test]
    fn a_standard_host_admits_only_the_descriptor_legs() {
        assert!(HostSupport::STANDARD.enforces(MethodFacet::RequestFds));
        assert!(HostSupport::STANDARD.enforces(MethodFacet::ResponseFds));
        assert!(!HostSupport::STANDARD.enforces(MethodFacet::StateCells));
        assert!(!HostSupport::STANDARD.enforces(MethodFacet::Privileges));
        assert!(!HostSupport::STANDARD.enforces(MethodFacet::PayloadSchema));
        assert!(!HostSupport::STANDARD.enforces(MethodFacet::DeadlineTier));
    }

    #[test]
    fn a_declaration_asking_for_an_unenforceable_facet_is_refused_by_name() {
        let key = MethodKey::new(
            "Provider/volume",
            "volume-binding",
            Some("volume/export".to_owned()),
            "export",
        );
        for (method, facet, declared) in [
            (
                ServiceMethod::serving_with(
                    "export-volume",
                    "export",
                    None,
                    MethodFdContract::NONE,
                    MethodFdContract::NONE,
                    &[],
                    &["volume-export"],
                    None,
                ),
                MethodFacet::Privileges,
                "volume-export".to_owned(),
            ),
            (
                ServiceMethod::serving_with(
                    "export-volume",
                    "export",
                    Some("volume-export@1"),
                    MethodFdContract::NONE,
                    MethodFdContract::NONE,
                    &[],
                    &[],
                    None,
                ),
                MethodFacet::PayloadSchema,
                "volume-export@1".to_owned(),
            ),
            (
                ServiceMethod::serving_with(
                    "export-volume",
                    "export",
                    None,
                    MethodFdContract::NONE,
                    MethodFdContract::NONE,
                    &["lifecycle-leases"],
                    &[],
                    None,
                ),
                MethodFacet::StateCells,
                "lifecycle-leases".to_owned(),
            ),
            (
                ServiceMethod::serving_with(
                    "export-volume",
                    "export",
                    None,
                    MethodFdContract::NONE,
                    MethodFdContract::NONE,
                    &[],
                    &[],
                    Some("bulk"),
                ),
                MethodFacet::DeadlineTier,
                "bulk".to_owned(),
            ),
            (
                ServiceMethod::serving_with(
                    "export-volume",
                    "export",
                    None,
                    MethodFdContract { max_fds: 9, fd_kind: Some("any") },
                    MethodFdContract::NONE,
                    &[],
                    &[],
                    None,
                ),
                MethodFacet::RequestFds,
                "9 descriptors".to_owned(),
            ),
        ] {
            assert_eq!(
                admit_facets(&key, &method, &plain()),
                Err(HostingRefusal::FacetUnenforced {
                    key: key.clone(),
                    facet,
                    declared: Some(declared),
                }),
                "the {facet} facet must be refused by name"
            );
        }
    }

    #[test]
    fn a_capable_host_admits_only_what_it_enforces() {
        let key = MethodKey::new(
            "Provider/volume",
            "volume-binding",
            Some("volume/export".to_owned()),
            "export",
        );
        let admitted = admit_facets(&key, &full_method(), &capable()).expect("admitted");
        assert_eq!(admitted.state_cells(), ["lifecycle-leases"]);
        assert_eq!(admitted.privileges(), ["volume-export"]);
        assert_eq!(admitted.payload_schema(), Some("volume-export@1"));
        assert_eq!(admitted.deadline_tier(), Some("bulk"));
        assert_eq!(admitted.request_fds().max_fds, 2);
        assert_eq!(admitted.response_fds().max_fds, 1);
    }

    #[test]
    fn a_cell_or_tier_the_host_cannot_satisfy_is_refused_by_value() {
        let key = MethodKey::new(
            "Provider/volume",
            "volume-binding",
            Some("volume/export".to_owned()),
            "export",
        );
        let wrong_cell = ServiceMethod::serving_with(
            "export-volume",
            "export",
            None,
            MethodFdContract::NONE,
            MethodFdContract::NONE,
            &["cell-store"],
            &[],
            None,
        );
        assert!(matches!(
            admit_facets(&key, &wrong_cell, &capable()),
            Err(HostingRefusal::FacetUnenforced {
                facet: MethodFacet::StateCells,
                declared: Some(value),
                ..
            }) if value == "cell-store"
        ));
        let wrong_tier = ServiceMethod::serving_with(
            "export-volume",
            "export",
            None,
            MethodFdContract::NONE,
            MethodFdContract::NONE,
            &[],
            &[],
            Some("realtime"),
        );
        assert!(matches!(
            admit_facets(&key, &wrong_tier, &capable()),
            Err(HostingRefusal::FacetUnenforced {
                facet: MethodFacet::DeadlineTier,
                declared: Some(value),
                ..
            }) if value == "realtime"
        ));
    }
}