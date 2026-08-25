//! Closed config-nixos service DTOs.

use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ConfigCaller, ConfigError, ConfigOperation, GuestConfigDocument, SERVICE_PACKAGE};

/// The only Guest configuration identifier accepted by this service.
pub const GUEST_CONFIG_IDENTIFIER: &str = "guest-config";
/// Maximum raw document size.
pub const MAX_CONFIG_BYTES: usize = 512 * 1024;
/// Maximum base64-encoded document size.
pub const MAX_CONFIG_ENCODED_BYTES: usize = MAX_CONFIG_BYTES.div_ceil(3) * 4;

/// Typed request for reading or staging the canonical Guest document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigSyncRequest {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed document identifier.
    pub identifier: String,
}

impl ConfigSyncRequest {
    /// Construct and validate a closed request.
    pub fn new(guest_ref: ResourceRef) -> Result<Self, ConfigError> {
        if guest_ref.resource_type().as_str() != "Guest" {
            return Err(ConfigError::InvalidRequest);
        }
        Ok(Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
        })
    }
}

/// Typed response containing only the bounded canonical Guest document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigSyncResponse {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed document identifier.
    pub identifier: String,
    /// Base64-encoded UTF-8 document.
    pub content_base64: String,
    /// Byte count computed by the Provider.
    pub bytes: usize,
    /// SHA-256 computed by the Provider.
    pub sha256: String,
}

impl ConfigSyncResponse {
    pub(crate) fn from_document(guest_ref: ResourceRef, document: GuestConfigDocument) -> Self {
        Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
            content_base64: document.content_base64(),
            bytes: document.len(),
            sha256: document.sha256(),
        }
    }

    /// Decode the response and reapply all document bounds.
    pub fn document(&self) -> Result<GuestConfigDocument, ConfigError> {
        if self.identifier != GUEST_CONFIG_IDENTIFIER
            || self.content_base64.len()
                > MAX_CONFIG_ENCODED_BYTES
        {
            return Err(ConfigError::InvalidRequest);
        }
        let bytes = STANDARD
            .decode(&self.content_base64)
            .map_err(|_| ConfigError::EncodingFailed)?;
        let document = GuestConfigDocument::new(bytes)?;
        if document.len() != self.bytes || document.sha256() != self.sha256 {
            return Err(ConfigError::EncodingFailed);
        }
        Ok(document)
    }
}

/// Typed host staging request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigStageRequest {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed document identifier.
    pub identifier: String,
    /// Base64-encoded document to validate and stage.
    pub content_base64: String,
}

impl ConfigStageRequest {
    /// Construct a stage request from one already validated document.
    pub fn new(
        guest_ref: ResourceRef,
        document: &GuestConfigDocument,
    ) -> Result<Self, ConfigError> {
        validate_guest_ref(&guest_ref)?;
        Ok(Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
            content_base64: document.content_base64(),
        })
    }

    /// Decode and validate the staged document.
    pub fn document(&self) -> Result<GuestConfigDocument, ConfigError> {
        validate_identifier(&self.identifier)?;
        if self.content_base64.len()
            > MAX_CONFIG_ENCODED_BYTES
        {
            return Err(ConfigError::DocumentTooLarge);
        }
        let bytes = STANDARD
            .decode(&self.content_base64)
            .map_err(|_| ConfigError::EncodingFailed)?;
        GuestConfigDocument::new(bytes)
    }
}

/// Typed staging response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigStageResponse {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Number of bytes staged.
    pub bytes: usize,
    /// Digest of staged bytes.
    pub sha256: String,
}

/// Typed request for a local diff operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigDiffRequest {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed staging document identifier.
    pub identifier: String,
    /// Stable local view identifier, not a file path.
    pub against: String,
}

impl ConfigDiffRequest {
    /// Construct a diff request from a stable content-view digest.
    pub fn new(guest_ref: ResourceRef, against: impl Into<String>) -> Result<Self, ConfigError> {
        validate_guest_ref(&guest_ref)?;
        let against = against.into();
        validate_view_identifier(&against)?;
        Ok(Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
            against,
        })
    }
}

/// Typed diff result. Diff text is deliberately not carried by the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigDiffResponse {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Whether the local views differ.
    pub differs: bool,
}

/// Typed request for approval of staged content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigApproveRequest {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed staging document identifier.
    pub identifier: String,
    /// Stable host configuration target identifier.
    pub destination: String,
}

impl ConfigApproveRequest {
    /// Construct an approval request for one opaque host target.
    pub fn new(
        guest_ref: ResourceRef,
        destination: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        validate_guest_ref(&guest_ref)?;
        let destination = destination.into();
        validate_destination(&destination)?;
        Ok(Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
            destination,
        })
    }
}

/// Typed approval result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigApproveResponse {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Number of bytes approved.
    pub bytes: usize,
    /// Digest of the exact approved document.
    pub sha256: String,
}

/// Typed request for rejecting staged content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigRejectRequest {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed staging document identifier.
    pub identifier: String,
}

impl ConfigRejectRequest {
    /// Construct a rejection request.
    pub fn new(guest_ref: ResourceRef) -> Result<Self, ConfigError> {
        validate_guest_ref(&guest_ref)?;
        Ok(Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
        })
    }
}

/// Typed rejection result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigRejectResponse {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Whether content was removed.
    pub removed: bool,
}

/// Typed request for staging status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigStatusRequest {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Closed staging document identifier.
    pub identifier: String,
}

impl ConfigStatusRequest {
    /// Construct a status request.
    pub fn new(guest_ref: ResourceRef) -> Result<Self, ConfigError> {
        validate_guest_ref(&guest_ref)?;
        Ok(Self {
            guest_ref,
            identifier: GUEST_CONFIG_IDENTIFIER.to_owned(),
        })
    }
}

/// Typed status result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigStatusResponse {
    /// Owning Guest resource.
    pub guest_ref: ResourceRef,
    /// Whether unapproved content is staged.
    pub pending: bool,
    /// Size of staged content when present.
    pub bytes: Option<usize>,
    /// Digest of staged content when present.
    pub sha256: Option<String>,
}

/// Closed descriptor for the service-only Provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigServiceDescriptor {
    /// Service package.
    pub package: String,
    /// Exact generated method names.
    pub methods: Vec<String>,
    /// Whether this Provider owns no ResourceType.
    pub service_only: bool,
}

impl ConfigServiceDescriptor {
    /// Return the canonical descriptor.
    pub fn canonical() -> Self {
        Self {
            package: SERVICE_PACKAGE.to_owned(),
            methods: ConfigOperation::ALL
                .into_iter()
                .map(|operation| operation.as_str().to_owned())
                .collect(),
            service_only: true,
        }
    }

    /// Validate an incoming descriptor.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let expected = Self::canonical();
        if self == &expected {
            Ok(())
        } else {
            Err(ConfigError::InvalidRequest)
        }
    }
}

/// Convert one response into a validated document.
pub fn decode_document(response: &ConfigSyncResponse) -> Result<GuestConfigDocument, ConfigError> {
    response.document()
}

pub(crate) fn validate_guest_ref(guest_ref: &ResourceRef) -> Result<(), ConfigError> {
    if guest_ref.resource_type().as_str() == "Guest" && !guest_ref.name().as_str().is_empty() {
        Ok(())
    } else {
        Err(ConfigError::InvalidRequest)
    }
}

pub(crate) fn validate_identifier(identifier: &str) -> Result<(), ConfigError> {
    if identifier == GUEST_CONFIG_IDENTIFIER {
        Ok(())
    } else {
        Err(ConfigError::InvalidRequest)
    }
}

pub(crate) fn validate_view_identifier(value: &str) -> Result<(), ConfigError> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(ConfigError::InvalidView);
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ConfigError::InvalidView);
    }
    Ok(())
}

pub(crate) fn validate_destination(value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 128
        || !value.is_ascii()
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_whitespace)
    {
        return Err(ConfigError::InvalidDestination);
    }
    Ok(())
}

/// In-memory host staging owner for one daemon authority.
///
/// The store contains only bounded, validated documents indexed by canonical
/// Guest reference. It never accepts paths, guest-provided identifiers, or
/// unvalidated bytes.
#[derive(Debug, Default)]
pub struct ConfigStagingStore {
    pending: BTreeMap<String, GuestConfigDocument>,
    approved: BTreeMap<String, ApprovedConfig>,
}

#[derive(Debug, Clone)]
struct ApprovedConfig {
    destination: String,
    bytes: usize,
    sha256: String,
}

impl ConfigStagingStore {
    /// Stage or replace one Guest document.
    pub fn stage(
        &mut self,
        caller: ConfigCaller,
        zone: &ZoneId,
        request: &ConfigStageRequest,
    ) -> Result<ConfigStageResponse, ConfigError> {
        authorize_host_operation(caller, &request.guest_ref, &request.identifier)?;
        let document = request.document()?;
        let guest_key = guest_key(zone, &request.guest_ref);
        let response = ConfigStageResponse {
            guest_ref: request.guest_ref.clone(),
            bytes: document.len(),
            sha256: document.sha256(),
        };
        self.approved.remove(&guest_key);
        self.pending.insert(guest_key, document);
        Ok(response)
    }

    /// Compare staged content to a stable local-view digest.
    pub fn diff(
        &self,
        caller: ConfigCaller,
        zone: &ZoneId,
        request: &ConfigDiffRequest,
    ) -> Result<ConfigDiffResponse, ConfigError> {
        authorize_host_operation(caller, &request.guest_ref, &request.identifier)?;
        validate_view_identifier(&request.against)?;
        let staged = self
            .pending
            .get(&guest_key(zone, &request.guest_ref))
            .ok_or(ConfigError::StagingMissing)?;
        Ok(ConfigDiffResponse {
            guest_ref: request.guest_ref.clone(),
            differs: staged.sha256() != request.against,
        })
    }

    /// Approve staged content for one opaque host target.
    ///
    /// Approval is idempotent so a caller can retry after the downstream
    /// host publish fails. The staged bytes are consumed into an internal
    /// approval receipt, and a matching retry returns the same response.
    pub fn approve(
        &mut self,
        caller: ConfigCaller,
        zone: &ZoneId,
        request: &ConfigApproveRequest,
    ) -> Result<ConfigApproveResponse, ConfigError> {
        authorize_host_operation(caller, &request.guest_ref, &request.identifier)?;
        validate_destination(&request.destination)?;
        let guest_key = guest_key(zone, &request.guest_ref);
        if let Some(staged) = self.pending.remove(&guest_key) {
            let bytes = staged.len();
            let sha256 = staged.sha256();
            self.approved.insert(
                guest_key,
                ApprovedConfig {
                    destination: request.destination.clone(),
                    bytes,
                    sha256: sha256.clone(),
                },
            );
            return Ok(ConfigApproveResponse {
                guest_ref: request.guest_ref.clone(),
                bytes,
                sha256,
            });
        }
        let approved = self
            .approved
            .get(&guest_key)
            .ok_or(ConfigError::StagingMissing)?;
        if approved.destination != request.destination {
            return Err(ConfigError::ApprovalConflict);
        }
        Ok(ConfigApproveResponse {
            guest_ref: request.guest_ref.clone(),
            bytes: approved.bytes,
            sha256: approved.sha256.clone(),
        })
    }

    /// Reject staged content, returning whether anything was removed.
    pub fn reject(
        &mut self,
        caller: ConfigCaller,
        zone: &ZoneId,
        request: &ConfigRejectRequest,
    ) -> Result<ConfigRejectResponse, ConfigError> {
        authorize_host_operation(caller, &request.guest_ref, &request.identifier)?;
        let guest_key = guest_key(zone, &request.guest_ref);
        let removed_pending = self.pending.remove(&guest_key).is_some();
        let removed_approved = self.approved.remove(&guest_key).is_some();
        Ok(ConfigRejectResponse {
            guest_ref: request.guest_ref.clone(),
            removed: removed_pending || removed_approved,
        })
    }

    /// Return bounded staging metadata without returning document bytes.
    pub fn status(
        &self,
        caller: ConfigCaller,
        zone: &ZoneId,
        request: &ConfigStatusRequest,
    ) -> Result<ConfigStatusResponse, ConfigError> {
        authorize_host_operation(caller, &request.guest_ref, &request.identifier)?;
        let staged = self.pending.get(&guest_key(zone, &request.guest_ref));
        Ok(ConfigStatusResponse {
            guest_ref: request.guest_ref.clone(),
            pending: staged.is_some(),
            bytes: staged.map(GuestConfigDocument::len),
            sha256: staged.map(GuestConfigDocument::sha256),
        })
    }
}

fn guest_key(zone: &ZoneId, guest_ref: &ResourceRef) -> String {
    format!("{}/{}", zone.as_str(), guest_ref.to_canonical_string())
}

fn authorize_host_operation(
    caller: ConfigCaller,
    guest_ref: &ResourceRef,
    identifier: &str,
) -> Result<(), ConfigError> {
    if !matches!(caller, ConfigCaller::Admin | ConfigCaller::Lifecycle) {
        return Err(ConfigError::Unauthorized);
    }
    validate_guest_ref(guest_ref)?;
    validate_identifier(identifier)
}
