//! The User family's contribution to its Provider's declaration (KTD1).
//!
//! `ADR-046-provider-model-and-packaging` gives `User` to exactly one
//! Provider, and that Provider is `Provider/system-core` - the same identity
//! the committed `User` rows already pin in `spec.providerRef`. This crate
//! therefore does not author a provider identity: it contributes the binding
//! half, the [`DriverDescriptor`] table the plane registers for the `User`
//! type, and the family crates are named by that identity's declaration
//! rather than by a second one.
//!
//! Declaring `Provider/user` here would be exactly the unattached authority
//! the unified declaration exists to remove (AE14): a mutable `Provider` row
//! selecting a `user` artifact would name a code identity with no manifest,
//! no packaging chain, and no bootstrap admission behind it, while the rows
//! that actually reconcile `User` name `Provider/system-core`. The identity,
//! the components, and the execution targets are declared once, in
//! [`d2b_provider_system_core::system_core_declaration`].
//!
//! What this crate's own tests hold is the part it can prove alone: that the
//! descriptor it registers serves the type the owning declaration licenses,
//! and that the declared effects service it carries is the very service that
//! declaration publishes. A drift between the two is what those assertions
//! fail on.

use d2b_provider_toolkit::{DriverDescriptor, ProviderImplementationBindings};

/// The User family's implementation bindings.
///
/// The binding half is the descriptor table the plane registers for the
/// `Provider/system-core` identity that owns `User`, and it is the only place
/// a decoder, factory, handler, or service enters that declaration: nothing
/// serializable can name one.
pub fn user_bindings(drivers: &'static [DriverDescriptor]) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use d2b_contracts_resource::v3::{ResourceTypeName, user::USER_RESOURCE_TYPE};
    use d2b_provider_system_core::{PROVIDER_REF, system_core_declaration};
    use d2b_provider_toolkit::DriverDescriptor;

    use crate::effects_service::USER_EFFECTS_SERVICE;
    use crate::test_support::{ScriptedProbe, recording_facets};

    static DRIVERS: LazyLock<[DriverDescriptor; 1]> =
        LazyLock::new(|| [crate::driver::user_descriptor(recording_facets(ScriptedProbe::new()))]);

    /// The one ResourceType this crate serves, named by the same contract
    /// constant the owning Provider's declaration and its own base spec use.
    fn owned_resource_type() -> ResourceTypeName {
        ResourceTypeName::parse(USER_RESOURCE_TYPE).expect("the User type name is canonical")
    }

    /// The bound descriptor serves the type the owning Provider's declaration
    /// licenses, and nothing else: this crate contributes implementation for
    /// `User` and declares no identity of its own, so a descriptor here that
    /// served an undeclared type would be a driver no declaration licenses.
    #[test]
    fn the_bound_descriptor_serves_a_type_the_owning_declaration_licenses() {
        let spec = system_core_declaration();
        let served = DRIVERS[0].resource_type.to_resource_type_name();
        assert_eq!(served.as_str(), owned_resource_type().as_str());
        assert!(
            spec.owned_resource_types()
                .iter()
                .any(|declared| declared.as_str() == served.as_str()),
            "the system-core declaration owns {}",
            served.as_str()
        );
        // The declaration owns the Provider's other type too; this crate
        // contributes the implementation for exactly one of them, and the
        // sibling family crate contributes the other. A descriptor here
        // serving both would mean one crate registered two types under a
        // single Provider identity it does not own.
        let served_count = DRIVERS
            .iter()
            .filter(|descriptor| {
                spec.owned_resource_types().iter().any(|declared| {
                    declared.as_str() == descriptor.resource_type.to_resource_type_name().as_str()
                })
            })
            .count();
        assert_eq!(served_count, 1, "this crate contributes one bound type");
    }

    /// The service this descriptor carries is the service the owning
    /// declaration publishes, by the same constant: the two halves cannot
    /// name different services because there is only one value.
    #[test]
    fn the_bound_service_is_the_one_the_owning_declaration_publishes() {
        let spec = system_core_declaration();
        let bound: Vec<&str> = DRIVERS[0]
            .services
            .iter()
            .map(|service| service.id)
            .collect();
        assert_eq!(bound, [USER_EFFECTS_SERVICE.id]);
        let declared: Vec<&str> = spec
            .components()
            .iter()
            .flat_map(|component| component.services())
            .map(|service| service.id().as_str())
            .collect();
        assert!(
            declared.contains(&USER_EFFECTS_SERVICE.id),
            "declared: {declared:?}"
        );
        // The method is read from that one service constant, so the
        // declaration cannot claim a probe this crate does not answer.
        let methods: Vec<&str> = spec
            .components()
            .iter()
            .flat_map(|component| component.methods())
            .map(|method| method.name().as_str())
            .collect();
        assert!(methods.contains(&"inspect-user"), "declared: {methods:?}");
    }

    /// The service the owning Provider's declaration publishes is this
    /// crate's own service: same identity, same methods. The two tables live
    /// in the crates that own them - the declaring Provider below its
    /// families - so this comparison is what holds them together, and a
    /// rename on either side fails here rather than at session dispatch.
    #[test]
    fn the_declared_service_is_this_crates_own_service() {
        let spec = system_core_declaration();
        let declared = spec
            .components()
            .iter()
            .flat_map(|component| component.services())
            .find(|service| service.id().as_str() == USER_EFFECTS_SERVICE.id)
            .expect("the owning declaration publishes this family's service");
        let declared_methods: Vec<&str> = declared
            .methods()
            .iter()
            .map(|method| method.as_str())
            .collect();
        let own_methods: Vec<&str> = USER_EFFECTS_SERVICE
            .methods
            .iter()
            .map(|method| method.name)
            .collect();
        assert_eq!(declared_methods, own_methods);
    }

    /// This crate registers no provider identity of its own: a `Provider`
    /// row selecting one would name an artifact with no manifest and no
    /// bootstrap admission behind it.
    #[test]
    fn this_crate_declares_no_provider_identity_of_its_own() {
        assert_eq!(
            system_core_declaration().provider().to_canonical_string(),
            PROVIDER_REF
        );
    }
}
