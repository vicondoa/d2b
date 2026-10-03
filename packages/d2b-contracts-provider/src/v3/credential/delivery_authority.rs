//! The one admitted `CredentialBinding` delivery path.
//!
//! A `Credential` Provider that delivers material has exactly one question to
//! answer before it may ask its client for anything: *is the delivery session
//! I am about to hand back the one this admitted relationship currently
//! authorizes?* Three separate Credential Provider implementations used to
//! answer that from the authenticated route alone - each with its own
//! comparison order, its own notion of a matching consumer, and its own
//! sequence counter - so a relationship the graph had already moved past could
//! still be served by whichever backend compared the fewest fields.
//!
//! This module is the shared answer. The relationship is reached only through
//! [`AdmittedCredentialDelivery`], which is a port: nothing here builds an
//! authority, and the source-side realization of `CredentialBinding` supplies
//! the one implementation. The function [`admit_credential_delivery`] is the
//! single gate, and it checks in one fixed order:
//!
//! 1. the method establishes a delivery session at all;
//! 2. the relationship's own fence still accepts the live evidence, so a
//!    changed `Credential` generation, consumer component generation, Provider
//!    generation, rotation generation, dependency revision, deadline, or
//!    expiry refuses before the client is asked (R35, R41);
//! 3. the presented session's audience is the relationship's audience, so a
//!    refresh cannot widen it (R24);
//! 4. the presented session's operation class is one the relationship grants,
//!    so a refresh cannot widen the allowed operations (R24);
//! 5. the presented session is the relationship's *current* delivery session,
//!    compared on every authority-bearing field including the replay sequence,
//!    so a superseded session or a replaced consumer is refused rather than
//!    renewed (R24, R35, R41, AE10).
//!
//! Every refusal is a stage and a reason from the shared admission vocabulary
//! plus a closed, field-free code, so an operator can tell a policy reduction
//! from a stale authority without the refusal ever carrying an audience, a
//! reference, a route digest, or any credential byte (R42). Nothing here is
//! serializable and nothing here carries material.

use d2b_contracts_resource::v3::{AdmissionStage, BindingRefusal, RefusalReason, ResourceGeneration};

use super::{
    AudienceToken, CredentialAuthorization, CredentialMethod, CredentialServiceError,
    CredentialServiceErrorCode, DeliveryIdentity, DeliverySessionParams, OperationClass,
};

/// Live, non-secret observations one Provider reads before it asks a client
/// for material.
///
/// Every field is an identity or a counter: the `Credential` generation the
/// source reported, the consumer component generation and Provider generation
/// the authenticated session carries, and the observation clock. None of them
/// is credential material, so the evidence may be logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialDeliveryEvidence {
    credential_generation: ResourceGeneration,
    consumer_component_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    now_unix_ms: u64,
}

impl CredentialDeliveryEvidence {
    /// Construct one observation set.
    pub const fn new(
        credential_generation: ResourceGeneration,
        consumer_component_generation: ResourceGeneration,
        provider_generation: ResourceGeneration,
        now_unix_ms: u64,
    ) -> Self {
        Self {
            credential_generation,
            consumer_component_generation,
            provider_generation,
            now_unix_ms,
        }
    }

    /// Return the `Credential` generation the source reported.
    pub const fn credential_generation(&self) -> ResourceGeneration {
        self.credential_generation
    }

    /// Return the consumer component generation the session carries.
    pub const fn consumer_component_generation(&self) -> ResourceGeneration {
        self.consumer_component_generation
    }

    /// Return the Provider generation the session carries.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the observation clock, in Unix milliseconds.
    pub const fn now_unix_ms(&self) -> u64 {
        self.now_unix_ms
    }
}

/// The admitted `CredentialBinding` relationship a Provider delivers under.
///
/// The trait is a port and deliberately has no constructor. An implementation
/// is the source side's own admitted authority: it answers only from state the
/// source committed, never from the session being admitted.
pub trait AdmittedCredentialDelivery: Send + Sync {
    /// The audience the admitted relationship is scoped to.
    fn audience(&self) -> &AudienceToken;

    /// Whether the admitted relationship grants `class`.
    fn grants(&self, class: OperationClass) -> bool;

    /// The exact identity of the relationship's current delivery session.
    fn current_delivery(&self) -> DeliveryIdentity;

    /// The stage-and-reason refusal when the live evidence no longer matches
    /// the relationship's fence, and `None` when it still does.
    ///
    /// The relationship owns the fence, so the refusal names the stage and
    /// reason that relationship reached: an evidence revision that moved is not
    /// the same answer as a lifetime that elapsed.
    fn fence_refusal(&self, evidence: &CredentialDeliveryEvidence) -> Option<BindingRefusal>;
}

/// One closed refusal from the admitted-delivery gate.
///
/// The value carries a stage, a reason, and a stable code, and nothing else:
/// no audience, no reference, no route digest, no generation, and no
/// credential byte (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialDeliveryAdmission {
    stage: AdmissionStage,
    reason: RefusalReason,
}

impl CredentialDeliveryAdmission {
    const fn new(stage: AdmissionStage, reason: RefusalReason) -> Self {
        Self { stage, reason }
    }

    /// The stage that refused.
    pub const fn stage(&self) -> AdmissionStage {
        self.stage
    }

    /// The reason that stage refused.
    pub const fn reason(&self) -> RefusalReason {
        self.reason
    }

    /// The closed, non-secret refusal code.
    pub const fn code(&self) -> &'static str {
        match (self.stage, self.reason) {
            (AdmissionStage::Prepare, RefusalReason::MandatoryFacetUnsupported) => {
                "credential-delivery-no-delivery-session"
            }
            (AdmissionStage::Admit, RefusalReason::SourcePolicyRefused) => {
                "credential-delivery-outside-admitted-policy"
            }
            (AdmissionStage::Admit, RefusalReason::StaleAuthority) => {
                "credential-delivery-stale-authority"
            }
            (AdmissionStage::Activate, RefusalReason::StaleAuthority) => {
                "credential-delivery-session-superseded"
            }
            (AdmissionStage::Revoke, RefusalReason::UnprovenEffect) => {
                "credential-delivery-new-use-blocked"
            }
            (AdmissionStage::Drain, RefusalReason::UnprovenEffect) => {
                "credential-delivery-lifetime-elapsed"
            }
            _ => "credential-delivery-refused",
        }
    }

    /// The closed service error this refusal travels as on the wire.
    ///
    /// The wire vocabulary is the Credential service's own: a caller learns
    /// that the operation was denied, and the stage and reason stay in the
    /// non-secret diagnostic above it.
    pub const fn service_error(&self) -> CredentialServiceError {
        CredentialServiceError::new(CredentialServiceErrorCode::OperationDenied)
    }
}

impl core::fmt::Display for CredentialDeliveryAdmission {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for CredentialDeliveryAdmission {}

impl From<BindingRefusal> for CredentialDeliveryAdmission {
    fn from(refusal: BindingRefusal) -> Self {
        Self::new(refusal.stage(), refusal.reason())
    }
}

/// Read the live evidence one authorization and observation clock carry.
///
/// The credential and consumer generations come from the presented delivery
/// session and the Provider generation from the authenticated session, so a
/// caller cannot supply evidence the session it is admitting does not itself
/// carry.
///
/// # Errors
///
/// Returns [`CredentialServiceErrorCode::OperationDenied`] when the
/// authorization establishes no delivery session, or when the authenticated
/// session carries no Provider generation.
pub fn observed_delivery_evidence(
    authorization: &CredentialAuthorization,
    now_unix_ms: u64,
) -> Result<CredentialDeliveryEvidence, CredentialServiceError> {
    let denied = || CredentialServiceError::new(CredentialServiceErrorCode::OperationDenied);
    let delivery = authorization.delivery_session_params().ok_or_else(denied)?;
    let provider_generation = authorization
        .authenticated_session()
        .and_then(|session| session.authenticated_subject().provider_generation())
        .ok_or_else(denied)?;
    Ok(CredentialDeliveryEvidence::new(
        delivery.credential_generation(),
        delivery.consumer_component_generation(),
        provider_generation,
        now_unix_ms,
    ))
}

/// Admit one delivery-bearing method against an admitted `CredentialBinding`
/// relationship, and return the delivery session it authorizes.
///
/// This is the one gate every Credential Provider realization runs before it
/// may ask its client for material. It is ordered and it stops at the first
/// refusal; the checks and what each one closes are described on the module.
/// The returned value is the *existing* delivery session, so nothing here
/// invents a session shape or a second sequence counter.
///
/// # Errors
///
/// Returns [`CredentialDeliveryAdmission`] when the method establishes no
/// delivery session, the relationship's fence no longer accepts the live
/// evidence, the presented session's audience or operation class is outside
/// the admitted policy, or the presented session is not the relationship's
/// current delivery session.
pub fn admit_credential_delivery(
    authorization: &CredentialAuthorization,
    method: CredentialMethod,
    relationship: &dyn AdmittedCredentialDelivery,
    evidence: &CredentialDeliveryEvidence,
) -> Result<DeliverySessionParams, CredentialDeliveryAdmission> {
    let no_session = || {
        CredentialDeliveryAdmission::new(
            AdmissionStage::Prepare,
            RefusalReason::MandatoryFacetUnsupported,
        )
    };
    if !method.requires_delivery() {
        return Err(no_session());
    }
    let params = authorization
        .delivery_session_params()
        .ok_or_else(no_session)?
        .clone();
    if let Some(refusal) = relationship.fence_refusal(evidence) {
        return Err(CredentialDeliveryAdmission::from(refusal));
    }
    let presented = params.delivery_identity();
    if presented.operation_class() != method.operation_class()
        || presented.audience() != relationship.audience()
        || !relationship.grants(presented.operation_class())
    {
        return Err(CredentialDeliveryAdmission::new(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    if !same_session(&presented, &relationship.current_delivery()) {
        return Err(CredentialDeliveryAdmission::new(
            AdmissionStage::Activate,
            RefusalReason::StaleAuthority,
        ));
    }
    Ok(params)
}

/// Whether the presented session is the admitted relationship's current one.
///
/// Every authority-bearing field is compared, including the replay sequence, so
/// a superseded session and a session issued for a replaced consumer are the
/// same answer: refused. The audience and operation class are compared by the
/// caller instead, against the relationship's policy rather than against the
/// session it issued, which is what stops a refresh from widening either one.
fn same_session(presented: &DeliveryIdentity, admitted: &DeliveryIdentity) -> bool {
    presented.credential_ref() == admitted.credential_ref()
        && presented.credential_uid() == admitted.credential_uid()
        && presented.credential_generation() == admitted.credential_generation()
        && presented.consumer_provider_ref() == admitted.consumer_provider_ref()
        && presented.consumer_component_generation() == admitted.consumer_component_generation()
        && presented.sequence() == admitted.sequence()
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        AdmissionStage, BindingRefusal, RefusalReason, ResourceRef, ResourceUid,
    };
    use crate::v3::credential::{DeliveryRouteDigest, MAX_DELIVERY_RECORD_BYTES};

    use super::*;

    const CREDENTIAL: &str = "Credential/api-key";
    const CONSUMER_PROVIDER: &str = "Provider/credential-secret-service";
    const AUDIENCE: &str = "azure-resource-manager";
    const ROUTE_DIGEST: &str =
        "sha256:6f1c1b6f2a6f2cbb1f2e5b2f0c3a7a2b6c9d0e1f2a3b4c5d6e7f809a1b2c3d4e";

    struct Relationship {
        scoped_audience: AudienceToken,
        granted: Vec<OperationClass>,
        current: DeliveryIdentity,
        refusal: Option<BindingRefusal>,
    }

    impl AdmittedCredentialDelivery for Relationship {
        fn audience(&self) -> &AudienceToken {
            &self.scoped_audience
        }

        fn grants(&self, class: OperationClass) -> bool {
            self.granted.contains(&class)
        }

        fn current_delivery(&self) -> DeliveryIdentity {
            self.current.clone()
        }

        fn fence_refusal(&self, _: &CredentialDeliveryEvidence) -> Option<BindingRefusal> {
            self.refusal
        }
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("reference")
    }

    fn uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid")
    }

    fn generation(value: u64) -> ResourceGeneration {
        ResourceGeneration::new(value).expect("generation")
    }

    fn scoped(value: &str) -> AudienceToken {
        AudienceToken::parse(value).expect("audience")
    }

    fn identity(class: OperationClass, sequence: u64, for_audience: &str) -> DeliveryIdentity {
        DeliveryIdentity::new(
            reference(CREDENTIAL),
            uid(),
            generation(1),
            reference(CONSUMER_PROVIDER),
            generation(1),
            scoped(for_audience),
            class,
            sequence,
        )
    }

    fn delivery(class: OperationClass, sequence: u64, for_audience: &str) -> DeliverySessionParams {
        DeliverySessionParams::new(
            reference(CREDENTIAL),
            uid(),
            generation(1),
            reference(CONSUMER_PROVIDER),
            generation(1),
            scoped(for_audience),
            class,
            u64::MAX,
            u64::MAX,
            DeliveryRouteDigest::parse(ROUTE_DIGEST).expect("route digest"),
            MAX_DELIVERY_RECORD_BYTES as u32,
            sequence,
        )
        .expect("delivery session")
    }

    fn evidence() -> CredentialDeliveryEvidence {
        CredentialDeliveryEvidence::new(generation(1), generation(1), generation(1), 1)
    }

    fn relationship(
        class: OperationClass,
        granted: &[OperationClass],
        sequence: u64,
    ) -> Relationship {
        Relationship {
            scoped_audience: scoped(AUDIENCE),
            granted: granted.to_vec(),
            current: identity(class, sequence, AUDIENCE),
            refusal: None,
        }
    }

    fn authorization(class: OperationClass, sequence: u64, for_audience: &str) -> CredentialAuthorization {
        let method = match class {
            OperationClass::RefreshToken => CredentialMethod::RefreshToken,
            _ => CredentialMethod::AcquireToken,
        };
        CredentialAuthorization::new(method, Some(delivery(class, sequence, for_audience)))
            .expect("authorization")
    }

    #[test]
    fn a_matching_session_is_admitted_unchanged() {
        let relationship = relationship(OperationClass::AcquireToken, &[OperationClass::AcquireToken], 4);
        let params = admit_credential_delivery(
            &authorization(OperationClass::AcquireToken, 4, AUDIENCE),
            CredentialMethod::AcquireToken,
            &relationship,
            &evidence(),
        )
        .expect("admitted delivery");
        assert_eq!(params.sequence(), 4);
    }

    #[test]
    fn a_widened_audience_is_refused_by_the_source_policy() {
        let relationship = relationship(
            OperationClass::RefreshToken,
            &[OperationClass::AcquireToken, OperationClass::RefreshToken],
            1,
        );
        let refusal = admit_credential_delivery(
            &authorization(OperationClass::RefreshToken, 1, "storage-full-access"),
            CredentialMethod::RefreshToken,
            &relationship,
            &evidence(),
        )
        .expect_err("a widened audience is refused");
        assert_eq!(refusal.stage(), AdmissionStage::Admit);
        assert_eq!(refusal.reason(), RefusalReason::SourcePolicyRefused);
    }

    #[test]
    fn an_ungranted_operation_is_refused_by_the_source_policy() {
        let relationship = relationship(OperationClass::AcquireToken, &[OperationClass::AcquireToken], 1);
        let refusal = admit_credential_delivery(
            &authorization(OperationClass::RefreshToken, 1, AUDIENCE),
            CredentialMethod::RefreshToken,
            &relationship,
            &evidence(),
        )
        .expect_err("an ungranted operation is refused");
        assert_eq!(refusal.code(), "credential-delivery-outside-admitted-policy");
    }

    #[test]
    fn a_superseded_session_is_refused_at_activation() {
        let relationship = relationship(OperationClass::AcquireToken, &[OperationClass::AcquireToken], 2);
        let refusal = admit_credential_delivery(
            &authorization(OperationClass::AcquireToken, 1, AUDIENCE),
            CredentialMethod::AcquireToken,
            &relationship,
            &evidence(),
        )
        .expect_err("a superseded session is refused");
        assert_eq!(refusal.stage(), AdmissionStage::Activate);
        assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
    }

    #[test]
    fn a_moved_fence_is_refused_with_the_relationship_own_reason() {
        let mut relationship = relationship(OperationClass::AcquireToken, &[OperationClass::AcquireToken], 1);
        relationship.refusal = Some(BindingRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
        ));
        let moved = CredentialDeliveryEvidence::new(generation(1), generation(2), generation(1), 1);
        let refusal = admit_credential_delivery(
            &authorization(OperationClass::AcquireToken, 1, AUDIENCE),
            CredentialMethod::AcquireToken,
            &relationship,
            &moved,
        )
        .expect_err("a moved fence is refused");
        assert_eq!(refusal.code(), "credential-delivery-stale-authority");
    }

    #[test]
    fn a_protocol_method_has_no_delivery_session_to_admit() {
        let relationship = relationship(OperationClass::AcquireToken, &[OperationClass::AcquireToken], 1);
        let authorization = CredentialAuthorization::new(CredentialMethod::RevokeToken, None)
            .expect("authorization");
        let refusal = admit_credential_delivery(
            &authorization,
            CredentialMethod::RevokeToken,
            &relationship,
            &evidence(),
        )
        .expect_err("a protocol operation has no delivery session");
        assert_eq!(refusal.stage(), AdmissionStage::Prepare);
        assert_eq!(refusal.reason(), RefusalReason::MandatoryFacetUnsupported);
    }

    #[test]
    fn evidence_is_read_from_the_session_rather_than_the_caller() {
        let authorization = authorization(OperationClass::AcquireToken, 1, AUDIENCE);
        assert_eq!(
            observed_delivery_evidence(&authorization, 7)
                .expect_err("a delivery without a Provider session is denied")
                .code(),
            CredentialServiceErrorCode::OperationDenied
        );
    }
}