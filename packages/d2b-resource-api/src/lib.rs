//! Asynchronous native resource API and authorization contracts.
//!
//! Transport dispatch is explicitly unregistered until an authenticated
//! ComponentSession router is present in the workspace.

pub mod adapter;
mod admission;
pub mod authz;
pub mod client;
pub mod emergency_gate;
pub mod error;
pub mod generated;
mod identity;
pub mod quota_gate;
pub mod service;
mod store;
pub mod zone_service;

pub use adapter::{
    AdapterBindingError, RESOURCE_API_REACHABILITY, ResourceApiReachability, UnregisteredBusAdapter,
};
pub use admission::{AdmissionError, AdmittedMutation};
pub use authz::StoreSealHandoffError;
pub use client::UnregisteredResourceClient;
pub use d2b_resource_store::PreparedStoreMutation;
pub use identity::AuthenticatedSubjectContext;
pub use service::ResourceService;
pub use store::{RedbBackend, ResourceStoreBackend, StoreBindingError};
pub use zone_service::{
    StrictWireMessage, ZoneCallContext, ZoneMethod, ZoneService, ZoneServiceError,
    ZoneServiceHandler,
};
