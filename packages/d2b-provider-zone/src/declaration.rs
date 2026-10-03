//! The Zone provider's unified declaration (KTD1).
//!
//! The Zone family's whole self-description used to be a per-type
//! [`DriverDescriptor`] the composition root registered by hand, so nothing
//! tied the type the plane served to a declared artifact, a component, or an
//! execution target. This module is the family's one authored source: the
//! serializable [`ProviderDeclarationSpec`] generators project, and the
//! [`ProviderImplementationBindings`] that realize it.
//!
//! The declaration is derived, not transcribed. The artifact id is the same
//! family name the plane's registration and the packaging
//! (`d2b-provider-${artifactId}`) already use, so the declared `Provider`
//! reference is the one the deployment already names; and the crate's own
//! tests hold the declared ResourceType against the bound descriptor's, so
//! the two halves cannot name different types.
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
    resource_schema::PlacementAnchor,
};
use d2b_resource_types::{DriverDescriptor, ProviderImplementationBindings};

/// The artifact this family is published as.
///
/// The family name is the artifact name everywhere the deployment already
/// uses it: the plane registers the family by this name, the packaging builds
/// `d2b-provider-zone` from it, and the declaration's own `Provider` reference
/// is derived from it.
pub const ZONE_ARTIFACT_ID: &str = "zone";

/// The Zone family's controller component identity.
const CONTROLLER: &str = "zone-controller";

/// The one ResourceType this family owns.
///
/// Read from the same vocabulary the bound descriptor is built from; the
/// crate's own tests hold the two against each other.
const OWNED_RESOURCE_TYPE: &str = "Zone";

/// The configuration digest this component declares.
///
/// A placeholder the packaging stage replaces with the verified build
/// output's digest; see the module note on why a placeholder fails closed.
const CONFIG_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The signed target artifact this component's Zone placement names.
///
/// A placeholder over the same reasoning as [`CONFIG_DIGEST`].
const TARGET_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The Zone provider's unified declaration.
///
/// # Panics
///
/// Panics when the frozen contract rejects the component this family
/// declares. Every input is a compile-time constant of this crate, so a
/// refusal here is a defect in the crate rather than a runtime condition, and
/// a `Provider` that cannot describe its own Zone controller must not start.
pub fn zone_declaration() -> ProviderDeclarationSpec {
    let component = component_descriptor();
    let placement =
        DeclaredPlacement::new([ControllerTargetKind::Zone], Some(PlacementAnchor::Zone))
            .expect("the Zone controller's Zone-singleton placement is well formed");
    let component = DeclaredComponent::new(
        component,
        placement,
        PresentationCapability::None,
        SetupRestriction::required_for(PresentationCapability::None).iter().copied(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the Zone controller's declared component is well formed");
    ProviderDeclarationSpec::new(artifact_id(), [component], [])
        .expect("the Zone family's declaration is well formed")
}

/// The Zone family's implementation bindings.
///
/// The binding half is the descriptor table the plane registers for this
/// provider, and it is the only place a decoder, factory, handler, or
/// service enters the declaration: nothing serializable can name one.
pub fn zone_bindings(drivers: &'static [DriverDescriptor]) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(ZONE_ARTIFACT_ID).expect("the Zone artifact identifier is canonical")
}

fn owned_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(OWNED_RESOURCE_TYPE).expect("the Zone type name is canonical")
}

/// The Zone controller's signed code identity.
///
/// A `Zone` row is the zone's own row: it is placed at the containing Zone,
/// reconciles in the system domain, declares no method of its own, and
/// creates no child.
fn component_descriptor() -> ComponentDescriptor {
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
    .expect("the Zone controller descriptor is well formed")
    .with_controller_placement(
        ControllerInstanceScope::ZoneSingleton,
        [ControllerTargetKind::Zone],
    )
    .expect("the Zone controller's scope is well formed")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Zone,
        digest(TARGET_DIGEST),
        [],
    )
    .expect("the Zone target capability is well formed")])
    .expect("the Zone controller's target capabilities are well formed")
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

    use d2b_provider_toolkit::declaration::provider::ProviderDeclaration;


    use super::*;

    static DRIVERS: LazyLock<[DriverDescriptor; 1]> =
        LazyLock::new(|| [crate::driver::zone_descriptor()]);

    fn declaration() -> ProviderDeclaration {
        ProviderDeclaration::new(zone_declaration(), zone_bindings(&DRIVERS[..]))
    }

    /// The two halves agree: the declaration owns exactly the type the bound
    /// driver serves, and it projects the provider the plane registers this
    /// family by.
    #[test]
    fn the_declaration_and_its_bindings_agree() {
        let declaration = declaration();
        declaration
            .validate()
            .expect("the Zone declaration's two halves agree");
        let identities = declaration
            .identities()
            .expect("the Zone declaration projects its identities");
        assert_eq!(identities.resource_types, ["Zone".to_owned()].into());
        assert_eq!(
            declaration.provider().to_canonical_string(),
            "Provider/zone"
        );
    }

    /// The declared type is the bound driver's own type, not a second
    /// transcription of it: a drift between the two is what this pins.
    #[test]
    fn the_declared_type_is_the_bound_drivers_type() {
        let bound = DRIVERS[0]
            .resource_type
            .to_resource_type_name();
        assert_eq!(bound.as_str(), owned_resource_type().as_str());
    }

    /// A `Zone` row is the zone's own row, so the component is a Zone
    /// singleton anchored at the Zone: a declaration that could place the
    /// controller anywhere else would not be this family's declaration.
    #[test]
    fn the_controller_is_a_zone_singleton_anchored_at_the_zone() {
        let declaration = zone_declaration();
        let component = declaration
            .components()
            .first()
            .expect("the Zone family declares one component");
        assert_eq!(
            component
                .component()
                .instance_scope()
                .expect("a controller declares its scope"),
            ControllerInstanceScope::ZoneSingleton
        );
        assert_eq!(component.placement().anchor(), Some(PlacementAnchor::Zone));
        assert!(
            component
                .placement()
                .targets()
                .contains(&ControllerTargetKind::Zone)
        );
    }
}
