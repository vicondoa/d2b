//! Authorization and bounded document policy for the config-nixos service.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_contracts_resource::v3::ResourceRef;
use sha2::{Digest, Sha256};

use crate::{
    GUEST_CONFIG_IDENTIFIER, MAX_CONFIG_BYTES, SERVICE_NAME, SERVICE_PACKAGE,
    service::{
        ConfigApproveRequest, ConfigDiffRequest, ConfigRejectRequest, ConfigServiceDescriptor,
        ConfigStageRequest, ConfigStatusRequest, ConfigSyncRequest, ConfigSyncResponse,
        validate_destination, validate_guest_ref, validate_identifier, validate_view_identifier,
    },
};

/// Caller role carried by the authenticated ComponentSession or local
/// operator session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigCaller {
    /// The owning Guest session, allowed to read its own document only.
    Guest,
    /// An administrator, allowed to operate on host staging state.
    Admin,
    /// A lifecycle-authorized operator, allowed to operate on host staging.
    Lifecycle,
    /// An ordinary user, which has no config-management authority.
    User,
}

impl ConfigCaller {
    fn can_read(self) -> bool {
        matches!(self, Self::Guest | Self::Admin | Self::Lifecycle)
    }

    fn can_stage(self) -> bool {
        matches!(self, Self::Admin | Self::Lifecycle)
    }
}

/// Redacted evidence proving that a Guest read belongs to the current
/// authenticated ComponentSession.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestSessionEvidence {
    guest_ref: ResourceRef,
    boot_identity: String,
    reconnect_generation: u64,
    authenticated: bool,
}

impl GuestSessionEvidence {
    /// Construct evidence after binding the Guest and reconnect generation.
    pub fn new(
        guest_ref: ResourceRef,
        boot_identity: impl Into<String>,
        reconnect_generation: u64,
    ) -> Result<Self, ConfigError> {
        if guest_ref.resource_type().as_str() != "Guest"
            || guest_ref.name().as_str().is_empty()
            || reconnect_generation == 0
        {
            return Err(ConfigError::SessionMismatch);
        }
        let boot_identity = boot_identity.into();
        if boot_identity.is_empty() || boot_identity.len() > 128 {
            return Err(ConfigError::SessionMismatch);
        }
        Ok(Self {
            guest_ref,
            boot_identity,
            reconnect_generation,
            authenticated: true,
        })
    }

    /// Borrow the authenticated Guest reference.
    pub fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the bounded boot identity commitment.
    pub fn boot_identity(&self) -> &str {
        &self.boot_identity
    }

    /// Return the reconnect generation bound by admission.
    pub const fn reconnect_generation(&self) -> u64 {
        self.reconnect_generation
    }

    /// Return whether the evidence is still current.
    pub const fn authenticated(&self) -> bool {
        self.authenticated
    }

    /// Mark the session stale without retaining a usable read capability.
    pub fn stale(mut self) -> Self {
        self.authenticated = false;
        self
    }
}

impl fmt::Debug for GuestSessionEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestSessionEvidence")
            .field("guest_ref", &self.guest_ref)
            .field("boot_identity", &"<redacted>")
            .field("reconnect_generation", &self.reconnect_generation)
            .field("authenticated", &self.authenticated)
            .finish()
    }
}

/// A validated Guest configuration document.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestConfigDocument {
    bytes: Vec<u8>,
}

impl GuestConfigDocument {
    /// Validate and retain one bounded non-empty UTF-8 Nix document.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, ConfigError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(ConfigError::EmptyDocument);
        }
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::DocumentTooLarge);
        }
        if std::str::from_utf8(&bytes).is_err() {
            return Err(ConfigError::InvalidUtf8);
        }
        Ok(Self { bytes })
    }

    /// Borrow the validated document bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the document size.
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Return whether the document is empty.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return a content digest computed from the received bytes.
    pub fn sha256(&self) -> String {
        let digest = Sha256::digest(&self.bytes);
        format!("sha256:{digest:x}")
    }

    /// Return the bounded base64 representation used by the service DTO.
    pub fn content_base64(&self) -> String {
        STANDARD.encode(&self.bytes)
    }
}

impl fmt::Debug for GuestConfigDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestConfigDocument")
            .field("bytes", &self.bytes.len())
            .field("sha256", &self.sha256())
            .finish()
    }
}

/// Closed config service operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigOperation {
    /// Read the Guest's canonical config document.
    ReadGuestConfig,
    /// Stage a received document in the host-side staging client.
    Stage,
    /// Compare a staged document with the caller-selected local view.
    Diff,
    /// Approve a staged document for a caller-authorized destination.
    Approve,
    /// Reject and remove a staged document.
    Reject,
    /// Report pending staging state.
    Status,
}

impl ConfigOperation {
    /// Return the exact generated service member.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadGuestConfig => "ConfigNixosService/ReadGuestConfig",
            Self::Stage => "ConfigNixosService/Stage",
            Self::Diff => "ConfigNixosService/Diff",
            Self::Approve => "ConfigNixosService/Approve",
            Self::Reject => "ConfigNixosService/Reject",
            Self::Status => "ConfigNixosService/Status",
        }
    }

    /// Return every operation in stable order.
    pub const ALL: [Self; 6] = [
        Self::ReadGuestConfig,
        Self::Stage,
        Self::Diff,
        Self::Approve,
        Self::Reject,
        Self::Status,
    ];
}

/// Stable errors emitted by config-nixos without guest content or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// The request named something other than a Guest or used a wrong
    /// closed identifier.
    InvalidRequest,
    /// The caller is not allowed to perform this operation.
    Unauthorized,
    /// The authenticated Guest session is absent, stale, or mismatched.
    SessionMismatch,
    /// The document is empty.
    EmptyDocument,
    /// The document exceeded the fixed raw-byte bound.
    DocumentTooLarge,
    /// The document is not UTF-8.
    InvalidUtf8,
    /// The service response could not be encoded under its fixed bound.
    EncodingFailed,
    /// The Guest's host-declared configuration working copy is unavailable.
    Unavailable,
    /// A staging record was requested but does not exist.
    StagingMissing,
    /// An already approved record was retried for a different destination.
    ApprovalConflict,
    /// The local comparison view identifier is not a content commitment.
    InvalidView,
    /// The host destination identifier is not a bounded opaque value.
    InvalidDestination,
}

impl ConfigError {
    /// Return a stable machine-readable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "config-request-invalid",
            Self::Unauthorized => "config-unauthorized",
            Self::SessionMismatch => "config-session-stale",
            Self::EmptyDocument => "config-document-empty",
            Self::DocumentTooLarge => "config-document-too-large",
            Self::InvalidUtf8 => "config-document-invalid-utf8",
            Self::EncodingFailed => "config-document-encoding-failed",
            Self::Unavailable => "config-document-unavailable",
            Self::StagingMissing => "config-stage-missing",
            Self::ApprovalConflict => "config-approval-conflict",
            Self::InvalidView => "config-view-invalid",
            Self::InvalidDestination => "config-destination-invalid",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ConfigError {}

/// Service-only config Provider policy.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConfigService;

impl ConfigService {
    /// Return the closed service descriptor.
    pub fn descriptor() -> ConfigServiceDescriptor {
        ConfigServiceDescriptor::canonical()
    }

    /// Read one Guest document through current authenticated session evidence.
    pub fn read_guest_config(
        &self,
        caller: ConfigCaller,
        request: &ConfigSyncRequest,
        evidence: &GuestSessionEvidence,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<ConfigSyncResponse, ConfigError> {
        if !caller.can_read()
            || caller != ConfigCaller::Guest
            || request.guest_ref != *evidence.guest_ref()
            || request.identifier != GUEST_CONFIG_IDENTIFIER
            || !evidence.authenticated()
        {
            return Err(if !caller.can_read() {
                ConfigError::Unauthorized
            } else {
                ConfigError::SessionMismatch
            });
        }
        let document = GuestConfigDocument::new(bytes)?;
        Ok(ConfigSyncResponse::from_document(
            request.guest_ref.clone(),
            document,
        ))
    }

    /// Validate a host-side stage request without accepting a path or
    /// arbitrary document identifier.
    pub fn stage(
        &self,
        caller: ConfigCaller,
        request: &ConfigSyncRequest,
        document: GuestConfigDocument,
    ) -> Result<ConfigSyncResponse, ConfigError> {
        if !caller.can_stage() {
            return Err(ConfigError::Unauthorized);
        }
        validate_guest_ref(&request.guest_ref)?;
        validate_identifier(&request.identifier)?;
        Ok(ConfigSyncResponse::from_document(
            request.guest_ref.clone(),
            document,
        ))
    }

    /// Authorize an admin-only staging mutation.
    pub fn authorize_admin(
        &self,
        caller: ConfigCaller,
        guest_ref: &ResourceRef,
    ) -> Result<(), ConfigError> {
        if !caller.can_stage() || guest_ref.resource_type().as_str() != "Guest" {
            return Err(if caller.can_stage() {
                ConfigError::InvalidRequest
            } else {
                ConfigError::Unauthorized
            });
        }
        Ok(())
    }

    /// Validate a typed operation payload against the closed service.
    pub fn validate_operation(
        &self,
        operation: ConfigOperation,
        payload: &serde_json::Value,
    ) -> Result<(), ConfigError> {
        match operation {
            ConfigOperation::ReadGuestConfig => {
                let request = serde_json::from_value::<ConfigSyncRequest>(payload.clone())
                    .map_err(|_| ConfigError::InvalidRequest)?;
                validate_guest_ref(&request.guest_ref)?;
                validate_identifier(&request.identifier)
            }
            ConfigOperation::Stage => {
                let request = serde_json::from_value::<ConfigStageRequest>(payload.clone())
                    .map_err(|_| ConfigError::InvalidRequest)?;
                validate_guest_ref(&request.guest_ref)?;
                validate_identifier(&request.identifier)?;
                request.document().map(|_| ())
            }
            ConfigOperation::Diff => {
                let request = serde_json::from_value::<ConfigDiffRequest>(payload.clone())
                    .map_err(|_| ConfigError::InvalidRequest)?;
                validate_guest_ref(&request.guest_ref)?;
                validate_identifier(&request.identifier)?;
                validate_view_identifier(&request.against)
            }
            ConfigOperation::Approve => {
                let request = serde_json::from_value::<ConfigApproveRequest>(payload.clone())
                    .map_err(|_| ConfigError::InvalidRequest)?;
                validate_guest_ref(&request.guest_ref)?;
                validate_identifier(&request.identifier)?;
                validate_destination(&request.destination)
            }
            ConfigOperation::Reject => {
                let request = serde_json::from_value::<ConfigRejectRequest>(payload.clone())
                    .map_err(|_| ConfigError::InvalidRequest)?;
                validate_guest_ref(&request.guest_ref)?;
                validate_identifier(&request.identifier)
            }
            ConfigOperation::Status => {
                let request = serde_json::from_value::<ConfigStatusRequest>(payload.clone())
                    .map_err(|_| ConfigError::InvalidRequest)?;
                validate_guest_ref(&request.guest_ref)?;
                validate_identifier(&request.identifier)
            }
        }
    }

    /// Ensure service identity remains tied to the canonical descriptor.
    pub fn service_identity(&self) -> (&'static str, &'static str) {
        (SERVICE_PACKAGE, SERVICE_NAME)
    }
}
