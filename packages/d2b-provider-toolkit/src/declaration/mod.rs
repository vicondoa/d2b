//! The declaration vocabulary a Provider publishes, and its canonical
//! emitters.
//!
//! The types themselves live in `d2b-resource-types`: exactly one crate
//! declares them, so a driver's descriptor and a provider's declaration have
//! one shape everywhere. This module is the toolkit's face on that
//! vocabulary, plus the two emitters a provider artifact needs - the signed
//! manifest and the root configuration schema - which are toolkit-owned
//! because they are the same bytes for every provider.

pub use d2b_resource_types::{
    AllowedSources, Cardinality, ChildCreation, ChildCustody, DriverDescriptor, IsolationPosture,
    MethodFdContract, OperationDef, OperationHandler, PlaneAdapter, PrincipalName,
    ProviderDeclaration, SelfBinding, ServiceDecl, ServiceMethod, StartupStep, StorageRoot,
    WellKnownType,
};

pub mod manifest;
pub mod schema;

pub use manifest::{
    CanonicalMismatch, VerificationError, emit_canonical, validate_for_installation,
    verify_canonical,
};
pub use schema::RootConfigSchema;
