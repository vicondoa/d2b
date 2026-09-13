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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts_resource::v3::ZoneId;
use d2b_provider_toolkit::{
    AttachError, Cardinality, DEFAULT_DRAIN_BUDGET_MS, DrainDeadline, DrainError,
    DriverDescriptor, IsolationPosture, Lifecycle, ProviderBase, ProviderDeclaration,
    ZonePlaneHandle,
};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};

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
    /// The plane needs an ambient async runtime to run the base.
    RuntimeUnavailable,
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
}

impl ProviderStartupError {
    /// The stable lower-kebab reason.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::RuntimeUnavailable => "provider-runtime-unavailable",
            Self::IdentityMissing => "provider-identity-missing",
            Self::Duplicate { .. } => "provider-duplicate",
            Self::Registration { .. } => "provider-registration-refused",
            Self::Plane(refusal) => refusal.reason,
            Self::Attach { .. } => "attach-refused",
            Self::Drain { code, .. } => code,
        }
    }

    /// The provider the failure names.
    pub(crate) const fn provider_ref(&self) -> &'static str {
        match self {
            Self::RuntimeUnavailable | Self::IdentityMissing => "",
            Self::Duplicate { provider_ref }
            | Self::Registration { provider_ref, .. }
            | Self::Attach { provider_ref }
            | Self::Drain { provider_ref, .. } => provider_ref,
            Self::Plane(refusal) => refusal.provider_ref,
        }
    }

    /// The failure as one line: reason, provider, and the offending row.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::RuntimeUnavailable | Self::IdentityMissing => self.code().to_owned(),
            Self::Duplicate { provider_ref } => {
                format!("{}:{}", self.code(), provider_ref)
            }
            Self::Registration {
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

/// The providers one plane starts, in the order they start.
pub(crate) struct ProviderSet {
    zone: ZoneId,
    state_root: PathBuf,
    providers: Vec<(ProviderDeclaration, Vec<DriverDescriptor>)>,
}

impl ProviderSet {
    /// Open a set for one zone, rooted at the zone's daemon-owned state
    /// directory.
    pub(crate) fn new(zone: ZoneId, state_root: PathBuf) -> Self {
        Self {
            zone,
            state_root,
            providers: Vec::new(),
        }
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
        let mut providers = Vec::with_capacity(declarations.len());
        let mut startup_order = Vec::with_capacity(declarations.len());
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
            startup_order.push(provider.provider_ref());
            providers.push(provider);
        }
        Ok(ProviderRuntime {
            port,
            providers,
            startup_order,
            drain_order: Mutex::new(Vec::new()),
            directory: registrations.take_directory(),
        })
    }
}

/// The providers one zone is running.
pub(crate) struct ProviderRuntime {
    port: Arc<ProductionPlanePort>,
    providers: Vec<ZoneProvider>,
    startup_order: Vec<&'static str>,
    drain_order: Mutex<Vec<&'static str>>,
    directory: ProviderDirectory,
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
