//! The `system-core` provider's unified declaration (KTD1).
//!
//! `Provider/system-core` is the one fixed core-controller process per Zone
//! (`ADR-046-provider-model-and-packaging`, section "system-core bootstrap"),
//! and it is the only Provider that owns `Host` and `User` - the closed
//! allowlist is [`crate::OWNED_RESOURCE_TYPES`], enforced in
//! [`crate::ownership`]. This module is that Provider's one authored source:
//! the serializable [`ProviderDeclarationSpec`] generators project, and the
//! [`ProviderImplementationBindings`] that realize it.
//!
//! # One artifact, two types, four components
//!
//! The two `system-core` types each have two halves, so the declaration
//! carries four components rather than one. A controller reconciles its rows
//! and publishes no method; the hosted effects service answers the family's
//! one `inspect-*` method at the Host execution target, which is where the
//! bounded capability, platform, and process probes run. Collapsing them
//! would either give a controller a method it does not serve or give a
//! service an owned ResourceType it does not reconcile.
//!
//! # One identity, not three
//!
//! The declared artifact is `system-core`, the crate this module lives in
//! and the family the plane registers and the packaging builds, so the
//! derived `Provider` reference is exactly the one the committed `Host` and
//! `User` rows already pin in `spec.providerRef`
//! ([`crate::PROVIDER_REF`]). `d2b-provider-host` and `d2b-provider-user`
//! contribute the driver descriptors and effects-service factories for their
//! own halves; they do not declare a `Provider/host` or `Provider/user`
//! identity, because a mutable row naming an artifact with no manifest and no
//! bootstrap admission behind it is exactly the unattached authority this
//! declaration exists to remove (AE14).
//!
//! # The declared service and methods are read, not transcribed
//!
//! The two declared services are the same [`ServiceDecl`] constants the
//! bound descriptors carry, so the spec cannot claim a method either family
//! does not serve; the crate's own tests hold the derived ResourceTypes
//! against [`OWNED_RESOURCE_TYPES`](crate::OWNED_RESOURCE_TYPES) and hold
//! the bound descriptors' own types against the declared ones.
//!
//! # The declared digest is a placeholder on purpose
//!
//! [`project_provider_graph`](d2b_contracts_provider::v3::project_provider_graph)
//! refuses a declaration whose declared configuration digest disagrees with
//! the verified build output's. A placeholder therefore cannot pass as a real
use d2b_contracts_provider::v3::{
    ArtifactDigest, BinaryRef, ComponentDescriptor, ComponentExecution, ComponentTargetCapability,
    ComponentType, ControllerInstanceScope, ControllerTargetKind, DeclaredComponent, DeclaredMethod,
    DeclaredPlacement, DeclaredService, EffectPortClass, PresentationCapability,
    ProviderDeclarationSpec, SetupRestriction,
};
use d2b_contracts_resource::v3::{
    ArtifactId, BoundedToken, ResourceTypeName, execution_policy::ExecutionDomain,
    host::HOST_RESOURCE_TYPE, resource_schema::PlacementAnchor, user::USER_RESOURCE_TYPE,
};
use d2b_provider_toolkit::{
    DriverDescriptor, ProviderImplementationBindings, ServiceDecl, ServiceMethod,
};

use crate::PROVIDER_NAME;

/// The artifact `system-core` is published as.
///
/// The crate name is the artifact name everywhere the deployment already uses
/// it: the plane registers the family by this name, the packaging builds
/// `d2b-provider-system-core` from it, and the declaration's own `Provider`
/// reference is derived from it.
pub const SYSTEM_CORE_ARTIFACT_ID: &str = PROVIDER_NAME;

/// The Host reconciliation controller's component identity.
const HOST_CONTROLLER: &str = "host-controller";

/// The Host hosted effects-service component identity.
const HOST_EFFECTS: &str = "host-effects";

/// The User reconciliation controller's component identity.
const USER_CONTROLLER: &str = "user-controller";

/// The User hosted effects-service component identity.
const USER_EFFECTS: &str = "user-effects";

/// The configuration digest every component declares.
///
/// A placeholder the packaging stage replaces with the verified build
/// output's digest; see the module note on why a placeholder fails closed.
const CONFIG_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The signed target artifact the Host placement every component names.
///
/// A placeholder over the same reasoning as [`CONFIG_DIGEST`].
const TARGET_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The Host family's declared effects service, as the bound `Host` descriptor
/// carries it.
///
/// `system-core` owns `Host`, so the service identity that provider publishes
/// is declared here rather than in the family crate that realizes it. The
/// value is the contract's, not a second transcription: `d2b-provider-host`
/// re-exports this constant, so the descriptor the plane registers and the
/// spec this crate publishes name the same service by construction.
pub const HOST_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "host.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-host")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The User family's declared effects service, over the same reasoning as
/// [`HOST_EFFECTS_SERVICE`].
pub const USER_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "user.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-user")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The `system-core` provider's unified declaration.
///
/// # Panics
///
/// Panics when the frozen contract rejects a component this Provider
/// declares. Every input is a compile-time constant of this crate or read from
/// its own service table, so a refusal here is a defect in the crate rather
/// than a runtime condition, and a `Provider` that cannot describe itself
/// must not start.
pub fn system_core_declaration() -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(
        artifact_id(),
        [
            controller_component(HOST_CONTROLLER, host_resource_type()),
            effects_component(HOST_EFFECTS, &HOST_EFFECTS_SERVICE),
            controller_component(USER_CONTROLLER, user_resource_type()),
            effects_component(USER_EFFECTS, &USER_EFFECTS_SERVICE),
        ],
        [],
    )
    .expect("the system-core declaration is well formed")
}

/// The `system-core` Provider's implementation bindings.
///
/// The binding half is the descriptor table the plane registers for this
/// Provider - the `Host` and `User` descriptors its two family crates
/// contribute - and it is the only place a decoder, factory, handler, or
/// service enters the declaration: nothing serializable can name one.
pub fn system_core_bindings(
    drivers: &'static [DriverDescriptor],
) -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(drivers)
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(SYSTEM_CORE_ARTIFACT_ID)
        .expect("the system-core artifact identifier is canonical")
}

/// The one ResourceType the Host controller owns, named by the same contract
/// constant this crate's ownership allowlist is built from.
fn host_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(HOST_RESOURCE_TYPE).expect("the Host type name is canonical")
}

/// The one ResourceType the User controller owns, over the same reasoning as
/// [`host_resource_type`].
fn user_resource_type() -> ResourceTypeName {
    ResourceTypeName::parse(USER_RESOURCE_TYPE).expect("the User type name is canonical")
}

/// A reconciling controller: it owns exactly one of this Provider's two types,
/// publishes no method, and is placed at the fixed Host execution target its
/// own rows carry.
fn controller_component(identity: &str, owned: ResourceTypeName) -> DeclaredComponent {
    let component = ComponentDescriptor::new(
        token(identity),
        ComponentType::Controller,
        [owned],
        [],
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the system-core controller descriptor is well formed")
    .with_controller_placement(
        ControllerInstanceScope::FixedExecutionTarget,
        [ControllerTargetKind::Host],
    )
    .expect("the system-core controller's scope is well formed")
    .with_target_capabilities([host_target_capability()])
    .expect("the system-core controller's target capabilities are well formed");
    let placement =
        DeclaredPlacement::new([ControllerTargetKind::Host], Some(PlacementAnchor::ExecutionRef))
            .expect("the system-core controller's execution-target placement is well formed");
    DeclaredComponent::new(
        component,
        placement,
        PresentationCapability::None,
        SetupRestriction::required_for(PresentationCapability::None).iter().copied(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the system-core controller's declared component is well formed")
}

/// A hosted effects service: it owns no ResourceType and publishes exactly the
/// methods its own [`ServiceDecl`] constant names, at the Host execution
/// target the probe runs on.
fn effects_component(identity: &str, service: &'static ServiceDecl) -> DeclaredComponent {
    let methods: Vec<BoundedToken> = service
        .methods
        .iter()
        .map(|method| token(method.name))
        .collect();
    let component = ComponentDescriptor::new(
        token(identity),
        ComponentType::Service,
        [],
        methods.iter().cloned(),
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("the system-core effects-service descriptor is well formed")
    // A service component is hosted as its own process, so it is launchable
    // rather than in-process: the frozen manifest contract refuses an
    // in-process service outright, and a declaration that could never be
    // admitted by a verified manifest would claim a component the packaging
    // stage cannot produce. The reconcilers above are the in-process half.
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse(identity).expect("the component's own binary reference"),
    })
    .with_target_capabilities([host_target_capability()])
    .expect("the system-core effects service's target capabilities are well formed");
    let placement = DeclaredPlacement::new([ControllerTargetKind::Host], None)
        .expect("the system-core effects service's host placement is well formed");
    let declared_service = DeclaredService::new(service.id, methods.clone())
        .expect("the system-core effects service's declared service is well formed");
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
        vec![declared_service],
        Vec::new(),
    )
    .expect("the system-core effects service's declared component is well formed")
}

/// The Host execution target every component of this Provider runs against.
fn host_target_capability() -> ComponentTargetCapability {
    ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(TARGET_DIGEST),
        [EffectPortClass::Substrate],
    )
    .expect("the system-core Host target capability is well formed")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("the declared identity is a bounded token")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("the declared digest is canonical")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::LazyLock;

    use d2b_provider_toolkit::declaration::provider::ProviderDeclaration;

    use super::*;

    /// The declaration and its bindings are two halves of one authored source,
    /// and the crate's own allowlist is the third fact that must agree with
    /// them: a type the declaration owns but [`crate::ownership`] does not
    /// would widen the most privileged surface in the Zone.
    ///
    /// The two descriptors themselves are the family crates' contribution and
    /// are bound by the composition root, so this half validates the
    /// declaration against its own ownership list and its own service tables
    /// rather than against descriptors this crate does not own.
    #[test]
    fn the_declaration_owns_exactly_the_types_this_provider_owns() {
        let spec = system_core_declaration();
        let owned: BTreeSet<String> = spec
            .owned_resource_types()
            .into_iter()
            .map(ResourceTypeName::to_canonical_string)
            .collect();
        let allowed: BTreeSet<String> = crate::OWNED_RESOURCE_TYPES
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        assert_eq!(owned, allowed);
        assert!(
            crate::DISOWNED_RESOURCE_TYPES
                .iter()
                .all(|disowned| !owned.contains(*disowned)),
            "no specification-explicitly-disowned type entered the declaration"
        );
    }

    /// One artifact, one derived `Provider` reference: the identity the
    /// committed `Host` and `User` rows already pin, not a second one.
    #[test]
    fn the_declared_provider_reference_is_the_one_the_rows_pin() {
        let spec = system_core_declaration();
        assert_eq!(
            spec.provider().to_canonical_string(),
            crate::PROVIDER_REF,
            "the derived Provider reference must equal the pinned constant"
        );
        assert_eq!(
            spec.artifact_id().as_str(),
            "system-core",
            "the artifact is the family the plane registers and packaging builds"
        );
    }

    /// The published methods are the crate's own: each declared service and
    /// its methods are read from the same [`ServiceDecl`] constant, so the
    /// declaration cannot claim a probe this Provider does not answer.
    #[test]
    fn the_declared_methods_are_the_services_own() {
        let spec = system_core_declaration();
        let methods: Vec<&str> = spec
            .components()
            .iter()
            .flat_map(|component| component.methods())
            .map(|method| method.name().as_str())
            .collect();
        assert_eq!(methods, ["inspect-host", "inspect-user"]);
        let services: Vec<&str> = spec
            .components()
            .iter()
            .flat_map(|component| component.services())
            .map(|service| service.id().as_str())
            .collect();
        assert_eq!(services, [HOST_EFFECTS_SERVICE.id, USER_EFFECTS_SERVICE.id]);
    }

    /// Every component of this Provider is placed at the Host execution
    /// target, and the controllers carry the anchor their fixed-target scope
    /// requires. A component placed somewhere else would reconcile rows on a
    /// substrate this bootstrap Provider does not own.
    #[test]
    fn every_component_is_placed_at_the_host_execution_target() {
        let spec = system_core_declaration();
        assert_eq!(spec.components().len(), 4);
        for component in spec.components() {
            assert_eq!(
                component.placement().targets(),
                &BTreeSet::from([ControllerTargetKind::Host]),
                "{} is placed at the Host execution target",
                component.component().component_id().as_str()
            );
            assert!(
                component
                    .component()
                    .target_capabilities()
                    .iter()
                    .any(|capability| capability.target_kind() == ControllerTargetKind::Host),
                "{} declares the signed Host target artifact",
                component.component().component_id().as_str()
            );
        }
        for controller in [HOST_CONTROLLER, USER_CONTROLLER] {
            let identity = token(controller);
            let component = spec
                .component(&identity)
                .expect("the controller is a declared component");
            assert_eq!(
                component.placement().anchor(),
                Some(PlacementAnchor::ExecutionRef),
                "{controller} anchors at its own rows' execution reference"
            );
        }
    }

    /// The four components are two reconcilers and two services, and the
    /// split is load bearing: a controller that published a method, or a
    /// service that owned a ResourceType, would claim work this Provider does
    /// not do.
    #[test]
    fn the_two_controllers_and_two_services_split_the_two_types() {
        let spec = system_core_declaration();
        for (identity, expected) in [
            (HOST_CONTROLLER, ComponentType::Controller),
            (HOST_EFFECTS, ComponentType::Service),
            (USER_CONTROLLER, ComponentType::Controller),
            (USER_EFFECTS, ComponentType::Service),
        ] {
            let component = spec
                .component(&token(identity))
                .expect("the component is declared");
            assert_eq!(component.component().component_type(), expected);
            if expected == ComponentType::Service {
                assert!(
                    component
                        .component()
                        .exported_resource_types()
                        .is_empty(),
                    "{identity} serves a method and reconciles no type"
                );
            }
        }
    }

    /// The declared half is a value, not a key or a handler: it projects
    /// deterministically and carries the same bytes every time, which is what
    /// a generator consumes.
    #[test]
    fn the_declaration_projects_deterministically() {
        let first = d2b_provider_toolkit::emit_declaration_canonical(&system_core_declaration());
        let second = d2b_provider_toolkit::emit_declaration_canonical(&system_core_declaration());
        assert_eq!(first, second);
    }

    /// The two halves are joined by the composition root, which binds both
    /// family descriptors. This is the shape that join takes, expressed
    /// against a binding table this crate can build, so the declaration's
    /// service and method identities are what a bound descriptor is checked
    /// against rather than a second transcription of them.
    #[test]
    fn the_bindings_half_is_the_two_family_descriptor_table() {
        static DRIVERS: LazyLock<[DriverDescriptor; 0]> = LazyLock::new(|| []);
        let declaration =
            ProviderDeclaration::new(system_core_declaration(), system_core_bindings(&DRIVERS[..]));
        // With no descriptor bound, the declaration owns two types nothing
        // serves: the refusal names them, which is the direction that matters.
        // The composition root binds both, and the refusal disappears.
        let error = declaration
            .validate()
            .expect_err("an unbound type is not a reconciled one");
        assert_eq!(
            error,
            d2b_provider_toolkit::declaration::provider::ProviderDeclarationError::ResourceTypeUndeclared(
                host_resource_type().to_canonical_string()
            )
        );
    }
}
