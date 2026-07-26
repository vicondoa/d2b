//! Asynchronous native resource API, authorization, and transport bindings.

pub mod authz;
pub mod client;
pub mod error;
pub mod generated;
pub mod service;

pub use client::ResourceClient;
pub use service::{ResourceService, TrustedRequest};
