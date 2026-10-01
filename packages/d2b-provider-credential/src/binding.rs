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

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_provider::v3::credential::{
    AdmittedCredentialDelivery, AudienceToken,
    CredentialDeliveryEvidence as ObservedDeliveryEvidence, CredentialLeaseState, CredentialMethod,
    CredentialSpec, DeliveryIdentity, DeliveryRouteDigest, DeliverySessionParams, OperationClass,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingArbitration, BindingAuthorization,
    BindingConsumerKind, BindingContractError, BindingKey, BindingKind, BindingLifecycleState,
    BindingRealizationFacet, BindingRealizationSupport, BindingRefusal, BindingRowError,
    BindingSourceDecision, BindingSpecFingerprint, ControllerGeneration, CredentialBindingRequest,
    CredentialBindingSpec, CredentialLifetime, CredentialOperation, FreshnessTuple,
    MAX_CREDENTIAL_LIFETIME_MS, MIN_CREDENTIAL_LIFETIME_MS, RefusalReason, RequestedRights,
    ResourceGeneration, ResourceRef, ResourceSpec, ResourceUid, SourceAdmission, ZoneId,
    admit_binding_request, canonical_json_bytes, framed_canonical_digest,
    identity::ReconnectGeneration,
};
use d2b_resource_runtime::context::{
    ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType,
};

use crate::driver::{CredentialDependencyFacts, CredentialLeaseFacts, CredentialSourcePolicy};
use crate::session::{
    CredentialResourceRuntimeError, CredentialRevocationInputs, CredentialRevocationOutcome,
    CredentialRevocationReport, CredentialRevocationRequest, CredentialSession,
    is_credential_provider_ref,
};

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

/// The domain the deterministic `CredentialBinding` row name is minted under.
const BINDING_ROW_DOMAIN: &str = "d2b:v3:credential-binding-row";

/// The delivery operation class one service operation class names.
///
/// `None` for the two protocol operations - revocation and metadata
/// inspection - which run against the `Credential` row itself, establish no
/// delivery session, and therefore name no `CredentialBinding` relationship
/// at all. This is the exact inverse of the mapping a delivery session mints
/// through, so the classes a row grants and the classes a committed row names
/// cannot drift apart.
pub const fn delivery_operation(class: OperationClass) -> Option<CredentialOperation> {
    match class {
        OperationClass::AcquireToken => Some(CredentialOperation::AcquireToken),
        OperationClass::RefreshToken => Some(CredentialOperation::RefreshToken),
        OperationClass::SignChallenge => Some(CredentialOperation::SignChallenge),
        OperationClass::RevokeToken | OperationClass::InspectMetadata => None,
    }
}

/// The stable consumer slot a `Credential` row's delivery occupies.
///
/// A `Credential` row names at most one consumer and declares no second
/// delivery to that consumer, so the relationship it implies occupies exactly
/// one slot. The name is a literal rather than a digest because nothing about
/// it varies: it is the consumer's own local name for "the credential this
/// row delivers".
pub const CREDENTIAL_DELIVERY_SLOT: &str = "delivery";

/// The source's committed decision for one credential delivery relationship.
///
/// The admitted right is the kind's own: a `CredentialBinding` delivers, so
/// it consumes. The realized facets are read back from
/// [`credential_binding_support`] rather than spelled a second time, so what
/// a committed row declares cannot drift from what admission enforces.
pub fn credential_source_decision() -> BindingSourceDecision {
    BindingSourceDecision::new(
        vec![RequestedRights::Consume],
        BindingArbitration::Shared,
        credential_binding_support().facets().to_vec(),
    )
    .expect("one admitted right and the family's declared facets are unique by construction")
}

/// One canonical `CredentialBinding` row a committed `Credential` row owns.
///
/// The row is the binding family's committed shape of the relationship, so it
/// carries the source's own decision about it rather than a bare request: a
/// boundary rebuilding the accepted graph reads the admitted rights, the
/// arbitration, and the realized facets off the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalCredentialBinding {
    /// Deterministic row name derived from the relationship's identities.
    pub name: String,
    /// Exact canonical row bytes committed as this row's base spec.
    pub spec: Vec<u8>,
}

/// The deterministic row name one committed delivery relationship is minted
/// under.
///
/// The name derives from the identities the row itself carries - the bound
/// `Credential`, the consumer, and the stable slot - and never from the
/// operation set, the lifetime, or a declaration position. Widening the
/// admitted operations or shortening the lease therefore updates one
/// relationship instead of minting a second row beside it, and two
/// relationships cannot collide by ordering.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when the identities do not
/// render canonical bytes or the minted name is not a bounded token.
pub fn credential_binding_row_name(
    row: &CredentialBindingSpec,
) -> Result<BoundedToken, BindingContractError> {
    let identities = [
        row.credential_ref().to_canonical_string(),
        row.execution_ref().to_canonical_string(),
        row.slot().as_str().to_owned(),
    ];
    let digest = framed_canonical_digest(
        BINDING_ROW_DOMAIN,
        &canonical_json_bytes(&identities).map_err(|_| BindingContractError::InvalidField)?,
    );
    BoundedToken::parse(format!("cred-binding-{}", &digest[7..31]))
        .map_err(|_| BindingContractError::InvalidField)
}

/// Derive the canonical `CredentialBinding` rows one committed `Credential`
/// row owns.
///
/// A row implies a delivery relationship only where it actually declares one.
/// The row's scope names where the material may be used and names a `Host` or
/// a `Guest`; the `Credential` binding kind admits every consumer kind except
/// the `Host`, because a host-level need there is a child target-support
/// ceiling rather than a binding row. So a row scoped to a `Guest` implies
/// exactly one row delivering that Guest the operations the row grants for
/// the longest lifetime the row's own ceiling admits, and a row scoped to
/// the `Host`, an unscoped row, and a row granting only the two non-delivery
/// service operations each imply no relationship and yield no rows.
///
/// Every fact on the emitted row is read through the family's own
/// [`CredentialSourcePolicy`] and operation vocabulary: the admitted classes
/// come from [`CredentialSourcePolicy::admits_class`] through the same
/// service-operation mapping the delivery session mints through, and the
/// lifetime is the row's own `maxLeaseLifetimeMs` ceiling held inside the
/// binding contract's bounds. A row's `consumerRef` Provider is deliberately
/// not the consumer here: that Provider is the fence a delivery session is
/// minted against, not a binding consumer.
///
/// The emitted bytes are the row contract itself ([`CredentialBindingSpec`]),
/// which admits the consumer through `admit_binding_row_refs`, so a row
/// naming a consumer the kind does not admit is refused here rather than
/// committed and served.
///
/// # Errors
///
/// Returns [`BindingContractError`] when the declared consumer names no
/// consumer kind at all, when the derived row does not survive the row
/// contract, or when the row's own lifetime ceiling has no representation
/// between [`MIN_CREDENTIAL_LIFETIME_MS`] and [`MAX_CREDENTIAL_LIFETIME_MS`].
pub fn canonical_binding_rows(
    credential_ref: &ResourceRef,
    spec: &CredentialSpec,
) -> Result<Vec<CanonicalCredentialBinding>, BindingContractError> {
    let policy = CredentialSourcePolicy::from_spec(spec);
    let Some(consumer) = spec.scope().execution_ref() else {
        return Ok(Vec::new());
    };
    let Some(kind) = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
    else {
        return Err(BindingContractError::WrongResourceType);
    };
    if !BindingKind::Credential.admits_consumer(kind) {
        return Ok(Vec::new());
    }
    // The operation set is what the row grants, read through the family's own
    // vocabulary. It is ordered here rather than relying on the row having
    // stored its own set sorted, so the committed row is canonical however the
    // source authored it.
    let mut operations: Vec<CredentialOperation> = policy
        .allowed_operations()
        .iter()
        .copied()
        .filter_map(delivery_operation)
        .collect();
    operations.sort_unstable();
    if operations.is_empty() {
        return Ok(Vec::new());
    }
    let row = CredentialBindingSpec::new(
        credential_ref.clone(),
        consumer.clone(),
        operations,
        delivery_lifetime_ms(policy.max_lease_lifetime_ms())?,
        BoundedToken::parse(CREDENTIAL_DELIVERY_SLOT)
            .map_err(|_| BindingContractError::InvalidField)?,
        credential_source_decision(),
    )
    .map_err(refuse_row)?;
    let name = credential_binding_row_name(&row)?;
    let bytes = canonical_json_bytes(&row).map_err(|_| BindingContractError::InvalidField)?;
    Ok(vec![CanonicalCredentialBinding {
        name: name.as_str().to_owned(),
        spec: bytes,
    }])
}

/// Why one derived delivery row is not committed.
///
/// A derived row is a candidate, not an authority: it becomes a committed
/// relationship only when it survives every bound the source row's own
/// committed spec imposes, and a row that does not is refused here rather than
/// committed and served. Every variant names which bound refused. None carries
/// a credential reference, an audience, or a Provider identity, so a refusal
/// reads the same whether it came from a committed spec or from a derivation
/// fault (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialBindingCommitRefusal {
    /// The derived bytes are not this family's own `CredentialBinding` row.
    RowContract(BindingContractError),
    /// The derived row is bound to a `Credential` other than the one deriving
    /// it.
    SourceMismatch,
    /// The derived row delivers somewhere the source row's own scope does not
    /// admit. The consumer is the source's `scope.executionRef` and never its
    /// `consumerRef`: that Provider is the fence a delivery session is minted
    /// against, not the party the material reaches.
    ConsumerNotScoped,
    /// The derived row's committed name is not the name these identities
    /// derive, which is the name fence the serving driver reads back.
    RowNameNotDerived,
    /// The derived row names a delivery operation the source's own policy
    /// does not grant.
    OperationNotAdmitted(CredentialOperation),
    /// The derived row asks for a lease longer than the source's own
    /// `maxLeaseLifetimeMs` ceiling.
    LifetimeAboveCeiling {
        /// The lease the row asks for.
        lifetime_ms: u64,
        /// The ceiling the source row's own policy imposes.
        ceiling_ms: u64,
    },
}

impl CredentialBindingCommitRefusal {
    /// The stable code this refusal reports under.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::RowContract(_) => "credential-binding-row-contract-refused",
            Self::SourceMismatch => "credential-binding-source-mismatch",
            Self::ConsumerNotScoped => "credential-binding-consumer-not-scoped",
            Self::RowNameNotDerived => "credential-binding-row-name-not-derived",
            Self::OperationNotAdmitted(_) => "credential-binding-operation-not-admitted",
            Self::LifetimeAboveCeiling { .. } => "credential-binding-lifetime-above-ceiling",
        }
    }
}

impl core::fmt::Display for CredentialBindingCommitRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for CredentialBindingCommitRefusal {}

/// Re-check one derived delivery row against the `Credential` row that
/// derived it, before it may be committed.
///
/// Six closed checks, every one of them read from the source row's own
/// committed spec through the family's own vocabulary:
///
/// - the bytes are the family's `CredentialBinding` row, not a look-alike;
/// - the row is bound to this very `Credential`;
/// - the row delivers to the source's `scope.executionRef`. Its
///   `consumerRef` is never the consumer: that Provider is the fence a
///   delivery session is minted against, so a row that delivered to it would
///   send the material to the wrong destination;
/// - the row's name is the name its own identities derive, which is the
///   fence [`credential_binding_row_name`] mints and the serving driver reads
///   back;
/// - every operation the row names is one the source's
///   `allowedOperations` grants, read through
///   [`CredentialSourcePolicy::admits_class`];
/// - the row's lifetime fits inside the source's own `maxLeaseLifetimeMs`
///   ceiling, where a zero ceiling leaves only the contract's own bound in
///   force.
///
/// The checks run on the bytes that would be committed, not on the inputs
/// they came from, so a row that reached this call by any other route is held
/// to exactly the same bounds as one the derivation just produced.
pub fn admitted_delivery_row(
    credential_ref: &ResourceRef,
    spec: &CredentialSpec,
    row: &CanonicalCredentialBinding,
) -> Result<(), CredentialBindingCommitRefusal> {
    let decoded: CredentialBindingSpec = serde_json::from_slice(&row.spec).map_err(|_| {
        CredentialBindingCommitRefusal::RowContract(BindingContractError::InvalidField)
    })?;
    if decoded.credential_ref() != credential_ref {
        return Err(CredentialBindingCommitRefusal::SourceMismatch);
    }
    if spec.scope().execution_ref() != Some(decoded.execution_ref()) {
        return Err(CredentialBindingCommitRefusal::ConsumerNotScoped);
    }
    let derived = credential_binding_row_name(&decoded)
        .map_err(|error| CredentialBindingCommitRefusal::RowContract(error))?;
    if derived.as_str() != row.name {
        return Err(CredentialBindingCommitRefusal::RowNameNotDerived);
    }
    let policy = CredentialSourcePolicy::from_spec(spec);
    for operation in decoded.operations() {
        if !policy.admits_class(operation_class(*operation)) {
            return Err(CredentialBindingCommitRefusal::OperationNotAdmitted(*operation));
        }
    }
    let ceiling = policy.max_lease_lifetime_ms();
    if ceiling != 0 && decoded.lifetime_ms() > ceiling {
        return Err(CredentialBindingCommitRefusal::LifetimeAboveCeiling {
            lifetime_ms: decoded.lifetime_ms(),
            ceiling_ms: ceiling,
        });
    }
    Ok(())
}

/// Derive the delivery rows one committed `Credential` row owns and hold each
/// of them to that row's own policy before any of them may be committed.
///
/// The derivation is [`canonical_binding_rows`] - the same one a boundary and
/// the serving driver agree on - and the returned rows are the exact bytes and
/// name this call site commits. One row outside the source's admitted
/// operations or above its lifetime ceiling refuses the whole set, so a pass
/// commits either every relationship the source declares or none of them.
pub fn admitted_binding_rows(
    credential_ref: &ResourceRef,
    spec: &CredentialSpec,
) -> Result<Vec<CanonicalCredentialBinding>, CredentialBindingCommitRefusal> {
    let rows = canonical_binding_rows(credential_ref, spec)
        .map_err(CredentialBindingCommitRefusal::RowContract)?;
    for row in &rows {
        admitted_delivery_row(credential_ref, spec, row)?;
    }
    Ok(rows)
}

/// The delivery lifetime one `Credential` row commits to, in milliseconds.
///
/// A zero cap is the Provider default, which leaves the binding contract's
/// own bound in force; a declared cap is the row's own ceiling, so the row
/// requests the longest lifetime it admits and no longer. A ceiling shorter
/// than the contract's own floor admits no lifetime the row could express,
/// which is a refusal rather than a silently widened bound.
fn delivery_lifetime_ms(max_lease_lifetime_ms: u64) -> Result<u64, BindingContractError> {
    let ceiling = if max_lease_lifetime_ms == 0 {
        MAX_CREDENTIAL_LIFETIME_MS
    } else {
        max_lease_lifetime_ms.min(MAX_CREDENTIAL_LIFETIME_MS)
    };
    if ceiling < MIN_CREDENTIAL_LIFETIME_MS {
        return Err(BindingContractError::OutOfRange);
    }
    Ok(ceiling)
}

/// The family-wide refusal one row-contract refusal is reported as.
const fn refuse_row(refusal: BindingRowError) -> BindingContractError {
    match refusal {
        BindingRowError::WrongSourceType => BindingContractError::WrongResourceType,
        BindingRowError::WrongConsumerType | BindingRowError::ConsumerNotAdmitted => {
            BindingContractError::UnsupportedConsumerKind
        }
        BindingRowError::InvalidOperations | BindingRowError::DuplicateOperation => {
            BindingContractError::InvalidCollection
        }
        BindingRowError::LifetimeOutOfBounds => BindingContractError::OutOfRange,
    }
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


    /// Fail closed unless new use is still admitted.
    fn check_admitted(&self) -> Result<(), CredentialDeliveryRefusal> {
        if self.admits_new_use {
            Ok(())
        } else {
            Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Revoke,
                RefusalReason::UnprovenEffect,
            ))
        }
    }

    /// Fail closed unless the observable generations still match every fence.
    ///
    /// `rotation` is `None` for a caller that cannot observe the source's
    /// credential rotation counter - a Provider-side session cannot, and the
    /// rotation fence is the minting source's own check. Every generation it
    /// *can* observe is compared.
    fn check_generations(
        &self,
        credential: ResourceGeneration,
        consumer: ResourceGeneration,
        provider: ResourceGeneration,
        rotation: Option<u64>,
    ) -> Result<(), CredentialDeliveryRefusal> {
        if credential != self.fence.credential_generation
            || consumer != self.fence.consumer_component_generation
            || provider != self.fence.provider_generation
            || rotation.is_some_and(|value| value != self.fence.rotation_generation)
        {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            ));
        }
        Ok(())
    }

    /// Fail closed unless the delivery lifetime has not elapsed.
    fn check_lifetime(&self, now_unix_ms: u64) -> Result<(), CredentialDeliveryRefusal> {
        if now_unix_ms >= self.fence.deadline_unix_ms || now_unix_ms >= self.fence.expiry_unix_ms {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Drain,
                RefusalReason::UnprovenEffect,
            ));
        }
        Ok(())
    }

    /// Fail closed unless the live evidence still matches every fence.
    fn check_fence(
        &self,
        evidence: &CredentialDeliveryEvidence,
    ) -> Result<(), CredentialDeliveryRefusal> {
        self.check_admitted()?;
        self.check_generations(
            evidence.credential_generation,
            evidence.consumer_component_generation,
            evidence.provider_generation,
            Some(evidence.rotation_generation),
        )?;
        if !self.admission.is_current(&evidence.dependencies) {
            return Err(CredentialDeliveryRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            ));
        }
        self.check_lifetime(evidence.now_unix_ms)
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

/// The admitted `CredentialBinding` relationship as a Credential Provider sees
/// it (U23).
///
/// This is the one implementation of the family-wide delivery port, so every
/// Credential Provider realization reaches the same admitted relationship
/// through the same gate instead of comparing the authenticated route against
/// its own hand-written rules. It adds no authority of its own: the audience,
/// the granted operation classes, and the current delivery session all come
/// from the relationship this object already holds, and the fence check is the
/// same one the mint path runs.
impl AdmittedCredentialDelivery for CredentialDeliveryAuthority {
    fn audience(&self) -> &AudienceToken {
        &self.audience
    }

    fn grants(&self, class: OperationClass) -> bool {
        self.operations.contains(&class)
    }

    fn current_delivery(&self) -> DeliveryIdentity {
        self.delivery_identity()
    }

    fn fence_refusal(&self, evidence: &ObservedDeliveryEvidence) -> Option<BindingRefusal> {
        // The admitted dependency revisions and the credential rotation
        // generation are the minting source's own evidence: a Provider-side
        // session carries neither, so they stay the mint path's check. Every
        // generation and every lifetime bound a Provider *can* observe is
        // compared here through the same steps `mint` runs.
        self.check_admitted()
            .and_then(|()| {
                self.check_generations(
                    evidence.credential_generation(),
                    evidence.consumer_component_generation(),
                    evidence.provider_generation(),
                    None,
                )
            })
            .and_then(|()| self.check_lifetime(evidence.now_unix_ms()))
            .err()
            .map(|refusal| BindingRefusal::new(refusal.stage(), refusal.reason()))
    }
}

// ---------------------------------------------------------------------------
// The `CredentialBinding` serving driver (U37)
// ---------------------------------------------------------------------------
//
// The driver serves the committed row the source admitted: it decodes the
// neutral contract through the row's own wire decoder, enforces the committed
// `BindingSourceDecision`, resolves the parent `Credential` row and the
// consumer row through the manager behind their fences, and drives what the
// family can honestly realize through [`CredentialBindingEffects`].
//
// # Which verbs of the realization this driver can honestly serve
//
// The `CredentialDelivery` realization is one facet, and separating what exists
// from what does not separates the verbs cleanly:
//
// - **`clock` IS routed.** Every lifetime bound is compared against the
//   observation clock, and it rides the port rather than the process clock so
//   the family holds no ambient authority and a test can pin the answer.
//
// - **`revoke` IS routed, through the preserved protocol call.** The daemon
//   holds an authenticated Credential session per Provider
//   ([`CredentialSession`](crate::session::CredentialSession)) whose
//   `revoke_credential` issues the `RevokeToken` ttrpc call on
//   `d2b.credential.v3.CredentialService`
//   (`packages/d2bd/src/credential_resource_runtime.rs:81`). This driver
//   builds a [`CredentialRevocationRequest`] through the same
//   [`CredentialRevocationInputs`] the `Credential` row's own teardown uses and
//   calls that same method, so there is one revocation authority rather than
//   two.
//
// - **`deliver` IS NOT routable, and this driver does not pretend otherwise.**
//   Minting delivery evidence is source-side and runs through
//   [`CredentialDeliveryAuthority::mint`], which takes the whole
//   [`CredentialBindingAdmission`]: the credential generation, the consumer
//   component generation, the Provider generation, the credential rotation
//   generation, the admitted dependency revisions, the audience, the delivery
//   route digest, and the token ceiling, and refuses when any of them moved.
//   None of that is reconstructible from a committed row plus a manager read -
//   the admitted dependency revisions and the delivery route digest in
//   particular exist only inside the source's admission. The receiving side
//   does not exist either: `CredentialSession` has exactly two methods,
//   `session_generation` and `revoke_credential`, so there is no
//   `AcquireToken`/`RefreshToken`/`SignChallenge` client to hand minted
//   [`DeliverySessionParams`] to.
//
//   The minimal broker-side surface that would close this is one method on
//   `CredentialSession`, plus the daemon-side delivery mint it forwards:
//
//   ```text
//   async fn deliver(
//       &self,
//       params: &DeliverySessionParams,
//       method: CredentialMethod,
//   ) -> Result<CredentialResponse, CredentialResourceRuntimeError>;
//   ```
//
//   where `params` is exactly what `CredentialDeliveryAuthority::mint`
//   produced and the daemon forwards it as the matching ttrpc method on
//   `d2b.credential.v3.CredentialService`. The method-name table and the
//   response shape already exist in the Provider server
//   (`d2b-provider-toolkit`, `server/credential.rs:331-335` and
//   `CredentialResponse::AcquireToken`); what is missing is the client side
//   and the mint, which are source-side by construction. Until both exist the
//   driver reports the delivery as a named refusal rather than as a delivery,
//   and there is no `deliver` verb on the port to imply otherwise.

/// The `Credential` ResourceType the committed relationship's source is.
const CREDENTIAL_RESOURCE_TYPE: &str = "Credential";

/// Canonical `CredentialBinding` ResourceType name.
pub const CREDENTIAL_BINDING_TYPE_NAME: &str =
    d2b_contracts_resource::v3::credential_binding::CREDENTIAL_BINDING_RESOURCE_TYPE;

/// The re-check cadence while the committed relationship is not serving.
///
/// The generations the source's own fence compares - the credential rotation
/// generation above all - reach this actor as no watch delivery on the binding
/// row, so an unserved relationship re-checks on this interval. The same shape
/// the Endpoint and Volume binding drivers use while their delivery is not yet
/// provable.
const CREDENTIAL_BINDING_RESYNC: Duration = Duration::from_secs(5);

/// Closed, field-free classifications of a serving failure on this row.
///
/// No variant carries a credential reference, an audience, a route digest, or a
/// generation: a failure names which check refused and nothing else (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingDriverErrorKind {
    /// The durable spec did not decode as the strict neutral binding
    /// contract, or the row name is not the one this source derives.
    SpecInvalid,
    /// The committed `BindingSourceDecision` does not admit what the row
    /// claims: an admitted-right set without the consuming right, realized
    /// facets without the delivery facet, a facet outside the family's
    /// declared support, or an arbitration this family never commits.
    DecisionRefused,
    /// The parent `Credential` row is present but its owner uid differs from
    /// this binding's owner: the manager would silently re-parent. Terminal.
    OwnerMismatch,
    /// The parent `Credential` row is not observable yet, or holds no usable
    /// credential spec. The former defers retryably (issue #511); the latter
    /// is terminal because the committed row cannot converge by retrying.
    ParentUnavailable,
    /// The parent row decodes, but its own source policy no longer admits this
    /// relationship's operation set or its lifetime.
    ParentPolicyRefused,
    /// The named consumer row is not observable yet (retryable) or does not
    /// exist at all (terminal).
    ConsumerUnavailable,
    /// The protocol revocation call could not be confirmed, so cleanup stays
    /// withheld (R36).
    RevocationUnconfirmed,
}

impl BindingDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::ParentUnavailable
            | Self::ConsumerUnavailable
            | Self::RevocationUnconfirmed => FailureClass::Retryable,
            Self::SpecInvalid
            | Self::DecisionRefused
            | Self::OwnerMismatch
            | Self::ParentPolicyRefused => FailureClass::Terminal,
        }
    }

    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid | Self::DecisionRefused => FailureKinds::BINDING_SPEC_INVALID,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::ParentUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::ParentPolicyRefused => FailureKinds::BINDING_PLAN_DERIVATION_INVALID,
            Self::ConsumerUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::RevocationUnconfirmed => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
        }
    }
}

/// Typed serving failure, mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`].
#[derive(Debug, Clone)]
pub struct BindingDriverError {
    kind: BindingDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl BindingDriverError {
    fn new(kind: BindingDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for BindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            BindingDriverErrorKind::SpecInvalid => "binding-spec-invalid",
            BindingDriverErrorKind::DecisionRefused => "binding-decision-refused",
            BindingDriverErrorKind::OwnerMismatch => "binding-owner-mismatch",
            BindingDriverErrorKind::ParentUnavailable => "binding-parent-unavailable",
            BindingDriverErrorKind::ParentPolicyRefused => "binding-parent-policy-refused",
            BindingDriverErrorKind::ConsumerUnavailable => "binding-consumer-unavailable",
            BindingDriverErrorKind::RevocationUnconfirmed => "binding-revocation-unconfirmed",
        })
    }
}

impl std::error::Error for BindingDriverError {}

/// Typed in-memory status projection (R11: never persisted).
///
/// Every field is a bound or a state. There is no field a credential byte
/// could occupy, so an audit record, a status API, and a publication snapshot
/// all read the same closed shape and none of them can leak one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialBindingDriverStatus {
    /// The relationship is fenced: pre-drain ran and new use is refused while
    /// the outstanding session drains.
    Draining {
        /// The absolute expiry the row's own lifetime bounds to.
        expiry_unix_ms: u64,
    },
    /// No delivery session is standing.
    ///
    /// There is deliberately no "admitted" or "delivered" variant: the delivery
    /// facet is the only facet this family commits and the mint path cannot be
    /// driven from a committed row, so a row that is structurally perfect still
    /// reports undelivered rather than a delivery this driver cannot mint. See
    /// the module section on which verbs the family can honestly serve.
    Undelivered {
        /// Closed, field-free: why no delivery session is standing.
        reason: UndeliveredReason,
    },
}

/// The closed set of reasons a delivery session is not standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndeliveredReason {
    /// The row's own lifetime has elapsed against the observation clock, so no
    /// delivery may be minted for it.
    LifetimeElapsed,
    /// The mint path is source-side and needs the full admission evidence, and
    /// no acquire client exists to receive what it mints. Named rather than
    /// approximated: the family does not claim a delivery it cannot mint.
    MintPathUnroutable,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one `CredentialBinding` row, exactly as
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BindingSpecEnvelope {
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for `CredentialBinding` rows.
pub fn credential_binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| BindingSpecEnvelope {
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The live-host surfaces the `CredentialBinding` serving driver needs.
///
/// No verb here carries credential material. There is deliberately no
/// `deliver` verb: the mint path cannot be driven from a committed row, and
/// declaring a verb the daemon cannot honour would be a surface nothing can
/// reach.
#[async_trait::async_trait]
pub trait CredentialBindingEffects: Send + Sync + 'static {
    /// The observation clock in unix milliseconds.
    ///
    /// Every lifetime bound this driver evaluates is compared against this
    /// clock rather than the process clock, so the family holds no ambient
    /// authority and a test can pin the answer.
    fn now_unix_ms(&self) -> u64;

    /// Provider + execution-target facts, the same read the `Credential`
    /// driver's own teardown binds its revocation request against.
    ///
    /// `Ok(None)` when the Provider row is not observable; `Err` when the
    /// dependency read itself failed, so a failed read is never answered as
    /// absence.
    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<Option<CredentialDependencyFacts>, CredentialResourceRuntimeError>;

    /// Provider-side lease facts for one `Credential` row.
    ///
    /// `None` is exactly the "no lease state" case, in which the revocation is
    /// skipped - the same skip the `Credential` row's own teardown performs.
    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts>;

    /// The live Provider session generation for one Credential Provider.
    ///
    /// `Ok(None)` means no session surface exists at all, and the inner `None`
    /// means the session exists but is not live. Both fail a revocation closed
    /// rather than letting it bind a zero generation (R28).
    async fn session_generation(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Option<ReconnectGeneration>, CredentialResourceRuntimeError>;

    /// Revoke one credential lease through the authenticated Provider
    /// session.
    ///
    /// This is the preserved protocol call, not a second authority: the
    /// production implementation is the daemon's
    /// [`CredentialSession`](crate::session::CredentialSession), whose
    /// `revoke_credential` issues the `RevokeToken` ttrpc call on
    /// `d2b.credential.v3.CredentialService`.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialResourceRuntimeError`] when the session refuses the
    /// request's identity, no session surface exists, or the Provider cannot
    /// confirm the revocation. The caller withholds cleanup on any of them
    /// (R36).
    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError>;
}

/// The provider-owned binding effects, built from the daemon-supplied facet
/// set this family already declares (R2).
///
/// The clock is the one host fact read without a facet: it is the process
/// observation every lifetime bound is measured against, and the composition
/// root holds no daemon type this could otherwise borrow.
pub struct CredentialBindingEffectsService {
    runtime: Arc<dyn crate::facets::CredentialRuntime>,
}

impl CredentialBindingEffectsService {
    /// Build the binding effects from one zone's daemon-supplied facet set.
    pub fn new(facets: crate::facets::CredentialEffectFacets) -> Self {
        Self { runtime: facets.runtime }
    }
}

#[async_trait::async_trait]
impl CredentialBindingEffects for CredentialBindingEffectsService {
    fn now_unix_ms(&self) -> u64 {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }

    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<Option<CredentialDependencyFacts>, CredentialResourceRuntimeError> {
        self.runtime.dependency_facts(provider_ref, execution_ref).await
    }

    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
        self.runtime.lease_facts(credential_ref).await
    }

    async fn session_generation(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Option<ReconnectGeneration>, CredentialResourceRuntimeError> {
        Ok(self.effects_session(provider_ref)?.session_generation())
    }

    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
        self.effects_session(&request.provider_ref)?
            .revoke_credential(request)
            .await
    }
}

impl CredentialBindingEffectsService {
    /// The authenticated Provider session one Provider reference names.
    ///
    /// This is the daemon's own `ProviderSupervisor` handoff registry, read
    /// through the family facet. `None` means no session surface exists at
    /// all, so a revocation bound to it fails closed rather than guessing
    /// (R28).
    fn effects_session(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Arc<dyn CredentialSession>, CredentialResourceRuntimeError> {
        self.runtime
            .session(provider_ref)
            .ok_or(CredentialResourceRuntimeError::InvalidResource)
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the `CredentialBinding`
/// driver factory for one zone: the zone, the zone-authority controller
/// generation folded into every revocation request (KTD7), and the
/// daemon-supplied facet set the family's own effects implementation is built
/// from (R2).
pub struct CredentialBindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: ZoneId,
    /// Zone controller generation folded into every revocation request.
    pub controller_generation: ControllerGeneration,
    /// The daemon-supplied facet set. The family never receives a
    /// daemon-built effect port.
    pub facets: crate::facets::CredentialEffectFacets,
}

/// [`ResourceDriverFactory`] for the `CredentialBinding` resource type.
/// Construction is infallible by contract (R3).
pub struct CredentialBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: CredentialBindingDriverArgs,
}

impl CredentialBindingDriverFactory {
    /// Build the factory for one zone's plane.
    pub fn new(args: CredentialBindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(CREDENTIAL_BINDING_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for CredentialBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(CredentialBindingDriver::new(CredentialBindingDriverArgs {
            zone: self.args.zone.clone(),
            controller_generation: self.args.controller_generation,
            facets: self.args.facets.clone(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// The parent `Credential` row's store-assigned identity, read through the
/// manager.
///
/// The revocation request binds the Credential row's uid and generation, so
/// they are read here rather than derived from the binding row: a binding row
/// carries the Credential's *reference*, never its identity, and inventing the
/// uid would let a request ride a credential the source never admitted.
struct ParentIdentity {
    uid: ResourceUid,
    generation: ResourceGeneration,
}

/// One committed `CredentialBinding` row's driver.
///
/// The driver holds no host state and no credential material: the Provider
/// session, the protocol revocation call, the dependency reads, and the
/// observation clock all arrive through [`CredentialBindingEffects`], which the
/// composition root builds from this family's already-declared facets.
pub struct CredentialBindingDriver {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    effects: Arc<dyn CredentialBindingEffects>,
    /// Rows this driver already registered a dependency watch on (R12/R17).
    /// Runtime-only (R6/R11): one registration per target keeps the dependency
    /// edge that wakes the actor on a dependency's death or readiness without
    /// accumulating manager watch entries.
    watched: Vec<ResourceKey>,
}

impl CredentialBindingDriver {
    fn new(args: CredentialBindingDriverArgs) -> Self {
        // The driver builds its effects from the declared facets; no externally
        // built port appears at this construction site (R2).
        let effects = Arc::new(CredentialBindingEffectsService::new(args.facets));
        Self {
            zone: args.zone,
            controller_generation: args.controller_generation,
            effects,
            watched: Vec::new(),
        }
    }

    fn error(&self, kind: BindingDriverErrorKind, op: DriverOp) -> BindingDriverError {
        BindingDriverError::new(kind, op)
    }

    /// Decode the stored envelope into the strict neutral binding contract.
    ///
    /// The wire decoder is the contract's own, so a stored row that is not
    /// canonical `CredentialBinding` bytes - an unknown field, a consumer this
    /// kind does not admit, a duplicate operation, a lifetime out of bounds -
    /// is refused here rather than half-read.
    fn decoded_binding(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<CredentialBindingSpec, BindingDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        let binding = serde_json::from_slice::<CredentialBindingSpec>(
            &envelope.base.to_canonical_bytes(),
        )
        .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        self.check_row_name(ctx, &binding, op)?;
        Ok(binding)
    }

    /// The row's own name must be the one this source derives.
    ///
    /// [`credential_binding_row_name`] is a deterministic function of the
    /// identities the row itself carries, so a committed row whose name is
    /// anything else was not minted by this source's admission. Checking it
    /// here is what makes the row a boundary reads back and the admission that
    /// minted it two views of ONE derivation rather than two descriptions that
    /// can drift.
    fn check_row_name(
        &self,
        ctx: &ResourceContext,
        binding: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        let derived = credential_binding_row_name(binding)
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        if derived.as_str() != ctx.key().name {
            return Err(self
                .error(BindingDriverErrorKind::SpecInvalid, op)
                .with_detail(FailureDetail::at("spec/rowName").comparison(
                    FailureComparison::new("binding.rowName", derived.as_str(), &ctx.key().name),
                )));
        }
        Ok(())
    }

    /// The committed `BindingSourceDecision` must admit what the row claims.
    ///
    /// Four refusals, all terminal, all read back out of the committed bytes:
    ///
    /// - an arbitration this family never commits. A `Credential` row names at
    ///   most one consumer and admits each delivery alongside its peers, so a
    ///   row claiming exclusivity was not minted here.
    /// - an admitted-right set that does not cover the consuming right. A
    ///   `CredentialBinding` delivers, so it consumes; a row admitted for
    ///   observation alone was not minted by this family.
    /// - realized facets that do not cover the delivery facet. A row committed
    ///   without the delivery facet declares no realization this family drives.
    /// - a facet the source committed that this family does not declare it can
    ///   realize. A committed facet is read back as something the source
    ///   admitted through, and the source may only commit what it can deliver.
    fn check_committed_decision(
        &self,
        binding: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        let source = binding.source();
        let support = credential_binding_support();
        if source.arbitration() != BindingArbitration::Shared {
            return Err(self.decision_error(op, "source.arbitration", "shared", "exclusive"));
        }
        let claimed = RequestedRights::Consume;
        if !source.admitted_rights().contains(&claimed) {
            return Err(self.decision_error(
                op,
                "source.admittedRights",
                wire(&claimed),
                "absent",
            ));
        }
        let required = BindingRealizationFacet::CredentialDelivery;
        if !source.realized_facets().contains(&required) {
            return Err(self.decision_error(
                op,
                "source.realizedFacets",
                wire(&required),
                "absent",
            ));
        }
        if let Some(unsupported) = source
            .realized_facets()
            .iter()
            .find(|facet| !support.facets().contains(facet))
        {
            return Err(self.decision_error(
                op,
                "source.realizedFacets",
                support
                    .facets()
                    .iter()
                    .map(wire)
                    .collect::<Vec<_>>()
                    .join(","),
                wire(unsupported),
            ));
        }
        Ok(())
    }

    /// The refusal detail for one committed decision that does not admit the
    /// row's own claim.
    fn decision_error(
        &self,
        op: DriverOp,
        field: &'static str,
        expected: impl core::fmt::Display,
        observed: impl core::fmt::Display,
    ) -> BindingDriverError {
        self.error(BindingDriverErrorKind::DecisionRefused, op)
            .with_detail(
                FailureDetail::at(match field {
                    "source.arbitration" => "spec/source.arbitration",
                    "source.admittedRights" => "spec/source.admittedRights",
                    _ => "spec/source.realizedFacets",
                })
                .comparison(FailureComparison::new(
                    field,
                    expected.to_string(),
                    observed.to_string(),
                )),
            )
    }

    /// The key of the parent `Credential` this binding declares.
    ///
    /// The key is built in this driver's own Zone, so a cross-Zone source is
    /// structurally unnameable rather than checked for: a primitive binding is
    /// same-Zone, and the Zone this row reconciles in is the Zone both sides
    /// live in.
    fn parent_credential_key(&self, binding: &CredentialBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            CREDENTIAL_RESOURCE_TYPE,
            binding.credential_ref().name().as_str(),
        )
    }

    /// The key of the consumer this binding delivers to.
    fn consumer_key(&self, binding: &CredentialBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            binding.execution_ref().resource_type().as_str(),
            binding.execution_ref().name().as_str(),
        )
    }

    /// The parent `Credential` row through the manager (R2: the driver never
    /// touches the spec store), with the same-Zone and owner fences.
    ///
    /// The binding's declared `Credential` must be the row the manager reports
    /// as this resource's owner: the Credential source is what mints the
    /// relationship, so a binding whose owner is a different row is one the
    /// manager would silently re-parent.
    async fn parent_credential(
        &self,
        ctx: &mut ResourceContext,
        binding: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<(ParentIdentity, CredentialSpec), BindingDriverError> {
        let key = self.parent_credential_key(binding);
        let lookup = ctx.lookup(&key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
                // A non-present read defers: the row may not be committed yet,
                // and an unreadable payload is not terminal by itself (#511).
                let mut detail = FailureDetail::at("parent/lookup");
                if let Some(comparison) = lookup.failure_comparison("parent.credential", "present")
                {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                return Err(self
                    .error(BindingDriverErrorKind::ParentUnavailable, op)
                    .with_detail(detail));
            }
        };
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            return Err(self
                .error(BindingDriverErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("parent/owner").comparison(
                    FailureComparison::new("parent.ownerUid", uid_hex(owner), uid_hex(&row.uid)),
                )));
        }
        let identity = ParentIdentity {
            uid: ResourceUid::from_bytes(&row.uid).map_err(|_| {
                self.error(BindingDriverErrorKind::ParentUnavailable, op)
                    .with_detail(Self::parent_row_detail("parent.uid"))
            })?,
            generation: ResourceGeneration::new(row.generation).map_err(|_| {
                self.error(BindingDriverErrorKind::ParentUnavailable, op)
                    .with_detail(Self::parent_row_detail("parent.generation"))
            })?,
        };
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .map_err(|_| self.parent_spec_invalid(op))?;
        let spec = serde_json::from_slice::<CredentialSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.parent_spec_invalid(op))?;
        Ok((identity, spec))
    }

    /// The terminal classification for a present parent row whose stored spec
    /// does not decode (issue #508: this is not an ownership mismatch).
    fn parent_spec_invalid(&self, op: DriverOp) -> BindingDriverError {
        self.error(BindingDriverErrorKind::ParentUnavailable, op)
            .with_detail(Self::parent_row_detail("parent.spec"))
    }

    /// The comparison naming which part of the parent row was unusable.
    fn parent_row_detail(field: &'static str) -> FailureDetail {
        FailureDetail::at("parent/decode")
            .comparison(FailureComparison::new(field, "a canonical Credential row", "decode failed"))
    }

    /// The consumer row through the manager, with the same-Zone fence.
    ///
    /// The consumer is not this row's owner - it is the party the delivery is
    /// made to - so there is no owner fence here. What is read back is the
    /// store-assigned identity the KTD3 key is derived from, so a consumer that
    /// was replaced under the same name produces a different key rather than
    /// silently continuing the old relationship.
    async fn consumer_uid(
        &self,
        ctx: &mut ResourceContext,
        binding: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<ResourceUid, BindingDriverError> {
        let key = self.consumer_key(binding);
        let lookup = ctx.lookup(&key).await;
        match lookup {
            RowLookup::Present { row, .. } => ResourceUid::from_bytes(&row.uid)
                .map_err(|_| self.error(BindingDriverErrorKind::ConsumerUnavailable, op)),
            _ => {
                let mut detail = FailureDetail::at("consumer/lookup");
                if let Some(comparison) =
                    lookup.failure_comparison("consumer.executionRef", "present")
                {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                Err(self
                    .error(BindingDriverErrorKind::ConsumerUnavailable, op)
                    .with_detail(detail))
            }
        }
    }

    /// The parent's own source policy must still admit this relationship.
    ///
    /// The committed decision on the binding row records what the source
    /// admitted; this is the second, independent half - the `Credential` row's
    /// own policy - and it is the half a consumer cannot influence. Every
    /// delivery class the row names must be one the row grants, and the row's
    /// lifetime must fit inside the parent's own ceiling.
    fn check_parent_policy(
        &self,
        credential: &CredentialSpec,
        binding: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        let policy = CredentialSourcePolicy::from_spec(credential);
        let refused = |field: &'static str, expected: String| {
            self.error(BindingDriverErrorKind::ParentPolicyRefused, op)
                .with_detail(FailureDetail::at("parent/policy").comparison(
                    FailureComparison::new(field, expected, "absent"),
                ))
        };
        for operation in binding.operations() {
            // `operation_class` is the same total mapping `canonical_binding_rows`
            // derives the row's own operation set through, so the check reads
            // the source policy in the vocabulary the row was minted in rather
            // than translating twice.
            if !policy.admits_class(operation_class(*operation)) {
                return Err(refused(
                    "credential.allowedOperations",
                    wire(operation),
                ));
            }
        }
        let ceiling = policy.max_lease_lifetime_ms();
        if ceiling != 0 && binding.lifetime_ms() > ceiling {
            return Err(refused("credential.maxLeaseLifetimeMs", ceiling.to_string()));
        }
        Ok(())
    }

    /// Every check a serving pass runs before it touches the protocol: the
    /// wire decode, the committed decision, the parent row behind its owner
    /// fence, that row's own policy, and the consumer row.
    async fn resolved(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<(CredentialBindingSpec, ParentIdentity, CredentialSpec), BindingDriverError> {
        let binding = self.decoded_binding(ctx, op)?;
        self.check_committed_decision(&binding, op)?;
        let (identity, credential) = self.parent_credential(ctx, &binding, op).await?;
        self.check_parent_policy(&credential, &binding, op)?;
        self.consumer_uid(ctx, &binding, op).await?;
        Ok((binding, identity, credential))
    }

    /// Register one dependency watch, at most once per target (R12/R17).
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// The absolute expiry the committed row's own lifetime bounds to.
    ///
    /// The row carries one bounded lifetime and the contract holds the hard
    /// deadline at or before the absolute expiry, so one instant carries both
    /// bounds. Deriving it from the committed value is what makes the status a
    /// projection of the row rather than a second policy.
    fn expiry_unix_ms(&self, binding: &CredentialBindingSpec) -> u64 {
        self.effects
            .now_unix_ms()
            .saturating_add(binding.lifetime_ms())
    }

    /// The delivery state this pass observes.
    ///
    /// One closed class is decided here with real evidence: a row whose own
    /// lifetime has elapsed against the observation clock admits no delivery.
    /// The other is the mint path itself - it is source-side, needs the full
    /// admission evidence, and has no acquire client to receive what it mints -
    /// so it is reported as named rather than approximated. A row that passes
    /// every structural check still lands here, which is the honest answer and
    /// not a defect in the checks.
    fn delivery_state(
        &self,
        ctx: &mut ResourceContext,
        binding: &CredentialBindingSpec,
    ) -> CredentialBindingDriverStatus {
        let expiry_unix_ms = self.expiry_unix_ms(binding);
        if expiry_unix_ms <= self.effects.now_unix_ms() {
            return undelivered(UndeliveredReason::LifetimeElapsed);
        }
        // The fence lives in the in-memory status slot rather than a durable
        // field (R11), so a pre-drain that ran is still read back as fenced on
        // the next pass instead of being re-admitted.
        if matches!(
            ctx.status::<CredentialBindingDriverStatus>(),
            Some(CredentialBindingDriverStatus::Draining { .. })
        ) {
            return CredentialBindingDriverStatus::Draining { expiry_unix_ms };
        }
        undelivered(UndeliveredReason::MintPathUnroutable)
    }
}

/// The hex spelling one compared uid renders as.
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The canonical wire spelling one committed vocabulary value renders as.
///
/// Read back through the contract's own serde rename rather than a second
/// hand-written spelling, so a failure detail cannot drift from the bytes the
/// row was decoded from.
fn wire<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|rendered| rendered.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unrenderable".to_owned())
}

/// The status an unserved relationship publishes, with the driver converging.
fn undelivered(reason: UndeliveredReason) -> CredentialBindingDriverStatus {
    CredentialBindingDriverStatus::Undelivered { reason }
}

#[async_trait::async_trait]
impl ResourceDriver for CredentialBindingDriver {
    type Error = BindingDriverError;

    fn classify_error(&self, error: &BindingDriverError) -> DriverFailure {
        let failure = match error.kind {
            BindingDriverErrorKind::SpecInvalid
            | BindingDriverErrorKind::DecisionRefused
            | BindingDriverErrorKind::OwnerMismatch
            | BindingDriverErrorKind::ParentPolicyRefused => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::ParentUnavailable
            | BindingDriverErrorKind::ConsumerUnavailable => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::RevocationUnconfirmed => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Structural validation: the wire decode, the committed decision, the
    /// parent `Credential` row behind its owner fence, that row's own source
    /// policy, and the named consumer row.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.resolved(ctx, DriverOp::Validate).await?;
        Ok(())
    }

    /// Adoption of the pre-restart incarnation (F2).
    ///
    /// A relationship with no minted delivery session has nothing to adopt: the
    /// mint path never persists a session this actor could find, and adopting a
    /// delivery it cannot prove would be the restart-adoption failure R41
    /// exists to prevent. So a restart reports `Missing` and the next reconcile
    /// pass re-derives the relationship from the committed row.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        self.resolved(ctx, DriverOp::Recover).await?;
        Ok(RecoveryOutcome::Missing)
    }

    /// One reconcile pass: resolve the committed relationship through its
    /// fences and publish the in-memory status (R11).
    ///
    /// An unserved relationship re-checks on the preserved resync cadence,
    /// because the generations the source's fence compares - the credential
    /// rotation generation above all - reach this actor as no watch delivery on
    /// the binding row.
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let (binding, _identity, _credential) = self.resolved(ctx, op).await?;
        // Dependency edges (R12/R17): the Credential row and the consumer row
        // both wake this actor when they change.
        self.watch_once(ctx, self.parent_credential_key(&binding)).await;
        self.watch_once(ctx, self.consumer_key(&binding)).await;
        let status = self.delivery_state(ctx, &binding);
        ctx.set_status(status);
        if matches!(
            ctx.status::<CredentialBindingDriverStatus>(),
            Some(CredentialBindingDriverStatus::Undelivered { .. })
        ) {
            ctx.requeue_after(CREDENTIAL_BINDING_RESYNC);
        }
        // `Satisfied` is the driver's own convergence: this pass did its work
        // and published its result. The serving state itself is the typed
        // status, and an unserved relationship re-checks above rather than
        // deferring the row, so a consumer's own launch never forms a startup
        // cycle with the observation that it can see the relationship.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Pre-drain (KTD10, R36): block NEW use before anything else is torn
    /// down.
    ///
    /// The fence is the driver's own in-memory status (R11): a relationship
    /// that has run pre-drain keeps reporting
    /// [`CredentialBindingDriverStatus::Draining`] so the next reconcile pass
    /// does not hand it back an undelivered-but-open state. The protocol's own
    /// revocation is what retires the lease, and that is [`Self::delete`]'s
    /// work. Idempotent under retry, and a row whose spec no longer decodes
    /// converges without effects.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok((binding, _identity, _credential)) = self.resolved(ctx, op).await else {
            // Nothing durable to fence: converged without effects.
            return Ok(());
        };
        let expiry_unix_ms = self.expiry_unix_ms(&binding);
        ctx.set_status(CredentialBindingDriverStatus::Draining { expiry_unix_ms });
        Ok(())
    }

    /// Drain step (R10, F3): the relationship owns no child rows, so this is
    /// the generic children-first finalization and it converges immediately.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ParentUnavailable, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown: revoke the bound credential's lease through the authenticated
    /// Provider session.
    ///
    /// The call is the preserved protocol `RevokeToken`, built from the same
    /// [`CredentialRevocationInputs`] the `Credential` row's own teardown uses
    /// and issued through the same [`CredentialSession`] method, so there is
    /// one revocation authority rather than two. An unconfirmed revoke withholds
    /// cleanup (R36): the row keeps its durable deleting mark and the pass
    /// retries rather than reporting a release it cannot prove. Idempotent
    /// under retry - a confirmed revoke replays as
    /// [`CredentialRevocationOutcome::AlreadyRevoked`], which is confirmed - and
    /// a row whose spec no longer decodes converges without effects.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok((binding, identity, credential)) = self.resolved(ctx, op).await else {
            // Nothing durable to clean up; converged without effects.
            return Ok(());
        };
        self.revoke(&binding, &identity, &credential, op).await
    }
}

impl CredentialBindingDriver {
    /// One confirmed-or-withheld revocation through the authenticated Provider
    /// session.
    ///
    /// The three facts a revocation request binds that the driver cannot
    /// invent are read, not derived: the Credential row's own uid and
    /// generation come from the parent manager read, the Provider row's
    /// generation from the dependency facet, and the live session generation
    /// from the session itself. A missing one fails closed rather than
    /// defaulting, because a revocation bound to a zero generation is exactly
    /// the request that must never reach a Provider (R28).
    async fn revoke(
        &self,
        binding: &CredentialBindingSpec,
        identity: &ParentIdentity,
        credential: &CredentialSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        // The old runner's gate, byte for byte: only an `Active` or `Unknown`
        // lease is revoked, and `Expired`, `Revoked`, and "no lease fact"
        // skip it. A relationship that delivered nothing has nothing to retire.
        let lease = self.effects.lease_facts(binding.credential_ref()).await;
        if !matches!(
            lease.map(|facts| facts.state),
            Some(CredentialLeaseState::Active | CredentialLeaseState::Unknown)
        ) {
            return Ok(());
        }
        let unconfirmed =
            || self.error(BindingDriverErrorKind::RevocationUnconfirmed, op);
        let invalid = || {
            self.error(BindingDriverErrorKind::ParentPolicyRefused, op)
                .with_detail(FailureDetail::at("delete/revokeIdentity").comparison(
                    FailureComparison::new(
                        "revocation.identity",
                        "a live provider session",
                        "unavailable",
                    ),
                ))
        };
        // A `Credential` row names the Provider its delivery is scoped to; a
        // row that names none has no authenticated Provider to revoke through.
        let provider_ref = credential.consumer_ref().cloned().ok_or_else(invalid)?;
        if !is_credential_provider_ref(&provider_ref) {
            return Err(invalid());
        }
        let facts = self
            .effects
            .dependency_facts(&provider_ref, binding.execution_ref())
            .await
            .map_err(|_| unconfirmed())?
            .ok_or_else(invalid)?;
        let provider_generation =
            ResourceGeneration::new(facts.provider_generation).map_err(|_| invalid())?;
        // The session generation is the Provider's own live evidence. The
        // session is reached through the effect port's own request, so a
        // revoked relationship that cannot name a live session fails closed
        // instead of defaulting to a zero generation.
        let session_generation = self
            .live_session_generation(&provider_ref)
            .await
            .ok_or_else(invalid)?;
        let rotation_generation = lease
            .map(|facts| facts.rotation_generation)
            .filter(|generation| *generation != 0)
            .unwrap_or(1);
        let request = CredentialRevocationRequest::new(CredentialRevocationInputs {
            zone: ZoneId::parse(self.zone.as_str()).map_err(|_| invalid())?,
            credential_ref: binding.credential_ref().clone(),
            credential_uid: identity.uid.clone(),
            credential_generation: identity.generation,
            user_ref: credential.scope().user_ref().cloned(),
            provider_ref,
            provider_generation,
            controller_generation: self.controller_generation,
            session_generation,
            rotation_generation,
        })
        .map_err(|_| invalid())?;
        let outcome = self.effects.revoke_credential(&request).await.map_err(|_| {
            self.error(BindingDriverErrorKind::RevocationUnconfirmed, op)
                .with_detail(
                    FailureDetail::at("delete/revoke")
                        .comparison(FailureComparison::new(
                            "binding.revocation",
                            "confirmed",
                            "unconfirmed",
                        )),
                )
        })?;
        if outcome.is_confirmed() {
            tracing::info!(
                credential = %binding.credential_ref().to_canonical_string(),
                confirmed = true,
                "credential binding lease revocation confirmed",
            );
            return Ok(());
        }
        tracing::warn!(
            credential = %binding.credential_ref().to_canonical_string(),
            "credential binding lease revocation unconfirmed; cleanup withheld",
        );
        Err(unconfirmed())
    }

    /// The live Provider session generation one revocation binds.
    ///
    /// `None` means no live session surface exists at all, which fails the
    /// revocation closed rather than binding a zero generation (R28).
    async fn live_session_generation(
        &self,
        provider_ref: &ResourceRef,
    ) -> Option<ReconnectGeneration> {
        self.effects
            .session_generation(provider_ref)
            .await
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The execution domains the `CredentialBinding` type can be reconciled in.
///
/// Derived from the placement contract: `CredentialBinding` names no placement
/// anchor (`PlacementAnchor::canonical_for` resolves none), so a relationship
/// row never carries the canonical `spec.executionRef` and the plane reconciles
/// it on its containing Zone's Host. The consumer reference in the spec selects
/// the Guest or Process that receives the delivery, never where the
/// relationship row itself is reconciled.
const CREDENTIAL_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the binding driver reads while reconciling.
///
/// Derived from the driver's row reads: the bound `Credential` row for its own
/// policy and the owner fence, and the consumer row for the store-assigned
/// identity the KTD3 key is derived from.
const CREDENTIAL_BINDING_READS: &[WellKnownType] = &[WellKnownType::CREDENTIAL];

/// The `CredentialBinding` type's driver declaration.
///
/// `CredentialBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane
/// cannot serve a committed delivery relationship without it, so it must be
/// registered before the plane opens. The type is not exportable:
/// `ResourceExport` admits only qualified `*.d2bus.org.*Service` types, so a
/// relationship can never be an export subject. The driver serves no broker
/// operations, mints no children, contributes no startup steps, and declares no
/// hosted effects service: a relationship delivers one credential to one
/// consumer and owns nothing else, and a `ServiceDecl` with no host behind it
/// would be a surface nothing can reach.
pub fn credential_binding_descriptor(
    args: CredentialBindingDriverArgs,
) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::CREDENTIAL_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: CREDENTIAL_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: CREDENTIAL_BINDING_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: credential_binding_spec_decoder(),
        factory: Arc::new(CredentialBindingDriverFactory::new(args)),
    }
}
