//! Provider registry producing drivers per resource type (U4, KTD3).

pub const MODULE_NAME: &str = "provider";

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::driver::{DynResourceDriver, ResourceDriverFactory};
use crate::identity::{ResourceKey, ResourceTypeName};

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
}

/// Registry mapping resource type names to driver factories (KTD3). The
/// manager (U3) resolves the factory for a desired resource's type here
/// before spawning its actor.
///
/// One factory per resource type: the rewrite's single-execution-model
/// invariant (R29) starts at registration - a second registration for a
/// known type is a wiring bug and fails without clobbering the first.
#[derive(Default)]
pub struct ProviderDirectory {
    factories: HashMap<ResourceTypeName, Arc<dyn ResourceDriverFactory>>,
}

impl ProviderDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a factory under every type it covers. Atomic: a duplicate
    /// type rejects the whole registration.
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

    /// The factory registered for a resource type, if any.
    pub fn lookup(&self, type_name: &ResourceTypeName) -> Option<Arc<dyn ResourceDriverFactory>> {
        self.factories.get(type_name).cloned()
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
            .ok_or_else(|| ProviderDirectoryError::UnknownType(type_name))?;
        Ok(factory.create(key).await)
    }
}

 #[cfg(test)]
 mod tests {
    use std::sync::Arc;

    use super::{ProviderDirectory, ProviderDirectoryError, ResourceDriverFactory};
    use crate::context::test_support::{fixture, test_row, DeadManager, FailingDecoder, NullRequeue};
    use crate::context::ResourceContext;
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

    /// The directory maps resource type names to factories and serves
    /// lookups by the `type_name` component of a `ResourceKey`.
    #[tokio::test]
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

    /// `create_driver` resolves the factory by the key's resource type and
    /// hands back the erased driver with classification intact.
    #[tokio::test]
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