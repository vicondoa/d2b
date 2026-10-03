//! The Azure virtual machine Guest provider's unified declaration (KTD1).
//!
//! This crate used to describe itself only through two configuration structs -
//! a provider root and a per-Guest settings blob - and a controller that went
//! straight to ARM. Nothing in the crate said what the provider *is*: which
//! resource type it reconciles, where its controller runs, or what it can
//! present to a consumer. This module is that missing statement, authored
//! once. [`azure_vm_declaration`] is the serializable
//! [`ProviderDeclarationSpec`] the packaging stage emits and the projection
//! consumes, [`azure_vm_bindings`] is the realization half, and the remote
//! authority in [`crate::authority`] reads its presentation ceiling rather
//! than restating it.
//!
//! # Why the provider reference is derived, not pinned
//!
//! [`ProviderDeclarationSpec::new`] derives the `Provider` reference from the
//! artifact id. The artifact id is the same string [`crate::PROVIDER_REF`]
//! already pins after `Provider/`, so the derived reference is the identity
//! the Guest family's registration table and the committed deployment rows
//! already name - not a second one. The crate's tests hold the two against
//! each other.
//!
//! # Why the presentation ceiling is `None`
//!
//! An Azure virtual machine is a remote compute instance reached over the
//! network. It is not a mount tree and it is not a namespace, so this backend
//! realizes neither [`PresentationCapability::FilesystemPresentation`] nor
//! [`PresentationCapability::NamespaceFirstServiceSource`]. Its data disks
//! are attached as provider-owned Azure disks, which is a resource identity
//! this Provider realizes, not a consumer-visible presentation it stands in
//! for. Declaring the ceiling here is what lets
//! [`crate::AzureVmRemoteAuthority`] refuse a local-style presentation
//! *before* it mutates anything in the subscription (KTD11, AE2-AE4).
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
/// it, so the derived `Provider` reference is exactly [`crate::PROVIDER_REF`].
pub const AZURE_VM_ARTIFACT_ID: &str = "runtime-azure-virtual-machine";

/// The Guest reconciliation controller's component identity.
pub const GUEST_CONTROLLER: &str = "azure-vm-guest-controller";

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
pub fn azure_vm_declaration() -> ProviderDeclarationSpec {
    let descriptor = ComponentDescriptor::new(
        token(GUEST_CONTROLLER),
        ComponentType::Controller,
        [owned_resource_type()],
        [],
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the Azure VM controller descriptor is well formed")
    .with_controller_placement(
        ControllerInstanceScope::FixedExecutionTarget,
        [ControllerTargetKind::Guest],
    )
    .expect("the Azure VM controller's scope is well formed")
    .with_target_capabilities([guest_target_capability()])
    .expect("the Azure VM controller's target capabilities are well formed");
    let placement = DeclaredPlacement::new(
        [ControllerTargetKind::Guest],
        Some(PlacementAnchor::ExecutionRef),
    )
    .expect("the Azure VM controller's execution-target placement is well formed");
    let component = DeclaredComponent::new(
        descriptor,
        placement,
        declared_presentation(),
        SetupRestriction::required_for(declared_presentation())
            .iter()
            .copied(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the Azure VM controller's declared component is well formed");
    ProviderDeclarationSpec::new(artifact_id(), [component], [])
        .expect("the Azure VM declaration is well formed")
}

/// This provider's implementation bindings.
///
/// The binding half is the descriptor table the composition root registers for
/// this provider, and it is the only place a decoder, factory, handler, or
/// service enters the declaration: nothing serializable can name one.
///
/// This crate contributes no descriptor of its own today - it is reached
/// through the Guest family's registration - so the table a caller passes is
/// empty until the composition root binds one. The function exists so the
/// declaration's realization has the same shape as every other provider's.
pub fn azure_vm_bindings(drivers: &'static [DriverDescriptor]) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

/// The presentation this remote backend realizes.
///
/// Read from the declaration rather than restated, so the authority and the
/// declaration cannot disagree about what an Azure VM Guest can present.
pub fn declared_presentation() -> PresentationCapability {
    PresentationCapability::None
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(AZURE_VM_ARTIFACT_ID).expect("the Azure VM artifact identifier is canonical")
}

fn owned_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(OWNED_RESOURCE_TYPE).expect("the Guest type name is canonical")
}

/// The Guest execution target the controller runs against.
///
/// The signed target artifact is the Guest the row anchors at, which is what
/// makes the placement an execution-reference placement rather than a Zone
/// one.
fn guest_target_capability() -> ComponentTargetCapability {
    ComponentTargetCapability::new(
        ControllerTargetKind::Guest,
        digest(TARGET_DIGEST),
        [EffectPortClass::Runtime],
    )
    .expect("the Azure VM Guest target capability is well formed")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("the declared identity is a bounded token")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("the declared digest is canonical")
}

#[cfg(test)]
mod tests {
    use super::{AZURE_VM_ARTIFACT_ID, GUEST_CONTROLLER, azure_vm_declaration, token};
    use d2b_contracts_provider::v3::{ComponentType, ControllerTargetKind, PresentationCapability};
    use d2b_provider_toolkit::emit_declaration_canonical;

    /// One artifact, one derived `Provider` reference: the identity the Guest
    /// family's own registration table pins, not a second one.
    #[test]
    fn the_declared_provider_reference_is_the_one_the_guest_family_pins() {
        let spec = azure_vm_declaration();
        assert_eq!(spec.artifact_id().as_str(), AZURE_VM_ARTIFACT_ID);
        assert_eq!(spec.provider().to_canonical_string(), crate::PROVIDER_REF);
    }

    /// The declaration owns exactly the one type this crate reconciles.
    #[test]
    fn the_declaration_owns_exactly_the_guest_type() {
        let spec = azure_vm_declaration();
        let owned: Vec<String> = spec
            .owned_resource_types()
            .into_iter()
            .map(|name| name.to_canonical_string())
            .collect();
        assert_eq!(owned, ["Guest"]);
    }

    /// The controller is a reconciler at the Guest execution target and
    /// publishes no method: this provider drives ARM, it does not answer a
    /// hosted service call.
    #[test]
    fn the_component_is_a_placed_controller_with_no_method() {
        let spec = azure_vm_declaration();
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
                .contains(&ControllerTargetKind::Guest)
        );
    }

    /// A remote backend realizes no local presentation, and the declaration
    /// says so before the authority has to refuse anything.
    #[test]
    fn a_remote_backend_declares_no_local_presentation() {
        let spec = azure_vm_declaration();
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
    }

    /// The declared half is a value, not a key or a handler: it projects
    /// deterministically and carries the same canonical bytes every time,
    /// which is what a packaging stage consumes and what the projection
    /// verifies against a verified build output.
    #[test]
    fn the_declaration_emits_the_same_canonical_bytes_every_time() {
        assert_eq!(
            emit_declaration_canonical(&azure_vm_declaration()),
            emit_declaration_canonical(&azure_vm_declaration())
        );
    }
}
