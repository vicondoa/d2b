//! The declaration vocabulary a Provider publishes, and its canonical
//! emitters.
//!
//! The types themselves live in `d2b-resource-types`: exactly one crate
//! declares them, so a driver's descriptor and a provider's declaration have
//! one shape everywhere. This module is the toolkit's face on that
//! vocabulary, plus the emitters a provider artifact needs - the signed
//! manifest, the unified declaration projection, and the root configuration
//! schema - which are toolkit-owned because they are the same bytes for
//! every provider.
//!
//! [`provider::ProviderDeclaration`] is the one authored source a provider
//! exports: its serializable semantic facets and the local constructors and
//! functions that realize them, held separately and cross-checked.
pub use d2b_resource_types::{
    AllowedSources, Cardinality, ChildCreation, ChildCustody, DriverDescriptor,
    IsolationPosture, MethodFdContract, OperationDef, OperationHandler, PlaneAdapter,
    PrincipalName, ProviderDeclaration, ProviderImplementationBindings, SelfBinding, ServiceDecl,
    ServiceMethod, StartupStep, StorageRoot, WellKnownType,
};

pub mod manifest;
pub mod provider;
pub mod schema;

pub use manifest::{
    CanonicalMismatch, VerificationError, emit_canonical, emit_declaration_canonical,
    validate_for_installation, verify_canonical,
};
// The unified provider declaration is reached through `declaration::provider`
// so the zone-level `declaration::ProviderDeclaration` keeps its meaning and
// its existing importers.
pub use provider::{DeclarationIdentities, ProviderDeclarationError};
pub use schema::RootConfigSchema;
