//! The admitted remote authority for one Azure virtual machine Guest.
//!
//! An Azure VM is not a local process. Starting one, attaching its one-time
//! bootstrap extension, and deleting it are all privileged effects against a
//! cloud account, so every ARM call this provider makes has to be
//! attributable to an admitted Guest and fenced against the credential
//! relationship the graph currently authorizes (R34, R35, R41).
//!
//! # What the gate decides, and in what order
//!
//! [`AzureVmRemoteAuthority::admit`] runs five checks, all of them before the
//! ARM endpoint is contacted, and stops at the first refusal:
//!
//! 1. the requested presentation is one this remote backend realizes. An ARM
//!    virtual machine is reached over the network: it presents no host mount
//!    tree and has no consumer namespace, so a local-style presentation is
//!    refused here rather than half-honoured against a cloud resource;
//! 2. the Guest the controller is reconciling is the Guest the accepted graph
//!    admitted - same uid, same generation;
//! 3. the subscription, resource group, tenant, and client the effect will
//!    address are the ones the authority was bound to;
//! 4. the credential relationship the graph admitted still authorizes this
//!    delivery session, through the shared
//!    [`admit_credential_delivery`](d2b_contracts_provider::v3::admit_credential_delivery)
//!    gate every Credential Provider realization runs (R24, AE10);
//! 5. the admitted session's audience is the ARM audience this provider
//!    actually requests, so a refresh cannot widen it (R24).
//!
//! # The reconciliation key is what makes a retry safe
//!
//! [`AzureVmReconciliationKey`] derives the ARM operation id and the cloud
//! resource name from the admitted relationship's own identity - the
//! authority Zone, the Guest uid, and the desired generation - and from
//! nothing else. The provider therefore has no way to ask ARM for a *second*
//! VM: every attempt, every retry, and every restart after a restart is the
//! same deterministic name, and the deterministic ARM idempotency token that
//! goes with it. An ambiguous provision that really created the machine is
//! reconciled on the next pass by looking up that name, not by creating
//! another one.
//!
//! # Nothing here is a broker syscall
//!
//! The gate decides whether *this provider* may talk to *this* subscription
//! for *this* admitted Guest. It does not resolve host paths, hand out
//! descriptors, or impersonate the local broker's effect carrier; the ARM
//! effect port behind it stays an ordinary cloud call.

use std::fmt;
use std::sync::Arc;

use d2b_contracts_provider::v3::{
    AdmittedCredentialDelivery, AudienceToken, CredentialAuthorization, CredentialDeliveryEvidence,
    CredentialMethod, DeliverySessionParams, OperationClass, PresentationCapability,
    admit_credential_delivery,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingRefusal, RefusalReason, ResourceRef, ResourceUid, ZoneId,
};

use crate::config::AzureVmGuestSettings;

/// The ARM audience this provider's credential must be scoped to.
///
/// Every ARM call - control plane or data plane - is an Azure Resource
/// Manager call, so a token minted for any other audience is refused by
/// [`AzureVmRemoteAuthority::admit`] rather than sent and rejected.
pub const AZURE_VM_CONTROL_AUDIENCE: &str = "https://management.azure.com/";

/// The identity the accepted graph admitted for one Guest row.
///
/// This is the graph's own identity, not the provider's configuration: a
/// controller handed a different Guest, a different generation, or a different
/// Provider row is refusing every remote mutation until it is rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureVmAdmittedGuest {
    zone: ZoneId,
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    provider_ref: ResourceRef,
    generation: u64,
}

impl AzureVmAdmittedGuest {
    /// Bind the identity one accepted `Guest` row carries.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmRemoteRefusal::ProviderIdentityMismatch`] when the
    /// admitted Provider row is not this Provider, and
    /// [`AzureVmRemoteRefusal::GuestIdentityMismatch`] when the generation is
    /// zero. Neither is retryable: a Guest whose graph identity does not name
    /// this provider has no business mutating this provider's subscription.
    pub fn new(
        zone: ZoneId,
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        provider_ref: ResourceRef,
        generation: u64,
    ) -> Result<Self, AzureVmRemoteRefusal> {
        if provider_ref.to_canonical_string() != crate::PROVIDER_REF {
            return Err(AzureVmRemoteRefusal::ProviderIdentityMismatch);
        }
        if generation == 0 {
            return Err(AzureVmRemoteRefusal::GuestIdentityMismatch);
        }
        Ok(Self {
            zone,
            guest_ref,
            guest_uid,
            provider_ref,
            generation,
        })
    }

    /// Borrow the authority Zone the graph admitted this Guest in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the admitted `Guest` reference.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Return the admitted Guest uid.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Borrow the admitted Provider row.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Return the desired generation this Guest was admitted at.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// The subscription, resource group, and identity one ARM effect may address.
///
/// Two authorities that disagree here are two different Azure accounts. The
/// comparison is on every field, so a provider reconfigured onto another
/// subscription or resource group cannot act on the account its authority was
/// admitted for.
#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmCloudIdentity {
    tenant_id: Option<String>,
    client_id: Option<String>,
    subscription_id: String,
    resource_group: String,
}

impl AzureVmCloudIdentity {
    /// Bind one Azure target from the provider configuration and the Guest's
    /// own settings.
    pub fn new(
        provider: &crate::config::AzureVmConfig,
        settings: &AzureVmGuestSettings,
    ) -> Self {
        Self {
            tenant_id: provider.tenant_id.as_ref().map(|value| value.as_str().to_owned()),
            client_id: provider.client_id.as_ref().map(|value| value.as_str().to_owned()),
            subscription_id: settings.subscription_id.as_str().to_owned(),
            resource_group: settings.resource_group.as_str().to_owned(),
        }
    }

    /// Borrow the subscription this controller addresses.
    pub fn subscription_id(&self) -> &str {
        &self.subscription_id
    }

    /// Borrow the resource group this controller addresses.
    pub fn resource_group(&self) -> &str {
        &self.resource_group
    }

    /// Bind one Azure target directly.
    pub fn from_parts(
        tenant_id: Option<String>,
        client_id: Option<String>,
        subscription_id: String,
        resource_group: String,
    ) -> Self {
        Self {
            tenant_id,
            client_id,
            subscription_id,
            resource_group,
        }
    }
}

impl fmt::Debug for AzureVmCloudIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmCloudIdentity")
            .field("tenant_id", &self.tenant_id.as_ref().map(|_| "<redacted>"))
            .field("client_id", &self.client_id.as_ref().map(|_| "<redacted>"))
            .field("subscription_id", &"<redacted>")
            .field("resource_group", &"<redacted>")
            .finish()
    }
}

/// The deterministic ARM identity one admitted Guest owns.
///
/// Both halves come from the admitted relationship's identity, so they are
/// the same on every attempt. `operation_id` is the ARM idempotency token
/// and `resource_name` is the cloud object's name: a retry after an ambiguous
/// response presents the first, and looks up the second, instead of creating
/// a second virtual machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureVmReconciliationKey {
    resource_name: String,
    operation_id: String,
}

impl AzureVmReconciliationKey {
    /// Derive the ARM identity for one admitted Guest at one generation and
    /// operation class.
    pub fn derive(guest: &AzureVmAdmittedGuest, operation_class: &str) -> Self {
        let mut digest = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut digest, guest.zone.as_str().as_bytes());
        sha2::Digest::update(&mut digest, [0]);
        sha2::Digest::update(&mut digest, guest.guest_uid.as_str().as_bytes());
        sha2::Digest::update(&mut digest, [0]);
        sha2::Digest::update(&mut digest, guest.generation.to_be_bytes());
        sha2::Digest::update(&mut digest, [0]);
        sha2::Digest::update(&mut digest, operation_class.as_bytes());
        let rendered = base32(&sha2::Digest::finalize(digest));
        Self {
            resource_name: format!("d2b-{}", &rendered[..20]),
            operation_id: rendered[..20].to_owned(),
        }
    }

    /// Return the cloud object name this Guest owns for this operation.
    pub fn resource_name(&self) -> &str {
        &self.resource_name
    }

    /// Return the deterministic ARM idempotency token for this operation.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

/// The closed set of remote operations this provider performs.
///
/// The purpose decides the credential method, so a call that does not carry
/// material never asks for a delivery session and a call that does can never
/// borrow another purpose's class (R24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmRemotePurpose {
    /// Read ARM: observe the VM, poll a long-running operation.
    Inspect,
    /// Start the VM provisioning long-running operation.
    Provision,
    /// Deliver or remove the one-time bootstrap extension.
    Bootstrap,
    /// Delete the VM or the provider-owned child resources.
    Delete,
}

impl AzureVmRemotePurpose {
    /// Whether this purpose can change remote state.
    pub const fn mutates_remote_state(self) -> bool {
        !matches!(self, Self::Inspect)
    }

    /// The credential service method this purpose runs under.
    pub const fn credential_method(self) -> CredentialMethod {
        match self {
            Self::Inspect => CredentialMethod::InspectMetadata,
            Self::Provision | Self::Bootstrap | Self::Delete => CredentialMethod::AcquireToken,
        }
    }

    /// The operation class this purpose's reconciliation key is derived
    /// under, so two purposes never share a cloud name.
    pub const fn operation_class(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Provision => "provision",
            Self::Bootstrap => "bootstrap",
            Self::Delete => "delete",
        }
    }

    /// Return the stable code for this purpose.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Inspect => "azure-vm-remote-inspect",
            Self::Provision => "azure-vm-remote-provision",
            Self::Bootstrap => "azure-vm-remote-bootstrap",
            Self::Delete => "azure-vm-remote-delete",
        }
    }
}

/// Closed, field-free refusals from the remote authority.
///
/// A refusal names a reason and nothing else: no audience, no reference, no
/// generation, no credential byte, and no Azure identifier (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmRemoteRefusal {
    /// The requested presentation is not one this remote backend realizes.
    PresentationUnsupported,
    /// The admitted Provider row is not this Provider.
    ProviderIdentityMismatch,
    /// The controller's Guest uid or generation is not the admitted one.
    GuestIdentityMismatch,
    /// The effect would address a different subscription or resource group.
    CloudIdentityMismatch,
    /// The presented method establishes no delivery session to admit.
    NoDeliverySession,
    /// The admitted credential relationship refused the live evidence.
    CredentialFenceRefused,
    /// The admitted session's audience or operation class is outside policy.
    CredentialPolicyRefused,
    /// The presented session is not the relationship's current one.
    CredentialSessionSuperseded,
    /// The admitted session's audience is not this provider's ARM audience.
    AudienceMismatch,
}

impl AzureVmRemoteRefusal {
    /// Return the closed, non-secret refusal code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PresentationUnsupported => "azure-vm-remote-presentation-unsupported",
            Self::ProviderIdentityMismatch => "azure-vm-remote-provider-identity-mismatch",
            Self::GuestIdentityMismatch => "azure-vm-remote-guest-identity-mismatch",
            Self::CloudIdentityMismatch => "azure-vm-remote-cloud-identity-mismatch",
            Self::NoDeliverySession => "azure-vm-remote-no-delivery-session",
            Self::CredentialFenceRefused => "azure-vm-remote-credential-fence-refused",
            Self::CredentialPolicyRefused => "azure-vm-remote-credential-policy-refused",
            Self::CredentialSessionSuperseded => "azure-vm-remote-credential-session-superseded",
            Self::AudienceMismatch => "azure-vm-remote-audience-mismatch",
        }
    }

    const fn from_stage_and_reason(stage: AdmissionStage, reason: RefusalReason) -> Self {
        match (stage, reason) {
            (AdmissionStage::Prepare, _) => Self::NoDeliverySession,
            (AdmissionStage::Admit, RefusalReason::StaleAuthority) => Self::CredentialFenceRefused,
            (AdmissionStage::Admit, _) => Self::CredentialPolicyRefused,
            _ => Self::CredentialSessionSuperseded,
        }
    }
}

impl From<BindingRefusal> for AzureVmRemoteRefusal {
    fn from(refusal: BindingRefusal) -> Self {
        Self::from_stage_and_reason(refusal.stage(), refusal.reason())
    }
}

impl From<d2b_contracts_provider::v3::CredentialDeliveryAdmission> for AzureVmRemoteRefusal {
    fn from(admission: d2b_contracts_provider::v3::CredentialDeliveryAdmission) -> Self {
        Self::from_stage_and_reason(admission.stage(), admission.reason())
    }
}

impl fmt::Display for AzureVmRemoteRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AzureVmRemoteRefusal {}

/// The effective-access evidence one admitted ARM call carries.
///
/// This is what "ARM will act on exactly this" means for a remote backend:
/// the exact admitted Guest, at the exact admitted subscription, under the
/// exact delivery session the graph currently authorizes, for the cloud
/// object and ARM operation token this key pins. No caller can widen it
/// after the fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureVmRemoteGrant {
    reconciliation: AzureVmReconciliationKey,
    guest_uid: ResourceUid,
    generation: u64,
    delivery_sequence: u64,
    operation_class: OperationClass,
}

impl AzureVmRemoteGrant {
    /// Return the deterministic ARM identity this call may act on.
    pub const fn reconciliation(&self) -> &AzureVmReconciliationKey {
        &self.reconciliation
    }

    /// Return the admitted Guest uid the call acts for.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Return the admitted generation the call acts at.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Return the admitted delivery session's replay sequence.
    pub const fn delivery_sequence(&self) -> u64 {
        self.delivery_sequence
    }

    /// Return the admitted credential operation class this call runs under.
    pub const fn operation_class(&self) -> OperationClass {
        self.operation_class
    }
}

/// The admitted remote authority for one Guest in one Azure subscription.
///
/// Construct one from the graph's accepted identity and the `CredentialBinding`
/// relationship the source admitted; bind it onto the controller with
/// [`crate::AzureVmController::with_admitted_authority`].
pub struct AzureVmRemoteAuthority {
    guest: AzureVmAdmittedGuest,
    cloud: AzureVmCloudIdentity,
    audience: AudienceToken,
    credential: Arc<dyn AdmittedCredentialDelivery>,
}

impl AzureVmRemoteAuthority {
    /// Bind the authority to one admitted Guest, one Azure target, and one
    /// admitted credential relationship.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmRemoteRefusal::AudienceMismatch`] when the ARM audience
    /// constant is not a bounded audience token. That is a defect in this
    /// crate rather than a runtime condition, so it is reported as a refusal.
    pub fn new(
        guest: AzureVmAdmittedGuest,
        cloud: AzureVmCloudIdentity,
        credential: Arc<dyn AdmittedCredentialDelivery>,
    ) -> Result<Self, AzureVmRemoteRefusal> {
        let audience = AudienceToken::parse(AZURE_VM_CONTROL_AUDIENCE)
            .map_err(|_| AzureVmRemoteRefusal::AudienceMismatch)?;
        Ok(Self {
            guest,
            cloud,
            audience,
            credential,
        })
    }

    /// Borrow the admitted Guest identity.
    pub const fn guest(&self) -> &AzureVmAdmittedGuest {
        &self.guest
    }

    /// Borrow the admitted Azure target.
    pub const fn cloud(&self) -> &AzureVmCloudIdentity {
        &self.cloud
    }

    /// Return the deterministic ARM identity for one operation class.
    pub fn reconciliation_key(
        &self,
        operation_class: AzureVmRemotePurpose,
    ) -> AzureVmReconciliationKey {
        AzureVmReconciliationKey::derive(&self.guest, operation_class.operation_class())
    }

    /// Admit one ARM operation, or refuse it before ARM is contacted.
    ///
    /// `guest_uid`, `generation`, and `cloud` are what the controller is
    /// actually reconciling and addressing, so a controller that drifted off
    /// its Guest or off its subscription is refused here.
    /// `requested_presentation` is the consumer-facing presentation the
    /// request asked for; the ceiling comes from this Provider's own
    /// declaration, so a request cannot talk this remote backend into a
    /// local-style presentation.
    ///
    /// # Errors
    ///
    /// Returns the [`AzureVmRemoteRefusal`] of the first failed check, in the
    /// order documented on the module.
    #[allow(clippy::too_many_arguments, reason = "one parameter per gate input")]
    pub fn admit(
        &self,
        guest_uid: &ResourceUid,
        generation: u64,
        cloud: Option<&AzureVmCloudIdentity>,
        purpose: AzureVmRemotePurpose,
        requested_presentation: PresentationCapability,
        authorization: &CredentialAuthorization,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<AzureVmRemoteGrant, AzureVmRemoteRefusal> {
        if !crate::declared_presentation().realizes(requested_presentation) {
            return Err(AzureVmRemoteRefusal::PresentationUnsupported);
        }
        if guest_uid != self.guest.guest_uid() || generation != self.guest.generation() {
            return Err(AzureVmRemoteRefusal::GuestIdentityMismatch);
        }
        if cloud != Some(&self.cloud) {
            return Err(AzureVmRemoteRefusal::CloudIdentityMismatch);
        }
        let method = purpose.credential_method();
        let session = if method.requires_delivery() {
            Some(admit_credential_delivery(
                authorization,
                method,
                self.credential.as_ref(),
                evidence,
            )?)
        } else {
            // An ARM read carries no material, so there is no delivery session
            // to admit. Two things still have to hold: the relationship's own
            // fence must still accept the live evidence - a revoked Credential
            // stops a read as surely as it stops a write - and a read that
            // presents a delivery session is refused rather than downgraded.
            if let Some(refusal) = self.credential.fence_refusal(evidence) {
                return Err(refusal.into());
            }
            if authorization.delivery_session_params().is_some() {
                return Err(AzureVmRemoteRefusal::NoDeliverySession);
            }
            None
        };
        if let Some(params) = session.as_ref()
            && params.audience() != &self.audience
        {
            return Err(AzureVmRemoteRefusal::AudienceMismatch);
        }
        Ok(AzureVmRemoteGrant {
            reconciliation: self.reconciliation_key(purpose),
            guest_uid: self.guest.guest_uid().clone(),
            generation: self.guest.generation(),
            delivery_sequence: session.as_ref().map(DeliverySessionParams::sequence).unwrap_or(0),
            operation_class: method.operation_class(),
        })
    }
}

impl fmt::Debug for AzureVmRemoteAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmRemoteAuthority")
            .field("guest", &self.guest)
            .field("cloud", &self.cloud)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

/// The delivery context one operation presents to the remote authority.
///
/// The Credential service adapter builds it: the authorization carries the
/// delivery session the adapter authorized, and the evidence carries the live
/// generations the adapter observed. Neither is constructed by the provider,
/// so a caller cannot present evidence the session it is admitting does not
/// itself carry.
#[derive(Clone, PartialEq, Eq)]
pub struct AzureVmDeliveryContext {
    authorization: CredentialAuthorization,
    evidence: CredentialDeliveryEvidence,
}

impl AzureVmDeliveryContext {
    /// Bind one operation's authorization to its live observations.
    pub const fn new(
        authorization: CredentialAuthorization,
        evidence: CredentialDeliveryEvidence,
    ) -> Self {
        Self {
            authorization,
            evidence,
        }
    }

    /// Borrow the adapter-authorized delivery session.
    pub const fn authorization(&self) -> &CredentialAuthorization {
        &self.authorization
    }

    /// Borrow the live, non-secret delivery observations.
    pub const fn evidence(&self) -> &CredentialDeliveryEvidence {
        &self.evidence
    }
}

impl fmt::Debug for AzureVmDeliveryContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AzureVmDeliveryContext(<redacted>)")
    }
}

/// The Credential service adapter this provider presents its operations to.
///
/// This is the seam the daemon composition root fills with the real
/// authenticated credential client. A Provider never opens an ambient
/// credential chain of its own, so this port is how an ARM token reaches a
/// control-plane call at all.
#[async_trait::async_trait]
pub trait AzureVmRemoteDeliveryPort: Send + Sync {
    /// Authorize one ARM operation, or refuse it without contacting ARM.
    ///
    /// # Errors
    ///
    /// Returns the [`AzureVmRemoteRefusal`] the adapter reached when the graph
    /// no longer authorizes a delivery session for this operation.
    async fn authorize(
        &self,
        purpose: AzureVmRemotePurpose,
        guest_uid: &ResourceUid,
        generation: u64,
    ) -> Result<AzureVmDeliveryContext, AzureVmRemoteRefusal>;
}

/// The admitted remote authority and the delivery port that feeds it.
///
/// Together they are the whole R34 attribution for one ARM call.
#[derive(Clone)]
pub struct AzureVmAdmittedRemote {
    authority: Arc<AzureVmRemoteAuthority>,
    delivery: Arc<dyn AzureVmRemoteDeliveryPort>,
}

impl AzureVmAdmittedRemote {
    /// Pair an admitted authority with its credential adapter.
    pub const fn new(
        authority: Arc<AzureVmRemoteAuthority>,
        delivery: Arc<dyn AzureVmRemoteDeliveryPort>,
    ) -> Self {
        Self {
            authority,
            delivery,
        }
    }

    /// Borrow the admitted authority.
    pub fn authority(&self) -> &Arc<AzureVmRemoteAuthority> {
        &self.authority
    }

    /// Ask the adapter for the authorized delivery session, then run the
    /// authority's five checks over it.
    ///
    /// # Errors
    ///
    /// Returns the [`AzureVmRemoteRefusal`] of the adapter or of the first
    /// failed authority check.
    pub async fn admit(
        &self,
        guest_uid: &ResourceUid,
        generation: u64,
        cloud: Option<&AzureVmCloudIdentity>,
        purpose: AzureVmRemotePurpose,
        requested_presentation: PresentationCapability,
    ) -> Result<AzureVmRemoteGrant, AzureVmRemoteRefusal> {
        // The presentation ceiling is checked before the credential adapter
        // is asked, so a request this backend cannot realize is refused
        // without asking for material it will never use.
        if !crate::declared_presentation().realizes(requested_presentation) {
            return Err(AzureVmRemoteRefusal::PresentationUnsupported);
        }
        let delivery = self
            .delivery
            .authorize(purpose, guest_uid, generation)
            .await?;
        self.authority.admit(
            guest_uid,
            generation,
            cloud,
            purpose,
            requested_presentation,
            delivery.authorization(),
            delivery.evidence(),
        )
    }
}

impl fmt::Debug for AzureVmAdmittedRemote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmAdmittedRemote")
            .field("authority", &self.authority)
            .finish_non_exhaustive()
    }
}

/// The terminal evidence one finalized Guest leaves behind.
///
/// Release is not complete when the delete was issued; it is complete when
/// the VM is absent, the one-time bootstrap extension is gone, the
/// provider-owned child resources are cleaned, and the controller has dropped
/// its finalizer. This value is that record, and it is the only thing a caller
/// may report as "the Guest is gone" (R36, AE18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureVmReleaseEvidence {
    reconciliation: AzureVmReconciliationKey,
    guest_uid: ResourceUid,
    generation: u64,
    vm_absent: bool,
    bootstrap_extension_absent: bool,
    child_cleanup_complete: bool,
    finalizer_released: bool,
}

impl AzureVmReleaseEvidence {
    /// Record the terminal state of one finalized Guest.
    #[allow(clippy::too_many_arguments, reason = "one field per release precondition")]
    pub const fn new(
        reconciliation: AzureVmReconciliationKey,
        guest_uid: ResourceUid,
        generation: u64,
        vm_absent: bool,
        bootstrap_extension_absent: bool,
        child_cleanup_complete: bool,
        finalizer_released: bool,
    ) -> Self {
        Self {
            reconciliation,
            guest_uid,
            generation,
            vm_absent,
            bootstrap_extension_absent,
            child_cleanup_complete,
            finalizer_released,
        }
    }

    /// Return the deterministic ARM identity the released Guest owned.
    pub const fn reconciliation(&self) -> &AzureVmReconciliationKey {
        &self.reconciliation
    }

    /// Return the released Guest uid.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Return the generation the released Guest was admitted at.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the cloud reported the virtual machine absent.
    pub const fn vm_absent(&self) -> bool {
        self.vm_absent
    }

    /// Whether the one-time bootstrap extension is confirmed removed.
    pub const fn bootstrap_extension_absent(&self) -> bool {
        self.bootstrap_extension_absent
    }

    /// Whether provider-owned child-resource cleanup is confirmed complete.
    pub const fn child_cleanup_complete(&self) -> bool {
        self.child_cleanup_complete
    }

    /// Whether the controller has dropped its finalizer.
    pub const fn finalizer_released(&self) -> bool {
        self.finalizer_released
    }

    /// Whether this evidence proves the remote Guest is gone.
    ///
    /// All four halves are required. A dropped finalizer without a confirmed
    /// absence is a lie about the cloud; a confirmed absence with the finalizer
    /// still installed means the Guest can be recreated by the next reconcile.
    pub const fn is_terminal(&self) -> bool {
        self.vm_absent
            && self.bootstrap_extension_absent
            && self.child_cleanup_complete
            && self.finalizer_released
    }
}

/// Render a digest in the lower-alphanumeric alphabet ARM accepts.
fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut output = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer = 0u16;
    let mut bits = 0u8;
    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(char::from(ALPHABET[usize::from((buffer >> bits) & 0x1f)]));
        }
    }
    if bits != 0 {
        output.push(char::from(ALPHABET[usize::from((buffer << (5 - bits)) & 0x1f)]));
    }
    output
}
