//! The `CredentialBinding` realization: typed secret delivery under the
//! common binding lifecycle (KTD14).
//!
//! A `CredentialBinding` names one `Credential`, one admitted consumer
//! component, one stable consumer slot, the audience, the operation classes,
//! and the lifetime bounds. This module realizes that relationship without
//! letting any credential material reach the graph: the request is the
//! canonical desired state, the source's own policy decides it, and the
//! delivery session's existing private evidence
//! ([`DeliverySessionParams`](d2b_contracts_provider::v3::credential::DeliverySessionParams))
//! is minted only for an authority that is still current.
//!
//! Three properties hold by construction, and the crate's tests assert the
//! rendered bytes rather than trusting the types:
//!
//! - **No material leaves this module.** The authority is not serializable,
//!   its `Debug` redacts every identity field, and the status projection
//!   renders a closed, non-secret field set. A graph spec, a generic binding
//!   status, an audit record, and a publication snapshot therefore have
//!   nothing to leak: they carry identity, policy, and state only.
//! - **Prior authority cannot renew itself.** A minted session is fenced by
//!   the Credential generation, the consumer component generation, the
//!   Provider generation, the credential rotation generation, the admitted
//!   dependency revisions, the audience, the operation class, the hard
//!   deadline, and a monotonic replay sequence. Changing any of them makes
//!   the earlier session unusable (R24, R35, AE10) instead of letting a stale
//!   session keep working.
//! - **Revocation stays protocol-specific.** The generic lifecycle observes a
//!   *conservative* report derived from the existing
//!   [`CredentialRevocationReport`](crate::session::CredentialRevocationReport).
//!   An unconfirmed remote revoke can never project `Released`; it leaves the
//!   relationship outstanding so cleanup stays withheld.

use d2b_contracts_provider::v3::credential::{
    AudienceToken, CredentialMethod, DeliveryIdentity, DeliveryRouteDigest, DeliverySessionParams,
    OperationClass,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingAuthorization, BindingKey, BindingLifecycleState,
    BindingRealizationFacet, BindingRealizationSupport, BindingRefusal, BindingSpecFingerprint,
    CredentialBindingRequest, CredentialLifetime, CredentialOperation, FreshnessTuple, RefusalReason,
    ResourceGeneration, ResourceRef, ResourceUid, SourceAdmission, ZoneId, admit_binding_request,
};

use crate::driver::CredentialSourcePolicy;
use crate::session::CredentialRevocationReport;

/// A closed delivery refusal from the Credential binding realization.
///
/// The stage and reason are the contract's own vocabulary (R42): a caller
/// always sees which stage refused and why, and never a partial success
/// dressed up as a refusal of something else. The stable code names the
/// refusal and carries no credential reference, audience, or provider
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialDeliveryRefusal {
    stage: AdmissionStage,
    reason: RefusalReason,
}

impl CredentialDeliveryRefusal {
    const fn new(stage: AdmissionStage, reason: RefusalReason) -> Self {
        Self { stage, reason }
    }

    /// The stage that refused.
    pub const fn stage(&self) -> AdmissionStage {
        self.stage
    }

    /// The typed reason the stage refused.
    pub const fn reason(&self) -> RefusalReason {
        self.reason
    }

    /// The stable refusal code for this stage and reason.
    pub const fn code(&self) -> &'static str {
        match (self.stage, self.reason) {
            (AdmissionStage::Authorize, RefusalReason::IdentityNotAuthorized) => {
                "credential-delivery-identity-not-authorized"
            }
            (AdmissionStage::Authorize, RefusalReason::LimitExceedsCeiling) => {
                "credential-delivery-dependency-ceiling"
            }
            (AdmissionStage::Normalize, RefusalReason::ConflictingDeclaration) => {
                "credential-delivery-unnameable-relationship"
            }
            (AdmissionStage::Admit, RefusalReason::ConflictingDeclaration) => {
                "credential-delivery-conflicting-declaration"
            }
            (AdmissionStage::Admit, RefusalReason::SourcePolicyRefused) => {
                "credential-delivery-source-policy-refused"
            }
            (AdmissionStage::Admit, RefusalReason::StaleAuthority) => {
                "credential-delivery-stale-authority"
            }
            (AdmissionStage::Prepare, RefusalReason::MandatoryFacetUnsupported) => {
                "credential-delivery-facet-unsupported"
            }
            (AdmissionStage::Prepare, RefusalReason::SourcePolicyRefused) => {
                "credential-delivery-preparation-refused"
            }
            (AdmissionStage::Reserve, RefusalReason::UnprovenEffect) => {
                "credential-delivery-dependency-unproven"
            }
            (AdmissionStage::Activate, RefusalReason::StaleAuthority) => {
                "credential-delivery-activation-fenced"
            }
            (AdmissionStage::Revoke, RefusalReason::UnprovenEffect) => {
                "credential-delivery-revocation-unconfirmed"
            }
            (AdmissionStage::Drain, RefusalReason::UnprovenEffect) => {
                "credential-delivery-drain-unproven"
            }
            _ => "credential-delivery-refused",
        }
    }
}

impl core::fmt::Display for CredentialDeliveryRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for CredentialDeliveryRefusal {}

impl From<BindingRefusal> for CredentialDeliveryRefusal {
    fn from(refusal: BindingRefusal) -> Self {
        Self {
            stage: refusal.stage(),
            reason: refusal.reason(),
        }
    }
}

/// The service-operation class one desired-state operation class names.
///
/// The mapping is total because [`CredentialOperation`] is defined as exactly
/// the three delivery classes. The two non-delivery service operations
/// (`revoke-token`, `inspect-metadata`) are protocol operations with no
/// delivery session and therefore no `CredentialBinding` at all.
const fn operation_class(operation: CredentialOperation) -> OperationClass {
    match operation {
        CredentialOperation::AcquireToken => OperationClass::AcquireToken,
        CredentialOperation::RefreshToken => OperationClass::RefreshToken,
        CredentialOperation::SignChallenge => OperationClass::SignChallenge,
    }
}

/// The service method one admitted operation class dispatches.
const fn delivery_method(operation: CredentialOperation) -> CredentialMethod {
    match operation {
        CredentialOperation::AcquireToken => CredentialMethod::AcquireToken,
        CredentialOperation::RefreshToken => CredentialMethod::RefreshToken,
        CredentialOperation::SignChallenge => CredentialMethod::SignChallenge,
    }
}

/// The Credential family's declared binding realization support.
///
/// Exactly one facet: credential material is delivered inside an admitted
/// delivery session. A `CredentialBinding` request that depends on any other
/// presentation facet is refused at admission rather than approximated.
pub fn credential_binding_support() -> BindingRealizationSupport {
    BindingRealizationSupport::new(vec![BindingRealizationFacet::CredentialDelivery])
        .expect("one declared binding facet is unique by construction")
}

/// The exact source-side identity one delivery authority is fenced against.
///
/// Every field is a fence: minting compares the live evidence against it, so
/// a changed Credential generation, consumer component generation, Provider
/// generation, or credential rotation generation invalidates the earlier
/// authority instead of renewing it. None of these fields is credential
/// material; they are identities, counters, and bounds.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialDeliveryFence {
    credential_ref: ResourceRef,
    credential_uid: ResourceUid,
    credential_generation: ResourceGeneration,
    consumer_provider_ref: ResourceRef,
    consumer_component_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    rotation_generation: u64,
    audience: AudienceToken,
    expiry_unix_ms: u64,
    deadline_unix_ms: u64,
    route_digest: DeliveryRouteDigest,
    max_token_bytes: u32,
}

impl core::fmt::Debug for CredentialDeliveryFence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialDeliveryFence")
            .field("credential_ref", &"<redacted>")
            .field("credential_uid", &"<redacted>")
            .field("credential_generation", &self.credential_generation)
            .field("consumer_provider_ref", &"<redacted>")
            .field("consumer_component_generation", &self.consumer_component_generation)
            .field("provider_generation", &self.provider_generation)
            .field("rotation_generation", &self.rotation_generation)
            .field("audience", &"<redacted>")
            .field("expiry_unix_ms", &self.expiry_unix_ms)
            .field("deadline_unix_ms", &self.deadline_unix_ms)
            .field("route_digest", &"<redacted>")
            .field("max_token_bytes", &self.max_token_bytes)
            .finish()
    }
}

/// The live evidence one delivery mint is checked against.
///
/// The caller supplies what the source side currently reports, and the mint
/// fails closed when anything moved (R35, R41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialDeliveryEvidence {
    credential_generation: ResourceGeneration,
    consumer_component_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    rotation_generation: u64,
    dependencies: Vec<FreshnessTuple>,
    now_unix_ms: u64,
}

impl CredentialDeliveryEvidence {
    /// Construct the evidence one observation carries.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        credential_generation: ResourceGeneration,
        consumer_component_generation: ResourceGeneration,
        provider_generation: ResourceGeneration,
        rotation_generation: u64,
        dependencies: Vec<FreshnessTuple>,
        now_unix_ms: u64,
    ) -> Self {
        Self {
            credential_generation,
            consumer_component_generation,
            provider_generation,
            rotation_generation,
            dependencies,
            now_unix_ms,
        }
    }
}

/// The source controller's own inputs for admitting one `CredentialBinding`
/// against one of its `Credential` rows.
///
/// Everything here comes from the source side: the committed row
/// identities, the policy read from the `Credential` spec, the current
/// generations, and the observation clock. The consumer contributes only the
/// request.
pub struct CredentialBindingAdmission {
    zone: ZoneId,
    source_uid: ResourceUid,
    consumer_uid: ResourceUid,
    source_policy: CredentialSourcePolicy,
    policy_fingerprint: BindingSpecFingerprint,
    authorization: BindingAuthorization,
    source: SourceAdmission,
    support: BindingRealizationSupport,
    dependencies: Vec<FreshnessTuple>,
    credential_ref: ResourceRef,
    credential_generation: ResourceGeneration,
    consumer_provider_ref: ResourceRef,
    consumer_component_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    rotation_generation: u64,
    audience: AudienceToken,
    route_digest: DeliveryRouteDigest,
    max_token_bytes: u32,
    now_unix_ms: u64,
}

impl CredentialBindingAdmission {
    /// Construct the source controller's admission inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
        source_policy: CredentialSourcePolicy,
        policy_fingerprint: BindingSpecFingerprint,
        authorization: BindingAuthorization,
        source: SourceAdmission,
        support: BindingRealizationSupport,
        dependencies: Vec<FreshnessTuple>,
        credential_ref: ResourceRef,
        credential_generation: ResourceGeneration,
        consumer_provider_ref: ResourceRef,
        consumer_component_generation: ResourceGeneration,
        provider_generation: ResourceGeneration,
        rotation_generation: u64,
        audience: AudienceToken,
        route_digest: DeliveryRouteDigest,
        max_token_bytes: u32,
        now_unix_ms: u64,
    ) -> Self {
        Self {
            zone,
            source_uid,
            consumer_uid,
            source_policy,
            policy_fingerprint,
            authorization,
            source,
            support,
            dependencies,
            credential_ref,
            credential_generation,
            consumer_provider_ref,
            consumer_component_generation,
            provider_generation,
            rotation_generation,
            audience,
            route_digest,
            max_token_bytes,
            now_unix_ms,
        }
    }
}

/// One admitted credential delivery relationship.
///
/// The value is the binding's private authority. It is deliberately not
/// serializable and it is constructed only from an admitted
/// [`CredentialBindingRequest`] plus the source's own policy. The only thing
/// it hands out is a
/// [`DeliverySessionParams`](d2b_contracts_provider::v3::credential::DeliverySessionParams)
/// for a mint that is still current, or a typed refusal.
pub struct CredentialDeliveryAuthority {
    request: CredentialBindingRequest,
    admission: BindingAdmission,
    policy_fingerprint: BindingSpecFingerprint,
    audience: AudienceToken,
    fence: CredentialDeliveryFence,
    lifetime: CredentialLifetime,
    operations: Vec<OperationClass>,
    /// The replay sequence of the most recently minted session; `0` before
    /// the first mint. A leg or a presented session carrying anything else is
    /// stale.
    current_sequence: u64,
    /// The sequence the next mint will carry.
    next_sequence: u64,
    /// Whether new use is still admitted. Revocation and drain only ever clear
    /// this; nothing in the mint path sets it.
    admits_new_use: bool,
}

impl core::fmt::Debug for CredentialDeliveryAuthority {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialDeliveryAuthority")
            .field("request", &self.request)
            .field("key", &self.admission.key())
            .field("policy_fingerprint", &self.policy_fingerprint)
            .field("fence", &self.fence)
            .field("current_sequence", &self.current_sequence)
            .field("next_sequence", &self.next_sequence)
            .field("admits_new_use", &self.admits_new_use)
            .finish()
    }
}

impl CredentialDeliveryAuthority {
    /// Admit one consumer's `CredentialBinding` request for one `Credential`
    /// row and build the fence its delivery sessions hang off.
    ///
    /// Admission is ordered and stops at the first refusal: the generic
    /// evaluator runs first (authorization, source decision, realization
    /// support, dependency freshness), then the source's own policy binds the
    /// request's audience, operation set, and lifetime bounds, and only then
    /// is the fence built. A well-formed request outside the source policy is
    /// refused here rather than at the provider.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialDeliveryRefusal`] when the subject is not
    /// authorized, the source's decision is for another relationship, the
    /// realization does not declare credential delivery, a dependency
    /// revision moved, or the request's audience, operations, or lifetime fall
    /// outside the source policy.
    pub fn admit(
        request: CredentialBindingRequest,
        inputs: CredentialBindingAdmission,
    ) -> Result<Self, CredentialDeliveryRefusal> {
        let key = request
            .key(inputs.zone, inputs.source_uid.clone(), inputs.consumer_uid.clone())
            .map_err(|_| {
                CredentialDeliveryRefusal::new(
                    AdmissionStage::Normalize,
                    RefusalReason::ConflictingDeclaration,
                )
            })?;
        let admission = admit_binding_request(
            &key,
            request.requested_rights(),
            request.required_facets(),
            &inputs.authorization,
            &inputs.source,
            &inputs.support,
            &inputs.dependencies,
        )?;
        // The source policy is the `Credential` row's own committed statement
        // of who may use it, for what, and for how long. A request cannot
        // widen it: the audience must be the row's audience, every requested
        // operation must be in the row's admitted set, and the requested
        // lifetime must fit inside the row's ceiling.
        inputs
            .source_policy
            .admits_delivery(&request, &inputs.audience)
            .map_err(CredentialDeliveryRefusal::from)?;
        if inputs.credential_ref != *request.source_ref() || inputs.rotation_generation == 0 {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        let lifetime = request.lifetime().clone();
        let fence = CredentialDeliveryFence {
            credential_ref: inputs.credential_ref,
            credential_uid: inputs.source_uid,
            credential_generation: inputs.credential_generation,
            consumer_provider_ref: inputs.consumer_provider_ref,
            consumer_component_generation: inputs.consumer_component_generation,
            provider_generation: inputs.provider_generation,
            rotation_generation: inputs.rotation_generation,
            audience: inputs.audience,
            expiry_unix_ms: inputs
                .now_unix_ms
                .saturating_add(lifetime.expires_in().as_millis()),
            deadline_unix_ms: inputs
                .now_unix_ms
                .saturating_add(lifetime.valid_for().as_millis()),
            route_digest: inputs.route_digest,
            max_token_bytes: inputs.max_token_bytes,
        };
        if fence.deadline_unix_ms > fence.expiry_unix_ms || fence.max_token_bytes == 0 {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Prepare,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        let operations = request
            .operations()
            .iter()
            .copied()
            .map(operation_class)
            .collect();
        Ok(Self {
            request,
            admission,
            policy_fingerprint: inputs.policy_fingerprint,
            audience: fence.audience.clone(),
            fence,
            lifetime,
            operations,
            current_sequence: 0,
            next_sequence: 1,
            admits_new_use: true,
        })
    }

    /// The admitted relationship's KTD3 key.
    pub const fn key(&self) -> &BindingKey {
        self.admission.key()
    }

    /// The exact desired request this authority realizes.
    pub const fn request(&self) -> &CredentialBindingRequest {
        &self.request
    }

    /// The digest of the `Credential` row's own spec at admission.
    ///
    /// It is non-secret freshness data: a change here means the source policy
    /// this authority was read from is no longer the committed one, so the
    /// authority is not renewed.
    pub const fn policy_fingerprint(&self) -> &BindingSpecFingerprint {
        &self.policy_fingerprint
    }

    /// The admitted audience.
    pub const fn audience(&self) -> &AudienceToken {
        &self.audience
    }

    /// The absolute expiry the source policy bounds this delivery to.
    pub const fn expiry_unix_ms(&self) -> u64 {
        self.fence.expiry_unix_ms
    }

    /// The hard deadline the source policy bounds this delivery to.
    pub const fn deadline_unix_ms(&self) -> u64 {
        self.fence.deadline_unix_ms
    }

    /// The requested lifetime bounds the admission bound.
    pub const fn lifetime(&self) -> &CredentialLifetime {
        &self.lifetime
    }

    /// Whether this relationship still admits new use.
    pub const fn admits_new_use(&self) -> bool {
        self.admits_new_use
    }

    /// The replay sequence of the most recently minted session; `0` before
    /// the first mint.
    pub const fn current_sequence(&self) -> u64 {
        self.current_sequence
    }

    /// The exact non-secret identity the current session is authorized under.
    pub fn delivery_identity(&self) -> DeliveryIdentity {
        DeliveryIdentity::new(
            self.fence.credential_ref.clone(),
            self.fence.credential_uid.clone(),
            self.fence.credential_generation,
            self.fence.consumer_provider_ref.clone(),
            self.fence.consumer_component_generation,
            self.audience.clone(),
            operation_class(self.admitted_operation()),
            self.current_sequence,
        )
    }

    /// Whether the requested operation class is one this relationship admits.
    pub fn admits_operation(&self, operation: CredentialOperation) -> bool {
        self.operations.contains(&operation_class(operation))
    }

    /// Mint the private delivery evidence for one admitted operation class.
    ///
    /// The mint is the only path that can ask a provider for credential
    /// material, and it succeeds only while every fence still matches: the
    /// Credential generation, the consumer component generation, the Provider
    /// generation, the credential rotation generation, the admitted
    /// dependency revisions, the hard deadline, and the absolute expiry. The
    /// operation must be one the request admitted, so a binding that admits
    /// only token acquisition cannot mint a refresh or a signature.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialDeliveryRefusal`] when new use is no longer
    /// admitted, the operation is outside the admitted set, the evidence
    /// moved, or the lifetime elapsed.
    pub fn mint(
        &mut self,
        operation: CredentialOperation,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<DeliverySessionParams, CredentialDeliveryRefusal> {
        self.check_fence(evidence)?;
        if !self.admits_operation(operation) {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        let params = self.build_params(operation_class(operation), self.next_sequence)?;
        self.current_sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(params)
    }

    /// Open one bounded consumer or helper leg over the current session.
    ///
    /// A leg is an attenuation of this authority, never a second one: it
    /// carries the sequence of the session it rides and can never outlive the
    /// relationship's own absolute expiry, so a helper cannot hold a delivery
    /// authority the consumer no longer has.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialDeliveryRefusal`] when no session has been minted
    /// yet, the leg identity is not a bounded token, or the evidence moved.
    pub fn open_leg(
        &self,
        leg: impl Into<String>,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<CredentialDeliveryLeg, CredentialDeliveryRefusal> {
        self.check_fence(evidence)?;
        if self.current_sequence == 0 {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Activate,
                RefusalReason::StaleAuthority,
            ));
        }
        let leg = BoundedToken::parse(leg.into()).map_err(|_| {
            CredentialDeliveryRefusal::new(
                AdmissionStage::Activate,
                RefusalReason::SourcePolicyRefused,
            )
        })?;
        Ok(CredentialDeliveryLeg {
            leg,
            sequence: self.current_sequence,
            expires_at_unix_ms: self.fence.expiry_unix_ms,
        })
    }

    /// Renew delivery through one leg.
    ///
    /// Renewal succeeds only when the leg rides the *current* session, is
    /// itself unexpired, and every fence still holds. A leg issued against an
    /// earlier session is stale: its sequence no longer matches, so the
    /// renewal is refused at activation and the consumer must be admitted
    /// again. An expired leg is refused the same way, which is what stops an
    /// expired delivery from being renewed through a helper leg that happened
    /// to outlive it.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialDeliveryRefusal`] when the leg is stale or
    /// expired, the operation is outside the admitted set, the evidence moved,
    /// or the lifetime elapsed.
    pub fn renew(
        &mut self,
        operation: CredentialOperation,
        leg: &CredentialDeliveryLeg,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<DeliverySessionParams, CredentialDeliveryRefusal> {
        if leg.sequence != self.current_sequence
            || leg.expires_at_unix_ms <= evidence.now_unix_ms
            || leg.expires_at_unix_ms > self.fence.expiry_unix_ms
        {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Activate,
                RefusalReason::StaleAuthority,
            ));
        }
        self.mint(operation, evidence)
    }

    /// Whether the presented identity is the one this authority issued.
    ///
    /// A session authorized under a different Credential generation, consumer
    /// component generation, audience, operation class, or replay sequence is
    /// a different authority: presenting it against this relationship fails
    /// rather than renewing the earlier delivery.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialDeliveryRefusal`] when any authority-bearing field
    /// of the presented identity differs.
    pub fn verifies(
        &self,
        presented: &DeliveryIdentity,
    ) -> Result<(), CredentialDeliveryRefusal> {
        if presented == &self.delivery_identity() {
            Ok(())
        } else {
            Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Activate,
                RefusalReason::StaleAuthority,
            ))
        }
    }

    /// Block new use without touching the outstanding session.
    ///
    /// This is the generic pre-drain edge (KTD10): the protocol's own
    /// revocation is still the only thing that retires the lease, but the
    /// relationship stops admitting new deliveries the moment this runs.
    pub fn fence_new_use(&mut self) {
        self.admits_new_use = false;
    }

    /// Project one revocation attempt into the conservative lifecycle state.
    ///
    /// The generic state never claims more than the protocol proved. An
    /// unconfirmed or unproven revoke leaves the relationship outstanding, so
    /// cleanup stays withheld, and it can never be reported as `Released`;
    /// only a confirmed revocation with no outstanding use reaches `Released`.
    pub const fn observe_revocation(
        &self,
        report: CredentialRevocationReport,
        outstanding_use: bool,
    ) -> BindingLifecycleState {
        if !self.admits_new_use {
            return BindingLifecycleState::Draining;
        }
        match report {
            CredentialRevocationReport::Unconfirmed => BindingLifecycleState::Degraded,
            CredentialRevocationReport::Retained => BindingLifecycleState::Revoking,
            CredentialRevocationReport::Released => {
                if outstanding_use {
                    BindingLifecycleState::Revoking
                } else {
                    BindingLifecycleState::Released
                }
            }
        }
    }

    /// The generic, non-secret status projection for this relationship.
    pub fn status(&self, report: Option<CredentialRevocationReport>) -> CredentialBindingStatus {
        let state = match report {
            Some(report) => self.observe_revocation(report, false),
            None if self.admits_new_use => BindingLifecycleState::Active,
            None => BindingLifecycleState::Revoking,
        };
        CredentialBindingStatus {
            state,
            expiry_unix_ms: self.fence.expiry_unix_ms,
            deadline_unix_ms: self.fence.deadline_unix_ms,
            sequence: self.current_sequence,
        }
    }

    /// The single operation this relationship admits, for the identity.
    ///
    /// A `CredentialBinding` admits an operation *set*; the session
    /// identity names the class the presented session was authorized for, so
    /// the comparison is against the first admitted class. Presenting a
    /// session for a different class is rejected by the contract's own
    /// method/authorization check, and by [`Self::admits_operation`] here.
    fn admitted_operation(&self) -> CredentialOperation {
        self.request
            .operations()
            .first()
            .copied()
            .unwrap_or(CredentialOperation::AcquireToken)
    }

    /// Build the private delivery evidence for one operation class.
    fn build_params(
        &self,
        operation: OperationClass,
        sequence: u64,
    ) -> Result<DeliverySessionParams, CredentialDeliveryRefusal> {
        DeliverySessionParams::new(
            self.fence.credential_ref.clone(),
            self.fence.credential_uid.clone(),
            self.fence.credential_generation,
            self.fence.consumer_provider_ref.clone(),
            self.fence.consumer_component_generation,
            self.audience.clone(),
            operation,
            self.fence.expiry_unix_ms,
            self.fence.deadline_unix_ms,
            self.fence.route_digest.clone(),
            self.fence.max_token_bytes,
            sequence,
        )
        .map_err(|_| {
            CredentialDeliveryRefusal::new(
                AdmissionStage::Prepare,
                RefusalReason::SourcePolicyRefused,
            )
        })
    }

    /// Fail closed unless the live evidence still matches every fence.
    fn check_fence(
        &self,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<(), CredentialDeliveryRefusal> {
        if !self.admits_new_use {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Revoke,
                RefusalReason::UnprovenEffect,
            ));
        }
        if evidence.credential_generation != self.fence.credential_generation
            || evidence.consumer_component_generation != self.fence.consumer_component_generation
            || evidence.provider_generation != self.fence.provider_generation
            || evidence.rotation_generation != self.fence.rotation_generation
        {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            ));
        }
        if !self.admission.is_current(&evidence.dependencies) {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            ));
        }
        if evidence.now_unix_ms >= self.fence.deadline_unix_ms
            || evidence.now_unix_ms >= self.fence.expiry_unix_ms
        {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Drain,
                RefusalReason::UnprovenEffect,
            ));
        }
        Ok(())
    }

    /// The service method one admitted operation class dispatches.
    ///
    /// The method is derived from the admitted operation, never supplied, so
    /// a delivery session is always established for one exact method.
    pub fn method_for(&self, operation: CredentialOperation) -> Option<CredentialMethod> {
        self.admits_operation(operation)
            .then(|| delivery_method(operation))
    }
}

/// One bounded consumer or helper leg over an admitted delivery.
///
/// A leg is an attenuation: it names the session it rides and can never
/// outlive the relationship. It carries no credential material and no
/// transcript.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialDeliveryLeg {
    leg: BoundedToken,
    sequence: u64,
    expires_at_unix_ms: u64,
}

impl core::fmt::Debug for CredentialDeliveryLeg {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialDeliveryLeg")
            .field("leg", &"<redacted>")
            .field("sequence", &self.sequence)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

impl CredentialDeliveryLeg {
    /// The leg's own identity token.
    pub fn as_str(&self) -> &str {
        self.leg.as_str()
    }

    /// The delivery sequence this leg rides.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The absolute expiry this leg can never outlive.
    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

/// The closed state codes the generic status publishes.
const fn state_code(state: BindingLifecycleState) -> &'static str {
    match state {
        BindingLifecycleState::Requested => "requested",
        BindingLifecycleState::Admitted => "admitted",
        BindingLifecycleState::Prepared => "prepared",
        BindingLifecycleState::Active => "active",
        BindingLifecycleState::Revoking => "revoking",
        BindingLifecycleState::Draining => "draining",
        BindingLifecycleState::Released => "released",
        BindingLifecycleState::Refused => "refused",
        BindingLifecycleState::Degraded => "degraded",
        BindingLifecycleState::Unknown => "unknown",
    }
}

/// The generic binding status for one credential delivery relationship.
///
/// The value is the *only* thing this module publishes about a live
/// relationship. Every field is a bound, a counter, or a state, so the
/// rendered form below is what an audit record, a publication snapshot, or a
/// status API answers, and it has no field a credential byte could occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialBindingStatus {
    state: BindingLifecycleState,
    expiry_unix_ms: u64,
    deadline_unix_ms: u64,
    sequence: u64,
}

impl CredentialBindingStatus {
    /// The observed generic lifecycle state.
    pub const fn state(&self) -> BindingLifecycleState {
        self.state
    }

    /// The absolute expiry the delivery authority is bounded to.
    pub const fn expiry_unix_ms(&self) -> u64 {
        self.expiry_unix_ms
    }

    /// The hard deadline the delivery authority is bounded to.
    pub const fn deadline_unix_ms(&self) -> u64 {
        self.deadline_unix_ms
    }

    /// The replay sequence the current session carries.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The rendered non-secret status object.
    ///
    /// The field set is closed and deliberately minimal: the observed state
    /// code, the two lifetime bounds, and the replay counter. The audience,
    /// the credential identity, the route digest, and the lease handle are
    /// absent by construction, and no field exists that could carry material.
    pub fn render(&self) -> serde_json::Value {
        serde_json::json!({
            "state": state_code(self.state),
            "expiryUnixMs": self.expiry_unix_ms,
            "deadlineUnixMs": self.deadline_unix_ms,
            "sequence": self.sequence,
        })
    }
}
