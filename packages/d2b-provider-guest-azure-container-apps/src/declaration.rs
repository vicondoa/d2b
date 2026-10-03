//! The Azure Container Apps Guest provider's unified declaration (KTD1).
//!
//! This crate's whole self-description used to be a bag of provider
//! configuration - a gateway reference, an environment id, a resource group -
//! read by a controller that then talked to a cloud control plane. Nothing in
//! the crate said what the provider *is*: which resource type it reconciles,
//! where its controller runs, or what it can present to a consumer. This
//! module is that missing statement, and it is authored once:
//! [`azure_container_apps_declaration`] is the serializable
//! [`ProviderDeclarationSpec`] the packaging stage emits and the projection
//! consumes, and the remote authority in [`crate::authority`] reads its
//! presentation ceiling rather than restating it.
//!
//! # Why the provider reference is derived, not pinned
//!
//! [`ProviderDeclarationSpec::new`] derives the `Provider` reference from the
//! artifact id. The artifact id is the same string
//! [`crate::PROVIDER_REF`] already pins after `Provider/`, so the derived
//! reference is the identity the Guest family's own registration table and
//! the committed deployment rows already name - not a second one. The crate's
//! tests hold the two against each other.
//!
//! # Why the presentation ceiling is `None`
//!
//! A Container Apps sandbox is a remote container group revision. It is not a
//! mount tree and it is not a namespace, so this backend can realize neither
//! [`PresentationCapability::FilesystemPresentation`] nor
//! [`PresentationCapability::NamespaceFirstServiceSource`]. Declaring the
//! ceiling here rather than discovering it at the call site is what lets
//! [`crate::AcaRemoteAuthority`] refuse a local-style presentation *before*
//! it mutates anything in Azure (KTD11, AE2-AE4).
//!
//! # The declared digests are placeholders on purpose
//!
//! [`project_provider_graph`](d2b_contracts_provider::v3::project_provider_graph)
//! refuses a declaration whose declared configuration digest disagrees with the
//! verified build output's. A placeholder therefore cannot pass as a real build
//! identity: until the packaging stage binds the verified digest, the
//! projection refuses by name instead of admitting a component no build
//! produced.

use d2b_contracts_provider::v3::{
    ArtifactDigest, ComponentDescriptor, ComponentTargetCapability, ComponentType,
    ControllerInstanceScope, ControllerTargetKind, DeclaredComponent, DeclaredPlacement,
    EffectPortClass, PresentationCapability, ProviderDeclarationSpec, SetupRestriction,
};
use d2b_contracts_resource::v3::{
    ArtifactId, BoundedToken, ResourceTypeName, execution_policy::ExecutionDomain,
    resource_schema::PlacementAnchor,
};
use d2b_provider_toolkit::{DriverDescriptor, ProviderImplementationBindings};

/// The artifact this provider is published as.
///
/// The crate name is the artifact name everywhere the deployment already uses
/// it, so the derived `Provider` reference is exactly
/// [`crate::PROVIDER_REF`].
pub const ACA_ARTIFACT_ID: &str = "runtime-azure-container-apps";

/// The Guest reconciliation controller's component identity.
pub const GUEST_CONTROLLER: &str = "aca-guest-controller";

/// The one ResourceType this provider reconciles.
const OWNED_RESOURCE_TYPE: &str = "Guest";

/// The configuration digest this component declares.
///
/// A placeholder the packaging stage replaces with the verified build output's
/// digest; see the module note on why a placeholder fails closed.
const CONFIG_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The signed target artifact this component's Guest placement names.
///
/// A placeholder over the same reasoning as [`CONFIG_DIGEST`].
const TARGET_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// This provider's unified declaration.
///
/// # Panics
///
/// Panics when the frozen contract rejects a component this crate declares.
/// Every input is a compile-time constant of this crate, so a refusal here is
/// a defect in the crate rather than a runtime condition, and a `Provider`
/// that cannot describe itself must not start.
pub fn azure_container_apps_declaration() -> ProviderDeclarationSpec {
    let component = ComponentDescriptor::new(
        token(GUEST_CONTROLLER),
        ComponentType::Controller,
        [owned_resource_type()],
        [],
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the Container Apps controller descriptor is well formed")
    .with_controller_placement(
        ControllerInstanceScope::FixedExecutionTarget,
        [ControllerTargetKind::Guest],
    )
    .expect("the Container Apps controller's scope is well formed")
    .with_target_capabilities([guest_target_capability()])
    .expect("the Container Apps controller's target capabilities are well formed");
    let placement = DeclaredPlacement::new(
        [ControllerTargetKind::Guest],
        Some(PlacementAnchor::ExecutionRef),
    )
    .expect("the Container Apps controller's execution-target placement is well formed");
    let component = DeclaredComponent::new(
        component,
        placement,
        declared_presentation(),
        SetupRestriction::required_for(declared_presentation())
            .iter()
            .copied(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the Container Apps controller's declared component is well formed");
    ProviderDeclarationSpec::new(artifact_id(), [component], [])
        .expect("the Container Apps declaration is well formed")
}

/// This provider's implementation bindings.
///
/// The binding half is the descriptor table the composition root registers for
/// this provider, and it is the only place a decoder, factory, handler, or
/// service enters the declaration: nothing serializable can name one.
///
/// This crate contributes no descriptor of its own today - it is reached
/// through the Guest family's registration, not through a `Guest`-typed
/// driver - so the table a caller passes is empty until the composition root
/// binds one. The function exists so the declaration's realization has the
/// same shape as every other provider's.
pub fn azure_container_apps_bindings(
    drivers: &'static [DriverDescriptor],
) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

/// The presentation this remote backend realizes.
///
/// Read from the declaration rather than restated, so the authority and the
/// declaration cannot disagree about what a Container Apps Guest can present.
pub fn declared_presentation() -> PresentationCapability {
    PresentationCapability::None
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(ACA_ARTIFACT_ID).expect("the Container Apps artifact identifier is canonical")
}

fn owned_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(OWNED_RESOURCE_TYPE).expect("the Guest type name is canonical")
}

/// The Guest execution target the controller runs against.
///
/// The signed target artifact is the Guest the row anchors at, which is what
/// makes the placement an execution-reference placement rather than a Zone one.
fn guest_target_capability() -> ComponentTargetCapability {
    ComponentTargetCapability::new(
        ControllerTargetKind::Guest,
        digest(TARGET_DIGEST),
        [EffectPortClass::Runtime],
    )
    .expect("the Container Apps Guest target capability is well formed")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("the declared identity is a bounded token")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("the declared digest is canonical")
}

#[cfg(test)]
mod tests {
    use super::{ACA_ARTIFACT_ID, GUEST_CONTROLLER, azure_container_apps_declaration, token};
    use d2b_contracts_provider::v3::{ComponentType, PresentationCapability};

    /// One artifact, one derived `Provider` reference: the identity the
    /// Guest family's own registration table pins, not a second one.
    #[test]
    fn the_declared_provider_reference_is_the_one_the_guest_family_pins() {
        let spec = azure_container_apps_declaration();
        assert_eq!(spec.artifact_id().as_str(), ACA_ARTIFACT_ID);
        assert_eq!(spec.provider().to_canonical_string(), crate::PROVIDER_REF);
    }

    /// The declaration owns exactly the one type this crate reconciles, so a
    /// second owned type would be authority nothing here acts for.
    #[test]
    fn the_declaration_owns_exactly_the_guest_type() {
        let spec = azure_container_apps_declaration();
        let owned: Vec<String> = spec
            .owned_resource_types()
            .into_iter()
            .map(|name| name.to_canonical_string())
            .collect();
        assert_eq!(owned, ["Guest"]);
    }

    /// The controller is a reconciler at the Guest execution target and
    /// publishes no method: this provider drives a cloud control plane, it
    /// does not answer a hosted service call.
    #[test]
    fn the_component_is_a_placed_controller_with_no_method() {
        let spec = azure_container_apps_declaration();
        let component = spec
            .component(&token(GUEST_CONTROLLER))
            .expect("the controller is a declared component");
        assert_eq!(
            component.component().component_type(),
            ComponentType::Controller
        );
        assert!(component.methods().is_empty());
        assert!(
            component
                .placement()
                .targets()
                .contains(&d2b_contracts_provider::v3::ControllerTargetKind::Guest)
        );
    }

    /// A remote backend realizes no local presentation, and the declaration
    /// says so before the authority has to refuse anything.
    #[test]
    fn a_remote_backend_declares_no_local_presentation() {
        let spec = azure_container_apps_declaration();
        let component = spec
            .component(&token(GUEST_CONTROLLER))
            .expect("the controller is a declared component");
        assert_eq!(component.presentation(), PresentationCapability::None);
        assert!(!PresentationCapability::None.realizes(
            PresentationCapability::FilesystemPresentation
        ));
        assert!(!PresentationCapability::None.realizes(
            PresentationCapability::NamespaceFirstServiceSource
        ));
        assert!(PresentationCapability::None.realizes(PresentationCapability::None));
    }
}
