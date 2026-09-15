//! Declaration vocabulary shared by the v3 resource plane and its per-type
//! provider crates.
//!
//! Resource knowledge lives in the crate that owns the resource type. This
//! crate carries only the resource-agnostic declaration vocabulary those
//! crates use: the driver descriptor, the well-known type names, the allowed
//! registration sources, child creations, broker operation definitions,
//! services, startup steps, and the provider declaration.
//!
//! Only declaration vocabulary lives here. Drivers, spec decoders, driver
//! factories, operation handlers, and the effects they drive live in the
//! per-type crates that declare them; a name in this crate is a vocabulary
//! entry, never an implementation.

#![deny(missing_docs)]

mod allowed_sources;
mod child_creation;
mod descriptor;
mod metadata;
mod operation;
mod provider;
mod resource_type;
mod service;
mod startup;

pub use allowed_sources::AllowedSources;
pub use child_creation::{ChildCreation, ChildCustody};
pub use descriptor::{CONVERTED_TYPE_VERBS, DriverDescriptor};
pub use metadata::{assert_metadata_registration, metadata_descriptor};
pub use operation::{
    OperationCtx, OperationDef, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload,
};
pub use provider::{
    Cardinality, IsolationPosture, PlaneAdapter, PrincipalName, ProviderDeclaration, SelfBinding,
    StorageRoot,
};
pub use resource_type::WellKnownType;
pub use service::{MethodFdContract, ServiceDecl, ServiceMethod};
pub use startup::StartupStep;
