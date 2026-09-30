//! The Provider provider's unified declaration (KTD1).
//!
//! The Provider family used to describe itself only through a per-type
//! [`DriverDescriptor`] the composition root registered by hand, so nothing
//! tied the type the plane served to a declared artifact, a component, or an
//! execution target. This module is the family's one authored source: the
//! serializable [`ProviderDeclarationSpec`] generators project, and the
//! [`ProviderImplementationBindings`] that realize it.
//!
//! `Provider` is the one core type with behavior beyond convergence: the
//! driver applies a readiness observation to a Provider's owned controller
//! `Process` rows and state `Volume` rows. The declaration says so by naming
//! the controller and by requiring the `Volume` capability it needs to read
//! those rows; it names no method, because the family publishes none.
//!
//! # The declared digest is a placeholder on purpose
//!
//! [`project_provider_graph`](d2b_contracts_provider::v3::project_provider_graph)
//! refuses a declaration whose declared configuration digest disagrees with
//! the verified build output's. A placeholder therefore cannot pass as a real
//! build identity: until the packaging stage binds the verified digest, the
//! projection refuses by name instead of admitting a component no build
//! produced.

use d2b_contracts_provider::v3::{
    ArtifactDigest, ComponentDescriptor, ComponentTargetCapability, ComponentType,
    ControllerInstanceScope, ControllerTargetKind, DeclaredComponent, DeclaredPlacement,
    PresentationCapability, ProviderDeclarationSpec, SetupRestriction,
};
use d2b_contracts_resource::v3::{
    ArtifactId, BoundedToken, ResourceTypeName, execution_policy::ExecutionDomain,
    operation::PROVIDER_RESOURCE_TYPE, resource_schema::PlacementAnchor,
};
use d2b_resource_types::{DriverDescriptor, ProviderImplementationBindings};

use crate::driver::PROVIDER_TYPE_NAME;

/// The artifact this family is published as.
///
/// The family name is the artifact name everywhere the deployment already
/// uses it: the plane registers the family by this name, the packaging builds
/// `d2b-provider-provider` from it, and the declaration's own `Provider`
/// reference is derived from it.
pub const PROVIDER_ARTIFACT_ID: &str = "provider";

/// The Provider family's controller component identity.
const CONTROLLER: &str = "provider-controller";

// The declaration names no required resource capability. A requirement is
// admitted only for a type the same declaration binds, and this family's
// only type is `Provider`; naming a capability for the `Volume` rows the
// observation reads would therefore be a claim the contract refuses rather
// than one it enforces. The signed capability matrix is where that claim
// belongs, and the verified artifact's matrix is what admits it.
/// The configuration digest this component declares.
///
/// A placeholder the packaging stage replaces with the verified build
/// output's digest; see the module note on why a placeholder fails closed.
const CONFIG_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The signed target artifact the controller placement names.
///
/// A placeholder over the same reasoning as [`CONFIG_DIGEST`].
const TARGET_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The Provider provider's unified declaration.
///
/// # Panics
///
/// Panics when the frozen contract rejects a component this family declares.
/// Every input is a compile-time constant of this crate, so a refusal here is
/// a defect in the crate rather than a runtime condition, and a `Provider`
/// that cannot describe itself must not start.
pub fn provider_declaration() -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(artifact_id(), [controller_component()], [])
        .expect("the Provider family's declaration is well formed")
}

/// The Provider family's implementation bindings.
///
/// The binding half is the descriptor table the plane registers for this
/// provider, and it is the only place a decoder, factory, handler, or
/// service enters the declaration: nothing serializable can name one.
pub fn provider_bindings(
    drivers: &'static [DriverDescriptor],
) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(PROVIDER_ARTIFACT_ID).expect("the Provider artifact identifier is canonical")
}


/// The one ResourceType this family owns, named by the crate's own constant
/// rather than a second transcription of it.
fn owned_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(PROVIDER_TYPE_NAME)
        .expect("the Provider type name is canonical")
}

/// The controller: it reconciles `Provider` rows and publishes no method.
fn component_descriptor() -> ComponentDescriptor {
    debug_assert_eq!(owned_resource_type().as_str(), PROVIDER_RESOURCE_TYPE);
    ComponentDescriptor::new(
        token(CONTROLLER),
        ComponentType::Controller,
        [owned_resource_type()],
        [],
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the Provider controller descriptor is well formed")
    .with_controller_placement(
        ControllerInstanceScope::FixedExecutionTarget,
        [ControllerTargetKind::Host],
    )
    .expect("the Provider controller's scope is well formed")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(TARGET_DIGEST),
        [],
    )
    .expect("the Provider target capability is well formed")])
    .expect("the Provider controller's target capabilities are well formed")
}

/// The declared component: the controller's placement is the Host execution
/// target, it presents nothing, and it creates no child.
fn controller_component() -> DeclaredComponent {
    let placement =
        DeclaredPlacement::new([ControllerTargetKind::Host], Some(PlacementAnchor::ExecutionRef))
            .expect("the Provider controller's execution-target placement is well formed");
    DeclaredComponent::new(
        component_descriptor(),
        placement,
        PresentationCapability::None,
        SetupRestriction::required_for(PresentationCapability::None).iter().copied(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the Provider controller's declared component is well formed")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("the declared identity is a bounded token")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("the declared digest is canonical")
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use d2b_contracts_provider::v3::{ProviderContractError, RequiredResourceCapability};
    use d2b_provider_toolkit::declaration::provider::ProviderDeclaration;

    use super::*;
    use crate::driver::ProviderDriverArgs;
    use crate::test_support::RecordingEffects;

    fn resource_type(value: &str) -> ResourceTypeName {
        ResourceTypeName::parse(value).expect("the declared type name is canonical")
    }

    static DRIVERS: LazyLock<[DriverDescriptor; 1]> = LazyLock::new(|| {
        [crate::driver::provider_descriptor(ProviderDriverArgs {
            effects: RecordingEffects::new(),
        })]
    });

    fn declaration() -> ProviderDeclaration {
        ProviderDeclaration::new(provider_declaration(), provider_bindings(&DRIVERS[..]))
    }

    /// The two halves agree: the declaration owns exactly the type the bound
    /// driver serves, and it projects the provider the plane registers this
    /// family by.
    #[test]
    fn the_declaration_and_its_bindings_agree() {
        let declaration = declaration();
        declaration
            .validate()
            .expect("the Provider declaration's two halves agree");
        let identities = declaration
            .identities()
            .expect("the Provider declaration projects its identities");
        assert_eq!(identities.resource_types, ["Provider".to_owned()].into());
        assert!(
            identities.services.is_empty(),
            "the Provider family publishes no service"
        );
        assert_eq!(
            declaration.provider().to_canonical_string(),
            "Provider/provider"
        );
    }

    /// The declared type is the bound driver's own type, not a second
    /// transcription of it: a drift between the two is what this pins.
    #[test]
    fn the_declared_type_is_the_bound_drivers_type() {
        assert_eq!(
            DRIVERS[0].resource_type.to_resource_type_name().as_str(),
            owned_resource_type().as_str()
        );
    }

    /// A required capability is admitted only for a type the same
    /// declaration binds, so the observation's need of the `Volume` rows it
    /// reads cannot be stated as a requirement here: the contract refuses
    /// the claim rather than letting it through unchecked.
    #[test]
    fn a_required_capability_for_an_unowned_type_is_refused() {
        let refusal = ProviderDeclarationSpec::new(
            artifact_id(),
            [controller_component()],
            [RequiredResourceCapability::new(
                resource_type("Volume"),
                token("state-volume"),
            )],
        )
        .expect_err("a capability requirement for a type this family does not serve");
        assert_eq!(refusal, ProviderContractError::MissingRequiredField);
    }
}
