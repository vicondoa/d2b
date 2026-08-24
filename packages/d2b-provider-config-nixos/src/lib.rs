//! Typed, service-only configuration exchange for NixOS Guests.
//!
//! This Provider deliberately owns no ResourceType.  It exposes the one
//! closed guest-config document through an authenticated ComponentSession and
//! leaves host-side staging to the caller's already-authorized local client.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod controller;
mod service;
mod ttrpc;

pub use controller::{
    ConfigCaller, ConfigError, ConfigOperation, ConfigService, GuestConfigDocument,
    GuestSessionEvidence,
};
pub use service::{
    ConfigApproveRequest, ConfigApproveResponse, ConfigDiffRequest, ConfigDiffResponse,
    ConfigRejectRequest, ConfigRejectResponse, ConfigServiceDescriptor, ConfigStageRequest,
    ConfigStageResponse, ConfigStagingStore, ConfigStatusRequest, ConfigStatusResponse,
    ConfigSyncRequest, ConfigSyncResponse, GUEST_CONFIG_IDENTIFIER, MAX_CONFIG_BYTES,
    decode_document,
};
pub use ttrpc::{
    ConfigNixosClient, ConfigServiceBackend, GuestConfigReader, create_ttrpc_services,
};

/// Canonical Provider reference.
pub const PROVIDER_REF: &str = "Provider/config-nixos";
/// Canonical Provider artifact identifier.
pub const ARTIFACT_ID: &str = "config-nixos";
/// Canonical ComponentSession service package.
pub const SERVICE_PACKAGE: &str = "d2b.config-nixos.v3";
/// Canonical service member prefix.
pub const SERVICE_NAME: &str = "ConfigNixosService";
