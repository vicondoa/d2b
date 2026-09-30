//! The unified provider declaration: one authored source per provider (KTD1).
//!
//! A provider used to describe itself three times: a signed manifest for its
//! code identity and configuration, a Rust [`DriverDescriptor`] per resource
//! type, and a [`ServiceDecl`] for the methods it answers. Nothing checked
//! that the three agreed, so a method could be declared and unbound, a
//! service could answer an operation no handler served, and a provider could
//! grow a resource type no shared family table knew about.
//!
//! This module is the replacement: one [`ProviderDeclaration`] holding two
//! halves that never restate each other.
//!
//! - [`ProviderDeclarationSpec`] is the serializable semantic surface: the
//!   supported resource types, the methods, the code identity, the
//!   configuration, the placement, the child creation, the required resource
//!   capabilities, the closed presentation capability, and the setup
//!   restrictions that presentation requires. It carries no handler, no
//!   factory, no decoder, and no key, so the canonical projection generators
//!   consume it cannot smuggle a privileged callable through a data file.
//! - [`ProviderImplementationBindings`] is the local constructor and function
//!   surface: the spec decoders, driver factories, operation handlers, and
//!   service declarations that realize those identities.
//!
//! Identity is declared exactly once, in the spec. The bindings reference
//! it, and [`ProviderDeclaration::validate`] refuses the two ways a binding
//! can disagree: a method with no implementation, and an implementation with
//! no declaration.
//!
//! # The row cannot introduce code
//!
//! [`ProviderDeclaration::resolve_implementation`] resolves a method's
//! implementation identity against the deployment's admitted artifacts, never
//! against a row. A mutable `Provider` row names an artifact id; if the
//! verified deployment admitted no such artifact, or the admitted manifest
//! does not declare the method, the resolution refuses. That is AE14: a row
//! cannot mint a compiled privileged handler.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts_provider::v3::{
    AdmittedProviderArtifact, DeclaredChild, ProviderContractError, ProviderDeclarationSpec,
    ProviderSpec,
};
use d2b_contracts_resource::v3::{
    BoundedText, BoundedToken, OperationImplementation, ResourceRef, ResourceTypeName,
};
use d2b_resource_types::{
    ChildCreation, OperationDef, ProviderImplementationBindings, ServiceDecl,
};
use thiserror::Error;

/// The four identities one provider declaration produces (AE1).
///
/// They are projections of a single authored source, not four authored
/// tables: every entry here is derived from the spec and confirmed against
/// the bindings, so a component, resource type, service, and operation that
/// disagree cannot be registered as one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationIdentities {
    /// The declared component identities.
    pub components: BTreeSet<BoundedToken>,
    /// The ResourceTypes this provider owns.
    pub resource_types: BTreeSet<String>,
    /// The declared service identities.
    pub services: BTreeSet<BoundedText>,
    /// The trusted implementation identity of every declared method, in
    /// declaration order.
    pub operations: Vec<OperationImplementation>,
}

/// Why a unified provider declaration is refused.
///
/// Every variant names the facet that refused, so an operator reads which
/// declaration is wrong rather than that two tables happened to differ.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderDeclarationError {
    /// The declaration owns a ResourceType no bound driver descriptor serves.
    #[error("resource type `{0}` is declared but no driver binding serves it")]
    ResourceTypeUndeclared(String),
    /// A bound driver descriptor serves a ResourceType the declaration does
    /// not own.
    #[error("driver binding serves resource type `{0}` that the declaration does not own")]
    ResourceTypeUnlicensed(String),
    /// A declared method has no implementation realizing it.
    #[error("method `{method}` on component `{component}` has no implementation")]
    MethodUnimplemented {
        /// The declared component identity.
        component: String,
        /// The declared method identity.
        method: String,
    },
    /// A bound implementation has no declaration naming it.
    #[error("implementation `{method}` on service `{service}` has no declaration")]
    ImplementationUndeclared {
        /// The bound service identity.
        service: String,
        /// The bound method identity.
        method: String,
    },
    /// A declared service has no bound service declaration.
    #[error("service `{service}` on component `{component}` is declared but not bound")]
    ServiceUnbound {
        /// The declared component identity.
        component: String,
        /// The declared service identity.
        service: String,
    },
    /// A bound service declaration has no declared service.
    #[error("bound service `{service}` is not declared")]
    ServiceUndeclared {
        /// The bound service identity.
        service: String,
    },
    /// A declared child creation has no bound realization.
    #[error("child creation of `{child}` by component `{component}` is declared but not realized")]
    ChildCreationUnrealized {
        /// The declared component identity.
        component: String,
        /// The declared child ResourceType.
        child: String,
    },
    /// A bound child creation has no declared license.
    #[error("child creation of `{child}` by `{provider}` is realized but not declared")]
    ChildCreationUndeclared {
        /// The declared provider that would perform the creation.
        provider: String,
        /// The realized child ResourceType.
        child: String,
    },
    /// A mutable `Provider` row selected an artifact no verified deployment
    /// admitted.
    #[error("provider row selects an artifact no verified deployment admitted")]
    ArtifactSelectionUnverified,
    /// The admitted artifact's signed manifest declares no such method.
    #[error("admitted artifact declares no method `{method}` on component `{component}`")]
    ImplementationNotAdmitted {
        /// The declared component identity.
        component: String,
        /// The declared method identity.
        method: String,
    },
    /// The bound ResourceType's signed capability matrix does not support a
    /// required capability.
    #[error("capability `{capability}` of resource type `{resource_type}` is not supported by the admitted binding")]
    RequiredCapabilityUnsupported {
        /// The bound ResourceType.
        resource_type: String,
        /// The required capability name.
        capability: String,
    },
    /// The identity projection could not be derived from the declaration.
    #[error("provider declaration identity projection failed: {0}")]
    IdentityProjection(ProviderContractError),
}

/// The bound handler and service tables, gathered once per validation.
struct BoundTables<'a> {
    services: Vec<&'a ServiceDecl>,
    operations: Vec<&'a OperationDef>,
    creations: Vec<&'a ChildCreation>,
}

impl BoundTables<'_> {
    /// Whether one bound child creation realizes this declared license.
    fn realizes_child(&self, child: &DeclaredChild) -> bool {
        let provider = child.provider().to_canonical_string();
        self.creations.iter().any(|creation| {
            creation.child.to_resource_type_name().as_str() == child.resource_type().as_str()
                && creation.provider_ref == provider
        })
    }

    /// Whether a bound service answers this method with real code.
    ///
    /// A method the session layer addresses directly is answered inside the
    /// declaring component's own process, and the bound [`ServiceDecl`] is
    /// that answer: the composition root hosts the service through the
    /// provider's declared effects-service factory, so there is no second
    /// table a zone-plane method could be missing from. A method that does
    /// name a committed operation row is dispatched through that row's
    /// handler instead, so the handler table is the authority for it - a
    /// method naming an operation no crate bound is still a method with no
    /// implementation.
    fn answers(&self, name: &str) -> bool {
        self.services.iter().any(|service| {
            // A method the service does not declare has no implementation at
            // all. That is a different fact from a declared method that names
            // no operation row, and conflating the two would let a
            // declaration claim any method name by omission.
            let Some(method) = service.method(name) else {
                return false;
            };
            match method.operation() {
                None => true,
                Some(operation) => self
                    .operations
                    .iter()
                    .any(|handler| handler.serves_named_operation(operation)),
            }
        })
    }
}

/// One provider's authoritative declaration (KTD1).
///
/// A provider exports one of these through its own crate interface. The
/// generators consume [`ProviderDeclaration::spec`] as canonical data, and
/// the composition root builds its runtime descriptors, handler tables, and
/// hosting references from [`ProviderDeclaration::bindings`].
#[derive(Clone)]
pub struct ProviderDeclaration {
    spec: ProviderDeclarationSpec,
    bindings: ProviderImplementationBindings,
}

impl ProviderDeclaration {
    /// Bind one provider's serializable facets to its local constructors and
    /// functions.
    ///
    /// The constructor does not validate the two halves against each other:
    /// a provider assembles its declaration from `const` tables, and the
    /// cross-half check runs once in
    /// [`ProviderDeclaration::validate`] when the declaration is loaded.
    pub const fn new(
        spec: ProviderDeclarationSpec,
        bindings: ProviderImplementationBindings,
    ) -> Self {
        Self { spec, bindings }
    }

    /// The serializable semantic facets generators project.
    pub const fn spec(&self) -> &ProviderDeclarationSpec {
        &self.spec
    }

    /// The local constructor and function bindings composition uses.
    pub const fn bindings(&self) -> &ProviderImplementationBindings {
        &self.bindings
    }

    /// The bound handler and service tables, gathered once per validation.
    fn bound_tables(&self) -> BoundTables<'static> {
        BoundTables {
            services: self.bindings.services().collect(),
            operations: self.bindings.operations().collect(),
            creations: self.bindings.creations().collect(),
        }
    }

    /// Refuse a declaration whose two halves disagree (R9-R11, R30-R33).
    ///
    /// The refusals are the two directions a binding can drift: a declared
    /// method with no implementation to serve it, and an implemented method
    /// no declaration names. The service, child-creation, and ResourceType
    /// identities are checked the same way, so one authored source produces
    /// one consistent registration view.
    ///
    /// Method identity is the method name within the provider's declared
    /// method space, which the spec keeps unique. That is the name the
    /// session layer addresses and the name a service method declares, so
    /// the pairing cannot be ambiguous.
    pub fn validate(&self) -> Result<(), ProviderDeclarationError> {
        for resource_type in self.spec.owned_resource_types() {
            if self.bindings.driver_for(resource_type.as_str()).is_none() {
                return Err(ProviderDeclarationError::ResourceTypeUndeclared(
                    resource_type.to_canonical_string(),
                ));
            }
        }
        for descriptor in self.bindings.drivers() {
            let bound_type = descriptor.resource_type.to_resource_type_name();
            if self
                .spec
                .owned_resource_types()
                .iter()
                .all(|declared| declared.as_str() != bound_type.as_str())
            {
                return Err(ProviderDeclarationError::ResourceTypeUnlicensed(
                    bound_type.as_str().to_owned(),
                ));
            }
        }

        let bound = self.bound_tables();
        for component in self.spec.components() {
            let identity = component.component().component_id().as_str().to_owned();
            for declared in component.services() {
                if !bound
                    .services
                    .iter()
                    .any(|service| service.id == declared.id().as_str())
                {
                    return Err(ProviderDeclarationError::ServiceUnbound {
                        component: identity.clone(),
                        service: declared.id().as_str().to_owned(),
                    });
                }
            }
            for method in component.methods() {
                let name = method.name().as_str();
                let implemented = match method.template() {
                    // A trusted executable template is the component's own
                    // launchable binary; there is no second template table
                    // for it to drift from.
                    Some(template) => component
                        .component()
                        .execution()
                        .binary_ref()
                        .is_some_and(|binary| binary.as_str() == template.as_str()),
                    // Any other method is served by the bound service that
                    // answers its name.
                    None => bound.answers(name),
                };
                if !implemented {
                    return Err(ProviderDeclarationError::MethodUnimplemented {
                        component: identity.clone(),
                        method: name.to_owned(),
                    });
                }
            }
        }

        for service in &bound.services {
            if !self.declares_service(service.id) {
                return Err(ProviderDeclarationError::ServiceUndeclared {
                    service: service.id.to_owned(),
                });
            }
            for method in service.methods {
                if !self.declares_method(method.name) {
                    return Err(ProviderDeclarationError::ImplementationUndeclared {
                        service: service.id.to_owned(),
                        method: method.name.to_owned(),
                    });
                }
            }
        }

        for component in self.spec.components() {
            let identity = component.component().component_id().as_str().to_owned();
            for child in component.children() {
                if !bound.realizes_child(child) {
                    return Err(ProviderDeclarationError::ChildCreationUnrealized {
                        component: identity.clone(),
                        child: child.resource_type().to_canonical_string(),
                    });
                }
            }
        }
        for creation in &bound.creations {
            if !self.declares_child(creation) {
                return Err(ProviderDeclarationError::ChildCreationUndeclared {
                    provider: creation.provider_ref.to_owned(),
                    child: creation.child.to_resource_type_name().as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Whether the provider declares the service identity.
    fn declares_service(&self, id: &str) -> bool {
        self.spec
            .components()
            .iter()
            .flat_map(|component| component.services())
            .any(|service| service.id().as_str() == id)
    }

    /// Whether the provider declares the method identity.
    fn declares_method(&self, name: &str) -> bool {
        self.spec
            .components()
            .iter()
            .flat_map(|component| component.methods())
            .any(|method| method.name().as_str() == name)
    }

    /// Whether the provider licenses this bound child creation.
    fn declares_child(&self, creation: &'static ChildCreation) -> bool {
        let child_type = creation.child.to_resource_type_name();
        self.spec
            .components()
            .iter()
            .flat_map(|component| component.children())
            .any(|child| {
                child.resource_type().as_str() == child_type.as_str()
                    && child.provider().to_canonical_string() == creation.provider_ref
            })
    }

    /// The four identities this declaration produces (AE1).
    ///
    /// # Errors
    ///
    /// Refuses when a declared method does not resolve to a trusted
    /// implementation identity.
    pub fn identities(&self) -> Result<DeclarationIdentities, ProviderDeclarationError> {
        let components = self
            .spec
            .components()
            .iter()
            .map(|component| component.component().component_id().clone())
            .collect();
        let resource_types = self
            .spec
            .owned_resource_types()
            .into_iter()
            .map(ResourceTypeName::to_canonical_string)
            .collect();
        let services = self
            .spec
            .components()
            .iter()
            .flat_map(|component| component.services())
            .map(|service| service.id().clone())
            .collect();
        Ok(DeclarationIdentities {
            components,
            resource_types,
            services,
            operations: self
                .spec
                .implementations()
                .map_err(ProviderDeclarationError::IdentityProjection)?,
        })
    }

    /// Resolve the trusted implementation identity of one declared method
    /// (R12, AE14).
    ///
    /// `row` is the mutable `Provider` resource row. It selects an artifact
    /// and nothing else: a row naming an artifact the verified deployment
    /// did not admit, or naming an admitted artifact whose signed manifest
    /// declares no such method, resolves to nothing. The returned identity
    /// therefore always names the declaring provider's own declared method,
    /// never code a row introduced.
    ///
    /// # Errors
    ///
    /// - [`ProviderDeclarationError::ArtifactSelectionUnverified`] when the
    ///   row selects an artifact no admitted artifact provides.
    /// - [`ProviderDeclarationError::ImplementationNotAdmitted`] when the
    ///   admitted manifest declares no such component method.
    /// - [`ProviderDeclarationError::IdentityProjection`] when the
    ///   declaration itself does not carry the method.
    pub fn resolve_implementation(
        &self,
        admitted: &[AdmittedProviderArtifact],
        row: &ProviderSpec,
        component: &BoundedToken,
        method: &BoundedToken,
    ) -> Result<OperationImplementation, ProviderDeclarationError> {
        let artifact = admitted
            .iter()
            .find(|artifact| artifact.artifact_id() == row.artifact_id())
            .ok_or(ProviderDeclarationError::ArtifactSelectionUnverified)?;
        if !artifact.declares_method(component, method) {
            return Err(ProviderDeclarationError::ImplementationNotAdmitted {
                component: component.as_str().to_owned(),
                method: method.as_str().to_owned(),
            });
        }
        self.spec
            .implementation(component, method)
            .map_err(ProviderDeclarationError::IdentityProjection)
    }

    /// Check the declared required resource capabilities against one admitted
    /// artifact's signed capability matrices (R9, R11).
    ///
    /// Absence is not support: a capability the bound ResourceType's signed
    /// matrix does not list is refused here rather than discovered when an
    /// effect silently does nothing.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProviderDeclarationError::RequiredCapabilityUnsupported`]
    /// naming the first unsupported facet in canonical order.
    pub fn admit_required_capabilities(
        &self,
        artifact: &AdmittedProviderArtifact,
    ) -> Result<(), ProviderDeclarationError> {
        let mut unsupported: BTreeMap<&str, &str> = BTreeMap::new();
        for requirement in self.spec.required_capabilities() {
            let supported = artifact
                .manifest()
                .binding_for(requirement.resource_type())
                .is_some_and(|binding| binding.capability_matrix().supports(requirement.capability()));
            if !supported {
                unsupported.insert(
                    requirement.resource_type().as_str(),
                    requirement.capability().as_str(),
                );
            }
        }
        match unsupported
            .into_iter()
            .next()
            .map(|(resource_type, capability)| (resource_type.to_owned(), capability.to_owned()))
        {
            Some((resource_type, capability)) => {
                Err(ProviderDeclarationError::RequiredCapabilityUnsupported {
                    resource_type,
                    capability,
                })
            }
            None => Ok(()),
        }
    }

    /// The `Provider` reference every implementation identity names.
    pub const fn provider(&self) -> &ResourceRef {
        self.spec.provider()
    }
}

impl core::fmt::Debug for ProviderDeclaration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderDeclaration")
            .field("component_count", &self.spec.components().len())
            .field("required_capability_count", &self.spec.required_capabilities().len())
            .field("bindings", &self.bindings)
            .finish_non_exhaustive()
    }
}
