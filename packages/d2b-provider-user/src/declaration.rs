//! The User provider's unified declaration (KTD1).
//!
//! The User family used to describe itself only through a per-type
//! [`DriverDescriptor`] the composition root registered by hand, so nothing
//! tied the type the plane served to a declared artifact, a component, an
//! execution target, or the one method the family actually publishes. This
//! module is the family's one authored source: the serializable
//! [`ProviderDeclarationSpec`] generators project, and the
//! [`ProviderImplementationBindings`] that realize it.
//!
//! The declaration is derived from the family's own tables rather than
//! transcribed. The declared service and its method are read from the same
//! [`USER_EFFECTS_SERVICE`] the descriptor carries, so the spec cannot claim
//! a method the crate does not serve; the declared ResourceType comes from the
//! contracts constant the User base spec itself uses, and the crate's tests
//! hold it against the bound descriptor's own type.
//!
//! # Two components, because there are two things
//!
//! The controller reconciles `User` rows and publishes no method. The hosted
//! effects service answers `inspect-user` at the Host execution target, which
//! is where the family's bounded discovery probe runs. Collapsing them would
//! either give the controller a method it does not serve or give the service
//! an owned ResourceType it does not reconcile.
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
    ControllerInstanceScope, ControllerTargetKind, DeclaredComponent, DeclaredMethod,
    DeclaredPlacement, DeclaredService, EffectPortClass, PresentationCapability,
    ProviderDeclarationSpec, SetupRestriction,
};
use d2b_contracts_resource::v3::{
    ArtifactId, BoundedToken, ResourceTypeName, execution_policy::ExecutionDomain,
    resource_schema::PlacementAnchor, user::USER_RESOURCE_TYPE,
};
use d2b_resource_types::{DriverDescriptor, ProviderImplementationBindings};

use crate::effects_service::USER_EFFECTS_SERVICE;

/// The artifact this family is published as.
///
/// The family name is the artifact name everywhere the deployment already
/// uses it: the plane registers the family by this name, the packaging builds
/// `d2b-provider-user` from it, and the declaration's own `Provider` reference
/// is derived from it.
pub const USER_ARTIFACT_ID: &str = "user";

/// The User family's controller component identity.
const CONTROLLER: &str = "user-controller";

/// The User family's hosted effects-service component identity.
const EFFECTS_SERVICE: &str = "user-effects";

/// The configuration digest both components declare.
///
/// A placeholder the packaging stage replaces with the verified build
/// output's digest; see the module note on why a placeholder fails closed.
const CONFIG_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The signed target artifact the Host placement names.
///
/// A placeholder over the same reasoning as [`CONFIG_DIGEST`].
const TARGET_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The User provider's unified declaration.
///
/// # Panics
///
/// Panics when the frozen contract rejects a component this family declares.
/// Every input is a compile-time constant of this crate or read from its own
/// service table, so a refusal here is a defect in the crate rather than a
/// runtime condition, and a `Provider` that cannot describe itself must not
/// start.
pub fn user_declaration() -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(artifact_id(), [controller_component(), service_component()], [])
        .expect("the User family's declaration is well formed")
}

/// The User family's implementation bindings.
///
/// The binding half is the descriptor table the plane registers for this
/// provider, and it is the only place a decoder, factory, handler, or
/// service enters the declaration: nothing serializable can name one.
pub fn user_bindings(drivers: &'static [DriverDescriptor]) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(USER_ARTIFACT_ID).expect("the User artifact identifier is canonical")
}

/// The one ResourceType this family owns, named by the same contract constant
/// the User base spec uses.
fn owned_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(USER_RESOURCE_TYPE).expect("the User type name is canonical")
}

/// The controller: it reconciles `User` rows and publishes no method.
fn controller_component() -> DeclaredComponent {
    let component = ComponentDescriptor::new(
        token(CONTROLLER),
        ComponentType::Controller,
        [owned_resource_type()],
        [],
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the User controller descriptor is well formed")
    .with_controller_placement(
        ControllerInstanceScope::FixedExecutionTarget,
        [ControllerTargetKind::Host],
    )
    .expect("the User controller's scope is well formed")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(TARGET_DIGEST),
        [EffectPortClass::Substrate],
    )
    .expect("the User target capability is well formed")])
    .expect("the User controller's target capabilities are well formed");
    let placement =
        DeclaredPlacement::new([ControllerTargetKind::Host], Some(PlacementAnchor::ExecutionRef))
            .expect("the User controller's execution-target placement is well formed");
    DeclaredComponent::new(
        component,
        placement,
        PresentationCapability::None,
        SetupRestriction::required_for(PresentationCapability::None).iter().copied(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the User controller's declared component is well formed")
}

/// The hosted effects service: the one method the family publishes, read from
/// the bound service declaration the descriptor carries.
fn service_component() -> DeclaredComponent {
    let methods: Vec<BoundedToken> = USER_EFFECTS_SERVICE
        .methods
        .iter()
        .map(|method| token(method.name))
        .collect();
    let component = ComponentDescriptor::new(
        token(EFFECTS_SERVICE),
        ComponentType::Service,
        [],
        methods.iter().cloned(),
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the User effects-service descriptor is well formed")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(TARGET_DIGEST),
        [EffectPortClass::Substrate],
    )
    .expect("the User target capability is well formed")])
    .expect("the User effects service's target capabilities are well formed");
    let placement = DeclaredPlacement::new([ControllerTargetKind::Host], None)
        .expect("the User effects service's host placement is well formed");
    let service = DeclaredService::new(USER_EFFECTS_SERVICE.id, methods.clone())
        .expect("the User effects service's declared service is well formed");
    let declared = methods
        .into_iter()
        .map(|method| DeclaredMethod::new(method, None, PresentationCapability::None))
        .collect();
    DeclaredComponent::new(
        component,
        placement,
        PresentationCapability::None,
        SetupRestriction::required_for(PresentationCapability::None).iter().copied(),
        declared,
        vec![service],
        Vec::new(),
    )
    .expect("the User effects service's declared component is well formed")
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
    use crate::test_support::{ScriptedProbe, recording_facets};

    static DRIVERS: LazyLock<[DriverDescriptor; 1]> =
        LazyLock::new(|| [crate::driver::user_descriptor(recording_facets(ScriptedProbe::new()))]);

    fn declaration() -> ProviderDeclaration {
        ProviderDeclaration::new(user_declaration(), user_bindings(&DRIVERS[..]))
    }

    /// The two halves agree: the declaration owns exactly the type the bound
    /// driver serves, and the bound effects service realizes the one method
    /// the declaration publishes.
    #[test]
    fn the_declaration_and_its_bindings_agree() {
        let declaration = declaration();
        declaration
            .validate()
            .expect("the User declaration's two halves agree");
        let identities = declaration
            .identities()
            .expect("the User declaration projects its identities");
        assert_eq!(identities.resource_types, ["User".to_owned()].into());
        let services: Vec<&str> = identities
            .services
            .iter()
            .map(|service| service.as_str())
            .collect();
        assert_eq!(services, [USER_EFFECTS_SERVICE.id]);
        assert_eq!(
            declaration.provider().to_canonical_string(),
            "Provider/user"
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

    /// The published method is the crate's own: the declared service and its
    /// methods are read from the bound `ServiceDecl`, so the declaration
    /// cannot claim a discovery probe the family does not answer.
    #[test]
    fn the_declared_method_is_the_services_own() {
        let declaration = user_declaration();
        let methods: Vec<&str> = declaration
            .components()
            .iter()
            .flat_map(|component| component.methods())
            .map(|method| method.name().as_str())
            .collect();
        assert_eq!(methods, ["inspect-user"]);
    }
}
