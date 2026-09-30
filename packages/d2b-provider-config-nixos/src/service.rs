//! Closed config-nixos service DTOs.

use std::collections::BTreeMap;
use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingAuthorization, BindingKey, BindingKind,
    BindingRefusal, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
    FreshnessTuple, RefusalReason, RequestedRights, ResourceRef, ResourceUid, SourceAdmission,
    VolumeBindingRequest, ZoneId,
    volume::AttachmentAccess,
    volume_binding::VolumePresentation,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ConfigCaller, ConfigError, GuestConfigDocument};

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
    pub(crate) identifier: String,
}

impl ConfigSyncRequest {
    /// Construct and validate a closed request.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the reference does not
    /// name a Guest.
    pub fn new(guest_ref: ResourceRef) -> Result<Self, ConfigError> {
        validate_guest_ref(&guest_ref)?;
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
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the identifier is not the
    /// closed Guest-config identifier or the base64 exceeds the encoded bound,
    /// [`ConfigError::EncodingFailed`] when the base64 does not decode, and
    /// the document bounds errors ([`ConfigError::EmptyDocument`],
    /// [`ConfigError::DocumentTooLarge`], [`ConfigError::InvalidUtf8`]) when
    /// the decoded bytes fail validation.
    pub fn document(&self) -> Result<GuestConfigDocument, ConfigError> {
        if self.identifier != GUEST_CONFIG_IDENTIFIER
            || self.content_base64.len() > MAX_CONFIG_ENCODED_BYTES
        {
            return Err(ConfigError::InvalidRequest);
        }
        let bytes = STANDARD.decode(&self.content_base64).map_err(|error| {
            tracing::warn!(
                resource = %self.guest_ref.to_canonical_string(),
                %error,
                "config-nixos document decode failed",
            );
            ConfigError::EncodingFailed
        })?;
        let document = GuestConfigDocument::new(bytes)?;
        if document.len() != self.bytes || document.sha256() != self.sha256 {
            tracing::warn!(
                resource = %self.guest_ref.to_canonical_string(),
                "config-nixos document integrity mismatch against response digest",
            );
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
    pub(crate) identifier: String,
    /// Base64-encoded document to validate and stage.
    pub content_base64: String,
}

impl ConfigStageRequest {
    /// Construct a stage request from one already validated document.
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the reference does not
    /// name a Guest.
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
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the identifier is not the
    /// closed Guest-config identifier, [`ConfigError::DocumentTooLarge`] when
    /// the base64 exceeds the encoded bound, [`ConfigError::EncodingFailed`]
    /// when the base64 does not decode, and the document bounds errors
    /// ([`ConfigError::EmptyDocument`], [`ConfigError::DocumentTooLarge`],
    /// [`ConfigError::InvalidUtf8`]) when the decoded bytes fail validation.
    pub fn document(&self) -> Result<GuestConfigDocument, ConfigError> {
        validate_identifier(&self.identifier)?;
        if self.content_base64.len() > MAX_CONFIG_ENCODED_BYTES {
            return Err(ConfigError::DocumentTooLarge);
        }
        let bytes = STANDARD.decode(&self.content_base64).map_err(|error| {
            tracing::warn!(
                resource = %self.guest_ref.to_canonical_string(),
                %error,
                "config-nixos staged document decode failed",
            );
            ConfigError::EncodingFailed
        })?;
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
    pub(crate) identifier: String,
    /// Stable local view identifier, not a file path.
    pub(crate) against: String,
}

impl ConfigDiffRequest {
    /// Construct a diff request from a stable content-view digest.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the reference does not
    /// name a Guest and [`ConfigError::InvalidView`] when the view identifier
    /// is not a `sha256:` content commitment.
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
    pub(crate) identifier: String,
    /// Stable host configuration target identifier.
    pub(crate) destination: String,
}

impl ConfigApproveRequest {
    /// Construct an approval request for one opaque host target.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the reference does not
    /// name a Guest and [`ConfigError::InvalidDestination`] when the
    /// destination is empty, longer than 128 bytes, non-ASCII, or contains a
    /// path separator or whitespace.
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
    pub(crate) identifier: String,
}

impl ConfigRejectRequest {
    /// Construct a rejection request.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the reference does not
    /// name a Guest.
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
    pub(crate) identifier: String,
}

impl ConfigStatusRequest {
    /// Construct a status request.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the reference does not
    /// name a Guest.
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

// ---------------------------------------------------------------------------
// The canonical declaration configuration publishes
// ---------------------------------------------------------------------------

/// The realization facet a Guest's configuration working copy requires.
///
/// The document reaches the Guest as a private mount tree carrying the exact
/// named view at the destination, so a backend that cannot enforce that facet
/// is refused rather than handed a weaker presentation.
pub const CONFIG_WORKING_COPY_FACETS: [BindingRealizationFacet; 1] =
    [BindingRealizationFacet::FilesystemPresentation];

/// The realization support this Provider's configuration delivery declares.
///
/// This is a fixed set derived from the one presentation configuration
/// actually delivers. It is not a parameter a caller can widen, so a caller
/// cannot declare support for a facet the configuration path never
/// realizes.
pub fn config_working_copy_support() -> &'static BindingRealizationSupport {
    static SUPPORT: std::sync::OnceLock<BindingRealizationSupport> = std::sync::OnceLock::new();
    SUPPORT.get_or_init(|| {
        BindingRealizationSupport::new(CONFIG_WORKING_COPY_FACETS.to_vec())
            .expect("the configuration facet set is fixed and duplicate-free")
    })
}

/// One configuration attachment declared against the typed binding contract.
///
/// The declaration names the exact `Volume` source, the exact `Guest`
/// consumer, the stable consumer slot, the named view, the access level, and
/// the consumer-side presentation. It carries no host source path, no
/// numerical principal, and no document bytes: the document is delivered
/// through the admitted relationship, not through an authored path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigAttachment {
    request: VolumeBindingRequest,
}

impl ConfigAttachment {
    /// Construct one attachment declaration from typed references.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRequest`] when the source is not a
    /// `Volume`, when the consumer is not an admitted consumer kind, when the
    /// slot or view is not a bounded token, or when the destination is not a
    /// consumer-side mount path.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        slot: BindingSlot,
        view: d2b_contracts_resource::v3::BoundedToken,
        access: AttachmentAccess,
        destination: &str,
    ) -> Result<Self, ConfigError> {
        let presentation = VolumePresentation::filesystem(destination)
            .map_err(|_| ConfigError::InvalidDestination)?;
        let request = VolumeBindingRequest::new(source, consumer, slot, view, access, presentation)
            .map_err(|_| ConfigError::InvalidRequest)?;
        Ok(Self { request })
    }

    /// Borrow the canonical desired request this declaration states.
    pub const fn request(&self) -> &VolumeBindingRequest {
        &self.request
    }

    /// The exact source the attachment reads from.
    pub const fn source_ref(&self) -> &ResourceRef {
        self.request.source_ref()
    }

    /// The exact consumer the attachment delivers to.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        self.request.consumer_ref()
    }

    /// The realization facets this attachment depends on.
    pub fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        self.request.required_facets()
    }

    /// The right this attachment asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        self.request.requested_rights()
    }

    /// Admit this attachment through the typed binding contract.
    ///
    /// Admission is the source Volume's decision, not configuration's: this
    /// Provider supplies the declaration and the freshness evidence, and the
    /// grant and the source admission arrive from the Role evaluation and the
    /// Volume owner respectively. A configuration document therefore cannot
    /// authorize its own delivery.
    ///
    /// # Errors
    ///
    /// Returns the typed [`BindingRefusal`] naming the enforcing stage. A
    /// caller that presents no authorization evidence is refused at
    /// `Authorize`, and a source that does not admit these rights is refused
    /// at `Admit`.
    pub fn admit(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
        authorization: &BindingAuthorization,
        source: &SourceAdmission,
        dependencies: &[FreshnessTuple],
    ) -> Result<BindingAdmission, BindingRefusal> {
        let key = BindingKey::new(
            zone,
            BindingKind::Volume,
            self.request.source_ref().clone(),
            source_uid,
            self.request.consumer_ref().clone(),
            consumer_uid,
            self.request.slot().clone(),
        )
        .map_err(|_| {
            BindingRefusal::new(AdmissionStage::Normalize, RefusalReason::ConflictingDeclaration)
        })?;
        d2b_contracts_resource::v3::admit_binding_request(
            &key,
            self.requested_rights(),
            self.required_facets(),
            authorization,
            source,
            config_working_copy_support(),
            dependencies,
        )
    }
}

/// The diagnostic a refused configuration attachment renders.
///
/// It names the relationship and the enforcing stage and carries the typed
/// reason. It never carries the document, the destination, a host path, or a
/// session identity (R42).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRefusal {
    // The attachment is boxed so the refusal travels by reference: it names
    // the relationship rather than carrying a second copy of it, and the
    // refusal a caller propagates stays a small value.
    attachment: Box<ConfigAttachment>,
    refusal: BindingRefusal,
}

impl ConfigRefusal {
    /// Construct a refusal for one attachment.
    pub fn new(attachment: ConfigAttachment, refusal: BindingRefusal) -> Self {
        Self {
            attachment: Box::new(attachment),
            refusal,
        }
    }

    /// Borrow the exact relationship that was refused.
    pub const fn attachment(&self) -> &ConfigAttachment {
        &self.attachment
    }

    /// Return the enforcing stage.
    pub const fn stage(&self) -> AdmissionStage {
        self.refusal.stage()
    }

    /// Return the typed reason.
    pub const fn reason(&self) -> RefusalReason {
        self.refusal.reason()
    }
}

impl fmt::Display for ConfigRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "config attachment {} -> {} refused at {:?} ({:?})",
            self.attachment.source_ref().to_canonical_string(),
            self.attachment.consumer_ref().to_canonical_string(),
            self.stage(),
            self.reason()
        )
    }
}

impl std::error::Error for ConfigRefusal {}

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
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unauthorized`] when the caller is not Admin,
    /// [`ConfigError::InvalidRequest`] when the reference or identifier is
    /// not the closed Guest-config pair, and the document bounds errors
    /// ([`ConfigError::EmptyDocument`], [`ConfigError::DocumentTooLarge`],
    /// [`ConfigError::InvalidUtf8`]).
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
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unauthorized`] when the caller is not Admin,
    /// [`ConfigError::InvalidRequest`] for a wrong reference or identifier,
    /// [`ConfigError::InvalidView`] for a malformed view identifier, and
    /// [`ConfigError::StagingMissing`] when nothing is staged for the Guest.
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
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unauthorized`] when the caller is not Admin,
    /// [`ConfigError::InvalidRequest`] for a wrong reference or identifier,
    /// [`ConfigError::InvalidDestination`] for a malformed destination,
    /// [`ConfigError::StagingMissing`] when nothing is staged or approved for
    /// the Guest, and [`ConfigError::ApprovalConflict`] when an approved
    /// receipt exists for a different destination.
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
            tracing::warn!(
                resource = %request.guest_ref.to_canonical_string(),
                "config-nixos approval rejected: destination conflicts with approved receipt",
            );
            return Err(ConfigError::ApprovalConflict);
        }
        Ok(ConfigApproveResponse {
            guest_ref: request.guest_ref.clone(),
            bytes: approved.bytes,
            sha256: approved.sha256.clone(),
        })
    }

    /// Reject staged content, returning whether anything was removed.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unauthorized`] when the caller is not Admin and
    /// [`ConfigError::InvalidRequest`] for a wrong reference or identifier.
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
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unauthorized`] when the caller is not Admin and
    /// [`ConfigError::InvalidRequest`] for a wrong reference or identifier.
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
    if !matches!(caller, ConfigCaller::Admin) {
        return Err(ConfigError::Unauthorized);
    }
    validate_guest_ref(guest_ref)?;
    validate_identifier(identifier)
}
