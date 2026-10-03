//! The admitted remote authority for one Azure Container Apps Guest.
//!
//! A sandbox in Azure Container Apps is not a local process. Provisioning one
//! mutates a cloud account, so every remote call this provider makes is a
//! privileged effect that R34 requires to be attributable to an admitted
//! resource, and R35 requires to be fenced against identity, policy, and
//! generation changes. Nothing in [`crate::controller`] may ask the control
//! plane to do anything until this module says so.
//!
//! # What the gate decides, and in what order
//!
//! [`AcaRemoteAuthority::admit`] runs five checks, all of them before any
//! remote call, and stops at the first refusal:
//!
//! 1. the requested presentation is one this remote backend realizes. A
//!    remote sandbox presents no local pathname and has no consumer network
//!    namespace, so a local-style presentation is refused here rather than
//!    half-honoured against a cloud resource;
//! 2. the Guest the controller holds is the Guest the accepted graph admitted
//!    - same uid, same generation;
//! 3. the cloud account, environment, and resource group the effect will
//!    address are the ones the authority was bound to;
//! 4. the credential relationship the graph admitted still authorizes this
//!    delivery session, through the shared
//!    [`admit_credential_delivery`](d2b_contracts_provider::v3::admit_credential_delivery)
//!    gate every Credential Provider realization runs (R24, R35, AE10);
//! 5. the admitted session's audience is the ARM audience this control plane
//!    actually requires, so a refresh cannot widen it (R24).
//!
//! The order matters: a presentation the backend cannot realize is refused
//! before the identity is even considered, and the credential is never asked
//! for until the identity and the cloud target already agree.
//!
//! # The reconciliation key is the point of the module
//!
//! [`AcaReconciliationKey`] is derived from the admitted relationship's own
//! identity - the authority Zone, the Guest uid, and the desired generation -
//! and nothing else. It is therefore stable across a retry, across a
//! controller restart, and across two concurrent reconciles of the same
//! admitted Guest. The control plane is asked for a sandbox *by that name*,
//! so an ambiguous create response resolves to the resource the first attempt
//! intended instead of minting a second one. That is a property of the
//! naming, not of the retry: the same admitted Guest always asks for the same
//! cloud name.
//!
//! # Nothing here is a broker syscall
//!
//! The gate decides whether *this provider* may talk to *this* cloud account
//! for *this* admitted Guest. It does not resolve host paths, hand out
//! descriptors, or stand in for the local broker's effect carrier; the remote
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

use crate::effects::{AcaConfiguredImageId, AcaResourceBinding, AcaSandboxId};

/// The ARM audience this provider's control-plane credential must be scoped to.
///
/// A Container Apps control-plane call is an Azure Resource Manager call, so a
/// token minted for any other audience is refused by [`AcaRemoteAuthority::admit`]
/// rather than sent and rejected by the service.
pub const ACA_CONTROL_AUDIENCE: &str = "https://management.azure.com/";

/// The identity the accepted graph admitted for one Guest row.
///
/// This is the graph's own identity, not the provider's configuration: a
/// controller that was handed a different Guest, a different generation, or a
/// different Provider row is refusing every remote mutation until it is
/// rebuilt (R35, R41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaAdmittedGuest {
    zone: ZoneId,
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    provider_ref: ResourceRef,
    generation: u64,
}

impl AcaAdmittedGuest {
    /// Bind the identity one accepted `Guest` row carries.
    ///
    /// # Errors
    ///
    /// Returns [`AcaRemoteRefusal::ProviderIdentityMismatch`] when the
    /// admitted Provider row is not this Provider, and
    /// [`AcaRemoteRefusal::GuestIdentityMismatch`] when the generation is
    /// zero. Neither is a runtime condition a caller can retry past: a Guest
    /// whose graph identity does not name this provider has no business
    /// mutating this provider's cloud account.
    pub fn new(
        zone: ZoneId,
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        provider_ref: ResourceRef,
        generation: u64,
    ) -> Result<Self, AcaRemoteRefusal> {
        if provider_ref.to_canonical_string() != crate::PROVIDER_REF {
            return Err(AcaRemoteRefusal::ProviderIdentityMismatch);
        }
        if generation == 0 {
            return Err(AcaRemoteRefusal::GuestIdentityMismatch);
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

/// The cloud account, environment, and resource group one remote effect may
/// address.
///
/// Two authorities that disagree here are two different cloud accounts. The
/// comparison is on every field, so a provider reconfigured onto another
/// subscription, tenant, or resource group cannot act on the account its
/// authority was admitted for.
#[derive(Clone, PartialEq, Eq)]
pub struct AcaCloudIdentity {
    tenant_id: String,
    client_id: String,
    subscription_id: String,
    environment_id: AcaConfiguredImageId,
    resource_group_id: AcaConfiguredImageId,
}

impl AcaCloudIdentity {
    /// Bind one cloud target.
    pub const fn new(
        tenant_id: String,
        client_id: String,
        subscription_id: String,
        environment_id: AcaConfiguredImageId,
        resource_group_id: AcaConfiguredImageId,
    ) -> Self {
        Self {
            tenant_id,
            client_id,
            subscription_id,
            environment_id,
            resource_group_id,
        }
    }
}

impl fmt::Debug for AcaCloudIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaCloudIdentity")
            .field("tenant_id", &"<redacted>")
            .field("client_id", &"<redacted>")
            .field("subscription_id", &"<redacted>")
            .field("environment_id", &self.environment_id)
            .field("resource_group_id", &self.resource_group_id)
            .finish()
    }
}

/// The deterministic cloud names one admitted Guest owns.
///
/// Derived from the admitted relationship's own identity, so every attempt,
/// every retry, and every restart asks the control plane for the same two
/// names. A retry after an ambiguous response therefore reconciles the
/// resource the first attempt intended: there is no second name for it to
/// create under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaReconciliationKey {
    sandbox: AcaSandboxId,
    disk_image: AcaConfiguredImageId,
}

impl AcaReconciliationKey {
    /// Derive the cloud names for one admitted Guest at one generation.
    pub fn derive(guest: &AcaAdmittedGuest) -> Self {
        let admitted = [
            guest.zone.as_str().as_bytes(),
            guest.guest_uid.as_str().as_bytes(),
            &guest.generation.to_be_bytes(),
        ];
        Self {
            sandbox: AcaSandboxId::parse(format!(
                "s-{}",
                resource_name("sandbox", &admitted)
            ))
            .expect("the derived sandbox name is an opaque identifier"),
            disk_image: AcaConfiguredImageId::parse(format!(
                "d-{}",
                resource_name("disk", &admitted)
            ))
            .expect("the derived disk image name is a lower-kebab opaque identifier"),
        }
    }

    /// Return the cloud sandbox name this Guest owns.
    pub const fn sandbox_name(&self) -> &AcaSandboxId {
        &self.sandbox
    }

    /// Return the cloud disk image name this Guest owns.
    pub const fn disk_image_name(&self) -> &AcaConfiguredImageId {
        &self.disk_image
    }
}

/// The closed set of remote operations this provider performs.
///
/// The purpose decides the credential method, so a call that does not carry
/// material never asks for a delivery session and a call that does can never
/// borrow another purpose's class (R24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaRemotePurpose {
    /// Read the control plane: find sandboxes, probe health.
    Inspect,
    /// Create or adopt a disk image or sandbox.
    Ensure,
    /// Resume a suspended sandbox.
    Start,
    /// Stop a running sandbox.
    Stop,
    /// Delete a sandbox.
    Destroy,
}

impl AcaRemotePurpose {
    /// Whether this purpose can change remote state.
    pub const fn mutates_remote_state(self) -> bool {
        !matches!(self, Self::Inspect)
    }

    /// The credential service method this purpose runs under.
    pub const fn credential_method(self) -> CredentialMethod {
        match self {
            Self::Inspect => CredentialMethod::InspectMetadata,
            Self::Ensure | Self::Start | Self::Stop | Self::Destroy => {
                CredentialMethod::AcquireToken
            }
        }
    }

    /// Return the stable refusal-free code for this purpose.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Inspect => "aca-remote-inspect",
            Self::Ensure => "aca-remote-ensure",
            Self::Start => "aca-remote-start",
            Self::Stop => "aca-remote-stop",
            Self::Destroy => "aca-remote-destroy",
        }
    }
}

/// Closed, field-free refusals from the remote authority.
///
/// A refusal names a reason and nothing else: no audience, no reference, no
/// generation, no credential byte, and no cloud identifier (R42). The caller
/// learns that the operation was denied and which of the five checks denied
/// it, from the stable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaRemoteRefusal {
    /// The requested presentation is not one this remote backend realizes.
    PresentationUnsupported,
    /// The admitted Provider row is not this Provider.
    ProviderIdentityMismatch,
    /// The controller's Guest uid or generation is not the admitted one.
    GuestIdentityMismatch,
    /// The effect would address a different cloud account or environment.
    CloudIdentityMismatch,
    /// The presented method establishes no delivery session to admit.
    NoDeliverySession,
    /// The admitted credential relationship refused the live evidence.
    CredentialFenceRefused,
    /// The admitted session's audience or operation class is outside policy.
    CredentialPolicyRefused,
    /// The presented session is not the relationship's current one.
    CredentialSessionSuperseded,
    /// The admitted session's audience is not this control plane's audience.
    AudienceMismatch,
}

impl AcaRemoteRefusal {
    /// Return the closed, non-secret refusal code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PresentationUnsupported => "aca-remote-presentation-unsupported",
            Self::ProviderIdentityMismatch => "aca-remote-provider-identity-mismatch",
            Self::GuestIdentityMismatch => "aca-remote-guest-identity-mismatch",
            Self::CloudIdentityMismatch => "aca-remote-cloud-identity-mismatch",
            Self::NoDeliverySession => "aca-remote-no-delivery-session",
            Self::CredentialFenceRefused => "aca-remote-credential-fence-refused",
            Self::CredentialPolicyRefused => "aca-remote-credential-policy-refused",
            Self::CredentialSessionSuperseded => "aca-remote-credential-session-superseded",
            Self::AudienceMismatch => "aca-remote-audience-mismatch",
        }
    }

    /// Map the shared admitted-delivery gate's own refusal onto this module's
    /// closed vocabulary, keeping the gate's stage and reason distinguishable.
    const fn from_stage_and_reason(
        stage: AdmissionStage,
        reason: RefusalReason,
    ) -> Self {
        match (stage, reason) {
            (AdmissionStage::Prepare, _) => Self::NoDeliverySession,
            (AdmissionStage::Admit, RefusalReason::StaleAuthority) => Self::CredentialFenceRefused,
            (AdmissionStage::Admit, _) => Self::CredentialPolicyRefused,
            _ => Self::CredentialSessionSuperseded,
        }
    }
}

impl From<BindingRefusal> for AcaRemoteRefusal {
    fn from(refusal: BindingRefusal) -> Self {
        Self::from_stage_and_reason(refusal.stage(), refusal.reason())
    }
}

impl From<d2b_contracts_provider::v3::CredentialDeliveryAdmission> for AcaRemoteRefusal {
    fn from(admission: d2b_contracts_provider::v3::CredentialDeliveryAdmission) -> Self {
        Self::from_stage_and_reason(admission.stage(), admission.reason())
    }
}

impl fmt::Display for AcaRemoteRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AcaRemoteRefusal {}

/// The effective-access evidence one admitted remote call carries.
///
/// This is what "the control plane will act on exactly this" means for a
/// remote backend: the exact admitted Guest, at the exact admitted cloud
/// target, under the exact delivery session the graph currently authorizes,
/// for the cloud resource whose name this key pins. A caller cannot widen any
/// of it after the fact - every field is read from the authority and the
/// admitted session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaRemoteGrant {
    reconciliation: AcaReconciliationKey,
    guest_uid: ResourceUid,
    generation: u64,
    delivery_sequence: u64,
    operation_class: OperationClass,
}

impl AcaRemoteGrant {
    /// Return the deterministic cloud names this call may act on.
    pub const fn reconciliation(&self) -> &AcaReconciliationKey {
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
    ///
    /// The sequence is the monotonic part of the session identity: a
    /// superseded session has a strictly different value, so a caller that
    /// re-presents an old sequence is not the session the graph authorizes
    /// now.
    pub const fn delivery_sequence(&self) -> u64 {
        self.delivery_sequence
    }

    /// Return the admitted operation class this call runs under.
    pub const fn operation_class(&self) -> OperationClass {
        self.operation_class
    }
}

/// The admitted remote authority for one Guest in one cloud account.
///
/// Construct one from the graph's accepted identity and the `CredentialBinding`
/// relationship the source admitted; bind it onto the controller with
/// [`crate::AcaController::with_admitted_authority`].
pub struct AcaRemoteAuthority {
    guest: AcaAdmittedGuest,
    cloud: AcaCloudIdentity,
    audience: AudienceToken,
    credential: Arc<dyn AdmittedCredentialDelivery>,
}

impl AcaRemoteAuthority {
    /// Bind the authority to one admitted Guest, one cloud target, and one
    /// admitted credential relationship.
    ///
    /// # Errors
    ///
    /// Returns [`AcaRemoteRefusal::AudienceMismatch`] when the control-plane
    /// audience constant is not a bounded audience token. That is a defect in
    /// this crate rather than a runtime condition, so it is reported as a
    /// refusal rather than panicking.
    pub fn new(
        guest: AcaAdmittedGuest,
        cloud: AcaCloudIdentity,
        credential: Arc<dyn AdmittedCredentialDelivery>,
    ) -> Result<Self, AcaRemoteRefusal> {
        let audience = AudienceToken::parse(ACA_CONTROL_AUDIENCE)
            .map_err(|_| AcaRemoteRefusal::AudienceMismatch)?;
        Ok(Self {
            guest,
            cloud,
            audience,
            credential,
        })
    }

    /// Borrow the admitted Guest identity.
    pub const fn guest(&self) -> &AcaAdmittedGuest {
        &self.guest
    }

    /// Borrow the admitted cloud target.
    pub const fn cloud(&self) -> &AcaCloudIdentity {
        &self.cloud
    }

    /// Return the deterministic cloud names this Guest owns.
    pub fn reconciliation_key(&self) -> AcaReconciliationKey {
        AcaReconciliationKey::derive(&self.guest)
    }

    /// Admit one remote operation, or refuse it before any remote call.
    ///
    /// `binding` is the identity the controller is reconciling and
    /// `cloud` is the identity the effect will actually address, so a
    /// controller that drifted off its Guest or off its cloud account is
    /// refused here. `requested_presentation` is the consumer-facing
    /// presentation the request asked for; the ceiling comes from this
    /// Provider's own declaration, so a request cannot talk this remote
    /// backend into a local-style presentation.
    ///
    /// # Errors
    ///
    /// Returns the [`AcaRemoteRefusal`] of the first failed check, in the
    /// order documented on the module: presentation, Guest identity, cloud
    /// identity, credential relationship, audience.
    pub fn admit(
        &self,
        binding: &AcaResourceBinding,
        cloud: Option<&AcaCloudIdentity>,
        purpose: AcaRemotePurpose,
        requested_presentation: PresentationCapability,
        authorization: &CredentialAuthorization,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<AcaRemoteGrant, AcaRemoteRefusal> {
        if !crate::declared_presentation().realizes(requested_presentation) {
            return Err(AcaRemoteRefusal::PresentationUnsupported);
        }
        if binding.guest_uid != *self.guest.guest_uid()
            || binding.provider_generation != self.guest.generation()
        {
            return Err(AcaRemoteRefusal::GuestIdentityMismatch);
        }
        if cloud != Some(&self.cloud) {
            return Err(AcaRemoteRefusal::CloudIdentityMismatch);
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
            // A control-plane read carries no material, so there is no
            // delivery session to admit. Two things still have to hold: the
            // relationship's own fence must still accept the live evidence -
            // a revoked Credential stops a read as surely as it stops a write
            // - and a read that presents a delivery session is smuggling
            // material past the gate, so it is refused rather than
            // downgraded.
            if let Some(refusal) = self.credential.fence_refusal(evidence) {
                return Err(refusal.into());
            }
            if authorization.delivery_session_params().is_some() {
                return Err(AcaRemoteRefusal::NoDeliverySession);
            }
            None
        };
        if let Some(params) = session.as_ref()
            && params.audience() != &self.audience
        {
            return Err(AcaRemoteRefusal::AudienceMismatch);
        }
        Ok(AcaRemoteGrant {
            reconciliation: self.reconciliation_key(),
            guest_uid: self.guest.guest_uid().clone(),
            generation: self.guest.generation(),
            delivery_sequence: session.as_ref().map(DeliverySessionParams::sequence).unwrap_or(0),
            operation_class: method.operation_class(),
        })
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
pub struct AcaDeliveryContext {
    authorization: CredentialAuthorization,
    evidence: CredentialDeliveryEvidence,
}

impl AcaDeliveryContext {
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

impl fmt::Debug for AcaDeliveryContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AcaDeliveryContext(<redacted>)")
    }
}

/// The Credential service adapter this provider presents its operations to.
///
/// This is the seam the daemon composition root fills with the real
/// authenticated credential client. A Provider never opens an ambient
/// credential chain of its own, so this port is how an ARM token reaches a
/// Container Apps control-plane call at all.
#[async_trait::async_trait]
pub trait AcaRemoteDeliveryPort: Send + Sync {
    /// Authorize one remote operation, or refuse it without contacting the
    /// control plane.
    ///
    /// # Errors
    ///
    /// Returns the [`AcaRemoteRefusal`] the adapter reached when the graph no
    /// longer authorizes a delivery session for this operation.
    async fn authorize(
        &self,
        purpose: AcaRemotePurpose,
        operation_id: crate::effects::AcaOperationId,
    ) -> Result<AcaDeliveryContext, AcaRemoteRefusal>;
}

/// The admitted remote authority and the delivery port that feeds it.
///
/// This is what a controller holds once the graph has admitted one: the
/// authority answers "may this Guest act on this cloud", and the port answers
/// "which delivery session does the graph currently authorize for this
/// operation". Together they are the whole R34 attribution for one remote
/// call.
#[derive(Clone)]
pub struct AcaAdmittedRemote {
    authority: Arc<AcaRemoteAuthority>,
    delivery: Arc<dyn AcaRemoteDeliveryPort>,
}

impl AcaAdmittedRemote {
    /// Pair an admitted authority with its credential adapter.
    pub const fn new(
        authority: Arc<AcaRemoteAuthority>,
        delivery: Arc<dyn AcaRemoteDeliveryPort>,
    ) -> Self {
        Self {
            authority,
            delivery,
        }
    }

    /// Borrow the admitted authority.
    pub fn authority(&self) -> &Arc<AcaRemoteAuthority> {
        &self.authority
    }

    /// Ask the adapter for the authorized delivery session, then run the
    /// authority's five checks over it.
    ///
    /// The presentation check runs first and inside [`AcaRemoteAuthority`],
    /// so a request this remote backend cannot present is refused without the
    /// credential adapter being asked at all.
    ///
    /// # Errors
    ///
    /// Returns the [`AcaRemoteRefusal`] of the adapter or of the first failed
    /// authority check.
    pub async fn admit(
        &self,
        binding: &AcaResourceBinding,
        cloud: Option<&AcaCloudIdentity>,
        purpose: AcaRemotePurpose,
        requested_presentation: PresentationCapability,
        operation_id: crate::effects::AcaOperationId,
    ) -> Result<AcaRemoteGrant, AcaRemoteRefusal> {
        // The presentation ceiling is checked before the credential adapter
        // is asked, so a request this backend cannot realize is refused
        // without asking for material it will never use.
        if !crate::declared_presentation().realizes(requested_presentation) {
            return Err(AcaRemoteRefusal::PresentationUnsupported);
        }
        let delivery = self.delivery.authorize(purpose, operation_id).await?;
        self.authority.admit(
            binding,
            cloud,
            purpose,
            requested_presentation,
            delivery.authorization(),
            delivery.evidence(),
        )
    }
}

impl fmt::Debug for AcaAdmittedRemote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaAdmittedRemote")
            .field("authority", &self.authority)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AcaRemoteAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaRemoteAuthority")
            .field("guest", &self.guest)
            .field("cloud", &self.cloud)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

/// The terminal evidence one finalized Guest leaves behind.
///
/// Release is not complete when the remote delete was issued; it is complete
/// when the control plane reported the sandbox gone and the controller no
/// longer holds its finalizer. This value is that pair, and it is the only
/// thing a caller may report as "the Guest is gone" (R36, AE18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaReleaseEvidence {
    reconciliation: AcaReconciliationKey,
    guest_uid: ResourceUid,
    generation: u64,
    deletion: crate::effects::AcaDeleteOutcome,
    finalizer_released: bool,
}
impl AcaReleaseEvidence {
    /// Record the terminal state of one finalized Guest.
    pub const fn new(
        reconciliation: AcaReconciliationKey,
        guest_uid: ResourceUid,
        generation: u64,
        deletion: crate::effects::AcaDeleteOutcome,
        finalizer_released: bool,
    ) -> Self {
        Self {
            reconciliation,
            guest_uid,
            generation,
            deletion,
            finalizer_released,
        }
    }

    /// Return the deterministic cloud names the released Guest owned.
    pub const fn reconciliation(&self) -> &AcaReconciliationKey {
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

    /// Return the delete outcome the control plane reported.
    pub const fn deletion(&self) -> crate::effects::AcaDeleteOutcome {
        self.deletion
    }

    /// Whether the controller has dropped its finalizer.
    pub const fn finalizer_released(&self) -> bool {
        self.finalizer_released
    }

    /// Whether this evidence proves the remote Guest is gone.
    ///
    /// Both halves are required: a delete the control plane confirmed but a
    /// finalizer that is still installed is not a release, and a finalizer
    /// that was dropped without a confirmed delete is not one either.
    pub const fn is_terminal(&self) -> bool {
        self.finalizer_released
    }
}

/// Derive one lower-kebab cloud resource name from a domain tag and parts.
///
/// A 128-bit FNV-1a digest, domain separated so the sandbox and disk image of
/// the same Guest never collide, rendered in lower-kebab. The value only has
/// to be stable and collision-free enough to name a cloud resource; what
/// makes a retry safe is that the same admitted Guest always derives the same
/// name, not the width of the digest.
fn resource_name(domain: &str, parts: &[&[u8]]) -> String {
    const OFFSET: u128 = 0x6c62272e_07bb0142_62b82175_295c58dd;
    const PRIME: u128 = 0x00000000_01000193;
    let mut hash = OFFSET;
    let mut absorb = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    };
    absorb(domain.as_bytes());
    for part in parts {
        absorb(part);
    }
    let mut name = String::with_capacity(RESOURCE_NAME_DIGITS);
    for index in 0..RESOURCE_NAME_DIGITS {
        let symbol = ((hash >> (index * 5)) & 0x1f) as usize;
        name.push(char::from(DIGIT_ALPHABET[symbol]));
    }
    name
}

/// Twenty-six base-32 symbols render a 128-bit digest in opaque-id shape.
const RESOURCE_NAME_DIGITS: usize = 26;

const DIGIT_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
