//! Asynchronous native resource API, authorization, and transport bindings.

pub mod adapter;
mod admission;
pub mod authz;
pub mod client;
pub mod error;
pub mod generated;
mod identity;
pub mod service;
mod store;

pub use adapter::{AdapterBindingError, AuthenticatedBusAdapter};
pub use admission::{
    AdmissionError, AdmissionVerifier, AdmittedMutation, PreparedStoreMutation, VerifiedMutation,
};
pub use client::ResourceClient;
pub use identity::AuthenticatedSubjectContext;
pub use service::ResourceService;
pub use store::{ResourceStore, ResourceStoreBackend};
