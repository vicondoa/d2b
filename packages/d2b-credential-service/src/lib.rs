//! The Credential service contract and its client and server halves.
//!
//! The service is intentionally not registered with a production bus listener.
//! Its server accepts an injected admission boundary so authorization must run
//! before Provider dispatch without creating a second subject-resolution path.

#![deny(missing_docs)]

pub mod client;
pub mod server;
pub mod service;

pub use client::{CredentialClient, CredentialTransport};
pub use server::{
    CredentialAdmission, CredentialAuthorization, CredentialProvider, CredentialServer,
};
pub use service::*;
