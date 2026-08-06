//! Exact Provider instance registration.
//!
//! Registration is intentionally transactional at the toolkit boundary:
//! descriptors are validated and duplicate references are rejected before the
//! runtime builder is allowed to consume any entry.

use std::{collections::BTreeSet, error::Error, fmt};

use d2b_provider::{ProviderDescriptor, ProviderRegistryBuilder, RegistryBuildError};

/// A value accepted by [`register_exact_instances`].
pub trait ExactRegistration<I> {
    /// Split an entry into its immutable descriptor and runtime handle.
    fn split(self) -> (ProviderDescriptor, I);
}

impl<I> ExactRegistration<I> for (ProviderDescriptor, I) {
    fn split(self) -> (ProviderDescriptor, I) {
        self
    }
}

/// Errors reported before or during exact registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolkitError {
    /// No instances were supplied.
    EmptyRegistration,
    /// A descriptor failed its own validation.
    DescriptorInvalid,
    /// Two entries used the same Provider reference.
    DuplicateProvider,
    /// The runtime registry rejected the transaction.
    Registry(RegistryBuildError),
}

impl fmt::Display for ToolkitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRegistration => "provider registration is empty",
            Self::DescriptorInvalid => "provider descriptor is invalid",
            Self::DuplicateProvider => "duplicate provider registration",
            Self::Registry(_) => "provider registry rejected registration",
        })
    }
}

impl Error for ToolkitError {}

impl From<RegistryBuildError> for ToolkitError {
    fn from(value: RegistryBuildError) -> Self {
        Self::Registry(value)
    }
}

/// Register a set of descriptor and instance pairs with exact identity checks.
///
/// All caller entries are validated before the builder is mutated. The
/// runtime builder still performs its Zone and generation checks, so this
/// helper does not weaken the registry's fail-closed admission.
pub fn register_exact_instances<I, Entries>(
    builder: &mut ProviderRegistryBuilder<I>,
    entries: Entries,
) -> Result<(), ToolkitError>
where
    Entries: IntoIterator,
    Entries::Item: ExactRegistration<I>,
{
    let entries: Vec<(ProviderDescriptor, I)> =
        entries.into_iter().map(ExactRegistration::split).collect();
    if entries.is_empty() {
        return Err(ToolkitError::EmptyRegistration);
    }

    let mut seen = BTreeSet::new();
    for (descriptor, _) in &entries {
        descriptor
            .validate()
            .map_err(|_| ToolkitError::DescriptorInvalid)?;
        if !seen.insert(descriptor.provider_ref().clone()) {
            return Err(ToolkitError::DuplicateProvider);
        }
    }

    for (descriptor, instance) in entries {
        builder.register_instance(descriptor, instance)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Fixture;
    use d2b_provider::{ProviderClass, ProviderRegistryBuilder};

    #[test]
    fn exact_registration_rejects_empty_and_duplicate_inputs_before_building() {
        let fixture = Fixture::new(ProviderClass::Runtime, 0).expect("fixture");
        let mut builder = ProviderRegistryBuilder::new(
            fixture.zone().clone(),
            fixture.descriptor.registry_generation(),
        );
        assert_eq!(
            register_exact_instances(&mut builder, Vec::<(ProviderDescriptor, ())>::new()),
            Err(ToolkitError::EmptyRegistration)
        );

        let mut builder = ProviderRegistryBuilder::new(
            fixture.zone().clone(),
            fixture.descriptor.registry_generation(),
        );
        let entries = [
            (fixture.descriptor.clone(), ()),
            (fixture.descriptor.clone(), ()),
        ];
        assert_eq!(
            register_exact_instances(&mut builder, entries),
            Err(ToolkitError::DuplicateProvider)
        );
    }

    #[test]
    fn exact_registration_builds_a_registry_with_the_supplied_handles() {
        let first = Fixture::new(ProviderClass::Runtime, 0).expect("fixture");
        let second = Fixture::new(ProviderClass::Runtime, 1).expect("fixture");
        let mut builder = ProviderRegistryBuilder::new(
            first.zone().clone(),
            first.descriptor.registry_generation(),
        );
        register_exact_instances(
            &mut builder,
            [
                (first.descriptor.clone(), "first"),
                (second.descriptor.clone(), "second"),
            ],
        )
        .expect("registration");
        let registry = builder.finish().expect("registry");
        assert_eq!(
            registry.instance(first.descriptor.provider_ref()),
            Some("first")
        );
        assert_eq!(
            registry.instance(second.descriptor.provider_ref()),
            Some("second")
        );
    }
}
