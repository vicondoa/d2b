//! Asynchronous native resource API and authorization contracts.
//!
//! Transport dispatch is explicitly unregistered until an authenticated
//! ComponentSession router is present in the workspace.

pub mod adapter;
mod admission;
pub mod authz;
pub mod client;
pub mod error;
pub mod generated;
mod identity;
pub mod service;
mod store;

pub use adapter::{
    AdapterBindingError, RESOURCE_API_REACHABILITY, ResourceApiReachability, UnregisteredBusAdapter,
};
pub use admission::{
    AdmissionError, AdmissionVerifier, AdmittedMutation, PreparedStoreMutation, StoreIdentity,
    VerifiedMutation,
};
pub use client::UnregisteredResourceClient;
pub use identity::AuthenticatedSubjectContext;
pub use service::ResourceService;
pub use store::{ResourceStore, ResourceStoreBackend};
