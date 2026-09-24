//! Provider registry producing drivers per resource type (U4, KTD3).

pub const MODULE_NAME: &str = "provider";

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::context::SpecDecoder;
use crate::driver::{DynResourceDriver, ResourceDriverFactory};
use crate::identity::{ResourceKey, ResourceTypeName};

/// The registry's view of one driver declaration.
///
/// The runtime holds the mechanism; the vocabulary crate holds the
/// declarations. The declaration types live in `d2b-resource-types`, which
/// depends on this crate for the [`SpecDecoder`] and
/// [`ResourceDriverFactory`] contracts, so this crate cannot name the
/// declaration type back. The crate that assembles a registry therefore
/// implements this view over its declaration type - one view per driver -
/// and the registry indexes only the view.
///
/// The view carries what registration has to enforce: the type name the
/// driver is keyed by, the allowed-source mask predicate that gates when the
/// driver may arrive, the operation references the handler table indexes, and
/// the decoder and factory the registry serves to the manager.
pub trait DriverRegistration: Send + Sync {
    /// The one resource type this declaration serves; the registry keys the
    /// driver by this name.
    fn resource_type(&self) -> ResourceTypeName;

    /// Whether the declaration's allowed-source mask lacks RUNTIME.
    ///
    /// A driver that requires plane registration must be registered before
    /// the plane opens: a late arrival of one is a provider-wiring bug, not a
    /// runtime provisioning path, and registration refuses it.
    fn requires_plane_registration(&self) -> bool;

    /// The committed operation references this driver serves, in canonical
    /// `Type/name` form.
    ///
    /// The registry is the handler table, so one reference has exactly one
    /// owner: a second registration naming the same reference is refused.
    fn operation_refs(&self) -> Vec<String>;

    /// The resource verbs this type supports, in the declaration's spelling.
    ///
    /// The seed constrains committed Role rules to this set: a rule that
    /// grants a verb the type does not declare is refused before the plane
    /// opens. The default keeps registrations that carry no verbs (fixtures,
    /// factory-only paths) working; a declared set is empty, never inferred.
    fn declared_verbs(&self) -> Vec<String> {
        Vec::new()
    }

    /// The spec decoder for this type's stored desired rows.
    fn decoder(&self) -> Arc<dyn SpecDecoder>;

    /// The factory that builds this type's driver.
    fn factory(&self) -> Arc<dyn ResourceDriverFactory>;
}

/// Provider directory failures.
#[derive(Debug, thiserror::Error)]
pub enum ProviderDirectoryError {
    /// A factory is already registered for this resource type; a second
    /// registration is a provider-wiring bug.
    #[error("a provider factory for resource type {0} is already registered")]
    DuplicateType(ResourceTypeName),
    /// No factory is registered for the requested resource type.
    #[error("no provider factory is registered for resource type {0}")]
    UnknownType(ResourceTypeName),
    /// An operation reference already has an owner: the handler table has one
    /// entry per reference, so a driver that registers a reference another
    /// driver already registered is refused.
    #[error("operation reference {operation_ref} is already registered by resource type {owner}")]
    ForeignOperation {
        /// The contested operation reference.
        operation_ref: String,
        /// The resource type whose driver registered it first.
        owner: ResourceTypeName,
    },
    /// A driver whose allowed-source mask lacks RUNTIME arrived after the
    /// plane opened; it can only have been registered before, so the
    /// registration is refused rather than served late.
    #[error("driver for resource type {type_name} must be registered before the plane opens")]
    RequiredBeforeOpen {
        /// The resource type whose driver arrived late.
        type_name: ResourceTypeName,
    },
}

/// Registry mapping resource type names to driver factories (KTD3). The
/// manager (U3) resolves the factory for a desired resource's type here
/// before spawning its actor.
///
/// One driver per resource type: the rewrite's single-execution-model
/// invariant (R29) starts at registration - a second registration for a
/// known type is a wiring bug and fails without clobbering the first. Alongside
/// the factory, a declaration-driven registration records the type's spec
/// decoder (the manager's per-type decode hook) and indexes the operation
/// references it serves.
///
/// The registry also carries the plane-open gate: drivers declared with an
/// allowed-source mask that lacks RUNTIME must be registered before the plane
/// opens, and [`ProviderDirectory::mark_plane_open`] closes that window.
/// What the registered set must *cover* at open is the generated
/// converted-type catalog's business: the plane compares
/// [`ProviderDirectory::registered_types`] against it.
///
/// [`ProviderDirectory::register`] is the factory-only path the manager's
/// in-crate fixtures use; [`ProviderDirectory::register_driver`] is the
/// declaration path the plane's assembly takes.
#[derive(Default)]
pub struct ProviderDirectory {
    factories: HashMap<ResourceTypeName, Arc<dyn ResourceDriverFactory>>,
    decoders: HashMap<ResourceTypeName, Arc<dyn SpecDecoder>>,
    operation_refs: HashMap<String, ResourceTypeName>,
    declared_verbs: HashMap<ResourceTypeName, Vec<String>>,
    /// Set by [`ProviderDirectory::mark_plane_open`].
    plane_open: bool,
}

impl ProviderDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a factory under every type it covers. Atomic: a duplicate
    /// type rejects the whole registration.
    ///
    /// This path carries no decoder, operation references, or allowed-source
    /// mask; use [`ProviderDirectory::register_driver`] for a full
    /// declaration.
    pub fn register(
        &mut self,
        factory: Arc<dyn ResourceDriverFactory>,
    ) -> Result<(), ProviderDirectoryError> {
        let mut fresh = HashSet::new();
        for type_name in factory.resource_types() {
            if self.factories.contains_key(type_name) || !fresh.insert(type_name.clone()) {
                return Err(ProviderDirectoryError::DuplicateType(type_name.clone()));
            }
        }
        for type_name in factory.resource_types() {
            self.factories.insert(type_name.clone(), factory.clone());
        }
        Ok(())
    }

    /// Register one driver declaration through the registry's view of it.
    ///
    /// Atomic like [`ProviderDirectory::register`]: every check runs before
    /// the registry changes, so a refused registration leaves the previous
    /// state intact. The checks are, in order:
    ///
    /// - a declaration whose allowed-source mask lacks RUNTIME cannot arrive
    ///   once the plane is open; the refusal names the type;
    /// - the resource type is not already registered;
    /// - no operation reference is already registered by another driver.
    pub fn register_driver(
        &mut self,
        driver: &dyn DriverRegistration,
    ) -> Result<(), ProviderDirectoryError> {
        let type_name = driver.resource_type();
        if self.plane_open && driver.requires_plane_registration() {
            return Err(ProviderDirectoryError::RequiredBeforeOpen { type_name });
        }
        if self.factories.contains_key(&type_name) {
            return Err(ProviderDirectoryError::DuplicateType(type_name));
        }
        let operation_refs = driver.operation_refs();
        for operation_ref in &operation_refs {
            if let Some(owner) = self.operation_refs.get(operation_ref) {
                return Err(ProviderDirectoryError::ForeignOperation {
                    operation_ref: operation_ref.clone(),
                    owner: owner.clone(),
                });
            }
        }
        self.factories.insert(type_name.clone(), driver.factory());
        self.decoders
            .insert(type_name.clone(), driver.decoder());
        self.declared_verbs
            .insert(type_name.clone(), driver.declared_verbs());
        for operation_ref in operation_refs {
            self.operation_refs.insert(operation_ref, type_name.clone());
        }
        Ok(())
    }

    /// The factory registered for a resource type, if any.
    pub fn lookup(&self, type_name: &ResourceTypeName) -> Option<Arc<dyn ResourceDriverFactory>> {
        self.factories.get(type_name).cloned()
    }

    /// Every registered resource type, sorted. The plane's startup
    /// cross-check compares this set against the generated converted-type
    /// catalog.
    pub fn registered_types(&self) -> Vec<ResourceTypeName> {
        let mut types = self.factories.keys().cloned().collect::<Vec<_>>();
        types.sort();
        types
    }

    /// The spec decoder of every registered driver, keyed by resource type.
    /// Registering through [`ProviderDirectory::register_driver`] is what
    /// populates this; the factory-only path leaves a type without one.
    pub fn decoders(&self) -> HashMap<ResourceTypeName, Arc<dyn SpecDecoder>> {
        self.decoders.clone()
    }

    /// The resource verbs one registered type declares, when it is
    /// registered. The seed's Role-rule check reads this: a rule may only
    /// grant verbs the type's own declaration carries.
    pub fn declared_verbs(&self, type_name: &ResourceTypeName) -> Option<&[String]> {
        self.declared_verbs.get(type_name).map(Vec::as_slice)
    }

    /// Mark the plane open: from here on a driver whose allowed-source mask
    /// lacks RUNTIME can no longer be registered, so a declaration that
    /// missed the open fails startup at its registration instead of being
    /// served late.
    pub fn mark_plane_open(&mut self) {
        self.plane_open = true;
    }

    /// Build the erased driver for a desired resource, resolving the
    /// factory by the key's `type_name` component.
    pub async fn create_driver(
        &self,
        key: &ResourceKey,
    ) -> Result<Box<dyn DynResourceDriver>, ProviderDirectoryError> {
        let type_name = ResourceTypeName::new(key.type_name.clone());
        let factory = self
            .lookup(&type_name)
            .ok_or(ProviderDirectoryError::UnknownType(type_name))?;
        Ok(factory.create(key).await)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{DriverRegistration, ProviderDirectory, ProviderDirectoryError, ResourceDriverFactory};
    use crate::context::test_support::{fixture, test_row, DeadManager, FailingDecoder, NullRequeue};
    use crate::context::{ResourceContext, SpecDecoder};
    use crate::driver::{DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver};
    use crate::error::{DriverFailure, DriverOp, FailureClass};
    use crate::identity::{ResourceKey, ResourceTypeName};

    /// Driver used to exercise the directory surface end to end.
    #[derive(Clone)]
    struct StubDriver {
        classify_terminal: bool,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("stub driver failure")]
    struct StubError;

    #[async_trait::async_trait]
    impl ResourceDriver for StubDriver {
        type Error = StubError;

        fn classify_error(&self, _error: &StubError) -> DriverFailure {
            if self.classify_terminal {
                DriverFailure::terminal(DriverOp::Validate)
            } else {
                DriverFailure::retryable(DriverOp::Validate)
            }
        }

        async fn validate(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            Err(StubError)
        }

        async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
            Ok(RecoveryOutcome::Missing)
        }

        async fn reconcile(&mut self, _ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
            Ok(ReconcileOutcome::Satisfied)
        }

        async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    struct StubFactory {
        types: Vec<ResourceTypeName>,
        classify_terminal: bool,
    }

    #[async_trait::async_trait]
    impl ResourceDriverFactory for StubFactory {
        fn resource_types(&self) -> &[ResourceTypeName] {
            &self.types
        }

        async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
            Box::new(StubDriver { classify_terminal: self.classify_terminal })
        }
    }

    /// One declared driver for the registry's declaration path.
    struct StubRegistration {
        resource_type: &'static str,
        requires_plane_registration: bool,
        operation_refs: Vec<&'static str>,
        decoder: Arc<dyn SpecDecoder>,
        factory: Arc<dyn ResourceDriverFactory>,
    }

    impl StubRegistration {
        fn new(resource_type: &'static str) -> Self {
            Self {
                resource_type,
                requires_plane_registration: true,
                operation_refs: Vec::new(),
                decoder: Arc::new(FailingDecoder),
                factory: Arc::new(StubFactory {
                    types: vec![ResourceTypeName::new(resource_type)],
                    classify_terminal: false,
                }),
            }
        }
    }

    impl DriverRegistration for StubRegistration {
        fn resource_type(&self) -> ResourceTypeName {
            ResourceTypeName::new(self.resource_type)
        }

        fn requires_plane_registration(&self) -> bool {
            self.requires_plane_registration
        }

        fn operation_refs(&self) -> Vec<String> {
            self.operation_refs
                .iter()
                .map(|operation_ref| (*operation_ref).to_owned())
                .collect()
        }

        fn decoder(&self) -> Arc<dyn SpecDecoder> {
            Arc::clone(&self.decoder)
        }

        fn factory(&self) -> Arc<dyn ResourceDriverFactory> {
            Arc::clone(&self.factory)
        }
    }



    /// The directory maps resource type names to factories and serves
    /// lookups by the `type_name` component of a `ResourceKey`.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn directory_registers_and_looks_up_factories_by_type() {
        let mut directory = ProviderDirectory::new();
        directory
            .register(Arc::new(StubFactory {
                types: vec![ResourceTypeName::new("Process"), ResourceTypeName::new("Volume")],
                classify_terminal: false,
            }))
            .expect("register");
        assert!(directory.lookup(&ResourceTypeName::new("Process")).is_some());
        assert!(directory.lookup(&ResourceTypeName::new("Volume")).is_some());
        assert!(directory.lookup(&ResourceTypeName::new("Endpoint")).is_none());
    }

    /// Registering two factories for one type is a provider-wiring bug and
    /// fails registration without clobbering the first factory.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn duplicate_type_registration_is_rejected() {
        let mut directory = ProviderDirectory::new();
        directory
            .register(Arc::new(StubFactory {
                types: vec![ResourceTypeName::new("Process")],
                classify_terminal: false,
            }))
            .expect("first register");
        let error = directory
            .register(Arc::new(StubFactory {
                types: vec![ResourceTypeName::new("Process")],
                classify_terminal: true,
            }))
            .unwrap_err();
        assert!(matches!(
            error,
            ProviderDirectoryError::DuplicateType(ref t) if *t == ResourceTypeName::new("Process")
        ));
        // The failed registration did not replace the original factory.
        let factory = directory.lookup(&ResourceTypeName::new("Process")).expect("original kept");
        let mut driver = factory.create(&ResourceKey::new("z", "Process", "worker-0")).await;
        let mut fixture = fixture(
            test_row("z", "Process", "worker-0"),
            DeadManager,
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let failure = driver.validate(&mut fixture.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Retryable, "original factory kept");
    }

    /// A declared driver registers its factory, decoder, and operation
    /// references, and the registry reports the set it serves.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn declared_drivers_register_and_report_their_types() {
        let mut directory = ProviderDirectory::new();
        let mut process = StubRegistration::new("Process");
        process.operation_refs = vec!["Process/spawn"];
        directory.register_driver(&process).expect("register Process");
        directory
            .register_driver(&StubRegistration::new("Volume"))
            .expect("register Volume");

        assert_eq!(
            directory.registered_types(),
            vec![ResourceTypeName::new("Process"), ResourceTypeName::new("Volume")]
        );
        assert!(directory.lookup(&ResourceTypeName::new("Process")).is_some());
        assert_eq!(directory.decoders().len(), 2);

        // A second declaration for a registered type is refused, naming the
        // type.
        let error = directory.register_driver(&StubRegistration::new("Process")).unwrap_err();
        assert!(matches!(
            &error,
            ProviderDirectoryError::DuplicateType(t) if *t == ResourceTypeName::new("Process")
        ));
    }

    /// One operation reference has one owner: a driver registering a
    /// reference another driver already registered is refused, and the
    /// refusal names the reference.
    #[test]
    fn foreign_operation_reference_registration_is_rejected() {
        let mut directory = ProviderDirectory::new();
        let mut spawner = StubRegistration::new("Process");
        spawner.operation_refs = vec!["Process/spawn", "Process/kill"];
        directory.register_driver(&spawner).expect("register Process");

        let mut thief = StubRegistration::new("Volume");
        thief.operation_refs = vec!["Process/kill"];
        let error = directory.register_driver(&thief).unwrap_err();
        match error {
            ProviderDirectoryError::ForeignOperation { operation_ref, owner } => {
                assert_eq!(operation_ref, "Process/kill");
                assert_eq!(owner, ResourceTypeName::new("Process"));
            }
            other => panic!("wrong refusal: {other}"),
        }
        // The refused registration changed nothing.
        assert!(directory.lookup(&ResourceTypeName::new("Volume")).is_none());
    }

    /// A driver whose mask lacks RUNTIME must be registered before the plane
    /// opens: after `mark_plane_open` its registration is refused, naming the
    /// type, while a driver that admits late registration still lands.
    #[test]
    fn a_driver_without_the_runtime_bit_cannot_arrive_after_the_plane_opens() {
        let mut directory = ProviderDirectory::new();
        directory.mark_plane_open();

        let error = directory
            .register_driver(&StubRegistration::new("Process"))
            .unwrap_err();
        match error {
            ProviderDirectoryError::RequiredBeforeOpen { type_name } => {
                assert_eq!(type_name, ResourceTypeName::new("Process"));
            }
            other => panic!("wrong refusal: {other}"),
        }

        let mut late = StubRegistration::new("Volume");
        late.requires_plane_registration = false;
        directory.register_driver(&late).expect("late registration admitted");
        assert_eq!(directory.registered_types(), vec![ResourceTypeName::new("Volume")]);
    }

    /// `create_driver` resolves the factory by the key's resource type and
    /// hands back the erased driver with classification intact.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn create_driver_builds_the_erased_driver_for_the_key_type() {
        let mut directory = ProviderDirectory::new();
        directory
            .register(Arc::new(StubFactory {
                types: vec![ResourceTypeName::new("Process")],
                classify_terminal: true,
            }))
            .expect("register");

        let key = ResourceKey::new("z", "Process", "worker-0");
        let mut driver = directory.create_driver(&key).await.expect("create_driver");

        let mut fixture = fixture(
            test_row("z", "Process", "worker-0"),
            DeadManager,
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let failure = driver.validate(&mut fixture.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);

        // Unregistered resource type fails lookup.
        let error = match directory
            .create_driver(&ResourceKey::new("z", "Endpoint", "e-0"))
            .await
        {
            Ok(_) => panic!("unregistered type must fail lookup"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ProviderDirectoryError::UnknownType(ref t) if *t == ResourceTypeName::new("Endpoint")
        ));
    }
}
