//! Asynchronous native resource API and authorization contracts.
//!
//! Transport dispatch is explicitly authenticated through the d2b-bus
//! ComponentSession boundary.

pub mod adapter;
mod admission;
pub mod authz;
mod client;
pub mod error;
pub mod generated;
mod identity;
pub mod service;
pub mod manager_backend;
mod store;
pub mod watch;

pub use adapter::{
    AdapterBindingError, ResourceBusAdapter, ScopedCommitFrameError, ScopedQueryFrameError,
    ScopedQueryMethod, attach_scoped_commit_frame, attach_scoped_query_frame,
    decode_scoped_commit_request, reject_scoped_commit_frame,
};
pub use admission::{AdmissionError, AdmittedMutation};
pub use protobuf;
pub use authz::{AuthorizationLease, StoreSealHandoffError};
pub use client::ResourceApiClient;
pub use d2b_contracts_resource::v3::PreparedStoreMutation;
pub use identity::AuthenticatedSubjectContext;
pub use store::{ResourceStoreBackend, StoreBindingError};
pub use service::{GuestLifecycleAdmission, ResourceService};
pub use watch::{WatchFrame, WatchSink, WatchSinkError};

